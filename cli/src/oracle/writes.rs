//! Oracle writes, each built with the pinned oracle crate's instruction
//! builders against the trusted deployment's oracle program, reviewed, and
//! signed by the attached wallet (see [`super::send`]).

use ameba_oracle::{
    constants::{
        ORACLE_EVIDENCE_RECORD_PREFIX, ORACLE_KILL_WINDOW_SECONDS,
        ORACLE_OPENING_ARCHIVE_URL_PREFIX, ORACLE_OPENING_WINDOW_SECONDS,
        ORACLE_PLACEMENT_WINDOW_SECONDS, ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS,
    },
    instruction::{
        self as ix, PrelistingEscrow, ProposeOracleSourceV3Params, SupportOracleSourceV3Params,
    },
    state::{
        OracleMonthState, OracleOpeningClaim, OracleOpeningClaimChallenge, OraclePhase,
        OracleSkuCoverageManifest, OracleSourceChallenge, OracleSourceStatus,
        OracleUpdateChallenge, OracleUpdateClaimV2, OracleUsdcRewardSchedulePhase,
    },
};
use chrono::NaiveDateTime;
use serde_json::{Value, json};
use solana_program::{hash::hashv, pubkey::Pubkey};

use super::{
    args::{EscrowKind, EvidenceKind, SendArgs, Target},
    chain::{MonthRef, Oracle, SourceRef, decode},
    send::{self, Tx, Write},
    skus::{self, hex, parse_hex32},
};
use crate::futures::{
    Report,
    ctx::Ctx,
    error::{Error, Result, bail},
    txbuild::{ata, create_ata_idempotent},
    units::{parse_decimal, parse_usdc, trim_decimal, usdc},
    view::{Paint, utc},
};

/// The oracle program's commitment domains for source type and definition
/// text (the futures API's candidate store commits with the same ones).
pub(super) const SOURCE_TYPE_DOMAIN: &str = "ameba-oracle-source-type-v1";
const SOURCE_DEFINITION_DOMAIN: &str = "ameba-oracle-source-definition-v1";
const SOURCE_ID_DOMAIN: &[u8] = b"ameba-petri-source-id-v1";
/// Bytes per evidence chunk (the program's limit) and chunks per transaction.
const CHUNK: usize = 192;
const CHUNKS_PER_TX: usize = 3;
pub(super) const OBJECT_HEADER: usize = 76;

pub(super) fn usdc_amount(text: &str, what: &str) -> Result<u64> {
    let atoms = parse_usdc(text)?;
    if atoms == 0 {
        bail(format!("{what} must be more than zero"))?;
    }
    Ok(atoms)
}

/// A source value in its six-decimal atoms (`75.5` → 75 500 000).
fn value_atoms(text: &str) -> Result<u64> {
    let atoms = parse_decimal(text, 6)?;
    u64::try_from(atoms)
        .ok()
        .filter(|a| *a > 0)
        .ok_or_else(|| Error::msg(format!("`{text}` is not a positive value")))
}

pub(super) fn random32() -> Result<[u8; 32]> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| Error::msg(format!("no randomness: {e}")))?;
    Ok(bytes)
}

pub(super) fn id_or_random(text: Option<&str>, what: &str) -> Result<[u8; 32]> {
    match text {
        Some(text) => parse_hex32(text, what),
        None => random32(),
    }
}

/// `SHA-256(domain || 0 || u32-LE length || text)`.
pub(super) fn semantic_hash(domain: &str, text: &str) -> [u8; 32] {
    let len = (text.len() as u32).to_le_bytes();
    hashv(&[domain.as_bytes(), &[0], &len, text.as_bytes()]).to_bytes()
}

/// The unix time a Wayback archive URL names, after checking its shape.
pub fn archive_time(url: &str) -> Result<u64> {
    let rest = url.strip_prefix(ORACLE_OPENING_ARCHIVE_URL_PREFIX).ok_or_else(|| {
        Error::msg(format!(
            "the archive URL must start with {ORACLE_OPENING_ARCHIVE_URL_PREFIX}<14-digit UTC time>/"
        ))
    })?;
    let stamp = rest.get(..14).unwrap_or("");
    if rest.as_bytes().get(14) != Some(&b'/') || !stamp.bytes().all(|b| b.is_ascii_digit()) {
        bail(format!(
            "the archive URL must be {ORACLE_OPENING_ARCHIVE_URL_PREFIX}<YYYYMMDDhhmmss>/<source URL>"
        ))?;
    }
    let at = NaiveDateTime::parse_from_str(stamp, "%Y%m%d%H%M%S")
        .map_err(|_| Error::msg(format!("`{stamp}` is not a valid UTC time")))?;
    u64::try_from(at.and_utc().timestamp())
        .map_err(|_| Error::msg("the archive time is before 1970"))
}

/// The bytes an evidence object holds for `text`, and their commitment.
pub fn evidence_preimage(kind: EvidenceKind, text: &str) -> Result<(u8, Vec<u8>, [u8; 32])> {
    Ok(match kind {
        EvidenceKind::Locator => {
            if !text.starts_with("https://") && !text.starts_with("http://") {
                bail("a locator is a public http(s) URL")?;
            }
            (
                1,
                text.as_bytes().to_vec(),
                hashv(&[b"locator", text.as_bytes()]).to_bytes(),
            )
        }
        EvidenceKind::Definition => {
            let len = (text.len() as u32).to_le_bytes();
            let mut bytes = SOURCE_DEFINITION_DOMAIN.as_bytes().to_vec();
            bytes.push(0);
            bytes.extend_from_slice(&len);
            bytes.extend_from_slice(text.as_bytes());
            let hash = hashv(&[&bytes]).to_bytes();
            (2, bytes, hash)
        }
        EvidenceKind::Archive => {
            archive_time(text)?;
            (3, text.as_bytes().to_vec(), ix::archive_url_hash(text))
        }
        // Committed like an archive URL, under the same domain.
        EvidenceKind::Record => {
            amoeba_evidence::parse_onchain_record(text)
                .ok_or_else(|| Error::msg("not an amoeba-evidence-v1 record"))?;
            (4, text.as_bytes().to_vec(), ix::archive_url_hash(text))
        }
    })
}

