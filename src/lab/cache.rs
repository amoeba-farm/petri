//! TUI cache policy and invalidation adapter for market details, charts,
//! settlement evidence and help. Last-view state in other features is separate.
//! Wallet reads, executable quotes, masks, plans and approvals bypass this engine.

use super::*;
use crate::cache::{DisplayCache, Policy};
use std::cell::{Ref, RefCell};

pub(super) struct GitbookRenderCache {
    pub(super) page_id: String,
    pub(super) page_revision: u64,
    pub(super) page_count: usize,
    pub(super) width: usize,
    pub(super) no_color: bool,
    pub(super) motion_tick: usize,
    pub(super) lines: Vec<Line<'static>>,
}

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

/// One app-owned entry point for reusable display data. Typed stores prevent
/// cross-feature key/value mixing; no global state or transaction authority.
pub(super) struct TuiCache {
    scope: ReadScope,
    details: DisplayCache<String, DishDetail>,
    charts: DisplayCache<String, chart::EmbeddedChart>,
    settlements: DisplayCache<(String, String), settlement_data::SettlementBundle>,
    help_pages: DisplayCache<String, GitbookPage>,
    help_revision: u64,
    help_render: RefCell<Option<GitbookRenderCache>>,
}

impl TuiCache {
    pub(super) fn new(
        backend: &str,
        network: &str,
        first_page: Option<(String, GitbookPage)>,
    ) -> Self {
        let mut help_pages = DisplayCache::new(HELP_PAGES);
        if let Some((id, page)) = first_page {
            help_pages.insert(id, page);
        }
        Self {
            scope: ReadScope::new(backend, network),
            details: DisplayCache::new(MARKET_DETAILS),
            charts: DisplayCache::new(CHARTS),
            settlements: DisplayCache::new(SETTLEMENTS),
            help_pages,
            help_revision: 1,
            help_render: RefCell::new(None),
        }
    }

    pub(super) fn details(&self) -> &DisplayCache<String, DishDetail> {
        &self.details
    }

    pub(super) fn details_mut(&mut self) -> &mut DisplayCache<String, DishDetail> {
        &mut self.details
    }

    pub(super) fn charts(&self) -> &DisplayCache<String, chart::EmbeddedChart> {
        &self.charts
    }

    pub(super) fn charts_mut(&mut self) -> &mut DisplayCache<String, chart::EmbeddedChart> {
        &mut self.charts
    }

    pub(super) fn settlements(
        &self,
    ) -> &DisplayCache<(String, String), settlement_data::SettlementBundle> {
        &self.settlements
    }

    pub(super) fn settlements_mut(
        &mut self,
    ) -> &mut DisplayCache<(String, String), settlement_data::SettlementBundle> {
        &mut self.settlements
    }

    pub(super) fn help_pages(&self) -> &DisplayCache<String, GitbookPage> {
        &self.help_pages
    }

    pub(super) fn store_help_page(&mut self, id: String, page: GitbookPage, protected: &String) {
        self.help_pages.insert_preserving(id, page, Some(protected));
        self.help_revision = self.help_revision.wrapping_add(1);
        self.help_render.get_mut().take();
    }

    pub(super) fn help_revision(&self) -> u64 {
        self.help_revision
    }

    pub(super) fn help_render(&self) -> Ref<'_, Option<GitbookRenderCache>> {
        self.help_render.borrow()
    }

    pub(super) fn store_help_render(&self, rendered: GitbookRenderCache) {
        *self.help_render.borrow_mut() = Some(rendered);
    }

    fn switch_scope(&mut self, backend: &str, network: &str) -> bool {
        let scope = ReadScope::new(backend, network);
        if self.scope == scope {
            return false;
        }
        self.invalidate_market();
        self.scope = scope;
        true
    }

    fn invalidate_market(&mut self) {
        self.details.clear();
        self.charts.clear();
        self.settlements.clear();
    }
}

impl LabApp {
    pub(super) fn ensure_read_cache_scope(&mut self, backend: &str) {
        if self
            .cache
            .switch_scope(backend, &self.onchain_config.network)
        {
            self.fence_market_reads();
            self.trading.detail = None;
            self.trading.chart = None;
            self.trading.settlement_bundle = None;
        }
    }

    /// Fence already-running reads as well as stored values after a mutation or
    /// scope switch. An older response must not repopulate invalidated caches.
    pub(super) fn invalidate_market_caches(&mut self) {
        self.cache.invalidate_market();
        self.fence_market_reads();
    }

    fn fence_market_reads(&mut self) {
        self.trading.detail_request = self.trading.detail_request.wrapping_add(1);
        self.trading.chart_request = self.trading.chart_request.wrapping_add(1);
        self.trading.settlement_request = self.trading.settlement_request.wrapping_add(1);
        self.trading.loading_detail = false;
        self.trading.loading_chart = false;
        self.trading.loading_settlement = false;
    }
}
