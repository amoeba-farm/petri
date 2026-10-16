import { endpoint, fail } from '../service/security.mjs';

export function createHttpClient(baseUrl, { timeoutMs = 20000 } = {}) {
  const base = endpoint(baseUrl);
  return async (method, path, body, credential) => {
    if (!path.startsWith('/v1/') && path !== '/health') throw fail('INVALID_ROUTE', 'Invalid workspace route.');
    let response;
    for (let attempt = 0; attempt < (method === 'GET' ? 2 : 1); attempt++) {
      try { response = await fetch(`${base}${path}`, { method, redirect: 'error', cache: 'no-store', signal: AbortSignal.timeout(timeoutMs),
        headers: { accept: 'application/json', 'content-type': 'application/json', ...(credential ? { authorization: `Bearer ${credential}` } : {}) },
        ...(body === undefined ? {} : { body: JSON.stringify(body) }) }); break; }
      catch { if (method !== 'GET' || attempt === 1) throw fail('WORKSPACE_UNAVAILABLE', 'Petri is unavailable. Open the research TUI or start its workspace service.', 503); }
    }
    const chunks = []; let size = 0;
    for await (const chunk of response.body || []) { size += chunk.length; if (size > 4 * 1024 * 1024) throw fail('INVALID_RESPONSE', 'The workspace response is too large.', 502); chunks.push(chunk); }
    let result;
    try { result = JSON.parse(Buffer.concat(chunks).toString('utf8')); } catch { throw fail('INVALID_RESPONSE', 'Invalid workspace response.', 502); }
    if (!response.ok || result.ok !== true) throw fail(result.error?.code || 'WORKSPACE_ERROR', typeof result.error?.message === 'string' ? result.error.message : 'The workspace rejected this request.', response.status);
    return result.data;
  };
}
