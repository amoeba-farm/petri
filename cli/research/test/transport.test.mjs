import test from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import { mkdtempSync, rmSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve, dirname } from 'node:path';
import { startServer } from '../service/server.mjs';
import { endpoint, token, privateDirectory, privateWrite, loadOwner } from '../service/security.mjs';
import { createHttpClient } from '../client/http.mjs';
import { createAgentClient } from '../client/agent.mjs';
import { createUpstream } from '../service/upstream.mjs';
import { ensureService, runtimeId } from '../service/lifecycle.mjs';
import { createMcpHandler, serveMcp } from '../client/mcp.mjs';
import { Readable, Writable } from 'node:stream';
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { coverageFixture, coverageNow } from './coverage-fixture.mjs';

const payout = 'So11111111111111111111111111111111111111112';
const cleanup = path => { const exact = resolve(path); assert.equal(dirname(exact), resolve(tmpdir())); assert(exact.split(/[\\/]/u).at(-1).startsWith('petri-research-test-')); rmSync(exact, { recursive: true, force: true, maxRetries: 20, retryDelay: 50 }); };
async function start(directory) {
  const config = { directory, port: 0, ownerToken: token(), masterKey: token(), upstream: 'http://127.0.0.1:4500' };
  const upstream = { async register(payoutAuthority) { return { agent: { agentId: token(), payoutAuthority }, credential: token() }; }, async candidates() { return { candidates: [], complete: true }; } };
  const service = await startServer(config, { upstream, clock: () => coverageNow, oracleReader: async () => coverageFixture() });
  const request = createHttpClient(service.baseUrl);
  const owner = (method, path, body) => request(method, `/v1/owner/${path}`, body, config.ownerToken);
  return { ...service, config, upstream, request, owner };
}

test('real HTTP login saves a private profile, resumes after restart, and never outputs credentials', async () => {
  const root = mkdtempSync(join(tmpdir(), 'petri-research-test-'));
  let server;
  try {
    server = await start(root);
    const baseUrl = server.baseUrl, profileDirectory = join(root, 'profiles');
    const client = createAgentClient({ name: 'cedar', harness: 'Codex', baseUrl, profileDirectory });
    const pending = await client.login();
    assert.equal(pending.status, 'pending'); assert.equal(pending.secret, undefined); assert.equal(pending.credential, undefined);
    const snapshot = await server.owner('GET', 'snapshot');
    assert.equal(snapshot.pairings[0].name, 'cedar');
    await server.owner('POST', 'settings', { payoutAuthority: payout });
    await server.owner('POST', 'pairings/approve', { pairingId: snapshot.pairings[0].id });
    const connected = await client.login(); assert.equal(connected.status, 'connected');
    const saved = JSON.parse(readFileSync(join(profileDirectory, 'cedar.json'), 'utf8'));
    assert(saved.credential); assert(!JSON.stringify(connected).includes(saved.credential));
    await server.owner('POST', 'discovery/refresh', {});
    const work = (await server.owner('GET', 'snapshot')).work[0];
    const run = (await client.startWork(work.id, 'run-1')).run;
    await client.report(run.id, { message: 'Read the public listing.', clientKey: 'e1' });
    const credentials = [saved.credential, server.config.ownerToken, server.config.masterKey];
    const exposed = JSON.stringify(await server.owner('GET', 'snapshot'));
    for (const secret of credentials) assert(!exposed.includes(secret));
    const config = { ...server.config, port: Number(new URL(baseUrl).port) }, upstream = server.upstream;
    await server.close(); server = await startServer(config, { upstream, clock: () => coverageNow, oracleReader: async () => coverageFixture() });
    const resumed = createAgentClient({ name: 'cedar', harness: 'Kimi Code', baseUrl, profileDirectory });
    assert.equal((await resumed.login()).agent.id, connected.agent.id);
    assert.equal((await resumed.activity()).events.at(-1).message, 'Read the public listing.');
    await resumed.closeSession(); const next = await resumed.openSession();
    assert.notEqual(next.id, connected.session.id);
  } finally { if (server) await server.close(); cleanup(root); }
});

