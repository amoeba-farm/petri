//! Bots on the host, with feeds set by hand: no network.

use ameba_sdk_futures::futures::{
    Book, Claim, Header, Maker, Order as BookOrder, STATUS_OPEN, Side, Tiers, Trader,
    pricing::plan_wide,
    program::processor::MAX_BOOK_LADDER_ARCS,
    state::{self, market_len},
};
use solana_program::pubkey::Pubkey;

use super::{
    bundle::{self, Bundle, DEFAULT_BUDGET},
    feeds::{BookRead, Closing, Holding, Index, Intent, Order, Outcome, Position, Sample},
    host::{Host, Proposed},
    keys,
    registry::{self, Github},
    script, send, wallet,
    weights::Weights,
};
use crate::compose::State;

const MARKET: &str = "Mkt1111111111111111111111111111111111111111";
const NEXT: &str = "Mkt2222222222222222222222222222222222222222";
const NOW: u64 = 1_800_000_000;

/// An open market of 200 one-point steps with `owner` holding `free` cash.
fn market(owner: Pubkey, free: u64) -> Book {
    let h = Header {
        status: STATUS_OPEN,
        grid: 200,
        trader_cap: 4,
        order_cap: 4,
        maker_cap: 2,
        claim_cap: 16,
        k: 10_000,
        step_value: 1_000_000,
        value_decimals: 6,
        expiry_ts: NOW + 30 * 86_400,
        next_seq: 1,
        ..Header::default()
    };
    let mut data = vec![0u8; market_len(200, 4, 4, 2, 16)];
    h.write(&mut data);
    Trader {
        owner,
        free,
        ..Trader::default()
    }
    .write(&mut data, h.trader_off(0));
    Book::decode(&data).unwrap()
}

/// `market` with the house maker's ladder on it, as `futures-maker` posts
/// it with its defaults (whole grid, five tiers over 64 steps).
fn with_house(mut book: Book) -> Book {
    let h = book.header;
    Trader {
        owner: Pubkey::new_unique(),
        free: 1 << 50,
        maker: 1,
        ..Trader::default()
    }
    .write(&mut book.data, h.trader_off(1));
    Maker {
        used: true,
        trader: 1,
        ..Maker::default()
    }
    .write(&mut book.data, h.maker_off(0));
    let tiers = Tiers {
        count: 5,
        ratio: 120,
        spread: 40,
    };
    let (l, core, knots) = plan_wide(100.0, 8.0, 10_000, 200, 32, 100, 0, (1_000, 1_000), tiers);
    state::write_ladder(&mut book.data, &h, 0, &l, core, &knots).unwrap();
    let mut mk = Maker::read(&book.data, h.maker_off(0));
    mk.seq = 1;
    mk.lo = l.lo;
    mk.hi = l.hi;
    mk.write(&mut book.data, h.maker_off(0));
    Book::decode(&book.data).unwrap()
}

/// Fills `MARKET`'s feeds as the worker would: index 100, and each bot's
/// wallet (`owner`) with 100 USDC.
fn read(host: &Host, owner: Pubkey) {
    read_in(host, MARKET, "ramx-nov26", owner, market(owner, 0));
}

fn read_book(host: &Host, owner: Pubkey, book: Book) {
    read_in(host, MARKET, "ramx-nov26", owner, book);
}

fn read_in(host: &Host, at: &str, label: &str, owner: Pubkey, book: Book) {
    let (_, feeds) = host
        .markets()
        .into_iter()
        .find(|(m, _)| m == at)
        .expect("the bot's market");
    feeds.index.set(Index {
        label: label.into(),
        symbol: "RAMX".into(),
        centre: 100.0,
        value: 100.0,
        step: 1.0,
        expiry_ts: NOW + 30 * 86_400,
        now: NOW,
    });
    feeds.book.set(BookRead {
        book,
        tradable: true,
    });
    for (_, feed) in host.bots_in(at) {
        feed.set(Position {
            owner,
            wallet: 100_000_000,
            ..Position::default()
        });
    }
}

fn ladder_bot(id: &str) -> Bundle {
    Bundle::parse(&bundle::template(id, MARKET)).unwrap()
}

/// A bot of one `strategy/script` row running `source`.
fn script_bot(id: &str, source: &str) -> Bundle {
    Bundle::parse(&format!(
        "[bot]\nid = \"{id}\"\nmarket = \"{MARKET}\"\n\n[[row]]\nid = \"strategy\"\nkind = \"strategy/script\"\nsource = '''\n{source}\n'''\n"
    ))
    .unwrap()
}

fn start(host: &mut Host, b: &Bundle) {
    host.start(&b.id, MARKET, &b.rows, b.budget_atoms())
        .unwrap();
}

fn quote(p: &Proposed) -> (u32, u64) {
    match &p.intent {
        Intent::Quote { ladder, lots, .. } => (ladder.bid_spread, *lots),
        other => panic!("not a quote: {other:?}"),
    }
}

/// `petri long --lots 1`'s order, as a script's `long(#{ lots: 1 })`.
fn one_lot() -> Intent {
    Intent::Open(Order {
        side: Side::Long,
        lots: Some(1),
        usd: None,
        leverage: 10,
        to: None,
        range: None,
        limit: None,
        post: false,
        slippage: "2".into(),
    })
}

#[test]
fn the_template_quotes_within_its_budget_and_cancels_when_stopped() {
    let owner = Pubkey::new_unique();
    let mut host = Host::new();
    let bot = ladder_bot("ladder");
    assert_eq!(bot.rows.len(), 4);
    start(&mut host, &bot);
    assert!(
        host.parts("ladder")
            .iter()
            .all(|p| p.state == State::Active)
    );
    assert_eq!(host.ticking(), 1);
    // Nothing read yet: nothing to quote.
    host.tick(NOW);
    assert!(host.drain().is_empty());
    read(&host, owner);
    host.tick(NOW);
    let sent = host.drain();
    assert_eq!(sent.len(), 1);
    let (spread, lots) = quote(&sent[0]);
    assert_eq!(spread, 100);
    assert!(lots > 0);
    let Intent::Quote {
        ladder,
        core,
        knots,
        ..
    } = &sent[0].intent
    else {
        unreachable!()
    };
    let book = market(owner, 0);
    let escrow = book.ladder_escrow(&owner, ladder, *core, knots).unwrap();
    assert!(escrow <= 100_000_000, "{escrow}");
    assert!(
        book.quote_proposal_core(&owner, ladder, *core, knots)
            .is_some()
    );
    // Stopping it takes its ladder down and leaves only the host's keys.
    host.stop("ladder");
    let sent = host.drain();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].intent, Intent::Cancel);
    assert_eq!(host.ticking(), 0);
    assert_eq!(host.provided(), 4);
    assert!(!host.any());
}

