//! The governed product SKU sets (the monorepo oracle's `governance/`
//! directory, compiled in) and the SKU Merkle proofs that source
//! proposals and supports carry. A bucket is one SKU: its id is the SKU label's
//! ASCII bytes, right-padded with zeros to 32 bytes.

use ameba_oracle::constants::{
    ORACLE_SKU_EMPTY_HASH_DOMAIN, ORACLE_SKU_LEAF_HASH_DOMAIN, ORACLE_SKU_NODE_HASH_DOMAIN,
};
use serde_json::Value;
use solana_program::hash::hashv;

use crate::futures::error::{Error, Result, bail};

const RAMX: &str = include_str!("../../../oracle/governance/ramx_product_sku_manifest.v1.json");
const NANDX: &str = include_str!("../../../oracle/governance/nandx_product_sku_manifest.v1.json");

/// One product's governed SKU set, ordered by encoded id as the program commits it.
#[derive(Clone, Debug)]
pub struct SkuSet {
    pub symbol: String,
    pub ids: Vec<[u8; 32]>,
    pub root: [u8; 32],
}

/// `label` as a bucket id: printable ASCII, 1 to 32 bytes, right-padded with zeros.
pub fn encode_label(label: &str) -> Result<[u8; 32]> {
    let bytes = label.as_bytes();
    if bytes.is_empty() || bytes.len() > 32 || !bytes.iter().all(|b| (0x20..0x7f).contains(b)) {
        bail(format!(
            "`{label}` is not a SKU label (1 to 32 printable ASCII bytes)"
        ))?;
    }
    let mut id = [0u8; 32];
    id[..bytes.len()].copy_from_slice(bytes);
    Ok(id)
}

/// The label of a bucket id, when it is a padded ASCII label.
pub fn decode_label(id: &[u8; 32]) -> Option<String> {
    let end = id.iter().position(|b| *b == 0).unwrap_or(32);
    if end == 0 || id[end..].iter().any(|b| *b != 0) {
        return None;
    }
    let text = std::str::from_utf8(&id[..end]).ok()?;
    text.bytes()
        .all(|b| (0x20..0x7f).contains(&b))
        .then(|| text.to_string())
}

fn labels(value: &Value, out: &mut Vec<String>) {
    if let Some(list) = value.get("skuLabels").and_then(Value::as_array) {
        out.extend(list.iter().filter_map(Value::as_str).map(str::to_string));
    }
    for key in ["groups", "subgroups"] {
        if let Some(children) = value.get(key).and_then(Value::as_array) {
            for child in children {
                labels(child, out);
            }
        }
    }
}

fn depth(count: usize) -> usize {
    let mut width = 1usize;
    let mut depth = 0;
    while width < count {
        width *= 2;
        depth += 1;
    }
    depth
}

fn leaf(index: usize, id: Option<&[u8; 32]>) -> [u8; 32] {
    let index = (index as u16).to_le_bytes();
    match id {
        Some(id) => hashv(&[ORACLE_SKU_LEAF_HASH_DOMAIN, &index, id]).to_bytes(),
        None => hashv(&[ORACLE_SKU_EMPTY_HASH_DOMAIN, &index]).to_bytes(),
    }
}

fn node(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    hashv(&[ORACLE_SKU_NODE_HASH_DOMAIN, left, right]).to_bytes()
}

