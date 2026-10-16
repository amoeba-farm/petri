use std::{
    env,
    ffi::OsString,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    str::FromStr,
    thread,
    time::{Duration, Instant},
};

use petri_conpty_harness::{
    HarnessError, HarnessResult, RunOptions, Scenario, TerminalSize, run_scenario,
};

const DEFAULT_TIMEOUT_MS: u64 = 15_000;
const WATCHDOG_GRACE: Duration = Duration::from_secs(10);

#[derive(Debug)]
enum Mode {
    Smoke,
    Scenario(Scenario),
    Worker(Scenario),
    List,
    Help,
}

#[derive(Debug)]
struct Cli {
    mode: Mode,
    binary: PathBuf,
    repo_dir: PathBuf,
    size: Option<TerminalSize>,
    timeout: Duration,
    artifact_dir: Option<PathBuf>,
    show_screen: bool,
}

fn main() {
    let code = match parse_cli().and_then(run_cli) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("FAIL {error}");
            1
        }
    };
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    std::process::exit(code);
}

fn parse_cli() -> HarnessResult<Cli> {
    let mut args = env::args_os().skip(1).peekable();
    let mut mode = None;
    let mut binary = None;
    let mut repo_dir = None;
    let mut size = None;
    let mut timeout = Duration::from_millis(DEFAULT_TIMEOUT_MS);
    let mut artifact_dir = None;
    let mut show_screen = false;

    while let Some(arg) = args.next() {
        let text = arg.to_string_lossy();
        match text.as_ref() {
            "--help" | "-h" => mode = Some(Mode::Help),
            "--list" => mode = Some(Mode::List),
            "--show-screen" => show_screen = true,
            "--binary" => binary = Some(PathBuf::from(next_value(&mut args, "--binary")?)),
            "--repo" => repo_dir = Some(PathBuf::from(next_value(&mut args, "--repo")?)),
            "--size" => {
                let value = next_value(&mut args, "--size")?;
                size = Some(TerminalSize::from_str(&value.to_string_lossy())?);
            }
            "--timeout-ms" => {
                let value = next_value(&mut args, "--timeout-ms")?;
                let millis = value
                    .to_string_lossy()
                    .parse::<u64>()
                    .map_err(|_| HarnessError::new("--timeout-ms must be a positive integer"))?;
                if millis == 0 {
                    return Err(HarnessError::new("--timeout-ms must be greater than zero"));
                }
                timeout = Duration::from_millis(millis);
            }
            "--artifacts" => {
                artifact_dir = Some(PathBuf::from(next_value(&mut args, "--artifacts")?));
            }
            "--worker" => {
                let value = next_value(&mut args, "--worker")?;
                mode = Some(Mode::Worker(Scenario::from_str(&value.to_string_lossy())?));
            }
            value if value.starts_with('-') => {
                return Err(HarnessError::new(format!("unknown option {value:?}")));
            }
            "smoke" | "all" => mode = Some(Mode::Smoke),
            value => mode = Some(Mode::Scenario(Scenario::from_str(value)?)),
        }
    }

    let repo_dir = repo_dir.unwrap_or_else(default_repo_dir);
    let binary = binary.unwrap_or_else(|| default_binary(&repo_dir));
    Ok(Cli {
        mode: mode.unwrap_or(Mode::Smoke),
        binary,
        repo_dir,
        size,
        timeout,
        artifact_dir,
        show_screen,
    })
}

fn next_value(
    args: &mut std::iter::Peekable<impl Iterator<Item = OsString>>,
    option: &str,
) -> HarnessResult<OsString> {
    args.next()
        .ok_or_else(|| HarnessError::new(format!("{option} requires a value")))
}

fn default_repo_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("harness must live under ameba_cli/tools")
        .to_path_buf()
}

fn default_binary(repo_dir: &Path) -> PathBuf {
    let release = repo_dir.join("target/release/petri.exe");
    if release.is_file() {
        return release;
    }
    let debug = repo_dir.join("target/debug/petri.exe");
    if debug.is_file() {
        return debug;
    }
    release
}

fn run_cli(cli: Cli) -> HarnessResult<()> {
    match cli.mode {
        Mode::Help => {
            print_help();
            Ok(())
        }
        Mode::List => {
            println!("smoke");
            for scenario in Scenario::ALL {
                println!("{}", scenario.name());
            }
            Ok(())
        }
        Mode::Worker(scenario) => run_worker(&cli, scenario),
        Mode::Scenario(scenario) => {
            let size = cli.size.unwrap_or_else(|| scenario.default_size());
            run_suite(&cli, &[(scenario, size)])
        }
        Mode::Smoke => {
            if cli.size.is_some() {
                return Err(HarnessError::new(
                    "--size is only valid for a single named scenario",
                ));
            }
            let cases = [
                (Scenario::Startup, TerminalSize::new(60, 20)),
                (Scenario::Startup, TerminalSize::new(80, 24)),
                (Scenario::Startup, TerminalSize::new(120, 32)),
                (Scenario::HelpKeyboardPreview, TerminalSize::new(80, 24)),
                (Scenario::HelpMouseMove, TerminalSize::new(80, 24)),
                (Scenario::GuideOff, TerminalSize::new(80, 24)),
                (Scenario::HelpResize, TerminalSize::new(120, 32)),
            ];
            run_suite(&cli, &cases)
        }
    }
}