#[test]
fn beside_the_house_maker_a_bot_narrows_into_the_room_left() {
    let owner = Pubkey::new_unique();
    let book = with_house(market(owner, 0));
    // The house ladder: 64 core steps x 5 tiers x 2 sides + 136 others x 2.
    assert_eq!(state::ladder_arcs(&book.data, &book.header), 912);
    let mut host = Host::new();
    start(&mut host, &ladder_bot("ladder"));
    read_book(&host, owner, book.clone());
    host.tick(NOW);
    let sent = host.drain();
    let Intent::Quote {
        ladder,
        core,
        knots,
        fitted,
        ..
    } = &sent[0].intent
    else {
        panic!("{sent:?}")
    };
    // 112 arcs left: one tier over 32 steps (64 arcs) is the largest that fits.
    assert!(fitted);
    assert_eq!((ladder.tiers.count, ladder.hi - ladder.lo), (1, 32));
    assert_eq!((core.lo, core.hi), (ladder.lo, ladder.hi));
    let arcs = book.quote_arcs(&owner, ladder, *core, knots).unwrap();
    assert_eq!(arcs, 912 + 64);
    assert!(arcs <= MAX_BOOK_LADDER_ARCS);
    assert!(
        book.quote_proposal_core(&owner, ladder, *core, knots)
            .is_some()
    );
}

#[test]
fn an_edited_weight_requotes_once_and_a_bad_edit_changes_nothing() {
    let owner = Pubkey::new_unique();
    let mut host = Host::new();
    let mut bot = ladder_bot("ladder");
    start(&mut host, &bot);
    read(&host, owner);
    host.tick(NOW);
    host.drain();
    host.tell("ladder", Outcome::Posted("sig".into()));
    bot.set("spread_bps", "200").unwrap();
    host.update("ladder", &bot.rows, bot.budget_atoms())
        .unwrap();
    host.tick(NOW);
    // The old ladder's cancel and the new ladder: the new one replaces it.
    let sent = host.drain();
    assert_eq!(sent.len(), 1);
    assert_eq!(quote(&sent[0]).0, 200);
    bot.set("tiers", "9").unwrap();
    assert!(
        host.update("ladder", &bot.rows, bot.budget_atoms())
            .is_err()
    );
    assert_eq!(host.ticking(), 1);
    assert!(
        host.parts("ladder")
            .iter()
            .all(|p| p.state == State::Active)
    );
}

/// `[bot] budget` caps what the ladder quotes with, and a new budget
/// reaches the running ladder without a restart.
#[test]
fn the_bots_budget_caps_its_ladder() {
    let owner = Pubkey::new_unique();
    let mut host = Host::new();
    let text =
        bundle::template("ladder", MARKET).replace("market = \"", "budget = 10\nmarket = \"");
    let bot = Bundle::parse(&text).unwrap();
    // [bot] budget wins over the weights row's 100.
    assert_eq!(bot.budget_atoms(), 10_000_000);
    start(&mut host, &bot);
    read(&host, owner);
    host.tick(NOW);
    let small = quote(&host.drain()[0]).1;
    host.tell("ladder", Outcome::Refused("again".into()));
    host.update("ladder", &bot.rows, 100_000_000).unwrap();
    host.tick(NOW);
    let large = quote(&host.drain()[0]).1;
    assert!(small < large, "{small} {large}");
}

#[test]
fn a_refused_quote_is_asked_again() {
    let owner = Pubkey::new_unique();
    let mut host = Host::new();
    start(&mut host, &ladder_bot("ladder"));
    read(&host, owner);
    host.tick(NOW);
    assert_eq!(host.drain().len(), 1);
    host.tell("ladder", Outcome::Refused("no".into()));
    host.tick(NOW);
    assert_eq!(host.drain().len(), 1);
}

#[test]
fn two_bots_in_one_market_keep_apart() {
    let owner = Pubkey::new_unique();
    let mut host = Host::new();
    start(&mut host, &ladder_bot("one"));
    start(&mut host, &ladder_bot("two"));
    assert_eq!(host.markets().len(), 1);
    read(&host, owner);
    host.tick(NOW);
    let bots: Vec<String> = host.drain().into_iter().map(|p| p.bot).collect();
    assert_eq!(bots, ["one", "two"]);
    host.stop("one");
    assert_eq!(host.ticking(), 1);
    assert!(host.parts("two").iter().all(|p| p.state == State::Active));
    host.tick(NOW);
    // One's cancel; two quotes on (its ladder never landed: no book shows it).
    let sent = host.drain();
    assert_eq!(sent.len(), 2);
    assert_eq!(
        (sent[0].bot.as_str(), &sent[0].intent),
        ("one", &Intent::Cancel)
    );
    assert_eq!(quote(&sent[1]).0, 100);
}

/// Each bot reads its own wallet: in one market, the one whose wallet has
/// USDC quotes and the one whose wallet has none does not.
#[test]
fn each_bot_reads_its_own_wallet() {
    let (rich, poor) = (Pubkey::new_unique(), Pubkey::new_unique());
    let mut host = Host::new();
    start(&mut host, &ladder_bot("rich"));
    start(&mut host, &ladder_bot("poor"));
    read(&host, rich);
    let feeds: std::collections::BTreeMap<_, _> = host.bots_in(MARKET).into_iter().collect();
    feeds["poor"].set(Position {
        owner: poor,
        ..Position::default()
    });
    host.tick(NOW);
    let sent = host.drain();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].bot, "rich");
    assert_eq!(host.position("poor").unwrap().owner, poor);
    assert_eq!(host.position("rich").unwrap().wallet, 100_000_000);
}

