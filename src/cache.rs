//! Bounded, process-local display caching. No transport, disk, credentials, or
//! transaction authority belongs here. Feature adapters choose identity and TTL.

use std::{
    borrow::Borrow,
    cell::Cell,
    collections::HashMap,
    hash::Hash,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Policy {
    pub capacity: usize,
    /// None is for content with its own revision/refresh lifecycle (help),
    /// never live market state.
    pub ttl: Option<Duration>,
}

struct Entry<V> {
    value: V,
    inserted: Instant,
    last_used: Cell<Instant>,
    ttl: Option<Duration>,
}

impl<V> Entry<V> {
    fn fresh_at(&self, now: Instant) -> bool {
        self.ttl
            .is_none_or(|ttl| now.saturating_duration_since(self.inserted) < ttl)
    }
}

/// Typed store shared by feature modules. Reads never renew freshness; only a
/// successful replacement does. Expired entries are misses, not stale authority.
pub(crate) struct DisplayCache<K, V> {
    policy: Policy,
    entries: HashMap<K, Entry<V>>,
    pending: Option<K>,
}

impl<K: Eq + Hash + Clone, V> DisplayCache<K, V> {
    pub fn new(policy: Policy) -> Self {
        Self {
            policy,
            entries: HashMap::new(),
            pending: None,
        }
    }

    pub fn get<Q: ?Sized + Eq + Hash>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
    {
        let entry = self.entries.get(key)?;
        let now = Instant::now();
        if !entry.fresh_at(now) {
            return None;
        }
        entry.last_used.set(now);
        Some(&entry.value)
    }

    pub fn contains_key<Q: ?Sized + Eq + Hash>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
    {
        self.get(key).is_some()
    }

    pub fn insert(&mut self, key: K, value: V) {
        self.insert_preserving(key, value, None);
    }

    /// Keep the visible help article resident while admitting hover previews.
    pub fn insert_preserving(&mut self, key: K, value: V, protected: Option<&K>) {
        let now = Instant::now();
        self.entries.retain(|_, entry| entry.fresh_at(now));
        if self.policy.capacity == 0 {
            return;
        }
        if !self.entries.contains_key(&key) && self.entries.len() >= self.policy.capacity {
            let oldest = self
                .entries
                .iter()
                .filter(|(candidate, _)| protected != Some(*candidate))
                .min_by_key(|(_, entry)| entry.last_used.get())
                .map(|(candidate, _)| candidate.clone());
            let Some(oldest) = oldest else {
                return;
            };
            self.entries.remove(&oldest);
        }
        self.entries.insert(
            key,
            Entry {
                value,
                inserted: now,
                last_used: Cell::new(now),
                ttl: self.policy.ttl,
            },
        );
    }

    pub fn remove<Q: ?Sized + Eq + Hash>(&mut self, key: &Q)
    where
        K: Borrow<Q>,
    {
        self.entries.remove(key);
    }

    /// Coalesce repeated reads of the active selection. Request generations in
    /// the adapter still decide whether a completion is allowed to write back.
    pub fn begin_fetch(&mut self, key: K, force: bool) -> bool {
        if !force && self.pending.as_ref() == Some(&key) {
            return false;
        }
        self.pending = Some(key);
        true
    }

    pub fn finish_fetch(&mut self, key: &K) {
        if self.pending.as_ref() == Some(key) {
            self.pending = None;
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.pending = None;
    }
}
