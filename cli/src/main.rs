#![recursion_limit = "256"]

mod agent_protocol;
mod backend;
mod chain_identity;
mod chart;
mod cli;
mod endpoints;
mod futures;
mod mcp_action_bridge;
mod mcp_actions;
mod mcp_setup;
mod onchain;
mod oracle;
mod petri_config;
mod release_update;
mod research;
mod solana_config;
mod terminal_brand;
mod wallet_signer;
mod windows_app_identity;
mod workspace_update;

use std::{
    env,
    io::{self, IsTerminal},
};

use backend::{CliError, json_string, string_at_key, terminal_safe_text};

use clap::{CommandFactory, FromArgMatches};
use cli::{
    Cli, Command, ConfigCommand, ConfigKey, McpCommand, OutputFormat, UpdateCommand, WalletCommand,
};
use futures::{reads, trade};
use serde_json::{Value, json};

fn main() {
    if let Some(result) = release_update::helper_entry() {
        if let Err(error) = result {
            eprintln!("Petri update: {error}");
            std::process::exit(1);
        }
        return;
    }
    windows_app_identity::apply();
    if let Err(error) = run() {
        let arguments = env::args().collect::<Vec<_>>();
        if arguments.iter().any(|a| a == "--json")
            || arguments
                .windows(2)
                .any(|a| a[0] == "--output" && a[1] == "json")
            || env::var("AMEBA_OUTPUT").ok().as_deref() == Some("json")
        {
            println!("{}", error.json());
            std::process::exit(error.exit_code());
        }
        eprintln!("error: {error}");
        std::process::exit(error.exit_code());
    }
}

fn run() -> Result<(), CliError> {
    dispatch_cli(parse_cli())
}

/// DEV-ONLY: REMOVE FOR PROD. A build without the dev faucet (`dev-faucet`)
/// leaves `petri faucet` out of the front door and the help sheet.
fn shown(command: &str) -> bool {
    cfg!(feature = "dev-faucet") || !command.trim_start_matches("petri ").starts_with("faucet")
}

/// The help sheet's `--profile` row: which profile this build starts in.
const PROFILE_SHEET: &str = if cfg!(feature = "dev-chain") {
    "dev (the dev chain; this build's default) or production"
} else {
    "production (mainnet; the default) or dev (the dev chain)"
};

/// The commands the front door suggests, also in its JSON.
const FRONT_DOOR: &[(&str, &str)] = &[
    ("petri markets", "Every market: price, index, settlement"),
    ("petri long ramx 100", "Go long 100 lots at 10x"),
    ("petri short ramx --usd 50", "Go short with about $50"),
    ("petri positions", "Your positions, orders and P&L"),
    ("petri close ramx <#>", "Close a position"),
    ("petri faucet --sol 1", "Dev USDC and SOL for your wallet"),
    ("petri help", "Every command"),
];