#[test]
fn a_bot_with_a_bad_row_does_not_start() {
    let mut host = Host::new();
    let mut rows = ladder_bot("ladder").rows;
    rows[1].kind = "signal/oracle-of-delphi".into();
    let error = host
        .start("ladder", MARKET, &rows, DEFAULT_BUDGET)
        .unwrap_err();
    assert_eq!(
        error,
        "fair: petri has no part called signal/oracle-of-delphi"
    );
    assert!(!host.any());
    assert_eq!(host.ticking(), 0);
    rows[1].kind = "signal/normal-fair".into();
    rows[1].config.insert("vol".into(), "0.5".into());
    let error = host
        .start("ladder", MARKET, &rows, DEFAULT_BUDGET)
        .unwrap_err();
    assert_eq!(error, "fair: signal/normal-fair takes no settings");
}

#[test]
fn the_host_keys_stay_the_hosts() {
    assert!(keys::host("clock"));
    assert!(keys::host(&keys::index(MARKET)));
    assert!(keys::host(&keys::book(MARKET)));
    assert!(keys::host(&keys::history(MARKET)));
    // A bot's own wallet and run are its root's: no row provides them.
    assert!(keys::host(&keys::position(MARKET)));
    assert!(keys::host(keys::RUN));
    assert!(!keys::host(keys::FAIR));
    assert!(!keys::host(keys::INTENTS));
    let realm = keys::realm("x");
    assert_eq!(
        realm.last().unwrap(),
        &("*".to_string(), "bot:x".to_string())
    );
    // ... and they resolve in the bot's realm, not the host's.
    assert!(!realm.iter().any(|(p, _)| p.starts_with("position")));
}

#[test]
fn weights_default_to_the_house_makers() {
    let w = Weights::parse(&Default::default()).unwrap();
    assert_eq!(w, Weights::default());
    let mut set = std::collections::BTreeMap::new();
    set.insert("budget".to_string(), "12.5".to_string());
    set.insert("spread_bps".to_string(), "150".to_string());
    let w = Weights::parse(&set).unwrap();
    assert_eq!((w.budget, w.spread_bps), (Some(12_500_000), 150));
    set.insert("spred_bps".to_string(), "1".to_string());
    assert_eq!(
        Weights::parse(&set).unwrap_err(),
        "there is no weight called spred_bps"
    );
}

#[test]
fn a_weight_edit_keeps_the_rest_of_the_file() {
    let mut bot = ladder_bot("ladder");
    bot.set("budget", "250").unwrap();
    bot.set("vol", "0.55").unwrap();
    let text = bot.text();
    assert!(
        text.contains("budget = 250       # most USDC it uses"),
        "{text}"
    );
    assert!(
        text.contains("vol = 0.55          # yearly volatility"),
        "{text}"
    );
    assert!(text.contains("# Fair value of every step: normal around the index."));
    let again = Bundle::parse(&text).unwrap();
    assert_eq!(again.weights().unwrap()["budget"], "250");
    assert_eq!(again.rows.len(), 4);
    assert!(bot.set("budget", "lots").is_err());
}

#[test]
fn bundles_say_what_is_wrong() {
    let bad = |text: &str| Bundle::parse(text).unwrap_err();
    assert_eq!(bad("x = 1"), "the file needs a [bot] section");
    assert_eq!(
        bad("[bot]\nid = \"a b\"\nmarket = \"m\""),
        "a bot's id is letters, digits, - and _"
    );
    assert_eq!(bad("[bot]\nid = \"a\""), "[bot] needs a market");
    assert_eq!(
        bad("[bot]\nid = \"a\"\nmarket = \"m\"\nsigner = \"x\""),
        "[bot] has no setting called signer"
    );
    assert_eq!(
        bad("[bot]\nid = \"a\"\nmarket = \"m\"\n[[row]]\nid = \"w\""),
        "row w needs a kind"
    );
    assert_eq!(
        bad("[bot]\nid = \"a\"\nmarket = \"m\"\nbudget = \"lots\""),
        "budget = lots is not an amount of USDC"
    );
    assert_eq!(
        bad("[bot]\nid = \"a\"\nmarket = \"m\"\nbudget = -5"),
        "budget = -5 is not an amount of USDC"
    );
    assert_eq!(
        bad("[bot]\nid = \"a\"\nmarket = \"m\"\nname = 5"),
        "[bot] name should be text in quotes"
    );
    let folder = std::env::temp_dir().join(format!("petri-bots-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("ladder.toml"),
        bundle::template("ladder", MARKET),
    )
    .unwrap();
    assert_eq!(bundle::fresh(&folder, "ladder").0, "ladder-2");
    assert_eq!(bundle::fresh(&folder, "other").0, "other");
    let all = bundle::read_all(&folder);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].2.as_ref().unwrap().id, "ladder");
    std::fs::remove_dir_all(&folder).unwrap();
}

// --- The bundle's settings and its hash -----------------------------------------

/// `docs/bots.md`'s test vector: the futures API tests the same one.
const VECTOR: &str = "[bot]\nid = \"dip-buyer\"\nmarket = \"ramx-nov26\"\nname = \"Dip buyer\"\nbudget = 100\nfrom = \"dip-buyer\"\n\n[[row]]\nid = \"strategy\"\nkind = \"strategy/script\"\nsource = '''\nfn tick(m) {\n  let id = 1;\n}\n'''\n";

#[test]
fn the_hash_is_the_docs_vector() {
    assert_eq!(
        bundle::hash(VECTOR),
        "03f3aea639cdd13e7bb4580da1df62e8a6b659d546a3204cf0d9a4697741e90a"
    );
    let b = Bundle::parse(VECTOR).unwrap();
    assert_eq!(b.hash(), bundle::hash(VECTOR));
    // The bytes hashed: the runner's lines in [bot] out, the row's `id` and
    // the script's `let id` kept.
    use sha2::{Digest, Sha256};
    let kept = "[bot]\nname = \"Dip buyer\"\n\n[[row]]\nid = \"strategy\"\nkind = \"strategy/script\"\nsource = '''\nfn tick(m) {\n  let id = 1;\n}\n'''\n";
    assert_eq!(bundle::hash(VECTOR), hex::encode(Sha256::digest(kept)));
}

