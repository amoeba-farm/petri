//! Components that can be mounted, changed and removed while petri runs,
//! leaving nothing behind: the model of Cordis ("A Programming Paradigm for
//! Spatiotemporal Composability", Shi, Zhang and Cui, arXiv 2608.25512),
//! ported small and synchronous. Petri's trading bots run on it
//! (`crate::bots`).
//!
//! * **Effects** (the paper's §3.1). Every change a component makes is paired
//!   with its inverse at the moment it is made ([`Ctx::effect`]). A fiber
//!   keeps its inverses in order; ending it runs them last first, so every
//!   state in between is recovered.
//! * **Coeffects** (§3.2). A component *injects* keys and *provides* keys.
//!   It runs while every key it injects is provided, and stops, its effects
//!   undone, when one goes away. Providing a key is itself an effect, so
//!   providers start before their consumers and consumers stop before their
//!   providers, without anyone ordering them.
//! * **Realms** (§3.2, isolation). A subtree can resolve key names to slots of
//!   its own: two bots that both provide `signal:fair` never meet, and
//!   neither can reach the other's.
//! * **Rows** ([`loader`], §5.2). A list of rows with stable ids, one fiber
//!   each, is reconciled against what runs: only changed rows restart, and a
//!   list that fails to build leaves the running one as it was.
//!
//! What the registry cannot undo is what left it: a sent transaction has no
//! inverse. A component that sends registers its own cleanup as an effect
//! (the bot's ladder registers its cancel).
//!
//! One owner calls the registry, and each call runs every transition it
//! causes to completion before returning: one transition at a time, the
//! paper's inertia, with no waiting between them. A component's `apply` is
//! synchronous; work over time is a subscription it registers, undone by
//! its inverse.

use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet},
};

pub mod loader;
#[cfg(test)]
mod tests;

pub type Key = String;
pub type FiberId = u64;
pub type Result<T> = std::result::Result<T, String>;
/// Undoes one effect.
pub type Inverse = Box<dyn FnOnce() + Send>;

/// The error of an effect on a fiber that is not starting or running.
pub const INACTIVE_EFFECT: &str = "INACTIVE_EFFECT";

/// A piece of a running program: what it needs, what it gives, and what it
/// does when everything it needs is there.
pub trait Component: Send + 'static {
    /// Keys that must all be provided for it to run.
    fn inject(&self) -> BTreeSet<Key> {
        BTreeSet::new()
    }
    /// Keys it may provide ([`Ctx::provide`] refuses any other).
    fn provide(&self) -> BTreeSet<Key> {
        BTreeSet::new()
    }
    /// Starts it. Every change it makes goes through `ctx`, with its inverse.
    fn apply(&mut self, ctx: &mut Ctx) -> Result<()>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Mounted, waiting for what it injects.
    Pending,
    Loading,
    Active,
    Unloading,
    /// Stopped: something it injects went away.
    Inactive,
    /// Its `apply` failed; everything it did was undone. It tries again when
    /// what it injects changes.
    Failed,
}

/// What a fiber undoes, last first.
enum Undo {
    Own(Inverse),
    /// The inverse of providing: the slot is removed and its consumers told.
    Provision(Slot),
}

/// A provided key: (realm, key).
type Slot = (String, Key);

struct Fiber {
    label: String,
    parent: Option<FiberId>,
    children: Vec<FiberId>,
    /// Out while its `apply` runs.
    component: Option<Box<dyn Component>>,
    state: State,
    inject: BTreeSet<Key>,
    provide: BTreeSet<Key>,
    /// The provider of each injected key when it started (the committed
    /// view): a provider replaced by another restarts it, even with an equal
    /// value.
    committed: BTreeMap<Key, FiberId>,
    undo: Vec<Undo>,
    /// Being disposed: it never starts again.
    retired: bool,
    /// Key patterns (`signal:*`, `*` or a whole key) and the realm they
    /// resolve in for this fiber and its subtree; the first match wins, and
    /// a key no fiber up the tree maps is in the default realm, "".
    realm: Vec<(String, String)>,
    error: Option<String>,
}

/// A fiber as callers see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub id: FiberId,
    pub label: String,
    pub state: State,
    pub error: Option<String>,
    /// Injected keys nobody provides yet.
    pub missing: Vec<Key>,
}

#[derive(Default)]
pub struct Registry {
    fibers: BTreeMap<FiberId, Fiber>,
    store: BTreeMap<Slot, (FiberId, Box<dyn Any + Send + Sync>)>,
    next: FiberId,
}

/// What a component's `apply` reaches the registry through.
pub struct Ctx<'a> {
    reg: &'a mut Registry,
    fiber: FiberId,
}

pub(crate) fn matches(pattern: &str, key: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => key.starts_with(prefix),
        None => pattern == key,
    }
}

