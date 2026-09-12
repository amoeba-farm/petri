//! Private commit/reveal material. Never included in the operation journal or MCP.
use crate::content_hash::sha256_hex as digest;
use crate::{
    backend::{BackendClient, CliError},
    operation_journal::{self, OperationRecord},
    petri_config,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Commitment {
    schema_version: u8,
    id: String,
    origin: String,
    deployment: String,
    created_at: String,
    request: Value,
    operations: Vec<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecoveryExport {
    schema_version: u8,
    commitment: Commitment,
    operations: Vec<OperationRecord>,
}
pub struct Material {
    pub id: String,
    pub request: Value,
    _claim: fs::File,
}
fn error() -> CliError {
    CliError::new(
        "Private Oracle material is unavailable, malformed, or belongs to another intent. No salt was replaced.",
    )
}
fn intent_id(origin: &str, deployment: &str, request: &Value) -> Result<String, CliError> {
    let identity = json!({
        "domain":"petri-oracle-commitment-v1", "origin":origin, "deployment":deployment,
        "owner":request["ownerPubkey"], "market":request["marketId"], "expiry":request["expiryId"],
        "action":request["actionType"], "source":request["sourceId"], "claim":request["claimId"],
        "dispute":request["disputeId"], "kind":request["disputeKind"]
    });
    Ok(digest(&serde_json::to_vec(&identity).map_err(|_| error())?))
}
fn validate_material(value: &Commitment) -> Result<(), CliError> {
    if value.schema_version != 1
        || value.operations.len() > 256
        || value.id != intent_id(&value.origin, &value.deployment, &value.request)?
        || !matches!(
            value.request["actionType"].as_str(),
            Some("commit_oracle_update_claim_v3" | "commit_oracle_emergency_vote_v3")
        )
        || value.request["secretSaltHex"].as_str().is_none_or(|s| {
            s.len() != 64
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
    {
        return Err(error());
    }
    Ok(())
}
fn root() -> Result<PathBuf, CliError> {
    Ok(petri_config::config_path()
        .map_err(CliError::new)?
        .parent()
        .ok_or_else(error)?
        .join("oracle-private"))
}
fn path(id: &str, suffix: &str) -> Result<PathBuf, CliError> {
    if id.len() != 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(error());
    }
    Ok(root()?.join(format!("{id}.{suffix}")))
}
fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, CliError> {
    let meta = fs::symlink_metadata(path).map_err(|_| error())?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 2 * 1024 * 1024 {
        return Err(error());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(error());
        }
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|_| error())?
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| error())?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err(error());
    }
    serde_json::from_slice(&bytes).map_err(|_| error())
}
fn save<T: Serialize>(path: &Path, value: &T) -> Result<(), CliError> {
    let bytes = serde_json::to_vec(value).map_err(|_| error())?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err(error());
    }
    petri_config::write_private_file(path, &bytes).map_err(CliError::new)
}
fn load(id: &str) -> Result<Commitment, CliError> {
    let value: Commitment = read(&path(id, "json")?)?;
    validate_material(&value)?;
    if value.schema_version != 1
        || value.id != id
        || value.request["secretSaltHex"]
            .as_str()
            .is_none_or(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(error());
    }
    Ok(value)
}
fn scope(value: &Commitment, backend: &BackendClient, request: &Value) -> Result<(), CliError> {
    if value.origin != backend.base_url()
        || value.deployment != crate::current_release::PROGRAM_DATA_PAYLOAD_SHA256
        || ["ownerPubkey", "marketId", "expiryId"]
            .iter()
            .any(|k| value.request[*k] != request[*k])
    {
        return Err(error());
    }
    Ok(())
}
pub fn secret_action(request: &Value) -> bool {
    matches!(
        request["actionType"].as_str(),
        Some(
            "commit_oracle_update_claim_v3"
                | "reveal_oracle_update_claim_v3"
                | "commit_oracle_emergency_vote_v3"
                | "reveal_oracle_emergency_vote_v2"
        )
    )
}
pub fn public_request(request: &Value) -> Value {
    let mut value = request.clone();
    if let Some(map) = value.as_object_mut() {
        map.remove("secretSaltHex");
    }
    value
}
fn require_original_commit(value: &Commitment, backend: &BackendClient) -> Result<(), CliError> {
    for id in &value.operations {
        if !operation_journal::exists(id)? {
            continue;
        }
        let record = operation_journal::load(id)?;
        record.require_scope(
            backend,
            value.request["ownerPubkey"].as_str().ok_or_else(error)?,
        )?;
        if record.operation != value.request["actionType"].as_str().unwrap_or("") {
            continue;
        }
        if record.request != public_request(&value.request) {
            return Err(error());
        }
        if record.signature.is_some() {
            // Do not trust a locally edited status string as proof of a landed commit.
            match crate::current_operation::recover_finalized_signature(backend, &record)? {
                Some(("confirmed", _)) => return Ok(()),
                Some(("failed_on_chain", _)) => continue,
                _ => {
                    return Err(CliError::new(
                        "The original commit is not finalized yet. Recover its signature before preparing a reveal.",
                    ));
                }
            }
        }
    }
    Err(CliError::new(
        "No finalized original commit was found for this private record. Prepare and approve its commit first.",
    ))
}
pub fn materialize(backend: &BackendClient, request: &Value) -> Result<Option<Material>, CliError> {
    if request.get("secretSaltHex").is_some() {
        return Err(CliError::new(
            "Salts come only from the private commitment store, never arguments or journals.",
        ));
    }
    if !secret_action(request) {
        return Ok(None);
    }
    let action = request["actionType"].as_str().ok_or_else(error)?;
    let reveal = action.starts_with("reveal_");
    let id = if reveal {
        request["commitmentId"]
            .as_str()
            .ok_or_else(error)?
            .to_owned()
    } else {
        // Claim/dispute identity, not the mutable payload: changing an intent cannot silently replace its salt.
        intent_id(
            backend.base_url(),
            crate::current_release::PROGRAM_DATA_PAYLOAD_SHA256,
            request,
        )?
    };
    let claim = petri_config::open_private_lock_file(&path(&id, "lock")?).map_err(CliError::new)?;
    claim
        .try_lock()
        .map_err(|_| CliError::new("This commitment is already open in another Petri action."))?;
    let file = path(&id, "json")?;
    let existing = file.try_exists().map_err(|_| error())?;
    let value = if existing {
        load(&id)?
    } else {
        if reveal {
            return Err(error());
        }
        let mut salt = [0u8; 32];
        getrandom::fill(&mut salt).map_err(|_| {
            CliError::new("The operating system could not generate a commitment salt.")
        })?;
        let mut secret = request.clone();
        secret["secretSaltHex"] =
            json!(salt.iter().map(|b| format!("{b:02x}")).collect::<String>());
        let value = Commitment {
            schema_version: 1,
            id: id.clone(),
            origin: backend.base_url().into(),
            deployment: crate::current_release::PROGRAM_DATA_PAYLOAD_SHA256.into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            request: secret,
            operations: Vec::new(),
        };
        // Persist before any network preparation. Never regenerate after a timeout or restart.
        save(&file, &value)?;
        value
    };
    scope(&value, backend, request)?;
    let mut secret = value.request.clone();
    if reveal {
        let expected = match value.request["actionType"].as_str() {
            Some("commit_oracle_update_claim_v3") => "reveal_oracle_update_claim_v3",
            Some("commit_oracle_emergency_vote_v3") => "reveal_oracle_emergency_vote_v2",
            _ => return Err(error()),
        };
        if action != expected {
            return Err(error());
        }
        require_original_commit(&value, backend)?;
        secret["actionType"] = json!(expected);
        let map = secret.as_object_mut().ok_or_else(error)?;
        map.remove("stakeAtomic");
        map.remove("sambaAmountAtomic");
    } else if public_request(&secret) != *request {
        return Err(error());
    }
    // Any uncertain original attempt must be reconciled, never replaced by a new preparation.
    for operation in &value.operations {
        if !operation_journal::exists(operation)? {
            continue;
        }
        let record = operation_journal::load(operation)?;
        if record.operation != action || record.signature.is_none() {
            continue;
        }
        match crate::current_operation::recover_finalized_signature(backend, &record)? {
            Some(("failed_on_chain", _)) => {}
            Some(("confirmed", _)) => {
                return Err(CliError::new(
                    "This commitment action already finalized. Continue with the next lifecycle step.",
                ));
            }
            _ => {
                return Err(CliError::new(
                    "This commitment action has an unresolved original signature. Recover it; do not repeat it.",
                ));
            }
        }
    }
    Ok(Some(Material {
        id,
        request: secret,
        _claim: claim,
    }))
}
pub fn store_plan(material: &Material, operation: &str, payload: &Value) -> Result<(), CliError> {
    let mut value = load(&material.id)?;
    if value.operations.len() >= 256 && !value.operations.iter().any(|id| id == operation) {
        return Err(CliError::new("Private commitment attempt limit reached."));
    }
    save(
        &path(operation, "plan")?,
        &json!({"commitmentId":material.id,"request":material.request,"payload":payload}),
    )?;
    if !value.operations.iter().any(|id| id == operation) {
        if value.operations.len() >= 256 {
            return Err(CliError::new("Private commitment attempt limit reached."));
        }
        value.operations.push(operation.into());
        save(&path(&material.id, "json")?, &value)?;
    }
    Ok(())
}
pub fn execution_payload(
    record: &OperationRecord,
    backend: &BackendClient,
) -> Result<(Value, Value), CliError> {
    if !secret_action(&record.request) {
        return Ok((record.request.clone(), record.prepared.clone()));
    }
    let id = record.prepared["privateCommitmentId"]
        .as_str()
        .ok_or_else(error)?;
    let value = load(id)?;
    scope(&value, backend, &record.request)?;
    for operation in &value.operations {
        if operation == &record.operation_id || !operation_journal::exists(operation)? {
            continue;
        }
        let other = operation_journal::load(operation)?;
        other.require_scope(backend, &record.owner)?;
        if other.operation == record.operation
            && other.signature.is_some()
            && !matches!(
                crate::current_operation::recover_finalized_signature(backend, &other)?,
                Some(("failed_on_chain", _))
            )
        {
            return Err(CliError::new(
                "Another preparation of this commitment was already submitted. Recover that original signature.",
            ));
        }
    }
    if !value.operations.contains(&record.operation_id) {
        return Err(error());
    }
    let plan: Value = read(&path(&record.operation_id, "plan")?)?;
    if plan["commitmentId"] != id
        || public_request(&plan["request"]) != record.request
        || plan["request"]["secretSaltHex"] != value.request["secretSaltHex"]
    {
        return Err(error());
    }
    if record.operation.starts_with("reveal_") {
        require_original_commit(&value, backend)?;
    }
    Ok((plan["request"].clone(), plan["payload"].clone()))
}
pub fn claim_for_execution(record: &OperationRecord) -> Result<Option<fs::File>, CliError> {
    if !secret_action(&record.request) {
        return Ok(None);
    }
    let id = record.prepared["privateCommitmentId"]
        .as_str()
        .ok_or_else(error)?;
    let file = petri_config::open_private_lock_file(&path(id, "lock")?).map_err(CliError::new)?;
    file.try_lock()
        .map_err(|_| CliError::new("This commitment is already open in another Petri action."))?;
    Ok(Some(file))
}