/// What an opening, update or challenge cites: a Wayback archive URL
/// (kind 3), or a web proof bundle, cited as its authenticated evidence
/// record (kind 4, `crate::evidence::onchain`).
#[derive(Clone, Copy, Debug)]
pub enum Cite<'a> {
    Archive(&'a str),
    Proof(&'a str),
}

impl<'a> Cite<'a> {
    pub fn of(archive_url: &'a Option<String>, evidence: &'a Option<String>) -> Self {
        match (archive_url, evidence) {
            (_, Some(bundle)) => Self::Proof(bundle),
            (Some(url), None) => Self::Archive(url),
            (None, None) => Self::Archive(""),
        }
    }
}

/// The cited text and what it carries, resolved for one source.
pub(super) struct Cited {
    pub text: String,
    pub kind: EvidenceKind,
    pub time: u64,
    pub state: u64,
    /// The proof's bundle digest (to publish with the claim).
    pub bundle: Option<String>,
}

pub(super) fn cite(
    ctx: &Ctx,
    oracle: &Oracle,
    month: &MonthRef,
    source: &SourceRef,
    cited: Cite,
    value: Option<&str>,
    signer: &Pubkey,
) -> Result<Cited> {
    match cited {
        Cite::Archive(url) => Ok(Cited {
            time: archive_time(url)?,
            state: value_atoms(value.ok_or_else(|| Error::msg("give --value"))?)?,
            text: url.to_string(),
            kind: EvidenceKind::Archive,
            bundle: None,
        }),
        Cite::Proof(bundle) => {
            let r = crate::evidence::onchain::record(
                ctx,
                &oracle.program,
                &month.address,
                &source.state.source_id,
                &source.state.canonical_locator_hash,
                signer,
                bundle,
            )?;
            if let Some(v) = value
                && value_atoms(v)? != r.atoms
            {
                bail(format!(
                    "--value {v} is not what the proof shows ({}); leave --value out to use it",
                    r.price
                ))?;
            }
            Ok(Cited {
                text: r.record,
                kind: EvidenceKind::Record,
                time: r.time,
                state: r.atoms,
                bundle: Some(r.digest),
            })
        }
    }
}

/// The cited time and page of an archive URL or an evidence record.
pub(super) fn archive_parts(text: &str) -> Result<(u64, String)> {
    if text.starts_with(ORACLE_EVIDENCE_RECORD_PREFIX) {
        let (_, time, _, page) = amoeba_evidence::parse_onchain_record(text)
            .ok_or_else(|| Error::msg("not an amoeba-evidence-v1 record"))?;
        return Ok((time, page.to_string()));
    }
    let time = archive_time(text)?;
    Ok((
        time,
        text[ORACLE_OPENING_ARCHIVE_URL_PREFIX.len() + 15..].to_string(),
    ))
}

pub(super) fn cited_kind(text: &str) -> EvidenceKind {
    if text.starts_with(ORACLE_EVIDENCE_RECORD_PREFIX) {
        EvidenceKind::Record
    } else {
        EvidenceKind::Archive
    }
}

/// The text a definition object's bytes hold: they are
/// `domain || 0 || u32-LE length || text` (see [`evidence_preimage`]).
pub(super) fn definition_text(body: &[u8]) -> Option<String> {
    let mut prefix = SOURCE_DEFINITION_DOMAIN.as_bytes().to_vec();
    prefix.push(0);
    let rest = body.strip_prefix(prefix.as_slice())?;
    let (length, text) = rest.split_at_checked(4)?;
    let length = u32::from_le_bytes(length.try_into().ok()?) as usize;
    (length == text.len())
        .then(|| String::from_utf8(text.to_vec()).ok())
        .flatten()
}

/// What an evidence object on chain says about itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectHeader {
    pub payer: Pubkey,
    pub kind: u8,
    pub hash: [u8; 32],
    pub total: usize,
    pub written: usize,
    pub sealed: bool,
}

fn object_header(oracle: &Oracle, object: &Pubkey) -> Result<Option<(ObjectHeader, Vec<u8>)>> {
    let Some(account) = oracle.ctx.rpc.account(object)? else {
        return Ok(None);
    };
    let d = &account.data;
    if account.owner != oracle.program || d.len() < OBJECT_HEADER || d[..5] != *b"OEB\x01\x01" {
        bail(format!("{object} is not an oracle evidence object"))?;
    }
    let header = ObjectHeader {
        payer: Pubkey::new_from_array(d[6..38].try_into().unwrap()),
        kind: d[38],
        hash: d[39..71].try_into().unwrap(),
        total: usize::from(u16::from_le_bytes([d[71], d[72]])),
        written: usize::from(u16::from_le_bytes([d[73], d[74]])),
        sealed: d[75] == 1,
    };
    let body =
        d[OBJECT_HEADER..OBJECT_HEADER + header.written.min(d.len() - OBJECT_HEADER)].to_vec();
    Ok(Some((header, body)))
}

/// The transactions that write and seal `bytes` as the payer's evidence
/// object; none when it is already sealed. Resumes a partial draft.
pub(super) fn evidence_txs(
    oracle: &Oracle,
    payer: &Pubkey,
    kind: u8,
    bytes: &[u8],
    hash: [u8; 32],
) -> Result<(Pubkey, Vec<Tx>, &'static str)> {
    let object = ix::find_evidence_object_address(&oracle.program, payer, kind, &hash);
    let total = u16::try_from(bytes.len())
        .ok()
        .filter(|t| *t > 0)
        .ok_or_else(|| Error::msg("evidence is empty or too long"))?;
    let start = match object_header(oracle, &object)? {
        Some((h, _)) if h.sealed => return Ok((object, Vec::new(), "already sealed")),
        Some((h, body)) => {
            if h.total != bytes.len() || !bytes.starts_with(&body) {
                bail(format!(
                    "evidence draft {object} holds different bytes; close it first with `petri oracle evidence close {object}`"
                ))?;
            }
            h.written
        }
        None => 0,
    };
    let chunks: Vec<(usize, &[u8])> = bytes[start..]
        .chunks(CHUNK)
        .enumerate()
        .map(|(i, c)| (start + i * CHUNK, c))
        .collect();
    let txs = chunks
        .chunks(CHUNKS_PER_TX)
        .map(|group| {
            let mut tx = Tx::new();
            for (offset, chunk) in group {
                tx.push(
                    "writeEvidence",
                    ix::write_evidence(
                        &oracle.program,
                        payer,
                        kind,
                        hash,
                        total,
                        *offset as u16,
                        chunk.to_vec(),
                    ),
                );
            }
            tx
        })
        .collect();
    Ok((object, txs, if start > 0 { "resumed" } else { "new" }))
}

