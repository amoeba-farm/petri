//! The heads documents built from chain. The futures API serves the boards and heads
//! (`/v1/oracle/heads*`, `api/api/src/oracle/heads.rs`); when it can't answer (an API without
//! the routes, or before its first read) petri builds the same documents itself, field for field,
//! from the accounts (`accounts.rs`) and `ameba_oracle::heads_view`, the code the API computes
//! them with: shares under the caps, thresholds, the channels' changes, the references, the
//! index, positions. `heads mine` (`mine`) is petri's own: the API's account document with the
//! names, head statuses and the follow-up commands the plugin runs.

use std::collections::{BTreeSet, HashMap};

use ameba_commodity_spec::{Product, molds};
use ameba_oracle::{
    heads_view::{self as hv, BoardAccounts, BoardView, ComponentView, ItemView},
    state::heads::{
        Delegation, Head, HeadAccessClass, HeadChallenge, HeadChallengeKind, HeadChallengeReason,
        HeadChallengeRuling, HeadClaim, HeadClaimKind, HeadClaimStatus, HeadStatus,
        HeadsContractStatus, HeadsEpoch, HeadsParams, HeadsStake, find_heads_epoch_address,
    },
};
use ameba_oracle_heads::{
    channels::{Channel, MixBasis, Refusal, SettlementTarget, Unavailable},
    rewards, settle,
};
use serde_json::{Value, json};
use solana_program::pubkey::Pubkey;

use super::{
    accounts::{Chain, offset},
    atoms, commands, time_or_null,
};
use crate::{
    futures::error::{Error, Result},
    oracle::{heads::accounts::BoardRef, skus::hex},
};

// ------------------------------------------------------------------------------------ words

pub fn status_word(status: HeadStatus) -> &'static str {
    match status {
        HeadStatus::Open => "open",
        HeadStatus::Active => "active",
        HeadStatus::Live => "live",
        HeadStatus::Retired => "retired",
        HeadStatus::Expired => "expired",
    }
}

pub fn access_word(class: HeadAccessClass) -> &'static str {
    match class {
        HeadAccessClass::PublicPage => "public-page",
        HeadAccessClass::LoginZkTls => "login-zktls",
        HeadAccessClass::OfflineQuote => "offline-quote",
        HeadAccessClass::Physical => "physical",
    }
}

pub fn channel_word(channel: Channel) -> &'static str {
    match channel {
        Channel::Spot => "spot",
        Channel::Contract => "contract",
        Channel::Lta => "lta",
    }
}

pub fn channel_name(channel: Channel) -> &'static str {
    match channel {
        Channel::Spot => "Spot",
        Channel::Contract => "Contract",
        Channel::Lta => "LTA",
    }
}

fn basis_word(basis: MixBasis) -> &'static str {
    match basis {
        MixBasis::None => "none",
        MixBasis::Assumed => "assumed",
        MixBasis::Matched => "matched",
    }
}

fn target_word(target: SettlementTarget) -> &'static str {
    match target {
        SettlementTarget::ContractChannel => "contract-channel",
        SettlementTarget::FixedMixReference => "fixed-mix",
    }
}

/// What a board's next listing settles on: its front contract's target, else its product's
/// mold's (`settlementTarget` of the listing terms document).
fn board_target(board: &Board) -> SettlementTarget {
    if board.view.front.is_some() {
        return board.view.settlement_target;
    }
    board
        .product
        .and_then(|p| SettlementTarget::from_code(p.mold().settlement_target.code()))
        .unwrap_or(board.view.settlement_target)
}

/// Whether a contract on `target` settles on `c`: Stage 1 on Contract components; the fixed
/// mix on every channel its item's mix weighs (a share above zero).
fn settles(target: SettlementTarget, item: Option<&ItemView>, c: &ComponentView) -> bool {
    match target {
        SettlementTarget::ContractChannel => c.channel == Channel::Contract,
        SettlementTarget::FixedMixReference => {
            item.is_some_and(|i| i.mix_bps[c.channel.index()] > 0)
        }
    }
}

/// The sentence a listing discloses: read from the listing terms document of the contract
/// expiring at `expiry`, or, for a board, the one its next listing would carry.
pub fn disclosure(product: Option<Product>, expiry: Option<u64>) -> Option<String> {
    let product = product?;
    let Some(expiry) = expiry.and_then(|e| i64::try_from(e).ok()) else {
        return Some(product.mold().disclosure.clone());
    };
    let document = molds::listing_terms_document(product, expiry);
    let json = document.get(molds::LISTING_TERMS_DOMAIN.len()..)?;
    serde_json::from_slice::<Value>(json).ok()?["disclosure"]
        .as_str()
        .map(str::to_owned)
}

fn contract_status_word(status: HeadsContractStatus) -> &'static str {
    match status {
        HeadsContractStatus::Accumulating => "live",
        HeadsContractStatus::Collecting => "frozen",
        HeadsContractStatus::Settled => "settled",
    }
}

fn degraded_word(reason: settle::Degraded) -> &'static str {
    match reason {
        settle::Degraded::FreezeCheckFailed => "freeze-check-failed",
        settle::Degraded::TooFewOrigins => "too-few-origins",
        settle::Degraded::TooLittleWeight => "too-little-weight",
    }
}

pub fn reason_word(reason: HeadChallengeReason) -> &'static str {
    match reason {
        HeadChallengeReason::ValueOrOther => "value-or-other",
        HeadChallengeReason::OffChannel => "off-channel",
        HeadChallengeReason::DuplicateOrigin => "duplicate-origin",
        HeadChallengeReason::ExpiredObservation => "expired-observation",
    }
}

pub fn kind_word(kind: HeadClaimKind) -> &'static str {
    match kind {
        HeadClaimKind::Fill => "fill",
        HeadClaimKind::Update => "update",
    }
}

/// A claim's status: committed, open (revealed, in its window), challenged, accepted (stood),
/// rejected (fell), expired (never revealed).
pub fn claim_status(c: &HeadClaim) -> &'static str {
    match c.status {
        HeadClaimStatus::Committed => "committed",
        HeadClaimStatus::Revealed if c.challenge_count > 0 => "challenged",
        HeadClaimStatus::Revealed => "open",
        HeadClaimStatus::Stood => "accepted",
        HeadClaimStatus::Fell => "rejected",
        HeadClaimStatus::Expired => "expired",
    }
}

/// A challenge's status: open (not ruled), upheld (won), dismissed (lost), settled (paid out).
pub fn challenge_status(c: &HeadChallenge) -> &'static str {
    match (c.ruling, c.settled) {
        (_, true) => "settled",
        (HeadChallengeRuling::Pending, false) => "open",
        (HeadChallengeRuling::Won, false) => "upheld",
        (HeadChallengeRuling::Lost, false) => "dismissed",
    }
}

fn hex_or_null(b: &[u8; 32]) -> Value {
    if b.iter().all(|x| *x == 0) {
        Value::Null
    } else {
        json!(hex(b))
    }
}

/// What people call an item ("DDR5 16Gb"): the commodity bucket's name, else its label.
pub fn item_name(product: Option<Product>, id: &[u8; 32]) -> String {
    product
        .and_then(|p| p.buckets().iter().find(|b| b.id32() == *id))
        .map(|b| b.name.to_owned())
        .or_else(|| hv::product_symbol(id))
        .unwrap_or_else(|| hex(id))
}

