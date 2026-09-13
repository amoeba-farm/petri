//! Background fetch and submission jobs plus user-safe result normalization.

use super::*;

pub(super) fn spawn_market_list_fetch(
    backend_url: String,
    fetch_tx: Sender<LabFetchResult>,
    request_id: u64,
) {
    thread::spawn(move || {
        let result = BackendClient::new(backend_url)
            .and_then(|backend| dish_list_payload(&backend))
            .map_err(|error| error.to_string());
        let _ = fetch_tx.send(LabFetchResult::MarketList { request_id, result });
    });
}

pub(super) fn spawn_detail_fetch(
    backend_url: String,
    fetch_tx: Sender<LabFetchResult>,
    request_id: u64,
    market_id: String,
) {
    thread::spawn(move || {
        let requested_market = market_id.clone();
        let result = BackendClient::new(backend_url)
            .and_then(|backend| live_dish_snapshot_payload(&backend, &requested_market))
            .map(|payload| detail_from_payload(&requested_market, &payload))
            .map_err(|error| error.to_string());
        let _ = fetch_tx.send(LabFetchResult::Detail {
            request_id,
            market_id,
            result,
        });
    });
}

pub(super) fn spawn_settlement_fetch(
    backend_url: String,
    fetch_tx: Sender<LabFetchResult>,
    request_id: u64,
    market_id: String,
    expiry_id: String,
) {
    thread::spawn(move || {
        let bundle = settlement_data::load_settlement_bundle(&backend_url, &market_id, &expiry_id);
        let _ = fetch_tx.send(LabFetchResult::Settlement {
            request_id,
            market_id,
            expiry_id,
            bundle,
        });
    });
}

pub(super) fn spawn_chart_fetch(
    backend_url: String,
    fetch_tx: Sender<LabFetchResult>,
    request_id: u64,
    key: String,
    market_id: String,
    month_label: String,
    args: ChartArgs,
) {
    thread::spawn(move || {
        let result = BackendClient::new(backend_url)
            .and_then(|backend| chart::fetch_embedded_chart(&backend, &args))
            .map_err(|error| error.to_string());
        let _ = fetch_tx.send(LabFetchResult::Chart {
            request_id,
            key,
            market_id,
            month_label,
            result,
        });
    });
}

pub(super) fn spawn_ledger_fetch(
    backend_url: String,
    onchain_config: OnchainConfig,
    fetch_tx: Sender<LabFetchResult>,
    request_id: u64,
    owner_pubkey: String,
) {
    thread::spawn(move || {
        let owner = owner_pubkey.clone();
        let (chain_history, mut issues) =
            match solana_history::fetch_account_history(&onchain_config, &owner, 50) {
                Ok(history) => (history, Vec::new()),
                Err(_) => (
                    solana_history::unavailable_account_history(&owner, 50),
                    vec![solana_history::wallet_activity_unavailable_issue()],
                ),
            };
        let backend = BackendClient::new(backend_url).ok();
        let owner_key = Pubkey::from_str(&owner).ok();
        let current_writer_sleeves = match backend
            .as_ref()
            .ok_or_else(|| CliError::new("Collective writer sleeves are temporarily unavailable."))
            .and_then(|backend| {
                backend
                    .get(&endpoints::writer_sleeves(None))
                    .and_then(|response| {
                        crate::current_trade_response_data(response, "writer sleeves")
                    })
            }) {
            Ok(payload) => Some(payload),
            Err(_) => {
                issues.push("Collective writer sleeves are temporarily unavailable.".to_string());
                None
            }
        };
        let liquidity_positions = match backend
            .as_ref()
            .ok_or_else(|| {
                CliError::new("Manager liquidity positions are temporarily unavailable.")
            })
            .and_then(|backend| {
                backend
                    .get(&endpoints::dlmm_positions(&owner))
                    .and_then(crate::backend::current_backend_payload)
                    .and_then(|response| positions::project_liquidity_positions(&response, &owner))
            }) {
            Ok(payload) => Some(payload),
            Err(_) => {
                issues.push(
                    "Manager liquidity positions are unavailable; no position state was inferred."
                        .to_string(),
                );
                None
            }
        };
        let current_collateral = match backend
            .as_ref()
            .zip(owner_key.as_ref())
            .ok_or_else(|| CliError::new("Current collateral is temporarily unavailable."))
            .and_then(|(backend, owner_key)| {
                backend
                    .get(&endpoints::user_collateral(&owner))
                    .and_then(|response| crate::current_trade_response_data(response, "collateral"))
                    .and_then(|data| {
                        crate::validate_current_collateral_payload(&data, owner_key)?;
                        Ok(data)
                    })
            }) {
            Ok(payload) => Some(payload),
            Err(_) => {
                issues.push("Current collateral is temporarily unavailable.".to_string());
                None
            }
        };
        let normalized_program_registry = solana_history::normalize_amoeba_program_registry(None);
        let amoeba_activity =
            solana_history::scan_amoeba_activity(&onchain_config, &chain_history, None, 50);
        let product_ledger = match backend
            .as_ref()
            .ok_or_else(|| CliError::new("Amoeba trade history is temporarily unavailable."))
            .and_then(|backend| backend.get(&endpoints::user_ledger(&owner, 50)))
            .and_then(|payload| {
                crate::chain_identity::validate_current_backend_envelope(&payload)?;
                Ok(payload)
            }) {
            Ok(payload) => Some(payload),
            Err(_) => {
                issues.push(solana_history::trade_history_unavailable_issue());
                None
            }
        };
        let amba_mint = wallet_balance::resolve_wallet_amba_mint(None);
        let wallet_balance = match wallet_balance::resolve_wallet_usdc_mint(
            &onchain_config.network,
            None,
        )
        .and_then(|usdc_mint| {
            wallet_balance::read_wallet_balance(&onchain_config, &owner, &usdc_mint, &amba_mint)
        }) {
            Ok(payload) => Some(payload),
            Err(_) => {
                issues.push("Wallet balances are temporarily unavailable.".to_string());
                None
            }
        };
        let mut payload = solana_history::build_account_ledger_payload_with_activity(
            &owner,
            chain_history,
            product_ledger,
            Some(amoeba_activity),
            Some(normalized_program_registry),
            issues,
        );
        if let Some(balance) = wallet_balance
            && let Some(object) = payload.as_object_mut()
        {
            object.insert("walletBalance".to_string(), balance);
        }
        if let Some(sleeves) = current_writer_sleeves
            && let Some(object) = payload.as_object_mut()
        {
            object.insert("currentWriterSleeves".to_string(), sleeves);
        }
        if let Some(positions) = liquidity_positions
            && let Some(object) = payload.as_object_mut()
        {
            object.insert("liquidityPositions".to_string(), positions);
        }
        if let Some(collateral) = current_collateral
            && let Some(object) = payload.as_object_mut()
        {
            object.insert("currentCollateral".to_string(), collateral);
        }
        let result = Ok(payload);
        let _ = fetch_tx.send(LabFetchResult::Ledger {
            request_id,
            owner_pubkey,
            result,
        });
    });
}

