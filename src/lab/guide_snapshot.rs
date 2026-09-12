//! Projection of the active Lab screen into bounded, validated Guide snapshots.

use super::*;

pub(super) fn screen_title(screen: LabScreen) -> &'static str {
    match screen {
        LabScreen::Terms => "wallet terms",
        LabScreen::Home => "home",
        LabScreen::Staking => "staking",
        LabScreen::Chain => "options",
        LabScreen::Chart => "month chart",
        LabScreen::OracleIntro => "oracle entry",
        LabScreen::Oracle => "oracle",
        LabScreen::OracleHelp => "oracle help",
        LabScreen::Help => "help",
        LabScreen::Detail => "market",
        LabScreen::Activity => "contract trades",
        LabScreen::Ledger => "wallet ledger",
    }
}

pub(super) fn lab_focus_id(focus: LabFocus) -> &'static str {
    match focus {
        LabFocus::Terms => "terms",
        LabFocus::Staking => "staking",
        LabFocus::Markets => "markets",
        LabFocus::MarketSeries => "market_months",
        LabFocus::HomeSummary => "market_summary",
        LabFocus::HomeActions => "home_actions",
        LabFocus::HomePreview => "home_preview",
        LabFocus::OracleIntro => "oracle_entry",
        LabFocus::OracleEarn => "oracle_earn",
        LabFocus::OracleHelp => "oracle_help",
        LabFocus::Chart => "month_chart",
        LabFocus::OracleOverview => "phase_timeline",
        LabFocus::OracleTasks => "oracle_tree",
        LabFocus::OracleActions => "source_actions",
        LabFocus::OraclePath => "source_detail",
        LabFocus::Help => "help",
        LabFocus::Detail => "market_detail",
        LabFocus::Activity => "contract_trades",
        LabFocus::Ledger => "wallet_ledger",
        LabFocus::Calls => "calls",
        LabFocus::Puts => "puts",
        LabFocus::Guide => "guide",
    }
}

pub(super) fn oracle_phase_id(phase: OraclePhase) -> &'static str {
    match phase {
        OraclePhase::Unavailable => "lifecycle_unavailable",
        OraclePhase::Upcoming => "upcoming",
        OraclePhase::SourceSubmission => "terminal_sku_source_submission",
        OraclePhase::Scramble => "scramble_prelisting",
        OraclePhase::Placement => "monthly_source_selection",
        OraclePhase::KillChallenge => "source_challenges",
        OraclePhase::ResolutionFreeze => "source_freeze",
        OraclePhase::OpeningPrint => "opening_values",
        OraclePhase::GameMode => "live_source_updates",
        OraclePhase::MonthClose => "settlement",
    }
}

pub(super) fn guide_phase_label(phase: OraclePhase) -> &'static str {
    match phase {
        OraclePhase::Unavailable => "Lifecycle unavailable",
        OraclePhase::Upcoming => "Upcoming",
        OraclePhase::SourceSubmission => "Terminal-SKU source submission",
        OraclePhase::Scramble => "Scramble (pre-listing)",
        OraclePhase::Placement => "Monthly source selection",
        OraclePhase::KillChallenge => "Source challenges",
        OraclePhase::ResolutionFreeze => "Source freeze",
        OraclePhase::OpeningPrint => "Opening values",
        OraclePhase::GameMode => "Live source updates",
        OraclePhase::MonthClose => "Settlement",
    }
}

pub(super) fn guide_phase_summary(phase: OraclePhase) -> &'static str {
    match phase {
        OraclePhase::Unavailable => {
            "Petri could not verify this month's authoritative on-chain lifecycle."
        }
        OraclePhase::Upcoming => "Scramble has not started for this future month.",
        OraclePhase::SourceSubmission => {
            "Complete every required terminal-SKU source before placement can begin."
        }
        OraclePhase::Scramble => {
            "Build and freeze the source map before contracts are listed for trading."
        }
        OraclePhase::Placement => "Choose and back public sources before the month starts.",
        OraclePhase::KillChallenge => {
            "Review bad source definitions before the source map freezes."
        }
        OraclePhase::ResolutionFreeze => "Resolve open challenges and freeze this month's sources.",
        OraclePhase::OpeningPrint => {
            "Submit, challenge, and accept each source's starting value with archive evidence."
        }
        OraclePhase::GameMode => "Track source changes against each source's own starting value.",
        OraclePhase::MonthClose => "Use the final accepted source values to settle the month.",
    }
}

