//! Public agent actions share CLI dispatch and SDK execution. No shell/packet escape hatch.
//! Authority is scoped to one validated request in this process, never an environment opt-in.
use crate::{
    backend::{BackendClient, CliError},
    cli::Cli,
    operation_journal,
};
use clap::{Parser, ValueEnum};
use serde_json::{Value, json};
use std::cell::RefCell;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Read,
    Identity,
    Prepare,
    Execute,
}

struct Field {
    name: String,
    flag: Option<String>,
    schema: Value,
    required: bool,
    repeat: bool,
}
struct Tool {
    name: String,
    description: String,
    mode: Mode,
    command: Vec<String>,
    fields: Vec<Field>,
}
#[derive(Clone)]
struct Context {
    mode: Mode,
    owner: Option<String>,
    operation: Option<String>,
    digest: Option<String>,
}
thread_local! { static CONTEXT: RefCell<Option<Context>> = const { RefCell::new(None) }; }
struct Scope;
impl Drop for Scope {
    fn drop(&mut self) {
        CONTEXT.with(|c| *c.borrow_mut() = None);
    }
}
fn context() -> Option<Context> {
    CONTEXT.with(|c| c.borrow().clone())
}
pub fn active() -> bool {
    context().is_some()
}
fn restricted() -> bool {
    active() || std::env::var("PETRI_MCP_READ_ONLY").is_ok_and(|v| v == "1")
}
pub fn identity_blocked() -> bool {
    restricted()
        && !context()
            .is_some_and(|c| matches!(c.mode, Mode::Identity | Mode::Prepare | Mode::Execute))
}
pub fn signing_blocked() -> bool {
    restricted()
        && !context()
            .is_some_and(|c| c.mode == Mode::Execute && c.operation.is_some() && c.digest.is_some())
}
pub fn preparation_blocked() -> bool {
    restricted() && !context().is_some_and(|c| matches!(c.mode, Mode::Prepare | Mode::Execute))
}
pub fn preparing() -> bool {
    context().is_some_and(|c| c.mode == Mode::Prepare)
}
pub fn check_owner(owner: &str) -> Result<(), CliError> {
    if context()
        .and_then(|c| c.owner)
        .is_some_and(|expected| expected != owner)
    {
        return Err(CliError::new(
            "The connected wallet differs from ownerPubkey. Nothing was signed or sent.",
        ));
    }
    Ok(())
}
/// Called at the shared executor boundary, after native admission and before signer loading.
pub fn authorize_execution(id: &str, digest: &str, owner: &str) -> Result<(), CliError> {
    if !restricted() {
        return Ok(());
    }
    let Some(c) = context() else {
        return Err(CliError::new(
            "MCP execution requires a typed approved operation.",
        ));
    };
    if c.mode != Mode::Execute
        || c.operation.as_deref() != Some(id)
        || c.digest.as_deref() != Some(digest)
        || c.owner.as_deref() != Some(owner)
    {
        return Err(CliError::new(
            "Execution does not match the exact approved operation, digest and wallet. Nothing was signed.",
        ));
    }
    Ok(())
}

