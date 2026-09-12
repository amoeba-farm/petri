//! GitBook Help cache, preview, hover, selection, and keyboard state transitions.

use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
enum HelpPreviewTargetIdentity {
    Category(String),
    Page(String),
}

impl LabApp {
    pub(super) fn help_link_for_nav(&self, nav_index: usize) -> Option<&gitbook::GitbookPageLink> {
        let rows = gitbook::nav_rows(&self.help_index, &self.help_expanded_categories);
        let row = rows.get(nav_index)?;
        let GitbookNavTarget::Page { category, page } = row.target else {
            return None;
        };
        self.help_index
            .categories
            .get(category)
            .and_then(|category| category.pages.get(page))
    }

    pub(super) fn cache_help_page(&mut self, page_id: String, page: GitbookPage) {
        let updates_visible_preview = self
            .help_preview
            .as_ref()
            .and_then(|preview| self.help_link_for_nav(preview.nav_index))
            .is_some_and(|link| link.id == page_id);
        if updates_visible_preview && let Some(preview) = self.help_preview.as_mut() {
            preview.page = Some(page.clone());
        }
        self.help_pages
            .insert_preserving(page_id, page, Some(&self.help_selected_page_id));
        self.help_page_revision = self.help_page_revision.wrapping_add(1);
        self.help_render_cache.get_mut().take();
    }

    pub(super) fn prepare_gitbook_help(&mut self) {
        self.help_preview = None;
        self.help_preview_failed_page_id = None;
        self.help_glossary_hover = None;
        self.help_hover_grace_ticks = None;
        if self.help_index.categories.is_empty() {
            self.help_index = gitbook::bundled_index();
            self.help_expanded_categories = vec![true; self.help_index.categories.len()];
        }
        if self.help_expanded_categories.len() != self.help_index.categories.len() {
            self.help_expanded_categories = vec![true; self.help_index.categories.len()];
        }
        if self
            .help_index
            .page_by_id(&self.help_selected_page_id)
            .is_none()
        {
            self.help_selected_page_id = self
                .help_index
                .first_page()
                .map(|page| page.id.clone())
                .unwrap_or_default();
        }
        if !self.help_selected_page_id.is_empty()
            && !self.help_pages.contains_key(&self.help_selected_page_id)
            && let Some(page) = self
                .help_index
                .page_by_id(&self.help_selected_page_id)
                .and_then(gitbook::bundled_page)
        {
            self.cache_help_page(self.help_selected_page_id.clone(), page);
        }
        self.sync_help_nav_selection();
        self.help_pane = HelpPane::Navigation;
        self.help_article_scroll = 0;
    }

    pub(super) fn current_help_link(&self) -> Option<&gitbook::GitbookPageLink> {
        self.help_index.page_by_id(&self.help_selected_page_id)
    }

    pub(super) fn current_help_page(&self) -> Option<&GitbookPage> {
        self.help_pages.get(&self.help_selected_page_id)
    }

    pub(super) fn help_preview_for_nav(
        &self,
        nav_index: usize,
        origin: HelpPreviewOrigin,
    ) -> Option<GitbookHelpPreview> {
        let rows = gitbook::nav_rows(&self.help_index, &self.help_expanded_categories);
        let row = rows.get(nav_index)?;
        let page = match row.target {
            GitbookNavTarget::Category(_) => None,
            GitbookNavTarget::Page { category, page } => self
                .help_index
                .categories
                .get(category)
                .and_then(|category| category.pages.get(page))
                .and_then(|link| {
                    self.help_pages
                        .get(&link.id)
                        .cloned()
                        .or_else(|| gitbook::bundled_page(link))
                }),
        };
        let reveal_tick = if origin == HelpPreviewOrigin::Keyboard || gitbook_reduced_motion() {
            self.spinner_tick
        } else {
            self.spinner_tick.saturating_add(HELP_PREVIEW_DWELL_TICKS)
        };
        Some(GitbookHelpPreview {
            nav_index,
            origin,
            scroll: 0,
            reveal_tick,
            page,
        })
    }

    fn help_preview_target_identity(&self, nav_index: usize) -> Option<HelpPreviewTargetIdentity> {
        let rows = gitbook::nav_rows(&self.help_index, &self.help_expanded_categories);
        let row = rows.get(nav_index)?;
        match row.target {
            GitbookNavTarget::Category(category) => {
                self.help_index.categories.get(category).map(|category| {
                    HelpPreviewTargetIdentity::Category(category.title.to_ascii_lowercase())
                })
            }
            GitbookNavTarget::Page { category, page } => self
                .help_index
                .categories
                .get(category)
                .and_then(|category| category.pages.get(page))
                .map(|page| HelpPreviewTargetIdentity::Page(page.id.clone())),
        }
    }

