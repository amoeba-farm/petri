//! Sending oracle transactions: the deployment identity gate, a review the
//! user confirms (or `--yes`), then each transaction signed by the attached
//! wallet as fee payer and sent through the RPC. `--simulate` runs the same
//! transactions through `simulateTransaction` without loading a signer.

use serde_json::{Value, json};
use solana_program::{instruction::Instruction, pubkey::Pubkey};
use solana_transaction::Transaction;

use ameba_sdk_futures::futures::governance::GateBinding;

use super::errors;
use crate::{
    chain_identity,
    futures::{
        Report,
        ctx::Ctx,
        error::{Error, Raiser, Result, bail, custom_error_name},
        exec::{confirm, useful_logs},
        governance,
        txbuild::{COMPUTE_UNIT_LIMIT, PACKET_SIZE, compute_unit_limit},
        view::Paint,
    },
};

/// One transaction: named instructions, sent atomically.
#[derive(Clone, Debug, Default)]
pub struct Tx {
    pub steps: Vec<(String, Instruction)>,
}

impl Tx {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn one(name: &str, ix: Instruction) -> Self {
        let mut tx = Self::new();
        tx.push(name, ix);
        tx
    }

    pub fn push(&mut self, name: &str, ix: Instruction) -> &mut Self {
        self.steps.push((name.to_string(), ix));
        self
    }

    fn names(&self) -> Vec<String> {
        self.steps.iter().map(|(n, _)| n.clone()).collect()
    }

    fn instructions(&self) -> Vec<Instruction> {
        let mut out = vec![compute_unit_limit(COMPUTE_UNIT_LIMIT)];
        out.extend(self.steps.iter().map(|(_, ix)| ix.clone()));
        out
    }
}

/// An oracle write ready to review and send.
pub struct Write {
    /// `petri oracle …` action name, for JSON.
    pub action: &'static str,
    /// One-line summary for the confirmation prompt.
    pub title: String,
    /// What the user is asked to confirm.
    pub review: Vec<String>,
    /// Extra JSON for the result (addresses, amounts, ids).
    pub details: Value,
    pub txs: Vec<Tx>,
}

/// Which program raised a custom error, from the `Program … failed` log line.
fn raiser(logs: &[String], oracle: &Pubkey) -> (bool, Raiser) {
    let failed = logs
        .iter()
        .rev()
        .find(|l| l.contains(" failed: "))
        .cloned()
        .unwrap_or_default();
    if failed.contains(&oracle.to_string()) {
        (true, Raiser::Unknown)
    } else if failed.contains(&spl_token::id().to_string()) {
        (false, Raiser::Token)
    } else {
        (false, Raiser::Unknown)
    }
}

