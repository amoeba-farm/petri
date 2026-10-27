//! Each bot's own wallet (`docs/bots.md`, "Bot wallets"): a Solana keypair
//! file in `bots/.wallets/` (folder 0700, file 0600), made on its first
//! start, never printed and never deleted by petri, so no funds are
//! stranded when a bundle goes. Every send of the bot is signed by it.
//!
//! On start the main wallet funds it up to its budget, with a little SOL
//! for its fees; on stop its USDC goes back. The plans are pure, so the
//! tests check them without a chain; the worker sends them.

use std::path::{Path, PathBuf};

use solana_keypair::Keypair;
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use solana_signer::Signer;

use crate::futures::{
    settings::{Settings, Source},
    txbuild::{self, Plan, SYSTEM_PROGRAM},
};

/// The SOL a bot wallet keeps for its fees, in lamports: 0.02 SOL
/// (`docs/bots.md`, "Bot wallets": "tops the bot wallet's SOL up to 0.02
/// SOL from the main wallet").
pub const SOL_FLOAT: u64 = 20_000_000;

/// Where bot `id`'s wallet lives in `folder` (petri's `bots/`).
pub fn path(folder: &Path, id: &str) -> PathBuf {
    folder.join(".wallets").join(format!("{id}.json"))
}

/// Bot `id`'s wallet's public key, if it has one yet.
pub fn pubkey(folder: &Path, id: &str) -> Option<Pubkey> {
    let path = path(folder, id);
    if !path.exists() {
        return None;
    }
    crate::wallet_signer::signer_pubkey_from_path(&path.to_string_lossy())
        .ok()?
        .parse()
        .ok()
}

/// Bot `id`'s wallet, made now if it has none: its path and public key.
pub fn open(folder: &Path, id: &str) -> Result<(PathBuf, Pubkey), String> {
    let path = path(folder, id);
    if let Some(key) = pubkey(folder, id) {
        return Ok((path, key));
    }
    if path.exists() {
        return Err(format!("{} is not a wallet petri can read", path.display()));
    }
    if crate::read_only::signing_blocked() {
        return Err("petri is read-only here (PETRI_MCP_READ_ONLY), so it makes no wallets".into());
    }
    let wallets = folder.join(".wallets");
    crate::petri_config::create_private_dir_all(&wallets)
        .map_err(|e| format!("couldn't make {}: {e}", wallets.display()))?;
    let keypair = Keypair::new();
    let bytes = serde_json::to_vec(&keypair.to_bytes().to_vec()).expect("keypair encodes");
    crate::petri_config::write_private_file(&path, &bytes)?;
    Ok((path, keypair.pubkey()))
}

/// The settings a bot's sends use: petri's, signing with the bot's wallet.
pub fn settings(main: &Settings, wallet: &Path) -> Settings {
    // Windows' canonical form, `\\?\C:\…`, is one the keypair checks refuse.
    let path = std::fs::canonicalize(wallet)
        .map(|p| crate::solana_config::plain_disk_path(&p.to_string_lossy()))
        .unwrap_or_else(|_| wallet.to_string_lossy().to_string());
    Settings {
        keypair: (path, Source::Flag),
        ..main.clone()
    }
}

/// What a start moves from the main wallet to the bot's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Funding {
    /// USDC atoms: what its wallet and free market cash lack of its budget.
    pub usdc: u64,
    /// Lamports: what its wallet lacks of [`SOL_FLOAT`].
    pub lamports: u64,
}

impl Funding {
    pub fn none(&self) -> bool {
        self.usdc == 0 && self.lamports == 0
    }
}

/// What starting a bot with `budget` moves to its wallet, holding `usdc`
/// in the wallet, `free` cash in its market and `lamports`.
pub fn funding(budget: u64, usdc: u64, free: u64, lamports: u64) -> Funding {
    Funding {
        usdc: budget.saturating_sub(usdc.saturating_add(free)),
        lamports: SOL_FLOAT.saturating_sub(lamports),
    }
}

/// `SystemProgram::Transfer`.
fn sol_transfer(from: &Pubkey, to: &Pubkey, lamports: u64) -> Instruction {
    let mut data = 2u32.to_le_bytes().to_vec();
    data.extend_from_slice(&lamports.to_le_bytes());
    Instruction {
        program_id: SYSTEM_PROGRAM,
        accounts: vec![AccountMeta::new(*from, true), AccountMeta::new(*to, false)],
        data,
    }
}

/// An SPL token transfer of `amount` from `from`'s USDC account to `to`'s.
fn usdc_transfer(from: &Pubkey, to: &Pubkey, mint: &Pubkey, amount: u64) -> Instruction {
    spl_token::instruction::transfer(
        &spl_token::id(),
        &txbuild::ata(from, mint),
        &txbuild::ata(to, mint),
        from,
        &[],
        amount,
    )
    .expect("a token transfer builds")
}

/// The main wallet's transaction funding bot wallet `bot` (it pays the fee
/// and, when the bot has no USDC account yet, the account's rent).
pub fn funding_plan(main: &Pubkey, bot: &Pubkey, mint: &Pubkey, f: Funding, ata: bool) -> Plan {
    let mut plan = Plan::new();
    if f.usdc > 0 {
        if !ata {
            plan.push("createAta", txbuild::create_ata_idempotent(main, bot, mint));
        }
        plan.push("transfer", usdc_transfer(main, bot, mint, f.usdc));
    }
    if f.lamports > 0 {
        plan.push("transferSol", sol_transfer(main, bot, f.lamports));
    }
    plan
}

/// The bot wallet's transaction moving all its `usdc` back to the main
/// wallet (making the main wallet's USDC account first if it has none).
pub fn sweep_plan(
    bot: &Pubkey,
    main: &Pubkey,
    mint: &Pubkey,
    usdc: u64,
    ata: bool,
) -> Option<Plan> {
    if usdc == 0 {
        return None;
    }
    let mut plan = Plan::new();
    if !ata {
        plan.push("createAta", txbuild::create_ata_idempotent(bot, main, mint));
    }
    plan.push("transfer", usdc_transfer(bot, main, mint, usdc));
    Some(plan)
}
