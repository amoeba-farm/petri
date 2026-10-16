import { randomUUID } from 'node:crypto';
import { fail, text, digest } from './security.mjs';

const job = (row, now) => row && ({ id: row.id, title: row.title, brief: row.brief, createdAt: row.created_at,
  archivedAt: row.archived_at, candidateScope: row.scope_json ? JSON.parse(row.scope_json) : null,
  objective: row.objective_json ? JSON.parse(row.objective_json) : null,
  guidance: row.guidance_json ? JSON.parse(row.guidance_json) : null,
  availability: row.discovery_key && row.availability === 'available' && row.valid_until <= now ? 'unknown' : row.availability || 'available' });
const runView = row => ({ id: row.id, workId: row.work_id, title: row.title, agentId: row.agent_id,
  sessionId: row.session_id, createdAt: row.created_at, finishedAt: row.finished_at, summary: row.summary,
  researchContext: row.context_json ? JSON.parse(row.context_json) : null });
const sessionView = row => ({ id: row.id, agentId: row.agent_id, harness: row.harness, createdAt: row.created_at,
  heartbeatAt: row.heartbeat_at, closedAt: row.closed_at });
const eventView = row => ({ sequence: row.seq, runId: row.run_id, actor: row.actor, kind: row.kind,
  message: row.message, referenceUrl: row.reference_url, createdAt: row.created_at });

export function createWorkStore(db, clock = Date.now) {
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
  return {
    listWork: (includeArchived = false, includeUnavailable = false) => db.all(`SELECT * FROM work ${includeArchived ? '' : 'WHERE archived_at IS NULL'} ORDER BY created_at DESC LIMIT 5000`)
      .map(row => job(row, clock())).filter(row => includeUnavailable || row.availability === 'available').slice(0, 200),
    archiveWork(id) {
      if (!db.get('SELECT id FROM work WHERE id=?', id)) throw fail('WORK_NOT_FOUND', 'Assignment not found.', 404);
      db.run('UPDATE work SET archived_at=coalesce(archived_at,?) WHERE id=?', clock(), id);
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
        addEvent(id, 'petri', 'run_started', `Started: ${work.title}`);
        return { ...runView(ownedRun(agent.id, id)), assignment: job(work, clock()) };
      });
    },
    report(agent, runId, body) {
      return db.atomic(() => {
        session(agent.id, body.sessionId); const run = ownedRun(agent.id, runId);
        const message = text(body.message, 4000, 'progress message'), clientKey = text(body.clientKey, 80, 'request key');
        if (!['note', 'source_found', 'blocked'].includes(body.kind)) throw fail('INVALID_EVENT', 'Report a note, source_found, or blocked event.');
        let reference = body.referenceUrl || null;
        if (reference) { reference = text(reference, 1024, 'source URL'); let url; try { url = new URL(reference); } catch { throw fail('INVALID_REFERENCE', 'Use a public HTTPS reference.'); }
          if (url.protocol !== 'https:' || url.username || url.password) throw fail('INVALID_REFERENCE', 'Use an HTTPS reference without credentials.'); }
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
    beginSubmission(agent, runId, body) {
      return db.atomic(() => {
        session(agent.id, body.sessionId); const run = ownedRun(agent.id, runId);
        const scope = run.context_json ? JSON.parse(run.context_json).candidateScope : JSON.parse(db.get('SELECT scope_json FROM work WHERE id=?', run.work_id).scope_json || 'null');
        if (scope && Object.entries(scope).some(([key, value]) => body.candidate?.[key] !== value)) {
          throw fail('ASSIGNMENT_MISMATCH', 'The candidate does not match this assignment’s requested scope.');
        }
        const clientKey = text(body.clientKey, 80, 'request key'), inputHash = digest(JSON.stringify(body.candidate));
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
    completeSubmission(id, result) {
      return db.atomic(() => {
        const row = db.get('SELECT * FROM submissions WHERE id=?', id);
        db.run("UPDATE submissions SET status='complete',result_json=? WHERE id=?", JSON.stringify(result), id);
        addEvent(row.run_id, 'backend', result.duplicate ? 'candidate_duplicate' : 'candidate_received',
          result.duplicate ? 'This source already exists; original attribution is preserved.' : 'Source candidate received and attributed. Oracle acceptance is pending.', result.candidate?.details?.canonicalUrl || null);
        return result;
      });
    },
    failSubmission(id) { db.run("UPDATE submissions SET status='retryable' WHERE id=? AND status='pending'", id); },
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