impl Ctx<'_> {
    /// Runs `effect` and keeps the inverse it returns (Algorithm 1).
    pub fn effect(&mut self, effect: impl FnOnce() -> Result<Inverse>) -> Result<()> {
        self.reg.effect(self.fiber, effect)
    }

    /// Provides `key` in this fiber's realm (Algorithm 2): `key` must be one
    /// it declared and not already provided there. Its inverse removes it.
    pub fn provide<T: Any + Send + Sync>(&mut self, key: &str, value: T) -> Result<()> {
        let fiber = &self.reg.fibers[&self.fiber];
        if !matches!(fiber.state, State::Loading | State::Active) {
            return Err(INACTIVE_EFFECT.into());
        }
        if !fiber.provide.contains(key) {
            return Err(format!("{} does not provide {key}", fiber.label));
        }
        let slot = (self.reg.realm_of(self.fiber, key), key.to_string());
        if self.reg.store.contains_key(&slot) {
            return Err(format!("{key} is already provided"));
        }
        self.reg
            .store
            .insert(slot.clone(), (self.fiber, Box::new(value)));
        self.reg
            .fibers
            .get_mut(&self.fiber)
            .expect("the fiber starting")
            .undo
            .push(Undo::Provision(slot.clone()));
        self.reg.notify(&slot);
        Ok(())
    }

    /// The value of `key` as this fiber resolves it.
    pub fn get<T: Any + Clone>(&self, key: &str) -> Option<T> {
        self.reg.get(self.fiber, key)
    }

    /// As [`Ctx::get`], for a key it injects (so there whenever it runs).
    pub fn need<T: Any + Clone>(&self, key: &str) -> Result<T> {
        self.get(key)
            .ok_or_else(|| format!("{key} is not there, or not what it should be"))
    }
}

impl Registry {
    /// Mounts `component` under `parent` (none: at the top) with `realm`'s
    /// key patterns, and starts it if what it injects is there.
    pub fn mount(
        &mut self,
        parent: Option<FiberId>,
        label: &str,
        component: Box<dyn Component>,
        realm: Vec<(String, String)>,
    ) -> FiberId {
        self.next += 1;
        let id = self.next;
        self.fibers.insert(
            id,
            Fiber {
                label: label.to_string(),
                parent,
                children: Vec::new(),
                inject: component.inject(),
                provide: component.provide(),
                component: Some(component),
                state: State::Pending,
                committed: BTreeMap::new(),
                undo: Vec::new(),
                retired: false,
                realm,
                error: None,
            },
        );
        if let Some(p) = parent.and_then(|p| self.fibers.get_mut(&p)) {
            p.children.push(id);
        }
        self.refresh(id);
        id
    }

    /// Ends `id` and its subtree, children first, undoing everything they
    /// did, and forgets them (retire, then remove).
    pub fn dispose(&mut self, id: FiberId) {
        if !self.fibers.contains_key(&id) {
            return;
        }
        let tree = self.tree(id);
        for f in &tree {
            if let Some(f) = self.fibers.get_mut(f) {
                f.retired = true;
            }
        }
        self.stop(id);
        if let Some(p) = self.fibers[&id].parent
            && let Some(p) = self.fibers.get_mut(&p)
        {
            p.children.retain(|c| *c != id);
        }
        for f in tree {
            self.fibers.remove(&f);
        }
    }

    /// Replaces `id`'s component: stops it (and so whatever relied on what
    /// it provided), then starts the new one.
    pub fn update(&mut self, id: FiberId, component: Box<dyn Component>) {
        if !self.fibers.contains_key(&id) {
            return;
        }
        self.stop(id);
        let f = self.fibers.get_mut(&id).expect("the fiber updated");
        f.inject = component.inject();
        f.provide = component.provide();
        f.component = Some(component);
        f.state = State::Pending;
        f.error = None;
        self.refresh(id);
    }

    pub fn status(&self, id: FiberId) -> Option<Status> {
        let f = self.fibers.get(&id)?;
        Some(Status {
            id,
            label: f.label.clone(),
            state: f.state,
            error: f.error.clone(),
            missing: f
                .inject
                .iter()
                .filter(|k| {
                    !self
                        .store
                        .contains_key(&(self.realm_of(id, k), (*k).clone()))
                })
                .cloned()
                .collect(),
        })
    }

    /// `id`'s children, in the order they were mounted.
    pub fn children(&self, id: FiberId) -> Vec<FiberId> {
        self.fibers
            .get(&id)
            .map(|f| f.children.clone())
            .unwrap_or_default()
    }

    /// The value of `key` as fiber `from` resolves it.
    pub fn get<T: Any + Clone>(&self, from: FiberId, key: &str) -> Option<T> {
        let slot = (self.realm_of(from, key), key.to_string());
        self.store.get(&slot)?.1.downcast_ref::<T>().cloned()
    }

    /// How many keys are provided, in every realm.
    #[cfg(test)]
    pub fn provided(&self) -> usize {
        self.store.len()
    }