    fn help_nav_index_for_target_identity(
        &self,
        target: &HelpPreviewTargetIdentity,
    ) -> Option<usize> {
        gitbook::nav_rows(&self.help_index, &self.help_expanded_categories)
            .iter()
            .position(|row| match (target, row.target) {
                (
                    HelpPreviewTargetIdentity::Category(expected),
                    GitbookNavTarget::Category(index),
                ) => self
                    .help_index
                    .categories
                    .get(index)
                    .is_some_and(|category| category.title.to_ascii_lowercase() == *expected),
                (
                    HelpPreviewTargetIdentity::Page(expected),
                    GitbookNavTarget::Page { category, page },
                ) => self
                    .help_index
                    .categories
                    .get(category)
                    .and_then(|category| category.pages.get(page))
                    .is_some_and(|candidate| candidate.id == *expected),
                _ => false,
            })
    }

    pub(super) fn set_help_hover_preview(&mut self, nav_index: usize) {
        if self
            .help_preview
            .as_ref()
            .is_some_and(|preview| preview.origin == HelpPreviewOrigin::Keyboard)
        {
            return;
        }
        self.help_glossary_hover = None;
        self.help_hover_grace_ticks = None;
        self.help_selected_nav = nav_index;
        if self.help_preview.as_ref().is_some_and(|preview| {
            preview.origin == HelpPreviewOrigin::Hover && preview.nav_index == nav_index
        }) {
            return;
        }
        self.help_preview_failed_page_id = None;
        self.help_preview = self.help_preview_for_nav(nav_index, HelpPreviewOrigin::Hover);
    }

    pub(super) fn clear_help_hover_preview(&mut self) {
        self.help_hover_grace_ticks = None;
        if self
            .help_preview
            .as_ref()
            .is_some_and(|preview| preview.origin == HelpPreviewOrigin::Hover)
        {
            self.help_preview = None;
        }
    }

    pub(super) fn toggle_keyboard_help_preview(&mut self) {
        self.help_glossary_hover = None;
        self.help_hover_grace_ticks = None;
        if self.help_pane != HelpPane::Navigation {
            self.help_pane = HelpPane::Navigation;
        }
        if self.help_preview.as_ref().is_some_and(|preview| {
            preview.origin == HelpPreviewOrigin::Keyboard
                && preview.nav_index == self.help_selected_nav
        }) {
            self.help_preview = None;
            self.status = "Topic preview closed.".to_string();
            return;
        }
        self.help_preview_failed_page_id = None;
        self.help_preview =
            self.help_preview_for_nav(self.help_selected_nav, HelpPreviewOrigin::Keyboard);
        if self.help_preview.is_some() {
            self.status =
                "Topic preview open. Use arrows to browse, PgUp/PgDn to scroll, Enter to open."
                    .to_string();
        }
    }

    pub(super) fn scroll_help_preview(&mut self, direction: isize, step: usize, max_scroll: usize) {
        let Some(preview) = self.help_preview.as_mut() else {
            return;
        };
        let current = preview.scroll.min(max_scroll);
        if direction < 0 {
            preview.scroll = current.saturating_sub(step);
        } else if direction > 0 {
            preview.scroll = current.saturating_add(step).min(max_scroll);
        }
    }

    pub(super) fn request_help_index(&mut self, fetch_tx: &Sender<LabFetchResult>, force: bool) {
        if self.loading_help_index {
            if force {
                self.status = "The published GitBook refresh is already running.".to_string();
            }
            return;
        }
        self.help_index_request = self.help_index_request.wrapping_add(1);
        self.loading_help_index = true;
        self.help_issue = None;
        self.status = if force {
            "Refreshing the published GitBook...".to_string()
        } else {
            "Checking the published GitBook...".to_string()
        };
        let root = docs_url();
        spawn_help_index_fetch(root, fetch_tx.clone(), self.help_index_request);
    }

    pub(super) fn refresh_gitbook_help_if_due(&mut self, fetch_tx: &Sender<LabFetchResult>) {
        if self.screen != LabScreen::Help
            || self.home_help_topic != HomeHelpTopic::Overview
            || self.loading_help_index
        {
            return;
        }
        if self.help_last_checked_at.is_some_and(|checked| {
            checked.elapsed() >= Duration::from_secs(HELP_REFRESH_TTL_SECONDS)
        }) {
            self.request_help_index(fetch_tx, true);
        }
    }