fn mold_item(product: Option<Product>, id: &[u8; 32]) -> Option<&'static molds::MoldItem> {
    let label = hv::product_symbol(id)?;
    product?.mold().item(&label)
}

pub fn mold_channel(channel: Channel) -> molds::Channel {
    molds::Channel::from_code(channel.code()).unwrap_or(molds::Channel::Spot)
}

/// "DDR5 16Gb Contract": component `index` of `board` (`item * 3 + channel`).
pub fn component_label(
    product: Option<Product>,
    board: &ameba_oracle::state::heads::HeadsBoard,
    index: u8,
) -> String {
    let i = usize::from(index).min(board.component_items.len() - 1);
    let item = usize::from(board.component_items[i]).min(board.item_bucket_ids.len() - 1);
    let channel = hv::channel_of(board.component_channels[i]);
    format!(
        "{} {}",
        item_name(product, &board.item_bucket_ids[item]),
        channel_name(channel)
    )
}

// ---------------------------------------------------------------------------------- snapshot

/// One board: its accounts and the view of them.
pub struct Board {
    pub symbol: String,
    pub product: Option<Product>,
    pub accounts: BoardAccounts,
    pub view: BoardView,
}

impl Board {
    fn label(&self, c: &ComponentView) -> String {
        component_label(self.product, &self.accounts.board, c.index)
    }
}

/// The heads accounts a document reads, at the chain clock `now`.
pub struct Snapshot {
    pub now: u64,
    pub boards: Vec<Board>,
    pub claims: Vec<(Pubkey, Box<HeadClaim>)>,
    pub challenges: Vec<(Pubkey, Box<HeadChallenge>)>,
    /// Heads' spec texts by commitment.
    pub specs: HashMap<[u8; 32], String>,
}

impl Snapshot {
    /// The boards of `products` (every product when empty), with their challenges, and
    /// the claims of `claims_of` (a head address) when given.
    pub fn read(chain: &Chain, products: &[Product], claims_of: Option<&Pubkey>) -> Result<Self> {
        let now = chain.now()?.unix;
        chain.config()?;
        let wanted: Vec<Product> = if products.is_empty() {
            Product::ALL.to_vec()
        } else {
            products.to_vec()
        };
        let mut boards = Vec::new();
        for product in wanted {
            let board = match chain.board(product) {
                Ok(b) => b,
                Err(_) if products.is_empty() => continue,
                Err(e) => return Err(e),
            };
            boards.push(Self::board(chain, &board, now)?);
        }
        let heads: BTreeSet<Pubkey> = boards
            .iter()
            .flat_map(|b| b.accounts.heads.iter().map(|(k, _)| *k))
            .collect();
        let challenges: Vec<_> = chain
            .scan::<HeadChallenge>(&[])?
            .into_iter()
            .filter(|(_, c)| heads.contains(&c.head))
            .collect();
        let claims = match claims_of {
            Some(head) => chain.scan::<HeadClaim>(&[(offset::CLAIM_HEAD, head)])?,
            None => Vec::new(),
        };
        let mut wanted: Vec<(Pubkey, [u8; 32])> = Vec::new();
        for (_, h) in boards.iter().flat_map(|b| &b.accounts.heads) {
            if !wanted.iter().any(|(_, hash)| *hash == h.spec_hash) {
                wanted.push((h.spec_object, h.spec_hash));
            }
        }
        let specs = chain.kind2_texts(&wanted)?;
        Ok(Self {
            now,
            boards,
            claims,
            challenges,
            specs,
        })
    }

    fn board(chain: &Chain, board: &BoardRef, now: u64) -> Result<Board> {
        let accounts = chain.board_accounts(board)?;
        let view = hv::view(&accounts, now);
        Ok(Board {
            symbol: board.symbol().to_string(),
            product: Some(board.product),
            accounts,
            view,
        })
    }
}

// --------------------------------------------------------------------------------- the board

fn params_json(p: &HeadsParams, max_liability_bps: u64) -> Value {
    json!({
        "slashRateBps": p.slash_rate_bps,
        "maxStrikes": p.max_strikes,
        "proposerSlashBps": p.proposer_slash_bps,
        "prizeCutBps": p.prize_cut_bps,
        "baseFillPrize": atoms(p.base_fill_prize),
        "fillPrizeSlopePpb": atoms(p.fill_prize_slope_ppb),
        "baseUpdatePrize": atoms(p.base_update_prize),
        "updatePrizeSlopePpb": atoms(p.update_prize_slope_ppb),
        "claimBond": atoms(p.claim_bond),
        "challengeBond": atoms(p.challenge_bond),
        "challengeWindowSecs": p.challenge_window_secs,
        "challengeExtensionSecs": p.challenge_extension_secs,
        "headTtlSecs": p.head_ttl_secs,
        "cooldownSecs": p.cooldown_secs,
        "minDelegators": p.min_delegators,
        "minLiveSources": p.min_live_sources,
        "minSettleWeight": atoms(p.min_settle_weight),
        "alphaBps": p.alpha_bps,
        "floorPost": atoms(p.floor_post),
        "betaBps": p.beta_bps,
        "floorActivate": atoms(p.floor_activate),
        "settlementBusinessDays": p.settlement_business_days,
        "maxLiabilityBps": max_liability_bps,
    })
}

/// A component's weight in its contract's index (Stage 1: the item's basket weight on its
/// Contract component; Stage 2: spread over the channels by the item's mix).
fn index_weight_bps(target: SettlementTarget, item: Option<&ItemView>, c: &ComponentView) -> u64 {
    let Some(item) = item else { return 0 };
    match target {
        SettlementTarget::ContractChannel => {
            if c.channel == Channel::Contract {
                u64::from(item.basket_weight_bps)
            } else {
                0
            }
        }
        SettlementTarget::FixedMixReference => {
            u64::from(item.basket_weight_bps) * u64::from(item.mix_bps[c.channel.index()]) / 10_000
        }
    }
}

fn reading_json(c: &ComponentView) -> Value {
    match &c.reading {
        hv::Reading::NoContract => json!({ "status": "no-contract", "changePpm": null }),
        hv::Reading::Live {
            change_ppm,
            eligible,
            duplicates,
            weight,
        } => json!({
            "status": "live",
            "changePpm": change_ppm,
            "eligibleSources": eligible,
            "duplicateSources": duplicates,
            "weight": atoms(*weight),
        }),
        hv::Reading::Degraded { reason } => json!({
            "status": "degraded",
            "changePpm": null,
            "reason": degraded_word(*reason),
        }),
        hv::Reading::Settled {
            change_ppm,
            live,
            reason,
        } => json!({
            "status": "settled",
            "changePpm": change_ppm,
            "live": live,
            "reason": reason.map(degraded_word),
        }),
    }
}

/// An item's reference (under a fixed-mix target, what the item settles at), with each
/// channel's part: its mix share, its change, and whether it would settle on its degraded path
/// (its last valid value).
fn reference_json(item: &ItemView, components: &[ComponentView]) -> Value {
    let channels: Vec<Value> = components
        .iter()
        .map(|c| {
            let (status, degraded, reason) = match &c.reading {
                hv::Reading::NoContract => ("no-contract", false, None),
                hv::Reading::Live { .. } => ("live", false, None),
                hv::Reading::Degraded { reason } => {
                    ("degraded", true, Some(degraded_word(*reason)))
                }
                hv::Reading::Settled { live, reason, .. } => {
                    ("settled", !live, reason.map(degraded_word))
                }
            };
            // Settling, a channel the mix weighs is held at its last valid value when degraded.
            let degraded = match item.reference {
                hv::ReferenceView::Available { degraded, .. } => degraded[c.channel.index()],
                hv::ReferenceView::Unavailable(_) => degraded,
            };
            json!({
                "channel": channel_word(c.channel),
                "mixBps": item.mix_bps[c.channel.index()],
                "changePpm": c.reading.live_change_ppm(),
                "status": status,
                "degraded": degraded,
                "reason": reason,
            })
        })
        .collect();
    let mut value = reference_body(item);
    value["channels"] = json!(channels);
    value
}

