import test from 'node:test';
import assert from 'node:assert/strict';
import { createApplication } from '../service/application.mjs';
import { assessCoverage } from '../service/coverage-policy.mjs';
import { token } from '../service/security.mjs';
import { createOracleReader } from '../service/discovery.mjs';
import http from 'node:http';

const now = Date.UTC(2026,9,1,20), bucket = '21'.repeat(32);
const protocol = { programId: 'canonical-program', cluster: 'testnet', namespace: 'test-namespace' };
const directive = { id: 'test-directive', revision: 1, enabled: true, marketIds: [], minActiveSources: 3, minDistinctHosts: 2, maxObservationAgeMs: 600000 };
function observations(count = 1) {
  const freshness = { observedAt: new Date(now).toISOString(), stale: false, refreshHealthy: true, readReady: true };
  const fact = { marketId: 'ramx', expiryId: 'RAMX-202610', oracleMonthAddress: 'canonical-month', phase: 'Game', sourceCount: count, onChainAvailable: true, oracleMonthAvailable: true };
  return {
    snapshot: { protocol, freshness, state: { months: [fact] }, sourceLinks: [{ ...fact, usage: 'source-display-only', observedAt: freshness.observedAt,
      sources: Array.from({ length: count }, (_, i) => ({ sourceId: i.toString(16).padStart(64,'0'), bucketId: bucket, bucketLabel: 'Example governed product', status: 'Active', canonicalLocator: { verified: true, url: `https://source-${i}.example/product` } })) }] },
    catalog: { protocol, freshness, markets: [{ marketId: 'ramx', onChainAvailable: true, expiries: [{ expiryId: fact.expiryId, settlementUtc: '2026-11-01T00:00:00.000Z' }] }] },
  };
}
test('the oracle reader uses only the two public GET routes and sends no workspace credential', async () => {
  const input = observations(), requests = [];
  const server = http.createServer((request, response) => {
    requests.push({ method: request.method, path: request.url, authorization: request.headers.authorization });
    response.setHeader('content-type', 'application/json');
    response.end(JSON.stringify({ ok: true, data: request.url === '/v1/oracle/catalog' ? input.catalog : input.snapshot }));
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  try {
    const observed = await createOracleReader(`http://127.0.0.1:${server.address().port}`)();
    assert.deepEqual(observed, input);
    assert.deepEqual(requests.map(r => r.path).sort(), ['/v1/oracle/catalog','/v1/oracle/public-snapshot']);
    assert(requests.every(r => r.method === 'GET' && r.authorization === undefined));
  } finally { await new Promise(resolve => server.close(resolve)); }
});
test('coverage goals use active sources and distinct verified hosts, not queued candidates', () => {
  const input = observations();
  let result = assessCoverage(input, [directive], now).assessments[0];
  assert.equal(result.status, 'gap'); assert.equal(result.activeSources, 1); assert.equal(result.distinctHosts, 1);
  assert.equal(result.bucketId, bucket); assert.equal(result.targets.activeSources, 3);
  assert.equal(assessCoverage(observations(3), [directive], now).assessments[0].status, 'covered');
  const sameHost = observations(3); sameHost.snapshot.sourceLinks[0].sources.forEach(s => { s.canonicalLocator.url = 'https://same.example/product'; });
  assert.deepEqual(assessCoverage(sameHost, [directive], now).assessments[0].reasons, ['insufficient_distinct_hosts']);
  const inactive = observations(3); inactive.snapshot.sourceLinks[0].sources[0].status = 'Rejected';
  assert.equal(assessCoverage(inactive, [directive], now).assessments[0].activeSources, 2);
});
test('uninitialized, stale, incomplete, duplicate and cross-series observations never become zero-source claims', () => {
  const cases = [
    input => { input.snapshot.state.months[0].oracleMonthAvailable = false; },
    input => { input.snapshot.freshness = { ...input.snapshot.freshness, stale: true }; },
    input => { input.snapshot.sourceLinks[0].sources = []; },
    input => { input.snapshot.sourceLinks[0].oracleMonthAddress = 'wrong-month'; },
    input => { input.snapshot.sourceLinks[0].sources[1].sourceId = input.snapshot.sourceLinks[0].sources[0].sourceId; },
    input => { input.snapshot.sourceLinks[0].observedAt = new Date(now - 600001).toISOString(); },
    input => { input.catalog.markets[0].expiries[0].settlementUtc = 'unknown'; },
  ];
  for (const change of cases) { const input = observations(3); change(input); const result = assessCoverage(input, [directive], now).assessments[0]; assert.equal(result.status, 'unknown'); assert.equal(result.activeSources, undefined); }
  const different = observations(); different.catalog.protocol = { ...protocol, programId: 'other' };
  assert.throws(() => assessCoverage(different, [directive], now), { code: 'ORACLE_DATA_INVALID' });
  const unknownHosts = observations(3); unknownHosts.snapshot.sourceLinks[0].sources[0].canonicalLocator = null;
  assert.equal(assessCoverage(unknownHosts, [directive], now).assessments[0].status, 'covered');
  unknownHosts.snapshot.sourceLinks[0].sources[1].canonicalLocator = null;
  const partial = assessCoverage(unknownHosts, [directive], now).assessments[0]; assert.equal(partial.status, 'unknown'); assert.equal(partial.distinctHosts, null);
});
test('settled series and paused or out-of-scope directives create no research work', () => {
  const input = observations(); input.catalog.markets[0].expiries[0].settlementUtc = new Date(now).toISOString();
  assert.equal(assessCoverage(input, [directive], now).assessments[0].status, 'inactive');
  assert.deepEqual(assessCoverage(input, [{ ...directive, enabled: false }], now).assessments, []);
  assert.deepEqual(assessCoverage(input, [{ ...directive, marketIds: ['nandx'] }], now).assessments, []);
  const empty = assessCoverage(observations(0), [directive], now).assessments[0];
  assert.equal(empty.status, 'gap'); assert.equal(empty.activeSources, 0); assert.equal(empty.bucketId, null);
});
test('discovery deduplicates work, preserves human guidance and run context, and resolves only on fresh oracle coverage', async () => {
  let time = now, input = observations(), unavailable = false;
  const ownerToken = token(), agentId = token();
  const app = createApplication({ ownerToken, masterKey: token(), directory: '.', upstream: 'http://127.0.0.1:4500' }, { databasePath: ':memory:', clock: () => time,
    oracleReader: async () => { if (unavailable) throw new Error('offline'); return input; },
    upstream: { register: async payoutAuthority => ({ agent: { agentId, payoutAuthority }, credential: token() }), candidates: async () => ({ candidates: [], complete: true }) } });
  const owner = (method, path, body = {}) => app.handle({ method, path: `/v1/owner/${path}`, body, credential: ownerToken });
  try {
    await owner('POST', 'discovery/refresh');
    let snapshot = await owner('GET', 'snapshot'); assert.equal(snapshot.directives.length, 1); assert.equal(snapshot.work.length, 1);
    const work = snapshot.work[0], policy = snapshot.directives[0];
    await owner('POST', 'work/refine', { workId: work.id, instruction: 'Prioritize manufacturer listings in the required region.' });
    await owner('POST', 'discovery/refresh');
    snapshot = await owner('GET', 'snapshot'); assert.equal(snapshot.work[0].id, work.id); assert.equal(snapshot.work[0].guidance.authorType, 'human');
    await owner('POST', 'settings', { payoutAuthority: 'So11111111111111111111111111111111111111112' });
    const pair = await app.handle({ method: 'POST', path: '/v1/login', body: { name: 'researcher', harness: 'Test' } });
    await owner('POST', 'pairings/approve', { pairingId: pair.pairingId });
    const login = await app.handle({ method: 'POST', path: '/v1/login/poll', body: { pairingId: pair.pairingId, secret: pair.secret } });
    const agent = (method, path, body = {}) => app.handle({ method, path, body, credential: login.credential });
    const session = (await agent('POST', '/v1/sessions')).session;
    const run = (await agent('POST', '/v1/runs', { workId: work.id, sessionId: session.id, clientKey: 'start' })).run;
    assert.equal(run.researchContext.objective.directiveRevision, 1);
    assert.equal(run.researchContext.guidance.instruction, 'Prioritize manufacturer listings in the required region.');
    await agent('POST', `/v1/runs/${run.id}/finish`, { sessionId: session.id, summary: 'Research done.' });
    assert.equal((await agent('GET', '/v1/work')).work.length, 1);
    input = observations(3); await owner('POST', 'discovery/refresh');
    assert.equal((await agent('GET', '/v1/work')).work.length, 0);
    assert.equal((await owner('GET', 'snapshot')).work[0].availability, 'resolved');
    input = observations(); await owner('POST', 'discovery/refresh');
    assert.equal((await agent('GET', '/v1/work')).work[0].id, work.id);
    unavailable = true; await owner('POST', 'discovery/refresh');
    assert.equal((await agent('GET', '/v1/work')).work.length, 0);
    await assert.rejects(agent('POST', '/v1/runs', { workId: work.id, sessionId: session.id, clientKey: 'stale' }), { code: 'WORK_NOT_FOUND' });
    unavailable = false; await owner('POST', 'discovery/refresh'); time += 600001;
    assert.equal((await agent('GET', '/v1/work')).work.length, 0);
    time = now;
    await owner('POST', 'directives', { id: policy.id, marketIds: [], minActiveSources: 3, minDistinctHosts: 2, enabled: false });
    await owner('POST', 'discovery/refresh'); assert.equal((await agent('GET', '/v1/work')).work.length, 0);
    assert.equal((await agent('GET', '/v1/activity')).runs[0].researchContext.objective.directiveRevision, 1);
    await assert.rejects(agent('POST', '/v1/owner/directives', { marketIds: [], minActiveSources: 1, minDistinctHosts: 1 }), { code: 'OWNER_REQUIRED' });
    await assert.rejects(owner('POST', 'work/refine', { workId: work.id, instruction: 'x', authorType: 'farm_ai' }), { code: 'INVALID_INPUT' });
  } finally { app.close(); }
});
