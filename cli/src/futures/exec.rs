//! Sending: locally built transactions through RPC (the owner pays), or
//! sponsor-prepared transactions from the API (checked, then signed here).

use std::str::FromStr;

use base64::{Engine, engine::general_purpose::STANDARD as B64};
use serde_json::Value;
use solana_program::pubkey::Pubkey;
use solana_signature::Signature;
use solana_signer::Signer;
use solana_transaction::Transaction;

use super::{
    ctx::Ctx,
    error::{Error, Raiser, Result, bail, custom_error_name, hint},
    market::Deployment,
    txbuild::{PACKET_SIZE, Plan},
};

/// The owner needs this much SOL to pay fees (and wallet rent) itself.
pub const LOCAL_MIN_LAMPORTS: u64 = 10_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    Local,
    Sponsored,
}

impl Route {
    pub fn word(self) -> &'static str {
        match self {
            Route::Local => "local",
            Route::Sponsored => "sponsored",
        }
    }
}

/// A landed transaction.
#[derive(Clone, Debug)]
pub struct Outcome {
    pub signature: String,
    pub route: Route,
    pub steps: Vec<String>,
    pub logs: Vec<String>,
    pub slot: u64,
    /// The transactions that moved compressed USDC into the wallet first.
    pub before: Vec<String>,
}

impl Outcome {
    pub fn json(&self) -> Value {
        let mut v = serde_json::json!({
            "signature": self.signature,
            "route": self.route.word(),
            "slot": self.slot,
            "instructions": self.steps,
        });
        if !self.before.is_empty() {
            v["before"] = serde_json::json!(self.before);
        }
        v
    }
}

fn raiser(logs: &[String], dep: &Deployment) -> Raiser {
    let line = logs
        .iter()
        .rev()
        .find(|l| l.contains("failed: custom program error"));
    let Some(line) = line else {
        return Raiser::Unknown;
    };
    let program = line
        .strip_prefix("Program ")
        .and_then(|r| r.split_whitespace().next())
        .and_then(|p| Pubkey::from_str(p).ok());
    match program {
        Some(p) if Some(p) == dep.futures => Raiser::Futures,
        Some(p) if p == spl_token::id() => Raiser::Token,
        _ => Raiser::Unknown,
    }
}

/// The program log lines worth showing with an error.
pub fn useful_logs(logs: &[String]) -> Vec<String> {
    let mut out: Vec<String> = logs
        .iter()
        .filter(|l| l.contains("Program log:") || l.contains("failed"))
        .cloned()
        .collect();
    let start = out.len().saturating_sub(6);
    out.drain(..start);
    out
}

/// A readable error for a transaction error value and its logs.
pub fn explain(err: &Value, logs: &[String], steps: &[String], dep: &Deployment) -> Error {
    let step = |i: u64| {
        steps
            .get(i as usize)
            .cloned()
            .unwrap_or_else(|| format!("instruction {i}"))
    };
    let message = if let Some([index, detail]) = err
        .get("InstructionError")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
    {
        let index = index.as_u64().unwrap_or(0);
        let what = if let Some(code) = detail.get("Custom").and_then(Value::as_u64) {
            custom_error_name(code as u32, raiser(logs, dep))
        } else {
            detail
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| detail.to_string())
        };
        let mut m = format!("{} failed: {what}", step(index));
        if let Some(h) = hint(&what) {
            m.push_str(&format!("\n  {h}"));
        }
        m
    } else {
        let text = err
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| err.to_string());
        match text.as_str() {
            // DEV-ONLY: REMOVE FOR PROD (the dev build's `petri faucet`).
            "AccountNotFound" | "InsufficientFundsForFee" | "InsufficientFundsForRent" => format!(
                "the owner cannot pay fees or rent ({text}); use --route sponsored{}",
                if cfg!(feature = "dev-faucet") {
                    " or `petri faucet --sol 1`"
                } else {
                    ""
                }
            ),
            "BlockhashNotFound" => "the blockhash expired; try again".to_string(),
            _ => format!("transaction failed: {text}"),
        }
    };
    Error::msg(message).with_logs(useful_logs(logs))
}

fn landed(
    ctx: &Ctx,
    signature: Signature,
    last_valid: u64,
    steps: Vec<String>,
    route: Route,
) -> Result<Outcome> {
    let dep = ctx.deployment()?;
    let err = ctx.rpc.confirm(&signature, last_valid)?;
    let tx = ctx.rpc.transaction(&signature)?.unwrap_or_default();
    if let Some(err) = err.or(tx.err.clone()) {
        let mut e = explain(&err, &tx.logs, &steps, dep);
        e.message = format!("{}\n  signature {signature}", e.message);
        return Err(e);
    }
    Ok(Outcome {
        signature: signature.to_string(),
        route,
        steps,
        logs: tx.logs,
        slot: tx.slot,
        before: Vec::new(),
    })
}

/// Signs with the owner as fee payer and sends through RPC.
pub fn send_local(ctx: &Ctx, owner: &dyn Signer, plan: &Plan) -> Result<Outcome> {
    crate::chain_identity::verify(ctx)?;
    let dep = ctx.deployment()?;
    let steps: Vec<String> = plan.names().iter().map(|s| s.to_string()).collect();
    let (blockhash, last_valid) = ctx.rpc.latest_blockhash()?;
    let tx = Transaction::new_signed_with_payer(
        &super::governance::bind_instructions(dep, &ctx.rpc, plan.instructions())?,
        Some(&owner.pubkey()),
        &[owner],
        blockhash,
    );
    let size = bincode::serialized_size(&tx).unwrap_or(0) as usize;
    if size > PACKET_SIZE {
        bail(format!(
            "the transaction is {size} bytes, over the {PACKET_SIZE}-byte limit"
        ))?;
    }
    match ctx.rpc.send(&tx)? {
        Err(failure) => {
            let err = failure
                .err
                .clone()
                .unwrap_or(Value::String(failure.message.clone()));
            Err(explain(&err, &failure.logs, &steps, dep))
        }
        Ok(signature) => landed(ctx, signature, last_valid, steps, Route::Local),
    }
}

