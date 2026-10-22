//! The governed product SKU sets (the monorepo oracle's `governance/`
//! directory, compiled in) and the SKU Merkle proofs that source
//! proposals and supports carry. A bucket is one SKU: its id is the SKU label's
//! ASCII bytes, right-padded with zeros to 32 bytes.
//!
//! A product has up to two governed sets. Its part-number list (manifest v1:
//! RAMX's 52 modules, NANDX's 48 SSDs) is what every month up to the launch
//! months and the CFM registry commit to; its commodity chip buckets
//! (`ameba_commodity_spec`, manifest v2) are what every month expiring after
//! 2026-11-01 commits to (the user moved RAMX and NANDX from modules to chips
//! on 2026-10-20). A month's coverage root says which one it is.

use ameba_commodity_spec::{Bucket, Product};
use ameba_oracle::constants::{
    ORACLE_SKU_EMPTY_HASH_DOMAIN, ORACLE_SKU_LEAF_HASH_DOMAIN, ORACLE_SKU_NODE_HASH_DOMAIN,
};
use serde_json::Value;
use solana_program::hash::hashv;

use crate::futures::error::{Error, Result, bail};

/// The part-number lists (manifest v1), by product symbol.
const PART_NUMBERS: &[(&str, &str)] = &[
    (
        "RAMX",
        include_str!("../../../oracle/governance/ramx_product_sku_manifest.v1.json"),
    ),
    (
        "NANDX",
        include_str!("../../../oracle/governance/nandx_product_sku_manifest.v1.json"),
    ),
];

/// Which governed list a set is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetKind {
    /// Module and SSD part numbers (manifest v1): months up to the launch months.
    PartNumbers,
    /// Commodity chip buckets (manifest v2): months expiring after 2026-11-01.
    Chips,
}

/// One product's governed SKU set, ordered by encoded id as the program commits it.
#[derive(Clone, Debug)]
pub struct SkuSet {
    pub symbol: String,
    pub ids: Vec<[u8; 32]>,
    pub root: [u8; 32],
    /// None for a set read from a month's own source buckets.
    pub kind: Option<SetKind>,
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
            kind: None,
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
            kind: None,
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

/// A product's part-number list (manifest v1): RAMX's 52 modules, NANDX's 48
/// SSDs. A product added with chip buckets only has none.
pub fn part_numbers(symbol: &str) -> Result<SkuSet> {
    let symbol = symbol.trim().to_ascii_uppercase();
    let (_, json) = PART_NUMBERS
        .iter()
        .find(|(s, _)| *s == symbol)
        .ok_or_else(|| Error::msg(format!("no governed part-number list for product {symbol}")))?;
    let mut set = SkuSet::parse(json)?;
    set.kind = Some(SetKind::PartNumbers);
    Ok(set)
}

/// A product's commodity chip buckets (manifest v2), held to the root the
/// commodity spec pins.
pub fn chips(product: Product) -> Result<SkuSet> {
    let manifest = product.manifest();
    let mut set = SkuSet::parse(manifest.json)?;
    if set.root != manifest.root() || set.ids != manifest.sorted_ids() {
        bail(format!(
            "governed {} chip manifest is not the commodity spec's",
            product.symbol()
        ))?;
    }
    set.kind = Some(SetKind::Chips);
    Ok(set)
}

/// Every governed SKU set of a product symbol (`RAMX`, `NANDX`): its
/// part-number list, then its chip buckets.
pub fn governed(symbol: &str) -> Result<Vec<SkuSet>> {
    let mut sets = Vec::new();
    if PART_NUMBERS
        .iter()
        .any(|(s, _)| s.eq_ignore_ascii_case(symbol.trim()))
    {
        sets.push(part_numbers(symbol)?);
    }
    if let Some(product) = Product::from_symbol(symbol.trim()) {
        sets.push(chips(product)?);
    }
    if sets.is_empty() {
        bail(format!("no governed SKU set for product {}", symbol.trim()))?;
    }
    Ok(sets)
}

/// The governed set of `symbol` whose root is `root` (a month's coverage root).
pub fn for_root(symbol: &str, root: &[u8; 32]) -> Option<SkuSet> {
    governed(symbol)
        .ok()?
        .into_iter()
        .find(|set| set.root == *root)
}

/// The commodity bucket a bucket id is, in any product.
pub fn chip_bucket(id: &[u8; 32]) -> Option<&'static Bucket> {
    Bucket::from_id32(id)
}

/// What people read for a bucket: a chip bucket's name ("DDR5 16Gb"), else
/// its SKU label ("KSM64R52BD8-32HA"), else its hex id.
pub fn bucket_name(id: &[u8; 32]) -> String {
    chip_bucket(id)
        .map(|bucket| bucket.name.to_string())
        .or_else(|| decode_label(id))
        .unwrap_or_else(|| hex(id))
}

/// `bucket_name` of a bucket given as a label or a 64-hex id (as the API's
/// documents name them); None when it is neither.
pub fn name_of(text: &str) -> Option<String> {
    let text = text.trim();
    if text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return id_name(text);
    }
    let id = encode_label(text).ok()?;
    Some(bucket_name(&id))
}

