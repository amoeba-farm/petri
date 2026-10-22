//! Sending oracle transactions: the deployment identity gate, a review the
//! user confirms (or `--yes`), then each transaction signed by the attached
//! wallet as fee payer and sent through the RPC. `--simulate` runs the same
//! transactions through `simulateTransaction` without loading a signer; one
//! that uses what an earlier one writes (its evidence) is left to the send's
//! preflight, since the earlier one is not on chain yet.

use std::collections::HashMap;

use serde_json::{Value, json};
use solana_message::{AddressLookupTableAccount, VersionedMessage, v0};
use solana_program::{hash::Hash, instruction::Instruction, pubkey::Pubkey};
use solana_signature::Signature;
use solana_transaction::{Transaction, versioned::VersionedTransaction};

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

/// One transaction: named instructions, sent atomically, as a legacy
/// transaction or (with `lookup_table`) a v0 one whose accounts resolve
/// through that address lookup table.
#[derive(Clone, Debug, Default)]
pub struct Tx {
    pub steps: Vec<(String, Instruction)>,
    pub lookup_table: Option<Pubkey>,
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

    /// The same transaction, sent as v0 through the lookup table `table`.
    pub fn through(mut self, table: Pubkey) -> Self {
        self.lookup_table = Some(table);
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

/// Bytes on the wire: an unsigned transaction already holds a zeroed slot
/// for every signature.
fn wire_size(tx: &Transaction) -> usize {
    bincode::serialized_size(tx).unwrap_or(u64::MAX) as usize
}

/// The address lookup table program.
pub const LOOKUP_TABLE_PROGRAM: Pubkey =
    Pubkey::from_str_const("AddressLookupTab1e1111111111111111111111111");
/// A lookup table's header (`LookupTableMeta`) before its addresses.
const LOOKUP_TABLE_META: usize = 56;

/// An address lookup table's addresses, from its account: owned by the
/// lookup table program, a table (state 1), not deactivated, then whole
/// 32-byte addresses. Entries are append-only, so the indexes a message
/// uses keep naming the accounts it was compiled with.
pub fn decode_table(
    key: &Pubkey,
    account: Option<&crate::futures::rpc::Account>,
) -> Result<AddressLookupTableAccount> {
    let account =
        account.ok_or_else(|| Error::msg(format!("lookup table {key} does not exist")))?;
    let d = &account.data;
    let deactivation = d
        .get(4..12)
        .map(|b| u64::from_le_bytes(b.try_into().unwrap()));
    if account.owner != LOOKUP_TABLE_PROGRAM
        || d.len() < LOOKUP_TABLE_META
        || d[..4] != [1, 0, 0, 0]
        || !(d.len() - LOOKUP_TABLE_META).is_multiple_of(32)
    {
        bail(format!("{key} is not an address lookup table"))?;
    }
    if deactivation != Some(u64::MAX) {
        bail(format!("lookup table {key} is deactivated"))?;
    }
    Ok(AddressLookupTableAccount {
        key: *key,
        addresses: d[LOOKUP_TABLE_META..]
            .chunks_exact(32)
            .map(|a| Pubkey::new_from_array(a.try_into().unwrap()))
            .collect(),
    })
}

/// The lookup tables a write's transactions go through, read once.
fn tables(ctx: &Ctx, write: &Write) -> Result<HashMap<Pubkey, AddressLookupTableAccount>> {
    let mut out = HashMap::new();
    for key in write.txs.iter().filter_map(|tx| tx.lookup_table) {
        if let std::collections::hash_map::Entry::Vacant(entry) = out.entry(key) {
            entry.insert(decode_table(&key, ctx.rpc.account(&key)?.as_ref())?);
        }
    }
    Ok(out)
}

/// One transaction as it goes on the wire.
#[derive(Debug)]
enum Wire {
    Legacy(Transaction),
    V0(VersionedTransaction),
}

impl Wire {
    fn size(&self) -> usize {
        match self {
            Wire::Legacy(tx) => wire_size(tx),
            Wire::V0(tx) => bincode::serialized_size(tx).unwrap_or(u64::MAX) as usize,
        }
    }

    fn simulate(&self, ctx: &Ctx) -> Result<crate::futures::rpc::Simulation> {
        match self {
            Wire::Legacy(tx) => ctx.rpc.simulate(tx),
            Wire::V0(tx) => ctx.rpc.simulate(tx),
        }
    }

    fn send(
        &self,
        ctx: &Ctx,
    ) -> Result<std::result::Result<Signature, crate::futures::rpc::RpcFailure>> {
        match self {
            Wire::Legacy(tx) => ctx.rpc.send(tx),
            Wire::V0(tx) => ctx.rpc.send(tx),
        }
    }
}

/// `ixs` as a v0 message resolving its other accounts through `table`.
fn v0_message(
    ixs: &[Instruction],
    payer: &Pubkey,
    table: &AddressLookupTableAccount,
    blockhash: Hash,
) -> Result<VersionedMessage> {
    v0::Message::try_compile(payer, ixs, std::slice::from_ref(table), blockhash)
        .map(VersionedMessage::V0)
        .map_err(|e| {
            Error::msg(format!(
                "cannot compile a v0 message through {}: {e}",
                table.key
            ))
        })
}

/// The unsigned transaction (each signature a zeroed slot).
fn compose(
    ixs: &[Instruction],
    payer: &Pubkey,
    table: Option<&AddressLookupTableAccount>,
) -> Result<Wire> {
    Ok(match table {
        None => Wire::Legacy(Transaction::new_with_payer(ixs, Some(payer))),
        Some(table) => {
            let message = v0_message(ixs, payer, table, Hash::default())?;
            let signers = usize::from(message.header().num_required_signatures);
            Wire::V0(VersionedTransaction {
                signatures: vec![Signature::default(); signers],
                message,
            })
        }
    })
}

fn table_of<'a>(
    tx: &Tx,
    tables: &'a HashMap<Pubkey, AddressLookupTableAccount>,
) -> Result<Option<&'a AddressLookupTableAccount>> {
    tx.lookup_table
        .map(|key| {
            tables
                .get(&key)
                .ok_or_else(|| Error::msg(format!("lookup table {key} was not read")))
        })
        .transpose()
}