    pub(super) fn request_current_help_page(
        &mut self,
        fetch_tx: &Sender<LabFetchResult>,
        force: bool,
    ) {
        let Some(link) = self.current_help_link().cloned() else {
            self.loading_help_page = false;
            return;
        };
        if link.url.is_empty() {
            if !self.help_pages.contains_key(&link.id)
                && let Some(page) = gitbook::bundled_page(&link)
            {
                self.cache_help_page(link.id.clone(), page);
            }
            self.loading_help_page = false;
            return;
        }
        if self.loading_help_page && self.help_page_request_id.as_deref() == Some(link.id.as_str())
        {
            return;
        }
        if !force
            && self
                .help_pages
                .get(&link.id)
                .is_some_and(|page| page.source == gitbook::GitbookSource::Live)
        {
            self.loading_help_page = false;
            return;
        }
        self.help_page_request = self.help_page_request.wrapping_add(1);
        self.help_page_request_id = Some(link.id.clone());
        self.loading_help_page = true;
        self.help_transition_tick = Some(self.spinner_tick);
        self.status = format!("Loading {} from GitBook...", link.title);
        spawn_help_page_fetch(link, fetch_tx.clone(), self.help_page_request);
    }

    pub(super) fn request_visible_help_preview_if_due(
        &mut self,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        let Some(preview) = self.help_preview.as_ref() else {
            return;
        };
        if self.spinner_tick < preview.reveal_tick
            || preview
                .page
                .as_ref()
                .is_some_and(|page| page.source == gitbook::GitbookSource::Live)
        {
            return;
        }
        let nav_index = preview.nav_index;
        let Some(link) = self.help_link_for_nav(nav_index).cloned() else {
            return;
        };
        if let Some(page) = self.help_pages.get(&link.id).cloned() {
            let is_live = page.source == gitbook::GitbookSource::Live;
            if let Some(preview) = self.help_preview.as_mut() {
                preview.page = Some(page);
            }
            if is_live {
                return;
            }
        } else if let Some(page) = gitbook::bundled_page(&link) {
            self.cache_help_page(link.id.clone(), page);
        }
        if link.url.is_empty()
            || (self.loading_help_page
                && self.help_page_request_id.as_deref() == Some(link.id.as_str()))
            || self
                .help_preview_failed_page_id
                .as_deref()
                .is_some_and(|page_id| page_id == link.id)
            || (self.loading_help_preview_page
                && self.help_preview_page_request_id.as_deref() == Some(link.id.as_str()))
        {
            return;
        }

        self.help_preview_page_request = self.help_preview_page_request.wrapping_add(1);
        self.help_preview_page_request_id = Some(link.id.clone());
        self.loading_help_preview_page = true;
        self.help_preview_failed_page_id = None;
        spawn_help_preview_page_fetch(link, fetch_tx.clone(), self.help_preview_page_request);
    }

    pub(super) fn apply_help_index_result(
        &mut self,
        request_id: u64,
        result: Result<GitbookIndex, String>,
    ) -> bool {
        if request_id != self.help_index_request {
            return false;
        }
        self.loading_help_index = false;
        self.help_last_checked_at = Some(Instant::now());
        let keyboard_preview = self
            .help_preview
            .as_ref()
            .filter(|preview| preview.origin == HelpPreviewOrigin::Keyboard)
            .and_then(|preview| {
                self.help_preview_target_identity(preview.nav_index)
                    .map(|target| (target, preview.clone()))
            });
        match result {
            Ok(index) => {
                self.help_preview = None;
                let previous_expansion = self
                    .help_index
                    .categories
                    .iter()
                    .enumerate()
                    .map(|(index, category)| {
                        (
                            category.title.to_ascii_lowercase(),
                            self.help_expanded_categories
                                .get(index)
                                .copied()
                                .unwrap_or(true),
                        )
                    })
                    .collect::<HashMap<_, _>>();
                self.help_index = index;
                self.help_expanded_categories = self
                    .help_index
                    .categories
                    .iter()
                    .map(|category| {
                        previous_expansion
                            .get(&category.title.to_ascii_lowercase())
                            .copied()
                            .unwrap_or(true)
                    })
                    .collect();
                if self
                    .help_index
                    .page_by_id(&self.help_selected_page_id)
                    .is_none()
                {
                    self.help_selected_page_id = self
                        .help_index
                        .first_page()
                        .map(|page| page.id.clone())
                        .unwrap_or_default();
                }
                if !self.help_pages.contains_key(&self.help_selected_page_id)
                    && let Some(page) = self.current_help_link().and_then(gitbook::bundled_page)
                {
                    self.cache_help_page(self.help_selected_page_id.clone(), page);
                }
                self.sync_help_nav_selection();
                if let Some((target, mut preview)) = keyboard_preview
                    && let Some(nav_index) = self.help_nav_index_for_target_identity(&target)
                {
                    preview.nav_index = nav_index;
                    self.help_selected_nav = nav_index;
                    self.help_nav_scroll = nav_index.saturating_sub(3);
                    self.help_preview = Some(preview);
                }
                self.help_issue = None;
                self.status = format!(
                    "GitBook ready: {} pages. Select a topic and press Enter.",
                    self.help_index.page_count()
                );
                true
            }
            Err(issue) => {
                self.help_issue = Some(issue);
                self.status = "Showing the bundled guide. Live GitBook is unavailable.".to_string();
                false
            }
        }
    }

