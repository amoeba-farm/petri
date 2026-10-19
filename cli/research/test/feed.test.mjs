// The owner's live feed of an agent's work: what it reports as it goes
// (searches, pages, values, petri writes), the owner's directives, and the
// snapshot the TUI reads.
import test from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve, dirname } from 'node:path';
import { execFile } from 'node:child_process';
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
    // A read names its page; references stay public HTTPS without credentials.
    await assert.rejects(report('read', 'No page given'), { code: 'INVALID_REFERENCE' });
    await assert.rejects(report('read', 'Plain HTTP', 'http://example.com/ram'), { code: 'INVALID_REFERENCE' });
    await assert.rejects(report('value', 'With a password', 'https://user:pw@example.com/ram'), { code: 'INVALID_REFERENCE' });
    // Only the owner, petri and the backend write their own kinds.
    for (const kind of ['directive', 'run_started', 'run_finished', 'candidate_received', 'Search', 'page']) await assert.rejects(report(kind, 'Forged'), { code: 'INVALID_EVENT' });
    // The 4,000-character limit stays as it was.
    await assert.rejects(report('search', 'x'.repeat(4001)), { code: 'INVALID_INPUT' });
    assert.equal((await report('search', 'x'.repeat(4000))).event.kind, 'search');

    // The snapshot: the agent's events oldest first, with its runs and who is on what.
    const snapshot = await f.snapshot(a.agent.id), { events, runs } = snapshot.selected;
    assert.deepEqual(events.map(e => [e.actor, e.kind]), [['petri', 'run_started'], ...steps.map(([kind]) => ['agent', kind]), ['agent', 'search']]);
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

test('work next tells the agent to report each search, page, value and petri write, for every kind of work', { timeout: 30000 }, async () => {
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
  try {
    const baseUrl = `http://127.0.0.1:${server.address().port}`, profiles = privateDirectory(join(root, 'profiles'));
    privateWrite(join(profiles, 'cedar.json'), { version: 1, name: 'cedar', baseUrl, credential: token(), sessionId: 'session-1' });
    for (const kind of ['source', 'update', 'opening']) {
      assignment = { id: 'work-1', title: 'RAMX · DDR5 RDIMM 64GB', candidateScope: { marketId: 'ramx', expiryId: 'RAMX-202612' },
        objective: { kind, expiryId: 'RAMX-202612', url: page, sourceAddress: 'source-address', bucketLabel: 'DDR5 RDIMM 64GB', openingBondAtomic: '1000000' } };
      const { stdout } = await promisify(execFile)(process.execPath, [fileURLToPath(new URL('../bin/petri.mjs', import.meta.url)),
        'work', 'next', '--agent', 'cedar', '--url', baseUrl, '--profile-dir', profiles], { windowsHide: true, timeout: 20000 });
      const lines = JSON.parse(stdout).data.nextStep.split('\n');
      const say = report => lines.find(line => line.includes(`work report ${run} `) && line.includes(` --kind ${report} `));
      for (const report of ['search', 'read', 'value', 'submit']) assert(say(report), `${kind}: ${report}`);
      assert.match(say('search'), /--message "<query>"$/u);
      assert.match(say('read'), /--source <page URL> --message/u);
      assert.match(say('submit'), /--message "<what it did, e\.g\. [^>]+>"$/u);
      if (kind === 'source') assert(say('source_found'));
    }
  } finally {
    await new Promise(done => server.close(done));
    const exact = resolve(root); assert.equal(dirname(exact), resolve(tmpdir())); rmSync(exact, { recursive: true, force: true });
  }
});