#[test]
fn the_hash_is_the_strategy() {
    let h = bundle::hash(VECTOR);
    // Another runner, month, budget or registry id: the same bot.
    for (from, to) in [
        ("id = \"dip-buyer\"", "id = \"dip-buyer-2\""),
        ("market = \"ramx-nov26\"", "market = \"nandx\""),
        ("budget = 100", "budget = 2500"),
        ("from = \"dip-buyer\"\n", ""),
        ("budget = 100\n", ""),
        ("  id = ", "id = "),
    ] {
        assert_eq!(bundle::hash(&VECTOR.replacen(from, to, 1)), h, "{to}");
    }
    // Its name, author, a row or the script: another bot.
    for (from, to) in [
        ("name = \"Dip buyer\"", "name = \"Dip seller\""),
        (
            "name = \"Dip buyer\"",
            "name = \"Dip buyer\"\nauthor = \"Me\"",
        ),
        ("id = \"strategy\"", "id = \"plan\""),
        ("let id = 1;", "let id = 2;"),
    ] {
        assert_ne!(bundle::hash(&VECTOR.replacen(from, to, 1)), h, "{to}");
    }
    // A runner's key in a table after [bot] is the strategy's.
    let later = VECTOR.replace("[[row]]\nid", "[[row]]\nbudget = 1\nid");
    assert_ne!(bundle::hash(&later), h);
    // Only a line that is exactly `[bot]` (blanks around it aside) opens
    // it, as the futures API reads it: after `[bot] # mine` the runner's
    // lines are the strategy's.
    use sha2::{Digest, Sha256};
    let padded = VECTOR.replace("[bot]\n", "  [bot]  \n");
    let kept = "  [bot]  \nname = \"Dip buyer\"\n\n[[row]]\nid = \"strategy\"\nkind = \"strategy/script\"\nsource = '''\nfn tick(m) {\n  let id = 1;\n}\n'''\n";
    assert_eq!(bundle::hash(&padded), hex::encode(Sha256::digest(kept)));
    let commented = VECTOR.replace("[bot]\n", "[bot] # mine\n");
    assert_eq!(
        bundle::hash(&commented),
        hex::encode(Sha256::digest(&commented))
    );
}

#[test]
fn bot_settings_and_their_defaults() {
    let b = Bundle::parse(VECTOR).unwrap();
    assert_eq!(b.name(), "Dip buyer");
    assert_eq!(b.from.as_deref(), Some("dip-buyer"));
    assert_eq!(b.author, None);
    assert_eq!(b.budget_atoms(), 100_000_000);
    assert_eq!(b.kinds(), ["strategy/script"]);
    // Publishing sends the file without `from`; the hash stays.
    let published = b.publish_text();
    assert!(!published.contains("from ="));
    assert!(published.contains("name = \"Dip buyer\""));
    assert_eq!(bundle::hash(&published), b.hash());
    // The author a publish writes in is the strategy's.
    let mut authored = b.clone();
    authored.set_bot("author", "Author1111").unwrap();
    assert_eq!(authored.author.as_deref(), Some("Author1111"));
    assert_ne!(authored.hash(), b.hash());
    // An old bundle: no name (its id), the weights row's budget.
    let old = ladder_bot("ladder");
    assert_eq!((old.name(), old.about.as_deref()), ("ladder", None));
    assert_eq!(old.budget_atoms(), 100_000_000);
    assert_eq!(
        old.kinds(),
        [
            "weights",
            "signal/normal-fair",
            "signal/inventory-skew",
            "strategy/ladder"
        ]
    );
    let mut row_only = old.clone();
    row_only.set("budget", "40").unwrap();
    assert_eq!(row_only.budget_atoms(), 40_000_000);
    // [bot] budget wins over the row's.
    row_only.set_budget(12.5).unwrap();
    assert_eq!(row_only.budget_atoms(), 12_500_000);
    assert!(row_only.text().contains("budget = 12.5\n"));
    // Neither: 100 USDC.
    let bare = Bundle::parse(&format!("[bot]\nid = \"a\"\nmarket = \"{MARKET}\"\n")).unwrap();
    assert_eq!(bare.budget_atoms(), DEFAULT_BUDGET);
    // An install's edits keep the rest of the file.
    let mut installed = b.clone();
    installed.set_bot("id", "dip-buyer-2").unwrap();
    installed.set_bot("market", "nandx").unwrap();
    let text = installed.text();
    assert!(text.contains("id = \"dip-buyer-2\"\nmarket = \"nandx\"\nname"));
    assert!(text.contains("fn tick(m) {\n  let id = 1;\n}"));
    assert_eq!(installed.hash(), b.hash());
    assert!(installed.set_bot("signer", "x").is_err());
}