fn dispatch_cli(cli: Cli) -> Result<(), CliError> {
    let Some(command) = cli.command.as_ref() else {
        let payload = json!({
            "ok": true,
            "command": "help",
            "title": "Petri",
            "description": "Trade RAMX and NANDX capped futures from the terminal",
            "wallet": front_door_wallet(&cli),
            "commands": FRONT_DOOR.iter().filter(|(c, _)| shown(c)).map(|(c, _)| *c).collect::<Vec<_>>(),
        });
        return emit_output(&cli, &payload, render_cli_front_door(&cli));
    };

    match command {
        Command::Tui {
            no_update_check,
            render,
            view,
        } => {
            let settings = futures::settings::Settings::from_cli(&cli).map_err(CliError::new)?;
            if cli.resolved_output() == OutputFormat::Json {
                research::command(&["owner".into(), "snapshot".into()], &settings.api_url.0)?;
            } else {
                research::run(
                    !no_update_check,
                    render.as_deref(),
                    view.as_deref(),
                    settings,
                )
                .map_err(CliError::new)?;
            }
        }
        Command::Research { args } => {
            let settings = futures::settings::Settings::from_cli(&cli).map_err(CliError::new)?;
            research::command(args, &settings.api_url.0)?
        }
        Command::Markets { market, expiry } => match market {
            None => emit_futures(&cli, reads::markets)?,
            Some(market) => {
                let arg = futures::args::MarketArg {
                    market: market.clone(),
                    expiry: expiry.clone(),
                };
                emit_futures(&cli, |ctx, p| reads::market(ctx, p, &arg))?
            }
        },
        Command::Book {
            market,
            range: shape,
            depth,
        } => emit_futures(&cli, |ctx, p| {
            reads::book(ctx, p, market, shape.as_deref(), *depth as usize)
        })?,
        Command::Trades { market, limit } => {
            emit_futures(&cli, |ctx, p| reads::trades(ctx, p, market, *limit))?
        }
        Command::Chart {
            market,
            window,
            height,
        } => emit_futures(&cli, |ctx, p| {
            reads::chart(ctx, p, market, *window, *height)
        })?,
        Command::Quote(args) => emit_futures(&cli, |ctx, p| trade::quote(ctx, p, args))?,
        Command::Long(args) => emit_futures(&cli, |ctx, p| {
            trade::open(ctx, p, futures::args::SideArg::Long, args)
        })?,
        Command::Short(args) => emit_futures(&cli, |ctx, p| {
            trade::open(ctx, p, futures::args::SideArg::Short, args)
        })?,
        Command::Positions { owner } => {
            emit_futures(&cli, |ctx, p| reads::positions(ctx, p, owner.as_deref()))?
        }
        Command::Close(args) => emit_futures(&cli, |ctx, p| trade::close(ctx, p, args))?,
        Command::Cancel(args) => emit_futures(&cli, |ctx, p| trade::cancel(ctx, p, args))?,
        Command::Deposit(args) => emit_futures(&cli, |ctx, p| trade::deposit(ctx, p, args))?,
        Command::Withdraw(args) => emit_futures(&cli, |ctx, p| trade::withdraw(ctx, p, args))?,
        Command::History { owner, limit } => emit_futures(&cli, |ctx, p| {
            reads::history(ctx, p, owner.as_deref(), *limit)
        })?,
        Command::Wallet { command, owner } => match command {
            None => emit_futures(&cli, |ctx, p| reads::wallet(ctx, p, owner.as_deref()))?,
            Some(WalletCommand::Address) => emit_futures(&cli, futures::ops::wallet_address)?,
            Some(WalletCommand::New { out, force }) => emit_futures(&cli, |ctx, p| {
                futures::ops::wallet_new(ctx, p, out.as_ref(), *force)
            })?,
        },
        // DEV-ONLY: REMOVE FOR PROD.
        #[cfg(feature = "dev-faucet")]
        Command::Faucet { owner, sol } => emit_futures(&cli, |ctx, p| {
            futures::funds::faucet(ctx, p, owner.as_deref(), sol.as_deref())
        })?,
        Command::Status => emit_futures(&cli, futures::ops::status)?,
        Command::Config { command } => {
            match command {
                None | Some(ConfigCommand::Show) => {}
                Some(ConfigCommand::Set { key, value }) => {
                    petri_config::set_futures_value(futures_config_key(*key), value)
                        .map_err(CliError::new)?;
                }
                Some(ConfigCommand::Reset { key }) => {
                    petri_config::reset_futures_value(futures_config_key(*key))
                        .map_err(CliError::new)?;
                }
            }
            let payload = build_config_show_payload(&cli)?;
            emit_output(&cli, &payload, render_config_show(&payload))?;
        }
        Command::Oracle { command } => {
            let report = oracle::run(&cli, command.as_ref())?;
            emit_output(&cli, &report.payload, report.plain)?;
        }
        Command::Mcp { command } => run_mcp(&cli, command)?,
        Command::Update {
            command,
            no_fetch,
            skip_shim,
            restart,
        } => run_update(&cli, command.as_ref(), *no_fetch, *skip_shim, *restart)?,
    }
    Ok(())
}

fn run_mcp(cli: &Cli, command: &McpCommand) -> Result<(), CliError> {
    match command {
        McpCommand::Serve => mcp_action_bridge::serve(cli)?,
        McpCommand::Actions => {
            let payload = mcp_actions::manifest();
            emit_output(
                cli,
                &payload,
                serde_json::to_string_pretty(&payload).unwrap_or_default(),
            )?;
        }
        McpCommand::Invoke { action, .. } => {
            mcp_actions::invoke_stdin(cli, action)?;
        }
        McpCommand::Status => {
            let status = mcp_setup::status().map_err(|error| {
                CliError::new(format!("could not read Petri MCP status: {error}"))
            })?;
            let payload = mcp_setup_payload("status", &status, false);
            emit_output(cli, &payload, render_mcp_setup_status(&status, None))?;
        }
        McpCommand::Enable => {
            let status = mcp_setup::enable()
                .map_err(|error| CliError::new(format!("could not enable Petri MCP: {error}")))?;
            let payload = mcp_setup_payload("enable", &status, true);
            emit_output(
                cli,
                &payload,
                render_mcp_setup_status(
                    &status,
                    Some("Petri MCP enabled. Restart or reload an AI agent that is already open."),
                ),
            )?;
        }
        McpCommand::Repair => {
            let repair = mcp_setup::repair().map_err(|error| {
                CliError::new(format!(
                    "could not repair the Petri agent connection: {error}\n\nNo disconnect was attempted. If repair still cannot complete, disable only Petri's managed connection, then connect it again."
                ))
            })?;
            let mut payload = mcp_setup_payload(
                "repair",
                &repair.status,
                repair.outcome == mcp_setup::McpRepairOutcome::Repaired,
            );
            if let Some(object) = payload.as_object_mut() {
                object.insert("repair_outcome".to_string(), json!(repair.outcome.as_str()));
            }
            let result_message = match repair.outcome {
                mcp_setup::McpRepairOutcome::AlreadyHealthy => {
                    "Connection check complete. Petri MCP is healthy; no settings were changed."
                }
                mcp_setup::McpRepairOutcome::Repaired => {
                    "Repair complete. Petri MCP is healthy. Restart or reload an AI agent that is already open."
                }
            };
            emit_output(
                cli,
                &payload,
                render_mcp_setup_status(&repair.status, Some(result_message)),
            )?;
        }
        McpCommand::Disable => {
            let status = mcp_setup::disable()
                .map_err(|error| CliError::new(format!("could not disable Petri MCP: {error}")))?;
            let payload = mcp_setup_payload("disable", &status, true);
            emit_output(
                cli,
                &payload,
                render_mcp_setup_status(
                    &status,
                    Some("Petri MCP disabled. Restart or reload an AI agent that is already open."),
                ),
            )?;
        }
        McpCommand::Manifest => {
            let payload = agent_protocol::mcp_manifest();
            emit_output(cli, &payload, agent_protocol::render_mcp_manifest(&payload))?;
        }
    }
    Ok(())
}

