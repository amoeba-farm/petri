//! Repeated web-proof trials, for `evidence/MEASUREMENTS.md`.
//!
//! ```text
//! cargo run --release -p amoeba-webproof --example measure -- \
//!     --runs 20 [--fixture | --url URL --manifest FILE] [--notary tcp://HOST:PORT]
//! ```
//!
//! Each trial runs the whole prover (MPC preprocessing, the request, the
//! commitments, the attestation), builds the presentation and verifies it
//! with `amoeba_evidence::verify_bundle`. Without `--notary` an in-process
//! notary on loopback attests (key [7; 32], a test key). `--fixture` proves
//! the signed-in test portal (test CA, fixture prices) on loopback. The
//! report gives p50/p95 per phase, the bytes on the notary link, sizes and
//! failures, with the first (cold) trial apart.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::Instant;

use amoeba_evidence::policy::Policy;
use amoeba_webproof::notary::NotaryConfig;
use amoeba_webproof::prover::ProveRequest;
use futures::{AsyncRead, AsyncWrite};

/// Counts the bytes each way on the notary link.
struct Counted<S> {
    inner: S,
    up: Arc<AtomicU64>,
    down: Arc<AtomicU64>,
}

impl<S: AsyncRead + Unpin> AsyncRead for Counted<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        let r = Pin::new(&mut this.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(n)) = &r {
            this.down.fetch_add(*n as u64, Ordering::Relaxed);
        }
        r
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Counted<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        let r = Pin::new(&mut this.inner).poll_write(cx, buf);
        if let Poll::Ready(Ok(n)) = &r {
            this.up.fetch_add(*n as u64, Ordering::Relaxed);
        }
        r
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }
    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_close(cx)
    }
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let i = ((p / 100.0) * (v.len() as f64 - 1.0)).round() as usize;
    v[i.min(v.len() - 1)]
}

const FIXTURE_MANIFEST: &str = r#"{"format":"amoeba-extraction-manifest-v1","name":"Fixture: DDR5 16GB","evidenceType":"catalog_price","origin":"portal.fixture.test","path":"/api/prices","content":"json","records":"/items","match":{"/sku":"DDR5-16G-4800"},"fields":{"price":"/unit_price","currency":"/currency","product":"/sku"}}"#;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let get = |k: &str| {
        args.iter()
            .position(|a| a == k)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let runs: usize = get("--runs").map(|r| r.parse()).transpose()?.unwrap_or(10);
    let fixture = args.iter().any(|a| a == "--fixture");

    let key = [7u8; 32];
    let notary_url = match get("--notary") {
        Some(u) => u,
        None => {
            let cfg = Arc::new(NotaryConfig::new(key));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let addr = listener.local_addr()?;
            tokio::spawn(async move {
                loop {
                    let Ok((s, _)) = listener.accept().await else {
                        continue;
                    };
                    let _ = s.set_nodelay(true);
                    let cfg = cfg.clone();
                    tokio::spawn(async move {
                        use tokio_util::compat::TokioAsyncReadCompatExt;
                        let _ = amoeba_webproof::notary::serve(s.compat(), &cfg).await;
                    });
                }
            });
            format!("tcp://{addr}")
        }
    };
    let notary_key = NotaryConfig::new(key).public_key()?;

    let mut portal = None;
    let (mut req, manifest, policy) = if fixture {
        let p = amoeba_evidence_fixture::start("127.0.0.1:0".parse()?, Default::default()).await?;
        let mut r = ProveRequest::new("https://portal.fixture.test/api/prices");
        r.connect_to = Some(p.addr.to_string());
        r.extra_roots = vec![p.ca_der.clone()];
        r.headers.push(("Cookie".into(), p.sign_in()));
        let policy =
            Policy::development("measure notary", notary_key.clone(), vec![p.ca_der.clone()]);
        portal = Some(p);
        (r, FIXTURE_MANIFEST.as_bytes().to_vec(), policy)
    } else {
        let url = get("--url")
            .ok_or_else(|| anyhow::anyhow!("--url URL --manifest FILE, or --fixture"))?;
        let manifest =
            std::fs::read(get("--manifest").ok_or_else(|| anyhow::anyhow!("--manifest FILE"))?)?;
        let policy = Policy::development("measure notary", notary_key.clone(), vec![]);
        (ProveRequest::new(url), manifest, policy)
    };
    if let Some(m) = get("--max-recv") {
        req.max_recv = m.parse()?;
    }

    let mut rows = Vec::new();
    let mut failures = Vec::new();
    for i in 0..runs {
        let up = Arc::new(AtomicU64::new(0));
        let down = Arc::new(AtomicU64::new(0));
        let t0 = Instant::now();
        let io = amoeba_webproof::transport::connect(&notary_url).await?;
        let io = Counted {
            inner: io,
            up: up.clone(),
            down: down.clone(),
        };
        match amoeba_webproof::prover::notarize(io, &req).await {
            Ok(n) => {
                let present_t = Instant::now();
                let presentation = amoeba_webproof::prover::present(&n)?;
                let present_ms = present_t.elapsed().as_secs_f64() * 1000.0;
                let bundle = amoeba_webproof::bundle(presentation, &manifest, vec![])?;
                let json = bundle.to_json();
                let vt = Instant::now();
                let v = amoeba_evidence::verify_bundle(json.as_bytes(), &policy);
                let verify_ms = vt.elapsed().as_secs_f64() * 1000.0;
                let ok = v.authenticated();
                if !ok {
                    failures.push(format!("run {i}: verification {:?}", v.stages));
                }
                rows.push(serde_json::json!({
                    "run": i,
                    "ok": ok,
                    "status": n.status,
                    "setupMs": n.timings.setup_ms,
                    "requestMs": n.timings.request_ms,
                    "proveMs": n.timings.prove_ms,
                    "attestMs": n.timings.attest_ms,
                    "notarizeMs": n.timings.total_ms,
                    "presentMs": present_ms,
                    "verifyMs": verify_ms,
                    "wallMs": t0.elapsed().as_secs_f64() * 1000.0,
                    "notaryUpBytes": up.load(Ordering::Relaxed),
                    "notaryDownBytes": down.load(Ordering::Relaxed),
                    "presentationBytes": bundle.presentation.len(),
                    "bundleBytes": json.len(),
                }));
            }
            Err(e) => failures.push(format!("run {i}: {e}")),
        }
        eprintln!("run {i} done");
    }
    let warm: Vec<&serde_json::Value> = rows
        .iter()
        .filter(|r| r["run"].as_u64() != Some(0))
        .collect();
    let mut summary = serde_json::Map::new();
    for k in [
        "setupMs",
        "requestMs",
        "proveMs",
        "attestMs",
        "notarizeMs",
        "presentMs",
        "verifyMs",
        "wallMs",
        "notaryUpBytes",
        "notaryDownBytes",
        "presentationBytes",
        "bundleBytes",
    ] {
        let mut v: Vec<f64> = warm.iter().filter_map(|r| r[k].as_f64()).collect();
        summary.insert(
            k.into(),
            serde_json::json!({ "p50": pct(&mut v, 50.0), "p95": pct(&mut v, 95.0), "n": v.len() }),
        );
    }
    let report = serde_json::json!({
        "target": req.url,
        "runs": runs,
        "succeeded": rows.len(),
        "failures": failures,
        "cold": rows.first(),
        "warm": summary,
        "maxSent": req.max_sent,
        "maxRecv": req.max_recv,
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    drop(portal);
    Ok(())
}
