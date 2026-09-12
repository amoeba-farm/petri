//! Typed consumer of the pinned SDK worker. Never accepts caller-provided transactions.
use crate::content_hash::sha256_hex as hash;
use crate::{
    backend::{BackendClient, CliError},
    onchain::OnchainConfig,
    operation_journal::{self, OperationRecord},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde_json::{Value, json};
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;
use solana_transaction::versioned::VersionedTransaction;
use std::{
    str::FromStr,
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug)]
pub enum Family {
    Collateral,
    Oracle,
    Liquidity,
}
impl Family {
    pub fn name(self) -> &'static str {
        match self {
            Self::Collateral => "collateral",
            Self::Oracle => "oracle",
            Self::Liquidity => "liquidity",
        }
    }
    fn parse(name: &str) -> Result<Self, CliError> {
        match name {
            "collateral" => Ok(Self::Collateral),
            "oracle" => Ok(Self::Oracle),
            "liquidity" => Ok(Self::Liquidity),
            _ => Err(CliError::new(
                "This operation uses a different execution command.",
            )),
        }
    }
}
fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str, CliError> {
    v.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| CliError::new(format!("SDK plan omitted {key}")))
}
fn writable_context(config: &OnchainConfig) -> Result<(), CliError> {
    if crate::mcp_actions::preparation_blocked() {
        return Err(CliError::new(
            "Wallet preparation requires a typed public MCP action.",
        ));
    }
    crate::chain_identity::verify_onchain_config_fresh(config)?;
    Ok(())
}
fn instructions(v: &Value) -> Result<Vec<Instruction>, CliError> {
    v["instructions"]
        .as_array()
        .ok_or_else(|| CliError::new("SDK instruction inventory missing"))?
        .iter()
        .map(|ix| {
            let program_id = Pubkey::from_str(text(ix, "programId")?)
                .map_err(|_| CliError::new("SDK program invalid"))?;
            let data = BASE64
                .decode(text(ix, "dataBase64")?)
                .map_err(|_| CliError::new("SDK instruction invalid"))?;
            let accounts = ix["accounts"]
                .as_array()
                .ok_or_else(|| CliError::new("SDK accounts missing"))?
                .iter()
                .map(|a| {
                    Ok(AccountMeta {
                        pubkey: Pubkey::from_str(text(a, "pubkey")?)
                            .map_err(|_| CliError::new("SDK account invalid"))?,
                        is_signer: a["isSigner"]
                            .as_bool()
                            .ok_or_else(|| CliError::new("SDK signer role invalid"))?,
                        is_writable: a["isWritable"]
                            .as_bool()
                            .ok_or_else(|| CliError::new("SDK write role invalid"))?,
                    })
                })
                .collect::<Result<Vec<_>, CliError>>()?;
            Ok(Instruction {
                program_id,
                accounts,
                data,
            })
        })
        .collect()
}
fn govern(config: &OnchainConfig, owner: Pubkey, validated: &Value) -> Result<(), CliError> {
    let context = crate::current_operation::observe_current_write_context(config)?;
    ameba_sdk::validate_current_governed_instruction_batch_v1(
        &context,
        owner,
        &instructions(validated)?,
    )
    .map_err(|e| CliError::new(e.to_string()))?;
    Ok(())
}