pub(super) fn spawn_staking_status_fetch(
    onchain_config: OnchainConfig,
    fetch_tx: Sender<LabFetchResult>,
    request_id: u64,
    owner_pubkey: String,
) {
    thread::spawn(move || {
        let owner = owner_pubkey.clone();
        let result = crate::staking::status(&onchain_config, Some(&owner))
            .and_then(|status| {
                serde_json::to_value(status).map_err(|error| {
                    CliError::new(format!("could not display staking status: {error}"))
                })
            })
            .map_err(|error| user_safe_staking_failure_reason(&error.to_string()));
        let _ = fetch_tx.send(LabFetchResult::StakingStatus {
            request_id,
            owner_pubkey,
            result,
        });
    });
}

pub(super) fn spawn_staking_action(
    onchain_config: OnchainConfig,
    fetch_tx: Sender<LabFetchResult>,
    request_id: u64,
    confirmation: StakingConfirmation,
) {
    thread::spawn(move || {
        let action = confirmation.action;
        let result = match action {
            StakingAction::Stake => crate::staking::stake(
                &onchain_config,
                confirmation.amount.as_deref().unwrap_or_default(),
            ),
            StakingAction::Activate => {
                crate::staking::activate(&onchain_config, confirmation.minimum_received.as_deref())
            }
            StakingAction::CancelQueue => crate::staking::cancel(&onchain_config),
            StakingAction::Unstake => crate::staking::unstake(
                &onchain_config,
                confirmation.amount.as_deref().unwrap_or_default(),
                confirmation.minimum_received.as_deref(),
            ),
            StakingAction::Claim => crate::staking::claim(&onchain_config),
            StakingAction::Refresh => unreachable!("refresh uses the status fetch"),
        }
        .and_then(|transaction| {
            serde_json::to_value(transaction).map_err(|error| {
                CliError::new(format!("could not display staking result: {error}"))
            })
        })
        .map(|payload| {
            let message = staking_action_success_message(action, &payload);
            (payload, message)
        })
        .map_err(|error| user_safe_staking_failure_reason(&error.to_string()));
        let _ = fetch_tx.send(LabFetchResult::StakingAction {
            request_id,
            action,
            result,
        });
    });
}

