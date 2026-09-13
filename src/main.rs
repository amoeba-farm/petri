#![recursion_limit = "256"]

mod agent_protocol;
mod app_context;
mod attached_wallet;
mod backend;
mod cache;
mod catalog;
mod chain_identity;
mod chart;
mod cli;
mod content_hash;
mod current_operation;
mod current_release;
mod endpoints;
mod gitbook;
mod guide;
mod lab;
mod market_surface;
mod mcp_actions;
mod mcp_setup;
mod onchain;
mod operation_journal;
mod oracle_carry;
mod oracle_commitments;
mod oracle_lifecycle;
mod oracle_recipe_weights;
mod oracle_submissions;
mod oracle_tui;
mod participation;
mod petri_config;
mod portable_operation;
mod positions;
mod release_update;
mod request_validation;
mod sdk_worker;
mod solana_config;
mod solana_history;
mod solana_rpc;
mod spread_oracle_plan;
mod staking;
mod terminal_brand;
mod terminal_keys;
mod trade_service;
mod update;
mod wallet_balance;
mod wallet_signer;
mod wallet_terms;
mod windows_app_identity;
mod workspace_update;
mod writer_action_mask;
mod writer_liquidity;
mod writer_output;

use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    io::{self, IsTerminal},
    path::PathBuf,
    process::Command as ProcessCommand,
    str::FromStr,
};

use app_context::build_onchain_config;
use backend::{
    BackendClient, CliError, array_at_key, current_backend_payload, json_string, string_at_key,
    terminal_safe_text, unwrap_data, value_at_key,
};
use clap::{CommandFactory, FromArgMatches};
use cli::{
    ChartArgs, Cli, CollectiveTradeArgs, Command, ConfigCommand, ConfigKey, ContractsArgs,
    HistoryArgs, HistoryTypeValue, LiquidityActionValue, LiquidityArgs, LiquidityCommand,
    MarketsCommand, McpCommand, OptionsChainArgs, OracleAmbaCommand, OracleAmbaCustodyArgs,
    OracleChallengeDraftArgs, OracleCommand, OracleDraftCommonArgs, OracleDraftsCommand,
    OracleEmergencyCommand, OracleEmergencyCommitArgs, OracleEmergencyRevealArgs,
    OracleOpeningChallengeArgs, OracleOpeningClaimArgs, OraclePrintsCommand, OracleRewardClaimArgs,
    OracleRewardsCommand, OracleSourceCommand, OracleSourceProposeArgs, OracleSourceSupportArgs,
    OracleStakeSettleArgs, OracleStakesCommand, OracleUpdateChallengeArgs, OracleUpdateCommitArgs,
    OracleUpdateExpireArgs, OracleUpdateRevealArgs, OracleUpdatesCommand, OutputFormat,
    SettlementsCommand, StakingCommand, TradesCommand, UpdateCommand, WalletCommand, WriterCommand,
};
use onchain::OnchainConfig;
use oracle_submissions::{OracleSubmissionDraft, OracleSubmissionField};
use request_validation::{canonical_pubkey_string, canonical_u64_string};
use serde_json::{Value, json};
use solana_program::hash::hashv;
use solana_pubkey::Pubkey;

fn main() {
    if let Some(result) = release_update::helper_entry() {
        if let Err(error) = result {
            eprintln!("Petri update: {error}");
            std::process::exit(1);
        }
        return;
    }
    windows_app_identity::apply();
    if let Err(error) = run() {
        let arguments = env::args().collect::<Vec<_>>();
        if arguments.iter().any(|a| a == "--json")
            || arguments
                .windows(2)
                .any(|a| a[0] == "--output" && a[1] == "json")
            || env::var("AMEBA_OUTPUT").ok().as_deref() == Some("json")
        {
            println!("{}", error.json());
            std::process::exit(error.exit_code());
        }
        if error.is_current_state_wait() {
            eprintln!("waiting: {error}");
            // A temporary finalized-observation wait is not a locally recoverable
            // prepare error. Keep the process non-successful so callers cannot
            // mistake the absence of a transaction target for a prepared plan.
            std::process::exit(75);
        }
        eprintln!("error: {error}");
        std::process::exit(error.exit_code());
    }
}

fn render_collective_trade_operation(label: &str, response: &Value) -> String {
    let data = unwrap_data(response);
    let plan = data
        .get("operationPlan")
        .or_else(|| data.get("plan"))
        .unwrap_or(data);
    let semantic = plan.get("semantic").unwrap_or(&Value::Null);
    let direction = string_at_key(semantic, &["direction"]).unwrap_or_else(|| "-".to_string());
    let amount_in = string_at_key(semantic, &["amountIn"]).unwrap_or_else(|| "-".to_string());
    let minimum_amount_out =
        string_at_key(semantic, &["minimumAmountOut"]).unwrap_or_else(|| "-".to_string());
    let deadline = string_at_key(semantic, &["deadlineTs"]).unwrap_or_else(|| "-".to_string());
    let operation_id = string_at_key(plan, &["operationId"]).unwrap_or_else(|| "-".to_string());
    format!(
        "{label}\nDirection: {direction}\nExact input atoms: {amount_in}\nMinimum output atoms: {minimum_amount_out}\nDeadline: {deadline}\nOperation: {operation_id}\nNothing has been signed or submitted."
    )
}

fn run_collective_trade_prepare_command(
    cli: &Cli,
    backend: &BackendClient,
    trade: &CollectiveTradeArgs,
) -> Result<(), CliError> {
    let trade_service::PreparedTrade {
        request,
        response,
        admitted,
        ..
    } = trade_service::prepare_exact_input(cli, backend, trade)?;
    if mcp_actions::preparing() {
        let id = operation_journal::record_prepared(backend, &request, &response, &admitted)?;
        let payload = json!({"ok":true,"operation":operation_journal::load(&id)?.public_value(),
            "review":request,"signing":{"willSign":false,"willSubmit":false}});
        return emit_output(
            cli,
            &payload,
            render_collective_trade_operation("Collective swap prepared", &response),
        );
    }
    emit_output(
        cli,
        &response,
        render_collective_trade_operation("Collective swap prepared", &response),
    )
}

fn run_collective_trade_submit_command(
    cli: &Cli,
    backend: &BackendClient,
    trade: &CollectiveTradeArgs,
) -> Result<(), CliError> {
    let trade_service::PreparedTrade {
        config,
        response: _,
        request: _,
        admitted,
    } = trade_service::prepare_exact_input(cli, backend, trade)?;
    let validated = admitted
        .swap_operation()
        .ok_or_else(|| CliError::new("The SDK did not admit a collective swap."))?;
    let deadline = validated
        .plan
        .semantic
        .deadline_ts
        .parse::<u64>()
        .map_err(|_| CliError::new("collective trade plan contains an invalid deadline"))?;
    let status_path = endpoints::dlmm_trade_status(&validated.plan.operation_id);
    let receipt = current_operation::sign_submit_validated_operation(
        &config,
        backend,
        endpoints::dlmm_trade_submit(),
        &status_path,
        "collective_swap_exact_in",
        &admitted,
        Some(deadline),
    )?;
    let payload = json!({
        "operation": "collective-secondary-trade",
        "status": "confirmed",
        "direction": trade.direction.as_request_value(),
        "amountIn": trade.amount_in,
        "minimumAmountOut": trade.minimum_amount_out,
        "receipt": receipt,
    });
    emit_output(
        cli,
        &payload,
        format!(
            "Collective trade confirmed | direction={} | exact input atoms={} | minimum output atoms={} | signature={}",
            trade.direction.as_request_value(),
            trade.amount_in,
            trade.minimum_amount_out,
            payload["receipt"]["signature"].as_str().unwrap_or("-")
        ),
    )
}

fn writer_write_request(owner: &str, body: Value) -> Result<Value, CliError> {
    let mut object = body
        .as_object()
        .cloned()
        .ok_or_else(|| CliError::new("writer request must be an object"))?;
    object.insert("owner".to_string(), Value::String(owner.to_string()));
    Ok(Value::Object(object))
}

fn writer_preview_post(
    cli: &Cli,
    backend: &BackendClient,
    endpoint: &str,
    request: Value,
) -> Result<(), CliError> {
    let response =
        current_backend_payload(backend.post_json_with_current_state_retry(endpoint, &request)?)?;
    validate_writer_close_preview_response(&response, &request)?;
    emit_output(
        cli,
        &response,
        writer_output::render_writer_close_preview(&response),
    )
}

fn writer_required_object<'a>(
    value: &'a Value,
    key: &str,
    label: &str,
) -> Result<&'a serde_json::Map<String, Value>, CliError> {
    value
        .get(key)
        .and_then(Value::as_object)
        .ok_or_else(|| CliError::new(format!("writer response is missing {label}")))
}

fn validate_writer_sleeve_identity(
    response: &Value,
    expected_sleeve: &str,
) -> Result<(), CliError> {
    let sleeve = writer_required_object(unwrap_data(response), "sleeve", "sleeve state")?;
    if sleeve.get("address").and_then(Value::as_str) != Some(expected_sleeve) {
        return Err(CliError::new(
            "writer response sleeve does not match the requested canonical sleeve",
        ));
    }
    Ok(())
}

fn validate_writer_close_request_identity(
    response: &Value,
    expected_request: &str,
) -> Result<(), CliError> {
    let request =
        writer_required_object(unwrap_data(response), "closeRequest", "close-request state")?;
    if request.get("address").and_then(Value::as_str) != Some(expected_request) {
        return Err(CliError::new(
            "writer response close request does not match the requested canonical address",
        ));
    }
    Ok(())
}

fn validate_writer_close_status_response(
    response: &Value,
    expected_request: &str,
) -> Result<(), CliError> {
    validate_writer_close_request_identity(response, expected_request)?;
    writer_output::validate_current_writer_close_status(response)
        .map_err(|issue| CliError::new(issue.to_string()))
}

fn validate_writer_decimal_text(value: Option<&Value>, label: &str) -> Result<(), CliError> {
    let raw = value
        .and_then(Value::as_str)
        .ok_or_else(|| CliError::new(format!("writer close preview is missing {label}")))?;
    if raw != "0" && (raw.starts_with('0') || !raw.as_bytes().iter().all(u8::is_ascii_digit)) {
        return Err(CliError::new(format!(
            "writer close preview {label} is not canonical unsigned decimal text"
        )));
    }
    if raw.is_empty() {
        return Err(CliError::new(format!(
            "writer close preview {label} is not canonical unsigned decimal text"
        )));
    }
    Ok(())
}

fn validate_writer_decimal_array(value: Option<&Value>, label: &str) -> Result<(), CliError> {
    let values = value
        .and_then(Value::as_array)
        .ok_or_else(|| CliError::new(format!("writer close preview is missing {label}")))?;
    for value in values {
        validate_writer_decimal_text(Some(value), label)?;
    }
    Ok(())
}

fn validate_writer_close_preview_response(
    response: &Value,
    request: &Value,
) -> Result<(), CliError> {
    const SEMANTIC_KEYS: &[&str] = &["sleeve", "flatParAtoms", "minimumWithdrawalAtoms"];
    const PREVIEW_DECIMALS: &[&str] = &[
        "reserveBefore",
        "reserveAfter",
        "lowerTailReserveBefore",
        "lowerTailReserveAfter",
        "upperTailReserveBefore",
        "upperTailReserveAfter",
        "reserveReduction",
        "minimumSafeWithdrawal",
        "statewiseBindingSettlement",
        "candidateCount",
        "withdrawal",
        "retainedWriterSurplus",
        "securityAfter",
    ];
    const PREVIEW_ARRAYS: &[&str] = &[
        "required",
        "remainingExternalOi",
        "candidateSettlements",
        "candidateSafeWithdrawals",
    ];

    let request = request
        .as_object()
        .ok_or_else(|| CliError::new("writer close preview request must be an object"))?;
    if request.len() != SEMANTIC_KEYS.len()
        || SEMANTIC_KEYS.iter().any(|key| !request.contains_key(*key))
    {
        return Err(CliError::new(
            "writer close preview request does not match the exact current semantic input",
        ));
    }
    let data = unwrap_data(response);
    let semantic = writer_required_object(data, "semantic", "close-preview semantic")?;
    if semantic.len() != SEMANTIC_KEYS.len()
        || SEMANTIC_KEYS
            .iter()
            .any(|key| semantic.get(*key) != request.get(*key))
    {
        return Err(CliError::new(
            "writer close preview semantic does not match the exact request",
        ));
    }
    let observation = writer_required_object(data, "currentObservation", "current observation")?;
    validate_writer_decimal_text(observation.get("observedAtSlot"), "observedAtSlot")?;

    let preview = writer_required_object(data, "preview", "verified close preview")?;
    if preview.get("ok") != Some(&Value::Bool(true)) {
        return Err(CliError::new(
            "writer close preview did not confirm a verified result",
        ));
    }
    for key in PREVIEW_DECIMALS {
        validate_writer_decimal_text(preview.get(*key), key)?;
    }
    for key in PREVIEW_ARRAYS {
        validate_writer_decimal_array(preview.get(*key), key)?;
    }
    let commitments = preview
        .get("commitments")
        .and_then(Value::as_object)
        .ok_or_else(|| CliError::new("writer response is missing close-preview commitments"))?;
    for key in ["group", "book", "policy", "deployment"] {
        let digest = commitments
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| {
                CliError::new(format!("writer close preview is missing {key} commitment"))
            })?;
        if digest.len() != 64
            || !digest
                .as_bytes()
                .iter()
                .all(|byte| byte.is_ascii_digit() || matches!(*byte, b'a'..=b'f'))
        {
            return Err(CliError::new(format!(
                "writer close preview {key} commitment is not a lowercase SHA-256 digest"
            )));
        }
    }
    Ok(())
}

fn writer_operation_plan<'a>(response: &'a Value) -> Result<&'a Value, CliError> {
    let data = unwrap_data(response);
    data.get("operationPlan")
        .or_else(|| data.get("plan"))
        .ok_or_else(|| CliError::new("writer response is missing operationPlan"))
}

#[derive(Clone, Copy)]
enum WriterCloseSubmission<'a> {
    None,
    Begin {
        asserted_request: Option<&'a str>,
    },
    Forward {
        close_request: &'a str,
    },
    // Reserved for a future reviewed wallet action ABI. The current public
    // cancel seam fails closed before preparation or wallet access.
    #[allow(dead_code)]
    Cancel {
        close_request: &'a str,
    },
}

fn exact_writer_stage<'a>(
    response: &'a Value,
) -> Result<&'a serde_json::Map<String, Value>, CliError> {
    let stage = unwrap_data(response)
        .get("stage")
        .and_then(Value::as_object)
        .ok_or_else(|| CliError::new("writer close response is missing its Lean-selected stage"))?;
    Ok(stage)
}

fn exact_stage_fields(
    stage: &serde_json::Map<String, Value>,
    expected: &[&str],
) -> Result<(), CliError> {
    let actual = stage.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(CliError::new(
            "writer close response contains a noncanonical stage shape",
        ));
    }
    Ok(())
}

fn validated_close_request_account(
    validated: &ameba_sdk::ValidatedWriterOperation,
) -> Result<String, CliError> {
    let instruction = validated.instructions.first().ok_or_else(|| {
        CliError::new("writer close operationPlan contains no validated instruction")
    })?;
    let index = match validated.plan.operation {
        ameba_sdk::WriterOperationKind::CloseBegin => 6,
        ameba_sdk::WriterOperationKind::CloseBasket => 3,
        ameba_sdk::WriterOperationKind::CloseFinalize => 6,
        ameba_sdk::WriterOperationKind::CloseCancel
            if instruction.data.get(1) == Some(&u8::MAX) =>
        {
            2
        }
        ameba_sdk::WriterOperationKind::CloseCancel => 3,
        _ => {
            return Err(CliError::new(
                "writer operationPlan is not a staged-close instruction",
            ));
        }
    };
    instruction
        .accounts
        .get(index)
        .map(|meta| meta.pubkey.to_string())
        .ok_or_else(|| CliError::new("writer close instruction omits its canonical request"))
}

fn validate_writer_close_submission(
    response: &Value,
    validated: &ameba_sdk::ValidatedWriterOperation,
    flow: WriterCloseSubmission<'_>,
) -> Result<Option<Value>, CliError> {
    let actor = validated
        .instructions
        .first()
        .and_then(|instruction| instruction.accounts.first())
        .filter(|meta| meta.is_signer)
        .map(|meta| meta.pubkey.to_string())
        .ok_or_else(|| CliError::new("writer close instruction omits its admitted signer"))?;
    if validated.plan.semantic.get("owner").and_then(Value::as_str) != Some(actor.as_str()) {
        return Err(CliError::new(
            "writer close instruction signer differs from its admitted actor",
        ));
    }
    match flow {
        WriterCloseSubmission::None => Ok(None),
        WriterCloseSubmission::Begin { asserted_request } => {
            if validated.plan.operation != ameba_sdk::WriterOperationKind::CloseBegin {
                return Err(CliError::new(
                    "writer close start returned the wrong operation",
                ));
            }
            let request = unwrap_data(response)
                .get("closeRequest")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    CliError::new("writer close start did not return its canonical request")
                })?;
            let request = canonical_pubkey_string(request, "close-request")?;
            if asserted_request.is_some_and(|asserted| asserted != request) {
                return Err(CliError::new(
                    "--close-request does not match the Edge-derived close request; nothing was signed or sent",
                ));
            }
            if validated_close_request_account(validated)? != request {
                return Err(CliError::new(
                    "writer close response and validated instruction disagree on the request",
                ));
            }
            Ok(Some(json!({
                "intent": "begin",
                "closeRequest": request,
                "stage": { "operation": "begin" },
            })))
        }
        WriterCloseSubmission::Forward { close_request }
        | WriterCloseSubmission::Cancel { close_request } => {
            if validated_close_request_account(validated)? != close_request {
                return Err(CliError::new(
                    "writer close request differs from the validated instruction",
                ));
            }
            let stage = exact_writer_stage(response)?;
            let instruction = validated.instructions.first().ok_or_else(|| {
                CliError::new("writer close operationPlan contains no validated instruction")
            })?;
            let operation = stage
                .get("operation")
                .and_then(Value::as_str)
                .ok_or_else(|| CliError::new("writer close stage is missing its operation"))?;
            match (flow, validated.plan.operation, operation) {
                (
                    WriterCloseSubmission::Forward { .. },
                    ameba_sdk::WriterOperationKind::CloseBasket,
                    "deposit_basket",
                ) => {
                    exact_stage_fields(stage, &["operation", "seriesIndex"])?;
                    let index = stage
                        .get("seriesIndex")
                        .and_then(Value::as_u64)
                        .filter(|value| *value < 20)
                        .ok_or_else(|| CliError::new("writer close basket index is invalid"))?;
                    if instruction.data.get(1).copied().map(u64::from) != Some(index) {
                        return Err(CliError::new(
                            "Lean-selected basket index differs from the validated instruction",
                        ));
                    }
                }
                (
                    WriterCloseSubmission::Forward { .. },
                    ameba_sdk::WriterOperationKind::CloseFinalize,
                    "finalize",
                ) => exact_stage_fields(stage, &["operation"])?,
                (
                    WriterCloseSubmission::Cancel { .. },
                    ameba_sdk::WriterOperationKind::CloseCancel,
                    "cancel_series",
                ) => {
                    exact_stage_fields(stage, &["operation", "seriesIndex"])?;
                    let index = stage
                        .get("seriesIndex")
                        .and_then(Value::as_u64)
                        .filter(|value| *value < 20)
                        .ok_or_else(|| {
                            CliError::new("writer close cancellation index is invalid")
                        })?;
                    if instruction.data.get(1).copied().map(u64::from) != Some(index) {
                        return Err(CliError::new(
                            "Lean-selected cancellation index differs from the validated instruction",
                        ));
                    }
                }
                (
                    WriterCloseSubmission::Cancel { .. },
                    ameba_sdk::WriterOperationKind::CloseCancel,
                    "cancel_flat",
                ) => {
                    exact_stage_fields(stage, &["operation"])?;
                    if instruction.data.get(1) != Some(&u8::MAX) {
                        return Err(CliError::new(
                            "Lean selected the Flat refund but the validated instruction did not",
                        ));
                    }
                }
                _ => {
                    return Err(CliError::new(
                        "Lean-selected writer close stage differs from the SDK-validated operation",
                    ));
                }
            }
            Ok(Some(json!({
                "intent": match flow {
                    WriterCloseSubmission::Forward { .. } => "forward",
                    WriterCloseSubmission::Cancel { .. } => "cancel",
                    _ => unreachable!(),
                },
                "closeRequest": close_request,
                "stage": Value::Object(stage.clone()),
            })))
        }
    }
}

fn writer_close_execution_deadline(
    response: &Value,
    validated: &ameba_sdk::ValidatedWriterOperation,
    flow: WriterCloseSubmission<'_>,
) -> Result<Option<u64>, CliError> {
    match flow {
        WriterCloseSubmission::None | WriterCloseSubmission::Cancel { .. } => Ok(None),
        WriterCloseSubmission::Begin { .. } => {
            let instruction = validated.instructions.first().ok_or_else(|| {
                CliError::new("writer close start contains no validated instruction")
            })?;
            let encoded = instruction
                .data
                .get(instruction.data.len().saturating_sub(8)..)
                .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
                .map(u64::from_le_bytes)
                .filter(|deadline| *deadline > 0)
                .ok_or_else(|| CliError::new("writer close start has no valid request deadline"))?;
            Ok(Some(encoded))
        }
        WriterCloseSubmission::Forward { .. } => {
            let data = unwrap_data(response);
            let admission = data
                .get("leanAdmission")
                .filter(|value| value.is_object())
                .ok_or_else(|| {
                    CliError::new("writer close advance lacks its exact Lean admission")
                })?;
            let reported_digest = data
                .get("leanAdmissionDigest")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    CliError::new("writer close advance lacks its Lean admission digest")
                })?;
            let admission_digest = writer_close_admission_digest(admission)?;
            if reported_digest != validated.plan.lean_admission_digest
                || admission_digest != validated.plan.lean_admission_digest
            {
                return Err(CliError::new(
                    "writer close advance deadline is not bound to the SDK-validated Lean admission; nothing was signed or sent",
                ));
            }
            let raw = admission
                .pointer("/facts/requestDeadlineUnixSeconds")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    CliError::new("writer close advance lacks its Lean-admitted request deadline")
                })?;
            let canonical = canonical_u64_string(raw, "request deadline", false)?;
            let deadline = canonical
                .parse::<u64>()
                .map_err(|_| CliError::new("writer close request deadline is invalid"))?;
            Ok(Some(deadline))
        }
    }
}

fn writer_close_admission_digest(admission: &Value) -> Result<String, CliError> {
    fn canonical(value: &Value) -> Value {
        match value {
            Value::Object(object) => {
                let mut keys = object.keys().collect::<Vec<_>>();
                keys.sort();
                let mut sorted = serde_json::Map::new();
                for key in keys {
                    sorted.insert(key.clone(), canonical(&object[key]));
                }
                Value::Object(sorted)
            }
            Value::Array(values) => Value::Array(values.iter().map(canonical).collect()),
            other => other.clone(),
        }
    }

    let bytes = serde_json::to_vec(&canonical(admission))
        .map_err(|_| CliError::new("writer close Lean admission could not be canonicalized"))?;
    Ok(lower_hex(&hashv(&[bytes.as_slice()]).to_bytes()))
}

fn submit_writer_operation(
    cli: &Cli,
    backend: &BackendClient,
    endpoint: &str,
    body: Value,
    allowed_operations: &[ameba_sdk::WriterOperationKind],
    label: &str,
    close_submission: WriterCloseSubmission<'_>,
) -> Result<(), CliError> {
    current_release::require_current_write_release()?;
    let config = build_onchain_config(cli)?;
    let context = current_operation::observe_current_write_context(&config)?;
    let owner = wallet_signer::signer_pubkey(&config)?;
    let request = writer_write_request(&owner, body)?;
    let response =
        current_backend_payload(backend.post_json_with_current_state_retry(endpoint, &request)?)?;
    let plan = writer_operation_plan(&response)?;
    let encoded = serde_json::to_string(plan)
        .map_err(|_| CliError::new("writer operationPlan could not be encoded"))?;
    let unvalidated_plan: ameba_sdk::WriterOperationPlan = serde_json::from_str(&encoded)
        .map_err(|_| CliError::new("writer operationPlan is not canonical JSON"))?;
    let setup = current_operation::rebuild_writer_operation_setup(&config, &unvalidated_plan)?;
    let admitted =
        ameba_sdk::parse_current_governed_writer_operation_json_v1(&context, &encoded, &setup)
            .map_err(|error| {
                CliError::new(format!(
                    "writer operationPlan failed pinned SDK validation: {error}"
                ))
            })?;
    let validated = admitted
        .writer_operation()
        .ok_or_else(|| CliError::new("The SDK did not admit a Writer operation."))?;
    if !allowed_operations.contains(&validated.plan.operation) {
        return Err(CliError::new(
            "writer operationPlan selected an action outside the requested flow",
        ));
    }
    ameba_sdk::require_expected_writer_semantic(&validated, validated.plan.operation, &request)
        .map_err(|error| {
            CliError::new(format!(
                "writer operationPlan differs from the explicit request: {error}"
            ))
        })?;
    let close = validate_writer_close_submission(&response, &validated, close_submission)?;
    let deadline = writer_close_execution_deadline(&response, &validated, close_submission)?;
    require_current_writer_operation_action(backend, &owner, &validated)?;
    if validated.execution_instruction_batches.len() != 1 {
        return Err(CliError::new(
            "this writer operation requires multiple sequential Light setup batches; Petri will not sign a partially resumable direct flow. Prepare it after the input is hot; nothing was signed or sent",
        ));
    }
    if mcp_actions::preparing() {
        let id = operation_journal::record_prepared(backend, &request, &response, &admitted)?;
        let payload = json!({"ok":true,"operation":operation_journal::load(&id)?.public_value(),
            "review":request,"writerOperation":validated.plan.operation,"close":close,
            "signing":{"willSign":false,"willSubmit":false},"nextStep":"Review then explicitly authorize operations.execute"});
        return emit_output(
            cli,
            &payload,
            format!("{label} prepared; nothing signed or submitted"),
        );
    }
    let status_path = endpoints::writer_operation_status(&validated.plan.operation_id);
    let receipt = current_operation::sign_submit_validated_operation(
        &config,
        backend,
        endpoints::writer_operation_submit(),
        &status_path,
        writer_operation_wire_name(validated.plan.operation),
        &admitted,
        deadline,
    )?;
    let mut payload = json!({
        "operation": label,
        "writerOperation": validated.plan.operation,
        "status": "confirmed",
        "receipt": receipt.clone(),
        "receipts": [receipt],
    });
    if let Some(close) = close {
        payload["close"] = close;
    }
    let close_suffix = payload
        .pointer("/close/closeRequest")
        .and_then(Value::as_str)
        .map(|request| format!(" | next=petri writers close-status --close-request {request}"))
        .unwrap_or_default();
    emit_output(
        cli,
        &payload,
        format!(
            "{label} confirmed | signature={}{}",
            payload["receipt"]["signature"].as_str().unwrap_or("-"),
            close_suffix,
        ),
    )
}

fn current_writer_operation_mask_kind(
    operation: ameba_sdk::WriterOperationKind,
) -> Result<writer_action_mask::CollectiveActionKind, CliError> {
    use ameba_sdk::WriterOperationKind as Operation;
    use writer_action_mask::CollectiveActionKind as Action;

    match operation {
        Operation::Deposit => Ok(Action::WriterDeposit),
        Operation::WithdrawPrincipal => Ok(Action::WriterWithdraw),
        Operation::AuctionRefund => Ok(Action::AuctionRefund),
        Operation::WriterLiquidityInitialize => Ok(Action::WriterLiquidityInitialize),
        Operation::WriterLiquidityAdd => Ok(Action::WriterLiquidityAdd),
        Operation::WriterLiquidityRemove => Ok(Action::WriterLiquidityRemove),
        Operation::WriterLiquiditySweep => Ok(Action::WriterLiquiditySweep),
        Operation::WriterLiquidityPolicyBegin
        | Operation::WriterLiquidityPolicyAppend
        | Operation::WriterLiquidityPolicySeal => Err(CliError::new(
            "writer liquidity policy setup belongs to the pre-capital operator flow",
        )),
        Operation::Bid => Ok(Action::AuctionBid),
        Operation::CloseBegin => Ok(Action::WriterCloseBegin),
        Operation::CloseBasket => Ok(Action::WriterCloseContinue),
        Operation::CloseFinalize => Ok(Action::WriterCloseFinalize),
        Operation::SettlementClaimFlat => Ok(Action::ClaimFlat),
        Operation::SettlementClaimCollective => Ok(Action::ClaimLong),
        Operation::CloseCancel => Err(CliError::new(
            writer_output::WRITER_CLOSE_CANCEL_ACTION_MASK_UNAVAILABLE,
        )),
    }
}