fn run_update(
    cli: &Cli,
    command: Option<&UpdateCommand>,
    no_fetch: bool,
    skip_shim: bool,
    restart: bool,
) -> Result<(), CliError> {
    if matches!(command, Some(UpdateCommand::Info)) {
        let info = release_update::information();
        emit_output(
            cli,
            &info,
            format!(
                "Petri {} | {} updates",
                env!("CARGO_PKG_VERSION"),
                if release_update::enabled() {
                    "preview release"
                } else {
                    "source"
                }
            ),
        )?;
        return Ok(());
    }
    if release_update::enabled() {
        if no_fetch || skip_shim {
            return Err(CliError::new(
                "--no-fetch and --skip-shim apply only to source-checkout updates.",
            ));
        }
        let report = run_release_update(
            command,
            cli.yes,
            restart,
            cli.resolved_output() == OutputFormat::Json,
        )?;
        let payload = serde_json::to_value(&report)
            .map_err(|_| CliError::new("Could not encode update status."))?;
        emit_output(cli, &payload, report.message)?;
        return Ok(());
    }
    if matches!(command, Some(UpdateCommand::Recover)) || restart {
        return Err(CliError::new(
            "Recovery and --restart apply only to standalone preview installations.",
        ));
    }
    let report = if matches!(command, Some(UpdateCommand::Check)) {
        workspace_update::check_workspace_update(no_fetch)?
    } else {
        workspace_update::run_workspace_update(no_fetch, skip_shim)?
    };
    let payload = serde_json::to_value(&report)
        .map_err(|error| CliError::new(format!("failed to encode update report: {error}")))?;
    emit_output(
        cli,
        &payload,
        workspace_update::render_update_report(&report),
    )?;
    if !report.ok {
        return Err(CliError::new(
            report
                .blocked_reason
                .unwrap_or_else(|| "Petri update did not complete".to_string()),
        ));
    }
    Ok(())
}

fn confirm_app_update(message: &str, yes: bool, json: bool) -> Result<bool, CliError> {
    if yes {
        return Ok(true);
    }
    if json || !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(CliError::new(
            "Check the release with petri update check, then use petri update --yes to approve it.",
        ));
    }
    use std::io::Write;
    println!("{message}");
    print!("Continue? [y/N] ");
    io::stdout()
        .flush()
        .map_err(|_| CliError::new("Could not display update confirmation."))?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .map_err(|_| CliError::new("Could not read update confirmation."))?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn run_release_update(
    command: Option<&UpdateCommand>,
    yes: bool,
    restart: bool,
    json: bool,
) -> Result<release_update::Report, CliError> {
    if matches!(command, Some(UpdateCommand::Recover)) {
        if !confirm_app_update(
            "Restore the saved previous Petri app files? Wallets and settings will not be changed.",
            yes,
            json,
        )? {
            return Err(CliError::new("Recovery cancelled. No files were changed."));
        }
        return release_update::recover(restart).map_err(CliError::new);
    }
    let report = release_update::check().map_err(CliError::new)?;
    if matches!(command, Some(UpdateCommand::Check)) || !report.update_available {
        return Ok(report);
    }
    let release = report
        .release
        .as_ref()
        .ok_or_else(|| CliError::new("Missing release information."))?;
    let prompt = format!(
        "Install Petri {} from {}?\nThis preview is not publisher-signed or Apple-notarized. Petri verifies the official GitHub download and SHA-256 before replacing app files. Wallets and settings will not be changed.",
        release.version, release.release_url
    );
    if !confirm_app_update(&prompt, yes, json)? {
        return Err(CliError::new("Update cancelled. No files were changed."));
    }
    release_update::prepare(release, restart).map_err(CliError::new)
}

