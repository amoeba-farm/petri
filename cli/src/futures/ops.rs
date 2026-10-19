//! Wallet creation and `petri status`: the cluster, clock, deployment,
//! programs (futures and oracle), futures API, markets and wallet.

use std::path::PathBuf;

use serde_json::{Value, json};
use solana_keypair::Keypair;
use solana_signer::Signer;

use super::{
    Report,
    ctx::Ctx,
    error::{Error, Result, bail},
    units::{short, sol},
    view::{Paint, Table, span, utc},
};
use crate::{petri_config, solana_config};

/// `petri wallet new`: a new keypair at Petri's wallet path, saved as the
/// signing wallet; with `--out`, written there and left unsaved.
pub fn wallet_new(_ctx: &Ctx, p: Paint, out: Option<&PathBuf>, force: bool) -> Result<Report> {
    let make_default = out.is_none();
    let path = match out {
        Some(path) => path.clone(),
        None => petri_config::config_path()
            .map_err(Error::msg)?
            .parent()
            .map(|d| d.join("futures-id.json"))
            .ok_or_else(|| Error::msg("no Petri config directory"))?,
    };
    if path.exists() && !force {
        bail(format!(
            "{} already exists; pass --force to replace it (its key is lost)",
            path.display()
        ))?;
    }
    // write_private_file makes any missing folders, for the owner alone; a
    // folder made here first, with the umask's mode, it refuses as not private.
    let keypair = Keypair::new();
    let bytes = serde_json::to_vec(&keypair.to_bytes().to_vec()).expect("keypair encodes");
    petri_config::write_private_file(&path, &bytes).map_err(Error::msg)?;
    // Windows' canonical form, `\\?\C:\…`, is one the keypair checks refuse.
    let path = std::fs::canonicalize(&path)
        .map(|p| PathBuf::from(solana_config::plain_disk_path(&p.to_string_lossy())))
        .unwrap_or(path);
    if make_default {
        petri_config::set_futures_value(petri_config::FuturesKey::Keypair, &path.to_string_lossy())
            .map_err(Error::msg)?;
    }
    let mut lines = vec![
        format!("{} {}", p.green("New wallet"), keypair.pubkey()),
        format!(
            "Saved to {} (private: never share this file)",
            path.display()
        ),
    ];
    if make_default {
        lines.push("Petri signs with it from now on.".into());
        // DEV-ONLY: REMOVE FOR PROD (the dev build's `petri faucet`).
        if cfg!(feature = "dev-faucet") {
            lines.push(p.dim("Next: petri faucet --sol 1"));
        }
    } else {
        lines.push(p.dim(&format!(
            "Sign with it: --keypair {} or `petri config set keypair {}`",
            path.display(),
            path.display()
        )));
    }
    Ok(Report::new(
        json!({"ok": true, "command": "wallet new", "pubkey": keypair.pubkey().to_string(),
               "path": path.display().to_string(), "default": make_default}),
        lines.join("\n"),
    ))
}

/// `petri wallet address`: the attached wallet's public key.
pub fn wallet_address(ctx: &Ctx, _p: Paint) -> Result<Report> {
    let pubkey = ctx.wallet_pubkey()?;
    Ok(Report::new(
        json!({"pubkey": pubkey.to_string()}),
        pubkey.to_string(),
    ))
}

fn check(ok: bool) -> &'static str {
    if ok { "ok" } else { "MISSING" }
}