fn writer_operation_wire_name(operation: ameba_sdk::WriterOperationKind) -> &'static str {
    use ameba_sdk::WriterOperationKind as Operation;

    match operation {
        Operation::Deposit => "deposit",
        Operation::WithdrawPrincipal => "withdraw_principal",
        Operation::AuctionRefund => "auction_refund",
        Operation::WriterLiquidityPolicyBegin => "writer_liquidity_policy_begin",
        Operation::WriterLiquidityPolicyAppend => "writer_liquidity_policy_append",
        Operation::WriterLiquidityPolicySeal => "writer_liquidity_policy_seal",
        Operation::WriterLiquidityInitialize => "writer_liquidity_initialize",
        Operation::WriterLiquidityAdd => "writer_liquidity_add",
        Operation::WriterLiquidityRemove => "writer_liquidity_remove",
        Operation::WriterLiquiditySweep => "writer_liquidity_sweep",
        Operation::Bid => "bid",
        Operation::CloseBegin => "close_begin",
        Operation::CloseBasket => "close_basket",
        Operation::CloseFinalize => "close_finalize",
        Operation::CloseCancel => "close_cancel",
        Operation::SettlementClaimCollective => "settlement_claim_collective",
        Operation::SettlementClaimFlat => "settlement_claim_flat",
    }
}

fn current_writer_operation_sleeve_account_index(
    operation: ameba_sdk::WriterOperationKind,
) -> Option<usize> {
    use ameba_sdk::WriterOperationKind as Operation;

    match operation {
        Operation::Bid
        | Operation::AuctionRefund
        | Operation::CloseBasket
        | Operation::CloseCancel => Some(1),
        Operation::Deposit
        | Operation::WithdrawPrincipal
        | Operation::CloseBegin
        | Operation::CloseFinalize
        | Operation::SettlementClaimCollective
        | Operation::SettlementClaimFlat => Some(2),
        Operation::WriterLiquidityPolicyBegin
        | Operation::WriterLiquidityPolicyAppend
        | Operation::WriterLiquidityPolicySeal
        | Operation::WriterLiquidityInitialize
        | Operation::WriterLiquidityAdd
        | Operation::WriterLiquidityRemove
        | Operation::WriterLiquiditySweep => None,
    }
}

fn require_current_writer_action(
    backend: &BackendClient,
    owner: &str,
    sleeve: &str,
    action: writer_action_mask::CollectiveActionKind,
) -> Result<writer_action_mask::WriterActionMask, CliError> {
    let response = backend.get(&endpoints::writer_available_actions(sleeve, owner))?;
    let mask = writer_action_mask::validate_current_writer_action_mask(&response, owner, sleeve)
        .map_err(|issue| {
            CliError::new(format!(
                "current wallet-specific writer availability is invalid: {issue}; nothing was prepared, signed, or sent"
            ))
        })?;
    mask.require_enabled(action).map_err(|issue| {
        CliError::new(format!(
            "current wallet-specific writer action is unavailable: {issue}; nothing was signed or sent"
        ))
    })?;
    Ok(mask)
}

fn require_current_writer_operation_action(
    backend: &BackendClient,
    owner: &str,
    validated: &ameba_sdk::ValidatedWriterOperation,
) -> Result<(), CliError> {
    let action = current_writer_operation_mask_kind(validated.plan.operation)?;
    let instruction = validated.instructions.first().ok_or_else(|| {
        CliError::new("SDK-validated writer operation contains no native instruction")
    })?;
    let sleeve = if let Some(index) =
        current_writer_operation_sleeve_account_index(validated.plan.operation)
    {
        instruction
            .accounts
            .get(index)
            .map(|account| account.pubkey.to_string())
            .ok_or_else(|| {
                CliError::new("SDK-validated writer operation does not bind its writer sleeve")
            })?
    } else {
        ameba_sdk::validated_writer_liquidity_sleeve(validated)
            .map_err(|error| {
                CliError::new(format!(
                    "SDK writer liquidity sleeve binding is invalid: {error}"
                ))
            })?
            .to_string()
    };
    require_current_writer_action(backend, owner, &sleeve, action)?;
    Ok(())
}

fn submit_flat_transfer(cli: &Cli, backend: &BackendClient, body: Value) -> Result<(), CliError> {
    current_release::require_current_write_release()?;
    let config = build_onchain_config(cli)?;
    let context = current_operation::observe_current_write_context(&config)?;
    let owner = wallet_signer::signer_pubkey(&config)?;
    let request = writer_write_request(&owner, body)?;
    let response = current_backend_payload(backend.post_json_with_current_state_retry(
        endpoints::writer_flat_transfer_prepare(),
        &request,
    )?)?;
    let plan = writer_operation_plan(&response)?;
    let encoded = serde_json::to_string(plan)
        .map_err(|_| CliError::new("Flat transfer operationPlan could not be encoded"))?;
    let unvalidated_plan: ameba_sdk::FlatTransferOperationPlan = serde_json::from_str(&encoded)
        .map_err(|_| CliError::new("Flat transfer operationPlan is not canonical JSON"))?;
    let setup = current_operation::rebuild_flat_transfer_setup(&config, &unvalidated_plan)?;
    let admitted = ameba_sdk::parse_current_governed_flat_transfer_operation_json_v1(
        &context, &encoded, &setup,
    )
    .map_err(|error| {
        CliError::new(format!(
            "Flat transfer operationPlan failed pinned SDK validation: {error}"
        ))
    })?;
    let validated = admitted
        .flat_operation()
        .ok_or_else(|| CliError::new("The SDK did not admit a Flat transfer."))?;
    let expected = json!({
        "owner": owner,
        "sleeve": request.get("sleeve").cloned().unwrap_or(Value::Null),
        "destinationOwner": request.get("destinationOwner").cloned().unwrap_or(Value::Null),
        "amountAtoms": request.get("amountAtoms").cloned().unwrap_or(Value::Null),
    });
    if serde_json::to_value(&validated.plan.semantic)
        .map_err(|_| CliError::new("Flat transfer semantics could not be encoded"))?
        != expected
    {
        return Err(CliError::new(
            "Flat transfer operationPlan differs from the explicit request",
        ));
    }
    let sleeve = expected
        .get("sleeve")
        .and_then(Value::as_str)
        .ok_or_else(|| CliError::new("Flat transfer request does not bind its writer sleeve"))?;
    require_current_writer_action(
        backend,
        &owner,
        sleeve,
        writer_action_mask::CollectiveActionKind::FlatTransfer,
    )?;
    if validated.execution_instruction_batches.len() != 1 {
        return Err(CliError::new(
            "Flat transfer needs a fresh sequential Light-account preparation; nothing was signed or sent",
        ));
    }
    if mcp_actions::preparing() {
        let id = operation_journal::record_prepared(backend, &request, &response, &admitted)?;
        let payload = json!({"ok":true,"operation":operation_journal::load(&id)?.public_value(),
            "review":expected,"signing":{"willSign":false,"willSubmit":false},
            "nextStep":"Review then explicitly authorize operations.execute"});
        return emit_output(
            cli,
            &payload,
            "Flat transfer prepared; nothing signed or submitted".into(),
        );
    }
    let status_path = endpoints::writer_operation_status(&validated.plan.operation_id);
    let receipt = current_operation::sign_submit_validated_operation(
        &config,
        backend,
        endpoints::writer_operation_submit(),
        &status_path,
        &validated.plan.operation,
        &admitted,
        None,
    )?;
    let payload = json!({
        "operation": "Flat transfer",
        "status": "confirmed",
        "receipt": receipt,
    });
    emit_output(
        cli,
        &payload,
        format!(
            "Flat transfer confirmed | signature={}",
            payload["receipt"]["signature"].as_str().unwrap_or("-")
        ),
    )
}

fn run_writer_command(
    cli: &Cli,
    backend: &BackendClient,
    command: &WriterCommand,
) -> Result<(), CliError> {
    if matches!(
        command,
        WriterCommand::Deposit { .. }
            | WriterCommand::Withdraw { .. }
            | WriterCommand::Refund { .. }
            | WriterCommand::LiquidityInitialize { .. }
            | WriterCommand::LiquidityAdd { .. }
            | WriterCommand::LiquidityRemove { .. }
            | WriterCommand::LiquiditySweep { .. }
            | WriterCommand::Bid { .. }
            | WriterCommand::Close { .. }
            | WriterCommand::Claim { .. }
            | WriterCommand::TransferFlat { .. }
    ) {
        current_release::require_current_write_release()?;
    }
    match command {
        WriterCommand::Capabilities => {
            let response = current_backend_payload(backend.get(endpoints::capabilities())?)?;
            writer_output::validate_current_writer_close_capabilities(&response).map_err(
                |issue| {
                    CliError::new(format!(
                        "current writer-close capability contract is invalid: {issue}"
                    ))
                },
            )?;
            emit_output(
                cli,
                &response,
                writer_output::render_writer_capabilities(&response),
            )
        }
        WriterCommand::AvailableActions { sleeve, owner } => {
            let sleeve = canonical_pubkey_string(sleeve, "sleeve")?;
            let owner = canonical_pubkey_string(owner, "owner")?;
            let response = current_backend_payload(
                backend.get(&endpoints::writer_available_actions(&sleeve, &owner))?,
            )?;
            let mask =
                writer_action_mask::validate_current_writer_action_mask(&response, &owner, &sleeve)
                    .map_err(|issue| {
                        CliError::new(format!("current wallet action mask is invalid: {issue}"))
                    })?;
            emit_output(
                cli,
                &response,
                writer_action_mask::render_writer_action_mask(&mask),
            )
        }
        WriterCommand::List { owner } => {
            if owner.is_some() {
                return Err(CliError::new(
                    "writers list cannot filter by owner because the current Amoeba route is a global sleeve catalog; omit --owner",
                ));
            }
            let response = current_backend_payload(backend.get(&endpoints::writer_sleeves(None))?)?;
            emit_output(
                cli,
                &response,
                writer_output::render_writer_sleeves(&response),
            )
        }
        WriterCommand::Show { sleeve } => {
            let sleeve = canonical_pubkey_string(sleeve, "sleeve")?;
            let response =
                current_backend_payload(backend.get(&endpoints::writer_sleeve(&sleeve))?)?;
            validate_writer_sleeve_identity(&response, &sleeve)?;
            emit_output(
                cli,
                &response,
                writer_output::render_writer_sleeve(&response),
            )
        }
        WriterCommand::Refunds {
            owner,
            cursor,
            limit,
        } => {
            let owner = canonical_pubkey_string(owner, "owner")?;
            if cursor
                .as_ref()
                .is_some_and(|value| !writer_liquidity::valid_cursor(value))
            {
                return Err(CliError::new(
                    "refund cursor is invalid; restart discovery without --cursor",
                ));
            }
            let response = current_backend_payload(backend.get(&endpoints::writer_refunds(
                &owner,
                cursor.as_deref(),
                *limit,
            ))?)?;
            let output = writer_liquidity::render_refunds(&response, &owner, *limit)
                .map_err(CliError::new)?;
            emit_output(cli, &response, output)
        }
        WriterCommand::Liquidity {
            sleeve,
            owner,
            series_index,
        } => {
            let sleeve = canonical_pubkey_string(sleeve, "sleeve")?;
            let owner = canonical_pubkey_string(owner, "owner")?;
            let response = current_backend_payload(backend.get(&endpoints::writer_liquidity(
                &sleeve,
                &owner,
                *series_index,
            ))?)?;
            let output =
                writer_liquidity::render_liquidity(&response, &owner, &sleeve, *series_index)
                    .map_err(CliError::new)?;
            emit_output(cli, &response, output)
        }
        WriterCommand::Withdraw { sleeve, amount } => submit_writer_operation(
            cli,
            backend,
            endpoints::writer_withdraw_prepare(),
            json!({"sleeve": canonical_pubkey_string(sleeve, "sleeve")?, "amountAtoms": canonical_u64_string(amount, "amount", false)?}),
            &[ameba_sdk::WriterOperationKind::WithdrawPrincipal],
            "writer principal withdrawal",
            WriterCloseSubmission::None,
        ),
        WriterCommand::Refund { auction, bid } => submit_writer_operation(
            cli,
            backend,
            endpoints::writer_refund_prepare(),
            json!({"auction": canonical_pubkey_string(auction, "auction")?, "bid": canonical_pubkey_string(bid, "bid")?}),
            &[ameba_sdk::WriterOperationKind::AuctionRefund],
            "historical auction refund",
            WriterCloseSubmission::None,
        ),
        WriterCommand::LiquidityInitialize {
            sleeve,
            series_index,
        } => submit_writer_operation(
            cli,
            backend,
            endpoints::writer_liquidity_initialize_prepare(),
            json!({"sleeve": canonical_pubkey_string(sleeve, "sleeve")?, "seriesIndex": series_index}),
            &[ameba_sdk::WriterOperationKind::WriterLiquidityInitialize],
            "writer liquidity initialize",
            WriterCloseSubmission::None,
        ),
        WriterCommand::LiquidityAdd {
            sleeve,
            series_index,
            issue_amount,
            bins,
        } => submit_writer_operation(
            cli,
            backend,
            endpoints::writer_liquidity_add_prepare(),
            json!({"sleeve": canonical_pubkey_string(sleeve, "sleeve")?, "seriesIndex": series_index,
                "issueAmountAtoms": canonical_u64_string(issue_amount, "issue-amount", true)?,
                "entries": writer_liquidity::parse_bins(bins, true).map_err(CliError::new)?}),
            &[ameba_sdk::WriterOperationKind::WriterLiquidityAdd],
            "writer liquidity add",
            WriterCloseSubmission::None,
        ),
        WriterCommand::LiquidityRemove {
            sleeve,
            series_index,
            bins,
        } => submit_writer_operation(
            cli,
            backend,
            endpoints::writer_liquidity_remove_prepare(),
            json!({"sleeve": canonical_pubkey_string(sleeve, "sleeve")?, "seriesIndex": series_index,
                "entries": writer_liquidity::parse_bins(bins, false).map_err(CliError::new)?}),
            &[ameba_sdk::WriterOperationKind::WriterLiquidityRemove],
            "writer liquidity remove",
            WriterCloseSubmission::None,
        ),
        WriterCommand::LiquiditySweep {
            sleeve,
            series_index,
        } => submit_writer_operation(
            cli,
            backend,
            endpoints::writer_liquidity_sweep_prepare(),
            json!({"sleeve": canonical_pubkey_string(sleeve, "sleeve")?, "seriesIndex": series_index}),
            &[ameba_sdk::WriterOperationKind::WriterLiquiditySweep],
            "writer liquidity proceeds sweep",
            WriterCloseSubmission::None,
        ),
        WriterCommand::Deposit { sleeve, amount } => submit_writer_operation(
            cli,
            backend,
            endpoints::writer_deposit_prepare(),
            json!({
                "sleeve": canonical_pubkey_string(sleeve, "sleeve")?,
                "principalAtoms": canonical_u64_string(amount, "amount", false)?,
            }),
            &[ameba_sdk::WriterOperationKind::Deposit],
            "writer deposit",
            WriterCloseSubmission::None,
        ),
        WriterCommand::Bid {
            auction,
            series_index,
            price,
            amount,
        } => submit_writer_operation(
            cli,
            backend,
            endpoints::writer_bid_prepare(),
            json!({
                "auction": canonical_pubkey_string(auction, "auction")?,
                "seriesIndex": series_index,
                "pricePerContractAtoms": canonical_u64_string(price, "price", false)?,
                "quantityAtoms": canonical_u64_string(amount, "amount", false)?,
            }),
            &[ameba_sdk::WriterOperationKind::Bid],
            "writer bid",
            WriterCloseSubmission::None,
        ),
        WriterCommand::ClosePreview {
            sleeve,
            amount,
            minimum_withdrawal,
        } => writer_preview_post(
            cli,
            backend,
            endpoints::writer_close_preview(),
            json!({
                "sleeve": canonical_pubkey_string(sleeve, "sleeve")?,
                "flatParAtoms": canonical_u64_string(amount, "amount", false)?,
                "minimumWithdrawalAtoms": canonical_u64_string(
                    minimum_withdrawal,
                    "minimum-withdrawal",
                    true,
                )?,
            }),
        ),
        WriterCommand::Close {
            sleeve,
            amount,
            minimum_withdrawal,
            close_request,
            cancel,
        } => {
            let asserted_close_request = close_request
                .as_deref()
                .map(|value| canonical_pubkey_string(value, "close-request"))
                .transpose()?;
            if let Some(sleeve) = sleeve {
                if *cancel {
                    return Err(CliError::new(
                        "--cancel cannot be combined with a new writer close",
                    ));
                }
                let amount = amount
                    .as_deref()
                    .ok_or_else(|| CliError::new("a new writer close requires --amount"))?;
                let minimum_withdrawal = minimum_withdrawal.as_deref().ok_or_else(|| {
                    CliError::new("a new writer close requires --minimum-withdrawal")
                })?;
                submit_writer_operation(
                    cli,
                    backend,
                    endpoints::writer_close_prepare(),
                    json!({
                        "sleeve": canonical_pubkey_string(sleeve, "sleeve")?,
                        "flatParAtoms": canonical_u64_string(amount, "amount", false)?,
                        "minimumWithdrawalAtoms": canonical_u64_string(
                            minimum_withdrawal,
                            "minimum-withdrawal",
                            true,
                        )?,
                    }),
                    &[ameba_sdk::WriterOperationKind::CloseBegin],
                    "writer close begin",
                    WriterCloseSubmission::Begin {
                        asserted_request: asserted_close_request.as_deref(),
                    },
                )
            } else {
                if amount.is_some() || minimum_withdrawal.is_some() {
                    return Err(CliError::new(
                        "writer close continuation accepts only --close-request and optional --cancel",
                    ));
                }
                let close_request = asserted_close_request.as_deref().ok_or_else(|| {
                    CliError::new("writer close continuation requires --close-request")
                })?;
                let body = json!({ "closeRequest": close_request });
                if *cancel {
                    Err(CliError::new(
                        writer_output::WRITER_CLOSE_CANCEL_ACTION_MASK_UNAVAILABLE,
                    ))
                } else {
                    submit_writer_operation(
                        cli,
                        backend,
                        endpoints::writer_close_next_prepare(),
                        body,
                        &[
                            ameba_sdk::WriterOperationKind::CloseBasket,
                            ameba_sdk::WriterOperationKind::CloseFinalize,
                        ],
                        "writer close advance",
                        WriterCloseSubmission::Forward { close_request },
                    )
                }
            }
        }
        WriterCommand::CloseStatus { close_request } => {
            let close_request = canonical_pubkey_string(close_request, "close-request")?;
            let response = current_backend_payload(
                backend.get(&endpoints::writer_close_status(&close_request))?,
            )?;
            validate_writer_close_status_response(&response, &close_request)?;
            emit_output(
                cli,
                &response,
                writer_output::render_writer_close_status(&response),
            )
        }
        WriterCommand::Claim {
            sleeve,
            variant,
            series_index,
            amount,
        } => {
            let series_index = match variant {
                cli::WriterClaimVariant::CollectiveLong => {
                    series_index.filter(|index| *index < 20).ok_or_else(|| {
                        CliError::new(
                            "collective-long claims require --series-index from 0 through 19",
                        )
                    })?
                }
                cli::WriterClaimVariant::FlatResidual if series_index.is_some() => {
                    return Err(CliError::new(
                        "flat-residual claims do not accept --series-index",
                    ));
                }
                cli::WriterClaimVariant::FlatResidual => u8::MAX,
            };
            submit_writer_operation(
                cli,
                backend,
                endpoints::writer_claim_prepare(),
                json!({
                    "sleeve": canonical_pubkey_string(sleeve, "sleeve")?,
                    "claimVariant": variant.as_request_value(),
                    "seriesIndex": series_index,
                    "amountAtoms": canonical_u64_string(amount, "amount", false)?,
                }),
                &[match variant {
                    cli::WriterClaimVariant::CollectiveLong => {
                        ameba_sdk::WriterOperationKind::SettlementClaimCollective
                    }
                    cli::WriterClaimVariant::FlatResidual => {
                        ameba_sdk::WriterOperationKind::SettlementClaimFlat
                    }
                }],
                "writer settlement claim",
                WriterCloseSubmission::None,
            )
        }
        WriterCommand::TransferFlat {
            sleeve,
            destination,
            amount,
        } => submit_flat_transfer(
            cli,
            backend,
            json!({
                "sleeve": canonical_pubkey_string(sleeve, "sleeve")?,
                "destinationOwner": canonical_pubkey_string(destination, "destination")?,
                "amountAtoms": canonical_u64_string(amount, "amount", false)?,
            }),
        ),
        WriterCommand::PolicyAudit { sleeve } => {
            let sleeve = canonical_pubkey_string(sleeve, "sleeve")?;
            let response =
                current_backend_payload(backend.get(&endpoints::writer_policy_audit(&sleeve))?)?;
            validate_writer_sleeve_identity(&response, &sleeve)?;
            emit_output(
                cli,
                &response,
                writer_output::render_writer_policy_audit(&response),
            )
        }
    }
}

fn run() -> Result<(), CliError> {
    dispatch_cli(parse_cli())
}