pub fn prepare(
    config: &OnchainConfig,
    backend: &BackendClient,
    family: Family,
    request: Value,
) -> Result<Value, CliError> {
    writable_context(config)?;
    if config.backend_url != backend.base_url() {
        return Err(CliError::new(
            "SDK service identity differs from the configured service",
        ));
    }
    let material = if matches!(family, Family::Oracle) {
        crate::oracle_commitments::materialize(backend, &request)?
    } else {
        None
    };
    let request = material
        .as_ref()
        .map(|m| m.request.clone())
        .unwrap_or(request);
    let owner = text(&request, "ownerPubkey")?;
    if config.backend_url != backend.base_url() {
        return Err(CliError::new(
            "SDK service identity differs from the configured service",
        ));
    }
    if request.get("secretSaltHex").is_some() && material.is_none() {
        return Err(CliError::new(
            "Secret-bearing actions must use Petri's private commitment workflow.",
        ));
    }
    if matches!(family, Family::Oracle) {
        crate::participation::validate_request(&request)?;
    }
    // First extraction can be slow. Do it before acquiring an expiring preparation.
    crate::sdk_worker::warmup()?;
    let endpoint = match family {
        Family::Collateral => {
            let payer =
                Pubkey::from_str(owner).map_err(|_| CliError::new("Wallet address invalid"))?;
            let (address, _) = ameba_sdk::derive_user_collateral_pda(&ameba_sdk::ID, &payer);
            crate::endpoints::position_action(&address.to_string())
        }
        Family::Oracle => crate::endpoints::oracle_draft_prepare().into(),
        Family::Liquidity => crate::endpoints::dlmm_liquidity_prepare().into(),
    };
    let prepared = backend.post_json(&endpoint, &request).map_err(|error| {
        if material.is_some() { CliError::new("Oracle preparation failed. Private material was retained; no transaction was signed or submitted.") } else { error }
    })?;
    if prepared["ok"] != true {
        return Err(CliError::new("Amoeba did not prepare this action"));
    }
    let validated =
        crate::sdk_worker::validate(config, family.name(), owner, &request, &prepared, None)?;
    let payer = Pubkey::from_str(owner).map_err(|_| CliError::new("Wallet address invalid"))?;
    govern(config, payer, &validated)?;
    let id = text(&validated, "operationId")?.to_owned();
    let digest = text(&validated, "preparedPlanDigest")?.to_owned();
    let _claim = operation_journal::claim(&id)?;
    let payload = json!({"response":prepared,"validation":validated});
    let public_request = crate::oracle_commitments::public_request(&request);
    let journal_payload = if let Some(material) = &material {
        json!({"privateCommitmentId":material.id})
    } else {
        payload.clone()
    };
    let now = chrono::Utc::now().to_rfc3339();
    let record = OperationRecord {
        schema_version: 1,
        operation_id: id.clone(),
        prepared_plan_digest: digest,
        owner: owner.into(),
        origin: backend.base_url().into(),
        deployment: crate::current_release::PROGRAM_DATA_PAYLOAD_SHA256.into(),
        channel: family.name().into(),
        operation: request
            .get("actionType")
            .or_else(|| request.get("action"))
            .and_then(Value::as_str)
            .unwrap_or(family.name())
            .into(),
        state: "prepared".into(),
        created_at: now.clone(),
        updated_at: now,
        request: public_request.clone(),
        prepared: journal_payload,
        signature: None,
        transaction_sha256: None,
        message_sha256: None,
    };
    // Never overwrite a prior attempt when the service returns the same operation ID.
    if operation_journal::exists(&id)? {
        let previous = operation_journal::load(&id)?;
        previous.require_scope(backend, owner)?;
        if previous.state != "prepared"
            || previous.signature.is_some()
            || previous.prepared_plan_digest != record.prepared_plan_digest
            || previous.request != public_request
        {
            return Err(CliError::new(
                "This operation already exists or was attempted. Recover its status.",
            ));
        }
    }
    if let Some(material) = &material {
        crate::oracle_commitments::store_plan(material, &id, &payload)?;
    }
    operation_journal::save(&record)?;
    Ok(
        json!({"ok":true,"operation":record.public_value(),"review":public_request,"commitmentId":material.as_ref().map(|m|&m.id),"signing":{"willSign":false,"willSubmit":false},
        "nextStep":format!("petri operations execute {id} --yes")}),
    )
}