fn reference_body(item: &ItemView) -> Value {
    match item.reference {
        hv::ReferenceView::Available {
            change_ppm,
            assumed,
            recorded,
            ..
        } => json!({
            "available": true,
            "changePpm": change_ppm,
            "assumed": assumed,
            "flagged": assumed,
            "recorded": recorded,
        }),
        hv::ReferenceView::Unavailable(gap) => {
            let (code, channel, reason): (&str, Option<Channel>, String) = match gap {
                hv::ReferenceGap::NoContract => {
                    ("no-contract", None, "No contract is listed yet.".to_owned())
                }
                hv::ReferenceGap::Reference(Unavailable::MixBasisNone) => (
                    "mix-basis-none",
                    None,
                    "This item has no channel mix.".to_owned(),
                ),
                hv::ReferenceGap::Reference(Unavailable::InvalidMix) => (
                    "invalid-mix",
                    None,
                    "The channel mix does not add up to 100%.".to_owned(),
                ),
            };
            json!({
                "available": false,
                "code": code,
                "channel": channel.map(channel_word),
                "reason": reason,
            })
        }
    }
}

fn item_component_json(
    board: &Board,
    item: &ItemView,
    c: &ComponentView,
    target: SettlementTarget,
) -> Value {
    let mold =
        mold_item(board.product, &item.bucket_id).map(|m| m.component(mold_channel(c.channel)));
    let origins: BTreeSet<String> = c
        .sources
        .iter()
        .filter(|s| !s.duplicate)
        .map(|s| hex(&s.origin))
        .collect();
    json!({
        "index": c.index,
        "channel": channel_word(c.channel),
        "label": board.label(c),
        "settles": settles(target, Some(item), c),
        "indexWeightBps": index_weight_bps(target, Some(item), c),
        "sentence": mold.map(|m| m.sentence.clone()),
        "sentenceHash": hex_or_null(&c.sentence_hash),
        "sentenceMatches": mold.map(|m| m.sentence_sha256 == c.sentence_hash),
        "liveSources": c.live_sources,
        "liveOrigins": c.live_origins,
        "origins": origins,
        "sources": c.sources.iter().map(|s| json!({
            "head": s.head_id,
            "shareBps": s.share_bps,
            "duplicate": s.duplicate,
            "origin": hex(&s.origin),
            "delegation": atoms(s.delegation),
            "weight": atoms(s.weight),
            "level": s.level.map(atoms),
            "opening": s.opening.map(atoms),
            "changePpm": s.change_ppm,
        })).collect::<Vec<_>>(),
        "changePpm": match &c.reading {
            hv::Reading::Live { change_ppm, .. } | hv::Reading::Settled { change_ppm, .. } => json!(change_ppm),
            _ => Value::Null,
        },
        "reading": reading_json(c),
    })
}

fn items_json(
    board: &Board,
    items: &[ItemView],
    components: &[ComponentView],
    target: SettlementTarget,
) -> Value {
    json!(items
        .iter()
        .map(|item| {
            let mold = mold_item(board.product, &item.bucket_id);
            let first = usize::from(item.index) * 3;
            json!({
                "index": item.index,
                "bucketId": hex(&item.bucket_id),
                "name": item_name(board.product, &item.bucket_id),
                "basketWeightBps": item.basket_weight_bps,
                "mixBps": { "spot": item.mix_bps[0], "contract": item.mix_bps[1], "lta": item.mix_bps[2] },
                "mixBasis": basis_word(item.mix_basis),
                "mixReason": mold.map(|m| m.mix_reason.clone()),
                "components": components
                    .get(first..(first + 3).min(components.len()))
                    .unwrap_or_default()
                    .iter()
                    .map(|c| item_component_json(board, item, c, target))
                    .collect::<Vec<_>>(),
                "reference": reference_json(
                    item,
                    components.get(first..(first + 3).min(components.len())).unwrap_or_default(),
                ),
            })
        })
        .collect::<Vec<_>>())
}

fn index_json(reading: &hv::IndexReading, period: Option<&Pubkey>) -> Value {
    let contract = period.map(|p| p.to_string());
    match reading {
        hv::IndexReading::Indicative {
            change_ppm,
            value,
            degraded,
        } => json!({
            "contract": contract,
            "status": "indicative",
            "changePpm": change_ppm,
            "value": value.map(atoms),
            "degradedComponents": degraded,
            "reason": null,
        }),
        hv::IndexReading::Settled { change_ppm, value } => json!({
            "contract": contract,
            "status": "settled",
            "changePpm": change_ppm,
            "value": atoms(*value),
            "degradedComponents": [],
            "reason": null,
        }),
        hv::IndexReading::Unavailable(gap) => json!({
            "contract": contract,
            "status": "unavailable",
            "changePpm": null,
            "value": null,
            "degradedComponents": [],
            "reason": match gap {
                hv::IndexGap::NoContract => "No contract is listed yet.".to_owned(),
                hv::IndexGap::Refused(Refusal::BasketWeights) => "The basket weights do not add up to 100%.".to_owned(),
                hv::IndexGap::Refused(Refusal::Unavailable { item, .. }) => format!("Item {item}'s reference is unavailable."),
            },
        }),
    }
}

fn contract_summary(board: &Board, i: usize) -> Value {
    let (address, k) = &board.accounts.contracts[i];
    let view = &board.view.contracts[i];
    let (change, value) = match &view.index {
        hv::IndexReading::Indicative {
            change_ppm, value, ..
        } => (json!(change_ppm), value.map(atoms).unwrap_or(Value::Null)),
        hv::IndexReading::Settled { change_ppm, value } => (json!(change_ppm), atoms(*value)),
        hv::IndexReading::Unavailable(_) => (Value::Null, Value::Null),
    };
    json!({
        "period": k.period.to_string(),
        "expiry": k.expiry_ts,
        "status": contract_status_word(k.status),
        "listingTs": k.listing_ts,
        "freezeTs": k.freeze_ts,
        "address": address.to_string(),
        "quarter": molds::Quarter::of_expiry(k.expiry_ts as i64).label(),
        "label": format!("{} {}", board.symbol, molds::Quarter::of_expiry(k.expiry_ts as i64).label()),
        "settlementTarget": target_word(view.target),
        "disclosure": disclosure(board.product, Some(k.expiry_ts)),
        "indexChangePpm": change,
        "value": value,
    })
}

fn open_challenges(snap: &Snapshot, head: &Pubkey) -> usize {
    snap.challenges
        .iter()
        .filter(|(_, c)| c.head == *head && c.ruling == HeadChallengeRuling::Pending)
        .count()
}