fn dispatch_cli(cli: Cli) -> Result<(), CliError> {
    if cli.command.is_none() {
        let payload = json!({
            "ok": true,
            "command": "help",
            "title": "Petri",
            "description": "Amoeba market terminal",
            "wallet": attached_wallet_payload(&attached_wallet::inspect_attached_wallet(&cli)),
            "commands": [
                "petri markets",
                "petri markets show <market>",
                "petri markets chart <market>",
                "petri contracts --market <market>",
                "petri writers list",
                "petri writers show --sleeve <sleeve-pubkey>",
                "petri liquidity",
                "petri staking",
                "petri history",
                "petri settlements show <market> <expiry>",
                "petri oracle recipe ramx --query KVR56U46BD8-32",
                "petri oracle drafts",
                "petri mcp status",
                "petri mcp enable",
                "petri mcp manifest",
                "petri help",
                "petri help <command>",
                "petri config show",
                "petri tui"
            ],
        });
        emit_output(&cli, &payload, render_cli_front_door(&cli))?;
        return Ok(());
    }

    let backend = BackendClient::new(&cli.backend_url)?;
    let command = cli
        .command
        .as_ref()
        .expect("command is present or no-arg TUI path is active");

    match command {
        Command::Commitments { command } => {
            let value = oracle_commitments::run(command)?;
            emit_output(
                &cli,
                &value,
                serde_json::to_string_pretty(&value).unwrap_or_default(),
            )?;
        }
        Command::Participate(args) => {
            let value = participation::run(&build_onchain_config(&cli)?, &backend, args)?;
            let output = if args.describe {
                serde_json::to_string_pretty(&value).unwrap_or_default()
            } else {
                portable_operation::render(&value)
            };
            emit_output(&cli, &value, output)?;
        }
        Command::Operations { command } => {
            use cli::OperationsCommand;
            let payload = match command {
                OperationsCommand::Execute { operation_id, yes } => portable_operation::execute(
                    &build_onchain_config(&cli)?,
                    &backend,
                    operation_id,
                    *yes,
                )?,
                OperationsCommand::List { owner } => operation_journal::list(owner.as_deref())?,
                OperationsCommand::Show { operation_id } => {
                    let record = operation_journal::load(operation_id)?;
                    json!({"ok":true,"operation":record.public_value(),"review":record.request})
                }
                OperationsCommand::Resume { operation_id } => {
                    operation_journal::recover(&backend, operation_id)?
                }
                OperationsCommand::Status {
                    operation_id,
                    watch,
                } => {
                    let mut result = operation_journal::recover(&backend, operation_id)?;
                    for _ in 0..if *watch { 30 } else { 0 } {
                        if matches!(
                            result.pointer("/operation/state").and_then(Value::as_str),
                            Some(
                                "confirmed"
                                    | "failed_on_chain"
                                    | "rejected"
                                    | "needs_fresh_preparation"
                            )
                        ) {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_secs(2));
                        result = operation_journal::recover(&backend, operation_id)?;
                    }
                    result
                }
            };
            emit_output(
                &cli,
                &payload,
                serde_json::to_string_pretty(&payload).unwrap_or_default(),
            )?;
        }
        Command::Markets { command } => match command {
            None => {
                let payload = market_surface::dish_list_payload(&backend)?;
                emit_output(
                    &cli,
                    &payload,
                    market_surface::render_dish_list_plain(&cli, &backend, &payload),
                )?;
            }
            Some(MarketsCommand::Show { market_id }) => {
                let payload = market_surface::dish_snapshot_payload(&backend, market_id)?;
                let agent_payload =
                    market_agent_payload(market_id, &payload, MarketAgentView::Show)?;
                emit_output(
                    &cli,
                    &agent_payload,
                    market_surface::render_dish_open(&cli, &backend, market_id, &payload),
                )?;
            }
            Some(MarketsCommand::Status { market_id }) => {
                let payload = market_surface::dish_snapshot_payload(&backend, market_id)?;
                let agent_payload =
                    market_agent_payload(market_id, &payload, MarketAgentView::Status)?;
                emit_output(
                    &cli,
                    &agent_payload,
                    market_surface::render_dish_status(&cli, &backend, market_id, &payload),
                )?;
            }
            Some(MarketsCommand::Print { market_id }) => {
                let payload = market_surface::dish_snapshot_payload(&backend, market_id)?;
                let agent_payload =
                    market_agent_payload(market_id, &payload, MarketAgentView::Print)?;
                emit_output(
                    &cli,
                    &agent_payload,
                    market_surface::render_print(&cli, &backend, market_id, &payload),
                )?;
            }
            Some(MarketsCommand::Chart { options }) => {
                run_chart_command(&cli, &backend, options)?;
            }
        },
        Command::Contracts { options } => {
            let options = contracts_to_chain_args(options)?;
            run_contracts_chain_command(&cli, &backend, &options)?;
        }
        Command::Trades { command } => match command {
            TradesCommand::Quote { intent, side } => {
                let payload = trade_service::prepare(
                    &cli,
                    &backend,
                    intent,
                    matches!(side, cli::TradeSide::Buy),
                )?;
                emit_output(&cli, &payload, trade_service::render_response(&payload))?;
            }
            TradesCommand::Buy { intent, yes } | TradesCommand::Sell { intent, yes } => {
                let prepared = trade_service::prepare(
                    &cli,
                    &backend,
                    intent,
                    matches!(command, TradesCommand::Buy { .. }),
                )?;
                let payload = if *yes {
                    trade_service::execute_reviewed(
                        &cli,
                        &backend,
                        prepared["operationId"]
                            .as_str()
                            .ok_or_else(|| CliError::new("Prepared operation ID missing."))?,
                    )?
                } else {
                    prepared
                };
                emit_output(&cli, &payload, trade_service::render_response(&payload))?;
            }
            TradesCommand::Execute { operation_id, yes } => {
                if !yes {
                    return Err(CliError::new("Explicit approval is required."));
                }
                let payload = trade_service::execute_reviewed(&cli, &backend, operation_id)?;
                emit_output(&cli, &payload, trade_service::render_response(&payload))?;
            }
            TradesCommand::Prepare { trade } => {
                run_collective_trade_prepare_command(&cli, &backend, trade)?;
            }
            TradesCommand::Submit { trade } => {
                run_collective_trade_submit_command(&cli, &backend, trade)?
            }
        },
        Command::Writers { command } => run_writer_command(&cli, &backend, command)?,
        Command::Liquidity { command, owner } => {
            ensure_liquidity_owner_scope(command.is_some(), owner.as_deref())?;
            match command {
                None => {
                    run_liquidity_positions_command(&cli, &backend, owner.as_deref())?;
                }
                Some(LiquidityCommand::Plan { action, liquidity }) => {
                    run_dlmm_liquidity_command(&cli, &backend, &liquidity, *action)?;
                }
                Some(LiquidityCommand::Add { liquidity }) => {
                    run_dlmm_liquidity_command(
                        &cli,
                        &backend,
                        liquidity,
                        LiquidityActionValue::Add,
                    )?;
                }
                Some(LiquidityCommand::Remove { liquidity }) => {
                    run_dlmm_liquidity_command(
                        &cli,
                        &backend,
                        liquidity,
                        LiquidityActionValue::Remove,
                    )?;
                }
                Some(LiquidityCommand::ClosePosition { liquidity }) => {
                    run_dlmm_liquidity_command(
                        &cli,
                        &backend,
                        liquidity,
                        LiquidityActionValue::ClosePosition,
                    )?;
                }
            }
        }
        Command::Wallet { command } => match command {
            None => run_wallet_balance_command(&cli, None, None, None)?,
            Some(WalletCommand::Address) => run_wallet_address_command(&cli)?,
            Some(WalletCommand::Balance {
                owner_pubkey,
                usdc_mint,
                amba_mint,
            }) => run_wallet_balance_command(
                &cli,
                owner_pubkey.as_deref(),
                usdc_mint.as_deref(),
                amba_mint.as_deref(),
            )?,
            Some(WalletCommand::Collateral { owner }) => {
                run_wallet_collateral_command(&cli, &backend, owner.as_deref())?
            }
        },
        Command::Staking {
            command,
            market,
            expiry,
        } => {
            if matches!(
                command,
                Some(
                    StakingCommand::Stake { .. }
                        | StakingCommand::Activate { .. }
                        | StakingCommand::Cancel
                        | StakingCommand::Unstake { .. }
                        | StakingCommand::Claim
                )
            ) {
                current_release::require_current_write_release()?;
            }
            let config = build_onchain_config(&cli)?;
            match command {
                None => {
                    let result = staking::status(&config, None)?;
                    let payload = serde_json::to_value(&result).map_err(|error| {
                        CliError::new(format!("could not display staking status: {error}"))
                    })?;
                    emit_output(&cli, &payload, staking::render_status(&result))?;
                }
                Some(StakingCommand::Status { options }) => {
                    let result = staking::status(&config, options.owner.as_deref())?;
                    let payload = serde_json::to_value(&result).map_err(|error| {
                        CliError::new(format!("could not display staking status: {error}"))
                    })?;
                    emit_output(&cli, &payload, staking::render_status(&result))?;
                }
                Some(StakingCommand::Stake { options }) => {
                    let value = participation::prepare_staking(
                        &config,
                        &backend,
                        participation::Action::Stake,
                        market.as_deref(),
                        expiry.as_deref(),
                        Some(&options.amount),
                        None,
                    )?;
                    emit_output(&cli, &value, portable_operation::render(&value))?;
                }
                Some(StakingCommand::Activate { options }) => {
                    let value = participation::prepare_staking(
                        &config,
                        &backend,
                        participation::Action::ActivateStake,
                        market.as_deref(),
                        expiry.as_deref(),
                        None,
                        options.min_received.as_deref(),
                    )?;
                    emit_output(&cli, &value, portable_operation::render(&value))?;
                }
                Some(StakingCommand::Cancel) => {
                    let result = staking::cancel(&config)?;
                    let payload = serde_json::to_value(&result).map_err(|error| {
                        CliError::new(format!(
                            "could not display staking cancellation result: {error}"
                        ))
                    })?;
                    emit_output(&cli, &payload, staking::render_transaction(&result))?;
                }
                Some(StakingCommand::Unstake { options }) => {
                    let value = participation::prepare_staking(
                        &config,
                        &backend,
                        participation::Action::Unstake,
                        market.as_deref(),
                        expiry.as_deref(),
                        Some(&options.amount),
                        options.min_received.as_deref(),
                    )?;
                    emit_output(&cli, &value, portable_operation::render(&value))?;
                }
                Some(StakingCommand::Claim) => {
                    let value = participation::prepare_staking(
                        &config,
                        &backend,
                        participation::Action::CompleteUnstake,
                        market.as_deref(),
                        expiry.as_deref(),
                        None,
                        None,
                    )?;
                    emit_output(&cli, &value, portable_operation::render(&value))?;
                }
            }
        }
        Command::History { history } => {
            run_history_command(&cli, &backend, history)?;
        }
        Command::Settlements { command } => match command {
            SettlementsCommand::Show {
                market_id,
                expiry_id,
            } => {
                let payload = current_backend_payload(
                    backend.get(&endpoints::dlmm_settlement(market_id, expiry_id))?,
                )?;
                emit_output(
                    &cli,
                    &payload,
                    render_settlement_show(&cli, &backend, market_id, expiry_id, &payload),
                )?;
            }
            SettlementsCommand::Check {
                market_id,
                expiry_id,
            } => {
                let payload = current_backend_payload(backend.get(
                    &endpoints::dlmm_settlement_oracle_preflight(market_id, expiry_id),
                )?)?;
                emit_output(
                    &cli,
                    &payload,
                    render_settlement_oracle_check(&cli, &backend, market_id, expiry_id, &payload),
                )?;
            }
            SettlementsCommand::Oracle {
                market_id,
                expiry_id,
            } => {
                let payload = current_backend_payload(
                    backend.get(&endpoints::dlmm_settlement_oracle(market_id, expiry_id))?,
                )?;
                emit_output(
                    &cli,
                    &payload,
                    render_settlement_oracle(&cli, &backend, market_id, expiry_id, &payload),
                )?;
            }
        },
        Command::Config { command } => match command {
            ConfigCommand::Show => {
                let payload = build_config_show_payload(&cli)?;
                emit_output(&cli, &payload, render_config_show(&payload))?;
            }
            ConfigCommand::Set {
                key: ConfigKey::BackendUrl,
                value,
            } => {
                let backend_url =
                    petri_config::normalize_amoeba_backend_url(value).map_err(CliError::new)?;
                petri_config::set_backend_url(&backend_url).map_err(CliError::new)?;
                let payload = build_config_show_payload_for_backend(&cli, &backend_url)?;
                emit_output(&cli, &payload, render_config_show(&payload))?;
            }
            ConfigCommand::Reset {
                key: ConfigKey::BackendUrl,
            } => {
                petri_config::reset_backend_url().map_err(CliError::new)?;
                let payload =
                    build_config_show_payload_for_backend(&cli, petri_config::DEFAULT_BACKEND_URL)?;
                emit_output(&cli, &payload, render_config_show(&payload))?;
            }
        },
        Command::Oracle { command } => match command {
            None => {
                let tree = load_oracle_index_tree(&backend, "ramx")?;
                let payload = build_oracle_recipe_payload(&tree, None, 24);
                emit_output(&cli, &payload, render_oracle_recipe(&payload))?;
            }
            Some(OracleCommand::Carry(args)) => {
                let value = oracle_carry::read(&build_onchain_config(&cli)?, &backend, args)?;
                emit_output(&cli, &value, oracle_carry::render(&value))?;
            }
            Some(OracleCommand::State) => {
                let payload =
                    current_backend_payload(backend.get(endpoints::dlmm_oracle_state())?)?;
                emit_output(&cli, &payload, render_spread_oracle_state(&payload))?;
            }
            Some(OracleCommand::Markets { market }) => {
                let payload = current_backend_payload(match nonempty_trimmed(market.as_deref()) {
                    Some(market_id) => backend.get(&endpoints::dlmm_oracle_market(&market_id))?,
                    None => backend.get(endpoints::dlmm_oracle_markets())?,
                })?;
                emit_output(&cli, &payload, render_spread_oracle_state(&payload))?;
            }
            Some(OracleCommand::Latest { market }) => {
                let payload = current_backend_payload(
                    backend.get(&endpoints::dlmm_oracle_latest(market.as_deref()))?,
                )?;
                emit_output(&cli, &payload, render_spread_oracle_latest(&payload))?;
            }
            Some(OracleCommand::History { market }) => {
                let payload = current_backend_payload(
                    backend.get(&endpoints::dlmm_oracle_history(market.as_deref()))?,
                )?;
                emit_output(&cli, &payload, render_spread_oracle_history(&payload))?;
            }
            Some(OracleCommand::Recipe { args }) => {
                let market = args.market.as_deref().unwrap_or("ramx");
                let tree = load_oracle_index_tree(&backend, market)?;
                let payload = build_oracle_recipe_payload(&tree, args.query.as_deref(), args.limit);
                emit_output(&cli, &payload, render_oracle_recipe(&payload))?;
            }
            Some(OracleCommand::Sources { command }) => match command {
                None => {
                    let tree = load_oracle_index_tree(&backend, "ramx")?;
                    let payload = build_oracle_recipe_payload(&tree, None, 24);
                    emit_output(&cli, &payload, render_oracle_recipe(&payload))?;
                }
                Some(OracleSourceCommand::Propose { draft }) => {
                    let tree = load_oracle_index_tree(&backend, &draft.common.market)?;
                    let payload = create_oracle_source_proposal_draft(draft, &tree)?;
                    emit_output(&cli, &payload, render_oracle_draft_created(&payload))?;
                }
                Some(OracleSourceCommand::Support { draft }) => {
                    let tree = load_oracle_index_tree(&backend, &draft.common.market)?;
                    let payload = create_oracle_source_support_draft(draft, &tree)?;
                    emit_output(&cli, &payload, render_oracle_draft_created(&payload))?;
                }
                Some(OracleSourceCommand::Challenge { draft }) => {
                    let tree = load_oracle_index_tree(&backend, &draft.common.market)?;
                    let payload = create_oracle_source_challenge_draft(draft, &tree)?;
                    emit_output(&cli, &payload, render_oracle_draft_created(&payload))?;
                }
            },
            Some(OracleCommand::Prints { command }) => match command {
                OraclePrintsCommand::Opening { draft } => {
                    let tree = load_oracle_index_tree(&backend, &draft.common.market)?;
                    let payload = create_oracle_opening_print_draft(draft, &tree)?;
                    emit_output(&cli, &payload, render_oracle_draft_created(&payload))?;
                }
                OraclePrintsCommand::Challenge { draft } => {
                    let tree = load_oracle_index_tree(&backend, &draft.common.market)?;
                    let payload = create_oracle_opening_claim_challenge_draft(draft, &tree)?;
                    emit_output(&cli, &payload, render_oracle_draft_created(&payload))?;
                }
            },
            Some(OracleCommand::Updates { command }) => match command {
                OracleUpdatesCommand::Commit { draft } => {
                    let payload = create_oracle_update_commit_draft(draft)?;
                    emit_output(&cli, &payload, render_oracle_draft_created(&payload))?;
                }
                OracleUpdatesCommand::Reveal { draft } => {
                    let payload = create_oracle_update_reveal_draft(draft)?;
                    emit_output(&cli, &payload, render_oracle_draft_created(&payload))?;
                }
                OracleUpdatesCommand::Expire { draft } => {
                    let payload = create_oracle_update_expiry_draft(draft)?;
                    emit_output(&cli, &payload, render_oracle_draft_created(&payload))?;
                }
                OracleUpdatesCommand::Challenge { draft } => {
                    let tree = load_oracle_index_tree(&backend, &draft.common.market)?;
                    let payload = create_oracle_update_challenge_draft(draft, &tree)?;
                    emit_output(&cli, &payload, render_oracle_draft_created(&payload))?;
                }
            },
            Some(OracleCommand::Emergency { command }) => match command {
                OracleEmergencyCommand::Commit { draft } => {
                    let payload = create_oracle_emergency_commit_draft(draft)?;
                    emit_output(&cli, &payload, render_oracle_draft_created(&payload))?;
                }
                OracleEmergencyCommand::Reveal { draft } => {
                    let payload = create_oracle_emergency_reveal_draft(draft)?;
                    emit_output(&cli, &payload, render_oracle_draft_created(&payload))?;
                }
            },
            Some(OracleCommand::Rewards { command }) => match command {
                OracleRewardsCommand::Claim { draft } => {
                    let payload = create_oracle_reward_claim_draft(draft)?;
                    emit_output(&cli, &payload, render_oracle_draft_created(&payload))?;
                }
            },
            Some(OracleCommand::Stakes { command }) => match command {
                OracleStakesCommand::Settle { draft } => {
                    let payload = create_oracle_stake_settlement_draft(draft)?;
                    emit_output(&cli, &payload, render_oracle_draft_created(&payload))?;
                }
            },
            Some(OracleCommand::Amba { command }) => match command {
                OracleAmbaCommand::Deposit { draft } => {
                    let payload = create_oracle_amba_custody_draft(draft, true)?;
                    emit_output(&cli, &payload, render_oracle_draft_created(&payload))?;
                }
                OracleAmbaCommand::Withdraw { draft } => {
                    let payload = create_oracle_amba_custody_draft(draft, false)?;
                    emit_output(&cli, &payload, render_oracle_draft_created(&payload))?;
                }
            },
            Some(OracleCommand::Drafts { command }) => match command {
                None => {
                    let store_path = resolve_oracle_draft_path(None)?;
                    let payload = build_oracle_drafts_list_payload(&store_path, 16)?;
                    emit_output(&cli, &payload, render_oracle_drafts_list(&payload))?;
                }
                Some(OracleDraftsCommand::List { limit, path }) => {
                    let store_path = resolve_oracle_draft_path(path.as_deref())?;
                    let payload = build_oracle_drafts_list_payload(&store_path, *limit)?;
                    emit_output(&cli, &payload, render_oracle_drafts_list(&payload))?;
                }
                Some(OracleDraftsCommand::Show { selector, path }) => {
                    let store_path = resolve_oracle_draft_path(path.as_deref())?;
                    let payload = preview_oracle_drafts(&store_path, selector)?;
                    emit_output(&cli, &payload, render_oracle_drafts_preview(&payload))?;
                }
                Some(OracleDraftsCommand::Validate { selector, path }) => {
                    let store_path = resolve_oracle_draft_path(path.as_deref())?;
                    let payload = validate_oracle_drafts(&store_path, selector)?;
                    emit_output(&cli, &payload, render_oracle_drafts_validate(&payload))?;
                }
                Some(OracleDraftsCommand::Submit { .. }) => {
                    return Err(CliError::new(
                        "current oracle transaction preparation is not_wired: Petri preserves semantic drafts only until the current SDK exposes the frozen oracle operator and immutable managed-signing plan",
                    ));
                }
            },
        },
        Command::Mcp { command } => match command {
            McpCommand::Actions => {
                let payload = mcp_actions::manifest();
                emit_output(
                    &cli,
                    &payload,
                    serde_json::to_string_pretty(&payload).unwrap_or_default(),
                )?;
            }
            McpCommand::Invoke { action, .. } => {
                mcp_actions::invoke_stdin(&cli, &backend, action)?;
            }
            McpCommand::Status => {
                let status = mcp_setup::status().map_err(|error| {
                    CliError::new(format!("could not read Petri MCP status: {error}"))
                })?;
                let payload = mcp_setup_payload("status", &status, false);
                emit_output(&cli, &payload, render_mcp_setup_status(&status, None))?;
            }
            McpCommand::Enable => {
                let status = mcp_setup::enable().map_err(|error| {
                    CliError::new(format!("could not enable Petri MCP: {error}"))
                })?;
                let payload = mcp_setup_payload("enable", &status, true);
                emit_output(
                    &cli,
                    &payload,
                    render_mcp_setup_status(
                        &status,
                        Some(
                            "Petri MCP enabled. Restart or reload an AI agent that is already open.",
                        ),
                    ),
                )?;
            }
            McpCommand::Repair => {
                let repair = mcp_setup::repair().map_err(|error| {
                    CliError::new(format!(
                        "could not repair the Petri agent connection: {error}\n\nNo disconnect was attempted. If repair still cannot complete, disable only Petri's managed connection, then connect it again."
                    ))
                })?;
                let mut payload = mcp_setup_payload(
                    "repair",
                    &repair.status,
                    repair.outcome == mcp_setup::McpRepairOutcome::Repaired,
                );
                if let Some(object) = payload.as_object_mut() {
                    object.insert("repair_outcome".to_string(), json!(repair.outcome.as_str()));
                }
                let result_message = match repair.outcome {
                    mcp_setup::McpRepairOutcome::AlreadyHealthy => {
                        "Connection check complete. Petri MCP is healthy; no settings were changed."
                    }
                    mcp_setup::McpRepairOutcome::Repaired => {
                        "Repair complete. Petri MCP is healthy. Restart or reload an AI agent that is already open. Wallet execution requires explicit approval."
                    }
                };
                emit_output(
                    &cli,
                    &payload,
                    render_mcp_setup_status(&repair.status, Some(result_message)),
                )?;
            }
            McpCommand::Disable => {
                let status = mcp_setup::disable().map_err(|error| {
                    CliError::new(format!("could not disable Petri MCP: {error}"))
                })?;
                let payload = mcp_setup_payload("disable", &status, true);
                emit_output(
                    &cli,
                    &payload,
                    render_mcp_setup_status(
                        &status,
                        Some(
                            "Petri MCP disabled. Restart or reload an AI agent that is already open.",
                        ),
                    ),
                )?;
            }
            McpCommand::Manifest => {
                let payload = agent_protocol::mcp_manifest();
                emit_output(
                    &cli,
                    &payload,
                    agent_protocol::render_mcp_manifest(&payload),
                )?;
            }
        },
        Command::Update {
            command,
            no_fetch,
            skip_shim,
            yes,
            restart,
        } => {
            if matches!(command, Some(UpdateCommand::Info)) {
                let info = release_update::information();
                emit_output(
                    &cli,
                    &info,
                    format!(
                        "Petri {} | {} updates",
                        env!("CARGO_PKG_VERSION"),
                        if release_update::enabled() {
                            "preview release"
                        } else {
                            "source"
                        }
                    ),
                )?;
                return Ok(());
            }
            if release_update::enabled() {
                if *no_fetch || *skip_shim {
                    return Err(CliError::new(
                        "--no-fetch and --skip-shim apply only to source-checkout updates.",
                    ));
                }
                let report = run_release_update(
                    command.as_ref(),
                    *yes,
                    *restart,
                    cli.resolved_output() == OutputFormat::Json,
                )?;
                let payload = serde_json::to_value(&report)
                    .map_err(|_| CliError::new("Could not encode update status."))?;
                emit_output(&cli, &payload, report.message)?;
                return Ok(());
            }
            if matches!(command, Some(UpdateCommand::Recover)) || *yes || *restart {
                return Err(CliError::new(
                    "Recovery, --yes, and --restart apply only to standalone preview installations.",
                ));
            }
            let check_only = matches!(command.as_ref(), Some(UpdateCommand::Check));
            let report = if check_only {
                workspace_update::check_workspace_update(*no_fetch)?
            } else {
                workspace_update::run_workspace_update(*no_fetch, *skip_shim)?
            };
            let payload = serde_json::to_value(&report).map_err(|error| {
                CliError::new(format!("failed to encode update report: {error}"))
            })?;
            emit_output(
                &cli,
                &payload,
                workspace_update::render_update_report(&report),
            )?;
            if !report.ok {
                return Err(CliError::new(
                    report
                        .blocked_reason
                        .unwrap_or_else(|| "Petri update did not complete".to_string()),
                ));
            }
        }
        Command::Tui {
            dish,
            no_update_check,
        } => match cli.resolved_output() {
            OutputFormat::Json => {
                let payload = if let Some(dish) = dish {
                    market_surface::dish_snapshot_payload(&backend, dish)?
                } else {
                    market_surface::dish_list_payload(&backend)?
                };
                emit_output(
                    &cli,
                    &payload,
                    market_surface::render_lab_overview(&cli, &backend, &payload),
                )?;
            }
            OutputFormat::Plain => {
                handle_lab_exit_action(lab::run_lab_bench(
                    &cli,
                    &backend,
                    dish.as_deref(),
                    !*no_update_check,
                )?)?;
            }
        },
        Command::Demo => match cli.resolved_output() {
            OutputFormat::Json => {
                let payload = market_surface::dish_snapshot_payload(&backend, "ramx")?;
                emit_output(
                    &cli,
                    &payload,
                    market_surface::render_dish_open(&cli, &backend, "ramx", &payload),
                )?;
            }
            OutputFormat::Plain => {
                if io::stdout().is_terminal() {
                    handle_lab_exit_action(lab::run_lab_bench(
                        &cli,
                        &backend,
                        Some("ramx"),
                        true,
                    )?)?;
                } else {
                    let payload = market_surface::dish_snapshot_payload(&backend, "ramx")?;
                    emit_output(
                        &cli,
                        &payload,
                        market_surface::render_dish_open(&cli, &backend, "ramx", &payload),
                    )?;
                }
            }
        },
    }

    Ok(())
}

fn handle_lab_exit_action(action: lab::LabExitAction) -> Result<(), CliError> {
    if release_update::enabled() && matches!(action, lab::LabExitAction::RunUpdate) {
        return run_petri_update_command();
    }
    let launched_by_repo_launcher = env::var("PETRI_LAUNCHER").ok().as_deref() == Some("1");
    match lab_exit_route(action, launched_by_repo_launcher) {
        LabExitRoute::Quit => Ok(()),
        LabExitRoute::LauncherExit(code) => std::process::exit(code),
        LabExitRoute::PetriUpdateCommand => run_petri_update_command(),
        LabExitRoute::DeveloperRebuild => {
            let report = workspace_update::run_workspace_update(false, false)?;
            println!("{}", workspace_update::render_update_report(&report));
            if report.ok {
                Ok(())
            } else {
                Err(CliError::new(report.blocked_reason.unwrap_or_else(|| {
                    "Petri update did not complete".to_string()
                })))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LabExitRoute {
    Quit,
    LauncherExit(i32),
    PetriUpdateCommand,
    DeveloperRebuild,
}

fn lab_exit_route(action: lab::LabExitAction, launched_by_repo_launcher: bool) -> LabExitRoute {
    match (action, launched_by_repo_launcher) {
        (lab::LabExitAction::Quit, _) => LabExitRoute::Quit,
        (lab::LabExitAction::RunUpdate, true) => {
            LabExitRoute::LauncherExit(lab::TUI_UPDATE_REQUESTED_EXIT_CODE)
        }
        (lab::LabExitAction::RunUpdate, false) => LabExitRoute::PetriUpdateCommand,
        (lab::LabExitAction::RunRebuild, true) => {
            LabExitRoute::LauncherExit(lab::TUI_REBUILD_REQUESTED_EXIT_CODE)
        }
        (lab::LabExitAction::RunRebuild, false) => LabExitRoute::DeveloperRebuild,
    }
}

fn run_petri_update_command() -> Result<(), CliError> {
    if release_update::enabled() {
        // Stay in this process: a waiting parent executable would keep the
        // Windows app locked while its update helper tries to replace it.
        let report = run_release_update(None, false, true, false)?;
        println!("{}", report.message);
        return Ok(());
    }
    let executable = env::current_exe()
        .map_err(|error| CliError::new(format!("failed to locate Petri: {error}")))?;
    println!("Running: petri update");
    let status = ProcessCommand::new(&executable)
        .arg("update")
        .status()
        .map_err(|error| CliError::new(format!("failed to start `petri update`: {error}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(CliError::new(format!(
            "`petri update` exited with status {}",
            status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        )))
    }
}

fn confirm_app_update(message: &str, yes: bool, json: bool) -> Result<bool, CliError> {
    if yes {
        return Ok(true);
    }
    if json || !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(CliError::new(
            "Check the release with petri update check, then use petri update --yes to approve it.",
        ));
    }
    use std::io::Write;
    println!("{message}");
    print!("Continue? [y/N] ");
    io::stdout()
        .flush()
        .map_err(|_| CliError::new("Could not display update confirmation."))?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .map_err(|_| CliError::new("Could not read update confirmation."))?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn run_release_update(
    command: Option<&UpdateCommand>,
    yes: bool,
    restart: bool,
    json: bool,
) -> Result<release_update::Report, CliError> {
    if matches!(command, Some(UpdateCommand::Recover)) {
        if !confirm_app_update(
            "Restore the saved previous Petri app files? Wallets and settings will not be changed.",
            yes,
            json,
        )? {
            return Err(CliError::new("Recovery cancelled. No files were changed."));
        }
        return release_update::recover(restart).map_err(CliError::new);
    }
    let report = release_update::check().map_err(CliError::new)?;
    if matches!(command, Some(UpdateCommand::Check)) || !report.update_available {
        return Ok(report);
    }
    let release = report
        .release
        .as_ref()
        .ok_or_else(|| CliError::new("Missing release information."))?;
    let prompt = format!(
        "Install Petri {} from {}?\nThis Devnet preview is not publisher-signed or Apple-notarized. Petri verifies the official GitHub download and SHA-256 before replacing app files. Wallets and settings will not be changed.",
        release.version, release.release_url
    );
    if !confirm_app_update(&prompt, yes, json)? {
        return Err(CliError::new("Update cancelled. No files were changed."));
    }
    release_update::prepare(release, restart).map_err(CliError::new)
}

fn mcp_setup_payload(
    action: &str,
    status: &mcp_setup::McpSetupStatus,
    restart_required: bool,
) -> Value {
    json!({
        "ok": true,
        "action": action,
        "codex": status.codex.as_str(),
        "claude_code": status.claude.as_str(),
        "runtime_ready": status.runtime_ready,
        "transaction_execution": "not_available",
        "signing_boundary": "The current release has no certified CLI, TUI, MCP, or hosted signing/submission path.",
        "fully_enabled": status.is_fully_enabled(),
        "partially_enabled": status.is_partially_enabled(),
        "needs_repair": status.needs_repair(),
        "has_conflict": status.has_conflict(),
        "restart_required": restart_required,
    })
}

fn render_mcp_setup_status(
    status: &mcp_setup::McpSetupStatus,
    result_message: Option<&str>,
) -> String {
    let mut lines = Vec::new();
    if let Some(message) = result_message {
        lines.push(message.to_string());
        lines.push(String::new());
    }
    lines.push("Petri MCP connection".to_string());
    lines.push(format!("  Codex:      {}", status.codex.as_str()));
    lines.push(format!("  Claude Code: {}", status.claude.as_str()));
    lines.push(format!(
        "  Runtime:    {}",
        if status.runtime_ready {
            "ready"
        } else {
            "not ready"
        }
    ));
    lines.push("  Transactions: not available through Petri MCP".to_string());
    if status.has_conflict() {
        lines.push("An existing unowned Petri MCP entry was left unchanged.".to_string());
    } else if status.needs_repair() {
        lines.push("Run `petri mcp repair` to diagnose and repair the connection.".to_string());
    }
    lines.join("\n")
}

fn parse_cli() -> Cli {
    let raw_args = env::args().collect::<Vec<_>>();
    let no_color = env::args_os().any(|arg| arg == "--no-color");
    let color_enabled = !no_color && io::stdout().is_terminal();
    if is_root_help_request(&raw_args) {
        println!(
            "{}",
            render_petri_help_sheet(
                color_enabled,
                env!("CARGO_PKG_VERSION"),
                help_sheet_terminal_width(),
            )
        );
        std::process::exit(0);
    }
    let color = if no_color || !io::stdout().is_terminal() {
        clap::ColorChoice::Never
    } else {
        clap::ColorChoice::Always
    };
    let matches = match Cli::command().color(color).try_get_matches() {
        Ok(matches) => matches,
        Err(error) => {
            let show_agent_hint = error.use_stderr();
            let exit_code = error.exit_code();
            if show_agent_hint {
                eprintln!("{}", strip_clap_help_hint(&error.to_string()));
                eprintln!("{}", agent_assist_hint());
            } else {
                let _ = error.print();
            }
            std::process::exit(exit_code);
        }
    };
    let backend_uses_default =
        matches.value_source("backend_url") == Some(clap::parser::ValueSource::DefaultValue);
    let mut cli = Cli::from_arg_matches(&matches).unwrap_or_else(|error| error.exit());
    let config_writes_backend = matches!(
        cli.command.as_ref(),
        Some(Command::Config {
            command: ConfigCommand::Set { .. } | ConfigCommand::Reset { .. }
        })
    );
    let explicit_backend = (!backend_uses_default).then_some(cli.backend_url.as_str());
    cli.backend_url = if backend_uses_default && config_writes_backend {
        petri_config::DEFAULT_BACKEND_URL.to_string()
    } else {
        petri_config::resolve_backend_url(explicit_backend).unwrap_or_else(|message| {
            Cli::command()
                .error(clap::error::ErrorKind::InvalidValue, message)
                .exit()
        })
    };
    cli
}

fn is_root_help_request(args: &[String]) -> bool {
    if args.len() <= 1 {
        return false;
    }
    let path = command_path_from_args(args);
    if path.len() == 1 && path[0] == "help" {
        return true;
    }
    path.is_empty()
        && args
            .iter()
            .skip(1)
            .any(|arg| matches!(arg.as_str(), "--help" | "-h"))
}

fn command_path_from_args(args: &[String]) -> Vec<String> {
    let value_globals = [
        "--backend-url",
        "--solana-config",
        "--cluster",
        "--commitment",
        "--keypair",
        "--output",
    ];
    let mut path = Vec::new();
    let mut index = 1;
    while index < args.len() {
        let arg = args[index].as_str();
        if arg == "--" {
            break;
        }
        if arg.starts_with("--") {
            let has_inline_value = arg.contains('=');
            let needs_value = value_globals
                .iter()
                .any(|global| arg == *global || arg.starts_with(&format!("{global}=")));
            index += 1;
            if needs_value && !has_inline_value {
                index += 1;
            }
            continue;
        }
        if arg.starts_with('-') {
            index += 1;
            continue;
        }
        path.push(arg.to_ascii_lowercase());
        if path.len() >= 3 {
            break;
        }
        index += 1;
    }
    path
}

fn agent_assist_hint() -> &'static str {
    "Agent assist: paste your goal and this Petri error into an MCP-aware coding agent such as Claude Code or Codex, and ask it to choose the right Petri command."
}

fn strip_clap_help_hint(message: &str) -> String {
    message
        .lines()
        .filter(|line| line.trim() != "For more information, try '--help'.")
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string()
}

fn contracts_to_chain_args(options: &ContractsArgs) -> Result<OptionsChainArgs, CliError> {
    let market = options
        .market
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| CliError::new("contracts requires --market <market>"))?;
    Ok(OptionsChainArgs {
        market: market.to_string(),
        expiry: options.expiry.clone(),
        rows: options.rows,
        all: options.all,
    })
}

fn run_contracts_chain_command(
    cli: &Cli,
    backend: &BackendClient,
    options: &OptionsChainArgs,
) -> Result<(), CliError> {
    let payload = market_surface::dish_snapshot_payload(backend, &options.market)?;
    validate_current_contract_snapshot(options, &payload)?;
    let mut lifecycles = BTreeMap::new();
    if options
        .expiry
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
    {
        let row = selected_current_contract_rows(options, &payload)?
            .into_iter()
            .next()
            .expect("exact contract selection was validated");
        let expiry_id = row
            .get("expiryId")
            .and_then(Value::as_str)
            .ok_or_else(|| CliError::new("selected contract series has no expiry id"))?;
        let lifecycle = fetch_authoritative_oracle_phase(backend, &options.market, expiry_id);
        lifecycles.insert(expiry_id.to_string(), lifecycle);
    }
    let agent_payload = options_chain_agent_payload(options, &payload, &lifecycles)?;
    let plain = render_options_chain(&agent_payload);
    emit_output(cli, &agent_payload, plain)
}

fn current_contract_snapshot<'a>(
    options: &OptionsChainArgs,
    payload: &'a Value,
) -> Result<&'a Value, CliError> {
    let data = unwrap_data(payload);
    let snapshot = data.get("snapshot").unwrap_or(data);
    let requested_market = options.market.trim().to_ascii_lowercase();
    if snapshot.get("marketId").and_then(Value::as_str) != Some(requested_market.as_str())
        || !matches!(snapshot.get("onChainAvailable"), Some(Value::Bool(_)))
    {
        return Err(CliError::new(
            "current contract snapshot has an invalid market identity or availability flag",
        ));
    }
    Ok(snapshot)
}