fn kind_name(kind: u8) -> &'static str {
    match kind {
        1 => "locator",
        2 => "definition",
        3 => "archive",
        4 => "evidence record",
        _ => "unknown",
    }
}

pub(super) fn read_body(text: Option<&str>, file: Option<&std::path::Path>) -> Result<String> {
    match (text, file) {
        (Some(text), _) => Ok(text.to_string()),
        (None, Some(path)) => std::fs::read_to_string(path)
            .map(|t| t.trim_end_matches(['\n', '\r']).to_string())
            .map_err(|e| Error::msg(format!("cannot read {}: {e}", path.display()))),
        (None, None) => bail("give --text or --file"),
    }
}

pub(super) fn month_line(month: &MonthRef) -> String {
    format!(
        "{} ({}, period {}, month {}, phase {})",
        month.period.expiry_id(),
        month.period.label(),
        month.period.status_name(),
        month.address,
        month.phase_name()
    )
}

pub(super) fn month_json(month: &MonthRef) -> Value {
    json!({
        "expiryId": month.period.expiry_id(),
        "period": month.period.address.to_string(),
        "month": month.address.to_string(),
        "phase": month.phase_name(),
    })
}

pub(super) fn source_json(source: &SourceRef) -> Value {
    json!({
        "source": source.address.to_string(),
        "sourceId": hex(&source.state.source_id),
        "bucket": source.bucket_label(),
        "bucketName": source.bucket_name(),
        "status": format!("{:?}", source.state.status),
        "currentState": trim_decimal(u128::from(source.state.current_state), 6),
    })
}

// ---------------------------------------------------------------- collateral

/// `petri oracle collateral [--owner]`.
pub fn collateral_show(ctx: &Ctx, p: Paint, owner: Option<&str>) -> Result<Report> {
    let oracle = Oracle::new(ctx)?;
    let owner = ctx.owner(owner)?;
    let ledger = oracle.collateral(&owner)?;
    let address = oracle.collateral_address(&owner);
    let lines = match &ledger {
        Some(l) => vec![
            p.bold(&format!("Oracle collateral of {owner}")),
            format!("  Available  {} USDC", usdc(l.available_balance)),
            format!("  Locked     {} USDC (bonds and stakes in escrow)", usdc(l.position_locked_balance)),
            format!("  Ledger     {address}"),
        ],
        None => vec![
            format!("{owner} has no oracle collateral yet ({address})."),
            "Deposit with `petri oracle collateral deposit <USDC>`; bonds and stakes are debited from it.".to_string(),
        ],
    };
    let payload = json!({
        "ok": true,
        "owner": owner.to_string(),
        "ledger": address.to_string(),
        "exists": ledger.is_some(),
        "availableAtoms": ledger.as_ref().map(|l| l.available_balance.to_string()),
        "lockedAtoms": ledger.as_ref().map(|l| l.position_locked_balance.to_string()),
        "oracle": oracle.program.to_string(),
    });
    Ok(Report::new(payload, lines.join("\n")))
}

/// `petri oracle collateral deposit <USDC>`: tags 9 (once) and 10.
pub fn collateral_deposit(ctx: &Ctx, p: Paint, amount: &str, send: &SendArgs) -> Result<Report> {
    send::run(ctx, p, deposit_write(ctx, amount)?, send.simulate)
}

/// The deposit to review and send (the CLI, and the TUI's Fund view).
pub fn deposit_write(ctx: &Ctx, amount: &str) -> Result<Write> {
    let oracle = Oracle::new(ctx)?;
    let owner = ctx.wallet_pubkey()?;
    let mint = ctx.deployment()?.usdc_mint;
    let atoms = usdc_amount(amount, "the deposit")?;
    let token = ata(&owner, &mint);
    let mut tx = Tx::new();
    let exists = oracle.collateral(&owner)?.is_some();
    if !exists {
        tx.push(
            "initCollateral",
            ix::init_user_collateral(&oracle.program, &owner),
        );
    }
    tx.push(
        "depositCollateral",
        ix::deposit_collateral(&oracle.program, &owner, &token, &mint, atoms),
    );
    let write = Write {
        action: "oracle.collateral.deposit",
        title: format!("Deposit {} USDC into oracle collateral", usdc(atoms)),
        review: vec![
            format!("From {token} (your USDC account) to the oracle bond vault"),
            if exists {
                "Ledger exists".to_string()
            } else {
                "Opens your collateral ledger first (rent paid by you)".to_string()
            },
        ],
        details: json!({"amountAtoms": atoms.to_string(), "ledger": oracle.collateral_address(&owner).to_string()}),
        txs: vec![tx],
    };
    Ok(write)
}

/// `petri oracle collateral withdraw <USDC|all>`: tag 11.
pub fn collateral_withdraw(ctx: &Ctx, p: Paint, amount: &str, send: &SendArgs) -> Result<Report> {
    send::run(ctx, p, withdraw_write(ctx, amount)?, send.simulate)
}

