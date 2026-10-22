import { join } from 'node:path';
import { runtimeId } from './lifecycle.mjs';
import { createDatabase } from './database.mjs';
import { createAuthStore } from './auth-store.mjs';
import { createWorkStore } from './work-store.mjs';
import { createUpstream } from './upstream.mjs';
import { createDirectiveStore } from './directive-store.mjs';
import { createDiscovery, createOracleReader } from './discovery.mjs';
import { fail, text, object, equalSecret, vault, digest } from './security.mjs';
import { decodeBundle } from './evidence.mjs';

export function createApplication(config, options = {}) {
  const db = createDatabase(options.databasePath || join(config.directory, 'workspace.sqlite'));
  const clock = options.clock || Date.now;
  const auth = createAuthStore(db, vault(config.masterKey), clock), work = createWorkStore(db, clock);
  const upstream = options.upstream || createUpstream(config.upstream);
  const directives = createDirectiveStore(db, clock);
  if (options.seedDirectives !== false) directives.seed();
  const discovery = createDiscovery(directives, options.oracleReader || (config.oracleReadUrl ? createOracleReader(config.oracleReadUrl) : null), clock);
  const candidateReads = new Map();
  function owner(credential) {
    if (!equalSecret(config.ownerToken, credential)) throw fail('OWNER_REQUIRED', 'This action requires the workspace owner.', 403);
  }
  async function readCandidates(agentId, force = false) {
    let entry = candidateReads.get(agentId);
    if (!force && entry && clock() - entry.time < 5000) return entry.result;
    if (entry?.pending) return entry.pending;
    entry = { ...(entry || {}), pending: upstream.candidates(agentId).then(result => {
      candidateReads.set(agentId, { result, time: clock() }); return result;
    }).catch(error => { candidateReads.delete(agentId); throw error; }) };
    candidateReads.set(agentId, entry); return entry.pending;
  }
  async function detail(agentId) {
    const agent = auth.get(agentId);
    if (!agent) throw fail('AGENT_NOT_FOUND', 'Agent not found.', 404);
    const data = { agent, ...work.snapshot(agentId) };
    try { return { ...data, ...(await readCandidates(agentId)), validationError: null }; }
    catch (error) { return { ...data, candidates: null, complete: false, validationError: error.message }; }
  }
  async function approve(pairingId) {
    const payout = auth.setting('payoutAuthority');
    if (!payout) throw fail('PAYOUT_REQUIRED', 'Connect a wallet before connecting an agent.');
    auth.beginApproval(pairingId);
    try {
      const registered = await upstream.register(payout);
      return { agent: auth.completeApproval(pairingId, registered, payout) };
    } catch (error) {
      auth.failApproval(pairingId, error.status ? error.message : 'Approval could not complete. Retry from Connect.');
      throw error;
    }
  }
  return {
    async handle({ method, path, query = new URLSearchParams(), credential, body = {} }) {
      if (method === 'GET' && path === '/health') return { name: 'petri-research', version: '0.1.0' };
      if (method === 'POST' && path === '/v1/login') {
        object(body, ['name', 'harness', 'loginKey'], ['name','harness']); return auth.start(body.name, body.harness, body.loginKey);
      }
      if (method === 'POST' && path === '/v1/login/poll') {
        object(body, ['pairingId', 'secret']);
        return auth.poll(text(body.pairingId, 32, 'pairing ID'), text(body.secret, 64, 'pairing secret'));
      }
      if (path.startsWith('/v1/owner/')) {
        owner(credential);
        if (method === 'GET' && path === '/v1/owner/ping') return { name: 'petri-research', runtime: options.runtimeId || runtimeId() };
        if (method === 'POST' && path === '/v1/owner/restart') {
          object(body, []);
          if (!options.restart) throw fail('RESTART_UNAVAILABLE', 'This workspace must be restarted manually.', 409);
          options.restart(); return { restarting: true };
        }
        if (method === 'GET' && path === '/v1/owner/snapshot') {
          const agents = auth.list(), selected = query.get('agentId') || agents[0]?.id;
          // Who is researching what right now (the TUI's "researching").
          const activity = work.activeRuns().filter(run => agents.some(agent => agent.id === run.agentId && !agent.revokedAt));
          const items = work.listWork(false, true).map(item => ({ ...item, researchers: activity.filter(run => run.workId === item.id).map(run => run.agentId) }));
          return { agents, pairings: auth.pending(), work: items, activity, directives: directives.list(), discovery: directives.status(), settings: { payoutAuthority: auth.setting('payoutAuthority') },
            selected: selected ? await detail(selected) : null };
        }
        if (method === 'POST' && path === '/v1/owner/settings') {
          object(body, ['payoutAuthority']);
          const payout = text(body.payoutAuthority, 44, 'wallet address');
          if (!/^[1-9A-HJ-NP-Za-km-z]{32,44}$/u.test(payout)) throw fail('INVALID_PAYOUT', 'Provide a Solana public address.');
          auth.setSetting('payoutAuthority', payout); return { payoutAuthority: payout };
        }
        if (method === 'POST' && path === '/v1/owner/pairings/approve') {
          object(body, ['pairingId']); return approve(text(body.pairingId, 32, 'pairing ID'));
        }
        if (method === 'POST' && path === '/v1/owner/pairings/reject') {
          object(body, ['pairingId']); auth.reject(text(body.pairingId, 32, 'pairing ID')); return { rejected: true };
        }
        if (method === 'POST' && path === '/v1/owner/agents/revoke') {
          object(body, ['agentId']); auth.revoke(text(body.agentId, 64, 'agent ID')); return { revoked: true };
        }
        if (method === 'POST' && path === '/v1/owner/work/archive') {
          object(body, ['workId']); work.archiveWork(text(body.workId, 36, 'assignment ID')); return { archived: true };
        }
        if (method === 'POST' && path === '/v1/owner/directives') {
          const directive = directives.save(body); void discovery.refresh(); return { directive };
        }
        if (method === 'POST' && path === '/v1/owner/work/focus') {
          object(body, ['workId','focused']); return { focused: work.focusWork(text(body.workId, 36, 'work ID'), body.focused) };
        }
        if (method === 'POST' && path === '/v1/owner/work/refine') {
          object(body, ['workId','instruction']);
          return { guidance: db.atomic(() => { const guidance = directives.refine(body.workId, body.instruction); work.directed(body.workId, guidance.instruction); return guidance; }) };
        }
        if (method === 'POST' && path === '/v1/owner/discovery/refresh') {
          object(body, []); return { discovery: await discovery.refresh() };
        }
        throw fail('NOT_FOUND', 'Route not found.', 404);
      }
      const agent = auth.authenticate(credential);
      if (method === 'GET' && path === '/v1/me') return { agent };
      if (method === 'GET' && path === '/v1/work') return { work: work.listWork(), directives: directives.list(), discovery: directives.status() };
      if (method === 'POST' && path === '/v1/work/next') {
        // How the agent runs petri here (`petri-live petri` on a dev PC).
        object(body, ['sessionId']); return { ...work.next(agent, { sessionId: text(body.sessionId, 36, 'session ID') }), petri: options.petriCommand ?? process.env.PETRI_COMMAND ?? null };
      }
      if (method === 'GET' && path === '/v1/activity') return detail(agent.id);
      if (method === 'GET' && path === '/v1/rewards') {
        const result = await readCandidates(agent.id);
        return { agentId: agent.id, complete: result.complete, outcomes: result.candidates.map(candidate => ({
          contentHash: candidate.contentHash, details: candidate.details, observation: candidate.observation ?? null })) };
      }
      if (method === 'POST' && path === '/v1/sessions') {
        object(body, ['harness'], []); return { session: work.openSession(agent, body.harness) };
      }
      if (method === 'POST' && path === '/v1/sessions/heartbeat') {
        object(body, ['sessionId']); return { session: work.heartbeat(agent, text(body.sessionId, 36, 'session ID')) };
      }
      if (method === 'POST' && path === '/v1/sessions/close') {
        object(body, ['sessionId']); work.closeSession(agent, text(body.sessionId, 36, 'session ID')); return { closed: true };
      }
      if (method === 'POST' && path === '/v1/runs') {
        object(body, ['sessionId', 'workId', 'clientKey']); return { run: work.startRun(agent, body) };
      }
      const match = /^\/v1\/runs\/([0-9a-f-]{36})\/(events|finish|candidates)$/u.exec(path);
      if (method === 'POST' && match) {
        if (match[2] === 'events') {
          object(body, ['sessionId', 'clientKey', 'kind', 'message', 'referenceUrl'], ['sessionId', 'clientKey', 'kind', 'message']);
          return { event: work.report(agent, match[1], body) };
        }
        if (match[2] === 'finish') {
          object(body, ['sessionId', 'summary']); return { run: work.finishRun(agent, match[1], body) };
        }
        object(body, ['sessionId', 'clientKey', 'candidate', 'evidenceBundle'], ['sessionId', 'clientKey', 'candidate']);
        if (!body.candidate || typeof body.candidate !== 'object' || Array.isArray(body.candidate)) throw fail('INVALID_CANDIDATE', 'Provide the candidate fields.');
        // A page behind the owner's sign-in comes with its web proof bundle.
        // Only its digests are kept here: the bundle itself goes to the API.
        const bundle = body.evidenceBundle === undefined ? null : decodeBundle(body.evidenceBundle);
        const attempt = work.beginSubmission(agent, match[1], body, bundle && digest(bundle));
        if (attempt.result) return attempt.result;
        try {
          const credential = auth.upstreamCredential(agent.id);
          let evidence = null;
          if (bundle) {
            // The API verifies the bundle itself; the candidate then names it,
            // and the API checks the proof is for its canonicalUrl.
            const uploaded = await upstream.uploadEvidence(bundle, credential);
            if (!/^[0-9a-f]{64}$/u.test(uploaded?.bundleDigest)) throw fail('UPSTREAM_INVALID', 'Invalid evidence response.', 502);
            evidence = { bundleDigest: uploaded.bundleDigest, trust: typeof uploaded.trust === 'string' && /^[a-z_]{1,32}$/u.test(uploaded.trust) ? uploaded.trust : null };
          }
          const result = await upstream.submit(evidence ? { ...body.candidate, authentication: { bundleDigest: evidence.bundleDigest } } : body.candidate, credential);
          if (!/^[0-9a-f]{64}$/u.test(result.candidate?.contentHash) || typeof result.duplicate !== 'boolean' ||
              !result.duplicate && result.candidate.agentId !== agent.id) throw fail('UPSTREAM_INVALID', 'Invalid candidate response or attribution.', 502);
          const saved = work.completeSubmission(attempt.id, evidence ? { ...result, evidence } : result); candidateReads.delete(agent.id); return saved;
        } catch (error) { work.failSubmission(attempt.id); throw error; }
      }
      throw fail('NOT_FOUND', 'Route not found.', 404);
    },
    startDiscovery: () => discovery.start(),
    close: () => { discovery.close(); db.close(); },
  };
}
