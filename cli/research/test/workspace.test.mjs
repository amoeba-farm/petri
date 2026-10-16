import test from 'node:test';
import assert from 'node:assert/strict';
import { createApplication } from '../service/application.mjs';
import { token, vault, digest } from '../service/security.mjs';
import { coverageFixture, coverageNow } from './coverage-fixture.mjs';

const payout = 'So11111111111111111111111111111111111111112';
const candidate = { marketId: 'ramx', expiryId: 'RAMX-202610', bucketId: '21'.repeat(32),
  canonicalUrl: 'https://example.com/ram', sourceType: 'retail product listing', productDefinition: '32GB DDR5-5600 kit',
  priceField: 'USD unit price', region: 'United States', currency: 'USD', evidence: 'Public offer and matching product specifications.' };

function fixture() {
  let now = coverageNow, registrations = 0, submissions = 0;
  const upstreamAgents = new Map(), candidates = new Map(), ownerToken = token();
  const upstream = {
    async register(payoutAuthority) { registrations++; const agent = { agentId: token(), payoutAuthority }, credential = token(); upstreamAgents.set(credential, agent); return { agent, credential }; },
    async submit(details, credential) {
      submissions++; const agent = upstreamAgents.get(credential); assert(agent);
      const existing = [...candidates.values()].find(c => c.details.canonicalUrl === details.canonicalUrl);
      if (existing) return { duplicate: true, candidate: existing };
      const value = { contentHash: digest(JSON.stringify(details)), agentId: agent.agentId, details, observation: null };
      candidates.set(value.contentHash, value); return { duplicate: false, candidate: value };
    },
    async candidates(agentId) { return { candidates: [...candidates.values()].filter(c => c.agentId === agentId), complete: true }; },
  };
  const application = createApplication({ ownerToken, masterKey: token(), directory: '.', upstream: 'http://127.0.0.1:4500' },
    { databasePath: ':memory:', upstream, clock: () => now, oracleReader: async () => coverageFixture(now) });
  const request = (method, path, body = {}, credential) => application.handle({ method, path, body, credential });
  const owner = (method, path, body = {}) => request(method, `/v1/owner/${path}`, body, ownerToken);
  async function connect(name) {
    await owner('POST', 'settings', { payoutAuthority: payout });
    const pairing = await request('POST', '/v1/login', { name, harness: 'Test harness' });
    await owner('POST', 'pairings/approve', { pairingId: pairing.pairingId });
    const result = await request('POST', '/v1/login/poll', { pairingId: pairing.pairingId, secret: pairing.secret });
    const call = (method, path, body = {}) => request(method, path, body, result.credential);
    const session = (await call('POST', '/v1/sessions', {})).session;
    return { ...result, call, session, pairing };
  }
  async function discover() { await owner('POST', 'discovery/refresh'); return (await owner('GET', 'snapshot')).work[0]; }
  return { application, request, owner, connect, discover, upstream, candidates, time: value => now += value,
    counts: () => ({ registrations, submissions }), close: () => application.close() };
}

test('pairing requires the owner and the unguessable polling secret; public views exclude credentials', async () => {
  const f = fixture(); try {
    const pending = await f.request('POST', '/v1/login', { name: 'cedar', harness: 'Codex' });
    await assert.rejects(f.request('POST', '/v1/owner/pairings/approve', { pairingId: pending.pairingId }), { code: 'OWNER_REQUIRED' });
    await assert.rejects(f.request('POST', '/v1/login/poll', { pairingId: pending.pairingId, secret: token() }), { code: 'UNAUTHORIZED' });
    await assert.rejects(f.owner('POST', 'pairings/approve', { pairingId: pending.pairingId }), { code: 'PAYOUT_REQUIRED' });
    await f.owner('POST', 'settings', { payoutAuthority: payout });
    await f.owner('POST', 'pairings/approve', { pairingId: pending.pairingId });
    const connected = await f.request('POST', '/v1/login/poll', { pairingId: pending.pairingId, secret: pending.secret });
    const snapshot = JSON.stringify(await f.owner('GET', 'snapshot'));
    assert(!snapshot.includes(pending.secret)); assert(!snapshot.includes(connected.credential));
    assert.equal(connected.agent.payoutAuthority, payout);
    assert.equal((await f.request('POST', '/v1/login/poll', { pairingId: pending.pairingId, secret: pending.secret })).credential, connected.credential);
  } finally { f.close(); }
});

test('expired requests and revoked credentials cannot reconnect or report work', async () => {
  const f = fixture(); try {
    const a = await f.connect('cedar');
    await f.owner('POST', 'agents/revoke', { agentId: a.agent.id });
    await assert.rejects(a.call('GET', '/v1/me'), { code: 'UNAUTHORIZED' });
    assert.equal((await f.request('POST', '/v1/login/poll', { pairingId: a.pairing.pairingId, secret: a.pairing.secret })).status, 'revoked');
    const pending = await f.request('POST', '/v1/login', { name: 'other', harness: 'Kimi Code' });
    f.time(600001);
    await assert.rejects(f.owner('POST', 'pairings/approve', { pairingId: pending.pairingId }), { code: 'PAIRING_EXPIRED' });
    await assert.rejects(f.request('POST', '/v1/login/poll', { pairingId: pending.pairingId, secret: pending.secret }), { code: 'PAIRING_EXPIRED' });
  } finally { f.close(); }
});