pub fn execute(
    config: &OnchainConfig,
    backend: &BackendClient,
    id: &str,
    yes: bool,
) -> Result<Value, CliError> {
    if !yes {
        return Err(CliError::new(
            "Review the prepared action, then pass --yes to approve that exact operation.",
        ));
    }
    writable_context(config)?;
    let peek = operation_journal::load(id)?;
    let _commitment_claim = crate::oracle_commitments::claim_for_execution(&peek)?;
    let _claim = operation_journal::claim(id)?;
    if config.backend_url != backend.base_url() {
        return Err(CliError::new(
            "SDK service identity differs from the configured service",
        ));
    }
    let mut record = operation_journal::load(id)?;
    let family = Family::parse(&record.channel)?;
    let owner = crate::wallet_signer::signer_pubkey(config)?;
    record.require_scope(backend, &owner)?;
    if record.state != "prepared" || record.signature.is_some() {
        return Err(CliError::new(
            "This operation was already attempted. Recover its original signature; no replay was made.",
        ));
    }
    if matches!(family, Family::Oracle) {
        crate::participation::validate_request(&record.request)?;
    }
    if record.request.get("secretSaltHex").is_some() {
        return Err(CliError::new(
            "Secrets cannot be supplied through an operation journal.",
        ));
    }
    let (request, payload) = crate::oracle_commitments::execution_payload(&record, backend)?;
    let response = &payload["response"];
    let initial = &payload["validation"];
    let validated = crate::sdk_worker::validate(
        config,
        family.name(),
        &owner,
        &request,
        response,
        Some(&initial["accounts"]),
    )?;
    if validated["operationId"] != id
        || validated["preparedPlanDigest"] != record.prepared_plan_digest
        || validated["messageSha256"] != initial["messageSha256"]
    {
        return Err(CliError::new(
            "The reviewed operation identity changed. Nothing was signed.",
        ));
    }
    crate::mcp_actions::authorize_execution(id, &record.prepared_plan_digest, &owner)?;
    let payer = Pubkey::from_str(&owner).map_err(|_| CliError::new("Wallet address invalid"))?;
    govern(config, payer, &validated)?;
    let bytes = BASE64
        .decode(text(&validated, "serializedTransactionBase64")?)
        .map_err(|_| CliError::new("SDK transaction encoding invalid"))?;
    if bytes.len() > 1232 {
        return Err(CliError::new(
            "Transaction exceeds the network packet limit",
        ));
    }
    let unsigned: VersionedTransaction =
        bincode::deserialize(&bytes).map_err(|_| CliError::new("SDK transaction invalid"))?;
    if unsigned.signatures.len() != 1
        || unsigned
            .signatures
            .iter()
            .any(|s| s.as_ref().iter().any(|b| *b != 0))
        || unsigned.message.static_account_keys().first() != Some(&payer)
        || hash(&unsigned.message.serialize()) != text(&validated, "messageSha256")?
    {
        return Err(CliError::new(
            "SDK transaction does not match the exact unsigned wallet message",
        ));
    }
    let signer = crate::wallet_signer::load_signer(config)?;
    crate::wallet_signer::require_admitted_signer(signer.as_ref(), &payer)?;
    let transaction = VersionedTransaction::try_new(unsigned.message, &[signer.as_ref()])
        .map_err(|_| CliError::new("Wallet did not approve the operation"))?;
    transaction
        .verify_and_hash_message()
        .map_err(|_| CliError::new("Wallet signature verification failed"))?;
    // Recheck the identical SDK plan and finalized business state after wallet approval.
    let after = crate::sdk_worker::validate(
        config,
        family.name(),
        &owner,
        &request,
        response,
        Some(&validated["accounts"]),
    )?;
    if after["messageSha256"] != validated["messageSha256"]
        || after["operationId"] != id
        || after["preparedPlanDigest"] != record.prepared_plan_digest
    {
        return Err(CliError::new(
            "Reviewed action changed during wallet approval",
        ));
    }
    govern(config, payer, &after)?;
    let signed = bincode::serialize(&transaction)
        .map_err(|_| CliError::new("Could not encode signed operation"))?;
    let signature = transaction.signatures[0].to_string();
    record.signature = Some(signature.clone());
    record.transaction_sha256 = Some(hash(&signed));
    record.message_sha256 = Some(hash(&transaction.message.serialize()));
    record.state = "transport_uncertain".into();
    record.updated_at = chrono::Utc::now().to_rfc3339();
    operation_journal::save(&record)?;
    let mut body = json!({"serializedTransactionBase64":BASE64.encode(&bytes),"signedTransactionBase64":BASE64.encode(&signed),"ownerPubkey":owner,
        "expectedSignature":signature,"expectedMessageSha256":record.message_sha256,"expectedTransactionSha256":record.transaction_sha256});
    for key in ["marketId", "expiryId"] {
        if let Some(value) = record.request.get(key) {
            body[key] = value.clone();
        }
    }
    // The hosted route accepts only a registered SDK preparation. One POST, never an automatic replay.
    let relay = backend.post_json(crate::endpoints::registered_transaction_submit(), &body);
    let started = Instant::now();
    loop {
        match crate::current_operation::recover_finalized_signature(backend, &record) {
            Ok(Some(("confirmed", evidence))) => {
                operation_journal::confirmed(id)?;
                return Ok(
                    json!({"ok":true,"operationId":id,"signature":signature,"state":"confirmed","evidence":evidence}),
                );
            }
            Ok(Some(("failed_on_chain", _))) => {
                operation_journal::failed_on_chain(id)?;
                return Err(CliError::confirmed_failure(
                    "The submitted operation failed on chain.",
                    id,
                    &signature,
                ));
            }
            _ => {}
        }
        if started.elapsed() > Duration::from_secs(35) {
            break;
        }
        thread::sleep(Duration::from_millis(700));
    }
    Err(CliError::uncertain(
        if relay.is_err() {
            "Submission outcome is uncertain. Recover the original signature; do not repeat this action."
        } else {
            "Submitted; finalized confirmation is pending. Recover the original signature."
        },
        id,
        &signature,
    ))
}