pub(super) fn spawn_trade_submit(
    args: Vec<String>,
    envs: Vec<(String, String)>,
    fetch_tx: Sender<LabFetchResult>,
    request_id: u64,
    action: TradeAction,
    summary: TradeConfirmationSummary,
    command: String,
) {
    thread::spawn(move || {
        let result = env::current_exe()
            .map_err(|error| format!("could not find petri executable: {error}"))
            .and_then(|exe| {
                ProcessCommand::new(exe)
                    .args(&args)
                    .envs(envs)
                    .output()
                    .map_err(|error| format!("could not start order submit: {error}"))
            })
            .and_then(|output| {
                let id = args.iter().position(|a| a=="execute").and_then(|i|args.get(i+1)).map(String::as_str).unwrap_or("unknown");
                let payload = serde_json::from_slice::<Value>(&output.stdout).ok();
                if output.status.success() {
                    let payload = payload.filter(|v| v["ok"]==true && v.pointer("/receipt/operationId").and_then(Value::as_str)==Some(id)
                        && v.pointer("/receipt/signature").and_then(Value::as_str).is_some_and(|s|!s.is_empty()))
                        .ok_or_else(||format!("Receipt missing or mismatched. Use petri operations resume {id}; do not resubmit."))?;
                    Ok(crate::trade_service::render_response(&payload))
                } else {
                    let message = payload.as_ref().and_then(|v|v.pointer("/error/message")).and_then(Value::as_str);
                    let pending = payload.as_ref().and_then(|v|v.pointer("/error/category")).and_then(Value::as_str)==Some("pending");
                    Err(match message {
                        Some(message) if pending => format!("{message}\nUse petri operations resume {id}; do not resubmit."),
                        Some(message) => format!("{message}\nOperation {id}; inspect its record with F8."),
                        None => format!("Trade process ended without a qualified receipt ({}). Use petri operations resume {id}; do not resubmit.",output.status),
                    })
                }
            });
        let _ = fetch_tx.send(LabFetchResult::TradeSubmit {
            request_id,
            action,
            summary,
            command,
            result,
        });
    });
}

pub(super) fn spawn_trade_prepare(
    submit: TradeTicketSubmit,
    fetch_tx: Sender<LabFetchResult>,
    request_id: u64,
    owner: String,
    expiry: String,
) {
    thread::spawn(move || {
        let result = env::current_exe()
            .map_err(|_| "Could not locate Petri.".to_string())
            .and_then(|exe| {
                ProcessCommand::new(exe)
                    .args(&submit.args)
                    .envs(submit.envs.iter().cloned())
                    .output()
                    .map_err(|e| e.to_string())
            })
            .and_then(|output| {
                if !output.status.success() {
                    let parsed = serde_json::from_slice::<Value>(&output.stdout).ok();
                    return Err(parsed
                        .as_ref()
                        .and_then(|v| v.pointer("/error/message"))
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .unwrap_or_else(|| compact_process_output(&output.stderr)));
                }
                serde_json::from_slice(&output.stdout)
                    .map_err(|_| "The prepared trade response could not be read.".to_string())
            });
        let _ = fetch_tx.send(LabFetchResult::TradePrepare {
            request_id,
            owner,
            expiry,
            submit,
            result,
        });
    });
}

pub(super) fn spawn_writer_command(
    args: Vec<String>,
    envs: Vec<(String, String)>,
    fetch_tx: Sender<LabFetchResult>,
    request_id: u64,
    action: WriterAction,
) {
    thread::spawn(move || {
        let result = env::current_exe()
            .map_err(|error| format!("could not find petri executable: {error}"))
            .and_then(|exe| {
                ProcessCommand::new(exe)
                    .args(&args)
                    .envs(envs)
                    .output()
                    .map_err(|error| format!("could not start writer action: {error}"))
            })
            .and_then(|output| {
                if output.status.success() {
                    serde_json::from_slice::<Value>(&output.stdout)
                        .map_err(|_| "Petri returned an unreadable writer result.".to_string())
                } else {
                    let stderr = compact_process_output(&output.stderr);
                    let stdout = compact_process_output(&output.stdout);
                    let raw = if stderr.is_empty() { stdout } else { stderr };
                    Err(user_safe_writer_failure_reason(&raw))
                }
            });
        let _ = fetch_tx.send(LabFetchResult::WriterCommand {
            request_id,
            action,
            result,
        });
    });
}

pub(super) fn spawn_writer_capabilities_fetch(
    backend_url: String,
    fetch_tx: Sender<LabFetchResult>,
    request_id: u64,
) {
    thread::spawn(move || {
        let result = (|| {
            let backend = BackendClient::new(backend_url)?;
            let response = backend.get(endpoints::capabilities())?;
            let payload = crate::current_trade_response_data(response, "release capabilities")?;
            writers::parse_writer_close_capabilities(&payload).map_err(CliError::new)
        })()
        .map_err(|_| {
            "Current writer-close availability could not be verified. Refresh Writers; nothing will be signed or sent."
                .to_string()
        });
        let _ = fetch_tx.send(LabFetchResult::WriterCapabilities { request_id, result });
    });
}

