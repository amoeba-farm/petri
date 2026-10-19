#![recursion_limit = "512"]
//! Emits the golden vectors that `test/futures-*.test.mjs` and the Rust
//! `rust/tests/vectors.rs` check the SDK against (`futures.json`): PDAs (with
//! the oracle `Period` of each product and expiry), every `Market`
//! instruction, a synthetic market account written by the program's own state
//! types with the fields, arcs and values the client reads from it, solved
//! proposals, tiers, payoffs, error names, the Earn vault (PDAs, every
//! `EarnVault` instruction, an operated order, vault and position accounts)
//! and the legacy pools' exit (`PoolWithdraw`, `PoolMigrate`). Everything
//! comes from the futures client crate (`ameba_futures_client`), the futures
//! program's state types and the oracle interface (`ameba_oracle_interface`);
//! nothing is re-derived by hand.
use std::{fs, path::PathBuf};

use ameba_futures_client::{
    self as client,
    governance::{gate_address, GateBinding, LOCALNET_CONTROLLER_PROGRAM_ID},
    payoff, state, Arc, ArcId, Book, Claim, Core, CreateArgs, EarnVault, Header, Knot, Ladder,
    Maker, Market, Order, Proposal, Side, Step, SubmitArgs, Taker, Tiers, Trader,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde_json::{json, Value};
use solana_program::{instruction::Instruction, pubkey::Pubkey};

fn key(n: u32) -> Pubkey {
    Pubkey::new_from_array(std::array::from_fn(|i| {
        (n.wrapping_mul(2_654_435_761).wrapping_add((i as u32) * 40_503) >> 7) as u8
    }))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|v| format!("{v:02x}")).collect()
}

fn s<T: ToString>(v: T) -> Value {
    Value::String(v.to_string())
}

/// The ASCII symbol zero-padded to 32 bytes, as the programs store it.
fn product_id(symbol: &str) -> [u8; 32] {
    let mut id = [0u8; 32];
    id[..symbol.len()].copy_from_slice(symbol.as_bytes());
    id
}

fn ix_json(name: &str, method: &str, args: Value, ix: &Instruction) -> Value {
    json!({
        "name": name,
        "method": method,
        "args": args,
        "programId": ix.program_id.to_string(),
        "data": hex(&ix.data),
        "accounts": ix.accounts.iter().map(|m| json!({
            "pubkey": m.pubkey.to_string(),
            "isSigner": m.is_signer,
            "isWritable": m.is_writable,
        })).collect::<Vec<_>>(),
    })
}

const EXPIRY: u64 = 1_793_491_200; // 2026-11-01T00:00:00Z

const GRID: u16 = 12;
const K: u64 = 10_000;

fn program() -> Pubkey {
    key(80)
}

fn market() -> Market {
    Market::derive(
        program(),
        key(81),
        key(82),
        key(83),
        key(84),
        &product_id("RAMX"),
        EXPIRY,
    )
}

fn market_json(m: &Market) -> Value {
    json!({
        "program": m.program.to_string(),
        "market": m.market.to_string(),
        "registry": m.registry.to_string(),
        "authority": m.authority.to_string(),
        "vault": m.vault.to_string(),
        "period": m.period.to_string(),
        "mint": m.mint.to_string(),
    })
}

fn side_name(side: Side) -> &'static str {
    match side {
        Side::Long => "long",
        Side::Short => "short",
    }
}

// ------------------------------------------------------------------ PDAs
fn pdas() -> Value {
    let p = program();
    let oracle = key(3);
    let mut cases = Vec::new();
    for symbol in ["RAMX", "NANDX", "ABCDEFGHIJKLMNOPQRSTUVWXYZ012345"] {
        for expiry in [EXPIRY, 1_796_083_200u64, 0, u64::MAX] {
            let product = product_id(symbol);
            let (registry, registry_bump) = state::registry_address(&p, &product, expiry);
            let (period, period_bump) =
                ameba_oracle_interface::find_period_address(&oracle, &product, expiry);
            cases.push(json!({
                "symbol": symbol,
                "productId": hex(&product),
                "expiryTs": s(expiry),
                "registry": registry.to_string(),
                "registryBump": registry_bump,
                "period": period.to_string(),
                "periodBump": period_bump,
            }));
        }
    }
    let markets: Vec<Value> = [key(81), key(90), key(91)]
        .iter()
        .map(|m| {
            let (authority, bump) = state::authority_address(&p, m);
            assert_eq!(authority, Market::authority_of(&p, m));
            json!({ "market": m.to_string(), "authority": authority.to_string(), "authorityBump": bump })
        })
        .collect();
    let pools: Vec<Value> = [(key(81), key(86)), (key(90), key(30))]
        .iter()
        .map(|(m, operator)| {
            let (pool, pool_bump) = state::pool_address(&p, m, operator);
            let lps: Vec<Value> = [key(30), key(31)]
                .iter()
                .map(|o| {
                    let (lp, bump) = state::lp_address(&p, &pool, o);
                    json!({ "owner": o.to_string(), "lp": lp.to_string(), "lpBump": bump })
                })
                .collect();
            json!({ "market": m.to_string(), "operator": operator.to_string(), "pool": pool.to_string(),
                    "poolBump": pool_bump, "positions": lps })
        })
        .collect();
    let vaults: Vec<Value> = [key(86), key(30)]
        .iter()
        .map(|operator| {
            let (vault, vault_bump) = state::vault_address(&p, operator);
            assert_eq!(vault, EarnVault::address(&p, operator));
            let positions: Vec<Value> = [key(30), key(31)]
                .iter()
                .map(|o| {
                    let (position, bump) = state::vault_lp_address(&p, &vault, o);
                    json!({ "owner": o.to_string(), "position": position.to_string(), "positionBump": bump })
                })
                .collect();
            json!({ "operator": operator.to_string(), "vault": vault.to_string(), "vaultBump": vault_bump,
                    "positions": positions })
        })
        .collect();
    json!({ "program": p.to_string(), "oracle": oracle.to_string(), "registries": cases, "authorities": markets,
            "pools": pools, "vaults": vaults })
}

