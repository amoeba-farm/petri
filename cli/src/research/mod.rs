pub mod fund;
mod http;
mod input;
pub mod markets;
mod model;
mod nav;
pub mod phase;
mod render;
mod research_page;
pub mod rewards;
mod runtime;
#[cfg(test)]
mod tests;
mod updates;

use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use model::{Action, App, View};
use ratatui::{
    Terminal,
    backend::{CrosstermBackend, TestBackend},
};
use std::{
    io::{self, Write},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            DisableMouseCapture,
            DisableBracketedPaste,
            LeaveAlternateScreen
        );
    }
}
fn clipboard(value: &str) -> Result<(), String> {
    let mut command;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command = Command::new("powershell.exe");
        command.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$input | Set-Clipboard",
        ]);
        command.creation_flags(0x08000000);
    }
    #[cfg(target_os = "macos")]
    {
        command = Command::new("pbcopy");
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        command = Command::new("wl-copy");
    }
    let mut child = command
        .env_remove("PETRI_RESEARCH_OWNER_TOKEN")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Clipboard unavailable. Use the setup instructions in the README.")?;
    child
        .stdin
        .take()
        .ok_or("Clipboard unavailable.")?
        .write_all(value.as_bytes())
        .map_err(|_| "Could not copy setup.")?;
    if !child.wait().map_err(|_| "Could not copy setup.")?.success() {
        return Err("Could not copy setup.".into());
    }
    Ok(())
}
/// `petri research ...` against the futures API at `api` (candidates and
/// oracle reads go where Petri's markets come from).
pub fn command(args: &[String], api: &str) -> Result<(), crate::backend::CliError> {
    runtime::Runtime::load(api)
        .map_err(crate::backend::CliError::new)?
        .command(args)
}

fn research_session(api: &str) -> Result<(http::Api, String, String, String), String> {
    let runtime = runtime::Runtime::load(api)?;
    let connection = runtime.connect()?;
    let api = http::Api::new(connection.base_url.clone(), connection.owner_token)?;
    let (node, entry) = runtime.agent_command();
    Ok((api, node, entry, connection.base_url))
}

fn parse_view(view: &str) -> Result<View, String> {
    View::ALL
        .into_iter()
        .find(|v| v.label().eq_ignore_ascii_case(view))
        .ok_or_else(|| {
            format!("Unknown view {view}; use markets, connect, activity, evidence or rewards.")
        })
}

