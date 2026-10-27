//! Rows: a list of components with stable ids, one fiber each, kept in step
//! with a changing list (the paper's loader, §5.2.1). Reconciling is a keyed
//! diff: a new row mounts, a changed one restarts, a gone one is disposed.
//! Every changed row is built before anything running is touched, so a list
//! with a bad row changes nothing (§5.2.2: a failed reload leaves the last
//! good tree running).

use std::collections::{BTreeMap, HashSet};

use super::{Component, FiberId, Registry, Result};

/// One row of a list: `kind` says what to build, `config` its settings (each
/// value as written).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub kind: String,
    pub disabled: bool,
    pub config: BTreeMap<String, String>,
}

/// The rows mounted under one fiber.
#[derive(Default)]
pub struct Loader {
    mounted: Vec<(String, FiberId, Row)>,
}

impl Loader {
    /// Brings the rows under `root` in line with `rows`, building each new
    /// or changed one with `build`.
    pub fn reconcile(
        &mut self,
        reg: &mut Registry,
        root: FiberId,
        rows: &[Row],
        build: &dyn Fn(&Row) -> Result<Box<dyn Component>>,
    ) -> Result<()> {
        let mut seen = HashSet::new();
        let want: Vec<&Row> = rows.iter().filter(|r| !r.disabled).collect();
        for row in &want {
            if !seen.insert(row.id.as_str()) {
                return Err(format!("two rows are called {}", row.id));
            }
        }
        let mut built = BTreeMap::new();
        for row in &want {
            let same = self
                .mounted
                .iter()
                .any(|(id, _, old)| *id == row.id && old == *row);
            if !same {
                let component = build(row).map_err(|e| format!("{}: {e}", row.id))?;
                built.insert(row.id.clone(), component);
            }
        }
        let (keep, gone): (Vec<_>, Vec<_>) = std::mem::take(&mut self.mounted)
            .into_iter()
            .partition(|(id, _, _)| seen.contains(id.as_str()));
        for (_, fiber, _) in gone {
            reg.dispose(fiber);
        }
        let mut was: BTreeMap<String, FiberId> =
            keep.into_iter().map(|(id, fiber, _)| (id, fiber)).collect();
        for row in want {
            let fiber = match (was.remove(&row.id), built.remove(&row.id)) {
                (Some(fiber), None) => fiber,
                (Some(fiber), Some(component)) => {
                    reg.update(fiber, component);
                    fiber
                }
                (None, Some(component)) => reg.mount(Some(root), &row.id, component, Vec::new()),
                (None, None) => unreachable!("every new row was built"),
            };
            self.mounted.push((row.id.clone(), fiber, row.clone()));
        }
        Ok(())
    }

    /// Each row's id and fiber, in the list's order.
    pub fn rows(&self) -> impl Iterator<Item = (&str, FiberId)> {
        self.mounted
            .iter()
            .map(|(id, fiber, _)| (id.as_str(), *fiber))
    }
}
