#!/usr/bin/env node
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { ensureService } from '../service/lifecycle.mjs';
import { setTimeout as delay } from 'node:timers/promises';
import { createAgentClient } from '../client/agent.mjs';
import { createHttpClient } from '../client/http.mjs';
import { serveMcp } from '../client/mcp.mjs';
import { loadOwner, stateDirectory, fail } from '../service/security.mjs';

function parse(argv) {
  const flags = {}, words = [];
  const boolean = new Set(['json', 'help', 'wait']);
  const allowed = new Set(['state-dir','upstream','oracle-url','port','agent','name','harness','url','profile-dir','file','key','message','kind','source','summary']);
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (!arg.startsWith('--')) { words.push(arg); continue; }
    const key = arg.slice(2);
    if (boolean.has(key)) flags[key] = true;
    else if (allowed.has(key) && argv[i + 1] && !argv[i + 1].startsWith('--')) flags[key] = argv[++i];
    else throw fail('INVALID_OPTION', `Unknown option or missing value: ${arg}`);
  }
  return { flags, words };
}
const help = `Petri — oracle research and data submission

  petri tui                             Open the clickable TUI
  petri research serve                   Run the local workspace service
  petri research agent login --name cedar --harness codex
  petri research agent status --name cedar
  petri research work list --agent cedar
  petri research work start <work-id> --agent cedar --key <request-key>
  petri research work report <run-id> --agent cedar --message <text>
  petri research work submit <run-id> --agent cedar --file candidate.json
  petri research work finish <run-id> --agent cedar --summary <text>
  petri research rewards --agent cedar
  petri research mcp --agent cedar --harness codex

Owner controls (also available in the TUI):
  petri research owner address <public-solana-address>
  petri research owner directive save --file directive.json
  petri research owner discover
  petri research owner refine <work-id> --message <research-guidance>
  petri research owner approve <pairing-id>
  petri research owner reject <pairing-id>
  petri research owner revoke <agent-id>

Options: --json, --state-dir <path>, --profile-dir <path>, --url <workspace-url>
Service options: --upstream <candidate-api-origin>, --oracle-url <oracle-read-origin>, --port <loopback-port>
First login returns a pairing code; approve it in Connect, then log in again.
Use --wait to wait for that approval. No model API key or wallet key is needed.
`;
function output(value, json = true) {
  // All command results are machine-readable; --json is accepted explicitly.
  process.stdout.write(`${JSON.stringify({ ok: true, data: value }, null, json ? 2 : 2)}\n`);
}
function jsonFile(path) {
  if (!path) throw fail('FILE_REQUIRED', 'Provide --file with a JSON file.');
  const value = readFileSync(resolve(path));
  if (value.length > 32768) throw fail('FILE_TOO_LARGE', 'The JSON file is too large.');
  return JSON.parse(value.toString('utf8'));
}
async function main() {
  const { flags, words } = parse(process.argv.slice(2));
  if (flags.help || words[0] === 'help') { process.stdout.write(help); return; }
  const command = words[0] || 'help';
  if (command === 'help') { process.stdout.write(help); return; }
  if (command === 'tui') throw fail('INVALID_COMMAND', 'Open the workspace with petri tui.');
  if (['serve','owner'].includes(command)) {
    const port = flags.port || process.env.PETRI_RESEARCH_PORT;
    const config = loadOwner(flags['state-dir'] || stateDirectory(), flags.upstream || process.env.PETRI_RESEARCH_UPSTREAM,
      port ? Number(port) : undefined, flags['oracle-url'] || process.env.PETRI_RESEARCH_ORACLE_URL);
    if (command === 'serve') {
      const { startServer } = await import('../service/server.mjs');
      const service = await startServer(config);
      process.stdout.write(`Petri research workspace: ${service.baseUrl}\n`);
      let closing = false;
      const close = async () => { if (closing) return; closing = true; await service.close(); };
      process.once('SIGINT', close); process.once('SIGTERM', close); return;
    }
    await ensureService(config);
    const request = createHttpClient(config.baseUrl), [_, action, id] = words;
    let value;
    if (action === 'address') value = await request('POST', '/v1/owner/settings', { payoutAuthority: id }, config.ownerToken);
    else if (action === 'directive' && id === 'save') value = await request('POST', '/v1/owner/directives', jsonFile(flags.file), config.ownerToken);
    else if (action === 'discover') value = await request('POST', '/v1/owner/discovery/refresh', {}, config.ownerToken);
    else if (action === 'refine') value = await request('POST', '/v1/owner/work/refine', { workId: id, instruction: flags.message }, config.ownerToken);
    else if (action === 'approve' || action === 'reject') value = await request('POST', `/v1/owner/pairings/${action}`, { pairingId: id }, config.ownerToken);
    else if (action === 'revoke') value = await request('POST', '/v1/owner/agents/revoke', { agentId: id }, config.ownerToken);
    else if (action === 'snapshot') value = await request('GET', '/v1/owner/snapshot', undefined, config.ownerToken);
    else throw fail('INVALID_COMMAND', 'Unknown owner command. Use --help.');
    output(value); return;
  }
  const client = createAgentClient({ name: flags.agent || flags.name || 'researcher', harness: flags.harness || 'Custom harness',
    baseUrl: flags.url, profileDirectory: flags['profile-dir'] });
  if (command === 'mcp') { await serveMcp(client); return; }
  let value;
  if (command === 'agent' && words[1] === 'login') {
    value = await client.login();
    if (flags.wait && value.status !== 'connected') {
      process.stderr.write(`Approve code ${value.userCode} in Petri’s Connect view.\n`);
      while (value.status !== 'connected' && !['rejected','revoked'].includes(value.status) && Date.now() < value.expiresAt) { await delay(2000); value = await client.login(); }
    }
  } else if (command === 'agent' && words[1] === 'status') value = await client.me();
  else if (command === 'session' && words[1] === 'open') value = await client.openSession();
  else if (command === 'session' && words[1] === 'close') value = await client.closeSession();
  else if (command === 'work' && words[1] === 'list') value = await client.listWork();
  else if (command === 'work' && words[1] === 'start') value = await client.startWork(words[2], flags.key);
  else if (command === 'work' && words[1] === 'report') value = await client.report(words[2], { kind: flags.kind || 'note', message: flags.message, referenceUrl: flags.source, clientKey: flags.key });
  else if (command === 'work' && words[1] === 'submit') value = await client.submit(words[2], jsonFile(flags.file), flags.key);
  else if (command === 'work' && words[1] === 'finish') value = await client.finish(words[2], flags.summary);
  else if (command === 'activity') value = await client.activity();
  else if (command === 'rewards') value = await client.rewards();
  else throw fail('INVALID_COMMAND', 'Unknown command. Use --help.');
  output(value);
}
main().catch(error => {
  const message = typeof error.message === 'string' ? error.message.replace(/[\x00-\x1f\x7f-\x9f\u202a-\u202e\u2066-\u2069]/gu, ' ') : 'Petri could not complete the request.';
  process.stderr.write(`${JSON.stringify({ ok: false, error: { code: error.code || 'PETRI_ERROR', message } })}\n`);
  process.exitCode = 1;
});