test('research runs reject cross-agent access and forged platform events', async () => {
  const f = fixture(); try {
    const a = await f.connect('cedar'), b = await f.connect('flint');
    const work = await f.discover();
    const input = { workId: work.id, sessionId: a.session.id, clientKey: 'run-1' };
    const run = (await a.call('POST', '/v1/runs', input)).run;
    assert.equal((await a.call('POST', '/v1/runs', input)).run.id, run.id);
    const body = { sessionId: b.session.id, clientKey: 'event-1', kind: 'note', message: 'Working' };
    await assert.rejects(b.call('POST', `/v1/runs/${run.id}/events`, body), { code: 'RUN_NOT_FOUND' });
    await assert.rejects(a.call('POST', `/v1/runs/${run.id}/events`, { ...body, sessionId: a.session.id, kind: 'reward_settled' }), { code: 'INVALID_EVENT' });
    await assert.rejects(a.call('POST', `/v1/runs/${run.id}/events`, { ...body, sessionId: a.session.id, actor: 'backend' }), { code: 'INVALID_INPUT' });
    const good = { ...body, sessionId: a.session.id };
    const event = (await a.call('POST', `/v1/runs/${run.id}/events`, good)).event;
    assert.equal(event.actor, 'agent');
    assert.equal((await a.call('POST', `/v1/runs/${run.id}/events`, good)).event.sequence, event.sequence);
    await assert.rejects(a.call('POST', `/v1/runs/${run.id}/events`, { ...good, message: 'Changed' }), { code: 'REQUEST_CONFLICT' });
    await assert.rejects(a.call('POST', '/v1/owner/work', { title: 'No', brief: 'No' }), { code: 'OWNER_REQUIRED' });
  } finally { f.close(); }
});

test('candidate submission is idempotent, bound to its assignment, and preserves duplicate attribution', async () => {
  const f = fixture(); try {
    const a = await f.connect('cedar'), b = await f.connect('flint');
    const work = await f.discover();
    const ar = (await a.call('POST', '/v1/runs', { workId: work.id, sessionId: a.session.id, clientKey: 'start' })).run;
    const br = (await b.call('POST', '/v1/runs', { workId: work.id, sessionId: b.session.id, clientKey: 'start' })).run;
    const input = { sessionId: a.session.id, clientKey: 'source', candidate };
    await assert.rejects(a.call('POST', `/v1/runs/${ar.id}/candidates`, { ...input, candidate: { ...candidate, marketId: 'nandx' } }), { code: 'ASSIGNMENT_MISMATCH' });
    const first = await a.call('POST', `/v1/runs/${ar.id}/candidates`, input);
    const replay = await a.call('POST', `/v1/runs/${ar.id}/candidates`, input);
    assert.deepEqual(replay, first); assert.equal(f.counts().submissions, 1);
    const duplicate = await b.call('POST', `/v1/runs/${br.id}/candidates`, { ...input, sessionId: b.session.id });
    assert.equal(duplicate.duplicate, true); assert.equal(duplicate.candidate.agentId, a.agent.id);
    assert.deepEqual((await b.call('GET', '/v1/rewards')).outcomes, []);
    assert.equal((await a.call('GET', '/v1/rewards')).outcomes[0].observation, null);
    await a.call('POST', `/v1/runs/${ar.id}/finish`, { sessionId: a.session.id, summary: 'Evidence submitted.' });
    assert.equal((await a.call('GET', '/v1/rewards')).outcomes[0].observation, null);
  } finally { f.close(); }
});

test('a new session resumes the same identity and run without creating rewards', async () => {
  const f = fixture(); try {
    const a = await f.connect('cedar');
    const w = await f.discover();
    const run = (await a.call('POST', '/v1/runs', { workId: w.id, sessionId: a.session.id, clientKey: 'run' })).run;
    await a.call('POST', '/v1/sessions/close', { sessionId: a.session.id });
    await assert.rejects(a.call('POST', `/v1/runs/${run.id}/events`, { sessionId: a.session.id, clientKey: 'e', kind: 'note', message: 'Resumed.' }), { code: 'SESSION_EXPIRED' });
    const next = (await a.call('POST', '/v1/sessions', { harness: 'Kimi Code' })).session;
    await a.call('POST', `/v1/runs/${run.id}/events`, { sessionId: next.id, clientKey: 'e', kind: 'note', message: 'Resumed.' });
    assert.equal((await a.call('GET', '/v1/activity')).runs[0].id, run.id);
    assert.equal((await a.call('GET', '/v1/me')).agent.id, a.agent.id);
  } finally { f.close(); }
});

test('private vault ciphertext is bound to its purpose', () => {
  const v = vault(token()), secret = token(), sealed = v.seal(secret, 'upstream:a');
  assert(!sealed.includes(secret)); assert.equal(v.open(sealed, 'upstream:a'), secret);
  assert.throws(() => v.open(sealed, 'upstream:b'));
});

test('a lost login response can be recovered without creating another pairing', async () => {
  const f = fixture(); try {
    const request = { name: 'cedar', harness: 'Codex', loginKey: token() };
    const first = await f.request('POST', '/v1/login', request);
    const retry = await f.request('POST', '/v1/login', request);
    assert.deepEqual(retry, first);
    assert.equal((await f.owner('GET', 'snapshot')).pairings.length, 1);
    await assert.rejects(f.request('POST', '/v1/login', { ...request, name: 'flint' }), { code: 'REQUEST_CONFLICT' });
  } finally { f.close(); }
});