test('the local HTTP boundary rejects foreign origins, host rebinding, methods and oversized requests', async () => {
  const root = mkdtempSync(join(tmpdir(), 'petri-research-test-')); const server = await start(root);
  try {
    for (const [headers, expected] of [[{ origin: 'https://evil.example' },403],[{ host: 'evil.example' },403]]) {
      const response = await new Promise((resolve, reject) => { const request = http.get(`${server.baseUrl}/health`, { headers }, response => { response.resume(); resolve(response); }); request.on('error', reject); });
      assert.equal(response.statusCode, expected); assert.equal(response.headers['cache-control'], 'no-store');
    }
    assert.equal((await fetch(`${server.baseUrl}/health`, { method: 'PUT' })).status, 405);
    assert.equal((await fetch(`${server.baseUrl}/v1/login`, { method: 'POST', body: '{}' })).status, 415);
    const tooLarge = await fetch(`${server.baseUrl}/v1/login`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ name: 'a'.repeat(40000), harness: 'X' }) });
    assert.equal(tooLarge.status, 413);
  } finally { await server.close(); cleanup(root); }
});

test('service replacement requires its owner, preserves state, and refuses active submissions', async () => {
  const root = mkdtempSync(join(tmpdir(), 'petri-research-test-'));
  const config = { directory: root, port: 0, ownerToken: token(), masterKey: token(), upstream: 'http://127.0.0.1:4500' };
  let release, entered;
  const started = new Promise(resolve => { entered = resolve; });
  const pending = new Promise(resolve => { release = resolve; });
  const upstream = { async register(payoutAuthority) { entered(); await pending; return { agent: { agentId: token(), payoutAuthority }, credential: token() }; } };
  let server = await startServer(config, { upstream, runtimeId: 'before-update', seedDirectives: false });
  try {
    const request = createHttpClient(server.baseUrl);
    const owner = (method, path, body) => request(method, `/v1/owner/${path}`, body, config.ownerToken);
    assert.equal((await owner('GET', 'ping')).runtime, 'before-update');
    await assert.rejects(request('POST', '/v1/owner/restart', {}, token()), { code: 'OWNER_REQUIRED' });
    await owner('POST', 'settings', { payoutAuthority: payout });
    const pairing = await request('POST', '/v1/login', { name: 'cedar', harness: 'Codex' });
    const saving = owner('POST', 'pairings/approve', { pairingId: pairing.pairingId });
    await started;
    await assert.rejects(owner('POST', 'restart', {}), { code: 'SERVICE_BUSY' });
    assert.equal((await owner('GET', 'ping')).runtime, 'before-update');
    release(); const approved = await saving;
    const port = Number(new URL(server.baseUrl).port);
    assert.equal((await owner('POST', 'restart', {})).restarting, true);
    await server.close();
    server = await startServer({ ...config, port }, { upstream, runtimeId: 'after-update', seedDirectives: false });
    const snapshot = await owner('GET', 'snapshot');
    assert.equal((await owner('GET', 'ping')).runtime, 'after-update');
    assert.equal(snapshot.settings.payoutAuthority, payout);
    assert.equal(snapshot.agents[0].id, approved.agent.id);
  } finally { release(); await server.close(); cleanup(root); }
});