fn string() -> Value {
    json!({"type":"string","minLength":1,"maxLength":4096})
}
fn identifier() -> Value {
    json!({"type":"string","minLength":1,"maxLength":160})
}
fn decimal() -> Value {
    json!({"type":"string","minLength":1,"maxLength":40,"description":"Exact decimal string; never a floating-point JSON number"})
}
fn digest() -> Value {
    json!({"type":"string","minLength":64,"maxLength":64,"description":"Canonical lowercase hexadecimal digest"})
}
// Callers enumerate the existing exposed values explicitly. A new CLI variant
// does not silently expand MCP's allowlist.
fn choice<T: ValueEnum>(variants: &[T]) -> Value {
    let values: Vec<String> = variants
        .iter()
        .map(|value| {
            value
                .to_possible_value()
                .expect("public choice")
                .get_name()
                .to_string()
        })
        .collect();
    json!({"type":"string","enum":values})
}
fn field(name: &str, flag: Option<&str>, schema: Value, required: bool) -> Field {
    Field {
        name: name.into(),
        flag: flag.map(str::to_owned),
        schema,
        required,
        repeat: false,
    }
}
fn option(name: &str, flag: &str, schema: Value) -> Field {
    field(name, Some(flag), schema, true)
}
fn repeated(name: &str, flag: &str, max: usize) -> Field {
    Field {
        name: name.into(),
        flag: Some(flag.into()),
        schema: json!({"type":"array","minItems":1,"maxItems":max,"items":string()}),
        required: true,
        repeat: true,
    }
}
fn tool(
    name: &str,
    description: &str,
    mode: Mode,
    command: &[&str],
    mut fields: Vec<Field>,
) -> Tool {
    if matches!(mode, Mode::Prepare | Mode::Execute) {
        fields.insert(0, field("ownerPubkey", None, identifier(), true));
    }
    Tool {
        name: name.into(),
        description: description.into(),
        mode,
        command: command.iter().map(|s| (*s).into()).collect(),
        fields,
    }
}
fn tools() -> Vec<Tool> {
    let mut tools = vec![
        tool(
            "wallet.address",
            "Read the locally connected wallet's public address. No signature or secret output.",
            Mode::Identity,
            &["wallet", "address"],
            vec![],
        ),
        tool(
            "oracle.carry",
            "Read current carry lineage, source provenance and progress. Maintenance remains outside the public hosted route.",
            Mode::Read,
            &["oracle", "carry"],
            vec![
                option("marketId", "--market", identifier()),
                option("expiryId", "--expiry", identifier()),
                field("sourceId", Some("--source"), digest(), false),
            ],
        ),
        tool(
            "commitments.list",
            "List public references to this owner's locally saved Oracle commitments; no salts or private export/import.",
            Mode::Read,
            &["commitments", "list"],
            vec![option("ownerPubkey", "--owner", identifier())],
        ),
        tool(
            "operations.execute",
            "Execute exactly one previously reviewed operation after explicit user authorization. Confirm owner, operationId and preparedPlanDigest. May spend funds; uncertain results must be recovered, never retried blindly.",
            Mode::Execute,
            &[],
            vec![
                field("operationId", None, digest(), true),
                field("preparedPlanDigest", None, digest(), true),
                field(
                    "approved",
                    None,
                    json!({"type":"boolean","const":true,"description":"True only after the user explicitly authorizes this exact reviewed action"}),
                    true,
                ),
            ],
        ),
    ];
    for name in ["operations.show", "operations.status", "operations.resume"] {
        tools.push(tool(name, "Read/reconcile one exact owner's operation. Status and resume may update local recovery references but never sign or resubmit.", Mode::Read, &[], vec![field("ownerPubkey",None,identifier(),true),field("operationId",None,digest(),true)]));
    }
    for (name, side) in [
        ("trade.buy", "buy"),
        ("trade.sell", "sell"),
        ("trade.quote", "buy"),
    ] {
        let mut fields = vec![
            option("marketId", "--market", identifier()),
            option("expiryId", "--expiry", identifier()),
            option("quantity", "--quantity", decimal()),
            option("limitPrice", "--limit-price", decimal()),
        ];
        let command = if name == "trade.quote" {
            fields.push(field(
                "side",
                Some("--side"),
                choice(&[crate::cli::TradeSide::Buy, crate::cli::TradeSide::Sell]),
                false,
            ));
            vec!["trades", "quote"]
        } else {
            vec!["trades", side]
        };
        tools.push(tool(name,"Prepare a bounded option ticket and save its exact review. Sell transfers owned long options, never a naked short. Does not sign; approve separately with operations.execute.",Mode::Prepare,&command,fields));
    }
    tools.push(tool("trade.prepare","Prepare an expert exact-input swap without signing. Returns a saved operation for separate approval.",Mode::Prepare,&["trades","prepare"],vec![option("marketAddress","--market",identifier()),option("direction","--direction",choice(&[crate::cli::CollectiveSwapDirectionValue::QuoteForOption,crate::cli::CollectiveSwapDirectionValue::OptionForQuote])),option("amountIn","--amount-in",decimal()),option("minimumAmountOut","--minimum-amount-out",decimal()),option("limitBinId","--limit-bin-id",json!({"type":"integer","minimum":0,"maximum":65535}))]));
    for action in crate::participation::ACTIONS {
        let name = action
            .to_possible_value()
            .expect("public action value")
            .get_name()
            .to_owned();
        let mut fields = Vec::new();
        let series = matches!(action.family(), crate::portable_operation::Family::Oracle);
        if series {
            fields.extend([
                option("marketId", "--market", identifier()),
                option("expiryId", "--expiry", identifier()),
            ]);
        }
        for (key, label, optional) in action.fields() {
            let mut schema = string();
            schema["description"] = json!(label);
            fields.push(field(
                key,
                Some(&format!("--field={key}")),
                schema,
                !optional,
            ));
        }
        let family = if name.contains("collateral") {
            "wallet"
        } else if matches!(
            action,
            crate::participation::Action::Stake
                | crate::participation::Action::ActivateStake
                | crate::participation::Action::Unstake
                | crate::participation::Action::CompleteUnstake
        ) {
            "staking"
        } else {
            "oracle"
        };
        tools.push(tool(&format!("{family}.{}.prepare",name.replace('-',"_")),&format!("Prepare {} using the shared CLI/TUI action and native SDK. Nothing is signed. Commit secrets are generated and retained locally; reveal uses commitmentId. Review then call operations.execute.",action.label()),Mode::Prepare,&["participate",&name],fields));
    }
    for action in ["add", "remove", "close-position"] {
        tools.push(tool(&format!("liquidity.{}.prepare",action.replace('-',"_")),"Prepare manager liquidity using exact string amounts and native SDK validation; separate from writer-owned liquidity. Approve the saved operation separately.",Mode::Prepare,&["liquidity",action],vec![option("marketId","--market",identifier()),option("expiryId","--expiry",identifier()),option("positionNonce","--position-nonce",decimal()),repeated("entries","--entry",128)]));
    }
    for action in ["deposit", "withdraw"] {
        tools.push(tool(&format!("writers.{action}.prepare"),"Prepare exact writer principal movement without signing; review and approve the saved operation separately.",Mode::Prepare,&["writers",action],vec![option("sleeve","--sleeve",identifier()),option("amountAtoms","--amount",decimal())]));
    }
    tools.push(tool(
        "writers.refund.prepare",
        "Prepare refund of one historical auction bid to its canonical destination.",
        Mode::Prepare,
        &["writers", "refund"],
        vec![
            option("auction", "--auction", identifier()),
            option("bid", "--bid", identifier()),
        ],
    ));
    tools.push(tool("writers.bid.prepare","Prepare a historical auction bid only when the same public CLI action is admitted by the current SDK and action mask.",Mode::Prepare,&["writers","bid"],vec![option("auction","--auction",identifier()),option("seriesIndex","--series-index",json!({"type":"integer","minimum":0,"maximum":19})),option("priceAtoms","--price",decimal()),option("amountAtoms","--amount",decimal())]));
    for action in ["initialize", "add", "remove", "sweep"] {
        let mut fields = vec![
            option("sleeve", "--sleeve", identifier()),
            option(
                "seriesIndex",
                "--series-index",
                json!({"type":"integer","minimum":0,"maximum":19}),
            ),
        ];
        if action == "add" {
            fields.push(option("issueAmountAtoms", "--issue-amount", decimal()));
        }
        if matches!(action, "add" | "remove") {
            fields.push(repeated("bins", "--bin", 8));
        }
        tools.push(tool(&format!("writers.liquidity_{action}.prepare"),"Prepare writer-owned liquidity using current custody, inventory and buyback limits. Does not sign.",Mode::Prepare,&["writers",&format!("liquidity-{action}")],fields));
    }
    tools.push(tool(
        "writers.close_begin.prepare",
        "Prepare only the first stage of a writer close. This does not complete the close.",
        Mode::Prepare,
        &["writers", "close"],
        vec![
            option("sleeve", "--sleeve", identifier()),
            option("amountAtoms", "--amount", decimal()),
            option("minimumWithdrawalAtoms", "--minimum-withdrawal", decimal()),
        ],
    ));
    tools.push(tool("writers.close_advance.prepare","Prepare the single next permitted close stage. Inspect close status again after execution; never alias cancellation to advancement.",Mode::Prepare,&["writers","close"],vec![option("closeRequest","--close-request",identifier())]));
    tools.push(tool("writers.claim.prepare","Prepare a current collective-long or Flat-residual settlement claim using exact current eligibility.",Mode::Prepare,&["writers","claim"],vec![option("sleeve","--sleeve",identifier()),option("variant","--variant",choice(&[crate::cli::WriterClaimVariant::CollectiveLong,crate::cli::WriterClaimVariant::FlatResidual])),field("seriesIndex",Some("--series-index"),json!({"type":"integer","minimum":0,"maximum":19}),false),option("amountAtoms","--amount",decimal())]));
    tools.push(tool(
        "writers.transfer_flat.prepare",
        "Prepare a Flat ownership transfer to the exact destination wallet. Does not sign.",
        Mode::Prepare,
        &["writers", "transfer-flat"],
        vec![
            option("sleeve", "--sleeve", identifier()),
            option("destinationOwner", "--destination", identifier()),
            option("amountAtoms", "--amount", decimal()),
        ],
    ));
    tools
}

