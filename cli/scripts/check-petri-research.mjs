// Offline binary smoke: disposable workspace, loopback oracle, no installer,
// wallet, external API, submission, or user-owned configuration.
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { mkdtempSync, readFileSync, rmSync, existsSync } from 'node:fs';
import http from 'node:http';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';

const execute = promisify(execFile);
const binary = path.resolve(process.argv[2] || `target/debug/petri${process.platform === 'win32' ? '.exe' : ''}`);
const root = mkdtempSync(path.join(os.tmpdir(), 'petri-research-binary-'));
const requests = [];
const oracle = http.createServer((request, response) => {
  requests.push(`${request.method} ${request.url}`);
  response.setHeader('Content-Type', 'application/json');
  response.end(JSON.stringify({ ok: true, data: { markets: [], series: [] } }));
});
await new Promise(resolve => oracle.listen(0, '127.0.0.1', resolve));
const address = `http://127.0.0.1:${oracle.address().port}`;
const reservation = net.createServer();
await new Promise(resolve => reservation.listen(0, '127.0.0.1', resolve));
const port = reservation.address().port;
await new Promise(resolve => reservation.close(resolve));
const env = { ...process.env, PETRI_CONFIG_PATH: path.join(root, 'config', 'petri.toml'),
  PETRI_RESEARCH_HOME: path.join(root, 'state'), PETRI_RESEARCH_UPSTREAM: address,
  PETRI_RESEARCH_ORACLE_URL: address, PETRI_RESEARCH_PORT: String(port),
  PETRI_RESEARCH_NODE: process.execPath, PETRI_UPDATE_CHECK: '0',
  PETRI_API_URL: address, NODE_OPTIONS: '', NODE_PATH: '' };
const run = async args => {
  console.log(`Checking petri ${args.join(' ')}`);
  const result = await execute(binary, args, { env, windowsHide: true, timeout: 20000, maxBuffer: 2 * 1024 * 1024 });
  return result;
};
try {
  const help = (await run(['--help'])).stdout;
  for (const command of ['markets', 'long', 'positions', 'tui', 'research']) assert(help.includes(command), command);
  const tuiHelp = (await run(['tui', '--help'])).stdout;
  assert.match(tuiHelp, /oracle research/i);
  assert.doesNotMatch(tuiHelp, /Lab Bench|\[MARKET\]/);
  await assert.rejects(run(['tui', 'ramx']));
  assert.match((await run(['research', '--help'])).stdout, /research work submit/);
  const view = (await run(['tui', '--no-update-check', '--render', '100x32', '--view', 'connect'])).stdout;
  for (const label of ['Connect', 'Activity', 'Evidence', 'Rewards']) assert(view.toLowerCase().includes(label.toLowerCase()), label);
  assert.doesNotMatch(view, /Lab Bench|Wallet Ledger|Buy\/Sell/);
  const snapshot = JSON.parse((await run(['--json', 'tui'])).stdout);
  assert.equal(snapshot.ok, true);
  assert.deepEqual(snapshot.data.agents, []);
  assert(!JSON.stringify(snapshot).includes('ownerToken'));
  const runtime = path.join(root, 'config', 'research-runtime');
  const current = JSON.parse(readFileSync(path.join(runtime, 'current.json'), 'utf8'));
  assert.match(current.bundleId, /^[a-f0-9]{64}$/);
  const agent = await execute(process.execPath, [path.join(runtime, 'agent.mjs'), '--help'],
    { env, windowsHide: true, timeout: 10000 });
  assert.match(agent.stdout, /research work submit/);
  assert(requests.every(request => ['GET /v1/oracle/catalog', 'GET /v1/oracle/public-snapshot'].includes(request)));
  console.log('Petri research binary smoke passed: CLI retained, oracle views rendered, private bootstrap and stable agent launcher work.');
} finally {
  const ownerFile = path.join(root, 'state', 'owner.json');
  if (existsSync(ownerFile)) {
    const config = JSON.parse(readFileSync(ownerFile, 'utf8'));
    for (let attempt = 0; attempt < 20; attempt++) {
      const response = await fetch(`http://127.0.0.1:${port}/v1/owner/restart`, { method: 'POST',
        headers: { 'content-type': 'application/json', authorization: `Bearer ${config.ownerToken}` },
        body: '{}', signal: AbortSignal.timeout(2000) }).catch(() => undefined);
      if (!response) break;
      const result = await response.json();
      if (response.ok) break;
      if (result.error?.code !== 'SERVICE_BUSY') throw new Error(`Test service cleanup failed: ${result.error?.code}`);
      await delay(100);
    }
  }
  oracle.closeAllConnections();
  await new Promise(resolve => oracle.close(resolve));
  const exact = path.resolve(root);
  assert.equal(path.dirname(exact), path.resolve(os.tmpdir()));
  assert(path.basename(exact).startsWith('petri-research-binary-'));
  rmSync(exact, { recursive: true, force: true, maxRetries: 20, retryDelay: 100 });
}