// ------------------------------------------------------------------ Instructions
fn instructions() -> Value {
    let m = market();
    let (owner, token, to, creator, oracle) = (key(30), key(32), key(33), key(36), key(3));
    let mut v = Vec::new();

    let create = CreateArgs {
        product_id: product_id("RAMX"),
        expiry_ts: EXPIRY,
        grid: 200,
        k: K,
        step_value: 1_000_000,
        value_decimals: 6,
        ref_value: 100_000_000,
        trader_cap: 512,
        order_cap: 256,
        maker_cap: 4,
        claim_cap: 4_096,
    };
    v.push(ix_json(
        "createMarket",
        "create",
        json!({
            "creator": creator.to_string(),
            "oracleProgram": oracle.to_string(),
            "params": {
                "symbol": "RAMX",
                "expiryTs": s(create.expiry_ts),
                "grid": create.grid,
                "k": s(create.k),
                "stepValue": s(create.step_value),
                "valueDecimals": create.value_decimals,
                "refValue": s(create.ref_value),
                "traderCap": create.trader_cap,
                "orderCap": create.order_cap,
                "makerCap": create.maker_cap,
                "claimCap": create.claim_cap,
            },
            "space": create.space(),
        }),
        &m.create(&creator, &oracle, &create),
    ));
    v.push(ix_json(
        "deposit",
        "deposit",
        json!({ "owner": owner.to_string(), "token": token.to_string(), "amount": s(25_000_000u64) }),
        &m.deposit(&owner, &token, 25_000_000),
    ));
    v.push(ix_json(
        "withdraw",
        "withdraw",
        json!({ "owner": owner.to_string(), "token": token.to_string(), "amount": s(7u64) }),
        &m.withdraw(&owner, &token, 7),
    ));
    v.push(ix_json(
        "withdrawAll",
        "withdraw",
        json!({ "owner": owner.to_string(), "token": token.to_string(), "amount": s(u64::MAX) }),
        &m.withdraw(&owner, &token, u64::MAX),
    ));
    let proposal = Proposal {
        fills: vec![
            (ArcId::Order(2), 3),
            (
                ArcId::Maker {
                    maker: 0,
                    step: 98,
                    side: Side::Short,
                    tier: 0,
                },
                3,
            ),
        ],
        taker_fill: 3,
        potentials: vec![0, -12_000, 5_000],
    }
    .encode();
    for (name, a) in [
        (
            "submitIoc",
            SubmitArgs {
                side: Side::Long,
                lo: 98,
                hi: 120,
                lots: 3,
                limit: 220_000,
                min_fill: 1,
                mode: client::MODE_IOC,
                close: None,
            },
        ),
        (
            "submitPost",
            SubmitArgs {
                side: Side::Short,
                lo: 0,
                hi: 200,
                lots: 1 << 40,
                limit: 0,
                min_fill: 0,
                mode: client::MODE_POST,
                close: None,
            },
        ),
        (
            "submitClose",
            SubmitArgs {
                side: Side::Short,
                lo: 90,
                hi: 110,
                lots: 2,
                limit: 0,
                min_fill: 1,
                mode: client::MODE_IOC,
                close: Some((4, 17)),
            },
        ),
    ] {
        v.push(ix_json(
            name,
            "submit",
            json!({
                "owner": owner.to_string(),
                "side": side_name(a.side),
                "lo": a.lo,
                "hi": a.hi,
                "lots": s(a.lots),
                "limit": s(a.limit),
                "minFill": s(a.min_fill),
                "mode": a.mode,
                "close": a.close.map(|(slot, seq)| json!({ "slot": slot, "seq": s(seq) })),
                "proposal": hex(&proposal),
            }),
            &m.submit(&owner, &a, proposal.clone()),
        ));
    }
    v.push(ix_json(
        "cancel",
        "cancel",
        json!({ "owner": owner.to_string(), "slot": 65_534, "seq": s(u64::MAX) }),
        &m.cancel(&owner, 65_534, u64::MAX),
    ));
    let ladder = Ladder {
        lo: 2,
        hi: 198,
        bid_spread: 300,
        ask_spread: 400,
        bid_lots: 10_000,
        ask_lots: 8_000,
        keep: true,
        tiers: Tiers {
            count: 4,
            ratio: 150,
            spread: 250,
        },
    };
    let knots = vec![
        Knot {
            pos: 2,
            price: 9_900,
        },
        Knot {
            pos: 100,
            price: 5_000,
        },
        Knot {
            pos: 198,
            price: u32::MAX,
        },
    ];
    let ladder_json = json!({
        "lo": ladder.lo, "hi": ladder.hi, "bidSpread": ladder.bid_spread, "askSpread": ladder.ask_spread,
        "bidLots": s(ladder.bid_lots), "askLots": s(ladder.ask_lots), "keep": ladder.keep,
        "tiers": { "count": ladder.tiers.count, "ratio": ladder.tiers.ratio, "spread": ladder.tiers.spread },
    });
    let knots_json: Vec<Value> = knots
        .iter()
        .map(|k| json!({ "pos": k.pos, "price": k.price }))
        .collect();
    let quote_proposal = Proposal {
        fills: vec![],
        taker_fill: 0,
        potentials: vec![0, 7, -7],
    }
    .encode();
    v.push(ix_json(
        "quote",
        "quote",
        json!({ "owner": owner.to_string(), "ladder": ladder_json, "knots": knots_json, "proposal": hex(&quote_proposal) }),
        &m.quote(&owner, &ladder, knots.clone(), quote_proposal.clone()),
    ));
    let core = Core { lo: 80, hi: 120 };
    v.push(ix_json(
        "quoteCore",
        "quoteCore",
        json!({ "owner": owner.to_string(), "ladder": ladder_json, "core": { "lo": core.lo, "hi": core.hi },
                "knots": knots_json, "proposal": hex(&quote_proposal) }),
        &m.quote_core(&owner, &ladder, core, knots, quote_proposal),
    ));
    v.push(ix_json(
        "cancelQuote",
        "cancelQuote",
        json!({ "owner": owner.to_string() }),
        &m.cancel_quote(&owner),
    ));
    v.push(ix_json(
        "split",
        "split",
        json!({ "owner": owner.to_string(), "claim": { "slot": 9, "seq": s(31u64) }, "at": 105 }),
        &m.split(&owner, (9, 31), 105),
    ));
    v.push(ix_json(
        "merge",
        "merge",
        json!({ "owner": owner.to_string(), "keep": { "slot": 1, "seq": s(2u64) }, "absorb": { "slot": u32::MAX - 1, "seq": s(4u64) } }),
        &m.merge(&owner, (1, 2), (u32::MAX - 1, 4)),
    ));
    v.push(ix_json(
        "redeem",
        "redeem",
        json!({ "owner": owner.to_string(), "long": { "slot": 5, "seq": s(6u64) }, "short": { "slot": 7, "seq": s(8u64) }, "lots": s(9u64) }),
        &m.redeem(&owner, (5, 6), (7, 8), 9),
    ));
    v.push(ix_json(
        "transfer",
        "transfer",
        json!({ "owner": owner.to_string(), "claim": { "slot": 3, "seq": s(12u64) }, "lots": s(2u64), "to": to.to_string() }),
        &m.transfer(&owner, (3, 12), 2, &to),
    ));
    v.push(ix_json("finalize", "finalize", json!({}), &m.finalize()));
    v.push(ix_json(
        "settle",
        "settle",
        json!({ "claims": [0, 7, u32::MAX] }),
        &m.settle(vec![0, 7, u32::MAX]),
    ));
    v.push(ix_json(
        "settleEmpty",
        "settle",
        json!({ "claims": [] }),
        &m.settle(vec![]),
    ));
    v.push(ix_json(
        "settleMaker",
        "settleMaker",
        json!({ "maker": 3 }),
        &m.settle_maker(3),
    ));
    v.push(ix_json(
        "prune",
        "prune",
        json!({ "orders": [1, 2, 65_535] }),
        &m.prune(vec![1, 2, 65_535]),
    ));
    v.push(ix_json(
        "heapFrame",
        "heapFrame",
        json!({ "bytes": client::HEAP_FRAME }),
        &client::heap_frame(client::HEAP_FRAME),
    ));
    v.push(ix_json(
        "computeLimit",
        "computeLimit",
        json!({ "units": 1_400_000 }),
        &client::compute_limit(1_400_000),
    ));
    // Legacy Earn pools (dev builds): their exit, and the operator acting for one.
    let (operator, payer) = (key(86), key(87));
    let pool = m.pool(&operator);
    v.push(ix_json(
        "poolWithdraw",
        "poolWithdraw",
        json!({ "pool": pool.to_string(), "owner": owner.to_string(), "token": token.to_string(), "shares": s(123u64) }),
        &m.pool_withdraw(&pool, &owner, &token, 123),
    ));
    v.push(ix_json(
        "poolWithdrawAll",
        "poolWithdraw",
        json!({ "pool": pool.to_string(), "owner": owner.to_string(), "token": token.to_string(), "shares": s(u64::MAX) }),
        &m.pool_withdraw(&pool, &owner, &token, u64::MAX),
    ));
    v.push(ix_json(
        "operatedCancel",
        "operated",
        json!({ "pool": pool.to_string(), "operator": operator.to_string(), "slot": 3, "seq": s(44u64) }),
        &client::operated(m.cancel(&pool, 3, 44), &operator),
    ));
    // The Earn vault: in `m`, a second market and a third; withdrawals draw
    // from the second, then `m` (pairs in that order).
    let earn = EarnVault::new(program(), operator, key(89), m.mint);
    let other = Market::derive(
        program(),
        key(90),
        key(92),
        key(93),
        m.mint,
        &product_id("NANDX"),
        EXPIRY,
    );
    let markets = [m.market, other.market, key(91)];
    let listed: Vec<String> = markets.iter().map(|k| k.to_string()).collect();
    v.push(ix_json(
        "createVault",
        "createVault",
        json!({ "payer": payer.to_string() }),
        &earn.create(&payer),
    ));
    v.push(ix_json(
        "vaultDeposit",
        "vaultDeposit",
        json!({ "owner": owner.to_string(), "token": token.to_string(), "amount": s(5_000_000u64),
                "payer": payer.to_string(), "markets": listed }),
        &earn.deposit(&owner, &token, 5_000_000, &payer, &markets),
    ));
    v.push(ix_json(
        "vaultWithdraw",
        "vaultWithdraw",
        json!({ "owner": owner.to_string(), "token": token.to_string(), "shares": s(123u64), "markets": listed,
                "draws": [market_json(&other), market_json(&m)] }),
        &earn.withdraw(&owner, &token, 123, &markets, &[other, m]),
    ));
    v.push(ix_json(
        "vaultWithdrawAll",
        "vaultWithdraw",
        json!({ "owner": owner.to_string(), "token": token.to_string(), "shares": s(u64::MAX), "markets": listed,
                "draws": [] }),
        &earn.withdraw(&owner, &token, u64::MAX, &markets, &[]),
    ));
    v.push(ix_json(
        "vaultFund",
        "vaultFund",
        json!({ "amount": s(2_500_000u64) }),
        &earn.fund(&m, 2_500_000),
    ));
    v.push(ix_json(
        "vaultDefund",
        "vaultDefund",
        json!({ "amount": s(1_000_000u64) }),
        &earn.defund(&m, 1_000_000),
    ));
    v.push(ix_json(
        "vaultDefundAll",
        "vaultDefund",
        json!({ "amount": s(u64::MAX) }),
        &earn.defund(&m, u64::MAX),
    ));
    v.push(ix_json(
        "poolMigrate",
        "poolMigrate",
        json!({ "pool": pool.to_string(), "holder": owner.to_string(), "payer": payer.to_string(), "markets": listed }),
        &m.pool_migrate(&pool, &owner, &earn, &payer, &markets),
    ));
    v.push(ix_json(
        "operatedVaultCancel",
        "operatedVault",
        json!({ "vault": earn.vault.to_string(), "operator": operator.to_string(), "slot": 3, "seq": s(44u64) }),
        &client::operated(m.cancel(&earn.vault, 3, 44), &operator),
    ));

    // A futures instruction bound to the controller's gate (every futures
    // instruction carries the governance envelope on a governed deployment).
    let controller = LOCALNET_CONTROLLER_PROGRAM_ID;
    let (gate, bump) = gate_address(&controller, &program());
    let mut gate_data = vec![0u8; 192];
    gate_data[..8].copy_from_slice(b"AGVGAT01");
    gate_data[8] = 1;
    gate_data[9] = bump;
    gate_data[10] = 1;
    gate_data[12..44].copy_from_slice(
        Pubkey::find_program_address(&[b"ameba-governance-v3", b"council"], &controller)
            .0
            .as_ref(),
    );
    gate_data[44..76].copy_from_slice(program().as_ref());
    gate_data[76..108].copy_from_slice(
        Pubkey::find_program_address(
            &[program().as_ref()],
            &solana_program::bpf_loader_upgradeable::ID,
        )
        .0
        .as_ref(),
    );
    gate_data[108..116].copy_from_slice(&11u64.to_le_bytes());
    let binding =
        GateBinding::from_account(controller, program(), gate, controller, &gate_data).unwrap();
    let raw = m.cancel(&owner, 4, 17);
    let bound = binding.bind(raw.clone()).unwrap();

    json!({
        "market": market_json(&m),
        "productSymbol": "RAMX",
        "expiryTs": s(EXPIRY),
        "heapFrame": client::HEAP_FRAME,
        "earnVault": {
            "program": earn.program.to_string(),
            "vault": earn.vault.to_string(),
            "operator": earn.operator.to_string(),
            "cash": earn.cash.to_string(),
            "mint": earn.mint.to_string(),
        },
        "cases": v,
        "governance": {
            "controller": controller.to_string(),
            "gate": gate.to_string(),
            "gateData": hex(&gate_data),
            "raw": ix_json("raw", "cancel", json!({}), &raw),
            "bound": ix_json("bound", "cancel", json!({}), &bound),
        },
    })
}