pub(super) fn guide_safe_action(
    command: &str,
    label: &str,
    permission_tier: &str,
) -> guide::GuideSafeActionSnapshot {
    guide::GuideSafeActionSnapshot {
        command: command.to_string(),
        label: label.to_string(),
        permission_tier: permission_tier.to_string(),
    }
}

pub(super) fn guide_ui_target(
    target_id: &str,
    label: &str,
    kind: &str,
    description: &str,
    permission_tier: &str,
    user_must_activate: bool,
) -> guide::GuideTargetSnapshot {
    guide::GuideTargetSnapshot {
        target_id: target_id.to_string(),
        label: label.to_string(),
        kind: kind.to_string(),
        description: description.to_string(),
        permission_tier: permission_tier.to_string(),
        user_must_activate,
    }
}

pub(super) fn guide_tool_result(
    step: u8,
    command: &guide::GuideCommand,
    result: &Result<(), String>,
) -> guide::GuideToolResult {
    let message = match result {
        Ok(()) => {
            "The command completed. Use the refreshed screen and offered targets for the next step."
                .to_string()
        }
        Err(message) => message.clone(),
    }
    .chars()
    .take(700)
    .collect();
    guide::GuideToolResult {
        step,
        command: command.tool_name().to_string(),
        target_id: command.target_id().map(str::to_string),
        ok: result.is_ok(),
        message,
    }
}

pub(super) fn guide_oracle_context_indices(
    tree: &OracleIndexTree,
    selected_index: usize,
    query: &str,
) -> Vec<usize> {
    let selected_index = selected_index.min(tree.nodes.len().saturating_sub(1));
    let mut indices = oracle_path_indices(tree, selected_index);
    if let Some(node) = tree.node(selected_index) {
        if let Some(parent) = node.parent
            && let Some(parent_node) = tree.node(parent)
        {
            indices.extend(parent_node.children.iter().copied());
        }
        indices.extend(node.children.iter().copied());
    }
    if !query.trim().is_empty() {
        indices.extend(tree.search_nodes(query));
    }
    indices.push(selected_index);
    let mut seen = HashSet::new();
    indices
        .into_iter()
        .filter(|index| seen.insert(*index))
        .take(12)
        .collect()
}

pub(super) fn guide_month_target_id(market_id: &str, expiry_id: &str) -> String {
    format!(
        "month:{}:{}",
        guide_target_segment(market_id),
        guide_target_segment(expiry_id)
    )
}

pub(super) fn guide_help_page_target_id(page_id: &str) -> String {
    format!("help-page:{page_id}")
}

pub(super) fn guide_trade_action_target_id(action: TradeAction, contract_id: &str) -> String {
    format!(
        "trade-action:{}:{contract_id}",
        match action {
            TradeAction::Buy => "buy",
            TradeAction::Sell => "sell",
        }
    )
}

pub(super) fn guide_oracle_action_target_id(action: OracleAction, node_id: &str) -> String {
    format!(
        "oracle-action:{}:{node_id}",
        guide_target_segment(action.label())
    )
}

pub(super) fn guide_oracle_form_mode_id(mode: OracleFormMode) -> &'static str {
    match mode {
        OracleFormMode::SourceProposal => "source-proposal",
        OracleFormMode::DefinitionEdit => "definition-edit",
        OracleFormMode::SourceSupport => "source-support",
        OracleFormMode::OpeningPrint => "opening-print",
        OracleFormMode::UpdateClaim => "update",
        OracleFormMode::Challenge => "challenge",
        OracleFormMode::RewardClaim => "reward-claim",
        OracleFormMode::StakeSettlement => "stake-settlement",
        OracleFormMode::AmbaDeposit => "amba-deposit",
        OracleFormMode::AmbaWithdraw => "amba-withdraw",
    }
}

pub(super) fn guide_oracle_form_id(form: &OracleFormDraft) -> String {
    format!("form:oracle:{}", guide_oracle_form_mode_id(form.mode))
}

