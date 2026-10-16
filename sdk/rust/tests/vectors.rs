//! Checks the futures client, reachable as `ameba_sdk::futures`, against
//! the golden vectors the TypeScript module is tested with
//! (`test/fixtures/futures/futures.json`).
use std::str::FromStr;

use ameba_sdk::futures::{
    self, Book, Core, CreateArgs, EarnVault, Instruction, Knot, Ladder, Market, Proposal, Pubkey,
    Side, SubmitArgs, Tiers,
};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use serde_json::Value;

fn vectors() -> Value {
    let path = format!(
        "{}/test/fixtures/futures/futures.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    v["vectors"].clone()
}

fn key(v: &Value) -> Pubkey {
    Pubkey::from_str(v.as_str().unwrap()).unwrap()
}
fn n<T: FromStr>(v: &Value) -> T
where
    T::Err: std::fmt::Debug,
{
    match v {
        Value::String(s) => s.parse().unwrap(),
        other => other.to_string().parse().unwrap(),
    }
}
fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
fn claim(v: &Value) -> (u32, u64) {
    (n(&v["slot"]), n(&v["seq"]))
}
fn ladder(v: &Value) -> Ladder {
    Ladder {
        lo: n(&v["lo"]),
        hi: n(&v["hi"]),
        bid_spread: n(&v["bidSpread"]),
        ask_spread: n(&v["askSpread"]),
        bid_lots: n(&v["bidLots"]),
        ask_lots: n(&v["askLots"]),
        keep: v["keep"].as_bool().unwrap(),
        tiers: Tiers {
            count: n(&v["tiers"]["count"]),
            ratio: n(&v["tiers"]["ratio"]),
            spread: n(&v["tiers"]["spread"]),
        },
    }
}
fn knots(v: &Value) -> Vec<Knot> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|k| Knot {
            pos: n(&k["pos"]),
            price: n(&k["price"]),
        })
        .collect()
}

/// A market from its vector's addresses (`market_json`).
fn market_of(v: &Value) -> Market {
    Market {
        program: key(&v["program"]),
        market: key(&v["market"]),
        registry: key(&v["registry"]),
        authority: key(&v["authority"]),
        vault: key(&v["vault"]),
        period: key(&v["period"]),
        mint: key(&v["mint"]),
    }
}
fn keys(v: &Value) -> Vec<Pubkey> {
    v.as_array().unwrap().iter().map(key).collect()
}

fn build(m: &Market, earn: &EarnVault, case: &Value) -> Instruction {
    let a = &case["args"];
    let owner = || key(&a["owner"]);
    match case["method"].as_str().unwrap() {
        "create" => {
            let p = &a["params"];
            let args = CreateArgs {
                product_id: ameba_sdk::futures::product_id(p["symbol"].as_str().unwrap()),
                expiry_ts: n(&p["expiryTs"]),
                grid: n(&p["grid"]),
                k: n(&p["k"]),
                step_value: n(&p["stepValue"]),
                value_decimals: n(&p["valueDecimals"]),
                ref_value: n(&p["refValue"]),
                trader_cap: n(&p["traderCap"]),
                order_cap: n(&p["orderCap"]),
                maker_cap: n(&p["makerCap"]),
                claim_cap: n(&p["claimCap"]),
            };
            assert_eq!(args.space(), n::<usize>(&a["space"]));
            m.create(&key(&a["creator"]), &key(&a["oracleProgram"]), &args)
        }
        "deposit" => m.deposit(&owner(), &key(&a["token"]), n(&a["amount"])),
        "withdraw" => m.withdraw(&owner(), &key(&a["token"]), n(&a["amount"])),
        "submit" => m.submit(
            &owner(),
            &SubmitArgs {
                side: if a["side"] == "long" {
                    Side::Long
                } else {
                    Side::Short
                },
                lo: n(&a["lo"]),
                hi: n(&a["hi"]),
                lots: n(&a["lots"]),
                limit: n(&a["limit"]),
                min_fill: n(&a["minFill"]),
                mode: n(&a["mode"]),
                close: (!a["close"].is_null()).then(|| claim(&a["close"])),
            },
            unhex(a["proposal"].as_str().unwrap()),
        ),
        "cancel" => m.cancel(&owner(), n(&a["slot"]), n(&a["seq"])),
        "quote" => m.quote(
            &owner(),
            &ladder(&a["ladder"]),
            knots(&a["knots"]),
            unhex(a["proposal"].as_str().unwrap()),
        ),
        "quoteCore" => m.quote_core(
            &owner(),
            &ladder(&a["ladder"]),
            Core {
                lo: n(&a["core"]["lo"]),
                hi: n(&a["core"]["hi"]),
            },
            knots(&a["knots"]),
            unhex(a["proposal"].as_str().unwrap()),
        ),
        "cancelQuote" => m.cancel_quote(&owner()),
        "split" => m.split(&owner(), claim(&a["claim"]), n(&a["at"])),
        "merge" => m.merge(&owner(), claim(&a["keep"]), claim(&a["absorb"])),
        "redeem" => m.redeem(
            &owner(),
            claim(&a["long"]),
            claim(&a["short"]),
            n(&a["lots"]),
        ),
        "transfer" => m.transfer(&owner(), claim(&a["claim"]), n(&a["lots"]), &key(&a["to"])),
        "finalize" => m.finalize(),
        "settle" => m.settle(
            a["claims"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| n(c))
                .collect(),
        ),
        "settleMaker" => m.settle_maker(n(&a["maker"])),
        "prune" => m.prune(
            a["orders"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| n(c))
                .collect(),
        ),
        "heapFrame" => futures::heap_frame(n(&a["bytes"])),
        "computeLimit" => futures::compute_limit(n(&a["units"])),
        "poolWithdraw" => m.pool_withdraw(
            &key(&a["pool"]),
            &owner(),
            &key(&a["token"]),
            n(&a["shares"]),
        ),
        "operated" => {
            let operator = key(&a["operator"]);
            assert_eq!(m.pool(&operator), key(&a["pool"]));
            futures::operated(
                m.cancel(&key(&a["pool"]), n(&a["slot"]), n(&a["seq"])),
                &operator,
            )
        }
        "createVault" => earn.create(&key(&a["payer"])),
        "vaultDeposit" => earn.deposit(
            &owner(),
            &key(&a["token"]),
            n(&a["amount"]),
            &key(&a["payer"]),
            &keys(&a["markets"]),
        ),
        "vaultWithdraw" => earn.withdraw(
            &owner(),
            &key(&a["token"]),
            n(&a["shares"]),
            &keys(&a["markets"]),
            &a["draws"]
                .as_array()
                .unwrap()
                .iter()
                .map(market_of)
                .collect::<Vec<_>>(),
        ),
        "vaultFund" => earn.fund(m, n(&a["amount"])),
        "vaultDefund" => earn.defund(m, n(&a["amount"])),
        "poolMigrate" => m.pool_migrate(
            &key(&a["pool"]),
            &key(&a["holder"]),
            earn,
            &key(&a["payer"]),
            &keys(&a["markets"]),
        ),
        "operatedVault" => {
            assert_eq!(earn.vault, key(&a["vault"]));
            futures::operated(
                m.cancel(&earn.vault, n(&a["slot"]), n(&a["seq"])),
                &key(&a["operator"]),
            )
        }
        other => panic!("no builder for {other}"),
    }
}