#[test]
fn the_running_list_keeps_the_bots_that_run() {
    let folder = std::env::temp_dir().join(format!("petri-bots-running-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    crate::petri_config::create_private_dir_all(&folder).unwrap();
    assert!(bundle::running(&folder).is_empty());
    bundle::set_running(&folder, "dip", true).unwrap();
    bundle::set_running(&folder, "hold", true).unwrap();
    bundle::set_running(&folder, "dip", true).unwrap();
    assert_eq!(bundle::running(&folder), ["dip", "hold"]);
    assert_eq!(
        std::fs::read_to_string(folder.join(".running")).unwrap(),
        "dip\nhold\n"
    );
    bundle::set_running(&folder, "dip", false).unwrap();
    bundle::set_running(&folder, "gone", false).unwrap();
    assert_eq!(bundle::running(&folder), ["hold"]);
    std::fs::remove_dir_all(&folder).unwrap();
}

// --- strategy/script ----------------------------------------------------------

/// Runs `source` as a bot's only row once, on the feeds `read` sets (and
/// `history`): what it proposed and what it says.
fn tick_once(source: &str, history: Vec<Sample>) -> (Vec<Intent>, Option<String>) {
    let mut host = Host::new();
    start(&mut host, &script_bot("s", source));
    read(&host, Pubkey::new_unique());
    let (_, feeds) = host.markets().into_iter().next().unwrap();
    feeds.history.set(history);
    host.tick(NOW);
    let sent = host.drain().into_iter().map(|p| p.intent).collect();
    (sent, host.note("s"))
}

#[test]
fn a_script_compiles_or_says_why() {
    assert!(script::compile("fn tick(m) { }").is_ok());
    let e = script::compile("fn tick(m) { let x = ; }").unwrap_err();
    assert!(e.starts_with("the script does not compile"), "{e}");
    assert_eq!(
        script::compile("fn step(m) { }").unwrap_err(),
        "the script needs a fn tick(m)"
    );
    // A row that does not compile is the bot's problem: it does not start.
    let mut host = Host::new();
    let mut bot = script_bot("s", "fn tick(m) { }");
    bot.rows[0]
        .config
        .insert("source".into(), "fn tick(m) {".into());
    let e = host
        .start("s", MARKET, &bot.rows, DEFAULT_BUDGET)
        .unwrap_err();
    assert!(
        e.starts_with("strategy: the script does not compile"),
        "{e}"
    );
    bot.rows[0].config.insert("vol".into(), "1".into());
    let e = host
        .start("s", MARKET, &bot.rows, DEFAULT_BUDGET)
        .unwrap_err();
    assert_eq!(e, "strategy: strategy/script has no setting called vol");
}

/// No clock, no sleep, no modules, no files: each is a function or a
/// module the sandbox doesn't have, and the tick that calls it is a note.
#[test]
fn a_script_reaches_nothing_outside() {
    for call in [
        "timestamp()",
        "sleep(1)",
        "exit()",
        "open_file(\"/etc/passwd\")",
        "import \"os\" as os",
    ] {
        let (sent, note) = tick_once(
            &format!("fn tick(m) {{ long(#{{ usd: 1 }}); {call}; }}"),
            vec![],
        );
        // Nothing it queued goes out.
        assert!(sent.is_empty(), "{call}");
        let note = note.unwrap_or_default();
        assert!(note.starts_with("Its script stopped"), "{call}: {note}");
    }
    // `print` and `debug` write nowhere and go on.
    let (sent, note) = tick_once(
        "fn tick(m) { print(\"x\"); debug(\"y\"); cancel(); }",
        vec![],
    );
    assert_eq!(sent, [Intent::CancelOrders]);
    assert_eq!(note, None);
}

/// A script that loops forever is stopped by the operation limit: that
/// look is its note, and the worker and its other bots run on.
#[test]
fn an_endless_script_stops_at_the_limit_and_the_rest_runs_on() {
    let owner = Pubkey::new_unique();
    let mut host = Host::new();
    start(&mut host, &script_bot("loop", "fn tick(m) { loop { } }"));
    start(&mut host, &ladder_bot("ladder"));
    read(&host, owner);
    host.tick(NOW);
    let note = host.note("loop").unwrap();
    assert!(note.contains("5,000,000 operations"), "{note}");
    let sent = host.drain();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].bot, "ladder");
    // It keeps running, and tries again next look.
    assert!(host.running("loop"));
    assert!(host.parts("loop").iter().all(|p| p.state == State::Active));
    host.tick(NOW);
    assert!(host.note("loop").unwrap().contains("operations"));
}

/// A script that grows a string or an array without end stops at the size
/// limits, well inside its operations, instead of taking petri's memory.
#[test]
fn a_script_that_grows_without_end_stops_at_the_size_limits() {
    for source in [
        "fn tick(m) { let s = \"x\"; for i in 0..40 { s += s; } cancel(); }",
        "fn tick(m) { let a = [1]; for i in 0..40 { a += a; } cancel(); }",
    ] {
        let (sent, note) = tick_once(source, vec![]);
        assert!(sent.is_empty(), "a failing tick sends nothing: {sent:?}");
        let note = note.expect("the look says why");
        assert!(
            !note.contains("operations"),
            "it stopped at a size, not the operation limit: {note}"
        );
    }
}

