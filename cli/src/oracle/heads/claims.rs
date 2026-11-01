//! Claims and challenges on heads: a fill (an active head) or an update (a live source), by
//! anyone with the claim bond, commits and reveals in one call (tag 210/15 then 210/16, two
//! transactions: the reveal comes a slot after the commit), a claim challenge (210/17) or a
//! source challenge (210/22) carries a bond, a reason code and a reason, and `settle` does what
//! is due once a window ends (210/19 finalize, 210/20 settle each challenge, 210/21 refund an
//! unrevealed commit, 210/23 resolve a source case, 210/10 expire an open head).
//!
//! Evidence: a Wayback capture is a kind-3 object, a web proof (its bundle file or digest, or
//! its `amoeba-evidence-v1;` record) kind 4, any other file kind 2 (opaque bytes: an offline
//! quote); kinds 3 and 4 fix the claim's time to their capture's. A challenge with no evidence
//! seals its reason as a kind-2 object instead, so whoever rules can read it on chain.

use std::time::Duration;

use ameba_oracle::{
    constants::{
        ORACLE_EVIDENCE_RECORD_PREFIX, ORACLE_OPENING_ARCHIVE_URL_PREFIX,
        ORACLE_UPDATE_MIN_REVEAL_DELAY_SLOTS,
    },
    instruction::{
        self as ix,
        heads::{
            self as hx, ChallengeClaimParams, ChallengeSourceParams, HeadsLtaTerms,
            RevealClaimParams, heads_claim_commitment, heads_reason_hash,
        },
    },
    state::heads::{
        HeadChallengeReason, HeadChallengeRuling, HeadClaimKind, HeadClaimStatus, HeadStatus,
        HeadsChannel, SOURCE_CASE_FELL, SOURCE_CASE_NONE, SOURCE_CASE_OPEN, SOURCE_CASE_STOOD,
        find_head_challenge_address,
    },
};
use ameba_oracle_heads::channels::LtaTerms;
use serde_json::{Value, json};
use solana_program::{hash::hashv, instruction::Instruction, pubkey::Pubkey};

use super::{
    accounts::{self, BoardRef, Chain},
    args::{ChallengeArgs, ClaimArgs, LtaArgs, ReasonCode, SourceChallengeArgs},
    at, atoms, commands,
    docs::{self, claim_status},
    landed, material, parse_time, parse_value, set, value,
    writes::{OBJECT_MAX, head_line, send, tx},
};
use crate::{
    futures::{
        Report,
        ctx::Ctx,
        error::{Error, Result, bail},
        units::{parse_usdc, usdc},
        view::Paint,
    },
    oracle::{
        send::{self as osend, Tx, Write},
        skus::{hex, parse_hex32},
        writes::{archive_time, evidence_txs, random32},
    },
};

/// How far ahead of the chain's clock a capture may be for a commit to wait for the clock
/// rather than refuse (early review B7: validators' clocks trail wall time by seconds).
const CLOCK_WAIT_SECS: u64 = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Fill,
    Update,
}

impl Kind {
    fn claim_kind(self) -> HeadClaimKind {
        match self {
            Kind::Fill => HeadClaimKind::Fill,
            Kind::Update => HeadClaimKind::Update,
        }
    }

    fn word(self) -> &'static str {
        match self {
            Kind::Fill => "fill",
            Kind::Update => "update",
        }
    }
}

// --------------------------------------------------------------------------------- evidence

/// What a claim or challenge cites, resolved: the sealed object's kind, bytes and commitment,
/// its capture time (kinds 3 and 4), and a web proof's value and bundle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cited {
    pub kind: u8,
    pub bytes: Vec<u8>,
    pub commitment: [u8; 32],
    pub time: Option<u64>,
    pub value: Option<u64>,
    pub bundle: Option<String>,
    pub what: String,
}

/// `--evidence`: a Wayback URL (kind 3), an evidence record or a web proof bundle (kind 4), or a
/// file of opaque bytes (kind 2).
pub fn cite(ctx: &Ctx, text: &str) -> Result<Cited> {
    let t = text.trim();
    if t.starts_with(ORACLE_OPENING_ARCHIVE_URL_PREFIX) {
        let time = archive_time(t)?;
        return Ok(Cited {
            kind: 3,
            bytes: t.as_bytes().to_vec(),
            commitment: ix::archive_url_hash(t),
            time: Some(time),
            value: None,
            bundle: None,
            what: format!("the Wayback capture {t}"),
        });
    }
    if t.starts_with(ORACLE_EVIDENCE_RECORD_PREFIX) {
        let (_, time, _, _) = amoeba_evidence::parse_onchain_record(t)
            .ok_or_else(|| Error::msg("not an amoeba-evidence-v1 record"))?;
        return Ok(record(t.to_string(), time, None, None));
    }
    if t.starts_with("https://") || t.starts_with("http://") {
        bail(format!(
            "Cite a Wayback capture of the page ({ORACLE_OPENING_ARCHIVE_URL_PREFIX}<14-digit UTC time>/<url>), a web proof, or a file, not the live page."
        ))?;
    }
    // A web proof: its bundle by file or digest in Petri's evidence folder.
    if let Ok((path, json)) = crate::evidence::load(t)
        && amoeba_evidence::bundle::peek_method(json.as_bytes()).is_some()
    {
        let v = amoeba_evidence::verify_bundle(json.as_bytes(), &crate::evidence::policy(ctx)?);
        if !v.authenticated() {
            let reason = [
                &v.stages.received,
                &v.stages.provenance,
                &v.stages.extraction,
            ]
            .iter()
            .find_map(|s| s.reason.clone())
            .unwrap_or_else(|| "it does not verify".into());
            bail(format!("{} does not verify: {reason}", path.display()))?;
        }
        let env = v.envelope.clone().expect("authenticated");
        let time = env
            .times
            .witnessed_at
            .value
            .parse::<u64>()
            .map_err(|_| Error::msg("the proof has no session time"))?;
        return Ok(record(
            amoeba_evidence::onchain_record(&env),
            time,
            Some(env.observation.price.atoms),
            Some(env.authentication.bundle_digest.clone()),
        ));
    }
    let path = std::path::Path::new(t);
    let bytes = std::fs::read(path).map_err(|e| Error::msg(format!("cannot read {t}: {e}")))?;
    if bytes.is_empty() {
        bail(format!("{t} is empty"))?;
    }
    if bytes.len() > OBJECT_MAX {
        bail(format!(
            "{t} is {} bytes; a sealed object holds at most {OBJECT_MAX}: cite a smaller file, or a Wayback capture or web proof.",
            bytes.len()
        ))?;
    }
    Ok(opaque(bytes, format!("the file {t}")))
}

