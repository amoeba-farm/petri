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
    /// Funding what researchers found (render.rs, fund.rs).
    Fund,
}
impl View {
    pub const ALL: [Self; 6] = [
        Self::Markets,
        Self::Connect,
        Self::Activity,
        Self::Evidence,
        Self::Rewards,
        Self::Fund,
    ];
    /// Tab text for narrow terminals.
    pub fn short_label(self) -> &'static str {
        match self {
            Self::Markets => "Mkt",
            Self::Connect => "Conn",
            Self::Activity => "Act",
            Self::Evidence => "Evid",
            Self::Rewards => "Rew",
            Self::Fund => "Fund",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Markets => "Markets",
            Self::Connect => "Connect",
            Self::Activity => "Research",
            Self::Evidence => "Evidence",
            Self::Rewards => "Rewards",
            Self::Fund => "Fund",
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
    #[serde(default)]
    pub objective: Value,
    #[serde(default)]
    pub guidance: Value,
    #[serde(default)]
    pub availability: String,
    /// The owner's "Research this": agents see it first.
    #[serde(default)]
    pub focused: bool,
    /// The agents researching it right now (their ids).
    #[serde(default)]
    pub researchers: Vec<String>,
}
/// What an agent is researching right now: its newest open run, while it
/// still checks in (the workspace's `activity`).
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    pub agent_id: String,
    pub work_id: String,
    #[serde(default)]
    pub run_id: Option<String>,
    /// The work's own title, for work no longer listed.
    #[serde(default)]
    pub title: String,
    pub last_message: Option<String>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    #[serde(default)]
    pub sequence: u64,
    /// The run it belongs to (Detail::runs).
    #[serde(default)]
    pub run_id: Option<String>,
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
    /// Its runs, newest first.
    #[serde(default)]
    pub runs: Vec<Run>,
    pub candidates: Option<Vec<Value>>,
    pub submissions: Vec<Value>,
    pub validation_error: Option<String>,
}
/// One research run of an agent: the work it took, as it was then.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub id: String,
    pub work_id: String,
    #[serde(default)]
    pub title: String,
    /// The work item when the run started (its objective).
    #[serde(default)]
    pub research_context: Value,
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
    pub activity: Vec<Activity>,
    #[serde(default)]
    pub discovery: Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    View(View),
    /// A market in the Markets view's list, by its place there.
    Market(usize),
    /// Closes the Markets view's ticket, as Esc does (a click outside it).
    CloseTicket,
    /// A find in the Fund view's list, by its place there.
    ShowFind(usize),
    /// Add funds or Withdraw in the Fund view.
    FundTicket(crate::research::fund::Kind),
    /// Closes the Fund view's ticket (a click outside it).
    CloseFundTicket,
    /// Sponsor the find with this content hash (from its pop-up).
    Sponsor(String),
    Agent(String),
    /// "+ New agent": Connect, set up afresh.
    NewAgent,
    Harness(usize),
    Name,
    Payout,
    /// Research's navigator goes to a product, month or category (a row or
    /// a crumb); it stops following the agent.
    Nav(crate::research::nav::Path),
    /// A task in the shown category (its work id).
    Task(String),
    /// The navigator follows the selected agent's work again.
    Follow,
    /// Scrolls the navigator's list to this row.
    Scroll(u16),
    /// The agent's feed shows its newest again.
    Newest,
    /// "Research this" (true) or "Stop researching" for an objective.
    Focus(String, bool),
    Refine(String),
    Approve(String),
    Reject(String),
    Revoke(String),
    ConfirmRevoke(String),
    /// Copies the message to paste into the agent's terminal (App::agent_setup).
    CopyCommand,
    /// Copies the message that gets a connected agent researching again
    /// (App::start_message).
    CopyStart(String),
    /// Opens the harness choice (Modal::Harness).
    ChooseHarness,
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
    /// Which agent harness the setup is for: one of App::HARNESSES.
    Harness,
    /// A find in the Fund view: what it is, who found it.
    Find(Box<crate::research::fund::Find>),
    Refine {
        work_id: String,
        instruction: String,
    },
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
    /// Connect shows a fresh setup for another agent ("+ New agent").
    pub new_agent: bool,
    /// Research's navigator for each agent (by id; "" with none selected).
    pub navs: std::collections::HashMap<String, crate::research::nav::Nav>,
    /// Where the selected agent's feed was drawn, and the furthest it can
    /// scroll (its top row with the newest at the bottom).
    pub feed_area: Rect,
    pub feed_max: usize,
    /// Each agent's last read (its events, finds and submissions), by id.
    pub details: std::collections::HashMap<String, Detail>,
    /// The workspace has been read once: until then the research views
    /// draw nothing rather than an empty workspace.
    pub loaded: bool,
    pub source: usize,
    pub scroll: u16,
    pub agent_scroll: usize,
    pub focus: usize,
    pub hits: Vec<Hit>,
    pub sidebar: Rect,
    pub modal: Option<Modal>,
    pub form_focus: usize,
    pub harness: usize,
    pub name: String,
    pub status: String,
    pub busy: bool,
    pub mutation_inflight: bool,
    pub pending: Option<Post>,
    pub quit: bool,
    pub refresh: bool,
    pub pointer: Option<(u16, u16)>,
    /// While a frame draws: whether the controls now registered are the open
    /// modal's (render::modal_view), the only ones the pointer highlights then.
    pub drawing_modal: bool,
    pub node_path: String,
    pub cli_path: String,
    pub base_url: String,
    pub markets: crate::research::markets::Markets,
    pub fund: crate::research::fund::Fund,
    pub rewards: crate::research::rewards::Rewards,
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
            new_agent: false,
            details: Default::default(),
            navs: Default::default(),
            feed_area: Rect::default(),
            feed_max: 0,
            loaded: false,
            source: 0,
            scroll: 0,
            agent_scroll: 0,
            focus: 0,
            hits: vec![],
            sidebar: Rect::default(),
            modal: None,
            form_focus: 0,
            harness: 0,
            name: "researcher".into(),
            status: String::new(),
            busy: false,
            mutation_inflight: false,
            pending: None,
            quit: false,
            refresh: true,
            pointer: None,
            drawing_modal: false,
            node_path,
            cli_path,
            base_url,
            markets: Default::default(),
            fund: Default::default(),
            rewards: Default::default(),
            research_error: None,
        }
    }
    pub fn update(&mut self, mut snapshot: Snapshot) {
        // The agent whose request was just approved: show it.
        if let Some(added) = snapshot.agents.iter().find(|a| {
            a.revoked_at.is_none()
                && !self.snapshot.agents.iter().any(|old| old.id == a.id)
                && self.snapshot.pairings.iter().any(|p| p.name == a.name)
        }) {
            self.selected_agent = Some(added.id.clone());
            self.new_agent = false;
            self.refresh = true;
        }
        if self
            .selected_agent
            .as_ref()
            .is_none_or(|id| !snapshot.agents.iter().any(|a| &a.id == id))
        {
            self.selected_agent = snapshot.agents.first().map(|a| a.id.clone());
        }
        // Each agent's last read stays, so switching to it draws at once.
        if let Some(detail) = snapshot.selected.take() {
            self.details.insert(detail.agent.id.clone(), detail);
        }
        self.details
            .retain(|id, _| snapshot.agents.iter().any(|a| &a.id == id));
        if self
            .selected_agent
            .as_ref()
            .is_some_and(|id| !self.details.contains_key(id))
        {
            self.refresh = true;
        }
        self.snapshot = snapshot;
        self.loaded = true;
        self.follow();
    }
    /// Research's navigator for the selected agent.
    pub fn nav(&self) -> crate::research::nav::Nav {
        self.navs
            .get(self.selected_agent.as_deref().unwrap_or_default())
            .cloned()
            .unwrap_or_default()
    }
    pub fn nav_mut(&mut self) -> &mut crate::research::nav::Nav {
        let key = self.selected_agent.clone().unwrap_or_default();
        self.navs.entry(key).or_default()
    }
    /// While the navigator follows the selected agent, it shows the work
    /// the agent is on.
    pub fn follow(&mut self) {
        let Some(id) = self.selected_agent.clone() else {
            return;
        };
        let place = crate::research::nav::place_of(self, &id);
        let nav = self.nav_mut();
        if let (true, Some((path, work))) = (nav.follow, place)
            && (nav.path != path || nav.task.as_ref() != Some(&work))
        {
            nav.path = path;
            nav.task = Some(work);
            self.scroll = 0;
        }
    }
    pub fn selected(&self) -> Option<&Detail> {
        self.details.get(self.selected_agent.as_ref()?)
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
                    | Action::Market(_)
                    | Action::CloseTicket
                    | Action::ShowFind(_)
                    | Action::FundTicket(_)
                    | Action::CloseFundTicket
                    | Action::Sponsor(_)
                    | Action::Agent(_)
                    | Action::Source(_)
                    | Action::Refresh
                    | Action::Quit
                    | Action::CopyCommand
                    | Action::CopyStart(_)
                    | Action::NewAgent
                    | Action::Nav(_)
                    | Action::Task(_)
                    | Action::Follow
                    | Action::Scroll(_)
                    | Action::Newest
                    | Action::ChooseHarness
            )
        {
            return;
        }
        match action {
            Action::View(view) => {
                self.view = view;
                self.scroll = 0;
                self.focus = 0;
            }
            Action::Market(index) => self.markets.show(index),
            Action::CloseTicket => self.markets.ticket = None,
            Action::ShowFind(index) => {
                self.form_focus = 0;
                self.modal = self
                    .fund
                    .snap
                    .finds
                    .get(index)
                    .cloned()
                    .map(|find| Modal::Find(Box::new(find)));
            }
            Action::FundTicket(kind) => self.fund.open(kind),
            Action::CloseFundTicket => self.fund.ticket = None,
            Action::Sponsor(hash) => {
                self.modal = None;
                self.fund.open(crate::research::fund::Kind::Sponsor(hash));
            }
            Action::Agent(id) => {
                self.selected_agent = Some(id);
                self.new_agent = false;
                self.source = 0;
                self.scroll = 0;
                self.refresh = true;
                // Its page follows its work.
                let nav = self.nav_mut();
                nav.follow = true;
                nav.feed_top = None;
                self.follow();
            }
            Action::NewAgent => {
                self.new_agent = true;
                self.view = View::Connect;
                self.scroll = 0;
                self.focus = 0;
            }
            Action::Harness(index) => {
                self.harness = index.min(Self::HARNESSES.len() - 1);
                if matches!(self.modal, Some(Modal::Harness)) {
                    self.modal = None;
                }
            }
            Action::ChooseHarness => self.modal = Some(Modal::Harness),
            Action::Name => {
                self.modal = Some(Modal::Name(self.name.clone()));
                self.form_focus = 0;
            }
            Action::Payout => {
                // Connecting a wallet: this Petri's own wallet unless another
                // is connected already.
                let connected = self.snapshot.settings.payout_authority.clone();
                self.modal = Some(Modal::Payout(if connected.is_empty() {
                    self.markets.snap.owner.clone().unwrap_or_default()
                } else {
                    connected
                }));
                self.form_focus = 0;
            }
            Action::Nav(path) => {
                let nav = self.nav_mut();
                nav.follow = false;
                nav.path = path;
                self.scroll = 0;
            }
            Action::Task(id) => {
                let nav = self.nav_mut();
                nav.follow = false;
                nav.task = Some(id);
            }
            Action::Follow => {
                self.nav_mut().follow = true;
                self.follow();
            }
            Action::Scroll(row) => self.scroll = row,
            Action::Newest => self.nav_mut().feed_top = None,
            Action::Focus(id, focused) => self.post(
                "/v1/owner/work/focus",
                serde_json::json!({"workId": id, "focused": focused}),
            ),
            Action::Refine(id) => {
                if let Some(work) = self.snapshot.work.iter().find(|w| w.id == id) {
                    self.modal = Some(Modal::Refine {
                        work_id: id,
                        instruction: field(&work.guidance, "instruction"),
                    });
                    self.form_focus = 0;
                }
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
            Action::CopyCommand | Action::CopyStart(_) => {}
        }
    }
    /// Queues one owner action. While another is pending or in flight the
    /// new one is quietly ignored: no loading text.
    pub fn post(&mut self, path: &'static str, body: Value) {
        if self.pending.is_some() || self.mutation_inflight {
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
                    self.status = "Enter your wallet's public address.".into();
                    return;
                }
                self.post(
                    "/v1/owner/settings",
                    serde_json::json!({"payoutAuthority":value.trim()}),
                );
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
    /// Whether a find's pop-up offers Sponsor it: not on the oracle yet, and
    /// a wallet to sponsor with.
    pub fn find_sponsorable(&self, find: &crate::research::fund::Find) -> bool {
        find.on_oracle.is_none() && self.fund.snap.owner.is_some()
    }

    pub fn form_fields(&self) -> usize {
        1
    }
    /// What to paste into the agent's terminal: the agent connects itself
    /// with the command in it, the person approves its code in Connect, and
    /// it researches from then on.
    pub fn agent_setup(&self) -> String {
        format!(
            "Connect to my Petri oracle research workspace as the agent \"{name}\" and research for it. \
             Run this command, tell me the pairing code it shows, and once I approve it \
             in Petri, run it again and follow what it prints. Keep researching until it says \
             nothing needs research:\n{command}",
            name = self.name,
            command = self.command(),
        )
    }
    /// The agent's login command (in agent_setup).
    pub fn command(&self) -> String {
        self.agent_command(vec![
            "agent".into(),
            "login".into(),
            "--name".into(),
            self.name.clone(),
            "--harness".into(),
            Self::HARNESSES[self.harness].into(),
            "--url".into(),
            self.base_url.clone(),
        ])
    }
    /// What gets a connected agent researching again, pasted into its
    /// terminal: `work next` and what it prints, until nothing is left.
    pub fn start_message(&self, agent: &Agent) -> String {
        format!(
            "Research for my Petri oracle workspace as the agent \"{name}\". Run this command \
             and follow what it prints. Keep researching until it says nothing needs research:\n{command}",
            name = agent.name,
            command = self.agent_command(vec![
                "work".into(),
                "next".into(),
                "--agent".into(),
                agent.name.clone(),
            ]),
        )
    }
    /// What agent `id` is researching right now, if anything.
    pub fn activity_of(&self, id: &str) -> Option<&Activity> {
        self.snapshot.activity.iter().find(|a| a.agent_id == id)
    }
    /// The agent's research command line with these arguments.
    fn agent_command(&self, rest: Vec<String>) -> String {
        let mut args = vec![self.cli_path.clone()];
        args.extend(rest);
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