/// A readable error for a transaction error value and its logs.
pub fn explain(err: &Value, logs: &[String], steps: &[String], oracle: &Pubkey) -> Error {
    let message = if let Some([index, detail]) = err
        .get("InstructionError")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
    {
        // Instruction 0 is the compute-unit limit.
        let index = index.as_u64().unwrap_or(0) as usize;
        let step = index
            .checked_sub(1)
            .and_then(|i| steps.get(i).cloned())
            .unwrap_or_else(|| format!("instruction {index}"));
        let what = if let Some(code) = detail.get("Custom").and_then(Value::as_u64) {
            let (by_oracle, raised) = raiser(logs, oracle);
            match errors::name(code as u32) {
                Some(name) if by_oracle || code >= 6000 => name,
                _ => custom_error_name(code as u32, raised),
            }
        } else {
            detail
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| detail.to_string())
        };
        format!("{step} failed: {what}")
    } else {
        let text = err
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| err.to_string());
        match text.as_str() {
            // DEV-ONLY: REMOVE FOR PROD (the dev build's `petri faucet`).
            "AccountNotFound" | "InsufficientFundsForFee" | "InsufficientFundsForRent" => format!(
                "the wallet cannot pay fees or rent ({text}); fund it with SOL{}",
                if cfg!(feature = "dev-faucet") {
                    " (`petri faucet --sol 1` on dev)"
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

fn validate_governance(dep: &crate::futures::market::Deployment) -> Result<()> {
    // The dev chain's oracle (release `localnet-dev`) may be ungated, in the
    // dev profile only.
    let legacy_localnet =
        dep.oracle_release_tag.as_deref() == Some("localnet-dev") && dep.dev_chain();
    let registered = dep
        .governance_controller
        .as_ref()
        .is_some_and(|g| g.targets.contains(&dep.oracle.to_string()));
    if !registered && !legacy_localnet {
        return bail("the trusted deployment does not register its oracle governance gate");
    }
    Ok(())
}

/// The oracle is its own governance target: when the deployment gates it,
/// every oracle instruction carries the oracle's gate account and current
/// epoch (`GateBinding::bind`); other instructions (the compute budget, the
/// token program) stay as built.
fn bind_oracle(gate: Option<&GateBinding>, ixs: Vec<Instruction>) -> Result<Vec<Instruction>> {
    let Some(gate) = gate else {
        return Ok(ixs);
    };
    ixs.into_iter()
        .map(|ix| {
            if ix.program_id == gate.target {
                gate.bind(ix)
                    .map_err(|e| Error::msg(format!("governance gate refused: {e}")))
            } else {
                Ok(ix)
            }
        })
        .collect()
}

fn instructions(ctx: &Ctx, tx: &Tx) -> Result<Vec<Instruction>> {
    let dep = ctx.deployment()?;
    validate_governance(dep)?;
    let gate = governance::gate(dep, &ctx.rpc, &dep.oracle)?;
    bind_oracle(gate.as_ref(), tx.instructions())
}

fn build(ctx: &Ctx, tx: &Tx, payer: &Pubkey) -> Result<Transaction> {
    Ok(Transaction::new_with_payer(
        &instructions(ctx, tx)?,
        Some(payer),
    ))
}

/// Simulates every transaction in order without signing; stops at the first
/// failure (later transactions may depend on earlier ones).
fn simulate(ctx: &Ctx, write: &Write, payer: &Pubkey, oracle: &Pubkey) -> Result<Value> {
    let mut results = Vec::new();
    for (i, tx) in write.txs.iter().enumerate() {
        let sim = ctx.rpc.simulate(&build(ctx, tx, payer)?)?;
        let ok = sim.err.is_none();
        let error = sim
            .err
            .as_ref()
            .map(|e| explain(e, &sim.logs, &tx.names(), oracle).message);
        results.push(json!({
            "transaction": i + 1,
            "steps": tx.names(),
            "ok": ok,
            "error": error,
            "unitsConsumed": sim.units,
            "logs": useful_logs(&sim.logs),
        }));
        if !ok {
            break;
        }
    }
    Ok(Value::Array(results))
}

/// Reviews, confirms and sends `write`, or simulates it with `simulate`.
pub fn run(ctx: &Ctx, p: Paint, write: Write, simulate_only: bool) -> Result<Report> {
    let (identity, _config) = chain_identity::verify_oracle(ctx)?;
    let oracle = identity.oracle;
    let payer = ctx.wallet_pubkey()?;
    for tx in &write.txs {
        let size = bincode::serialized_size(&build(ctx, tx, &payer)?).unwrap_or(0) as usize + 64;
        if size > PACKET_SIZE {
            bail(format!(
                "{} would be {size} bytes, over the {PACKET_SIZE}-byte packet limit",
                tx.names().join(" + ")
            ))?;
        }
    }
    let plan: Vec<Value> = write
        .txs
        .iter()
        .map(|tx| json!({"steps": tx.names()}))
        .collect();
    let mut lines = vec![p.bold(&write.title)];
    lines.extend(write.review.iter().map(|l| format!("  {l}")));
    lines.push(format!(
        "  {} transaction(s), fee payer {payer}, oracle {oracle}",
        write.txs.len()
    ));
    if simulate_only {
        let results = simulate(ctx, &write, &payer, &oracle)?;
        let all_ok = results
            .as_array()
            .is_some_and(|r| r.len() == write.txs.len() && r.iter().all(|t| t["ok"] == true));
        for result in results.as_array().into_iter().flatten() {
            let steps = result["steps"]
                .as_array()
                .map(|s| {
                    s.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(" + ")
                })
                .unwrap_or_default();
            match result["error"].as_str() {
                None => lines.push(format!(
                    "  simulated {steps}: ok ({} compute units)",
                    result["unitsConsumed"].as_u64().unwrap_or(0)
                )),
                Some(error) => lines.push(format!("  simulated {steps}: {}", p.red(error))),
            }
        }
        lines.push("Nothing was signed or sent.".to_string());
        let payload = json!({
            "ok": true,
            "action": write.action,
            "simulated": true,
            "wouldSucceed": all_ok,
            "feePayer": payer.to_string(),
            "identity": identity.json(),
            "transactions": results,
            "details": write.details,
        });
        return Ok(Report::new(payload, lines.join("\n")));
    }
    if !ctx.s.json {
        eprintln!("{}", lines.join("\n"));
    }
    confirm(ctx, &format!("{}?", write.title))?;
    // The review went to stderr; the result lists what landed.
    let mut lines = vec![p.bold(&write.title)];
    let signer = ctx.signer()?;
    if signer.pubkey() != payer {
        bail("the attached wallet changed while preparing; nothing was sent")?;
    }
    let mut sent = Vec::new();
    for (i, tx) in write.txs.iter().enumerate() {
        let (blockhash, last_valid) = ctx.rpc.latest_blockhash()?;
        let signed = Transaction::new_signed_with_payer(
            &instructions(ctx, tx)?,
            Some(&payer),
            &[signer.as_ref()],
            blockhash,
        );
        let names = tx.names();
        let fail = |e: Error| {
            let mut e = e;
            if i > 0 {
                e.message = format!(
                    "{}\n  {i} earlier transaction(s) landed: {}",
                    e.message,
                    sent.iter()
                        .map(|s: &Value| s["signature"].as_str().unwrap_or("").to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            e
        };
        let signature = match ctx.rpc.send(&signed)? {
            Err(failure) => {
                let err = failure
                    .err
                    .clone()
                    .unwrap_or(Value::String(failure.message.clone()));
                return Err(fail(explain(&err, &failure.logs, &names, &oracle)));
            }
            Ok(signature) => signature,
        };
        let err = ctx.rpc.confirm(&signature, last_valid)?;
        let landed = ctx.rpc.transaction(&signature)?.unwrap_or_default();
        if let Some(err) = err.or(landed.err.clone()) {
            let mut e = explain(&err, &landed.logs, &names, &oracle);
            e.message = format!("{}\n  signature {signature}", e.message);
            return Err(fail(e));
        }
        lines.push(format!(
            "  {} {}: {signature} (slot {})",
            p.green("sent"),
            names.join(" + "),
            landed.slot
        ));
        sent.push(json!({
            "signature": signature.to_string(),
            "slot": landed.slot,
            "steps": names,
        }));
    }
    let payload = json!({
        "ok": true,
        "action": write.action,
        "feePayer": payer.to_string(),
        "identity": identity.json(),
        "plan": plan,
        "transactions": sent,
        "details": write.details,
    });
    Ok(Report::new(payload, lines.join("\n")))
}

use solana_signer::Signer as _;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oracle_errors_are_named_with_their_step() {
        let oracle = Pubkey::new_unique();
        let logs = vec![format!(
            "Program {oracle} failed: custom program error: 0x177f"
        )];
        let e = explain(
            &json!({"InstructionError": [1, {"Custom": 6015}]}),
            &logs,
            &["writeEvidence".to_string()],
            &oracle,
        );
        assert!(
            e.message
                .starts_with("writeEvidence failed: InvalidPeriodAccount"),
            "{}",
            e.message
        );
        let e = explain(
            &json!({"InstructionError": [0, "ProgramFailedToComplete"]}),
            &[],
            &["writeEvidence".to_string()],
            &oracle,
        );
        assert_eq!(e.message, "instruction 0 failed: ProgramFailedToComplete");
    }

    #[test]
    fn only_explicit_trusted_localnet_legacy_oracle_can_omit_gate() {
        use crate::futures::settings::Profile;
        use crate::futures::{governance::GovernanceDeployment, market::Deployment};
        let mut dep = Deployment {
            oracle_release_tag: None,
            governance_controller: None,
            oracle: Pubkey::new_unique(),
            futures: None,
            usdc_mint: Pubkey::new_unique(),
            sponsor: None,
            genesis_hash: None,
            source: "fixture".into(),
            profile: Profile::Dev,
        };
        assert!(validate_governance(&dep).is_err());
        dep.oracle_release_tag = Some("localnet-dev".into());
        assert!(validate_governance(&dep).is_err());
        dep.genesis_hash = Some("A5ZLaudxEFvFScV6RzNtNN7fzshRxQgXhC6JB6dishHX".into());
        validate_governance(&dep).unwrap();
        // Only in the dev profile.
        dep.profile = Profile::Production;
        assert!(validate_governance(&dep).is_err());
        dep.profile = Profile::Dev;
        dep.oracle_release_tag = Some("governed".into());
        assert!(validate_governance(&dep).is_err());
        dep.governance_controller = Some(GovernanceDeployment {
            program: Pubkey::new_unique().to_string(),
            targets: vec![dep.oracle.to_string()],
        });
        validate_governance(&dep).unwrap();
    }

    #[test]
    fn oracle_instructions_carry_the_oracle_gate() {
        use ameba_sdk_futures::futures::governance::{
            GOVERNANCE_TAIL_LEN, LOCALNET_CONTROLLER_PROGRAM_ID, gate_address,
        };
        let oracle = Pubkey::new_unique();
        let controller = LOCALNET_CONTROLLER_PROGRAM_ID;
        let gate = GateBinding {
            controller,
            target: oracle,
            address: gate_address(&controller, &oracle).0,
            epoch: 3,
        };
        let write = Instruction {
            program_id: oracle,
            accounts: vec![],
            data: vec![1],
        };
        let tx = Tx::one("write", write.clone());
        // Ungoverned: the instructions go out as built.
        assert_eq!(
            bind_oracle(None, tx.instructions()).unwrap(),
            tx.instructions()
        );
        let bound = bind_oracle(Some(&gate), tx.instructions()).unwrap();
        assert_eq!(bound[0], compute_unit_limit(COMPUTE_UNIT_LIMIT));
        assert_eq!(bound[1], gate.bind(write).unwrap());
        assert_eq!(bound[1].accounts.last().unwrap().pubkey, gate.address);
        assert_eq!(bound[1].data.len(), 1 + GOVERNANCE_TAIL_LEN);
    }

    #[test]
    fn every_transaction_starts_with_the_compute_limit() {
        let tx = Tx::one(
            "x",
            Instruction {
                program_id: Pubkey::new_unique(),
                accounts: vec![],
                data: vec![1],
            },
        );
        let ixs = tx.instructions();
        assert_eq!(ixs.len(), 2);
        assert_eq!(ixs[0], compute_unit_limit(COMPUTE_UNIT_LIMIT));
    }
}
