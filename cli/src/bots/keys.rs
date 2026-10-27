//! The keys a bot may inject. The host provides the market's feeds and the
//! clock, the same for every bot; each bot's root provides what is the
//! bot's alone (its wallet's position, its run) inside the bot's realm; its
//! rows provide the rest there.

/// The chain's clock: one tick for every bot ([`super::feeds::Clock`]).
pub const CLOCK: &str = "clock";
/// The bot's way out ([`super::feeds::Sink`]), provided by its root.
pub const INTENTS: &str = "intents";
/// This run of the bot: its budget, its vol and the script's memory
/// ([`super::host::Run`]), provided by its root.
pub const RUN: &str = "run";
/// Its editable numbers ([`super::weights::Weights`]).
pub const WEIGHTS: &str = "weights";
/// What each step of the grid is worth ([`super::signals::Fair`]).
pub const FAIR: &str = "signal:fair";
/// How far it leans against what it holds ([`super::signals::Skew`]).
pub const SKEW: &str = "signal:skew";

/// The market's index, as the API serves it.
pub fn index(market: &str) -> String {
    format!("index:{market}")
}

/// The market account, read from chain.
pub fn book(market: &str) -> String {
    format!("book:{market}")
}

/// The bot's wallet in the market: its cash, inventory, holdings and USDC.
pub fn position(market: &str) -> String {
    format!("position:{market}")
}

/// The market's index samples of the last day ([`super::feeds::History`]).
pub fn history(market: &str) -> String {
    format!("history:{market}")
}

/// The keys the host provides, which every bot reads from the host.
const HOST: [&str; 4] = [CLOCK, "index:*", "book:*", "history:*"];

/// The keys each bot's root provides for it alone, in its realm.
const ROOT: [&str; 2] = ["position:*", RUN];

/// Whether `key` is one the host or a bot's root provides, which no row
/// may provide.
pub fn host(key: &str) -> bool {
    HOST.iter()
        .chain(ROOT.iter())
        .any(|p| crate::compose::matches(p, key))
}

/// A bot's realm: the host's keys from the host, every other key its own
/// (its root's and its rows'), so no bot reaches another's wallet, signals,
/// weights or way out.
pub fn realm(bot: &str) -> Vec<(String, String)> {
    HOST.iter()
        .map(|p| (p.to_string(), String::new()))
        .chain([("*".to_string(), format!("bot:{bot}"))])
        .collect()
}
