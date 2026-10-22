import http from 'node:http';
import { createApplication } from './application.mjs';
import { fail } from './security.mjs';
import { MAX_SUBMISSION_BODY } from './evidence.mjs';

export async function startServer(config, options = {}) {
  let application, closing;
  let draining = false, activeRequests = 0;
  const idleWaiters = [];
  const server = http.createServer(async (request, response) => {
    activeRequests++;
    response.setHeader('Content-Type', 'application/json; charset=utf-8');
    response.setHeader('Cache-Control', 'no-store');
    response.setHeader('X-Content-Type-Options', 'nosniff');
    try {
      if (draining) throw fail('WORKSPACE_UNAVAILABLE', 'Petri is restarting. Retry shortly.', 503);
      const port = server.address()?.port;
      const hosts = [`127.0.0.1:${port}`, `localhost:${port}`];
      if (!hosts.includes(request.headers.host) || request.headers.origin && !hosts.some(host => request.headers.origin === `http://${host}`)) {
        throw fail('ORIGIN_DENIED', 'This workspace accepts local application requests only.', 403);
      }
      if (!['GET', 'POST'].includes(request.method)) throw fail('METHOD_NOT_ALLOWED', 'Method not allowed.', 405);
      const url = new URL(request.url, 'http://127.0.0.1');
      if (url.origin !== 'http://127.0.0.1' || url.username || url.password) throw fail('INVALID_PATH', 'Invalid request path.');
      let body = {};
      if (request.method === 'POST') {
        if (!/^application\/json(?:\s*;|$)/iu.test(request.headers['content-type'] || '')) throw fail('INVALID_CONTENT_TYPE', 'Use application/json.', 415);
        // A candidate may bring a web proof bundle (service/evidence.mjs).
        const limit = /^\/v1\/runs\/[0-9a-f-]{36}\/candidates$/u.test(url.pathname) ? MAX_SUBMISSION_BODY : 32768;
        const chunks = []; let size = 0;
        for await (const chunk of request) {
          size += chunk.length;
          if (size > limit) throw fail('BODY_TOO_LARGE', 'The request is too large.', 413);
          chunks.push(chunk);
        }
        try { body = JSON.parse(Buffer.concat(chunks).toString('utf8')); } catch { throw fail('INVALID_JSON', 'Invalid JSON request.'); }
      }
      const authorization = request.headers.authorization;
      const credential = typeof authorization === 'string' && /^Bearer [0-9a-f]{64}$/u.test(authorization) ? authorization.slice(7) : undefined;
      const data = await application.handle({ method: request.method, path: url.pathname, query: url.searchParams, credential, body });
      response.writeHead(200); response.end(JSON.stringify({ ok: true, data }));
    } catch (error) {
      response.writeHead(error.status || 500);
      response.end(JSON.stringify({ ok: false, error: { code: error.status ? error.code : 'INTERNAL_ERROR',
        message: error.status ? error.message : 'The workspace could not complete this request.' } }));
    } finally {
      activeRequests--;
      if (!activeRequests) for (const resolve of idleWaiters.splice(0)) resolve();
    }
  });
  server.requestTimeout = 20000; server.headersTimeout = 10000;
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(config.port, '127.0.0.1', resolve); });
  function close() {
    if (closing) return closing;
    draining = true;
    closing = new Promise((resolve, reject) => server.close(async error => {
      // A disconnected HTTP client can leave an accepted upstream submission
      // running. Keep its database open until that handler has completed too.
      if (activeRequests) await new Promise(resolve => idleWaiters.push(resolve));
      application.close();
      if (error) reject(error); else resolve();
    }));
    return closing;
  }
  try { application = createApplication(config, { ...options, restart: () => {
    if (activeRequests > 1) throw fail('SERVICE_BUSY', 'Research requests are still in progress. Retry shortly.', 409);
    draining = true;
    // Let the accepted restart response finish; server.close drains other active requests.
    setImmediate(() => { void close(); });
  } }); }
  catch (error) { await new Promise(resolve => server.close(resolve)); throw error; }
  application.startDiscovery();
  return { baseUrl: `http://127.0.0.1:${server.address().port}`, server, application,
    close };
}
