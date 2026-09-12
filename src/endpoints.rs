pub fn chain_identity() -> &'static str {
    "/chain/identity"
}

pub fn capabilities() -> &'static str {
    "/capabilities"
}

pub fn dlmm_markets() -> &'static str {
    "/dlmm/markets"
}

pub fn dlmm_market_snapshot(market_id: &str) -> String {
    format!("/dlmm/markets/{}/snapshot", path_segment(market_id))
}

pub fn dlmm_market_chart(market_id: &str, window_ms: Option<u64>) -> String {
    let mut path = format!("/dlmm/markets/{}/chart", path_segment(market_id));
    if let Some(window_ms) = window_ms {
        path.push_str(&format!("?windowMs={window_ms}"));
    }
    path
}

pub fn dlmm_expiry_chart(market_id: &str, expiry_id: &str, window_ms: Option<u64>) -> String {
    let mut path = format!(
        "/dlmm/markets/{}/expiries/{}/chart",
        path_segment(market_id),
        path_segment(expiry_id)
    );
    if let Some(window_ms) = window_ms {
        path.push_str(&format!("?windowMs={window_ms}"));
    }
    path
}

pub fn dlmm_oracle_state() -> &'static str {
    "/dlmm/oracle/state"
}

pub fn dlmm_oracle_markets() -> &'static str {
    "/dlmm/oracle/markets"
}

pub fn dlmm_oracle_market(market_id: &str) -> String {
    format!("/dlmm/oracle/markets/{}", path_segment(market_id))
}

pub fn dlmm_oracle_latest(market_id: Option<&str>) -> String {
    match market_id.and_then(nonempty_trimmed) {
        Some(market_id) => format!("{}/latest", dlmm_oracle_market(market_id)),
        None => "/dlmm/oracle/latest".to_string(),
    }
}

pub fn dlmm_oracle_history(market_id: Option<&str>) -> String {
    match market_id.and_then(nonempty_trimmed) {
        Some(market_id) => format!("{}/history", dlmm_oracle_market(market_id)),
        None => "/dlmm/oracle/history".to_string(),
    }
}

pub fn dlmm_settlement(market_id: &str, expiry_id: &str) -> String {
    format!(
        "/dlmm/markets/{}/settlements/{}",
        path_segment(market_id),
        path_segment(expiry_id)
    )
}

pub fn dlmm_settlement_oracle(market_id: &str, expiry_id: &str) -> String {
    format!("{}/oracle", dlmm_settlement(market_id, expiry_id))
}

pub fn dlmm_settlement_oracle_preflight(market_id: &str, expiry_id: &str) -> String {
    format!("{}/preflight", dlmm_settlement_oracle(market_id, expiry_id))
}

pub fn user_ledger(owner_pubkey: &str, limit: usize) -> String {
    format!("/users/{}/ledger?limit={limit}", path_segment(owner_pubkey))
}

pub fn user_collateral(owner_pubkey: &str) -> String {
    format!("/users/{}/collateral", path_segment(owner_pubkey))
}

pub fn dlmm_positions(owner_pubkey: &str) -> String {
    format!("/dlmm/positions?owner={}", path_segment(owner_pubkey))
}

pub fn dlmm_trade_prepare() -> &'static str {
    "/dlmm/trades/prepare"
}

pub fn dlmm_trade_submit() -> &'static str {
    "/dlmm/trades/submit"
}

pub fn dlmm_trade_lifetime() -> &'static str {
    "/dlmm/trades/transaction-lifetime"
}

pub fn dlmm_trade_status(operation_id: &str) -> String {
    format!("/dlmm/trades/status/{}", path_segment(operation_id))
}

pub fn writer_sleeves(owner_pubkey: Option<&str>) -> String {
    match owner_pubkey.and_then(nonempty_trimmed) {
        Some(owner) => format!("/dlmm/writer-sleeves?owner={}", path_segment(owner)),
        None => "/dlmm/writer-sleeves".to_string(),
    }
}

pub fn writer_sleeve(sleeve: &str) -> String {
    format!("/dlmm/writer-sleeves/{}", path_segment(sleeve))
}