// ------------------------------------------------------------------ Accounts
fn header(status: u8, settlement_fine: u64) -> Header {
    Header {
        status,
        authority_bump: 254,
        value_decimals: 6,
        grid: GRID,
        trader_cap: 4,
        order_cap: 4,
        maker_cap: 2,
        registry_bump: 253,
        claim_cap: 6,
        claim_hint: 3,
        product_id: product_id("RAMX"),
        expiry_ts: EXPIRY,
        period: key(83),
        oracle_program: key(3),
        quote_mint: key(84),
        vault: key(82),
        creator: key(85),
        k: K,
        step_value: 1_000_000,
        ref_fine: 60_000,
        settlement_fine,
        next_seq: 40,
        total_free: 1_234_567,
        total_reserved: 2_345_678,
        backing: 3_456_789,
        custody: 7_037_034,
        trades: 17,
        volume_lots: 1 << 40,
        volume_premium: (1u128 << 70) + 12_345,
        last_entry_fine: 61_234,
        claims_open: 4,
        total_paid_out: 77,
        settled_ts: if status == state::STATUS_OPEN {
            0
        } else {
            1_793_500_000
        },
        last_trade_ts: 1_792_047_367,
        // No stake-backed credit: those header bytes stay zero.
        ..Header::default()
    }
}