    /// Runs `effect` for fiber `id` and keeps its inverse; refused unless
    /// the fiber is starting or running.
    fn effect(&mut self, id: FiberId, effect: impl FnOnce() -> Result<Inverse>) -> Result<()> {
        let running = self
            .fibers
            .get(&id)
            .is_some_and(|f| matches!(f.state, State::Loading | State::Active));
        if !running {
            return Err(INACTIVE_EFFECT.into());
        }
        let inverse = effect()?;
        self.fibers
            .get_mut(&id)
            .expect("the running fiber")
            .undo
            .push(Undo::Own(inverse));
        Ok(())
    }

    /// `id` and every fiber under it.
    fn tree(&self, id: FiberId) -> Vec<FiberId> {
        let mut all = vec![id];
        let mut at = 0;
        while at < all.len() {
            if let Some(f) = self.fibers.get(&all[at]) {
                all.extend(f.children.iter().copied());
            }
            at += 1;
        }
        all
    }

    /// The realm `key` resolves in for fiber `id` (Def. 24): the first
    /// pattern that matches it, from `id` up the tree.
    fn realm_of(&self, id: FiberId, key: &str) -> String {
        let mut at = Some(id);
        while let Some(f) = at.and_then(|i| self.fibers.get(&i)) {
            if let Some((_, realm)) = f.realm.iter().find(|(p, _)| matches(p, key)) {
                return realm.clone();
            }
            at = f.parent;
        }
        String::new()
    }

    /// The providers of everything `id` injects, or `None` when it must not
    /// run: retired, its parent not running, or a key missing.
    fn target(&self, id: FiberId) -> Option<BTreeMap<Key, FiberId>> {
        let f = self.fibers.get(&id)?;
        if f.retired {
            return None;
        }
        if let Some(p) = f.parent
            && self.fibers.get(&p).map(|p| p.state) != Some(State::Active)
        {
            return None;
        }
        f.inject
            .iter()
            .map(|k| {
                let slot = (self.realm_of(id, k), k.clone());
                self.store.get(&slot).map(|(p, _)| (k.clone(), *p))
            })
            .collect()
    }

    /// Tells every fiber that injects `slot` (in its realm) that it changed
    /// (Algorithm 3).
    fn notify(&mut self, slot: &Slot) {
        let consumers: Vec<FiberId> = self
            .fibers
            .iter()
            .filter(|(id, f)| f.inject.contains(&slot.1) && self.realm_of(**id, &slot.1) == slot.0)
            .map(|(id, _)| *id)
            .collect();
        for id in consumers {
            self.refresh(id);
        }
    }

    /// Starts, stops or restarts `id` as its target view says (Algorithm 6):
    /// a change of what it injects is activating, deactivating or neutral
    /// (Def. 22). A fiber mid-transition is left alone.
    fn refresh(&mut self, id: FiberId) {
        let Some(f) = self.fibers.get(&id) else {
            return;
        };
        let (state, committed) = (f.state, f.committed.clone());
        match (state, self.target(id)) {
            (State::Active, Some(t)) if t == committed => {}
            (State::Active, Some(_)) => {
                self.stop(id);
                self.refresh(id);
            }
            (State::Active, None) => self.stop(id),
            (State::Pending | State::Inactive | State::Failed, Some(t)) => self.reload(id, t),
            _ => {}
        }
    }

    fn reload(&mut self, id: FiberId, view: BTreeMap<Key, FiberId>) {
        let f = self.fibers.get_mut(&id).expect("the fiber starting");
        f.state = State::Loading;
        f.committed = view;
        f.error = None;
        let mut component = f.component.take().expect("one transition at a time");
        let result = component.apply(&mut Ctx {
            reg: self,
            fiber: id,
        });
        self.fibers
            .get_mut(&id)
            .expect("the fiber starting")
            .component = Some(component);
        match result {
            Ok(()) => {
                self.fibers.get_mut(&id).expect("the fiber").state = State::Active;
                for child in self.children(id) {
                    self.refresh(child);
                }
            }
            Err(error) => {
                // L-Raise: undo what it did, then say why.
                self.recover(id);
                let f = self.fibers.get_mut(&id).expect("the fiber");
                f.state = State::Failed;
                f.committed.clear();
                f.error = Some(error);
            }
        }
    }

    /// Stops `id`: its subtree first (the last mounted first), then its own
    /// effects, last first.
    fn stop(&mut self, id: FiberId) {
        for child in self.children(id).into_iter().rev() {
            self.stop(child);
        }
        let Some(f) = self.fibers.get_mut(&id) else {
            return;
        };
        if f.state != State::Active {
            return;
        }
        f.state = State::Unloading;
        self.recover(id);
        let f = self.fibers.get_mut(&id).expect("the fiber stopping");
        f.committed.clear();
        f.state = State::Inactive;
    }

    /// Runs `id`'s inverses, last first (`recover`, Def. 6). Undoing a
    /// provision stops its consumers before the next inverse runs.
    fn recover(&mut self, id: FiberId) {
        while let Some(undo) = self.fibers.get_mut(&id).and_then(|f| f.undo.pop()) {
            match undo {
                Undo::Own(inverse) => inverse(),
                Undo::Provision(slot) => {
                    self.store.remove(&slot);
                    self.notify(&slot);
                }
            }
        }
    }
}
