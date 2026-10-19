#!/usr/bin/env node
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { ensureService } from '../service/lifecycle.mjs';
import { setTimeout as delay } from 'node:timers/promises';
import { createAgentClient } from '../client/agent.mjs';
import { createHttpClient } from '../client/http.mjs';
import { serveMcp } from '../client/mcp.mjs';
import { loadOwner, stateDirectory, fail } from '../service/security.mjs';
import { usdc } from '../service/coverage-policy.mjs';

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
  petri research work next --agent cedar   Start (or resume) the next research run
  petri research work list --agent cedar
  petri research work start <work-id> --agent cedar --key <request-key>
  petri research work report <run-id> --agent cedar --kind search|read|value|submit|note --message <text> [--source <https URL>]
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
// This command as the agent's shell runs it again (PowerShell on Windows).
function self(words, flags) {
  const quote = value => process.platform === 'win32' ? `'${value.replace(/'/gu, "''")}'` : `'${value.replace(/'/gu, `'"'"'`)}'`;
  const options = [['--agent', flags.agent || flags.name || 'researcher'], ...(flags['profile-dir'] ? [['--profile-dir', flags['profile-dir']]] : [])];
  return `${process.platform === 'win32' ? '& ' : ''}${[process.execPath, process.argv[1]].map(quote).join(' ')} ${words} ${options.map(([key, value]) => `${key} ${quote(value)}`).join(' ')}`;
}
// What the agent does with a research run: the exact commands, in order,
// by what the month takes now (sources while it forms, opening values in
// Opening, updates in Game, reward claims once its rewards are frozen).
function assignmentSteps(result, flags) {
  if (result.status !== 'assigned') return { ...result, nextStep: `Nothing needs research right now. In a few minutes, run: ${self('work next', flags)}` };
  const run = result.run.id, assignment = result.assignment, objective = assignment.objective || {};
  const directive = assignment.guidance?.instruction, petri = result.petri || 'petri';
  const month = objective.expiryId || assignment.candidateScope?.expiryId;
  const owner = directive ? `The owner's directive for it comes first: ${directive}` : 'The owner has given no directive for it.';
  // The owner's live feed in the TUI: one short line per step, as it happens.
  const say = self(`work report ${run}`, flags), wrote = { opening: 'opening value submitted', update: 'update committed' }[objective.kind] ?? 'sponsored 30d7e0f9…';
  const report = ['Report as you go, one short line each:',
    `- each search: ${say} --kind search --message "<query>"`,
    `- each page you open: ${say} --kind read --source <page URL> --message "<page title>"`,
    `- each value you read: ${say} --kind value --source <page URL> --message "<value> <unit>, <product>"`,
    `- each petri write: ${say} --kind submit --message "<what it did, e.g. ${wrote}>"`,
    ...(objective.kind === 'opening' || objective.kind === 'update' ? [] : [`- each price page worth submitting: ${say} --kind source_found --source <page URL> --message "<product, price>"`]),
    `- anything else: ${say} --message "<text>" (--kind blocked if stuck)`].join('\n');
  const finish = `When you are done with it, run: ${self(`work finish ${run}`, flags)} --summary "<what you did>"`;
  const next = `Then run ${self('work next', flags)} again for the next one. Keep going until it says nothing needs research.`;
  if (objective.kind === 'opening') {
    // The oracle's opening rules: the stake is exactly the product's opening
    // bond; the archive is of the source's own URL, captured from the
    // Opening's start up to the chain's clock; the claim is accepted 300
    // slots after it lands, and only before listing.
    const source = objective.sourceAddress ?? objective.sourceId ?? '<source address>', url = objective.url ?? '<page URL>';
    const bond = usdc(objective.openingBondAtomic), start = objective.openingStartUtc, listing = objective.listingUtc;
    const until = listing ? ` (${listing})` : '';
    return { ...result, directive: directive || null, nextStep: [
      `Give this frozen source its opening value now: ${assignment.title} (${month}). Its page: ${objective.url ?? 'see "assignment"'}.`,
      owner,
      `The month is in Opening until listing${until}, on the chain's clock: each frozen source needs an opening value, the price its page shows now, where the source starts in Game. Anyone submits it, bonded from their petri wallet with the product's opening bond; a source with none by listing sits out the month.`,
      `1. Open the page and read the value it publishes for this exact product (${objective.bucketLabel ?? 'see the brief'}), in the page's own units.`,
      `2. Archive the page: open https://web.archive.org/save/${url} and note the 14-digit UTC capture time it gives. The archive URL is https://web.archive.org/web/<capture time>/${url}, with the page URL exactly as written here. The capture time must be at or after the Opening's start${start ? ` (${start}, ${start.replace(/\D/gu, '').slice(0, 14)})` : ''} and not after the chain's clock (${petri} status shows it as Clock). If the chain's clock is behind the capture, use an earlier capture of the page from that range (its Wayback calendar lists them) or wait until the chain's clock passes it.`,
      `3. Submit it with the owner's wallet: ${petri} oracle prints opening ${month} --source ${source} --value <value> --archive-url <archive URL> --stake ${bond ?? '<opening bond>'} --yes (${bond ? 'the stake must be exactly this product\'s opening bond' : 'the stake must be exactly the product\'s opening bond: petri names it if you give another'}). Submit it some minutes before listing: it can only be accepted 300 slots after it lands. The bond comes from the owner's oracle collateral; if petri says it is short, deposit the bond first: ${petri} oracle collateral deposit ${bond ?? '<opening bond>'} --yes`,
      `4. Anyone can challenge it for 300 slots (about two minutes). Then accept it: ${petri} oracle prints finalize ${month} --source ${source} --yes. If petri says it can't be accepted yet, wait a minute and run it again. It must be accepted before listing${until}. If it was challenged, the oracle's council decides it: report that and move on.`,
      `5. Return the bond to the owner's collateral: ${petri} oracle stakes settle ${month} --kind opening-claim --source ${source} --yes`,
      `At listing the month enters Game, once every frozen source has its opening value or is set aside (anyone sets an unopened one aside: ${petri} oracle prints expire ${month} --source <source address> --yes) and the oracle has weighted its sources. Then anyone can start Game: ${petri} oracle prints finalize-phase ${month} --yes.`,
      report, finish, next,
    ].join('\n') };
  }
  if (objective.kind === 'claims') {
    return { ...result, directive: directive || null, nextStep: [
      `Claim the owner's rewards of ${month}. They go to the owner's oracle collateral.`,
      owner,
      `1. List them: ${petri} oracle rewards ${month}`,
      `2. If any are to claim: ${petri} oracle rewards claim ${month} --yes`,
      `3. For each sponsored source to claim: ${petri} oracle sponsorships claim <source> --yes`,
      report, finish, next,
    ].join('\n') };
  }
  if (objective.kind === 'update') {
    return { ...result, directive: directive || null, nextStep: [
      `Update this live source now: ${assignment.title} (${month}). Its page: ${objective.url ?? 'see "assignment"'}.`,
      owner,
      'The month is in Game: its sources are set, and each update is a staked new value from the owner\'s petri wallet.',
      `1. Open the page and read the value it publishes for this exact product (${objective.bucketLabel ?? 'see the brief'}).`,
      `2. Archive the page: open https://web.archive.org/save/${objective.url ?? '<page URL>'} and keep the snapshot URL it gives (https://web.archive.org/web/<14-digit time>/...).`,
      `3. Commit the update: ${petri} oracle updates commit ${month} --source ${objective.sourceAddress ?? '<source address>'} --value <value> --archive-url <snapshot URL> --stake 1 --yes (if petri names another stake for this product, use that).`,
      `4. Reveal it within a minute, with the claim address commit printed: ${petri} oracle updates reveal <CLAIM> --yes`,
      'Skip the update if the value has not changed: an unchanged value earns nothing.',
      report, finish, next,
    ].join('\n') };
  }
  return { ...result, directive: directive || null, nextStep: [
    `Research this now: ${assignment.title}. Its brief and exact scope (candidateScope) are in "assignment". The month takes new sources now.`,
    owner,
    report,
    `Submit each page you can verify: write its candidate JSON (marketId, expiryId, bucketId from candidateScope; canonicalUrl, sourceType, productDefinition, priceField, region, currency, evidence) to a file, then run: ${self(`work submit ${run}`, flags)} --file <file>`,
    `A submitted find goes on the oracle once a sponsor posts its bond. If the owner asks you to sponsor finds with their wallet: ${petri} oracle sponsor <contentHash> --yes. A product counts as covered once a source of it has a supporter: ${petri} oracle sources support ${month} --source <source address> --stake 1 --yes. When every product is covered, the owner decides when to close the source window (Scramble's challenge days follow): ${petri} oracle sources close ${month} --yes.`,
    finish, next,
  ].join('\n') };
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
    if (value.status === 'connected') value = { ...value, nextStep: `Connected. Start researching: run ${self('work next', flags)} and follow what it prints.` };
  } else if (command === 'agent' && words[1] === 'status') value = await client.me();
  else if (command === 'session' && words[1] === 'open') value = await client.openSession();
  else if (command === 'session' && words[1] === 'close') value = await client.closeSession();
  else if (command === 'work' && words[1] === 'list') value = await client.listWork();
  else if (command === 'work' && words[1] === 'next') value = assignmentSteps(await client.next(), flags);
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