fn mcp_setup_payload(
    action: &str,
    status: &mcp_setup::McpSetupStatus,
    restart_required: bool,
) -> Value {
    json!({
        "ok": true,
        "action": action,
        "codex": status.codex.as_str(),
        "claude_code": status.claude.as_str(),
        "runtime_ready": status.runtime_ready,
        "transaction_execution": "not_available",
        "signing_boundary": "Petri MCP is read-only; trades and oracle writes sign with the local wallet in a terminal.",
        "fully_enabled": status.is_fully_enabled(),
        "partially_enabled": status.is_partially_enabled(),
        "needs_repair": status.needs_repair(),
        "has_conflict": status.has_conflict(),
        "restart_required": restart_required,
    })
}

fn render_mcp_setup_status(
    status: &mcp_setup::McpSetupStatus,
    result_message: Option<&str>,
) -> String {
    let mut lines = Vec::new();
    if let Some(message) = result_message {
        lines.push(message.to_string());
        lines.push(String::new());
    }
    lines.push("Petri MCP connection".to_string());
    lines.push(format!("  Codex:      {}", status.codex.as_str()));
    lines.push(format!("  Claude Code: {}", status.claude.as_str()));
    lines.push(format!(
        "  Runtime:    {}",
        if status.runtime_ready {
            "ready"
        } else {
            "not ready"
        }
    ));
    lines.push("  Transactions: not available through Petri MCP".to_string());
    if status.has_conflict() {
        lines.push("An existing unowned Petri MCP entry was left unchanged.".to_string());
    } else if status.needs_repair() {
        lines.push("Run `petri mcp repair` to diagnose and repair the connection.".to_string());
    }
    lines.join("\n")
}

fn parse_cli() -> Cli {
    let raw_args = env::args().collect::<Vec<_>>();
    let no_color = env::args_os().any(|arg| arg == "--no-color");
    let color_enabled = !no_color && io::stdout().is_terminal();
    if is_root_help_request(&raw_args) {
        println!(
            "{}",
            render_petri_help_sheet(color_enabled, help_sheet_terminal_width())
        );
        std::process::exit(0);
    }
    let color = if no_color || !io::stdout().is_terminal() {
        clap::ColorChoice::Never
    } else {
        clap::ColorChoice::Always
    };
    let matches = match Cli::command().color(color).try_get_matches() {
        Ok(matches) => matches,
        Err(error) => {
            let show_agent_hint = error.use_stderr();
            let exit_code = error.exit_code();
            if show_agent_hint {
                eprintln!("{}", strip_clap_help_hint(&error.to_string()));
                eprintln!("{}", agent_assist_hint());
            } else {
                let _ = error.print();
            }
            std::process::exit(exit_code);
        }
    };
    Cli::from_arg_matches(&matches).unwrap_or_else(|error| error.exit())
}

fn is_root_help_request(args: &[String]) -> bool {
    if args.len() <= 1 {
        return false;
    }
    let path = command_path_from_args(args);
    if path.len() == 1 && path[0] == "help" {
        return true;
    }
    path.is_empty()
        && args
            .iter()
            .skip(1)
            .any(|arg| matches!(arg.as_str(), "--help" | "-h"))
}

fn command_path_from_args(args: &[String]) -> Vec<String> {
    let value_globals = [
        "--solana-config",
        "--keypair",
        "--output",
        "--rpc",
        "--api",
        "--deployment",
        "--route",
    ];
    let mut path = Vec::new();
    let mut index = 1;
    while index < args.len() {
        let arg = args[index].as_str();
        if arg == "--" {
            break;
        }
        if arg.starts_with("--") {
            let has_inline_value = arg.contains('=');
            let needs_value = value_globals
                .iter()
                .any(|global| arg == *global || arg.starts_with(&format!("{global}=")));
            index += 1;
            if needs_value && !has_inline_value {
                index += 1;
            }
            continue;
        }
        if arg.starts_with('-') {
            index += 1;
            continue;
        }
        path.push(arg.to_ascii_lowercase());
        if path.len() >= 3 {
            break;
        }
        index += 1;
    }
    path
}

fn agent_assist_hint() -> &'static str {
    "Agent assist: paste your goal and this Petri error into an MCP-aware coding agent such as Claude Code or Codex, and ask it to choose the right Petri command."
}

fn strip_clap_help_hint(message: &str) -> String {
    message
        .lines()
        .filter(|line| line.trim() != "For more information, try '--help'.")
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string()
}
fn render_cli_front_door(cli: &Cli) -> String {
    let output_is_tty = io::stdout().is_terminal();
    let terminal_width = if output_is_tty {
        crossterm::terminal::size()
            .map(|(width, _)| usize::from(width))
            .unwrap_or(terminal_brand::WIDE_BANNER_MIN_COLUMNS)
    } else {
        terminal_brand::WIDE_BANNER_MIN_COLUMNS
    };
    render_cli_front_door_for_terminal(cli, output_is_tty, terminal_width)
}

