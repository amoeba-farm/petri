//! Extraction manifests: how to read one economic record out of an
//! authenticated response.
//!
//! Anyone may write one, for any source, without Amoeba's approval. A
//! manifest is untrusted input: it only says where to look. The verifier
//! applies it to the authenticated bytes itself, under fixed rules
//! (`crate::extract`): exactly one record, every field from that record,
//! exact decimals. It is declarative and bounded (JSON pointers, literal
//! matches); there is no executable adapter to sandbox.
//!
//! A manifest is content-addressed by its canonical JSON
//! (`manifest_hash`). Its `selector_digest` covers only what fixes the
//! economic series (origin, path, record selection, price, currency, unit,
//! product), so a frozen source can take an updated manifest (a renamed
//! label, an added context field) while a change to the tracked field shows
//! up as a different selector.

use crate::hash::tagged;
use crate::json::{self, Json, Node};

pub const MANIFEST_FORMAT: &str = "amoeba-extraction-manifest-v1";
pub const MANIFEST_HASH_DOMAIN: &str = "amoeba-extraction-manifest-v1";
pub const SELECTOR_DOMAIN: &str = "amoeba-extraction-selector-v1";
/// Bound on hostile input (prompt v2, section 9: "enforce size/time limits").
pub const MAX_MANIFEST_BYTES: usize = 16 * 1024;

/// The economic kind of record the manifest reads. Authentication never
/// changes it: a quote proven over TLS is still a quote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceType {
    /// A price a seller lists for anyone with access (catalog, price list).
    CatalogPrice,
    /// A price offered to this buyer (it may be negotiated).
    Quotation,
    /// A published index, benchmark or report value.
    PublishedBenchmark,
    /// A completed transaction (an order, invoice or fill).
    TransactionRecord,
}

impl EvidenceType {
    pub(crate) fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "catalog_price" => Self::CatalogPrice,
            "quotation" => Self::Quotation,
            "published_benchmark" => Self::PublishedBenchmark,
            "transaction_record" => Self::TransactionRecord,
            _ => return None,
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CatalogPrice => "catalog_price",
            Self::Quotation => "quotation",
            Self::PublishedBenchmark => "published_benchmark",
            Self::TransactionRecord => "transaction_record",
        }
    }
}

/// The record fields a manifest can name, all read from the one matched
/// record.
pub const FIELDS: &[&str] = &[
    "price",
    "currency",
    "unit",
    "product",
    "productName",
    "priceType",
    "quantityTier",
    "terms",
    "recordId",
    "revision",
    "observedAt",
];
/// Fields a contributor may state when the source does not print them. The
/// statement stays an unverified annotation.
pub const ASSUMABLE: &[&str] = &["currency", "unit", "priceType", "quantityTier"];

#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub name: String,
    pub evidence_type: EvidenceType,
    /// The server name the proof must be for (lowercase DNS name).
    pub origin: String,
    /// The request path (no query) the proof must be for.
    pub path: String,
    /// Pointer to the array of records (or to the one record object).
    pub records: String,
    /// Record-relative pointer → literal the record must hold.
    pub matches: Vec<(String, Json)>,
    /// Field name → record-relative pointer.
    pub fields: Vec<(String, String)>,
    /// Record-relative pointers that must be present (commercial qualifiers).
    pub context: Vec<String>,
    /// Field name → contributor-stated value (unverified).
    pub assume: Vec<(String, String)>,
    /// The canonical bytes (what `manifest_hash` covers).
    pub canonical: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid extraction manifest: {0}")]
pub struct ManifestError(pub String);

fn bad<T>(what: impl Into<String>) -> Result<T, ManifestError> {
    Err(ManifestError(what.into()))
}

fn dns_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 253
        && name.split('.').count() >= 2
        && name.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        })
}

fn string<'a>(node: &'a Node, key: &str) -> Result<&'a str, ManifestError> {
    match node.pointer(&format!("/{key}")).map(|n| &n.value) {
        Some(Json::String(s)) => Ok(s),
        Some(_) => bad(format!("`{key}` must be a string")),
        None => bad(format!("`{key}` is missing")),
    }
}

fn pointer(text: &str, what: &str) -> Result<String, ManifestError> {
    if !json::valid_pointer(text) || text.len() > 256 {
        return bad(format!("{what} `{text}` is not a JSON pointer"));
    }
    Ok(text.to_string())
}