/// A head as a board lists it.
pub fn head_json(snap: &Snapshot, board: &Board, key: &Pubkey, h: &Head) -> Value {
    let component = board.view.component(h.component);
    let weights: Vec<Value> = board
        .accounts
        .contracts
        .iter()
        .filter(|(_, k)| k.status != HeadsContractStatus::Settled)
        .filter_map(|(ck, k)| {
            let raw = hv::head_raw_weight(h, k, snap.now).unwrap_or(0);
            let effective = board
                .accounts
                .weights_of(ck, h.component)
                .and_then(|w| w.weight(h.head_id));
            let listed = (0..usize::from(k.listed_count).min(k.listed_head_ids.len()))
                .any(|i| k.listed_head_ids[i] == h.head_id);
            (raw > 0 || listed || effective.is_some()).then(|| {
                json!({
                    "period": k.period.to_string(),
                    "rawWeight": atoms(raw),
                    "effectiveWeight": effective.map(atoms),
                })
            })
        })
        .collect();
    json!({
        "id": h.head_id,
        "address": key.to_string(),
        "product": board.symbol,
        "component": h.component,
        "name": h.name(),
        "spec": snap.specs.get(&h.spec_hash).cloned().unwrap_or_default(),
        "specHash": hex(&h.spec_hash),
        "accessClass": access_word(h.access_class),
        "proposer": h.proposer.to_string(),
        "status": status_word(h.status),
        "delegation": atoms(h.delegation),
        "delegators": h.delegators,
        "postedAt": h.posted_ts,
        "expiresAt": h.expires_ts,
        "activeAt": time_or_null(h.active_ts),
        "liveAt": time_or_null(h.live_ts),
        "retiredAt": time_or_null(h.retired_ts),
        "strikes": h.strike_count,
        "value": if h.value == 0 { Value::Null } else { atoms(h.value) },
        "sourceTs": time_or_null(h.value_source_ts),
        "shareBps": if h.status == HeadStatus::Live { json!(board.view.share_bps(h.head_id)) } else { Value::Null },
        "openClaims": h.open_claims,
        "openChallenges": open_challenges(snap, key),
        "weights": weights,
        "channel": component.map(|c| channel_word(c.channel)),
        "componentLabel": component.map(|c| board.label(c)),
        "originId": hex_or_null(&h.origin_id),
        "duplicate": h.status == HeadStatus::Live && component.is_some_and(|c| c.sources.iter().any(|s| s.head_id == h.head_id && s.duplicate)),
    })
}

/// `items` with each one's mix and basis from its product's mold schedule for `quarter` (an item
/// the molds don't know keeps the board's).
fn scheduled(board: &Board, items: &[ItemView], quarter: molds::Quarter) -> Vec<ItemView> {
    items
        .iter()
        .map(|item| {
            let mut item = item.clone();
            if let Some(m) = mold_item(board.product, &item.bucket_id) {
                item.mix_bps = m.mix_for(quarter);
                item.mix_basis = MixBasis::from_code(m.mix_basis.code()).unwrap_or(item.mix_basis);
            }
            item
        })
        .collect()
}

/// One board, as `GET /v1/oracle/heads/:product` serves it.
pub fn board_json(snap: &Snapshot, board: &Board) -> Value {
    let b = &board.accounts.board;
    let v = &board.view;
    let front = v.front.map(|i| &board.accounts.contracts[i].1);
    let mut heads: Vec<&(Pubkey, Box<Head>)> = board.accounts.heads.iter().collect();
    heads.sort_by_key(|(_, h)| {
        let rank = match h.status {
            HeadStatus::Live => 0,
            HeadStatus::Active => 1,
            HeadStatus::Open => 2,
            HeadStatus::Retired => 3,
            HeadStatus::Expired => 4,
        };
        (rank, std::cmp::Reverse(h.delegation), h.head_id)
    });
    let mut contracts: Vec<usize> = (0..board.accounts.contracts.len()).collect();
    contracts.sort_by_key(|i| board.accounts.contracts[*i].1.expiry_ts);
    let target = board_target(board);
    // The items' mixes: the front contract's, frozen for its quarter; without one, the molds'
    // schedule for the quarter the next listing covers (the one holding now).
    let quarter = match front {
        Some(k) => molds::Quarter::of_expiry(k.expiry_ts as i64),
        None => molds::Quarter::of_expiry(snap.now as i64 + 1),
    };
    let items = match front {
        Some(_) => v.items.clone(),
        None => scheduled(board, &v.items, quarter),
    };
    json!({
        "product": board.symbol,
        "address": board.accounts.address.to_string(),
        "params": params_json(&b.params, v.max_liability_bps),
        "components": v.components.iter().map(|c| json!({
            "index": c.index,
            "bucketId": hex(&b.item_bucket_ids[usize::from(c.item).min(7)]),
            "label": board.label(c),
            "basketWeightBps": index_weight_bps(target, items.get(usize::from(c.item)), c),
            "liveSources": c.live_sources,
            "item": c.item,
            "channel": channel_word(c.channel),
            "liveOrigins": c.live_origins,
            "settles": settles(target, items.get(usize::from(c.item)), c),
            "sentenceHash": hex_or_null(&c.sentence_hash),
        })).collect::<Vec<_>>(),
        "totals": {
            "delegation": atoms(b.total_delegation),
            "liveDelegation": atoms(b.live_delegation),
            "reserve": atoms(b.reserve),
            "prizePool": atoms(b.prize_funded_total.saturating_sub(b.prize_paid_total)),
        },
        "thresholds": {
            "minPost": atoms(v.thresholds.min_post),
            "activation": atoms(v.thresholds.activation),
        },
        "heads": heads.iter().map(|(k, h)| head_json(snap, board, k, h)).collect::<Vec<_>>(),
        "contracts": contracts.iter().map(|i| contract_summary(board, *i)).collect::<Vec<_>>(),
        "settlementTarget": target_word(target),
        "disclosure": disclosure(board.product, front.map(|k| k.expiry_ts)),
        "quarter": quarter.label(),
        "items": items_json(board, &items, &v.components, target),
        "index": index_json(&v.front_index(), front.map(|k| &k.period)),
    })
}

/// `GET /v1/oracle/heads`.
pub fn document(snap: &Snapshot) -> Value {
    json!({
        "boards": snap.boards.iter().map(|b| board_json(snap, b)).collect::<Vec<_>>(),
        "updatedAt": snap.now,
    })
}

/// What a claim's prize is now: paid once claimed, its points once it stood, what it would earn
/// while open, nothing once it fell or expired.
pub fn claim_prize(board: &ameba_oracle::state::heads::HeadsBoard, h: &Head, c: &HeadClaim) -> u64 {
    let params = board.params.to_params();
    match c.status {
        _ if c.prize_claimed => c.prize_paid,
        HeadClaimStatus::Stood => c.prize_points,
        HeadClaimStatus::Committed | HeadClaimStatus::Revealed => match c.kind {
            HeadClaimKind::Fill => rewards::fill_prize(&params, h.delegation),
            HeadClaimKind::Update => rewards::update_prize(&params, h.delegation),
        },
        HeadClaimStatus::Fell | HeadClaimStatus::Expired => 0,
    }
}

fn evidence_json(kind: u8, commitment: &[u8; 32], object: &Pubkey) -> Value {
    if kind == 0 {
        return Value::Null;
    }
    json!({ "kind": kind, "commitment": hex(commitment), "object": object.to_string() })
}