#[test]
fn a_scripts_actions_become_intents_in_order() {
    let (sent, note) = tick_once(
        r#"fn tick(m) {
  long(#{ usd: 10, leverage: 5 });
  short(#{ lots: 3, range: "90-120", limit: 105.5, post: true, slippage: 1 });
  close(#{ side: "long" });
  close(#{});
  cancel();
  say("bought the dip");
}"#,
        vec![],
    );
    assert_eq!(
        sent,
        [
            Intent::Open(Order {
                side: Side::Long,
                lots: None,
                usd: Some("10".into()),
                leverage: 5,
                to: None,
                range: None,
                limit: None,
                post: false,
                slippage: "2".into(),
            }),
            Intent::Open(Order {
                side: Side::Short,
                lots: Some(3),
                usd: None,
                leverage: 10,
                to: None,
                range: Some("90-120".into()),
                limit: Some("105.5".into()),
                post: true,
                slippage: "1".into(),
            }),
            Intent::Close(Closing {
                side: Some(Side::Long),
                ..Closing::default()
            }),
            Intent::Close(Closing::default()),
            Intent::CancelOrders,
        ]
    );
    assert_eq!(note.as_deref(), Some("bought the dip"));
}

#[test]
fn a_scripts_bad_order_is_its_note() {
    for (call, why) in [
        ("long(#{})", "long needs usd or lots"),
        (
            "long(#{ usd: 1, lots: 2 })",
            "long takes usd or lots, not both",
        ),
        (
            "short(#{ lots: 0 })",
            "lots should be a whole number of at least 1",
        ),
        (
            "long(#{ usd: 1, size: 2 })",
            "long has no setting called size",
        ),
        ("long(#{ usd: 1, post: true })", "long: post needs a limit"),
        (
            "long(#{ usd: 1, range: \"90-120\", leverage: 3 })",
            "long: range names the whole range, without leverage or to",
        ),
        ("close(#{ side: \"up\" })", "side is long or short, not up"),
    ] {
        let (sent, note) = tick_once(&format!("fn tick(m) {{ cancel(); {call}; }}"), vec![]);
        assert!(sent.is_empty(), "{call}");
        let note = note.unwrap_or_default();
        assert!(note.contains(why), "{call}: {note}");
    }
}

#[test]
fn a_script_reads_m() {
    let wall = send::wall();
    let history = vec![
        Sample {
            t: wall - 3_600,
            v: 125.0,
        },
        Sample {
            t: wall - 60,
            v: 110.0,
        },
    ];
    let (_, note) = tick_once(
        r#"fn tick(m) {
  let h = m.history;
  say(`${m.market}|${m.symbol}|${m.index}|${m.step}|${m.low}|${m.high}|${m.expiry_in}|${m.now}|${m.tradable}|${m.cash}|${m.budget}|${m.holdings.len()}|${m.held.len()}|${m.orders}|${m.ladder}|${h.len()}|${h[0].v}|${m.vol}|${m.change(3600)}|${m.change(30)}|${m.change(7200)}|${m.prob_above(m.index)}`);
}"#,
        history,
    );
    let note = note.unwrap();
    let parts: Vec<&str> = note.split('|').collect();
    assert_eq!(
        &parts[..18],
        [
            "ramx-nov26",
            "RAMX",
            "100.0",
            "1.0",
            "0.0",
            "200.0",
            "2592000",
            "1800000000",
            "true",
            "100.0",
            "100.0",
            "0",
            "0",
            "0",
            "false",
            "2",
            "125.0",
            "0.4",
        ],
        "{note}"
    );
    let number = |i: usize| parts[i].parse::<f64>().unwrap();
    assert!((number(18) - (100.0 / 125.0 - 1.0)).abs() < 1e-9, "{note}");
    assert!((number(19) - (100.0 / 110.0 - 1.0)).abs() < 1e-9, "{note}");
    assert_eq!(number(20), 0.0);
    assert!((number(21) - 0.5).abs() < 1e-6, "{note}");
}

/// `m.holdings` is the month it trades; `m.held` every month it holds.
#[test]
fn a_script_sees_its_holdings_here_and_in_other_months() {
    let mut host = Host::new();
    let bot = script_bot(
        "s",
        r#"fn tick(m) {
  let h = m.holdings[0];
  let old = m.held[1];
  say(`${m.holdings.len()}|${m.held.len()}|${h.side}|${h.low}|${h.high}|${h.lots}|${h.cost}|${h.value}|${m.held[0].market}|${old.market}|${old.side}`);
}"#,
    );
    start(&mut host, &bot);
    read(&host, Pubkey::new_unique());
    let holding = |side, cost, value| Holding {
        side,
        lo: 90,
        hi: 120,
        low: "90".into(),
        high: "120.5".into(),
        lots: 3,
        cost,
        value,
    };
    let (_, feed) = host.bots_in(MARKET).remove(0);
    feed.set(Position {
        holdings: vec![holding(Side::Long, 1_500_000, 2_250_000)],
        elsewhere: vec![("ramx-oct26".into(), holding(Side::Short, 1, 2))],
        ..Position::default()
    });
    host.tick(NOW);
    assert_eq!(
        host.note("s").as_deref(),
        Some("1|2|long|90.0|120.5|3|1.5|2.25|ramx-nov26|ramx-oct26|short")
    );
}

#[test]
fn a_scripts_memory_lasts_the_run_and_m_is_read_only() {
    let mut host = Host::new();
    let bot = script_bot(
        "s",
        r#"fn tick(m) {
  if !("n" in m.memory) { m.memory.n = 0; }
  m.memory.n += 1;
  m.index = 0.0;
  say(`${m.memory.n}|${m.index}`);
}"#,
    );
    start(&mut host, &bot);
    read(&host, Pubkey::new_unique());
    for n in 1..=3 {
        host.tick(NOW);
        assert_eq!(host.note("s"), Some(format!("{n}|0.0")));
    }
    // What it wrote into `m` other than its memory is gone next look.
    let (_, feeds) = host.markets().into_iter().next().unwrap();
    assert_eq!(feeds.index.get().unwrap().value, 100.0);
    // Its memory goes with it to the next month; a new run starts empty.
    host.roll("s", NEXT, &bot.rows).unwrap();
    read_in(
        &host,
        NEXT,
        "ramx-dec26",
        Pubkey::new_unique(),
        market(Pubkey::new_unique(), 0),
    );
    host.tick(NOW);
    assert_eq!(host.note("s").as_deref(), Some("4|0.0"));
    host.stop("s");
    start(&mut host, &bot);
    read(&host, Pubkey::new_unique());
    host.tick(NOW);
    assert_eq!(host.note("s").as_deref(), Some("1|0.0"));
}

/// A product bot moved to the next month: its ladder comes down where it
/// was, it trades there, and `m.market` says so.
#[test]
fn a_rolled_bot_trades_the_next_month() {
    let owner = Pubkey::new_unique();
    let mut host = Host::new();
    let text = format!(
        "{}\n[[row]]\nid = \"strategy\"\nkind = \"strategy/script\"\nsource = '''\nfn tick(m) {{ say(m.market); }}\n'''\n",
        bundle::template("both", MARKET)
    );
    let bot = Bundle::parse(&text).unwrap();
    start(&mut host, &bot);
    read(&host, owner);
    host.tick(NOW);
    assert_eq!(host.drain().len(), 1);
    assert_eq!(host.note("both").as_deref(), Some("ramx-nov26"));
    host.roll("both", NEXT, &bot.rows).unwrap();
    read_in(&host, NEXT, "ramx-dec26", owner, market(owner, 0));
    host.tick(NOW);
    let sent = host.drain();
    assert_eq!(
        sent.iter()
            .map(|p| (p.market.as_str(), p.intent.ladder()))
            .collect::<Vec<_>>(),
        [(MARKET, true), (NEXT, true)]
    );
    assert_eq!(sent[0].intent, Intent::Cancel);
    assert_eq!(quote(&sent[1]).0, 100);
    assert_eq!(host.note("both").as_deref(), Some("ramx-dec26"));
    // The old month's feeds go once no bot runs there.
    assert_eq!(host.markets().len(), 1);
}

/// The drain keeps a bot's last ladder intent and every order, in the order
/// they were proposed.
#[test]
fn the_drain_keeps_every_order_and_the_last_ladder() {
    let owner = Pubkey::new_unique();
    let mut host = Host::new();
    let text = format!(
        "{}\n[[row]]\nid = \"strategy\"\nkind = \"strategy/script\"\nsource = '''\nfn tick(m) {{ long(#{{ lots: 1 }}); }}\n'''\n",
        bundle::template("both", MARKET)
    );
    let mut bot = Bundle::parse(&text).unwrap();
    start(&mut host, &bot);
    read(&host, owner);
    host.tick(NOW); // its ladder, then its long
    host.tick(NOW); // its long again (the ladder is up as asked)
    bot.set("spread_bps", "200").unwrap();
    host.update("both", &bot.rows, bot.budget_atoms()).unwrap(); // the old ladder's cancel
    host.tick(NOW); // a long, then the new ladder (its rows restarted after the script)
    let sent = host.drain();
    assert_eq!(sent.len(), 4, "{sent:?}");
    for p in &sent[..3] {
        assert_eq!(p.intent, one_lot());
    }
    assert_eq!(quote(&sent[3]).0, 200);
}

// --- Bot wallets ----------------------------------------------------------------