pub(super) fn guide_field_id(label: &str) -> String {
    let mut id = String::new();
    let mut separator = false;
    for character in label.chars() {
        if character.is_ascii_alphanumeric() {
            if separator && !id.is_empty() {
                id.push('_');
            }
            separator = false;
            id.push(character.to_ascii_lowercase());
        } else {
            separator = true;
        }
    }
    id.trim_matches('_').to_string()
}

pub(super) fn guide_oracle_field_snapshot(field: &OracleFormField) -> guide::GuideFieldSnapshot {
    guide::GuideFieldSnapshot {
        field_id: guide_field_id(field.label),
        label: field.label.to_string(),
        value: (!field.value.is_empty()).then(|| field.value.clone()),
        required: field.required,
        editable_by_guide: field.editable,
    }
}

pub(super) fn validate_guide_trade_fields(
    fields: &[guide::GuideFieldValue],
) -> Result<(Option<String>, Option<String>), String> {
    let mut price = None;
    let mut quantity = None;
    let mut seen = HashSet::new();
    for field in fields {
        if !seen.insert(field.field_id.as_str()) {
            return Err("A ticket field was provided twice. Nothing changed.".to_string());
        }
        match field.field_id.as_str() {
            "price" => {
                let value = field
                    .value
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|value| value.is_finite() && *value > 0.0);
                let Some(value) = value else {
                    return Err("Price must be a positive number. Nothing changed.".to_string());
                };
                price = Some(
                    format_decimal(value, 6)
                        .trim_end_matches('0')
                        .trim_end_matches('.')
                        .to_string(),
                );
            }
            "quantity" => {
                let value = field
                    .value
                    .trim()
                    .parse::<u64>()
                    .ok()
                    .filter(|value| *value > 0 && *value <= 1_000_000);
                let Some(value) = value else {
                    return Err(
                        "Contracts must be a positive whole number. Nothing changed.".to_string(),
                    );
                };
                quantity = Some(value.to_string());
            }
            _ => {
                return Err(format!(
                    "{} is not an editable ticket field. Nothing changed.",
                    field.field_id
                ));
            }
        }
    }
    Ok((price, quantity))
}

pub(super) fn apply_guide_oracle_fields(
    form: &mut OracleFormDraft,
    fields: &[guide::GuideFieldValue],
) -> Result<(), String> {
    let mut seen = HashSet::new();
    let mut patches = Vec::new();
    for patch in fields {
        if !seen.insert(patch.field_id.as_str()) {
            return Err("An evidence field was provided twice. Nothing changed.".to_string());
        }
        if patch.value.chars().count() > 600 || patch.value.chars().any(char::is_control) {
            return Err("An evidence field is too long or invalid. Nothing changed.".to_string());
        }
        let Some((index, field)) = form
            .fields
            .iter()
            .enumerate()
            .find(|(_, field)| guide_field_id(field.label) == patch.field_id)
        else {
            return Err(format!(
                "{} is not a field in this evidence form. Nothing changed.",
                patch.field_id
            ));
        };
        if !field.editable {
            return Err(format!(
                "{} is filled by Petri and cannot be changed by the Guide. Nothing changed.",
                field.label
            ));
        }
        let value = patch.value.trim().to_string();
        if field.label.contains("URL") && !value.is_empty() && !value.starts_with("http") {
            return Err(format!(
                "{} must be a public URL. Nothing changed.",
                field.label
            ));
        }
        if matches!(
            field.label,
            "Stake/support" | "Stake" | "Stake/bond" | "Amount"
        ) && !value.is_empty()
            && value
                .parse::<f64>()
                .ok()
                .filter(|number| number.is_finite() && *number > 0.0)
                .is_none()
        {
            return Err(format!(
                "{} must be a positive amount. Nothing changed.",
                field.label
            ));
        }
        patches.push((index, value));
    }
    for (index, value) in patches {
        if let Some(field) = form.fields.get_mut(index) {
            field.value = value;
            form.field_selected = index;
        }
    }
    Ok(())
}

pub(super) fn guide_market_target_id(market_id: &str) -> String {
    format!("market:{}", guide_target_segment(market_id))
}

