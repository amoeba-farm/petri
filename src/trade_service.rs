//! Human intent -> exact current Market -> SDK ticket -> shared governed executor.
//! No local quote/reserve formula; presentation cards are never signing authority.
use crate::{
    attached_wallet::attached_wallet_pubkey,
    backend::unwrap_data,
    request_validation::{canonical_pubkey_string, canonical_u64_string},
};
use crate::{
    backend::{BackendClient, CliError},
    cli::{Cli, CollectiveSwapDirectionValue, CollectiveTradeArgs},
    current_operation, market_surface,
};
use serde_json::{Value, json};
use solana_pubkey::Pubkey;
use std::str::FromStr;

fn collective_trade_request(cli: &Cli, trade: &CollectiveTradeArgs) -> Result<Value, CliError> {
    Ok(json!({
        "owner": attached_wallet_pubkey(cli)?,
        "market": canonical_pubkey_string(&trade.market, "market")?,
        "direction": trade.direction.as_request_value(),
        "amountIn": canonical_u64_string(&trade.amount_in, "amount-in", false)?,
        "minimumAmountOut": canonical_u64_string(
            &trade.minimum_amount_out,
            "minimum-amount-out",
            false,
        )?,
        "limitBinId": trade.limit_bin_id,
    }))
}

fn validate_collective_trade_response(
    response: &Value,
    request: &Value,
    context: &ameba_sdk::CurrentGovernedWriteContextV1,
) -> Result<ameba_sdk::CurrentGovernedOperationV1, CliError> {
    let data = unwrap_data(response);
    let plan = data
        .get("operationPlan")
        .or_else(|| data.get("plan"))
        .ok_or_else(|| CliError::new("collective trade response is missing operationPlan"))?;
    let encoded = serde_json::to_string(plan).map_err(|error| {
        CliError::new(format!(
            "collective trade operationPlan could not be encoded: {error}"
        ))
    })?;
    let admitted =
        ameba_sdk::parse_current_governed_collective_swap_operation_json_v1(context, &encoded)
            .map_err(|error| {
                CliError::new(format!(
                    "collective trade operationPlan failed pinned SDK validation: {error}"
                ))
            })?;
    let validated = admitted
        .swap_operation()
        .ok_or_else(|| CliError::new("The SDK did not admit a collective swap."))?;
    let direction = match request.get("direction").and_then(Value::as_str) {
        Some("QuoteForOption") => ameba_sdk::CollectiveSwapDirection::QuoteForOption,
        Some("OptionForQuote") => ameba_sdk::CollectiveSwapDirection::OptionForQuote,
        _ => return Err(CliError::new("collective trade direction is invalid")),
    };
    let expected = ameba_sdk::ExpectedCollectiveSwapRequest {
        trader: Pubkey::from_str(
            request
                .get("owner")
                .and_then(Value::as_str)
                .ok_or_else(|| CliError::new("collective trade request is missing trader"))?,
        )
        .map_err(|error| CliError::new(format!("invalid trader public key: {error}")))?,
        market: Pubkey::from_str(
            request
                .get("market")
                .and_then(Value::as_str)
                .ok_or_else(|| CliError::new("collective trade request is missing market"))?,
        )
        .map_err(|error| CliError::new(format!("invalid market public key: {error}")))?,
        direction,
        amount_in: request
            .get("amountIn")
            .and_then(Value::as_str)
            .and_then(|raw| raw.parse().ok())
            .ok_or_else(|| CliError::new("collective trade request has invalid amountIn"))?,
        minimum_amount_out: request
            .get("minimumAmountOut")
            .and_then(Value::as_str)
            .and_then(|raw| raw.parse().ok())
            .ok_or_else(|| {
                CliError::new("collective trade request has invalid minimumAmountOut")
            })?,
        limit_bin_id: u16::try_from(
            request
                .get("limitBinId")
                .and_then(Value::as_u64)
                .ok_or_else(|| CliError::new("collective trade request has invalid limitBinId"))?,
        )
        .map_err(|_| CliError::new("collective trade limitBinId exceeds u16"))?,
    };
    ameba_sdk::require_expected_collective_swap_request(&validated, &expected).map_err(
        |error| {
            CliError::new(format!(
                "collective trade plan differs from the explicit request: {error}"
            ))
        },
    )?;
    Ok(admitted)
}

