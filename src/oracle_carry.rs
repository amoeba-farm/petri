//! Native SDK carry-forward reads; never accepts raw accounts or maintenance packets.
use crate::{
    backend::{BackendClient, CliError},
    onchain::OnchainConfig,
};
use serde_json::{Value, json};
#[derive(Debug, clap::Args)]
pub struct Args {
    #[arg(long)]
    pub market: String,
    #[arg(long)]
    pub expiry: String,
    #[arg(
        long,
        help = "Exact source ID (64 lowercase hex); omit for period lineage only"
    )]
    pub source: Option<String>,
}
pub fn read(
    config: &OnchainConfig,
    backend: &BackendClient,
    args: &Args,
) -> Result<Value, CliError> {
    if config.backend_url != backend.base_url() {
        return Err(CliError::new("Carry read service identity mismatch"));
    }
    crate::chain_identity::verify_onchain_config_fresh(config)?;
    let market = args.market.to_ascii_lowercase();
    crate::market_surface::parse_current_series_id(&market, &args.expiry)?;
    let mut request = json!({"marketId":market,"expiryId":args.expiry});
    if let Some(source) = &args.source {
        request["sourceId"] = json!(source);
    }
    // Read-only worker identity sentinel: never asks for or loads a wallet.
    crate::sdk_worker::validate(
        config,
        "carry",
        "11111111111111111111111111111111",
        &request,
        &Value::Null,
        None,
    )
}
pub fn render(value: &Value) -> String {
    let v = &value["carry"];
    let text = |v: &Value| {
        v.as_str().map(str::to_owned).unwrap_or_else(|| {
            if v.is_null() {
                "unknown".into()
            } else {
                v.to_string()
            }
        })
    };
    let mut lines = vec![
        format!("{} — Oracle carry", text(&v["expiryId"])),
        format!("Finalized slot: {}", text(&v["observedSlot"])),
    ];
    if v["period"].is_null() {
        lines.push("No carry period registered for this series.".into());
    } else {
        lines.push(format!(
            "Predecessor: {}\nImports: {} / {}",
            text(&v["period"]["predecessor"]),
            text(&v["progress"]["importsCompleted"]),
            text(&v["progress"]["importsRequired"])
        ));
    }
    if !v["sourceId"].is_null() {
        lines.push(format!(
            "Source: {}\nState: {} | Opening: {}\nOriginal observation: {} | Accepted: {}\nCheckpoints remaining: {}\nStage to inspect (not admission): {}",
            text(&v["sourceId"]), text(&v["status"]), text(&v["openingProvenance"]),
            text(&v["originalObservedAt"]), text(&v["acceptedAt"]),
            text(&v["progress"]["checkpointsRemaining"]), text(&v["maintenance"]["candidateStage"])
        ));
    }
    lines.push("Read only. Carry maintenance has no hosted public preparation route.".into());
    lines.join("\n")
}