impl SkuSet {
    fn parse(text: &str) -> Result<Self> {
        let value: Value = serde_json::from_str(text)
            .map_err(|e| Error::msg(format!("governed SKU manifest is not JSON: {e}")))?;
        let symbol = value
            .pointer("/product/symbol")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::msg("governed SKU manifest has no product symbol"))?
            .to_string();
        let mut names = Vec::new();
        labels(&value, &mut names);
        let mut ids = names
            .iter()
            .map(|l| encode_label(l))
            .collect::<Result<Vec<_>>>()?;
        ids.sort_unstable();
        ids.dedup();
        let required = value
            .get("requiredSkuCount")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        if ids.len() != required || required == 0 {
            bail(format!(
                "governed {symbol} SKU manifest lists {} distinct SKUs, not {required}",
                ids.len()
            ))?;
        }
        let mut set = Self {
            symbol,
            ids,
            root: [0; 32],
        };
        set.root = set.compute_root();
        let declared = value
            .get("requiredSkuRootHex")
            .and_then(Value::as_str)
            .unwrap_or("");
        if hex(&set.root) != declared {
            bail(format!(
                "governed {} SKU manifest root does not match its declared root",
                set.symbol
            ))?;
        }
        Ok(set)
    }

    /// A set from already-encoded ids (sorted and deduplicated here).
    pub fn from_ids(symbol: &str, mut ids: Vec<[u8; 32]>) -> Self {
        ids.sort_unstable();
        ids.dedup();
        let mut set = Self {
            symbol: symbol.to_string(),
            ids,
            root: [0; 32],
        };
        set.root = set.compute_root();
        set
    }

    fn level0(&self) -> Vec<[u8; 32]> {
        let width = 1usize << depth(self.ids.len());
        (0..width).map(|i| leaf(i, self.ids.get(i))).collect()
    }

    fn compute_root(&self) -> [u8; 32] {
        if self.ids.is_empty() {
            return [0; 32];
        }
        let mut nodes = self.level0();
        while nodes.len() > 1 {
            nodes = nodes.chunks(2).map(|p| node(&p[0], &p[1])).collect();
        }
        nodes[0]
    }

    /// The index and sibling path of a SKU id.
    pub fn proof(&self, id: &[u8; 32]) -> Result<(u16, Vec<[u8; 32]>)> {
        let index = self.ids.binary_search(id).map_err(|_| {
            Error::msg(format!(
                "`{}` is not a governed {} SKU",
                decode_label(id).unwrap_or_else(|| hex(id)),
                self.symbol
            ))
        })?;
        let mut nodes = self.level0();
        let mut position = index;
        let mut path = Vec::new();
        while nodes.len() > 1 {
            path.push(nodes[position ^ 1]);
            nodes = nodes.chunks(2).map(|p| node(&p[0], &p[1])).collect();
            position /= 2;
        }
        Ok((index as u16, path))
    }
}

/// The governed SKU set of a product symbol (`RAMX`, `NANDX`).
pub fn for_symbol(symbol: &str) -> Result<SkuSet> {
    match symbol.to_ascii_uppercase().as_str() {
        "RAMX" => SkuSet::parse(RAMX),
        "NANDX" => SkuSet::parse(NANDX),
        other => bail(format!("no governed SKU set for product {other}")),
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Parses 64 hex characters into 32 bytes.
pub fn parse_hex32(text: &str, what: &str) -> Result<[u8; 32]> {
    let text = text.trim().trim_start_matches("0x");
    if text.len() != 64 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail(format!("{what} must be 64 hex characters"))?;
    }
    let mut out = [0u8; 32];
    for (i, chunk) in text.as_bytes().chunks(2).enumerate() {
        out[i] = u8::from_str_radix(std::str::from_utf8(chunk).unwrap_or("00"), 16)
            .map_err(|_| Error::msg(format!("{what} must be 64 hex characters")))?;
    }
    Ok(out)
}

/// A bucket given as a SKU label or as its 64-hex id.
pub fn parse_bucket(text: &str) -> Result<[u8; 32]> {
    let trimmed = text.trim();
    if trimmed.len() == 64 && trimmed.bytes().all(|b| b.is_ascii_hexdigit()) {
        return parse_hex32(trimmed, "bucket id");
    }
    encode_label(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn governed_roots_match_the_oracle_programs_pinned_roots() {
        let ramx = for_symbol("ramx").unwrap();
        assert_eq!(ramx.ids.len(), 52);
        assert!(hex(&ramx.root).starts_with("52a574e7"));
        let nandx = for_symbol("NANDX").unwrap();
        assert_eq!(nandx.ids.len(), 48);
        assert!(hex(&nandx.root).starts_with("41bb5dc7"));
    }

    #[test]
    fn proofs_verify_like_the_program() {
        let set = for_symbol("RAMX").unwrap();
        for id in set.ids.iter().step_by(7) {
            let (index, path) = set.proof(id).unwrap();
            let mut current = leaf(usize::from(index), Some(id));
            for (level, sibling) in path.iter().enumerate() {
                current = if (usize::from(index) >> level) & 1 == 0 {
                    node(&current, sibling)
                } else {
                    node(sibling, &current)
                };
            }
            assert_eq!(current, set.root);
        }
        assert!(set.proof(&encode_label("NOT-A-SKU").unwrap()).is_err());
    }

    #[test]
    fn labels_round_trip_and_reject_bad_input() {
        let id = encode_label("KSM64R52BD8-32HA").unwrap();
        assert_eq!(decode_label(&id).as_deref(), Some("KSM64R52BD8-32HA"));
        assert_eq!(parse_bucket("KSM64R52BD8-32HA").unwrap(), id);
        assert_eq!(parse_bucket(&hex(&id)).unwrap(), id);
        assert!(encode_label("").is_err());
        assert!(encode_label(&"x".repeat(33)).is_err());
        assert!(encode_label("tab\there").is_err());
    }
}
