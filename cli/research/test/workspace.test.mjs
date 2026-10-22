import test from 'node:test';
import assert from 'node:assert/strict';
import { createApplication } from '../service/application.mjs';
import { token, vault, digest } from '../service/security.mjs';
import { MAX_BUNDLE_BYTES, encodeBundle } from '../service/evidence.mjs';
import { DatabaseSync } from 'node:sqlite';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve, dirname } from 'node:path';
import { coverageFixture, coverageNow } from './coverage-fixture.mjs';

const payout = 'So11111111111111111111111111111111111111112';
const candidate = { marketId: 'ramx', expiryId: 'RAMX-202610', bucketId: '21'.repeat(32),
  canonicalUrl: 'https://example.com/ram', sourceType: 'retail product listing', productDefinition: '32GB DDR5-5600 kit',
  priceField: 'USD unit price', region: 'United States', currency: 'USD', evidence: 'Public offer and matching product specifications.' };

function fixture({ databasePath = ':memory:' } = {}) {
  let now = coverageNow, registrations = 0, submissions = 0;
  const upstreamAgents = new Map(), candidates = new Map(), ownerToken = token(), uploads = [], submitted = [];
  const upstream = {
    async register(payoutAuthority) { registrations++; const agent = { agentId: token(), payoutAuthority }, credential = token(); upstreamAgents.set(credential, agent); return { agent, credential }; },
    // POST /oracle/evidence: the API verifies the bundle and names it by digest.
    async uploadEvidence(bundle, credential) {
      assert(upstreamAgents.get(credential)); uploads.push(Buffer.from(bundle));
      if (upstream.rejectEvidence) throw Object.assign(new Error('The API could not verify this web proof.'), { code: 'EVIDENCE_REJECTED', status: 422 });
      return { bundleDigest: digest(bundle), proofDigest: token(), observationId: token(), trust: 'development', stored: true, duplicate: false };
    },
    async submit(details, credential) {
      submissions++; submitted.push(details); const agent = upstreamAgents.get(credential); assert(agent);
      const existing = [...candidates.values()].find(c => c.details.canonicalUrl === details.canonicalUrl);
      if (existing) return { duplicate: true, candidate: existing };
      const value = { contentHash: digest(JSON.stringify(details)), agentId: agent.agentId, details, observation: null };
      candidates.set(value.contentHash, value); return { duplicate: false, candidate: value };
    },
    async candidates(agentId) { return { candidates: [...candidates.values()].filter(c => c.agentId === agentId), complete: true }; },
  };
  const application = createApplication({ ownerToken, masterKey: token(), directory: '.', upstream: 'http://127.0.0.1:4500' },
    { databasePath, upstream, clock: () => now, oracleReader: async () => coverageFixture(now) });
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
  return { application, ownerToken, request, owner, connect, discover, upstream, candidates, uploads, submitted, time: value => now += value,
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

test('work next starts the owner’s pick, resumes it, and the owner sees who is researching what', async () => {
  const f = fixture(); try {
    const a = await f.connect('cedar');
    const work = await f.discover();
    await f.owner('POST', 'work/focus', { workId: work.id, focused: true });
    const first = await a.call('POST', '/v1/work/next', { sessionId: a.session.id });
    assert.equal(first.status, 'assigned'); assert.equal(first.assignment.id, work.id); assert.equal(first.resumed, false);
    const again = await a.call('POST', '/v1/work/next', { sessionId: a.session.id });
    assert.equal(again.run.id, first.run.id); assert.equal(again.resumed, true);
    let snapshot = await f.owner('GET', 'snapshot');
    assert.deepEqual(snapshot.activity.map(run => [run.agentId, run.workId]), [[a.agent.id, work.id]]);
    assert.deepEqual(snapshot.work.find(item => item.id === work.id).researchers, [a.agent.id]);
    await a.call('POST', `/v1/runs/${first.run.id}/events`, { sessionId: a.session.id, clientKey: 'e', kind: 'note', message: 'Checked a listing.' });
    assert.equal((await f.owner('GET', 'snapshot')).activity[0].lastMessage, 'Checked a listing.');
    // A finished run is not research; the next one starts afresh.
    await a.call('POST', `/v1/runs/${first.run.id}/finish`, { sessionId: a.session.id, summary: 'Done.' });
    assert.deepEqual((await f.owner('GET', 'snapshot')).activity, []);
    const next = await a.call('POST', '/v1/work/next', { sessionId: a.session.id });
    assert.notEqual(next.run.id, first.run.id);
    // An agent that stopped checking in is not researching.
    f.time(20 * 60000 + 1);
    snapshot = await f.owner('GET', 'snapshot');
    assert.deepEqual(snapshot.activity, []);
    assert.deepEqual(snapshot.work.find(item => item.id === work.id).researchers, []);
  } finally { f.close(); }
});

// A bundle as petri evidence prove writes it. Its presentation stands in for
// the TLSNotary proof; the request it records named a cookie the proof hides.
const secrets = ['session=hunter2-cookie', 'hunter2-password'];
const bundleFile = (note = 'Read in the owner\u2019s signed-in browser.') => Buffer.from(`${JSON.stringify({ format: 'amoeba-evidence-bundle-v1', method: 'web_proof',
  backend: { name: 'tlsnotary', revision: '5f67389de3f8962227fddffd71d78a852acc9b01', mode: 'mpc' },
  presentation: Buffer.from(`GET /api/prices HTTP/1.1\r\nCookie: ${secrets[0]}\r\nX-Password: ${secrets[1]}\r\n`).toString('base64'),
  manifest: { format: 'amoeba-extraction-manifest-v1', name: 'Distributor X: DDR5 kit', evidenceType: 'catalog_price', origin: 'portal.distributor.example',
    path: '/api/prices', content: 'json', records: '/items', match: { '/sku': 'DDR5-32G-5600' }, fields: { price: '/unit_price', currency: '/currency' } },
  annotations: { note } }, null, 2)}\n`);
const proven = { ...candidate, canonicalUrl: 'https://portal.distributor.example/api/prices', sourceType: 'distributor price list' };

test('a candidate with a web proof uploads the bundle unchanged, then names its digest; retries keep it idempotent', async () => {
  const f = fixture(); try {
    const a = await f.connect('cedar'), work = await f.discover();
    const run = (await a.call('POST', '/v1/runs', { workId: work.id, sessionId: a.session.id, clientKey: 'start' })).run;
    const bundle = bundleFile(), input = { sessionId: a.session.id, clientKey: 'proven', candidate: proven, evidenceBundle: encodeBundle(bundle) };
    // The API refuses the proof: nothing is submitted, and the same key may retry.
    f.upstream.rejectEvidence = true;
    await assert.rejects(a.call('POST', `/v1/runs/${run.id}/candidates`, input), { code: 'EVIDENCE_REJECTED' });
    assert.equal(f.counts().submissions, 0);
    f.upstream.rejectEvidence = false;
    const first = await a.call('POST', `/v1/runs/${run.id}/candidates`, input);
    // The API got the file's exact bytes, then the candidate with only the bundle digest.
    assert.equal(f.uploads.length, 2); assert(f.uploads.every(upload => upload.equals(bundle)));
    assert.deepEqual(f.submitted.at(-1), { ...proven, authentication: { bundleDigest: digest(bundle) } });
    assert.deepEqual(first.evidence, { bundleDigest: digest(bundle), trust: 'development' });
    assert.equal(first.duplicate, false);
    // A lost response: the same key and bundle return the same result, with no new upload.
    assert.deepEqual(await a.call('POST', `/v1/runs/${run.id}/candidates`, input), first);
    assert.equal(f.uploads.length, 2); assert.equal(f.counts().submissions, 1);
    // The key covers the bundle: another bundle, or none, is another request.
    await assert.rejects(a.call('POST', `/v1/runs/${run.id}/candidates`, { ...input, evidenceBundle: encodeBundle(bundleFile('Another note.')) }), { code: 'REQUEST_CONFLICT' });
    const { evidenceBundle, ...plain } = input;
    await assert.rejects(a.call('POST', `/v1/runs/${run.id}/candidates`, plain), { code: 'REQUEST_CONFLICT' });
    // The feed and the agent's page carry the digest.
    const snapshot = await f.application.handle({ method: 'GET', path: '/v1/owner/snapshot', query: new URLSearchParams({ agentId: a.agent.id }), credential: f.ownerToken });
    const received = snapshot.selected.events.find(event => event.kind === 'candidate_received');
    assert.equal(received.actor, 'backend'); assert.equal(received.referenceUrl, proven.canonicalUrl);
    assert(received.message.includes(digest(bundle)), received.message);
    assert.equal(snapshot.selected.submissions[0].evidence.bundleDigest, digest(bundle));
  } finally { f.close(); }
});

test('a bundle over the verifier\u2019s 2 MiB, or a file that is not a bundle, is refused before any upload', async () => {
  const f = fixture(); try {
    const a = await f.connect('cedar'), work = await f.discover();
    const run = (await a.call('POST', `/v1/runs`, { workId: work.id, sessionId: a.session.id, clientKey: 'start' })).run;
    const send = (clientKey, bytes) => a.call('POST', `/v1/runs/${run.id}/candidates`, { sessionId: a.session.id, clientKey, candidate: proven, evidenceBundle: encodeBundle(bytes) });
    const padded = size => { const head = Buffer.from('{"format":"amoeba-evidence-bundle-v1","presentation":"'), tail = Buffer.from('"}');
      return Buffer.concat([head, Buffer.alloc(size - head.length - tail.length, 'A'), tail]); };
    await assert.rejects(send('large', padded(MAX_BUNDLE_BYTES + 1)), { code: 'BUNDLE_TOO_LARGE' });
    await assert.rejects(send('manifest', Buffer.from('{"format":"amoeba-extraction-manifest-v1"}')), { code: 'INVALID_BUNDLE' });
    await assert.rejects(send('text', Buffer.from('not json')), { code: 'INVALID_BUNDLE' });
    await assert.rejects(a.call('POST', `/v1/runs/${run.id}/candidates`, { sessionId: a.session.id, clientKey: 'encoding', candidate: proven, evidenceBundle: '***' }), { code: 'INVALID_BUNDLE' });
    assert.equal(f.uploads.length, 0); assert.equal(f.counts().submissions, 0);
    // Exactly 2 MiB is the verifier's bound, not over it.
    await send('largest', padded(MAX_BUNDLE_BYTES));
    assert.equal(f.uploads.length, 1); assert.equal(f.uploads[0].length, MAX_BUNDLE_BYTES);
  } finally { f.close(); }
});

test('no cookie, password or proof bytes ever reach the stored events, submissions or snapshot', async () => {
  const root = mkdtempSync(join(tmpdir(), 'petri-research-test-')), path = join(root, 'workspace.sqlite');
  const f = fixture({ databasePath: path });
  let closed = false;
  try {
    const a = await f.connect('cedar'), work = await f.discover();
    const run = (await a.call('POST', '/v1/runs', { workId: work.id, sessionId: a.session.id, clientKey: 'start' })).run;
    const bundle = bundleFile(), presentation = JSON.parse(bundle).presentation;
    await a.call('POST', `/v1/runs/${run.id}/events`, { sessionId: a.session.id, clientKey: 'read', kind: 'read', message: 'Distributor X price list (signed in)', referenceUrl: 'https://portal.distributor.example/account/prices' });
    await a.call('POST', `/v1/runs/${run.id}/candidates`, { sessionId: a.session.id, clientKey: 'proven', candidate: proven, evidenceBundle: encodeBundle(bundle) });
    // A reference that carries a sign-in is refused, not stored.
    await assert.rejects(a.call('POST', `/v1/runs/${run.id}/events`, { sessionId: a.session.id, clientKey: 'leak', kind: 'read', message: 'Prices', referenceUrl: `https://portal.distributor.example/prices?token=${secrets[1]}` }), { code: 'INVALID_REFERENCE' });
    const snapshot = JSON.stringify(await f.application.handle({ method: 'GET', path: '/v1/owner/snapshot', query: new URLSearchParams({ agentId: a.agent.id }), credential: f.ownerToken }));
    const activity = JSON.stringify(await a.call('GET', '/v1/activity'));
    f.close(); closed = true;
    const db = new DatabaseSync(path);
    const stored = JSON.stringify(['events', 'submissions', 'runs', 'work'].map(table => db.prepare(`SELECT * FROM ${table}`).all()));
    assert.equal(db.prepare('SELECT bundle_digest FROM submissions').get().bundle_digest, digest(bundle));
    db.close();
    for (const text of [stored, snapshot, activity]) {
      for (const secret of [...secrets, presentation, 'Cookie', 'X-Password']) assert(!text.includes(secret), `${secret} leaked`);
    }
  } finally { if (!closed) f.close(); const exact = resolve(root); assert.equal(dirname(exact), resolve(tmpdir())); rmSync(exact, { recursive: true, force: true }); }
});