pub(super) fn guide_contract_target_id(detail: &DishDetail, quote: &OptionQuote) -> String {
    format!(
        "contract:{}:{}:{}:{}:{}",
        guide_target_segment(&detail.id),
        guide_target_segment(&detail.expiry_id),
        match quote.kind {
            OptionKind::Call => "call",
            OptionKind::Put => "put",
        },
        guide_target_segment(&quote.lower_strike),
        guide_target_segment(&quote.upper_strike),
    )
}

pub(super) fn guide_target_segment(value: &str) -> String {
    value
        .trim()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .take(64)
        .collect()
}

pub(super) fn guide_contract_snapshot(
    detail: &DishDetail,
    quote: &OptionQuote,
    lifecycle_tradeable: bool,
) -> guide::GuideContractSnapshot {
    let risk = trade_risk_preview(quote, TradeAction::Buy);
    let quote_available = positive_quote(quote.ask).is_some();
    let liquidity_available = quote
        .depth_usd
        .is_some_and(|depth| depth.is_finite() && depth > 0.0);
    let executable =
        lifecycle_tradeable && quote_available && liquidity_available && quote.prepare_eligible;
    guide::GuideContractSnapshot {
        contract_id: guide_contract_target_id(detail, quote),
        market_id: detail.id.clone(),
        expiry_id: detail.expiry_id.clone(),
        month: detail.expiry_label.clone(),
        kind: match quote.kind {
            OptionKind::Call => "call spread",
            OptionKind::Put => "put spread",
        }
        .to_string(),
        range: format!("{}-{}", quote.lower_strike, quote.upper_strike),
        bid: quote.bid,
        ask: quote.ask,
        mid: quote.mid,
        probability_itm: quote.probability_itm,
        probability_cap_hit: quote.probability_cap_hit,
        depth_usd: quote.depth_usd,
        volume: quote.volume,
        open_interest: quote.open_interest,
        maximum_loss: risk.map(|risk| risk.max_loss_per_contract),
        maximum_gain: risk.map(|risk| risk.max_gain_per_contract),
        maximum_payout: risk.map(|risk| risk.max_payout_per_contract),
        quote_available,
        liquidity_available,
        prepare_eligible: quote.prepare_eligible,
        executable,
        status: quote.status.clone(),
    }
}

pub(super) fn guide_trade_route_price(quote: &OptionQuote, action: TradeAction) -> Option<f64> {
    match action {
        TradeAction::Buy => positive_quote(quote.ask),
        TradeAction::Sell => positive_quote(quote.bid),
    }
}

pub(super) fn guide_trade_action_is_executable(quote: &OptionQuote, action: TradeAction) -> bool {
    guide_trade_route_price(quote, action).is_some()
        && quote
            .depth_usd
            .is_some_and(|depth| depth.is_finite() && depth > 0.0)
        && quote.prepare_eligible
}

pub(super) fn guide_node_snapshot(
    tree: &OracleIndexTree,
    index: usize,
) -> Option<guide::GuideNodeSnapshot> {
    let node = tree.node(index)?;
    let path = oracle_path_indices(tree, index)
        .into_iter()
        .filter_map(|index| tree.node(index).map(|node| node.label.as_str()))
        .collect::<Vec<_>>()
        .join(" / ");
    Some(guide::GuideNodeSnapshot {
        node_id: node.node_id.clone(),
        label: node.label.clone(),
        kind: match node.kind {
            OracleNodeKind::Market => "market",
            OracleNodeKind::Generation => "generation",
            OracleNodeKind::FormFactor => "form_factor",
            OracleNodeKind::RowBucket => "row_bucket",
            OracleNodeKind::TerminalPin => "source",
        }
        .to_string(),
        path,
        description: node.description.clone(),
        weight_pct: Some(if node.kind == OracleNodeKind::TerminalPin {
            node.row_weight_pct
        } else {
            node.weight_pct
        }),
    })
}

pub(super) fn exact_tree_node_id_for_live_source(
    tree: Option<&OracleIndexTree>,
    source_id: &str,
) -> Option<String> {
    tree?
        .nodes
        .iter()
        .find(|node| node.node_id == source_id)
        .map(|node| node.node_id.clone())
}

pub(super) fn humanize_guide_label(value: &str) -> String {
    let value = value.replace(['_', '-'], " ");
    let mut characters = value.trim().chars();
    match characters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
        None => "Review".to_string(),
    }
}
