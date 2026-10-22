//! HTTP/1.1 over an authenticated, partially disclosed TLS transcript.
//!
//! A web proof discloses only part of what was sent and received: the
//! contributor's cookies and authorization headers stay hidden (their bytes
//! read as zero). These rules decide what the disclosed bytes establish:
//!
//! - Request: the request line and the `Host` header are fully disclosed;
//!   every other header discloses at least its name. `GET` only, no body.
//! - Response: the status line, every header name and every header value
//!   except `Set-Cookie`'s are disclosed; framing comes from disclosed
//!   headers only; the whole body is disclosed; nothing follows the one
//!   response.
//!
//! Requiring the whole body closes the door on misleading redactions: with
//! hidden bytes inside a JSON body, a verifier cannot tell a hidden
//! `"},{"sku":"B",...` from a hidden string and could pair one row's
//! product with another row's price. Partial body disclosure is unsupported
//! until a construction can prove what the hidden bytes are not.

use std::ops::Range;

/// One direction of an authenticated transcript: the bytes (hidden ones are
/// zero) and the ranges that were disclosed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transcript {
    pub bytes: Vec<u8>,
    authed: Vec<Range<usize>>,
}

impl Transcript {
    pub fn new(bytes: Vec<u8>, ranges: impl IntoIterator<Item = Range<usize>>) -> Self {
        let mut authed: Vec<Range<usize>> = ranges
            .into_iter()
            .filter(|r| r.start < r.end)
            .map(|r| r.start.min(bytes.len())..r.end.min(bytes.len()))
            .collect();
        authed.sort_by_key(|r| r.start);
        let mut merged: Vec<Range<usize>> = Vec::new();
        for r in authed {
            match merged.last_mut() {
                Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
                _ => merged.push(r),
            }
        }
        Self {
            bytes,
            authed: merged,
        }
    }