/// The withdrawal to review and send (the CLI, and the TUI's Fund view).
pub fn withdraw_write(ctx: &Ctx, amount: &str) -> Result<Write> {
    let oracle = Oracle::new(ctx)?;
    let owner = ctx.wallet_pubkey()?;
    let mint = ctx.deployment()?.usdc_mint;
    let ledger = oracle
        .collateral(&owner)?
        .ok_or_else(|| Error::msg(format!("{owner} has no oracle collateral")))?;
    let atoms = if amount.trim().eq_ignore_ascii_case("all") {
        ledger.available_balance
    } else {
        usdc_amount(amount, "the withdrawal")?
    };
    if atoms == 0 || atoms > ledger.available_balance {
        bail(format!(
            "{} USDC is available to withdraw",
            usdc(ledger.available_balance)
        ))?;
    }
    let token = ata(&owner, &mint);
    let mut tx = Tx::new();
    tx.push(
        "createTokenAccount",
        create_ata_idempotent(&owner, &owner, &mint),
    );
    tx.push(
        "withdrawCollateral",
        ix::withdraw_collateral(&oracle.program, &owner, &token, &mint, atoms),
    );
    let write = Write {
        action: "oracle.collateral.withdraw",
        title: format!("Withdraw {} USDC from oracle collateral", usdc(atoms)),
        review: vec![format!("To {token} (your USDC account)")],
        details: json!({"amountAtoms": atoms.to_string()}),
        txs: vec![tx],
    };
    Ok(write)
}

// ------------------------------------------------------------------ evidence

/// `petri oracle evidence write --kind … --text|--file`.
pub fn evidence_write(
    ctx: &Ctx,
    p: Paint,
    kind: EvidenceKind,
    text: Option<&str>,
    file: Option<&std::path::Path>,
    send: &SendArgs,
) -> Result<Report> {
    let oracle = Oracle::new(ctx)?;
    let payer = ctx.wallet_pubkey()?;
    let body = read_body(text, file)?;
    let (kind_byte, bytes, hash) = evidence_preimage(kind, &body)?;
    let (object, txs, state) = evidence_txs(&oracle, &payer, kind_byte, &bytes, hash)?;
    let details = json!({
        "object": object.to_string(),
        "kind": kind_name(kind_byte),
        "hash": hex(&hash),
        "bytes": bytes.len(),
    });
    if txs.is_empty() {
        let payload = json!({"ok": true, "action": "oracle.evidence.write", "alreadySealed": true, "details": details});
        return Ok(Report::new(
            payload,
            format!(
                "Evidence {object} is already sealed ({} bytes, hash {}).",
                bytes.len(),
                hex(&hash)
            ),
        ));
    }
    let write = Write {
        action: "oracle.evidence.write",
        title: format!(
            "Write {} evidence ({} bytes) to {object}",
            kind_name(kind_byte),
            bytes.len()
        ),
        review: vec![
            format!("Commitment {}", hex(&hash)),
            format!("{state} object; sealed by its final chunk; public and immutable once sealed"),
        ],
        details,
        txs,
    };
    send::run(ctx, p, write, send.simulate)
}

/// `petri oracle evidence show`.
pub fn evidence_show(
    ctx: &Ctx,
    p: Paint,
    object: Option<&str>,
    kind: Option<EvidenceKind>,
    text: Option<&str>,
    payer: Option<&str>,
) -> Result<Report> {
    let oracle = Oracle::new(ctx)?;
    let address = match (object, kind, text) {
        (Some(object), _, _) => ctx.parse_pubkey(object, "evidence object")?,
        (None, Some(kind), Some(text)) => {
            let payer = ctx.owner(payer)?;
            let (kind, _, hash) = evidence_preimage(kind, text)?;
            ix::find_evidence_object_address(&oracle.program, &payer, kind, &hash)
        }
        _ => bail("give an object address, or --kind and --text")?,
    };
    let Some((h, body)) = object_header(&oracle, &address)? else {
        return Ok(Report::new(
            json!({"ok": true, "object": address.to_string(), "exists": false}),
            format!("No evidence object at {address}."),
        ));
    };
    // A definition object holds `domain || 0 || u32 length || text`.
    let mut prefix = SOURCE_DEFINITION_DOMAIN.as_bytes().to_vec();
    prefix.push(0);
    let shown = if h.kind == 2 && body.starts_with(&prefix) && body.len() >= prefix.len() + 4 {
        &body[prefix.len() + 4..]
    } else {
        &body[..]
    };
    let text = String::from_utf8_lossy(shown).to_string();
    let lines = [
        p.bold(&format!("Evidence {address}")),
        format!("  Kind     {} ({})", kind_name(h.kind), h.kind),
        format!("  Payer    {}", h.payer),
        format!("  Hash     {}", hex(&h.hash)),
        format!(
            "  Bytes    {} of {}{}",
            h.written,
            h.total,
            if h.sealed { ", sealed" } else { ", draft" }
        ),
        format!("  Content  {}", crate::backend::terminal_safe_text(&text)),
    ];
    let payload = json!({
        "ok": true, "object": address.to_string(), "exists": true,
        "kind": kind_name(h.kind), "payer": h.payer.to_string(), "hash": hex(&h.hash),
        "total": h.total, "written": h.written, "sealed": h.sealed, "content": text,
    });
    Ok(Report::new(payload, lines.join("\n")))
}

/// `petri oracle evidence close <OBJECT>`: tag 30 selector 15.
pub fn evidence_close(ctx: &Ctx, p: Paint, object: &str, send: &SendArgs) -> Result<Report> {
    let oracle = Oracle::new(ctx)?;
    let payer = ctx.wallet_pubkey()?;
    let object = ctx.parse_pubkey(object, "evidence object")?;
    match object_header(&oracle, &object)? {
        None => bail(format!("no evidence object at {object}"))?,
        Some((h, _)) if h.sealed => {
            bail(format!("{object} is sealed; sealed evidence is permanent"))?
        }
        Some((h, _)) if h.payer != payer => bail(format!(
            "{object} was written by {}, not this wallet",
            h.payer
        ))?,
        Some(_) => {}
    }
    let write = Write {
        action: "oracle.evidence.close",
        title: format!("Close evidence draft {object}"),
        review: vec!["Refunds the draft's rent to this wallet".to_string()],
        details: json!({"object": object.to_string()}),
        txs: vec![Tx::one(
            "closeEvidenceDraft",
            ix::close_evidence_draft(&oracle.program, &payer, &object),
        )],
    };
    send::run(ctx, p, write, send.simulate)
}

// ------------------------------------------------------------------- sources