fn current_contract_rows<'a>(
    options: &OptionsChainArgs,
    payload: &'a Value,
) -> Result<&'a Vec<Value>, CliError> {
    current_contract_snapshot(options, payload)?
        .get("expiries")
        .and_then(Value::as_array)
        .ok_or_else(|| CliError::new("current contract snapshot is missing series rows"))
}

fn selected_current_contract_rows<'a>(
    options: &OptionsChainArgs,
    payload: &'a Value,
) -> Result<Vec<&'a Value>, CliError> {
    let rows = current_contract_rows(options, payload)?;
    let has_exact_expiry = options
        .expiry
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    let mut selected = match options.expiry.as_deref().map(str::trim) {
        Some(requested) if !requested.is_empty() => rows
            .iter()
            .filter(|row| row.get("expiryId").and_then(Value::as_str) == Some(requested))
            .collect::<Vec<_>>(),
        _ => rows.iter().collect::<Vec<_>>(),
    };
    if has_exact_expiry && selected.len() != 1 {
        return Err(CliError::new(
            "requested current contract series is not uniquely present",
        ));
    }
    selected.sort_by(|left, right| {
        left.get("settlementUtc")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .cmp(
                right
                    .get("settlementUtc")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            )
            .then_with(|| {
                left.get("optionKind")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .cmp(
                        right
                            .get("optionKind")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                    )
            })
            .then_with(|| {
                left.get("expiryId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .cmp(
                        right
                            .get("expiryId")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                    )
            })
    });
    if !has_exact_expiry && !options.all {
        if !(1..=36).contains(&options.rows) {
            return Err(CliError::new("contracts --rows must be from 1 through 36"));
        }
        selected.truncate(options.rows);
    }
    if selected.is_empty() {
        return Err(CliError::new(
            "no current contract series matched the selection",
        ));
    }
    Ok(selected)
}

fn validate_current_contract_snapshot(
    options: &OptionsChainArgs,
    payload: &Value,
) -> Result<(), CliError> {
    let market_id = options.market.trim().to_ascii_lowercase();
    let rows = current_contract_rows(options, payload)?;
    if rows.is_empty() {
        return Err(CliError::new(
            "current contract snapshot has no series rows",
        ));
    }
    for row in rows {
        validate_current_contract_row(&market_id, row)?;
    }
    let _ = selected_current_contract_rows(options, payload)?;
    Ok(())
}

fn validate_current_contract_row(market_id: &str, row: &Value) -> Result<(), CliError> {
    market_surface::validate_current_market_row(market_id, row)
}

fn run_chart_command(
    cli: &Cli,
    backend: &BackendClient,
    options: &ChartArgs,
) -> Result<(), CliError> {
    if should_open_guide_enabled_chart(cli, options, io::stdout().is_terminal()) {
        return handle_lab_exit_action(lab::run_lab_chart_bench(cli, backend, options, true)?);
    }
    let fetch = chart::fetch_chart_payload(backend, options)?;
    match cli.resolved_output() {
        OutputFormat::Json => emit_output(
            cli,
            &fetch.payload,
            chart::render_chart_static(cli, backend, &fetch),
        ),
        OutputFormat::Plain => emit_output(
            cli,
            &fetch.payload,
            chart::render_chart_static(cli, backend, &fetch),
        ),
    }
}

fn should_open_guide_enabled_chart(cli: &Cli, options: &ChartArgs, is_terminal: bool) -> bool {
    cli.resolved_output() == OutputFormat::Plain && !options.static_view && is_terminal
}

fn run_wallet_address_command(cli: &Cli) -> Result<(), CliError> {
    let config = build_onchain_config(cli)?;
    let pubkey = load_keypair_pubkey(&config)?;
    let payload = json!({ "pubkey": pubkey });
    emit_output(cli, &payload, pubkey)
}

fn run_wallet_balance_command(
    cli: &Cli,
    owner_pubkey: Option<&str>,
    usdc_mint: Option<&str>,
    amba_mint: Option<&str>,
) -> Result<(), CliError> {
    let config = build_onchain_config(cli)?;
    let owner = resolve_wallet_owner_pubkey(&config, owner_pubkey)?;
    let usdc_mint = wallet_balance::resolve_wallet_usdc_mint(&cli.cluster, usdc_mint)?;
    let amba_mint = wallet_balance::resolve_wallet_amba_mint(amba_mint);
    let payload = wallet_balance::read_wallet_balance(&config, &owner, &usdc_mint, &amba_mint)?;
    emit_output(
        cli,
        &payload,
        wallet_balance::render_wallet_balance(&payload),
    )
}

pub(crate) fn validate_current_collateral_payload(
    data: &Value,
    expected_owner: &Pubkey,
) -> Result<(), CliError> {
    let data = exact_object_fields(data, &["protocol", "collateral"], "current collateral data")?;
    let collateral = data
        .get("collateral")
        .ok_or_else(|| CliError::new("current collateral response is missing collateral"))?;
    let collateral_value = collateral;
    let collateral = exact_object_fields(
        collateral,
        &[
            "stateNamespace",
            "cluster",
            "releaseTag",
            "releaseCommit",
            "ownerPubkey",
            "userCollateralPda",
            "userCollateralExists",
            "availableBalance",
            "lockedBalance",
            "requiredBalance",
            "canSubmitTrade",
            "lastActionSlot",
            "statusMessage",
        ],
        "current collateral",
    )?;
    let namespace = std::str::from_utf8(ameba_sdk::constants::CURRENT_STATE_NAMESPACE_SEED)
        .expect("SDK namespace is UTF-8");
    if collateral.get("stateNamespace").and_then(Value::as_str) != Some(namespace)
        || collateral.get("cluster").and_then(Value::as_str)
            != Some(ameba_sdk::protocol::CURRENT_PROTOCOL_CLUSTER)
        || collateral.get("releaseTag").and_then(Value::as_str)
            != Some(ameba_sdk::protocol::CURRENT_PROTOCOL_RELEASE)
        || collateral.get("releaseCommit").and_then(Value::as_str)
            != Some(ameba_sdk::protocol::CURRENT_PROTOCOL_SOURCE_COMMIT)
        || collateral
            .get("userCollateralExists")
            .and_then(Value::as_bool)
            .is_none()
        || collateral
            .get("canSubmitTrade")
            .and_then(Value::as_bool)
            .is_none()
        || collateral
            .get("statusMessage")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
    {
        return Err(CliError::new(
            "current collateral response does not match the current projection",
        ));
    }
    let owner = required_prepared_pubkey(
        collateral_value,
        &["ownerPubkey"],
        "current collateral.ownerPubkey",
    )?;
    let expected_pda =
        ameba_sdk::protocol::derive_user_collateral_pda(&ameba_sdk::ID, expected_owner).0;
    if owner != *expected_owner
        || required_prepared_pubkey(
            collateral_value,
            &["userCollateralPda"],
            "current collateral.userCollateralPda",
        )? != expected_pda
    {
        return Err(CliError::new(
            "current collateral response is bound to a foreign owner or PDA",
        ));
    }
    for field in ["availableBalance", "lockedBalance", "requiredBalance"] {
        canonical_prepared_u64(
            collateral
                .get(field)
                .ok_or_else(|| CliError::new(format!("current collateral is missing {field}")))?,
            &format!("current collateral.{field}"),
        )?;
    }
    if collateral.get("lastActionSlot").is_some_and(|value| {
        !value.is_null()
            && canonical_prepared_u64(value, "current collateral.lastActionSlot").is_err()
    }) {
        return Err(CliError::new(
            "current collateral lastActionSlot is not canonical",
        ));
    }
    Ok(())
}

fn render_current_collateral(data: &Value) -> String {
    let collateral = data.get("collateral").unwrap_or(&Value::Null);
    let field = |key: &str| string_at_key(collateral, &[key]).unwrap_or_else(|| "-".to_string());
    let exists = collateral.get("userCollateralExists") == Some(&Value::Bool(true));
    let can_trade = collateral.get("canSubmitTrade") == Some(&Value::Bool(true));
    let next = if !exists {
        "Collateral account is absent. Initialization remains unavailable until the SDK/Lean prepare route proves this PDA is absent."
    } else if can_trade {
        "Collateral is ready for bounded current trades."
    } else {
        "Collateral is not sufficient for the selected trade; deposit/withdraw preparation remains validation-only until the current action contract is frozen."
    };
    [
        format!(
            "wallet={} | collateralPda={}",
            field("ownerPubkey"),
            field("userCollateralPda")
        ),
        format!(
            "available={} | locked={} | required={} | canSubmitTrade={can_trade}",
            field("availableBalance"),
            field("lockedBalance"),
            field("requiredBalance"),
        ),
        format!("status={}", field("statusMessage")),
        format!("Next: {next}"),
    ]
    .join("\n")
}

fn run_wallet_collateral_command(
    cli: &Cli,
    backend: &BackendClient,
    owner: Option<&str>,
) -> Result<(), CliError> {
    let config = build_onchain_config(cli)?;
    let owner = resolve_wallet_owner_pubkey(&config, owner)?;
    let owner_pubkey = Pubkey::from_str(&owner)
        .map_err(|error| CliError::new(format!("invalid owner pubkey {owner}: {error}")))?;
    let response = backend.get(&endpoints::user_collateral(&owner))?;
    let data = current_trade_response_data(response, "collateral")?;
    validate_current_collateral_payload(&data, &owner_pubkey)?;
    emit_output(cli, &data, render_current_collateral(&data))
}

fn run_liquidity_positions_command(
    cli: &Cli,
    backend: &BackendClient,
    owner: Option<&str>,
) -> Result<(), CliError> {
    let config = build_onchain_config(cli)?;
    let owner = resolve_wallet_owner_pubkey(&config, owner)?;
    let response = current_backend_payload(backend.get(&endpoints::dlmm_positions(&owner))?)?;
    let payload = positions::project_liquidity_positions(&response, &owner)?;
    emit_output(
        cli,
        &payload,
        positions::render_liquidity_positions(&payload),
    )
}

fn ensure_liquidity_owner_scope(has_subcommand: bool, owner: Option<&str>) -> Result<(), CliError> {
    if has_subcommand && owner.is_some() {
        return Err(CliError::new(
            "liquidity --owner is available only for the read-only position list; omit --owner when using a liquidity subcommand",
        ));
    }
    Ok(())
}

fn run_history_command(
    cli: &Cli,
    backend: &BackendClient,
    history: &HistoryArgs,
) -> Result<(), CliError> {
    ensure_authoritative_history_type(history.activity_type)?;
    let config = build_onchain_config(cli)?;
    let owner = resolve_wallet_owner_pubkey(&config, history.owner_pubkey.as_deref())?;
    let limit = usize::from(history.limit).clamp(1, 250);
    let (chain_history, mut issues) =
        match solana_history::fetch_account_history(&config, &owner, limit) {
            Ok(history) => (history, Vec::new()),
            Err(_) => (
                solana_history::unavailable_account_history(&owner, limit),
                vec![solana_history::wallet_activity_unavailable_issue()],
            ),
        };
    let path = endpoints::user_ledger(&owner, limit);
    let product_ledger = match backend.get(&path) {
        Ok(payload) => {
            chain_identity::validate_current_backend_envelope(&payload)?;
            Some(payload)
        }
        Err(_) => {
            issues.push(solana_history::trade_history_unavailable_issue());
            None
        }
    };
    let normalized_program_registry = solana_history::normalize_amoeba_program_registry(None);
    let amoeba_activity =
        solana_history::scan_amoeba_activity(&config, &chain_history, None, limit);
    let mut payload = solana_history::build_account_ledger_payload_with_activity(
        &owner,
        chain_history,
        product_ledger,
        Some(amoeba_activity),
        Some(normalized_program_registry),
        issues,
    );
    if let Some(object) = payload.as_object_mut() {
        object.insert(
            "filterType".to_string(),
            json!(history_type_label(history.activity_type)),
        );
    }
    emit_output(
        cli,
        &payload,
        render_trade_ledger(cli, backend, &owner, &payload),
    )
}

fn history_type_label(value: HistoryTypeValue) -> &'static str {
    match value {
        HistoryTypeValue::Trades => "trades",
        HistoryTypeValue::Claims => "claims",
        HistoryTypeValue::Oracle => "oracle",
        HistoryTypeValue::All => "all",
    }
}

fn ensure_authoritative_history_type(value: HistoryTypeValue) -> Result<(), CliError> {
    if value == HistoryTypeValue::All {
        return Ok(());
    }
    Err(CliError::new(format!(
        "history --type {} is unavailable because current transaction evidence cannot authoritatively classify that activity; use --type all",
        history_type_label(value)
    )))
}

fn render_cli_front_door(cli: &Cli) -> String {
    let output_is_tty = io::stdout().is_terminal();
    let terminal_width = if output_is_tty {
        crossterm::terminal::size()
            .map(|(width, _)| usize::from(width))
            .unwrap_or(terminal_brand::WIDE_BANNER_MIN_COLUMNS)
    } else {
        terminal_brand::WIDE_BANNER_MIN_COLUMNS
    };
    render_cli_front_door_for_terminal(cli, output_is_tty, terminal_width)
}

fn render_cli_front_door_for_terminal(
    cli: &Cli,
    output_is_tty: bool,
    terminal_width: usize,
) -> String {
    let mut lines = if output_is_tty && cli.resolved_output() != OutputFormat::Json {
        terminal_brand::terminal_banner_lines_for_width(terminal_width)
            .into_iter()
            .map(|line| {
                render_cli_brand_line_enabled(
                    terminal_brand::color_enabled(cli.no_color, output_is_tty),
                    &line,
                )
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let wallet = attached_wallet::inspect_attached_wallet(cli);
    lines.push(front_door_paint(
        cli,
        ANSI_DIM,
        &attached_wallet_plain_line(&wallet),
    ));
    lines.push(String::new());
    lines.push("Inspect bounded hardware markets from the command line.".to_string());
    lines.push("View risk, wallet exposure, oracle evidence, and settlement state.".to_string());
    lines.push(
        "V3 support: wallet changes require an active gate and initialized markets; unavailable release evidence returns CURRENT_PROGRAM_WRITE_ABI_UNAVAILABLE."
            .to_string(),
    );
    lines.push("Use `petri help` for help.".to_string());
    lines.push(String::new());
    lines.push(front_door_heading(cli, "Usage:"));
    lines.push(format!(
        "  {}",
        front_door_paint(cli, ANSI_BOLD_GREEN, "petri <command> [options]")
    ));
    lines.push(String::new());
    lines.push(front_door_heading(cli, "Common commands:"));
    lines.push(front_door_command_line(
        cli,
        "petri markets",
        "Find live hardware markets",
    ));
    lines.push(front_door_command_line(
        cli,
        "petri markets show <market>",
        "Open one market",
    ));
    lines.push(front_door_command_line(
        cli,
        "petri markets chart <market>",
        "View price history",
    ));
    lines.push(front_door_command_line(
        cli,
        "petri contracts --market <market>",
        "Browse tradable contracts",
    ));
    lines.push(front_door_command_line(
        cli,
        "petri writers list",
        "Show collective writer sleeves",
    ));
    lines.push(front_door_command_line(
        cli,
        "petri liquidity",
        "Request manager positions (current route may fail closed)",
    ));
    lines.push(front_door_command_line(
        cli,
        "petri staking",
        "View AMBA and sAMBA staking",
    ));
    lines.push(front_door_command_line(
        cli,
        "petri history",
        "Review recent activity",
    ));
    lines.push(front_door_command_line(
        cli,
        "petri oracle drafts",
        "Review oracle evidence drafts",
    ));
    lines.push(front_door_command_line(
        cli,
        "petri config show",
        "Show user-safe config",
    ));
    lines.push(front_door_command_line(cli, "petri tui", "Open Lab Bench"));
    lines.push(String::new());
    lines.push(front_door_heading(cli, "Help:"));
    lines.push(front_door_command_line(
        cli,
        "petri help",
        "Show all command families",
    ));
    lines.push(front_door_command_line(
        cli,
        "petri help <command>",
        "Show flags for one command",
    ));
    lines.push(String::new());
    lines.push(front_door_heading(cli, "Agent assist:"));
    lines.push(
        "  Paste your goal into an MCP-aware coding agent such as Claude Code or Codex".to_string(),
    );
    lines.push("  and ask it to choose the right Petri command.".to_string());
    lines.join("\n")
}

const ANSI_RESET: &str = "\x1b[0m";
const ANSI_BOLD_CYAN: &str = "\x1b[1;36m";
const ANSI_BOLD_GREEN: &str = "\x1b[1;32m";
const ANSI_BRAND_TITLE: &str = "\x1b[38;2;78;165;202;48;2;78;165;202m";
const ANSI_BRAND_OUTLINE: &str = "\x1b[38;2;229;229;229m";
const ANSI_BRAND_BODY: &str = "\x1b[38;2;231;231;231m";
const ANSI_BOLD_YELLOW: &str = "\x1b[1;33m";
const ANSI_DIM: &str = "\x1b[2m";

fn render_cli_brand_line_enabled(
    color_enabled: bool,
    line: &[terminal_brand::BrandSpan],
) -> String {
    line.iter()
        .map(|span| {
            let code = match span.tone {
                terminal_brand::BrandTone::Title => ANSI_BRAND_TITLE,
                terminal_brand::BrandTone::Outline => ANSI_BRAND_OUTLINE,
                terminal_brand::BrandTone::Body => ANSI_BRAND_BODY,
                terminal_brand::BrandTone::Plain => "",
            };
            let text = if color_enabled && span.fill {
                " ".repeat(span.text.chars().count())
            } else {
                span.text.clone()
            };
            front_door_paint_enabled(color_enabled && !code.is_empty(), code, &text)
        })
        .collect::<String>()
        .trim_end()
        .to_string()
}

fn front_door_heading(cli: &Cli, text: &str) -> String {
    front_door_paint(cli, ANSI_BOLD_CYAN, text)
}

fn front_door_command_line(cli: &Cli, command: &str, description: &str) -> String {
    let command_cell = format!("{command:<40}");
    format!(
        "  {} {}",
        front_door_paint(cli, ANSI_BOLD_GREEN, &command_cell),
        description
    )
}

fn front_door_paint(cli: &Cli, code: &str, text: &str) -> String {
    front_door_paint_enabled(!cli.no_color && io::stdout().is_terminal(), code, text)
}

fn front_door_paint_enabled(enabled: bool, code: &str, text: &str) -> String {
    if enabled {
        format!("{code}{text}{ANSI_RESET}")
    } else {
        text.to_string()
    }
}

#[derive(Clone, Copy)]
struct HelpSheetRow {
    name: &'static str,
    description: &'static str,
}

struct HelpSheetOption<'a> {
    flag: &'a str,
    description: &'a [&'a str],
}

const HELP_SHEET_DEFAULT_WIDTH: usize = 100;
const HELP_SHEET_MIN_WIDTH: usize = 24;
const HELP_REFERENCE_MAX_WIDTH: usize = 100;

const HELP_SHEET_COMMANDS: &[HelpSheetRow] = &[
    HelpSheetRow {
        name: "markets",
        description: "List markets or open a market view",
    },
    HelpSheetRow {
        name: "contracts",
        description: "Browse listed option contracts",
    },
    HelpSheetRow {
        name: "trades",
        description: "Review trade grammar (current release is read-only)",
    },
    HelpSheetRow {
        name: "writers",
        description: "Inspect writer sleeves (wallet changes unavailable)",
    },
    HelpSheetRow {
        name: "liquidity",
        description: "Request manager positions or preview liquidity inputs",
    },
    HelpSheetRow {
        name: "wallet",
        description: "Read wallet address and balances",
    },
    HelpSheetRow {
        name: "staking",
        description: "View AMBA/sAMBA state (wallet changes unavailable)",
    },
    HelpSheetRow {
        name: "history",
        description: "Read indexed wallet history",
    },
    HelpSheetRow {
        name: "settlements",
        description: "Inspect settlement records",
    },
    HelpSheetRow {
        name: "oracle",
        description: "Read current oracle state and review semantic drafts",
    },
    HelpSheetRow {
        name: "config",
        description: "Inspect or update Amoeba service configuration",
    },
    HelpSheetRow {
        name: "mcp",
        description: "Connect Petri to supported AI agents",
    },
    HelpSheetRow {
        name: "update",
        description: "Check for and install Petri updates",
    },
    HelpSheetRow {
        name: "tui",
        description: "Open the interactive Lab Bench TUI",
    },
    HelpSheetRow {
        name: "demo",
        description: "Open the current RAMX market",
    },
    HelpSheetRow {
        name: "help",
        description: "Print this message or command-specific help",
    },
];

const HELP_SHEET_OPTIONS: &[HelpSheetOption<'static>] = &[
    HelpSheetOption {
        flag: "-h, --help",
        description: &["Print help"],
    },
    HelpSheetOption {
        flag: "-V, --version",
        description: &["Print version"],
    },
    HelpSheetOption {
        flag: "--json",
        description: &["Shortcut for --output json"],
    },
    HelpSheetOption {
        flag: "--quiet",
        description: &["Suppress plain output;", "JSON output is still printed"],
    },
    HelpSheetOption {
        flag: "--yes",
        description: &["Compatibility flag; wallet", "changes remain unavailable"],
    },
    HelpSheetOption {
        flag: "--no-color",
        description: &["Disable terminal color"],
    },
];

fn help_sheet_terminal_width() -> usize {
    if io::stdout().is_terminal() {
        crossterm::terminal::size()
            .map(|(width, _)| usize::from(width))
            .unwrap_or(HELP_SHEET_DEFAULT_WIDTH)
    } else {
        HELP_SHEET_DEFAULT_WIDTH
    }
}

fn render_petri_help_sheet(color_enabled: bool, _version: &str, terminal_width: usize) -> String {
    let width = terminal_width.clamp(HELP_SHEET_MIN_WIDTH, HELP_REFERENCE_MAX_WIDTH);
    render_petri_help_reference(color_enabled, width)
}

fn render_petri_help_reference(color_enabled: bool, width: usize) -> String {
    let mut lines = Vec::new();
    lines.push(help_sheet_paint(
        color_enabled,
        ANSI_BOLD_YELLOW,
        "Petri CLI",
    ));
    lines.push(String::new());
    lines.extend(help_sheet_wrap_words(
        "Trade monthly RAM and NAND hardware markets with fixed maximum loss and transparent oracle settlement.",
        width,
    ));
    lines.push(String::new());
    lines.extend(help_sheet_wrap_words(
        "If no subcommand is specified, Petri opens the market front door.",
        width,
    ));
    lines.push(String::new());
    lines.push(help_sheet_paint(color_enabled, ANSI_BOLD_CYAN, "Usage:"));
    lines.extend(help_sheet_usage_reference_lines(color_enabled, width));
    lines.push(String::new());
    lines.push(help_sheet_paint(color_enabled, ANSI_BOLD_CYAN, "Commands:"));
    for row in HELP_SHEET_COMMANDS {
        push_help_reference_row(
            &mut lines,
            color_enabled,
            row.name,
            row.description,
            width,
            14,
            ANSI_BOLD_CYAN,
        );
    }
    lines.push(String::new());
    lines.push(help_sheet_paint(color_enabled, ANSI_BOLD_CYAN, "Options:"));
    for option in HELP_SHEET_OPTIONS {
        let description = option.description.join(" ");
        push_help_reference_row(
            &mut lines,
            color_enabled,
            option.flag,
            &description,
            width,
            22,
            ANSI_BOLD_CYAN,
        );
    }
    push_help_reference_row(
        &mut lines,
        color_enabled,
        "--output <OUTPUT>",
        "Output format: plain or json",
        width,
        22,
        ANSI_BOLD_CYAN,
    );
    push_help_reference_row(
        &mut lines,
        color_enabled,
        "--backend-url <URL>",
        "Use a different Amoeba service origin",
        width,
        22,
        ANSI_BOLD_CYAN,
    );
    push_help_reference_row(
        &mut lines,
        color_enabled,
        "--keypair <PATH>",
        "Resolve a wallet address for inspection only",
        width,
        22,
        ANSI_BOLD_CYAN,
    );
    lines.push(String::new());
    lines.push(help_sheet_paint(color_enabled, ANSI_BOLD_CYAN, "Help:"));
    push_help_reference_row(
        &mut lines,
        color_enabled,
        "petri help <command>",
        "Show flags and examples for one command",
        width,
        22,
        ANSI_BOLD_CYAN,
    );
    push_help_reference_row(
        &mut lines,
        color_enabled,
        "petri <command> --help",
        "Show command-specific options",
        width,
        22,
        ANSI_BOLD_CYAN,
    );
    lines.join("\n")
}

fn help_sheet_usage_reference_lines(color_enabled: bool, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for usage in [
        "petri [OPTIONS] [COMMAND]",
        "petri [OPTIONS] <COMMAND> [ARGS]",
    ] {
        let indent = 2.min(width.saturating_sub(1));
        let body_width = width.saturating_sub(indent).max(1);
        for line in help_sheet_wrap_words(usage, body_width) {
            lines.push(format!(
                "{}{}",
                " ".repeat(indent),
                help_sheet_paint(color_enabled, ANSI_BOLD_GREEN, &line)
            ));
        }
    }
    lines
}

fn push_help_reference_row(
    lines: &mut Vec<String>,
    color_enabled: bool,
    key: &str,
    description: &str,
    width: usize,
    key_width: usize,
    key_color: &str,
) {
    let indent = 2.min(width.saturating_sub(1));
    let content_width = width.saturating_sub(indent).max(1);
    let key_len = key.chars().count();
    if key_len <= key_width && content_width >= key_width + 14 {
        let description_width = content_width.saturating_sub(key_width + 2).max(1);
        let description_lines = help_sheet_wrap_words(description, description_width);
        lines.push(format!(
            "{}{}  {}",
            " ".repeat(indent),
            help_sheet_cell(color_enabled, key, key_width, key_color),
            description_lines.first().cloned().unwrap_or_default()
        ));
        for line in description_lines.into_iter().skip(1) {
            lines.push(format!(
                "{}{}  {}",
                " ".repeat(indent),
                " ".repeat(key_width),
                line
            ));
        }
        return;
    }

    let key_width = content_width;
    for key_line in help_sheet_wrap_words(key, key_width) {
        lines.push(format!(
            "{}{}",
            " ".repeat(indent),
            help_sheet_paint(color_enabled, key_color, &key_line)
        ));
    }
    let description_indent = (indent + 2).min(width.saturating_sub(1));
    let description_width = width.saturating_sub(description_indent).max(1);
    for line in help_sheet_wrap_words(description, description_width) {
        lines.push(format!("{}{}", " ".repeat(description_indent), line));
    }
}

fn help_sheet_wrap_words(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    if text.is_empty() {
        return vec![String::new()];
    }

    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let word_len = word.chars().count();
        if word_len > width {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            let mut chunk = String::new();
            let mut chunk_len = 0;
            for ch in word.chars() {
                if chunk_len == width {
                    lines.push(chunk);
                    chunk = String::new();
                    chunk_len = 0;
                }
                chunk.push(ch);
                chunk_len += 1;
            }
            current = chunk;
        } else if current.is_empty() {
            current.push_str(word);
        } else if current.chars().count() + 1 + word_len <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(current);
            current = word.to_string();
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        vec![String::new()]
    } else {
        lines
    }
}

fn help_sheet_cell(color_enabled: bool, text: &str, width: usize, code: &str) -> String {
    let padded = help_sheet_plain_cell(text, width);
    help_sheet_paint(color_enabled, code, &padded)
}

fn help_sheet_plain_cell(text: &str, width: usize) -> String {
    let clipped = if text.chars().count() > width {
        text.chars().take(width).collect::<String>()
    } else {
        text.to_string()
    };
    format!("{clipped:<width$}")
}

fn help_sheet_paint(color_enabled: bool, code: &str, text: &str) -> String {
    front_door_paint_enabled(color_enabled, code, text)
}

fn attached_wallet_plain_line(wallet: &attached_wallet::AttachedWallet) -> String {
    let account = wallet
        .pubkey
        .as_deref()
        .map(short_pubkey)
        .unwrap_or_else(|| "not attached".to_string());
    format!(
        "Account: {account} | wallet {} | {}",
        wallet.status_label(),
        wallet.path_source.label()
    )
}

fn attached_wallet_payload(wallet: &attached_wallet::AttachedWallet) -> Value {
    json!({
        "attached": wallet.is_attached(),
        "accountPubkey": wallet.pubkey,
        "keypairPath": wallet.keypair_path,
        "keypairSource": wallet.path_source.label(),
        "keypairFile": wallet.keypair_file,
        "issue": wallet.issue,
    })
}

