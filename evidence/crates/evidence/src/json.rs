//! A strict, bounded JSON parser that keeps each value's byte span.
//!
//! Authenticated bytes are parsed exactly once, by this parser, so every
//! verifier reads the same values: RFC 8259 grammar only, no duplicate
//! object keys (parsers disagree on which one wins), bounded nesting. The
//! spans let a preview show which bytes a value came from. The canonical
//! form (`canonical`) gives manifests a content address that does not
//! depend on whitespace or key order.

use std::ops::Range;

/// Nesting limit for hostile input (`amoeba_permissionless_web_proofs`
/// prompt v2, section 9: "enforce size/time limits, safe parsing").
pub const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    /// The number exactly as written (`12.50` stays `12.50`).
    Number(String),
    String(String),
    Array(Vec<Node>),
    Object(Vec<(String, Node)>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub value: Json,
    pub span: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid JSON at byte {at}: {what}")]
pub struct JsonError {
    pub at: usize,
    pub what: &'static str,
}

pub fn parse(bytes: &[u8]) -> Result<Node, JsonError> {
    if std::str::from_utf8(bytes).is_err() {
        return Err(JsonError {
            at: 0,
            what: "not UTF-8",
        });
    }
    let mut p = Parser { b: bytes, i: 0 };
    p.ws();
    let node = p.value(0)?;
    p.ws();
    if p.i != bytes.len() {
        return Err(p.err("trailing data"));
    }
    Ok(node)
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn err(&self, what: &'static str) -> JsonError {
        JsonError { at: self.i, what }
    }
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }
    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }
    fn eat(&mut self, lit: &[u8]) -> bool {
        if self.b[self.i..].starts_with(lit) {
            self.i += lit.len();
            true
        } else {
            false
        }
    }
    fn value(&mut self, depth: usize) -> Result<Node, JsonError> {
        if depth > MAX_DEPTH {
            return Err(self.err("nested too deeply"));
        }
        let start = self.i;
        let value = match self.peek() {
            Some(b'{') => self.object(depth)?,
            Some(b'[') => self.array(depth)?,
            Some(b'"') => Json::String(self.string()?),
            Some(b't') if self.eat(b"true") => Json::Bool(true),
            Some(b'f') if self.eat(b"false") => Json::Bool(false),
            Some(b'n') if self.eat(b"null") => Json::Null,
            Some(b'-' | b'0'..=b'9') => Json::Number(self.number()?),
            _ => return Err(self.err("expected a value")),
        };
        Ok(Node {
            value,
            span: start..self.i,
        })
    }
    fn object(&mut self, depth: usize) -> Result<Json, JsonError> {
        self.i += 1;
        let mut members: Vec<(String, Node)> = Vec::new();
        // Seen keys, so a duplicate check stays linear in a hostile
        // object's size (SipHash: no chosen-collision slowdown either).
        let mut seen = std::collections::HashSet::new();
        self.ws();
        if self.eat(b"}") {
            return Ok(Json::Object(members));
        }
        loop {
            self.ws();
            if self.peek() != Some(b'"') {
                return Err(self.err("expected a key"));
            }
            let key_at = self.i;
            let key = self.string()?;
            if !seen.insert(key.clone()) {
                return Err(JsonError {
                    at: key_at,
                    what: "duplicate key",
                });
            }
            self.ws();
            if !self.eat(b":") {
                return Err(self.err("expected `:`"));
            }
            self.ws();
            let v = self.value(depth + 1)?;
            members.push((key, v));
            self.ws();
            if self.eat(b",") {
                continue;
            }
            if self.eat(b"}") {
                return Ok(Json::Object(members));
            }
            return Err(self.err("expected `,` or `}`"));
        }
    }
    fn array(&mut self, depth: usize) -> Result<Json, JsonError> {
        self.i += 1;
        let mut items = Vec::new();
        self.ws();
        if self.eat(b"]") {
            return Ok(Json::Array(items));
        }
        loop {
            self.ws();
            items.push(self.value(depth + 1)?);
            self.ws();
            if self.eat(b",") {
                continue;
            }
            if self.eat(b"]") {
                return Ok(Json::Array(items));
            }
            return Err(self.err("expected `,` or `]`"));
        }
    }
    fn hex4(&mut self) -> Result<u32, JsonError> {
        let digits = self
            .b
            .get(self.i..self.i + 4)
            .ok_or_else(|| self.err("short \\u escape"))?;
        let text = std::str::from_utf8(digits).map_err(|_| self.err("bad \\u escape"))?;
        if !text.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(self.err("bad \\u escape"));
        }
        self.i += 4;
        u32::from_str_radix(text, 16).map_err(|_| self.err("bad \\u escape"))
    }
    fn string(&mut self) -> Result<String, JsonError> {
        self.i += 1;
        let mut out = String::new();
        loop {
            let start = self.i;
            while let Some(b) = self.peek() {
                if b == b'"' || b == b'\\' || b < 0x20 {
                    break;
                }
                self.i += 1;
            }
            // Safe: the whole input was checked to be UTF-8 and we split
            // only at ASCII bytes.
            out.push_str(std::str::from_utf8(&self.b[start..self.i]).unwrap_or_default());
            match self.peek() {
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.i += 1;
                    let c = self.peek().ok_or_else(|| self.err("short escape"))?;
                    self.i += 1;
                    match c {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&hi) {
                                if !self.eat(b"\\u") {
                                    return Err(self.err("lone surrogate"));
                                }
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return Err(self.err("lone surrogate"));
                                }
                                0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                            } else if (0xDC00..0xE000).contains(&hi) {
                                return Err(self.err("lone surrogate"));
                            } else {
                                hi
                            };
                            out.push(
                                char::from_u32(code).ok_or_else(|| self.err("bad code point"))?,
                            );
                        }
                        _ => return Err(self.err("bad escape")),
                    }
                }
                Some(_) => return Err(self.err("control character in string")),
                None => return Err(self.err("unterminated string")),
            }
        }
    }
    fn number(&mut self) -> Result<String, JsonError> {
        let start = self.i;
        self.eat(b"-");
        match self.peek() {
            Some(b'0') => self.i += 1,
            Some(b'1'..=b'9') => {
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.i += 1;
                }
            }
            _ => return Err(self.err("bad number")),
        }
        if self.eat(b".") {
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.err("bad number"));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.i += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.err("bad number"));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.i += 1;
            }
        }
        Ok(String::from_utf8_lossy(&self.b[start..self.i]).into_owned())
    }
}