pub(super) fn spawn_writer_action_mask_fetch(
    backend_url: String,
    fetch_tx: Sender<LabFetchResult>,
    request_id: u64,
    action: WriterAction,
    owner: String,
    sleeve: String,
) {
    thread::spawn(move || {
        let result = (|| {
            let backend = BackendClient::new(backend_url)?;
            let response = backend.get(&endpoints::writer_available_actions(&sleeve, &owner))?;
            crate::writer_action_mask::validate_current_writer_action_mask(
                &response, &owner, &sleeve,
            )
            .map_err(CliError::new)
        })()
        .map_err(|error| {
            format!(
                "Current wallet-specific writer availability could not be verified: {error}. Nothing was signed or sent."
            )
        });
        let _ = fetch_tx.send(LabFetchResult::WriterActionMask {
            request_id,
            action,
            owner,
            sleeve,
            result,
        });
    });
}

pub(super) fn spawn_liquidity_preview(
    args: Vec<String>,
    envs: Vec<(String, String)>,
    binding: liquidity::LiquidityPreviewBinding,
    fetch_tx: Sender<LabFetchResult>,
    request_id: u64,
) {
    thread::spawn(move || {
        let result = env::current_exe()
            .map_err(|_| "Petri could not find its liquidity preview command.".to_string())
            .and_then(|exe| {
                ProcessCommand::new(exe)
                    .args(&args)
                    .envs(envs)
                    .output()
                    .map_err(|_| "Petri could not start the liquidity preview command.".to_string())
            })
            .and_then(|output| {
                if output.status.success() {
                    serde_json::from_slice::<Value>(&output.stdout)
                        .map_err(|_| "Petri returned an unreadable liquidity preview.".to_string())
                } else {
                    let stderr = compact_process_output(&output.stderr);
                    let stdout = compact_process_output(&output.stdout);
                    let raw = if stderr.is_empty() { stdout } else { stderr };
                    Err(liquidity::user_safe_liquidity_preview_failure(&raw))
                }
            })
            .and_then(|payload| {
                liquidity::validate_liquidity_preview_response(&payload, &binding)?;
                Ok(payload)
            });
        let _ = fetch_tx.send(LabFetchResult::LiquidityPreview { request_id, result });
    });
}

pub(super) fn user_safe_writer_failure_reason(raw: &str) -> String {
    let normalized = raw.to_ascii_lowercase();
    if normalized.contains("insufficient")
        || normalized.contains("balance")
        || normalized.contains("funds")
    {
        "The wallet does not have enough available balance for this writer action.".to_string()
    } else if normalized.contains("no legal writer close operation")
        || normalized.contains("writer close stage changed")
        || normalized.contains("close request deadline expired")
    {
        "That close request has no permitted step for this wallet right now. Refresh Close status before trying again; nothing was signed or sent."
            .to_string()
    } else if normalized.contains("phase")
        || normalized.contains("window")
        || normalized.contains("not permitted")
        || normalized.contains("not ready")
    {
        "The selected sleeve is not in a phase that permits this action.".to_string()
    } else if normalized.contains("identity")
        || normalized.contains("deployment")
        || normalized.contains("genesis")
        || normalized.contains("program")
    {
        "Petri could not verify the current Amoeba deployment, so nothing was signed or sent."
            .to_string()
    } else if normalized.contains("timeout")
        || normalized.contains("network")
        || normalized.contains("connection")
        || normalized.contains("unavailable")
        || normalized.contains("rpc")
    {
        "The current writer service or chain connection is unavailable. Nothing was assumed."
            .to_string()
    } else {
        "Petri could not complete the writer action. Check wallet history before retrying; no successful result was assumed."
            .to_string()
    }
}

pub(super) fn compact_process_output(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let compact = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" | ");
    truncate_text(&compact, 240)
}

