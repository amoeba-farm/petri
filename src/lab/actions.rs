//! Shared SDK actions, with asynchronous preparation and explicit exact-plan approval.
use super::*;
use crate::participation::{ACTIONS, Action};
use crate::portable_operation::{self, Family};

enum Mode {
    Choose,
    Edit,
    Review(Value),
    Busy,
    Result(String),
}
pub(super) struct ActionPanel {
    mode: Mode,
    selected: usize,
    field: usize,
    values: Vec<String>,
    owner: String,
    market: String,
    expiry: String,
    approve: bool,
    scroll: u16,
}
impl LabApp {
    pub(super) fn open_action_panel(&mut self) {
        self.action_panel_request = self.action_panel_request.wrapping_add(1);
        let market = self.selected_id();
        let expiry = self
            .selected_chart_expiry()
            .map(|e| e.id.clone())
            .unwrap_or_default();
        self.action_panel = Some(ActionPanel {
            mode: Mode::Choose,
            selected: 0,
            field: 0,
            values: Vec::new(),
            owner: self.wallet.pubkey.clone().unwrap_or_default(),
            market,
            expiry,
            approve: false,
            scroll: 0,
        });
    }
    pub(super) fn review_action_payload(&mut self, value: Value) {
        self.open_action_panel();
        self.action_panel.as_mut().unwrap().mode = Mode::Review(value);
    }
    pub(super) fn open_specific_action(&mut self, action: Action) {
        let source = self.selected_oracle_node().and_then(|node| {
            current_spread_oracle_live(self).and_then(|live| {
                live.observations
                    .iter()
                    .find(|observation| spread_oracle_observation_matches_node(observation, node))
                    .map(|o| o.source_id_hex.clone())
            })
        });
        self.open_action_panel();
        let panel = self.action_panel.as_mut().unwrap();
        panel.selected = ACTIONS.iter().position(|a| *a == action).unwrap_or(0);
        panel.values = if matches!(action.family(), Family::Oracle) {
            vec![panel.market.clone(), panel.expiry.clone()]
        } else {
            Vec::new()
        };
        panel
            .values
            .extend(action.fields().iter().map(|_| String::new()));
        if action != Action::ProposeSource
            && let Some(source) = source
            && let Some(index) = action
                .fields()
                .iter()
                .position(|(key, _, _)| *key == "sourceId")
        {
            panel.values[index + 2] = source;
        }
        panel.mode = Mode::Edit;
    }
    pub(super) fn open_oracle_reward_action(&mut self, claim: &SpreadOracleRewardClaim) -> bool {
        // Translate read-projection labels only to current USDC reward kinds.
        // Challenge rewards have no equivalent here and must not be guessed.
        let reward_kind = match claim.kind.as_str() {
            "proposer" | "source_proposer" => "proposer",
            "support" | "source_support" => "support",
            "opening" => "opening",
            "update" | "game_update" => "update",
            _ => {
                self.status =
                    "This reward type is not supported by the current claim action.".into();
                return false;
            }
        };
        let is_id =
            |value: &&str| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit());
        let Some(source_id) = claim.source_id_hex.as_deref().filter(is_id) else {
            self.status =
                "The selected reward has no valid source ID. Refresh rewards before claiming."
                    .into();
            return false;
        };
        let claim_id = if reward_kind == "update" {
            let Some(id) = claim.claim_id_hex.as_deref().filter(is_id) else {
                self.status = "The selected update reward has no valid claim ID. Refresh rewards before claiming.".into();
                return false;
            };
            id
        } else {
            ""
        };
        self.open_specific_action(Action::ClaimReward);
        let panel = self.action_panel.as_mut().unwrap();
        for ((name, _, _), value) in Action::ClaimReward
            .fields()
            .iter()
            .zip(panel.values.iter_mut().skip(2))
        {
            *value = match *name {
                "rewardKind" => reward_kind,
                "sourceId" => source_id,
                "claimId" => claim_id,
                _ => continue,
            }
            .to_string();
        }
        true
    }
    pub(super) fn handle_action_panel_key(
        &mut self,
        key: &KeyEvent,
        tx: &Sender<LabFetchResult>,
    ) -> bool {
        let Some(panel) = self.action_panel.as_mut() else {
            return false;
        };
        if matches!(panel.mode, Mode::Busy) {
            return true;
        }
        if matches!(key.code, KeyCode::Esc | KeyCode::F(9)) {
            self.action_panel = None;
            return true;
        }
        if key.code == KeyCode::PageUp {
            panel.scroll = panel.scroll.saturating_sub(8);
            return true;
        }
        if key.code == KeyCode::PageDown {
            panel.scroll = panel.scroll.saturating_add(8);
            return true;
        }
        match &panel.mode {
            Mode::Choose => match key.code {
                KeyCode::Up => panel.selected = panel.selected.saturating_sub(1),
                KeyCode::Down => panel.selected = (panel.selected + 1).min(ACTIONS.len() - 1),
                KeyCode::Enter => {
                    if panel.owner.is_empty() {
                        panel.mode =
                            Mode::Result("Attach a wallet before preparing an action.".into());
                        return true;
                    }
                    let action = ACTIONS[panel.selected];
                    panel.values = if matches!(action.family(), Family::Oracle) {
                        vec![panel.market.clone(), panel.expiry.clone()]
                    } else {
                        Vec::new()
                    };
                    panel
                        .values
                        .extend(action.fields().iter().map(|_| String::new()));
                    panel.field = 0;
                    panel.scroll = 0;
                    panel.mode = Mode::Edit;
                }
                _ => {}
            },
            Mode::Edit => {
                match key.code {
                    KeyCode::Tab | KeyCode::Down => {
                        panel.field = (panel.field + 1).min(panel.values.len())
                    }
                    KeyCode::BackTab | KeyCode::Up => panel.field = panel.field.saturating_sub(1),
                    KeyCode::Backspace => {
                        if let Some(input) = panel.values.get_mut(panel.field) {
                            input.pop();
                        }
                    }
                    KeyCode::Char(c)
                        if !c.is_control()
                            && !key
                                .modifiers
                                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        if let Some(input) = panel.values.get_mut(panel.field) {
                            if input.len() + c.len_utf8() <= 4096 {
                                input.push(c);
                            }
                        }
                    }
                    KeyCode::Enter if panel.field < panel.values.len() => panel.field += 1,
                    KeyCode::Enter => {
                        let action = ACTIONS[panel.selected];
                        let scoped = matches!(action.family(), Family::Oracle);
                        let offset = if scoped { 2 } else { 0 };
                        let fields = action
                            .fields()
                            .iter()
                            .zip(panel.values.iter().skip(offset))
                            .filter(|(_, v)| !v.is_empty())
                            .map(|((name, _, _), v)| format!("{name}={v}"))
                            .collect::<Vec<_>>();
                        let request = crate::participation::request(
                            action,
                            &panel.owner,
                            if scoped { Some(&panel.values[0]) } else { None },
                            if scoped { Some(&panel.values[1]) } else { None },
                            &fields,
                        );
                        match request {
                            Err(error) => panel.mode = Mode::Result(error.to_string()),
                            Ok(request) => {
                                panel.mode = Mode::Busy;
                                self.action_panel_request =
                                    self.action_panel_request.wrapping_add(1);
                                let id = self.action_panel_request;
                                let config = self.onchain_config.clone();
                                let tx = tx.clone();
                                thread::spawn(move || {
                                    let result = BackendClient::new(config.backend_url.clone())
                                        .and_then(|backend| {
                                            portable_operation::prepare(
                                                &config,
                                                &backend,
                                                action.family(),
                                                request,
                                            )
                                        })
                                        .map_err(|e| e.to_string());
                                    let _ = tx.send(LabFetchResult::ActionPanel {
                                        id,
                                        executed: false,
                                        result,
                                    });
                                });
                            }
                        }
                    }
                    _ => {}
                }
                panel.scroll = panel.field.saturating_sub(5).min(u16::MAX as usize) as u16;
            }
            Mode::Review(value) => {
                match key.code {
                    KeyCode::Left | KeyCode::Right | KeyCode::Tab => {
                        panel.approve = !panel.approve;
                        panel.scroll = 0;
                    }
                    KeyCode::Enter => {
                        if !panel.approve {
                            self.action_panel = None;
                            return true;
                        }
                        let operation = value
                            .pointer("/operation/operationId")
                            .and_then(Value::as_str)
                            .map(str::to_owned);
                        let owner = value.pointer("/operation/owner").and_then(Value::as_str);
                        if owner != Some(panel.owner.as_str())
                            || self.wallet.pubkey.as_deref() != Some(panel.owner.as_str())
                        {
                            panel.mode = Mode::Result(
                                "Attached wallet changed. Prepare a fresh review.".into(),
                            );
                            return true;
                        }
                        if let Some(operation) = operation {
                            panel.mode = Mode::Busy;
                            self.action_panel_request = self.action_panel_request.wrapping_add(1);
                            let id = self.action_panel_request;
                            let config = self.onchain_config.clone();
                            let tx = tx.clone();
                            thread::spawn(move || {
                                let result=BackendClient::new(config.backend_url.clone()).and_then(|backend|portable_operation::execute(&config,&backend,&operation,true)).map_err(|e|format!("{e}\nOperation: {operation}\nUse F8 to recover its status."));
                                let _ = tx.send(LabFetchResult::ActionPanel {
                                    id,
                                    executed: true,
                                    result,
                                });
                            });
                        }
                    }
                    _ => {}
                }
            }
            Mode::Result(_) => {
                if key.code == KeyCode::Enter {
                    panel.mode = Mode::Choose;
                    panel.scroll = 0;
                }
            }
            Mode::Busy => {}
        }
        true
    }
    pub(super) fn apply_action_panel(
        &mut self,
        id: u64,
        executed: bool,
        result: Result<Value, String>,
        backend_url: &str,
        tx: &Sender<LabFetchResult>,
    ) {
        if id != self.action_panel_request {
            return;
        }
        let Some(panel) = self.action_panel.as_mut() else {
            return;
        };
        panel.approve = false;
        panel.scroll = 0;
        panel.mode = match result {
            Ok(value) if !executed => Mode::Review(value),
            Ok(value) => Mode::Result(portable_operation::render(&value)),
            Err(error) => Mode::Result(error),
        };
        if executed {
            self.invalidate_market_caches();
            self.request_ledger(backend_url, tx, true);
            self.request_selected_detail(backend_url, tx, true);
        }
    }
}
pub(super) fn draw(frame: &mut Frame<'_>, cli: &Cli, root: Rect, app: &LabApp) {
    let Some(panel) = &app.action_panel else {
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
        .title("Wallet & Oracle actions")
        .style(tui_alt_panel_style(cli));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let content=match &panel.mode {
        Mode::Choose=>format!("Esc close | ↑↓ choose | Enter open\n\n{}",ACTIONS.iter().enumerate().map(|(i,a)|format!("{} {}",if i==panel.selected{"›"}else{" "},a.label())).collect::<Vec<_>>().join("\n")),
        Mode::Edit=>{
            let action=ACTIONS[panel.selected];let mut labels=Vec::new();
            if matches!(action.family(),Family::Oracle){labels.extend(["Product","Exact series"]);}
            labels.extend(action.fields().iter().map(|(_,label,_)|*label));
            format!("{} | Wallet: {}\nTab/Enter next | Esc cancel | PgUp/PgDn scroll\n\n{}\n{} Prepare review",action.label(),crate::backend::terminal_safe_text(&panel.owner),
                labels.iter().zip(&panel.values).enumerate().map(|(i,(label,value))|format!("{} {label}: {}",if i==panel.field{"›"}else{" "},crate::backend::terminal_safe_text(value))).collect::<Vec<_>>().join("\n"),if panel.field==panel.values.len(){"›"}else{" "})
        },
        Mode::Review(value)=>format!("{} | ←→ select | Enter confirm | PgUp/PgDn scroll\n\n{}",if panel.approve{"Cancel   [Approve exact action]"}else{"[Cancel]   Approve exact action"},portable_operation::render(value)),
        Mode::Busy=>"Checking the exact action… Wallet approval may be requested after confirmation.\nWait for the result; do not repeat this action.".into(),
        Mode::Result(result)=>format!("Esc close | Enter actions | PgUp/PgDn scroll\n\n{result}"),
    };
    // Preserve layout newlines while sanitizing controls within each line.
    // Editable values above are sanitized before they enter the layout.
    let text = content
        .lines()
        .map(|line| Line::from(crate::backend::terminal_safe_text(line)))
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .scroll((panel.scroll, 0)),
        inner,
    );
}
