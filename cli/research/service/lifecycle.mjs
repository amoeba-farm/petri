import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import { createHttpClient } from '../client/http.mjs';
import { fail } from './security.mjs';

export const runtimeId = () => process.env.PETRI_RESEARCH_RUNTIME || 'source';
export async function ensureService(config) {
  if (Number(process.versions.node.split('.')[0]) !== 24) throw fail('NODE_VERSION', 'Petri research requires Node 24.');
  const request = createHttpClient(config.baseUrl);
  const startupRequest = createHttpClient(config.baseUrl, { timeoutMs: 1000 });
  const ping = () => startupRequest('GET', '/v1/owner/ping', undefined, config.ownerToken);
  try {
    const current = await ping();
    if (current.runtime === runtimeId()) return;
    if (!current.runtime) throw fail('SERVICE_RESTART_REQUIRED', 'Close the earlier standalone research service before opening Petri. Your workspace is preserved.');
    // The owner-authenticated drain stops new work, completes in-flight HTTP
    // requests, closes the database, then releases the port for the new runtime.
    for (let attempt = 0; ; attempt++) {
      try { await request('POST', '/v1/owner/restart', {}, config.ownerToken); break; }
      catch (error) { if (error.code !== 'SERVICE_BUSY' || attempt >= 59) throw error; await delay(500); }
    }
    const stopDeadline = Date.now() + 10000;
    for (let attempt = 0; attempt < 100; attempt++) {
      await delay(100);
      try { await ping(); }
      catch (error) { if (error.code === 'WORKSPACE_UNAVAILABLE') break; throw error; }
      if (attempt === 99 || Date.now() >= stopDeadline) throw fail('SERVICE_BUSY', 'The previous workspace is still finishing requests. Try opening Petri again shortly.');
    }
  } catch (error) { if (error.code !== 'WORKSPACE_UNAVAILABLE') throw error; }
  const entry = fileURLToPath(new URL('../bin/petri.mjs', import.meta.url));
  const child = spawn(process.execPath, [entry, 'serve', '--state-dir', config.directory,
    '--upstream', config.upstream, '--oracle-url', config.oracleReadUrl, '--port', String(config.port)],
    { detached: true, windowsHide: true, stdio: 'ignore' });
  let spawnError;
  child.once('error', error => { spawnError = error; });
  child.unref();
  const startDeadline = Date.now() + 10000;
  for (let attempt = 0; attempt < 100 && Date.now() < startDeadline; attempt++) {
    if (spawnError) throw fail('SERVICE_START_FAILED', 'The research service could not start.');
    await delay(100);
    try {
      const current = await ping();
      if (current.runtime === runtimeId()) return;
      throw fail('SERVICE_VERSION', 'Another version of the research workspace is using this port.');
    } catch (error) { if (error.code !== 'WORKSPACE_UNAVAILABLE') throw error; }
  }
  throw fail('SERVICE_START_FAILED', 'The research workspace could not start. Run petri research serve to see the error.');
}
