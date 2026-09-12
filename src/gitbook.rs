use reqwest::Url;
use reqwest::blocking::{Client, Response};
use reqwest::header::ACCEPT;
use reqwest::redirect::Policy;
use std::io::Read;
use std::sync::OnceLock;
use std::time::Duration;

const INDEX_LIMIT_BYTES: usize = 512 * 1024;
const PAGE_LIMIT_BYTES: usize = 1024 * 1024;
const MAX_PAGES: usize = 500;
const DOCS_CONNECT_TIMEOUT_SECONDS: u64 = 4;
const DOCS_REQUEST_TIMEOUT_SECONDS: u64 = 10;

const BUNDLED_SUMMARY: &str = include_str!("../assets/gitbook/SUMMARY.md");

const BUNDLED_PAGES: &[(&str, &str)] = &[
    ("README.md", include_str!("../assets/gitbook/README.md")),
    (
        "docs/synthetics/01-about-synthetics.md",
        include_str!("../assets/gitbook/docs/synthetics/01-about-synthetics.md"),
    ),
    (
        "docs/synthetics/02-what-synthetics-offer-individuals.md",
        include_str!("../assets/gitbook/docs/synthetics/02-what-synthetics-offer-individuals.md"),
    ),
    (
        "docs/overview/01-why-amoeba-farm.md",
        include_str!("../assets/gitbook/docs/overview/01-why-amoeba-farm.md"),
    ),
    (
        "docs/overview/02-markets-ramx-nandx-future.md",
        include_str!("../assets/gitbook/docs/overview/02-markets-ramx-nandx-future.md"),
    ),
    (
        "docs/getting-started/01-where-to-begin.md",
        include_str!("../assets/gitbook/docs/getting-started/01-where-to-begin.md"),
    ),
    (
        "docs/getting-started/02-cli.md",
        include_str!("../assets/gitbook/docs/getting-started/02-cli.md"),
    ),
    (
        "docs/getting-started/03-web-client.md",
        include_str!("../assets/gitbook/docs/getting-started/03-web-client.md"),
    ),
    (
        "docs/getting-started/04-claude-code-codex.md",
        include_str!("../assets/gitbook/docs/getting-started/04-claude-code-codex.md"),
    ),
    (
        "docs/trading/01-about-capped-options.md",
        include_str!("../assets/gitbook/docs/trading/01-about-capped-options.md"),
    ),
    (
        "docs/trading/02-fees.md",
        include_str!("../assets/gitbook/docs/trading/02-fees.md"),
    ),
    (
        "docs/oracle/01-how-it-is-structured.md",
        include_str!("../assets/gitbook/docs/oracle/01-how-it-is-structured.md"),
    ),
    (
        "docs/oracle/02-token-holders-and-contributors.md",
        include_str!("../assets/gitbook/docs/oracle/02-token-holders-and-contributors.md"),
    ),
    (
        "docs/oracle/03-reward-collection.md",
        include_str!("../assets/gitbook/docs/oracle/03-reward-collection.md"),
    ),
    (
        "docs/developer/01-api.md",
        include_str!("../assets/gitbook/docs/developer/01-api.md"),
    ),
    (
        "docs/04-products-indexes-buckets.md",
        include_str!("../assets/gitbook/docs/04-products-indexes-buckets.md"),
    ),
    (
        "docs/lifecycle/01-market-lifecycle.md",
        include_str!("../assets/gitbook/docs/lifecycle/01-market-lifecycle.md"),
    ),
    (
        "docs/contributors/02-evidence-rules.md",
        include_str!("../assets/gitbook/docs/contributors/02-evidence-rules.md"),
    ),
    (
        "docs/markets/03-risk-disclosures.md",
        include_str!("../assets/gitbook/docs/markets/03-risk-disclosures.md"),
    ),
    (
        "docs/reference/glossary.md",
        include_str!("../assets/gitbook/docs/reference/glossary.md"),
    ),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitbookSource {
    Live,
    Bundled,
}

impl GitbookSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Live => "live GitBook",
            Self::Bundled => "bundled guide",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitbookPageLink {
    pub id: String,
    pub title: String,
    pub url: String,
    pub description: String,
    pub source_path: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitbookCategory {
    pub title: String,
    pub pages: Vec<GitbookPageLink>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitbookIndex {
    pub title: String,
    pub categories: Vec<GitbookCategory>,
    pub index_url: String,
    pub source: GitbookSource,
    structured_navigation: bool,
}

impl GitbookIndex {
    pub fn page_count(&self) -> usize {
        self.categories
            .iter()
            .map(|category| category.pages.len())
            .sum()
    }

    pub fn first_page(&self) -> Option<&GitbookPageLink> {
        self.categories
            .iter()
            .find_map(|category| category.pages.first())
    }

    pub fn page_by_id(&self, id: &str) -> Option<&GitbookPageLink> {
        self.categories
            .iter()
            .flat_map(|category| category.pages.iter())
            .find(|page| page.id == id)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MarkdownBlock {
    Heading {
        level: u8,
        text: String,
    },
    Paragraph(String),
    Bullet {
        depth: usize,
        text: String,
    },
    Numbered {
        depth: usize,
        number: usize,
        text: String,
    },
    Quote(String),
    Code(String),
    Rule,
    Blank,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitbookPage {
    pub title: String,
    pub url: String,
    pub blocks: Vec<MarkdownBlock>,
    pub source: GitbookSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitbookGlossaryEntry {
    pub term: String,
    pub definition: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitbookNavTarget {
    Category(usize),
    Page { category: usize, page: usize },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitbookNavRow {
    pub target: GitbookNavTarget,
    pub label: String,
    pub depth: usize,
    pub expanded: bool,
}

pub fn nav_rows(index: &GitbookIndex, expanded: &[bool]) -> Vec<GitbookNavRow> {
    let mut rows = Vec::new();
    for (category_index, category) in index.categories.iter().enumerate() {
        let category_expanded = expanded.get(category_index).copied().unwrap_or(true);
        rows.push(GitbookNavRow {
            target: GitbookNavTarget::Category(category_index),
            label: category.title.clone(),
            depth: 0,
            expanded: category_expanded,
        });
        if category_expanded {
            rows.extend(category.pages.iter().enumerate().map(|(page_index, page)| {
                GitbookNavRow {
                    target: GitbookNavTarget::Page {
                        category: category_index,
                        page: page_index,
                    },
                    label: page.title.clone(),
                    depth: 1,
                    expanded: false,
                }
            }));
        }
    }
    rows
}

pub fn index_url(docs_root: &str) -> Result<String, String> {
    let trimmed = docs_root.trim();
    if trimmed.is_empty() {
        return Err("The published docs address is not configured.".to_string());
    }
    let mut base =
        Url::parse(trimmed).map_err(|_| "The published docs address is not valid.".to_string())?;
    validate_network_url(&base)?;
    if base.path().ends_with("/llms.txt") || base.path() == "llms.txt" {
        return Ok(base.to_string());
    }
    if !base.path().ends_with('/') {
        let path = format!("{}/", base.path());
        base.set_path(&path);
    }
    base.join("llms.txt")
        .map(|url| url.to_string())
        .map_err(|_| "Petri could not build the published docs index address.".to_string())
}

pub fn fetch_index(docs_root: &str) -> Result<GitbookIndex, String> {
    let url = index_url(docs_root)?;
    let markdown = fetch_markdown(&url, INDEX_LIMIT_BYTES)?;
    let live = parse_index(&url, &markdown)?;
    Ok(merge_with_bundled_navigation(live))
}

pub fn fetch_page(link: &GitbookPageLink) -> Result<GitbookPage, String> {
    if link.url.trim().is_empty() {
        return bundled_page(link)
            .ok_or_else(|| "This guide page is not included in the offline copy.".to_string());
    }
    let markdown = fetch_markdown(&link.url, PAGE_LIMIT_BYTES)?;
    let page = parse_page(&link.url, &markdown)?;
    if page.title.eq_ignore_ascii_case("Page Not Found") {
        return Err(
            "That page moved in the published GitBook. Refresh the guide index.".to_string(),
        );
    }
    Ok(page)
}

pub fn parse_index(index_url: &str, markdown: &str) -> Result<GitbookIndex, String> {
    let base = Url::parse(index_url)
        .map_err(|_| "The published docs index address is not valid.".to_string())?;
    validate_network_url(&base)?;
    let mut title = "Amoeba Farm Docs".to_string();
    let mut section = "Documentation".to_string();
    let mut categories: Vec<GitbookCategory> = Vec::new();
    let mut page_count = 0usize;
    let mut structured_navigation = false;

    for raw_line in markdown.lines() {
        let line = sanitize_terminal_text(raw_line).trim().to_string();
        if let Some(value) = line.strip_prefix("# ") {
            let value = plain_inline(value);
            if !value.is_empty() {
                title = value;
            }
            continue;
        }
        if let Some(value) = line.strip_prefix("## ") {
            let value = plain_inline(value);
            if !value.is_empty() {
                structured_navigation |= !generic_section_title(&value);
                section = value;
            }
            continue;
        }
        let Some((page_title, raw_url, description)) = parse_markdown_link_row(&line) else {
            continue;
        };
        if page_count >= MAX_PAGES {
            break;
        }
        let Ok(page_url) = base.join(&raw_url) else {
            continue;
        };
        if validate_page_url(&base, &page_url).is_err() {
            continue;
        }
        let category_title = if generic_section_title(&section) {
            category_from_page_url(&base, &page_url)
        } else {
            section.clone()
        };
        let category_index = categories
            .iter()
            .position(|category| category.title.eq_ignore_ascii_case(&category_title))
            .unwrap_or_else(|| {
                categories.push(GitbookCategory {
                    title: category_title,
                    pages: Vec::new(),
                });
                categories.len() - 1
            });
        categories[category_index].pages.push(GitbookPageLink {
            id: page_id_from_url(&page_url),
            title: page_title,
            url: page_url.to_string(),
            description,
            source_path: None,
        });
        page_count += 1;
    }

    if page_count == 0 {
        return Err("The published GitBook did not provide any readable pages.".to_string());
    }

    Ok(GitbookIndex {
        title,
        categories,
        index_url: base.to_string(),
        source: GitbookSource::Live,
        structured_navigation,
    })
}

pub fn bundled_index() -> GitbookIndex {
    parse_bundled_summary(BUNDLED_SUMMARY)
}

pub fn bundled_page(link: &GitbookPageLink) -> Option<GitbookPage> {
    let source_path = link.source_path.as_deref()?;
    let (_, markdown) = BUNDLED_PAGES
        .iter()
        .find(|(path, _)| *path == source_path)?;
    let mut page = parse_page(&format!("bundled://{source_path}"), markdown).ok()?;
    page.source = GitbookSource::Bundled;
    Some(page)
}

pub fn bundled_glossary_entries() -> &'static [GitbookGlossaryEntry] {
    static ENTRIES: OnceLock<Vec<GitbookGlossaryEntry>> = OnceLock::new();
    ENTRIES.get_or_init(parse_bundled_glossary_entries)
}

fn parse_bundled_glossary_entries() -> Vec<GitbookGlossaryEntry> {
    let Some((_, markdown)) = BUNDLED_PAGES
        .iter()
        .find(|(path, _)| *path == "docs/reference/glossary.md")
    else {
        return Vec::new();
    };
    let Ok(page) = parse_page("bundled://docs/reference/glossary.md", markdown) else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    let mut pending_terms = Vec::new();
    for block in page.blocks {
        match block {
            MarkdownBlock::Heading { level: 2, text } => {
                pending_terms = text
                    .split(" / ")
                    .map(str::trim)
                    .filter(|term| !term.is_empty())
                    .map(str::to_string)
                    .collect();
            }
            MarkdownBlock::Paragraph(definition) => {
                if !definition.trim().is_empty() {
                    entries.extend(pending_terms.drain(..).map(|term| GitbookGlossaryEntry {
                        term,
                        definition: definition.clone(),
                    }));
                }
            }
            MarkdownBlock::Blank => {}
            _ => pending_terms.clear(),
        }
    }
    entries
}

fn parse_bundled_summary(summary: &str) -> GitbookIndex {
    let mut categories: Vec<GitbookCategory> = Vec::new();
    let mut category_title = "Start Here".to_string();
    for raw_line in summary.lines() {
        let line = sanitize_terminal_text(raw_line).trim().to_string();
        if let Some(value) = line.strip_prefix("## ") {
            category_title = plain_inline(value);
            continue;
        }
        let Some((title, path, _)) = parse_markdown_link_row(&line.replace("* [", "- [")) else {
            continue;
        };
        if !BUNDLED_PAGES.iter().any(|(known, _)| *known == path) {
            continue;
        }
        let category_index = categories
            .iter()
            .position(|category| category.title == category_title)
            .unwrap_or_else(|| {
                categories.push(GitbookCategory {
                    title: category_title.clone(),
                    pages: Vec::new(),
                });
                categories.len() - 1
            });
        let description = BUNDLED_PAGES
            .iter()
            .find(|(known, _)| *known == path)
            .and_then(|(_, markdown)| parse_page("bundled://preview", markdown).ok())
            .and_then(|page| {
                page.blocks.into_iter().find_map(|block| match block {
                    MarkdownBlock::Paragraph(text) => Some(text),
                    _ => None,
                })
            })
            .unwrap_or_default();
        categories[category_index].pages.push(GitbookPageLink {
            id: path.to_ascii_lowercase(),
            title,
            url: String::new(),
            description,
            source_path: Some(path),
        });
    }
    GitbookIndex {
        title: "Amoeba Farm".to_string(),
        categories,
        index_url: String::new(),
        source: GitbookSource::Bundled,
        structured_navigation: true,
    }
}

fn merge_with_bundled_navigation(live: GitbookIndex) -> GitbookIndex {
    if live.structured_navigation {
        let bundled = bundled_index();
        let mut live = live;
        for live_page in live
            .categories
            .iter_mut()
            .flat_map(|category| category.pages.iter_mut())
        {
            let bundled_page = bundled
                .categories
                .iter()
                .flat_map(|category| category.pages.iter())
                .find(|bundled_page| {
                    normalize_title_key(&live_page.title)
                        == normalize_title_key(&bundled_page.title)
                });
            if let Some(bundled_page) = bundled_page {
                live_page.source_path = bundled_page.source_path.clone();
                if live_page.description.is_empty() {
                    live_page.description = bundled_page.description.clone();
                }
            }
        }
        return live;
    }
    let bundled = bundled_index();
    let mut remaining = live
        .categories
        .iter()
        .flat_map(|category| category.pages.iter().cloned())
        .collect::<Vec<_>>();
    let mut categories = Vec::new();

    for bundled_category in bundled.categories {
        let mut pages = Vec::new();
        for bundled_page in bundled_category.pages {
            let match_index = remaining.iter().position(|live_page| {
                normalize_title_key(&live_page.title) == normalize_title_key(&bundled_page.title)
            });
            if let Some(match_index) = match_index {
                let mut live_page = remaining.remove(match_index);
                live_page.id = bundled_page.id;
                live_page.source_path = bundled_page.source_path;
                if live_page.description.is_empty() {
                    live_page.description = bundled_page.description;
                }
                pages.push(live_page);
            }
        }
        if !pages.is_empty() {
            categories.push(GitbookCategory {
                title: bundled_category.title,
                pages,
            });
        }
    }

    for live_category in live.categories {
        let leftovers = live_category
            .pages
            .into_iter()
            .filter(|page| remaining.iter().any(|candidate| candidate.url == page.url))
            .collect::<Vec<_>>();
        if leftovers.is_empty() {
            continue;
        }
        if let Some(existing) = categories
            .iter_mut()
            .find(|category| category.title.eq_ignore_ascii_case(&live_category.title))
        {
            existing.pages.extend(leftovers);
        } else {
            categories.push(GitbookCategory {
                title: live_category.title,
                pages: leftovers,
            });
        }
    }

    GitbookIndex {
        title: live.title,
        categories,
        index_url: live.index_url,
        source: GitbookSource::Live,
        structured_navigation: false,
    }
}

pub fn parse_page(url: &str, markdown: &str) -> Result<GitbookPage, String> {
    let sanitized = sanitize_terminal_text(markdown);
    let mut title = "Documentation".to_string();
    let mut blocks = Vec::new();
    let mut code_fence = false;
    let mut frontmatter = false;
    let mut at_start = true;
    let mut paragraph = Vec::<String>::new();

    fn flush_paragraph(blocks: &mut Vec<MarkdownBlock>, paragraph: &mut Vec<String>) {
        if !paragraph.is_empty() {
            blocks.push(MarkdownBlock::Paragraph(paragraph.join(" ")));
            paragraph.clear();
        }
    }

    for raw_line in sanitized.lines() {
        let trimmed = raw_line.trim();
        if at_start && trimmed == "---" {
            frontmatter = true;
            at_start = false;
            continue;
        }
        at_start = false;
        if frontmatter {
            if trimmed == "---" {
                frontmatter = false;
            }
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            flush_paragraph(&mut blocks, &mut paragraph);
            code_fence = !code_fence;
            continue;
        }
        if code_fence {
            blocks.push(MarkdownBlock::Code(raw_line.to_string()));
            continue;
        }
        if is_gitbook_wrapper(trimmed) || is_gitbook_index_preamble(trimmed) {
            continue;
        }
        if let Some((level, heading)) = parse_heading(trimmed) {
            flush_paragraph(&mut blocks, &mut paragraph);
            let heading = plain_inline(heading);
            if heading.eq_ignore_ascii_case("Agent Instructions")
                && (title != "Documentation" || !blocks.is_empty())
            {
                while matches!(
                    blocks.last(),
                    Some(MarkdownBlock::Blank | MarkdownBlock::Rule)
                ) {
                    blocks.pop();
                }
                break;
            }
            if level == 1 && title == "Documentation" && !heading.is_empty() {
                title = heading.clone();
                continue;
            }
            if !heading.is_empty() {
                blocks.push(MarkdownBlock::Heading {
                    level,
                    text: heading,
                });
            }
            continue;
        }
        if trimmed.is_empty() {
            flush_paragraph(&mut blocks, &mut paragraph);
            if !blocks.is_empty() && !matches!(blocks.last(), Some(MarkdownBlock::Blank)) {
                blocks.push(MarkdownBlock::Blank);
            }
            continue;
        }
        if matches!(trimmed, "---" | "***" | "___") {
            flush_paragraph(&mut blocks, &mut paragraph);
            blocks.push(MarkdownBlock::Rule);
            continue;
        }
        if let Some(text) = trimmed.strip_prefix("> ") {
            flush_paragraph(&mut blocks, &mut paragraph);
            blocks.push(MarkdownBlock::Quote(plain_inline(text)));
            continue;
        }
        let indent = raw_line
            .chars()
            .take_while(|character| character.is_whitespace())
            .count();
        if let Some(text) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .or_else(|| trimmed.strip_prefix("+ "))
        {
            flush_paragraph(&mut blocks, &mut paragraph);
            blocks.push(MarkdownBlock::Bullet {
                depth: indent / 2,
                text: plain_inline(text),
            });
            continue;
        }
        if let Some((number, text)) = parse_numbered_item(trimmed) {
            flush_paragraph(&mut blocks, &mut paragraph);
            blocks.push(MarkdownBlock::Numbered {
                depth: indent / 2,
                number,
                text: plain_inline(text),
            });
            continue;
        }
        if trimmed.starts_with('|') && trimmed.ends_with('|') {
            flush_paragraph(&mut blocks, &mut paragraph);
            let cells = trimmed
                .trim_matches('|')
                .split('|')
                .map(|cell| cell.trim())
                .collect::<Vec<_>>();
            let separator = cells.iter().all(|cell| {
                !cell.is_empty() && cell.chars().all(|character| matches!(character, '-' | ':'))
            });
            if !separator {
                blocks.push(MarkdownBlock::Code(format!(
                    "| {} |",
                    cells
                        .into_iter()
                        .map(plain_inline)
                        .collect::<Vec<_>>()
                        .join(" | ")
                )));
            }
            continue;
        }
        let text = plain_inline(trimmed);
        if !text.is_empty() {
            paragraph.push(text);
        }
    }
    flush_paragraph(&mut blocks, &mut paragraph);
    while matches!(blocks.last(), Some(MarkdownBlock::Blank)) {
        blocks.pop();
    }
    if blocks.is_empty() {
        return Err("The selected GitBook page did not contain readable text.".to_string());
    }
    Ok(GitbookPage {
        title,
        url: url.to_string(),
        blocks,
        source: if url.starts_with("http://") || url.starts_with("https://") {
            GitbookSource::Live
        } else {
            GitbookSource::Bundled
        },
    })
}

pub fn sanitize_terminal_text(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '\u{1b}' {
            match chars.next() {
                Some('[') => {
                    for next in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&next) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    let mut saw_escape = false;
                    for next in chars.by_ref() {
                        if next == '\u{7}' || (saw_escape && next == '\\') {
                            break;
                        }
                        saw_escape = next == '\u{1b}';
                    }
                }
                Some(_) | None => {}
            }
            continue;
        }
        if character == '\n' || character == '\r' {
            output.push(character);
            continue;
        }
        if character == '\t' {
            output.push_str("    ");
            continue;
        }
        if character.is_control()
            || matches!(
                character,
                '\u{200b}'
                    | '\u{200c}'
                    | '\u{200d}'
                    | '\u{202a}'..='\u{202e}'
                    | '\u{2066}'..='\u{2069}'
                    | '\u{feff}'
            )
        {
            continue;
        }
        output.push(character);
    }
    output
}

pub fn loading_frame(tick: usize) -> [&'static str; 3] {
    const FRAMES: [[&str; 3]; 4] = [
        ["   .-.   ", "  ( . )  ", "   `-'   "],
        ["  .---.  ", " (  o  ) ", "  `---'  "],
        [" .-----. ", "(   O   )", " `-----' "],
        ["  .---.  ", " ( o o ) ", "  `---'  "],
    ];
    FRAMES[tick % FRAMES.len()]
}

pub fn section_divider_frame(tick: usize) -> &'static str {
    const FRAMES: [&str; 4] = ["-- . --", "- (.) -", "-- o --", "- (O) -"];
    FRAMES[tick % FRAMES.len()]
}

fn fetch_markdown(url: &str, limit: usize) -> Result<String, String> {
    let requested =
        Url::parse(url).map_err(|_| "The published docs page address is not valid.".to_string())?;
    validate_network_url(&requested)?;
    let redirect_origin = requested.clone();
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(DOCS_CONNECT_TIMEOUT_SECONDS))
        .timeout(Duration::from_secs(DOCS_REQUEST_TIMEOUT_SECONDS))
        .redirect(Policy::custom(move |attempt| {
            if redirect_target_allowed(&redirect_origin, attempt.url(), attempt.previous().len()) {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .user_agent("petri-cli")
        .build()
        .map_err(|_| "Petri could not prepare the docs reader.".to_string())?;
    let response = client
        .get(requested.clone())
        .header(ACCEPT, "text/markdown, text/plain;q=0.9")
        .send()
        .map_err(|_| "The published GitBook is not reachable right now.".to_string())?;
    if !same_origin(&requested, response.url()) {
        return Err("The published docs redirected outside their configured site.".to_string());
    }
    if !response.status().is_success() {
        return Err("The published GitBook is not available right now.".to_string());
    }
    read_response_limited(response, limit)
}

fn read_response_limited(response: Response, limit: usize) -> Result<String, String> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(
            "The published docs response is too large for the terminal reader.".to_string(),
        );
    }
    let mut bytes = Vec::with_capacity(limit.min(64 * 1024));
    response
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Petri could not read the published docs response.".to_string())?;
    if bytes.len() > limit {
        return Err(
            "The published docs response is too large for the terminal reader.".to_string(),
        );
    }
    String::from_utf8(bytes)
        .map_err(|_| "The published docs response was not valid text.".to_string())
}

fn validate_network_url(url: &Url) -> Result<(), String> {
    match url.scheme() {
        "https" => Ok(()),
        "http" if is_loopback_host(url.host_str().unwrap_or_default()) => Ok(()),
        _ => Err("Published docs must use HTTPS.".to_string()),
    }
}

fn validate_page_url(index: &Url, page: &Url) -> Result<(), String> {
    validate_network_url(page)?;
    if !same_origin(index, page) || !page.path().to_ascii_lowercase().ends_with(".md") {
        return Err("The GitBook index included an unsupported page address.".to_string());
    }
    Ok(())
}

fn same_origin(left: &Url, right: &Url) -> bool {
    left.scheme() == right.scheme()
        && left.host_str().map(str::to_ascii_lowercase)
            == right.host_str().map(str::to_ascii_lowercase)
        && left.port_or_known_default() == right.port_or_known_default()
}

fn redirect_target_allowed(origin: &Url, target: &Url, previous_count: usize) -> bool {
    previous_count < 5 && validate_network_url(target).is_ok() && same_origin(origin, target)
}

fn is_loopback_host(host: &str) -> bool {
    matches!(
        host.to_ascii_lowercase().as_str(),
        "localhost" | "127.0.0.1" | "::1"
    )
}

fn parse_markdown_link_row(line: &str) -> Option<(String, String, String)> {
    let line = line.strip_prefix("- [")?;
    let title_end = line.find("](")?;
    let title = plain_inline(&line[..title_end]);
    let rest = &line[title_end + 2..];
    let url_end = rest.find(')')?;
    let url = rest[..url_end].trim().to_string();
    let description = rest[url_end + 1..]
        .trim()
        .strip_prefix(':')
        .unwrap_or(rest[url_end + 1..].trim())
        .trim();
    (!title.is_empty() && !url.is_empty()).then(|| (title, url, plain_inline(description)))
}

fn generic_section_title(section: &str) -> bool {
    matches!(
        normalize_title_key(section).as_str(),
        "documentation" | "docs" | "pages" | "contents"
    )
}

fn category_from_page_url(index: &Url, page: &Url) -> String {
    let index_root = index.path().trim_end_matches("llms.txt").trim_matches('/');
    let mut relative = page.path().trim_matches('/');
    if !index_root.is_empty() {
        relative = relative
            .strip_prefix(index_root)
            .unwrap_or(relative)
            .trim_matches('/');
    }
    let mut parts = relative.split('/').filter(|part| !part.is_empty());
    let first = parts.next().unwrap_or("start-here");
    let second = parts.next();
    if first.eq_ignore_ascii_case("docs") {
        return second
            .map(title_case_slug)
            .unwrap_or_else(|| "Start Here".to_string());
    }
    if second.is_none() {
        "Start Here".to_string()
    } else {
        title_case_slug(first)
    }
}

fn title_case_slug(value: &str) -> String {
    value
        .trim_end_matches(".md")
        .split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn page_id_from_url(url: &Url) -> String {
    url.path()
        .trim_matches('/')
        .trim_end_matches(".md")
        .to_ascii_lowercase()
}

fn normalize_title_key(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn parse_heading(line: &str) -> Option<(u8, &str)> {
    let hashes = line
        .chars()
        .take_while(|character| *character == '#')
        .count();
    if !(1..=6).contains(&hashes) || line.chars().nth(hashes) != Some(' ') {
        return None;
    }
    Some((hashes as u8, line[hashes + 1..].trim()))
}

fn parse_numbered_item(line: &str) -> Option<(usize, &str)> {
    let digits = line
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .count();
    if digits == 0 || !line[digits..].starts_with(". ") {
        return None;
    }
    let number = line[..digits].parse().ok()?;
    Some((number, line[digits + 2..].trim()))
}

fn is_gitbook_wrapper(line: &str) -> bool {
    (line.starts_with("{%") && line.ends_with("%}")) || matches!(line, ":::" | "::::")
}

fn is_gitbook_index_preamble(line: &str) -> bool {
    let text = line.strip_prefix('>').map(str::trim).unwrap_or(line);
    text.contains("For the complete documentation index")
        && text.contains("llms.txt")
        && text.contains("Markdown versions of documentation pages")
}

fn plain_inline(value: &str) -> String {
    let mut text = strip_html_tags(value);
    text = replace_markdown_images(&text);
    text = replace_markdown_links(&text);
    for marker in ["**", "__", "~~", "`"] {
        text = text.replace(marker, "");
    }
    text.trim().to_string()
}

fn strip_html_tags(value: &str) -> String {
    let mut output = String::new();
    let mut in_tag = false;
    for character in value.chars() {
        match character {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => output.push(character),
            _ => {}
        }
    }
    output
}

fn replace_markdown_images(value: &str) -> String {
    let mut output = String::new();
    let mut remaining = value;
    while let Some(start) = remaining.find("![") {
        output.push_str(&remaining[..start]);
        let after = &remaining[start + 2..];
        let Some(label_end) = after.find("](") else {
            output.push_str(&remaining[start..]);
            return output;
        };
        let target = &after[label_end + 2..];
        let Some(url_end) = target.find(')') else {
            output.push_str(&remaining[start..]);
            return output;
        };
        output.push_str("[image: ");
        output.push_str(&after[..label_end]);
        output.push(']');
        remaining = &target[url_end + 1..];
    }
    output.push_str(remaining);
    output
}

fn replace_markdown_links(value: &str) -> String {
    let mut output = String::new();
    let mut remaining = value;
    while let Some(start) = remaining.find('[') {
        output.push_str(&remaining[..start]);
        let after = &remaining[start + 1..];
        let Some(label_end) = after.find("](") else {
            output.push_str(&remaining[start..]);
            return output;
        };
        let target = &after[label_end + 2..];
        let Some(url_end) = target.find(')') else {
            output.push_str(&remaining[start..]);
            return output;
        };
        output.push_str(&after[..label_end]);
        let url = target[..url_end].trim();
        if url.starts_with("http://") || url.starts_with("https://") {
            output.push_str(" (");
            output.push_str(url);
            output.push(')');
        }
        remaining = &target[url_end + 1..];
    }
    output.push_str(remaining);
    output
}