#[allow(clippy::too_many_arguments)]
pub fn source_propose(
    ctx: &Ctx,
    p: Paint,
    target: &Target,
    bucket: &str,
    url: &str,
    source_type: &str,
    definition: Option<&str>,
    definition_file: Option<&std::path::Path>,
    bond: &str,
    source_id: Option<&str>,
) -> Result<Report> {
    let oracle = Oracle::new(ctx)?;
    let month = oracle.month(&target.market, target.expiry.as_deref())?;
    let bucket_id = skus::parse_bucket(bucket)?;
    let definition = read_body(definition, definition_file)?;
    let listing_bond = usdc_amount(bond, "the listing bond")?;
    let source_id = source_id
        .map(|id| parse_hex32(id, "source id"))
        .transpose()?;
    let mut write = propose_write(
        &oracle,
        &month,
        &Proposal {
            bucket_id,
            url,
            source_type,
            definition: &definition,
            listing_bond,
            source_id,
        },
    )?;
    // A chip month's sources count as comparison series only.
    if oracle.chip_product(&month).ok().flatten().is_some()
        && !ameba_commodity_spec::SeriesDefinition::claims_schema(&definition)
    {
        write.review.push(format!(
            "{} is on chip buckets: a source counts once its definition is a comparison series (`petri oracle series template`, then `petri oracle series propose`)",
            month.period.expiry_id()
        ));
    }
    send::run(ctx, p, write, target.send.simulate)
}

/// What a source proposal registers.
pub(super) struct Proposal<'a> {
    pub bucket_id: [u8; 32],
    pub url: &'a str,
    pub source_type: &'a str,
    pub definition: &'a str,
    pub listing_bond: u64,
    /// Derived from the month, bucket and URL when None.
    pub source_id: Option<[u8; 32]>,
}

/// The source proposal to review and send: its locator and definition
/// evidence, then the proposal with its bond and SKU proof (`sources
/// propose`, `series propose`).
pub(super) fn propose_write(
    oracle: &Oracle,
    month: &MonthRef,
    proposal: &Proposal,
) -> Result<Write> {
    let proposer = oracle.ctx.wallet_pubkey()?;
    let bucket_id = proposal.bucket_id;
    let url = proposal.url;
    let (sku_index, sku_proof) = oracle.sku_proof(month, &bucket_id)?;
    let (_, locator_bytes, locator_hash) = evidence_preimage(EvidenceKind::Locator, url)?;
    let (_, definition_bytes, definition_hash) =
        evidence_preimage(EvidenceKind::Definition, proposal.definition)?;
    let source_id = proposal.source_id.unwrap_or_else(|| {
        hashv(&[
            SOURCE_ID_DOMAIN,
            month.address.as_ref(),
            &bucket_id,
            &locator_hash,
        ])
        .to_bytes()
    });
    let listing_bond = proposal.listing_bond;
    let source = ix::find_oracle_source_address(&oracle.program, &month.address, &source_id);
    let mut txs = Vec::new();
    let (locator_object, locator_txs, _) =
        evidence_txs(oracle, &proposer, 1, &locator_bytes, locator_hash)?;
    let (definition_object, definition_txs, _) =
        evidence_txs(oracle, &proposer, 2, &definition_bytes, definition_hash)?;
    txs.extend(locator_txs);
    txs.extend(definition_txs);
    txs.push(Tx::one(
        "proposeSource",
        ix::propose_oracle_source_v3(
            &oracle.program,
            &proposer,
            &proposer,
            &month.period.address,
            &month.address,
            ProposeOracleSourceV3Params {
                source_id,
                bucket_id,
                source_type_hash: semantic_hash(SOURCE_TYPE_DOMAIN, proposal.source_type),
                canonical_locator_hash: locator_hash,
                source_definition_hash: definition_hash,
                listing_bond,
                sku_index,
                sku_proof,
            },
        ),
    ));
    Ok(Write {
        action: "oracle.sources.propose",
        title: format!(
            "Propose a {} source for {} with a {} USDC listing bond",
            skus::bucket_name(&bucket_id),
            month.period.expiry_id(),
            usdc(listing_bond)
        ),
        review: vec![
            month_line(month),
            format!("URL {url}"),
            format!("Source {source} (id {})", hex(&source_id)),
            format!("Evidence: locator {locator_object}, definition {definition_object}"),
            "The bond is debited from your oracle collateral".to_string(),
        ],
        details: json!({"month": month_json(month), "source": source.to_string(), "sourceId": hex(&source_id), "bondAtoms": listing_bond.to_string()}),
        txs,
    })
}

/// When a month takes sources and support, on the chain's clock (the
/// program's `ensure_oracle_coverage_source_submission_window`): while it
/// takes sources, from its planned Scramble start for as long as Scramble's
/// review days still fit before its expiry; in Scramble, on its four
/// placement days, before the recipe freeze.
pub(super) fn check_placement(
    month: &OracleMonthState,
    coverage: &OracleSkuCoverageManifest,
    expiry: u64,
    now: u64,
    id: &str,
) -> Result<()> {
    let at = |t: u64| utc(i64::try_from(t).unwrap_or(i64::MAX));
    if month.weight_scheme_version == 255 {
        bail(format!(
            "{id}'s recipe is being frozen: it takes no more support"
        ))?;
    }
    match month.phase {
        OraclePhase::SourceSubmission => {
            let review = ORACLE_KILL_WINDOW_SECONDS
                + ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS
                + ORACLE_OPENING_WINDOW_SECONDS;
            if coverage.coverage_finalized {
                bail(format!("{id} has closed its source window"))?;
            }
            if now < coverage.planned_scramble_start_ts {
                bail(format!(
                    "{id} takes sources and support from {}; the chain's clock says {}",
                    at(coverage.planned_scramble_start_ts),
                    at(now)
                ))?;
            }
            if now.saturating_add(review) >= expiry {
                bail(format!(
                    "{id} is too close to its expiry ({}) for Scramble's review days: it takes no more support",
                    at(expiry)
                ))?;
            }
        }
        OraclePhase::Scramble => {
            let ends = month
                .scramble_start_ts
                .saturating_add(ORACLE_PLACEMENT_WINDOW_SECONDS);
            if now < month.scramble_start_ts || now >= ends {
                bail(format!(
                    "{id} takes support on Scramble's four placement days, from {} to {}; the chain's clock says {}",
                    at(month.scramble_start_ts),
                    at(ends),
                    at(now)
                ))?;
            }
        }
        phase => bail(format!(
            "{id} is in {phase:?}: sources are supported while the month takes sources and on Scramble's four placement days"
        ))?,
    }
    Ok(())
}