pub fn run(
    check_updates: bool,
    render_size: Option<&str>,
    view: Option<&str>,
    futures: crate::futures::settings::Settings,
) -> Result<(), String> {
    use std::io::IsTerminal;
    if render_size.is_none() && (!io::stdin().is_terminal() || !io::stdout().is_terminal()) {
        return Err(
            "Open petri tui in a terminal, or use petri research --help for commands.".into(),
        );
    }
    let view = view.map(parse_view).transpose()?;
    // Markets and Fund render without the research workspace. The TUI
    // itself opens at once and connects to the workspace in the background
    // (below): starting it can take seconds.
    let markets_render = render_size.is_some() && matches!(view, Some(View::Markets | View::Fund));
    let (api, mut app) = if markets_render || render_size.is_none() {
        (None, App::new(String::new(), String::new(), String::new()))
    } else {
        match research_session(&futures.api_url.0) {
            Ok((api, node, entry, base_url)) => (Some(api), App::new(node, entry, base_url)),
            Err(error) => {
                let mut app = App::new(String::new(), String::new(), String::new());
                app.research_error = Some(error);
                (None, app)
            }
        }
    };
    if let Some(view) = view {
        app.view = view;
    }
    if let Some(size) = render_size {
        let (width, height) = size.split_once('x').ok_or("Use --render 100x32.")?;
        let width = width
            .parse::<u16>()
            .map_err(|_| "Invalid width.")?
            .clamp(20, 240);
        let height = height
            .parse::<u16>()
            .map_err(|_| "Invalid height.")?
            .clamp(10, 100);
        if app.view == View::Markets {
            let ctx = crate::futures::ctx::Ctx::new(futures);
            app.markets.snap = markets::fetch(&ctx, None);
            app.markets.loaded = true;
        } else if app.view == View::Fund {
            let ctx = crate::futures::ctx::Ctx::new(futures);
            app.fund.take_snapshot(fund::fetch(&ctx));
        } else if let Some(api) = &api {
            // Research's products are the oracle's months (nav.rs).
            if app.view == View::Activity {
                let ctx = crate::futures::ctx::Ctx::new(futures);
                app.fund.take_snapshot(fund::fetch(&ctx));
            }
            app.update(api.snapshot(None)?);
        }
        let mut terminal =
            Terminal::new(TestBackend::new(width, height)).map_err(|e| e.to_string())?;
        terminal
            .draw(|frame| render::draw(frame, &mut app))
            .map_err(|e| e.to_string())?;
        for y in 0..height {
            let row = (0..width)
                .map(|x| terminal.backend().buffer()[(x, y)].symbol())
                .collect::<String>();
            println!("{}", row.trim_end());
        }
        return Ok(());
    }
    let (session_tx, session) = std::sync::mpsc::channel();
    let api_url = futures.api_url.0.clone();
    std::thread::spawn(move || {
        let _ = session_tx.send(research_session(&api_url));
    });
    app.fund.start(futures.clone());
    app.rewards.start(futures.clone());
    app.markets.start(futures);
    let (mut commands, mut replies) = (None, None);
    // Agents whose last read was asked for in the background, so switching
    // to one draws at once.
    let mut prefetched = std::collections::HashSet::new();
    enable_raw_mode().map_err(|e| e.to_string())?;
    let _guard = TerminalGuard;
    execute!(
        io::stdout(),
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )
    .map_err(|e| e.to_string())?;
    let mut terminal =
        Terminal::new(CrosstermBackend::new(io::stdout())).map_err(|e| e.to_string())?;
    let mut last_refresh = Instant::now() - Duration::from_secs(3);
    let mut fetch_error = false;
    let mut last_input = Instant::now();
    let mut updates = updates::Updates::new(check_updates);
    while !app.quit {
        if let Ok(result) = session.try_recv() {
            match result {
                Ok((api, node, entry, base_url)) => {
                    app.node_path = node;
                    app.cli_path = entry;
                    app.base_url = base_url;
                    let (c, r) = http::worker(api);
                    commands = Some(c);
                    replies = Some(r);
                }
                Err(error) => app.research_error = Some(error),
            }
        }
        app.markets.tick(app.view == View::Markets);
        app.fund.tick(app.view == View::Fund);
        app.rewards
            .tick(app.view == View::Rewards, &app.snapshot.agents);
        while let Some(Ok(reply)) = replies.as_ref().map(|r| r.try_recv()) {
            app.busy = false;
            match reply {
                http::Reply::Snapshot(Ok(snapshot)) => {
                    if fetch_error {
                        app.status.clear();
                        fetch_error = false;
                    }
                    app.update(snapshot);
                }
                http::Reply::Snapshot(Err(error)) => {
                    app.status = error;
                    fetch_error = true;
                }
                http::Reply::Post(Ok(())) => {
                    app.mutation_inflight = false;
                    app.modal = None;
                    app.status.clear();
                    app.refresh = true;
                }
                http::Reply::Post(Err(error)) => {
                    app.mutation_inflight = false;
                    app.status = error;
                    app.refresh = true;
                }
            }
        }
        match updates.tick(updates::safe_to_restart(&app, last_input.elapsed())) {
            Some(updates::Action::Restart) => break,
            Some(updates::Action::Error(error)) => app.status = model::safe(&error),
            None => {}
        }
        // While an update installs, input is set aside (below) and nothing
        // is said: no loading text.
        if let Some(commands) = commands
            .as_ref()
            .filter(|_| !app.busy && !updates.installing())
        {
            if let Some(post) = app.pending.take() {
                app.mutation_inflight = true;
                commands
                    .send(http::Command::Post(post))
                    .map_err(|_| "Workspace worker stopped.")?;
                app.busy = true;
            } else if app.refresh || last_refresh.elapsed() >= Duration::from_secs(3) {
                app.refresh = false;
                last_refresh = Instant::now();
                commands
                    .send(http::Command::Snapshot(app.selected_agent.clone()))
                    .map_err(|_| "Workspace worker stopped.")?;
                app.busy = true;
            } else if let Some(id) = app
                .snapshot
                .agents
                .iter()
                .map(|a| a.id.clone())
                .find(|id| !app.details.contains_key(id) && !prefetched.contains(id))
            {
                prefetched.insert(id.clone());
                commands
                    .send(http::Command::Snapshot(Some(id)))
                    .map_err(|_| "Workspace worker stopped.")?;
                app.busy = true;
            }
        }
        terminal
            .draw(|frame| render::draw(frame, &mut app))
            .map_err(|e| e.to_string())?;
        let frame = Duration::from_millis(100);
        if event::poll(frame).map_err(|e| e.to_string())? {
            if updates.installing() {
                let _ = event::read();
                continue;
            }
            last_input = Instant::now();
            if let Some(action) = input::handle(&mut app, event::read().map_err(|e| e.to_string())?)
            {
                match action {
                    Action::CopyCommand => {
                        app.status = match clipboard(&app.agent_setup()) {
                            Ok(()) => "Copied.".into(),
                            Err(error) => error,
                        };
                    }
                    Action::CopyStart(id) => {
                        if let Some(agent) = app.snapshot.agents.iter().find(|a| a.id == id) {
                            app.status = match clipboard(&app.start_message(agent)) {
                                Ok(()) => "Copied.".into(),
                                Err(error) => error,
                            };
                        }
                    }
                    other => app.activate(other),
                }
            }
        }
    }
    Ok(())
}