fn render_cli_front_door_for_terminal(
    cli: &Cli,
    output_is_tty: bool,
    terminal_width: usize,
) -> String {
    let mut lines = if output_is_tty && cli.resolved_output() != OutputFormat::Json {
        terminal_brand::terminal_banner_lines_for_width(terminal_width)
            .into_iter()
            .map(|line| {
                render_cli_brand_line_enabled(
                    terminal_brand::color_enabled(cli.no_color, output_is_tty),
                    &line,
                )
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let wallet = front_door_wallet(cli);
    lines.push(front_door_paint(
        cli,
        ANSI_DIM,
        &match wallet["pubkey"].as_str() {
            Some(pubkey) => format!("Wallet {pubkey}"),
            None => "No wallet yet: petri wallet new".to_string(),
        },
    ));
    lines.push(String::new());
    lines.push("Trade ranges of the RAMX and NANDX memory-chip indexes.".to_string());
    lines.push(
        "A position pays per point the index settles inside its range: it never loses more than it cost, and is never liquidated."
            .to_string(),
    );
    lines.push(String::new());
    lines.push(front_door_heading(cli, "Start here:"));
    for (command, description) in FRONT_DOOR.iter().filter(|(c, _)| shown(c)) {
        lines.push(front_door_command_line(cli, command, description));
    }
    lines.join("\n")
}

const ANSI_RESET: &str = "\x1b[0m";
const ANSI_BOLD_CYAN: &str = "\x1b[1;36m";
const ANSI_BOLD_GREEN: &str = "\x1b[1;32m";
const ANSI_BRAND_TITLE: &str = "\x1b[38;2;78;165;202;48;2;78;165;202m";
const ANSI_BRAND_OUTLINE: &str = "\x1b[38;2;229;229;229m";
const ANSI_BRAND_BODY: &str = "\x1b[38;2;231;231;231m";
const ANSI_BOLD_YELLOW: &str = "\x1b[1;33m";
const ANSI_DIM: &str = "\x1b[2m";

fn render_cli_brand_line_enabled(
    color_enabled: bool,
    line: &[terminal_brand::BrandSpan],
) -> String {
    line.iter()
        .map(|span| {
            let code = match span.tone {
                terminal_brand::BrandTone::Title => ANSI_BRAND_TITLE,
                terminal_brand::BrandTone::Outline => ANSI_BRAND_OUTLINE,
                terminal_brand::BrandTone::Body => ANSI_BRAND_BODY,
                terminal_brand::BrandTone::Plain => "",
            };
            let text = if color_enabled && span.fill {
                " ".repeat(span.text.chars().count())
            } else {
                span.text.clone()
            };
            front_door_paint_enabled(color_enabled && !code.is_empty(), code, &text)
        })
        .collect::<String>()
        .trim_end()
        .to_string()
}

fn front_door_heading(cli: &Cli, text: &str) -> String {
    front_door_paint(cli, ANSI_BOLD_CYAN, text)
}

fn front_door_command_line(cli: &Cli, command: &str, description: &str) -> String {
    let command_cell = format!("{command:<28}");
    format!(
        "  {} {}",
        front_door_paint(cli, ANSI_BOLD_GREEN, &command_cell),
        description
    )
}

fn front_door_paint(cli: &Cli, code: &str, text: &str) -> String {
    front_door_paint_enabled(!cli.no_color && io::stdout().is_terminal(), code, text)
}

fn front_door_paint_enabled(enabled: bool, code: &str, text: &str) -> String {
    if enabled {
        format!("{code}{text}{ANSI_RESET}")
    } else {
        text.to_string()
    }
}

const HELP_SHEET_DEFAULT_WIDTH: usize = 100;
const HELP_SHEET_MIN_WIDTH: usize = 24;
const HELP_REFERENCE_MAX_WIDTH: usize = 100;
const HELP_KEY_WIDTH: usize = 28;

/// `petri help`: every command, grouped, then the global options and examples.
const HELP_SHEET: &[(&str, &[(&str, &str)])] = &[
    (
        "Markets",
        &[
            (
                "markets [market]",
                "Every market, or one: price, index, suggested range",
            ),
            (
                "book <market>",
                "What a range costs, level by level (--range 90-120)",
            ),
            ("trades <market>", "Recent trades"),
            (
                "chart <market> [24h]",
                "Price chart: 1h, 24h, 7d, 30d or all",
            ),
        ],
    ),
    (
        "Trade",
        &[
            (
                "long <market> <lots>",
                "Buy a range: -x leverage, --to, --range, --limit, --usd",
            ),
            (
                "short <market> <lots>",
                "Sell a range, with the same options",
            ),
            ("quote long|short ...", "Price an order without sending it"),
            (
                "positions [owner]",
                "Positions, orders and cash per market, with P&L",
            ),
            ("close <market> <#|all>", "Close a position at the book"),
            ("cancel <market> <#|all>", "Cancel a resting order"),
            (
                "deposit <market> <usd>",
                "Move USDC from your wallet into a market",
            ),
            (
                "withdraw <market> <usd|all>",
                "Move free USDC back to your wallet",
            ),
            (
                "history [owner]",
                "Trades, orders, deposits and settlements",
            ),
        ],
    ),
    (
        "Wallet",
        &[
            (
                "wallet [owner]",
                "Address, SOL, USDC and cash in each market",
            ),
            ("wallet new", "Create a signing wallet"),
            ("faucet [--sol 1]", "Dev USDC (and SOL) for your wallet"),
            ("status", "Check the cluster, programs, API and wallet"),
        ],
    ),
    (
        "Oracle and research",
        &[
            ("oracle", "Oracle months, sources, evidence and bonds"),
            ("research", "Connect research agents"),
            ("tui", "The terminal interface: markets and oracle research"),
        ],
    ),
    (
        "Setup",
        &[
            (
                "config [set|reset]",
                "Saved settings: profile, rpc, api, keypair, deployment",
            ),
            ("mcp", "Connect Petri to AI agents (read-only)"),
            ("update", "Check for and install updates"),
        ],
    ),
    (
        "Global options",
        &[
            ("--json", "Print JSON"),
            ("--yes", "Send without asking"),
            ("--keypair <PATH>", "The wallet that signs"),
            ("--profile <NAME>", PROFILE_SHEET),
            (
                "--rpc <URL>",
                "Solana RPC [https://api.mainnet-beta.solana.com]",
            ),
            ("--api <URL>", "Futures API [https://api.amoeba.farm]"),
            ("--route <ROUTE>", "Who pays fees: auto, local or sponsored"),
            ("--no-color", "No color"),
        ],
    ),
    (
        "Examples",
        &[
            (
                "petri long ramx 100",
                "100 lots at 10x on the suggested range",
            ),
            (
                "petri long ramx 100 -x 5 --to 130",
                "5x, the range ending at 130",
            ),
            ("petri short nandx --usd 50", "Spend about $50 going short"),
            ("petri long ramx 100 --limit 99", "Rest a bid at 99.00"),
            ("petri close ramx-nov26 78", "Close position #78"),
        ],
    ),
];

fn help_sheet_terminal_width() -> usize {
    if io::stdout().is_terminal() {
        crossterm::terminal::size()
            .map(|(width, _)| usize::from(width))
            .unwrap_or(HELP_SHEET_DEFAULT_WIDTH)
    } else {
        HELP_SHEET_DEFAULT_WIDTH
    }
}

fn render_petri_help_sheet(color_enabled: bool, terminal_width: usize) -> String {
    let width = terminal_width.clamp(HELP_SHEET_MIN_WIDTH, HELP_REFERENCE_MAX_WIDTH);
    let mut lines = vec![format!(
        "{} {}",
        help_sheet_paint(color_enabled, ANSI_BOLD_YELLOW, "Petri"),
        env!("CARGO_PKG_VERSION")
    )];
    lines.extend(help_sheet_wrap_words(
        "Trade ranges of the RAMX and NANDX memory-chip indexes. A position never loses more than it cost and is never liquidated.",
        width,
    ));
    lines.push(String::new());
    lines.push(help_sheet_paint(color_enabled, ANSI_BOLD_CYAN, "Usage:"));
    lines.extend(help_sheet_usage_reference_lines(color_enabled, width));
    for (heading, rows) in HELP_SHEET {
        lines.push(String::new());
        lines.push(help_sheet_paint(
            color_enabled,
            ANSI_BOLD_CYAN,
            &format!("{heading}:"),
        ));
        let examples = *heading == "Examples";
        for (key, description) in rows.iter().filter(|(key, _)| shown(key)) {
            push_help_reference_row(
                &mut lines,
                color_enabled,
                key,
                description,
                width,
                if examples { 36 } else { HELP_KEY_WIDTH },
                if examples {
                    ANSI_BOLD_GREEN
                } else {
                    ANSI_BOLD_CYAN
                },
            );
        }
    }
    lines.push(String::new());
    lines.extend(help_sheet_wrap_words(
        "Name a market by symbol (ramx: the open one expiring first), by name (ramx-nov26) or by address. Every command explains itself with --help.",
        width,
    ));
    lines.join("\n")
}

fn help_sheet_usage_reference_lines(color_enabled: bool, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for usage in ["petri <command> [args] [options]"] {
        let indent = 2.min(width.saturating_sub(1));
        let body_width = width.saturating_sub(indent).max(1);
        for line in help_sheet_wrap_words(usage, body_width) {
            lines.push(format!(
                "{}{}",
                " ".repeat(indent),
                help_sheet_paint(color_enabled, ANSI_BOLD_GREEN, &line)
            ));
        }
    }
    lines
}

fn push_help_reference_row(
    lines: &mut Vec<String>,
    color_enabled: bool,
    key: &str,
    description: &str,
    width: usize,
    key_width: usize,
    key_color: &str,
) {
    let indent = 2.min(width.saturating_sub(1));
    let content_width = width.saturating_sub(indent).max(1);
    let key_len = key.chars().count();
    if key_len <= key_width && content_width >= key_width + 14 {
        let description_width = content_width.saturating_sub(key_width + 2).max(1);
        let description_lines = help_sheet_wrap_words(description, description_width);
        lines.push(format!(
            "{}{}  {}",
            " ".repeat(indent),
            help_sheet_cell(color_enabled, key, key_width, key_color),
            description_lines.first().cloned().unwrap_or_default()
        ));
        for line in description_lines.into_iter().skip(1) {
            lines.push(format!(
                "{}{}  {}",
                " ".repeat(indent),
                " ".repeat(key_width),
                line
            ));
        }
        return;
    }

    let key_width = content_width;
    for key_line in help_sheet_wrap_words(key, key_width) {
        lines.push(format!(
            "{}{}",
            " ".repeat(indent),
            help_sheet_paint(color_enabled, key_color, &key_line)
        ));
    }
    let description_indent = (indent + 2).min(width.saturating_sub(1));
    let description_width = width.saturating_sub(description_indent).max(1);
    for line in help_sheet_wrap_words(description, description_width) {
        lines.push(format!("{}{}", " ".repeat(description_indent), line));
    }
}

fn help_sheet_wrap_words(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    if text.is_empty() {
        return vec![String::new()];
    }

    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let word_len = word.chars().count();
        if word_len > width {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            let mut chunk = String::new();
            let mut chunk_len = 0;
            for ch in word.chars() {
                if chunk_len == width {
                    lines.push(chunk);
                    chunk = String::new();
                    chunk_len = 0;
                }
                chunk.push(ch);
                chunk_len += 1;
            }
            current = chunk;
        } else if current.is_empty() {
            current.push_str(word);
        } else if current.chars().count() + 1 + word_len <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(current);
            current = word.to_string();
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        vec![String::new()]
    } else {
        lines
    }
}

fn help_sheet_cell(color_enabled: bool, text: &str, width: usize, code: &str) -> String {
    let padded = help_sheet_plain_cell(text, width);
    help_sheet_paint(color_enabled, code, &padded)
}

fn help_sheet_plain_cell(text: &str, width: usize) -> String {
    let clipped = if text.chars().count() > width {
        text.chars().take(width).collect::<String>()
    } else {
        text.to_string()
    };
    format!("{clipped:<width$}")
}

fn help_sheet_paint(color_enabled: bool, code: &str, text: &str) -> String {
    front_door_paint_enabled(color_enabled, code, text)
}

/// The signing wallet as trades resolve it: `--keypair`, the saved setting,
/// the Solana CLI config, or the default path.
fn front_door_wallet(cli: &Cli) -> Value {
    let Ok(settings) = futures::settings::Settings::from_cli(cli) else {
        return json!({ "attached": false });
    };
    let pubkey = wallet_signer::signer_pubkey_from_path(&settings.keypair.0).ok();
    json!({
        "attached": pubkey.is_some(),
        "pubkey": pubkey,
        "keypairPath": settings.keypair.0,
        "keypairSource": settings.keypair.1.word(),
    })
}

fn emit_futures(
    cli: &Cli,
    command: impl FnOnce(
        &futures::ctx::Ctx,
        futures::view::Paint,
    ) -> futures::error::Result<futures::Report>,
) -> Result<(), CliError> {
    let report = futures::run(cli, command)?;
    emit_output(cli, &report.payload, report.plain)
}

fn futures_config_key(key: ConfigKey) -> petri_config::FuturesKey {
    match key {
        ConfigKey::RpcUrl => petri_config::FuturesKey::RpcUrl,
        ConfigKey::FuturesApiUrl => petri_config::FuturesKey::FuturesApiUrl,
        ConfigKey::Keypair => petri_config::FuturesKey::Keypair,
        ConfigKey::Deployment => petri_config::FuturesKey::Deployment,
        ConfigKey::Profile => petri_config::FuturesKey::Profile,
    }
}

fn build_config_show_payload(cli: &Cli) -> Result<Value, CliError> {
    let solana_cli_config = solana_config::load_solana_cli_config(cli.solana_config.as_deref())?;
    let settings = futures::settings::Settings::from_cli(cli).map_err(CliError::new)?;
    Ok(json!({
        "profile": settings.profile.0.name(),
        "profileSource": settings.profile.1.word(),
        "cluster": settings.profile.0.cluster(),
        "rpcUrl": settings.rpc_url.0,
        "rpcUrlSource": settings.rpc_url.1.word(),
        "apiUrl": settings.api_url.0,
        "apiUrlSource": settings.api_url.1.word(),
        "keypairPath": settings.keypair.0,
        "keypairSource": settings.keypair.1.word(),
        "keypairPubkey": wallet_signer::signer_pubkey_from_path(&settings.keypair.0).ok(),
        "keypairFile": solana_config::keypair_file_security_label(&settings.keypair.0),
        "allowInsecureKeypair": cli.allow_insecure_keypair,
        "deployment": settings.deployment.as_ref().map(|d| d.0.display().to_string()),
        "route": format!("{:?}", settings.route).to_lowercase(),
        "petriConfigPath": petri_config::config_path()
            .map_err(CliError::new)?
            .display()
            .to_string(),
        "solanaConfigPath": solana_cli_config
            .as_ref()
            .map(|config| config.path.display().to_string())
            .unwrap_or_else(|| solana_config::resolve_config_path(cli.solana_config.as_deref()).display().to_string()),
        "solanaConfigLoaded": solana_cli_config.is_some(),
        "output": format!("{:?}", cli.resolved_output()).to_lowercase(),
    }))
}

fn render_config_show(payload: &Value) -> String {
    let text = |key: &str| string_at_key(payload, &[key]).unwrap_or_else(|| "-".to_string());
    let keypair_note = match text("keypairFile").as_str() {
        "missing" => " · missing: create one with `petri wallet new`".to_string(),
        "-" | "ok" | "private" => String::new(),
        other => format!(" · {other}"),
    };
    render_terminal_lines([
        format!("Settings, saved in {}", text("petriConfigPath")),
        format!(
            "  profile     {} · {}  ({})",
            text("profile"),
            text("cluster"),
            text("profileSource")
        ),
        format!(
            "  rpc         {}  ({})",
            text("rpcUrl"),
            text("rpcUrlSource")
        ),
        format!(
            "  api         {}  ({})",
            text("apiUrl"),
            text("apiUrlSource")
        ),
        format!(
            "  keypair     {}  ({}){keypair_note}",
            text("keypairPath"),
            text("keypairSource")
        ),
        format!(
            "  wallet      {}",
            match text("keypairPubkey").as_str() {
                "-" => "none yet".to_string(),
                pubkey => pubkey.to_string(),
            }
        ),
        format!(
            "  deployment  {}",
            match text("deployment").as_str() {
                "-" => "from the API".to_string(),
                path => path.to_string(),
            }
        ),
        format!("  route       {}", text("route")),
        "Change one: petri config set rpc <URL> · petri config reset rpc".to_string(),
    ])
}

fn emit_output(cli: &Cli, payload: &Value, plain: String) -> Result<(), CliError> {
    if mcp_actions::capture_output(payload) {
        return Ok(());
    }
    match cli.resolved_output() {
        OutputFormat::Plain => println!("{plain}"),
        OutputFormat::Json => {
            let public;
            let payload = if mcp_actions::active() {
                public = mcp_actions::public_output(payload);
                &public
            } else {
                payload
            };
            println!("{}", json_string(payload)?);
        }
    }
    Ok(())
}

const MAX_PLAIN_OUTPUT_LINE_CHARS: usize = 1024;

fn render_terminal_lines(lines: impl IntoIterator<Item = String>) -> String {
    lines
        .into_iter()
        .map(|line| short_text(&line, MAX_PLAIN_OUTPUT_LINE_CHARS))
        .collect::<Vec<_>>()
        .join("\n")
}

fn short_text(value: &str, max_chars: usize) -> String {
    let safe = terminal_safe_text(value);
    let value = safe.as_str();
    let char_count = value.chars().count();
    if char_count <= max_chars || max_chars < 12 {
        return value.to_string();
    }
    let head_len = (max_chars - 3) / 2;
    let tail_len = max_chars - 3 - head_len;
    let head = value.chars().take(head_len).collect::<String>();
    let tail = value
        .chars()
        .rev()
        .take(tail_len)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("{head}...{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_config_show_names_every_connection_setting() {
        let rendered = render_config_show(&json!({
            "profile": "dev", "profileSource": "flag/env", "cluster": "localnet",
            "rpcUrl": "http://127.0.0.1:8899", "rpcUrlSource": "default",
            "apiUrl": "http://127.0.0.1:4500", "apiUrlSource": "default",
            "keypairPath": "/k.json", "keypairFile": "missing", "deployment": "/d.json",
            "route": "auto",
        }));
        assert!(rendered.contains("  profile     dev · localnet  (flag/env)"));
        assert!(rendered.contains("  rpc         http://127.0.0.1:8899  (default)"));
        assert!(rendered.contains("  api         http://127.0.0.1:4500  (default)"));
        assert!(rendered.contains("petri wallet new"));
        assert!(rendered.contains("  deployment  /d.json"));
    }

    #[test]
    fn help_sheet_lists_every_visible_command() {
        let sheet = render_petri_help_sheet(false, 100);
        for c in Cli::command().get_subcommands() {
            if c.is_hide_set() || c.get_name() == "help" {
                continue;
            }
            assert!(
                sheet.contains(&format!("  {}", c.get_name())),
                "the help sheet leaves out `{}`",
                c.get_name()
            );
        }
        assert!(sheet.lines().all(|l| l.chars().count() <= 100));
    }

    #[test]
    fn plain_output_is_terminal_safe_and_line_bounded() {
        let long = "x".repeat(MAX_PLAIN_OUTPUT_LINE_CHARS * 2);
        let rendered = render_terminal_lines(["a\u{1b}[31mb".to_string(), long]);
        assert!(!rendered.contains('\u{1b}'));
        assert!(
            rendered
                .lines()
                .all(|l| l.chars().count() <= MAX_PLAIN_OUTPUT_LINE_CHARS)
        );
    }
}
