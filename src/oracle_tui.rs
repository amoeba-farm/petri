use serde_json::Value;
use std::collections::{HashMap, HashSet};

pub(crate) const DEFAULT_ORACLE_NODE_INDEX: usize = 0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OracleNodeKind {
    Market,
    Generation,
    FormFactor,
    RowBucket,
    TerminalPin,
}

impl OracleNodeKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Market => "market recipe",
            Self::Generation => "memory generation",
            Self::FormFactor => "form factor",
            Self::RowBucket => "product row",
            Self::TerminalPin => "source pin",
        }
    }

    fn from_value(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "market" | "root" | "root_recipe" => Some(Self::Market),
            "generation" | "memory_generation" => Some(Self::Generation),
            "form_factor" | "form factor" | "formfactor" => Some(Self::FormFactor),
            "row_bucket" | "row bucket" | "rowbucket" | "bucket" => Some(Self::RowBucket),
            "terminal_pin" | "terminal pin" | "terminalpin" | "pin" | "source" => {
                Some(Self::TerminalPin)
            }
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RamxOracleNode {
    pub(crate) node_id: String,
    pub(crate) label: String,
    pub(crate) kind: OracleNodeKind,
    pub(crate) parent: Option<usize>,
    pub(crate) weight_bps: u32,
    pub(crate) row_weight_bps: u32,
    pub(crate) weight_pct: f64,
    pub(crate) row_weight_pct: f64,
    pub(crate) pin_count: u16,
    pub(crate) children: Vec<usize>,
    pub(crate) description: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OracleIndexTree {
    pub(crate) market_id: String,
    pub(crate) symbol: String,
    pub(crate) display_name: String,
    pub(crate) row_bucket_count: usize,
    pub(crate) terminal_pin_count: usize,
    pub(crate) nodes: Vec<RamxOracleNode>,
    root_index: usize,
}

#[derive(Debug)]
struct ParsedOracleNode {
    node_id: String,
    parent_node_id: Option<String>,
    label: String,
    kind: OracleNodeKind,
    weight_bps: u32,
    row_weight_bps: u32,
    sort_order: i64,
    description: Option<String>,
}

impl OracleIndexTree {
    pub(crate) fn from_payload(payload: &Value) -> Result<Self, String> {
        if matches!(payload.get("ok").and_then(Value::as_bool), Some(false)) {
            let message = payload
                .get("error")
                .and_then(|error| text_at_keys(error, &["message", "code"]))
                .unwrap_or_else(|| "Backend returned an oracle index error.".to_string());
            return Err(message);
        }

        let data = object_at_key(payload, "data").unwrap_or(payload);
        let market_id = text_at_keys(data, &["marketId", "market_id", "id"])
            .ok_or_else(|| "Oracle index payload is missing marketId.".to_string())?;
        let symbol =
            text_at_keys(data, &["symbol"]).unwrap_or_else(|| market_id.to_ascii_uppercase());
        let display_name = text_at_keys(data, &["displayName", "display_name", "name", "label"])
            .unwrap_or_else(|| symbol.clone());
        let root_node_id = text_at_keys(data, &["rootNodeId", "root_node_id"]);
        let node_values = array_at_key(data, "nodes")
            .ok_or_else(|| "Oracle index payload is missing nodes.".to_string())?;

        let mut parsed_nodes = Vec::with_capacity(node_values.len());
        let mut id_to_index = HashMap::with_capacity(node_values.len());
        for (position, value) in node_values.iter().enumerate() {
            if !value.is_object() {
                return Err(format!("Oracle index node #{position} is not an object."));
            }
            let node_id = text_at_keys(value, &["nodeId", "node_id", "id"])
                .ok_or_else(|| format!("Oracle index node #{position} is missing nodeId."))?;
            if id_to_index.insert(node_id.clone(), position).is_some() {
                return Err(format!("Oracle index contains duplicate nodeId {node_id}."));
            }
            let kind_label = text_at_keys(value, &["kind", "nodeKind", "node_kind"])
                .ok_or_else(|| format!("Oracle index node {node_id} is missing kind."))?;
            let kind = OracleNodeKind::from_value(&kind_label).ok_or_else(|| {
                format!("Oracle index node {node_id} has unknown kind {kind_label}.")
            })?;
            let label = text_at_keys(value, &["label", "name", "displayName", "display_name"])
                .ok_or_else(|| format!("Oracle index node {node_id} is missing label."))?;
            let parent_node_id = text_at_keys(value, &["parentNodeId", "parent_node_id"])
                .filter(|parent| !parent.trim().is_empty());
            let weight_bps = if kind == OracleNodeKind::TerminalPin {
                0
            } else {
                bps_at_keys(
                    value,
                    &["weightBps", "weight_bps"],
                    &["weightPct", "weight_pct"],
                    0,
                )
            };
            let row_weight_bps = bps_at_keys(
                value,
                &["rowWeightBps", "row_weight_bps"],
                &["rowWeightPct", "row_weight_pct"],
                weight_bps,
            );
            let sort_order =
                i64_at_keys(value, &["sortOrder", "sort_order"]).unwrap_or(position as i64);

            parsed_nodes.push(ParsedOracleNode {
                node_id,
                parent_node_id,
                label,
                kind,
                weight_bps,
                row_weight_bps,
                sort_order,
                description: text_at_keys(value, &["description", "definition", "summary"]),
            });
        }

        let mut nodes = parsed_nodes
            .iter()
            .map(|parsed| RamxOracleNode {
                node_id: parsed.node_id.clone(),
                label: parsed.label.clone(),
                kind: parsed.kind,
                parent: None,
                weight_bps: parsed.weight_bps,
                row_weight_bps: parsed.row_weight_bps,
                weight_pct: f64::from(parsed.weight_bps) / 100.0,
                row_weight_pct: f64::from(parsed.row_weight_bps) / 100.0,
                pin_count: 0,
                children: Vec::new(),
                description: parsed.description.clone(),
            })
            .collect::<Vec<_>>();

        for (index, parsed) in parsed_nodes.iter().enumerate() {
            let Some(parent_node_id) = parsed.parent_node_id.as_ref() else {
                continue;
            };
            let parent_index = id_to_index.get(parent_node_id).copied().ok_or_else(|| {
                format!(
                    "Oracle index node {} references missing parent {}.",
                    parsed.node_id, parent_node_id
                )
            })?;
            nodes[index].parent = Some(parent_index);
            nodes[parent_index].children.push(index);
        }

        for node in nodes.iter_mut() {
            node.children.sort_by(|left, right| {
                parsed_nodes[*left]
                    .sort_order
                    .cmp(&parsed_nodes[*right].sort_order)
                    .then_with(|| {
                        nodes_label_cmp(&parsed_nodes[*left].label, &parsed_nodes[*right].label)
                    })
            });
        }

        if nodes.is_empty() {
            return Err("Oracle index has no nodes.".to_string());
        }

        let root_index = root_node_id
            .as_ref()
            .and_then(|node_id| id_to_index.get(node_id).copied())
            .or_else(|| nodes.iter().position(|node| node.parent.is_none()))
            .unwrap_or(DEFAULT_ORACLE_NODE_INDEX);

        let mut memo = vec![None; nodes.len()];
        for index in 0..nodes.len() {
            let mut visiting = HashSet::new();
            nodes[index].pin_count = descendant_pin_count(index, &nodes, &mut memo, &mut visiting)?;
        }

        let row_bucket_count = nodes
            .iter()
            .filter(|node| node.kind == OracleNodeKind::RowBucket)
            .count();
        let terminal_pin_count = nodes
            .iter()
            .filter(|node| node.kind == OracleNodeKind::TerminalPin)
            .count();

        Ok(Self {
            market_id,
            symbol,
            display_name,
            row_bucket_count,
            terminal_pin_count,
            nodes,
            root_index,
        })
    }

    pub(crate) fn root_index(&self) -> usize {
        self.root_index
    }

    pub(crate) fn node(&self, index: usize) -> Option<&RamxOracleNode> {
        self.nodes.get(index)
    }

    pub(crate) fn parent(&self, index: usize) -> Option<usize> {
        self.node(index).and_then(|node| node.parent)
    }

    pub(crate) fn child_indices(&self, parent_index: usize) -> Vec<usize> {
        self.node(parent_index)
            .map(|node| node.children.clone())
            .unwrap_or_default()
    }

    pub(crate) fn first_child(&self, parent_index: usize) -> Option<usize> {
        self.node(parent_index)
            .and_then(|node| node.children.first().copied())
    }

    pub(crate) fn sibling_indices(&self, index: usize) -> Vec<usize> {
        self.parent(index)
            .map(|parent| self.child_indices(parent))
            .unwrap_or_else(|| vec![self.root_index])
    }

    pub(crate) fn terminal_pin_indices(&self) -> Vec<usize> {
        self.nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| (node.kind == OracleNodeKind::TerminalPin).then_some(index))
            .collect()
    }

    pub(crate) fn row_bucket_indices(&self) -> Vec<usize> {
        self.nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| (node.kind == OracleNodeKind::RowBucket).then_some(index))
            .collect()
    }

    pub(crate) fn first_terminal_pin(&self, index: usize) -> Option<usize> {
        let node = self.node(index)?;
        if node.kind == OracleNodeKind::TerminalPin {
            return Some(index);
        }
        node.children
            .iter()
            .find_map(|child_index| self.first_terminal_pin(*child_index))
    }

    pub(crate) fn row_bucket_for(&self, index: usize) -> Option<usize> {
        let mut current = Some(index);
        while let Some(node_index) = current {
            let node = self.node(node_index)?;
            if node.kind == OracleNodeKind::RowBucket {
                return Some(node_index);
            }
            current = node.parent;
        }
        None
    }

    pub(crate) fn node_depth(&self, index: usize) -> usize {
        let mut depth = 0;
        let mut current = self.parent(index);
        while let Some(parent_index) = current {
            depth += 1;
            current = self.parent(parent_index);
        }
        depth
    }

    pub(crate) fn breadcrumb(&self, index: usize) -> Vec<&str> {
        let mut labels = Vec::new();
        let mut current = Some(index);
        while let Some(node_index) = current {
            let Some(node) = self.node(node_index) else {
                break;
            };
            labels.push(node.label.as_str());
            current = node.parent;
        }
        labels.reverse();
        labels
    }

    pub(crate) fn find_node_index(&self, selector: &str) -> Option<usize> {
        let selector = selector.trim();
        if selector.is_empty() {
            return None;
        }
        if let Ok(index) = selector.parse::<usize>() {
            if index < self.nodes.len() {
                return Some(index);
            }
        }
        let normalized = selector.to_ascii_lowercase();
        self.nodes
            .iter()
            .position(|node| {
                node.label.eq_ignore_ascii_case(selector)
                    || node.node_id.eq_ignore_ascii_case(selector)
            })
            .or_else(|| {
                self.nodes.iter().position(|node| {
                    node.label.to_ascii_lowercase().contains(&normalized)
                        || node.node_id.to_ascii_lowercase().contains(&normalized)
                })
            })
    }

    pub(crate) fn search_nodes(&self, query: &str) -> Vec<usize> {
        let query = query.trim().to_ascii_lowercase();
        if query.is_empty() {
            return Vec::new();
        }
        self.nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| {
                let path = self.breadcrumb(index).join(" / ").to_ascii_lowercase();
                let kind = node.kind.label().to_ascii_lowercase();
                let kind_aliases = match node.kind {
                    OracleNodeKind::Market => "market root root_recipe",
                    OracleNodeKind::Generation => "generation memory_generation",
                    OracleNodeKind::FormFactor => "form_factor form factor",
                    OracleNodeKind::RowBucket => "row_bucket row bucket",
                    OracleNodeKind::TerminalPin => "terminal_pin terminal pin source",
                };
                let description = node
                    .description
                    .as_deref()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                (node.label.to_ascii_lowercase().contains(&query)
                    || node.node_id.to_ascii_lowercase().contains(&query)
                    || path.contains(&query)
                    || kind.contains(&query)
                    || kind_aliases.contains(&query)
                    || description.contains(&query))
                .then_some(index)
            })
            .collect()
    }
}

