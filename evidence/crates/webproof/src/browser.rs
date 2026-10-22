//! The contributor's own browser as the sign-in companion.
//!
//! Petri never asks for a source password. The contributor signs in to the
//! source in a real browser (Chrome, Edge, Brave or Chromium) that petri
//! starts with its own profile and the DevTools Protocol on loopback; an
//! agent with browser tools can drive the same window, with the person
//! signing in themselves. For the proof, petri asks that browser for the
//! cookies it would send to the one URL being proven and its User-Agent,
//! and uses them in the TLS session it opens to that source. They go to
//! the source only: never to the notary (MPC mode), never into the
//! presentation, the bundle, a file or a log.
//!
//! The profile keeps only what is signed in there, so other browsing is
//! never in reach. While the window is open, other programs of the same
//! user on this computer can reach its DevTools port too; close it when
//! done.

use std::path::{Path, PathBuf};

use async_tungstenite::tokio::connect_async;
use async_tungstenite::tungstenite::Message;
use futures::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Browser executables, most common first.
pub fn candidates() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if cfg!(target_os = "macos") {
        for app in [
            "Google Chrome.app/Contents/MacOS/Google Chrome",
            "Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            "Brave Browser.app/Contents/MacOS/Brave Browser",
            "Chromium.app/Contents/MacOS/Chromium",
        ] {
            out.push(Path::new("/Applications").join(app));
            if let Some(home) = std::env::var_os("HOME") {
                out.push(Path::new(&home).join("Applications").join(app));
            }
        }
    } else if cfg!(windows) {
        let roots = ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
            .iter()
            .filter_map(std::env::var_os)
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        for root in roots {
            for exe in [
                "Google/Chrome/Application/chrome.exe",
                "Microsoft/Edge/Application/msedge.exe",
                "BraveSoftware/Brave-Browser/Application/brave.exe",
                "Chromium/Application/chrome.exe",
            ] {
                out.push(root.join(exe));
            }
        }
    } else if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            for exe in [
                "google-chrome",
                "google-chrome-stable",
                "chromium",
                "chromium-browser",
                "microsoft-edge",
                "brave-browser",
            ] {
                out.push(dir.join(exe));
            }
        }
    }
    out
}

pub fn find() -> Option<PathBuf> {
    candidates().into_iter().find(|p| p.is_file())
}

/// Starts the browser on `url` with its own profile in `profile_dir` and
/// DevTools on a loopback port the browser picks (it writes the port to
/// `<profile>/DevToolsActivePort`). Returns at once; the window stays open
/// for the contributor.
pub fn open(browser: &Path, profile_dir: &Path, url: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(profile_dir)?;
    std::process::Command::new(browser)
        .arg(format!("--user-data-dir={}", profile_dir.display()))
        .arg("--remote-debugging-address=127.0.0.1")
        .arg("--remote-debugging-port=0")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    Ok(())
}

/// The DevTools address of a browser petri opened on `profile_dir`.
pub fn devtools_of(profile_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(profile_dir.join("DevToolsActivePort")).ok()?;
    let port: u16 = text.lines().next()?.trim().parse().ok()?;
    Some(format!("http://127.0.0.1:{port}"))
}

/// What the proof needs from the signed-in browser. The cookie values are
/// never printed (`Debug` shows names only).
#[derive(Clone)]
pub struct SignedIn {
    pub cookie_header: Option<String>,
    pub cookie_names: Vec<String>,
    pub user_agent: Option<String>,
}

impl std::fmt::Debug for SignedIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignedIn")
            .field("cookie_names", &self.cookie_names)
            .field("user_agent", &self.user_agent)
            .finish()
    }
}

impl SignedIn {
    /// The headers to send in the proven request.
    pub fn headers(&self) -> Vec<(String, String)> {
        let mut h = Vec::new();
        if let Some(ua) = &self.user_agent {
            h.push(("User-Agent".to_string(), ua.clone()));
        }
        if let Some(c) = &self.cookie_header {
            h.push(("Cookie".to_string(), c.clone()));
        }
        h
    }
}

/// A loopback-only HTTP GET (DevTools' JSON endpoints).
async fn local_get(devtools: &str, path: &str, method: &str) -> anyhow::Result<serde_json::Value> {
    let addr = devtools
        .strip_prefix("http://")
        .ok_or_else(|| anyhow::anyhow!("DevTools address must be http://127.0.0.1:<port>"))?;
    let host = addr.split(':').next().unwrap_or("");
    if host != "127.0.0.1" && host != "localhost" && host != "[::1]" {
        anyhow::bail!("petri reads browser sessions on this computer only (127.0.0.1)");
    }
    let mut s = tokio::net::TcpStream::connect(addr).await?;
    s.write_all(
        format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n").as_bytes(),
    )
    .await?;
    let mut buf = Vec::new();
    s.take(4 * 1024 * 1024).read_to_end(&mut buf).await?;
    let split = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| anyhow::anyhow!("DevTools answered something unexpected"))?;
    Ok(serde_json::from_slice(&buf[split + 4..])?)
}

async fn page_socket(devtools: &str) -> anyhow::Result<String> {
    let list = local_get(devtools, "/json/list", "GET").await?;
    if let Some(ws) = list.as_array().and_then(|targets| {
        targets
            .iter()
            .find(|t| t["type"] == "page")
            .and_then(|t| t["webSocketDebuggerUrl"].as_str())
    }) {
        return Ok(ws.to_string());
    }
    let created = local_get(devtools, "/json/new?about:blank", "PUT").await?;
    created["webSocketDebuggerUrl"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("the browser has no page to read"))
}

/// Reads the cookies the browser would send to `url`, and its User-Agent.
pub async fn signed_in(devtools: &str, url: &str) -> anyhow::Result<SignedIn> {
    let version = local_get(devtools, "/json/version", "GET").await?;
    let user_agent = version["User-Agent"].as_str().map(str::to_string);
    let ws_url = page_socket(devtools).await?;
    if !ws_url.starts_with("ws://127.0.0.1:") && !ws_url.starts_with("ws://localhost:") {
        anyhow::bail!("the browser's DevTools socket is not on this computer");
    }
    let (mut ws, _) = connect_async(ws_url.as_str()).await?;
    let call =
        serde_json::json!({ "id": 1, "method": "Network.getCookies", "params": { "urls": [url] } });
    ws.send(Message::Text(call.to_string().into())).await?;
    let cookies = loop {
        let msg = ws
            .next()
            .await
            .ok_or_else(|| anyhow::anyhow!("the browser closed the DevTools connection"))??;
        let Message::Text(text) = msg else { continue };
        let v: serde_json::Value = serde_json::from_str(&text)?;
        if v["id"] == 1 {
            if let Some(err) = v.get("error") {
                anyhow::bail!("the browser refused: {}", err["message"]);
            }
            break v["result"]["cookies"]
                .as_array()
                .cloned()
                .unwrap_or_default();
        }
    };
    let _ = ws.close(None).await;
    let mut pairs = Vec::new();
    let mut names = Vec::new();
    for c in cookies {
        if let (Some(n), Some(v)) = (c["name"].as_str(), c["value"].as_str()) {
            names.push(n.to_string());
            pairs.push(format!("{n}={v}"));
        }
    }
    Ok(SignedIn {
        cookie_header: (!pairs.is_empty()).then(|| pairs.join("; ")),
        cookie_names: names,
        user_agent,
    })
}