fn build(
    ctx: &Ctx,
    tx: &Tx,
    payer: &Pubkey,
    tables: &HashMap<Pubkey, AddressLookupTableAccount>,
) -> Result<Wire> {
    compose(&instructions(ctx, tx)?, payer, table_of(tx, tables)?)
}

/// The wire size of `tx` as a legacy transaction (what goes out without a
/// lookup table).
pub fn legacy_size(ctx: &Ctx, tx: &Tx, payer: &Pubkey) -> Result<usize> {
    Ok(wire_size(&Transaction::new_with_payer(
        &instructions(ctx, tx)?,
        Some(payer),
    )))
}

/// `steps` in order, in as few transactions as fit a packet once each
/// oracle instruction carries its gate (the account and tail
/// `GateBinding::bind` adds).
pub fn pack(oracle: &Pubkey, payer: &Pubkey, steps: Vec<(String, Instruction)>) -> Vec<Tx> {
    let gate = GateBinding {
        controller: Pubkey::default(),
        target: *oracle,
        address: Pubkey::new_from_array([1; 32]),
        epoch: 0,
    };
    let fits = |tx: &Tx| {
        bind_oracle(Some(&gate), tx.instructions()).is_ok_and(|ixs| {
            wire_size(&Transaction::new_with_payer(&ixs, Some(payer))) <= PACKET_SIZE
        })
    };
    let mut txs: Vec<Tx> = Vec::new();
    for (name, ix) in steps {
        if let Some(last) = txs.last_mut() {
            let mut grown = last.clone();
            grown.push(&name, ix.clone());
            if fits(&grown) {
                *last = grown;
                continue;
            }
        }
        txs.push(Tx::one(&name, ix));
    }
    txs
}

