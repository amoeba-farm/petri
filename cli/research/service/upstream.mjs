import { endpoint, fail } from './security.mjs';

export function createUpstream(baseUrl, fetcher = fetch) {
  const base = endpoint(baseUrl);
  // `raw`: a body sent exactly as given (an evidence bundle's own bytes).
  async function request(method, path, body, credential, { raw, rejected } = {}) {
    let response;
    try {
      response = await fetcher(`${base}${path}`, { method, redirect: 'error', cache: 'no-store',
        signal: AbortSignal.timeout(15000), headers: { accept: 'application/json', 'content-type': 'application/json',
          ...(credential ? { authorization: `Bearer ${credential}` } : {}) }, ...(raw !== undefined ? { body: raw } : body === undefined ? {} : { body: JSON.stringify(body) }) });
    } catch { throw fail('UPSTREAM_UNAVAILABLE', 'The source validation service is unavailable. Your workspace is still available.', 503); }
    if (Number(response.headers.get('content-length')) > 2 * 1024 * 1024) throw fail('UPSTREAM_INVALID', 'The validation response is too large.', 502);
    let size = 0, chunks = [];
    for await (const chunk of response.body || []) {
      size += chunk.length;
      if (size > 2 * 1024 * 1024) throw fail('UPSTREAM_INVALID', 'The validation response is too large.', 502);
      chunks.push(chunk);
    }
    let result;
    try { result = JSON.parse(Buffer.concat(chunks).toString('utf8')); } catch { throw fail('UPSTREAM_INVALID', 'The validation service returned an invalid response.', 502); }
    if (!response.ok || result.ok !== true || !result.data) {
      // Remote error text is untrusted and may contain transport details or secrets.
      if (response.status < 500 && rejected) throw fail(...rejected, 422);
      throw fail('SOURCE_REJECTED', response.status >= 500 ? 'Source validation is unavailable.' : 'The source service rejected this request. Check the required source fields and the connected wallet.', response.status >= 500 ? 503 : 422);
    }
    return result.data;
  }
  return {
    register: payoutAuthority => request('POST', '/oracle/agents', { payoutAuthority }),
    submit: (details, credential) => request('POST', '/oracle/source-candidates', details, credential),
    // A web proof bundle, its bytes unchanged: the API verifies and stores it
    // and answers with its bundleDigest (422 when it does not verify).
    uploadEvidence: (bundle, credential) => request('POST', '/oracle/evidence', undefined, credential, { raw: bundle,
      rejected: ['EVIDENCE_REJECTED', 'The API could not verify this web proof. Check it with petri evidence verify <bundle file>, prove the page again if it fails, and resubmit.'] }),
    async candidates(agentId) {
      const candidates = []; let after = '';
      for (let page = 0; page < 10; page++) {
        const result = await request('GET', `/oracle/source-candidates?agentId=${encodeURIComponent(agentId)}&limit=100${after ? `&after=${after}` : ''}`);
        if (!Array.isArray(result.candidates) || result.candidates.some(c => c.agentId !== agentId)) throw fail('UPSTREAM_INVALID', 'Invalid candidate attribution.', 502);
        candidates.push(...result.candidates);
        if (result.candidates.length < 100) return { candidates, complete: true };
        if (!/^[0-9a-f]{64}$/u.test(result.nextCursor) || result.nextCursor <= after) throw fail('UPSTREAM_INVALID', 'Invalid candidate cursor.', 502);
        after = result.nextCursor;
      }
      return { candidates, complete: false };
    },
  };
}