/// Shared preparation only. Each adapter retains its output, journal and
/// approval policy; reviewed execution still re-observes its saved operation.
pub(crate) struct PreparedTrade {
    pub config: crate::onchain::OnchainConfig,
    pub request: Value,
    pub response: Value,
    pub admitted: ameba_sdk::CurrentGovernedOperationV1,
}

pub(crate) fn prepare_exact_input(
    cli: &Cli,
    backend: &BackendClient,
    trade: &CollectiveTradeArgs,
) -> Result<PreparedTrade, CliError> {
    crate::current_release::require_current_write_release()?;
    let config = crate::app_context::build_onchain_config(cli)?;
    // Identity must precede hardware-wallet public-key access and preparation.
    let context = current_operation::observe_current_write_context(&config)?;
    let request = collective_trade_request(cli, trade)?;
    let response = crate::backend::current_backend_payload(
        backend
            .post_json_with_current_state_retry(crate::endpoints::dlmm_trade_prepare(), &request)?,
    )?;
    let admitted = validate_collective_trade_response(&response, &request, &context)?;
    Ok(PreparedTrade {
        config,
        request,
        response,
        admitted,
    })
}

#[derive(Clone, Debug, clap::Args)]
pub struct TradeIntent {
    #[arg(long, help = "Product, for example ramx")]
    pub market: String,
    #[arg(
        long,
        alias = "series",
        help = "Exact listed series, for example RAMX-202610-CALL-01"
    )]
    pub expiry: String,
    #[arg(
        long,
        help = "Whole contracts; buys spend the limit-price budget and receive at least this quantity"
    )]
    pub quantity: String,
    #[arg(
        long,
        help = "USDC per contract at the limit bin, before the native pool fee; review gross input and net minimum output"
    )]
    pub limit_price: String,
}

/// Decimal parsing is integer-only. No rounding or exponent syntax is accepted.
pub fn decimal_atoms(value: &str, decimals: u8) -> Result<u64, CliError> {
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty()
        || !whole.bytes().all(|c| c.is_ascii_digit())
        || !fraction.bytes().all(|c| c.is_ascii_digit())
        || fraction.len() > usize::from(decimals)
        || (whole.len() > 1 && whole.starts_with('0'))
        || value.ends_with('.')
    {
        return Err(CliError::new(
            "Enter a positive decimal amount without exponents or excess precision.",
        ));
    }
    let scale = 10u64
        .checked_pow(u32::from(decimals))
        .ok_or_else(|| CliError::new("Unsupported token precision."))?;
    let whole = whole.parse::<u64>().ok().and_then(|n| n.checked_mul(scale));
    let fractional = if fraction.is_empty() {
        Some(0)
    } else {
        fraction
            .parse::<u64>()
            .ok()
            .and_then(|n| n.checked_mul(10u64.pow(u32::from(decimals) - fraction.len() as u32)))
    };
    whole
        .and_then(|n| n.checked_add(fractional?))
        .filter(|n| *n > 0)
        .ok_or_else(|| CliError::new("Amount must be positive and fit the token amount range."))
}