/// `bucket_name` of a 64-hex bucket id, when the id is a label.
pub fn id_name(id: &str) -> Option<String> {
    let id = parse_hex32(id, "bucket").ok()?;
    let name = bucket_name(&id);
    (name != hex(&id)).then_some(name)
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

/// A bucket given as a SKU label, as its 64-hex id, or as a chip bucket's
/// (or research candidate's) name or id in any case ("DDR5 16Gb",
/// "ddr5_16gbit").
pub fn parse_bucket(text: &str) -> Result<[u8; 32]> {
    let trimmed = text.trim();
    if trimmed.len() == 64 && trimmed.bytes().all(|b| b.is_ascii_hexdigit()) {
        return parse_hex32(trimmed, "bucket id");
    }
    let chip = Product::ALL
        .iter()
        .flat_map(|product| {
            product
                .buckets()
                .iter()
                .chain(product.research_candidates())
        })
        .find(|b| b.name.eq_ignore_ascii_case(trimmed) || b.id.eq_ignore_ascii_case(trimmed));
    match chip {
        Some(bucket) => Ok(bucket.id32()),
        None => encode_label(trimmed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn governed_roots_match_the_oracle_programs_pinned_roots() {
        // The part-number lists: older months and the CFM registry.
        let ramx = part_numbers("ramx").unwrap();
        assert_eq!(ramx.ids.len(), 52);
        assert!(hex(&ramx.root).starts_with("52a574e7"));
        let nandx = part_numbers("NANDX").unwrap();
        assert_eq!(nandx.ids.len(), 48);
        assert!(hex(&nandx.root).starts_with("41bb5dc7"));
        // The chip buckets: months expiring after 2026-11-01.
        let ramx = chips(Product::Ramx).unwrap();
        // Four since 2026-10-21: DDR5 32Gb is a research candidate, not registered.
        assert_eq!(ramx.ids.len(), 4);
        assert!(hex(&ramx.root).starts_with("bdd88959"));
        assert!(
            ramx.proof(&encode_label("DDR5_32GBIT").unwrap())
                .unwrap_err()
                .message
                .contains("is not a governed RAMX SKU")
        );
        assert_eq!(
            hex(&ramx.root),
            Product::Ramx.manifest().required_sku_root_hex
        );
        let nandx = chips(Product::Nandx).unwrap();
        assert_eq!(nandx.ids.len(), 4);
        assert!(hex(&nandx.root).starts_with("918d6559"));
        // RAMX and NANDX have both (part numbers first); every product its
        // buckets; a month's root picks one.
        for product in Product::ALL {
            let sets = governed(product.symbol()).unwrap();
            let kinds: Vec<_> = sets.iter().map(|s| s.kind).collect();
            assert_eq!(kinds.last(), Some(&Some(SetKind::Chips)));
            assert_eq!(
                kinds.contains(&Some(SetKind::PartNumbers)),
                PART_NUMBERS.iter().any(|(s, _)| *s == product.symbol())
            );
            for set in &sets {
                assert_eq!(
                    for_root(product.symbol(), &set.root).unwrap().kind,
                    set.kind
                );
            }
        }
        assert!(for_root("RAMX", &[7; 32]).is_none());
        assert!(governed("BTCX").is_err());
    }

    #[test]
    fn proofs_verify_like_the_program() {
        let verify = |set: &SkuSet, id: &[u8; 32]| {
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
        };
        let set = part_numbers("RAMX").unwrap();
        for id in set.ids.iter().step_by(7) {
            verify(&set, id);
        }
        assert!(set.proof(&encode_label("NOT-A-SKU").unwrap()).is_err());
        for product in Product::ALL {
            let set = chips(product).unwrap();
            for bucket in product.buckets() {
                verify(&set, &bucket.id32());
            }
            // A part number is not a chip bucket.
            assert!(
                set.proof(&encode_label("KSM64R52BD8-32HA").unwrap())
                    .is_err()
            );
        }
    }

    #[test]
    fn chip_buckets_read_by_their_names() {
        let id = encode_label("DDR5_16GBIT").unwrap();
        assert_eq!(bucket_name(&id), "DDR5 16Gb");
        assert_eq!(name_of("DDR5_16GBIT").as_deref(), Some("DDR5 16Gb"));
        assert_eq!(name_of(&hex(&id)).as_deref(), Some("DDR5 16Gb"));
        assert_eq!(name_of("tlc_1tb").as_deref(), Some("1Tb TLC"));
        // Part numbers read as themselves; an unlabelled id has no name.
        assert_eq!(
            name_of("KSM64R52BD8-32HA").as_deref(),
            Some("KSM64R52BD8-32HA")
        );
        assert_eq!(name_of(&"ff".repeat(32)), None);
        assert_eq!(id_name("a165995eac5c"), None);
        assert_eq!(id_name(&hex(&id)).as_deref(), Some("DDR5 16Gb"));
        assert_eq!(bucket_name(&[0xff; 32]), "ff".repeat(32));
        // A chip bucket is given by its id, its name, in any case.
        for text in ["DDR5_16GBIT", "ddr5_16gbit", "DDR5 16Gb", "ddr5 16gb"] {
            assert_eq!(parse_bucket(text).unwrap(), id, "{text}");
        }
        assert_eq!(
            parse_bucket("512gb tlc").unwrap(),
            encode_label("tlc_512gb").unwrap()
        );
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