fn short_pubkey(value: &str) -> String {
    if value.len() <= 16 {
        return value.to_string();
    }
    format!("{}...{}", &value[..8], &value[value.len() - 6..])
}

fn resolve_oracle_draft_path(path: Option<&str>) -> Result<PathBuf, CliError> {
    path.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(oracle_submissions::default_path)
        .ok_or_else(|| {
            CliError::new(
                "oracle draft store path is unavailable; set AMEBA_ORACLE_SUBMISSIONS_PATH",
            )
        })
}

fn nonempty_trimmed(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn build_oracle_drafts_list_payload(path: &PathBuf, limit: usize) -> Result<Value, CliError> {
    let drafts = oracle_submissions::load_recent_at_path(path, limit)?;
    Ok(json!({
        "ok": true,
        "action": "oracle_drafts_list",
        "path": path.display().to_string(),
        "count": drafts.len(),
        "drafts": drafts,
    }))
}

fn load_oracle_index_tree(
    backend: &BackendClient,
    market_id: &str,
) -> Result<oracle_tui::OracleIndexTree, CliError> {
    let market_id = market_id.trim();
    if market_id.is_empty() {
        return Err(CliError::new("oracle market id is required"));
    }
    let load_error = || {
        CliError::new(format!(
            "Could not load {} oracle source recipe. Try again when the oracle is reachable.",
            market_id.to_uppercase()
        ))
    };
    let payload = backend
        .get(&endpoints::dlmm_oracle_market(market_id))
        .map_err(|_| load_error())?;
    chain_identity::validate_current_backend_envelope(&payload)?;
    oracle_tui::OracleIndexTree::from_payload(&payload).map_err(|_| load_error())
}

fn build_oracle_recipe_payload(
    tree: &oracle_tui::OracleIndexTree,
    query: Option<&str>,
    limit: usize,
) -> Value {
    let query = query.map(str::trim).filter(|value| !value.is_empty());
    let limit = limit.max(1);
    let matches = query
        .map(|query| tree.search_nodes(query))
        .unwrap_or_else(|| tree.child_indices(tree.root_index()));
    let rows = tree
        .row_bucket_indices()
        .into_iter()
        .map(|index| oracle_node_payload(tree, index))
        .collect::<Vec<_>>();
    let nodes = matches
        .into_iter()
        .take(limit)
        .map(|index| oracle_node_payload(tree, index))
        .collect::<Vec<_>>();

    json!({
        "ok": true,
        "action": "oracle_recipe",
        "marketId": tree.market_id,
        "recipeId": format!("{}-oracle-index", tree.market_id),
        "root": oracle_node_payload(tree, tree.root_index()),
        "sourceTipRows": tree.row_bucket_count,
        "terminalPinCount": tree.terminal_pin_count,
        "query": query.unwrap_or(""),
        "matchedCount": nodes.len(),
        "nodes": nodes,
        "rowBuckets": rows,
        "agentNotes": [
            "Use terminal source pins for source proposals, opening prints, update claims, and challenges.",
            "Row bucket weights apply once after source-local deltas are aggregated inside the row.",
            "Drafts are local until previewed, prepared by the Amoeba API, and explicitly executed as ameba_spread transactions."
        ],
    })
}

fn oracle_node_payload(tree: &oracle_tui::OracleIndexTree, index: usize) -> Value {
    let Some(node) = tree.node(index) else {
        return json!({
            "index": index,
            "missing": true,
        });
    };
    json!({
        "index": index,
        "nodeId": node.node_id,
        "label": node.label,
        "kind": node.kind.label(),
        "parentIndex": node.parent,
        "weightPct": node.weight_pct,
        "rowWeightPct": node.row_weight_pct,
        "pinCount": node.pin_count,
        "breadcrumb": tree.breadcrumb(index),
        "childIndices": tree.child_indices(index),
        "firstTerminalPinIndex": tree.first_terminal_pin(index),
        "rowBucketIndex": tree.row_bucket_for(index),
    })
}

fn create_oracle_source_proposal_draft(
    args: &OracleSourceProposeArgs,
    tree: &oracle_tui::OracleIndexTree,
) -> Result<Value, CliError> {
    ensure_allowed_source_category(&args.source_category)?;
    ensure_positive_finite("stake-support", args.stake_support)?;
    let node_index = resolve_oracle_node(tree, &args.node, true)?;
    let node = tree
        .node(node_index)
        .ok_or_else(|| CliError::new("selected oracle node is unavailable"))?;
    let row_label = oracle_row_label(tree, node_index);
    let fields = vec![
        oracle_field("Terminal source pin", node.label.as_str()),
        oracle_field("Source category", args.source_category.trim()),
        oracle_field("Canonical locator", args.canonical_locator.trim()),
        oracle_field("Source definition", args.source_definition.trim()),
        oracle_field("Bucket/row target", row_label),
        oracle_field("Stake/support", args.stake_support.to_string()),
    ];
    ensure_nonempty_field(&fields, "Canonical locator")?;
    ensure_nonempty_field(&fields, "Source definition")?;
    let draft = build_oracle_draft(
        tree,
        &args.common.market,
        &args.common.month,
        args.common.expiry.as_deref(),
        "Propose source",
        "Placement",
        node_index,
        "PLACED",
        None,
        "SourceRegistry + ScrambleManager",
        format!(
            "Source category={}; Canonical locator={}",
            args.source_category.trim(),
            args.canonical_locator.trim()
        ),
        fields,
    );
    save_oracle_draft(args.common.path.as_deref(), draft)
}

fn create_oracle_source_support_draft(
    args: &OracleSourceSupportArgs,
    tree: &oracle_tui::OracleIndexTree,
) -> Result<Value, CliError> {
    ensure_positive_finite("stake", args.stake)?;
    let node_index = resolve_oracle_node(tree, &args.node, false)?;
    let node = tree
        .node(node_index)
        .ok_or_else(|| CliError::new("selected oracle node is unavailable"))?;
    if !matches!(
        node.kind,
        oracle_tui::OracleNodeKind::RowBucket | oracle_tui::OracleNodeKind::TerminalPin
    ) {
        return Err(CliError::new(
            "source support drafts require a row bucket or terminal source pin",
        ));
    }
    let target = if node.kind == oracle_tui::OracleNodeKind::TerminalPin {
        node.label.clone()
    } else {
        oracle_row_label(tree, node_index)
    };
    let fields = vec![
        oracle_field("Target row/source", target),
        oracle_field("Stake/support", args.stake.to_string()),
        oracle_field("Support note", args.note.trim()),
    ];
    let draft = build_oracle_draft(
        tree,
        &args.common.market,
        &args.common.month,
        args.common.expiry.as_deref(),
        "Back source",
        "Placement",
        node_index,
        "PLACED",
        None,
        "Treasury + StakeLedger + WeightEngine",
        format!("Stake/support={}", args.stake),
        fields,
    );
    save_oracle_draft(args.common.path.as_deref(), draft)
}

fn create_oracle_emergency_commit_draft(
    args: &OracleEmergencyCommitArgs,
) -> Result<Value, CliError> {
    if args.samba_amount == 0 {
        return Err(CliError::new("samba-amount must be positive"));
    }
    let commit_hash = normalize_required_32_byte_hex("commit-hash", &args.commit_hash)?;
    let fields = vec![
        oracle_field("Commit hash", commit_hash.as_str()),
        oracle_field("sAMBA amount", args.samba_amount.to_string()),
    ];
    let draft = build_oracle_direct_draft(
        &args.common,
        "Commit emergency vote",
        "Emergency Vote",
        "emergency vote",
        "emergency vote",
        "emergency vote",
        "EMERGENCY_OPEN",
        Some("COMMITTED"),
        "EmergencyGovernor",
        format!(
            "Commit hash={commit_hash}; sAMBA amount={}",
            args.samba_amount
        ),
        fields,
    );
    save_oracle_draft(args.common.path.as_deref(), draft)
}

fn create_oracle_emergency_reveal_draft(
    args: &OracleEmergencyRevealArgs,
) -> Result<Value, CliError> {
    normalize_required_32_byte_hex("salt", &args.salt)?;
    let choice = normalize_emergency_choice(
        "choice",
        None,
        required_trimmed_value("choice", &args.choice)?,
    )?;
    let fields = vec![
        oracle_field("Choice", choice.as_str()),
        oracle_secret_field("Salt"),
    ];
    let draft = build_oracle_direct_draft(
        &args.common,
        "Reveal emergency vote",
        "Emergency Vote",
        "emergency vote",
        "emergency vote",
        "emergency vote",
        "EMERGENCY_OPEN",
        Some("REVEALED"),
        "EmergencyGovernor",
        format!("Emergency vote choice={}", choice.as_str()),
        fields,
    );
    save_oracle_draft(args.common.path.as_deref(), draft)
}

fn create_oracle_amba_custody_draft(
    args: &OracleAmbaCustodyArgs,
    is_deposit: bool,
) -> Result<Value, CliError> {
    if args.amount == 0 {
        return Err(CliError::new("amount must be positive"));
    }
    let mint = validate_pubkey_value("mint", &args.mint)?;
    let user_token_account = args
        .user_token_account
        .as_deref()
        .map(|value| validate_pubkey_value("user-token-account", value))
        .transpose()?;
    let vault_token_account =
        validate_pubkey_value("vault-token-account", &args.vault_token_account)?;
    let mut fields = vec![
        oracle_field("Amount", args.amount.to_string()),
        oracle_field("Mint", mint.as_str()),
        oracle_field("Vault token account", vault_token_account.as_str()),
    ];
    if let Some(user_token_account) = user_token_account.as_deref() {
        fields.insert(2, oracle_field("User token account", user_token_account));
    }
    let action = if is_deposit {
        "Deposit AMBA tokens"
    } else {
        "Withdraw AMBA tokens"
    };
    let next_state = if is_deposit {
        Some("AMBA_DEPOSITED")
    } else {
        Some("AMBA_WITHDRAWN")
    };
    let draft = build_oracle_direct_draft(
        &args.common,
        action,
        "AMBA Token",
        "AMBA",
        "major token custody",
        "AMBA",
        "TOKEN_CONFIGURED",
        next_state,
        "OracleMajorTokenConfig + SPLToken",
        format!(
            "{} amount={} base units; mint={mint}; user token account={}",
            if is_deposit {
                "Deposit AMBA"
            } else {
                "Withdraw AMBA"
            },
            args.amount,
            user_token_account
                .as_deref()
                .unwrap_or("backend-derived signer associated token account")
        ),
        fields,
    );
    save_oracle_draft(args.common.path.as_deref(), draft)
}

fn create_oracle_reward_claim_draft(args: &OracleRewardClaimArgs) -> Result<Value, CliError> {
    let kind = args.kind.as_draft_value();
    let source_id = args
        .source_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let claim_id = args
        .claim_id
        .as_deref()
        .map(|value| normalize_required_32_byte_hex("claim-id", value))
        .transpose()?;
    if matches!(kind, "source_proposer" | "source_support" | "opening") && source_id.is_none() {
        return Err(CliError::new(
            "current source-family rewards require --source-id from the oracle read projection",
        ));
    }
    if kind == "update" && (source_id.is_none() || claim_id.is_none()) {
        return Err(CliError::new(
            "current update rewards require --source-id and --claim-id from the oracle read projection",
        ));
    }
    let mut fields = vec![oracle_field("Reward kind", kind)];
    if let Some(source_id) = source_id {
        fields.push(oracle_field("Source id", source_id));
    }
    if let Some(claim_id) = claim_id.as_deref() {
        fields.push(oracle_field("Claim id", claim_id));
    }
    let subject_label = claim_id.as_deref().or(source_id).unwrap_or(kind);
    let draft = build_oracle_direct_draft(
        &args.common,
        "Claim USDC oracle reward",
        "USDC Oracle Rewards",
        subject_label,
        "current oracle reward subject",
        source_id.unwrap_or(subject_label),
        "REWARD_CLAIMABLE",
        Some("REWARD_CLAIMED"),
        "OracleUsdcRewardVault + OracleUsdcRewardReceipt",
        format!("Claim {kind} USDC oracle reward for {subject_label}"),
        fields,
    );
    save_oracle_draft(args.common.path.as_deref(), draft)
}

fn create_oracle_stake_settlement_draft(args: &OracleStakeSettleArgs) -> Result<Value, CliError> {
    let kind = args.kind.as_draft_value();
    let source_id = args
        .source_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let subject_pda = args
        .subject_pda
        .as_deref()
        .map(|value| validate_pubkey_value("subject-pda", value))
        .transpose()?;
    if kind == "listing_bond" {
        if source_id.is_none() && subject_pda.is_none() {
            return Err(CliError::new(
                "listing-bond settlement requires --source-id or --subject-pda",
            ));
        }
        if source_id.is_some() && subject_pda.is_some() {
            return Err(CliError::new(
                "listing-bond settlement accepts exactly one selector: --source-id or --subject-pda",
            ));
        }
    } else {
        if subject_pda.is_none() {
            return Err(CliError::new(format!(
                "{kind} settlement requires --subject-pda from oracle latest or the TUI"
            )));
        }
        if source_id.is_some() {
            return Err(CliError::new(
                "--source-id is only valid for listing-bond settlement; use the canonical --subject-pda for this kind",
            ));
        }
    }
    let mut fields = vec![oracle_field("Stake kind", kind)];
    if let Some(source_id) = source_id {
        fields.push(oracle_field("Source id", source_id));
    }
    if let Some(subject_pda) = subject_pda.as_deref() {
        fields.push(oracle_field("Subject PDA", subject_pda));
    }
    let selector = subject_pda.as_deref().or(source_id).unwrap_or(kind);
    let draft = build_oracle_direct_draft(
        &args.common,
        "Settle oracle stake",
        "Oracle Stake",
        selector,
        "stake or bond",
        source_id.unwrap_or(selector),
        "STAKE_LOCKED",
        Some("STAKE_SETTLED"),
        "Stake settlement",
        format!(
            "Settle {kind} record {selector}; canonical discovery derives every linked identity and the program derives refund or slash"
        ),
        fields,
    );
    save_oracle_draft(args.common.path.as_deref(), draft)
}

fn create_oracle_opening_print_draft(
    args: &OracleOpeningClaimArgs,
    tree: &oracle_tui::OracleIndexTree,
) -> Result<Value, CliError> {
    ensure_finite("raw-value", args.raw_value)?;
    ensure_positive_finite("stake", args.stake)?;
    ensure_opening_archive_url(&args.archive_url, &args.canonical_locator, &args.timestamp)?;
    let node_index = resolve_oracle_node(tree, &args.node, true)?;
    let node = tree
        .node(node_index)
        .ok_or_else(|| CliError::new("selected oracle node is unavailable"))?;
    let fields = vec![
        oracle_field("Source", node.label.as_str()),
        oracle_field("Raw value", args.raw_value.to_string()),
        oracle_field("Timestamp", args.timestamp.trim()),
        oracle_field("Canonical locator", args.canonical_locator.trim()),
        oracle_field("Source definition", args.source_definition.trim()),
        oracle_field("Wayback archive URL", args.archive_url.trim()),
        oracle_field("Stake", args.stake.to_string()),
    ];
    ensure_nonempty_field(&fields, "Timestamp")?;
    ensure_nonempty_field(&fields, "Canonical locator")?;
    ensure_nonempty_field(&fields, "Source definition")?;
    let draft = build_oracle_draft(
        tree,
        &args.common.market,
        &args.common.month,
        args.common.expiry.as_deref(),
        "Opening print",
        "Opening Print",
        node_index,
        "OPENING_PENDING",
        None,
        "opening claim evidence",
        format!(
            "Opening value={}; Timestamp={}",
            args.raw_value,
            args.timestamp.trim()
        ),
        fields,
    );
    save_oracle_draft(args.common.path.as_deref(), draft)
}

fn create_oracle_opening_claim_challenge_draft(
    args: &OracleOpeningChallengeArgs,
    tree: &oracle_tui::OracleIndexTree,
) -> Result<Value, CliError> {
    ensure_finite("corrected-value", args.corrected_value)?;
    ensure_positive_finite("stake-bond", args.stake_bond)?;
    ensure_opening_archive_url(&args.archive_url, &args.canonical_locator, &args.timestamp)?;
    let node_index = resolve_oracle_node(tree, &args.node, true)?;
    let node = tree
        .node(node_index)
        .ok_or_else(|| CliError::new("selected oracle node is unavailable"))?;
    let fields = vec![
        oracle_field("Target source", node.label.as_str()),
        oracle_field("Challenge reason", args.reason.trim()),
        oracle_field("Corrected value", args.corrected_value.to_string()),
        oracle_field("Timestamp", args.timestamp.trim()),
        oracle_field("Canonical locator", args.canonical_locator.trim()),
        oracle_field("Source definition", args.source_definition.trim()),
        oracle_field("Wayback archive URL", args.archive_url.trim()),
        oracle_field("Stake/bond", args.stake_bond.to_string()),
    ];
    for label in [
        "Challenge reason",
        "Timestamp",
        "Canonical locator",
        "Source definition",
    ] {
        ensure_nonempty_field(&fields, label)?;
    }
    let draft = build_oracle_draft(
        tree,
        &args.common.market,
        &args.common.month,
        args.common.expiry.as_deref(),
        "Challenge",
        "Opening Print",
        node_index,
        "CHALLENGED",
        Some("CHALLENGED"),
        "opening claim challenge",
        format!("Opening challenge={}", args.reason.trim()),
        fields,
    );
    save_oracle_draft(args.common.path.as_deref(), draft)
}

fn create_oracle_source_challenge_draft(
    args: &OracleChallengeDraftArgs,
    tree: &oracle_tui::OracleIndexTree,
) -> Result<Value, CliError> {
    create_oracle_challenge_draft(args, tree, "Kill Challenge")
}

fn create_oracle_update_challenge_draft(
    args: &OracleUpdateChallengeArgs,
    tree: &oracle_tui::OracleIndexTree,
) -> Result<Value, CliError> {
    ensure_public_url("archive-url", &args.archive_url)?;
    let node_index = resolve_oracle_node(tree, &args.node, true)?;
    let node = tree
        .node(node_index)
        .ok_or_else(|| CliError::new("selected oracle node is unavailable"))?;
    let claimant = validate_nonzero_pubkey_value("claimant", &args.claimant)?;
    let claim_id = normalize_nonzero_update_identifier(
        "claim-id",
        "claim",
        &args.claim_id,
        node.label.as_str(),
    )?;
    let fields = vec![
        oracle_field("Target source", node.label.as_str()),
        oracle_field("Claim id", claim_id.as_str()),
        oracle_field("Claimant", claimant.as_str()),
        oracle_field("Challenge reason", args.reason.trim()),
        oracle_field("Corrected value", args.corrected_value.to_string()),
        oracle_field("Evidence / Archive Link", args.archive_url.trim()),
        oracle_field("Wayback archive URL", args.archive_url.trim()),
        oracle_field("Stake/bond", args.stake_bond.to_string()),
    ];
    ensure_nonempty_field(&fields, "Challenge reason")?;
    let draft = build_oracle_draft(
        tree,
        &args.common.market,
        &args.common.month,
        args.common.expiry.as_deref(),
        "Challenge",
        "Game Mode",
        node_index,
        "CHALLENGED",
        Some("CHALLENGED"),
        "ChallengeManager + EmergencyGovernor",
        format!(
            "Challenge claimant-scoped update claim={claim_id}; reason={}",
            args.reason.trim()
        ),
        fields,
    );
    save_oracle_draft(args.common.path.as_deref(), draft)
}

fn create_oracle_update_commit_draft(args: &OracleUpdateCommitArgs) -> Result<Value, CliError> {
    ensure_positive_finite("stake", args.stake)?;
    let source_id = required_trimmed_value("source-id", &args.source_id)?;
    let claim_id = normalize_update_identifier("claim-id", "claim", &args.claim_id, source_id)?;
    let commit_hash = normalize_required_32_byte_hex("commit-hash", &args.commit_hash)?;
    let fields = vec![
        oracle_field("Source id", source_id),
        oracle_field("Claim id", &claim_id),
        oracle_field("Commit hash", commit_hash),
        oracle_field("Stake", args.stake.to_string()),
    ];
    let draft = build_oracle_direct_draft(
        &args.common,
        "Commit update claim v2",
        "Game Mode",
        source_id,
        "oracle source",
        source_id,
        "ACTIVE",
        Some("COMMITTED"),
        "OracleCommitReveal + OracleUsdcRewards",
        format!(
            "Claimant-scoped update claim={claim_id}; Stake={}",
            args.stake
        ),
        fields,
    );
    save_oracle_draft(args.common.path.as_deref(), draft)
}

fn create_oracle_update_reveal_draft(args: &OracleUpdateRevealArgs) -> Result<Value, CliError> {
    ensure_finite("raw-value", args.raw_value)?;
    ensure_public_url("archive-url", &args.archive_url)?;
    let source_id = required_trimmed_value("source-id", &args.source_id)?;
    let claim_id = normalize_update_identifier("claim-id", "claim", &args.claim_id, source_id)?;
    normalize_required_32_byte_hex("secret-salt", &args.secret_salt)?;
    let mut fields = vec![
        oracle_field("Source id", source_id),
        oracle_field("Claim id", &claim_id),
        oracle_field("Prior state", args.prior_state.to_string()),
        oracle_field("Raw value", args.raw_value.to_string()),
        oracle_field("Timestamp", args.timestamp.trim()),
        oracle_field("Wayback archive URL", args.archive_url.trim()),
        oracle_secret_field("Secret salt"),
    ];
    ensure_nonempty_field(&fields, "Timestamp")?;
    if let Some(evidence_hash) = args
        .evidence_hash
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let evidence_hash = normalize_required_32_byte_hex("evidence-hash", evidence_hash)?;
        fields.push(oracle_field("Evidence hash", evidence_hash));
    }
    let draft = build_oracle_direct_draft(
        &args.common,
        "Reveal update claim v2",
        "Game Mode",
        source_id,
        "oracle source",
        source_id,
        "ACTIVE",
        Some("REVEALED"),
        "OracleCommitReveal + OracleUsdcRewards",
        format!(
            "Claimant-scoped update claim={claim_id}; Timestamp={}",
            args.timestamp.trim()
        ),
        fields,
    );
    save_oracle_draft(args.common.path.as_deref(), draft)
}

fn create_oracle_update_expiry_draft(args: &OracleUpdateExpireArgs) -> Result<Value, CliError> {
    let source_id = required_trimmed_value("source-id", &args.source_id)?;
    let claim_id = normalize_update_identifier("claim-id", "claim", &args.claim_id, source_id)?;
    let claimant = required_trimmed_value("claimant", &args.claimant)?;
    Pubkey::from_str(claimant).map_err(|error| {
        CliError::new(format!("claimant must be a valid Solana address: {error}"))
    })?;
    let draft = build_oracle_direct_draft(
        &args.common,
        "Settle expired update commitment v2",
        "Game Mode",
        source_id,
        "oracle source",
        source_id,
        "ACTIVE",
        Some("EXPIRED"),
        "OracleCommitReveal",
        format!("Settle expired unrevealed claim={claim_id}"),
        vec![
            oracle_field("Source id", source_id),
            oracle_field("Claim id", claim_id),
            oracle_field("Claimant", claimant),
        ],
    );
    save_oracle_draft(args.common.path.as_deref(), draft)
}

fn create_oracle_challenge_draft(
    args: &OracleChallengeDraftArgs,
    tree: &oracle_tui::OracleIndexTree,
    phase: &str,
) -> Result<Value, CliError> {
    ensure_positive_finite("stake-bond", args.stake_bond)?;
    ensure_public_url("archive-url", &args.archive_url)?;
    if let Some(value) = args.corrected_value {
        ensure_finite("corrected-value", value)?;
    }
    let node_index = resolve_oracle_node(tree, &args.node, true)?;
    let node = tree
        .node(node_index)
        .ok_or_else(|| CliError::new("selected oracle node is unavailable"))?;
    let mut fields = vec![
        oracle_field("Target source", node.label.as_str()),
        oracle_field("Challenge reason", args.reason.trim()),
    ];
    if let Some(value) = args
        .comparison_source_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        fields.push(oracle_field("Comparison source id", value));
    }
    if let Some(value) = args.corrected_value {
        fields.push(oracle_field("Corrected value", value.to_string()));
    }
    fields.extend([
        oracle_field("Evidence / Archive Link", args.archive_url.trim()),
        oracle_field("Wayback archive URL", args.archive_url.trim()),
        oracle_field("Stake/bond", args.stake_bond.to_string()),
    ]);
    ensure_nonempty_field(&fields, "Challenge reason")?;
    let draft = build_oracle_draft(
        tree,
        &args.common.market,
        &args.common.month,
        args.common.expiry.as_deref(),
        "Challenge",
        phase,
        node_index,
        "CHALLENGED",
        Some("CHALLENGED"),
        "ChallengeManager + EmergencyGovernor",
        format!("Challenge reason={}", args.reason.trim()),
        fields,
    );
    save_oracle_draft(args.common.path.as_deref(), draft)
}

fn build_oracle_draft(
    tree: &oracle_tui::OracleIndexTree,
    market_id: &str,
    month_label: &str,
    expiry_id: Option<&str>,
    action: &str,
    phase: &str,
    node_index: usize,
    source_state: &str,
    update_state: Option<&str>,
    modules: &str,
    summary: String,
    fields: Vec<OracleSubmissionField>,
) -> OracleSubmissionDraft {
    let node = tree
        .node(node_index)
        .expect("resolved oracle node should exist");
    OracleSubmissionDraft {
        id: String::new(),
        created_at_unix_seconds: 0,
        market_id: market_id.trim().to_ascii_lowercase(),
        month_label: month_label.trim().to_string(),
        expiry_id: expiry_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_uppercase),
        oracle_month: None,
        action: action.to_string(),
        phase: phase.to_string(),
        breadcrumb: tree
            .breadcrumb(node_index)
            .into_iter()
            .map(str::to_string)
            .collect(),
        node_label: node.label.clone(),
        node_kind: node.kind.label().to_string(),
        row_label: oracle_row_label(tree, node_index),
        source_state: source_state.to_string(),
        update_state: update_state.map(str::to_string),
        modules: modules.to_string(),
        summary,
        fields,
        backend_status: String::new(),
    }
}

#[allow(clippy::too_many_arguments)]
fn build_oracle_direct_draft(
    common: &OracleDraftCommonArgs,
    action: &str,
    phase: &str,
    node_label: &str,
    node_kind: &str,
    row_label: &str,
    source_state: &str,
    update_state: Option<&str>,
    modules: &str,
    summary: String,
    fields: Vec<OracleSubmissionField>,
) -> OracleSubmissionDraft {
    let market_id = common.market.trim().to_ascii_lowercase();
    let month_label = common.month.trim().to_string();
    OracleSubmissionDraft {
        id: String::new(),
        created_at_unix_seconds: 0,
        market_id: market_id.clone(),
        month_label: month_label.clone(),
        expiry_id: common
            .expiry
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_uppercase),
        oracle_month: None,
        action: action.to_string(),
        phase: phase.to_string(),
        breadcrumb: vec![
            market_id.to_ascii_uppercase(),
            month_label,
            node_label.to_string(),
        ],
        node_label: node_label.to_string(),
        node_kind: node_kind.to_string(),
        row_label: row_label.to_string(),
        source_state: source_state.to_string(),
        update_state: update_state.map(str::to_string),
        modules: modules.to_string(),
        summary,
        fields,
        backend_status: String::new(),
    }
}

fn validate_oracle_draft_semantics(draft: &OracleSubmissionDraft) -> Result<(), CliError> {
    for (label, value) in [
        ("market id", draft.market_id.as_str()),
        ("month label", draft.month_label.as_str()),
        ("action", draft.action.as_str()),
        ("phase", draft.phase.as_str()),
        ("node label", draft.node_label.as_str()),
        ("row label", draft.row_label.as_str()),
        ("source state", draft.source_state.as_str()),
        ("summary", draft.summary.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(CliError::new(format!(
                "oracle semantic draft is missing {label}"
            )));
        }
    }
    if draft.breadcrumb.is_empty()
        || draft.breadcrumb.len() > 64
        || draft.breadcrumb.iter().any(|part| part.trim().is_empty())
    {
        return Err(CliError::new(
            "oracle semantic draft breadcrumb must contain 1..=64 nonempty labels",
        ));
    }
    if draft.fields.len() > 64 {
        return Err(CliError::new(
            "oracle semantic draft cannot contain more than 64 fields",
        ));
    }
    let mut labels = BTreeSet::new();
    for field in &draft.fields {
        let label = field.label.trim();
        if label.is_empty() || !labels.insert(label) {
            return Err(CliError::new(
                "oracle semantic draft field labels must be nonempty and unique",
            ));
        }
    }
    Ok(())
}

fn save_oracle_draft(path: Option<&str>, draft: OracleSubmissionDraft) -> Result<Value, CliError> {
    let store_path = resolve_oracle_draft_path(path)?;
    let (saved, ()) = oracle_submissions::append_validated_at_path(
        &store_path,
        draft,
        validate_oracle_draft_semantics,
    )?;
    Ok(json!({
        "ok": true,
        "action": "oracle_draft_create",
        "path": store_path.display().to_string(),
        "draft": saved,
        "transactionPreparation": {
            "status": "not_wired",
            "authority": "ameba-sdk/operator",
        },
    }))
}

