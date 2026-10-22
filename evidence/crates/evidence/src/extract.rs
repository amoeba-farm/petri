//! Applying a manifest to an authenticated HTTP exchange.
//!
//! The rules are fixed here, not in the manifest:
//!
//! - the request went to the manifest's origin and path, and the server
//!   answered `200`;
//! - the body parses as strict JSON (`crate::json`);
//! - exactly one record matches; none or several is an error, never a pick;
//! - every field comes from that one record, so a price is never paired
//!   with another row's product, unit or date;
//! - the price is an exact six-decimal value (`crate::decimal`);
//! - context fields the manifest names must be present.
//!
//! Values are recomputed here from the authenticated bytes; nothing a
//! bundle or a client states about them is used.

use std::ops::Range;

use crate::decimal::{self, DecimalError};
use crate::http::{Request, Response};
use crate::json::{self, Json, Node};
use crate::manifest::Manifest;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    /// Read from authenticated source bytes.
    Authenticated,
    /// Stated by the contributor; not verified.
    ContributorAnnotation,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Field {
    pub value: String,
    pub provenance: Provenance,
    /// Where an authenticated value sits inside the record (`None` for
    /// annotations).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
}

impl Field {
    fn authenticated(value: String, pointer: &str) -> Self {
        Self {
            value,
            provenance: Provenance::Authenticated,
            pointer: Some(pointer.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extracted {
    /// Absolute pointer of the matched record in the body (`/items/3`).
    pub record_pointer: String,
    /// The record's bytes in the body.
    pub record_span: Range<usize>,
    /// The price as printed, with its six-decimal atoms.
    pub price: Field,
    pub price_atoms: u64,
    /// The other named fields in manifest order, then the annotations.
    pub fields: Vec<(String, Field)>,
    /// Context pointer → canonical JSON of the value.
    pub context: Vec<(String, String)>,
}

impl Extracted {
    pub fn get(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|(k, _)| k == name).map(|(_, f)| f)
    }
    pub fn value(&self, name: &str) -> Option<&str> {
        self.get(name).map(|f| f.value.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExtractError {
    #[error("the proof is for {got}, the manifest reads {want}")]
    WrongOrigin { got: String, want: String },
    #[error("the proof is for the path {got}, the manifest reads {want}")]
    WrongPath { got: String, want: String },
    #[error("the source answered {0}, not 200")]
    Status(u16),
    #[error("the body is not JSON the manifest can read: {0}")]
    Body(String),
    #[error("`records` ({0}) is not in the body")]
    NoRecords(String),
    #[error("no record matches the manifest")]
    NoMatch,
    #[error("{0} records match the manifest; it must select exactly one")]
    Ambiguous(usize),
    #[error("the record has no `{field}` at {pointer}")]
    MissingField { field: String, pointer: String },
    #[error("`{field}` at {pointer} is not a string or number")]
    NotScalar { field: String, pointer: String },
    #[error("price: {0}")]
    Price(#[from] DecimalError),
}

fn matches(record: &Node, m: &Manifest) -> bool {
    m.matches.iter().all(
        |(p, want)| match (record.pointer(p).map(|n| &n.value), want) {
            (Some(Json::String(a)), Json::String(b)) => a == b,
            (Some(Json::Number(a)), Json::Number(b)) => a == b,
            (Some(Json::Bool(a)), Json::Bool(b)) => a == b,
            _ => false,
        },
    )
}

fn scalar(record: &Node, field: &str, pointer: &str) -> Result<String, ExtractError> {
    let node = record
        .pointer(pointer)
        .ok_or_else(|| ExtractError::MissingField {
            field: field.into(),
            pointer: pointer.into(),
        })?;
    match &node.value {
        Json::String(s) | Json::Number(s) => Ok(s.clone()),
        Json::Bool(b) => Ok(b.to_string()),
        _ => Err(ExtractError::NotScalar {
            field: field.into(),
            pointer: pointer.into(),
        }),
    }
}

/// The request's path without its query.
pub fn request_path(target: &str) -> &str {
    target.split_once('?').map_or(target, |(p, _)| p)
}

pub fn extract(
    m: &Manifest,
    request: &Request,
    response: &Response,
) -> Result<Extracted, ExtractError> {
    if request.host != m.origin {
        return Err(ExtractError::WrongOrigin {
            got: request.host.clone(),
            want: m.origin.clone(),
        });
    }
    let path = request_path(&request.target);
    if path != m.path {
        return Err(ExtractError::WrongPath {
            got: path.into(),
            want: m.path.clone(),
        });
    }
    if response.status != 200 {
        return Err(ExtractError::Status(response.status));
    }
    let root = json::parse(&response.body).map_err(|e| ExtractError::Body(e.to_string()))?;
    let container = root
        .pointer(&m.records)
        .ok_or_else(|| ExtractError::NoRecords(m.records.clone()))?;
    let (record_pointer, record) = match &container.value {
        Json::Array(items) => {
            let hits: Vec<(usize, &Node)> = items
                .iter()
                .enumerate()
                .filter(|(_, n)| matches(n, m))
                .collect();
            match hits.as_slice() {
                [] => return Err(ExtractError::NoMatch),
                [(i, n)] => (format!("{}/{i}", m.records), *n),
                many => return Err(ExtractError::Ambiguous(many.len())),
            }
        }
        Json::Object(_) => {
            if !matches(container, m) {
                return Err(ExtractError::NoMatch);
            }
            (m.records.clone(), container)
        }
        _ => return Err(ExtractError::NoRecords(m.records.clone())),
    };
    let price_ptr = m.field("price").unwrap_or_default();
    let price_text = scalar(record, "price", price_ptr)?;
    let price_atoms = decimal::parse_atoms(&price_text)?;
    let mut fields = Vec::new();
    for (name, ptr) in &m.fields {
        if name == "price" {
            continue;
        }
        fields.push((
            name.clone(),
            Field::authenticated(scalar(record, name, ptr)?, ptr),
        ));
    }
    for (name, value) in &m.assume {
        fields.push((
            name.clone(),
            Field {
                value: value.clone(),
                provenance: Provenance::ContributorAnnotation,
                pointer: None,
            },
        ));
    }
    let mut context = Vec::new();
    for ptr in &m.context {
        let node = record
            .pointer(ptr)
            .ok_or_else(|| ExtractError::MissingField {
                field: "context".into(),
                pointer: ptr.clone(),
            })?;
        context.push((
            ptr.clone(),
            String::from_utf8_lossy(&json::canonical(node)).into_owned(),
        ));
    }
    Ok(Extracted {
        record_pointer,
        record_span: record.span.clone(),
        price: Field::authenticated(price_text, price_ptr),
        price_atoms,
        fields,
        context,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::http::HeaderView;
    use crate::manifest::FIXTURE_MANIFEST;

    pub(crate) const BODY: &str = r#"{"fixture":true,"items":[
 {"id":"rec-1","revision":3,"sku":"DDR5-16G-4800","name":"DDR5 16GB 4800","unit_price":"41.25","currency":"USD","unit":"module","price_type":"list","min_qty":1,"region":"US","updated_at":"2026-10-19"},
 {"id":"rec-2","revision":1,"sku":"DDR5-32G-4800","name":"DDR5 32GB 4800","unit_price":"79.90","currency":"USD","unit":"module","price_type":"list","min_qty":1,"region":"US","updated_at":"2026-10-19"}
]}"#;

    pub(crate) fn exchange(
        host: &str,
        target: &str,
        status: u16,
        body: &str,
    ) -> (Request, Response) {
        (
            Request {
                method: "GET".into(),
                target: target.into(),
                host: host.into(),
                headers: vec![],
            },
            Response {
                status,
                headers: vec![HeaderView {
                    name: "Content-Type".into(),
                    value: Some("application/json".into()),
                }],
                content_type: Some("application/json".into()),
                body: body.as_bytes().to_vec(),
            },
        )
    }

    fn manifest() -> Manifest {
        Manifest::parse(FIXTURE_MANIFEST.as_bytes()).unwrap()
    }

    #[test]
    fn one_record_all_fields() {
        let (req, resp) = exchange("portal.fixture.test", "/api/prices?page=1", 200, BODY);
        let x = extract(&manifest(), &req, &resp).unwrap();
        assert_eq!(x.record_pointer, "/items/0");
        assert_eq!(
            (x.price.value.as_str(), x.price_atoms),
            ("41.25", 41_250_000)
        );
        assert_eq!(x.value("currency"), Some("USD"));
        assert_eq!(x.value("revision"), Some("3"));
        assert_eq!(x.value("observedAt"), Some("2026-10-19"));
        assert_eq!(
            x.context,
            vec![("/region".to_string(), "\"US\"".to_string())]
        );
        assert!(BODY[x.record_span.clone()].contains("rec-1"));
    }

    #[test]
    fn binding_refusals() {
        let m = manifest();
        let (req, resp) = exchange("evil.fixture.test", "/api/prices", 200, BODY);
        assert!(matches!(
            extract(&m, &req, &resp),
            Err(ExtractError::WrongOrigin { .. })
        ));
        let (req, resp) = exchange("portal.fixture.test", "/api/other", 200, BODY);
        assert!(matches!(
            extract(&m, &req, &resp),
            Err(ExtractError::WrongPath { .. })
        ));
        let (req, resp) = exchange("portal.fixture.test", "/api/prices", 403, BODY);
        assert_eq!(extract(&m, &req, &resp), Err(ExtractError::Status(403)));
    }

    #[test]
    fn mixed_rows_and_ambiguity() {
        // Two rows match: never pick one.
        let doubled = BODY.replace("DDR5-32G-4800", "DDR5-16G-4800");
        let (req, resp) = exchange("portal.fixture.test", "/api/prices", 200, &doubled);
        assert_eq!(
            extract(&manifest(), &req, &resp),
            Err(ExtractError::Ambiguous(2))
        );
        // The matched row lacks a field: never borrow it from another row.
        let missing = BODY.replacen(r#""currency":"USD","#, "", 1);
        let (req, resp) = exchange("portal.fixture.test", "/api/prices", 200, &missing);
        assert!(matches!(
            extract(&manifest(), &req, &resp),
            Err(ExtractError::MissingField { .. })
        ));
        let none = BODY.replace("DDR5-16G-4800", "DDR4");
        let (req, resp) = exchange("portal.fixture.test", "/api/prices", 200, &none);
        assert_eq!(
            extract(&manifest(), &req, &resp),
            Err(ExtractError::NoMatch)
        );
    }

    #[test]
    fn price_must_be_exact() {
        for (price, ok) in [
            ("\"41.25\"", true),
            ("41.25", true),
            ("4.125e1", false),
            ("\"$41.25\"", false),
            ("\"41.2500001\"", false),
        ] {
            let body = BODY.replacen("\"41.25\"", price, 1);
            let (req, resp) = exchange("portal.fixture.test", "/api/prices", 200, &body);
            assert_eq!(extract(&manifest(), &req, &resp).is_ok(), ok, "{price}");
        }
    }
}