test('bootstrap replaces an older service with the bundled version and leaves workspace data intact', async () => {
  const root = mkdtempSync(join(tmpdir(), 'petri-research-test-'));
  let server, request, config;
  try {
    config = loadOwner(join(root, 'state'), 'http://127.0.0.1:1', undefined, 'http://127.0.0.1:1');
    server = await startServer({ ...config, port: 0 }, { runtimeId: 'old-test-bundle', seedDirectives: false });
    config = loadOwner(config.directory, undefined, Number(new URL(server.baseUrl).port));
    request = createHttpClient(config.baseUrl);
    await request('POST', '/v1/owner/settings', { payoutAuthority: payout }, config.ownerToken);
    await ensureService(config);
    assert.equal((await request('GET', '/v1/owner/ping', undefined, config.ownerToken)).runtime, runtimeId());
    assert.equal((await request('GET', '/v1/owner/snapshot', undefined, config.ownerToken)).settings.payoutAuthority, payout);
    // A second bootstrap reuses the exact runtime instead of spawning another server.
    await ensureService(config);
  } finally {
    if (server) await server.close();
    if (request) {
      try { await request('POST', '/v1/owner/restart', {}, config.ownerToken); } catch {}
      for (let attempt = 0; attempt < 100; attempt++) {
        try { await request('GET', '/v1/owner/ping', undefined, config.ownerToken); }
        catch { break; }
        await new Promise(resolve => setTimeout(resolve, 50));
      }
    }
    cleanup(root);
  }
});

test('upstream credentials are never forwarded through redirects and remote HTTP is refused', async () => {
  for (const url of ['http://example.com','https://user:pass@example.com','https://example.com/path','https://example.com?key=secret']) assert.throws(() => endpoint(url), { code: 'INVALID_ENDPOINT' });
  let destinationRequests = 0;
  const receiver = http.createServer((req, res) => { destinationRequests++; res.end('{}'); }); await new Promise(resolve => receiver.listen(0, '127.0.0.1', resolve));
  const redirect = http.createServer((req, res) => { res.writeHead(307, { location: `http://127.0.0.1:${receiver.address().port}/collect` }); res.end(); }); await new Promise(resolve => redirect.listen(0, '127.0.0.1', resolve));
  try {
    await assert.rejects(createUpstream(`http://127.0.0.1:${redirect.address().port}`).submit({}, token()), { code: 'UPSTREAM_UNAVAILABLE' });
    assert.equal(destinationRequests, 0);
  } finally { await Promise.all([new Promise(resolve => receiver.close(resolve)), new Promise(resolve => redirect.close(resolve))]); }
});

test('private storage refuses unrelated directories and writes complete replacement profiles', () => {
  const root = mkdtempSync(join(tmpdir(), 'petri-research-test-'));
  try {
    const privatePath = privateDirectory(join(root, 'safe')); const file = join(privatePath, 'agent.json');
    privateWrite(file, { version: 1, value: 'first' }); privateWrite(file, { version: 1, value: 'second' });
    assert.equal(JSON.parse(readFileSync(file, 'utf8')).value, 'second');
    writeFileSync(join(root, 'unrelated.txt'), 'untouched');
    assert.throws(() => privateDirectory(root), { code: 'UNSAFE_STORAGE' });
    assert.equal(readFileSync(join(root, 'unrelated.txt'), 'utf8'), 'untouched');
  } finally { cleanup(root); }
});

test('an existing workspace cannot redirect its stored researcher credentials to another backend', () => {
  const root = mkdtempSync(join(tmpdir(), 'petri-research-test-'));
  try {
    const state = join(root, 'state');
    loadOwner(state, 'http://127.0.0.1:4500');
    assert.throws(() => loadOwner(state, 'https://different.example'), { code: 'UPSTREAM_MISMATCH' });
    assert.equal(loadOwner(state).upstream, 'http://127.0.0.1:4500');
  } finally { cleanup(root); }
});

test('a workspace bound to the retired candidate API is rebound to the futures API', () => {
  const root = mkdtempSync(join(tmpdir(), 'petri-research-test-'));
  try {
    const state = join(root, 'state');
    loadOwner(state);
    const path = join(state, 'owner.json');
    const stored = JSON.parse(readFileSync(path, 'utf8'));
    writeFileSync(path, JSON.stringify({ ...stored, upstream: 'http://127.0.0.1:4184', oracleReadUrl: 'http://127.0.0.1:4184' }));
    const config = loadOwner(state, 'http://127.0.0.1:4500', undefined, 'http://127.0.0.1:4500');
    assert.equal(config.upstream, 'http://127.0.0.1:4500');
    assert.equal(config.oracleReadUrl, 'http://127.0.0.1:4500');
    assert.throws(() => loadOwner(state, 'https://different.example'), { code: 'UPSTREAM_MISMATCH' });
  } finally { cleanup(root); }
});

