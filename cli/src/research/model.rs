use ratatui::layout::Rect;
use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Markets,
    Connect,
    Activity,
    Evidence,
    Rewards,
}
impl View {
    pub const ALL: [Self; 5] = [
        Self::Markets,
        Self::Connect,
        Self::Activity,
        Self::Evidence,
        Self::Rewards,
    ];
    /// Tab text for narrow terminals.
    pub fn short_label(self) -> &'static str {
        match self {
            Self::Markets => "Mkt",
            Self::Connect => "Conn",
            Self::Activity => "Act",
            Self::Evidence => "Evid",
            Self::Rewards => "Rew",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Markets => "Markets",
            Self::Connect => "Connect",
            Self::Activity => "Activity",
            Self::Evidence => "Evidence",
            Self::Rewards => "Rewards",
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub harness: String,
    pub payout_authority: String,
    pub revoked_at: Option<u64>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Pairing {
    pub id: String,
    pub code: String,
    pub name: String,
    pub harness: String,
    pub status: String,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Work {
    #[serde(default)]
    pub id: String,
    pub title: String,
    pub brief: String,
    #[serde(default)]
    pub objective: Value,
    #[serde(default)]
    pub guidance: Value,
    #[serde(default)]
    pub availability: String,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Directive {
    pub id: String,
    pub market_ids: Vec<String>,
    pub min_active_sources: u64,
    pub min_distinct_hosts: u64,
    pub max_observation_age_ms: u64,
    pub enabled: bool,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub actor: String,
    pub kind: String,
    pub message: String,
    pub reference_url: Option<String>,
    pub created_at: u64,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Detail {
    pub agent: Agent,
    pub events: Vec<Event>,
    pub candidates: Option<Vec<Value>>,
    pub submissions: Vec<Value>,
    pub validation_error: Option<String>,
    pub complete: bool,
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub payout_authority: String,
}
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Snapshot {
    pub agents: Vec<Agent>,
    pub pairings: Vec<Pairing>,
    pub work: Vec<Work>,
    pub settings: Settings,
    pub selected: Option<Detail>,
    #[serde(default)]
    pub directives: Vec<Directive>,
    #[serde(default)]
    pub discovery: Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    View(View),
    Agent(String),
    Harness(usize),
    Name,
    Payout,
    ActivitySection(usize),
    NewDirective,
    EditDirective(String),
    ToggleDirective(String),
    Discover,
    Refine(String),
    ShowWork(usize),
    Approve(String),
    Reject(String),
    Revoke(String),
    ConfirmRevoke(String),
    CopyCommand,
    CopyMcp,
    Source(usize),
    FormFocus(usize),
    Save,
    Cancel,
    Refresh,
    Quit,
}
#[derive(Clone, Debug)]
pub struct Hit {
    pub area: Rect,
    pub action: Action,
}
#[derive(Clone, Debug)]
pub enum Modal {
    Payout(String),
    Name(String),
    Directive {
        id: Option<String>,
        markets: String,
        sources: String,
        hosts: String,
        enabled: bool,
        max_age: u64,
    },
    Refine {
        work_id: String,
        instruction: String,
    },
    Assignment(Work),
    Revoke(Agent),
}
#[derive(Clone, Debug)]
pub struct Post {
    pub path: &'static str,
    pub body: Value,
}

pub struct App {
    pub view: View,
    pub snapshot: Snapshot,
    pub selected_agent: Option<String>,
    pub source: usize,
    pub scroll: u16,
    pub agent_scroll: usize,
    pub focus: usize,
    pub hits: Vec<Hit>,
    pub sidebar: Rect,
    pub modal: Option<Modal>,
    pub form_focus: usize,
    pub harness: usize,
    pub activity_section: usize,
    pub name: String,
    pub status: String,
    pub busy: bool,
    pub mutation_inflight: bool,
    pub pending: Option<Post>,
    pub quit: bool,
    pub refresh: bool,
    pub pointer: Option<(u16, u16)>,
    pub node_path: String,
    pub cli_path: String,
    pub base_url: String,
    pub markets: crate::research::markets::Markets,
    /// Why the oracle research workspace could not start, if it could not.
    pub research_error: Option<String>,
}
impl App {
    pub const HARNESSES: [&'static str; 5] = [
        "Claude Code",
        "Codex",
        "DeepSeek",
        "Kimi Code",
        "Gemini CLI",
    ];
    pub fn new(node_path: String, cli_path: String, base_url: String) -> Self {
        Self {
            view: View::Markets,
            snapshot: Snapshot::default(),
            selected_agent: None,
            source: 0,
            scroll: 0,
            agent_scroll: 0,
            focus: 0,
            hits: vec![],
            sidebar: Rect::default(),
            modal: None,
            form_focus: 0,
            harness: 0,
            activity_section: 0,
            name: "researcher".into(),
            status: String::new(),
            busy: false,
            mutation_inflight: false,
            pending: None,
            quit: false,
            refresh: true,
            pointer: None,
            node_path,
            cli_path,
            base_url,
            markets: Default::default(),
            research_error: None,
        }
    }
    pub fn update(&mut self, mut snapshot: Snapshot) {
        if self
            .selected_agent
            .as_ref()
            .is_none_or(|id| !snapshot.agents.iter().any(|a| &a.id == id))
        {
            self.selected_agent = snapshot.agents.first().map(|a| a.id.clone());
        }
        if snapshot
            .selected
            .as_ref()
            .is_some_and(|d| Some(&d.agent.id) != self.selected_agent.as_ref())
        {
            snapshot.selected = None;
            self.refresh = true;
        }
        self.snapshot = snapshot;
    }
    pub fn selected(&self) -> Option<&Detail> {
        self.snapshot
            .selected
            .as_ref()
            .filter(|d| Some(&d.agent.id) == self.selected_agent.as_ref())
    }
    pub fn sources(&self) -> Vec<Value> {
        let Some(detail) = self.selected() else {
            return vec![];
        };
        if let Some(candidates) = &detail.candidates {
            return candidates.clone();
        }
        let mut found = vec![];
        for item in &detail.submissions {
            if let Some(candidate) = item.get("candidate") {
                if candidate["agentId"].as_str() == Some(&detail.agent.id)
                    && !found
                        .iter()
                        .any(|old: &Value| old["contentHash"] == candidate["contentHash"])
                {
                    found.push(candidate.clone());
                }
            }
        }
        found
    }
    pub fn activate(&mut self, action: Action) {
        if self.mutation_inflight
            && !matches!(
                action,
                Action::View(_)
                    | Action::Agent(_)
                    | Action::Source(_)
                    | Action::Refresh
                    | Action::Quit
                    | Action::CopyCommand
                    | Action::CopyMcp
            )
        {
            self.status = "Saving…".into();
            return;
        }
        match action {
            Action::View(view) => {
                self.view = view;
                self.scroll = 0;
                self.focus = 0;
            }
            Action::Agent(id) => {
                self.selected_agent = Some(id);
                self.source = 0;
                self.scroll = 0;
                self.refresh = true;
                self.activity_section = 1;
            }
            Action::Harness(index) => self.harness = index.min(Self::HARNESSES.len() - 1),
            Action::Name => {
                self.modal = Some(Modal::Name(self.name.clone()));
                self.form_focus = 0;
            }
            Action::Payout => {
                self.modal = Some(Modal::Payout(
                    self.snapshot.settings.payout_authority.clone(),
                ));
                self.form_focus = 0;
            }
            Action::ActivitySection(section) => {
                self.activity_section = section;
                self.scroll = 0;
            }
            Action::NewDirective => {
                self.modal = Some(Modal::Directive {
                    id: None,
                    markets: String::new(),
                    sources: "3".into(),
                    hosts: "2".into(),
                    enabled: true,
                    max_age: 600000,
                });
                self.form_focus = 0;
            }
            Action::EditDirective(id) => {
                if let Some(directive) = self.snapshot.directives.iter().find(|d| d.id == id) {
                    self.modal = Some(Modal::Directive {
                        id: Some(id),
                        markets: directive.market_ids.join(", "),
                        sources: directive.min_active_sources.to_string(),
                        hosts: directive.min_distinct_hosts.to_string(),
                        enabled: directive.enabled,
                        max_age: directive.max_observation_age_ms,
                    });
                    self.form_focus = 0;
                }
            }
            Action::ToggleDirective(id) => {
                if let Some(d) = self
                    .snapshot
                    .directives
                    .iter()
                    .find(|d| d.id == id)
                    .cloned()
                {
                    self.post("/v1/owner/directives", serde_json::json!({"id":d.id,"marketIds":d.market_ids,"minActiveSources":d.min_active_sources,
                      "minDistinctHosts":d.min_distinct_hosts,"maxObservationAgeMs":d.max_observation_age_ms,"enabled":!d.enabled}));
                }
            }
            Action::Discover => self.post("/v1/owner/discovery/refresh", serde_json::json!({})),
            Action::Refine(id) => {
                if let Some(work) = self.snapshot.work.iter().find(|w| w.id == id) {
                    self.modal = Some(Modal::Refine {
                        work_id: id,
                        instruction: field(&work.guidance, "instruction"),
                    });
                    self.form_focus = 0;
                }
            }
            Action::ShowWork(index) => {
                self.modal = self
                    .snapshot
                    .work
                    .get(index)
                    .cloned()
                    .map(Modal::Assignment);
                self.form_focus = 0;
                self.scroll = 0;
            }
            Action::Source(index) => {
                self.source = index;
                self.scroll = 0;
            }
            Action::Approve(id) => self.post(
                "/v1/owner/pairings/approve",
                serde_json::json!({"pairingId":id}),
            ),
            Action::Reject(id) => self.post(
                "/v1/owner/pairings/reject",
                serde_json::json!({"pairingId":id}),
            ),
            Action::Revoke(id) => {
                self.modal = self
                    .snapshot
                    .agents
                    .iter()
                    .find(|a| a.id == id)
                    .cloned()
                    .map(Modal::Revoke);
                self.form_focus = 0;
            }
            Action::ConfirmRevoke(id) => {
                self.modal = None;
                self.post("/v1/owner/agents/revoke", serde_json::json!({"agentId":id}));
            }
            Action::FormFocus(index) => self.form_focus = index,
            Action::Cancel => self.modal = None,
            Action::Save => self.save(),
            Action::Refresh => self.refresh = true,
            Action::Quit => self.quit = true,
            Action::CopyCommand | Action::CopyMcp => {}
        }
    }
    pub fn post(&mut self, path: &'static str, body: Value) {
        if self.pending.is_some() || self.mutation_inflight {
            self.status = "Please wait for the current action.".into();
            return;
        }
        self.pending = Some(Post { path, body });
        self.status.clear();
    }
    fn save(&mut self) {
        match self.modal.clone() {
            Some(Modal::Name(value)) => {
                if value.is_empty()
                    || value.len() > 40
                    || !value.chars().all(|c| {
                        c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_'
                    })
                    || !value
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_alphanumeric())
                {
                    self.status = "Use a lowercase agent name, up to 40 characters.".into();
                    return;
                }
                self.name = value;
                self.modal = None;
            }
            Some(Modal::Payout(value)) => {
                if value.trim().is_empty() {
                    self.status = "Enter your public Solana reward address.".into();
                    return;
                }
                self.post(
                    "/v1/owner/settings",
                    serde_json::json!({"payoutAuthority":value.trim()}),
                );
            }
            Some(Modal::Directive {
                id,
                markets,
                sources,
                hosts,
                enabled,
                max_age,
            }) => {
                let (Ok(sources), Ok(hosts)) =
                    (sources.trim().parse::<u64>(), hosts.trim().parse::<u64>())
                else {
                    self.status = "Enter whole-number coverage targets.".into();
                    return;
                };
                let markets: Vec<_> = markets
                    .split(|c: char| c == ',' || c.is_whitespace())
                    .filter(|s| !s.is_empty())
                    .collect();
                let mut body = serde_json::json!({"marketIds":markets,"minActiveSources":sources,"minDistinctHosts":hosts,"maxObservationAgeMs":max_age,"enabled":enabled});
                if let Some(id) = id {
                    body["id"] = Value::String(id);
                }
                self.post("/v1/owner/directives", body);
            }
            Some(Modal::Refine {
                work_id,
                instruction,
            }) => self.post(
                "/v1/owner/work/refine",
                serde_json::json!({"workId":work_id,"instruction":instruction.trim()}),
            ),
            _ => {}
        }
    }
    pub fn edit(&mut self, value: &str, backspace: bool) {
        if self.mutation_inflight {
            return;
        }
        let (target, limit, multiline) = match &mut self.modal {
            Some(Modal::Payout(text)) | Some(Modal::Name(text)) if self.form_focus == 0 => {
                (text, 80, false)
            }
            Some(Modal::Directive { markets, .. }) if self.form_focus == 0 => (markets, 512, false),
            Some(Modal::Directive { sources, .. }) if self.form_focus == 1 => (sources, 3, false),
            Some(Modal::Directive { hosts, .. }) if self.form_focus == 2 => (hosts, 3, false),
            Some(Modal::Refine { instruction, .. }) if self.form_focus == 0 => {
                (instruction, 8000, true)
            }
            _ => return,
        };
        if backspace {
            target.pop();
            return;
        }
        for c in value.chars() {
            if target.len() + c.len_utf8() > limit {
                break;
            }
            if !c.is_control() || c == '\n' && multiline {
                target.push(c);
            }
        }
    }
    pub fn hit(&self, x: u16, y: u16) -> Option<Action> {
        self.hits
            .iter()
            .rev()
            .find(|hit| hit.area.contains((x, y).into()))
            .map(|hit| hit.action.clone())
    }
    pub fn form_fields(&self) -> usize {
        if matches!(self.modal, Some(Modal::Directive { .. })) {
            3
        } else {
            1
        }
    }
    pub fn command(&self, mcp: bool) -> String {
        let args = if mcp {
            vec![
                self.cli_path.clone(),
                "mcp".into(),
                "--agent".into(),
                self.name.clone(),
            ]
        } else {
            vec![
                self.cli_path.clone(),
                "agent".into(),
                "login".into(),
                "--name".into(),
                self.name.clone(),
            ]
        };
        let mut args = args;
        args.extend([
            "--harness".into(),
            Self::HARNESSES[self.harness].into(),
            "--url".into(),
            self.base_url.clone(),
        ]);
        if mcp {
            if self.harness == 1 {
                return format!(
                    "[mcp_servers.petri_research]\ncommand = {}\nargs = {}",
                    serde_json::to_string(&self.node_path).unwrap_or_default(),
                    serde_json::to_string(&args).unwrap_or_default()
                );
            }
            return serde_json::to_string_pretty(&serde_json::json!({"mcpServers":{"petri-research":{"command":self.node_path,"args":args}}})).unwrap_or_default();
        }
        #[cfg(not(windows))]
        let quote = |value: &str| format!("'{}'", value.replace('\'', "'\"'\"'"));
        #[cfg(windows)]
        let quote = |value: &str| format!("'{}'", value.replace('\'', "''"));
        format!(
            "{}{} {}",
            if cfg!(windows) { "& " } else { "" },
            quote(&self.node_path),
            args.iter().map(|s| quote(s)).collect::<Vec<_>>().join(" ")
        )
    }
}

pub fn safe(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_control() && c != '\n'
                || ('\u{202a}'..='\u{202e}').contains(&c)
                || ('\u{2066}'..='\u{2069}').contains(&c)
            {
                ' '
            } else {
                c
            }
        })
        .collect()
}
pub fn field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(safe)
        .unwrap_or_default()
}
pub fn usdc(value: &Value) -> Option<String> {
    let amount = value.as_str()?.parse::<u64>().ok()?;
    let fractional = format!("{:06}", amount % 1_000_000);
    Some(format!(
        "{}{} USDC",
        amount / 1_000_000,
        if amount % 1_000_000 == 0 {
            String::new()
        } else {
            format!(".{}", fractional.trim_end_matches('0'))
        }
    ))
}