    pub(super) fn apply_help_page_result(
        &mut self,
        request_id: u64,
        page_id: String,
        result: Result<GitbookPage, String>,
    ) {
        if request_id != self.help_page_request
            || self.help_page_request_id.as_deref() != Some(page_id.as_str())
        {
            return;
        }
        self.loading_help_page = false;
        self.help_page_request_id = None;
        match result {
            Ok(page) => {
                let title = page.title.clone();
                self.cache_help_page(page_id, page);
                self.help_issue = None;
                self.help_transition_tick = Some(self.spinner_tick);
                self.status = format!("{title} updated from GitBook.");
            }
            Err(issue) => {
                let has_last_readable_page = self.help_pages.contains_key(&page_id);
                self.help_issue = Some(issue);
                self.status = if has_last_readable_page {
                    "Keeping the last readable guide page. Live text is unavailable.".to_string()
                } else {
                    "That live guide page is unavailable right now.".to_string()
                };
            }
        }
    }

    pub(super) fn apply_help_preview_page_result(
        &mut self,
        request_id: u64,
        page_id: String,
        result: Result<GitbookPage, String>,
    ) {
        let is_current_request = request_id == self.help_preview_page_request
            && self.help_preview_page_request_id.as_deref() == Some(page_id.as_str());
        match result {
            Ok(page) => {
                self.cache_help_page(page_id.clone(), page);
                if is_current_request {
                    self.loading_help_preview_page = false;
                    self.help_preview_page_request_id = None;
                    self.help_preview_failed_page_id = None;
                }
            }
            Err(_) if is_current_request => {
                self.loading_help_preview_page = false;
                self.help_preview_page_request_id = None;
                self.help_preview_failed_page_id = Some(page_id);
            }
            Err(_) => {}
        }
    }

    pub(super) fn sync_help_nav_selection(&mut self) {
        let rows = gitbook::nav_rows(&self.help_index, &self.help_expanded_categories);
        if let Some(index) = rows.iter().position(|row| match row.target {
            GitbookNavTarget::Page { category, page } => self
                .help_index
                .categories
                .get(category)
                .and_then(|category| category.pages.get(page))
                .is_some_and(|candidate| candidate.id == self.help_selected_page_id),
            GitbookNavTarget::Category(_) => false,
        }) {
            self.help_selected_nav = index;
        } else if !rows.is_empty() {
            self.help_selected_nav = self.help_selected_nav.min(rows.len() - 1);
        } else {
            self.help_selected_nav = 0;
        }
        self.help_nav_scroll = self.help_selected_nav.saturating_sub(3);
    }

    pub(super) fn select_help_nav_offset(&mut self, offset: isize) {
        let row_count = gitbook::nav_rows(&self.help_index, &self.help_expanded_categories).len();
        if row_count == 0 {
            return;
        }
        self.help_selected_nav = if offset < 0 {
            self.help_selected_nav.saturating_sub(offset.unsigned_abs())
        } else {
            self.help_selected_nav
                .saturating_add(offset as usize)
                .min(row_count - 1)
        };
        self.help_nav_scroll = self.help_selected_nav.saturating_sub(3);
        if self
            .help_preview
            .as_ref()
            .is_some_and(|preview| preview.origin == HelpPreviewOrigin::Keyboard)
        {
            self.help_preview_failed_page_id = None;
            self.help_preview =
                self.help_preview_for_nav(self.help_selected_nav, HelpPreviewOrigin::Keyboard);
        } else {
            self.help_preview = None;
        }
    }