test('the production futures API is a workspace endpoint like any other, never rebound', () => {
  const root = mkdtempSync(join(tmpdir(), 'petri-research-test-'));
  try {
    const state = join(root, 'state');
    const created = loadOwner(state);
    assert.equal(created.upstream, 'https://api.amoeba.farm');
    assert.equal(created.oracleReadUrl, 'https://api.amoeba.farm');
    const config = loadOwner(state, 'https://api.amoeba.farm', undefined, 'https://api.amoeba.farm');
    assert.equal(config.upstream, 'https://api.amoeba.farm');
    assert.equal(config.oracleReadUrl, 'https://api.amoeba.farm');
    assert.throws(() => loadOwner(state, 'http://127.0.0.1:4500'), { code: 'UPSTREAM_MISMATCH' });
  } finally { cleanup(root); }
});

test('MCP supports initialization, bounded schemas and structured tools without owner or finance operations', async () => {
  const handler = createMcpHandler({ login: async () => ({ status: 'pending', userCode: 'CODE-TEST' }), listWork: async () => ({ work: [] }) });
  const send = (id, method, params) => handler({ jsonrpc: '2.0', id, method, params });
  assert.equal((await send(1, 'tools/list')).error.code, -32002);
  assert.equal((await send(2, 'initialize', { protocolVersion: '2025-06-18' })).result.protocolVersion, '2025-06-18');
  assert.equal(await handler({ jsonrpc: '2.0', method: 'notifications/initialized' }), null);
  const list = (await send(3, 'tools/list')).result.tools;
  assert.equal(list.length, 11); assert(!list.some(t => /approve|fund|withdraw|trade|owner/u.test(t.name)));
  const login = (await send(4, 'tools/call', { name: 'petri_login', arguments: {} })).result;
  assert.equal(login.structuredContent.status, 'pending'); assert.equal(login.isError, false);
  assert.equal((await send(5, 'tools/call', { name: 'petri_login', arguments: { ownerToken: 'no' } })).error.code, -32602);
  assert.equal((await send(6, 'tools/call', { name: 'petri_work_start', arguments: { workId: {}, clientKey: 'x' } })).error.code, -32602);
});

test('MCP tells the agent to use browser use for signed-in pages and takes a web proof bundle with a candidate', async () => {
  const submitted = [];
  const handler = createMcpHandler({ submit: async (...args) => { submitted.push(args); return { duplicate: false }; },
    next: async () => ({ status: 'idle' }) }, { agent: 'cedar' });
  const send = (id, method, params) => handler({ jsonrpc: '2.0', id, method, params });
  const { instructions } = (await send(1, 'initialize', { protocolVersion: '2025-06-18' })).result;
  for (const text of ['follow its nextStep', 'use browser use so the owner signs in themselves', 'petri evidence browser <url>', 'chrome-devtools-mcp --browserUrl',
    'Never ask for, type or store their password.', 'petri evidence check-manifest', 'petri evidence prove <JSON URL> --manifest <manifest> --browser', 'preview of what is disclosed', 'evidenceBundle']) {
    assert(instructions.includes(text), text);
  }
  assert.doesNotMatch(instructions, /login/iu);
  const tools = (await send(2, 'tools/list')).result.tools, tool = tools.find(t => t.name === 'petri_submit_candidate');
  assert.equal(tools.length, 11);
  assert.deepEqual(tool.inputSchema.required, ['runId', 'clientKey', 'candidate']);
  assert.equal(tool.inputSchema.properties.evidenceBundle.type, 'string');
  assert.match(tool.inputSchema.properties.evidenceBundle.description, /petri evidence prove wrote \(--out\).*at most 2 MiB/u);
  assert.match(tools.find(t => t.name === 'petri_work_next').description, /nextStep/u);
  // Every classic field (a series candidate is the next test's).
  const candidate = Object.fromEntries(Object.keys(tool.inputSchema.properties.candidate.properties).filter(key => key !== 'series').map(key => [key, 'x']));
  const call = args => send(3, 'tools/call', { name: 'petri_submit_candidate', arguments: { runId: 'r', clientKey: 'k', candidate, ...args } });
  assert.equal((await call({ evidenceBundle: '/tmp/bundle.json' })).result.isError, false);
  assert.equal((await call({})).result.isError, false);
  assert.deepEqual(submitted, [['r', candidate, 'k', '/tmp/bundle.json'], ['r', candidate, 'k', undefined]]);
  assert.equal((await call({ evidenceBundle: { path: 'x' } })).error.code, -32602);
  // petri_work_next carries the steps the CLI prints.
  assert.match((await send(4, 'tools/call', { name: 'petri_work_next', arguments: {} })).result.structuredContent.nextStep, /^Nothing needs research right now\./u);
});

