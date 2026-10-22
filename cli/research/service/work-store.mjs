import { randomUUID } from 'node:crypto';
import { fail, text, digest, secretInUrl } from './security.mjs';

const job = (row, now) => row && ({ id: row.id, title: row.title, brief: row.brief, createdAt: row.created_at,
  archivedAt: row.archived_at, candidateScope: row.scope_json ? JSON.parse(row.scope_json) : null,
  objective: row.objective_json ? JSON.parse(row.objective_json) : null,
  guidance: row.guidance_json ? JSON.parse(row.guidance_json) : null,
  focused: row.focused_at != null,
  availability: row.discovery_key && row.availability === 'available' && row.valid_until <= now ? 'unknown' : row.availability || 'available' });
const runView = row => ({ id: row.id, workId: row.work_id, title: row.title, agentId: row.agent_id,
  sessionId: row.session_id, createdAt: row.created_at, finishedAt: row.finished_at, summary: row.summary,
  researchContext: row.context_json ? JSON.parse(row.context_json) : null });
const sessionView = row => ({ id: row.id, agentId: row.agent_id, harness: row.harness, createdAt: row.created_at,
  heartbeatAt: row.heartbeat_at, closedAt: row.closed_at });
const eventView = row => ({ sequence: row.seq, runId: row.run_id, actor: row.actor, kind: row.kind,
  message: row.message, referenceUrl: row.reference_url, createdAt: row.created_at });

// What an agent reports as it works, one short line each: the owner's live
// feed in the TUI. search: the query; read: a page it opened (its URL);
// value: a value read off a page; submit: a petri write it made.
export const REPORT_KINDS = ['search', 'read', 'value', 'submit', 'note', 'source_found', 'blocked'];

// An agent counts as researching for the owner's view while it has checked in
// this recently: a display rule (the TUI's "researching"), never a limit on agents.
const ACTIVE_MS = 20 * 60000;

// How much of its targets a gap still lacks, 0 to 1 (the TUI's temperature).
function need(work) {
  const o = work.objective; if (!o?.targets) return 0;
  const earns = atoms => BigInt(/^\d+$/u.test(atoms ?? '') ? atoms : '0') > 0n ? 0.5 : 0;
  // An update earns from its month's remaining rewards, an opening from its
  // product's opening budget, if any.
  if (o.kind === 'update') return earns(o.rewardsRemainingAtomic);
  if (o.kind === 'opening') return earns(o.openingBudgetAtomic);
  if (o.kind === 'claims') return 1;
  const short = (have, want) => want > 0 ? Math.min(1, Math.max(0, (want - (have || 0)) / want)) : 0;
  return Math.max(short(o.activeSources, o.targets.activeSources), typeof o.distinctHosts === 'number' ? short(o.distinctHosts, o.targets.distinctHosts) : 0);
}

