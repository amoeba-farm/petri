//! The prover ↔ notary connection around the TLSNotary session.
//!
//! ```text
//! prover → notary   "amoeba-notary/1 tlsn=<revision>\n"
//! notary → prover   "ok <alg> <public key hex>\n"   or   "no <reason>\n"
//!                   … TLSNotary session (MPC-TLS, commitments) …
//! prover → notary   u32-LE length ‖ attestation request (bincode)
//! notary → prover   u32-LE length ‖ attestation (bincode)
//! ```
//!
//! The hello tells the prover which key will sign before the expensive
//! MPC preprocessing starts, and lets a notary refuse a prover of another
//! TLSNotary revision with a readable reason. Length prefixes instead of a
//! half-closed socket keep the exchange working over WebSocket bridges.

use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const HELLO: &str = "amoeba-notary/1";
/// Largest framed message (hostile input bound, prompt v2 section 9).
pub const MAX_FRAME: usize = 1024 * 1024;
const MAX_LINE: usize = 512;

#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("the notary connection failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("the notary refused: {0}")]
    Refused(String),
    #[error("the notary sent something unexpected")]
    Protocol,
}

/// Reads one `\n`-terminated line byte by byte, so nothing of the session
/// that follows is consumed.
pub async fn read_line<S: AsyncRead + Unpin>(io: &mut S) -> Result<String, WireError> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        io.read_exact(&mut byte).await?;
        if byte[0] == b'\n' {
            break;
        }
        line.push(byte[0]);
        if line.len() > MAX_LINE {
            return Err(WireError::Protocol);
        }
    }
    String::from_utf8(line).map_err(|_| WireError::Protocol)
}

pub async fn write_line<S: AsyncWrite + Unpin>(io: &mut S, line: &str) -> Result<(), WireError> {
    io.write_all(line.as_bytes()).await?;
    io.write_all(b"\n").await?;
    io.flush().await?;
    Ok(())
}

pub async fn write_frame<S: AsyncWrite + Unpin>(io: &mut S, bytes: &[u8]) -> Result<(), WireError> {
    if bytes.len() > MAX_FRAME {
        return Err(WireError::Protocol);
    }
    io.write_all(&(bytes.len() as u32).to_le_bytes()).await?;
    io.write_all(bytes).await?;
    io.flush().await?;
    Ok(())
}

pub async fn read_frame<S: AsyncRead + Unpin>(io: &mut S) -> Result<Vec<u8>, WireError> {
    let mut len = [0u8; 4];
    io.read_exact(&mut len).await?;
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(WireError::Protocol);
    }
    let mut buf = vec![0u8; len];
    io.read_exact(&mut buf).await?;
    Ok(buf)
}

pub fn hello_line() -> String {
    format!("{HELLO} tlsn={}", amoeba_evidence::policy::TLSN_REVISION)
}
