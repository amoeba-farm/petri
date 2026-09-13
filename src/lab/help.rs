//! GitBook Help cache, preview, hover, selection, and keyboard state transitions.

use super::*;

impl LabApp {
    pub(super) fn cache_help_page(&mut self, page_id: String, page: GitbookPage) {
        let updates_visible_preview = self
            .help
            .preview
            .as_ref()
            .and_then(|preview| self.help.link_for_nav(preview.nav_index))
            .is_some_and(|link| link.id == page_id);
        if updates_visible_preview && let Some(preview) = self.help.preview.as_mut() {
            preview.page = Some(page.clone());
        }
        self.cache
            .store_help_page(page_id, page, &self.help.selected_page_id);
    }

    pub(super) fn prepare_gitbook_help(&mut self) {
        self.help.preview = None;
        self.help.preview_failed_page_id = None;
        self.help.glossary_hover = None;
        self.help.hover_grace_ticks = None;
        if self.help.index.categories.is_empty() {
            self.help.index = gitbook::bundled_index();
            self.help.expanded_categories = vec![true; self.help.index.categories.len()];
        }
        if self.help.expanded_categories.len() != self.help.index.categories.len() {
            self.help.expanded_categories = vec![true; self.help.index.categories.len()];
        }
        if self
            .help
            .index
            .page_by_id(&self.help.selected_page_id)
            .is_none()
        {
            self.help.selected_page_id = self
                .help
                .index
                .first_page()
                .map(|page| page.id.clone())
                .unwrap_or_default();
        }
        if !self.help.selected_page_id.is_empty()
            && !self
                .cache
                .help_pages()
                .contains_key(&self.help.selected_page_id)
            && let Some(page) = self
                .help
                .index
                .page_by_id(&self.help.selected_page_id)
                .and_then(gitbook::bundled_page)
        {
            self.cache_help_page(self.help.selected_page_id.clone(), page);
        }
        self.help.sync_nav_selection();
        self.help.pane = HelpPane::Navigation;
        self.help.article_scroll = 0;
    }

    pub(super) fn current_help_page(&self) -> Option<&GitbookPage> {
        self.cache.help_pages().get(&self.help.selected_page_id)
    }

