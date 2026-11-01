//! The heads writes of the stake and the board: stake, unstake, post, delegate, undelegate,
//! sync, the delegator rewards and a claim's prize. Each checks what the program will check against the accounts as it loads them (the
//! exact minimum, what is free, the head's state) and refuses in plain words before anything is
//! signed; each transaction asks for the heap frame the heads instructions need.

use ameba_commodity_spec::Product;
use ameba_oracle::{
    associated_token::{
        create_associated_token_account_idempotent, get_associated_token_address_with_program_id,
    },
    heads_view as hv,
    instruction::{self as ix, heads as hx},
    state::heads::{
        Delegation, HeadAccessClass, HeadClaimStatus, HeadStatus, HeadsBoard, HeadsConfig,
    },
};
use serde_json::{Value, json};
use solana_program::{hash::hashv, instruction::Instruction, pubkey::Pubkey};

use super::{
    accounts::{self, BoardRef, Chain},
    amba,
    args::{Access, PostArgs},
    at, atoms, commands,
    docs::{self, Mine},
    flatten, landed, parse_amba, parse_amba_or_all, set,
};
use crate::{
    futures::{
        Report,
        ctx::Ctx,
        error::{Error, Result, bail},
        units::usdc,
        view::Paint,
    },
    oracle::{
        send::{self, Tx, Write},
        writes::evidence_txs,
    },
};

/// The most bytes a sealed object holds (the spec's `"<name>. <spec>"`, an offline quote).
pub const OBJECT_MAX: usize = 4096;

/// A heads transaction of one instruction, with the heap frame.
pub fn tx(name: &str, ix: Instruction) -> Tx {
    Tx::one(name, ix).with_heap()
}

/// Sends a heads write and lifts its result for the plugin.
pub fn send(ctx: &Ctx, p: Paint, write: Write, simulate: bool) -> Result<Report> {
    let mut report = flatten(send::run(ctx, p, write, simulate).map_err(plain_error)?);
    // A simulation's refusals, in the same words.
    if let Some(txs) = report.payload["transactions"].as_array_mut() {
        for t in txs {
            if let Some(e) = t["error"].as_str() {
                t["error"] = json!(plain(e));
            }
        }
    }
    Ok(report)
}

/// A heads program error in plain words: the first line of `message` (`delegate failed:
/// InsufficientHeadsStake: … (oracle error 6316)`) becomes its sentence; what follows (what
/// landed, the signature) stays.
pub fn plain(message: &str) -> String {
    let (first, rest) = message
        .split_once('\n')
        .map_or((message, None), |(a, b)| (a, Some(b)));
    let code = first
        .rsplit_once("(oracle error ")
        .and_then(|(_, tail)| tail.trim_end_matches(')').parse::<u32>().ok());
    match code.and_then(crate::oracle::errors::heads_words) {
        Some(words) => match rest {
            Some(rest) => format!("{words}\n{rest}"),
            None => words.to_string(),
        },
        None => message.to_string(),
    }
}

/// [`plain`] on an error, keeping its logs.
pub fn plain_error(mut e: Error) -> Error {
    e.message = plain(&e.message);
    e
}

/// The head's words as a review line: `RAMX #3 "Mouser DDR5" (DDR5 16Gb Spot, live)`.
pub fn head_line(board: &BoardRef, h: &ameba_oracle::state::heads::Head) -> String {
    format!(
        "{} #{} \"{}\" ({}, {})",
        board.symbol(),
        h.head_id,
        h.name(),
        docs::component_label(Some(board.product), &board.board, h.component),
        docs::status_word(h.status)
    )
}

fn base(board: &BoardRef, h: &ameba_oracle::state::heads::Head) -> Value {
    json!({ "product": board.symbol(), "head": h.head_id, "name": h.name() })
}

fn merge(a: &mut Value, b: Value) {
    if let (Some(a), Value::Object(b)) = (a.as_object_mut(), b) {
        a.extend(b);
    }
}