/// The program's rules on a support stake (tag 183), past the calendar: a
/// candidate of someone else's, not supported by this wallet yet, at the
/// product's exact support bond.
pub(super) fn check_support(
    source: &SourceRef,
    supporter: &Pubkey,
    supported: bool,
    stake: Option<u64>,
    bond: u64,
) -> Result<u64> {
    if source.state.status != OracleSourceStatus::Candidate {
        bail(format!(
            "source {} is {:?}: only a candidate source takes support",
            source.address, source.state.status
        ))?;
    }
    if source.state.proposer == *supporter {
        bail(format!(
            "you proposed source {}: its proposer can't support it",
            source.address
        ))?;
    }
    if supported {
        bail(format!(
            "you support source {} already: one support stake a wallet",
            source.address
        ))?;
    }
    match stake {
        Some(given) if given != bond => {
            let exact = trim_decimal(u128::from(bond), 6);
            bail(format!(
                "the support stake of this product is exactly {exact} USDC: pass --stake {exact}, or leave --stake out"
            ))
        }
        _ => Ok(bond),
    }
}

/// `petri oracle sources support`: tag 183, held to the program's rules
/// first; the stake is the product's exact support bond when `--stake` is
/// left out.
pub fn source_support(
    ctx: &Ctx,
    p: Paint,
    target: &Target,
    source: &str,
    stake: Option<&str>,
) -> Result<Report> {
    let oracle = Oracle::new(ctx)?;
    let supporter = ctx.wallet_pubkey()?;
    let month = oracle.month(&target.market, target.expiry.as_deref())?;
    let id = month.period.expiry_id();
    let source = oracle.source(&month, source)?;
    let now = u64::try_from(ctx.rpc.clock()?.unix_timestamp).unwrap_or(0);
    check_placement(
        &month.state,
        &oracle.coverage(&month)?,
        month.period.period.expiry_ts,
        now,
        &id,
    )?;
    match oracle.reward_schedule(&month)? {
        Some(s) if s.phase == OracleUsdcRewardSchedulePhase::Funded => {}
        Some(s) => bail(format!(
            "{id}'s rewards are {:?}, not funded: it takes no support",
            s.phase
        ))?,
        None => bail(format!(
            "{id} has no reward schedule, so it takes no support"
        ))?,
    }
    let pool = oracle.sku_pool(&month, &source.state.bucket_id)?;
    let position = ix::find_oracle_support_address(
        &oracle.program,
        &month.address,
        &source.address,
        &supporter,
    );
    let stake = check_support(
        &source,
        &supporter,
        ctx.rpc.account(&position)?.is_some(),
        stake.map(|s| usdc_amount(s, "the stake")).transpose()?,
        pool.support_bond,
    )?;
    let available = oracle
        .collateral(&supporter)?
        .map_or(0, |l| l.available_balance);
    if available < stake {
        bail(format!(
            "the support stake is {} USDC and your oracle collateral has {} USDC available: deposit first (`petri oracle collateral deposit <USDC>`)",
            usdc(stake),
            usdc(available)
        ))?;
    }
    let (sku_index, sku_proof) = oracle.sku_proof(&month, &source.state.bucket_id)?;
    let write = Write {
        action: "oracle.sources.support",
        title: format!(
            "Support source {} with {} USDC",
            source.address,
            usdc(stake)
        ),
        review: vec![
            month_line(&month),
            format!("Bucket {}", source.bucket_name()),
            format!(
                "Stake {} USDC from your oracle collateral, the product's support bond: back once the source opens or is set aside, whatever becomes of it",
                usdc(stake)
            ),
        ],
        details: json!({
            "month": month_json(&month),
            "source": source_json(&source),
            "bucketName": source.bucket_name(),
            "stakeAtoms": stake.to_string(),
            "position": position.to_string(),
        }),
        txs: vec![Tx::one(
            "supportSource",
            ix::support_oracle_source_v3(
                &oracle.program,
                &supporter,
                &month.period.address,
                &month.address,
                &source.address,
                &source.state.bucket_id,
                SupportOracleSourceV3Params {
                    stake,
                    sku_index,
                    sku_proof,
                },
            ),
        )],
    };
    send::run(ctx, p, write, target.send.simulate)
}

// -------------------------------------------------------------------- prints

/// The opening claim of a source, which must have one (`stakes settle`).
pub(super) fn opening_claim(
    oracle: &Oracle,
    month: &MonthRef,
    source: &SourceRef,
) -> Result<(Pubkey, OracleOpeningClaim)> {
    let claim = oracle.opening_claim(source)?.ok_or_else(|| {
        Error::msg(format!(
            "source {} of {} has no opening print",
            source.address,
            month.period.expiry_id()
        ))
    })?;
    Ok((claim.address, claim.state))
}

/// `petri oracle sources close`: finalizes the month's SKU coverage (184,
/// anyone may send it). It ends the source window: from then the month runs
/// Scramble's challenge days, its freeze, Opening and Game on the calendar.
pub fn sources_close(ctx: &Ctx, p: Paint, target: &Target) -> Result<Report> {
    let oracle = Oracle::new(ctx)?;
    let cranker = ctx.wallet_pubkey()?;
    let month = oracle.month(&target.market, target.expiry.as_deref())?;
    let coverage = oracle.coverage(&month)?;
    if coverage.coverage_finalized {
        bail(format!(
            "{} has closed its source window already",
            month.period.expiry_id()
        ))?;
    }
    if coverage.covered_sku_count < coverage.required_sku_count {
        bail(format!(
            "{} of {} products of {} have a supported source: each needs one before its source window can close",
            coverage.covered_sku_count,
            coverage.required_sku_count,
            month.period.expiry_id()
        ))?;
    }
    let write = Write {
        action: "oracle.sources.close",
        title: format!(
            "Close the source window of {}: Scramble's challenge days start",
            month.period.expiry_id()
        ),
        review: vec![
            month_line(&month),
            format!(
                "All {} products have a supported source",
                coverage.required_sku_count
            ),
        ],
        details: json!({"month": month_json(&month), "products": coverage.required_sku_count}),
        txs: vec![Tx::one(
            "finalizeSkuCoverage",
            ix::finalize_oracle_sku_coverage(
                &oracle.program,
                &cranker,
                &month.period.address,
                &month.address,
            ),
        )],
    };
    send::run(ctx, p, write, target.send.simulate)
}