impl Manifest {
    pub fn parse(bytes: &[u8]) -> Result<Self, ManifestError> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return bad("larger than 16 KiB");
        }
        let root = json::parse(bytes).map_err(|e| ManifestError(e.to_string()))?;
        let Json::Object(members) = &root.value else {
            return bad("not a JSON object");
        };
        const KEYS: &[&str] = &[
            "format",
            "name",
            "evidenceType",
            "origin",
            "path",
            "content",
            "records",
            "match",
            "fields",
            "context",
            "assume",
        ];
        if let Some((k, _)) = members.iter().find(|(k, _)| !KEYS.contains(&k.as_str())) {
            return bad(format!("unknown key `{k}`"));
        }
        if string(&root, "format")? != MANIFEST_FORMAT {
            return bad(format!("`format` must be `{MANIFEST_FORMAT}`"));
        }
        let name = string(&root, "name")?.to_string();
        if name.is_empty() || name.len() > 200 {
            return bad("`name` must be 1-200 characters");
        }
        let evidence_type = EvidenceType::parse(string(&root, "evidenceType")?)
            .ok_or_else(|| ManifestError("`evidenceType` must be catalog_price, quotation, published_benchmark or transaction_record".into()))?;
        let origin = string(&root, "origin")?.to_string();
        if !dns_name(&origin) {
            return bad("`origin` must be a lowercase DNS name");
        }
        let path = string(&root, "path")?.to_string();
        if !path.starts_with('/')
            || path.contains(['?', '#'])
            || path.bytes().any(|c| c <= b' ' || c >= 0x7f)
        {
            return bad("`path` must be an absolute path without query or fragment");
        }
        let content = string(&root, "content")?;
        if content != "json" {
            return bad(format!("content `{content}` is not supported (only json)"));
        }
        let records = pointer(string(&root, "records")?, "`records`")?;
        let mut matches = Vec::new();
        if let Some(m) = root.pointer("/match") {
            let Json::Object(pairs) = &m.value else {
                return bad("`match` must be an object");
            };
            for (p, v) in pairs {
                let p = pointer(p, "a `match` key")?;
                if p.is_empty() {
                    return bad("a `match` key must name a field");
                }
                match &v.value {
                    Json::String(_) | Json::Number(_) | Json::Bool(_) => {
                        matches.push((p, v.value.clone()))
                    }
                    _ => return bad("`match` values must be strings, numbers or booleans"),
                }
            }
        }
        let mut fields = Vec::new();
        let Some(Node {
            value: Json::Object(pairs),
            ..
        }) = root.pointer("/fields")
        else {
            return bad("`fields` must be an object");
        };
        for (k, v) in pairs {
            if !FIELDS.contains(&k.as_str()) {
                return bad(format!("unknown field `{k}`"));
            }
            let Json::String(p) = &v.value else {
                return bad(format!("field `{k}` must be a JSON pointer string"));
            };
            let p = pointer(p, &format!("field `{k}`"))?;
            if p.is_empty() {
                return bad(format!("field `{k}` must name a field of the record"));
            }
            fields.push((k.clone(), p));
        }
        if !fields.iter().any(|(k, _)| k == "price") {
            return bad("`fields.price` is required");
        }
        let mut context = Vec::new();
        if let Some(c) = root.pointer("/context") {
            let Json::Array(items) = &c.value else {
                return bad("`context` must be an array");
            };
            if items.len() > 32 {
                return bad("`context` names more than 32 fields");
            }
            for item in items {
                let Json::String(p) = &item.value else {
                    return bad("`context` entries must be JSON pointer strings");
                };
                context.push(pointer(p, "a `context` entry")?);
            }
        }
        let mut assume = Vec::new();
        if let Some(a) = root.pointer("/assume") {
            let Json::Object(pairs) = &a.value else {
                return bad("`assume` must be an object");
            };
            for (k, v) in pairs {
                if !ASSUMABLE.contains(&k.as_str()) {
                    return bad(format!(
                        "`assume.{k}` is not allowed (only {})",
                        ASSUMABLE.join(", ")
                    ));
                }
                if fields.iter().any(|(f, _)| f == k) {
                    return bad(format!(
                        "`assume.{k}` contradicts `fields.{k}`: the record's own value wins"
                    ));
                }
                let Json::String(s) = &v.value else {
                    return bad(format!("`assume.{k}` must be a string"));
                };
                if s.is_empty() || s.len() > 100 {
                    return bad(format!("`assume.{k}` must be 1-100 characters"));
                }
                assume.push((k.clone(), s.clone()));
            }
        }
        Ok(Self {
            name,
            evidence_type,
            origin,
            path,
            records,
            matches,
            fields,
            context,
            assume,
            canonical: json::canonical(&root),
        })
    }

    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, p)| p.as_str())
    }

    /// The content address: `tagged(manifest domain, canonical JSON)`.
    pub fn manifest_hash(&self) -> [u8; 32] {
        tagged(MANIFEST_HASH_DOMAIN, &[&self.canonical])
    }

    /// What fixes the economic series: origin, path, record selection and
    /// the price, currency, unit and product pointers.
    pub fn selector_digest(&self) -> [u8; 32] {
        let mut matches: Vec<String> = self
            .matches
            .iter()
            .map(|(p, v)| {
                let node = Node {
                    value: v.clone(),
                    span: 0..0,
                };
                format!("{p}={}", String::from_utf8_lossy(&json::canonical(&node)))
            })
            .collect();
        matches.sort();
        let matches = matches.join("\n");
        let f = |k: &str| self.field(k).unwrap_or("").as_bytes().to_vec();
        tagged(
            SELECTOR_DOMAIN,
            &[
                self.origin.as_bytes(),
                self.path.as_bytes(),
                self.records.as_bytes(),
                matches.as_bytes(),
                &f("price"),
                &f("currency"),
                &f("unit"),
                &f("product"),
            ],
        )
    }
}