test('an evidence upload sends the bundle\u2019s exact bytes and never echoes the API\u2019s refusal text', async () => {
  const seen = [];
  let status = 200;
  const api = http.createServer(async (request, response) => {
    const chunks = []; for await (const chunk of request) chunks.push(chunk);
    seen.push({ method: request.method, url: request.url, type: request.headers['content-type'], authorization: request.headers.authorization, body: Buffer.concat(chunks) });
    response.writeHead(status, { 'content-type': 'application/json' });
    response.end(JSON.stringify(status === 200 ? { ok: true, data: { bundleDigest: 'cd'.repeat(32), trust: 'development', stored: true, duplicate: false } }
      : { ok: false, code: 'EVIDENCE_UNVERIFIED', message: 'secret transport detail', verification: { stage: 'provenance' } }));
  });
  await new Promise(resolve => api.listen(0, '127.0.0.1', resolve));
  try {
    const upstream = createUpstream(`http://127.0.0.1:${api.address().port}`), credential = token();
    const bytes = Buffer.from('{\n  "format": "amoeba-evidence-bundle-v1",\n  "manifest": { "assume": { "unit": "1.0" } }\n}\n');
    assert.equal((await upstream.uploadEvidence(bytes, credential)).bundleDigest, 'cd'.repeat(32));
    assert.deepEqual([seen[0].method, seen[0].url, seen[0].type, seen[0].authorization], ['POST', '/oracle/evidence', 'application/json', `Bearer ${credential}`]);
    assert(seen[0].body.equals(bytes));
    status = 422;
    const failure = await upstream.uploadEvidence(bytes, credential).then(() => null, error => error);
    assert.equal(failure.code, 'EVIDENCE_REJECTED'); assert.equal(failure.status, 422);
    assert(!failure.message.includes('secret transport detail'));
    assert.match(failure.message, /petri evidence verify/u);
  } finally { await new Promise(resolve => api.close(resolve)); }
});