pub fn print_finalize_phase(ctx: &Ctx, p: Paint, target: &Target) -> Result<Report> {
    let oracle = Oracle::new(ctx)?;
    let cranker = ctx.wallet_pubkey()?;
    let month = oracle.month(&target.market, target.expiry.as_deref())?;
    let write = Write {
        action: "oracle.prints.finalize-phase",
        title: format!(
            "Close the opening phase of {}: the month enters Game and the period goes Live",
            month.period.expiry_id()
        ),
        review: vec![month_line(&month)],
        details: json!({"month": month_json(&month)}),
        txs: vec![Tx::one(
            "finalizeOpeningPhase",
            ix::finalize_oracle_opening_phase(
                &oracle.program,
                &cranker,
                &month.period.address,
                &month.address,
            ),
        )],
    };
    send::run(ctx, p, write, target.send.simulate)
}

// ------------------------------------------------------------------- updates

pub(super) fn update_claim(oracle: &Oracle, claim: &str) -> Result<(Pubkey, OracleUpdateClaimV2)> {
    let key = oracle.ctx.parse_pubkey(claim, "update claim")?;
    let account = oracle.ctx.rpc.account(&key)?;
    let state: OracleUpdateClaimV2 = decode(
        &oracle.program,
        &key,
        account.as_ref(),
        OracleUpdateClaimV2::LEN,
        "update claim",
    )?;
    let c = &state.claim;
    if !c.is_initialized
        || ix::find_oracle_update_claim_address(
            &oracle.program,
            &c.month,
            &c.source,
            &c.claimant,
            &c.claim_id,
        ) != key
    {
        bail(format!("{key} is not a canonical update claim"))?;
    }
    Ok((key, state))
}

/// The month of a claim, checked to be the target's.
pub(super) fn claim_month(
    oracle: &Oracle,
    target: &Target,
    month_key: &Pubkey,
) -> Result<MonthRef> {
    let month = oracle.month(&target.market, target.expiry.as_deref())?;
    if month.address != *month_key {
        bail(format!(
            "the claim belongs to month {month_key}, not {}",
            month.period.expiry_id()
        ))?;
    }
    Ok(month)
}

// Commits, reveals, challenges and expiry of updates: `updates.rs`.

// -------------------------------------------------------------------- stakes

fn challenger_of<T: borsh::BorshDeserialize>(
    oracle: &Oracle,
    key: &Pubkey,
    len: usize,
    what: &str,
    get: impl Fn(&T) -> Pubkey,
) -> Result<Pubkey> {
    let account = oracle.ctx.rpc.account(key)?;
    let state: T = decode(&oracle.program, key, account.as_ref(), len, what)?;
    Ok(get(&state))
}

/// `petri oracle stakes settle MONTH --kind … --source …`: tag 169 on the
/// one escrow named, its parties read from chain ([`super::stakes::Settle`]
/// builds it, as `stakes settle --all` does).
#[allow(clippy::too_many_arguments)]
pub fn stake_settle(
    ctx: &Ctx,
    p: Paint,
    target: &Target,
    kind: EscrowKind,
    source: &str,
    owner: Option<&str>,
    claim: Option<&str>,
    challenge: Option<&str>,
) -> Result<Report> {
    use super::stakes::Settle;
    let oracle = Oracle::new(ctx)?;
    let cranker = ctx.wallet_pubkey()?;
    let month = oracle.month(&target.market, target.expiry.as_deref())?;
    let source = oracle.source(&month, source)?;
    let program = oracle.program;
    let settle = match kind {
        EscrowKind::ListingBond => Settle::Prelisting(PrelistingEscrow::ListingBond {
            source: source.address,
            proposer: ctx.owner(owner)?,
        }),
        EscrowKind::SupportStake => Settle::Prelisting(PrelistingEscrow::SupportStake {
            source: source.address,
            supporter: ctx.owner(owner)?,
        }),
        EscrowKind::SourceChallenge => {
            let challenge = ctx.parse_pubkey(need(challenge, "--challenge")?, "challenge")?;
            let account = ctx.rpc.account(&challenge)?;
            let c: OracleSourceChallenge = decode(
                &program,
                &challenge,
                account.as_ref(),
                OracleSourceChallenge::LEN,
                "source challenge",
            )?;
            Settle::Prelisting(PrelistingEscrow::SourceChallenge {
                source: source.address,
                proposer: source.state.proposer,
                challenge,
                challenger: c.challenger,
                comparison_source: (c.comparison_source != Pubkey::default())
                    .then_some(c.comparison_source),
            })
        }
        EscrowKind::OpeningClaim => {
            let (claim, state) = opening_claim(&oracle, &month, &source)?;
            Settle::OpeningClaim {
                claim,
                claimant: state.claimant,
            }
        }
        EscrowKind::OpeningChallenge => {
            let (claim, state) = opening_claim(&oracle, &month, &source)?;
            let challenge = ctx.parse_pubkey(need(challenge, "--challenge")?, "challenge")?;
            Settle::OpeningChallenge {
                claim,
                challenge,
                claimant: state.claimant,
                challenger: challenger_of::<OracleOpeningClaimChallenge>(
                    &oracle,
                    &challenge,
                    OracleOpeningClaimChallenge::LEN,
                    "opening challenge",
                    |c| c.challenger,
                )?,
            }
        }
        EscrowKind::UpdateClaim => {
            let (claim, state) = update_claim(&oracle, need(claim, "--claim")?)?;
            Settle::UpdateClaim {
                claim,
                claimant: state.claim.claimant,
            }
        }
        EscrowKind::UpdateChallenge => {
            let (claim, state) = update_claim(&oracle, need(claim, "--claim")?)?;
            let challenge = ctx.parse_pubkey(need(challenge, "--challenge")?, "challenge")?;
            Settle::UpdateChallenge {
                claim,
                challenge,
                claimant: state.claim.claimant,
                challenger: challenger_of::<OracleUpdateChallenge>(
                    &oracle,
                    &challenge,
                    OracleUpdateChallenge::LEN,
                    "update challenge",
                    |c| c.challenger,
                )?,
            }
        }
    };
    let mut tx = Tx::new();
    for (name, ix) in settle.steps(&program, &cranker, &month, &source.address) {
        tx.push(&name, ix);
    }
    let write = Write {
        action: "oracle.stakes.settle",
        title: format!("Settle the {kind:?} escrow of source {}", source.address),
        review: vec![
            month_line(&month),
            "Settles into the owners' oracle collateral (permissionless)".to_string(),
        ],
        details: json!({"month": month_json(&month), "source": source_json(&source), "kind": format!("{kind:?}")}),
        txs: vec![tx],
    };
    send::run(ctx, p, write, target.send.simulate)
}

