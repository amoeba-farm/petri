//! Wallet-scoped operation recovery. No site display feeds or signing.
use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(super) struct ReadPanel {
    pub id: u64,
    pub title: String,
    pub content: String,
    pub issue: Option<String>,
    pub selected: usize,
    pub scroll: u16,
    pub operations: Vec<String>,
    pub stop: Arc<AtomicBool>,
}
impl Drop for ReadPanel {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
impl LabApp {
    fn open_oracle_read_panel(&mut self, commitments: bool, tx: &Sender<LabFetchResult>) {
        self.read_panel_request = self.read_panel_request.wrapping_add(1);
        let id = self.read_panel_request;
        let stop = Arc::new(AtomicBool::new(false));
        self.read_panel = Some(ReadPanel {
            id,
            title: if commitments {
                "Private commitment references"
            } else {
                "Oracle carry — selected series/source"
            }
            .into(),
            content: "Loading…".into(),
            issue: None,
            selected: 0,
            scroll: 0,
            operations: Vec::new(),
            stop: stop.clone(),
        });
        let config = self.onchain_config.clone();
        let owner = self.wallet.pubkey.clone();
        let market = self.selected_id();
        let expiry = self
            .selected_chart_expiry()
            .map(|e| e.id.clone())
            .unwrap_or_default();
        let source = self.selected_oracle_node().and_then(|node| {
            current_spread_oracle_live(self).and_then(|live| {
                live.observations
                    .iter()
                    .find(|observation| spread_oracle_observation_matches_node(observation, node))
                    .map(|o| o.source_id_hex.clone())
            })
        });
        let tx = tx.clone();
        thread::spawn(move || {
            let result = (|| -> Result<(String, Vec<String>), CliError> {
                if commitments {
                    let owner = owner.ok_or_else(|| {
                        CliError::new("Attach a wallet to view its saved commitment references.")
                    })?;
                    let value = crate::oracle_commitments::run(
                        &crate::oracle_commitments::Command::List { owner },
                    )?;
                    Ok((
                        serde_json::to_string_pretty(&value).unwrap_or_default(),
                        Vec::new(),
                    ))
                } else {
                    let backend = BackendClient::new(config.backend_url.clone())?;
                    let value = crate::oracle_carry::read(
                        &config,
                        &backend,
                        &crate::oracle_carry::Args {
                            market,
                            expiry,
                            source,
                        },
                    )?;
                    Ok((crate::oracle_carry::render(&value), Vec::new()))
                }
            })()
            .map_err(|e| e.to_string());
            if !stop.load(Ordering::Relaxed) {
                let _ = tx.send(LabFetchResult::ReadPanel { id, result });
            }
        });
    }
    pub(super) fn open_operations_panel(&mut self, tx: &Sender<LabFetchResult>) {
        self.read_panel_request = self.read_panel_request.wrapping_add(1);
        let id = self.read_panel_request;
        let stop = Arc::new(AtomicBool::new(false));
        self.read_panel = Some(ReadPanel {
            id,
            title: "Operations — Enter recovers selected status".into(),
            content: "Loading…".into(),
            issue: None,
            selected: 0,
            scroll: 0,
            operations: Vec::new(),
            stop: stop.clone(),
        });
        let owner = self.wallet.pubkey.clone();
        let tx = tx.clone();
        thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let result=(|| -> Result<(String,Vec<String>),CliError> {
                        // No configured-wallet access in the background; the owner is a captured public key.
                        let Some(owner)=owner.as_deref() else {return Ok(("Attach a wallet to inspect its operations.".into(),Vec::new()));};
                        let value=crate::operation_journal::list(Some(owner))?;
                        let rows=value["operations"].as_array().ok_or_else(||CliError::new("Operation inventory missing."))?;
                        let ids=rows.iter().filter_map(|r|r["operationId"].as_str().map(str::to_owned)).collect();
                        let mut content=rows.iter().map(|r|format!("{}  {}  {}\n{}",r["state"].as_str().unwrap_or("unknown"),r["channel"].as_str().unwrap_or(""),r["updatedAt"].as_str().unwrap_or(""),r["operationId"].as_str().unwrap_or(""))).collect::<Vec<_>>().join("\n\n");
                        if content.is_empty(){content="No readable operations recorded for this wallet.".into();}
                        if value["issues"].as_array().is_some_and(|items|!items.is_empty()){content.push_str("\nSome local records could not be read.");}
                        if value.pointer("/coverage/exhaustive")==Some(&Value::Bool(false)){content.push_str("\nLocal inventory limit reached; use an exact operation ID for older records.");}
                        Ok((content,ids))
                })().map_err(|e|e.to_string());
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                if tx.send(LabFetchResult::ReadPanel { id, result }).is_err() {
                    break;
                }
                for _ in 0..150 {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
            }
        });
    }
    pub(super) fn handle_read_panel_key(
        &mut self,
        key: &KeyEvent,
        backend_url: &str,
        tx: &Sender<LabFetchResult>,
    ) -> bool {
        if self.read_panel.is_none() {
            return false;
        }
        match key.code {
            KeyCode::Esc | KeyCode::F(8) => {
                self.read_panel = None;
            }
            KeyCode::Char('r') => self.open_operations_panel(tx),
            KeyCode::Char('c') => self.open_oracle_read_panel(false, tx),
            KeyCode::Char('k') => self.open_oracle_read_panel(true, tx),
            KeyCode::Up => {
                let p = self.read_panel.as_mut().unwrap();
                p.selected = p.selected.saturating_sub(1);
                p.scroll = p.selected.saturating_mul(3).min(u16::MAX as usize) as u16;
            }
            KeyCode::Down => {
                let p = self.read_panel.as_mut().unwrap();
                p.selected = (p.selected + 1).min(p.operations.len().saturating_sub(1));
                p.scroll = p.selected.saturating_mul(3).min(u16::MAX as usize) as u16;
            }
            KeyCode::PageUp => {
                let p = self.read_panel.as_mut().unwrap();
                p.scroll = p.scroll.saturating_sub(10);
            }
            KeyCode::PageDown => {
                let p = self.read_panel.as_mut().unwrap();
                p.scroll = p.scroll.saturating_add(10);
            }
            KeyCode::Enter => {
                let p = self.read_panel.as_mut().unwrap();
                if let Some(operation) = p.operations.get(p.selected).cloned() {
                    // Stop inventory refreshes and invalidate queued responses before showing a receipt.
                    p.stop.store(true, Ordering::Relaxed);
                    self.read_panel_request = self.read_panel_request.wrapping_add(1);
                    p.id = self.read_panel_request;
                    p.scroll = 0;
                    p.selected = 0;
                    let id = p.id;
                    let backend_url = backend_url.to_owned();
                    let tx = tx.clone();
                    thread::spawn(move || {
                        let result = BackendClient::new(backend_url)
                            .and_then(|b| crate::operation_journal::recover(&b, &operation))
                            .map(|v| {
                                (
                                    serde_json::to_string_pretty(&v).unwrap_or_default(),
                                    vec![operation],
                                )
                            })
                            .map_err(|e| e.to_string());
                        let _ = tx.send(LabFetchResult::ReadPanel { id, result });
                    });
                }
            }
            _ => {}
        }
        true
    }
}
pub(super) fn draw(frame: &mut Frame<'_>, cli: &Cli, root: Rect, app: &LabApp) {
    let Some(panel) = &app.read_panel else {
        return;
    };
    let area = Rect {
        x: root.x + 1,
        y: root.y + 1,
        width: root.width.saturating_sub(2),
        height: root.height.saturating_sub(2),
    };
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(panel.title.clone())
        .style(tui_alt_panel_style(cli));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let mut lines = vec![Line::from(format!(
        "Esc close | r operations | c carry | k commitments | ↑↓ select | PgUp/PgDn scroll | Selected {} | Enter status only",
        panel.selected + 1
    ))];
    if let Some(issue) = &panel.issue {
        lines.push(Line::from(Span::styled(
            format!("Refresh unavailable — retained last view: {issue}"),
            style(cli, Color::Yellow),
        )));
    }
    lines.extend(
        panel
            .content
            .lines()
            .skip(panel.scroll as usize)
            .map(|s| Line::from(crate::backend::terminal_safe_text(s))),
    );
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}
