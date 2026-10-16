//! Futures: the futures client crate, re-exported.
//!
//! * The futures market (`ameba-futures`), the product users trade.
//!   [`Market`] derives a market's addresses and builds every instruction
//!   with its exact accounts (`submit`, `cancel`, `deposit`, `withdraw`,
//!   `quote`, `split`, `merge`, `redeem`, `transfer`, settlement, and the
//!   legacy Earn pools' `pool_withdraw` and `pool_migrate`); [`EarnVault`]
//!   builds the Earn vault's (`create`, `deposit`, `withdraw`, `fund`,
//!   `defund`, with [`operated`] for its operator's trading, and
//!   [`decode_vault`] / [`decode_vault_position`]); [`Book`] decodes the
//!   market account (traders, orders, maker ladders, claims) and solves the
//!   package an order is owed (`Book::propose`, the proposal `Submit`
//!   carries); [`payoff`],
//!   [`premium_at`] and [`entry_of`] convert units; [`error_name`] names its
//!   errors.
//! * [`governance`]: bind each instruction with
//!   [`governance::GateBinding::bind`] on a governed deployment.
//! * [`oracle`]: the oracle `Period` layout the markets settle on;
//!   [`product_id`] and [`product_symbol`] convert product symbols.
//!
//! ```no_run
//! use ameba_sdk::futures::{Book, Market, Side, SubmitArgs, MODE_IOC};
//! # fn demo(program: ameba_sdk::futures::Pubkey, market: ameba_sdk::futures::Pubkey,
//! #         data: &[u8], owner: ameba_sdk::futures::Pubkey) {
//! let book = Book::decode(data).unwrap();
//! let h = book.header;
//! let m = Market::derive(program, market, h.vault, h.period, h.quote_mint, &h.product_id, h.expiry_ts);
//! let order = SubmitArgs { side: Side::Long, lo: 86, hi: 104, lots: 3, limit: 180_000, min_fill: 1, mode: MODE_IOC, close: None };
//! let proposed = book.propose(&order.taker());
//! let ix = m.submit(&owner, &order, proposed.proposal);
//! # let _ = ix;
//! # }
//! ```
pub use ameba_futures_client::*;
pub use solana_program::{
    instruction::{AccountMeta, Instruction},
    program_error::ProgramError,
    pubkey::Pubkey,
};