test('a near-2 MiB bundle goes from the agent\u2019s file through the loopback workspace to the API', { timeout: 30000 }, async () => {
  const root = mkdtempSync(join(tmpdir(), 'petri-research-test-'));
  const uploads = [], submissions = [];
  const upstream = { async register(payoutAuthority) { return { agent: { agentId: 'ef'.repeat(32), payoutAuthority }, credential: token() }; },
    async candidates() { return { candidates: [], complete: true }; },
    async uploadEvidence(bundle) { uploads.push(Buffer.from(bundle)); return { bundleDigest: '12'.repeat(32), trust: 'development' }; },
    async submit(details) { submissions.push(details); return { duplicate: false, candidate: { contentHash: '34'.repeat(32), agentId: 'ef'.repeat(32), details } }; } };
  const config = { directory: root, port: 0, ownerToken: token(), masterKey: token(), upstream: 'http://127.0.0.1:4500' };
  const server = await startServer(config, { upstream, clock: () => coverageNow, oracleReader: async () => coverageFixture() });
  try {
    const owner = (method, path, body) => createHttpClient(server.baseUrl)(method, `/v1/owner/${path}`, body, config.ownerToken);
    const client = createAgentClient({ name: 'cedar', harness: 'Codex', baseUrl: server.baseUrl, profileDirectory: join(root, 'profiles') });
    await client.login(); await owner('POST', 'settings', { payoutAuthority: payout });
    await owner('POST', 'pairings/approve', { pairingId: (await owner('GET', 'snapshot')).pairings[0].id });
    await client.login(); await owner('POST', 'discovery/refresh', {});
    const work = (await owner('GET', 'snapshot')).work[0], run = (await client.startWork(work.id, 'run-1')).run;
    const head = Buffer.from('{"format":"amoeba-evidence-bundle-v1","presentation":"'), tail = Buffer.from('"}\n');
    const bytes = Buffer.concat([head, Buffer.alloc(2 * 1024 * 1024 - head.length - tail.length, 'A'), tail]), file = join(root, 'bundle.json');
    writeFileSync(file, bytes);
    const candidate = { ...work.candidateScope, canonicalUrl: 'https://portal.distributor.example/api/prices', sourceType: 'distributor price list', productDefinition: '32GB DDR5-5600 kit',
      priceField: 'USD unit price', region: 'United States', currency: 'USD', evidence: 'Signed-in distributor price list, proven with a web proof.' };
    const result = await client.submit(run.id, candidate, 'proven', file);
    assert.equal(result.evidence.bundleDigest, '12'.repeat(32));
    assert.equal(uploads.length, 1); assert(uploads[0].equals(bytes));
    assert.deepEqual(submissions[0].authentication, { bundleDigest: '12'.repeat(32) });
    // One byte more is refused in the agent's own process, before any request.
    writeFileSync(file, Buffer.concat([bytes, Buffer.from(' ')]));
    await assert.rejects(client.submit(run.id, candidate, 'too-large', file), { code: 'BUNDLE_TOO_LARGE' });
    assert.equal(uploads.length, 1);
    // Other routes keep the ordinary 32 KiB bound.
    const tooLarge = await fetch(`${server.baseUrl}/v1/sessions`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ harness: 'a'.repeat(40000) }) });
    assert.equal(tooLarge.status, 413);
  } finally { await server.close(); cleanup(root); }
});

test('MCP takes a classic candidate with every field, or a series candidate with its scope, evidence and series', async () => {
  const submitted = [];
  const handler = createMcpHandler({ submit: async (runId, candidate) => { submitted.push(candidate); return { duplicate: false }; } });
  const send = (id, method, params) => handler({ jsonrpc: '2.0', id, method, params });
  await send(1, 'initialize', { protocolVersion: '2025-06-18' });
  const tool = (await send(2, 'tools/list')).result.tools.find(t => t.name === 'petri_submit_candidate');
  assert.equal(tool.inputSchema.properties.candidate.properties.series.type, 'object');
  const scope = { marketId: 'ramx', expiryId: 'RAMX-202612', bucketId: '44'.repeat(32), evidence: 'The contract table row' };
  const classic = { ...scope, canonicalUrl: 'https://example.com/p', sourceType: 'retailer listing', productDefinition: 'DDR5', priceField: 'price', region: 'US', currency: 'USD' };
  const series = { schema: 'amoeba.comparison-series.v1', bucket: 'DDR5_16GBIT', origin: { locator: 'https://example.com/p' } };
  const call = (id, candidate) => send(id, 'tools/call', { name: 'petri_submit_candidate', arguments: { runId: 'r', clientKey: `k${id}`, candidate } });
  assert.equal((await call(3, classic)).result.isError, false);
  assert.equal((await call(4, { ...scope, series })).result.isError, false);
  assert.deepEqual(submitted.at(-1).series, series);
  // Neither every field nor a series; a series that is not an object; no evidence.
  assert.equal((await call(5, { ...scope, canonicalUrl: 'https://example.com/p' })).error.code, -32602);
  assert.equal((await call(6, { ...scope, series: 'DDR5' })).error.code, -32602);
  const { evidence, ...noEvidence } = scope;
  assert.equal((await call(7, { ...noEvidence, series })).error.code, -32602);
  assert.equal(submitted.length, 2);
});

