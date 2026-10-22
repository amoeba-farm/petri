//! **TEST ONLY.** Exports real web-proof bundles for the futures API's tests
//! (`api/api/fixtures/evidence/`), which run on the API's toolchain and can't
//! run the TLSNotary prover (`tlsn` needs Rust 1.95).
//!
//! It starts two fixture portals (each with a test CA generated at start-up,
//! fixture prices) and a notary with a fixed **test** signing key
//! (`[7; 32]`, the same key `e2e.rs` uses: it signs only test attestations,
//! and only a test-only `Policy::development` lists its public key), proves
//! `/api/prices` on them with a session cookie, bound to the test wallet
//! `[9; 32]`, and writes:
//!
//! - `bundle.json`: `https://portal.fixture.test/api/prices` (manifest:
//!   DDR5-16G-4800);
//! - `bundle-other-row.json`: the same presentation with a manifest for the
//!   DDR5-32G-4800 row;
//! - `bundle-public.json`: the same record on the second portal,
//!   `https://portal.fixture-distributor.example/api/prices`. The candidate rules
//!   refuse `.test` names (`api/api/src/candidates/rules.rs`,
//!   `NON_PUBLIC_TLDS`), so candidate tests need a name they accept;
//!   `.example` is reserved (RFC 2606) and never resolves;
//! - `bundle-public-query.json`: `…/api/prices?view=all` on the second
//!   portal, another locator for the same observation;
//! - `notary-public-key.hex`: the test notary's compressed secp256k1 key;
//! - `fixture-ca.der.b64`: the two portals' test CA certificates (DER,
//!   base64, one per line), which the test policy adds as development roots.
//!
//! Lines starting with `#` in the `.hex` and `.b64` files are comments. None
//! of these is a secret: a public key and test CA certificates (their
//! private keys are dropped with the portals). Run by hand, from
//! `evidence/`:
//!
//! ```bash
//! cargo test -p amoeba-webproof --test export_fixture_bundles -- --ignored
//! ```
//!
//! `EVIDENCE_FIXTURE_OUT` overrides the output folder.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use amoeba_evidence::bundle::Bundle;
use amoeba_evidence::policy::Policy;
use amoeba_evidence::{Destination, Outcome, check_destination, verify_bundle};
use amoeba_evidence_fixture as fixture;
use amoeba_webproof::notary::NotaryConfig;
use amoeba_webproof::prover::ProveRequest;
use base64::Engine as _;

const MANIFEST: &str = r#"{
  "format": "amoeba-extraction-manifest-v1",
  "name": "Fixture distributor: DDR5 16GB module (TEST ONLY)",
  "evidenceType": "catalog_price",
  "origin": "portal.fixture.test",
  "path": "/api/prices",
  "content": "json",
  "records": "/items",
  "match": { "/sku": "DDR5-16G-4800" },
  "fields": {
    "price": "/unit_price", "currency": "/currency", "unit": "/unit", "product": "/sku",
    "productName": "/name", "priceType": "/price_type", "quantityTier": "/min_qty",
    "recordId": "/id", "revision": "/revision", "observedAt": "/updated_at"
  },
  "context": ["/region"]
}"#;

const PUBLIC_DOMAIN: &str = "portal.fixture-distributor.example";
/// TEST ONLY notary signing key (see the module docs).
const TEST_NOTARY_KEY: [u8; 32] = [7; 32];
/// The test wallet the proofs are bound to.
const TEST_WALLET: [u8; 32] = [9; 32];

fn out_dir() -> PathBuf {
    std::env::var_os("EVIDENCE_FIXTURE_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../api/api/fixtures/evidence")
        })
}

