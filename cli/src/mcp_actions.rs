//! Public agent actions share CLI dispatch. No shell, packet, signing or
//! wallet-locator escape hatch: the resident action is the connected wallet's
//! public address (reads go through the MCP server's CLI wrappers). Trades and oracle writes sign with the local wallet and are
//! run by the user in a terminal.
use crate::{backend::CliError, cli::Cli};
use clap::Parser;
use serde_json::{Value, json};
use std::cell::RefCell;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Identity,
}

struct Field {
    name: String,
    flag: Option<String>,
    schema: Value,
    required: bool,
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
}
thread_local! { static CONTEXT: RefCell<Option<Context>> = const { RefCell::new(None) }; }
thread_local! { static OUTPUT: RefCell<Option<Value>> = const { RefCell::new(None) }; }
thread_local! { static CAPTURING: RefCell<bool> = const { RefCell::new(false) }; }
struct OutputScope;
impl Drop for OutputScope {
    fn drop(&mut self) {
        CAPTURING.with(|slot| *slot.borrow_mut() = false);
        OUTPUT.with(|slot| *slot.borrow_mut() = None);
    }
}
pub(crate) fn capture_output(value: &Value) -> bool {
    if !CAPTURING.with(|slot| *slot.borrow()) {
        return false;
    }
    OUTPUT.with(|slot| *slot.borrow_mut() = Some(public_output(value)));
    true
}
pub(crate) fn invoke_captured(cli: &Cli, name: &str, raw: &str) -> Result<Value, CliError> {
    if CAPTURING.with(|slot| *slot.borrow()) || active() {
        return Err(CliError::new("Nested agent action is unavailable."));
    }
    CAPTURING.with(|slot| *slot.borrow_mut() = true);
    let _capture = OutputScope;
    invoke(cli, name, raw)?;
    OUTPUT
        .with(|slot| slot.borrow_mut().take())
        .ok_or_else(|| CliError::new("Agent action returned no public result."))
}
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
/// Reading the wallet's public key is allowed only inside the identity action.
pub fn identity_blocked() -> bool {
    restricted() && !context().is_some_and(|c| c.mode == Mode::Identity)
}
/// MCP never signs.
pub fn signing_blocked() -> bool {
    restricted()
}
/// Kept for the signer boundary: no action binds an owner, so any owner passes.
pub fn check_owner(_owner: &str) -> Result<(), CliError> {
    Ok(())
}

fn tool(name: &str, description: &str, mode: Mode, command: &[&str], fields: Vec<Field>) -> Tool {
    Tool {
        name: name.into(),
        description: description.into(),
        mode,
        command: command.iter().map(|s| (*s).into()).collect(),
        fields,
    }
}
fn tools() -> Vec<Tool> {
    vec![tool(
        "wallet.address",
        "Read the locally connected wallet's public address. No signature or secret output.",
        Mode::Identity,
        &["wallet", "address"],
        vec![],
    )]
}

fn definition(t: &Tool) -> Value {
    let properties: serde_json::Map<String, Value> = t
        .fields
        .iter()
        .map(|f| (f.name.clone(), f.schema.clone()))
        .collect();
    json!({"name":t.name,"title":t.name,"description":t.description,
        "inputSchema":{"type":"object","properties":properties,"required":t.fields.iter().filter(|f|f.required).map(|f|&f.name).collect::<Vec<_>>(),"additionalProperties":false},
        "annotations":{"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true,"openWorldHint":true},
        "status":"wired_shared_public_action","bridgeVersion":1,
        "cli":["petri","--json","mcp","invoke",t.name,"--request-stdin"]})
}
pub fn manifest() -> Value {
    json!({"ok":true,"protocol":"petri-public-actions.v1","transport":"resident-v1","tools":tools().iter().map(definition).collect::<Vec<_>>()})
}
fn valid_text(value: &Value, schema: &Value) -> bool {
    value.as_str().is_some_and(|s| {
        !s.contains('\0')
            && !s.chars().any(char::is_control)
            && !s.is_empty()
            && s.len() <= schema["maxLength"].as_u64().unwrap_or(160) as usize
    })
}

pub fn invoke_stdin(cli: &Cli, name: &str) -> Result<(), CliError> {
    use std::io::Read;
    let mut raw = String::new();
    std::io::stdin()
        .lock()
        .take(24 * 1024 + 1)
        .read_to_string(&mut raw)
        .map_err(|_| CliError::new("Could not read the bounded public action request."))?;
    invoke(cli, name, &raw)
}

fn invoke(cli: &Cli, name: &str, raw: &str) -> Result<(), CliError> {
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
            Some(v) if !valid_text(v, &f.schema) => {
                return Err(CliError::new(format!(
                    "{} has an invalid type, value or size.",
                    f.name
                )));
            }
            _ => {}
        }
    }
    CONTEXT.with(|slot| *slot.borrow_mut() = Some(Context { mode: t.mode }));
    let _scope = Scope;
    let mut argv = vec!["petri".to_owned(), "--json".into()];
    argv.extend(t.command);
    for f in &t.fields {
        if let (Some(flag), Some(value)) = (&f.flag, args.get(&f.name).and_then(Value::as_str)) {
            argv.push(format!("{flag}={value}"));
        }
    }
    let mut child = Cli::try_parse_from(argv)
        .map_err(|_| CliError::new("Action fields do not match the public CLI grammar."))?;
    // Preserve the selected connection and local wallet config; never accept them from tool arguments.
    child.solana_config = cli.solana_config.clone();
    child.keypair = cli.keypair.clone();
    child.profile = cli.profile;
    child.rpc = cli.rpc.clone();
    child.futures_api = cli.futures_api.clone();
    child.deployment = cli.deployment.clone();
    crate::dispatch_cli(child)
}

/// Restrict agent output to public results; private material stays in Petri storage.
pub fn public_output(value: &Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .filter(|(k, _)| {
                    !matches!(
                        k.as_str(),
                        "secretSaltHex"
                            | "secretSalt"
                            | "secret_salt"
                            | "salt"
                            | "keypair"
                            | "revealMaterial"
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_lists_only_reads_and_the_wallet_address() {
        let manifest = manifest();
        let names: Vec<&str> = manifest["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["wallet.address"]);
        assert!(
            manifest["tools"]
                .as_array()
                .unwrap()
                .iter()
                .all(|t| t["annotations"]["readOnlyHint"] == true)
        );
    }

    #[test]
    fn private_fields_never_reach_agents() {
        let value = json!({"a": 1, "secretSalt": "x", "nested": [{"revealMaterial": "/p"}]});
        assert_eq!(public_output(&value), json!({"a": 1, "nested": [{}]}));
    }

    #[test]
    fn mcp_context_blocks_signing() {
        let _scope = Scope;
        assert!(!active());
        CONTEXT.with(|slot| {
            *slot.borrow_mut() = Some(Context {
                mode: Mode::Identity,
            })
        });
        assert!(active());
        assert!(!identity_blocked());
        assert!(signing_blocked());
    }
}
