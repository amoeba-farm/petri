import { randomBytes } from 'node:crypto';
import { digest, token, fail, text } from './security.mjs';

const PAIR_LIFETIME = 10 * 60 * 1000;
const publicAgent = row => row && ({ id: row.id, name: row.name, harness: row.harness,
  payoutAuthority: row.payout, createdAt: row.created_at, revokedAt: row.revoked_at });
const publicPair = row => ({ id: row.id, code: row.code, name: row.name, harness: row.harness,
  status: row.status, expiresAt: row.expires_at, error: row.error });

export function createAuthStore(db, vault, clock = Date.now) {
  function pairing(id) {
    const row = db.get('SELECT * FROM pairings WHERE id=?', id);
    if (!row || row.expires_at <= clock()) throw fail('PAIRING_EXPIRED', 'The login request expired. Start login again.', 410);
    return row;
  }
  const findAgent = id => publicAgent(db.get('SELECT * FROM agents WHERE id=?', id));
  return {
    setting: key => db.get('SELECT value FROM settings WHERE key=?', key)?.value || '',
    setSetting(key, value) { db.run('INSERT INTO settings VALUES (?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value', key, value); },
    start(name, harness, loginKey = token()) {
      name = text(name, 40, 'agent name'); harness = text(harness, 60, 'harness name');
      if (!/^[0-9a-f]{64}$/u.test(loginKey)) throw fail('INVALID_LOGIN_KEY', 'Provide a random 32-byte login key as lowercase hex.');
      if (!/^[a-z0-9][a-z0-9_-]{0,39}$/u.test(name)) throw fail('INVALID_NAME', 'Use lowercase letters, digits, hyphens or underscores for the agent name.');
      return db.atomic(() => {
        const retry = db.get('SELECT * FROM pairings WHERE secret_hash=?', digest(loginKey));
        if (retry) {
          if (retry.name !== name || retry.harness !== harness) throw fail('REQUEST_CONFLICT', 'This login key belongs to another request.', 409);
          if (retry.expires_at <= clock()) throw fail('PAIRING_EXPIRED', 'The login request expired. Start login again.', 410);
          return { pairingId: retry.id, userCode: `${retry.code.slice(0,4)}-${retry.code.slice(4)}`, secret: loginKey, expiresAt: retry.expires_at };
        }
        if (db.get('SELECT id FROM agents WHERE name=?', name)) throw fail('AGENT_EXISTS', 'This agent already exists. Reuse its saved profile, or choose a new name.', 409);
        const now = clock();
        db.run('DELETE FROM pairings WHERE expires_at < ?', now - 86400000);
        if (db.get('SELECT count(*) AS n FROM agents').n >= 1000) throw fail('AGENT_LIMIT', 'Workspace agent capacity reached.', 429);
        if (db.get("SELECT id FROM pairings WHERE name=? AND expires_at>? AND status IN ('pending','approving','failed')", name, now)) {
          throw fail('LOGIN_PENDING', 'A login request for this name is already pending. Resume that request.', 409);
        }
        const id = randomBytes(16).toString('hex'), secret = loginKey, code = randomBytes(4).toString('hex').toUpperCase();
        db.run('INSERT INTO pairings(id,code,secret_hash,name,harness,status,created_at,expires_at) VALUES (?,?,?,?,?,?,?,?)',
          id, code, digest(secret), name, harness, 'pending', now, now + PAIR_LIFETIME);
        return { pairingId: id, userCode: `${code.slice(0, 4)}-${code.slice(4)}`, secret, expiresAt: now + PAIR_LIFETIME };
      });
    },
    poll(id, secret) {
      const row = pairing(id);
      if (typeof secret !== 'string' || row.secret_hash !== digest(secret)) throw fail('UNAUTHORIZED', 'Invalid login request.', 401);
      if (row.status !== 'approved') return { status: row.status, error: row.error, expiresAt: row.expires_at };
      const agent = findAgent(row.agent_id);
      if (!agent || agent.revokedAt !== null) throw fail('AGENT_REVOKED', 'This agent connection has been revoked.', 401);
      return { status: 'approved', agent, credential: vault.open(row.delivery_sealed, `pair:${id}`) };
    },
    pending() { return db.all("SELECT * FROM pairings WHERE expires_at>? AND status IN ('pending','approving','failed') ORDER BY created_at", clock()).map(publicPair); },
    beginApproval(id) {
      return db.atomic(() => {
        const row = pairing(id);
        if (!['pending', 'failed'].includes(row.status)) throw fail('PAIRING_BUSY', 'This login request is already being handled.', 409);
        db.run("UPDATE pairings SET status='approving',error=NULL WHERE id=?", id);
        return row;
      });
    },
    completeApproval(id, registration, payoutAuthority) {
      return db.atomic(() => {
        const pair = pairing(id), agent = registration?.agent;
        if (pair.status !== 'approving') throw fail('PAIRING_BUSY', 'This request cannot be approved.', 409);
        if (!/^[0-9a-f]{64}$/u.test(agent?.agentId) || agent.payoutAuthority !== payoutAuthority || !/^[0-9a-f]{64}$/u.test(registration?.credential)) {
          throw fail('UPSTREAM_INVALID', 'Invalid researcher registration response.', 502);
        }
        const credential = token();
        db.run('INSERT INTO agents VALUES (?,?,?,?,?,?,?,NULL)', agent.agentId, pair.name, pair.harness, payoutAuthority,
          digest(credential), vault.seal(registration.credential, `upstream:${agent.agentId}`), clock());
        db.run("UPDATE pairings SET status='approved',agent_id=?,delivery_sealed=? WHERE id=?", agent.agentId, vault.seal(credential, `pair:${id}`), id);
        return findAgent(agent.agentId);
      });
    },
    failApproval(id, message) { db.run("UPDATE pairings SET status='failed',error=? WHERE id=? AND status='approving'", message, id); },
    reject(id) {
      const row = pairing(id);
      if (!['pending', 'failed'].includes(row.status)) throw fail('PAIRING_BUSY', 'This login request is already being handled.', 409);
      db.run("UPDATE pairings SET status='rejected' WHERE id=?", id);
    },
    authenticate(credential) {
      if (typeof credential !== 'string' || !/^[0-9a-f]{64}$/u.test(credential)) throw fail('UNAUTHORIZED', 'Log in to Petri first.', 401);
      const row = db.get('SELECT * FROM agents WHERE token_hash=? AND revoked_at IS NULL', digest(credential));
      if (!row) throw fail('UNAUTHORIZED', 'The agent credential is invalid or revoked.', 401);
      return publicAgent(row);
    },
    get: findAgent,
    list: () => db.all('SELECT * FROM agents ORDER BY created_at DESC LIMIT 1000').map(publicAgent),
    upstreamCredential(id) {
      const row = db.get('SELECT upstream_sealed FROM agents WHERE id=? AND revoked_at IS NULL', id);
      if (!row) throw fail('UNAUTHORIZED', 'The agent credential is invalid or revoked.', 401);
      return vault.open(row.upstream_sealed, `upstream:${id}`);
    },
    revoke(id) {
      if (!findAgent(id)) throw fail('AGENT_NOT_FOUND', 'Agent not found.', 404);
      db.atomic(() => {
        db.run('UPDATE agents SET revoked_at=coalesce(revoked_at,?) WHERE id=?', clock(), id);
        db.run('UPDATE sessions SET closed_at=coalesce(closed_at,?) WHERE agent_id=?', clock(), id);
        db.run('UPDATE pairings SET delivery_sealed=NULL,status=\'revoked\' WHERE agent_id=?', id);
      });
    },
  };
}