async fn prove(notary: SocketAddr, portal: &fixture::Fixture, url: &str, manifest: &str) -> Bundle {
    let mut req = ProveRequest::new(url);
    req.connect_to = Some(portal.addr.to_string());
    req.extra_roots = vec![portal.ca_der.clone()];
    req.binding = Some(TEST_WALLET);
    req.headers.push(("Cookie".into(), portal.sign_in()));
    let (bundle, _, status) = amoeba_webproof::prove(
        &format!("tcp://{notary}"),
        &req,
        manifest.as_bytes(),
        vec![(
            "note".into(),
            "TEST ONLY: fixture portal and test notary, not market data".into(),
        )],
    )
    .await
    .unwrap();
    assert_eq!(status, 200);
    bundle
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "TEST ONLY: writes the futures API's evidence fixtures; run by hand"]
async fn export_fixture_bundles() {
    let portal = fixture::start("127.0.0.1:0".parse().unwrap(), fixture::Config::default())
        .await
        .unwrap();
    let public = fixture::start(
        "127.0.0.1:0".parse().unwrap(),
        fixture::Config {
            domain: PUBLIC_DOMAIN.into(),
            ..fixture::Config::default()
        },
    )
    .await
    .unwrap();
    let cfg = Arc::new(NotaryConfig::new(TEST_NOTARY_KEY));
    let notary_key = cfg.public_key().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let notary = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (s, _) = listener.accept().await.unwrap();
            s.set_nodelay(true).unwrap();
            let cfg = cfg.clone();
            tokio::spawn(async move {
                use tokio_util::compat::TokioAsyncReadCompatExt;
                let _ = amoeba_webproof::notary::serve(s.compat(), &cfg).await;
            });
        }
    });

    let bundle = prove(
        notary,
        &portal,
        "https://portal.fixture.test/api/prices",
        MANIFEST,
    )
    .await;
    let mut other = bundle.clone();
    other.manifest = amoeba_evidence::manifest::Manifest::parse(
        MANIFEST
            .replace("DDR5-16G-4800", "DDR5-32G-4800")
            .replace("DDR5 16GB", "DDR5 32GB")
            .as_bytes(),
    )
    .unwrap();
    let public_manifest = MANIFEST.replace("portal.fixture.test", PUBLIC_DOMAIN);
    let public_url = format!("https://{PUBLIC_DOMAIN}/api/prices");
    let on_public = prove(notary, &public, &public_url, &public_manifest).await;
    let on_public_query = prove(
        notary,
        &public,
        &format!("{public_url}?view=all"),
        &public_manifest,
    )
    .await;

    // Check what we export before writing it.
    let policy = Policy::development(
        "test notary",
        notary_key.clone(),
        vec![portal.ca_der.clone(), public.ca_der.clone()],
    );
    let mut observations = Vec::new();
    for (b, url, atoms) in [
        (
            &bundle,
            "https://portal.fixture.test/api/prices".to_string(),
            41_250_000,
        ),
        (
            &other,
            "https://portal.fixture.test/api/prices".to_string(),
            79_900_000,
        ),
        (&on_public, public_url.clone(), 41_250_000),
        (
            &on_public_query,
            format!("{public_url}?view=all"),
            41_250_000,
        ),
    ] {
        let mut v = verify_bundle(b.to_json().as_bytes(), &policy);
        assert!(v.authenticated(), "{:#?}", v.stages);
        let env = v.envelope.as_ref().unwrap();
        assert_eq!(env.observation.price.atoms, atoms);
        observations.push(env.observation_id.clone());
        check_destination(
            &mut v,
            &Destination::Candidate {
                market: "RAMX".into(),
                expiry: "RAMX-202612".into(),
                bucket: "b1".into(),
                canonical_url: url,
                submitter: Some(TEST_WALLET),
            },
        );
        assert_eq!(v.stages.eligibility.outcome, Outcome::Passed);
    }
    // Two locators of one record are one observation.
    assert_eq!(observations[2], observations[3]);

    let dir = out_dir();
    std::fs::create_dir_all(&dir).unwrap();
    for (name, b) in [
        ("bundle.json", &bundle),
        ("bundle-other-row.json", &other),
        ("bundle-public.json", &on_public),
        ("bundle-public-query.json", &on_public_query),
    ] {
        std::fs::write(dir.join(name), b.to_json()).unwrap();
    }
    let source = "# from evidence/crates/webproof/tests/export_fixture_bundles.rs";
    std::fs::write(
        dir.join("notary-public-key.hex"),
        format!(
            "# TEST ONLY: the test notary's public key (secp256k1, compressed)\n{source}\n{}\n",
            hex::encode(&notary_key)
        ),
    )
    .unwrap();
    let b64 = |der: &[u8]| base64::engine::general_purpose::STANDARD.encode(der);
    std::fs::write(
        dir.join("fixture-ca.der.b64"),
        format!(
            "# TEST ONLY: the fixture portals' test CA certificates (DER, base64), one per line:\n\
             # portal.fixture.test, then {PUBLIC_DOMAIN}\n{source}\n{}\n{}\n",
            b64(&portal.ca_der),
            b64(&public.ca_der)
        ),
    )
    .unwrap();
    eprintln!("wrote the evidence fixtures to {}", dir.display());
}
