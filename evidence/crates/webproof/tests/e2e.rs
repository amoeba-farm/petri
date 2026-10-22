//! Real web proofs, end to end: the signed-in fixture portal (test CA,
//! labelled fixture prices), a real notary over TCP, the prover with the
//! portal's session cookie, then independent verification with
//! `amoeba_evidence::verify_bundle`. No step is mocked; the only test
//! scaffolding is the development trust root and the development notary
//! key, and every result says so.

use std::net::SocketAddr;
use std::sync::Arc;

use amoeba_evidence::policy::{Policy, Trust};
use amoeba_evidence::tlsn::ServerTrust;
use amoeba_evidence::{Destination, Outcome, check_destination, verify_bundle};
use amoeba_evidence_fixture as fixture;
use amoeba_webproof::notary::NotaryConfig;
use amoeba_webproof::prover::ProveRequest;

const MANIFEST: &str = r#"{
  "format": "amoeba-extraction-manifest-v1",
  "name": "Fixture distributor: DDR5 16GB module",
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

const WALLET: [u8; 32] = [9; 32];

struct World {
    portal: fixture::Fixture,
    notary_addr: SocketAddr,
    notary_key: Vec<u8>,
}

async fn world() -> World {
    let portal = fixture::start("127.0.0.1:0".parse().unwrap(), fixture::Config::default())
        .await
        .unwrap();
    let cfg = Arc::new(NotaryConfig::new([7; 32]));
    let notary_key = cfg.public_key().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let notary_addr = listener.local_addr().unwrap();
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
    World {
        portal,
        notary_addr,
        notary_key,
    }
}