pub(super) fn user_safe_trade_failure_reason(raw: &str) -> &'static str {
    let normalized = raw.to_ascii_lowercase();
    if normalized.contains("insufficient")
        || normalized.contains("balance")
        || normalized.contains("funds")
        || normalized.contains("liquidity")
    {
        "The wallet or pool reported insufficient funds or liquidity."
    } else if normalized.contains("slippage")
        || normalized.contains("price")
        || normalized.contains("quote")
        || normalized.contains("depth")
        || normalized.contains("route")
    {
        "The quote or available depth changed before submission."
    } else if normalized.contains("timeout")
        || normalized.contains("timed out")
        || normalized.contains("network")
        || normalized.contains("connection")
        || normalized.contains("rpc")
        || normalized.contains("service")
        || normalized.contains("unavailable")
    {
        "The transaction service was unavailable or timed out."
    } else if normalized.contains("wallet")
        || normalized.contains("signature")
        || normalized.contains("authorization")
        || normalized.contains("unauthorized")
        || normalized.contains("approval")
        || normalized.contains("rejected")
        || normalized.contains("cancelled")
        || normalized.contains("canceled")
    {
        "Wallet approval or authorization did not complete."
    } else if normalized.contains("invalid") || normalized.contains("expired") {
        "The prepared order was no longer valid."
    } else {
        "The order could not be submitted."
    }
}

pub(super) fn user_safe_staking_failure_reason(raw: &str) -> String {
    let normalized = raw.to_ascii_lowercase();
    if normalized.contains("not wired")
        || normalized.contains("route")
        || normalized.contains("unavailable")
        || normalized.contains("not available")
        || normalized.contains("not found")
    {
        "Staking is not available from this Petri connection yet. No balance was changed."
            .to_string()
    } else if normalized.contains("temporarily paused")
        || normalized.contains("protocol paused")
        || normalized.contains("contract paused")
    {
        "Staking actions are temporarily paused for this Amoeba deployment. No balance was changed."
            .to_string()
    } else if normalized.contains("governance")
        || normalized.contains("supply changes are locked")
        || normalized.contains("emergency checkpoint")
    {
        "Staking changes are paused while an emergency vote is unresolved. No balance was changed."
            .to_string()
    } else if normalized.contains("staking activation wait")
        || normalized.contains("queued amba") && normalized.contains("seven-day")
    {
        "This AMBA is still in its seven-day staking activation wait. No balance was changed."
            .to_string()
    } else if normalized.contains("reward intake")
        || normalized.contains("pending amba rewards")
        || normalized.contains("pending rewards")
    {
        "Activation is waiting for pending rewards to be included in the sAMBA share rate. No balance was changed."
            .to_string()
    } else if normalized.contains("seven-day")
        || normalized.contains("unbonding period")
        || normalized.contains("not claimable")
    {
        "This AMBA is still in its seven-day unstaking wait. No balance was changed.".to_string()
    } else if normalized.contains("insufficient") || normalized.contains("balance") {
        "The attached wallet does not have enough available tokens for this staking action. No balance was changed."
            .to_string()
    } else if normalized.contains("wallet")
        || normalized.contains("keypair")
        || normalized.contains("signer")
    {
        "Attach a wallet before changing staking balances.".to_string()
    } else if normalized.contains("identity") || normalized.contains("deployment") {
        "Petri could not verify the selected Amoeba deployment, so it did not sign or send anything."
            .to_string()
    } else {
        "The staking action could not be completed. No balance was changed.".to_string()
    }
}

pub(super) fn staking_action_success_message(action: StakingAction, payload: &Value) -> String {
    if let Some(message) = string_at_key(payload, &["summary", "message", "status"])
        .filter(|message| !message.trim().is_empty())
    {
        return truncate_text(&message, 180);
    }
    match action {
        StakingAction::Stake => "AMBA queued. It can be activated after seven days.".to_string(),
        StakingAction::Activate => {
            "Queued AMBA activated and sAMBA received. Refreshing balances...".to_string()
        }
        StakingAction::CancelQueue => {
            "Queued AMBA returned to the available balance. Refreshing balances...".to_string()
        }
        StakingAction::Unstake => {
            "Unstaking started. The fixed AMBA amount is available after seven days.".to_string()
        }
        StakingAction::Claim => {
            "Ready AMBA moved into your available balance. Refreshing staking balances..."
                .to_string()
        }
        StakingAction::Refresh => "Staking balances refreshed.".to_string(),
    }
}

pub(super) fn staking_status_root(payload: &Value) -> &Value {
    let mut current = payload;
    for _ in 0..4 {
        let Some(next) = value_at_key(
            current,
            &[
                "staking",
                "stakingStatus",
                "staking_status",
                "data",
                "result",
            ],
        )
        .filter(|value| value.is_object()) else {
            break;
        };
        current = next;
    }
    current
}

pub(super) fn staking_status_value(payload: Option<&Value>, keys: &[&str]) -> Option<String> {
    payload
        .map(staking_status_root)
        .and_then(|status| string_at_key(status, keys))
        .filter(|value| !value.trim().is_empty())
}

pub(super) fn staking_status_number(payload: &Value, keys: &[&str]) -> Option<f64> {
    value_at_key(staking_status_root(payload), keys).and_then(number_from_value)
}