fn account(status: u8, settlement_fine: u64) -> (Header, Vec<u8>) {
    let h = header(status, settlement_fine);
    let mut b =
        vec![0u8; state::market_len(h.grid, h.trader_cap, h.order_cap, h.maker_cap, h.claim_cap)];
    h.write(&mut b);
    let traders = [
        Some(Trader {
            owner: key(30),
            free: 5_000_000,
            reserved: 1_200_000,
            claims: 2,
            orders: 0,
            maker: 1,
            paid_out: 0,
        }),
        Some(Trader {
            owner: key(31),
            free: 100,
            reserved: 100_000,
            claims: 1,
            orders: 2,
            maker: 0,
            paid_out: 9,
        }),
        None,
        Some(Trader {
            owner: key(32),
            free: 1 << 50,
            reserved: 0,
            claims: 1,
            orders: 1,
            maker: 2,
            paid_out: 0,
        }),
    ];
    for (i, t) in traders.iter().enumerate() {
        if let Some(t) = t {
            t.write(&mut b, h.trader_off(i as u16));
        }
    }
    let orders = [
        Some(Order {
            seq: 5,
            trader: 1,
            side: 0,
            flags: 0,
            lo: 3,
            hi: 7,
            lots: 4,
            price: 25_000,
            escrow_per_lot: 25_000,
            claim: client::NO_CLAIM,
            placed_lots: 6,
            placed_ts: 1_792_000_000,
        }),
        None,
        Some(Order {
            seq: 9,
            trader: 3,
            side: 1,
            flags: client::ORDER_CLOSE,
            lo: 2,
            hi: 5,
            lots: 2,
            price: 22_000,
            escrow_per_lot: 0,
            claim: 1,
            placed_lots: 2,
            placed_ts: 1_792_000_100,
        }),
        Some(Order {
            seq: 11,
            trader: 1,
            side: 1,
            flags: 0,
            lo: 8,
            hi: 12,
            lots: 0,
            price: 30_000,
            escrow_per_lot: 10_000,
            claim: client::NO_CLAIM,
            placed_lots: 5,
            placed_ts: 1_792_000_200,
        }),
    ];
    for (i, o) in orders.iter().enumerate() {
        if let Some(o) = o {
            o.write(&mut b, h.order_off(i as u16));
        }
    }
    Maker {
        seq: 3,
        trader: 0,
        used: true,
        lo: 2,
        hi: 10,
        escrow: 999_999,
        bid_spread: 300,
        ask_spread: 400,
        quoted_ts: 1_792_000_300,
        tiers: Tiers {
            count: 3,
            ratio: 50,
            spread: 500,
        },
        bid_total: 10,
        ask_total: 8,
    }
    .write(&mut b, h.maker_off(0));
    for x in 0..GRID {
        let mut st = Step::default();
        if (2..10).contains(&x) {
            let mid = 9_000 - 700 * u32::from(x);
            st.ask = mid + 400;
            st.bid = mid - 300;
            st.bid_lots = match x {
                3 => 6,
                5 => 0,
                _ => 10,
            };
            st.ask_lots = match x {
                4 => 3,
                8 => 0,
                _ => 8,
            };
        }
        st.inv = match x {
            0 => 5,
            3 => 3,
            6 => -2,
            11 => -1,
            _ => 0,
        };
        st.write(&mut b, h.step_off(0, x));
    }
    // A cancelled ladder (seq 0) that still holds inventory.
    Maker {
        seq: 0,
        trader: 3,
        used: true,
        ..Maker::default()
    }
    .write(&mut b, h.maker_off(1));
    for (x, inv) in [(4u16, 2i64), (5, -3)] {
        Step {
            inv,
            ..Step::default()
        }
        .write(&mut b, h.step_off(1, x));
    }
    let claims = [
        Some(Claim {
            lots: 3,
            seq: 12,
            trader: 0,
            side: 0,
            lo: 4,
            hi: 9,
            cost: 45_000,
            reserved: 0,
            opened_ts: 1_791_900_000,
        }),
        Some(Claim {
            lots: 2,
            seq: 13,
            trader: 3,
            side: 0,
            lo: 2,
            hi: 5,
            cost: 30_000,
            reserved: 2,
            opened_ts: 1_791_900_100,
        }),
        None,
        Some(Claim {
            lots: 7,
            seq: 20,
            trader: 1,
            side: 1,
            lo: 0,
            hi: 12,
            cost: 500_000,
            reserved: 0,
            opened_ts: 1_791_900_200,
        }),
        Some(Claim {
            lots: 1,
            seq: 21,
            trader: 0,
            side: 1,
            lo: 6,
            hi: 7,
            cost: 4_000,
            reserved: 0,
            opened_ts: 1_791_900_300,
        }),
        None,
    ];
    for (i, c) in claims.iter().enumerate() {
        if let Some(c) = c {
            c.write(&mut b, h.claim_off(i as u32));
        }
    }
    (h, b)
}