fn validate_oracle_drafts(path: &PathBuf, selector: &str) -> Result<Value, CliError> {
    let drafts = oracle_submissions::load_all_at_path(path)?;
    let selected = select_oracle_drafts(&drafts, selector)?;
    let results = selected
        .iter()
        .map(|draft| oracle_draft_validation_result(draft, false))
        .collect::<Vec<_>>();
    let failed_count = results
        .iter()
        .filter(|result| result.get("ok").and_then(Value::as_bool) == Some(false))
        .count();
    Ok(json!({
        "ok": failed_count == 0,
        "action": "oracle_drafts_validate",
        "path": path.display().to_string(),
        "selector": selector,
        "selectedCount": results.len(),
        "failedCount": failed_count,
        "results": results,
    }))
}

fn preview_oracle_drafts(path: &PathBuf, selector: &str) -> Result<Value, CliError> {
    let drafts = oracle_submissions::load_all_at_path(path)?;
    let selected = select_oracle_drafts(&drafts, selector)?;
    let results = selected
        .iter()
        .map(|draft| oracle_draft_validation_result(draft, true))
        .collect::<Vec<_>>();
    let failed_count = results
        .iter()
        .filter(|result| result.get("ok").and_then(Value::as_bool) == Some(false))
        .count();
    Ok(json!({
        "ok": failed_count == 0,
        "action": "oracle_drafts_preview",
        "path": path.display().to_string(),
        "selector": selector,
        "selectedCount": results.len(),
        "failedCount": failed_count,
        "results": results,
    }))
}

fn oracle_draft_validation_result(draft: &OracleSubmissionDraft, include_dispatch: bool) -> Value {
    let mut result = match validate_oracle_draft_semantics(draft) {
        Ok(()) => {
            json!({
                "ok": true,
                "id": draft.id,
                "action": draft.action,
                "phase": draft.phase,
                "nodeLabel": draft.node_label,
                "rowLabel": draft.row_label,
                "transactionPreparation": "not_wired",
            })
        }
        Err(error) => json!({
            "ok": false,
            "id": draft.id,
            "action": draft.action,
            "phase": draft.phase,
            "nodeLabel": draft.node_label,
            "rowLabel": draft.row_label,
            "issue": error.to_string(),
        }),
    };
    if include_dispatch {
        result["review"] = oracle_draft_review(draft);
        result["dependency"] = json!({
            "authority": "ameba-sdk/operator",
            "required": "frozen current oracle plan and managed-signing receipt",
        });
    }
    result
}

fn oracle_draft_review(draft: &OracleSubmissionDraft) -> Value {
    let fields = draft
        .fields
        .iter()
        .map(|field| {
            let value = if oracle_field_is_secret(&field.label) {
                "[secret omitted; re-enter at transaction preparation]"
            } else {
                field.value.as_str()
            };
            json!({
                "label": field.label,
                "value": value,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "createdAtUnixSeconds": draft.created_at_unix_seconds,
        "marketId": draft.market_id,
        "monthLabel": draft.month_label,
        "expiryId": draft.expiry_id,
        "oracleMonth": draft.oracle_month,
        "breadcrumb": draft.breadcrumb,
        "nodeLabel": draft.node_label,
        "nodeKind": draft.node_kind,
        "rowLabel": draft.row_label,
        "sourceState": draft.source_state,
        "updateState": draft.update_state,
        "modules": draft.modules,
        "summary": draft.summary,
        "fields": fields,
        "backendStatus": draft.backend_status,
    })
}

fn oracle_field_is_secret(label: &str) -> bool {
    let label = label.to_ascii_lowercase();
    label.contains("salt") || label.contains("secret")
}

fn select_oracle_drafts(
    drafts: &[oracle_submissions::OracleSubmissionDraft],
    selector: &str,
) -> Result<Vec<oracle_submissions::OracleSubmissionDraft>, CliError> {
    let selector = selector.trim();
    if drafts.is_empty() {
        return Err(CliError::new("no oracle drafts are queued locally"));
    }
    if selector.eq_ignore_ascii_case("all") {
        let mut selected = drafts.to_vec();
        selected.sort_by_key(|draft| draft.created_at_unix_seconds);
        return Ok(selected);
    }
    if selector.is_empty() || selector.eq_ignore_ascii_case("latest") {
        let latest = drafts
            .iter()
            .max_by_key(|draft| draft.created_at_unix_seconds)
            .expect("drafts is not empty");
        return Ok(vec![latest.clone()]);
    }
    drafts
        .iter()
        .find(|draft| draft.id == selector)
        .cloned()
        .map(|draft| vec![draft])
        .ok_or_else(|| CliError::new(format!("oracle draft {selector} was not found")))
}

fn resolve_oracle_node(
    tree: &oracle_tui::OracleIndexTree,
    selector: &str,
    require_terminal: bool,
) -> Result<usize, CliError> {
    let index = tree.find_node_index(selector).ok_or_else(|| {
        CliError::new(format!(
            "{} oracle node {selector:?} was not found; run `petri oracle recipe --query {selector}`",
            tree.symbol
        ))
    })?;
    let node = tree
        .node(index)
        .ok_or_else(|| CliError::new("selected oracle node is unavailable"))?;
    if require_terminal && node.kind != oracle_tui::OracleNodeKind::TerminalPin {
        return Err(CliError::new(format!(
            "{} is a {}; choose a terminal source pin for this draft",
            node.label,
            node.kind.label()
        )));
    }
    Ok(index)
}

fn oracle_row_label(tree: &oracle_tui::OracleIndexTree, node_index: usize) -> String {
    tree.row_bucket_for(node_index)
        .and_then(|index| tree.node(index).map(|node| node.label.clone()))
        .unwrap_or_else(|| tree.display_name.clone())
}

fn oracle_field(label: impl Into<String>, value: impl Into<String>) -> OracleSubmissionField {
    OracleSubmissionField {
        label: label.into(),
        value: value.into(),
    }
}

fn oracle_secret_field(label: impl Into<String>) -> OracleSubmissionField {
    OracleSubmissionField {
        label: label.into(),
        value: "[secret omitted; re-enter at transaction preparation]".to_string(),
    }
}

fn ensure_nonempty_field(fields: &[OracleSubmissionField], label: &str) -> Result<(), CliError> {
    let value = fields
        .iter()
        .find(|field| field.label == label)
        .map(|field| field.value.trim())
        .unwrap_or("");
    if value.is_empty() {
        return Err(CliError::new(format!("{label} is required")));
    }
    Ok(())
}

fn ensure_finite(label: &str, value: f64) -> Result<(), CliError> {
    if !value.is_finite() {
        return Err(CliError::new(format!("{label} must be finite")));
    }
    Ok(())
}

fn ensure_positive_finite(label: &str, value: f64) -> Result<(), CliError> {
    ensure_finite(label, value)?;
    if value <= 0.0 {
        return Err(CliError::new(format!("{label} must be positive")));
    }
    Ok(())
}

fn ensure_public_url(label: &str, value: &str) -> Result<(), CliError> {
    let value = value.trim();
    if !(value.starts_with("https://") || value.starts_with("http://")) {
        return Err(CliError::new(format!("{label} must be a public URL")));
    }
    Ok(())
}

fn ensure_opening_archive_url(
    archive_url: &str,
    canonical_locator: &str,
    source_time: &str,
) -> Result<(), CliError> {
    spread_oracle_plan::validate_opening_archive_url(archive_url, canonical_locator, source_time)
}

fn required_trimmed_value<'a>(label: &str, value: &'a str) -> Result<&'a str, CliError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err(CliError::new(format!("{label} is required")))
    } else {
        Ok(trimmed)
    }
}

fn normalize_emergency_choice(
    label: &str,
    kind: Option<&str>,
    value: &str,
) -> Result<String, CliError> {
    let trimmed = value.trim();
    let normalized = trimmed.to_ascii_lowercase().replace(['-', ' '], "_");
    let choice = match normalized.as_str() {
        "0" | "keep" | "keep_source" | "reject_challenge" | "accept_original" | "original"
        | "accept" => 0,
        "1" | "kill" | "kill_source" | "accept_challenge" | "correct" | "corrected"
        | "accept_corrected" => 1,
        "2"
        | "keep_prior_state"
        | "prior_state"
        | "source_inactive_for_month"
        | "inactive"
        | "reject_both" => 2,
        _ => {
            return Err(CliError::new(format!(
                "{label} must be a supported emergency choice name or numeric choice"
            )));
        }
    };
    if let Some(kind) = kind {
        let max = match kind {
            "source" => 1,
            "update" | "opening" => 2,
            _ => 2,
        };
        if choice > max {
            return Err(CliError::new(format!(
                "{label} choice {choice} is invalid for {kind} emergencies"
            )));
        }
    }
    Ok(choice.to_string())
}

fn stable_32_byte_hash(parts: &[&str]) -> String {
    let byte_parts = parts.iter().map(|part| part.as_bytes()).collect::<Vec<_>>();
    format!("0x{}", lower_hex(&hashv(&byte_parts).to_bytes()))
}

fn lower_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use core::fmt::Write as _;
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

fn normalize_required_32_byte_hex(label: &str, value: &str) -> Result<String, CliError> {
    let normalized = value
        .trim()
        .strip_prefix("0x")
        .or_else(|| value.trim().strip_prefix("0X"))
        .or_else(|| value.trim().strip_prefix("hex:"))
        .or_else(|| value.trim().strip_prefix("HEX:"))
        .unwrap_or_else(|| value.trim());
    if normalized.len() == 64 && normalized.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(format!("0x{}", normalized.to_ascii_lowercase()))
    } else {
        Err(CliError::new(format!(
            "{label} must be a 32-byte hex string"
        )))
    }
}

fn normalize_update_identifier(
    label: &str,
    domain: &str,
    value: &str,
    source_id: &str,
) -> Result<String, CliError> {
    let value = required_trimmed_value(label, value)?;
    Ok(normalize_required_32_byte_hex(label, value)
        .unwrap_or_else(|_| stable_32_byte_hash(&[domain, value, source_id])))
}

fn normalize_nonzero_update_identifier(
    label: &str,
    domain: &str,
    value: &str,
    source_id: &str,
) -> Result<String, CliError> {
    let normalized = normalize_update_identifier(label, domain, value, source_id)?;
    if normalized
        .trim_start_matches("0x")
        .bytes()
        .all(|byte| byte == b'0')
    {
        return Err(CliError::new(format!("{label} must be nonzero")));
    }
    Ok(normalized)
}

fn validate_pubkey_value(label: &str, value: &str) -> Result<String, CliError> {
    let trimmed = required_trimmed_value(label, value)?;
    Pubkey::from_str(trimmed).map_err(|error| {
        CliError::new(format!("{label} must be a valid Solana pubkey: {error}"))
    })?;
    Ok(trimmed.to_string())
}

fn validate_nonzero_pubkey_value(label: &str, value: &str) -> Result<String, CliError> {
    let normalized = validate_pubkey_value(label, value)?;
    if Pubkey::from_str(&normalized).expect("validated pubkey") == Pubkey::default() {
        return Err(CliError::new(format!("{label} must be nonzero")));
    }
    Ok(normalized)
}

fn ensure_allowed_source_category(value: &str) -> Result<(), CliError> {
    if is_allowed_v1_source_category(value) {
        Ok(())
    } else {
        Err(CliError::new(
            "source-category must be public retailer, distributor, manufacturer/store, benchmark/assessment, or public API",
        ))
    }
}

fn is_allowed_v1_source_category(category: &str) -> bool {
    let normalized = category.to_ascii_lowercase();
    [
        "retailer",
        "distributor",
        "manufacturer",
        "store",
        "benchmark",
        "assessment",
        "public api",
        "api",
    ]
    .iter()
    .any(|allowed| normalized.contains(allowed))
}

fn render_oracle_recipe(payload: &Value) -> String {
    let source_tip_rows =
        string_at_key(payload, &["sourceTipRows"]).unwrap_or_else(|| "-".to_string());
    let terminal_pins =
        string_at_key(payload, &["terminalPinCount"]).unwrap_or_else(|| "-".to_string());
    let query = string_at_key(payload, &["query"]).unwrap_or_default();
    let nodes = array_at_key(payload, &["nodes"])
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let mut lines = vec![
        "RAMX-MOD oracle recipe".to_string(),
        format!("rows={source_tip_rows} | terminalPins={terminal_pins} | query={query}"),
    ];
    for node in nodes {
        lines.push(render_oracle_recipe_node(node));
    }
    lines.push("Draft with: petri oracle sources propose <sku> ...".to_string());
    render_terminal_lines(lines)
}

fn render_oracle_recipe_node(node: &Value) -> String {
    let index = string_at_key(node, &["index"]).unwrap_or_else(|| "-".to_string());
    let label = string_at_key(node, &["label"]).unwrap_or_else(|| "-".to_string());
    let kind = string_at_key(node, &["kind"]).unwrap_or_else(|| "-".to_string());
    let weight = string_at_key(node, &["weightPct"]).unwrap_or_else(|| "0".to_string());
    let row_weight = string_at_key(node, &["rowWeightPct"]).unwrap_or_else(|| "0".to_string());
    let pin_count = string_at_key(node, &["pinCount"]).unwrap_or_else(|| "0".to_string());
    format!(
        "{index} | {label} | {kind} | weight={weight}% | rowWeight={row_weight}% | pins={pin_count}"
    )
}

fn oracle_payload<'a>(payload: &'a Value) -> &'a Value {
    value_at_path(payload, &["oracle"])
        .or_else(|| value_at_path(payload, &["data", "oracle"]))
        .unwrap_or(payload)
}

fn render_spread_oracle_state(payload: &Value) -> String {
    let oracle = oracle_payload(payload);
    let totals = value_at_path(oracle, &["totals"])
        .or_else(|| value_at_path(payload, &["totals"]))
        .unwrap_or(&Value::Null);
    let markets = array_at_key(oracle, &["markets"])
        .or_else(|| array_at_key(payload, &["markets"]))
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let source = string_at_key(oracle, &["source"])
        .or_else(|| string_at_key(payload, &["source"]))
        .unwrap_or_else(|| "ameba_spread_onchain".to_string());
    let market_count =
        string_at_key(totals, &["markets"]).unwrap_or_else(|| markets.len().to_string());
    let month_count = string_at_key(totals, &["months"]).unwrap_or_else(|| "-".to_string());
    let source_count = string_at_key(totals, &["sources"]).unwrap_or_else(|| "-".to_string());
    let settlement_count =
        string_at_key(totals, &["settlements"]).unwrap_or_else(|| "-".to_string());
    let mut lines = vec![
        "Spread oracle state".to_string(),
        format!(
            "source={source} | markets={market_count} | months={month_count} | sources={source_count} | settlements={settlement_count}"
        ),
    ];

    if let Some(market) = value_at_path(oracle, &["market"]) {
        lines.push(render_spread_oracle_market_line(market));
    } else if markets.is_empty() {
        lines.push("No spread oracle markets were discovered.".to_string());
    } else {
        lines.extend(
            markets
                .iter()
                .take(12)
                .map(render_spread_oracle_market_line),
        );
    }

    render_terminal_lines(lines)
}

fn render_spread_oracle_market_line(market: &Value) -> String {
    let market_id =
        string_at_key(market, &["marketId", "market_id"]).unwrap_or_else(|| "-".to_string());
    let symbol = string_at_key(market, &["marketSymbol", "market_symbol"])
        .unwrap_or_else(|| market_id.clone());
    let months =
        string_at_key(market, &["monthCount", "month_count"]).unwrap_or_else(|| "0".to_string());
    let sources =
        string_at_key(market, &["sourceCount", "source_count"]).unwrap_or_else(|| "0".to_string());
    let game = string_at_key(market, &["gameMonthCount", "game_month_count"])
        .unwrap_or_else(|| "0".to_string());
    let blocked = string_at_key(market, &["blockedMonthCount", "blocked_month_count"])
        .unwrap_or_else(|| "0".to_string());
    let latest = value_at_path(market, &["latestSettlement"])
        .and_then(|settlement| {
            string_at_key(settlement, &["expiryId", "expiry_id"])
                .or_else(|| string_at_key(settlement, &["settlementId", "settlement_id"]))
        })
        .unwrap_or_else(|| "none".to_string());
    format!(
        "{market_id} ({symbol}) | months={months} | sources={sources} | game={game} | blocked={blocked} | latest={latest}"
    )
}

fn render_spread_oracle_latest(payload: &Value) -> String {
    let oracle = oracle_payload(payload);
    let market =
        string_at_key(oracle, &["marketId", "market_id"]).unwrap_or_else(|| "all".to_string());
    let month = value_at_path(oracle, &["oracleMonth"]).unwrap_or(&Value::Null);
    let month_id = string_at_key(month, &["oracleMonth", "oracle_month"])
        .unwrap_or_else(|| "none".to_string());
    let phase = string_at_key(month, &["phase"]).unwrap_or_else(|| "-".to_string());
    let expiry =
        string_at_key(month, &["expiryId", "expiry_id"]).unwrap_or_else(|| "-".to_string());
    let latest = value_at_path(oracle, &["latest"]).unwrap_or(&Value::Null);
    let latest_label = string_at_key(latest, &["settlementId", "settlement_id"])
        .or_else(|| string_at_key(latest, &["expiryId", "expiry_id"]))
        .unwrap_or_else(|| "none".to_string());
    let issues = array_at_key(oracle, &["issues"])
        .map(Vec::len)
        .unwrap_or_default();
    let escrows = array_at_key(month, &["escrows"])
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let weight_scheme = string_at_key(month, &["weightScheme", "weight_scheme"])
        .unwrap_or_else(|| "unknown".to_string());
    let weight_version = string_at_key(
        month,
        &[
            "weightSchemeVersion",
            "weight_scheme_version",
            "weightVersion",
            "weight_version",
        ],
    )
    .filter(|version| matches!(version.as_str(), "1" | "255"))
    .unwrap_or_else(|| "unknown".to_string());
    let weight_verified = value_at_key(month, &["weightVerified", "weight_verified"])
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let effective_weight_total = string_at_key(
        month,
        &["effectiveWeightTotalBps", "effective_weight_total_bps"],
    )
    .filter(|_| weight_verified)
    .unwrap_or_else(|| "unverified".to_string());
    let weight_manifest = string_at_key(
        month,
        &["weightManifestHashHex", "weight_manifest_hash_hex"],
    )
    .unwrap_or_else(|| "none".to_string());
    let active_weight_version = string_at_key(
        month,
        &[
            "activeWeightSchemeVersion",
            "active_weight_scheme_version",
            "activeWeightVersion",
            "active_weight_version",
        ],
    )
    .filter(|version| matches!(version.as_str(), "1" | "255"))
    .unwrap_or_else(|| "unknown".to_string());
    let active_weight_verified =
        value_at_key(month, &["activeWeightVerified", "active_weight_verified"])
            .and_then(Value::as_bool)
            .unwrap_or(false);
    let active_weight_manifest = string_at_key(
        month,
        &[
            "activeWeightManifestHashHex",
            "active_weight_manifest_hash_hex",
        ],
    )
    .unwrap_or_else(|| "none".to_string());
    let ready = escrows
        .iter()
        .filter(|escrow| {
            value_at_key(escrow, &["settlementEligible", "settlement_eligible"])
                .and_then(Value::as_bool)
                == Some(true)
        })
        .count();
    let mut lines = vec![
        "Spread oracle latest".to_string(),
        format!(
            "market={market} | oracleMonth={month_id} | expiry={expiry} | phase={phase} | latestSettlement={latest_label} | issues={issues}"
        ),
        format!(
            "weightScheme={weight_scheme} | version={weight_version} | verified={weight_verified} | effectiveWeightTotalBps={effective_weight_total} | manifest={weight_manifest}"
        ),
        format!(
            "activeWeightVersion={active_weight_version} | activeWeightVerified={active_weight_verified} | activeWeightManifest={active_weight_manifest}"
        ),
    ];
    if !weight_verified {
        lines.push(
            "weightStatus=NOT_VERIFIED; current manifest weights are not verified effective global weights"
                .to_string(),
        );
    }
    if !active_weight_verified {
        lines.push(
            "activeWeightStatus=NOT_FINALIZED; updates, DLMM eligibility, settlement, and closeout remain blocked"
                .to_string(),
        );
    }
    if let Some(sources) = array_at_key(month, &["sources"]) {
        lines.extend(sources.iter().take(8).map(|source| {
            let source_id = string_at_key(source, &["sourceIdHex", "source_id_hex"])
                .or_else(|| string_at_key(source, &["source"]))
                .unwrap_or_else(|| "-".to_string());
            let bucket = string_at_key(source, &["bucketWeightBps", "bucket_weight_bps"])
                .unwrap_or_else(|| "unknown".to_string());
            let raw = (weight_version == "1")
                .then(|| string_at_key(source, &["frozenWeightBps", "frozen_weight_bps"]))
                .flatten()
                .unwrap_or_else(|| "unknown".to_string());
            let source_verified = value_at_key(source, &["weightVerified", "weight_verified"])
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let effective = string_at_key(source, &["effectiveWeightBps", "effective_weight_bps"])
                .filter(|_| source_verified)
                .unwrap_or_else(|| "unknown".to_string());
            let source_active_verified = value_at_key(
                source,
                &["activeWeightVerified", "active_weight_verified"],
            )
            .and_then(Value::as_bool)
            .unwrap_or(false);
            let active = string_at_key(source, &["activeWeightBps", "active_weight_bps"])
                .filter(|_| source_active_verified)
                .unwrap_or_else(|| "NOT_FINALIZED".to_string());
            let active_effective = string_at_key(
                source,
                &[
                    "activeEffectiveWeightBps",
                    "active_effective_weight_bps",
                ],
            )
            .filter(|_| source_active_verified)
            .unwrap_or_else(|| "NOT_FINALIZED".to_string());
            format!(
                "source={source_id} | bucketWeightBps={bucket} | frozenWeightBps={raw} | frozenEffectiveWeightBps={effective} | frozenVerified={source_verified} | activeWeightBps={active} | activeEffectiveWeightBps={active_effective} | activeVerified={source_active_verified}"
            )
        }));
    }
    if !escrows.is_empty() {
        lines.push(format!(
            "stakeRecords={} | readyToSettle={ready}",
            escrows.len()
        ));
        lines.extend(escrows.iter().take(8).map(|escrow| {
            let kind = string_at_key(escrow, &["kind"])
                .unwrap_or_else(|| "stake".to_string())
                .replace('_', " ");
            let amount =
                string_at_key(escrow, &["amount"]).unwrap_or_else(|| "-".to_string());
            let disposition = string_at_key(escrow, &["disposition"])
                .unwrap_or_else(|| "unsettled".to_string());
            let status = string_at_key(escrow, &["status"])
                .unwrap_or_else(|| "pending".to_string());
            let terminal_outcome = string_at_key(
                escrow,
                &["terminalOutcome", "terminal_outcome"],
            )
            .unwrap_or_else(|| "-".to_string());
            let eligibility = if value_at_key(
                escrow,
                &["settlementEligible", "settlement_eligible"],
            )
            .and_then(Value::as_bool)
                == Some(true)
            {
                "ready"
            } else {
                "waiting"
            };
            format!(
                "stake={kind} | amount={amount} | status={status} | outcome={terminal_outcome} | disposition={disposition} | settlement={eligibility}"
            )
        }));
    }
    render_terminal_lines(lines)
}

fn render_spread_oracle_history(payload: &Value) -> String {
    let oracle = oracle_payload(payload);
    let market =
        string_at_key(oracle, &["marketId", "market_id"]).unwrap_or_else(|| "all".to_string());
    let history = array_at_key(oracle, &["history"])
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let months = array_at_key(oracle, &["oracleMonths", "oracle_months"])
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let mut lines = vec![
        "Spread oracle history".to_string(),
        format!(
            "market={market} | settlements={} | oracleMonths={}",
            history.len(),
            months.len()
        ),
    ];
    if history.is_empty() {
        lines.push("No spread oracle settlements were discovered.".to_string());
    } else {
        lines.extend(
            history
                .iter()
                .take(10)
                .map(render_spread_oracle_settlement_line),
        );
    }
    render_terminal_lines(lines)
}

fn render_spread_oracle_settlement_line(settlement: &Value) -> String {
    let id = string_at_key(settlement, &["settlementId", "settlement_id"])
        .unwrap_or_else(|| "-".to_string());
    let market =
        string_at_key(settlement, &["marketId", "market_id"]).unwrap_or_else(|| "-".to_string());
    let expiry =
        string_at_key(settlement, &["expiryId", "expiry_id"]).unwrap_or_else(|| "-".to_string());
    let price = string_at_key(
        settlement,
        &["settlementPriceDisplay", "settlement_price_display"],
    )
    .or_else(|| {
        string_at_key(
            settlement,
            &["settlementPriceAtomic", "settlement_price_atomic"],
        )
    })
    .unwrap_or_else(|| "-".to_string());
    let settled_at = string_at_key(settlement, &["settlementUtc", "settlement_utc"])
        .unwrap_or_else(|| "-".to_string());
    format!("{id} | market={market} | expiry={expiry} | price={price} | settledAt={settled_at}")
}

fn render_oracle_drafts_list(payload: &Value) -> String {
    let path = string_at_key(payload, &["path"]).unwrap_or_else(|| "-".to_string());
    let drafts = array_at_key(payload, &["drafts"])
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if drafts.is_empty() {
        return [
            "Oracle drafts".to_string(),
            format!("store={path}"),
            "No local RAMX-MOD oracle drafts are queued.".to_string(),
        ]
        .join("\n");
    }

    let mut lines = vec![
        "Oracle drafts".to_string(),
        format!("store={path} | count={}", drafts.len()),
    ];
    lines.extend(drafts.iter().map(render_oracle_draft_line));
    lines.push("Validate with: petri oracle drafts validate latest".to_string());
    lines.push("Show with: petri oracle drafts show latest".to_string());
    lines.push("Transaction preparation: not_wired (SDK operator required)".to_string());
    lines.join("\n")
}

fn render_oracle_draft_created(payload: &Value) -> String {
    let path = string_at_key(payload, &["path"]).unwrap_or_else(|| "-".to_string());
    let draft = value_at_path(payload, &["draft"]).unwrap_or(&Value::Null);
    let id = string_at_key(draft, &["id"]).unwrap_or_else(|| "-".to_string());
    let action = string_at_key(draft, &["action"]).unwrap_or_else(|| "-".to_string());
    let node =
        string_at_key(draft, &["node_label", "nodeLabel"]).unwrap_or_else(|| "-".to_string());
    let row = string_at_key(draft, &["row_label", "rowLabel"]).unwrap_or_else(|| "-".to_string());
    [
        "Oracle draft created".to_string(),
        format!("{id} | {action} | {node} | row={row}"),
        format!("store={path}"),
        "Next: petri oracle drafts validate latest && petri oracle drafts show latest".to_string(),
    ]
    .join("\n")
}

fn render_oracle_drafts_validate(payload: &Value) -> String {
    render_oracle_drafts_check("Oracle draft validation", payload)
}

fn render_oracle_drafts_preview(payload: &Value) -> String {
    render_oracle_drafts_check("Oracle draft preview", payload)
}

fn render_oracle_drafts_check(title: &str, payload: &Value) -> String {
    let selector = string_at_key(payload, &["selector"]).unwrap_or_else(|| "-".to_string());
    let selected = string_at_key(payload, &["selectedCount"]).unwrap_or_else(|| "0".to_string());
    let failed = string_at_key(payload, &["failedCount"]).unwrap_or_else(|| "0".to_string());
    let results = array_at_key(payload, &["results"])
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let mut lines = vec![
        title.to_string(),
        format!("selector={selector} | selected={selected} | failed={failed}"),
    ];
    lines.extend(results.iter().map(render_oracle_draft_check_result));
    lines.join("\n")
}

fn render_oracle_draft_check_result(result: &Value) -> String {
    let id = oracle_review_text(result, &["id"]);
    let action = oracle_review_text(result, &["action"]);
    let mut lines = if result.get("ok").and_then(Value::as_bool) == Some(true) {
        vec![format!(
            "{id} | {action} | semantic draft ok | transaction=not_wired"
        )]
    } else {
        let issue = oracle_review_text(result, &["issue"]);
        vec![format!("{id} | {action} | issue={issue}")]
    };
    let Some(review) = result.get("review") else {
        return lines.join("\n");
    };

    let breadcrumb = review
        .get("breadcrumb")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(Value::as_str)
                .map(terminal_safe_text)
                .collect::<Vec<_>>()
                .join(" > ")
        })
        .filter(|breadcrumb| !breadcrumb.is_empty())
        .unwrap_or_else(|| "unavailable".to_string());
    lines.extend([
        format!(
            "  Market: {} | Month: {} | Expiry: {} | Oracle month: {}",
            oracle_review_text(review, &["marketId"]),
            oracle_review_text(review, &["monthLabel"]),
            oracle_review_text(review, &["expiryId"]),
            oracle_review_text(review, &["oracleMonth"]),
        ),
        format!(
            "  Phase: {} | Node: {} ({}) | Row: {}",
            oracle_review_text(result, &["phase"]),
            oracle_review_text(review, &["nodeLabel"]),
            oracle_review_text(review, &["nodeKind"]),
            oracle_review_text(review, &["rowLabel"]),
        ),
        format!(
            "  Source: {} | Update: {} | Created: {}",
            oracle_review_text(review, &["sourceState"]),
            oracle_review_text(review, &["updateState"]),
            oracle_review_text(review, &["createdAtUnixSeconds"]),
        ),
        format!("  Breadcrumb: {breadcrumb}"),
        format!("  Summary: {}", oracle_review_text(review, &["summary"])),
        format!("  Modules: {}", oracle_review_text(review, &["modules"])),
        format!(
            "  Backend status: {}",
            oracle_review_text(review, &["backendStatus"])
        ),
    ]);
    let fields = review
        .get("fields")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if fields.is_empty() {
        lines.push("  Fields: none".to_string());
    } else {
        lines.push("  Fields:".to_string());
        lines.extend(fields.iter().map(|field| {
            format!(
                "    {}: {}",
                oracle_review_text(field, &["label"]),
                oracle_review_text(field, &["value"]),
            )
        }));
    }
    lines.join("\n")
}