fn descendant_pin_count(
    index: usize,
    nodes: &[RamxOracleNode],
    memo: &mut [Option<u16>],
    visiting: &mut HashSet<usize>,
) -> Result<u16, String> {
    if let Some(count) = memo[index] {
        return Ok(count);
    }
    if !visiting.insert(index) {
        return Err(format!(
            "Oracle index contains a cycle at node {}.",
            nodes[index].node_id
        ));
    }

    let count = if nodes[index].kind == OracleNodeKind::TerminalPin {
        1
    } else {
        let mut total = 0u16;
        for child_index in &nodes[index].children {
            total =
                total.saturating_add(descendant_pin_count(*child_index, nodes, memo, visiting)?);
        }
        total
    };
    visiting.remove(&index);
    memo[index] = Some(count);
    Ok(count)
}

fn object_at_key<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.get(key).filter(|child| child.is_object())
}

fn array_at_key<'a>(value: &'a Value, key: &str) -> Option<&'a Vec<Value>> {
    value.get(key).and_then(Value::as_array)
}

fn text_at_keys(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| text_from_value(value.get(*key)?))
}

fn text_from_value(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn i64_at_keys(value: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| i64_from_value(value.get(*key)?))
}

fn i64_from_value(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn bps_at_keys(value: &Value, bps_keys: &[&str], percent_keys: &[&str], default: u32) -> u32 {
    let bps = bps_keys
        .iter()
        .find_map(|key| number_from_value(value.get(*key)?))
        .map(|number| number.round() as i64)
        .or_else(|| {
            percent_keys
                .iter()
                .find_map(|key| number_from_value(value.get(*key)?))
                .map(|number| (number * 100.0).round() as i64)
        })
        .unwrap_or(default as i64);
    bps.clamp(0, 10_000) as u32
}

fn number_from_value(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().trim_end_matches('%').parse::<f64>().ok(),
        _ => None,
    }
}

fn nodes_label_cmp(left: &str, right: &str) -> std::cmp::Ordering {
    left.to_ascii_lowercase().cmp(&right.to_ascii_lowercase())
}
