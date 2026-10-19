import { DatabaseSync } from 'node:sqlite';

export function createDatabase(path) {
  const db = new DatabaseSync(path);
  db.exec('PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=3000;');
  db.exec(`
    CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL) STRICT;
    CREATE TABLE IF NOT EXISTS agents (
      id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, harness TEXT NOT NULL, payout TEXT NOT NULL,
      token_hash TEXT NOT NULL UNIQUE, upstream_sealed TEXT NOT NULL, created_at INTEGER NOT NULL, revoked_at INTEGER
    ) STRICT;
    CREATE TABLE IF NOT EXISTS pairings (
      id TEXT PRIMARY KEY, code TEXT NOT NULL UNIQUE, secret_hash TEXT NOT NULL, name TEXT NOT NULL,
      harness TEXT NOT NULL, status TEXT NOT NULL, created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL,
      agent_id TEXT REFERENCES agents(id), delivery_sealed TEXT, error TEXT
    ) STRICT;
    CREATE TABLE IF NOT EXISTS sessions (
      id TEXT PRIMARY KEY, agent_id TEXT NOT NULL REFERENCES agents(id), harness TEXT NOT NULL,
      created_at INTEGER NOT NULL, heartbeat_at INTEGER NOT NULL, closed_at INTEGER
    ) STRICT;
    CREATE TABLE IF NOT EXISTS work (
      id TEXT PRIMARY KEY, title TEXT NOT NULL, brief TEXT NOT NULL, created_at INTEGER NOT NULL,
      archived_at INTEGER, scope_json TEXT
    ) STRICT;
    CREATE TABLE IF NOT EXISTS runs (
      id TEXT PRIMARY KEY, work_id TEXT NOT NULL REFERENCES work(id), agent_id TEXT NOT NULL REFERENCES agents(id),
      session_id TEXT NOT NULL REFERENCES sessions(id), client_key TEXT NOT NULL, created_at INTEGER NOT NULL,
      finished_at INTEGER, summary TEXT, UNIQUE(agent_id, client_key)
    ) STRICT;
    CREATE TABLE IF NOT EXISTS events (
      seq INTEGER PRIMARY KEY AUTOINCREMENT, run_id TEXT NOT NULL REFERENCES runs(id), actor TEXT NOT NULL,
      kind TEXT NOT NULL, message TEXT NOT NULL, reference_url TEXT, created_at INTEGER NOT NULL,
      client_key TEXT, UNIQUE(run_id, client_key)
    ) STRICT;
    CREATE TABLE IF NOT EXISTS submissions (
      id INTEGER PRIMARY KEY AUTOINCREMENT, run_id TEXT NOT NULL REFERENCES runs(id), client_key TEXT NOT NULL,
      input_hash TEXT NOT NULL, status TEXT NOT NULL, result_json TEXT, created_at INTEGER NOT NULL,
      UNIQUE(run_id, client_key)
    ) STRICT;
    CREATE INDEX IF NOT EXISTS events_run ON events(run_id, seq);
    CREATE INDEX IF NOT EXISTS runs_agent ON runs(agent_id, created_at);
    CREATE TABLE IF NOT EXISTS directives (
      id TEXT PRIMARY KEY, markets_json TEXT NOT NULL, min_sources INTEGER NOT NULL,
      min_hosts INTEGER NOT NULL, max_age_ms INTEGER NOT NULL, enabled INTEGER NOT NULL,
      revision INTEGER NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
    ) STRICT;
  `);
  // Preserve earlier local research history while moving new work to directives.
  for (const [table, column, type] of [
    ['work','objective_json','TEXT'], ['work','discovery_key','TEXT'],
    ['work','availability','TEXT'], ['work','valid_until','INTEGER'],
    ['work','guidance_json','TEXT'], ['runs','context_json','TEXT'],
    // The owner's "Research this": focused objectives come first for agents.
    ['work','focused_at','INTEGER'],
    // When discovery archived the work because the oracle stopped listing
    // its series (undone if the series is listed again).
    ['work','retired_at','INTEGER'],
  ]) if (!db.prepare(`PRAGMA table_info(${table})`).all().some(row => row.name === column)) db.exec(`ALTER TABLE ${table} ADD COLUMN ${column} ${type}`);
  db.exec('CREATE UNIQUE INDEX IF NOT EXISTS work_discovery ON work(discovery_key) WHERE discovery_key IS NOT NULL;');
  // An interrupted non-financial registration or submission may be retried explicitly.
  db.prepare("UPDATE pairings SET status='failed', error='Approval was interrupted. Retry from Connect.' WHERE status='approving'").run();
  db.prepare("UPDATE submissions SET status='retryable' WHERE status='pending'").run();
  return {
    get: (sql, ...args) => db.prepare(sql).get(...args),
    all: (sql, ...args) => db.prepare(sql).all(...args),
    run: (sql, ...args) => db.prepare(sql).run(...args),
    atomic(fn) {
      db.exec('BEGIN IMMEDIATE');
      try { const value = fn(); db.exec('COMMIT'); return value; }
      catch (error) { db.exec('ROLLBACK'); throw error; }
    },
    close: () => db.close(),
  };
}