fn resolve(
    cli: &Cli,
    backend: &BackendClient,
    intent: &TradeIntent,
    buy: bool,
) -> Result<(CollectiveTradeArgs, Value), CliError> {
    let product = intent.market.to_ascii_lowercase();
    market_surface::parse_current_series_id(&product, &intent.expiry)?;
    let snapshot = market_surface::dish_snapshot_payload(backend, &product)?;
    let rows = snapshot
        .pointer("/data/market/expiries")
        .or_else(|| snapshot.pointer("/data/expiries"))
        .and_then(Value::as_array)
        .ok_or_else(|| CliError::new("Current series inventory is unavailable."))?;
    let row = rows
        .iter()
        .find(|row| row["expiryId"] == intent.expiry)
        .ok_or_else(|| CliError::new("The selected exact series is not listed."))?;
    if row["poolStatus"] != "Active" {
        return Err(CliError::new(
            "This series is not open for trading. Inspect its settlement or position actions.",
        ));
    }
    let config = crate::app_context::build_onchain_config(cli)?;
    let (address, market, block_time) =
        current_operation::read_trade_market(&config, &intent.expiry)?;
    let quantity = crate::request_validation::canonical_u64_string(
        &intent.quantity,
        "contract quantity",
        false,
    )?
    .parse::<u64>()
    .unwrap();
    let quantity_atoms = quantity
        .checked_mul(ameba_sdk::CANONICAL_CONTRACT_SIZE_ATOMIC)
        .ok_or_else(|| CliError::new("Contract quantity is too large."))?;
    let price = decimal_atoms(&intent.limit_price, 6)?;
    let bounds = ameba_sdk::current_amoeba_dlmm_pool_bounds(&market)
        .map_err(|e| CliError::new(e.to_string()))?;
    if price % bounds.tick_size_quote_atomic != 0 {
        return Err(CliError::new(format!(
            "Price must align to the current {}-atom price step.",
            bounds.tick_size_quote_atomic
        )));
    }
    let bin = u16::try_from(price / bounds.tick_size_quote_atomic)
        .map_err(|_| CliError::new("Price is outside this contract's range."))?;
    let ticket = ameba_sdk::build_current_amoeba_dlmm_trade_ticket(
        &market,
        if buy {
            ameba_sdk::CurrentTradeTicketSide::Bid
        } else {
            ameba_sdk::CurrentTradeTicketSide::Ask
        },
        quantity_atoms,
        bin,
        block_time,
    )
    .map_err(|e| CliError::new(e.to_string()))?;
    let review = json!({"side":if buy {"buy"} else {"sell_owned_long"},"series":intent.expiry,
        "requestedContracts":quantity.to_string(),"limitPriceUsdc":intent.limit_price,
        "inputAtoms":ticket.gross_amount_in.to_string(),"minimumOutputAtoms":ticket.minimum_amount_out.to_string(),
        "inputToken":if buy {"USDC"} else {"contracts"},"outputToken":if buy {"contracts"} else {"USDC"},
        "poolFeeBps":ticket.pool_bounds.taker_fee_bps,"poolFeeIncluded":true,
        "maximumPayoutPerContractUsdcAtoms":market.instrument.max_payout_per_contract.to_string(),
        "settlementTimestamp":market.instrument.expiry_ts.to_string(),"networkAndSetupCosts":"additional_not_quoted"});
    Ok((
        CollectiveTradeArgs {
            market: address.to_string(),
            direction: if buy {
                CollectiveSwapDirectionValue::QuoteForOption
            } else {
                CollectiveSwapDirectionValue::OptionForQuote
            },
            amount_in: ticket.gross_amount_in.to_string(),
            minimum_amount_out: ticket.minimum_amount_out.to_string(),
            limit_bin_id: ticket.target_bin_id,
        },
        review,
    ))
}

pub fn prepare(
    cli: &Cli,
    backend: &BackendClient,
    intent: &TradeIntent,
    buy: bool,
) -> Result<Value, CliError> {
    crate::current_release::require_current_write_release()?;
    let (trade, mut review) = resolve(cli, backend, intent, buy)?;
    let PreparedTrade {
        request,
        response,
        admitted,
        ..
    } = prepare_exact_input(cli, backend, &trade)?;
    let id = crate::operation_journal::record_prepared(backend, &request, &response, &admitted)?;
    review["owner"] = json!(admitted.payer().to_string());
    review["network"] = json!(cli.cluster);
    review["deadlineTimestamp"] = json!(
        admitted
            .swap_operation()
            .map(|s| s.plan.semantic.deadline_ts.as_str())
    );
    Ok(
        json!({"ok":true, "operationId":id, "operation":crate::operation_journal::load(&id)?.public_value(), "request":request, "prepared":response,
        "review":review,
        "execution":"exact_input", "positionEffect": if buy {"acquire_long"} else {"sell_owned_long"},
        "nextStep":"Review the exact input and minimum output, then explicitly approve this operation."}),
    )
}