fn definition(t: &Tool) -> Value {
    let properties: serde_json::Map<String, Value> = t
        .fields
        .iter()
        .map(|f| (f.name.clone(), f.schema.clone()))
        .collect();
    let read_only = t.mode == Mode::Identity
        || (t.mode == Mode::Read
            && !matches!(t.name.as_str(), "operations.status" | "operations.resume"));
    json!({"name":t.name,"title":t.name,"description":t.description,
        "inputSchema":{"type":"object","properties":properties,"required":t.fields.iter().filter(|f|f.required).map(|f|&f.name).collect::<Vec<_>>(),"additionalProperties":false},
        "annotations":{"readOnlyHint":read_only,"destructiveHint":t.mode==Mode::Execute,"idempotentHint":read_only,"openWorldHint":true},
        "status":"wired_shared_public_action","bridgeVersion":1,
        "cli":["petri","--json","mcp","invoke",t.name,"--request-stdin"]})
}
pub fn manifest() -> Value {
    json!({"ok":true,"protocol":"petri-public-actions.v1","tools":tools().iter().map(definition).collect::<Vec<_>>()})
}
fn validate(value: &Value, schema: &Value) -> bool {
    if let Some(expected) = schema.get("const") {
        if value != expected {
            return false;
        }
    }
    if let Some(values) = schema["enum"].as_array() {
        if !values.contains(value) {
            return false;
        }
    }
    match schema["type"].as_str() {
        Some("string") => value.as_str().is_some_and(|s| {
            !s.contains('\0')
                && !s.chars().any(char::is_control)
                && s.len() >= schema["minLength"].as_u64().unwrap_or(0) as usize
                && s.len() <= schema["maxLength"].as_u64().unwrap_or(4096) as usize
        }),
        Some("boolean") => value.is_boolean(),
        Some("integer") => value.as_u64().is_some_and(|v| {
            v >= schema["minimum"].as_u64().unwrap_or(0)
                && v <= schema["maximum"].as_u64().unwrap_or(u64::MAX)
        }),
        Some("array") => value.as_array().is_some_and(|a| {
            a.len() >= schema["minItems"].as_u64().unwrap_or(0) as usize
                && a.len() <= schema["maxItems"].as_u64().unwrap_or(128) as usize
                && a.iter().all(|v| validate(v, &schema["items"]))
        }),
        _ => false,
    }
}

