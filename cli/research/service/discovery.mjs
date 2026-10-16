import { endpoint, fail } from './security.mjs';
import { assessCoverage } from './coverage-policy.mjs';

// The futures API's oracle documents.
export const ORACLE_SNAPSHOT_PATH = '/v1/oracle/public-snapshot';
export const ORACLE_CATALOG_PATH = '/v1/oracle/catalog';

export function createOracleReader(baseUrl, fetcher = fetch) {
  const origin = endpoint(baseUrl);
  async function read(path) {
    const response = await fetcher(`${origin}${path}`, { method: 'GET', redirect: 'error', cache: 'no-store',
      signal: AbortSignal.timeout(15000), headers: { accept: 'application/json' } });
    if (!response.ok || Number(response.headers.get('content-length')) > 4 * 1024 * 1024) throw fail('ORACLE_UNAVAILABLE', 'The oracle read service is unavailable.', 503);
    const chunks = []; let size = 0;
    for await (const chunk of response.body || []) {
      size += chunk.length;
      if (size > 4 * 1024 * 1024) throw fail('ORACLE_UNAVAILABLE', 'The oracle response exceeds the read bound.', 503);
      chunks.push(chunk);
    }
    const result = JSON.parse(Buffer.concat(chunks).toString('utf8'));
    if (result?.ok !== true || !result.data) throw fail('ORACLE_UNAVAILABLE', 'The oracle read service is unavailable.', 503);
    return result.data;
  }
  return async () => {
    // Prepared public views only. Never fan out into per-source RPC reads.
    const [snapshot, catalog] = await Promise.all([
      read(ORACLE_SNAPSHOT_PATH), read(ORACLE_CATALOG_PATH),
    ]);
    return { snapshot, catalog };
  };
}
export function createDiscovery(store, reader, clock = Date.now) {
  let pending, timer, closed = false;
  async function refresh() {
    if (pending) return pending;
    if (!reader || closed) return store.status();
    const revisions = JSON.stringify(store.list());
    pending = (async () => {
      try {
        const observations = await Promise.resolve().then(reader);
        if (!closed && revisions === JSON.stringify(store.list())) store.record(assessCoverage(observations, store.list(), clock()));
      } catch { if (!closed) store.fail(); }
      finally {
        pending = undefined;
        if (!closed && revisions !== JSON.stringify(store.list())) void refresh();
      }
      return closed ? null : store.status();
    })();
    return pending;
  }
  return { refresh,
    start() { if (!timer && reader) { void refresh(); timer = setInterval(() => { void refresh(); }, 60000); timer.unref(); } },
    close() { closed = true; clearInterval(timer); },
  };
}