pub fn render(value: &Value) -> String {
    if let Some(review) = value.get("review") {
        let action = crate::participation::ACTIONS
            .iter()
            .copied()
            .find(|a| Some(a.wire()) == review.get("actionType").and_then(Value::as_str));
        let readable = if let Some(action) = action {
            let mut lines = vec![
                action.label().to_owned(),
                format!(
                    "Wallet: {}",
                    review["ownerPubkey"].as_str().unwrap_or("unavailable")
                ),
            ];
            if let Some(series) = review.get("expiryId").and_then(Value::as_str) {
                lines.push(format!("Series: {series}"));
            }
            let fields = match action {
                crate::participation::Action::RevealUpdate => {
                    crate::participation::Action::CommitUpdate.fields()
                }
                crate::participation::Action::RevealEmergencyVote => {
                    crate::participation::Action::CommitEmergencyVote.fields()
                }
                _ => action.fields(),
            };
            if let Some(id) = value["commitmentId"].as_str() {
                lines.push(format!("Private recovery reference: {id}"));
            }
            for (key, label, _) in fields {
                if let Some(field) = review.get(*key) {
                    let text = field
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| field.to_string());
                    if *key == "amount" {
                        if let Ok(amount) = text.parse::<u64>() {
                            lines.push(format!(
                                "USDC amount: {}.{:06}",
                                amount / 1_000_000,
                                amount % 1_000_000
                            ));
                        } else {
                            lines.push("USDC amount: invalid review; prepare again".into());
                        }
                    } else {
                        lines.push(format!("{label}: {text}"));
                    }
                }
            }
            lines.join("\n")
        } else {
            serde_json::to_string_pretty(review).unwrap_or_default()
        };
        format!(
            "Review action\n{}\n\nNothing signed or submitted.\n{}",
            crate::backend::terminal_safe_text(&readable),
            value["nextStep"].as_str().unwrap_or("")
        )
    } else {
        format!(
            "{}\nOperation: {}\nSignature: {}",
            value["state"].as_str().unwrap_or("unknown"),
            value["operationId"].as_str().unwrap_or(""),
            value["signature"].as_str().unwrap_or("")
        )
    }
}