/// For each transaction, the earlier transactions of the same write whose
/// accounts it uses (an evidence object they write, say), by index. Signers
/// (the fee payer) don't count.
fn waits_for(txs: &[Tx]) -> Vec<Vec<usize>> {
    let written: Vec<Vec<Pubkey>> = txs
        .iter()
        .map(|tx| {
            tx.steps
                .iter()
                .flat_map(|(_, ix)| &ix.accounts)
                .filter(|a| a.is_writable && !a.is_signer)
                .map(|a| a.pubkey)
                .collect()
        })
        .collect();
    txs.iter()
        .enumerate()
        .map(|(i, tx)| {
            (0..i)
                .filter(|&j| {
                    tx.steps
                        .iter()
                        .flat_map(|(_, ix)| &ix.accounts)
                        .any(|a| written[j].contains(&a.pubkey))
                })
                .collect()
        })
        .collect()
}

/// Why a transaction waits, in plain words: simulated alone against today's
/// chain it would fail for want of the earlier ones, and the send's
/// preflight checks it once they landed.
fn waiting_note(txs: &[Tx], waits: &[usize]) -> String {
    let evidence = waits
        .iter()
        .all(|&j| txs[j].steps.iter().all(|(n, _)| n == "writeEvidence"));
    if evidence {
        return "checked when sent, once the evidence above is on chain".to_string();
    }
    let numbers: Vec<String> = waits.iter().map(|j| (j + 1).to_string()).collect();
    format!(
        "checked when sent, once transaction{} {} {} on chain",
        if numbers.len() == 1 { "" } else { "s" },
        numbers.join(", "),
        if numbers.len() == 1 { "is" } else { "are" }
    )
}