pub(super) fn staking_status_flag(payload: &Value, keys: &[&str]) -> bool {
    let Some(value) = value_at_key(staking_status_root(payload), keys) else {
        return false;
    };
    match value {
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|number| number != 0.0),
        Value::String(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "true" | "yes" | "locked" | "ready" | "1"
        ),
        _ => false,
    }
}

pub(super) fn staking_status_is_available(payload: &Value) -> bool {
    value_at_key(staking_status_root(payload), &["available"])
        .and_then(|value| match value {
            Value::Bool(flag) => Some(*flag),
            Value::String(value) => value.trim().parse::<bool>().ok(),
            _ => None,
        })
        .unwrap_or(false)
}

pub(super) fn staking_availability_note(payload: &Value) -> String {
    staking_status_value(Some(payload), &["availabilityNote", "availability_note"])
        .map(|note| truncate_text(note.trim(), 180))
        .unwrap_or_else(|| {
            "Staking is not available for this verified deployment. No balance was changed."
                .to_string()
        })
}

pub(super) fn staking_status_value_is_positive(payload: &Value, keys: &[&str]) -> bool {
    staking_status_number(payload, keys).is_some_and(|value| value.is_finite() && value > 0.0)
        || staking_status_value(Some(payload), keys).is_some_and(|value| {
            let value = value.trim().trim_start_matches('+');
            !value.starts_with('-')
                && value
                    .chars()
                    .any(|character| matches!(character, '1'..='9'))
        })
}

pub(super) fn staking_status_u64(payload: &Value, keys: &[&str]) -> Option<u64> {
    value_at_key(staking_status_root(payload), keys).and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_str().and_then(|value| value.trim().parse().ok()))
    })
}

pub(super) fn format_staking_countdown(seconds: u64) -> String {
    if seconds == 0 {
        return "ready now".to_string();
    }
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m")
    } else {
        format!("{seconds}s")
    }
}

// Retained as a deterministic status formatter while staking mutations are
// statically unavailable in the current release.
#[allow(dead_code)]
pub(super) fn staking_claim_wait_message(status: &Value) -> String {
    if !staking_status_value_is_positive(
        status,
        &[
            "pendingUnstakeAmba",
            "pending_unstake_amba",
            "ownerPendingUnstakeAmba",
            "owner_pending_unstake_amba",
            "unbondingAmba",
            "unbonding_amba",
        ],
    ) {
        return "No unstaking claim is waiting.".to_string();
    }
    if let Some(seconds) = staking_status_u64(
        status,
        &[
            "secondsUntilUnstakeReady",
            "seconds_until_unstake_ready",
            "secondsUntilClaimable",
            "seconds_until_claimable",
            "unstakeSecondsRemaining",
        ],
    )
    .filter(|seconds| *seconds > 0)
    {
        return format!(
            "This AMBA is still in its seven-day unstaking wait ({} remaining).",
            format_staking_countdown(seconds)
        );
    }
    if let Some(claimable_at) = staking_status_value(
        Some(status),
        &[
            "unstakeClaimableAt",
            "unstakeClaimableAtTs",
            "unstake_claimable_at",
            "unstake_claimable_at_ts",
            "claimableAtUnix",
            "claimable_at_unix",
        ],
    ) {
        return format!(
            "This AMBA is still in its seven-day unstaking wait. Ready at {claimable_at}."
        );
    }
    "This AMBA is still in its seven-day unstaking wait.".to_string()
}

// Retained as a deterministic status formatter while staking mutations are
// statically unavailable in the current release.
#[allow(dead_code)]
pub(super) fn staking_activation_wait_message(status: &Value) -> String {
    if !staking_status_value_is_positive(
        status,
        &[
            "queuedAmba",
            "queued_amba",
            "queuedAmbaAtoms",
            "queued_amba_atoms",
        ],
    ) {
        return "No AMBA is queued for activation.".to_string();
    }
    if let Some(seconds) = staking_status_u64(
        status,
        &["secondsUntilActivation", "seconds_until_activation"],
    )
    .filter(|seconds| *seconds > 0)
    {
        return format!(
            "Queued AMBA is still in its seven-day activation wait ({} remaining).",
            format_staking_countdown(seconds)
        );
    }
    staking_status_value(
        Some(status),
        &["activationBlockedReason", "activation_blocked_reason"],
    )
    .unwrap_or_else(|| "Queued AMBA is not ready to activate yet.".to_string())
}

pub(super) fn truncate_text(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    if max_chars <= 3 {
        return text.chars().take(max_chars).collect();
    }
    let mut truncated = text.chars().take(max_chars - 3).collect::<String>();
    truncated.push_str("...");
    truncated
}