#[test]
fn a_start_funds_the_wallet_up_to_its_budget() {
    use wallet::{Funding, SOL_FLOAT, funding};
    // A new wallet: the whole budget and the SOL float.
    assert_eq!(
        funding(100_000_000, 0, 0, 0),
        Funding {
            usdc: 100_000_000,
            lamports: SOL_FLOAT
        }
    );
    // Its wallet's USDC and its free cash in the market count.
    assert_eq!(
        funding(100_000_000, 30_000_000, 50_000_000, SOL_FLOAT).usdc,
        20_000_000
    );
    // Enough already: nothing moves (a lower budget moves nothing back).
    assert!(funding(100_000_000, 60_000_000, 70_000_000, SOL_FLOAT + 1).none());
    assert_eq!(funding(0, 0, 0, 5_000_000).lamports, SOL_FLOAT - 5_000_000);
}

#[test]
fn the_funding_and_sweep_plans() {
    use crate::futures::txbuild::{SYSTEM_PROGRAM, ata};
    let (main, bot, mint) = (
        Pubkey::new_unique(),
        Pubkey::new_unique(),
        Pubkey::new_unique(),
    );
    let f = wallet::funding(100_000_000, 0, 0, 0);
    let plan = wallet::funding_plan(&main, &bot, &mint, f, false);
    assert_eq!(
        plan.names(),
        ["computeBudget", "createAta", "transfer", "transferSol"]
    );
    let ix = plan.instructions();
    // The bot's USDC account, paid for by the main wallet.
    assert_eq!(ix[1].accounts[0].pubkey, main);
    assert_eq!(ix[1].accounts[1].pubkey, ata(&bot, &mint));
    // USDC from the main wallet's account to the bot's, signed by the main.
    let t = &ix[2];
    assert_eq!(t.program_id, spl_token::id());
    assert_eq!(
        (
            t.accounts[0].pubkey,
            t.accounts[1].pubkey,
            t.accounts[2].pubkey
        ),
        (ata(&main, &mint), ata(&bot, &mint), main)
    );
    assert!(t.accounts[2].is_signer);
    assert_eq!(t.data[0], 3);
    assert_eq!(t.data[1..9], 100_000_000u64.to_le_bytes());
    // SOL: a system transfer of the float.
    let s = &ix[3];
    assert_eq!(s.program_id, SYSTEM_PROGRAM);
    assert_eq!(s.data[..4], 2u32.to_le_bytes());
    assert_eq!(s.data[4..], wallet::SOL_FLOAT.to_le_bytes());
    assert_eq!((s.accounts[0].pubkey, s.accounts[1].pubkey), (main, bot));
    // A wallet with its USDC account and its SOL gets only what it lacks.
    let plan = wallet::funding_plan(
        &main,
        &bot,
        &mint,
        wallet::Funding {
            usdc: 5,
            lamports: 0,
        },
        true,
    );
    assert_eq!(plan.names(), ["computeBudget", "transfer"]);
    // Stopping: all of its USDC back to the main wallet, signed by the bot.
    assert!(wallet::sweep_plan(&bot, &main, &mint, 0, true).is_none());
    let plan = wallet::sweep_plan(&bot, &main, &mint, 42, true).unwrap();
    assert_eq!(plan.names(), ["computeBudget", "transfer"]);
    let t = &plan.instructions()[1];
    assert_eq!(
        (
            t.accounts[0].pubkey,
            t.accounts[1].pubkey,
            t.accounts[2].pubkey
        ),
        (ata(&bot, &mint), ata(&main, &mint), bot)
    );
    assert_eq!(t.data[1..9], 42u64.to_le_bytes());
    let plan = wallet::sweep_plan(&bot, &main, &mint, 42, false).unwrap();
    assert_eq!(plan.names(), ["computeBudget", "createAta", "transfer"]);
}