fn oracle_review_text(value: &Value, keys: &[&str]) -> String {
    string_at_key(value, keys)
        .filter(|value| !value.trim().is_empty())
        .map(|value| terminal_safe_text(&value))
        .unwrap_or_else(|| "unavailable".to_string())
}

fn render_oracle_draft_line(draft: &Value) -> String {
    let id = string_at_key(draft, &["id"]).unwrap_or_else(|| "-".to_string());
    let action = string_at_key(draft, &["action"]).unwrap_or_else(|| "-".to_string());
    let node =
        string_at_key(draft, &["nodeLabel", "node_label"]).unwrap_or_else(|| "-".to_string());
    let row = string_at_key(draft, &["rowLabel", "row_label"]).unwrap_or_else(|| "-".to_string());
    let status = string_at_key(draft, &["backendStatus", "backend_status"])
        .unwrap_or_else(|| "-".to_string());
    format!("{id} | {action} | {node} | row={row} | {status}")
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CurrentLiquidityEntry {
    Add {
        bin_id: u16,
        maximum_option_amount: u64,
        maximum_quote_amount: u64,
        minimum_shares: u128,
    },
    Remove {
        bin_id: u16,
        shares: u128,
        minimum_option_out: u64,
        minimum_quote_out: u64,
    },
}

fn run_dlmm_liquidity_command(
    cli: &Cli,
    backend: &BackendClient,
    liquidity: &LiquidityArgs,
    action: LiquidityActionValue,
) -> Result<(), CliError> {
    let config = build_onchain_config(cli)?;
    crate::chain_identity::verify_onchain_config_fresh(&config)?;
    let payload = build_dlmm_liquidity_payload(&config, liquidity, action)?;
    let response = portable_operation::prepare(
        &config,
        backend,
        portable_operation::Family::Liquidity,
        payload,
    )?;
    emit_output(cli, &response, portable_operation::render(&response))
}

fn build_dlmm_liquidity_payload(
    config: &OnchainConfig,
    liquidity: &LiquidityArgs,
    action: LiquidityActionValue,
) -> Result<Value, CliError> {
    let market_id = liquidity.market.trim().to_ascii_lowercase();
    if market_id.is_empty() {
        return Err(CliError::new("liquidity market id cannot be empty"));
    }
    let expiry_id = liquidity.expiry.trim().to_string();
    if expiry_id.is_empty() {
        return Err(CliError::new("liquidity expiry id cannot be empty"));
    }
    let action_value = action.as_request_value();
    let entries = parse_current_liquidity_argument_entries(&liquidity.entries, action_value)?;
    Ok(json!({
        "marketId": market_id,
        "expiryId": expiry_id,
        "ownerPubkey": load_keypair_pubkey(config)?,
        "action": action_value,
        "positionNonce": liquidity.position_nonce.to_string(),
        "entries": entries.iter().map(current_liquidity_entry_json).collect::<Vec<_>>(),
    }))
}

fn parse_current_liquidity_argument_entries(
    raw_entries: &[String],
    action: &str,
) -> Result<Vec<CurrentLiquidityEntry>, CliError> {
    if !matches!(action, "add" | "remove" | "close_position") {
        return Err(CliError::new("liquidity action is not current"));
    }
    if raw_entries.is_empty() || raw_entries.len() > 32 {
        return Err(CliError::new(
            "liquidity requires between 1 and 32 --entry values",
        ));
    }
    let mut entries = Vec::with_capacity(raw_entries.len());
    let mut previous_bin = 0_u16;
    for (index, raw) in raw_entries.iter().enumerate() {
        let parts = raw.split(':').collect::<Vec<_>>();
        if parts.len() != 4
            || parts
                .iter()
                .any(|part| part.is_empty() || *part != part.trim())
        {
            return Err(CliError::new(format!(
                "liquidity --entry {} must be BIN:AMOUNT_A:AMOUNT_B:AMOUNT_C",
                index + 1
            )));
        }
        let bin = parse_current_liquidity_u128(parts[0], &format!("entry {} bin", index + 1))?;
        let bin = u16::try_from(bin).map_err(|_| {
            CliError::new(format!(
                "liquidity entry {} bin must be within 1..2048",
                index + 1
            ))
        })?;
        if !(1..=2048).contains(&bin) || bin <= previous_bin {
            return Err(CliError::new(
                "liquidity bins must be unique and strictly ascending within 1..2048",
            ));
        }
        previous_bin = bin;
        let entry = if action == "add" {
            CurrentLiquidityEntry::Add {
                bin_id: bin,
                maximum_option_amount: parse_current_liquidity_u64(
                    parts[1],
                    &format!("entry {} maximum option amount", index + 1),
                )?,
                maximum_quote_amount: parse_current_liquidity_u64(
                    parts[2],
                    &format!("entry {} maximum quote amount", index + 1),
                )?,
                minimum_shares: parse_current_liquidity_u128(
                    parts[3],
                    &format!("entry {} minimum shares", index + 1),
                )?,
            }
        } else {
            let shares =
                parse_current_liquidity_u128(parts[1], &format!("entry {} shares", index + 1))?;
            if shares == 0 {
                return Err(CliError::new(format!(
                    "liquidity entry {} shares must be positive",
                    index + 1
                )));
            }
            CurrentLiquidityEntry::Remove {
                bin_id: bin,
                shares,
                minimum_option_out: parse_current_liquidity_u64(
                    parts[2],
                    &format!("entry {} minimum option output", index + 1),
                )?,
                minimum_quote_out: parse_current_liquidity_u64(
                    parts[3],
                    &format!("entry {} minimum quote output", index + 1),
                )?,
            }
        };
        entries.push(entry);
    }
    Ok(entries)
}

fn current_liquidity_entry_json(entry: &CurrentLiquidityEntry) -> Value {
    match entry {
        CurrentLiquidityEntry::Add {
            bin_id,
            maximum_option_amount,
            maximum_quote_amount,
            minimum_shares,
        } => json!({
            "binId": bin_id,
            "maximumOptionAmount": maximum_option_amount.to_string(),
            "maximumQuoteAmount": maximum_quote_amount.to_string(),
            "minimumShares": minimum_shares.to_string(),
        }),
        CurrentLiquidityEntry::Remove {
            bin_id,
            shares,
            minimum_option_out,
            minimum_quote_out,
        } => json!({
            "binId": bin_id,
            "shares": shares.to_string(),
            "minimumOptionOut": minimum_option_out.to_string(),
            "minimumQuoteOut": minimum_quote_out.to_string(),
        }),
    }
}

fn parse_current_liquidity_u128(raw: &str, label: &str) -> Result<u128, CliError> {
    if raw.is_empty()
        || (raw.len() > 1 && raw.starts_with('0'))
        || !raw.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(CliError::new(format!(
            "{label} must be a canonical unsigned decimal"
        )));
    }
    raw.parse::<u128>().map_err(|_| {
        CliError::new(format!(
            "{label} is outside the supported unsigned 128-bit range"
        ))
    })
}

fn parse_current_liquidity_u64(raw: &str, label: &str) -> Result<u64, CliError> {
    let value = parse_current_liquidity_u128(raw, label)?;
    u64::try_from(value).map_err(|_| CliError::new(format!("{label} is outside the u64 range")))
}

fn normalized_oracle_phase(phase: &str) -> String {
    oracle_lifecycle::normalize_phase_label(phase)
}

fn integer_i64_from_value(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
        .or_else(|| {
            value.as_str().and_then(|value| {
                value.trim().parse().ok().or_else(|| {
                    chrono::DateTime::parse_from_rfc3339(value.trim())
                        .ok()
                        .map(|timestamp| timestamp.timestamp())
                })
            })
        })
}

fn authoritative_schedule_value<'a>(month: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    value_at_key(month, keys).or_else(|| {
        value_at_key(
            month,
            &[
                "schedule",
                "lifecycleSchedule",
                "lifecycle_schedule",
                "oracleSchedule",
                "oracle_schedule",
            ],
        )
        .and_then(|schedule| value_at_key(schedule, keys))
    })
}

fn derive_scheduled_oracle_phase(
    raw_phase: &str,
    schedule_version: Option<u64>,
    scramble_start_ts: Option<i64>,
    listing_ts: Option<i64>,
    expiry_ts: Option<i64>,
    now_ts: i64,
) -> String {
    oracle_lifecycle::classify_oracle_lifecycle(
        raw_phase,
        schedule_version,
        scramble_start_ts,
        listing_ts,
        expiry_ts,
        now_ts,
    )
    .label()
    .to_string()
}

fn fetch_authoritative_oracle_phase(
    backend: &BackendClient,
    market_id: &str,
    expiry_id: &str,
) -> Result<String, CliError> {
    let market_id = market_id.trim();
    let expiry_id = expiry_id.trim();
    let payload = backend
        .get(&endpoints::dlmm_oracle_market(market_id))
        .map_err(|error| {
            CliError::new(format!(
                "could not verify the authoritative on-chain lifecycle for market {}, series {}: {error}",
                market_id.to_ascii_uppercase(),
                expiry_id.to_ascii_uppercase()
            ))
        })?;
    let data = unwrap_data(&payload);
    let oracle = value_at_key(data, &["oracle"]).unwrap_or(data);
    let month = array_at_key(oracle, &["oracleMonths", "oracle_months"])
        .and_then(|months| {
            months.iter().find(|month| {
                string_at_key(month, &["expiryId", "expiry_id"])
                    .is_some_and(|candidate| candidate.eq_ignore_ascii_case(expiry_id))
            })
        })
        .ok_or_else(|| {
            CliError::new(format!(
                "authoritative on-chain lifecycle is unavailable for market {}, series {}; no option order was prepared",
                market_id.to_ascii_uppercase(),
                expiry_id.to_ascii_uppercase()
            ))
        })?;
    let raw_phase = string_at_key(month, &["phase"])
        .map(|phase| phase.trim().to_string())
        .filter(|phase| !phase.is_empty())
        .ok_or_else(|| {
            CliError::new(format!(
                "authoritative on-chain lifecycle has no phase for market {}, series {}; no option order was prepared",
                market_id.to_ascii_uppercase(),
                expiry_id.to_ascii_uppercase()
            ))
        })?;
    let schedule_version =
        authoritative_schedule_value(month, &["scheduleVersion", "schedule_version"]).and_then(
            |value| {
                value
                    .as_u64()
                    .or_else(|| value.as_str().and_then(|value| value.trim().parse().ok()))
            },
        );
    let backend_scramble_start_ts = authoritative_schedule_value(
        month,
        &[
            "scrambleStartTs",
            "scramble_start_ts",
            "scrambleStartUnix",
            "scramble_start_unix",
        ],
    )
    .and_then(integer_i64_from_value);
    let backend_listing_ts = authoritative_schedule_value(
        month,
        &["listingTs", "listing_ts", "listingUnix", "listing_unix"],
    )
    .and_then(integer_i64_from_value);
    let backend_expiry_ts = authoritative_schedule_value(
        month,
        &[
            "expiryTs",
            "expiry_ts",
            "settlementTs",
            "settlement_ts",
            "settlementUtc",
            "settlement_utc",
        ],
    )
    .and_then(integer_i64_from_value);
    let now_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok());
    Ok(now_ts
        .map(|now_ts| {
            derive_scheduled_oracle_phase(
                &raw_phase,
                schedule_version,
                backend_scramble_start_ts,
                backend_listing_ts,
                backend_expiry_ts,
                now_ts,
            )
        })
        .unwrap_or_else(|| "Lifecycle unavailable".to_string()))
}

fn authoritative_phase_is_game(phase: &str) -> bool {
    matches!(
        normalized_oracle_phase(phase).as_str(),
        "2" | "game" | "game_mode"
    )
}

fn load_keypair_pubkey(config: &OnchainConfig) -> Result<String, CliError> {
    wallet_signer::signer_pubkey(config)
}

fn build_config_show_payload(cli: &Cli) -> Result<Value, CliError> {
    build_config_show_payload_for_backend(cli, &cli.backend_url)
}

fn build_config_show_payload_for_backend(cli: &Cli, backend_url: &str) -> Result<Value, CliError> {
    let solana_cli_config = solana_config::load_solana_cli_config(cli.solana_config.as_deref())?;
    let keypair_path =
        solana_config::resolve_keypair_path(cli.keypair.as_deref(), solana_cli_config.as_ref());
    let backend_url =
        petri_config::normalize_amoeba_backend_url(backend_url).map_err(CliError::new)?;
    let rpc_url = petri_config::rpc_gateway_url(&backend_url).map_err(CliError::new)?;
    let commitment =
        solana_config::resolve_commitment(cli.commitment.as_deref(), solana_cli_config.as_ref());
    let keypair_pubkey = wallet_signer::signer_pubkey_from_path(&keypair_path).ok();

    Ok(json!({
        "backendUrl": backend_url,
        "cluster": cli.cluster,
        "petriConfigPath": petri_config::config_path()
            .map_err(CliError::new)?
            .display()
            .to_string(),
        "solanaConfigPath": solana_cli_config
            .as_ref()
            .map(|config| config.path.display().to_string())
            .unwrap_or_else(|| solana_config::resolve_config_path(cli.solana_config.as_deref()).display().to_string()),
        "solanaConfigLoaded": solana_cli_config.is_some(),
        "amoebaReadGatewayUrl": rpc_url,
        "commitment": commitment,
        "keypairPath": keypair_path,
        "keypairPubkey": keypair_pubkey,
        "keypairFile": solana_config::keypair_file_security_label(&keypair_path),
        "allowInsecureKeypair": cli.allow_insecure_keypair,
        "output": format!("{:?}", cli.resolved_output()).to_lowercase(),
        "currentRelease": {
            "liveReleaseLabel": current_release::LIVE_RELEASE_LABEL,
            "sdkCommit": current_release::SDK_PACKAGE_COMMIT,
            "packageBuildIdentity": ameba_sdk::current_sdk_package_build_identity_v1().ok(),
            "writeCompatibility": current_release::WRITE_COMPATIBILITY,
            "walletChangesAvailable": false,
            "packageWriteCapable": current_release::require_current_write_release().is_ok(),
            "runtimePermission": "requires-finalized-verification",
        },
    }))
}

fn render_config_show(payload: &Value) -> String {
    let backend_url = string_at_key(payload, &["backendUrl"]).unwrap_or_else(|| "-".to_string());
    let cluster = string_at_key(payload, &["cluster"]).unwrap_or_else(|| "-".to_string());
    let petri_config_path =
        string_at_key(payload, &["petriConfigPath"]).unwrap_or_else(|| "-".to_string());
    let config_path =
        string_at_key(payload, &["solanaConfigPath"]).unwrap_or_else(|| "-".to_string());
    let config_loaded =
        string_at_key(payload, &["solanaConfigLoaded"]).unwrap_or_else(|| "false".to_string());
    let rpc_url =
        string_at_key(payload, &["amoebaReadGatewayUrl"]).unwrap_or_else(|| "-".to_string());
    let commitment = string_at_key(payload, &["commitment"]).unwrap_or_else(|| "-".to_string());
    let keypair_path = string_at_key(payload, &["keypairPath"]).unwrap_or_else(|| "-".to_string());
    let keypair_pubkey =
        string_at_key(payload, &["keypairPubkey"]).unwrap_or_else(|| "-".to_string());
    let keypair_file =
        string_at_key(payload, &["keypairFile"]).unwrap_or_else(|| "unknown".to_string());
    let allow_insecure =
        string_at_key(payload, &["allowInsecureKeypair"]).unwrap_or_else(|| "false".to_string());
    let output = string_at_key(payload, &["output"]).unwrap_or_else(|| "plain".to_string());
    let write_compatibility = value_at_path(payload, &["currentRelease", "writeCompatibility"])
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| "unavailable".to_string());

    [
        format!("backend={backend_url} | cluster={cluster}"),
        format!("petriConfig={petri_config_path}"),
        format!("solanaConfig={config_path} | loaded={config_loaded}"),
        format!("amoebaReadGateway={rpc_url} | commitment={commitment}"),
        format!(
            "keypairPath={keypair_path} | pubkey={keypair_pubkey} | keypairFile={keypair_file} | allowInsecureKeypair={allow_insecure}"
        ),
        format!("output={output}"),
        format!("currentRelease=v3 | runtimePermission=requires-finalized-verification | walletChanges={write_compatibility}"),
    ]
    .join("\n")
}

fn resolve_wallet_owner_pubkey(
    config: &OnchainConfig,
    owner_pubkey: Option<&str>,
) -> Result<String, CliError> {
    if let Some(owner) = owner_pubkey
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Pubkey::from_str(owner)
            .map_err(|error| CliError::new(format!("invalid owner pubkey {owner}: {error}")))?;
        return Ok(owner.to_string());
    }

    load_keypair_pubkey(config)
}

fn parse_policy_pubkey(value: &str, label: &str) -> Result<Pubkey, CliError> {
    Pubkey::from_str(value.trim())
        .map_err(|error| CliError::new(format!("{label} is not a valid Solana address: {error}")))
}

fn required_prepared_pubkey(value: &Value, keys: &[&str], label: &str) -> Result<Pubkey, CliError> {
    let raw = string_at_key(value, keys)
        .filter(|raw| !raw.trim().is_empty())
        .ok_or_else(|| CliError::new(format!("trade prepare response is missing {label}")))?;
    parse_policy_pubkey(&raw, label)
}

pub(crate) fn current_trade_response_data(
    response: Value,
    operation: &str,
) -> Result<Value, CliError> {
    let object = exact_object_fields(&response, &["ok", "data"], operation)?;
    if object.get("ok") != Some(&Value::Bool(true)) {
        return Err(CliError::new(format!(
            "Amoeba {operation} response did not confirm success"
        )));
    }
    let data = object
        .get("data")
        .ok_or_else(|| CliError::new(format!("Amoeba {operation} response is missing data")))?;
    if !data.is_object() {
        return Err(CliError::new(format!(
            "Amoeba {operation} response data must be an object"
        )));
    }
    chain_identity::validate_current_protocol_data(data)?;
    Ok(data.clone())
}

fn exact_object_fields<'a>(
    value: &'a Value,
    expected: &[&str],
    label: &str,
) -> Result<&'a serde_json::Map<String, Value>, CliError> {
    let object = value
        .as_object()
        .ok_or_else(|| CliError::new(format!("trade prepare response has invalid {label}")))?;
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err(CliError::new(format!(
            "trade prepare response has unsupported {label} fields"
        )));
    }
    Ok(object)
}

fn canonical_prepared_u64(value: &Value, label: &str) -> Result<u64, CliError> {
    let raw = value.as_str().ok_or_else(|| {
        CliError::new(format!(
            "trade prepare response {label} must be a canonical u64 string"
        ))
    })?;
    if raw.is_empty()
        || (raw.len() > 1 && raw.starts_with('0'))
        || !raw.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(CliError::new(format!(
            "trade prepare response {label} must be a canonical u64 string"
        )));
    }
    let parsed = raw.parse::<u64>().map_err(|_| {
        CliError::new(format!(
            "trade prepare response {label} is outside the u64 range"
        ))
    })?;
    if parsed.to_string() != raw {
        return Err(CliError::new(format!(
            "trade prepare response {label} is not canonical"
        )));
    }
    Ok(parsed)
}

fn bool_at_key(payload: &Value, keys: &[&str]) -> Option<bool> {
    value_at_key(payload, keys).and_then(|value| match value {
        Value::Bool(flag) => Some(*flag),
        Value::String(text) => match text.as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        },
        _ => None,
    })
}