#[cfg(test)]
pub(crate) const FIXTURE_MANIFEST: &str = r#"{
  "format": "amoeba-extraction-manifest-v1",
  "name": "Fixture distributor: DDR5 16GB module",
  "evidenceType": "catalog_price",
  "origin": "portal.fixture.test",
  "path": "/api/prices",
  "content": "json",
  "records": "/items",
  "match": { "/sku": "DDR5-16G-4800" },
  "fields": {
    "price": "/unit_price",
    "currency": "/currency",
    "unit": "/unit",
    "product": "/sku",
    "productName": "/name",
    "priceType": "/price_type",
    "quantityTier": "/min_qty",
    "recordId": "/id",
    "revision": "/revision",
    "observedAt": "/updated_at"
  },
  "context": ["/region"]
}"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_addresses() {
        let m = Manifest::parse(FIXTURE_MANIFEST.as_bytes()).unwrap();
        assert_eq!(m.origin, "portal.fixture.test");
        assert_eq!(m.field("price"), Some("/unit_price"));
        // Layout and key order do not change the address.
        let compact = String::from_utf8(m.canonical.clone()).unwrap();
        assert_eq!(
            Manifest::parse(compact.as_bytes()).unwrap().manifest_hash(),
            m.manifest_hash()
        );
    }

    #[test]
    fn selector_tracks_the_economic_field_only() {
        let m = Manifest::parse(FIXTURE_MANIFEST.as_bytes()).unwrap();
        let renamed = FIXTURE_MANIFEST.replace("Fixture distributor", "Renamed");
        let renamed = Manifest::parse(renamed.as_bytes()).unwrap();
        assert_ne!(renamed.manifest_hash(), m.manifest_hash());
        assert_eq!(renamed.selector_digest(), m.selector_digest());
        let other_field = FIXTURE_MANIFEST.replace("\"/unit_price\"", "\"/list_price\"");
        assert_ne!(
            Manifest::parse(other_field.as_bytes())
                .unwrap()
                .selector_digest(),
            m.selector_digest()
        );
        let other_row = FIXTURE_MANIFEST.replace("DDR5-16G-4800", "DDR5-32G-4800");
        assert_ne!(
            Manifest::parse(other_row.as_bytes())
                .unwrap()
                .selector_digest(),
            m.selector_digest()
        );
    }

    #[test]
    fn refusals() {
        let cases = [
            FIXTURE_MANIFEST.replace("\"json\"", "\"html\""),
            FIXTURE_MANIFEST.replace("portal.fixture.test", "Portal.Fixture.Test"),
            FIXTURE_MANIFEST.replace("\"/api/prices\"", "\"/api/prices?x=1\""),
            FIXTURE_MANIFEST.replace("\"price\": \"/unit_price\",", ""),
            FIXTURE_MANIFEST.replace("\"context\"", "\"script\""),
            FIXTURE_MANIFEST.replace(
                "\"context\": [\"/region\"]",
                "\"assume\": {\"currency\": \"USD\"}",
            ),
            FIXTURE_MANIFEST.replace("catalog_price", "trusted_price"),
        ];
        for case in cases {
            assert!(Manifest::parse(case.as_bytes()).is_err(), "{case}");
        }
        let big = format!("{{\"name\":\"{}\"}}", "x".repeat(MAX_MANIFEST_BYTES));
        assert!(Manifest::parse(big.as_bytes()).is_err());
    }
}