fn header_json(h: &Header) -> Value {
    json!({
        "status": h.status,
        "authorityBump": h.authority_bump,
        "valueDecimals": h.value_decimals,
        "grid": h.grid,
        "traderCap": h.trader_cap,
        "orderCap": h.order_cap,
        "makerCap": h.maker_cap,
        "registryBump": h.registry_bump,
        "claimCap": h.claim_cap,
        "claimHint": h.claim_hint,
        "productId": hex(&h.product_id),
        "expiryTs": s(h.expiry_ts),
        "period": h.period.to_string(),
        "oracleProgram": h.oracle_program.to_string(),
        "quoteMint": h.quote_mint.to_string(),
        "vault": h.vault.to_string(),
        "creator": h.creator.to_string(),
        "k": s(h.k),
        "stepValue": s(h.step_value),
        "refFine": s(h.ref_fine),
        "settlementFine": s(h.settlement_fine),
        "nextSeq": s(h.next_seq),
        "totalFree": s(h.total_free),
        "totalReserved": s(h.total_reserved),
        "backing": s(h.backing),
        "custody": s(h.custody),
        "trades": s(h.trades),
        "volumeLots": s(h.volume_lots),
        "volumePremium": s(h.volume_premium),
        "lastEntryFine": s(h.last_entry_fine),
        "claimsOpen": s(h.claims_open),
        "totalPaidOut": s(h.total_paid_out),
        "settledTs": s(h.settled_ts),
        "lastTradeTs": s(h.last_trade_ts),
    })
}

fn trader_json(slot: u16, t: &Trader) -> Value {
    json!({
        "slot": slot, "owner": t.owner.to_string(), "free": s(t.free), "reserved": s(t.reserved),
        "claims": t.claims, "orders": t.orders, "maker": t.maker, "paidOut": s(t.paid_out),
    })
}

fn order_json(slot: u16, o: &Order) -> Value {
    json!({
        "slot": slot, "seq": s(o.seq), "trader": o.trader, "side": o.side, "flags": o.flags, "lo": o.lo, "hi": o.hi,
        "lots": s(o.lots), "price": s(o.price), "escrowPerLot": s(o.escrow_per_lot), "claim": o.claim,
        "placedLots": s(o.placed_lots), "placedTs": s(o.placed_ts),
    })
}

fn claim_json(slot: u32, c: &Claim) -> Value {
    json!({
        "slot": slot, "lots": s(c.lots), "seq": s(c.seq), "trader": c.trader, "side": c.side, "lo": c.lo, "hi": c.hi,
        "cost": s(c.cost), "reserved": s(c.reserved), "openedTs": s(c.opened_ts),
    })
}

fn maker_json(slot: u8, m: &Maker) -> Value {
    json!({
        "slot": slot, "seq": s(m.seq), "trader": m.trader, "used": m.used, "lo": m.lo, "hi": m.hi,
        "escrow": s(m.escrow), "bidSpread": m.bid_spread, "askSpread": m.ask_spread, "quotedTs": s(m.quoted_ts),
        "tiers": { "count": m.tiers.count, "ratio": m.tiers.ratio, "spread": m.tiers.spread },
        "bidTotal": s(m.bid_total), "askTotal": s(m.ask_total),
    })
}

fn step_json(pos: u16, st: &Step) -> Value {
    json!({ "pos": pos, "ask": st.ask, "bid": st.bid, "askLots": s(st.ask_lots), "bidLots": s(st.bid_lots), "inv": s(st.inv) })
}

fn arc_id_json(id: &ArcId) -> Value {
    match *id {
        ArcId::Order(slot) => json!({ "kind": "order", "slot": slot }),
        ArcId::Maker {
            maker,
            step,
            side,
            tier,
        } => {
            json!({ "kind": "maker", "maker": maker, "step": step, "side": side as u8, "tier": tier })
        }
    }
}