    pub(super) fn activate_help_selection(&mut self, fetch_tx: &Sender<LabFetchResult>) {
        self.help_preview = None;
        let rows = gitbook::nav_rows(&self.help_index, &self.help_expanded_categories);
        let Some(row) = rows.get(self.help_selected_nav).cloned() else {
            return;
        };
        match row.target {
            GitbookNavTarget::Category(category) => {
                if let Some(expanded) = self.help_expanded_categories.get_mut(category) {
                    *expanded = !*expanded;
                }
                self.sync_help_nav_selection();
                self.status = if self
                    .help_expanded_categories
                    .get(category)
                    .copied()
                    .unwrap_or(false)
                {
                    format!("{} expanded.", row.label)
                } else {
                    format!("{} collapsed.", row.label)
                };
            }
            GitbookNavTarget::Page { category, page } => {
                let Some(link) = self
                    .help_index
                    .categories
                    .get(category)
                    .and_then(|category| category.pages.get(page))
                    .cloned()
                else {
                    return;
                };
                self.help_selected_page_id = link.id.clone();
                if !self.help_pages.contains_key(&link.id)
                    && let Some(page) = gitbook::bundled_page(&link)
                {
                    self.cache_help_page(link.id.clone(), page);
                }
                self.help_pane = HelpPane::Article;
                self.help_article_scroll = 0;
                self.help_transition_tick = Some(self.spinner_tick);
                // Keep the last readable copy visible, but revalidate a live page whenever
                // the reader returns to it so GitBook edits do not stay stuck in memory.
                self.request_current_help_page(fetch_tx, true);
            }
        }
    }

    pub(super) fn scroll_help_article(&mut self, direction: isize, step: usize) {
        self.help_glossary_hover = None;
        self.help_hover_grace_ticks = None;
        if direction < 0 {
            self.help_article_scroll = self.help_article_scroll.saturating_sub(step);
        } else if direction > 0 {
            self.help_article_scroll = self.help_article_scroll.saturating_add(step);
        }
    }

    pub(super) fn handle_gitbook_help_key(
        &mut self,
        key: &KeyEvent,
        cli: &Cli,
        root: Rect,
        fetch_tx: &Sender<LabFetchResult>,
    ) -> bool {
        if self.screen != LabScreen::Help || self.home_help_topic != HomeHelpTopic::Overview {
            return false;
        }
        if key.code == KeyCode::Char(' ') && key.kind != KeyEventKind::Press {
            return true;
        }
        self.help_glossary_hover = None;
        if self.help_preview.is_some() {
            match key.code {
                KeyCode::Esc => {
                    self.help_preview = None;
                    self.status = "Topic preview closed.".to_string();
                    return true;
                }
                KeyCode::Char(' ') => {
                    self.toggle_keyboard_help_preview();
                    return true;
                }
                KeyCode::PageUp => {
                    let max_scroll =
                        gitbook_help_preview_max_scroll_for_root(cli, root, self).unwrap_or(0);
                    self.scroll_help_preview(-1, PANEL_SCROLL_STEP, max_scroll);
                    return true;
                }
                KeyCode::PageDown => {
                    let max_scroll =
                        gitbook_help_preview_max_scroll_for_root(cli, root, self).unwrap_or(0);
                    self.scroll_help_preview(1, PANEL_SCROLL_STEP, max_scroll);
                    return true;
                }
                _ => {}
            }
        }
        match key.code {
            KeyCode::Backspace => self.open_home(),
            KeyCode::Left => {
                self.help_preview = None;
                self.help_pane = HelpPane::Navigation;
            }
            KeyCode::Right => {
                self.help_preview = None;
                self.help_pane = HelpPane::Article;
            }
            KeyCode::Tab | KeyCode::BackTab => {
                self.help_preview = None;
                self.help_pane = match self.help_pane {
                    HelpPane::Navigation => HelpPane::Article,
                    HelpPane::Article => HelpPane::Navigation,
                };
            }
            KeyCode::Up | KeyCode::Char('k') => match self.help_pane {
                HelpPane::Navigation => self.select_help_nav_offset(-1),
                HelpPane::Article => self.scroll_help_article(-1, 1),
            },
            KeyCode::Down | KeyCode::Char('j') => match self.help_pane {
                HelpPane::Navigation => self.select_help_nav_offset(1),
                HelpPane::Article => self.scroll_help_article(1, 1),
            },
            KeyCode::PageUp => self.scroll_help_article(-1, PANEL_SCROLL_STEP),
            KeyCode::PageDown => self.scroll_help_article(1, PANEL_SCROLL_STEP),
            KeyCode::Enter => self.activate_help_selection(fetch_tx),
            KeyCode::Char(' ') => self.toggle_keyboard_help_preview(),
            KeyCode::Char('r' | 'R') => {
                self.help_preview = None;
                self.request_help_index(fetch_tx, true);
            }
            _ => return false,
        }
        true
    }
}
