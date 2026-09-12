//! Durable recovery references. Stored plans are untrusted and revalidated by the SDK.
//! No key material, salts, or signed transaction packets are stored here.
use crate::{
    backend::{BackendClient, CliError},
    current_release, petri_config,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{fs, io::Read, path::PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationRecord {
    pub schema_version: u8,
    pub operation_id: String,
    pub prepared_plan_digest: String,
    pub owner: String,
    pub origin: String,
    pub deployment: String,
    pub channel: String,
    pub operation: String,
    pub state: String,
    pub created_at: String,
    pub updated_at: String,
    pub request: Value,
    pub prepared: Value,
    pub signature: Option<String>,
    pub transaction_sha256: Option<String>,
    pub message_sha256: Option<String>,
}
impl OperationRecord {
    pub fn require_scope(&self, backend: &BackendClient, owner: &str) -> Result<(), CliError> {
        if self.schema_version != 1
            || self.owner != owner
            || self.origin != backend.base_url()
            || self.deployment != current_release::PROGRAM_DATA_PAYLOAD_SHA256
        {
            return Err(CliError::new(
                "Operation belongs to another wallet, service, or deployment. Nothing was submitted.",
            ));
        }
        Ok(())
    }
    pub fn public_value(&self) -> Value {
        json!({"operationId":self.operation_id, "owner":self.owner,"deployment":self.deployment,
            "channel":self.channel,"operation":self.operation,"state":self.state,
            "createdAt":self.created_at,"updatedAt":self.updated_at,"signature":self.signature,
            "preparedPlanDigest":self.prepared_plan_digest,"retryAuthorized":false})
    }
}
fn root() -> Result<PathBuf, CliError> {
    let config = petri_config::config_path().map_err(CliError::new)?;
    Ok(config
        .parent()
        .ok_or_else(|| CliError::new("Petri data directory is unavailable."))?
        .join("operations"))
}
fn record_path(id: &str) -> Result<PathBuf, CliError> {
    if id.len() != 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(CliError::new(
            "Operation ID must be a canonical 64-character digest.",
        ));
    }
    Ok(root()?.join(format!("{id}.json")))
}
pub fn save(record: &OperationRecord) -> Result<(), CliError> {
    let bytes = serde_json::to_vec(record)
        .map_err(|_| CliError::new("Could not encode operation reference."))?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err(CliError::new("Operation reference exceeds its size bound."));
    }
    petri_config::write_private_file(&record_path(&record.operation_id)?, &bytes)
        .map_err(CliError::new)
}
pub fn exists(id: &str) -> Result<bool, CliError> {
    Ok(record_path(id)?
        .try_exists()
        .map_err(|_| CliError::new("Could not inspect the operation reference"))?)
}
pub fn load(id: &str) -> Result<OperationRecord, CliError> {
    let path = record_path(id)?;
    let metadata = fs::symlink_metadata(&path)
        .map_err(|_| CliError::new("No local recovery reference for this operation."))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 2 * 1024 * 1024
    {
        return Err(CliError::new(
            "Operation reference is not a bounded regular file.",
        ));
    }
    let file =
        fs::File::open(&path).map_err(|_| CliError::new("Could not open operation reference."))?;
    let mut bytes = Vec::new();
    file.take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| CliError::new("Could not read operation reference."))?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err(CliError::new("Operation reference exceeds its size bound."));
    }
    let value: OperationRecord = serde_json::from_slice(&bytes)
        .map_err(|_| CliError::new("Operation reference is malformed."))?;
    if value.schema_version != 1 || value.operation_id != id {
        return Err(CliError::new("Operation reference identity mismatch."));
    }
    Ok(value)
}
pub fn list(owner: Option<&str>) -> Result<Value, CliError> {
    let entries = match fs::read_dir(root()?) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(json!({"ok":true,"operations":[]}));
        }
        Err(_) => return Err(CliError::new("Could not read the operation journal.")),
    };
    let mut records = Vec::new();
    let mut issues = Vec::new();
    let mut exhaustive = true;
    for (index, entry) in entries.take(4097).enumerate() {
        if index == 4096 {
            exhaustive = false;
            break;
        }
        let entry = entry.map_err(|_| CliError::new("Could not enumerate recovery records."))?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        let record = match load(id) {
            Ok(record) => record,
            Err(_) => {
                issues.push("An unreadable local operation record was omitted.");
                continue;
            }
        };
        if owner.is_none_or(|o| record.owner == o) {
            records.push(record.public_value());
        }
    }
    records.sort_by(|a, b| b["updatedAt"].as_str().cmp(&a["updatedAt"].as_str()));
    Ok(
        json!({"ok":true,"operations":records,"issues":issues,"coverage":{"exhaustive":exhaustive},"retryAuthorized":false}),
    )
}
pub fn record_prepared(
    backend: &BackendClient,
    request: &Value,
    response: &Value,
    admitted: &ameba_sdk::CurrentGovernedOperationV1,
) -> Result<String, CliError> {
    let id = admitted.operation_id().to_owned();
    let _claim = claim(&id)?;
    if record_path(&id)?.exists() {
        let previous = load(&id)?;
        previous.require_scope(backend, &admitted.payer().to_string())?;
        if previous.prepared_plan_digest != admitted.prepared_plan_digest()
            || previous.request != *request
        {
            return Err(CliError::new("Operation reference changed identity."));
        }
        if previous.state != "prepared" || previous.signature.is_some() {
            return Err(CliError::new(
                "This operation was already attempted. Recover the original operation; no fresh preparation or replay was authorized.",
            ));
        }
        return Ok(id);
    }
    let now = chrono::Utc::now().to_rfc3339();
    save(&OperationRecord {
        schema_version: 1,
        operation_id: id.clone(),
        prepared_plan_digest: admitted.prepared_plan_digest().to_owned(),
        owner: admitted.payer().to_string(),
        origin: backend.base_url().to_owned(),
        deployment: current_release::PROGRAM_DATA_PAYLOAD_SHA256.to_owned(),
        channel: if admitted.swap_operation().is_some() {
            "trade"
        } else {
            "writer"
        }
        .into(),
        operation: if admitted.swap_operation().is_some() {
            "collective_swap_exact_in".into()
        } else if let Some(writer) = admitted.writer_operation() {
            crate::writer_operation_wire_name(writer.plan.operation).into()
        } else if let Some(flat) = admitted.flat_operation() {
            flat.plan.operation.clone()
        } else {
            return Err(CliError::new("Unknown governed operation family."));
        },
        state: "prepared".into(),
        created_at: now.clone(),
        updated_at: now,
        request: request.clone(),
        prepared: response.clone(),
        signature: None,
        transaction_sha256: None,
        message_sha256: None,
    })?;
    Ok(id)
}