fn arc_json(a: &Arc) -> Value {
    json!({
        "id": arc_id_json(&a.id), "side": a.side as u8, "lo": a.lo, "hi": a.hi, "price": s(a.price),
        "cap": s(a.cap), "rank": s(a.rank),
    })
}

/// A final market: its header and what each trader holds is worth at the
/// settlement (the records are the open market's).
fn final_json(h: &Header, b: &[u8]) -> Value {
    let full = book_json(h, b);
    json!({
        "data": full["data"],
        "header": full["header"],
        "traderValues": full["traderValues"],
        "owners": full["book"]["owners"],
    })
}

fn book_json(h: &Header, b: &[u8]) -> Value {
    let book = Book::decode(b).unwrap();
    assert_eq!(book.header, *h);
    let all_traders: Vec<Value> = (0..h.trader_cap)
        .map(|i| trader_json(i, &Trader::read(b, h.trader_off(i))))
        .collect();
    let all_orders: Vec<Value> = (0..h.order_cap)
        .map(|i| order_json(i, &Order::read(b, h.order_off(i))))
        .collect();
    let all_claims: Vec<Value> = (0..h.claim_cap)
        .map(|i| claim_json(i, &Claim::read(b, h.claim_off(i))))
        .collect();
    let all_makers: Vec<Value> = (0..h.maker_cap)
        .map(|m| {
            let mut v = maker_json(m, &Maker::read(b, h.maker_off(m)));
            v["steps"] = (0..h.grid)
                .map(|x| step_json(x, &Step::read(b, h.step_off(m, x))))
                .collect();
            v
        })
        .collect();
    let values: Vec<Value> = (0..h.trader_cap)
        .map(|t| {
            json!({ "slot": t, "value": state::trader_value(b, h, t).map(|(lo, hi)| json!([s(lo), s(hi)])) })
        })
        .collect();
    let owners: Vec<Value> = [key(30), key(31), key(32), key(99)]
        .iter()
        .map(|o| {
            let (lo, hi) = book.value_of(o);
            json!({ "owner": o.to_string(), "trader": book.trader_of(o).map(|(i, _)| i), "value": [s(lo), s(hi)] })
        })
        .collect();
    json!({
        "data": B64.encode(b),
        "header": header_json(h),
        "traders": all_traders,
        "orders": all_orders,
        "makers": all_makers,
        "claims": all_claims,
        "book": {
            "traders": book.traders().iter().map(|(i, _)| *i).collect::<Vec<_>>(),
            "orders": book.orders().iter().map(|(i, _)| *i).collect::<Vec<_>>(),
            "makers": book.makers().iter().map(|(m, _, steps)| json!({
                "slot": m, "steps": steps.iter().map(|(x, _)| *x).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "claims": book.claims().iter().map(|(i, _)| *i).collect::<Vec<_>>(),
            "claimsOf0": book.claims_of(0).iter().map(|(i, _)| *i).collect::<Vec<_>>(),
            "owners": owners,
        },
        "arcs": book.arcs().iter().map(arc_json).collect::<Vec<_>>(),
        "ladderArcs": state::ladder_arcs(b, h),
        "traderValues": values,
    })
}

fn proposals(b: &[u8]) -> Value {
    let book = Book::decode(b).unwrap();
    let takers = [
        Taker {
            side: Side::Long,
            lo: 3,
            hi: 7,
            limit: 40_000,
            qty: 5,
        },
        Taker {
            side: Side::Short,
            lo: 2,
            hi: 5,
            limit: 0,
            qty: 3,
        },
        Taker {
            side: Side::Long,
            lo: 6,
            hi: 7,
            limit: 10_000,
            qty: 20,
        },
        Taker {
            side: Side::Long,
            lo: 0,
            hi: 12,
            limit: 1,
            qty: 1,
        },
    ];
    takers
        .iter()
        .map(|t| {
            let p = book.propose(t);
            let decoded = Proposal::decode(&p.proposal).unwrap();
            json!({
                "taker": { "side": t.side as u8, "lo": t.lo, "hi": t.hi, "limit": s(t.limit), "qty": s(t.qty) },
                "fills": p.solution.fills.iter().map(|f| s(*f)).collect::<Vec<_>>(),
                "takerFill": s(p.solution.taker_fill),
                "potentials": p.solution.potentials.iter().map(|v| s(*v)).collect::<Vec<_>>(),
                "premium": s(p.solution.premium),
                "proposal": hex(&p.proposal),
                "decoded": decoded.fills.iter().map(|(id, m)| json!({ "id": arc_id_json(id), "lots": s(*m) })).collect::<Vec<_>>(),
            })
        })
        .collect()
}

fn codec() -> Value {
    let mk = |maker, step, side, tier| ArcId::Maker {
        maker,
        step,
        side,
        tier,
    };
    let mut fills = vec![
        (ArcId::Order(0), 1),
        (ArcId::Order(3), 7),
        (ArcId::Order(9), 1 << 62),
        (mk(0, 10, Side::Long, 0), 5),
        (mk(0, 10, Side::Long, 1), 6),
        (mk(0, 10, Side::Short, 0), 2),
        (mk(0, 11, Side::Long, 0), 5),
        (mk(0, 11, Side::Long, 1), 6),
        (mk(0, 12, Side::Long, 0), 5),
        (mk(2, 4, Side::Short, 7), 9),
        (mk(255, 65_534, Side::Short, 7), u64::MAX),
    ];
    fills.sort();
    let cases: Vec<Proposal> = vec![
        Proposal {
            fills,
            taker_fill: 12,
            potentials: vec![0, -10_000, 25, i64::MIN / 8, 7, i64::MAX, i64::MIN, -1],
        },
        Proposal::default(),
        Proposal {
            fills: vec![(mk(1, 5, Side::Short, 0), 3)],
            taker_fill: 0,
            potentials: vec![],
        },
    ];
    let good: Vec<Value> = cases
        .iter()
        .map(|p| {
            let e = p.encode();
            assert_eq!(Proposal::decode(&e).unwrap(), *p);
            json!({
                "hex": hex(&e),
                "fills": p.fills.iter().map(|(id, m)| json!({ "id": arc_id_json(id), "lots": s(*m) })).collect::<Vec<_>>(),
                "takerFill": s(p.taker_fill),
                "potentials": p.potentials.iter().map(|v| s(*v)).collect::<Vec<_>>(),
            })
        })
        .collect();
    let first = cases[0].encode();
    let mut longer = first.clone();
    longer.push(0);
    // Two order fills at the same slot (second delta 0), a zero-lot fill, a
    // tier past MAX_TIERS, a duplicate maker fill, an over-long varint.
    let bad: Vec<Vec<u8>> = vec![
        first[..first.len() - 1].to_vec(),
        longer,
        vec![2, 1, 1, 0, 1, 0, 0, 0],
        vec![1, 1, 0, 0, 0, 0],
        vec![0, 1, 0, 16, 1, 1, 1, 0, 0],
        vec![0, 2, 0, 0, 1, 2, 1, 0, 0, 1, 2, 1, 0, 0],
        vec![
            0, 0, 0, 1, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x01,
        ],
        vec![],
    ];
    let bad: Vec<Value> = bad
        .iter()
        .map(|b| {
            assert!(Proposal::decode(b).is_err(), "{}", hex(b));
            Value::String(hex(b))
        })
        .collect();
    json!({ "good": good, "bad": bad })
}

fn tiers() -> Value {
    let mut out = Vec::new();
    for count in [0u8, 1, 3, 8] {
        for ratio in [50u16, 120, 300] {
            let t = Tiers {
                count,
                ratio,
                spread: 250,
            };
            for total in [0u64, 7, 10, 7_440] {
                let remaining: Vec<Value> = [0u64, 1, total / 2, total.saturating_sub(1), total, total + 3]
                    .iter()
                    .map(|r| json!({ "remaining": s(r), "lots": t.remaining(total, *r)[..t.n()].iter().map(|v| s(*v)).collect::<Vec<_>>() }))
                    .collect();
                out.push(json!({
                    "count": count, "ratio": ratio, "total": s(total),
                    "sizes": t.sizes(total)[..t.n()].iter().map(|v| s(*v)).collect::<Vec<_>>(),
                    "remaining": remaining,
                    "bidPrices": (0..t.n()).map(|i| s(t.price(9_800, Side::Long, i, K))).collect::<Vec<_>>(),
                    "askPrices": (0..t.n()).map(|i| s(t.price(9_800, Side::Short, i, K))).collect::<Vec<_>>(),
                }));
            }
        }
    }
    Value::Array(out)
}

fn math() -> Value {
    let mut payoffs = Vec::new();
    for (lo, hi) in [(98u16, 120u16), (0, 200), (5, 6), (199, 200)] {
        for sv in [
            0u64,
            1,
            u64::from(lo) * K,
            u64::from(lo) * K + 1,
            1_000_000,
            u64::from(hi) * K - 1,
            u64::from(hi) * K,
            5_000_000,
            u64::MAX,
        ] {
            payoffs.push(json!({
                "lo": lo, "hi": hi, "s": s(sv), "k": s(K),
                "set": s(payoff::set_value(lo, hi, K)),
                "long": s(payoff::long_per_lot(lo, hi, sv, K)),
                "short": s(payoff::short_per_lot(lo, hi, sv, K)),
            }));
        }
    }
    let settle: Vec<Value> = [
        (0u64, 1_000_000u64, K),
        (95_000_000, 1_000_000, K),
        (95_123_456, 1_000_000, K),
        (95_123_456, 0, K),
        (u64::MAX, 1, u64::MAX),
        (100_000_000, 3, 7),
    ]
    .iter()
    .map(|(v, step, k)| json!({ "value": s(v), "stepValue": s(step), "k": s(k), "fine": s(payoff::settle_fine(*v, *step, *k)) }))
    .collect();
    let contributions: Vec<Value> = [
        (Side::Long, 98u16, 120u16, 20_000u64),
        (Side::Short, 98, 120, 20_000),
        (Side::Short, 98, 120, 300_000),
        (Side::Long, 0, 1, 0),
    ]
    .iter()
    .map(|(side, lo, hi, price)| {
        json!({
            "side": *side as u8, "lo": lo, "hi": hi, "price": s(price), "k": s(K),
            "contribution": s(payoff::contribution(*side, *lo, *hi, *price, K)),
        })
    })
    .collect();
    let entries: Vec<Value> = [
        (98u16, 120u16, 1_000_000u64),
        (98, 120, 0),
        (98, 120, 9_999_999),
        (0, 200, 950_000),
    ]
    .iter()
    .map(|(lo, hi, e)| {
        json!({
            "lo": lo, "hi": hi, "entryFine": s(e), "k": s(K),
            "premium": s(client::premium_at(*lo, *hi, *e, K)),
            "entryOf": s(client::entry_of(*lo, client::premium_at(*lo, *hi, *e, K), K)),
        })
    })
    .collect();
    json!({ "payoffs": payoffs, "settleFine": settle, "contributions": contributions, "entries": entries })
}

fn errors() -> Value {
    (client::ERROR_BASE..=client::ERROR_BASE + 34)
        .chain([0, 1, u32::MAX])
        .map(|c| json!({ "code": c, "name": client::error_name(c) }))
        .collect()
}

fn registry() -> Value {
    let r = state::Registry {
        bump: 253,
        market: key(81),
    };
    let mut b = vec![0u8; state::REGISTRY_LEN];
    r.write(&mut b);
    assert_eq!(state::Registry::read(&b).unwrap(), r);
    json!({ "data": B64.encode(&b), "bump": r.bump, "market": r.market.to_string() })
}

fn pool_accounts() -> Value {
    let pool = state::Pool {
        bump: 252,
        market: key(81),
        operator: key(86),
        shares: 219_296_678_349,
        deposited: 219_396_678_349,
        withdrawn: 100_000_000,
        created_ts: 1_792_040_000,
    };
    let mut pb = vec![0u8; state::POOL_LEN];
    pool.write(&mut pb);
    assert_eq!(client::decode_pool(&pb).unwrap(), pool);
    let lp = state::LpPosition {
        bump: 251,
        pool: key(88),
        owner: key(30),
        shares: u64::MAX,
        deposited: 7,
        withdrawn: 0,
    };
    let mut lb = vec![0u8; state::LP_LEN];
    lp.write(&mut lb);
    assert_eq!(client::decode_lp(&lb).unwrap(), lp);
    json!({
        "pool": {
            "data": B64.encode(&pb), "bump": pool.bump, "market": pool.market.to_string(),
            "operator": pool.operator.to_string(), "shares": s(pool.shares), "deposited": s(pool.deposited),
            "withdrawn": s(pool.withdrawn), "createdTs": s(pool.created_ts),
        },
        "lp": {
            "data": B64.encode(&lb), "bump": lp.bump, "pool": lp.pool.to_string(), "owner": lp.owner.to_string(),
            "shares": s(lp.shares), "deposited": s(lp.deposited), "withdrawn": s(lp.withdrawn),
        },
    })
}

fn vault_accounts() -> Value {
    let vault = state::Vault {
        bump: 250,
        operator: key(86),
        quote_mint: key(84),
        cash_account: key(89),
        cash: 1_234_567,
        shares: 219_296_678_349,
        deposited: 219_396_678_349,
        withdrawn: 100_000_000,
        created_ts: 1_792_040_000,
        markets: vec![key(81), key(90), key(91)],
        rewards: None,
    };
    let mut vb = vec![0u8; state::VAULT_LEN];
    vault.write(&mut vb);
    assert_eq!(client::decode_vault(&vb).unwrap(), vault);
    // The same vault with an AMBA stream (`VaultRewards` grew it).
    let r = state::Rewards {
        mint: key(92),
        account: key(93),
        house: key(94),
        decimals: 9,
        start_ts: 1_790_812_800,
        end_ts: 1_793_491_200,
        amount: 100_000_000_000_000,
        last_ts: 1_791_000_000,
        acc: (1u128 << 100) + 12_345,
        house_shares: 3_000_000_000,
        distributed: 6_990_000_000_000,
        paid: 1_000_000_000_000,
    };
    let rewarded = state::Vault {
        rewards: Some(r),
        ..vault.clone()
    };
    let mut rb = vec![0u8; state::VAULT_REWARDS_LEN];
    rewarded.write(&mut rb);
    assert_eq!(client::decode_vault(&rb).unwrap(), rewarded);
    let position = state::VaultPosition {
        bump: 249,
        vault: EarnVault::address(&program(), &key(86)),
        owner: key(30),
        shares: u64::MAX,
        deposited: 7,
        withdrawn: 0,
        reward_snapshot: (1u128 << 99) + 5,
        reward_pending: 123_456_789,
        reward_paid: state::REWARD_PAID_MAX,
    };
    let mut pb = vec![0u8; state::VAULT_LP_LEN];
    position.write(&mut pb);
    assert_eq!(client::decode_vault_position(&pb).unwrap(), position);
    json!({
        "vault": {
            "data": B64.encode(&vb), "bump": vault.bump, "operator": vault.operator.to_string(),
            "quoteMint": vault.quote_mint.to_string(), "cashAccount": vault.cash_account.to_string(),
            "cash": s(vault.cash), "shares": s(vault.shares), "deposited": s(vault.deposited),
            "withdrawn": s(vault.withdrawn), "createdTs": s(vault.created_ts),
            "markets": vault.markets.iter().map(|k| k.to_string()).collect::<Vec<_>>(),
            "rewards": null,
        },
        "rewarded": {
            "data": B64.encode(&rb),
            "rewards": {
                "mint": r.mint.to_string(), "account": r.account.to_string(), "house": r.house.to_string(),
                "decimals": r.decimals, "startTs": s(r.start_ts), "endTs": s(r.end_ts), "amount": s(r.amount),
                "lastTs": s(r.last_ts), "acc": s(r.acc), "houseShares": s(r.house_shares),
                "distributed": s(r.distributed), "paid": s(r.paid),
            },
        },
        "position": {
            "data": B64.encode(&pb), "bump": position.bump, "vault": position.vault.to_string(),
            "owner": position.owner.to_string(), "shares": s(position.shares), "deposited": s(position.deposited),
            "withdrawn": s(position.withdrawn), "rewardSnapshot": s(position.reward_snapshot),
            "rewardPending": s(position.reward_pending), "rewardPaid": s(position.reward_paid),
        },
    })
}

/// Every vector, as `futures.json` holds them.
fn vectors() -> Value {
    let (open_h, open_b) = account(state::STATUS_OPEN, 0);
    let (settled_h, settled_b) = account(state::STATUS_SETTLED, 61_234);
    let (failed_h, failed_b) = account(state::STATUS_FAILED, 60_000);
    json!({
        "pdas": pdas(),
        "instructions": instructions(),
        "accounts": {
            "address": key(81).to_string(),
            "program": program().to_string(),
            "open": book_json(&open_h, &open_b),
            "settled": final_json(&settled_h, &settled_b),
            "failed": final_json(&failed_h, &failed_b),
            "registry": registry(),
            "pools": pool_accounts(),
            "vault": vault_accounts(),
        },
        "proposals": proposals(&open_b),
        "codec": codec(),
        "tiers": tiers(),
        "math": math(),
        "errors": errors(),
    })
}

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "test/fixtures/futures".into()));
    fs::create_dir_all(&dir).unwrap();
    let source = json!({
        "generator": "tools/futures-vectors",
        "futuresCommit": std::env::var("FUTURES_COMMIT").unwrap_or_else(|_| "unknown".into()),
    });
    let doc = json!({ "source": source, "vectors": vectors() });
    let path = dir.join("futures.json");
    fs::write(&path, serde_json::to_string_pretty(&doc).unwrap() + "\n").unwrap();
    println!("wrote {}", path.display());
}
