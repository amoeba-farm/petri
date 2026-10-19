import { join, resolve } from 'node:path';
import { existsSync, readFileSync, lstatSync } from 'node:fs';
import { randomUUID } from 'node:crypto';
import { createHttpClient } from './http.mjs';
import { endpoint, fail, privateDirectory, privateWrite, stateDirectory, token } from '../service/security.mjs';
import { REPORT_KINDS } from '../service/work-store.mjs';

export function createAgentClient({ name = 'researcher', harness = 'Custom harness', baseUrl, profileDirectory } = {}) {
  if (!/^[a-z0-9][a-z0-9_-]{0,39}$/u.test(name)) throw fail('INVALID_NAME', 'Use lowercase letters, digits, hyphens or underscores for the agent name.');
  const directory = profileDirectory ? privateDirectory(resolve(profileDirectory)) : privateDirectory(join(privateDirectory(stateDirectory()), 'profiles'));
  const path = join(directory, `${name}.json`);
  function read() {
    if (!existsSync(path)) return { version: 1, name, baseUrl: endpoint(baseUrl || 'http://127.0.0.1:4185') };
    if (lstatSync(path).isSymbolicLink()) throw fail('UNSAFE_STORAGE', 'Agent profiles must not be symbolic links.');
    const value = JSON.parse(readFileSync(path, 'utf8'));
    if (value.version !== 1 || value.name !== name || baseUrl && endpoint(baseUrl) !== value.baseUrl) throw fail('PROFILE_MISMATCH', 'This agent profile belongs to another workspace. Use a separate profile directory.');
    endpoint(value.baseUrl); return value;
  }
  function save(profile) { privateWrite(path, profile); }
  async function authorized(method, route, body) {
    const profile = read();
    if (!profile.credential) throw fail('LOGIN_REQUIRED', 'Log this agent into Petri first.', 401);
    return createHttpClient(profile.baseUrl)(method, route, body, profile.credential);
  }
  async function session(force = false) {
    const profile = read();
    if (profile.sessionId && !force) {
      try { return (await authorized('POST', '/v1/sessions/heartbeat', { sessionId: profile.sessionId })).session; }
      catch (error) { if (error.code !== 'SESSION_EXPIRED') throw error; }
    }
    const result = await authorized('POST', '/v1/sessions', { harness });
    save({ ...read(), sessionId: result.session.id }); return result.session;
  }
  return {
    async login() {
      const profile = read(), request = createHttpClient(profile.baseUrl);
      if (profile.credential) { const result = await authorized('GET', '/v1/me'); return { status: 'connected', agent: result.agent, session: await session() }; }
      if (profile.pending) {
        let result;
        try { result = await request('POST', '/v1/login/poll', { pairingId: profile.pending.pairingId, secret: profile.pending.secret }); }
        catch (error) {
          if (error.code === 'PAIRING_EXPIRED') { delete profile.pending; save(profile); throw error; }
          throw error;
        }
        if (result.status === 'approved') {
          if (!/^[0-9a-f]{64}$/u.test(result.credential) || result.agent?.name !== name) throw fail('INVALID_RESPONSE', 'Invalid login response.', 502);
          save({ ...profile, pending: undefined, loginAttempt: undefined, credential: result.credential, agentId: result.agent.id });
          return { status: 'connected', agent: result.agent, session: await session() };
        }
        return { status: result.status, userCode: profile.pending.userCode, expiresAt: result.expiresAt,
          nextStep: result.status === 'rejected' ? 'The workspace owner rejected this connection.' : 'Ask the owner to approve this code in Petri’s Connect view.', error: result.error || null };
      }
      const attempt = profile.loginAttempt || { name, harness, loginKey: token() };
      if (!profile.loginAttempt) { profile.loginAttempt = attempt; save(profile); }
      let pending;
      try { pending = await request('POST', '/v1/login', attempt); }
      catch (error) { if (error.code === 'PAIRING_EXPIRED') { delete profile.loginAttempt; save(profile); } throw error; }
      if (!/^[0-9a-f]{64}$/u.test(pending.secret) || !/^[0-9a-f]{32}$/u.test(pending.pairingId)) throw fail('INVALID_RESPONSE', 'Invalid login response.', 502);
      save({ ...profile, loginAttempt: undefined, pending });
      return { status: 'pending', userCode: pending.userCode, expiresAt: pending.expiresAt, nextStep: 'Ask the owner to approve this code in Petri’s Connect view, then call login again.' };
    },
    me: () => authorized('GET', '/v1/me'),
    openSession: () => session(true),
    async closeSession() { const profile = read(); if (!profile.sessionId) return { closed: true }; const result = await authorized('POST', '/v1/sessions/close', { sessionId: profile.sessionId }); delete profile.sessionId; save(profile); return result; },
    listWork: () => authorized('GET', '/v1/work'),
    async next() { return authorized('POST', '/v1/work/next', { sessionId: (await session()).id }); },
    activity: () => authorized('GET', '/v1/activity'),
    rewards: () => authorized('GET', '/v1/rewards'),
    async startWork(workId, clientKey = randomUUID()) { return authorized('POST', '/v1/runs', { workId, clientKey, sessionId: (await session()).id }); },
    // One step of the agent's work, for the owner's live feed (REPORT_KINDS).
    async report(runId, { kind = 'note', message, referenceUrl, clientKey = randomUUID() }) {
      if (!REPORT_KINDS.includes(kind)) throw fail('INVALID_EVENT', `Report one of: ${REPORT_KINDS.join(', ')}.`);
      return authorized('POST', `/v1/runs/${encodeURIComponent(runId)}/events`, { sessionId: (await session()).id, clientKey, kind, message, ...(referenceUrl ? { referenceUrl } : {}) });
    },
    async submit(runId, candidate, clientKey = randomUUID()) { return authorized('POST', `/v1/runs/${encodeURIComponent(runId)}/candidates`, { sessionId: (await session()).id, clientKey, candidate }); },
    async finish(runId, summary) { return authorized('POST', `/v1/runs/${encodeURIComponent(runId)}/finish`, { sessionId: (await session()).id, summary }); },
  };
}
