// The owner's live feed of an agent's work: what it reports as it goes
// (searches, pages, values, petri writes), the owner's directives, and the
// snapshot the TUI reads.
import test from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve, dirname } from 'node:path';
import { execFile, spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';
import { createApplication } from '../service/application.mjs';
import { REPORT_KINDS } from '../service/work-store.mjs';
import { token, privateDirectory, privateWrite } from '../service/security.mjs';
import { createMcpHandler } from '../client/mcp.mjs';
import { coverageFixture, coverageNow } from './coverage-fixture.mjs';

const payout = 'So11111111111111111111111111111111111111112';
const page = 'https://example.com/ram';

function workspace() {
  let now = coverageNow;
  const ownerToken = token();
  const upstream = { async register(payoutAuthority) { return { agent: { agentId: token(), payoutAuthority }, credential: token() }; },
    async candidates() { return { candidates: [], complete: true }; } };
  const app = createApplication({ ownerToken, masterKey: token(), directory: '.', upstream: 'http://127.0.0.1:4500' },
    { databasePath: ':memory:', upstream, clock: () => now, oracleReader: async () => coverageFixture(now) });
  const request = (method, path, body = {}, credential) => app.handle({ method, path, body, credential });
  const owner = (method, path, body = {}) => request(method, `/v1/owner/${path}`, body, ownerToken);
  const snapshot = agentId => app.handle({ method: 'GET', path: '/v1/owner/snapshot', query: new URLSearchParams({ agentId }), credential: ownerToken });
  async function connect(name) {
    await owner('POST', 'settings', { payoutAuthority: payout });
    const pairing = await request('POST', '/v1/login', { name, harness: 'Codex' });
    await owner('POST', 'pairings/approve', { pairingId: pairing.pairingId });
    const result = await request('POST', '/v1/login/poll', { pairingId: pairing.pairingId, secret: pairing.secret });
    const call = (method, path, body = {}) => request(method, path, body, result.credential);
    return { ...result, call, session: (await call('POST', '/v1/sessions', {})).session };
  }
  async function discover() { await owner('POST', 'discovery/refresh'); return (await owner('GET', 'snapshot')).work[0]; }
  return { owner, snapshot, connect, discover, time: value => now += value, now: () => now, close: () => app.close() };
}

test('an agent reports its searches, pages, values and petri writes; malformed reports are refused', async () => {
  const f = workspace(); try {
    const a = await f.connect('cedar'), work = await f.discover();
    const run = (await a.call('POST', '/v1/work/next', { sessionId: a.session.id })).run;
    let key = 0;
    const report = (kind, message, referenceUrl) => a.call('POST', `/v1/runs/${run.id}/events`,
      { sessionId: a.session.id, clientKey: `step-${++key}`, kind, message, ...(referenceUrl ? { referenceUrl } : {}) });
    const steps = [['search', 'DDR5 RDIMM 64GB price'], ['read', 'Crucial 64GB DDR5-4800 RDIMM', page],
      ['value', '289.99 USD, DDR5 RDIMM 64GB', page], ['value', '289.99 USD, DDR5 RDIMM 64GB'], ['submit', 'update committed'],
      ['submit', 'sponsored 30d7e0f9…', 'https://example.com/tx'], ['note', 'Comparing two listings.'],
      ['source_found', 'DDR5 RDIMM 64GB at 289.99 USD', page], ['blocked', 'The page needs a login.']];
    for (const [kind, message, reference] of steps) {
      const { event } = await report(kind, message, reference);
      assert.deepEqual([event.actor, event.kind, event.message, event.referenceUrl], ['agent', kind, message, reference ?? null]);
    }
    // A read names its page; references stay HTTPS without credentials.
    await assert.rejects(report('read', 'No page given'), { code: 'INVALID_REFERENCE' });
    await assert.rejects(report('read', 'Plain HTTP', 'http://example.com/ram'), { code: 'INVALID_REFERENCE' });
    await assert.rejects(report('value', 'With a password', 'https://user:pw@example.com/ram'), { code: 'INVALID_REFERENCE' });
    // A page read in the owner's signed-in browser is a reference like any
    // other, as long as its URL carries no token, key or session id.
    const portal = 'https://portal.distributor.example/account/prices?region=us';
    assert.equal((await report('read', 'Distributor X price list (signed in)', portal)).event.referenceUrl, portal);
    for (const leak of ['https://portal.distributor.example/prices?token=abc', 'https://portal.distributor.example/prices?access_token=abc',
      'https://portal.distributor.example/prices;jsessionid=ABC123', 'https://portal.distributor.example/prices#id_token=abc',
      'https://files.example.com/prices.json?X-Amz-Signature=abc', 'https://portal.distributor.example/prices?API_KEY=abc',
      'https://portal.distributor.example/prices?p=%74oken%3Dabc']) {
      await assert.rejects(report('read', 'Leaky link', leak), { code: 'INVALID_REFERENCE' }, leak);
    }
    // Only the owner, petri and the backend write their own kinds.
    for (const kind of ['directive', 'run_started', 'run_finished', 'candidate_received', 'Search', 'page']) await assert.rejects(report(kind, 'Forged'), { code: 'INVALID_EVENT' });
    // The 4,000-character limit stays as it was.
    await assert.rejects(report('search', 'x'.repeat(4001)), { code: 'INVALID_INPUT' });
    assert.equal((await report('search', 'x'.repeat(4000))).event.kind, 'search');

    // The snapshot: the agent's events oldest first, with its runs and who is on what.
    const snapshot = await f.snapshot(a.agent.id), { events, runs } = snapshot.selected;
    assert.deepEqual(events.map(e => [e.actor, e.kind]), [['petri', 'run_started'], ...steps.map(([kind]) => ['agent', kind]), ['agent', 'read'], ['agent', 'search']]);
    assert.deepEqual(events.map(e => e.sequence), events.map(e => e.sequence).sort((x, y) => x - y));
    for (const event of events) {
      assert.deepEqual(Object.keys(event).sort(), ['actor', 'createdAt', 'kind', 'message', 'referenceUrl', 'runId', 'sequence']);
      assert.equal(event.runId, run.id); assert.equal(event.createdAt, coverageNow);
    }
    assert.equal(events[2].referenceUrl, page);
    assert.deepEqual([runs[0].id, runs[0].workId, runs[0].createdAt, runs[0].finishedAt], [run.id, work.id, coverageNow, null]);
    assert.deepEqual(snapshot.activity, [{ agentId: a.agent.id, workId: work.id, runId: run.id, title: work.title,
      startedAt: coverageNow, lastSeenAt: coverageNow, lastMessage: 'x'.repeat(4000) }]);
  } finally { f.close(); }
});

test('the owner’s directive shows in the feed of each agent on the work as it is set, and opens later runs', async () => {
  const f = workspace(); try {
    const a = await f.connect('cedar'), b = await f.connect('flint'), c = await f.connect('birch'), work = await f.discover();
    const feed = async agent => (await f.snapshot(agent.agent.id)).selected.events;
    const ar = (await a.call('POST', '/v1/runs', { workId: work.id, sessionId: a.session.id, clientKey: 'a-1' })).run;
    const cr = (await c.call('POST', '/v1/runs', { workId: work.id, sessionId: c.session.id, clientKey: 'c-1' })).run;
    await c.call('POST', `/v1/runs/${cr.id}/finish`, { sessionId: c.session.id, summary: 'Done.' });
    await a.call('POST', `/v1/runs/${ar.id}/events`, { sessionId: a.session.id, clientKey: 's', kind: 'search', message: 'DDR5 RDIMM 64GB' });
    f.time(1000);
    await f.owner('POST', 'work/refine', { workId: work.id, instruction: 'Try Newegg first.' });
    // a is on it: the directive lands in its run's feed, at the time it was set.
    const directive = (await feed(a)).at(-1);
    assert.deepEqual(directive, { sequence: directive.sequence, runId: ar.id, actor: 'owner', kind: 'directive',
      message: 'Try Newegg first.', referenceUrl: null, createdAt: coverageNow + 1000 });
    // The agent sees it too; its last report stays its own.
    assert.equal((await a.call('GET', '/v1/activity')).events.at(-1).kind, 'directive');
    assert.equal((await f.owner('GET', 'snapshot')).activity.find(run => run.agentId === a.agent.id).lastMessage, 'DDR5 RDIMM 64GB');
    // c had finished: nothing new in its feed.
    assert(!(await feed(c)).some(event => event.actor === 'owner'));
    // b starts it later: its run opens with the directive.
    f.time(1000);
    const next = await b.call('POST', '/v1/work/next', { sessionId: b.session.id });
    assert.equal(next.assignment.id, work.id); assert.equal(next.assignment.guidance.instruction, 'Try Newegg first.');
    assert.deepEqual((await feed(b)).map(e => [e.runId, e.actor, e.kind, e.message, e.createdAt]),
      [[next.run.id, 'petri', 'run_started', `Started: ${work.title}`, coverageNow + 2000], [next.run.id, 'owner', 'directive', 'Try Newegg first.', coverageNow + 2000]]);
    // A changed or cleared directive reaches both open runs; a run started after clearing opens without one.
    f.time(1000);
    await f.owner('POST', 'work/refine', { workId: work.id, instruction: 'Newegg is out of stock: try B&H.' });
    await f.owner('POST', 'work/refine', { workId: work.id, instruction: '' });
    for (const agent of [a, b]) assert.deepEqual((await feed(agent)).slice(-2).map(e => [e.actor, e.kind, e.message, e.createdAt]),
      [['owner', 'directive', 'Newegg is out of stock: try B&H.', coverageNow + 3000], ['owner', 'directive', 'Directive cleared.', coverageNow + 3000]]);
    const later = (await c.call('POST', '/v1/runs', { workId: work.id, sessionId: c.session.id, clientKey: 'c-2' })).run;
    assert.deepEqual((await feed(c)).filter(e => e.runId === later.id).map(e => e.kind), ['run_started']);
    // Only the owner refines.
    await assert.rejects(a.call('POST', '/v1/owner/work/refine', { workId: work.id, instruction: 'No.' }), { code: 'OWNER_REQUIRED' });
  } finally { f.close(); }
});

test('the MCP report tool offers every report kind and refuses others', async () => {
  const reports = [];
  const handler = createMcpHandler({ report: async (runId, args) => { reports.push([runId, args.kind, args.referenceUrl]); return { event: { kind: args.kind } }; } });
  const send = (id, method, params) => handler({ jsonrpc: '2.0', id, method, params });
  await send(1, 'initialize', { protocolVersion: '2025-06-18' });
  const tool = (await send(2, 'tools/list')).result.tools.find(t => t.name === 'petri_report_progress');
  assert.deepEqual(tool.inputSchema.properties.kind.enum, ['search', 'read', 'value', 'submit', 'note', 'source_found', 'blocked']);
  assert.deepEqual(tool.inputSchema.properties.kind.enum, REPORT_KINDS);
  for (const kind of ['search', 'read', 'value', 'submit']) assert.match(tool.description, new RegExp(`\\b${kind} \\(`, 'u'));
  const args = kind => ({ runId: 'r', clientKey: kind, kind, message: 'One line.', referenceUrl: page });
  assert.equal((await send(3, 'tools/call', { name: 'petri_report_progress', arguments: args('read') })).result.isError, false);
  assert.deepEqual(reports, [['r', 'read', page]]);
  assert.equal((await send(4, 'tools/call', { name: 'petri_report_progress', arguments: args('directive') })).error.code, -32602);
});

test('work next tells the agent to report each step and how to prove a signed-in page, the same over the CLI and MCP', { timeout: 30000 }, async () => {
  const root = mkdtempSync(join(tmpdir(), 'petri-research-test-')), run = '0f1e2d3c-4b5a-4968-8776-655443322110';
  let assignment;
  // A workspace that hands out one assignment: work next prints its steps.
  const server = http.createServer(async (request, response) => {
    for await (const _ of request);
    response.setHeader('content-type', 'application/json');
    const data = request.url === '/v1/work/next' ? { status: 'assigned', resumed: false, run: { id: run }, assignment, petri: 'petri' } : { session: { id: 'session-1' } };
    response.end(JSON.stringify({ ok: true, data }));
  });
  await new Promise(done => server.listen(0, '127.0.0.1', done));
  const bin = fileURLToPath(new URL('../bin/petri.mjs', import.meta.url));
  let mcp;
  try {
    const baseUrl = `http://127.0.0.1:${server.address().port}`, profiles = privateDirectory(join(root, 'profiles'));
    privateWrite(join(profiles, 'cedar.json'), { version: 1, name: 'cedar', baseUrl, credential: token(), sessionId: 'session-1' });
    // The MCP server of the same agent: petri_work_next returns the same steps.
    mcp = spawn(process.execPath, [bin, 'mcp', '--agent', 'cedar', '--url', baseUrl, '--profile-dir', profiles], { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
    const replies = createInterface({ input: mcp.stdout })[Symbol.asyncIterator]();
    let id = 0;
    const send = async (method, params) => { mcp.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id: ++id, method, params })}\n`); return JSON.parse((await replies.next()).value).result; };
    await send('initialize', { protocolVersion: '2025-06-18' });
    for (const [kind, chip] of ['source', 'update', 'opening'].flatMap(kind => [[kind, false], [kind, true]])) {
      assignment = { id: 'work-1', title: 'RAMX · DDR5 RDIMM 64GB', candidateScope: { marketId: 'ramx', expiryId: 'RAMX-202612' },
        objective: { kind, expiryId: 'RAMX-202612', url: page, sourceAddress: 'source-address', bucketLabel: chip ? 'DDR5_16GBIT' : 'DDR5 RDIMM 64GB',
          ...(chip ? { chip, bucketName: 'DDR5 16Gb', seriesId: 'example-ddr5-16gb' } : {}), openingBondAtomic: '1000000' } };
      const { stdout } = await promisify(execFile)(process.execPath, [bin,
        'work', 'next', '--agent', 'cedar', '--url', baseUrl, '--profile-dir', profiles], { windowsHide: true, timeout: 20000 });
      const nextStep = JSON.parse(stdout).data.nextStep, lines = nextStep.split('\n');
      const say = report => lines.find(line => line.includes(`work report ${run} `) && line.includes(` --kind ${report} `));
      for (const report of ['search', 'read', 'value', 'submit']) assert(say(report), `${kind}: ${report}`);
      assert.match(say('search'), /--message "<query>"$/u);
      assert.match(say('read'), /--source <page URL> --message/u);
      assert.match(say('submit'), /--message "<what it did, e\.g\. [^>]+>"$/u);
      if (kind === 'source') assert(say('source_found'));
      // A page behind a sign-in: browser use, the owner signs in, a web proof.
      const line = text => lines.find(item => item.includes(text));
      const target = kind === 'source' ? '<page URL>' : page;
      assert(line(`- Open it with browser use: petri evidence browser ${target}. It prints a DevTools address`), kind);
      assert(line('chrome-devtools-mcp --browserUrl, Playwright connectOverCDP'), kind);
      assert(line('- Ask the owner to sign in there themselves. Never ask for, type or store their password.'), kind);
      assert(line('"format":"amoeba-extraction-manifest-v1"'), kind);
      assert(line('petri evidence check-manifest <manifest file> --body <saved response>'), kind);
      assert(line(`petri evidence prove ${kind === 'source' ? '<JSON URL>' : page} --manifest <manifest file> --browser --out <bundle file> --json`), kind);
      assert(line('- Show the owner the preview it prints (what is disclosed)'), kind);
      if (kind === 'source' && !chip) {
        assert(line(`work submit ${run} `).includes('--file <file>'));
        assert.match(line('--evidence-bundle'), /work submit 0f1e2d3c-[^ ]+ --agent 'cedar' --profile-dir '[^']+' --file <file> --evidence-bundle <bundle file>, with the JSON URL as canonicalUrl\./u);
        assert(line('Only a catalog_price or published_benchmark can be a source'));
      } else if (kind === 'source') {
        // A series' page is its origin.locator: the proven JSON URL goes there.
        assert.match(line('--evidence-bundle'), /Put the JSON URL in the series' origin\.locator, check it again, and submit it with the bundle: .*work submit 0f1e2d3c-[^ ]+ --agent 'cedar' --profile-dir '[^']+' --file <file> --evidence-bundle <bundle file>\./u);
      } else if (!chip) {
        assert(line('--evidence <bundle file> in place of --archive-url'), kind);
        assert(lines.indexOf(line('instead of step 2')) > lines.indexOf(line('2. Archive the page')), kind);
        assert(lines.indexOf(line('--evidence <bundle file> in place of')) < lines.indexOf(lines.find(item => item.startsWith('3. '))), kind);
      } else {
        // An observation cites the proof in its evidenceReference instead of the archive.
        assert(line('Put the bundle file in evidenceReference in place of the archive URL'), kind);
        assert(lines.indexOf(line('instead of step 3')) > lines.indexOf(line('3. Archive the page')), kind);
        assert(lines.indexOf(line('Put the bundle file in evidenceReference')) < lines.indexOf(lines.find(item => item.startsWith('4. '))), kind);
      }
      // Plain words for the agent: "sign in", never "login".
      assert.doesNotMatch(nextStep, /login/iu);
      const result = await send('tools/call', { name: 'petri_work_next', arguments: {} });
      assert.equal(result.isError, false); assert.equal(result.structuredContent.nextStep, nextStep, kind);
      // A chip month's sources are series, its prints observations, through petri's own checks.
      const has = text => lines.some(item => item.includes(text));
      if (chip && kind === 'source') {
        assert(has('petri oracle series template --bucket DDR5_16GBIT --out series.json'), nextStep);
        assert(has('petri oracle series check series.json RAMX-202612') && has('series: the checked series object') && has('origin.row'));
      } else if (chip) {
        assert(has('petri oracle observe template RAMX-202612 --source source-address --out obs.json'), nextStep);
        assert(has('petri oracle observe RAMX-202612 --source source-address --file obs.json --yes') && has('evidenceReference'));
        assert(has(kind === 'update' ? 'petri oracle updates reveal <CLAIM> --yes' : 'petri oracle prints finalize RAMX-202612'));
        assert(!has('petri oracle prints opening') && !has('petri oracle updates commit'));
      } else assert(!has('oracle series') && !has('oracle observe'));
    }
  } finally {
    if (mcp) { mcp.stdin.end(); if (mcp.exitCode === null) await new Promise(done => { mcp.once('exit', done); setTimeout(() => mcp.kill(), 2000).unref(); }); }
    await new Promise(done => server.close(done));
    const exact = resolve(root); assert.equal(dirname(exact), resolve(tmpdir())); rmSync(exact, { recursive: true, force: true });
  }
});

test('work submit --evidence-bundle sends the bundle file\u2019s bytes with the candidate, and refuses one over 2 MiB', { timeout: 30000 }, async () => {
  const root = mkdtempSync(join(tmpdir(), 'petri-research-test-')), run = '0f1e2d3c-4b5a-4968-8776-655443322110';
  const bodies = [];
  const server = http.createServer(async (request, response) => {
    const chunks = []; for await (const chunk of request) chunks.push(chunk);
    if (request.url.endsWith('/candidates')) bodies.push(JSON.parse(Buffer.concat(chunks)));
    response.setHeader('content-type', 'application/json');
    response.end(JSON.stringify({ ok: true, data: request.url.endsWith('/candidates') ? { duplicate: false, evidence: { bundleDigest: 'ab'.repeat(32) } } : { session: { id: 'session-1' } } }));
  });
  await new Promise(done => server.listen(0, '127.0.0.1', done));
  try {
    const baseUrl = `http://127.0.0.1:${server.address().port}`, profiles = privateDirectory(join(root, 'profiles'));
    privateWrite(join(profiles, 'cedar.json'), { version: 1, name: 'cedar', baseUrl, credential: token(), sessionId: 'session-1' });
    const candidate = join(root, 'candidate.json'), bundle = join(root, 'bundle.json'), large = join(root, 'large.json');
    writeFileSync(candidate, JSON.stringify({ marketId: 'ramx', canonicalUrl: 'https://portal.distributor.example/api/prices' }));
    const bytes = Buffer.from('{\n  "format": "amoeba-evidence-bundle-v1",\n  "presentation": "AAAA"\n}\n');
    writeFileSync(bundle, bytes);
    writeFileSync(large, Buffer.concat([Buffer.from('{"format":"amoeba-evidence-bundle-v1","presentation":"'), Buffer.alloc(2 * 1024 * 1024, 'A'), Buffer.from('"}')]));
    const submit = file => promisify(execFile)(process.execPath, [fileURLToPath(new URL('../bin/petri.mjs', import.meta.url)), 'work', 'submit', run,
      '--agent', 'cedar', '--url', baseUrl, '--profile-dir', profiles, '--key', 'proven', '--file', candidate, '--evidence-bundle', file], { windowsHide: true, timeout: 20000 });
    const { stdout } = await submit(bundle);
    assert.equal(JSON.parse(stdout).data.evidence.bundleDigest, 'ab'.repeat(32));
    assert.deepEqual(Object.keys(bodies[0]).sort(), ['candidate', 'clientKey', 'evidenceBundle', 'sessionId']);
    assert(Buffer.from(bodies[0].evidenceBundle, 'base64').equals(bytes));
    for (const [file, code] of [[large, 'BUNDLE_TOO_LARGE'], [candidate, 'INVALID_BUNDLE'], [join(root, 'missing.json'), 'BUNDLE_NOT_FOUND']]) {
      const failure = await submit(file).then(() => null, error => error);
      assert(failure, file); assert.equal(JSON.parse(failure.stderr).error.code, code);
    }
    assert.equal(bodies.length, 1);
  } finally {
    await new Promise(done => server.close(done));
    const exact = resolve(root); assert.equal(dirname(exact), resolve(tmpdir())); rmSync(exact, { recursive: true, force: true });
  }
});
