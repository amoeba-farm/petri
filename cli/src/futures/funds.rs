//! DEV-ONLY: REMOVE FOR PROD. The dev faucet: test USDC from the futures
//! API, and SOL from a local validator's airdrop. Compiled only with the
//! `dev-faucet` feature (the dev build); a release build has no `petri faucet`.

use serde_json::{Value, json};

use super::{
    Report,
    ctx::Ctx,
    error::{Error, Result, bail},
    units::{parse_decimal, usdc_str},
    view::Paint,
};

/// `petri faucet [owner] [--sol N]` (dev clusters)
pub fn faucet(ctx: &Ctx, p: Paint, owner: Option<&str>, sol: Option<&str>) -> Result<Report> {
    let owner = ctx.owner(owner)?;
    let mut lines = Vec::new();
    let mut payload = json!({"ok": true, "command": "faucet", "owner": owner.to_string()});
    let minted = ctx.api.faucet(&owner.to_string())?;
    lines.push(format!(
        "{} ${} test USDC to {} · {}",
        p.green("Minted"),
        usdc_str(minted.get("amount").and_then(Value::as_str).unwrap_or("0")),
        minted
            .get("usdcAta")
            .and_then(Value::as_str)
            .unwrap_or("your token account"),
        minted
            .get("signature")
            .and_then(Value::as_str)
            .unwrap_or("")
    ));
    payload["usdc"] = minted;
    if let Some(sol) = sol {
        let lamports =
            u64::try_from(parse_decimal(sol, 9)?).map_err(|_| Error::msg("too much SOL"))?;
        let signature = ctx.rpc.request_airdrop(&owner, lamports)?;
        let (_, last_valid) = ctx.rpc.latest_blockhash()?;
        if let Some(err) = ctx.rpc.confirm(&signature, last_valid)? {
            bail(format!("the airdrop failed: {err}"))?;
        }
        lines.push(format!("{} {sol} SOL · {signature}", p.green("Airdropped")));
        payload["sol"] =
            json!({"lamports": lamports.to_string(), "signature": signature.to_string()});
    }
    lines.push(p.dim("Next: petri markets · petri long ramx 100"));
    Ok(Report::new(payload, lines.join("\n")))
}