export function createWorkStore(db, clock = Date.now) {
  // What the owner chose to research first, then the newest.
  const listWork = (includeArchived = false, includeUnavailable = false) => db.all(`SELECT * FROM work ${includeArchived ? '' : 'WHERE archived_at IS NULL'} ORDER BY focused_at IS NULL, focused_at DESC, created_at DESC LIMIT 5000`)
    .map(row => job(row, clock())).filter(row => includeUnavailable || row.availability === 'available').slice(0, 200);
  // Each agent's newest open run, while its agent is still checking in.
  function activeRuns() {
    // A run outlives its session (a new session resumes it): the agent's
    // newest open session says whether it is still at work.
    return db.all(`SELECT runs.id,runs.work_id,runs.agent_id,runs.created_at,work.title,seen.heartbeat_at FROM runs
      JOIN work ON work.id=runs.work_id
      JOIN (SELECT agent_id,max(heartbeat_at) AS heartbeat_at FROM sessions WHERE closed_at IS NULL GROUP BY agent_id) AS seen ON seen.agent_id=runs.agent_id
      WHERE runs.finished_at IS NULL AND seen.heartbeat_at>=?
      ORDER BY runs.created_at DESC`, clock() - ACTIVE_MS)
      .filter((row, index, rows) => rows.findIndex(other => other.agent_id === row.agent_id) === index)
      .map(row => {
        const last = db.get("SELECT message,created_at FROM events WHERE run_id=? AND actor='agent' ORDER BY seq DESC LIMIT 1", row.id);
        return { agentId: row.agent_id, workId: row.work_id, runId: row.id, title: row.title, startedAt: row.created_at,
          lastSeenAt: row.heartbeat_at, lastMessage: last?.message ?? null };
      });
  }
  function session(agentId, id) {
    const value = db.get('SELECT * FROM sessions WHERE id=? AND agent_id=? AND closed_at IS NULL', id, agentId);
    if (!value || value.heartbeat_at < clock() - 12 * 3600000 || value.created_at < clock() - 7 * 86400000) throw fail('SESSION_EXPIRED', 'Open a new agent session.', 401);
    db.run('UPDATE sessions SET heartbeat_at=? WHERE id=?', clock(), id);
    return value;
  }
  function ownedRun(agentId, id) {
    const row = db.get('SELECT runs.*,work.title FROM runs JOIN work ON work.id=runs.work_id WHERE runs.id=? AND agent_id=?', id, agentId);
    if (!row) throw fail('RUN_NOT_FOUND', 'Research run not found.', 404);
    return row;
  }
  function addEvent(runId, actor, kind, message, reference = null, clientKey = null) {
    db.run('INSERT INTO events(run_id,actor,kind,message,reference_url,created_at,client_key) VALUES (?,?,?,?,?,?,?)',
      runId, actor, kind, message, reference, clock(), clientKey);
  }
  // A new run opens with the run's work and the owner's directive for it, if any.
  function started(runId, work) {
    addEvent(runId, 'petri', 'run_started', `Started: ${work.title}`);
    if (work.guidance?.instruction) addEvent(runId, 'owner', 'directive', work.guidance.instruction);
  }
  return {
    listWork,
    activeRuns,
    // The agent's next research run: the owner's picks first, then what
    // needs the most, skipping what another agent is on while others are
    // open. It resumes the agent's open run on the same work and closes its
    // other open runs, so one agent researches one thing at a time.
    next(agent, body) {
      return db.atomic(() => {
        const current = session(agent.id, body.sessionId);
        const others = new Set(activeRuns().filter(run => run.agentId !== agent.id).map(run => run.workId));
        const open = db.all('SELECT runs.*,work.title FROM runs JOIN work ON work.id=runs.work_id WHERE agent_id=? AND finished_at IS NULL ORDER BY runs.created_at DESC', agent.id);
        // An opening or update stakes the owner's USDC: one that earns nothing
        // (no rewards for it this month) comes up only when the owner picked it.
        const staked = item => item.objective.kind === 'update' || item.objective.kind === 'opening';
        // A month's claims come up once the workspace worked on that month,
        // until a run on them finishes (again when the owner picks them).
        const month = scope => scope && `${scope.marketId}:${scope.expiryId}`;
        const worked = new Set(db.all('SELECT DISTINCT work.scope_json FROM runs JOIN work ON work.id=runs.work_id')
          .map(row => month(row.scope_json ? JSON.parse(row.scope_json) : null)));
        const due = item => {
          if (item.objective.kind !== 'claims') return true;
          const done = db.get('SELECT max(finished_at) AS t FROM runs WHERE work_id=?', item.id).t;
          const picked = db.get('SELECT focused_at FROM work WHERE id=?', item.id).focused_at;
          return worked.has(month(item.candidateScope)) && (done == null || picked != null && picked > done);
        };
        const work = listWork().filter(item => item.objective && (!staked(item) || item.focused || need(item) > 0) && due(item));
        // Openings first after the owner's picks: the Opening lasts a day,
        // and a source with no opening value by listing sits out the month.
        const opening = item => item.objective.kind === 'opening' ? 1 : 0;
        const ranked = work.map((item, index) => ({ item, index }))
          .sort((a, b) => (b.item.focused - a.item.focused) || (opening(b.item) - opening(a.item)) || (need(b.item) - need(a.item)) || (a.index - b.index)).map(entry => entry.item);
        const free = ranked.filter(item => !others.has(item.id));
        const top = (free.length ? free : ranked)[0];
        const resume = open.find(run => run.work_id === top?.id) || (!top?.focused && open.find(run => ranked.some(item => item.id === run.work_id)));
        const chosen = resume ? ranked.find(item => item.id === resume.work_id) : top;
        for (const run of open) if (run.id !== resume?.id) {
          const summary = chosen ? `Moved on to ${chosen.title}.` : 'Nothing left to research.';
          db.run('UPDATE runs SET finished_at=?,summary=? WHERE id=?', clock(), summary, run.id);
          addEvent(run.id, 'petri', 'run_finished', summary);
        }
        if (!chosen) return { status: 'idle' };
        if (resume) return { status: 'assigned', resumed: true, run: runView(ownedRun(agent.id, resume.id)), assignment: chosen };
        const id = randomUUID();
        db.run('INSERT INTO runs(id,work_id,agent_id,session_id,client_key,created_at,finished_at,summary,context_json) VALUES (?,?,?,?,?,?,NULL,NULL,?)',
          id, chosen.id, agent.id, current.id, `next:${id}`, clock(), JSON.stringify(chosen));
        started(id, chosen);
        return { status: 'assigned', resumed: false, run: runView(ownedRun(agent.id, id)), assignment: chosen };
      });
    },
    // The owner's new directive for a work item, in the feed of every agent
    // on it as it is set (a run that starts later opens with it).
    directed(workId, instruction) {
      for (const run of db.all('SELECT id FROM runs WHERE work_id=? AND finished_at IS NULL', workId)) addEvent(run.id, 'owner', 'directive', instruction || 'Directive cleared.');
    },
    // The owner's "Research this" (or its undoing). Agents may still take any
    // objective; focused ones come first in their list.
    focusWork(id, focused) {
      if (typeof focused !== 'boolean') throw fail('INVALID_INPUT', 'Say whether to focus this objective.');
      if (!db.get('SELECT id FROM work WHERE id=? AND archived_at IS NULL', id)) throw fail('WORK_NOT_FOUND', 'Choose a discovered coverage gap.', 404);
      db.run('UPDATE work SET focused_at=? WHERE id=?', focused ? clock() : null, id);
      return focused;
    },
    archiveWork(id) {
      if (!db.get('SELECT id FROM work WHERE id=?', id)) throw fail('WORK_NOT_FOUND', 'Assignment not found.', 404);
      // The owner's archive stays, whatever discovery sees later.
      db.run('UPDATE work SET archived_at=coalesce(archived_at,?),retired_at=NULL WHERE id=?', clock(), id);
    },
    openSession(agent, harness) {
      const id = randomUUID(), now = clock(); harness = text(harness || agent.harness, 60, 'harness name');
      db.run('INSERT INTO sessions VALUES (?,?,?,?,?,NULL)', id, agent.id, harness, now, now);
      return sessionView(db.get('SELECT * FROM sessions WHERE id=?', id));
    },
    heartbeat(agent, id) { session(agent.id, id); return sessionView(db.get('SELECT * FROM sessions WHERE id=?', id)); },
    closeSession(agent, id) { session(agent.id, id); db.run('UPDATE sessions SET closed_at=? WHERE id=?', clock(), id); },
    startRun(agent, body) {
      const clientKey = text(body.clientKey, 80, 'request key');
      return db.atomic(() => {
        session(agent.id, body.sessionId);
        const previous = db.get('SELECT runs.*,work.title FROM runs JOIN work ON work.id=runs.work_id WHERE agent_id=? AND client_key=?', agent.id, clientKey);
        if (previous) {
          if (previous.work_id !== body.workId) throw fail('REQUEST_CONFLICT', 'This request key belongs to another assignment.', 409);
          return runView(previous);
        }
        const work = db.get('SELECT * FROM work WHERE id=? AND archived_at IS NULL', body.workId);
        if (!work || job(work, clock()).availability !== 'available') throw fail('WORK_NOT_FOUND', 'This research objective is unavailable or needs a fresh oracle assessment.', 404);
        if (db.get('SELECT count(*) AS n FROM runs WHERE agent_id=? AND finished_at IS NULL', agent.id).n >= 20) throw fail('RUN_LIMIT', 'Finish an existing research run before opening another.', 429);
        const id = randomUUID();
        db.run('INSERT INTO runs(id,work_id,agent_id,session_id,client_key,created_at,finished_at,summary,context_json) VALUES (?,?,?,?,?,?,NULL,NULL,?)',
          id, work.id, agent.id, body.sessionId, clientKey, clock(), JSON.stringify(job(work, clock())));
        started(id, job(work, clock()));
        return { ...runView(ownedRun(agent.id, id)), assignment: job(work, clock()) };
      });
    },
    report(agent, runId, body) {
      return db.atomic(() => {
        session(agent.id, body.sessionId); const run = ownedRun(agent.id, runId);
        const message = text(body.message, 4000, 'progress message'), clientKey = text(body.clientKey, 80, 'request key');
        if (!REPORT_KINDS.includes(body.kind)) throw fail('INVALID_EVENT', `Report one of: ${REPORT_KINDS.join(', ')}.`);
        // A page may be public or behind the owner's sign-in (read in the
        // signed-in browser): either way an HTTPS URL that carries no
        // credentials, so no sign-in reaches the feed.
        let reference = body.referenceUrl || null;
        if (body.kind === 'read' && !reference) throw fail('INVALID_REFERENCE', 'Give the page you read as an HTTPS reference.');
        if (reference) { reference = text(reference, 1024, 'source URL'); let url; try { url = new URL(reference); } catch { throw fail('INVALID_REFERENCE', 'Use an HTTPS reference.'); }
          if (url.protocol !== 'https:' || url.username || url.password) throw fail('INVALID_REFERENCE', 'Use an HTTPS reference without credentials.');
          if (secretInUrl(reference)) throw fail('INVALID_REFERENCE', 'Give the page URL without any token, key or session id in it.'); }
        const previous = db.get('SELECT * FROM events WHERE run_id=? AND client_key=?', runId, clientKey);
        if (previous) {
          if (previous.kind !== body.kind || previous.message !== message || previous.reference_url !== reference) throw fail('REQUEST_CONFLICT', 'This event key has different content.', 409);
          return eventView(previous);
        }
        if (run.finished_at !== null) throw fail('RUN_FINISHED', 'This research run is finished.', 409);
        if (db.get('SELECT count(*) AS n FROM events WHERE run_id=?', runId).n >= 2000) throw fail('EVENT_LIMIT', 'This run has reached its event limit.', 429);
        addEvent(runId, 'agent', body.kind, message, reference, clientKey);
        return eventView(db.get('SELECT * FROM events WHERE run_id=? AND client_key=?', runId, clientKey));
      });
    },
    finishRun(agent, runId, body) {
      return db.atomic(() => {
        session(agent.id, body.sessionId); const run = ownedRun(agent.id, runId), summary = text(body.summary, 4000, 'completion summary');
        if (run.finished_at !== null) {
          if (run.summary !== summary) throw fail('REQUEST_CONFLICT', 'This run already has a different completion summary.', 409);
          return runView(run);
        }
        db.run('UPDATE runs SET finished_at=?,summary=? WHERE id=?', clock(), summary, runId);
        addEvent(runId, 'agent', 'run_finished', summary);
        return runView(ownedRun(agent.id, runId));
      });
    },
    // `bundleSha`: the SHA-256 of the web proof bundle that comes with the
    // candidate, if any. Its bytes fix the bundle's digest, so a retry with
    // the same key must bring the same bundle.
    beginSubmission(agent, runId, body, bundleSha = null) {
      return db.atomic(() => {
        session(agent.id, body.sessionId); const run = ownedRun(agent.id, runId);
        const scope = run.context_json ? JSON.parse(run.context_json).candidateScope : JSON.parse(db.get('SELECT scope_json FROM work WHERE id=?', run.work_id).scope_json || 'null');
        if (scope && Object.entries(scope).some(([key, value]) => body.candidate?.[key] !== value)) {
          throw fail('ASSIGNMENT_MISMATCH', 'The candidate does not match this assignment’s requested scope.');
        }
        const clientKey = text(body.clientKey, 80, 'request key');
        const inputHash = digest(JSON.stringify(bundleSha ? { candidate: body.candidate, bundleSha } : body.candidate));
        const previous = db.get('SELECT * FROM submissions WHERE run_id=? AND client_key=?', runId, clientKey);
        if (previous) {
          if (previous.input_hash !== inputHash) throw fail('REQUEST_CONFLICT', 'This submission key has different content.', 409);
          if (previous.status === 'complete') return { result: JSON.parse(previous.result_json) };
          if (previous.status === 'pending') throw fail('SUBMISSION_PENDING', 'This submission is still being checked.', 409);
          db.run("UPDATE submissions SET status='pending' WHERE id=?", previous.id);
          return { id: previous.id };
        }
        if (run.finished_at !== null) throw fail('RUN_FINISHED', 'This research run is finished.', 409);
        if (db.get('SELECT count(*) AS n FROM submissions WHERE run_id=?', runId).n >= 100) throw fail('SUBMISSION_LIMIT', 'This run has reached its submission limit.', 429);
        const result = db.run('INSERT INTO submissions(run_id,client_key,input_hash,status,created_at) VALUES (?,?,?,\'pending\',?)', runId, clientKey, inputHash, clock());
        return { id: Number(result.lastInsertRowid) };
      });
    },
    // A candidate with a web proof keeps the bundle's digest (the API's
    // address for it, GET /oracle/evidence/:digest), in its row and its result.
    completeSubmission(id, result) {
      return db.atomic(() => {
        const row = db.get('SELECT * FROM submissions WHERE id=?', id), bundle = result.evidence?.bundleDigest ?? null;
        db.run("UPDATE submissions SET status='complete',result_json=?,bundle_digest=? WHERE id=?", JSON.stringify(result), bundle, id);
        const proof = bundle ? ` Web proof: bundle ${bundle}.` : '';
        addEvent(row.run_id, 'backend', result.duplicate ? 'candidate_duplicate' : 'candidate_received',
          (result.duplicate ? 'This source already exists; original attribution is preserved.' : 'Source candidate received and attributed. Oracle acceptance is pending.') + proof, result.candidate?.details?.canonicalUrl || null);
        return result;
      });
    },
    failSubmission(id) { db.run("UPDATE submissions SET status='retryable' WHERE id=? AND status='pending'", id); },
    // The agent's page in the TUI: its runs and its newest events, oldest
    // first (its own reports, petri's run events, the backend's and the owner's
    // directives). 300 events hold the current run's feed and several before it.
    snapshot(agentId) {
      return {
        sessions: db.all('SELECT * FROM sessions WHERE agent_id=? ORDER BY created_at DESC LIMIT 20', agentId).map(sessionView),
        runs: db.all('SELECT runs.*,work.title FROM runs JOIN work ON work.id=runs.work_id WHERE agent_id=? ORDER BY created_at DESC LIMIT 100', agentId).map(runView),
        events: db.all('SELECT events.* FROM events JOIN runs ON runs.id=events.run_id WHERE runs.agent_id=? ORDER BY seq DESC LIMIT 300', agentId).reverse().map(eventView),
        submissions: db.all('SELECT submissions.run_id,submissions.result_json FROM submissions JOIN runs ON runs.id=submissions.run_id WHERE agent_id=? AND result_json IS NOT NULL ORDER BY submissions.id DESC LIMIT 100', agentId)
          .map(row => ({ runId: row.run_id, ...JSON.parse(row.result_json) })),
      };
    },
    ownedRun,
  };
}