pub fn invoke_stdin(cli: &Cli, backend: &BackendClient, name: &str) -> Result<(), CliError> {
    use std::io::Read;
    let mut raw = String::new();
    std::io::stdin()
        .lock()
        .take(24 * 1024 + 1)
        .read_to_string(&mut raw)
        .map_err(|_| CliError::new("Could not read the bounded public action request."))?;
    invoke(cli, backend, name, &raw)
}

fn invoke(cli: &Cli, backend: &BackendClient, name: &str, raw: &str) -> Result<(), CliError> {
    if raw.len() > 24 * 1024 || active() {
        return Err(CliError::new("Nested or oversized MCP action request."));
    }
    let t = tools()
        .into_iter()
        .find(|t| t.name == name)
        .ok_or_else(|| CliError::new("Unknown public MCP action."))?;
    let args: Value =
        serde_json::from_str(raw).map_err(|_| CliError::new("Action requires a JSON object."))?;
    let object = args
        .as_object()
        .ok_or_else(|| CliError::new("Action requires a JSON object."))?;
    if object
        .keys()
        .any(|key| !t.fields.iter().any(|f| &f.name == key))
    {
        return Err(CliError::new("Undeclared action fields are not accepted."));
    }
    for f in &t.fields {
        match args.get(&f.name) {
            None if f.required => return Err(CliError::new(format!("{} is required.", f.name))),
            Some(v) if !validate(v, &f.schema) => {
                return Err(CliError::new(format!(
                    "{} has an invalid type, value or size.",
                    f.name
                )));
            }
            _ => {}
        }
    }
    let owner = args["ownerPubkey"]
        .as_str()
        .map(|s| crate::request_validation::canonical_pubkey_string(s, "ownerPubkey"))
        .transpose()?;
    let c = Context {
        mode: t.mode,
        owner,
        operation: args["operationId"].as_str().map(str::to_owned),
        digest: args["preparedPlanDigest"].as_str().map(str::to_owned),
    };
    CONTEXT.with(|slot| *slot.borrow_mut() = Some(c));
    let _scope = Scope;
    if name.starts_with("operations.") {
        let id = args["operationId"]
            .as_str()
            .ok_or_else(|| CliError::new("Operation ID required."))?;
        let record = operation_journal::load(id)?;
        let owner = context()
            .and_then(|c| c.owner)
            .ok_or_else(|| CliError::new("An exact operation owner is required."))?;
        record.require_scope(backend, &owner)?;
        let payload = if t.mode == Mode::Execute {
            authorize_execution(id, &record.prepared_plan_digest, &record.owner)?;
            if record.state != "prepared" || record.signature.is_some() {
                return Err(CliError::new(
                    "This operation was already attempted. Recover its status; no replay was made.",
                ));
            }
            match record.channel.as_str() {
                "trade" => crate::trade_service::execute_reviewed(cli, backend, id)?,
                "writer" => execute_writer(cli, backend, &record)?,
                "collateral" | "oracle" | "liquidity" => crate::portable_operation::execute(
                    &crate::app_context::build_onchain_config(cli)?,
                    backend,
                    id,
                    true,
                )?,
                _ => return Err(CliError::new("Unknown public operation family.")),
            }
        } else if name == "operations.show" {
            json!({"ok":true,"operation":record.public_value(),"review":record.request})
        } else {
            operation_journal::recover(backend, id)?
        };
        return crate::emit_output(
            cli,
            &payload,
            serde_json::to_string_pretty(&payload).unwrap_or_default(),
        );
    }
    let mut argv = vec!["petri".to_owned(), "--json".into()];
    argv.extend(t.command);
    for f in &t.fields {
        let (Some(flag), Some(value)) = (&f.flag, args.get(&f.name)) else {
            continue;
        };
        let values = if f.repeat {
            value.as_array().cloned().unwrap_or_default()
        } else {
            vec![value.clone()]
        };
        for value in values {
            let text = value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string());
            if let Some(key) = flag.strip_prefix("--field=") {
                argv.push(format!("--field={key}={text}"));
            } else {
                argv.push(format!("{flag}={text}"));
            }
        }
    }
    let mut child = Cli::try_parse_from(argv)
        .map_err(|_| CliError::new("Action fields do not match the public CLI grammar."))?;
    // Preserve the selected connection and local signer config; never accept them from tool arguments.
    child.backend_url = cli.backend_url.clone();
    child.cluster = cli.cluster.clone();
    child.solana_config = cli.solana_config.clone();
    child.keypair = cli.keypair.clone();
    child.commitment = cli.commitment.clone();
    crate::dispatch_cli(child)
}