/// `GET /v1/oracle/heads/:product/:headId`: the head with its claims, challenges, delegations
/// and accepted prints.
pub fn head_detail_json(snap: &Snapshot, board: &Board, key: &Pubkey, h: &Head) -> Value {
    let mut value = head_json(snap, board, key, h);
    let lta = value["channel"] == "lta";
    let mut claims: Vec<&(Pubkey, Box<HeadClaim>)> =
        snap.claims.iter().filter(|(_, c)| c.head == *key).collect();
    claims.sort_by_key(|(_, c)| std::cmp::Reverse(c.seq));
    let seq_of = |target: &Pubkey| {
        snap.claims
            .iter()
            .find(|(k, _)| k == target)
            .map(|(_, c)| c.seq)
    };
    let mut challenges: Vec<&(Pubkey, Box<HeadChallenge>)> = snap
        .challenges
        .iter()
        .filter(|(_, c)| c.head == *key)
        .collect();
    challenges.sort_by_key(|(_, c)| (std::cmp::Reverse(c.opened_ts), c.index));
    let mut delegations: Vec<&(Pubkey, Box<Delegation>)> =
        board.accounts.delegations_on(key).collect();
    delegations.sort_by_key(|(_, d)| (std::cmp::Reverse(d.active + d.fresh), d.owner));
    let b = &board.accounts.board;
    value["claims"] = json!(claims
        .iter()
        .map(|(k, c)| json!({
            "address": k.to_string(),
            "seq": c.seq,
            "kind": kind_word(c.kind),
            "fetcher": c.fetcher.to_string(),
            "value": if c.status == HeadClaimStatus::Committed || c.value == 0 { Value::Null } else { atoms(c.value) },
            "sourceTs": time_or_null(c.source_ts),
            "originId": hex_or_null(&c.origin_id),
            "lta": if lta && c.status != HeadClaimStatus::Committed {
                json!({
                    "effectiveFrom": time_or_null(c.lta_effective_from_ts),
                    "nextReset": time_or_null(c.lta_next_reset_ts),
                    "floor": if c.lta_floor == 0 { Value::Null } else { atoms(c.lta_floor) },
                    "ceiling": if c.lta_ceiling == 0 { Value::Null } else { atoms(c.lta_ceiling) },
                })
            } else { Value::Null },
            "evidence": evidence_json(c.evidence_kind, &c.evidence_commitment, &c.evidence_object),
            "bond": atoms(c.bond),
            "committedAt": time_or_null(c.commit_ts),
            "revealedAt": time_or_null(c.revealed_ts),
            "windowEndsAt": time_or_null(c.window_end_ts),
            "challenges": c.challenge_count,
            "status": claim_status(c),
            "prize": atoms(claim_prize(b, h, c)),
        }))
        .collect::<Vec<_>>());
    value["challenges"] = json!(challenges
        .iter()
        .map(|(k, c)| json!({
            "address": k.to_string(),
            "target": if c.kind == HeadChallengeKind::Source { "source" } else { "claim" },
            "seq": if c.kind == HeadChallengeKind::Claim { json!(seq_of(&c.target)) } else { Value::Null },
            "claim": if c.kind == HeadChallengeKind::Claim { json!(c.target.to_string()) } else { Value::Null },
            "challenger": c.challenger.to_string(),
            "reason": reason_word(c.reason),
            "bond": atoms(c.bond),
            "alternativeValue": if c.alternative_value == 0 { Value::Null } else { atoms(c.alternative_value) },
            "alternativeSourceTs": time_or_null(c.alternative_source_ts),
            "evidence": evidence_json(c.evidence_kind, &c.evidence_commitment, &c.evidence_object),
            "openedAt": time_or_null(c.opened_ts),
            "ruledAt": time_or_null(c.ruled_ts),
            "status": challenge_status(c),
            "payout": atoms(c.payout_usdc),
            "payoutAmba": atoms(c.payout_amba),
        }))
        .collect::<Vec<_>>());
    value["delegations"] = json!(
        delegations
            .iter()
            .map(|(k, d)| {
                let p = hv::position(b, h, d);
                json!({
                    "address": k.to_string(),
                    "owner": d.owner.to_string(),
                    "amount": atoms(p.amount),
                    "cooling": atoms(p.cooling),
                    "escrow": atoms(p.escrow),
                    "proposer": d.is_proposer,
                })
            })
            .collect::<Vec<_>>()
    );
    value["prints"] = json!(
        hv::head_prints(h)
            .iter()
            .rev()
            .map(|p| {
                let mut print = json!({ "sourceTs": p.source_ts, "value": atoms(p.value) });
                if lta {
                    print["effectiveFrom"] = time_or_null(p.terms.effective_from_ts);
                    print["nextReset"] = time_or_null(p.terms.next_reset_ts);
                }
                print
            })
            .collect::<Vec<_>>()
    );
    value
}

/// `GET /v1/oracle/heads/contracts/:period/terms`, from chain: the canonical listing terms
/// document of the contract's product and expiry (`molds::listing_terms_document`) beside the
/// commitment the contract listed with, and the mold's sentences beside the frozen hashes.
pub fn terms_json(chain: &Chain, period: &Pubkey) -> Result<Value> {
    chain.config()?;
    let key = ameba_oracle::state::heads::find_heads_contract_address(&chain.program(), period).0;
    let k = chain
        .maybe::<ameba_oracle::state::heads::HeadsContract>(&key, "contract")?
        .ok_or_else(|| Error::msg(format!("No contract is listed for period {period}.")))?;
    terms_doc(&k)
}

/// The terms document of contract `k`.
pub fn terms_doc(k: &ameba_oracle::state::heads::HeadsContract) -> Result<Value> {
    let symbol = hv::product_symbol(&k.product_id).unwrap_or_default();
    let product = Product::from_symbol(&symbol)
        .ok_or_else(|| Error::msg(format!("{symbol} has no channel mold.")))?;
    let expiry = i64::try_from(k.expiry_ts)
        .map_err(|_| Error::msg("That contract's expiry is out of range."))?;
    let document = molds::listing_terms_document(product, expiry);
    let sha = molds::sha256(&document);
    let mold = product.mold();
    let sentences: Vec<Value> = (0..usize::from(k.component_count).min(24))
        .map(|c| {
            let bucket = k.item_bucket_ids[usize::from(k.component_items[c]).min(7)];
            let channel = hv::channel_of(k.component_channels[c]);
            let component = hv::product_symbol(&bucket)
                .and_then(|label| mold.item(&label))
                .map(|item| item.component(mold_channel(channel)));
            json!({
                "index": c,
                "item": k.component_items[c],
                "bucketId": hex(&bucket),
                "channel": channel_word(channel),
                "sentence": component.map(|m| m.sentence.clone()),
                "sentenceSha256": component.map(|m| m.sentence_sha256_hex()),
                "onChain": hex_or_null(&k.component_sentence_hashes[c]),
                "matches": component.map(|m| m.sentence_sha256 == k.component_sentence_hashes[c]),
            })
        })
        .collect();
    let target =
        ameba_oracle_heads::channels::SettlementTarget::from_code(k.settlement_target as u8)
            .unwrap_or(SettlementTarget::ContractChannel);
    Ok(json!({
        "period": k.period.to_string(),
        "product": symbol,
        "expiry": k.expiry_ts,
        "settlementTarget": target_word(target),
        "molds": molds::CHANNEL_MOLDS_VERSION,
        "moldsSha256": molds::CHANNEL_MOLDS_SHA256_HEX,
        "document": String::from_utf8_lossy(&document),
        "documentSha256": hex(&sha),
        "termsHash": hex_or_null(&k.terms_hash),
        "matches": sha == k.terms_hash,
        "sentences": sentences,
    }))
}