#[test]
fn a_bot_wallet_is_private_and_stays() {
    let folder = std::env::temp_dir().join(format!("petri-bots-wallet-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    crate::petri_config::create_private_dir_all(&folder).unwrap();
    assert_eq!(wallet::pubkey(&folder, "dip"), None);
    let (path, key) = wallet::open(&folder, "dip").unwrap();
    assert_eq!(path, folder.join(".wallets").join("dip.json"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&folder.join(".wallets")), 0o700);
        assert_eq!(mode(&path), 0o600);
    }
    // Opened again, it is the same wallet.
    assert_eq!(wallet::open(&folder, "dip").unwrap().1, key);
    assert_eq!(wallet::pubkey(&folder, "dip"), Some(key));
    // Its sends sign with it; everything else is petri's.
    use clap::Parser;
    let cli = crate::cli::Cli::parse_from(["petri", "--profile", "dev"]);
    let main = crate::futures::settings::Settings::merge(&cli, &Default::default());
    let s = wallet::settings(&main, &path);
    assert!(s.keypair.0.ends_with("dip.json"), "{}", s.keypair.0);
    assert_eq!((s.api_url, s.rpc_url), (main.api_url, main.rpc_url));
    std::fs::remove_dir_all(&folder).unwrap();
}

#[test]
fn a_bots_position_is_read_from_the_book() {
    let owner = Pubkey::new_unique();
    let mut book = market(owner, 7_000_000);
    let h = book.header;
    let mut t = Trader::read(&book.data, h.trader_off(0));
    t.reserved = 3_000_000;
    t.write(&mut book.data, h.trader_off(0));
    let claims = [
        (0u8, 90u16, 120u16, 2u64, 1_000_000u64),
        (1, 95, 105, 4, 500_000),
    ];
    for (i, (side, lo, hi, lots, cost)) in claims.into_iter().enumerate() {
        Claim {
            lots,
            seq: 1,
            trader: 0,
            side,
            lo,
            hi,
            cost,
            ..Claim::default()
        }
        .write(&mut book.data, h.claim_off(i as u32));
    }
    BookOrder {
        seq: 3,
        trader: 0,
        lots: 1,
        lo: 90,
        hi: 120,
        ..BookOrder::default()
    }
    .write(&mut book.data, h.order_off(0));
    let book = Book::decode(&book.data).unwrap();
    // The index at 100.
    let p = send::position_of(&book, &owner, 25_000_000, Some(100.0));
    assert_eq!(
        (p.owner, p.free, p.cash, p.wallet),
        (owner, 7_000_000, 10_000_000, 25_000_000)
    );
    assert_eq!((p.orders, p.ladder, p.inventory), (1, false, 0));
    assert_eq!(p.holdings.len(), 2);
    let long = &p.holdings[0];
    assert_eq!(
        (long.side, long.low.as_str(), long.high.as_str()),
        (Side::Long, "90", "120")
    );
    // A long 90–120 at 100 pays 10 steps of k a lot; a short 95–105, 5.
    assert_eq!(
        (long.lots, long.cost, long.value),
        (2, 1_000_000, 2 * 10 * 10_000)
    );
    let short = &p.holdings[1];
    assert_eq!((short.side, short.value), (Side::Short, 4 * 5 * 10_000));
    // Another wallet holds nothing here.
    let other = send::position_of(&book, &Pubkey::new_unique(), 0, Some(100.0));
    assert!(other.holdings.is_empty() && other.orders == 0 && other.cash == 0);
}

#[test]
fn other_months_come_from_the_wallets_account() {
    use crate::futures::api::{Account, AccountMarket, ClaimView};
    let claim = |side: &str| ClaimView {
        side: side.into(),
        lo: 90,
        hi: 120,
        lo_value: "90".into(),
        hi_value: "120".into(),
        lots: 2,
        cost: "1000000".into(),
        payout_at_index: Some("1500000".into()),
        ..ClaimView::default()
    };
    let account = Account {
        markets: vec![
            AccountMarket {
                market: MARKET.into(),
                symbol: "RAMX".into(),
                label: "RAMX Nov 26".into(),
                claims: vec![claim("long")],
                ..AccountMarket::default()
            },
            AccountMarket {
                market: NEXT.into(),
                symbol: "RAMX".into(),
                label: "RAMX Oct 26".into(),
                claims: vec![claim("short"), claim("long")],
                ..AccountMarket::default()
            },
        ],
        ..Account::default()
    };
    let others = send::elsewhere(&account, MARKET);
    assert_eq!(others.len(), 2);
    assert_eq!(others[0].0, "ramx-oct26");
    assert_eq!(
        (others[0].1.side, others[0].1.cost, others[0].1.value),
        (Side::Short, 1_000_000, 1_500_000)
    );
    assert_eq!(send::elsewhere(&account, "").len(), 3);
}

#[test]
fn a_close_narrows_by_range_and_side() {
    use crate::futures::{api::ClaimView, shape::Grid};
    let grid = Grid {
        grid: 200,
        k: 10_000,
        step: 1_000_000,
        decimals: 6,
    };
    let claim = ClaimView {
        side: "long".into(),
        lo: 90,
        hi: 120,
        ..ClaimView::default()
    };
    assert!(send::closes(&Closing::default(), &grid, &claim));
    let side = |s| Closing {
        side: Some(s),
        ..Closing::default()
    };
    assert!(send::closes(&side(Side::Long), &grid, &claim));
    assert!(!send::closes(&side(Side::Short), &grid, &claim));
    let range = |r: &str| Closing {
        range: Some(r.into()),
        ..Closing::default()
    };
    assert!(send::closes(&range("90-120"), &grid, &claim));
    assert!(!send::closes(&range("90-121"), &grid, &claim));
    assert!(!send::closes(&range("nonsense"), &grid, &claim));
}

#[test]
fn the_history_keeps_a_sample_a_minute_for_a_day() {
    use crate::futures::api::IndexPoint;
    let now = 2_000_000_000;
    // The API's samples, about every 30 s over two days, newest first.
    let points: Vec<IndexPoint> = (0..2 * 2_880)
        .map(|i| IndexPoint {
            t: now - i * 30,
            value: 100.0 + i as f64,
        })
        .collect();
    let history = send::warm(&points, now);
    assert_eq!(history.len(), 1_441);
    assert!(history.windows(2).all(|w| w[1].t - w[0].t == 60));
    assert_eq!(history[0].t, now - 86_400);
    assert_eq!(history.last().unwrap().t, now);
    // Its own reads: one a minute; older than a day drops off.
    let mut h = history;
    send::record(
        &mut h,
        Sample {
            t: now + 30,
            v: 1.0,
        },
    );
    assert_eq!(h.last().unwrap().t, now);
    send::record(
        &mut h,
        Sample {
            t: now + 60,
            v: 1.0,
        },
    );
    assert_eq!(h.last().unwrap().t, now + 60);
    assert_eq!(h[0].t, now + 60 - 86_400);
}

// --- The registry ---------------------------------------------------------------

#[test]
fn what_the_registry_signatures_sign() {
    assert_eq!(
        registry::publish_message("03f3ae"),
        "amoeba bot publish v1\n03f3ae"
    );
    assert_eq!(
        registry::run_message("Wallet1", "Owner1", Some("dip-buyer"), "03f3ae"),
        "amoeba bot run v1\nWallet1\nOwner1\ndip-buyer\n03f3ae"
    );
    // An unpublished bot names no bot.
    assert_eq!(
        registry::run_message("Wallet1", "Owner1", None, "03f3ae"),
        "amoeba bot run v1\nWallet1\nOwner1\n\n03f3ae"
    );
}

#[test]
fn github_files_and_their_defaults() {
    let g = Github::parse("github:amoeba-farm/nand-hold").unwrap();
    assert_eq!(g.spec(), "amoeba-farm/nand-hold/petri-bot.toml@HEAD");
    assert_eq!(g.from(), "github:amoeba-farm/nand-hold/petri-bot.toml@HEAD");
    assert_eq!(
        g.raw_url(),
        "https://raw.githubusercontent.com/amoeba-farm/nand-hold/HEAD/petri-bot.toml"
    );
    let g = Github::parse("me/bots/nand/hold.toml@v1.2").unwrap();
    assert_eq!(
        (
            g.owner.as_str(),
            g.repo.as_str(),
            g.path.as_str(),
            g.reference.as_str()
        ),
        ("me", "bots", "nand/hold.toml", "v1.2")
    );
    assert_eq!(
        g.raw_url(),
        "https://raw.githubusercontent.com/me/bots/v1.2/nand/hold.toml"
    );
    assert_eq!(
        Github::parse("me/bots@feature/roll").unwrap().reference,
        "feature/roll"
    );
    for bad in [
        "me",
        "github:me",
        "me/",
        "/bots",
        "me/bots/../x",
        "me/bots@",
        "me/bo ts",
        "me/bots/a?b=1",
        "me/bots@x#y",
    ] {
        assert!(Github::parse(bad).is_err(), "{bad}");
    }
}