pub(super) fn guide_markdown_block_text(block: &MarkdownBlock) -> Option<&str> {
    match block {
        MarkdownBlock::Paragraph(text)
        | MarkdownBlock::Bullet { text, .. }
        | MarkdownBlock::Numbered { text, .. }
        | MarkdownBlock::Quote(text) => Some(text.trim()).filter(|text| !text.is_empty()),
        MarkdownBlock::Heading { level, text } if *level > 1 => {
            Some(text.trim()).filter(|text| !text.is_empty())
        }
        MarkdownBlock::Heading { .. }
        | MarkdownBlock::Code(_)
        | MarkdownBlock::Rule
        | MarkdownBlock::Blank => None,
    }
}

pub(super) fn spawn_oracle_tree_fetch(
    backend_url: String,
    fetch_tx: Sender<LabFetchResult>,
    request_id: u64,
    market_id: String,
) {
    thread::spawn(move || {
        let requested_market = market_id.clone();
        let result = fetch_oracle_tree_from_api(&backend_url, &requested_market);
        let _ = fetch_tx.send(LabFetchResult::OracleTree {
            request_id,
            market_id,
            result,
        });
    });
}

pub(super) fn spawn_oracle_live_fetch(
    backend_url: String,
    fetch_tx: Sender<LabFetchResult>,
    request_id: u64,
    market_id: String,
    expiry_id: String,
) {
    thread::spawn(move || {
        let requested_market = market_id.clone();
        let requested_expiry = expiry_id.clone();
        let result = fetch_oracle_live_from_api(&backend_url, &requested_market, &requested_expiry);
        let _ = fetch_tx.send(LabFetchResult::OracleLive {
            request_id,
            market_id,
            expiry_id,
            result,
        });
    });
}

pub(super) fn spawn_oracle_rewards_fetch(
    backend_url: String,
    fetch_tx: Sender<LabFetchResult>,
    request_id: u64,
    market_id: String,
    expiry_id: String,
    owner_pubkey: String,
) {
    thread::spawn(move || {
        let requested_market = market_id.clone();
        let requested_expiry = expiry_id.clone();
        let requested_owner = owner_pubkey.clone();
        let result = fetch_oracle_rewards_from_api(
            &backend_url,
            &requested_market,
            &requested_expiry,
            &requested_owner,
        );
        let _ = fetch_tx.send(LabFetchResult::OracleRewards {
            request_id,
            market_id,
            expiry_id,
            owner_pubkey,
            result,
        });
    });
}

pub(super) fn spawn_update_check(fetch_tx: Sender<LabFetchResult>, request_id: u64) {
    thread::spawn(move || {
        let result = crate::update::check_for_tui(false)
            .map_err(|error| user_facing_update_check_error(&error.to_string()));
        let _ = fetch_tx.send(LabFetchResult::UpdateCheck { request_id, result });
    });
}

pub(super) fn spawn_help_index_fetch(
    docs_root: String,
    fetch_tx: Sender<LabFetchResult>,
    request_id: u64,
) {
    thread::spawn(move || {
        let result = gitbook::fetch_index(&docs_root);
        let _ = fetch_tx.send(LabFetchResult::HelpIndex { request_id, result });
    });
}

pub(super) fn spawn_help_page_fetch(
    link: gitbook::GitbookPageLink,
    fetch_tx: Sender<LabFetchResult>,
    request_id: u64,
) {
    thread::spawn(move || {
        let page_id = link.id.clone();
        let result = gitbook::fetch_page(&link);
        let _ = fetch_tx.send(LabFetchResult::HelpPage {
            request_id,
            page_id,
            result,
        });
    });
}

pub(super) fn spawn_help_preview_page_fetch(
    link: gitbook::GitbookPageLink,
    fetch_tx: Sender<LabFetchResult>,
    request_id: u64,
) {
    thread::spawn(move || {
        let page_id = link.id.clone();
        let result = gitbook::fetch_page(&link);
        let _ = fetch_tx.send(LabFetchResult::HelpPreviewPage {
            request_id,
            page_id,
            result,
        });
    });
}

pub(super) fn spawn_guide_probe(
    config: guide::GuideConfig,
    fetch_tx: Sender<LabFetchResult>,
    request_id: u64,
) {
    thread::spawn(move || {
        let status = guide::detect_provider(&config);
        let _ = fetch_tx.send(LabFetchResult::GuideProbe { request_id, status });
    });
}

pub(super) fn spawn_guide_request(
    connection: guide::GuideProviderConnection,
    request: guide::GuideRequest,
    provider_session_id: Option<String>,
    state_revision: String,
    fetch_tx: Sender<LabFetchResult>,
    request_id: u64,
) {
    thread::spawn(move || {
        let progress_tx = fetch_tx.clone();
        let mut progress = |event| {
            let _ = progress_tx.send(LabFetchResult::GuideProgress { request_id, event });
        };
        let result = guide::ask_with_provider(
            &connection,
            &request,
            provider_session_id.as_deref(),
            &mut progress,
        );
        let _ = fetch_tx.send(LabFetchResult::GuideReply {
            request_id,
            state_revision,
            result,
        });
    });
}