/// An AMBA token account's balance: owned by `token_program`, of `mint`, its amount.
fn token_balance(
    ctx: &Ctx,
    key: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
) -> Result<Option<u64>> {
    let Some(account) = ctx.rpc.account(key)? else {
        return Ok(None);
    };
    let d = &account.data;
    if account.owner != *token_program || d.len() < 72 || d[..32] != mint.to_bytes() {
        bail(format!("{key} is not your AMBA token account"))?;
    }
    Ok(Some(u64::from_le_bytes(d[64..72].try_into().unwrap())))
}

// ------------------------------------------------------------------------------- the stake

/// `petri oracle heads stake <amba>`: tag 210/4, AMBA from the wallet into the oracle.
pub fn stake(ctx: &Ctx, p: Paint, amount: &str, simulate: bool) -> Result<Report> {
    let chain = Chain::new(ctx)?;
    let program = chain.program();
    let config: Box<HeadsConfig> = chain.config()?;
    let owner = ctx.wallet_pubkey()?;
    let atoms_in = parse_amba(amount)?;
    let token = get_associated_token_address_with_program_id(
        &owner,
        &config.amba_mint,
        &config.amba_token_program,
    );
    let held =
        token_balance(ctx, &token, &config.amba_mint, &config.amba_token_program)?.unwrap_or(0);
    if held < atoms_in {
        bail(format!("Your wallet holds {}.", amba(held)))?;
    }
    let before = chain.stake(&owner)?;
    let free_before = before.as_ref().map_or(0, |s| s.free);
    let write = Write {
        action: "heads.stake",
        title: format!("Stake {} in the oracle", amba(atoms_in)),
        review: vec![
            format!(
                "From {token} (your AMBA, {} held) to the oracle's AMBA vault",
                amba(held)
            ),
            format!(
                "Free stake now {}; staked AMBA backs heads, and only free stake unstakes",
                amba(free_before)
            ),
            if before.is_some() {
                "Stake account exists".to_string()
            } else {
                "Opens your oracle stake account first (rent paid by you)".to_string()
            },
        ],
        details: json!({ "amount": atoms(atoms_in), "owner": owner.to_string() }),
        txs: vec![tx(
            "stake",
            hx::stake(
                &program,
                &owner,
                &owner,
                &token,
                &config.amba_mint,
                &config.amba_token_program,
                atoms_in,
            ),
        )],
    };
    let mut report = send(ctx, p, write, simulate)?;
    if landed(&report) {
        let free = chain.free(&owner)?;
        set(
            &mut report,
            json!({ "free": atoms(free), "credited": atoms(free.saturating_sub(free_before)) }),
        );
        report
            .plain
            .push_str(&format!("\nFree stake: {}.", amba(free)));
    }
    Ok(report)
}

/// `petri oracle heads unstake <amba|all>`: tag 210/5, free AMBA back to the wallet.
pub fn unstake(ctx: &Ctx, p: Paint, amount: &str, simulate: bool) -> Result<Report> {
    let chain = Chain::new(ctx)?;
    let program = chain.program();
    let config = chain.config()?;
    let owner = ctx.wallet_pubkey()?;
    let free = chain.free(&owner)?;
    if free == 0 {
        bail("You have no free AMBA staked in the oracle.")?;
    }
    let atoms_out = parse_amba_or_all(amount, free)?;
    if atoms_out > free {
        bail(format!("You can unstake up to {}.", amba(free)))?;
    }
    let token = get_associated_token_address_with_program_id(
        &owner,
        &config.amba_mint,
        &config.amba_token_program,
    );
    let mut t = Tx::new().with_heap();
    if ctx.rpc.account(&token)?.is_none() {
        t.push(
            "createAmbaAta",
            create_associated_token_account_idempotent(
                &owner,
                &owner,
                &config.amba_mint,
                &config.amba_token_program,
            ),
        );
    }
    t.push(
        "unstake",
        hx::unstake(
            &program,
            &owner,
            &token,
            &config.amba_mint,
            &config.amba_token_program,
            atoms_out,
        ),
    );
    let write = Write {
        action: "heads.unstake",
        title: format!("Unstake {} from the oracle", amba(atoms_out)),
        review: vec![
            format!("To {token} (your AMBA account)"),
            format!("Free stake {} → {}", amba(free), amba(free - atoms_out)),
        ],
        details: json!({ "amount": atoms(atoms_out), "owner": owner.to_string() }),
        txs: vec![t],
    };
    let mut report = send(ctx, p, write, simulate)?;
    if landed(&report) {
        set(&mut report, json!({ "free": atoms(chain.free(&owner)?) }));
    }
    Ok(report)
}