fn run_worker(cli: &Cli, scenario: Scenario) -> HarnessResult<()> {
    let size = cli.size.unwrap_or_else(|| scenario.default_size());
    let mut options = RunOptions::new(cli.binary.clone(), cli.repo_dir.clone(), cli.timeout);
    options.artifact_dir = cli.artifact_dir.clone();
    let report = run_scenario(&options, scenario, size)?;
    println!(
        "PASS {} @ {} ({:.2}s): {}",
        report.scenario.name(),
        report.size,
        report.elapsed.as_secs_f64(),
        report.detail
    );
    if let Some(path) = report.artifact {
        println!("artifact: {}", path.display());
    }
    if cli.show_screen {
        println!("--- current screen ---");
        println!("{}", report.screen);
        println!("--- end screen ---");
    }
    Ok(())
}

fn run_suite(cli: &Cli, cases: &[(Scenario, TerminalSize)]) -> HarnessResult<()> {
    let current_exe = env::current_exe().map_err(|error| {
        HarnessError::new(format!("could not resolve harness executable: {error}"))
    })?;
    let mut failures = Vec::new();
    for &(scenario, size) in cases {
        match run_case_process(&current_exe, cli, scenario, size) {
            Ok(output) => print!("{output}"),
            Err(error) => {
                eprintln!("FAIL {} @ {}: {error}", scenario.name(), size);
                failures.push(format!("{}@{}", scenario.name(), size));
            }
        }
    }
    if failures.is_empty() {
        println!("PASS {} ConPTY scenario(s)", cases.len());
        Ok(())
    } else {
        Err(HarnessError::new(format!(
            "{} of {} ConPTY scenarios failed: {}",
            failures.len(),
            cases.len(),
            failures.join(", ")
        )))
    }
}

fn run_case_process(
    executable: &Path,
    cli: &Cli,
    scenario: Scenario,
    size: TerminalSize,
) -> HarnessResult<String> {
    let mut command = Command::new(executable);
    command
        .arg("--worker")
        .arg(scenario.name())
        .arg("--binary")
        .arg(&cli.binary)
        .arg("--repo")
        .arg(&cli.repo_dir)
        .arg("--size")
        .arg(size.to_string())
        .arg("--timeout-ms")
        .arg(cli.timeout.as_millis().to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(directory) = &cli.artifact_dir {
        command.arg("--artifacts").arg(directory);
    }
    if cli.show_screen {
        command.arg("--show-screen");
    }
    let mut child = command
        .spawn()
        .map_err(|error| HarnessError::new(format!("could not start scenario worker: {error}")))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| HarnessError::new("scenario worker stdout was unavailable"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| HarnessError::new("scenario worker stderr was unavailable"))?;
    let stdout_thread = thread::spawn(move || read_stream(stdout));
    let stderr_thread = thread::spawn(move || read_stream(stderr));
    let deadline = Instant::now() + cli.timeout + WATCHDOG_GRACE;

    let status = loop {
        if let Some(status) = child.try_wait().map_err(|error| {
            HarnessError::new(format!("could not poll scenario worker: {error}"))
        })? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let stdout = join_stream(stdout_thread);
            let stderr = join_stream(stderr_thread);
            return Err(HarnessError::new(format!(
                "outer watchdog stopped a hung scenario worker after {} ms\n{}{}",
                (cli.timeout + WATCHDOG_GRACE).as_millis(),
                stdout,
                stderr
            )));
        }
        thread::sleep(Duration::from_millis(20));
    };
    let stdout = join_stream(stdout_thread);
    let stderr = join_stream(stderr_thread);
    if status.success() {
        Ok(format!("{stdout}{stderr}"))
    } else {
        Err(HarnessError::new(format!(
            "worker exited with {status}\n{stdout}{stderr}"
        )))
    }
}

fn read_stream(mut stream: impl Read) -> Vec<u8> {
    let mut output = Vec::new();
    let _ = stream.read_to_end(&mut output);
    output
}

fn join_stream(thread: thread::JoinHandle<Vec<u8>>) -> String {
    thread
        .join()
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default()
}

fn print_help() {
    println!(
        "Petri native ConPTY QA harness (development and release validation only)\n\n\
Usage:\n  petri-conpty-harness [smoke|SCENARIO] [OPTIONS]\n\n\
Scenarios:\n  smoke                    Run the full isolated suite (default)\n  startup                  Verify real alternate-screen startup\n  help-keyboard-preview    Verify Space-pinned Help preview\n  help-mouse-move          Verify a real SGR no-button mouse move\n  guide-off                Verify the provider-off Guide state\n  help-resize              Verify 60x20, 80x24, and 120x32 redraws\n\n\
Options:\n  --binary PATH            Petri release/debug executable\n  --repo PATH              ameba_cli repository root\n  --size COLSxROWS         Override one named scenario's initial size\n  --timeout-ms N           Per-scenario hard deadline (default 15000)\n  --artifacts DIR          Opt in to normalized text captures\n  --show-screen            Print normalized passing screens\n  --list                   List scenario names\n  -h, --help               Show this help\n\n\
The child environment is cleared, provider access is forced off, data sources\n\
are a harness-owned loopback 404 server, and no wallet or production network is used."
    );
}