impl Node {
    /// RFC 6901 JSON pointer (`""` is the node itself, `/a/0/b~1c`).
    pub fn pointer(&self, pointer: &str) -> Option<&Node> {
        if pointer.is_empty() {
            return Some(self);
        }
        let rest = pointer.strip_prefix('/')?;
        let mut node = self;
        for raw in rest.split('/') {
            let token = raw.replace("~1", "/").replace("~0", "~");
            node = match &node.value {
                Json::Object(members) => &members.iter().find(|(k, _)| *k == token)?.1,
                Json::Array(items) => {
                    if token.is_empty()
                        || (token.len() > 1 && token.starts_with('0'))
                        || !token.bytes().all(|b| b.is_ascii_digit())
                    {
                        return None;
                    }
                    items.get(token.parse::<usize>().ok()?)?
                }
                _ => return None,
            };
        }
        Some(node)
    }
}

/// A valid RFC 6901 pointer (empty, or `/`-separated tokens whose `~` is
/// always `~0` or `~1`).
pub fn valid_pointer(pointer: &str) -> bool {
    if pointer.is_empty() {
        return true;
    }
    let Some(rest) = pointer.strip_prefix('/') else {
        return false;
    };
    rest.split('/').all(|t| {
        let b = t.as_bytes();
        b.iter()
            .enumerate()
            .all(|(i, c)| *c != b'~' || matches!(b.get(i + 1), Some(b'0' | b'1')))
    })
}

/// Canonical bytes: object keys sorted by their UTF-8 bytes, no whitespace,
/// strings with only the escapes JSON requires, numbers as written.
pub fn canonical(node: &Node) -> Vec<u8> {
    let mut out = Vec::new();
    write_canonical(&node.value, &mut out);
    out
}