// ------------------------------------------------------------------------------------ post

fn access_class(a: Access) -> HeadAccessClass {
    match a {
        Access::PublicPage => HeadAccessClass::PublicPage,
        Access::LoginZktls => HeadAccessClass::LoginZkTls,
        Access::OfflineQuote => HeadAccessClass::OfflineQuote,
        Access::Physical => HeadAccessClass::Physical,
    }
}

fn squash(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// A component of `board` by its index or its label ("DDR5 16Gb Contract", any case and
/// spacing; "DDR5_16GBIT contract" too). An item's bucket alone has three components: refused
/// with their names.
pub fn component(board: &BoardRef, text: &str) -> Result<u8> {
    let count = board.board.component_count;
    let labels: Vec<String> = (0..count)
        .map(|i| docs::component_label(Some(board.product), &board.board, i))
        .collect();
    let listing = || {
        labels
            .iter()
            .enumerate()
            .map(|(i, l)| format!("{i} {l}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let t = text.trim();
    if let Ok(i) = t.parse::<u8>() {
        if i < count {
            return Ok(i);
        }
        bail(format!(
            "{} has no component {i}: its components are {}.",
            board.symbol(),
            listing()
        ))?;
    }
    let wanted = squash(t);
    if let Some(i) = labels.iter().position(|l| squash(l) == wanted) {
        return Ok(i as u8);
    }
    // A bucket label with the channel: "DDR5_16GBIT Contract".
    for i in 0..count {
        let c = usize::from(i);
        let item = usize::from(board.board.component_items[c]).min(7);
        let channel = hv::channel_of(board.board.component_channels[c]);
        let bucket = hv::product_symbol(&board.board.item_bucket_ids[item]).unwrap_or_default();
        if squash(&format!("{bucket}{}", docs::channel_name(channel))) == wanted {
            return Ok(i);
        }
    }
    let items: Vec<String> = (0..count)
        .filter(|i| {
            let item = usize::from(board.board.component_items[usize::from(*i)]).min(7);
            let id = &board.board.item_bucket_ids[item];
            squash(&hv::product_symbol(id).unwrap_or_default()) == wanted
                || squash(&docs::item_name(Some(board.product), id)) == wanted
        })
        .map(|i| format!("{i} {}", labels[usize::from(i)]))
        .collect();
    if !items.is_empty() {
        bail(format!(
            "{t} is an item with three components, one per channel: {}. Name one.",
            items.join(", ")
        ))?;
    }
    bail(format!(
        "{} has no component \"{t}\": its components are {}.",
        board.symbol(),
        listing()
    ))
}

/// The mold's target sentence of `board`'s component (the default spec).
fn mold_sentence(product: Product, board: &HeadsBoard, index: u8) -> Option<String> {
    let c = usize::from(index);
    let item = usize::from(board.component_items[c]).min(7);
    let label = hv::product_symbol(&board.item_bucket_ids[item])?;
    let channel = docs::mold_channel(hv::channel_of(board.component_channels[c]));
    Some(
        product
            .mold()
            .item(&label)?
            .component(channel)
            .sentence
            .clone(),
    )
}

/// `petri oracle heads post …`: the spec sealed as a kind-2 object (`"<name>. <spec>"`, as the
/// site seals it), then tag 210/6 with the first delegation.
pub fn post(ctx: &Ctx, p: Paint, a: &PostArgs) -> Result<Report> {
    let chain = Chain::new(ctx)?;
    let program = chain.program();
    chain.config()?;
    let owner = ctx.wallet_pubkey()?;
    let board = chain.board(accounts::product(&a.product)?)?;
    let index = component(&board, &a.component)?;
    let label = docs::component_label(Some(board.product), &board.board, index);
    if board.board.component_sentence_hashes[usize::from(index)] == [0; 32] {
        bail(format!(
            "{label} has no target sentence on chain yet, so it takes no heads."
        ))?;
    }
    let name = a.name.trim();
    if name.is_empty() {
        bail("Name the source.")?;
    }
    if name.len() > ameba_oracle::state::heads::HEAD_NAME_MAX_BYTES {
        bail("That name is too long (at most 64 bytes). Shorten it.")?;
    }
    let spec = match a.spec.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => s.to_string(),
        None => mold_sentence(board.product, &board.board, index).ok_or_else(|| {
            Error::msg(format!(
                "{label} has no sentence in the molds: say what the source reports with --spec."
            ))
        })?,
    };
    let document = format!("{name}. {spec}");
    if document.len() > OBJECT_MAX {
        bail("That spec is too long. Shorten it.")?;
    }
    let min_post = board.min_post();
    let amount = match &a.amount {
        Some(text) => parse_amba(text)?,
        None => min_post,
    };
    if amount < min_post {
        bail(format!("Posting takes at least {}.", amba(min_post)))?;
    }
    let free = chain.free(&owner)?;
    if free < amount {
        bail(format!(
            "You have {} staked and free. Stake more AMBA to post with {}.",
            amba(free),
            amba(amount)
        ))?;
    }
    let hash = hashv(&[document.as_bytes()]).to_bytes();
    let (object, mut txs, state) =
        evidence_txs(&chain.oracle, &owner, 2, document.as_bytes(), hash)?;
    let head_id = board.board.head_count;
    let access = access_class(a.access);
    txs.push(tx(
        "postHead",
        hx::post_head(
            &program, &owner, &owner, &board.id, head_id, index, access, amount, name, hash,
            &object,
        )
        .map_err(|e| Error::msg(format!("cannot build the post: {e}")))?,
    ));
    let p_ = board.params();
    let write = Write {
        action: "heads.post",
        title: format!("Post \"{name}\" on {} {label} with {}", board.symbol(), amba(amount)),
        review: vec![
            format!("Spec ({} bytes, sealed as your public object {object}, {state}): {}", document.len(), crate::backend::terminal_safe_text(&spec)),
            format!("Delegates {} of your {} free at once (the minimum to post is {})", amba(amount), amba(free), amba(min_post)),
            format!(
                "It earns nothing until it is live; it turns active at {} with {} delegators, and expires after {} days if it doesn't",
                amba(board.activation()),
                p_.min_delegators,
                p_.head_ttl_secs / 86_400
            ),
            "You vouch that the source exists: a lost source challenge takes all of your delegation on it".to_string(),
        ],
        details: json!({
            "product": board.symbol(), "head": head_id, "name": name, "amount": atoms(amount),
            "component": index, "componentLabel": label, "accessClass": docs::access_word(access),
            "spec": spec, "specHash": crate::oracle::skus::hex(&hash), "specObject": object.to_string(),
        }),
        txs,
    };
    let mut report = send(ctx, p, write, a.send.simulate)?;
    if landed(&report) {
        let (_, h) = chain.head(&board, head_id)?;
        set(
            &mut report,
            json!({ "status": docs::status_word(h.status) }),
        );
        report.plain.push_str(&format!(
            "\nHead {head_id} is {}.",
            docs::status_word(h.status)
        ));
    }
    Ok(report)
}

// ----------------------------------------------------------------------------- delegation

/// `petri oracle heads delegate <head> <amba> --product P`: tag 210/7.
pub fn delegate(
    ctx: &Ctx,
    p: Paint,
    product: &str,
    head: u32,
    amount: &str,
    simulate: bool,
) -> Result<Report> {
    let chain = Chain::new(ctx)?;
    let program = chain.program();
    chain.config()?;
    let owner = ctx.wallet_pubkey()?;
    let board = chain.board(accounts::product(product)?)?;
    let (_, h) = chain.head(&board, head)?;
    let now = chain.now()?.unix;
    match h.status {
        HeadStatus::Open if now >= h.expires_ts => bail(format!(
            "Head {head} did not turn active by {} and takes no more delegation.",
            at(h.expires_ts)
        ))?,
        HeadStatus::Retired => bail(format!(
            "Head {head} is retired after its strikes and takes no delegation."
        ))?,
        HeadStatus::Expired => bail(format!("Head {head} expired and takes no delegation."))?,
        _ => {}
    }
    let atoms_in = parse_amba(amount)?;
    let free = chain.free(&owner)?;
    if free < atoms_in {
        bail(format!(
            "You have {} staked and free. Stake more AMBA to delegate {}.",
            amba(free),
            amba(atoms_in)
        ))?;
    }
    let a = board.head_addresses(&program, head);
    let existing = chain.delegation(&a, &owner)?;
    let params = board.params();
    let mut review = vec![
        format!("{} delegated by {} now", amba(h.delegation), h.delegators),
        format!("From your free stake ({})", amba(free)),
        format!(
            "When the source itself loses a challenge, every delegation on it is slashed {}% (burned) and it takes a strike; at {} strikes it retires",
            params.slash_rate_bps / 100,
            params.max_strikes
        ),
    ];
    if h.status == HeadStatus::Open {
        review.push(format!(
            "It turns active at {} with {} delegator{}",
            amba(board.activation()),
            params.min_delegators,
            if params.min_delegators == 1 { "" } else { "s" }
        ));
    }
    if existing.is_none() {
        review.push(
            "Opens your delegation on it (rent paid by you, returned when it closes)".to_string(),
        );
    }
    let mut details = base(&board, &h);
    merge(&mut details, json!({ "amount": atoms(atoms_in) }));
    let write = Write {
        action: "heads.delegate",
        title: format!("Delegate {} to {}", amba(atoms_in), head_line(&board, &h)),
        review,
        details,
        txs: vec![tx(
            "delegate",
            hx::delegate(&program, &owner, &owner, &board.id, head, atoms_in),
        )],
    };
    let mut report = send(ctx, p, write, simulate)?;
    if landed(&report) {
        let (_, h) = chain.head(&board, head)?;
        set(
            &mut report,
            json!({ "status": docs::status_word(h.status), "delegation": atoms(h.delegation), "delegators": h.delegators }),
        );
        report.plain.push_str(&format!(
            "\nHead {head} is {} with {}.",
            docs::status_word(h.status),
            amba(h.delegation)
        ));
    }
    Ok(report)
}

fn position_json(
    board: &BoardRef,
    h: &ameba_oracle::state::heads::Head,
    d: &Delegation,
) -> (hv::Position, Value) {
    let pos = hv::position(&board.board, h, d);
    let v = json!({
        "delegated": atoms(pos.amount),
        "cooling": atoms(pos.cooling),
        "escrow": atoms(pos.escrow),
        "cooldownEndsAt": pos.cooldown_ends_at.map(|t| json!(t)).unwrap_or(Value::Null),
        "rewards": atoms(pos.rewards),
    });
    (pos, v)
}

/// `petri oracle heads undelegate <head> <amba|all> --product P`: tag 210/8.
pub fn undelegate(
    ctx: &Ctx,
    p: Paint,
    product: &str,
    head: u32,
    amount: &str,
    simulate: bool,
) -> Result<Report> {
    let chain = Chain::new(ctx)?;
    let program = chain.program();
    chain.config()?;
    let owner = ctx.wallet_pubkey()?;
    let board = chain.board(accounts::product(product)?)?;
    let (_, h) = chain.head(&board, head)?;
    let a = board.head_addresses(&program, head);
    let (_, d) = chain
        .delegation(&a, &owner)?
        .ok_or_else(|| Error::msg(format!("You have no delegation on head {head}.")))?;
    let delegated = d.active.saturating_add(d.fresh);
    if delegated == 0 {
        bail(format!("Nothing of yours is delegated on head {head} now."))?;
    }
    let atoms_out = parse_amba_or_all(amount, delegated)?;
    if atoms_out > delegated {
        bail(format!("You can undelegate up to {}.", amba(delegated)))?;
    }
    let params = board.params();
    let remaining = params.max_strikes.saturating_sub(h.strike_count);
    let held_share = if d.is_proposer {
        "all of it while a source challenge can still take it, since you posted the head"
            .to_string()
    } else {
        format!(
            "what its remaining {remaining} strike{} could take ({}% of it) stays in escrow until every contract it weighed has settled",
            if remaining == 1 { "" } else { "s" },
            ameba_oracle_heads::liability::escrow_required(
                10_000,
                params.slash_rate_bps,
                remaining
            ) / 100
        )
    };
    let mut details = base(&board, &h);
    merge(&mut details, json!({ "amount": atoms(atoms_out) }));
    let write = Write {
        action: "heads.undelegate",
        title: format!(
            "Undelegate {} from {}",
            amba(atoms_out),
            head_line(&board, &h)
        ),
        review: vec![
            format!("Delegated now {}", amba(delegated)),
            format!(
                "It cools for {} days; {held_share}",
                params.cooldown_secs / 86_400
            ),
            format!(
                "Then `{}` moves what is due back to your free stake",
                commands::sync(board.symbol(), head)
            ),
        ],
        details,
        txs: vec![tx(
            "undelegate",
            hx::undelegate(&program, &owner, &board.id, head, atoms_out),
        )],
    };
    let mut report = send(ctx, p, write, simulate)?;
    if landed(&report) {
        let (_, h) = chain.head(&board, head)?;
        if let Some((_, d)) = chain.delegation(&a, &owner)? {
            let (pos, v) = position_json(&board, &h, &d);
            set(&mut report, v);
            if let Some(t) = pos.cooldown_ends_at {
                report.plain.push_str(&format!(
                    "\nCooling until {}{}.",
                    at(t),
                    if pos.escrow > 0 {
                        format!("; {} stays in escrow", amba(pos.escrow))
                    } else {
                        String::new()
                    }
                ));
            }
        }
    }
    Ok(report)
}

/// `petri oracle heads sync <head> --product P [--owner]`: tag 210/9 (anyone may send it).
pub fn sync(
    ctx: &Ctx,
    p: Paint,
    product: &str,
    head: u32,
    owner: Option<&str>,
    simulate: bool,
) -> Result<Report> {
    let chain = Chain::new(ctx)?;
    let program = chain.program();
    chain.config()?;
    let owner = ctx.owner(owner)?;
    let board = chain.board(accounts::product(product)?)?;
    let (_, h) = chain.head(&board, head)?;
    let a = board.head_addresses(&program, head);
    let (_, d) = chain
        .delegation(&a, &owner)?
        .ok_or_else(|| Error::msg(format!("{owner} has no delegation on head {head}.")))?;
    if chain.stake(&owner)?.is_none() {
        bail(format!("{owner} has no oracle stake account"))?;
    }
    let (pos, _) = position_json(&board, &h, &d);
    let now = chain.now()?.unix;
    let free_before = chain.free(&owner)?;
    let due = if pos.cooling > 0 && pos.cooldown_ends_at.is_some_and(|t| now >= t) {
        format!("{} is due back to the free stake", amba(pos.cooling))
    } else if pos.cooling > 0 {
        format!(
            "{} is cooling until {}: nothing is due yet, but strikes and rewards come up to date",
            amba(pos.cooling),
            pos.cooldown_ends_at.map(at).unwrap_or_default()
        )
    } else {
        "Applies strikes, accrues rewards and closes the delegation once it is empty".to_string()
    };
    let mut details = base(&board, &h);
    merge(&mut details, json!({ "owner": owner.to_string() }));
    let write = Write {
        action: "heads.sync",
        title: format!(
            "Sync the delegation of {} on {}",
            crate::futures::units::short(&owner.to_string()),
            head_line(&board, &h)
        ),
        review: vec![due],
        details,
        txs: vec![tx(
            "syncDelegation",
            hx::sync_delegation(&program, &board.id, head, &owner, &d.rent_payer),
        )],
    };
    let mut report = send(ctx, p, write, simulate)?;
    if landed(&report) {
        let free = chain.free(&owner)?;
        let released = free.saturating_sub(free_before);
        set(
            &mut report,
            json!({ "released": atoms(released), "free": atoms(free) }),
        );
        if released > 0 {
            report
                .plain
                .push_str(&format!("\nReleased {} to the free stake.", amba(released)));
        }
    }
    Ok(report)
}

// --------------------------------------------------------------------------- money claimed

/// `petri oracle heads rewards --claim`: tag 210/29 for every delegation with rewards, into the
/// oracle collateral (opened first when there is none).
pub fn claim_rewards(ctx: &Ctx, p: Paint, simulate: bool) -> Result<Report> {
    let owner = ctx.wallet_pubkey()?;
    let reads = Chain::reads(ctx)?;
    let m = Mine::read(&reads, &owner)?;
    let program = reads.program();
    let mut steps: Vec<(String, Instruction)> = Vec::new();
    let mut heads = Vec::new();
    let mut total = 0u64;
    for (_, d) in &m.delegations {
        let Some(h) = m.heads.get(&d.head) else {
            continue;
        };
        let Some((symbol, board)) = m.boards.get(&h.board) else {
            continue;
        };
        let pos = hv::position(board, h, d);
        if pos.rewards == 0 {
            continue;
        }
        total = total.saturating_add(pos.rewards);
        heads.push(json!({ "product": symbol, "head": h.head_id, "name": h.name(), "rewards": atoms(pos.rewards) }));
        steps.push((
            "claimRewards".to_string(),
            hx::claim_rewards(&program, &owner, &board.product_id, h.head_id),
        ));
    }
    if steps.is_empty() {
        bail("Nothing to claim yet.")?;
    }
    let chain = Chain::new(ctx)?;
    let before = chain.oracle.collateral(&owner)?;
    if before.is_none() {
        steps.insert(
            0,
            (
                "initCollateral".to_string(),
                ix::init_user_collateral(&program, &owner),
            ),
        );
    }
    // One claim a transaction (each loads the board and the head), the collateral opened with
    // the first.
    let mut txs: Vec<Tx> = Vec::new();
    for (name, ix) in steps {
        match txs.last_mut() {
            Some(last) if last.steps.iter().all(|(n, _)| n == "initCollateral") => {
                last.push(&name, ix);
            }
            _ => txs.push(tx(&name, ix)),
        }
    }
    let write = Write {
        action: "heads.rewards",
        title: format!("Claim {} USDC of delegator rewards", usdc(total)),
        review: vec![
            format!(
                "From {} delegation{} into your oracle collateral",
                heads.len(),
                if heads.len() == 1 { "" } else { "s" }
            ),
            "petri oracle collateral withdraw takes it out".to_string(),
        ],
        details: json!({ "amount": atoms(total), "heads": heads }),
        txs,
    };
    let mut report = send(ctx, p, write, simulate)?;
    if landed(&report)
        && let Some(after) = chain.oracle.collateral(&owner)?
    {
        let paid = after
            .available_balance
            .saturating_sub(before.map_or(0, |c| c.available_balance));
        set(&mut report, json!({ "amount": atoms(paid) }));
    }
    Ok(report)
}

/// `petri oracle heads prize <head> <seq> --product P`: tag 210/30, an accepted claim's prize
/// after its calendar month ended.
pub fn prize(
    ctx: &Ctx,
    p: Paint,
    product: &str,
    head: u32,
    seq: u32,
    simulate: bool,
) -> Result<Report> {
    let chain = Chain::new(ctx)?;
    let program = chain.program();
    chain.config()?;
    let owner = ctx.wallet_pubkey()?;
    let board = chain.board(accounts::product(product)?)?;
    let (_, h) = chain.head(&board, head)?;
    let a = board.head_addresses(&program, head);
    let (_, c) = chain.claim(&a, head, seq)?;
    if c.fetcher != owner {
        bail(format!(
            "Claim {seq} on head {head} is {}'s, not yours.",
            c.fetcher
        ))?;
    }
    if c.prize_claimed {
        bail(format!(
            "You claimed claim {seq}'s prize already ({} USDC).",
            usdc(c.prize_paid)
        ))?;
    }
    if c.status != HeadClaimStatus::Stood {
        bail(format!(
            "Claim {seq} on head {head} is {}: only an accepted claim earns a prize.",
            docs::claim_status(&c)
        ))?;
    }
    let epoch = chain.epoch(&board.address, c.epoch)?.ok_or_else(|| {
        Error::msg(format!(
            "Claim {seq}'s prize month has no prize pool on chain."
        ))
    })?;
    let now = chain.now()?.unix;
    if now < epoch.end_ts {
        bail(format!(
            "Prizes of that month pay out once it ends, at {}.",
            at(epoch.end_ts)
        ))?;
    }
    let payout = ameba_oracle_heads::rewards::prize_payout(
        c.prize_points,
        epoch.prize_points,
        epoch.prize_pool,
    );
    if chain.oracle.collateral(&owner)?.is_none() {
        bail(
            "You have no oracle collateral to receive it; deposit first with petri oracle collateral deposit <USDC>.",
        )?;
    }
    let mut details = base(&board, &h);
    merge(&mut details, json!({ "seq": seq, "amount": atoms(payout) }));
    let write = Write {
        action: "heads.prize",
        title: format!(
            "Claim the {} USDC prize of claim {seq} on {}",
            usdc(payout),
            head_line(&board, &h)
        ),
        review: vec![
            format!(
                "Its month's pool: {} USDC for {} prize points (scaled down pro rata when short)",
                usdc(epoch.prize_pool),
                epoch.prize_points
            ),
            "Into your oracle collateral".to_string(),
        ],
        details,
        txs: vec![tx(
            "claimPrize",
            hx::claim_prize(&program, &owner, &board.id, head, seq, c.epoch),
        )],
    };
    let mut report = send(ctx, p, write, simulate)?;
    if landed(&report) {
        let (_, c) = chain.claim(&a, head, seq)?;
        set(&mut report, json!({ "amount": atoms(c.prize_paid) }));
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_refusals_read_as_plain_words() {
        assert_eq!(
            plain(
                "delegate failed: InsufficientHeadsStake: not enough free AMBA stake (oracle error 6316)"
            ),
            "You don't have enough free AMBA staked in the oracle for that."
        );
        assert_eq!(
            plain("commitClaim failed: InvalidHeadsState: x (oracle error 6312)\n  signature 5ig"),
            format!(
                "{}\n  signature 5ig",
                crate::oracle::errors::heads_words(6312).unwrap()
            )
        );
        assert_eq!(
            plain("the blockhash expired; try again"),
            "the blockhash expired; try again"
        );
    }
}