pub fn writer_available_actions(sleeve: &str, owner_pubkey: &str) -> String {
    format!(
        "{}/available-actions?owner={}",
        writer_sleeve(sleeve),
        path_segment(owner_pubkey)
    )
}

pub fn writer_policy_audit(sleeve: &str) -> String {
    format!("{}/policy-audit", writer_sleeve(sleeve))
}

pub fn writer_close_status(close_request: &str) -> String {
    format!(
        "/dlmm/writer-sleeves/close-requests/{}",
        path_segment(close_request)
    )
}

pub fn writer_deposit_prepare() -> &'static str {
    "/dlmm/writer-sleeves/deposits/prepare"
}

pub fn writer_bid_prepare() -> &'static str {
    "/dlmm/writer-sleeves/bids/prepare"
}

pub fn writer_withdraw_prepare() -> &'static str {
    "/dlmm/writer-sleeves/withdrawals/prepare"
}

pub fn writer_refund_prepare() -> &'static str {
    "/dlmm/writer-sleeves/refunds/prepare"
}

pub fn writer_refunds(owner: &str, cursor: Option<&str>, limit: u8) -> String {
    let mut path = format!(
        "/dlmm/writer-sleeves/refunds?owner={}&limit={limit}",
        path_segment(owner)
    );
    if let Some(cursor) = cursor {
        path.push_str(&format!("&cursor={}", path_segment(cursor)));
    }
    path
}

pub fn writer_liquidity(sleeve: &str, owner: &str, series_index: u8) -> String {
    format!(
        "{}/liquidity?owner={}&seriesIndex={series_index}",
        writer_sleeve(sleeve),
        path_segment(owner)
    )
}

pub fn writer_liquidity_initialize_prepare() -> &'static str {
    "/dlmm/writer-sleeves/liquidity/initialize/prepare"
}

pub fn writer_liquidity_add_prepare() -> &'static str {
    "/dlmm/writer-sleeves/liquidity/add/prepare"
}

pub fn writer_liquidity_remove_prepare() -> &'static str {
    "/dlmm/writer-sleeves/liquidity/remove/prepare"
}

pub fn writer_liquidity_sweep_prepare() -> &'static str {
    "/dlmm/writer-sleeves/liquidity/sweep/prepare"
}

pub fn writer_close_preview() -> &'static str {
    "/dlmm/writer-sleeves/closes/preview"
}

pub fn writer_close_prepare() -> &'static str {
    "/dlmm/writer-sleeves/closes/prepare"
}

pub fn writer_close_next_prepare() -> &'static str {
    "/dlmm/writer-sleeves/closes/next/prepare"
}

#[allow(dead_code)] // Reserved for a future reviewed wallet action ABI.
pub fn writer_close_cancel_next_prepare() -> &'static str {
    "/dlmm/writer-sleeves/closes/cancel/next/prepare"
}

pub fn writer_claim_prepare() -> &'static str {
    "/dlmm/writer-sleeves/claims/prepare"
}

pub fn writer_flat_transfer_prepare() -> &'static str {
    "/dlmm/writer-sleeves/flat-transfers/prepare"
}

pub fn writer_operation_submit() -> &'static str {
    "/dlmm/writer-sleeves/submit"
}

pub fn writer_operation_status(operation_id: &str) -> String {
    format!("/dlmm/writer-sleeves/status/{}", path_segment(operation_id))
}

pub fn dlmm_liquidity_prepare() -> &'static str {
    "/dlmm/liquidity/prepare"
}

pub fn position_action(position: &str) -> String {
    format!("/dlmm/positions/{}/action", path_segment(position))
}
pub fn oracle_draft_prepare() -> &'static str {
    "/dlmm/oracle/drafts/prepare"
}
pub fn registered_transaction_submit() -> &'static str {
    "/chain/transactions/submit"
}

fn path_segment(value: &str) -> String {
    percent_encode_path_segment(value.trim())
}

fn nonempty_trimmed(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn percent_encode_path_segment(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match *byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                output.push(char::from(*byte));
            }
            other => output.push_str(&format!("%{other:02X}")),
        }
    }
    output
}
