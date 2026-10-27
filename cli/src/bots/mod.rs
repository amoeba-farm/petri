//! Trading bots: built by their users (and their agents) inside petri, each
//! a sandbox on the compose runtime (`crate::compose`). A bot is a bundle
//! file ([`bundle`]): its market and a list of rows, each a part: its
//! weights, signals and a strategy. Petri runs it as a subtree with a realm
//! of its own, so it can be started, edited and stopped while petri runs,
//! leaving nothing behind, and no bot reaches another.
//!
//! **Bots propose, petri signs.** A part never sees a key, a signer or a
//! client. It reads the feeds the host provides ([`keys`]) and proposes
//! [`feeds::Intent`]s; the host checks, builds, signs and sends them
//! ([`send`]). An open ladder is the one thing stopping a bot can't simply
//! undo, so the ladder registers its own cancel, which goes out as the bot
//! stops. A strategy can also be a sandboxed script ([`script`]).
//!
//! Each running bot trades from a wallet of its own ([`wallet`]), funded
//! from petri's wallet up to its budget when it starts and swept back when
//! it stops, so its results on chain are its own: the futures API's
//! registry ([`registry`]) ranks published bots by them (`docs/bots.md`).
//!
//! The Bots view (`research/bots.rs`) lists the bundles, starts and stops
//! them and edits their weights; `petri bots` drives the same worker.

pub mod bundle;
pub mod feeds;
pub mod host;
pub mod keys;
pub mod ladder;
pub mod registry;
pub mod script;
pub mod send;
pub mod signals;
#[cfg(test)]
mod tests;
pub mod wallet;
pub mod weights;