#[test]
fn every_instruction_matches_the_vectors() {
    let v = vectors();
    let ins = &v["instructions"];
    let mv = &ins["market"];
    let m = Market::derive(
        key(&mv["program"]),
        key(&mv["market"]),
        key(&mv["vault"]),
        key(&mv["period"]),
        key(&mv["mint"]),
        &ameba_sdk::futures::product_id(ins["productSymbol"].as_str().unwrap()),
        n(&ins["expiryTs"]),
    );
    assert_eq!(m.registry, key(&mv["registry"]));
    assert_eq!(m.authority, key(&mv["authority"]));
    let ev = &ins["earnVault"];
    let earn = EarnVault::new(
        key(&ev["program"]),
        key(&ev["operator"]),
        key(&ev["cash"]),
        key(&ev["mint"]),
    );
    assert_eq!(earn.vault, key(&ev["vault"]));
    for case in ins["cases"].as_array().unwrap() {
        let ix = build(&m, &earn, case);
        assert_eq!(ix.program_id, key(&case["programId"]), "{}", case["name"]);
        assert_eq!(hex(&ix.data), case["data"], "{}", case["name"]);
        let metas: Vec<(Pubkey, bool, bool)> = ix
            .accounts
            .iter()
            .map(|a| (a.pubkey, a.is_signer, a.is_writable))
            .collect();
        let want: Vec<(Pubkey, bool, bool)> = case["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| {
                (
                    key(&a["pubkey"]),
                    a["isSigner"].as_bool().unwrap(),
                    a["isWritable"].as_bool().unwrap(),
                )
            })
            .collect();
        assert_eq!(metas, want, "{}", case["name"]);
    }
}

#[test]
fn the_market_account_and_its_proposals() {
    let v = vectors();
    let a = &v["accounts"]["open"];
    let data = B64.decode(a["data"].as_str().unwrap()).unwrap();
    let book = Book::decode(&data).unwrap();
    assert_eq!(book.header.grid, n::<u16>(&a["header"]["grid"]));
    assert_eq!(book.arcs().len(), a["arcs"].as_array().unwrap().len());
    for o in a["book"]["owners"].as_array().unwrap() {
        let (lo, hi) = book.value_of(&key(&o["owner"]));
        assert_eq!(
            (lo.to_string(), hi.to_string()),
            (n(&o["value"][0]), n(&o["value"][1]))
        );
    }
    for p in v["proposals"].as_array().unwrap() {
        let bytes = unhex(p["proposal"].as_str().unwrap());
        let decoded = Proposal::decode(&bytes).unwrap();
        assert_eq!(decoded.encode(), bytes);
        assert_eq!(decoded.taker_fill, n::<u64>(&p["takerFill"]));
    }
    let pools = &v["accounts"]["pools"];
    let pool = futures::decode_pool(&B64.decode(pools["pool"]["data"].as_str().unwrap()).unwrap())
        .unwrap();
    assert_eq!(pool.operator, key(&pools["pool"]["operator"]));
    let lp =
        futures::decode_lp(&B64.decode(pools["lp"]["data"].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!(lp.shares, n::<u64>(&pools["lp"]["shares"]));
    let earn = &v["accounts"]["vault"];
    let vault =
        futures::decode_vault(&B64.decode(earn["vault"]["data"].as_str().unwrap()).unwrap())
            .unwrap();
    assert_eq!(vault.operator, key(&earn["vault"]["operator"]));
    assert_eq!(vault.markets, keys(&earn["vault"]["markets"]));
    let position = futures::decode_vault_position(
        &B64.decode(earn["position"]["data"].as_str().unwrap())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(position.shares, n::<u64>(&earn["position"]["shares"]));
    for c in v["pdas"]["vaults"].as_array().unwrap() {
        let program = key(&v["pdas"]["program"]);
        assert_eq!(
            EarnVault::address(&program, &key(&c["operator"])),
            key(&c["vault"])
        );
    }
    for (code, name) in v["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (n::<u32>(&e["code"]), e["name"].as_str()))
    {
        assert_eq!(futures::error_name(code), name);
    }
}
