//! Ameba SDK, Rust half.
//!
//! [`futures`] re-exports the futures client crate (`ameba-futures-client`,
//! linked by path from this monorepo's `futures/`): the futures market
//! (market addresses and every instruction ([`futures::Market`]), the market
//! account decoder with the exact package solver (`Book`, `Book::propose`),
//! the proposal codec, payoffs and error names),
//! [`futures::governance`] for binding instructions to the controller's gate
//! epochs, and [`futures::oracle`] for the oracle `Period` accounts.
//!
//! The TypeScript half (`ameba-sdk/futures`) is pinned to the same crate by
//! golden vectors.
pub mod futures;