fn need<'a>(value: Option<&'a str>, flag: &str) -> Result<&'a str> {
    value.ok_or_else(|| Error::msg(format!("this kind needs {flag}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_urls_name_their_capture_time() {
        assert_eq!(
            archive_time("https://web.archive.org/web/20261013120000/https://example.com/p")
                .unwrap(),
            1_791_892_800
        );
        assert!(archive_time("https://example.com").is_err());
        assert!(archive_time("https://web.archive.org/web/2026101312/https://x.com").is_err());
        assert!(archive_time("https://web.archive.org/web/20261313120000/https://x.com").is_err());
    }

    #[test]
    fn evidence_commitments_match_the_program_and_the_candidate_store() {
        let (kind, bytes, hash) =
            evidence_preimage(EvidenceKind::Locator, "https://example.com/ram").unwrap();
        assert_eq!(kind, 1);
        assert_eq!(bytes, b"https://example.com/ram");
        assert_eq!(
            hash,
            hashv(&[b"locator", b"https://example.com/ram"]).to_bytes()
        );
        let (kind, bytes, hash) =
            evidence_preimage(EvidenceKind::Definition, "Product: x").unwrap();
        assert_eq!(kind, 2);
        assert_eq!(hash, hashv(&[&bytes]).to_bytes());
        assert_eq!(hash, semantic_hash(SOURCE_DEFINITION_DOMAIN, "Product: x"));
        let url = "https://web.archive.org/web/20261013120000/https://example.com/ram";
        let (kind, _, hash) = evidence_preimage(EvidenceKind::Archive, url).unwrap();
        assert_eq!((kind, hash), (3, ix::archive_url_hash(url)));
        assert!(evidence_preimage(EvidenceKind::Locator, "ftp://x").is_err());
    }

    #[test]
    fn support_is_taken_while_the_month_places_sources() {
        let day = 86_400;
        let start = 1_792_281_600;
        let month = |phase| OracleMonthState {
            phase,
            scramble_start_ts: start,
            ..Default::default()
        };
        let coverage = OracleSkuCoverageManifest {
            planned_scramble_start_ts: start - 2 * day,
            ..Default::default()
        };
        let expiry = start + 40 * day;
        let fails = |m: &OracleMonthState, c: &OracleSkuCoverageManifest, now, expiry| {
            check_placement(m, c, expiry, now, "RAMX-202612")
                .unwrap_err()
                .message
        };
        let taking = month(OraclePhase::SourceSubmission);
        assert!(check_placement(&taking, &coverage, expiry, start - day, "RAMX-202612").is_ok());
        assert!(
            fails(&taking, &coverage, start - 3 * day, expiry)
                .contains("takes sources and support from")
        );
        assert!(
            fails(&taking, &coverage, start, start + 3 * day).contains("too close to its expiry")
        );
        let closed = OracleSkuCoverageManifest {
            coverage_finalized: true,
            ..coverage.clone()
        };
        assert!(fails(&taking, &closed, start, expiry).contains("closed its source window"));
        // Scramble's four placement days.
        let scramble = month(OraclePhase::Scramble);
        assert!(
            check_placement(
                &scramble,
                &closed,
                expiry,
                start + 4 * day - 1,
                "RAMX-202612"
            )
            .is_ok()
        );
        assert!(fails(&scramble, &closed, start + 4 * day, expiry).contains("four placement days"));
        assert!(
            fails(&month(OraclePhase::Opening), &closed, start, expiry).contains("is in Opening")
        );
        let freezing = OracleMonthState {
            weight_scheme_version: 255,
            ..scramble
        };
        assert!(fails(&freezing, &closed, start, expiry).contains("being frozen"));
    }

    #[test]
    fn a_support_stake_is_the_products_bond_on_someone_elses_candidate() {
        let supporter = Pubkey::new_unique();
        let source = SourceRef {
            address: Pubkey::new_unique(),
            state: ameba_oracle::state::OracleSourceState {
                status: OracleSourceStatus::Candidate,
                proposer: Pubkey::new_unique(),
                ..Default::default()
            },
        };
        assert_eq!(
            check_support(&source, &supporter, false, None, 1_000_000).unwrap(),
            1_000_000
        );
        assert_eq!(
            check_support(&source, &supporter, false, Some(1_000_000), 1_000_000).unwrap(),
            1_000_000
        );
        let fails = |s: &SourceRef, who: &Pubkey, supported, stake| {
            check_support(s, who, supported, stake, 1_000_000)
                .unwrap_err()
                .message
        };
        assert!(
            fails(&source, &supporter, false, Some(2_000_000))
                .contains("exactly 1 USDC: pass --stake 1, or leave --stake out")
        );
        assert!(fails(&source, &supporter, true, None).contains("already"));
        assert!(
            fails(&source, &source.state.proposer, false, None)
                .contains("its proposer can't support it")
        );
        let mut frozen = source.clone();
        frozen.state.status = OracleSourceStatus::Frozen;
        assert!(fails(&frozen, &supporter, false, None).contains("only a candidate"));
    }

    #[test]
    fn values_and_amounts_parse_in_atoms() {
        assert_eq!(value_atoms("75.5").unwrap(), 75_500_000);
        assert!(value_atoms("0").is_err());
        assert_eq!(usdc_amount("2.5", "x").unwrap(), 2_500_000);
        assert!(usdc_amount("0", "x").is_err());
    }
}