    /// Everything disclosed (for tests).
    #[cfg(test)]
    pub fn full(bytes: Vec<u8>) -> Self {
        let len = bytes.len();
        Self::new(bytes, std::iter::once(0..len))
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Whether every byte of `r` was disclosed.
    pub fn authed(&self, r: Range<usize>) -> bool {
        if r.start >= r.end {
            return true;
        }
        self.authed
            .iter()
            .any(|a| a.start <= r.start && r.end <= a.end)
    }

    /// The text with every hidden byte shown as `·` (for previews).
    pub fn display(&self) -> String {
        let mut out = String::new();
        let mut i = 0;
        for a in &self.authed {
            if a.start > i {
                out.push_str(&"·".repeat((a.start - i).min(64)));
                if a.start - i > 64 {
                    out.push_str(&format!("[{} hidden]", a.start - i));
                }
            }
            out.push_str(&String::from_utf8_lossy(&self.bytes[a.clone()]));
            i = a.end;
        }
        if i < self.bytes.len() {
            out.push_str(&"·".repeat((self.bytes.len() - i).min(64)));
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HttpError {
    #[error("the {0} is not disclosed")]
    Hidden(&'static str),
    #[error("malformed {0}")]
    Malformed(&'static str),
    #[error("{0} is not supported")]
    Unsupported(String),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HeaderView {
    pub name: String,
    /// `None` when the value was not disclosed.
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    /// The request target as sent (`/path?query`).
    pub target: String,
    pub host: String,
    pub headers: Vec<HeaderView>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<HeaderView>,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

/// Response headers whose values may stay hidden: they carry the
/// contributor's session, never the content.
const HIDEABLE_RESPONSE_HEADERS: &[&str] = &["set-cookie"];

struct Lines {
    block_end: usize,
    lines: Vec<Range<usize>>,
}

/// Splits the header block (start line plus headers) at disclosed CRLFs.
fn header_lines(t: &Transcript, what: &'static str) -> Result<Lines, HttpError> {
    let b = &t.bytes;
    let end = b
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or(HttpError::Malformed(what))?;
    if !t.authed(end..end + 4) {
        return Err(HttpError::Hidden(what));
    }
    let mut lines = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < end {
        if b[i] == b'\r' && b.get(i + 1) == Some(&b'\n') {
            if !t.authed(i..i + 2) {
                return Err(HttpError::Hidden(what));
            }
            lines.push(start..i);
            start = i + 2;
            i += 2;
        } else {
            if b[i] == b'\n' || (b[i] == b'\r' && t.authed(i..i + 1)) {
                return Err(HttpError::Malformed(what));
            }
            i += 1;
        }
    }
    lines.push(start..end);
    Ok(Lines {
        block_end: end + 4,
        lines,
    })
}

fn token(bytes: &[u8]) -> bool {
    !bytes.is_empty()
        && bytes
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(c))
}

/// Parses one header line: the name and colon must be disclosed; the value
/// is `None` when any of it is hidden.
fn header(t: &Transcript, line: Range<usize>) -> Result<HeaderView, HttpError> {
    let b = &t.bytes[line.clone()];
    let colon = b
        .iter()
        .position(|c| *c == b':')
        .ok_or(HttpError::Malformed("header"))?;
    if !t.authed(line.start..line.start + colon + 1) || !token(&b[..colon]) {
        return Err(HttpError::Hidden("header name"));
    }
    let name = String::from_utf8_lossy(&b[..colon]).into_owned();
    let value_range = line.start + colon + 1..line.end;
    let value = if t.authed(value_range.clone()) {
        let v = std::str::from_utf8(&t.bytes[value_range])
            .map_err(|_| HttpError::Malformed("header value"))?;
        if v.bytes().any(|c| c == 0 || c == b'\r' || c == b'\n') {
            return Err(HttpError::Malformed("header value"));
        }
        Some(v.trim_matches([' ', '\t']).to_string())
    } else {
        None
    };
    Ok(HeaderView { name, value })
}

fn find<'a>(headers: &'a [HeaderView], name: &str) -> Vec<&'a HeaderView> {
    headers
        .iter()
        .filter(|h| h.name.eq_ignore_ascii_case(name))
        .collect()
}

/// The one request a web proof may contain.
pub fn parse_request(t: &Transcript) -> Result<Request, HttpError> {
    let Lines { block_end, lines } = header_lines(t, "request header block")?;
    let first = lines[0].clone();
    if !t.authed(first.clone()) {
        return Err(HttpError::Hidden("request line"));
    }
    let line =
        std::str::from_utf8(&t.bytes[first]).map_err(|_| HttpError::Malformed("request line"))?;
    let mut parts = line.split(' ');
    let (Some(method), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(HttpError::Malformed("request line"));
    };
    if version != "HTTP/1.1" {
        return Err(HttpError::Unsupported(format!("HTTP version {version}")));
    }
    if method != "GET" {
        return Err(HttpError::Unsupported(format!("method {method}")));
    }
    if !target.starts_with('/') || target.bytes().any(|c| c <= b' ' || c >= 0x7f) {
        return Err(HttpError::Malformed("request target"));
    }
    let headers = lines[1..]
        .iter()
        .map(|l| header(t, l.clone()))
        .collect::<Result<Vec<_>, _>>()?;
    let hosts = find(&headers, "host");
    let host = match hosts.as_slice() {
        [
            HeaderView {
                value: Some(host), ..
            },
        ] => host.to_ascii_lowercase(),
        [_] => return Err(HttpError::Hidden("Host header")),
        _ => return Err(HttpError::Malformed("Host header")),
    };
    if t.len() != block_end {
        return Err(HttpError::Unsupported(
            "a request body or a second request".into(),
        ));
    }
    Ok(Request {
        method: method.to_string(),
        target: target.to_string(),
        host,
        headers,
    })
}

/// The one response a web proof may contain, with its body decoded.
pub fn parse_response(t: &Transcript) -> Result<Response, HttpError> {
    let Lines { block_end, lines } = header_lines(t, "response header block")?;
    let first = lines[0].clone();
    if !t.authed(first.clone()) {
        return Err(HttpError::Hidden("status line"));
    }
    let line =
        std::str::from_utf8(&t.bytes[first]).map_err(|_| HttpError::Malformed("status line"))?;
    let rest = line
        .strip_prefix("HTTP/1.1 ")
        .ok_or_else(|| HttpError::Unsupported(format!("status line `{line}`")))?;
    let code = rest.get(..3).ok_or(HttpError::Malformed("status line"))?;
    if !code.bytes().all(|c| c.is_ascii_digit()) || !(rest.len() == 3 || rest.as_bytes()[3] == b' ')
    {
        return Err(HttpError::Malformed("status line"));
    }
    let status: u16 = code
        .parse()
        .map_err(|_| HttpError::Malformed("status line"))?;
    let mut headers = Vec::new();
    for l in &lines[1..] {
        let h = header(t, l.clone())?;
        if h.value.is_none()
            && !HIDEABLE_RESPONSE_HEADERS
                .iter()
                .any(|n| h.name.eq_ignore_ascii_case(n))
        {
            return Err(HttpError::Hidden("response header value"));
        }
        headers.push(h);
    }
    let single = |name: &str| -> Result<Option<String>, HttpError> {
        match find(&headers, name).as_slice() {
            [] => Ok(None),
            [h] => Ok(h.value.clone()),
            _ => Err(HttpError::Malformed("repeated framing header")),
        }
    };
    if let Some(enc) = single("content-encoding")?
        && !enc.eq_ignore_ascii_case("identity")
    {
        return Err(HttpError::Unsupported(format!("content encoding `{enc}`")));
    }
    let content_type = single("content-type")?;
    let te = single("transfer-encoding")?;
    let cl = single("content-length")?;
    let b = &t.bytes;
    let (body, end) = match (te, cl) {
        (Some(_), Some(_)) => {
            return Err(HttpError::Malformed(
                "both Transfer-Encoding and Content-Length",
            ));
        }
        (Some(te), None) => {
            if !te.eq_ignore_ascii_case("chunked") {
                return Err(HttpError::Unsupported(format!("transfer encoding `{te}`")));
            }
            let mut body = Vec::new();
            let mut i = block_end;
            loop {
                let nl = b[i..]
                    .windows(2)
                    .position(|w| w == b"\r\n")
                    .ok_or(HttpError::Malformed("chunk"))?;
                if !t.authed(i..i + nl + 2) {
                    return Err(HttpError::Hidden("chunk framing"));
                }
                let size_text = std::str::from_utf8(&b[i..i + nl])
                    .map_err(|_| HttpError::Malformed("chunk"))?;
                if size_text.is_empty()
                    || size_text.len() > 8
                    || !size_text.bytes().all(|c| c.is_ascii_hexdigit())
                {
                    return Err(HttpError::Unsupported("chunk extensions".into()));
                }
                let size = usize::from_str_radix(size_text, 16)
                    .map_err(|_| HttpError::Malformed("chunk"))?;
                i += nl + 2;
                if size == 0 {
                    if b.get(i..i + 2) != Some(b"\r\n") || !t.authed(i..i + 2) {
                        return Err(HttpError::Unsupported("chunked trailers".into()));
                    }
                    break (body, i + 2);
                }
                let data = i..i.checked_add(size).ok_or(HttpError::Malformed("chunk"))?;
                if data.end + 2 > b.len() || &b[data.end..data.end + 2] != b"\r\n" {
                    return Err(HttpError::Malformed("chunk"));
                }
                if !t.authed(data.start..data.end + 2) {
                    return Err(HttpError::Hidden("response body"));
                }
                body.extend_from_slice(&b[data.clone()]);
                i = data.end + 2;
            }
        }
        (None, Some(cl)) => {
            if cl.is_empty() || cl.len() > 10 || !cl.bytes().all(|c| c.is_ascii_digit()) {
                return Err(HttpError::Malformed("Content-Length"));
            }
            let n: usize = cl
                .parse()
                .map_err(|_| HttpError::Malformed("Content-Length"))?;
            let end = block_end
                .checked_add(n)
                .filter(|e| *e <= b.len())
                .ok_or(HttpError::Malformed("body length"))?;
            (b[block_end..end].to_vec(), end)
        }
        (None, None) => (b[block_end..].to_vec(), b.len()),
    };
    if end != b.len() {
        return Err(HttpError::Unsupported("more than one response".into()));
    }
    if !t.authed(block_end..end) {
        return Err(HttpError::Hidden("response body"));
    }
    Ok(Response {
        status,
        headers,
        content_type,
        body,
    })
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;

    fn hide(bytes: &[u8], hidden: &[Range<usize>]) -> Transcript {
        let mut b = bytes.to_vec();
        for r in hidden {
            b[r.clone()].iter_mut().for_each(|x| *x = 0);
        }
        let mut ranges = Vec::new();
        let mut i = 0;
        for r in hidden {
            ranges.push(i..r.start);
            i = r.end;
        }
        ranges.push(i..b.len());
        Transcript::new(b, ranges)
    }

    const REQ: &[u8] = b"GET /api/prices?sku=A HTTP/1.1\r\nHost: Shop.Example\r\nCookie: session=secret\r\nAccept: */*\r\n\r\n";

    #[test]
    fn request_with_hidden_cookie() {
        let at = REQ
            .windows(14)
            .position(|w| w == b"session=secret")
            .unwrap();
        let t = hide(REQ, &[at..at + 14]);
        let r = parse_request(&t).unwrap();
        assert_eq!(r.target, "/api/prices?sku=A");
        assert_eq!(r.host, "shop.example");
        assert_eq!(
            r.headers[1],
            HeaderView {
                name: "Cookie".into(),
                value: None
            }
        );
        assert!(t.display().contains("Cookie: ··············"));
    }

    #[test]
    fn request_needs_line_and_host() {
        assert_eq!(
            parse_request(&hide(REQ, &[5..9])),
            Err(HttpError::Hidden("request line"))
        );
        let at = REQ.windows(12).position(|w| w == b"Shop.Example").unwrap();
        assert_eq!(
            parse_request(&hide(REQ, &[at..at + 4])),
            Err(HttpError::Hidden("Host header"))
        );
        let at = REQ.windows(6).position(|w| w == b"Cookie").unwrap();
        assert_eq!(
            parse_request(&hide(REQ, &[at..at + 3])),
            Err(HttpError::Hidden("header name"))
        );
        let post = b"POST / HTTP/1.1\r\nHost: a\r\n\r\n";
        assert!(matches!(
            parse_request(&Transcript::full(post.to_vec())),
            Err(HttpError::Unsupported(_))
        ));
        let two = b"GET / HTTP/1.1\r\nHost: a\r\n\r\nGET /x HTTP/1.1\r\nHost: a\r\n\r\n";
        assert!(matches!(
            parse_request(&Transcript::full(two.to_vec())),
            Err(HttpError::Unsupported(_))
        ));
    }

    #[test]
    fn response_framings() {
        let cl = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nSet-Cookie: s=1\r\nContent-Length: 7\r\n\r\n{\"a\":1}";
        let at = cl.windows(3).position(|w| w == b"s=1").unwrap();
        let r = parse_response(&hide(cl, &[at..at + 3])).unwrap();
        assert_eq!((r.status, r.body.as_slice()), (200, &b"{\"a\":1}"[..]));
        assert_eq!(r.content_type.as_deref(), Some("application/json"));

        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\n{\"a\r\n4\r\n\":1}\r\n0\r\n\r\n";
        let r = parse_response(&Transcript::full(chunked.to_vec())).unwrap();
        assert_eq!(r.body, b"{\"a\":1}");

        let close = b"HTTP/1.1 404 Not Found\r\n\r\nnope";
        assert_eq!(
            parse_response(&Transcript::full(close.to_vec()))
                .unwrap()
                .status,
            404
        );
    }

    #[test]
    fn response_refusals() {
        let base = b"HTTP/1.1 200 OK\r\nX-Account: alice\r\nContent-Length: 7\r\n\r\n{\"a\":1}";
        let at = base.windows(5).position(|w| w == b"alice").unwrap();
        assert_eq!(
            parse_response(&hide(base, &[at..at + 5])),
            Err(HttpError::Hidden("response header value"))
        );
        let body_at = base.len() - 3;
        assert_eq!(
            parse_response(&hide(base, &[body_at..body_at + 1])),
            Err(HttpError::Hidden("response body"))
        );
        let gz = b"HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\n\r\nxx";
        assert!(matches!(
            parse_response(&Transcript::full(gz.to_vec())),
            Err(HttpError::Unsupported(_))
        ));
        let both = b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nTransfer-Encoding: chunked\r\n\r\nx";
        assert!(parse_response(&Transcript::full(both.to_vec())).is_err());
        let extra = b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\nxHTTP/1.1 200 OK\r\n\r\n";
        assert!(matches!(
            parse_response(&Transcript::full(extra.to_vec())),
            Err(HttpError::Unsupported(_))
        ));
        let dup = b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nContent-Length: 1\r\n\r\nx";
        assert!(parse_response(&Transcript::full(dup.to_vec())).is_err());
    }
}