/// Simulates every transaction in order without signing; stops at the first
/// failure. A transaction that uses what an earlier one writes is not
/// simulated, since the earlier one is not on chain yet.
fn simulate(
    ctx: &Ctx,
    write: &Write,
    payer: &Pubkey,
    oracle: &Pubkey,
    tables: &HashMap<Pubkey, AddressLookupTableAccount>,
) -> Result<Value> {
    let waits = waits_for(&write.txs);
    let mut results = Vec::new();
    for (i, tx) in write.txs.iter().enumerate() {
        if !waits[i].is_empty() {
            results.push(json!({
                "transaction": i + 1,
                "steps": tx.names(),
                "simulated": false,
                "ok": null,
                "error": null,
                "waitsFor": waits[i].iter().map(|j| j + 1).collect::<Vec<_>>(),
                "note": waiting_note(&write.txs, &waits[i]),
            }));
            continue;
        }
        let sim = build(ctx, tx, payer, tables)?.simulate(ctx)?;
        let ok = sim.err.is_none();
        let error = sim
            .err
            .as_ref()
            .map(|e| explain(e, &sim.logs, &tx.names(), oracle).message);
        results.push(json!({
            "transaction": i + 1,
            "steps": tx.names(),
            "simulated": true,
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

/// Every transaction was reached and every one simulated passed.
fn simulated_ok(results: &[Value], count: usize) -> bool {
    results.len() == count
        && results
            .iter()
            .all(|t| t["simulated"] == false || t["ok"] == true)
}

/// Reviews, confirms and sends `write`, or simulates it with `simulate`.
pub fn run(ctx: &Ctx, p: Paint, write: Write, simulate_only: bool) -> Result<Report> {
    let (identity, _config) = chain_identity::verify_oracle(ctx)?;
    let oracle = identity.oracle;
    let payer = ctx.wallet_pubkey()?;
    let tables = tables(ctx, &write)?;
    for tx in &write.txs {
        let size = build(ctx, tx, &payer, &tables)?.size();
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
        .map(|tx| {
            json!({
                "steps": tx.names(),
                "version": if tx.lookup_table.is_some() { "v0" } else { "legacy" },
                "lookupTable": tx.lookup_table.map(|t| t.to_string()),
            })
        })
        .collect();
    let mut lines = vec![p.bold(&write.title)];
    lines.extend(write.review.iter().map(|l| format!("  {l}")));
    lines.push(format!(
        "  {} transaction(s), fee payer {payer}, oracle {oracle}",
        write.txs.len()
    ));
    if simulate_only {
        let results = simulate(ctx, &write, &payer, &oracle, &tables)?;
        let all_ok = results
            .as_array()
            .is_some_and(|r| simulated_ok(r, write.txs.len()));
        let not_simulated: Vec<Value> = results
            .as_array()
            .into_iter()
            .flatten()
            .filter(|t| t["simulated"] == false)
            .map(|t| t["transaction"].clone())
            .collect();
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
                None if result["simulated"] == false => lines.push(format!(
                    "  {steps}: {}",
                    result["note"].as_str().unwrap_or_default()
                )),
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
            "notSimulated": not_simulated,
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
        let ixs = instructions(ctx, tx)?;
        let signed = match table_of(tx, &tables)? {
            None => Wire::Legacy(Transaction::new_signed_with_payer(
                &ixs,
                Some(&payer),
                &[signer.as_ref()],
                blockhash,
            )),
            Some(table) => Wire::V0(
                VersionedTransaction::try_new(
                    v0_message(&ixs, &payer, table, blockhash)?,
                    &[signer.as_ref()],
                )
                .map_err(|e| Error::msg(format!("cannot sign the v0 transaction: {e}")))?,
            ),
        };
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
        let signature = match signed.send(ctx)? {
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

    /// The lookup table ops creates for a month (contract of 2026-10-21):
    /// period, month, coverage manifest, reward schedule, carry period, the
    /// system program, the oracle's gate, then every SKU pool by bucket id.
    fn month_table(
        program: &Pubkey,
        period: &Pubkey,
        month: &Pubkey,
        gate: &Pubkey,
        buckets: &[[u8; 32]],
    ) -> AddressLookupTableAccount {
        use ameba_oracle::{instruction as ix, state};
        let schedule = state::derive_oracle_usdc_reward_schedule_pda(program, month).0;
        let mut addresses = vec![
            *period,
            *month,
            state::derive_oracle_sku_coverage_manifest_pda(program, month).0,
            schedule,
            ix::find_carry_period_address(program, month),
            solana_program::system_program::ID,
            *gate,
        ];
        addresses.extend(
            buckets
                .iter()
                .map(|b| state::derive_oracle_usdc_sku_pool_pda(program, &schedule, b).0),
        );
        AddressLookupTableAccount {
            key: Pubkey::new_unique(),
            addresses,
        }
    }

    #[test]
    fn a_chip_months_sponsorship_fits_a_packet_as_v0_through_its_table() {
        use ameba_oracle::{
            instruction::{self as ix, ProposeOracleSourceV3Params},
            sponsorship::{DEFAULT_RETENTION_BPS, SponsorTerms},
        };
        let (program, sponsor) = (Pubkey::new_unique(), Pubkey::new_unique());
        let (period, month) = (Pubkey::new_unique(), Pubkey::new_unique());
        // RAMX's four chip buckets: a two-level SKU proof.
        let chips = super::super::skus::chips(ameba_commodity_spec::Product::Ramx).unwrap();
        let bucket = chips.ids[2];
        let (sku_index, sku_proof) = chips.proof(&bucket).unwrap();
        assert_eq!(sku_proof.len(), 2);
        let activation = Tx::one(
            "sponsorSource",
            ix::sponsored_activate(
                &program,
                &sponsor,
                &sponsor,
                &sponsor,
                &period,
                &month,
                SponsorTerms {
                    agent_id: [1; 32],
                    researcher: Pubkey::new_unique(),
                    candidate_hash: [2; 32],
                    funding: 0,
                    researcher_bps: 2_000,
                    retention_bps: DEFAULT_RETENTION_BPS,
                    maximum_bond: 1_000_000,
                    maximum_outstanding: 5_000_000,
                },
                ProposeOracleSourceV3Params {
                    source_id: [2; 32],
                    bucket_id: bucket,
                    source_type_hash: [3; 32],
                    canonical_locator_hash: [4; 32],
                    source_definition_hash: [5; 32],
                    listing_bond: 1_000_000,
                    sku_index,
                    sku_proof,
                },
            ),
        );
        // A governed oracle: the gate account and tail on the instruction.
        let gate = GateBinding {
            controller: Pubkey::new_unique(),
            target: program,
            address: Pubkey::new_unique(),
            epoch: 7,
        };
        let ixs = bind_oracle(Some(&gate), activation.instructions()).unwrap();
        let table = month_table(&program, &period, &month, &gate.address, &chips.ids);
        let legacy = compose(&ixs, &sponsor, None).unwrap().size();
        let v0 = compose(&ixs, &sponsor, Some(&table)).unwrap().size();
        // Over the packet as legacy; as v0 through the table it fits.
        assert!(legacy > PACKET_SIZE && v0 <= PACKET_SIZE, "{legacy} {v0}");
        // With the compute-unit limit and the gate: 1283 bytes legacy, 1071 as v0.
        assert_eq!((legacy, v0), (1_283, 1_071));
    }

    #[test]
    fn a_lookup_table_reads_from_its_account() {
        let key = Pubkey::new_unique();
        let addresses = [Pubkey::new_unique(), Pubkey::new_unique()];
        let mut data = vec![1, 0, 0, 0];
        data.extend(u64::MAX.to_le_bytes());
        data.resize(LOOKUP_TABLE_META, 0);
        for a in &addresses {
            data.extend(a.to_bytes());
        }
        let account = |data: Vec<u8>, owner| crate::futures::rpc::Account {
            lamports: 1,
            owner,
            data,
            executable: false,
        };
        let table = decode_table(&key, Some(&account(data.clone(), LOOKUP_TABLE_PROGRAM))).unwrap();
        assert_eq!((table.key, table.addresses), (key, addresses.to_vec()));
        assert!(decode_table(&key, None).is_err());
        assert!(decode_table(&key, Some(&account(data.clone(), Pubkey::new_unique()))).is_err());
        let mut closing = data.clone();
        closing[4..12].copy_from_slice(&100u64.to_le_bytes());
        let error = decode_table(&key, Some(&account(closing, LOOKUP_TABLE_PROGRAM))).unwrap_err();
        assert!(error.message.contains("deactivated"), "{}", error.message);
        let mut torn = data;
        torn.pop();
        assert!(decode_table(&key, Some(&account(torn, LOOKUP_TABLE_PROGRAM))).is_err());
    }

    #[test]
    fn the_size_check_measures_the_signed_transaction() {
        use solana_keypair::Keypair;
        use solana_signer::Signer;
        let payer = Keypair::new();
        let ix = Instruction::new_with_bytes(
            Pubkey::new_unique(),
            &[7; 900],
            vec![solana_program::instruction::AccountMeta::new(
                payer.pubkey(),
                true,
            )],
        );
        let mut tx = Transaction::new_with_payer(&[ix], Some(&payer.pubkey()));
        let unsigned = wire_size(&tx);
        tx.sign(&[&payer], solana_program::hash::Hash::new_unique());
        assert_eq!(unsigned, bincode::serialize(&tx).unwrap().len());
    }

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
    fn steps_that_need_earlier_evidence_wait_for_the_send() {
        use ameba_oracle::instruction as ix;
        let program = Pubkey::new_unique();
        let payer = Pubkey::new_unique();
        let other = Pubkey::new_unique();
        let write = |payer: &Pubkey, kind: u8, offset: u16| {
            ix::write_evidence(&program, payer, kind, [kind; 32], 400, offset, vec![1])
        };
        let locator = ix::find_evidence_object_address(&program, &payer, 1, &[1; 32]);
        let definition = ix::find_evidence_object_address(&program, &payer, 2, &[2; 32]);
        let activate = Instruction {
            program_id: program,
            accounts: vec![
                solana_program::instruction::AccountMeta::new(payer, true),
                solana_program::instruction::AccountMeta::new_readonly(locator, false),
                solana_program::instruction::AccountMeta::new_readonly(definition, false),
            ],
            data: vec![1],
        };
        let mut evidence = Tx::one("writeEvidence", write(&payer, 1, 0));
        evidence.push("writeEvidence", write(&payer, 1, 192));
        let txs = vec![
            evidence,
            Tx::one("writeEvidence", write(&payer, 1, 384)),
            Tx::one("writeEvidence", write(&payer, 2, 0)),
            Tx::one("sponsorSource", activate.clone()),
        ];
        // The fee payer is in every transaction; only the objects count.
        assert_eq!(
            waits_for(&txs),
            vec![vec![], vec![0], vec![], vec![0usize, 1, 2]]
        );
        assert_eq!(
            waiting_note(&txs, &[0, 1, 2]),
            "checked when sent, once the evidence above is on chain"
        );
        // Evidence already sealed: the activation simulates on its own.
        assert_eq!(
            waits_for(&[Tx::one("sponsorSource", activate)]),
            vec![Vec::<usize>::new()]
        );
        // Another wallet's evidence is not this activation's.
        let unrelated = vec![
            Tx::one("writeEvidence", write(&other, 1, 0)),
            Tx::one("writeEvidence", write(&payer, 2, 0)),
        ];
        assert_eq!(waits_for(&unrelated), vec![Vec::<usize>::new(); 2]);
        let mixed = vec![
            Tx::one("council", write(&payer, 1, 0)),
            Tx::one("sponsorSource", write(&payer, 1, 192)),
        ];
        assert_eq!(
            waiting_note(&mixed, &[0]),
            "checked when sent, once transaction 1 is on chain"
        );
    }

    #[test]
    fn a_simulation_passes_when_every_simulated_step_did() {
        let ok = json!({"simulated": true, "ok": true});
        let waits = json!({"simulated": false, "ok": null});
        let failed = json!({"simulated": true, "ok": false});
        assert!(simulated_ok(&[ok.clone(), waits.clone()], 2));
        assert!(!simulated_ok(&[ok.clone(), failed], 3));
        // Stopped early: a transaction was never reached.
        assert!(!simulated_ok(&[ok], 2));
        assert!(simulated_ok(&[waits], 1));
    }

    #[test]
    fn steps_pack_into_transactions_that_fit_once_gated() {
        use solana_program::instruction::AccountMeta;
        let oracle = Pubkey::new_unique();
        let payer = Pubkey::new_unique();
        // Seven fresh accounts each: a few to a transaction.
        let step = |_| {
            let accounts = (0..7)
                .map(|_| AccountMeta::new(Pubkey::new_unique(), false))
                .chain([AccountMeta::new(payer, true)])
                .collect();
            (
                "claimReward".to_string(),
                Instruction {
                    program_id: oracle,
                    accounts,
                    data: vec![174, 0],
                },
            )
        };
        let txs = pack(&oracle, &payer, (0..9).map(step).collect());
        assert!(txs.len() > 1 && txs.len() < 9, "{}", txs.len());
        assert_eq!(txs.iter().map(|t| t.steps.len()).sum::<usize>(), 9);
        let gate = GateBinding {
            controller: Pubkey::new_unique(),
            target: oracle,
            address: Pubkey::new_unique(),
            epoch: 7,
        };
        for tx in &txs {
            let ixs = bind_oracle(Some(&gate), tx.instructions()).unwrap();
            assert!(wire_size(&Transaction::new_with_payer(&ixs, Some(&payer))) <= PACKET_SIZE);
        }
        // Each transaction is as full as it can be: the next step would not fit.
        let first = &txs[0];
        let mut grown = first.clone();
        grown.push("claimReward", txs[1].steps[0].1.clone());
        let ixs = bind_oracle(Some(&gate), grown.instructions()).unwrap();
        assert!(wire_size(&Transaction::new_with_payer(&ixs, Some(&payer))) > PACKET_SIZE);
        assert!(pack(&oracle, &payer, Vec::new()).is_empty());
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