fn decimal_display(value: &Value) -> String {
    let Some(atoms) = value.as_str().and_then(|s| s.parse::<u64>().ok()) else {
        return "unavailable".into();
    };
    let fraction = format!("{:06}", atoms % 1_000_000);
    let fraction = fraction.trim_end_matches('0');
    if fraction.is_empty() {
        (atoms / 1_000_000).to_string()
    } else {
        format!("{}.{fraction}", atoms / 1_000_000)
    }
}

fn time_display(value: &Value) -> String {
    value
        .as_str()
        .and_then(|s| s.parse::<i64>().ok())
        .and_then(|s| chrono::DateTime::from_timestamp(s, 0))
        .map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| "unavailable".into())
}

pub fn render_response(payload: &Value) -> String {
    let Some(review) = payload.get("review") else {
        let receipt = &payload["receipt"];
        return format!(
            "Trade confirmed\nOperation {}\nSignature {}",
            receipt["operationId"].as_str().unwrap_or("unavailable"),
            receipt["signature"].as_str().unwrap_or("unavailable")
        );
    };
    let text = |key: &str| review[key].as_str().unwrap_or("unavailable");
    let mut lines = vec![
        format!(
            "{} · {} · {}",
            if text("side") == "buy" {
                "Buy"
            } else {
                "Sell owned options"
            },
            text("series"),
            text("network")
        ),
        format!("Wallet {}", text("owner")),
        format!(
            "Spend exactly {} {}",
            decimal_display(&review["inputAtoms"]),
            text("inputToken")
        ),
        format!(
            "Receive at least {} {}",
            decimal_display(&review["minimumOutputAtoms"]),
            text("outputToken")
        ),
    ];
    if text("side") == "buy" {
        lines.push(format!(
            "Maximum trade loss {} USDC",
            decimal_display(&review["inputAtoms"])
        ));
        lines.push(format!(
            "Contract payout cap {} USDC each",
            decimal_display(&review["maximumPayoutPerContractUsdcAtoms"])
        ));
    } else {
        lines.push(
            "Sells existing options; no new short. Prior purchase P&L is not included.".into(),
        );
    }
    lines.extend([
        format!("Settles {}", time_display(&review["settlementTimestamp"])),
        format!(
            "Native pool fee {} bps included in these amounts",
            review["poolFeeBps"]
        ),
        "Network/setup costs are additional; not quoted here.".into(),
        format!(
            "Approve before {}",
            time_display(&review["deadlineTimestamp"])
        ),
        format!(
            "Operation {}",
            payload["operationId"].as_str().unwrap_or("unavailable")
        ),
    ]);
    crate::backend::terminal_safe_text(&lines.join("\n"))
}

pub fn execute_reviewed(cli: &Cli, backend: &BackendClient, id: &str) -> Result<Value, CliError> {
    crate::current_release::require_current_write_release()?;
    let record = crate::operation_journal::load(id)?;
    record.require_scope(
        backend,
        &crate::attached_wallet::attached_wallet_pubkey(cli)?,
    )?;
    if record.state != "prepared" {
        return Err(CliError::new(
            "This operation was already attempted. Use operations resume; it never resubmits.",
        ));
    }
    let config = crate::app_context::build_onchain_config(cli)?;
    let context = current_operation::observe_current_write_context(&config)?;
    let admitted = validate_collective_trade_response(&record.prepared, &record.request, &context)?;
    let deadline = admitted
        .swap_operation()
        .and_then(|s| s.plan.semantic.deadline_ts.parse::<u64>().ok())
        .ok_or_else(|| {
            CliError::new("The reviewed operation has no current deadline. Prepare a new ticket.")
        })?;
    let receipt = current_operation::sign_submit_validated_operation(
        &config,
        backend,
        crate::endpoints::dlmm_trade_submit(),
        &crate::endpoints::dlmm_trade_status(id),
        "collective_swap_exact_in",
        &admitted,
        Some(deadline),
    )?;
    Ok(json!({"ok":true,"receipt":receipt}))
}