pub(super) fn env_update_check_enabled() -> bool {
    env::var(PETRI_UPDATE_CHECK_ENV)
        .map(|value| !env_flag_is_false(&value))
        .unwrap_or(true)
}

pub(super) fn env_flag_is_false(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "off" | "no"
    )
}

pub(super) fn user_facing_update_check_error(error: &str) -> String {
    let trimmed = error.trim();
    if trimmed.is_empty() {
        "Petri update check is unavailable.".to_string()
    } else {
        format!("Petri update check is unavailable: {trimmed}")
    }
}

pub(super) fn fetch_oracle_tree_from_api(
    backend_url: &str,
    market_id: &str,
) -> Result<OracleTreeFetch, String> {
    let market_id = market_id.trim();
    if market_id.is_empty() {
        return Err("oracle market id is required".to_string());
    }
    let load_error = || {
        format!(
            "Could not load {} oracle source recipe.",
            market_id.to_uppercase()
        )
    };
    let backend = BackendClient::new(backend_url).map_err(|_| load_error())?;
    let payload = backend
        .get(&endpoints::dlmm_oracle_market(market_id))
        .map_err(|error| oracle_tree_fetch_error_message(market_id, &error.to_string()))?;
    crate::chain_identity::validate_current_backend_envelope(&payload)
        .map_err(|error| oracle_tree_fetch_error_message(market_id, &error.to_string()))?;
    let tree = OracleIndexTree::from_payload(&payload)
        .map_err(CliError::new)
        .map_err(|_| load_error())?;
    Ok(OracleTreeFetch { tree, notice: None })
}

pub(super) fn fetch_oracle_live_from_api(
    backend_url: &str,
    market_id: &str,
    expiry_id: &str,
) -> Result<SpreadOracleLiveState, String> {
    let market_id = market_id.trim();
    if market_id.is_empty() {
        return Err("oracle market id is required".to_string());
    }
    let expiry_id = expiry_id.trim();
    if expiry_id.is_empty() {
        return Err("oracle expiry id is required".to_string());
    }
    let backend = BackendClient::new(backend_url)
        .map_err(|_| "Live oracle observations are temporarily unavailable.".to_string())?;
    let payload = backend
        .get(&endpoints::dlmm_oracle_market(market_id))
        .map_err(|_| "Live oracle observations are temporarily unavailable.".to_string())?;
    crate::chain_identity::validate_current_backend_envelope(&payload)
        .map_err(|_| "Live oracle observations are not from the current API.".to_string())?;
    spread_oracle_live_state_for_expiry_from_payload(market_id, expiry_id, &payload)
}

pub(super) fn fetch_oracle_rewards_from_api(
    backend_url: &str,
    market_id: &str,
    expiry_id: &str,
    owner_pubkey: &str,
) -> Result<SpreadOracleRewardState, String> {
    let owner_pubkey = owner_pubkey.trim();
    if owner_pubkey.is_empty() {
        return Err("Attach a wallet to check oracle rewards.".to_string());
    }
    let backend = BackendClient::new(backend_url)
        .map_err(|_| "Oracle reward availability is temporarily unavailable.".to_string())?;
    let payload = backend
        .get(endpoints::dlmm_oracle_state())
        .map_err(|_| "Oracle reward availability is temporarily unavailable.".to_string())?;
    crate::chain_identity::validate_current_backend_envelope(&payload)
        .map_err(|_| "Oracle rewards are not from the current API.".to_string())?;
    Ok(spread_oracle_reward_state_from_payload(
        market_id,
        expiry_id,
        owner_pubkey,
        &payload,
    ))
}

pub(super) fn oracle_tree_fetch_error_message(market_id: &str, error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if lower.contains("http 404") || lower.contains("no active oracle index definition") {
        return format!(
            "No active {} oracle source recipe is published yet.",
            market_id.trim().to_uppercase()
        );
    }
    format!(
        "Could not load {} oracle source recipe.",
        market_id.trim().to_uppercase()
    )
}

pub(super) fn oracle_tree_issue_is_retryable(issue: &str) -> bool {
    !issue
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("no active ")
}

pub(super) fn chart_cache_key(args: &ChartArgs) -> String {
    format!(
        "{}|{}|{}|{}",
        args.market.to_ascii_lowercase(),
        args.expiry.as_deref().unwrap_or("-"),
        args.range.label(),
        args.points
    )
}