/// Asks the API to prepare `action`, verifies the transaction with `check`
/// (given the trusted sponsor), signs it as the owner and submits it for the
/// sponsor's co-signature. Refuses an API that serves other programs than
/// the trusted deployment. When the action spends compressed USDC in
/// transactions of their own (`before`), each is checked (`check`'s third
/// argument is true for them) and signed with the main one, then submitted
/// in order, each landing before the next.
pub fn send_prepared(
    ctx: &Ctx,
    owner: &dyn Signer,
    action: &str,
    params: Value,
    check: impl Fn(&Transaction, &Pubkey, bool) -> Result<()>,
) -> Result<Outcome> {
    crate::chain_identity::verify(ctx)?;
    let dep = ctx.deployment()?;
    let config = ctx.api.config()?;
    if !same_programs(dep, &config.programs) {
        bail(format!(
            "refusing the sponsored route: the API serves other programs than this client trusts ({})",
            dep.source
        ))?;
    }
    let api_sponsor = Pubkey::from_str(&config.sponsor).ok();
    let sponsor = match (dep.sponsor, api_sponsor) {
        (Some(trusted), Some(api)) if trusted != api => {
            return Err(Error::msg(format!(
                "refusing the sponsored route: the API sponsor {api} is not the deployment's {trusted}"
            )));
        }
        (Some(trusted), _) => trusted,
        (None, Some(api)) => api,
        (None, None) => bail("the API names no sponsor")?,
    };
    if config.governance_controller != dep.governance_controller {
        return Err(Error::msg(
            "API governance deployment differs from the trusted deployment",
        ));
    }
    let owner_key = owner.pubkey();
    let prepared = ctx.api.prepare(&owner_key.to_string(), action, params)?;
    // Every transaction is checked and signed before any is submitted.
    let sign = |part: &super::api::Prepared, before: bool| -> Result<String> {
        let bytes = B64
            .decode(part.transaction.trim())
            .map_err(|_| Error::msg("the prepared transaction is not base64"))?;
        let mut tx: Transaction = bincode::deserialize(&bytes)
            .map_err(|_| Error::msg("the prepared transaction does not decode"))?;
        let checked = super::governance::verify_prepared(dep, &ctx.rpc, &tx)?;
        check(&checked, &sponsor, before)?;
        let signers = usize::from(tx.message.header.num_required_signatures);
        let position = tx.message.account_keys[..signers]
            .iter()
            .position(|k| *k == owner_key)
            .ok_or_else(|| {
                Error::msg("the prepared transaction does not ask for your signature")
            })?;
        if tx.signatures.len() < signers {
            tx.signatures.resize(signers, Signature::default());
        }
        tx.signatures[position] = owner.sign_message(&tx.message_data());
        Ok(B64.encode(bincode::serialize(&tx).expect("transaction serializes")))
    };
    let before: Vec<(&super::api::Prepared, String)> = prepared
        .before
        .iter()
        .map(|part| Ok((part, sign(part, true)?)))
        .collect::<Result<_>>()?;
    let wire = sign(&prepared, false)?;
    let steps_of = |part: &super::api::Prepared| -> Vec<String> {
        part.summary
            .get("instructions")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    let submit = |part: &super::api::Prepared, wire: &str, what: &str| -> Result<Outcome> {
        let signature = ctx.api.submit(&part.id, wire).map_err(|e| {
            let message = match e.message.split_once(": ").map(|(_, rest)| rest) {
                Some(rest) => {
                    let mut m = format!("{what} failed: {rest}");
                    if let Some(h) = rest.split([' ', ':', '(']).find_map(hint) {
                        m.push_str(&format!("\n  {h}"));
                    }
                    m
                }
                None => e.message.clone(),
            };
            Error::msg(message).with_logs(useful_logs(&e.logs))
        })?;
        let signature = Signature::from_str(&signature)
            .map_err(|_| Error::msg(format!("the API returned a bad signature {signature}")))?;
        landed(
            ctx,
            signature,
            part.last_valid_block_height,
            steps_of(part),
            Route::Sponsored,
        )
    };
    let mut moved = Vec::new();
    for (part, wire) in &before {
        moved.push(submit(part, wire, "moving your compressed USDC")?.signature);
    }
    let mut outcome = submit(&prepared, &wire, action)?;
    outcome.before = moved;
    Ok(outcome)
}

/// Whether the API serves the programs this client trusts (the futures
/// program and the oracle).
pub fn same_programs(dep: &Deployment, api: &super::api::Programs) -> bool {
    api.oracle == dep.oracle.to_string() && api.futures == dep.futures.map(|f| f.to_string())
}

/// Asks before a money-moving action unless `--yes`. Refuses without a
/// terminal to ask on.
pub fn confirm(ctx: &Ctx, question: &str) -> Result<()> {
    use std::io::{BufRead, IsTerminal, Write};
    if ctx.s.yes {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        bail("not confirmed: pass --yes to proceed without a prompt")?;
    }
    eprint!("{question} [y/N] ");
    std::io::stderr().flush().ok();
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        bail("cancelled; nothing was sent")
    }
}
