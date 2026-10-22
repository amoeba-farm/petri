//! Making web proofs (TLSNotary, MPC mode).
//!
//! - `prover`: petri's side. It connects to the source from the
//!   contributor's machine with their own session, runs MPC-TLS with a
//!   notary and builds the presentation under fixed disclosure rules.
//! - `notary`: the attesting side (`amoeba-notary`).
//! - `transport`: TCP and WebSocket links to a notary.
//! - `browser`: the contributor's own browser as the sign-in companion
//!   (Chrome DevTools Protocol on loopback): they sign in to the source
//!   there, and petri reads that site's session for the proof.
//! - `wire`: the framing around the TLSNotary session.
//!
//! Verification lives in `amoeba-evidence`, which the API and petri share.

pub mod browser;
pub mod kms;
pub mod notary;
pub mod prover;
pub mod transport;
pub mod wire;

use amoeba_evidence::bundle::{Bundle, Method};
use amoeba_evidence::manifest::{Manifest, ManifestError};
use amoeba_evidence::policy::TLSN_REVISION;

/// Packs a presentation and its manifest into a portable bundle.
pub fn bundle(
    presentation: Vec<u8>,
    manifest_json: &[u8],
    annotations: Vec<(String, String)>,
) -> Result<Bundle, ManifestError> {
    Ok(Bundle {
        method: Method::WebProof,
        backend_name: "tlsnotary".into(),
        backend_revision: TLSN_REVISION.into(),
        backend_mode: "mpc".into(),
        presentation,
        manifest: Manifest::parse(manifest_json)?,
        annotations,
    })
}

/// Proves `req` through the notary at `notary_url` and returns the bundle
/// with the timings.
pub async fn prove(
    notary_url: &str,
    req: &prover::ProveRequest,
    manifest_json: &[u8],
    annotations: Vec<(String, String)>,
) -> anyhow::Result<(Bundle, prover::Timings, u16)> {
    // Check the manifest before spending a session on it.
    Manifest::parse(manifest_json)?;
    let io = transport::connect(notary_url)
        .await
        .map_err(|e| anyhow::anyhow!("the notary at {notary_url} could not be reached: {e}"))?;
    let notarized = prover::notarize(io, req).await?;
    let presentation = prover::present(&notarized)?;
    let status = notarized.status;
    let timings = notarized.timings.clone();
    drop(notarized);
    Ok((
        bundle(presentation, manifest_json, annotations)?,
        timings,
        status,
    ))
}
