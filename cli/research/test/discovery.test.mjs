import test from 'node:test';
import assert from 'node:assert/strict';
import { createApplication } from '../service/application.mjs';
import { assessCoverage, describeGap } from '../service/coverage-policy.mjs';
import { token } from '../service/security.mjs';
import { createOracleReader } from '../service/discovery.mjs';
import http from 'node:http';

const now = Date.UTC(2026,9,1,20), bucket = '21'.repeat(32);
const protocol = { programId: 'canonical-program', cluster: 'testnet', namespace: 'test-namespace' };
const directive = { id: 'test-directive', revision: 1, enabled: true, marketIds: [], minActiveSources: 3, minDistinctHosts: 2, maxObservationAgeMs: 600000 };
function observations(count = 1) {
  const freshness = { observedAt: new Date(now).toISOString(), stale: false, refreshHealthy: true, readReady: true };
  const fact = { marketId: 'ramx', expiryId: 'RAMX-202610', oracleMonthAddress: 'canonical-month', phase: 'SourceSubmission', sourceCount: count, onChainAvailable: true, oracleMonthAvailable: true };
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
    // The owner's "Research this" survives a fresh discovery.
    await owner('POST', 'work/focus', { workId: work.id, focused: true });
    await assert.rejects(owner('POST', 'work/focus', { workId: work.id, focused: 'yes' }), { code: 'INVALID_INPUT' });
    await assert.rejects(owner('POST', 'work/focus', { workId: 'f'.repeat(36), focused: true }), { code: 'WORK_NOT_FOUND' });
    await owner('POST', 'discovery/refresh');
    snapshot = await owner('GET', 'snapshot'); assert.equal(snapshot.work[0].id, work.id); assert.equal(snapshot.work[0].guidance.authorType, 'human');
    assert.equal(snapshot.work[0].focused, true);
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
    assert.equal((await agent('GET', '/v1/work')).work[0].focused, true);
    await assert.rejects(agent('POST', '/v1/owner/work/focus', { workId: work.id, focused: false }), { code: 'OWNER_REQUIRED' });
    await owner('POST', 'work/focus', { workId: work.id, focused: false });
    assert.equal((await agent('GET', '/v1/work')).work[0].focused, false);
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
test('each phase takes its own work: sources while a month forms, openings in Opening, updates in Game, nothing in between', () => {
  const day = 86400000;
  // Taking sources: every product the month lists, a proposed source holding its place.
  const forming = observations(1);
  const month = forming.snapshot.state.months[0];
  month.skus = [{ bucketId: bucket, bucketLabel: 'Example governed product' }, { bucketId: '31'.repeat(32), bucketLabel: 'DDR5 RDIMM 64GB' }];
  forming.snapshot.sourceLinks[0].sources[0].status = 'Candidate';
  let all = assessCoverage(forming, [directive], now).assessments;
  assert.deepEqual(all.map(a => [a.kind, a.bucketLabel, a.status, a.activeSources]),
    [['source', 'Example governed product', 'gap', 1], ['source', 'DDR5 RDIMM 64GB', 'gap', 0]]);
  // Scramble: sources until placement ends (day 4), then the month's sources are set.
  const scramble = observations(1);
  for (const fact of [scramble.snapshot.state.months[0], scramble.snapshot.sourceLinks[0]]) fact.phase = 'Scramble';
  scramble.snapshot.state.months[0].schedule = { scrambleStartUtc: new Date(now - 3 * day).toISOString() };
  assert.equal(assessCoverage(scramble, [directive], now).assessments[0].kind, 'source');
  scramble.snapshot.state.months[0].schedule = { scrambleStartUtc: new Date(now - 5 * day).toISOString() };
  all = assessCoverage(scramble, [directive], now).assessments;
  assert.deepEqual(all.map(a => a.status), ['inactive']);
  // Opening: one opening per frozen source still without one, with its
  // product's bond and budget; none for an opened, pending or rejected source.
  const opening = observations(5);
  for (const fact of [opening.snapshot.state.months[0], opening.snapshot.sourceLinks[0]]) fact.phase = 'Opening';
  opening.snapshot.state.months[0].schedule = { scrambleStartUtc: new Date(now - 7 * day - 3600000).toISOString(), listingUtc: new Date(now + day - 3600000).toISOString() };
  opening.snapshot.state.months[0].skus = [{ bucketId: bucket, bucketLabel: 'Example governed product', openingBondAtomic: '2500000', openingBudgetAtomic: '30000000' }];
  const sources = opening.snapshot.sourceLinks[0].sources;
  sources.forEach((s, i) => { s.sourceAddress = `source-address-${i}`; s.status = 'Frozen'; s.openingSubmitted = false; });
  Object.assign(sources[1], { status: 'Active', openingSubmitted: true });
  sources[2].status = 'OpeningPending';
  sources[3].status = 'Rejected';
  delete sources[4].openingSubmitted;
  all = assessCoverage(opening, [directive], now).assessments;
  assert.deepEqual(all.map(a => [a.kind, a.sourceAddress, a.status, a.openingBondAtomic, a.openingBudgetAtomic]),
    [['opening', 'source-address-0', 'gap', '2500000', '30000000'], ['opening', 'source-address-4', 'gap', '2500000', '30000000']]);
  assert.equal(all[0].url, 'https://source-0.example/product');
  assert.equal(all[0].openingStartUtc, new Date(now - 3600000).toISOString());
  assert.equal(all[0].listingUtc, new Date(now + day - 3600000).toISOString());
  const brief = describeGap(all[0]);
  assert.equal(brief.title, 'RAMX · Example governed product · opening');
  assert.match(brief.brief, /opening bond \(2\.5 USDC\)/u);
  // Without the products' bonds (an older API), the opening still comes up.
  delete opening.snapshot.state.months[0].skus;
  assert.deepEqual(assessCoverage(opening, [directive], now).assessments.map(a => [a.kind, a.openingBondAtomic]), [['opening', null], ['opening', null]]);
  // Game: one update per live source, none for a rejected one, and no source gaps.
  const game = observations(3);
  for (const fact of [game.snapshot.state.months[0], game.snapshot.sourceLinks[0]]) fact.phase = 'Game';
  game.snapshot.sourceLinks[0].sources[2].status = 'Rejected';
  game.snapshot.sourceLinks[0].sources.forEach((s, i) => { s.sourceAddress = `source-address-${i}`; });
  game.snapshot.state.months[0].rewards = { remainingAtomic: '15000000' };
  all = assessCoverage(game, [directive], now).assessments;
  assert.deepEqual(all.map(a => [a.kind, a.sourceAddress, a.status, a.rewardsRemainingAtomic]),
    [['update', 'source-address-0', 'gap', '15000000'], ['update', 'source-address-1', 'gap', '15000000']]);
  assert.equal(all[0].url, 'https://source-0.example/product');
});
test('work next hands out openings before updates, and an unpaid one only when the owner picks it', async () => {
  const day = 86400000, ownerToken = token(), agentId = token();
  // RAMX-202611 is in Opening with one frozen source; RAMX-202610 in Game with rewards.
  const input = observations(1), game = input.snapshot.state.months[0], gameLinks = input.snapshot.sourceLinks[0];
  game.phase = gameLinks.phase = 'Game'; game.rewards = { remainingAtomic: '15000000' };
  gameLinks.sources[0].sourceAddress = 'live-source';
  const opening = { ...game, expiryId: 'RAMX-202611', oracleMonthAddress: 'opening-month', phase: 'Opening', rewards: { remainingAtomic: '50000000' },
    schedule: { scrambleStartUtc: new Date(now - 7 * day).toISOString(), listingUtc: new Date(now + day).toISOString() },
    skus: [{ bucketId: bucket, bucketLabel: 'Example governed product', openingBondAtomic: '1000000', openingBudgetAtomic: '30000000' }] };
  input.snapshot.state.months.push(opening);
  input.snapshot.sourceLinks.push({ ...gameLinks, expiryId: opening.expiryId, oracleMonthAddress: opening.oracleMonthAddress, phase: 'Opening',
    sources: [{ ...gameLinks.sources[0], sourceId: 'ab'.repeat(32), sourceAddress: 'frozen-source', status: 'Frozen', openingSubmitted: false }] });
  input.catalog.markets[0].expiries.push({ expiryId: opening.expiryId, settlementUtc: '2026-12-01T00:00:00.000Z' });
  const app = createApplication({ ownerToken, masterKey: token(), directory: '.', upstream: 'http://127.0.0.1:4500' }, { databasePath: ':memory:', clock: () => now,
    oracleReader: async () => input,
    upstream: { register: async payoutAuthority => ({ agent: { agentId, payoutAuthority }, credential: token() }), candidates: async () => ({ candidates: [], complete: true }) } });
  const owner = (method, path, body = {}) => app.handle({ method, path: `/v1/owner/${path}`, body, credential: ownerToken });
  try {
    await owner('POST', 'discovery/refresh');
    await owner('POST', 'settings', { payoutAuthority: 'So11111111111111111111111111111111111111112' });
    const pair = await app.handle({ method: 'POST', path: '/v1/login', body: { name: 'researcher', harness: 'Test' } });
    await owner('POST', 'pairings/approve', { pairingId: pair.pairingId });
    const login = await app.handle({ method: 'POST', path: '/v1/login/poll', body: { pairingId: pair.pairingId, secret: pair.secret } });
    const agent = (method, path, body = {}) => app.handle({ method, path, body, credential: login.credential });
    const session = (await agent('POST', '/v1/sessions')).session;
    const kinds = (await owner('GET', 'snapshot')).work.map(w => [w.objective.kind, w.candidateScope.sourceId ? 'per source' : 'per product']).sort();
    assert.deepEqual(kinds, [['opening', 'per source'], ['update', 'per source']]);
    let next = await agent('POST', '/v1/work/next', { sessionId: session.id });
    assert.equal(next.assignment.objective.kind, 'opening');
    assert.equal(next.assignment.objective.sourceAddress, 'frozen-source');
    assert.equal(next.assignment.objective.openingBondAtomic, '1000000');
    assert.equal(next.assignment.title, 'RAMX · Example governed product · opening');
    await agent('POST', `/v1/runs/${next.run.id}/finish`, { sessionId: session.id, summary: 'Opened.' });
    // A product without an opening budget: its opening waits for the owner's pick.
    opening.skus[0].openingBudgetAtomic = '0';
    await owner('POST', 'discovery/refresh');
    next = await agent('POST', '/v1/work/next', { sessionId: session.id });
    assert.equal(next.assignment.objective.kind, 'update');
    const unpaid = (await owner('GET', 'snapshot')).work.find(w => w.objective.kind === 'opening');
    await owner('POST', 'work/focus', { workId: unpaid.id, focused: true });
    next = await agent('POST', '/v1/work/next', { sessionId: session.id });
    assert.equal(next.assignment.id, unpaid.id);
  } finally { app.close(); }
});
test('a month taking sources keeps its products while a proposal waits for a supporter', () => {
  // Its on-chain count is the supported sources (none yet); the inventory lists the proposal.
  const forming = observations(1);
  forming.snapshot.state.months[0].sourceCount = 0;
  forming.snapshot.sourceLinks[0].sources[0].status = 'Candidate';
  forming.snapshot.state.months[0].skus = [{ bucketId: bucket, bucketLabel: 'Example governed product' }];
  const [gap] = assessCoverage(forming, [directive], now).assessments;
  assert.equal(gap.kind, 'source'); assert.equal(gap.status, 'gap'); assert.equal(gap.activeSources, 1);
  // In Game the inventory is the recipe and must match the month's count.
  const game = observations(2);
  for (const fact of [game.snapshot.state.months[0], game.snapshot.sourceLinks[0]]) fact.phase = 'Game';
  game.snapshot.state.months[0].sourceCount = 3;
  assert.deepEqual(assessCoverage(game, [directive], now).assessments.map(a => a.status), ['unknown']);
});
test('a settled month with frozen rewards gives its claims once, to a workspace that worked on it', async () => {
  const ownerToken = token(), agentId = token();
  let time = now;
  // RAMX-202610 in Game, RAMX-202609 settled and frozen already (nobody here worked on it).
  const input = observations(1), game = input.snapshot.state.months[0], gameLinks = input.snapshot.sourceLinks[0];
  game.phase = gameLinks.phase = 'Game'; game.rewards = { phase: 'Funded', remainingAtomic: '15000000' };
  gameLinks.sources[0].sourceAddress = 'live-source';
  const old = { ...game, expiryId: 'RAMX-202609', oracleMonthAddress: 'old-month', phase: 'Settled', rewards: { phase: 'EntitlementsFinalized', remainingAtomic: '9000000' } };
  input.snapshot.state.months.push(old);
  input.catalog.markets[0].expiries.push({ expiryId: old.expiryId, settlementUtc: '2026-10-01T00:00:00.000Z' });
  const assessed = assessCoverage(input, [directive], now).assessments;
  assert.deepEqual(assessed.map(a => [a.expiryId, a.kind, a.status]), [['RAMX-202610', 'update', 'gap'], ['RAMX-202609', 'claims', 'gap']]);
  assert.equal(describeGap(assessed[1]).title, 'RAMX · RAMX-202609 · claims');
  // Settled but not frozen yet: nothing to do.
  assert.deepEqual(assessCoverage({ ...input, snapshot: { ...input.snapshot, state: { months: [{ ...old, rewards: { phase: 'Funded' } }] } } }, [directive], now)
    .assessments.filter(a => a.expiryId === old.expiryId).map(a => [a.kind ?? null, a.status]), [[null, 'inactive']]);
  const app = createApplication({ ownerToken, masterKey: token(), directory: '.', upstream: 'http://127.0.0.1:4500' }, { databasePath: ':memory:', clock: () => time++,
    oracleReader: async () => input,
    upstream: { register: async payoutAuthority => ({ agent: { agentId, payoutAuthority }, credential: token() }), candidates: async () => ({ candidates: [], complete: true }) } });
  const owner = (method, path, body = {}) => app.handle({ method, path: `/v1/owner/${path}`, body, credential: ownerToken });
  try {
    await owner('POST', 'discovery/refresh');
    await owner('POST', 'settings', { payoutAuthority: 'So11111111111111111111111111111111111111112' });
    const pair = await app.handle({ method: 'POST', path: '/v1/login', body: { name: 'researcher', harness: 'Test' } });
    await owner('POST', 'pairings/approve', { pairingId: pair.pairingId });
    const login = await app.handle({ method: 'POST', path: '/v1/login/poll', body: { pairingId: pair.pairingId, secret: pair.secret } });
    const agent = (method, path, body = {}) => app.handle({ method, path, body, credential: login.credential });
    const session = (await agent('POST', '/v1/sessions')).session;
    const next = () => agent('POST', '/v1/work/next', { sessionId: session.id });
    // RAMX-202609's claims never come up: no run here was on that month.
    let run = await next();
    assert.equal(run.assignment.objective.kind, 'update');
    await agent('POST', `/v1/runs/${run.run.id}/finish`, { sessionId: session.id, summary: 'Updated.' });
    // RAMX-202610 settles and its rewards freeze: its claims come up first, once.
    game.phase = gameLinks.phase = 'Settled'; game.rewards = { phase: 'EntitlementsFinalized', remainingAtomic: '15000000' };
    input.catalog.markets[0].expiries[0].settlementUtc = new Date(now - 1000).toISOString();
    await owner('POST', 'discovery/refresh');
    run = await next();
    assert.equal(run.assignment.objective.kind, 'claims');
    assert.equal(run.assignment.candidateScope.expiryId, 'RAMX-202610');
    await agent('POST', `/v1/runs/${run.run.id}/finish`, { sessionId: session.id, summary: 'Claimed.' });
    assert.equal((await next()).status, 'idle');
    // The owner's pick brings them back for one more run.
    const claims = (await owner('GET', 'snapshot')).work.find(w => w.objective.kind === 'claims' && w.candidateScope.expiryId === 'RAMX-202610');
    await owner('POST', 'work/focus', { workId: claims.id, focused: true });
    run = await next();
    assert.equal(run.assignment.id, claims.id);
    await agent('POST', `/v1/runs/${run.run.id}/finish`, { sessionId: session.id, summary: 'Nothing left.' });
    assert.equal((await next()).status, 'idle');
  } finally { app.close(); }
});
test('a refresh archives the work of a series the oracle stopped listing, and only a successful one', async () => {
  const ownerToken = token(), agentId = token();
  // RAMX-202610 and RAMX-202611 both take sources: a gap each.
  const listed = () => {
    const input = observations(1), [fact] = input.snapshot.state.months, [links] = input.snapshot.sourceLinks;
    const next = { ...fact, expiryId: 'RAMX-202611', oracleMonthAddress: 'next-month' };
    input.snapshot.state.months.push(next);
    input.snapshot.sourceLinks.push({ ...links, expiryId: next.expiryId, oracleMonthAddress: next.oracleMonthAddress });
    input.catalog.markets[0].expiries.push({ expiryId: next.expiryId, settlementUtc: '2026-12-01T00:00:00.000Z' });
    return input;
  };
  let input = listed(), unavailable = false;
  const app = createApplication({ ownerToken, masterKey: token(), directory: '.', upstream: 'http://127.0.0.1:4500' }, { databasePath: ':memory:', clock: () => now,
    oracleReader: async () => { if (unavailable) throw new Error('offline'); return input; },
    upstream: { register: async payoutAuthority => ({ agent: { agentId, payoutAuthority }, credential: token() }), candidates: async () => ({ candidates: [], complete: true }) } });
  const owner = (method, path, body = {}) => app.handle({ method, path: `/v1/owner/${path}`, body, credential: ownerToken });
  const refresh = () => owner('POST', 'discovery/refresh');
  const work = async () => (await owner('GET', 'snapshot')).work;
  const months = async () => (await work()).map(w => w.candidateScope.expiryId).sort();
  const both = ['RAMX-202610', 'RAMX-202611'];
  try {
    await refresh();
    assert.deepEqual(await months(), both);
    const later = (await work()).find(w => w.candidateScope.expiryId === 'RAMX-202611');
    await owner('POST', 'work/focus', { workId: later.id, focused: true });
    // A failed refresh archives nothing: an unreachable oracle, or a read
    // without RAMX-202611 that does not hold together.
    unavailable = true; await refresh();
    assert.deepEqual(await months(), both);
    unavailable = false; input = observations(1); input.catalog.protocol = { ...protocol, programId: 'other' }; await refresh();
    assert.equal((await owner('GET', 'snapshot')).discovery.error !== null, true);
    assert.deepEqual(await months(), both);
    // A series still listed stays, whatever its assessment: its month not
    // read, past settlement, or left out by every directive. Its work is
    // only withheld from agents.
    input = listed(); input.snapshot.state.months[1].oracleMonthAvailable = false; await refresh();
    assert.deepEqual(await months(), both);
    assert.equal((await work()).find(w => w.id === later.id).availability, 'unknown');
    input = listed(); input.catalog.markets[0].expiries[1].settlementUtc = new Date(now).toISOString(); await refresh();
    assert.equal((await owner('GET', 'snapshot')).discovery.assessments.find(a => a.expiryId === 'RAMX-202611').status, 'inactive');
    assert.deepEqual(await months(), both);
    const [policy] = (await owner('GET', 'snapshot')).directives;
    await owner('POST', 'directives', { id: policy.id, marketIds: ['nandx'], minActiveSources: 3, minDistinctHosts: 2 });
    input = listed(); await refresh();
    assert.deepEqual((await owner('GET', 'snapshot')).discovery.assessments, []);
    assert.deepEqual(await months(), both);
    await owner('POST', 'directives', { id: policy.id, marketIds: [], minActiveSources: 3, minDistinctHosts: 2 });
    await refresh();
    // The catalog drops RAMX-202611: its work is archived, gone from every list.
    input = observations(1); await refresh();
    assert.deepEqual(await months(), ['RAMX-202610']);
    await assert.rejects(owner('POST', 'work/focus', { workId: later.id, focused: false }), { code: 'WORK_NOT_FOUND' });
    // Listed again, it comes back as it was.
    input = listed(); await refresh();
    const back = (await work()).find(w => w.id === later.id);
    assert.equal(back.focused, true); assert.equal(back.availability, 'available');
    // The owner's own archive stays, listed or not.
    await owner('POST', 'work/archive', { workId: later.id });
    input = observations(1); await refresh();
    input = listed(); await refresh();
    assert.deepEqual(await months(), ['RAMX-202610']);
  } finally { app.close(); }
});