    pub(super) fn help_preview_for_nav(
        &self,
        nav_index: usize,
        origin: HelpPreviewOrigin,
    ) -> Option<GitbookHelpPreview> {
        let rows = gitbook::nav_rows(&self.help.index, &self.help.expanded_categories);
        let row = rows.get(nav_index)?;
        let page = match row.target {
            GitbookNavTarget::Category(_) => None,
            GitbookNavTarget::Page { category, page } => self
                .help
                .index
                .categories
                .get(category)
                .and_then(|category| category.pages.get(page))
                .and_then(|link| {
                    self.cache
                        .help_pages()
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

    pub(super) fn set_help_hover_preview(&mut self, nav_index: usize) {
        if self
            .help
            .preview
            .as_ref()
            .is_some_and(|preview| preview.origin == HelpPreviewOrigin::Keyboard)
        {
            return;
        }
        self.help.glossary_hover = None;
        self.help.hover_grace_ticks = None;
        self.help.selected_nav = nav_index;
        if self.help.preview.as_ref().is_some_and(|preview| {
            preview.origin == HelpPreviewOrigin::Hover && preview.nav_index == nav_index
        }) {
            return;
        }
        self.help.preview_failed_page_id = None;
        self.help.preview = self.help_preview_for_nav(nav_index, HelpPreviewOrigin::Hover);
    }

    pub(super) fn toggle_keyboard_help_preview(&mut self) {
        self.help.glossary_hover = None;
        self.help.hover_grace_ticks = None;
        if self.help.pane != HelpPane::Navigation {
            self.help.pane = HelpPane::Navigation;
        }
        if self.help.preview.as_ref().is_some_and(|preview| {
            preview.origin == HelpPreviewOrigin::Keyboard
                && preview.nav_index == self.help.selected_nav
        }) {
            self.help.preview = None;
            self.status = "Topic preview closed.".to_string();
            return;
        }
        self.help.preview_failed_page_id = None;
        self.help.preview =
            self.help_preview_for_nav(self.help.selected_nav, HelpPreviewOrigin::Keyboard);
        if self.help.preview.is_some() {
            self.status =
                "Topic preview open. Use arrows to browse, PgUp/PgDn to scroll, Enter to open."
                    .to_string();
        }
    }

    pub(super) fn request_help_index(&mut self, fetch_tx: &Sender<LabFetchResult>, force: bool) {
        if self.help.loading_index {
            if force {
                self.status = "The published GitBook refresh is already running.".to_string();
            }
            return;
        }
        self.help.index_request = self.help.index_request.wrapping_add(1);
        self.help.loading_index = true;
        self.help.issue = None;
        self.status = if force {
            "Refreshing the published GitBook...".to_string()
        } else {
            "Checking the published GitBook...".to_string()
        };
        let root = docs_url();
        spawn_help_index_fetch(root, fetch_tx.clone(), self.help.index_request);
    }

    pub(super) fn refresh_gitbook_help_if_due(&mut self, fetch_tx: &Sender<LabFetchResult>) {
        if self.screen != LabScreen::Help
            || self.home_help_topic != HomeHelpTopic::Overview
            || self.help.loading_index
        {
            return;
        }
        if self.help.last_checked_at.is_some_and(|checked| {
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
        let Some(link) = self.help.current_link().cloned() else {
            self.help.loading_page = false;
            return;
        };
        if link.url.is_empty() {
            if !self.cache.help_pages().contains_key(&link.id)
                && let Some(page) = gitbook::bundled_page(&link)
            {
                self.cache_help_page(link.id.clone(), page);
            }
            self.help.loading_page = false;
            return;
        }
        if self.help.loading_page && self.help.page_request_id.as_deref() == Some(link.id.as_str())
        {
            return;
        }
        if !force
            && self
                .cache
                .help_pages()
                .get(&link.id)
                .is_some_and(|page| page.source == gitbook::GitbookSource::Live)
        {
            self.help.loading_page = false;
            return;
        }
        self.help.page_request = self.help.page_request.wrapping_add(1);
        self.help.page_request_id = Some(link.id.clone());
        self.help.loading_page = true;
        self.help.transition_tick = Some(self.spinner_tick);
        self.status = format!("Loading {} from GitBook...", link.title);
        spawn_help_page_fetch(link, fetch_tx.clone(), self.help.page_request);
    }

    pub(super) fn request_visible_help_preview_if_due(
        &mut self,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        let Some(preview) = self.help.preview.as_ref() else {
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
        let Some(link) = self.help.link_for_nav(nav_index).cloned() else {
            return;
        };
        if let Some(page) = self.cache.help_pages().get(&link.id).cloned() {
            let is_live = page.source == gitbook::GitbookSource::Live;
            if let Some(preview) = self.help.preview.as_mut() {
                preview.page = Some(page);
            }
            if is_live {
                return;
            }
        } else if let Some(page) = gitbook::bundled_page(&link) {
            self.cache_help_page(link.id.clone(), page);
        }
        if link.url.is_empty()
            || (self.help.loading_page
                && self.help.page_request_id.as_deref() == Some(link.id.as_str()))
            || self
                .help
                .preview_failed_page_id
                .as_deref()
                .is_some_and(|page_id| page_id == link.id)
            || (self.help.loading_preview_page
                && self.help.preview_page_request_id.as_deref() == Some(link.id.as_str()))
        {
            return;
        }

        self.help.preview_page_request = self.help.preview_page_request.wrapping_add(1);
        self.help.preview_page_request_id = Some(link.id.clone());
        self.help.loading_preview_page = true;
        self.help.preview_failed_page_id = None;
        spawn_help_preview_page_fetch(link, fetch_tx.clone(), self.help.preview_page_request);
    }

    pub(super) fn apply_help_index_result(
        &mut self,
        request_id: u64,
        result: Result<GitbookIndex, String>,
    ) -> bool {
        if request_id != self.help.index_request {
            return false;
        }
        self.help.loading_index = false;
        self.help.last_checked_at = Some(Instant::now());
        let keyboard_preview = self
            .help
            .preview
            .as_ref()
            .filter(|preview| preview.origin == HelpPreviewOrigin::Keyboard)
            .and_then(|preview| {
                self.help
                    .preview_target_identity(preview.nav_index)
                    .map(|target| (target, preview.clone()))
            });
        match result {
            Ok(index) => {
                self.help.preview = None;
                let previous_expansion = self
                    .help
                    .index
                    .categories
                    .iter()
                    .enumerate()
                    .map(|(index, category)| {
                        (
                            category.title.to_ascii_lowercase(),
                            self.help
                                .expanded_categories
                                .get(index)
                                .copied()
                                .unwrap_or(true),
                        )
                    })
                    .collect::<HashMap<_, _>>();
                self.help.index = index;
                self.help.expanded_categories = self
                    .help
                    .index
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
                    .help
                    .index
                    .page_by_id(&self.help.selected_page_id)
                    .is_none()
                {
                    self.help.selected_page_id = self
                        .help
                        .index
                        .first_page()
                        .map(|page| page.id.clone())
                        .unwrap_or_default();
                }
                if !self
                    .cache
                    .help_pages()
                    .contains_key(&self.help.selected_page_id)
                    && let Some(page) = self.help.current_link().and_then(gitbook::bundled_page)
                {
                    self.cache_help_page(self.help.selected_page_id.clone(), page);
                }
                self.help.sync_nav_selection();
                if let Some((target, mut preview)) = keyboard_preview
                    && let Some(nav_index) = self.help.nav_index_for_target_identity(&target)
                {
                    preview.nav_index = nav_index;
                    self.help.selected_nav = nav_index;
                    self.help.nav_scroll = nav_index.saturating_sub(3);
                    self.help.preview = Some(preview);
                }
                self.help.issue = None;
                self.status = format!(
                    "GitBook ready: {} pages. Select a topic and press Enter.",
                    self.help.index.page_count()
                );
                true
            }
            Err(issue) => {
                self.help.issue = Some(issue);
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
        if request_id != self.help.page_request
            || self.help.page_request_id.as_deref() != Some(page_id.as_str())
        {
            return;
        }
        self.help.loading_page = false;
        self.help.page_request_id = None;
        match result {
            Ok(page) => {
                let title = page.title.clone();
                self.cache_help_page(page_id, page);
                self.help.issue = None;
                self.help.transition_tick = Some(self.spinner_tick);
                self.status = format!("{title} updated from GitBook.");
            }
            Err(issue) => {
                let has_last_readable_page = self.cache.help_pages().contains_key(&page_id);
                self.help.issue = Some(issue);
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
        let is_current_request = request_id == self.help.preview_page_request
            && self.help.preview_page_request_id.as_deref() == Some(page_id.as_str());
        match result {
            Ok(page) => {
                self.cache_help_page(page_id.clone(), page);
                if is_current_request {
                    self.help.loading_preview_page = false;
                    self.help.preview_page_request_id = None;
                    self.help.preview_failed_page_id = None;
                }
            }
            Err(_) if is_current_request => {
                self.help.loading_preview_page = false;
                self.help.preview_page_request_id = None;
                self.help.preview_failed_page_id = Some(page_id);
            }
            Err(_) => {}
        }
    }

    pub(super) fn select_help_nav_offset(&mut self, offset: isize) {
        let row_count = gitbook::nav_rows(&self.help.index, &self.help.expanded_categories).len();
        if row_count == 0 {
            return;
        }
        self.help.selected_nav = if offset < 0 {
            self.help.selected_nav.saturating_sub(offset.unsigned_abs())
        } else {
            self.help
                .selected_nav
                .saturating_add(offset as usize)
                .min(row_count - 1)
        };
        self.help.nav_scroll = self.help.selected_nav.saturating_sub(3);
        if self
            .help
            .preview
            .as_ref()
            .is_some_and(|preview| preview.origin == HelpPreviewOrigin::Keyboard)
        {
            self.help.preview_failed_page_id = None;
            self.help.preview =
                self.help_preview_for_nav(self.help.selected_nav, HelpPreviewOrigin::Keyboard);
        } else {
            self.help.preview = None;
        }
    }

    pub(super) fn activate_help_selection(&mut self, fetch_tx: &Sender<LabFetchResult>) {
        self.help.preview = None;
        let rows = gitbook::nav_rows(&self.help.index, &self.help.expanded_categories);
        let Some(row) = rows.get(self.help.selected_nav).cloned() else {
            return;
        };
        match row.target {
            GitbookNavTarget::Category(category) => {
                if let Some(expanded) = self.help.expanded_categories.get_mut(category) {
                    *expanded = !*expanded;
                }
                self.help.sync_nav_selection();
                self.status = if self
                    .help
                    .expanded_categories
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
                    .help
                    .index
                    .categories
                    .get(category)
                    .and_then(|category| category.pages.get(page))
                    .cloned()
                else {
                    return;
                };
                self.help.selected_page_id = link.id.clone();
                if !self.cache.help_pages().contains_key(&link.id)
                    && let Some(page) = gitbook::bundled_page(&link)
                {
                    self.cache_help_page(link.id.clone(), page);
                }
                self.help.pane = HelpPane::Article;
                self.help.article_scroll = 0;
                self.help.transition_tick = Some(self.spinner_tick);
                // Keep the last readable copy visible, but revalidate a live page whenever
                // the reader returns to it so GitBook edits do not stay stuck in memory.
                self.request_current_help_page(fetch_tx, true);
            }
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
        self.help.glossary_hover = None;
        if self.help.preview.is_some() {
            match key.code {
                KeyCode::Esc => {
                    self.help.preview = None;
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
                    self.help.scroll_preview(-1, PANEL_SCROLL_STEP, max_scroll);
                    return true;
                }
                KeyCode::PageDown => {
                    let max_scroll =
                        gitbook_help_preview_max_scroll_for_root(cli, root, self).unwrap_or(0);
                    self.help.scroll_preview(1, PANEL_SCROLL_STEP, max_scroll);
                    return true;
                }
                _ => {}
            }
        }
        match key.code {
            KeyCode::Backspace => self.open_home(),
            KeyCode::Left => {
                self.help.preview = None;
                self.help.pane = HelpPane::Navigation;
            }
            KeyCode::Right => {
                self.help.preview = None;
                self.help.pane = HelpPane::Article;
            }
            KeyCode::Tab | KeyCode::BackTab => {
                self.help.preview = None;
                self.help.pane = match self.help.pane {
                    HelpPane::Navigation => HelpPane::Article,
                    HelpPane::Article => HelpPane::Navigation,
                };
            }
            KeyCode::Up | KeyCode::Char('k') => match self.help.pane {
                HelpPane::Navigation => self.select_help_nav_offset(-1),
                HelpPane::Article => self.help.scroll_article(-1, 1),
            },
            KeyCode::Down | KeyCode::Char('j') => match self.help.pane {
                HelpPane::Navigation => self.select_help_nav_offset(1),
                HelpPane::Article => self.help.scroll_article(1, 1),
            },
            KeyCode::PageUp => self.help.scroll_article(-1, PANEL_SCROLL_STEP),
            KeyCode::PageDown => self.help.scroll_article(1, PANEL_SCROLL_STEP),
            KeyCode::Enter => self.activate_help_selection(fetch_tx),
            KeyCode::Char(' ') => self.toggle_keyboard_help_preview(),
            KeyCode::Char('r' | 'R') => {
                self.help.preview = None;
                self.request_help_index(fetch_tx, true);
            }
            _ => return false,
        }
        true
    }
}
