//! Help navigation state and local transitions, independent of LabApp and network effects.
use super::{GitbookGlossaryHover, GitbookHelpPreview, HelpPane, HelpPreviewOrigin};
use crate::gitbook::{self, GitbookIndex, GitbookNavTarget};
use std::time::Instant;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum HelpPreviewTargetIdentity {
    Category(String),
    Page(String),
}

pub(super) struct HelpState {
    pub(super) index: GitbookIndex,
    pub(super) selected_page_id: String,
    pub(super) selected_nav: usize,
    pub(super) expanded_categories: Vec<bool>,
    pub(super) pane: HelpPane,
    pub(super) nav_scroll: usize,
    pub(super) article_scroll: usize,
    pub(super) preview: Option<GitbookHelpPreview>,
    pub(super) glossary_hover: Option<GitbookGlossaryHover>,
    pub(super) hover_grace_ticks: Option<usize>,
    pub(super) index_request: u64,
    pub(super) page_request: u64,
    pub(super) page_request_id: Option<String>,
    pub(super) preview_page_request: u64,
    pub(super) preview_page_request_id: Option<String>,
    pub(super) loading_index: bool,
    pub(super) loading_page: bool,
    pub(super) loading_preview_page: bool,
    pub(super) preview_failed_page_id: Option<String>,
    pub(super) issue: Option<String>,
    pub(super) transition_tick: Option<usize>,
    pub(super) last_checked_at: Option<Instant>,
}

impl HelpState {
    pub(super) fn new(index: GitbookIndex) -> Self {
        let selected_page_id = index
            .first_page()
            .map(|page| page.id.clone())
            .unwrap_or_default();
        let expanded_categories = vec![true; index.categories.len()];
        Self {
            index,
            selected_page_id,
            selected_nav: 1,
            expanded_categories,
            pane: HelpPane::Navigation,
            nav_scroll: 0,
            article_scroll: 0,
            preview: None,
            glossary_hover: None,
            hover_grace_ticks: None,
            index_request: 0,
            page_request: 0,
            page_request_id: None,
            preview_page_request: 0,
            preview_page_request_id: None,
            loading_index: false,
            loading_page: false,
            loading_preview_page: false,
            preview_failed_page_id: None,
            issue: None,
            transition_tick: None,
            last_checked_at: None,
        }
    }

    pub(super) fn link_for_nav(&self, nav_index: usize) -> Option<&gitbook::GitbookPageLink> {
        let rows = gitbook::nav_rows(&self.index, &self.expanded_categories);
        let row = rows.get(nav_index)?;
        let GitbookNavTarget::Page { category, page } = row.target else {
            return None;
        };
        self.index
            .categories
            .get(category)
            .and_then(|category| category.pages.get(page))
    }

    pub(super) fn current_link(&self) -> Option<&gitbook::GitbookPageLink> {
        self.index.page_by_id(&self.selected_page_id)
    }

    pub(super) fn preview_target_identity(
        &self,
        nav_index: usize,
    ) -> Option<HelpPreviewTargetIdentity> {
        let rows = gitbook::nav_rows(&self.index, &self.expanded_categories);
        let row = rows.get(nav_index)?;
        match row.target {
            GitbookNavTarget::Category(category) => {
                self.index.categories.get(category).map(|category| {
                    HelpPreviewTargetIdentity::Category(category.title.to_ascii_lowercase())
                })
            }
            GitbookNavTarget::Page { category, page } => self
                .index
                .categories
                .get(category)
                .and_then(|category| category.pages.get(page))
                .map(|page| HelpPreviewTargetIdentity::Page(page.id.clone())),
        }
    }

    pub(super) fn nav_index_for_target_identity(
        &self,
        target: &HelpPreviewTargetIdentity,
    ) -> Option<usize> {
        gitbook::nav_rows(&self.index, &self.expanded_categories)
            .iter()
            .position(|row| match (target, row.target) {
                (
                    HelpPreviewTargetIdentity::Category(expected),
                    GitbookNavTarget::Category(index),
                ) => self
                    .index
                    .categories
                    .get(index)
                    .is_some_and(|category| category.title.to_ascii_lowercase() == *expected),
                (
                    HelpPreviewTargetIdentity::Page(expected),
                    GitbookNavTarget::Page { category, page },
                ) => self
                    .index
                    .categories
                    .get(category)
                    .and_then(|category| category.pages.get(page))
                    .is_some_and(|candidate| candidate.id == *expected),
                _ => false,
            })
    }

    pub(super) fn clear_hover_preview(&mut self) {
        self.hover_grace_ticks = None;
        if self
            .preview
            .as_ref()
            .is_some_and(|preview| preview.origin == HelpPreviewOrigin::Hover)
        {
            self.preview = None;
        }
    }

    pub(super) fn scroll_preview(&mut self, direction: isize, step: usize, max_scroll: usize) {
        let Some(preview) = self.preview.as_mut() else {
            return;
        };
        let current = preview.scroll.min(max_scroll);
        if direction < 0 {
            preview.scroll = current.saturating_sub(step);
        } else if direction > 0 {
            preview.scroll = current.saturating_add(step).min(max_scroll);
        }
    }

    pub(super) fn sync_nav_selection(&mut self) {
        let rows = gitbook::nav_rows(&self.index, &self.expanded_categories);
        if let Some(index) = rows.iter().position(|row| match row.target {
            GitbookNavTarget::Page { category, page } => self
                .index
                .categories
                .get(category)
                .and_then(|category| category.pages.get(page))
                .is_some_and(|candidate| candidate.id == self.selected_page_id),
            GitbookNavTarget::Category(_) => false,
        }) {
            self.selected_nav = index;
        } else if !rows.is_empty() {
            self.selected_nav = self.selected_nav.min(rows.len() - 1);
        } else {
            self.selected_nav = 0;
        }
        self.nav_scroll = self.selected_nav.saturating_sub(3);
    }

    pub(super) fn scroll_article(&mut self, direction: isize, step: usize) {
        self.glossary_hover = None;
        self.hover_grace_ticks = None;
        if direction < 0 {
            self.article_scroll = self.article_scroll.saturating_sub(step);
        } else if direction > 0 {
            self.article_scroll = self.article_scroll.saturating_add(step);
        }
    }
}