// --------------------------------------------------------------------------------- heads mine

/// The accounts `heads mine` reads for one owner.
pub struct Mine {
    pub owner: Pubkey,
    pub now: u64,
    pub stake: Option<Box<HeadsStake>>,
    /// Every board the owner touches, by address, with its symbol.
    pub boards: HashMap<Pubkey, (String, Box<ameba_oracle::state::heads::HeadsBoard>)>,
    /// Every head the owner touches, by address.
    pub heads: HashMap<Pubkey, Box<Head>>,
    pub delegations: Vec<(Pubkey, Box<Delegation>)>,
    pub claims: Vec<(Pubkey, Box<HeadClaim>)>,
    pub challenges: Vec<(Pubkey, Box<HeadChallenge>)>,
    /// The claims the owner's challenges target, by address (their numbers).
    pub targets: HashMap<Pubkey, u32>,
    /// Prize epochs by (board, month).
    pub epochs: HashMap<(Pubkey, u32), Box<HeadsEpoch>>,
    /// Reason texts of the owner's challenges that sealed theirs (kind 2 committing to the
    /// reason hash), by reason hash.
    pub reasons: HashMap<[u8; 32], String>,
}

impl Mine {
    pub fn read(chain: &Chain, owner: &Pubkey) -> Result<Self> {
        let now = chain.now()?.unix;
        chain.config()?;
        let stake = chain.stake(owner)?;
        let delegations = chain.scan::<Delegation>(&[(offset::DELEGATION_OWNER, owner)])?;
        let claims = chain.scan::<HeadClaim>(&[(offset::CLAIM_FETCHER, owner)])?;
        let challenges = chain.scan::<HeadChallenge>(&[(offset::CHALLENGE_CHALLENGER, owner)])?;
        let mut head_keys: BTreeSet<Pubkey> = BTreeSet::new();
        head_keys.extend(delegations.iter().map(|(_, d)| d.head));
        head_keys.extend(claims.iter().map(|(_, c)| c.head));
        head_keys.extend(challenges.iter().map(|(_, c)| c.head));
        let head_keys: Vec<Pubkey> = head_keys.into_iter().collect();
        let heads: HashMap<Pubkey, Box<Head>> = head_keys
            .iter()
            .zip(chain.many::<Head>(&head_keys)?)
            .filter_map(|(k, h)| h.map(|h| (*k, h)))
            .collect();
        let board_keys: Vec<Pubkey> = heads
            .values()
            .map(|h| h.board)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let boards = board_keys
            .iter()
            .zip(chain.many::<ameba_oracle::state::heads::HeadsBoard>(&board_keys)?)
            .filter_map(|(k, b)| {
                let b = b?;
                Some((*k, (hv::product_symbol(&b.product_id)?, b)))
            })
            .collect();
        let target_keys: Vec<Pubkey> = challenges
            .iter()
            .filter(|(_, c)| c.kind == HeadChallengeKind::Claim)
            .map(|(_, c)| c.target)
            .collect();
        let targets = target_keys
            .iter()
            .zip(chain.many::<HeadClaim>(&target_keys)?)
            .filter_map(|(k, c)| c.map(|c| (*k, c.seq)))
            .collect();
        let epoch_ids: Vec<(Pubkey, u32)> = claims
            .iter()
            .filter(|(_, c)| c.status == HeadClaimStatus::Stood && !c.prize_claimed)
            .map(|(_, c)| (c.board, c.epoch))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let program = chain.program();
        let epoch_keys: Vec<Pubkey> = epoch_ids
            .iter()
            .map(|(b, m)| find_heads_epoch_address(&program, b, *m).0)
            .collect();
        let epochs = epoch_ids
            .into_iter()
            .zip(chain.many::<HeadsEpoch>(&epoch_keys)?)
            .filter_map(|(id, e)| e.map(|e| (id, e)))
            .collect();
        let objects: Vec<(Pubkey, [u8; 32])> = challenges
            .iter()
            .filter(|(_, c)| c.evidence_kind == 2)
            .map(|(_, c)| (c.evidence_object, c.evidence_commitment))
            .collect();
        let texts = chain.kind2_texts(&objects)?;
        let reasons = challenges
            .iter()
            .filter_map(|(_, c)| {
                let text = texts.get(&c.evidence_commitment)?;
                (ameba_oracle::instruction::heads::heads_reason_hash(text.as_bytes())
                    == c.reason_hash)
                    .then(|| (c.reason_hash, text.clone()))
            })
            .collect();
        Ok(Self {
            owner: *owner,
            now,
            stake,
            boards,
            heads,
            delegations,
            claims,
            challenges,
            targets,
            epochs,
            reasons,
        })
    }

    /// The head at `key` with its board and product symbol.
    fn head(&self, key: &Pubkey) -> Option<(&str, &ameba_oracle::state::heads::HeadsBoard, &Head)> {
        let h = self.heads.get(key)?;
        let (symbol, board) = self.boards.get(&h.board)?;
        Some((symbol.as_str(), board.as_ref(), h.as_ref()))
    }
}

fn by_product(symbol: &str) -> usize {
    Product::ALL
        .iter()
        .position(|p| p.symbol() == symbol)
        .unwrap_or(Product::ALL.len())
}