fn write_canonical(value: &Json, out: &mut Vec<u8>) {
    match value {
        Json::Null => out.extend_from_slice(b"null"),
        Json::Bool(true) => out.extend_from_slice(b"true"),
        Json::Bool(false) => out.extend_from_slice(b"false"),
        Json::Number(n) => out.extend_from_slice(n.as_bytes()),
        Json::String(s) => write_string(s, out),
        Json::Array(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_canonical(&item.value, out);
            }
            out.push(b']');
        }
        Json::Object(members) => {
            let mut sorted: Vec<&(String, Node)> = members.iter().collect();
            sorted.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
            out.push(b'{');
            for (i, (k, v)) in sorted.into_iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_string(k, out);
                out.push(b':');
                write_canonical(&v.value, out);
            }
            out.push(b'}');
        }
    }
}

fn write_string(s: &str, out: &mut Vec<u8>) {
    out.push(b'"');
    for c in s.chars() {
        match c {
            '"' => out.extend_from_slice(b"\\\""),
            '\\' => out.extend_from_slice(b"\\\\"),
            c if (c as u32) < 0x20 => {
                out.extend_from_slice(format!("\\u{:04x}", c as u32).as_bytes())
            }
            c => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
    out.push(b'"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_huge_object_parses_in_linear_time() {
        // ~170k keys in just under 2 MiB (the bundle cap): quadratic
        // duplicate checks took minutes on this; it must take well under a
        // second, and a duplicate at the very end is still found.
        let mut doc = String::from("{");
        for n in 0..170_000 {
            doc.push_str(&format!("\"k{n:x}\":0,"));
        }
        doc.push_str("\"end\":1}");
        assert!(doc.len() < 2 * 1024 * 1024);
        let started = std::time::Instant::now();
        parse(doc.as_bytes()).unwrap();
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
        let dup = doc.replace("\"end\":1}", "\"k0\":1}");
        assert_eq!(parse(dup.as_bytes()).unwrap_err().what, "duplicate key");
    }

    #[test]
    fn parses_with_spans() {
        let text = br#" {"items":[{"sku":"A","price":12.50}],"ok":true} "#;
        let root = parse(text).unwrap();
        let price = root.pointer("/items/0/price").unwrap();
        assert_eq!(price.value, Json::Number("12.50".into()));
        assert_eq!(&text[price.span.clone()], b"12.50");
        let record = root.pointer("/items/0").unwrap();
        assert_eq!(&text[record.span.clone()], br#"{"sku":"A","price":12.50}"#);
        assert!(root.pointer("/items/1").is_none());
        assert!(root.pointer("/items/00").is_none());
    }

    #[test]
    fn strictness() {
        for bad in [
            &br#"{"a":1,"a":2}"#[..],
            b"[1,]",
            b"{\"a\":01}",
            b"\"\x01\"",
            b"[1] x",
            b"{'a':1}",
            b"1.",
            b"\"\\ud800\"",
            b"NaN",
        ] {
            assert!(parse(bad).is_err(), "{}", String::from_utf8_lossy(bad));
        }
        let mut deep = vec![b'['; MAX_DEPTH + 2];
        deep.extend(vec![b']'; MAX_DEPTH + 2]);
        assert!(parse(&deep).is_err());
    }

    #[test]
    fn escapes_and_pointers() {
        let root = parse(br#"{"a/b":{"m~n":"\u00e9\ud83d\ude00\n"}}"#).unwrap();
        assert_eq!(
            root.pointer("/a~1b/m~0n").unwrap().value,
            Json::String("é😀\n".into())
        );
        assert!(valid_pointer("/a~1b/m~0n"));
        assert!(!valid_pointer("a"));
        assert!(!valid_pointer("/a~2"));
    }

    #[test]
    fn canonical_ignores_layout_and_order() {
        let a = parse(br#"{ "b": [1, 2], "a": "x\"" }"#).unwrap();
        let b = parse(br#"{"a":"x\"","b":[1,2]}"#).unwrap();
        assert_eq!(canonical(&a), canonical(&b));
        assert_eq!(canonical(&a), br#"{"a":"x\"","b":[1,2]}"#.to_vec());
    }
}