/// Restrict agent output to public reviews and receipts; private material stays in Petri storage.
pub fn public_output(value: &Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .filter(|(k, _)| {
                    !matches!(
                        k.as_str(),
                        "prepared"
                            | "secretSaltHex"
                            | "secretSalt"
                            | "secret_salt"
                            | "salt"
                            | "keypair"
                            | "serializedTransactionBase64"
                            | "transactionBase64"
                            | "signedTransactionBase64"
                    )
                })
                .map(|(k, v)| (k.clone(), public_output(v)))
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(public_output).collect()),
        _ => value.clone(),
    }
}

fn execute_writer(
    cli: &Cli,
    backend: &BackendClient,
    record: &operation_journal::OperationRecord,
) -> Result<Value, CliError> {
    use ameba_sdk::WriterOperationKind as W;
    crate::current_release::require_current_write_release()?;
    let config = crate::app_context::build_onchain_config(cli)?;
    let context = crate::current_operation::observe_current_write_context(&config)?;
    let owner = crate::wallet_signer::signer_pubkey(&config)?;
    record.require_scope(backend, &owner)?;
    let encoded = serde_json::to_string(crate::writer_operation_plan(&record.prepared)?)
        .map_err(|_| CliError::new("Invalid saved writer plan."))?;
    let mut close = None;
    let (admitted, deadline, operation) = if record.request.get("destinationOwner").is_some() {
        let plan: ameba_sdk::FlatTransferOperationPlan = serde_json::from_str(&encoded)
            .map_err(|_| CliError::new("Invalid saved Flat plan."))?;
        let setup = crate::current_operation::rebuild_flat_transfer_setup(&config, &plan)?;
        let admitted = ameba_sdk::parse_current_governed_flat_transfer_operation_json_v1(
            &context, &encoded, &setup,
        )
        .map_err(|e| CliError::new(e.to_string()))?;
        let validated = admitted
            .flat_operation()
            .ok_or_else(|| CliError::new("Expected Flat transfer."))?;
        let expected = json!({"owner":owner,"sleeve":record.request["sleeve"],"destinationOwner":record.request["destinationOwner"],"amountAtoms":record.request["amountAtoms"]});
        if serde_json::to_value(&validated.plan.semantic)
            .map_err(|_| CliError::new("Invalid Flat semantics."))?
            != expected
        {
            return Err(CliError::new(
                "Saved Flat transfer differs from the reviewed request.",
            ));
        }
        crate::require_current_writer_action(
            backend,
            &owner,
            expected["sleeve"]
                .as_str()
                .ok_or_else(|| CliError::new("Missing sleeve."))?,
            crate::writer_action_mask::CollectiveActionKind::FlatTransfer,
        )?;
        let operation = validated.plan.operation.clone();
        (admitted, None, operation)
    } else {
        let plan: ameba_sdk::WriterOperationPlan = serde_json::from_str(&encoded)
            .map_err(|_| CliError::new("Invalid saved writer plan."))?;
        let setup = crate::current_operation::rebuild_writer_operation_setup(&config, &plan)?;
        let admitted =
            ameba_sdk::parse_current_governed_writer_operation_json_v1(&context, &encoded, &setup)
                .map_err(|e| CliError::new(e.to_string()))?;
        let validated = admitted
            .writer_operation()
            .ok_or_else(|| CliError::new("Expected writer operation."))?;
        if !matches!(
            validated.plan.operation,
            W::Deposit
                | W::WithdrawPrincipal
                | W::AuctionRefund
                | W::WriterLiquidityInitialize
                | W::WriterLiquidityAdd
                | W::WriterLiquidityRemove
                | W::WriterLiquiditySweep
                | W::Bid
                | W::CloseBegin
                | W::CloseBasket
                | W::CloseFinalize
                | W::SettlementClaimCollective
                | W::SettlementClaimFlat
        ) {
            return Err(CliError::new("Not a supported public writer action."));
        }
        ameba_sdk::require_expected_writer_semantic(
            &validated,
            validated.plan.operation,
            &record.request,
        )
        .map_err(|e| CliError::new(e.to_string()))?;
        let flow = match validated.plan.operation {
            W::CloseBegin => crate::WriterCloseSubmission::Begin {
                asserted_request: None,
            },
            W::CloseBasket | W::CloseFinalize => crate::WriterCloseSubmission::Forward {
                close_request: record.request["closeRequest"]
                    .as_str()
                    .ok_or_else(|| CliError::new("Missing exact close request."))?,
            },
            _ => crate::WriterCloseSubmission::None,
        };
        close = crate::validate_writer_close_submission(&record.prepared, &validated, flow)?;
        let deadline = crate::writer_close_execution_deadline(&record.prepared, &validated, flow)?;
        crate::require_current_writer_operation_action(backend, &owner, &validated)?;
        let operation = crate::writer_operation_wire_name(validated.plan.operation).to_owned();
        (admitted, deadline, operation)
    };
    authorize_execution(
        admitted.operation_id(),
        admitted.prepared_plan_digest(),
        &admitted.payer().to_string(),
    )?;
    let receipt = crate::current_operation::sign_submit_validated_operation(
        &config,
        backend,
        crate::endpoints::writer_operation_submit(),
        &crate::endpoints::writer_operation_status(&record.operation_id),
        &operation,
        &admitted,
        deadline,
    )?;
    Ok(json!({"ok":true,"receipt":receipt,"close":close}))
}