#[derive(Debug, clap::Subcommand)]
pub enum Command {
    /// List local commitment references for a wallet; never display salts.
    List {
        #[arg(long)]
        owner: String,
    },
    /// Export one secret-bearing recovery file to a new private file. Keep it offline.
    Export {
        commitment_id: String,
        #[arg(long)]
        out: PathBuf,
    },
    /// Restore private material and original-signature references from an export; never submits.
    Import {
        #[arg(long)]
        file: PathBuf,
    },
}
pub fn run(command: &Command) -> Result<Value, CliError> {
    if (crate::mcp_actions::active()
        || std::env::var("PETRI_MCP_READ_ONLY").is_ok_and(|v| v == "1"))
        && !matches!(command, Command::List { .. })
    {
        return Err(CliError::new(
            "Private commitment export/import stays local; MCP may list public references and prepare a reveal by commitment ID.",
        ));
    }
    match command {
        Command::List { owner } => {
            let mut records = Vec::new();
            let mut exhaustive = true;
            let mut unreadable = 0;
            match fs::read_dir(root()?) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(error()),
                Ok(entries) => {
                    for (index, entry) in entries.take(4097).enumerate() {
                        if index == 4096 {
                            exhaustive = false;
                            break;
                        }
                        let entry = entry.map_err(|_| error())?;
                        let file = entry.path();
                        if file.extension().and_then(|s| s.to_str()) != Some("json") {
                            continue;
                        }
                        let id = file
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or_default();
                        match load(id) { Ok(v) if v.request["ownerPubkey"]==*owner=>records.push(json!({"commitmentId":v.id,"owner":owner,"market":v.request["marketId"],"expiry":v.request["expiryId"],"action":v.request["actionType"],"sourceId":v.request["sourceId"],"claimId":v.request["claimId"],"disputeId":v.request["disputeId"],"createdAt":v.created_at,"operations":v.operations,"origin":v.origin,"deployment":v.deployment})),Ok(_)=>{},Err(_)=>unreadable+=1 }
                    }
                }
            }
            Ok(
                json!({"ok":true,"commitments":records,"coverage":{"exhaustive":exhaustive,"unreadable":unreadable}}),
            )
        }
        Command::Export { commitment_id, out } => {
            let claim = petri_config::open_private_lock_file(&path(commitment_id, "lock")?)
                .map_err(CliError::new)?;
            claim.try_lock().map_err(|_|CliError::new("This commitment is open in another Petri action. Export after that action finishes."))?;
            let value = load(commitment_id)?;
            if !out.is_absolute() || out.try_exists().map_err(|_| error())? {
                return Err(CliError::new(
                    "Choose a new absolute export file path; existing files are never overwritten.",
                ));
            }
            let operations = value
                .operations
                .iter()
                .filter_map(|id| match operation_journal::exists(id) {
                    Ok(false) => None,
                    _ => Some(operation_journal::load(id)),
                })
                .collect::<Result<Vec<_>, _>>()?;
            // Original-signature references make recovery possible after losing
            // the ordinary journal. Prepared packets are deliberately not exported.
            save(
                out,
                &RecoveryExport {
                    schema_version: 1,
                    commitment: value,
                    operations,
                },
            )?;
            Ok(
                json!({"ok":true,"commitmentId":commitment_id,"exported":true,"secretBearing":true,"message":"Private recovery material exported. Keep the file offline; do not paste it into chat."}),
            )
        }
        Command::Import { file } => {
            let export: RecoveryExport = read(file)?;
            let value = &export.commitment;
            validate_material(value)?;
            if export.schema_version != 1
                || value.schema_version != 1
                || value.operations.len() > 256
                || export.operations.len() > 256
                || !matches!(
                    value.request["actionType"].as_str(),
                    Some("commit_oracle_update_claim_v3" | "commit_oracle_emergency_vote_v3")
                )
            {
                return Err(error());
            }
            let backend = BackendClient::new(value.origin.clone())?;
            scope(value, &backend, &value.request)?;
            let owner = value.request["ownerPubkey"].as_str().ok_or_else(error)?;
            let file = path(&value.id, "json")?;
            let claim = petri_config::open_private_lock_file(&path(&value.id, "lock")?)
                .map_err(CliError::new)?;
            claim.try_lock().map_err(|_| {
                CliError::new("This commitment is already open in another Petri action.")
            })?;
            let mut merged = value.clone();
            if file.try_exists().map_err(|_| error())? {
                let existing = load(&value.id)?;
                if existing.request != value.request
                    || existing.origin != value.origin
                    || existing.deployment != value.deployment
                {
                    return Err(error());
                }
                for operation in existing.operations {
                    if !merged.operations.contains(&operation) {
                        merged.operations.push(operation);
                    }
                }
                validate_material(&merged)?;
            }
            let mut operations = std::collections::HashSet::new();
            for record in &export.operations {
                record.require_scope(&backend, owner)?;
                if !operations.insert(record.operation_id.clone())
                    || !value.operations.contains(&record.operation_id)
                    || record.channel != "oracle"
                    || !secret_action(&record.request)
                    || record.request.get("secretSaltHex").is_some()
                    || record.prepared != json!({"privateCommitmentId":value.id})
                    || ["ownerPubkey", "marketId", "expiryId"]
                        .iter()
                        .any(|k| record.request[*k] != value.request[*k])
                {
                    return Err(error());
                }
                // Validate every identity before changing local files.
                let _ = path(&record.operation_id, "plan")?;
            }
            save(&file, &merged)?;
            // Revalidate the secret's encoding using the normal bounded loader.
            load(&value.id)?;
            for record in &export.operations {
                let _claim = operation_journal::claim(&record.operation_id)?;
                if !operation_journal::exists(&record.operation_id)? {
                    let mut restored = record.clone();
                    restored.state = if restored.signature.is_some() {
                        "transport_uncertain"
                    } else {
                        "preparation_not_restored"
                    }
                    .into();
                    operation_journal::save(&restored)?;
                }
            }
            Ok(
                json!({"ok":true,"commitmentId":value.id,"imported":true,"willSign":false,"willSubmit":false,"nextStep":"Recover the original commit signature. Prepare a fresh reveal with this commitment ID; imported status is not chain proof."}),
            )
        }
    }
}
