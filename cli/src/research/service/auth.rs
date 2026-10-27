//! Agents and their pairing: an agent asks to log in, the owner approves its
//! code, the agent collects its credential with the pairing's secret. Only
//! digests of credentials are kept; the ones the workspace must give out or
//! use (the agent's on approval, its API credential) are sealed.

use super::{
    db::{self, Row, atomic, count},
    vault::Vault,
};
use crate::research::workspace::{Failure, digest, fail, fail_status, text, token};
use rusqlite::{Connection, params};
use serde_json::{Value, json};

const PAIR_LIFETIME: i64 = 10 * 60 * 1000;

fn is_hex(value: &Value, length: usize) -> bool {
    value.as_str().is_some_and(|s| {
        s.len() == length
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

pub fn valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    (1..=40).contains(&bytes.len())
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
}

fn public_agent(row: &Row) -> Value {
    json!({ "id": row["id"], "name": row["name"], "harness": row["harness"],
        "payoutAuthority": row["payout"], "createdAt": row["created_at"], "revokedAt": row["revoked_at"] })
}

fn public_pair(row: &Row) -> Value {
    json!({ "id": row["id"], "code": row["code"], "name": row["name"], "harness": row["harness"],
        "status": row["status"], "expiresAt": row["expires_at"], "error": row["error"] })
}

fn expired() -> Failure {
    fail_status(
        "PAIRING_EXPIRED",
        "The login request expired. Start login again.",
        410,
    )
}

fn pairing(db: &Connection, now: i64, id: &str) -> Result<Row, Failure> {
    db::get(db, "SELECT * FROM pairings WHERE id=?", [id])?
        .filter(|row| row["expires_at"].as_i64().is_some_and(|at| at > now))
        .ok_or_else(expired)
}

fn user_code(code: &str) -> String {
    format!(
        "{}-{}",
        &code[..4.min(code.len())],
        &code[4.min(code.len())..]
    )
}

pub fn find_agent(db: &Connection, id: &str) -> Result<Option<Value>, Failure> {
    Ok(db::get(db, "SELECT * FROM agents WHERE id=?", [id])?.map(|row| public_agent(&row)))
}

pub fn setting(db: &Connection, key: &str) -> Result<String, Failure> {
    Ok(
        db::get(db, "SELECT value FROM settings WHERE key=?", [key])?
            .and_then(|row| row["value"].as_str().map(str::to_string))
            .unwrap_or_default(),
    )
}

pub fn set_setting(db: &Connection, key: &str, value: &str) -> Result<(), Failure> {
    db::run(
        db,
        "INSERT INTO settings VALUES (?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        [key, value],
    )?;
    Ok(())
}

/// An agent's login request: a pairing for the owner to approve, or the same
/// one again for a retry with its login key.
pub fn start(
    db: &Connection,
    now: i64,
    name: &Value,
    harness: &Value,
    login_key: Option<&Value>,
) -> Result<Value, Failure> {
    let name = text(name, 40, "agent name")?;
    let harness = text(harness, 60, "harness name")?;
    let login_key = match login_key {
        None => token(),
        Some(key) if is_hex(key, 64) => key.as_str().unwrap_or_default().to_string(),
        Some(_) => {
            return Err(fail(
                "INVALID_LOGIN_KEY",
                "Provide a random 32-byte login key as lowercase hex.",
            ));
        }
    };
    if !valid_name(&name) {
        return Err(fail(
            "INVALID_NAME",
            "Use lowercase letters, digits, hyphens or underscores for the agent name.",
        ));
    }
    atomic(db, || {
        if let Some(retry) = db::get(
            db,
            "SELECT * FROM pairings WHERE secret_hash=?",
            [digest(&login_key)],
        )? {
            if retry["name"] != name || retry["harness"] != harness {
                return Err(fail_status(
                    "REQUEST_CONFLICT",
                    "This login key belongs to another request.",
                    409,
                ));
            }
            let expires = retry["expires_at"].as_i64().unwrap_or_default();
            if expires <= now {
                return Err(expired());
            }
            return Ok(json!({ "pairingId": retry["id"],
                "userCode": user_code(retry["code"].as_str().unwrap_or_default()),
                "secret": login_key, "expiresAt": expires }));
        }
        if db::get(db, "SELECT id FROM agents WHERE name=?", [&name])?.is_some() {
            return Err(fail_status(
                "AGENT_EXISTS",
                "This agent already exists. Reuse its saved profile, or choose a new name.",
                409,
            ));
        }
        db::run(
            db,
            "DELETE FROM pairings WHERE expires_at < ?",
            [now - 86_400_000],
        )?;
        if count(db, "SELECT count(*) AS n FROM agents", [])? >= 1000 {
            return Err(fail_status(
                "AGENT_LIMIT",
                "Workspace agent capacity reached.",
                429,
            ));
        }
        if db::get(
            db,
            "SELECT id FROM pairings WHERE name=? AND expires_at>? AND status IN ('pending','approving','failed')",
            params![name, now],
        )?
        .is_some()
        {
            return Err(fail_status(
                "LOGIN_PENDING",
                "A login request for this name is already pending. Resume that request.",
                409,
            ));
        }
        let id = token()[..32].to_string();
        let code = token()[..8].to_uppercase();
        db::run(
            db,
            "INSERT INTO pairings(id,code,secret_hash,name,harness,status,created_at,expires_at) VALUES (?,?,?,?,?,?,?,?)",
            params![
                id,
                code,
                digest(&login_key),
                name,
                harness,
                "pending",
                now,
                now + PAIR_LIFETIME
            ],
        )?;
        Ok(
            json!({ "pairingId": id, "userCode": user_code(&code), "secret": login_key,
            "expiresAt": now + PAIR_LIFETIME }),
        )
    })
}

/// The pairing's state for the agent that holds its secret: its credential
/// once the owner approved it.
pub fn poll(
    db: &Connection,
    vault: &Vault,
    now: i64,
    id: &str,
    secret: &str,
) -> Result<Value, Failure> {
    let row = pairing(db, now, id)?;
    if row["secret_hash"].as_str() != Some(digest(secret).as_str()) {
        return Err(fail_status("UNAUTHORIZED", "Invalid login request.", 401));
    }
    if row["status"] != "approved" {
        return Ok(
            json!({ "status": row["status"], "error": row["error"], "expiresAt": row["expires_at"] }),
        );
    }
    let agent = find_agent(db, row["agent_id"].as_str().unwrap_or_default())?
        .filter(|agent| agent["revokedAt"].is_null())
        .ok_or_else(|| {
            fail_status(
                "AGENT_REVOKED",
                "This agent connection has been revoked.",
                401,
            )
        })?;
    let credential = vault.open(
        row["delivery_sealed"].as_str().unwrap_or_default(),
        &format!("pair:{id}"),
    )?;
    Ok(json!({ "status": "approved", "agent": agent, "credential": credential }))
}

pub fn pending(db: &Connection, now: i64) -> Result<Vec<Value>, Failure> {
    Ok(db::all(
        db,
        "SELECT * FROM pairings WHERE expires_at>? AND status IN ('pending','approving','failed') ORDER BY created_at",
        [now],
    )?
    .iter()
    .map(public_pair)
    .collect())
}

fn busy(row: &Row) -> Result<(), Failure> {
    if row["status"] == "pending" || row["status"] == "failed" {
        Ok(())
    } else {
        Err(fail_status(
            "PAIRING_BUSY",
            "This login request is already being handled.",
            409,
        ))
    }
}

pub fn begin_approval(db: &Connection, now: i64, id: &str) -> Result<(), Failure> {
    atomic(db, || {
        busy(&pairing(db, now, id)?)?;
        db::run(
            db,
            "UPDATE pairings SET status='approving',error=NULL WHERE id=?",
            [id],
        )?;
        Ok(())
    })
}

/// The approved agent, registered with the API as `registration` (its
/// researcher identity and API credential) for the owner's wallet.
pub fn complete_approval(
    db: &Connection,
    vault: &Vault,
    now: i64,
    id: &str,
    registration: &Value,
    payout_authority: &str,
) -> Result<Value, Failure> {
    atomic(db, || {
        let pair = pairing(db, now, id)?;
        let agent = &registration["agent"];
        if pair["status"] != "approving" {
            return Err(fail_status(
                "PAIRING_BUSY",
                "This request cannot be approved.",
                409,
            ));
        }
        if !is_hex(&agent["agentId"], 64)
            || agent["payoutAuthority"].as_str() != Some(payout_authority)
            || !is_hex(&registration["credential"], 64)
        {
            return Err(fail_status(
                "UPSTREAM_INVALID",
                "Invalid researcher registration response.",
                502,
            ));
        }
        let agent_id = agent["agentId"].as_str().unwrap_or_default();
        let credential = token();
        let upstream = vault.seal(
            registration["credential"].as_str().unwrap_or_default(),
            &format!("upstream:{agent_id}"),
        );
        db::run(
            db,
            "INSERT INTO agents VALUES (?,?,?,?,?,?,?,NULL)",
            params![
                agent_id,
                pair["name"].as_str(),
                pair["harness"].as_str(),
                payout_authority,
                digest(&credential),
                upstream,
                now
            ],
        )?;
        db::run(
            db,
            "UPDATE pairings SET status='approved',agent_id=?,delivery_sealed=? WHERE id=?",
            params![agent_id, vault.seal(&credential, &format!("pair:{id}")), id],
        )?;
        find_agent(db, agent_id).map(|agent| agent.unwrap_or(Value::Null))
    })
}

pub fn fail_approval(db: &Connection, id: &str, message: &str) -> Result<(), Failure> {
    db::run(
        db,
        "UPDATE pairings SET status='failed',error=? WHERE id=? AND status='approving'",
        [message, id],
    )?;
    Ok(())
}

pub fn reject(db: &Connection, now: i64, id: &str) -> Result<(), Failure> {
    busy(&pairing(db, now, id)?)?;
    db::run(db, "UPDATE pairings SET status='rejected' WHERE id=?", [id])?;
    Ok(())
}

/// The agent a request's credential belongs to.
pub fn authenticate(db: &Connection, credential: Option<&str>) -> Result<Value, Failure> {
    let credential = credential
        .filter(|c| is_hex(&json!(c), 64))
        .ok_or_else(|| fail_status("UNAUTHORIZED", "Log in to Petri first.", 401))?;
    let row = db::get(
        db,
        "SELECT * FROM agents WHERE token_hash=? AND revoked_at IS NULL",
        [digest(credential)],
    )?
    .ok_or_else(|| {
        fail_status(
            "UNAUTHORIZED",
            "The agent credential is invalid or revoked.",
            401,
        )
    })?;
    Ok(public_agent(&row))
}

pub fn list(db: &Connection) -> Result<Vec<Value>, Failure> {
    Ok(db::all(
        db,
        "SELECT * FROM agents ORDER BY created_at DESC LIMIT 1000",
        [],
    )?
    .iter()
    .map(public_agent)
    .collect())
}

/// The agent's API credential, for its submissions.
pub fn upstream_credential(db: &Connection, vault: &Vault, id: &str) -> Result<String, Failure> {
    let row = db::get(
        db,
        "SELECT upstream_sealed FROM agents WHERE id=? AND revoked_at IS NULL",
        [id],
    )?
    .ok_or_else(|| {
        fail_status(
            "UNAUTHORIZED",
            "The agent credential is invalid or revoked.",
            401,
        )
    })?;
    vault.open(
        row["upstream_sealed"].as_str().unwrap_or_default(),
        &format!("upstream:{id}"),
    )
}

pub fn revoke(db: &Connection, now: i64, id: &str) -> Result<(), Failure> {
    if find_agent(db, id)?.is_none() {
        return Err(fail_status("AGENT_NOT_FOUND", "Agent not found.", 404));
    }
    atomic(db, || {
        db::run(
            db,
            "UPDATE agents SET revoked_at=coalesce(revoked_at,?) WHERE id=?",
            params![now, id],
        )?;
        db::run(
            db,
            "UPDATE sessions SET closed_at=coalesce(closed_at,?) WHERE agent_id=?",
            params![now, id],
        )?;
        db::run(
            db,
            "UPDATE pairings SET delivery_sealed=NULL,status='revoked' WHERE agent_id=?",
            [id],
        )?;
        Ok(())
    })
}