fn record(text: String, time: u64, value: Option<u64>, bundle: Option<String>) -> Cited {
    Cited {
        kind: 4,
        commitment: ix::archive_url_hash(&text),
        bytes: text.into_bytes(),
        time: Some(time),
        value,
        what: match &bundle {
            Some(d) => format!("the web proof {d}"),
            None => "an authenticated evidence record".to_string(),
        },
        bundle,
    }
}

/// Opaque bytes as a kind-2 object: committed by their SHA-256.
fn opaque(bytes: Vec<u8>, what: String) -> Cited {
    Cited {
        kind: 2,
        commitment: hashv(&[&bytes]).to_bytes(),
        bytes,
        time: None,
        value: None,
        bundle: None,
        what,
    }
}

/// An origin: `PUBLISHER/REPORT` (`commodity_spec::molds::origin_id`) or its 64-hex id.
pub fn origin(text: &str) -> Result<[u8; 32]> {
    let t = text.trim();
    if t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit()) {
        let id = parse_hex32(t, "origin id")?;
        if id == [0; 32] {
            bail("An origin id is never zero.")?;
        }
        return Ok(id);
    }
    let Some((publisher, report)) = t.split_once('/') else {
        bail(format!(
            "`{t}` is not an origin: give PUBLISHER/REPORT, who first published the price and the report it is in (\"TrendForce/DRAM contract price table\"), or the 64-hex origin id"
        ))?
    };
    if publisher.trim().is_empty() || report.trim().is_empty() {
        bail("An origin names both its publisher and its report: PUBLISHER/REPORT.")?;
    }
    Ok(ameba_commodity_spec::molds::origin_id(publisher, report))
}

/// The LTA terms a claim declares: only on an LTA head, consistent, and current.
fn lta_terms(
    head: u32,
    channel: HeadsChannel,
    a: &LtaArgs,
    value: u64,
    source_ts: u64,
    now: u64,
) -> Result<HeadsLtaTerms> {
    if channel != HeadsChannel::Lta {
        if !a.is_empty() {
            bail(format!(
                "Only an LTA head's claims carry agreement terms; head {head} observes another channel."
            ))?;
        }
        return Ok(HeadsLtaTerms::default());
    }
    let time = |t: &Option<String>| {
        t.as_deref()
            .map(parse_time)
            .transpose()
            .map(|v| v.unwrap_or(0))
    };
    let price = |t: &Option<String>| {
        t.as_deref()
            .map(parse_value)
            .transpose()
            .map(|v| v.unwrap_or(0))
    };
    let terms = HeadsLtaTerms {
        effective_from_ts: time(&a.effective_from)?,
        next_reset_ts: time(&a.next_reset)?,
        floor: price(&a.floor)?,
        ceiling: price(&a.ceiling)?,
    };
    let lta = LtaTerms {
        effective_from_ts: terms.effective_from_ts,
        next_reset_ts: terms.next_reset_ts,
        floor: terms.floor,
        ceiling: terms.ceiling,
    };
    if !lta.valid() {
        bail(
            "Those terms don't hold together: the next reset comes after the effective date, and the floor is at most the ceiling.",
        )?;
    }
    if terms.effective_from_ts > source_ts {
        bail(format!(
            "That price takes effect on {}, after the capture ({}): the oracle takes an LTA price only once it is in effect, so capture it again from then.",
            at(terms.effective_from_ts),
            at(source_ts)
        ))?;
    }
    if lta.expired_at(now) {
        bail(format!(
            "That price's next reset ({}) has passed: an LTA claim is the price in effect now.",
            at(terms.next_reset_ts)
        ))?;
    }
    if !lta.within_band(value) {
        bail("That value is outside the agreement's floor and ceiling.")?;
    }
    Ok(terms)
}

fn usdc_bond(text: Option<&str>, floor: u64) -> Result<u64> {
    let bond = match text {
        Some(t) => parse_usdc(t)?,
        None => floor,
    };
    if bond < floor {
        bail(format!("The bond is at least {} USDC.", usdc(floor)))?;
    }
    Ok(bond)
}

/// The board's challenge bond floor: at least one atom, so every winner has a share.
fn challenge_floor(board: &BoardRef) -> u64 {
    board.board.params.challenge_bond.max(1)
}

fn collateral_for(chain: &Chain, owner: &Pubkey, amount: u64, what: &str) -> Result<u64> {
    let available = chain
        .oracle
        .collateral(owner)?
        .map_or(0, |c| c.available_balance);
    if available < amount {
        bail(format!(
            "Your oracle collateral has {} USDC available; {what} is {} USDC. Deposit with petri oracle collateral deposit <USDC>.",
            usdc(available),
            usdc(amount)
        ))?;
    }
    Ok(available)
}

fn evidence_json(c: Option<&Cited>, object: Option<&Pubkey>) -> Value {
    match (c, object) {
        (Some(c), Some(o)) => {
            json!({ "kind": c.kind, "commitment": hex(&c.commitment), "object": o.to_string() })
        }
        _ => Value::Null,
    }
}

// ----------------------------------------------------------------------------- fill, update

