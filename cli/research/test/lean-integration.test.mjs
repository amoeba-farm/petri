import test from 'node:test';
import assert from 'node:assert/strict';
import { DatabaseSync } from 'node:sqlite';
import http from 'node:http';
import { pathToFileURL } from 'node:url';
import { join } from 'node:path';
import { createApplication } from '../service/application.mjs';
import { token } from '../service/security.mjs';
import { coverageFixture, coverageNow } from './coverage-fixture.mjs';

const backendRoot = process.env.PETRI_TEST_LEAN_BACKEND;
test('workspace pairing and research submit through the real compiled Lean candidate authority', { skip: !backendRoot }, async () => {
  const load = file => import(pathToFileURL(join(backendRoot, file)).href);
  const [{ createOracleCandidateHttp }, { createOracleCandidateDecision }, { createOracleCandidateStore }, { oracleCandidateConfiguration }] = await Promise.all([
    load('edge/src/oracle-candidate-http.mjs'), load('edge/src/oracle-candidate-decision.mjs'), load('edge/src/oracle-candidate-store.mjs'), load('edge/src/oracle-candidate-config.mjs'),
  ]);
  const database = new DatabaseSync(':memory:'), config = { oracleSources: { candidates: oracleCandidateConfiguration({}) } };
  const store = createOracleCandidateStore(config, { database });
  const lean = createOracleCandidateDecision({ executable: join(backendRoot, `.lake/build/bin/oracle_sponsorship_decision${process.platform === 'win32' ? '.exe' : ''}`) });
  const candidateApi = createOracleCandidateHttp({ config, store, lean });
  const server = http.createServer(async (request, response) => {
    response.setHeader('content-type', 'application/json');
    try {
      const url = new URL(request.url, 'http://127.0.0.1');
      const result = await candidateApi.handle({ method: request.method, pathname: url.pathname, searchParams: url.searchParams, request, requestContext: {},
        parseJsonBody: async input => { const chunks = []; for await (const chunk of input) chunks.push(chunk); return JSON.parse(Buffer.concat(chunks)); } });
      response.writeHead(result.statusCode); response.end(JSON.stringify({ ok: true, data: result.data }));
    } catch (error) { response.writeHead(error.status || 400); response.end(JSON.stringify({ ok: false, error: { code: error.code } })); }
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const ownerToken = token();
  const app = createApplication({ ownerToken, masterKey: token(), directory: '.', upstream: `http://127.0.0.1:${server.address().port}` },
    { databasePath: ':memory:', clock: () => coverageNow, oracleReader: async () => coverageFixture() });
  const request = (method, path, body = {}, credential = ownerToken) => app.handle({ method, path, body, credential });
  try {
    await request('POST', '/v1/owner/settings', { payoutAuthority: 'So11111111111111111111111111111111111111112' });
    const pairing = await request('POST', '/v1/login', { name: 'local-integration-agent', harness: 'CLI integration test' });
    await request('POST', '/v1/owner/pairings/approve', { pairingId: pairing.pairingId });
    const loggedIn = await request('POST', '/v1/login/poll', { pairingId: pairing.pairingId, secret: pairing.secret });
    const agent = (method, path, body) => request(method, path, body, loggedIn.credential);
    await request('POST', '/v1/owner/discovery/refresh');
    const work = (await request('GET', '/v1/owner/snapshot')).work[0];
    const session = (await agent('POST', '/v1/sessions', { harness: 'Custom DeepSeek harness' })).session;
    const run = (await agent('POST', '/v1/runs', { workId: work.id, sessionId: session.id, clientKey: 'start' })).run;
    await agent('POST', `/v1/runs/${run.id}/events`, { sessionId: session.id, clientKey: 'progress', kind: 'source_found', message: 'Collected exact public offer and specifications.' });
    const candidate = { marketId: 'ramx', expiryId: 'RAMX-202610', bucketId: '21'.repeat(32), canonicalUrl: 'https://example.com/local-research-test',
      sourceType: 'retail product listing', productDefinition: '32GB DDR5-5600 kit', priceField: 'USD unit price', region: 'United States', currency: 'USD', evidence: 'Exact public offer and matching specification captured for local integration testing.' };
    const result = await agent('POST', `/v1/runs/${run.id}/candidates`, { sessionId: session.id, clientKey: 'source', candidate });
    assert.equal(result.candidate.agentId, loggedIn.agent.id); assert.equal(result.duplicate, false); assert.equal(result.candidate.observation, null);
    await assert.rejects(agent('POST', `/v1/runs/${run.id}/candidates`, { sessionId: session.id, clientKey: 'bad', candidate: { ...candidate, canonicalUrl: 'https://127.0.0.1/private' } }), { code: 'SOURCE_REJECTED' });
    await agent('POST', `/v1/runs/${run.id}/finish`, { sessionId: session.id, summary: 'Local research test complete.' });
    const history = await agent('GET', '/v1/activity');
    assert.equal(history.events.find(e => e.kind === 'candidate_received').actor, 'backend');
    assert.equal(history.events.at(-1).actor, 'agent');
    assert.equal((await agent('GET', '/v1/rewards')).outcomes[0].observation, null);
  } finally { app.close(); await new Promise(resolve => server.close(resolve)); candidateApi.close(); database.close(); }
});