test('MCP framing rejects an oversized line and continues with the next request', async () => {
  let output = ''; const sink = new Writable({ write(chunk, encoding, callback) { output += chunk; callback(); } });
  const input = Readable.from(['x'.repeat(70000), '\n', JSON.stringify({ jsonrpc: '2.0', id: 7, method: 'ping' }), '\n']);
  await serveMcp({}, input, sink);
  const replies = output.trim().split('\n').map(JSON.parse);
  assert.equal(replies.length, 2); assert.equal(replies[0].error.code, -32600); assert.equal(replies[1].id, 7);
});

test('the launched MCP process pairs, resumes and completes a research run over HTTP', { timeout: 30000 }, async () => {
  const root = mkdtempSync(join(tmpdir(), 'petri-research-test-')), server = await start(root);
  const child = spawn(process.execPath, [fileURLToPath(new URL('../bin/petri.mjs', import.meta.url)), 'mcp', '--agent', 'mcp-researcher',
    '--harness', 'Custom DeepSeek harness', '--url', server.baseUrl, '--profile-dir', join(root, 'profiles')], { windowsHide: true, stdio: ['pipe','pipe','pipe'] });
  const replies = createInterface({ input: child.stdout })[Symbol.asyncIterator]();
  let id = 0, stderr = '';
  child.stderr.on('data', chunk => { stderr += chunk; });
  async function send(method, params) {
    const requestId = ++id;
    child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id: requestId, method, params })}\n`);
    const reply = await replies.next(); assert.equal(reply.done, false, stderr);
    const message = JSON.parse(reply.value); assert.equal(message.id, requestId); assert.equal(message.error, undefined);
    return message.result;
  }
  const call = async (name, args = {}) => { const result = await send('tools/call', { name, arguments: args }); assert.equal(result.isError, false, JSON.stringify(result)); return result.structuredContent; };
  try {
    await send('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'Petri integration test', version: '1' } });
    child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' })}\n`);
    assert.equal((await send('tools/list')).tools.length, 11);
    assert.equal((await call('petri_login')).status, 'pending');
    await server.owner('POST', 'settings', { payoutAuthority: payout });
    const snapshot = await server.owner('GET', 'snapshot');
    await server.owner('POST', 'pairings/approve', { pairingId: snapshot.pairings[0].id });
    assert.equal((await call('petri_login')).status, 'connected');
    await server.owner('POST', 'discovery/refresh', {});
    const work = (await server.owner('GET', 'snapshot')).work[0];
    assert.equal((await call('petri_work_list')).work[0].id, work.id);
    // The steps come with the run over MCP too, the sign-in path included.
    const next = await call('petri_work_next');
    assert.equal(next.assignment.id, work.id);
    assert.match(next.nextStep, /evidence browser <page URL>/u); assert.match(next.nextStep, /--evidence-bundle <bundle file>/u);
    await call('petri_run_finish', { runId: next.run.id, summary: 'Looked at the gap.' });
    const run = (await call('petri_work_start', { workId: work.id, clientKey: 'first-run' })).run;
    await call('petri_report_progress', { runId: run.id, clientKey: 'first-report', kind: 'note', message: 'Read the exact source.' });
    await call('petri_run_finish', { runId: run.id, summary: 'Research complete.' });
    const activity = await call('petri_activity');
    assert.equal(activity.events.at(-1).message, 'Research complete.');
    assert.deepEqual((await call('petri_rewards')).outcomes, []);
    await call('petri_session_close');
  } finally {
    child.stdin.end();
    if (child.exitCode === null) await new Promise(resolve => { child.once('exit', resolve); setTimeout(() => child.kill(), 2000).unref(); });
    await server.close(); cleanup(root);
  }
});