/// `petri oracle heads fill|update <head> --product P --value V --time T --origin O …`.
pub fn claim(ctx: &Ctx, p: Paint, kind: Kind, a: &ClaimArgs) -> Result<Report> {
    let chain = Chain::new(ctx)?;
    let program = chain.program();
    chain.config()?;
    let fetcher = ctx.wallet_pubkey()?;
    let board = chain.board(accounts::product(&a.product)?)?;
    let head = a.head;
    let (head_key, h) = chain.head(&board, head)?;
    let addresses = board.head_addresses(&program, head);
    let params = board.params();
    let symbol = board.symbol();
    match (kind, h.status) {
        (Kind::Fill, HeadStatus::Active) if h.open_fill => bail(format!(
            "A fill of head {head} is open (claim {}): a different value challenges it (petri oracle heads challenge {head} {} --product {symbol} --value <V> --reason \"…\").",
            h.open_fill_seq, h.open_fill_seq
        ))?,
        (Kind::Fill, HeadStatus::Active) | (Kind::Update, HeadStatus::Live) => {}
        (Kind::Fill, HeadStatus::Open) => bail(format!(
            "Head {head} is still gathering backing: it can be filled once it is active."
        ))?,
        (Kind::Fill, HeadStatus::Live) => bail(format!(
            "Head {head} is live already: keep it current with petri oracle heads update."
        ))?,
        (Kind::Update, HeadStatus::Active) => bail(format!(
            "Head {head} is not live yet: an active head takes a fill (petri oracle heads fill)."
        ))?,
        (_, HeadStatus::Retired | HeadStatus::Expired) | (Kind::Update, HeadStatus::Open) => {
            bail(format!(
                "Head {head} is {}: it takes no claims.",
                docs::status_word(h.status)
            ))?
        }
    }
    let bond = params.claim_bond;
    let available = collateral_for(&chain, &fetcher, bond, "the claim bond")?;
    let cited = a
        .evidence
        .evidence
        .as_deref()
        .map(|e| cite(ctx, e))
        .transpose()?;
    let claim_value = match (&a.value, cited.as_ref().and_then(|c| c.value)) {
        (Some(v), proven) => {
            let v = parse_value(v)?;
            if proven.is_some_and(|x| x != v) {
                bail(format!(
                    "--value {} is not what the proof shows ({}); leave --value out to use it.",
                    value(v),
                    value(proven.unwrap_or(0))
                ))?;
            }
            v
        }
        (None, Some(proven)) => proven,
        (None, None) => bail("Give --value: the number the head's spec asks for, in its unit.")?,
    };
    let source_ts = match (&a.time, cited.as_ref().and_then(|c| c.time)) {
        (Some(t), captured) => {
            let t = parse_time(t)?;
            if captured.is_some_and(|c| c != t) {
                bail(format!(
                    "The evidence was captured at {}, but --time is {}: a claim's time is its capture's (leave --time out to use it).",
                    at(captured.unwrap_or(0)),
                    at(t)
                ))?;
            }
            t
        }
        (None, Some(captured)) => captured,
        (None, None) => bail("Give --time: when the source published the value.")?,
    };
    // A commit never runs ahead of its observation: the program refuses a reveal dated after its
    // commit, so a capture a little ahead of the chain's clock waits for it (early review B7).
    let mut now = chain.now()?;
    if source_ts > now.unix && source_ts - now.unix <= CLOCK_WAIT_SECS {
        while now.unix < source_ts {
            std::thread::sleep(Duration::from_secs(1));
            now = chain.now()?;
        }
    }
    if source_ts > now.unix {
        bail(format!(
            "--time {} is after the chain's clock ({}): the oracle takes no time from its future.",
            at(source_ts),
            at(now.unix)
        ))?;
    }
    let origin_id = origin(&a.origin)?;
    let lta = lta_terms(head, h.channel, &a.lta, claim_value, source_ts, now.unix)?;
    let seq = h.claim_seq;
    let claim_key = addresses.claim(&program, seq);
    let (evidence_kind, evidence_commitment) = cited
        .as_ref()
        .map_or((0, [0; 32]), |c| (c.kind, c.commitment));
    let reveal = RevealClaimParams {
        value: claim_value,
        source_ts,
        origin_id,
        lta,
        salt: random32()?,
        evidence_kind,
        evidence_commitment,
    };
    let commit_hash = heads_claim_commitment(&head_key, seq, &fetcher, &reveal);
    let mut txs = Vec::new();
    let mut object = None;
    if let Some(c) = &cited {
        let (o, written, _) =
            evidence_txs(&chain.oracle, &fetcher, c.kind, &c.bytes, c.commitment)?;
        object = Some(o);
        txs.extend(written);
    }
    txs.push(tx(
        "commitClaim",
        hx::commit_claim(
            &program,
            &fetcher,
            &fetcher,
            &board.id,
            head,
            seq,
            kind.claim_kind(),
            commit_hash,
            bond,
        ),
    ));
    let saved = if a.send.simulate {
        None
    } else {
        Some(material::save(
            &claim_key,
            material::Material::of(
                &program,
                symbol,
                head,
                seq,
                &fetcher,
                &reveal,
                object.as_ref(),
                cited.as_ref().and_then(|c| c.bundle.clone()),
            ),
        )?)
    };
    let prize = match kind {
        Kind::Fill => ameba_oracle_heads::rewards::fill_prize(&params, h.delegation),
        Kind::Update => ameba_oracle_heads::rewards::update_prize(&params, h.delegation),
    };
    let window = params.challenge_window_secs;
    let mut review = vec![
        format!(
            "{} as of {}, origin {}",
            value(claim_value),
            at(source_ts),
            crate::futures::units::short(&hex(&origin_id))
        ),
        match &cited {
            Some(c) => format!(
                "Evidence: {} (sealed as your public object {})",
                c.what,
                object.map(|o| o.to_string()).unwrap_or_default()
            ),
            None => {
                "No evidence cited: a challenger only has to show the value is wrong".to_string()
            }
        },
        format!(
            "Bond {} USDC from your oracle collateral ({} available): the board's claim bond",
            usdc(bond),
            usdc(available)
        ),
        format!(
            "Commits now and reveals a slot later (two transactions); then it is open to challenge for {}",
            crate::futures::view::span(i64::try_from(window).unwrap_or(i64::MAX))
        ),
        match kind {
            Kind::Fill => format!(
                "If it stands, the head goes live and you earn the fill prize (about {} USDC); if it falls, the bond goes to its challengers",
                usdc(prize)
            ),
            Kind::Update => format!(
                "If it stands, you earn an update prize (about {} USDC) and it is the source's new value; if it falls, the bond goes to its challengers",
                usdc(prize)
            ),
        },
    ];
    if lta != HeadsLtaTerms::default() {
        review.push(format!(
            "LTA terms: in effect from {}, next reset {}, floor {}, ceiling {}",
            if lta.effective_from_ts == 0 {
                "-".into()
            } else {
                at(lta.effective_from_ts)
            },
            if lta.next_reset_ts == 0 {
                "-".into()
            } else {
                at(lta.next_reset_ts)
            },
            if lta.floor == 0 {
                "-".into()
            } else {
                value(lta.floor)
            },
            if lta.ceiling == 0 {
                "-".into()
            } else {
                value(lta.ceiling)
            },
        ));
    }
    let details = json!({
        "product": symbol, "head": head, "name": h.name(), "seq": seq, "kind": kind.word(),
        "claim": claim_key.to_string(), "value": atoms(claim_value), "sourceTs": source_ts,
        "originId": hex(&origin_id), "bond": atoms(bond), "prize": atoms(prize),
        "evidence": evidence_json(cited.as_ref(), object.as_ref()),
        "revealMaterial": saved.as_ref().map(|p| p.display().to_string()),
    });
    let action = match kind {
        Kind::Fill => "heads.fill",
        Kind::Update => "heads.update",
    };
    let write = Write {
        action,
        title: format!(
            "{} {} with {}",
            if kind == Kind::Fill { "Fill" } else { "Update" },
            head_line(&board, &h),
            value(claim_value)
        ),
        review,
        details,
        txs,
    };
    let mut report = send(ctx, p, write, a.send.simulate)?;
    if !landed(&report) {
        report.plain.push_str("\n  The reveal follows once the commit lands (not simulated: the claim is not on chain yet).");
        set(&mut report, json!({ "revealed": false }));
        return Ok(report);
    }
    // Straight on to the reveal: it is due from the slot after the commit.
    let revealed = await_reveal(&chain, &addresses, head, seq)
        .and_then(|_| reveal_send(ctx, p, &chain, &board, head, seq, true));
    match revealed {
        Ok(r) => {
            let mut transactions = report.payload["transactions"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            transactions.extend(
                r.payload["transactions"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default(),
            );
            set(
                &mut report,
                json!({
                    "revealed": true,
                    "signature": r.payload["signature"],
                    "windowEndsAt": r.payload["windowEndsAt"],
                    "transactions": transactions,
                }),
            );
            report.plain.push('\n');
            report.plain.push_str(&r.plain);
            Ok(report)
        }
        Err(e) => {
            let why = e.message.replace('\n', " | ");
            let command = commands::reveal(symbol, head, seq);
            let deadline = chain
                .claim(&addresses, head, seq)
                .map(|(_, c)| c.reveal_deadline_slot)
                .ok();
            report.plain.push_str(&format!(
                "\n{}\n  Reveal it {}: {command}",
                p.red(&format!("The commit landed, but the reveal failed: {why}")),
                deadline
                    .map(|d| format!("by slot {d}"))
                    .unwrap_or_else(|| "soon".to_string()),
            ));
            set(
                &mut report,
                json!({
                    "ok": false,
                    "revealed": false,
                    "revealCommand": command,
                    "revealDeadlineSlot": deadline,
                    "error": {
                        "code": "ORACLE_REVEAL_FAILED",
                        "category": "execution_failed",
                        "message": format!("the commit landed, but the reveal failed: {why}"),
                        "retryable": true,
                    },
                }),
            );
            Ok(report)
        }
    }
}

/// Waits until the chain reaches the claim's first reveal slot; refuses once its window closed.
fn await_reveal(
    chain: &Chain,
    a: &ameba_oracle::instruction::heads::HeadAddresses,
    head: u32,
    seq: u32,
) -> Result<()> {
    for _ in 0..600 {
        let slot = chain.ctx().rpc.slot()?;
        if let Ok((_, c)) = chain.claim(a, head, seq) {
            if slot > c.reveal_deadline_slot {
                bail(format!(
                    "its reveal window closed at slot {}",
                    c.reveal_deadline_slot
                ))?;
            }
            if slot >= c.commit_slot + ORACLE_UPDATE_MIN_REVEAL_DELAY_SLOTS {
                return Ok(());
            }
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    bail("the chain did not reach the reveal slot in time")
}

/// `petri oracle heads reveal <head> <seq> --product P`: tag 210/16 from Petri's private file.
pub fn reveal(
    ctx: &Ctx,
    p: Paint,
    product: &str,
    head: u32,
    seq: u32,
    simulate: bool,
) -> Result<Report> {
    let chain = Chain::new(ctx)?;
    chain.config()?;
    let board = chain.board(accounts::product(product)?)?;
    if simulate {
        return reveal_write(ctx, &chain, &board, head, seq)
            .and_then(|(w, _)| send(ctx, p, w, true));
    }
    reveal_send(ctx, p, &chain, &board, head, seq, false)
}

/// The reveal of claim `seq`, checked against the claim and its saved material.
fn reveal_write(
    ctx: &Ctx,
    chain: &Chain,
    board: &BoardRef,
    head: u32,
    seq: u32,
) -> Result<(Write, material::Material)> {
    let program = chain.program();
    let fetcher = ctx.wallet_pubkey()?;
    let (head_key, h) = chain.head(board, head)?;
    let a = board.head_addresses(&program, head);
    let (claim_key, c) = chain.claim(&a, head, seq)?;
    if c.fetcher != fetcher {
        bail(format!(
            "Claim {seq} on head {head} is {}'s: only its fetcher reveals it.",
            c.fetcher
        ))?;
    }
    if c.status != HeadClaimStatus::Committed {
        bail(format!(
            "Claim {seq} on head {head} is {} already.",
            claim_status(&c)
        ))?;
    }
    let slot = ctx.rpc.slot()?;
    if slot > c.reveal_deadline_slot {
        bail(format!(
            "Claim {seq}'s reveal window closed at slot {}; its bond comes back with {}.",
            c.reveal_deadline_slot,
            commands::settle(board.symbol(), head, Some(seq))
        ))?;
    }
    let m = material::load(&claim_key, head, seq)?;
    let params = m.params()?;
    if heads_claim_commitment(&head_key, seq, &fetcher, &params) != c.commit_hash {
        bail(format!(
            "Petri's saved reveal material is not what claim {seq} committed to."
        ))?;
    }
    let object = m.evidence_object()?;
    let write = Write {
        action: "heads.reveal",
        title: format!(
            "Reveal claim {seq} on {}: {}",
            head_line(board, &h),
            value(params.value)
        ),
        review: vec![
            format!("{} as of {}", value(params.value), at(params.source_ts)),
            format!(
                "Then open to challenge for {}",
                crate::futures::view::span(
                    i64::try_from(board.params().challenge_window_secs).unwrap_or(i64::MAX)
                )
            ),
        ],
        details: json!({
            "product": board.symbol(), "head": head, "name": h.name(), "seq": seq,
            "kind": docs::kind_word(c.kind), "value": atoms(params.value), "sourceTs": params.source_ts,
            "bond": atoms(c.bond),
        }),
        txs: vec![tx(
            "revealClaim",
            hx::reveal_claim(
                &program,
                &fetcher,
                &board.id,
                head,
                seq,
                params,
                object.as_ref(),
            )
            .map_err(|e| Error::msg(format!("cannot build the reveal: {e}")))?,
        )],
    };
    Ok((write, m))
}

fn reveal_send(
    ctx: &Ctx,
    p: Paint,
    chain: &Chain,
    board: &BoardRef,
    head: u32,
    seq: u32,
    confirmed: bool,
) -> Result<Report> {
    let (write, m) = reveal_write(ctx, chain, board, head, seq)?;
    let report = if confirmed {
        osend::run_confirmed(ctx, p, write).map_err(super::writes::plain_error)?
    } else {
        osend::run(ctx, p, write, false).map_err(super::writes::plain_error)?
    };
    let mut report = super::flatten(report);
    let a = board.head_addresses(&chain.program(), head);
    let claim_key = a.claim(&chain.program(), seq);
    material::mark_revealed(&claim_key)?;
    if let Ok((_, c)) = chain.claim(&a, head, seq) {
        set(&mut report, json!({ "windowEndsAt": c.window_end_ts }));
        report.plain.push_str(&format!(
            "\nOpen to challenge until {}.",
            at(c.window_end_ts)
        ));
    }
    if let Some(digest) = &m.evidence_bundle
        && let Err(e) = crate::evidence::onchain::publish(ctx, digest)
    {
        report.plain.push_str(&format!(
            "\n  The web proof did not upload ({}); petri evidence submit can retry.",
            e.message
        ));
    }
    Ok(report)
}

// ------------------------------------------------------------------------------- challenges

fn reason_code(c: ReasonCode) -> HeadChallengeReason {
    match c {
        ReasonCode::Value => HeadChallengeReason::ValueOrOther,
        ReasonCode::OffChannel => HeadChallengeReason::OffChannel,
        ReasonCode::DuplicateOrigin => HeadChallengeReason::DuplicateOrigin,
        ReasonCode::ExpiredObservation => HeadChallengeReason::ExpiredObservation,
    }
}

/// The reason text, checked, and what the challenge cites: `--evidence`, else the reason
/// sealed as a kind-2 object.
fn reason_and_cite(ctx: &Ctx, reason: &str, evidence: Option<&str>) -> Result<(String, Cited)> {
    let reason = reason.trim().to_string();
    if reason.is_empty() {
        bail("Say what is wrong and what shows it with --reason.")?;
    }
    if reason.len() > OBJECT_MAX {
        bail("That reason is too long. Shorten it.")?;
    }
    let cited = match evidence {
        Some(e) => cite(ctx, e)?,
        None => opaque(reason.as_bytes().to_vec(), "your reason".to_string()),
    };
    Ok((reason, cited))
}

/// `petri oracle heads challenge <head> <seq> --product P --reason R …`: tag 210/17.
pub fn challenge(ctx: &Ctx, p: Paint, a: &ChallengeArgs) -> Result<Report> {
    let chain = Chain::new(ctx)?;
    let program = chain.program();
    chain.config()?;
    let challenger = ctx.wallet_pubkey()?;
    let board = chain.board(accounts::product(&a.claim.product)?)?;
    let (head, seq) = (a.claim.head, a.claim.seq);
    let (_, h) = chain.head(&board, head)?;
    let addresses = board.head_addresses(&program, head);
    let (claim_key, c) = chain.claim(&addresses, head, seq)?;
    let now = chain.now()?.unix;
    match c.status {
        HeadClaimStatus::Revealed if now < c.window_end_ts => {}
        HeadClaimStatus::Revealed => bail(format!(
            "Claim {seq}'s challenge window closed at {}.",
            at(c.window_end_ts)
        ))?,
        HeadClaimStatus::Committed => bail(format!(
            "Claim {seq} is not revealed yet: a claim is challenged once its value is on chain."
        ))?,
        _ => bail(format!("Claim {seq} is {} already.", claim_status(&c)))?,
    }
    let pda = find_head_challenge_address(&program, &claim_key, &challenger).0;
    if ctx.rpc.account(&pda)?.is_some() {
        bail(format!(
            "You challenged claim {seq} on head {head} already."
        ))?;
    }
    let bond = usdc_bond(a.bond.as_deref(), challenge_floor(&board))?;
    let available = collateral_for(&chain, &challenger, bond, "the challenge bond")?;
    let (reason, cited) = reason_and_cite(ctx, &a.reason, a.evidence.evidence.as_deref())?;
    let alternative = a
        .value
        .as_deref()
        .map(parse_value)
        .transpose()?
        .or_else(|| {
            // A web proof shows the right value itself.
            a.evidence.evidence.as_ref().and(cited.value)
        });
    let alternative_ts = match alternative {
        None => 0,
        Some(_) => match (&a.time, cited.time) {
            (Some(t), captured) => {
                let t = parse_time(t)?;
                if captured.is_some_and(|x| x != t) {
                    bail(format!(
                        "The evidence was captured at {}, but --time is {}: leave --time out to use the capture's.",
                        at(captured.unwrap_or(0)),
                        at(t)
                    ))?;
                }
                t
            }
            (None, Some(captured)) => captured,
            (None, None) => c.source_ts,
        },
    };
    let (object, mut txs, _) = evidence_txs(
        &chain.oracle,
        &challenger,
        cited.kind,
        &cited.bytes,
        cited.commitment,
    )?;
    let params = ChallengeClaimParams {
        bond,
        reason: reason_code(a.code),
        alternative_value: alternative.unwrap_or(0),
        alternative_source_ts: alternative_ts,
        evidence_kind: cited.kind,
        evidence_commitment: cited.commitment,
        reason_hash: heads_reason_hash(reason.as_bytes()),
    };
    txs.push(tx(
        "challengeClaim",
        hx::challenge_claim(
            &program,
            &challenger,
            &challenger,
            &board.id,
            head,
            seq,
            params.clone(),
            Some(&object),
        )
        .map_err(|e| Error::msg(format!("cannot build the challenge: {e}")))?,
    ));
    let extension = board.params().challenge_extension_secs;
    let write = Write {
        action: "heads.challenge",
        title: format!(
            "Challenge claim {seq} ({} {}) on {}",
            docs::kind_word(c.kind),
            value(c.value),
            head_line(&board, &h)
        ),
        review: vec![
            format!("Ground: {}; reason: {}", docs::reason_word(params.reason), crate::backend::terminal_safe_text(&reason)),
            match alternative {
                Some(v) => format!("The right value: {} as of {}", value(v), at(alternative_ts)),
                None => "No other value given".to_string(),
            },
            format!("Evidence: {} (public object {object})", cited.what),
            format!("Bond {} USDC from your oracle collateral ({} available)", usdc(bond), usdc(available)),
            "If it falls, its challengers share its bond and get theirs back; if it stands, your bond goes to its fetcher".to_string(),
            if c.window_end_ts.saturating_sub(now) < extension {
                format!("A late challenge: the window moves to {}", at(now + extension))
            } else {
                format!("The council rules once the window ends at {}", at(c.window_end_ts))
            },
        ],
        details: json!({
            "product": board.symbol(), "head": head, "name": h.name(), "seq": seq, "target": "claim",
            "claim": claim_key.to_string(), "bond": atoms(bond), "code": docs::reason_word(params.reason),
            "reason": reason, "value": alternative.map(atoms),
            "evidence": evidence_json(Some(&cited), Some(&object)),
        }),
        txs,
    };
    let mut report = send(ctx, p, write, a.send.simulate)?;
    if landed(&report)
        && let Ok((_, c)) = chain.claim(&addresses, head, seq)
    {
        set(
            &mut report,
            json!({ "windowEndsAt": c.window_end_ts, "challenges": c.challenge_count }),
        );
    }
    Ok(report)
}

/// `petri oracle heads challenge-source <head> --product P --reason R`: tag 210/22.
pub fn challenge_source(ctx: &Ctx, p: Paint, a: &SourceChallengeArgs) -> Result<Report> {
    let chain = Chain::new(ctx)?;
    let program = chain.program();
    chain.config()?;
    let challenger = ctx.wallet_pubkey()?;
    let board = chain.board(accounts::product(&a.head.product)?)?;
    let head = a.head.head;
    let (head_key, h) = chain.head(&board, head)?;
    if !matches!(h.status, HeadStatus::Active | HeadStatus::Live) {
        bail(format!(
            "Head {head} is {}: only an active or live head's source is challenged.",
            docs::status_word(h.status)
        ))?;
    }
    let now = chain.now()?.unix;
    match h.source_case_status {
        SOURCE_CASE_OPEN if now >= h.source_case_window_end_ts => bail(format!(
            "Head {head}'s source case closed to new challenges at {}; it waits for its ruling.",
            at(h.source_case_window_end_ts)
        ))?,
        SOURCE_CASE_FELL if h.source_case_settled_winners < h.source_case_won => bail(format!(
            "Head {head}'s last source case is still paying its challengers: {}.",
            commands::settle(board.symbol(), head, None)
        ))?,
        _ => {}
    }
    let pda = find_head_challenge_address(&program, &head_key, &challenger).0;
    if ctx.rpc.account(&pda)?.is_some() {
        bail(format!(
            "You have a source challenge on head {head} already; once it is settled ({}) you can challenge again.",
            commands::settle(board.symbol(), head, None)
        ))?;
    }
    let bond = usdc_bond(a.bond.as_deref(), challenge_floor(&board))?;
    let available = collateral_for(&chain, &challenger, bond, "the challenge bond")?;
    let (reason, cited) = reason_and_cite(ctx, &a.reason, a.evidence.evidence.as_deref())?;
    let (object, mut txs, _) = evidence_txs(
        &chain.oracle,
        &challenger,
        cited.kind,
        &cited.bytes,
        cited.commitment,
    )?;
    let params = ChallengeSourceParams {
        bond,
        reason: reason_code(a.code),
        evidence_kind: cited.kind,
        evidence_commitment: cited.commitment,
        reason_hash: heads_reason_hash(reason.as_bytes()),
    };
    txs.push(tx(
        "challengeSource",
        hx::challenge_source(
            &program,
            &challenger,
            &challenger,
            &board.id,
            head,
            params.clone(),
            Some(&object),
        )
        .map_err(|e| Error::msg(format!("cannot build the challenge: {e}")))?,
    ));
    let write = Write {
        action: "heads.challenge-source",
        title: format!("Challenge the source {}", head_line(&board, &h)),
        review: vec![
            format!("Ground: {}; reason: {}", docs::reason_word(params.reason), crate::backend::terminal_safe_text(&reason)),
            format!("Evidence: {} (public object {object})", cited.what),
            format!("Bond {} USDC from your oracle collateral ({} available)", usdc(bond), usdc(available)),
            "If the source falls, its winning challengers share the proposer's whole delegation on it, the other delegators are slashed and it takes a strike; if it stands, your bond goes to the proposer".to_string(),
        ],
        details: json!({
            "product": board.symbol(), "head": head, "name": h.name(), "target": "source",
            "bond": atoms(bond), "code": docs::reason_word(params.reason), "reason": reason,
            "evidence": evidence_json(Some(&cited), Some(&object)),
        }),
        txs,
    };
    let mut report = send(ctx, p, write, a.send.simulate)?;
    if landed(&report)
        && let Ok((_, h)) = chain.head(&board, head)
    {
        set(
            &mut report,
            json!({ "windowEndsAt": h.source_case_window_end_ts }),
        );
    }
    Ok(report)
}

// ------------------------------------------------------------------------------------ settle

/// The order challenges are paid in: every one but the last winner, then the last winner (its
/// share is what is left).
fn settle_order(
    challenges: &mut [(Pubkey, Box<ameba_oracle::state::heads::HeadChallenge>)],
    last_winner: Option<u16>,
) {
    challenges.sort_by_key(|(_, c)| {
        let last = last_winner == Some(c.index) && c.ruling == HeadChallengeRuling::Won;
        (last, c.index)
    });
}

/// `petri oracle heads settle <head> [<seq>] --product P`: what is due, permissionlessly.
pub fn settle(
    ctx: &Ctx,
    p: Paint,
    product: &str,
    head: u32,
    seq: Option<u32>,
    simulate: bool,
) -> Result<Report> {
    let chain = Chain::new(ctx)?;
    let program = chain.program();
    chain.config()?;
    let payer = ctx.wallet_pubkey()?;
    let board = chain.board(accounts::product(product)?)?;
    let (head_key, h) = chain.head(&board, head)?;
    let addresses = board.head_addresses(&program, head);
    let now = chain.now()?.unix;
    let symbol = board.symbol();
    let mut steps: Vec<(String, Instruction)> = Vec::new();
    let mut review = Vec::new();
    let mut details = json!({ "product": symbol, "head": head, "name": h.name() });
    match seq {
        Some(seq) => {
            let (claim_key, c) = chain.claim(&addresses, head, seq)?;
            details["seq"] = json!(seq);
            let finalizing = match c.status {
                HeadClaimStatus::Committed => {
                    let slot = ctx.rpc.slot()?;
                    if slot <= c.reveal_deadline_slot {
                        bail(format!(
                            "Claim {seq} is committed and its fetcher can still reveal it until slot {} (now {slot}).",
                            c.reveal_deadline_slot
                        ))?;
                    }
                    steps.push((
                        "refundExpiredCommit".to_string(),
                        hx::refund_expired_commit(&program, &board.id, head, seq, &c.fetcher),
                    ));
                    review.push(format!(
                        "Claim {seq} was never revealed: its bond goes back to its fetcher"
                    ));
                    false
                }
                HeadClaimStatus::Revealed => {
                    if now < c.window_end_ts {
                        bail(format!(
                            "Claim {seq} is open to challenge until {}.",
                            at(c.window_end_ts)
                        ))?;
                    }
                    let timed_out = c.ruled_count != c.challenge_count;
                    if timed_out && now < c.ruling_deadline_ts {
                        bail(format!(
                            "{} of claim {seq}'s {} challenges wait for the council's ruling (until {}; then they time out).",
                            c.challenge_count - c.ruled_count,
                            c.challenge_count,
                            at(c.ruling_deadline_ts)
                        ))?;
                    }
                    if chain.oracle.collateral(&c.fetcher)?.is_none() {
                        bail(format!(
                            "Claim {seq}'s fetcher has no oracle collateral to settle into."
                        ))?;
                    }
                    let month = ameba_oracle_heads::time::month_id(now);
                    steps.push((
                        "finalizeClaim".to_string(),
                        hx::finalize_claim(
                            &program, &payer, &board.id, head, seq, &c.fetcher, month,
                        ),
                    ));
                    review.push(if c.challenge_count == 0 {
                        format!("Claim {seq} stood unchallenged: it is accepted")
                    } else if timed_out {
                        format!(
                            "Claim {seq} resolves at its ruling deadline on the rulings it has; its unruled challenges time out (their bonds back)"
                        )
                    } else {
                        format!(
                            "Claim {seq}'s {} challenges are ruled: it is resolved once",
                            c.challenge_count
                        )
                    });
                    true
                }
                HeadClaimStatus::Expired => bail(format!(
                    "Claim {seq} was never revealed and its bond went back."
                ))?,
                HeadClaimStatus::Stood | HeadClaimStatus::Fell => false,
            };
            // Once the claim resolved, an unruled challenge timed out: it settles too (its bond back).
            let resolved =
                finalizing || matches!(c.status, HeadClaimStatus::Stood | HeadClaimStatus::Fell);
            let mut challenges: Vec<_> = chain
                .challenges_of(&head_key, &claim_key)?
                .into_iter()
                .filter(|(_, x)| {
                    !x.settled && (resolved || x.ruling != HeadChallengeRuling::Pending)
                })
                .collect();
            if finalizing || matches!(c.status, HeadClaimStatus::Stood | HeadClaimStatus::Fell) {
                // After the finalize the claim is Stood or Fell; the last winner pays last.
                settle_order(&mut challenges, (c.won_count > 0).then_some(c.last_winner));
                for (_, x) in &challenges {
                    steps.push((
                        "settleChallenge".to_string(),
                        hx::settle_claim_challenge(
                            &program,
                            &board.id,
                            head,
                            seq,
                            &x.challenger,
                            &x.rent_payer,
                        ),
                    ));
                }
                if !challenges.is_empty() {
                    review.push(format!(
                        "Pays its {} challenge{} out",
                        challenges.len(),
                        if challenges.len() == 1 { "" } else { "s" }
                    ));
                }
            }
            if steps.is_empty() {
                bail(format!(
                    "Claim {seq} on head {head} is {}: nothing is due.",
                    claim_status(&c)
                ))?;
            }
        }
        None => {
            if h.status == HeadStatus::Open && now >= h.expires_ts {
                steps.push((
                    "expireHead".to_string(),
                    hx::expire_head(&program, &board.id, head),
                ));
                review.push(format!(
                    "Head {head} did not turn active by {}: it expires and its delegations cool",
                    at(h.expires_ts)
                ));
            }
            let mut finalizing = false;
            match h.source_case_status {
                SOURCE_CASE_OPEN if now < h.source_case_window_end_ts => review.push(format!(
                    "Its source case is open to challenge until {}",
                    at(h.source_case_window_end_ts)
                )),
                SOURCE_CASE_OPEN
                    if h.source_case_ruled != h.source_case_challenges
                        && now < h.source_case_deadline_ts =>
                {
                    bail(format!(
                        "{} of head {head}'s source challenges wait for the council's ruling (until {}; then they time out).",
                        h.source_case_challenges - h.source_case_ruled,
                        at(h.source_case_deadline_ts)
                    ))?
                }
                SOURCE_CASE_OPEN => {
                    steps.push((
                        "finalizeSourceChallenges".to_string(),
                        hx::finalize_source_challenges(&program, &board.id, head, &h.proposer),
                    ));
                    review.push("Its source case is ruled: it is resolved once".to_string());
                    finalizing = true;
                }
                SOURCE_CASE_NONE | SOURCE_CASE_STOOD | SOURCE_CASE_FELL => {}
                _ => {}
            }
            // Once a case resolved, its unruled challenges timed out: they settle too (bonds back).
            let resolved =
                finalizing || matches!(h.source_case_status, SOURCE_CASE_STOOD | SOURCE_CASE_FELL);
            let mut challenges: Vec<_> = chain
                .challenges_of(&head_key, &head_key)?
                .into_iter()
                .filter(|(_, x)| {
                    !x.settled
                        && (x.ruling != HeadChallengeRuling::Pending
                            || x.case_number < h.source_case
                            || resolved)
                })
                .collect();
            if finalizing || matches!(h.source_case_status, SOURCE_CASE_STOOD | SOURCE_CASE_FELL) {
                settle_order(
                    &mut challenges,
                    (h.source_case_won > 0).then_some(h.source_case_last_winner),
                );
                for (_, x) in &challenges {
                    if x.ruling == HeadChallengeRuling::Won && chain.stake(&x.challenger)?.is_none()
                    {
                        bail(format!(
                            "Challenger {} won a share of the proposer's AMBA but has no oracle stake account to receive it: they stake first (petri oracle heads stake <AMBA>).",
                            x.challenger
                        ))?;
                    }
                    steps.push((
                        "settleChallenge".to_string(),
                        hx::settle_source_challenge(
                            &program,
                            &board.id,
                            head,
                            &x.challenger,
                            &x.rent_payer,
                        ),
                    ));
                }
                if !challenges.is_empty() {
                    review.push(format!(
                        "Pays its {} source challenge{} out",
                        challenges.len(),
                        if challenges.len() == 1 { "" } else { "s" }
                    ));
                }
            }
            if steps.is_empty() {
                bail(format!(
                    "Nothing is due on head {head} itself{}; a claim's settlement names it: {}.",
                    review
                        .first()
                        .map(|r| format!(" ({})", r.to_lowercase()))
                        .unwrap_or_default(),
                    commands::settle(symbol, head, Some(0)).replace(" 0 ", " <SEQ> ")
                ))?;
            }
        }
    }
    details["steps"] = json!(steps.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>());
    // One instruction a transaction: each loads the board, the head and its companions, and
    // a challenge's settlement needs its claim resolved by the one before.
    let txs: Vec<Tx> = steps.into_iter().map(|(name, ix)| tx(&name, ix)).collect();
    let write = Write {
        action: "heads.settle",
        title: format!(
            "Settle what is due on {}{}",
            head_line(&board, &h),
            seq.map(|s| format!(", claim {s}")).unwrap_or_default()
        ),
        review,
        details,
        txs,
    };
    let mut report = send(ctx, p, write, simulate)?;
    if landed(&report)
        && let Some(seq) = seq
        && let Ok((_, c)) = chain.claim(&addresses, head, seq)
    {
        set(
            &mut report,
            json!({ "status": claim_status(&c), "headStatus": docs::status_word(chain.head(&board, head)?.1.status) }),
        );
        report
            .plain
            .push_str(&format!("\nClaim {seq} is {}.", claim_status(&c)));
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ameba_oracle::state::heads::{HeadChallenge, zeroed_box};

    #[test]
    fn origins_are_the_molds_or_hex() {
        let id = origin("TrendForce/DRAM contract price table").unwrap();
        assert_eq!(
            id,
            ameba_commodity_spec::molds::origin_id("TrendForce", "DRAM contract price table")
        );
        // The publisher and report are normalized (case, spacing) by the shared rule.
        assert_eq!(
            origin(" trendforce /dram  CONTRACT price table").unwrap(),
            id
        );
        assert_eq!(origin(&hex(&id)).unwrap(), id);
        assert!(origin("TrendForce").is_err());
        assert!(origin("/report").is_err());
        assert!(origin(&"00".repeat(32)).is_err());
    }

    #[test]
    fn opaque_evidence_commits_to_its_bytes() {
        let c = opaque(b"quote".to_vec(), "q".into());
        assert_eq!((c.kind, c.commitment), (2, hashv(&[b"quote"]).to_bytes()));
        assert_eq!(c.time, None);
        let r = record(
            "amoeba-evidence-v1;m=web;t=1793178000;d=ab;u=https://x".into(),
            1_793_178_000,
            Some(5),
            None,
        );
        assert_eq!(r.kind, 4);
        assert_eq!(
            r.commitment,
            ix::archive_url_hash("amoeba-evidence-v1;m=web;t=1793178000;d=ab;u=https://x")
        );
    }

    #[test]
    fn lta_terms_only_on_lta_heads_and_current() {
        let none = LtaArgs::default();
        let some = LtaArgs {
            next_reset: Some("2000".into()),
            ..LtaArgs::default()
        };
        assert_eq!(
            lta_terms(1, HeadsChannel::Contract, &none, 5, 1000, 1000).unwrap(),
            HeadsLtaTerms::default()
        );
        assert!(lta_terms(1, HeadsChannel::Spot, &some, 5, 1000, 1000).is_err());
        assert_eq!(
            lta_terms(1, HeadsChannel::Lta, &some, 5, 1000, 1000)
                .unwrap()
                .next_reset_ts,
            2000
        );
        assert!(
            lta_terms(1, HeadsChannel::Lta, &some, 5, 2000, 2000).is_err(),
            "past its reset"
        );
        let band = LtaArgs {
            floor: Some("4".into()),
            ceiling: Some("6".into()),
            ..LtaArgs::default()
        };
        assert!(
            lta_terms(1, HeadsChannel::Lta, &band, 7_000_000, 10, 10).is_err(),
            "outside the band"
        );
        assert!(lta_terms(1, HeadsChannel::Lta, &band, 5_000_000, 10, 10).is_ok());
        let backwards = LtaArgs {
            effective_from: Some("3000".into()),
            next_reset: Some("2000".into()),
            ..LtaArgs::default()
        };
        assert!(lta_terms(1, HeadsChannel::Lta, &backwards, 5, 1000, 1000).is_err());
        // Announced ahead of its date: refused until a capture from the date on (round 3 R3).
        let announced = LtaArgs {
            effective_from: Some("3000".into()),
            ..LtaArgs::default()
        };
        assert!(lta_terms(1, HeadsChannel::Lta, &announced, 5, 2999, 2999).is_err());
        assert!(lta_terms(1, HeadsChannel::Lta, &announced, 5, 3000, 3100).is_ok());
    }

    #[test]
    fn the_last_winner_is_paid_last() {
        let mut list: Vec<(Pubkey, Box<HeadChallenge>)> = (0..4u16)
            .map(|i| {
                let mut c = zeroed_box::<HeadChallenge>();
                c.index = i;
                c.ruling = if i % 2 == 0 {
                    HeadChallengeRuling::Won
                } else {
                    HeadChallengeRuling::Lost
                };
                (Pubkey::new_unique(), c)
            })
            .collect();
        settle_order(&mut list, Some(0));
        assert_eq!(
            list.iter().map(|(_, c)| c.index).collect::<Vec<_>>(),
            vec![1, 2, 3, 0]
        );
    }
}