fn emit_output(cli: &Cli, payload: &Value, plain: String) -> Result<(), CliError> {
    match cli.resolved_output() {
        OutputFormat::Plain => {
            if !cli.quiet {
                println!("{plain}");
            }
        }
        OutputFormat::Json => {
            let public;
            let payload = if mcp_actions::active() {
                public = mcp_actions::public_output(payload);
                &public
            } else {
                payload
            };
            println!("{}", json_string(payload)?);
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum MarketAgentView {
    Show,
    Status,
    Print,
}

impl MarketAgentView {
    fn label(self) -> &'static str {
        match self {
            Self::Show => "market",
            Self::Status => "status",
            Self::Print => "print",
        }
    }
}

fn market_agent_payload(
    requested_market: &str,
    payload: &Value,
    view: MarketAgentView,
) -> Result<Value, CliError> {
    let data = unwrap_data(payload);
    let snapshot = data
        .get("snapshot")
        .unwrap_or(data)
        .as_object()
        .ok_or_else(|| CliError::new("current market response is missing snapshot"))?;
    let requested_market = requested_market.trim().to_ascii_lowercase();
    if snapshot.get("marketId").and_then(Value::as_str) != Some(requested_market.as_str())
        || snapshot
            .get("onChainAvailable")
            .and_then(Value::as_bool)
            .is_none()
    {
        return Err(CliError::new(
            "current market snapshot has an invalid market identity or availability flag",
        ));
    }
    let expiries = snapshot
        .get("expiries")
        .and_then(Value::as_array)
        .ok_or_else(|| CliError::new("current market snapshot is missing exact series rows"))?;
    for row in expiries {
        validate_current_contract_row(&requested_market, row)?;
    }
    let front_expiry = expiries.first().map_or(Value::Null, |row| {
        json!({
            "id": row.get("expiryId").cloned().unwrap_or(Value::Null),
            "label": row.get("label").cloned().unwrap_or(Value::Null),
            "optionKind": row.get("optionKind").cloned().unwrap_or(Value::Null),
            "settlementUtc": row.get("settlementUtc").cloned().unwrap_or(Value::Null),
            "lowerStrike": row.get("lowerStrike").cloned().unwrap_or(Value::Null),
            "upperStrike": row.get("upperStrike").cloned().unwrap_or(Value::Null),
            "priceDisplayDecimals": row.get("priceDisplayDecimals").cloned().unwrap_or(Value::Null),
            "quoteDisplayDecimals": row.get("quoteDisplayDecimals").cloned().unwrap_or(Value::Null),
            "poolStatus": row.get("poolStatus").cloned().unwrap_or(Value::Null),
            "contractRows": 1,
        })
    });
    Ok(json!({
        "ok": true,
        "view": view.label(),
        "marketId": requested_market,
        "symbol": snapshot.get("symbol").cloned().unwrap_or(Value::Null),
        "title": snapshot.get("title").cloned().unwrap_or(Value::Null),
        "source": snapshot.get("source").cloned().unwrap_or(Value::Null),
        "asOf": snapshot.get("asOf").cloned().unwrap_or(Value::Null),
        "freshness": public_freshness(&Value::Object(snapshot.clone())),
        "marketExecution": public_execution(&Value::Object(snapshot.clone())),
        "onChainAvailable": snapshot.get("onChainAvailable").cloned().unwrap_or(Value::Null),
        "frontExpiry": front_expiry,
        "expiryCount": expiries.len(),
        "issues": bounded_public_issues(payload, &Value::Object(snapshot.clone())),
        "next": "Use contracts.chain with an exact expiryId for one current in-program DLMM series."
    }))
}

fn options_chain_agent_payload(
    options: &OptionsChainArgs,
    payload: &Value,
    lifecycles: &BTreeMap<String, Result<String, CliError>>,
) -> Result<Value, CliError> {
    let snapshot = current_contract_snapshot(options, payload)
        .expect("current contract snapshot validated before rendering");
    let selected = selected_current_contract_rows(options, payload)?;
    let rows_total = current_contract_rows(options, payload)?.len();
    let exact_selection = options
        .expiry
        .as_deref()
        .is_some_and(|expiry| !expiry.trim().is_empty());
    let mut rows = Vec::with_capacity(selected.len());
    let mut available_calls = 0usize;
    let mut available_puts = 0usize;
    let mut eligibility_checked = exact_selection;

    for row in selected {
        let expiry_id = row
            .get("expiryId")
            .and_then(Value::as_str)
            .expect("validated expiryId");
        let lifecycle = lifecycles.get(expiry_id);
        eligibility_checked &= matches!(lifecycle, Some(Ok(_)));
        let lifecycle_phase = lifecycle.and_then(|value| value.as_ref().ok().map(String::as_str));
        let lifecycle_issue =
            lifecycle.and_then(|value| value.as_ref().err().map(ToString::to_string));
        let series =
            project_contract_series(snapshot, row, lifecycle_phase, lifecycle_issue.as_deref());
        if series.get("preparationEligible") == Some(&Value::Bool(true)) {
            match series.get("optionKind").and_then(Value::as_str) {
                Some("call_spread") => available_calls += 1,
                Some("put_spread") => available_puts += 1,
                _ => {}
            }
        }
        rows.push(series);
    }

    let single = exact_selection.then(|| rows[0].clone());
    let single_expiry = single.as_ref().map(|series| {
        json!({
            "id": public_value(series, &["seriesId"]),
            "label": public_value(series, &["label"]),
            "settlementUtc": public_value(series, &["settlementUtc"]),
            "priceDisplayDecimals": public_value(series, &["priceDisplayDecimals"]),
            "quoteDisplayDecimals": public_value(series, &["quoteDisplayDecimals"]),
        })
    });
    let single_lifecycle = single.as_ref().map(|series| {
        json!({
            "source": if eligibility_checked { "on_chain" } else { "unavailable" },
            "phase": public_value(series, &["lifecyclePhase"]),
            "tradeWindowOpen": public_value(series, &["tradeWindowOpen"]),
        })
    });
    let rows_shown = rows.len();
    let available_count = available_calls + available_puts;
    let market_id = snapshot
        .get("marketId")
        .and_then(Value::as_str)
        .expect("validated marketId");
    Ok(json!({
        "ok": true,
        "marketId": market_id,
        "symbol": public_value(snapshot, &["symbol"]),
        "title": public_value(snapshot, &["title"]),
        "asOf": public_value(snapshot, &["asOf"]),
        "freshness": public_freshness(snapshot),
        "marketExecution": public_execution(snapshot),
        "lifecycle": single_lifecycle,
        "expiry": single_expiry,
        "series": single,
        "rows": rows,
        "rowsShown": rows_shown,
        "rowsTotal": rows_total,
        "availableCalls": available_calls,
        "availablePuts": available_puts,
        "availableCount": available_count,
        "eligibilityAttempted": exact_selection,
        "eligibilityChecked": eligibility_checked,
        "issues": bounded_public_issues(payload, snapshot),
        "selectionRule": if exact_selection {
            if eligibility_checked {
                "One exact current series row is selected by expiryId and checked against authoritative lifecycle state; no aliases or synthesized ids are accepted."
            } else {
                "One exact current series row is selected by expiryId, but authoritative lifecycle state is unavailable; no aliases or synthesized ids are accepted."
            }
        } else if options.all {
            "All exact current series rows are sorted by settlement, option kind, and series id."
        } else {
            "Exact current series rows are sorted by settlement, option kind, and series id, then bounded by --rows."
        }
    }))
}

fn project_contract_series(
    snapshot: &Value,
    row: &Value,
    lifecycle_phase: Option<&str>,
    lifecycle_issue: Option<&str>,
) -> Value {
    let expiry_id = row
        .get("expiryId")
        .and_then(Value::as_str)
        .expect("validated expiryId");
    let option_kind = row
        .get("optionKind")
        .and_then(Value::as_str)
        .expect("validated optionKind");
    let quote = market_surface::option_quotes_from_expiry(Some(row))
        .into_iter()
        .next()
        .expect("validated current quote projection");
    let lifecycle_tradeable = lifecycle_phase.is_some_and(authoritative_phase_is_game);
    let market_current = snapshot.get("status").and_then(Value::as_str) == Some("current");
    let on_chain_available = snapshot.get("onChainAvailable") == Some(&Value::Bool(true));
    let snapshot_trade_ready = snapshot
        .get("freshness")
        .and_then(|freshness| freshness.get("tradeReady"))
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let preparation_eligible = lifecycle_tradeable
        && market_current
        && on_chain_available
        && snapshot_trade_ready
        && quote.prepare_eligible;
    let maximum_payout = quote
        .upper_strike
        .parse::<f64>()
        .ok()
        .zip(quote.lower_strike.parse::<f64>().ok())
        .map(|(upper, lower)| upper - lower)
        .filter(|value| value.is_finite() && *value > 0.0);
    let maximum_loss = quote.ask.filter(|value| value.is_finite() && *value > 0.0);
    let maximum_gain = maximum_payout
        .zip(maximum_loss)
        .map(|(payout, loss)| (payout - loss).max(0.0));
    let mut issues = Vec::new();
    if !lifecycle_tradeable {
        issues.push(match (lifecycle_phase, lifecycle_issue) {
            (Some(phase), _) => {
                format!("{expiry_id} is in authoritative on-chain phase '{phase}', not Game Mode.")
            }
            (None, Some(issue)) => issue.to_string(),
            (None, None) => format!(
                "Lifecycle eligibility was not checked in list view; use --expiry {expiry_id} for the authoritative on-chain phase."
            ),
        });
    }
    if lifecycle_phase.is_some() || lifecycle_issue.is_some() {
        if !market_current {
            issues.push(format!(
                "{expiry_id} is unavailable while the current market is paused."
            ));
        }
        if !on_chain_available {
            issues.push(format!(
                "{expiry_id} is unavailable because current on-chain market state is not available."
            ));
        }
        if !snapshot_trade_ready {
            issues.push(format!(
                "{expiry_id} cannot be prepared until the authoritative market snapshot is healthy, fresh, and trade-ready."
            ));
        }
        if !quote.prepare_eligible {
            issues.push(format!(
                "{expiry_id} requires an Active current program pool; reported pool status is '{}'.",
                quote.status
            ));
        }
    }
    json!({
        "seriesId": expiry_id,
        "label": public_value(row, &["label"]),
        "settlementUtc": public_value(row, &["settlementUtc"]),
        "priceDisplayDecimals": public_value(row, &["priceDisplayDecimals"]),
        "quoteDisplayDecimals": public_value(row, &["quoteDisplayDecimals"]),
        "optionKind": option_kind,
        "lowerStrike": quote.lower_strike,
        "upperStrike": quote.upper_strike,
        "bid": quote.bid,
        "ask": quote.ask,
        "mid": quote.mid,
        "depthUsd": quote.depth_usd,
        "volume": quote.volume,
        "openInterest": quote.open_interest,
        "probabilityItm": quote.probability_itm,
        "probabilityCapHit": quote.probability_cap_hit,
        "poolStatus": quote.status,
        "onChainAvailable": snapshot.get("onChainAvailable").cloned().unwrap_or(Value::Null),
        "lifecyclePhase": lifecycle_phase,
        "tradeWindowOpen": lifecycle_tradeable,
        "preparationEligible": preparation_eligible,
        "maximumLoss": maximum_loss,
        "maximumGain": maximum_gain,
        "maximumPayout": maximum_payout,
        "issues": issues,
    })
}

fn render_options_chain(projected: &Value) -> String {
    let field =
        |value: &Value, key: &str| string_at_key(value, &[key]).unwrap_or_else(|| "-".to_string());
    let eligibility_attempted = projected.get("eligibilityAttempted") == Some(&Value::Bool(true));
    let eligibility_checked = projected.get("eligibilityChecked") == Some(&Value::Bool(true));
    let mut lines = vec![
        format!(
            "{} options | {}",
            field(&projected, "symbol"),
            field(&projected, "marketId")
        ),
        if eligibility_checked {
            format!(
                "Showing {} of {} current series | {} eligible",
                field(projected, "rowsShown"),
                field(projected, "rowsTotal"),
                field(projected, "availableCount"),
            )
        } else if eligibility_attempted {
            format!(
                "Showing {} of {} exact current series | authoritative lifecycle unavailable",
                field(projected, "rowsShown"),
                field(projected, "rowsTotal"),
            )
        } else {
            format!(
                "Showing {} of {} current series | lifecycle eligibility is checked in exact --expiry view",
                field(projected, "rowsShown"),
                field(projected, "rowsTotal"),
            )
        },
        "Series | Kind | Lower | Upper | Bid | Ask | Phase | Status".to_string(),
    ];
    if let Some(rows) = projected.get("rows").and_then(Value::as_array) {
        for series in rows {
            lines.push(format!(
                "{} | {} | {} | {} | {} | {} | {} | {}",
                field(series, "seriesId"),
                field(series, "optionKind"),
                field(series, "lowerStrike"),
                field(series, "upperStrike"),
                field(series, "bid"),
                field(series, "ask"),
                field(series, "lifecyclePhase"),
                if series.get("preparationEligible") == Some(&Value::Bool(true)) {
                    "eligible"
                } else if eligibility_attempted && !eligibility_checked {
                    "unavailable"
                } else if !eligibility_checked {
                    "not checked"
                } else {
                    "waiting"
                },
            ));
            lines.push(format!(
                "  settles {} | max loss {} | max gain {} | max payout {} | depth {}",
                field(series, "settlementUtc"),
                field(series, "maximumLoss"),
                field(series, "maximumGain"),
                field(series, "maximumPayout"),
                field(series, "depthUsd"),
            ));
            if let Some(row_issues) = series.get("issues").and_then(Value::as_array) {
                let row_issues = row_issues
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>();
                if !row_issues.is_empty() {
                    lines.push(format!("  note: {}", row_issues.join(" | ")));
                }
            }
        }
    }
    let issues = projected
        .get("issues")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    if !issues.is_empty() {
        lines.push(format!("Note: {}", issues.join(" | ")));
    }
    lines.join("\n")
}

fn public_value(value: &Value, keys: &[&str]) -> Value {
    value_at_key(value, keys).cloned().unwrap_or(Value::Null)
}

fn public_freshness(snapshot: &Value) -> Value {
    let Some(freshness) = value_at_key(snapshot, &["freshness"]) else {
        return Value::Null;
    };
    json!({
        "source": public_value(freshness, &["source"]),
        "observedAt": public_value(freshness, &["observedAt"]),
        "observedAtSlot": public_value(freshness, &["observedAtSlot"]),
        "ageMs": public_value(freshness, &["ageMs"]),
        "maximumAgeMs": public_value(freshness, &["maximumAgeMs"]),
        "stale": public_value(freshness, &["stale"]),
        "refreshHealthy": public_value(freshness, &["refreshHealthy"]),
        "tradeReady": public_value(freshness, &["tradeReady"]),
        "lastAttemptAt": public_value(freshness, &["lastAttemptAt"]),
        "lastErrorCode": public_value(freshness, &["lastErrorCode"]),
    })
}

fn public_execution(snapshot: &Value) -> Value {
    let Some(execution) = value_at_key(snapshot, &["execution"]) else {
        return Value::Null;
    };
    json!({
        "marketReady": execution.get("ready").and_then(Value::as_bool),
        "message": public_value(execution, &["message"]),
        "note": "Market eligibility is read evidence only; the current release cannot prepare or sign transactions."
    })
}

fn bounded_public_issues(payload: &Value, snapshot: &Value) -> Vec<String> {
    let mut issues = Vec::new();
    for context in [payload, unwrap_data(payload), snapshot] {
        if let Some(values) = array_at_key(context, &["issues"]) {
            for issue in values.iter().filter_map(Value::as_str) {
                let issue = issue.trim();
                if !issue.is_empty() && !issues.iter().any(|known| known == issue) {
                    issues.push(issue.chars().take(240).collect());
                }
                if issues.len() >= 8 {
                    return issues;
                }
            }
        }
    }
    for issue in market_surface::current_freshness_issues(snapshot) {
        if !issues.iter().any(|known| known == &issue) {
            issues.push(issue);
        }
        if issues.len() >= 8 {
            break;
        }
    }
    issues
}

fn render_settlement_show(
    cli: &Cli,
    backend: &BackendClient,
    market_id: &str,
    expiry_id: &str,
    payload: &Value,
) -> String {
    let record =
        value_at_key(unwrap_data(payload), &["settlement"]).unwrap_or_else(|| unwrap_data(payload));
    let status = string_at_key(record, &["status"]).unwrap_or_else(|| "unknown".to_string());
    let settlement_price_atomic = string_at_key(
        record,
        &["settlementPriceAtomic", "settlement_price_atomic"],
    )
    .unwrap_or_else(|| "-".to_string());
    let settlement_price_display = string_at_key(
        record,
        &["settlementPriceDisplay", "settlement_price_display"],
    )
    .unwrap_or_else(|| "-".to_string());
    let settled_at =
        string_at_key(record, &["settledAt", "settled_at"]).unwrap_or_else(|| "-".to_string());
    let source_uri =
        string_at_key(record, &["sourceUri", "source_uri"]).unwrap_or_else(|| "-".to_string());
    let observation_count = array_at_key(record, &["observations"])
        .map(|items| items.len().to_string())
        .unwrap_or_else(|| "0".to_string());

    render_terminal_lines([
        format!("settlement for {market_id}/{expiry_id}"),
        format!("backend={} | cluster={}", backend.base_url(), cli.cluster),
        format!("status={status} | settledAt={settled_at} | observations={observation_count}"),
        format!(
            "settlementPriceAtomic={settlement_price_atomic} | settlementPriceDisplay={settlement_price_display}"
        ),
        format!("sourceUri={source_uri}"),
    ])
}

fn render_settlement_oracle(
    cli: &Cli,
    backend: &BackendClient,
    market_id: &str,
    expiry_id: &str,
    payload: &Value,
) -> String {
    let oracle = value_at_key(unwrap_data(payload), &["oracle", "settlementOracle"])
        .unwrap_or_else(|| unwrap_data(payload));
    let status = string_at_key(oracle, &["status"]).unwrap_or_else(|| "unknown".to_string());
    let status_reason = string_at_key(oracle, &["statusReason", "status_reason"])
        .unwrap_or_else(|| "-".to_string());
    let finalizable = bool_at_key(oracle, &["finalizable"])
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string());
    let finality_reason = string_at_key(oracle, &["finalityReason", "finality_reason"])
        .unwrap_or_else(|| "-".to_string());
    let settlement_ts = string_at_key(oracle, &["settlementTs", "settlement_ts"])
        .unwrap_or_else(|| "-".to_string());
    let settlement_utc = string_at_key(oracle, &["settlementUtc", "settlement_utc"])
        .unwrap_or_else(|| "-".to_string());
    let fetched_at =
        string_at_key(oracle, &["fetchedAt", "fetched_at"]).unwrap_or_else(|| "-".to_string());
    let computation =
        string_at_key(oracle, &["computation", "model"]).unwrap_or_else(|| "-".to_string());
    let base_oracle_atomic = string_at_key(oracle, &["baseOracleAtomic", "base_oracle_atomic"])
        .unwrap_or_else(|| "-".to_string());
    let index_delta_bps = string_at_key(oracle, &["indexDeltaBps", "index_delta_bps"])
        .unwrap_or_else(|| "-".to_string());
    let settlement_price_atomic = string_at_key(
        oracle,
        &["settlementPriceAtomic", "settlement_price_atomic"],
    )
    .unwrap_or_else(|| "-".to_string());
    let settlement_price_display = string_at_key(
        oracle,
        &["settlementPriceDisplay", "settlement_price_display"],
    )
    .unwrap_or_else(|| "-".to_string());
    let source_uri =
        string_at_key(oracle, &["sourceUri", "source_uri"]).unwrap_or_else(|| "-".to_string());
    let source_digest = string_at_key(oracle, &["sourceDigestHex", "source_digest_hex"])
        .unwrap_or_else(|| "-".to_string());
    let observations = array_at_key(oracle, &["observations"])
        .map(|items| items.len().to_string())
        .unwrap_or_else(|| "0".to_string());
    let message_sha = string_at_key(oracle, &["messageSha256Hex", "message_sha256_hex"])
        .unwrap_or_else(|| "-".to_string());
    let signed_payload_sha = string_at_key(
        oracle,
        &["signedPayloadSha256Hex", "signed_payload_sha256_hex"],
    )
    .unwrap_or_else(|| "-".to_string());

    render_terminal_lines([
        format!("settlement oracle for {market_id}/{expiry_id}"),
        format!("backend={} | cluster={}", backend.base_url(), cli.cluster),
        format!("status={status} | finalizable={finalizable} | observations={observations}"),
        format!("statusReason={status_reason} | finalityReason={finality_reason}"),
        format!(
            "settlementTs={settlement_ts} | settlementUtc={settlement_utc} | fetchedAt={fetched_at}"
        ),
        format!("computation={computation}"),
        format!("baseOracleAtomic={base_oracle_atomic} | indexDeltaBps={index_delta_bps}"),
        format!(
            "settlementPriceAtomic={settlement_price_atomic} | settlementPriceDisplay={settlement_price_display}"
        ),
        format!("sourceUri={source_uri}"),
        format!("sourceDigestHex={source_digest}"),
        format!("messageSha256Hex={message_sha} | signedPayloadSha256Hex={signed_payload_sha}"),
    ])
}

fn render_settlement_oracle_check(
    cli: &Cli,
    backend: &BackendClient,
    market_id: &str,
    expiry_id: &str,
    payload: &Value,
) -> String {
    let preflight = value_at_key(unwrap_data(payload), &["preflight", "oraclePreflight"])
        .unwrap_or_else(|| unwrap_data(payload));
    let status = string_at_key(preflight, &["status"]).unwrap_or_else(|| "unknown".to_string());
    let finalizable = bool_at_key(preflight, &["finalizable"])
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string());
    let ready_for_package = bool_at_key(preflight, &["readyForPackage", "ready_for_package"])
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string());
    let ready_for_signing = bool_at_key(preflight, &["readyForSigning", "ready_for_signing"])
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string());
    let ready_for_submission =
        bool_at_key(preflight, &["readyForSubmission", "ready_for_submission"])
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string());
    let package_source_configured = bool_at_key(
        preflight,
        &["packageSourceConfigured", "package_source_configured"],
    )
    .map(|value| value.to_string())
    .unwrap_or_else(|| "-".to_string());
    let signer_endpoint_count =
        string_at_key(preflight, &["signerEndpointCount", "signer_endpoint_count"])
            .unwrap_or_else(|| "-".to_string());
    let signer_expected_count = string_at_key(
        preflight,
        &[
            "signerEndpointExpectedCount",
            "signer_endpoint_expected_count",
        ],
    )
    .unwrap_or_else(|| "-".to_string());
    let settlement_utc = string_at_key(preflight, &["settlementUtc", "settlement_utc"])
        .unwrap_or_else(|| "-".to_string());
    let finality_reason = string_at_key(preflight, &["finalityReason", "finality_reason"])
        .unwrap_or_else(|| "-".to_string());
    let fetched_at =
        string_at_key(preflight, &["fetchedAt", "fetched_at"]).unwrap_or_else(|| "-".to_string());
    let oracle_month = value_at_key(preflight, &["selectedOracleMonth", "selected_oracle_month"]);
    let oracle_month_id = oracle_month
        .and_then(|month| string_at_key(month, &["oracleMonth", "oracle_month"]))
        .unwrap_or_else(|| "-".to_string());
    let oracle_phase = oracle_month
        .and_then(|month| string_at_key(month, &["phase"]))
        .unwrap_or_else(|| "-".to_string());
    let market_pda = oracle_month
        .and_then(|month| string_at_key(month, &["marketPda", "market_pda"]))
        .unwrap_or_else(|| "-".to_string());
    let recipe_hash = oracle_month
        .and_then(|month| string_at_key(month, &["recipeHashHex", "recipe_hash_hex"]))
        .unwrap_or_else(|| "-".to_string());
    let settlement_base_oracle_atomic = oracle_month
        .and_then(|month| {
            string_at_key(
                month,
                &[
                    "settlementBaseOracleAtomic",
                    "settlement_base_oracle_atomic",
                ],
            )
        })
        .unwrap_or_else(|| "-".to_string());
    let index_delta_bps = oracle_month
        .and_then(|month| string_at_key(month, &["indexDeltaBps", "index_delta_bps"]))
        .unwrap_or_else(|| "-".to_string());
    let spread_oracle_output =
        value_at_key(preflight, &["spreadOracleOutput", "spread_oracle_output"]);
    let output_price = spread_oracle_output
        .and_then(|output| {
            string_at_key(
                output,
                &["settlementPriceAtomic", "settlement_price_atomic"],
            )
        })
        .unwrap_or_else(|| "-".to_string());
    let output_compatible = spread_oracle_output
        .and_then(|output| {
            bool_at_key(
                output,
                &[
                    "settlementPackageCompatible",
                    "settlement_package_compatible",
                ],
            )
        })
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string());
    let source_count = oracle_month
        .and_then(|month| string_at_key(month, &["sourceCount", "source_count"]))
        .unwrap_or_else(|| "0".to_string());
    let frozen_source_count = oracle_month
        .and_then(|month| string_at_key(month, &["frozenSourceCount", "frozen_source_count"]))
        .unwrap_or_else(|| "0".to_string());
    let opened_source_count = oracle_month
        .and_then(|month| string_at_key(month, &["openedSourceCount", "opened_source_count"]))
        .unwrap_or_else(|| "0".to_string());
    let decoded_source_count = oracle_month
        .and_then(|month| array_at_key(month, &["sources"]))
        .map(|sources| sources.len().to_string())
        .unwrap_or_else(|| "0".to_string());
    let blockers = array_at_key(preflight, &["blockers"])
        .map(|items| render_string_array(items))
        .unwrap_or_else(|| "none".to_string());
    let issues = array_at_key(preflight, &["issues"])
        .map(|items| render_string_array(items))
        .unwrap_or_else(|| "none".to_string());

    render_terminal_lines([
        format!("oracle settlement check for {market_id}/{expiry_id}"),
        format!("backend={} | cluster={}", backend.base_url(), cli.cluster),
        format!(
            "status={status} | finalizable={finalizable} | packageReady={ready_for_package} | signingReady={ready_for_signing} | submissionReady={ready_for_submission}"
        ),
        format!("settlementUtc={settlement_utc} | fetchedAt={fetched_at}"),
        format!("finalityReason={finality_reason}"),
        format!(
            "oracleMonth={oracle_month_id} | marketPda={market_pda} | phase={oracle_phase} | recipeHash={recipe_hash}"
        ),
        format!(
            "baseOracleAtomic={settlement_base_oracle_atomic} | indexDeltaBps={index_delta_bps} | spreadPriceAtomic={output_price} | spreadPackageCompatible={output_compatible}"
        ),
        format!(
            "sources decoded={decoded_source_count} | registered={source_count} | frozen={frozen_source_count} | acceptedOpenings={opened_source_count}"
        ),
        format!(
            "packageSourceConfigured={package_source_configured} | signers={signer_endpoint_count}/{signer_expected_count}"
        ),
        format!("blockers={blockers}"),
        format!("issues={issues}"),
    ])
}

fn render_string_array(items: &[Value]) -> String {
    let rendered = items
        .iter()
        .filter_map(|item| match item {
            Value::String(text) => {
                let trimmed = text.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_string())
            }
            Value::Number(number) => Some(number.to_string()),
            Value::Bool(flag) => Some(flag.to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();
    if rendered.is_empty() {
        "none".to_string()
    } else {
        rendered.join(" | ")
    }
}

fn render_trade_ledger(
    cli: &Cli,
    _backend: &BackendClient,
    owner_pubkey: &str,
    payload: &Value,
) -> String {
    let chain_history = value_at_key(payload, &["chainHistory"]).unwrap_or(payload);
    let chain_available = solana_history::account_history_is_available(chain_history);
    let signatures = solana_history::account_history_signatures(chain_history);
    let product_payload =
        value_at_key(payload, &["productLedger"]).filter(|value| !value.is_null());
    let product_ledger = product_payload
        .map(unwrap_data)
        .map(|root| value_at_key(root, &["ledger"]).unwrap_or(root));
    let summaries = product_ledger.and_then(|ledger| array_at_key(ledger, &["summaries"]));
    let events = product_ledger.and_then(|ledger| array_at_key(ledger, &["events"]));
    let has_product_rows = summaries.map(|items| !items.is_empty()).unwrap_or(false)
        || events.map(|items| !items.is_empty()).unwrap_or(false);
    let amoeba_activity =
        value_at_key(payload, &["amoebaActivity"]).filter(|value| !value.is_null());
    let amoeba_matches = amoeba_activity.and_then(|activity| array_at_key(activity, &["matches"]));
    let has_amoeba_matches = amoeba_matches
        .map(|items| !items.is_empty())
        .unwrap_or(false);
    let account_empty = chain_available
        && solana_history::account_history_is_empty(chain_history)
        && !has_amoeba_matches
        && !has_product_rows;

    let mut lines = if !chain_available && !has_product_rows {
        vec![
            format!("Account: {}", short_pubkey(owner_pubkey)),
            "Wallet activity is temporarily unavailable.".to_string(),
            "Try refreshing again shortly.".to_string(),
        ]
    } else if account_empty {
        let mut lines = terminal_brand::empty_word_lines();
        lines.push(format!("Account: {}", short_pubkey(owner_pubkey)));
        lines.push("No wallet transaction history was found for this account.".to_string());
        lines
    } else {
        vec![format!(
            "Wallet ledger for {} | cluster {}",
            short_pubkey(owner_pubkey),
            cli.cluster
        )]
    };

    if !account_empty && (chain_available || has_product_rows || has_amoeba_matches) {
        lines.push("".to_string());
        push_amoeba_activity_plain_lines(
            &mut lines,
            amoeba_activity,
            product_ledger,
            summaries,
            events,
        );
        lines.push("".to_string());
        push_recent_wallet_plain_lines(&mut lines, chain_history, &signatures);
    }

    push_ledger_issue_plain_lines(&mut lines, payload);

    lines.join("\n")
}

fn push_amoeba_activity_plain_lines(
    lines: &mut Vec<String>,
    amoeba_activity: Option<&Value>,
    product_ledger: Option<&Value>,
    summaries: Option<&Vec<Value>>,
    events: Option<&Vec<Value>>,
) {
    lines.push("Amoeba wallet activity".to_string());
    let matches = amoeba_activity.and_then(|activity| array_at_key(activity, &["matches"]));
    let match_count = matches.map(|items| items.len()).unwrap_or(0);
    let scanned = amoeba_activity
        .and_then(|activity| string_at_key(activity, &["scannedTransactionCount"]))
        .map(|value| short_text(&value, 24))
        .unwrap_or_else(|| "0".to_string());
    let program_count = amoeba_activity
        .and_then(|activity| string_at_key(activity, &["programCount"]))
        .map(|value| short_text(&value, 24))
        .unwrap_or_else(|| "1".to_string());
    if match_count == 0 {
        lines.push("No matching recent wallet transactions found.".to_string());
        lines.push(format!(
            "Checked {scanned} recent transactions against {program_count} Amoeba programs."
        ));
    } else {
        lines.push(format!(
            "{} matching transaction{} found",
            match_count,
            if match_count == 1 { "" } else { "s" }
        ));
        if let Some(items) = matches {
            for (index, item) in items.iter().take(6).enumerate() {
                let signature =
                    string_at_key(item, &["signature"]).unwrap_or_else(|| "-".to_string());
                let block_time = string_at_key(item, &["blockTime", "block_time"])
                    .map(|value| short_text(&solana_history::format_ledger_timestamp(&value), 64))
                    .unwrap_or_else(|| "-".to_string());
                let status = string_at_key(item, &["confirmationStatus", "confirmation_status"])
                    .map(|value| short_text(&value, 32))
                    .unwrap_or_else(|| "-".to_string());
                let label = if index == 0 { "Latest" } else { "Recent" };
                lines.push(format!(
                    "{label}: {block_time} | {status} | {} | {}",
                    short_text(&signature, 28),
                    matched_program_labels(item)
                ));
            }
        }
    }

    if let Some(ledger) = product_ledger {
        let generated_at = string_at_key(ledger, &["generatedAt", "generated_at"])
            .map(|value| short_text(&solana_history::format_ledger_timestamp(&value), 64))
            .unwrap_or_else(|| "-".to_string());
        let summary_count = summaries.map(|items| items.len()).unwrap_or(0);
        let event_count = events.map(|items| items.len()).unwrap_or(0);
        if summary_count > 0 || event_count > 0 {
            lines.push("".to_string());
            lines.push("Indexed Amoeba history".to_string());
            lines.push(format!("Last updated {generated_at}"));
            lines.push(format!(
                "Recent markets {summary_count} | recent events {event_count}"
            ));
        }

        if let Some(totals) = value_at_key(ledger, &["totals"]) {
            if !totals.is_null() {
                lines.push(
                    "Totals are available in diagnostics only because token units could not be converted safely."
                        .to_string(),
                );
            }
        }

        if let Some(items) = summaries.filter(|items| !items.is_empty()) {
            lines.push("Recent markets:".to_string());
            for summary in items.iter().take(8) {
                let market = string_at_key(summary, &["marketId", "market_id"])
                    .map(|value| short_text(&value, 64))
                    .unwrap_or_else(|| "-".to_string());
                let expiry = string_at_key(summary, &["expiryId", "expiry_id"])
                    .map(|value| short_text(&value, 64))
                    .unwrap_or_else(|| "-".to_string());
                let event_count = string_at_key(summary, &["eventCount", "event_count"])
                    .map(|value| short_text(&value, 24))
                    .unwrap_or_else(|| "0".to_string());
                lines.push(format!("{market}/{expiry} | {event_count} events"));
            }
        }

        if let Some(items) = events.filter(|items| !items.is_empty()) {
            lines.push("Recent events:".to_string());
            for event in items.iter().take(12) {
                let occurred = string_at_key(
                    event,
                    &["occurredAt", "occurred_at", "blockTime", "block_time"],
                )
                .map(|value| short_text(&solana_history::format_ledger_timestamp(&value), 64))
                .unwrap_or_else(|| "-".to_string());
                let event_type = string_at_key(event, &["eventType", "event_type"])
                    .map(|value| short_text(&value, 48))
                    .unwrap_or_else(|| "-".to_string());
                let market = string_at_key(event, &["marketId", "market_id"])
                    .map(|value| short_text(&value, 64))
                    .unwrap_or_else(|| "-".to_string());
                let expiry = string_at_key(event, &["expiryId", "expiry_id"])
                    .map(|value| short_text(&value, 64))
                    .unwrap_or_else(|| "-".to_string());
                lines.push(format!("{occurred} | {event_type} | {market}/{expiry}"));
            }
        }
    }
}

fn push_recent_wallet_plain_lines(
    lines: &mut Vec<String>,
    chain_history: &Value,
    signatures: &[Value],
) {
    lines.push("Recent wallet activity".to_string());
    if !solana_history::account_history_is_available(chain_history) {
        lines.push("Wallet activity is temporarily unavailable.".to_string());
        return;
    }
    lines.push(format!("{} transactions found", signatures.len()));
    if signatures.is_empty() {
        lines.push("No recent wallet transactions found.".to_string());
        return;
    }
    for (index, item) in signatures.iter().take(8).enumerate() {
        let signature = string_at_key(item, &["signature"]).unwrap_or_else(|| "-".to_string());
        let block_time = string_at_key(item, &["blockTime", "block_time"])
            .map(|value| short_text(&solana_history::format_ledger_timestamp(&value), 64))
            .unwrap_or_else(|| "-".to_string());
        let status = string_at_key(item, &["confirmationStatus", "confirmation_status"])
            .map(|value| short_text(&value, 32))
            .unwrap_or_else(|| "-".to_string());
        let issue = value_at_key(item, &["err"])
            .filter(|value| !value.is_null())
            .map(|value| value.to_string())
            .filter(|value| value != "ok")
            .map(|value| format!(" | issue {}", short_text(&value, 24)))
            .unwrap_or_default();
        let label = if index == 0 { "Latest" } else { "Recent" };
        lines.push(format!(
            "{label}: {block_time} | {status} | {}{issue}",
            short_text(&signature, 28)
        ));
    }
}

fn push_ledger_issue_plain_lines(lines: &mut Vec<String>, payload: &Value) {
    if let Some(issues) = array_at_key(payload, &["issues"]).filter(|items| !items.is_empty()) {
        lines.push("".to_string());
        for issue in issues {
            let label =
                solana_history::user_facing_ledger_issue(issue.as_str().unwrap_or("ledger issue"));
            lines.push(format!("note: {}", short_text(&label, 256)));
        }
    }
}

fn matched_program_labels(item: &Value) -> String {
    array_at_key(item, &["matchedPrograms", "matched_programs"])
        .map(|programs| {
            programs
                .iter()
                .filter_map(|program| string_at_key(program, &["label", "name", "programId"]))
                .take(8)
                .map(|label| short_text(&label, 64))
                .collect::<Vec<_>>()
        })
        .filter(|labels| !labels.is_empty())
        .map(|labels| short_text(&labels.join(", "), 256))
        .unwrap_or_else(|| "Amoeba".to_string())
}

const MAX_PLAIN_OUTPUT_LINE_CHARS: usize = 1024;

fn render_terminal_lines(lines: impl IntoIterator<Item = String>) -> String {
    lines
        .into_iter()
        .map(|line| short_text(&line, MAX_PLAIN_OUTPUT_LINE_CHARS))
        .collect::<Vec<_>>()
        .join("\n")
}

fn short_text(value: &str, max_chars: usize) -> String {
    let safe = terminal_safe_text(value);
    let value = safe.as_str();
    let char_count = value.chars().count();
    if char_count <= max_chars || max_chars < 12 {
        return value.to_string();
    }
    let head_len = (max_chars - 3) / 2;
    let tail_len = max_chars - 3 - head_len;
    let head = value.chars().take(head_len).collect::<String>();
    let tail = value
        .chars()
        .rev()
        .take(tail_len)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("{head}...{tail}")
}

fn value_at_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    Some(current)
}