pub struct SubmissionClaim {
    _file: fs::File,
}
pub fn claim(id: &str) -> Result<SubmissionClaim, CliError> {
    let path = record_path(id)?.with_extension("lock");
    let file = petri_config::open_private_lock_file(&path).map_err(CliError::new)?;
    file.try_lock().map_err(|_| CliError::coded("OPERATION_BUSY", "pending",
        "Another Petri context is working on this operation. Wait for it, then recover status; do not resubmit.",false))?;
    Ok(SubmissionClaim { _file: file })
}

pub fn before_relay(
    backend: &BackendClient,
    admitted: &ameba_sdk::CurrentGovernedOperationV1,
    operation: &str,
    signature: &str,
    transaction_hash: &str,
    message_hash: &str,
) -> Result<(), CliError> {
    let id = admitted.operation_id();
    if !record_path(id)?.exists() {
        record_prepared(backend, &Value::Null, &Value::Null, admitted)?;
    }
    let mut record = load(id)?;
    record.require_scope(backend, &admitted.payer().to_string())?;
    if record.state != "prepared"
        || record.signature.is_some()
        || record.prepared_plan_digest != admitted.prepared_plan_digest()
    {
        return Err(CliError::new(
            "This operation already has a submission attempt. Recover its status before any fresh action.",
        ));
    }
    record.operation = operation.to_owned();
    record.state = "transport_uncertain".into();
    record.signature = Some(signature.to_owned());
    record.transaction_sha256 = Some(transaction_hash.to_owned());
    record.message_sha256 = Some(message_hash.to_owned());
    record.updated_at = chrono::Utc::now().to_rfc3339();
    save(&record)
}
pub fn reserve(
    backend: &BackendClient,
    admitted: &ameba_sdk::CurrentGovernedOperationV1,
) -> Result<SubmissionClaim, CliError> {
    if !record_path(admitted.operation_id())?.exists() {
        record_prepared(backend, &Value::Null, &Value::Null, admitted)?;
    }
    let claim = claim(admitted.operation_id())?;
    let record = load(admitted.operation_id())?;
    record.require_scope(backend, &admitted.payer().to_string())?;
    if record.state != "prepared" || record.signature.is_some() {
        return Err(CliError::new(
            "This operation has already been attempted. Recover its status; no replay was made.",
        ));
    }
    Ok(claim)
}
pub fn confirmed(id: &str) -> Result<(), CliError> {
    let mut record = load(id)?;
    record.state = "confirmed".into();
    record.updated_at = chrono::Utc::now().to_rfc3339();
    save(&record)
}
pub fn failed_on_chain(id: &str) -> Result<(), CliError> {
    let mut record = load(id)?;
    record.state = "failed_on_chain".into();
    record.updated_at = chrono::Utc::now().to_rfc3339();
    save(&record)
}
pub fn recover(backend: &BackendClient, id: &str) -> Result<Value, CliError> {
    let _claim = claim(id)?;
    let mut record = load(id)?;
    record.require_scope(backend, &record.owner)?;
    if matches!(
        record.channel.as_str(),
        "collateral" | "oracle" | "liquidity"
    ) {
        if record.signature.is_some() {
            if let Some((state, _)) =
                crate::current_operation::recover_finalized_signature(backend, &record)?
            {
                record.state = state.into();
                record.updated_at = chrono::Utc::now().to_rfc3339();
                save(&record)?;
            }
        }
        return Ok(
            json!({"ok":true,"operation":record.public_value(),"retryAuthorized":false,
            "nextStep":if record.state=="prepared" {"Review the exact operation before approving it."}else if record.state=="confirmed" {"Refresh balances and positions."}else{"Recover the original signature; do not replay this action."}}),
        );
    }
    let path = match record.channel.as_str() {
        "trade" => crate::endpoints::dlmm_trade_status(id),
        "writer" => crate::endpoints::writer_operation_status(id),
        _ => return Err(CliError::new("Unsupported operation recovery channel.")),
    };
    let status = backend.get(&path).and_then(|response| {
        crate::current_operation::validate_recovered_operation(&record, &response)
            .map(|state| (state, response))
    });
    let (state, response) = match status {
        Ok(("confirmed", response)) => ("confirmed", response),
        other if record.signature.is_some() => {
            // An absent/expired server record is not proof of non-execution.
            // Reconcile the original packet on the identity-verified finalized chain.
            let onchain = crate::current_operation::recover_finalized_signature(backend, &record);
            match onchain {
                Ok(Some((state, evidence))) => (state, evidence),
                Ok(None) => (
                    "transport_uncertain",
                    json!({"source":"finalized_chain","found":false,
                    "message":"Original signature is not visible; this does not authorize a retry."}),
                ),
                Err(error) => match other {
                    Ok((state, response)) => (state, response),
                    Err(status_error) => (
                        "transport_uncertain",
                        json!({"statusIssue":status_error.to_string(),"chainIssue":error.to_string()}),
                    ),
                },
            }
        }
        other => other?,
    };
    record.state = state.to_owned();
    record.updated_at = chrono::Utc::now().to_rfc3339();
    save(&record)?;
    Ok(
        json!({"ok":true,"operation":record.public_value(),"status":response,
        "nextStep":if state == "confirmed" {"Refresh positions."} else {"Reconcile this operation and known signature before preparing another action."},"retryAuthorized":false}),
    )
}
