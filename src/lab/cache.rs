//! TUI cache policy and invalidation adapter for market details, charts,
//! settlement evidence and help. Last-view state in other features is separate.
//! Wallet reads, executable quotes, masks, plans and approvals bypass this engine.

use super::*;
use crate::cache::Policy;

pub(super) const MARKET_DETAILS: Policy = Policy {
    capacity: 32,
    ttl: Some(Duration::from_secs(10)),
};
pub(super) const CHARTS: Policy = Policy {
    capacity: 32,
    ttl: Some(Duration::from_secs(30)),
};
pub(super) const SETTLEMENTS: Policy = Policy {
    capacity: 24,
    ttl: Some(Duration::from_secs(15)),
};
pub(super) const HELP_PAGES: Policy = Policy {
    capacity: HELP_PAGE_CACHE_LIMIT,
    ttl: None,
};

/// Cache policy stays in DisplayCache and each feature retains its reducer.
/// Only a newly admitted fetch advances the generation; both outcomes load.
pub(super) fn begin_cached_read(started: bool, generation: &mut u64, loading: &mut bool) -> bool {
    *loading = true;
    if started {
        *generation = generation.wrapping_add(1);
    }
    started
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ReadScope {
    backend: String,
    network: String,
    release: &'static str,
}

impl ReadScope {
    pub(super) fn new(backend: &str, network: &str) -> Self {
        Self {
            backend: backend.trim().trim_end_matches('/').to_owned(),
            network: network.to_owned(),
            release: crate::current_release::SDK_PACKAGE_COMMIT,
        }
    }
}

impl LabApp {
    pub(super) fn ensure_read_cache_scope(&mut self, backend: &str) {
        let scope = ReadScope::new(backend, &self.onchain_config.network);
        if self.read_cache_scope != scope {
            self.invalidate_market_caches();
            self.read_cache_scope = scope;
            self.detail = None;
            self.chart = None;
            self.settlement_bundle = None;
        }
    }

    /// Fence already-running reads as well as stored values after a mutation or
    /// scope switch. An older response must not repopulate invalidated caches.
    pub(super) fn invalidate_market_caches(&mut self) {
        self.detail_cache.clear();
        self.chart_cache.clear();
        self.settlement_cache.clear();
        self.detail_request = self.detail_request.wrapping_add(1);
        self.chart_request = self.chart_request.wrapping_add(1);
        self.settlement_request = self.settlement_request.wrapping_add(1);
        self.loading_detail = false;
        self.loading_chart = false;
        self.loading_settlement = false;
    }
}