/// `petri oracle heads mine`: the account document (`GET /v1/accounts/:owner/heads`) with each
/// head's name and status, the proposer flag, and the follow-up commands that move the owner's
/// money along (a delegation's `syncCommand`, a claim's `prizeCommand`, the rewards'
/// `claimCommand`), exactly the forms the plugin runs.
pub fn mine_json(m: &Mine) -> Value {
    let mut delegations: Vec<(usize, u32, Value)> = Vec::new();
    let mut claimable = 0u64;
    for (key, d) in &m.delegations {
        let Some((symbol, board, h)) = m.head(&d.head) else {
            continue;
        };
        let p = hv::position(board, h, d);
        claimable = claimable.saturating_add(p.rewards);
        // Something to release now (the cooldown is over and the escrow allows it), or an
        // emptied delegation to close.
        let due = (p.cooling > 0 && p.cooldown_ends_at.is_some_and(|t| m.now >= t))
            || (p.amount == 0 && p.cooling == 0 && p.escrow == 0 && d.rewards_accrued == 0);
        let mut row = json!({
            "address": key.to_string(),
            "product": symbol,
            "head": h.head_id,
            "name": h.name(),
            "headStatus": status_word(h.status),
            "proposer": d.is_proposer,
            "amount": atoms(p.amount),
            "cooling": atoms(p.cooling),
            "escrow": atoms(p.escrow),
            "cooldownEndsAt": p.cooldown_ends_at.map(|t| json!(t)).unwrap_or(Value::Null),
            "rewards": atoms(p.rewards),
        });
        if due {
            row["syncCommand"] = json!(commands::sync(symbol, h.head_id));
        }
        delegations.push((by_product(symbol), h.head_id, row));
    }
    delegations.sort_by_key(|(p, h, _)| (*p, *h));
    let mut claims: Vec<(usize, u32, u32, Value)> = Vec::new();
    for (key, c) in &m.claims {
        let Some((symbol, board, h)) = m.head(&c.head) else {
            continue;
        };
        let mut row = json!({
            "address": key.to_string(),
            "product": symbol,
            "head": h.head_id,
            "name": h.name(),
            "seq": c.seq,
            "kind": kind_word(c.kind),
            "value": if c.status == HeadClaimStatus::Committed || c.value == 0 { Value::Null } else { atoms(c.value) },
            "sourceTs": time_or_null(c.source_ts),
            "bond": atoms(c.bond),
            "windowEndsAt": time_or_null(c.window_end_ts),
            "status": claim_status(c),
            "prize": atoms(claim_prize(board, h, c)),
        });
        // A standing claim's prize, once its month's epoch has ended.
        let epoch = m.epochs.get(&(c.board, c.epoch));
        if c.status == HeadClaimStatus::Stood
            && !c.prize_claimed
            && c.prize_points > 0
            && epoch.is_some_and(|e| m.now >= e.end_ts)
        {
            row["prize"] = atoms(epoch.map_or(0, |e| {
                rewards::prize_payout(c.prize_points, e.prize_points, e.prize_pool)
            }));
            row["prizeCommand"] = json!(commands::prize(symbol, h.head_id, c.seq));
        }
        claims.push((by_product(symbol), h.head_id, c.seq, row));
    }
    claims.sort_by_key(|(p, h, s, _)| (*p, *h, std::cmp::Reverse(*s)));
    let mut challenges: Vec<(usize, u32, Value)> = Vec::new();
    for (key, c) in &m.challenges {
        let Some((symbol, _, h)) = m.head(&c.head) else {
            continue;
        };
        let claim = c.kind == HeadChallengeKind::Claim;
        challenges.push((
            by_product(symbol),
            h.head_id,
            json!({
                "address": key.to_string(),
                "product": symbol,
                "head": h.head_id,
                "name": h.name(),
                "target": if claim { "claim" } else { "source" },
                "seq": if claim { json!(m.targets.get(&c.target)) } else { Value::Null },
                "claim": if claim { json!(c.target.to_string()) } else { Value::Null },
                "bond": atoms(c.bond),
                "status": challenge_status(c),
                "code": reason_word(c.reason),
                "reason": m.reasons.get(&c.reason_hash).cloned().unwrap_or_else(|| reason_word(c.reason).to_string()),
                "payout": atoms(c.payout_usdc),
            }),
        ));
    }
    challenges.sort_by_key(|(p, h, _)| (*p, *h));
    let mut rewards = json!({ "claimable": atoms(claimable) });
    if claimable > 0 {
        rewards["claimCommand"] = json!(commands::rewards());
    }
    json!({
        "owner": m.owner.to_string(),
        "stake": { "free": atoms(m.stake.as_ref().map_or(0, |s| s.free)) },
        "delegations": delegations.into_iter().map(|(_, _, v)| v).collect::<Vec<_>>(),
        "claims": claims.into_iter().map(|(_, _, _, v)| v).collect::<Vec<_>>(),
        "challenges": challenges.into_iter().map(|(_, _, v)| v).collect::<Vec<_>>(),
        "rewards": rewards,
        "updatedAt": m.now,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ameba_oracle::{
        instruction::heads::heads_reason_hash,
        state::heads::{HeadsBoard, find_head_address, find_heads_board_address, zeroed_box},
    };
    use solana_program::hash::hashv;

    const NOW: u64 = 1_793_250_000;
    const AMBA: u64 = 1_000_000_000;
    const USDC: u64 = 1_000_000;

    fn key(name: &str) -> Pubkey {
        Pubkey::new_from_array(hashv(&[b"petri-heads-test", name.as_bytes()]).to_bytes())
    }

    fn head(
        program: &Pubkey,
        board: &Pubkey,
        id: u32,
        name: &str,
        status: HeadStatus,
    ) -> (Pubkey, Box<Head>) {
        let mut h = zeroed_box::<Head>();
        h.board = *board;
        h.head_id = id;
        h.component = 1;
        h.status = status;
        h.name_len = name.len() as u8;
        h.name[..name.len()].copy_from_slice(name.as_bytes());
        h.delegation = 3_000 * AMBA;
        (find_head_address(program, board, id).0, h)
    }

    /// The owner's part as `heads mine` reads it: a delegation to release, one earning, a
    /// claim whose month ended, a challenge with its sealed reason.
    pub(crate) fn mine() -> Mine {
        let program = key("oracle");
        let owner = key("owner");
        let id = crate::oracle::skus::encode_label("RAMX").unwrap();
        let board_key = find_heads_board_address(&program, &id).0;
        let mut board = zeroed_box::<HeadsBoard>();
        board.product_id = id;
        board.params = HeadsParams::spec_defaults();
        let (k0, h0) = head(
            &program,
            &board_key,
            0,
            "TrendForce DDR5 16Gb contract price",
            HeadStatus::Live,
        );
        let (k2, h2) = head(
            &program,
            &board_key,
            2,
            "Arrow DDR5 16Gb weekly",
            HeadStatus::Live,
        );
        let mut earning = zeroed_box::<Delegation>();
        (
            earning.head,
            earning.owner,
            earning.board,
            earning.is_proposer,
        ) = (k0, owner, board_key, true);
        earning.active = 1_000 * AMBA;
        earning.rewards_accrued = 2_500_000;
        let mut cooled = zeroed_box::<Delegation>();
        (cooled.head, cooled.owner, cooled.board) = (k2, owner, board_key);
        (cooled.held, cooled.held_principal) = (1_000 * AMBA, 1_000 * AMBA);
        (cooled.cooldown_end_ts, cooled.exit_ts, cooled.created_ts) =
            (NOW - 60, NOW - 1_209_660, NOW - 2_000_000);
        let month = ameba_oracle_heads::time::month_id(NOW - 40 * 86_400);
        let mut claim = zeroed_box::<HeadClaim>();
        (claim.head, claim.board, claim.seq, claim.fetcher) = (k0, board_key, 3, owner);
        (claim.kind, claim.status) = (HeadClaimKind::Update, HeadClaimStatus::Stood);
        (
            claim.value,
            claim.source_ts,
            claim.bond,
            claim.window_end_ts,
        ) = (4_850_000, NOW - 40 * 86_400, USDC, NOW - 39 * 86_400);
        (claim.prize_points, claim.epoch) = (USDC, month);
        let mut epoch = zeroed_box::<HeadsEpoch>();
        (epoch.board, epoch.month_id, epoch.end_ts) = (board_key, month, NOW - 86_400);
        (epoch.prize_points, epoch.prize_pool) = (2 * USDC, USDC);
        let target = key("claim 0 of head 2");
        let reason = "The quote is for DDR4 4Gb, not 8Gb.";
        let mut challenge = zeroed_box::<HeadChallenge>();
        (challenge.target, challenge.head, challenge.challenger) = (target, k2, owner);
        (challenge.bond, challenge.reason_hash) = (USDC, heads_reason_hash(reason.as_bytes()));
        (challenge.evidence_kind, challenge.evidence_commitment) =
            (2, hashv(&[reason.as_bytes()]).to_bytes());
        let mut stake = zeroed_box::<HeadsStake>();
        (stake.owner, stake.free) = (owner, 500 * AMBA);
        Mine {
            owner,
            now: NOW,
            stake: Some(stake),
            boards: HashMap::from([(board_key, ("RAMX".to_string(), board))]),
            heads: HashMap::from([(k0, h0), (k2, h2)]),
            delegations: vec![(key("d0"), earning), (key("d2"), cooled)],
            claims: vec![(key("claim"), claim)],
            challenges: vec![(key("challenge"), challenge)],
            targets: HashMap::from([(target, 0)]),
            epochs: HashMap::from([((board_key, month), epoch)]),
            reasons: HashMap::from([(heads_reason_hash(reason.as_bytes()), reason.to_string())]),
        }
    }

    fn keys(v: &Value) -> BTreeSet<String> {
        v.as_object()
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// The fields and follow-ups the plugin reads (`plugins/petri/client.js`, `MineView`,
    /// `headsDueOf`; `lib/oracle.mjs`, `STEPS`), and every field of the API's account document.
    #[test]
    fn mine_is_the_account_document_with_the_plugins_follow_ups() {
        let doc = mine_json(&mine());
        assert_eq!(doc["stake"]["free"], "500000000000");
        let d = &doc["delegations"];
        assert_eq!(
            (d[0]["head"].as_u64(), d[0]["proposer"].as_bool()),
            (Some(0), Some(true))
        );
        assert_eq!(d[0]["headStatus"], "live");
        assert!(
            d[0].get("syncCommand").is_none(),
            "nothing to release on an earning delegation"
        );
        assert_eq!(d[1]["cooling"], "1000000000000");
        assert_eq!(
            d[1]["syncCommand"],
            "petri oracle heads sync 2 --product RAMX"
        );
        assert!(doc.get("seats").is_none(), "fetchers hold no seats");
        let c = &doc["claims"][0];
        assert_eq!(
            (c["status"].as_str(), c["kind"].as_str()),
            (Some("accepted"), Some("update"))
        );
        assert_eq!(c["prize"], "500000", "its share of a short month's pool");
        assert_eq!(
            c["prizeCommand"],
            "petri oracle heads prize 0 3 --product RAMX"
        );
        let ch = &doc["challenges"][0];
        assert_eq!(
            (
                ch["target"].as_str(),
                ch["seq"].as_u64(),
                ch["status"].as_str()
            ),
            (Some("claim"), Some(0), Some("open"))
        );
        assert_eq!(ch["reason"], "The quote is for DDR4 4Gb, not 8Gb.");
        assert_eq!(doc["rewards"]["claimable"], "2500000");
        assert_eq!(
            doc["rewards"]["claimCommand"],
            "petri oracle heads rewards --claim"
        );
        // Every field the plugin's notes name.
        for (list, wanted) in [
            (
                "delegations",
                &[
                    "product",
                    "head",
                    "name",
                    "headStatus",
                    "proposer",
                    "amount",
                    "cooling",
                    "escrow",
                    "cooldownEndsAt",
                ][..],
            ),
            (
                "claims",
                &[
                    "product",
                    "head",
                    "name",
                    "seq",
                    "kind",
                    "value",
                    "bond",
                    "status",
                    "windowEndsAt",
                    "prize",
                ][..],
            ),
            (
                "challenges",
                &[
                    "product", "head", "name", "target", "seq", "bond", "status", "reason",
                ][..],
            ),
        ] {
            let got = keys(&doc[list][0]);
            for k in wanted {
                assert!(got.contains(*k), "{list}.{k}");
            }
        }
        // Every field of the API's account document, but `rewards` (an object here, with its
        // claim command; the plugin reads either).
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../api/api/fixtures/oracle-heads.json"
        ))
        .unwrap();
        let fixture: Value = serde_json::from_str(&text).unwrap();
        let account = &fixture["channels"]["account"];
        for list in ["delegations", "claims", "challenges"] {
            let missing: Vec<String> = keys(&account[list][0])
                .difference(&keys(&doc[list][0]))
                .cloned()
                .collect();
            assert!(missing.is_empty(), "{list} lacks {missing:?}");
        }
        assert!(keys(&account["stake"]).is_subset(&keys(&doc["stake"])));
        if let Ok(dir) = std::env::var("PETRI_HEADS_DOCS_OUT") {
            std::fs::write(
                format!("{dir}/mine.json"),
                serde_json::to_vec_pretty(&doc).unwrap(),
            )
            .unwrap();
        }
    }

    /// A contract's listing terms built from chain are the API's document, byte for byte.
    #[test]
    fn terms_are_the_apis_document() {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../api/api/fixtures/oracle-heads.json"
        ))
        .unwrap();
        let fixture: Value = serde_json::from_str(&text).unwrap();
        let want = &fixture["channels"]["terms"];
        let mold = Product::Ramx.mold();
        let mut k = zeroed_box::<ameba_oracle::state::heads::HeadsContract>();
        k.period = want["period"].as_str().unwrap().parse().unwrap();
        k.product_id = crate::oracle::skus::encode_label("RAMX").unwrap();
        k.expiry_ts = want["expiry"].as_u64().unwrap();
        k.terms_hash = molds::listing_terms_sha256(Product::Ramx, k.expiry_ts as i64);
        k.item_count = mold.items.len() as u8;
        for (i, item) in mold.items.iter().enumerate() {
            k.item_bucket_ids[i] = crate::oracle::skus::encode_label(item.bucket.id).unwrap();
            for c in &item.components {
                let at = usize::from(c.index);
                k.component_items[at] = i as u8;
                k.component_channels[at] = c.channel.code();
                k.component_sentence_hashes[at] = c.sentence_sha256;
            }
        }
        k.component_count = (mold.items.len() * 3) as u8;
        k.settlement_target = match mold.settlement_target.code() {
            0 => ameba_oracle::state::heads::HeadsSettlementTarget::ContractChannel,
            _ => ameba_oracle::state::heads::HeadsSettlementTarget::FixedMixReference,
        };
        let got = terms_doc(&k).unwrap();
        // The document is the contract's quarter's: its id and the mix it lists with.
        let listed: Value = serde_json::from_str(
            got["document"]
                .as_str()
                .unwrap()
                .split_once('\n')
                .unwrap()
                .1,
        )
        .unwrap();
        let expiry = k.expiry_ts as i64;
        assert_eq!(listed["quarter"], molds::Quarter::of_expiry(expiry).id());
        assert_eq!(
            listed["mixCommitment"],
            hex(&molds::listing_mix_commitment(Product::Ramx, expiry))
        );
        let mixes: Vec<Value> = molds::listing_mixes(Product::Ramx, expiry)
            .iter()
            .map(|(mix, _)| json!(mix))
            .collect();
        let got_mixes: Vec<Value> = listed["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["mixBps"].clone())
            .collect();
        assert_eq!(got_mixes, mixes);
        if want["moldsSha256"] == molds::CHANNEL_MOLDS_SHA256_HEX {
            assert_eq!(&got, want);
        } else {
            // The API's fixture predates these molds: the same document, field for field.
            assert_eq!(keys(&got), keys(want));
            assert_eq!(
                got["sentences"].as_array().map(Vec::len),
                want["sentences"].as_array().map(Vec::len)
            );
            assert_eq!(got["matches"], true);
        }
    }

    #[test]
    fn an_owner_with_nothing_gets_the_empty_document() {
        let mut m = mine();
        m.stake = None;
        m.delegations.clear();
        m.claims.clear();
        m.challenges.clear();
        let doc = mine_json(&m);
        assert_eq!(doc["stake"]["free"], "0");
        assert_eq!(doc["rewards"], json!({ "claimable": "0" }));
        for list in ["delegations", "claims", "challenges"] {
            assert_eq!(doc[list], json!([]));
        }
    }
}