impl World {
    fn request(&self, cookie: Option<&str>) -> ProveRequest {
        let mut r = ProveRequest::new("https://portal.fixture.test/api/prices");
        r.connect_to = Some(self.portal.addr.to_string());
        r.extra_roots = vec![self.portal.ca_der.clone()];
        r.binding = Some(WALLET);
        if let Some(c) = cookie {
            r.headers.push(("Cookie".into(), c.into()));
        }
        r
    }
    fn policy(&self) -> Policy {
        Policy::development(
            "test notary",
            self.notary_key.clone(),
            vec![self.portal.ca_der.clone()],
        )
    }
    async fn prove(
        &self,
        req: &ProveRequest,
        manifest: &str,
    ) -> (amoeba_evidence::bundle::Bundle, u16) {
        let (bundle, timings, status) = amoeba_webproof::prove(
            &format!("tcp://{}", self.notary_addr),
            req,
            manifest.as_bytes(),
            vec![("note".into(), "e2e test".into())],
        )
        .await
        .unwrap();
        eprintln!("proof timings: {timings:?}");
        (bundle, status)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn signed_in_page_proves_and_verifies() {
    let w = world().await;
    let cookie = w.portal.sign_in();
    let (bundle, status) = w.prove(&w.request(Some(&cookie)), MANIFEST).await;
    assert_eq!(status, 200);
    let json = bundle.to_json();

    // The session cookie never leaves the TLS session.
    let secret = cookie.strip_prefix("session=").unwrap();
    assert!(
        !json.contains(secret),
        "the bundle leaks the session cookie"
    );
    let presentation_text = String::from_utf8_lossy(&bundle.presentation);
    assert!(!presentation_text.contains(secret));

    // Independent verification from the bundle bytes alone.
    let policy = w.policy();
    let v = verify_bundle(json.as_bytes(), &policy);
    assert!(v.authenticated(), "{:#?}", v.stages);
    let env = v.envelope.clone().unwrap();
    assert_eq!(env.observation.price.atoms, 41_250_000);
    assert_eq!(env.observation.price.text, "41.25");
    assert_eq!(env.observation.fields["currency"].value, "USD");
    assert_eq!(env.origin.locator, "https://portal.fixture.test/api/prices");
    assert_eq!(env.origin.server_name, "portal.fixture.test");
    assert_eq!(env.authentication.trust, Trust::Development);
    assert_eq!(
        env.authentication.server_trust,
        ServerTrust::DevelopmentRoot
    );
    assert_eq!(
        env.authentication.bound_to.as_deref(),
        Some(hex::encode(WALLET).as_str())
    );
    assert_eq!(env.annotations["note"], "e2e test");
    let d = v.disclosure.clone().unwrap();
    assert!(
        d.hidden_request_headers
            .iter()
            .any(|h| h.eq_ignore_ascii_case("cookie"))
    );
    assert!(!d.request.contains(secret));
    assert!(d.body.contains(fixture::NOTICE));

    // The bundle round-trips with the same digest.
    let again = amoeba_evidence::bundle::Bundle::parse(json.as_bytes(), 1 << 22).unwrap();
    assert_eq!(again.digest(), bundle.digest());

    // Eligibility per destination.
    let candidate = |url: &str, who: [u8; 32]| Destination::Candidate {
        market: "RAMX".into(),
        expiry: "2026-12".into(),
        bucket: "b1".into(),
        canonical_url: url.into(),
        submitter: Some(who),
    };
    let mut ok = v.clone();
    check_destination(
        &mut ok,
        &candidate("https://portal.fixture.test/api/prices", WALLET),
    );
    assert_eq!(
        ok.stages.eligibility.outcome,
        Outcome::Passed,
        "{:?}",
        ok.stages.eligibility
    );
    assert!(ok.envelope.as_ref().unwrap().submission_id.is_some());
    let mut other_url = v.clone();
    check_destination(
        &mut other_url,
        &candidate("https://portal.fixture.test/api/other", WALLET),
    );
    assert_eq!(other_url.stages.eligibility.outcome, Outcome::Failed);
    let mut other_wallet = v.clone();
    check_destination(
        &mut other_wallet,
        &candidate("https://portal.fixture.test/api/prices", [1; 32]),
    );
    assert_eq!(other_wallet.stages.eligibility.outcome, Outcome::Failed);
    let mut frozen = v.clone();
    check_destination(
        &mut frozen,
        &Destination::Update {
            oracle_program: "prog".into(),
            month: "month".into(),
            source_id: [3; 32],
            locator: "https://portal.fixture.test/api/prices".into(),
            selector_digest: Some([0; 32]),
            claimant: Some(WALLET),
        },
    );
    assert_eq!(frozen.stages.eligibility.outcome, Outcome::Failed);

    // A quotation is recorded as one, and is not admitted as a source.
    let mut quote = bundle.clone();
    quote.manifest = amoeba_evidence::manifest::Manifest::parse(
        MANIFEST.replace("catalog_price", "quotation").as_bytes(),
    )
    .unwrap();
    let mut qv = verify_bundle(quote.to_json().as_bytes(), &policy);
    assert!(qv.authenticated());
    check_destination(
        &mut qv,
        &candidate("https://portal.fixture.test/api/prices", WALLET),
    );
    assert_eq!(qv.stages.eligibility.outcome, Outcome::Failed);
    assert!(
        qv.stages
            .eligibility
            .reason
            .as_deref()
            .unwrap()
            .contains("quotation")
    );

    // Tampering: one flipped byte anywhere in the presentation fails.
    for at in [
        10,
        bundle.presentation.len() / 2,
        bundle.presentation.len() - 5,
    ] {
        let mut t = bundle.clone();
        t.presentation[at] ^= 1;
        let tv = verify_bundle(t.to_json().as_bytes(), &policy);
        assert!(!tv.authenticated(), "byte {at} flipped still verified");
    }

    // A notary outside the policy, or the production policy, is refused.
    let stranger = Policy::development(
        "other",
        NotaryConfig::new([8; 32]).public_key().unwrap(),
        vec![w.portal.ca_der.clone()],
    );
    let sv = verify_bundle(json.as_bytes(), &stranger);
    assert_eq!(sv.stages.provenance.outcome, Outcome::Failed);
    assert!(
        sv.stages
            .provenance
            .reason
            .as_deref()
            .unwrap()
            .contains("not a notary this verifier accepts")
    );
    assert_eq!(
        verify_bundle(json.as_bytes(), &Policy::production())
            .stages
            .provenance
            .outcome,
        Outcome::Failed
    );
    // Without the development root the test certificate does not verify.
    let no_root = Policy::development("test notary", w.notary_key.clone(), vec![]);
    assert_eq!(
        verify_bundle(json.as_bytes(), &no_root)
            .stages
            .provenance
            .outcome,
        Outcome::Failed
    );

    // A forged verification flag in the bundle is refused, not trusted.
    let forged = json.replacen("{\n", "{\n  \"verified\": true,\n", 1);
    let fv = verify_bundle(forged.as_bytes(), &policy);
    assert_eq!(fv.stages.received.outcome, Outcome::Failed);

    // The same proof, another manifest: another row of the same record set
    // is a different observation; a manifest for another origin fails.
    let mut row = bundle.clone();
    row.manifest = amoeba_evidence::manifest::Manifest::parse(
        MANIFEST
            .replace("DDR5-16G-4800", "DDR5-32G-4800")
            .as_bytes(),
    )
    .unwrap();
    let rv = verify_bundle(row.to_json().as_bytes(), &policy);
    assert!(rv.authenticated());
    assert_eq!(
        rv.envelope.as_ref().unwrap().observation.price.atoms,
        79_900_000
    );
    assert_ne!(
        rv.envelope.as_ref().unwrap().observation_id,
        env.observation_id
    );
    let mut elsewhere = bundle.clone();
    elsewhere.manifest = amoeba_evidence::manifest::Manifest::parse(
        MANIFEST
            .replace("portal.fixture.test", "prices.fixture.test")
            .as_bytes(),
    )
    .unwrap();
    let ev = verify_bundle(elsewhere.to_json().as_bytes(), &policy);
    assert_eq!(ev.stages.extraction.outcome, Outcome::Failed);

    // A fresh session over the unchanged record is the same observation;
    // a new revision is a new one.
    let (second, _) = w.prove(&w.request(Some(&cookie)), MANIFEST).await;
    let v2 = verify_bundle(second.to_json().as_bytes(), &policy);
    assert!(v2.authenticated());
    let env2 = v2.envelope.unwrap();
    assert_ne!(
        env2.authentication.proof_digest,
        env.authentication.proof_digest
    );
    assert_eq!(env2.observation_id, env.observation_id);

    let mut items = fixture::default_items();
    items[0]["revision"] = 4.into();
    items[0]["unit_price"] = "42.10".into();
    w.portal.set_items(items);
    let (third, _) = w.prove(&w.request(Some(&cookie)), MANIFEST).await;
    let v3 = verify_bundle(third.to_json().as_bytes(), &policy);
    let env3 = v3.envelope.unwrap();
    assert_eq!(env3.observation.price.atoms, 42_100_000);
    assert_ne!(env3.observation_id, env.observation_id);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn signed_out_proof_reads_no_price() {
    let w = world().await;
    let (bundle, status) = w.prove(&w.request(None), MANIFEST).await;
    assert_eq!(status, 401);
    let v = verify_bundle(bundle.to_json().as_bytes(), &w.policy());
    // Authentic, but the source refused: no observation.
    assert_eq!(v.stages.provenance.outcome, Outcome::Passed);
    assert_eq!(v.stages.extraction.outcome, Outcome::Failed);
    assert!(v.envelope.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn notary_refuses_oversized_sessions() {
    let w = world().await;
    let mut req = w.request(Some(&w.portal.sign_in()));
    req.max_recv = 1 << 20;
    let io = amoeba_webproof::transport::connect(&format!("tcp://{}", w.notary_addr))
        .await
        .unwrap();
    let err = amoeba_webproof::prover::notarize(io, &req)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("failed") || err.to_string().contains("refused"),
        "{err}"
    );
}
