//! Unsigned manager-liquidity preview state for the integrated Ledger workspace.
//!
//! The TUI intentionally delegates the actual request grammar and numeric
//! validation to the public `petri liquidity plan` command. This module only
//! tokenizes the compact multi-entry field, builds a shell-free argv vector,
//! and checks that a successful child result is still bound to the visible
//! wallet and semantic request. It never prepares transaction bytes, signs, or
//! submits.

use super::*;

pub(super) use crate::cli::LiquidityActionValue as LiquidityPreviewAction;

impl LiquidityPreviewAction {
    const ALL: [Self; 3] = [Self::Add, Self::Remove, Self::ClosePosition];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Add => "Add",
            Self::Remove => "Remove",
            Self::ClosePosition => "Close position",
        }
    }

    fn offset(self, offset: isize) -> Self {
        let current = Self::ALL
            .iter()
            .position(|action| *action == self)
            .unwrap_or_default() as isize;
        Self::ALL[(current + offset).rem_euclid(Self::ALL.len() as isize) as usize]
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum LiquidityPreviewField {
    #[default]
    Action,
    Market,
    Expiry,
    PositionNonce,
    Entries,
}

impl LiquidityPreviewField {
    pub(super) const ALL: [Self; 5] = [
        Self::Action,
        Self::Market,
        Self::Expiry,
        Self::PositionNonce,
        Self::Entries,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Action => "Action",
            Self::Market => "Market",
            Self::Expiry => "Expiry",
            Self::PositionNonce => "Position nonce",
            Self::Entries => "Entries",
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct LiquidityPreviewForm {
    pub(super) action: LiquidityPreviewAction,
    pub(super) market: String,
    pub(super) expiry: String,
    pub(super) position_nonce: String,
    pub(super) entries: String,
    pub(super) selected_field: usize,
}

impl LiquidityPreviewForm {
    pub(super) fn from_position(position: Option<&Value>) -> Self {
        Self {
            market: position
                .and_then(|row| string_at_key(row, &["marketId"]))
                .unwrap_or_default(),
            expiry: position
                .and_then(|row| string_at_key(row, &["seriesId", "expiryId"]))
                .unwrap_or_default(),
            ..Self::default()
        }
    }

    pub(super) fn selected_field(&self) -> LiquidityPreviewField {
        LiquidityPreviewField::ALL
            .get(self.selected_field)
            .copied()
            .unwrap_or_default()
    }

    pub(super) fn field_value(&self, field: LiquidityPreviewField) -> String {
        match field {
            LiquidityPreviewField::Action => self.action.label().to_string(),
            LiquidityPreviewField::Market => self.market.clone(),
            LiquidityPreviewField::Expiry => self.expiry.clone(),
            LiquidityPreviewField::PositionNonce => self.position_nonce.clone(),
            LiquidityPreviewField::Entries => self.entries.clone(),
        }
    }

    fn selected_text_mut(&mut self) -> Option<&mut String> {
        match self.selected_field() {
            LiquidityPreviewField::Action => None,
            LiquidityPreviewField::Market => Some(&mut self.market),
            LiquidityPreviewField::Expiry => Some(&mut self.expiry),
            LiquidityPreviewField::PositionNonce => Some(&mut self.position_nonce),
            LiquidityPreviewField::Entries => Some(&mut self.entries),
        }
    }

    fn selected_text_limit(&self) -> usize {
        match self.selected_field() {
            LiquidityPreviewField::Action => 0,
            LiquidityPreviewField::Market => 64,
            LiquidityPreviewField::Expiry => 128,
            LiquidityPreviewField::PositionNonce => 20,
            LiquidityPreviewField::Entries => 4096,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct LiquidityPreviewResult {
    pub(super) ok: bool,
    pub(super) payload: Option<Value>,
    pub(super) message: String,
}

impl LabApp {
    pub(super) fn open_liquidity_preview_form(&mut self) {
        if self.liquidity_preview_is_running() {
            self.status = "The unsigned liquidity preview is still loading.".to_string();
            return;
        }
        if self.wallet.pubkey.is_none() {
            self.status =
                "Attach a wallet before requesting an owner-bound liquidity preview.".to_string();
            return;
        }
        self.liquidity_preview_result = None;
        self.liquidity_preview_form = Some(LiquidityPreviewForm::from_position(
            self.liquidity_position_rows()
                .get(self.ledger_position_selected),
        ));
        self.ledger_pane = LedgerPane::Detail;
        self.status =
            "Unsigned Lean preview form opened. Nothing will be prepared, signed, or submitted."
                .to_string();
    }

    pub(super) fn move_liquidity_preview_field(&mut self, offset: isize) {
        let Some(form) = self.liquidity_preview_form.as_mut() else {
            return;
        };
        if offset == 0 {
            return;
        }
        form.selected_field = (form.selected_field as isize + offset)
            .rem_euclid(LiquidityPreviewField::ALL.len() as isize)
            as usize;
        self.status = format!(
            "Editing {}.",
            form.selected_field().label().to_ascii_lowercase()
        );
    }

    pub(super) fn cycle_liquidity_preview_action(&mut self, offset: isize) {
        let Some(form) = self.liquidity_preview_form.as_mut() else {
            return;
        };
        if form.selected_field() == LiquidityPreviewField::Action && offset != 0 {
            form.action = form.action.offset(offset);
            self.status = format!("Preview action: {}.", form.action.label());
        }
    }

    pub(super) fn push_liquidity_preview_char(&mut self, character: char) {
        if character.is_control() {
            return;
        }
        let Some(form) = self.liquidity_preview_form.as_mut() else {
            return;
        };
        if form.selected_field() == LiquidityPreviewField::Action {
            if matches!(character, ' ' | 'a' | 'A' | 'r' | 'R' | 'c' | 'C') {
                form.action = match character {
                    'a' | 'A' => LiquidityPreviewAction::Add,
                    'r' | 'R' => LiquidityPreviewAction::Remove,
                    'c' | 'C' => LiquidityPreviewAction::ClosePosition,
                    _ => form.action.offset(1),
                };
                self.status = format!("Preview action: {}.", form.action.label());
            }
            return;
        }
        let limit = form.selected_text_limit();
        if let Some(input) = form.selected_text_mut()
            && input.chars().count() < limit
        {
            input.push(character);
        }
    }

    pub(super) fn backspace_liquidity_preview_input(&mut self) {
        if let Some(input) = self
            .liquidity_preview_form
            .as_mut()
            .and_then(LiquidityPreviewForm::selected_text_mut)
        {
            input.pop();
        }
    }

    pub(super) fn cancel_liquidity_preview_form(&mut self) {
        self.liquidity_preview_form = None;
        self.status =
            "Liquidity preview cancelled. Nothing was prepared, signed, or submitted.".to_string();
    }

    pub(super) fn request_liquidity_preview(
        &mut self,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        if self.liquidity_preview_is_running() {
            self.status = "The unsigned liquidity preview is already loading.".to_string();
            return;
        }
        let Some(form) = self.liquidity_preview_form.as_ref() else {
            return;
        };
        let Some(owner_pubkey) = self.wallet.pubkey.clone() else {
            self.status =
                "Attach a wallet before requesting an owner-bound liquidity preview.".to_string();
            return;
        };
        let args = match liquidity_preview_command_args(form) {
            Ok(args) => args,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let envs = liquidity_preview_command_envs(
            backend_url,
            &self.onchain_config.network,
            &self.wallet.keypair_path,
        );
        let binding = match liquidity_preview_binding(&owner_pubkey, form) {
            Ok(binding) => binding,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        self.liquidity_preview_request = self.liquidity_preview_request.wrapping_add(1);
        self.liquidity_preview_inflight = Some(self.liquidity_preview_request);
        self.liquidity_preview_result = None;
        self.liquidity_preview_form = None;
        self.status =
            "Preparing SDK-validated liquidity review; nothing will be signed.".to_string();
        spawn_liquidity_preview(
            args,
            envs,
            binding,
            fetch_tx.clone(),
            self.liquidity_preview_request,
        );
    }

    pub(super) fn liquidity_preview_is_running(&self) -> bool {
        self.liquidity_preview_inflight.is_some()
    }

    pub(super) fn apply_liquidity_preview_result(
        &mut self,
        request_id: u64,
        result: Result<Value, String>,
    ) {
        if self.liquidity_preview_inflight != Some(request_id) {
            return;
        }
        self.liquidity_preview_inflight = None;
        self.liquidity_preview_form = None;
        match result {
            Ok(payload) => {
                let message = "SDK-validated liquidity review ready. Nothing signed or submitted. Open F9 to review and approve the operation."
                    .to_string();
                self.liquidity_preview_result = Some(LiquidityPreviewResult {
                    ok: true,
                    payload: Some(payload),
                    message: message.clone(),
                });
                self.status = message;
            }
            Err(error) => {
                self.liquidity_preview_result = Some(LiquidityPreviewResult {
                    ok: false,
                    payload: None,
                    message: error.clone(),
                });
                self.status = error;
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LiquidityPreviewBinding {
    owner_pubkey: String,
    market: String,
    expiry: String,
    action: LiquidityPreviewAction,
    position_nonce: String,
    entries: Vec<String>,
}

pub(super) fn liquidity_preview_binding(
    owner_pubkey: &str,
    form: &LiquidityPreviewForm,
) -> Result<LiquidityPreviewBinding, String> {
    let position_nonce = form.position_nonce.trim().parse::<u64>().map_err(|_| {
        "Position nonce must be an unsigned 64-bit integer from an authoritative source."
            .to_string()
    })?;
    Ok(LiquidityPreviewBinding {
        owner_pubkey: owner_pubkey.to_string(),
        market: form.market.trim().to_ascii_lowercase(),
        expiry: form.expiry.trim().to_string(),
        action: form.action,
        position_nonce: position_nonce.to_string(),
        entries: liquidity_preview_entries(&form.entries)?,
    })
}

pub(super) fn liquidity_preview_entries(raw: &str) -> Result<Vec<String>, String> {
    let entries = raw
        .split(|character: char| character.is_whitespace() || matches!(character, ',' | ';'))
        .filter(|entry| !entry.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if entries.is_empty() || entries.len() > 32 {
        return Err(
            "Enter between 1 and 32 liquidity entries as BIN:AMOUNT_A:AMOUNT_B:AMOUNT_C."
                .to_string(),
        );
    }
    Ok(entries)
}

pub(super) fn liquidity_preview_command_args(
    form: &LiquidityPreviewForm,
) -> Result<Vec<String>, String> {
    let market = form.market.trim();
    if market.is_empty() {
        return Err("Enter the exact current market id.".to_string());
    }
    let expiry = form.expiry.trim();
    if expiry.is_empty() {
        return Err("Enter the exact current expiry id.".to_string());
    }
    let position_nonce = form.position_nonce.trim();
    if position_nonce.is_empty() {
        return Err(
            "Enter the exact position nonce from an authoritative source; Petri will not infer it."
                .to_string(),
        );
    }
    position_nonce.parse::<u64>().map_err(|_| {
        "Position nonce must be an unsigned 64-bit integer from an authoritative source."
            .to_string()
    })?;
    let entries = liquidity_preview_entries(&form.entries)?;

    let mut args = vec![
        "--json".to_string(),
        "--quiet".to_string(),
        "liquidity".to_string(),
        "plan".to_string(),
        "--action".to_string(),
        form.action.cli_value().to_string(),
        "--market".to_string(),
        market.to_string(),
        "--expiry".to_string(),
        expiry.to_string(),
        "--position-nonce".to_string(),
        position_nonce.to_string(),
    ];
    for entry in entries {
        args.extend(["--entry".to_string(), entry]);
    }
    Ok(args)
}

pub(super) fn liquidity_preview_command_envs(
    backend_url: &str,
    network: &str,
    keypair_path: &str,
) -> Vec<(String, String)> {
    vec![
        ("AMEBA_BACKEND_URL".to_string(), backend_url.to_string()),
        ("AMEBA_CLUSTER".to_string(), network.to_string()),
        ("SOLANA_KEYPAIR".to_string(), keypair_path.to_string()),
    ]
}

pub(super) fn validate_liquidity_preview_response(
    payload: &Value,
    binding: &LiquidityPreviewBinding,
) -> Result<(), String> {
    let request = payload.get("review").unwrap_or(&Value::Null);
    let field = |key: &str| request.get(key).and_then(Value::as_str);
    let entries = request
        .get("entries")
        .and_then(Value::as_array)
        .and_then(|entries| liquidity_preview_response_entries(binding.action, entries));
    let signing = payload.get("signing").unwrap_or(&Value::Null);
    let exact = field("ownerPubkey") == Some(binding.owner_pubkey.as_str())
        && field("marketId") == Some(binding.market.as_str())
        && field("expiryId") == Some(binding.expiry.as_str())
        && field("action") == Some(binding.action.as_request_value())
        && field("positionNonce") == Some(binding.position_nonce.as_str())
        && entries.as_ref() == Some(&binding.entries)
        && signing.get("willSign").and_then(Value::as_bool) == Some(false)
        && signing.get("willSubmit").and_then(Value::as_bool) == Some(false)
        && payload
            .get("operation")
            .and_then(|value| value.get("state"))
            .and_then(Value::as_str)
            == Some("prepared");
    if exact {
        Ok(())
    } else {
        Err("Petri rejected the liquidity preview because its returned request or unsigned status did not match the visible form and wallet. Nothing was prepared, signed, or submitted."
            .to_string())
    }
}

fn liquidity_preview_response_entries(
    action: LiquidityPreviewAction,
    entries: &[Value],
) -> Option<Vec<String>> {
    entries
        .iter()
        .map(|entry| {
            let bin = entry.get("binId").and_then(display_unsigned_json)?;
            let (amount_a, amount_b, amount_c) = match action {
                LiquidityPreviewAction::Add => (
                    entry
                        .get("maximumOptionAmount")
                        .and_then(display_unsigned_json)?,
                    entry
                        .get("maximumQuoteAmount")
                        .and_then(display_unsigned_json)?,
                    entry.get("minimumShares").and_then(display_unsigned_json)?,
                ),
                LiquidityPreviewAction::Remove | LiquidityPreviewAction::ClosePosition => (
                    entry.get("shares").and_then(display_unsigned_json)?,
                    entry
                        .get("minimumOptionOut")
                        .and_then(display_unsigned_json)?,
                    entry
                        .get("minimumQuoteOut")
                        .and_then(display_unsigned_json)?,
                ),
            };
            Some(format!("{bin}:{amount_a}:{amount_b}:{amount_c}"))
        })
        .collect()
}

fn display_unsigned_json(value: &Value) -> Option<String> {
    match value {
        Value::String(value)
            if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            Some(value.clone())
        }
        Value::Number(value) if value.is_u64() => Some(value.to_string()),
        _ => None,
    }
}

pub(super) fn user_safe_liquidity_preview_failure(raw: &str) -> String {
    let normalized = raw.to_ascii_lowercase();
    if normalized.contains("position nonce") || normalized.contains("position-nonce") {
        "The position nonce was missing or invalid. Copy the exact nonce from an authoritative source; Petri will not infer it."
            .to_string()
    } else if normalized.contains("entry")
        || normalized.contains("bin")
        || normalized.contains("ascending")
    {
        "The liquidity entries were invalid. Use 1-32 strictly ascending BIN:AMOUNT_A:AMOUNT_B:AMOUNT_C values."
            .to_string()
    } else if normalized.contains("wallet")
        || normalized.contains("keypair")
        || normalized.contains("signer")
    {
        "Petri could not bind the preview to the attached wallet. Check the configured signing source; nothing was signed."
            .to_string()
    } else if normalized.contains("identity")
        || normalized.contains("deployment")
        || normalized.contains("program")
        || normalized.contains("genesis")
    {
        "Petri could not verify the current Amoeba deployment, so no preview was accepted."
            .to_string()
    } else if normalized.contains("unavailable")
        || normalized.contains("timeout")
        || normalized.contains("connection")
        || normalized.contains("network")
        || normalized.contains("http")
    {
        "The unsigned liquidity preview is temporarily unavailable. No state was changed."
            .to_string()
    } else {
        "Petri could not load the unsigned liquidity preview. No state was changed or inferred."
            .to_string()
    }
}