/// `petri status`
pub fn status(ctx: &Ctx, p: Paint) -> Result<Report> {
    let mut lines = vec![p.bold("Status")];
    let mut payload = json!({"ok": true, "command": "status"});
    let row = |name: &str, text: String| format!("  {name:<11} {text}");

    // Profile.
    let (profile, source) = ctx.s.profile;
    lines.push(row(
        "Profile",
        format!(
            "{} · {} ({})",
            profile.name(),
            profile.cluster(),
            source.word()
        ),
    ));
    payload["profile"] = json!({"name": profile.name(), "cluster": profile.cluster(),
        "source": source.word()});

    // Cluster and clock.
    let genesis = ctx.rpc.genesis_hash();
    match &genesis {
        Ok(genesis) => {
            let version = ctx.rpc.version().unwrap_or_else(|_| "?".into());
            let slot = ctx.rpc.slot().unwrap_or(0);
            let height = ctx.rpc.block_height().unwrap_or(0);
            let clock = ctx.rpc.clock()?;
            let wall = chrono::Utc::now().timestamp();
            let lag = wall - clock.unix_timestamp;
            lines.push(row(
                "Cluster",
                format!(
                    "{} · solana {version} · genesis {} · slot {slot}",
                    ctx.rpc.url(),
                    short(genesis)
                ),
            ));
            lines.push(row(
                "Clock",
                format!(
                    "{} · {}",
                    utc(clock.unix_timestamp),
                    if lag.abs() < 60 {
                        "in step with the wall clock".to_string()
                    } else if lag > 0 {
                        format!(
                            "{} behind the wall clock; expiries follow the cluster",
                            span(lag)
                        )
                    } else {
                        format!(
                            "{} ahead of the wall clock; expiries follow the cluster",
                            span(-lag)
                        )
                    }
                ),
            ));
            payload["cluster"] = json!({"rpc": ctx.rpc.url(), "genesisHash": genesis, "version": version,
                "slot": slot, "blockHeight": height, "clockUnixTimestamp": clock.unix_timestamp,
                "wallUnixTimestamp": wall, "clockLagSeconds": lag});
        }
        Err(e) => {
            lines.push(row(
                "Cluster",
                format!("{} · unreachable: {e}", ctx.rpc.url()),
            ));
            payload["cluster"] = json!({"rpc": ctx.rpc.url(), "error": e.message});
        }
    }

    // Futures API.
    match (ctx.api.health(), ctx.api.config()) {
        (Ok(health), Ok(config)) => {
            let same = ctx
                .deployment()
                .ok()
                .is_some_and(|d| super::exec::same_programs(d, &config.programs));
            lines.push(row(
                "API",
                format!(
                    "{} · {} · indexer slot {} · programs {}",
                    ctx.api.base(),
                    if health.get("ok").and_then(Value::as_bool).unwrap_or(false) {
                        "ok"
                    } else {
                        "NOT ok"
                    },
                    health.get("slot").and_then(Value::as_u64).unwrap_or(0),
                    if same {
                        "match"
                    } else {
                        "DIFFER from the deployment (sponsored sends are refused)"
                    }
                ),
            ));
            payload["api"] = json!({"url": ctx.api.base(), "health": health, "programsMatch": same,
                "sponsor": config.sponsor, "cluster": config.cluster});
        }
        (Err(e), _) | (_, Err(e)) => {
            lines.push(row("API", format!("{} · unavailable: {e}", ctx.api.base())));
            payload["api"] = json!({"url": ctx.api.base(), "error": e.message});
        }
    }

    // Deployment and programs.
    match ctx.deployment() {
        Ok(dep) => {
            lines.push(row("Deployment", format!("from {}", dep.source)));
            if let (Some(expected), Ok(actual)) = (&dep.genesis_hash, &genesis)
                && expected != actual
            {
                lines.push(row(
                    "",
                    format!(
                        "the deployment names genesis {} but the RPC is {}",
                        short(expected),
                        short(actual)
                    ),
                ));
            }
            let mut table = Table::new(&["PROGRAM", "ADDRESS", ""]);
            let mut list = Vec::new();
            let named = [("futures", dep.futures), ("oracle", Some(dep.oracle))];
            for (name, key) in named {
                let Some(key) = key else { continue };
                let ok = ctx
                    .rpc
                    .account(&key)
                    .ok()
                    .flatten()
                    .is_some_and(|a| a.executable);
                table.row(vec![name.into(), key.to_string(), check(ok).into()]);
                list.push(json!({"name": name, "address": key.to_string(), "executable": ok}));
            }
            let mint_ok = ctx.rpc.account(&dep.usdc_mint).ok().flatten().is_some();
            table.row(vec![
                "USDC mint".into(),
                dep.usdc_mint.to_string(),
                check(mint_ok).into(),
            ]);
            lines.extend(table.lines().into_iter().map(|l| format!("    {l}")));
            payload["programs"] = json!(list);
            match crate::chain_identity::verify_oracle(ctx) {
                Ok((identity, config)) => {
                    lines.push(row(
                        "Identity",
                        format!(
                            "verified: genesis, programs, oracle config ({})",
                            if config.paused { "paused" } else { "active" }
                        ),
                    ));
                    payload["identity"] = json!({"verified": true, "identity": identity.json(), "oraclePaused": config.paused});
                }
                Err(e) => {
                    lines.push(row("Identity", format!("NOT verified: {e}")));
                    payload["identity"] = json!({"verified": false, "error": e.message});
                }
            }
            payload["deployment"] = json!({"source": dep.source, "usdcMint": dep.usdc_mint.to_string(),
                "usdcMintExists": mint_ok, "sponsor": dep.sponsor.map(|s| s.to_string()),
                "futures": dep.futures.map(|f| f.to_string())});
        }
        Err(e) => {
            lines.push(row("Deployment", format!("unknown: {e}")));
            payload["deployment"] = json!({"error": e.message});
        }
    }

    // Markets.
    match super::api::markets(&ctx.api) {
        Ok(list) => {
            let names: Vec<String> = list
                .iter()
                .map(|m| format!("{} ({})", m.label, m.status))
                .collect();
            lines.push(row(
                "Markets",
                if names.is_empty() {
                    "none".into()
                } else {
                    names.join(", ")
                },
            ));
            payload["markets"] = json!(names);
        }
        Err(e) => lines.push(row("Markets", format!("unavailable ({e})"))),
    }

    // Wallet.
    match ctx.wallet_pubkey() {
        Ok(owner) => {
            let lamports = ctx.rpc.balance(&owner).unwrap_or(0);
            lines.push(row(
                "Wallet",
                format!(
                    "{owner} · {} SOL · sends {}",
                    sol(lamports),
                    match ctx.s.route {
                        super::args::RouteArg::Local => "locally (--route local)",
                        super::args::RouteArg::Sponsored =>
                            "through the sponsor (--route sponsored)",
                        super::args::RouteArg::Auto
                            if lamports >= super::exec::LOCAL_MIN_LAMPORTS =>
                            "locally (you pay fees)",
                        super::args::RouteArg::Auto => "through the sponsor (no SOL for fees)",
                    }
                ),
            ));
            payload["wallet"] =
                json!({"pubkey": owner.to_string(), "lamports": lamports.to_string()});
        }
        Err(e) => {
            lines.push(row("Wallet", e.message.clone()));
            payload["wallet"] = json!({"error": e.message});
        }
    }
    Ok(Report::new(payload, lines.join("\n")))
}
