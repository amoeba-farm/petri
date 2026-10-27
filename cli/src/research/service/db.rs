//! The workspace's SQLite file (`workspace.sqlite`), with the Node service's
//! schema and its additions: a workspace either one made opens in the other.
//! Rows read as JSON objects keyed by column, as the stores use them.

use super::super::workspace::Failure;
use rusqlite::{Connection, Params, types::ValueRef};
use serde_json::{Map, Value, json};
use std::path::Path;

pub type Row = Map<String, Value>;

pub fn failed(error: rusqlite::Error) -> Failure {
    Failure {
        code: None,
        message: format!("The workspace database failed: {error}"),
        status: None,
    }
}

pub fn open(path: &Path) -> Result<Connection, Failure> {
    let db = Connection::open(path).map_err(failed)?;
    db.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=3000;",
    )
    .map_err(failed)?;
    db.execute_batch(
        "
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
  ",
    )
    .map_err(failed)?;
    // Preserve earlier local research history while moving new work to directives.
    for (table, column, kind) in [
        ("work", "objective_json", "TEXT"),
        ("work", "discovery_key", "TEXT"),
        ("work", "availability", "TEXT"),
        ("work", "valid_until", "INTEGER"),
        ("work", "guidance_json", "TEXT"),
        ("runs", "context_json", "TEXT"),
        // The owner's "Research this": focused objectives come first for agents.
        ("work", "focused_at", "INTEGER"),
        // When discovery archived the work because the oracle stopped listing
        // its series (undone if the series is listed again).
        ("work", "retired_at", "INTEGER"),
        // The web proof bundle a submission came with (its API digest).
        ("submissions", "bundle_digest", "TEXT"),
    ] {
        let columns = all(&db, &format!("PRAGMA table_info({table})"), [])?;
        if !columns.iter().any(|row| row["name"] == column) {
            db.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {kind}"))
                .map_err(failed)?;
        }
    }
    db.execute_batch(
        "CREATE UNIQUE INDEX IF NOT EXISTS work_discovery ON work(discovery_key) WHERE discovery_key IS NOT NULL;",
    )
    .map_err(failed)?;
    // An interrupted non-financial registration or submission may be retried explicitly.
    run(
        &db,
        "UPDATE pairings SET status='failed', error='Approval was interrupted. Retry from Connect.' WHERE status='approving'",
        [],
    )?;
    run(
        &db,
        "UPDATE submissions SET status='retryable' WHERE status='pending'",
        [],
    )?;
    Ok(db)
}

fn read(row: &rusqlite::Row, names: &[String]) -> rusqlite::Result<Row> {
    let mut out = Row::new();
    for (index, name) in names.iter().enumerate() {
        let value = match row.get_ref(index)? {
            ValueRef::Null => Value::Null,
            ValueRef::Integer(n) => json!(n),
            ValueRef::Real(n) => json!(n),
            ValueRef::Text(text) | ValueRef::Blob(text) => {
                Value::String(String::from_utf8_lossy(text).into_owned())
            }
        };
        out.insert(name.clone(), value);
    }
    Ok(out)
}

pub fn all(db: &Connection, sql: &str, params: impl Params) -> Result<Vec<Row>, Failure> {
    let mut statement = db.prepare_cached(sql).map_err(failed)?;
    let names: Vec<String> = statement.column_names().map(|s| s.to_string()).collect();
    let rows = statement
        .query_map(params, |row| read(row, &names))
        .map_err(failed)?;
    rows.collect::<rusqlite::Result<_>>().map_err(failed)
}

pub fn get(db: &Connection, sql: &str, params: impl Params) -> Result<Option<Row>, Failure> {
    let mut statement = db.prepare_cached(sql).map_err(failed)?;
    let names: Vec<String> = statement.column_names().map(|s| s.to_string()).collect();
    let mut rows = statement.query(params).map_err(failed)?;
    match rows.next().map_err(failed)? {
        Some(row) => read(row, &names).map(Some).map_err(failed),
        None => Ok(None),
    }
}

/// Runs one statement: the id of the row it inserted.
pub fn run(db: &Connection, sql: &str, params: impl Params) -> Result<i64, Failure> {
    db.prepare_cached(sql)
        .and_then(|mut statement| statement.execute(params))
        .map_err(failed)?;
    Ok(db.last_insert_rowid())
}

/// A count (`SELECT count(*) AS n ...`).
pub fn count(db: &Connection, sql: &str, params: impl Params) -> Result<i64, Failure> {
    Ok(get(db, sql, params)?
        .and_then(|row| row["n"].as_i64())
        .unwrap_or(0))
}

/// `f` in one immediate transaction: all of it, or none of it when it fails
/// (or panics).
pub fn atomic<T>(db: &Connection, f: impl FnOnce() -> Result<T, Failure>) -> Result<T, Failure> {
    struct Open<'a>(&'a Connection, bool);
    impl Drop for Open<'_> {
        fn drop(&mut self) {
            if !self.1 {
                let _ = self.0.execute_batch("ROLLBACK");
            }
        }
    }
    db.execute_batch("BEGIN IMMEDIATE").map_err(failed)?;
    let mut open = Open(db, false);
    let value = f()?;
    db.execute_batch("COMMIT").map_err(failed)?;
    open.1 = true;
    Ok(value)
}

/// A JSON column's value (`null` when empty).
pub fn parsed(row: &Row, column: &str) -> Value {
    row.get(column)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .and_then(|text| serde_json::from_str(text).ok())
        .unwrap_or(Value::Null)
}
