//! The paper's properties, one test each.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

use super::{
    Component, Ctx, FiberId, INACTIVE_EFFECT, Registry, Result, State,
    loader::{Loader, Row},
};

type Log = Arc<Mutex<Vec<String>>>;

fn keys(keys: &[&str]) -> BTreeSet<String> {
    keys.iter().map(|k| k.to_string()).collect()
}

fn said(log: &Log) -> Vec<String> {
    std::mem::take(&mut *log.lock().unwrap())
}

/// Provides `key` = `value`, and logs its start and its undo.
struct Provide {
    name: &'static str,
    key: &'static str,
    value: u64,
    inject: &'static [&'static str],
    log: Log,
}

impl Component for Provide {
    fn inject(&self) -> BTreeSet<String> {
        keys(self.inject)
    }
    fn provide(&self) -> BTreeSet<String> {
        keys(&[self.key])
    }
    fn apply(&mut self, ctx: &mut Ctx) -> Result<()> {
        let (log, name) = (self.log.clone(), self.name);
        ctx.effect(move || {
            log.lock().unwrap().push(format!("{name} on"));
            Ok(Box::new(move || {
                log.lock().unwrap().push(format!("{name} off"))
            }))
        })?;
        ctx.provide(self.key, self.value)
    }
}

/// Reads what it injects when it starts; logs it and its stop.
struct Use {
    name: &'static str,
    inject: &'static [&'static str],
    log: Log,
}

impl Component for Use {
    fn inject(&self) -> BTreeSet<String> {
        keys(self.inject)
    }
    fn apply(&mut self, ctx: &mut Ctx) -> Result<()> {
        let seen: Vec<String> = self
            .inject
            .iter()
            .map(|k| ctx.need::<u64>(k).map(|v| format!("{k}={v}")))
            .collect::<Result<_>>()?;
        let (log, name) = (self.log.clone(), self.name);
        ctx.effect(move || {
            log.lock()
                .unwrap()
                .push(format!("{name} on {}", seen.join(" ")));
            Ok(Box::new(move || {
                log.lock().unwrap().push(format!("{name} off"))
            }))
        })
    }
}

/// Does one effect, then fails.
struct Broken {
    log: Log,
}

impl Component for Broken {
    fn inject(&self) -> BTreeSet<String> {
        keys(&["k"])
    }
    fn provide(&self) -> BTreeSet<String> {
        keys(&["half"])
    }
    fn apply(&mut self, ctx: &mut Ctx) -> Result<()> {
        ctx.provide("half", 1u64)?;
        let log = self.log.clone();
        ctx.effect(move || Ok(Box::new(move || log.lock().unwrap().push("undone".into()))))?;
        Err("no".into())
    }
}

/// A root that does nothing itself.
struct Root;

impl Component for Root {
    fn apply(&mut self, _: &mut Ctx) -> Result<()> {
        Ok(())
    }
}

fn provider(name: &'static str, key: &'static str, value: u64, log: &Log) -> Box<dyn Component> {
    Box::new(Provide {
        name,
        key,
        value,
        inject: &[],
        log: log.clone(),
    })
}

fn user(name: &'static str, inject: &'static [&'static str], log: &Log) -> Box<dyn Component> {
    Box::new(Use {
        name,
        inject,
        log: log.clone(),
    })
}

fn state(reg: &Registry, id: FiberId) -> State {
    reg.status(id).unwrap().state
}

#[test]
fn consumers_follow_their_providers() {
    // Def. 22: activating, deactivating and neutral changes.
    let log = Log::default();
    let mut reg = Registry::default();
    let b = reg.mount(None, "b", user("b", &["k"], &log), vec![]);
    assert_eq!(state(&reg, b), State::Pending);
    assert_eq!(reg.status(b).unwrap().missing, vec!["k".to_string()]);
    let a = reg.mount(None, "a", provider("a", "k", 7, &log), vec![]);
    assert_eq!(state(&reg, b), State::Active);
    assert_eq!(said(&log), ["a on", "b on k=7"]);
    // A provider unrelated to b is neutral to it.
    let c = reg.mount(None, "c", provider("c", "other", 1, &log), vec![]);
    assert_eq!(said(&log), ["c on"]);
    // The consumer stops before the provider's own undo runs (Thm. 63).
    reg.dispose(a);
    assert_eq!(state(&reg, b), State::Inactive);
    assert_eq!(said(&log), ["b off", "a off"]);
    let a = reg.mount(None, "a", provider("a", "k", 8, &log), vec![]);
    assert_eq!(state(&reg, b), State::Active);
    assert_eq!(said(&log), ["a on", "b on k=8"]);
    reg.dispose(b);
    reg.dispose(a);
    reg.dispose(c);
    assert_eq!(said(&log), ["b off", "a off", "c off"]);
    assert_eq!(reg.provided(), 0);
}

#[test]
fn a_new_provider_restarts_its_consumers_even_with_the_same_value() {
    // The committed view holds provider identities, not values.
    let log = Log::default();
    let mut reg = Registry::default();
    let a = reg.mount(None, "a", provider("a", "k", 7, &log), vec![]);
    let b = reg.mount(None, "b", user("b", &["k"], &log), vec![]);
    said(&log);
    reg.update(a, provider("a2", "k", 7, &log));
    assert_eq!(said(&log), ["b off", "a off", "a2 on", "b on k=7"]);
    assert_eq!(state(&reg, b), State::Active);
}

#[test]
fn realms_keep_bots_apart() {
    // Def. 24: each root resolves `signal:fair` in its own realm; the host's
    // `index` resolves in the default realm for both.
    let log = Log::default();
    let mut reg = Registry::default();
    reg.mount(None, "host", provider("host", "index", 100, &log), vec![]);
    let realm = |name: &str| {
        vec![
            ("index".to_string(), String::new()),
            ("*".to_string(), name.to_string()),
        ]
    };
    let one = reg.mount(None, "one", Box::new(Root), realm("one"));
    let two = reg.mount(None, "two", Box::new(Root), realm("two"));
    reg.mount(
        Some(one),
        "fair",
        provider("fair1", "signal:fair", 1, &log),
        vec![],
    );
    reg.mount(
        Some(two),
        "fair",
        provider("fair2", "signal:fair", 2, &log),
        vec![],
    );
    let u1 = reg.mount(
        Some(one),
        "quote",
        user("q1", &["signal:fair", "index"], &log),
        vec![],
    );
    let u2 = reg.mount(
        Some(two),
        "quote",
        user("q2", &["signal:fair", "index"], &log),
        vec![],
    );
    let lines = said(&log);
    assert!(
        lines.contains(&"q1 on signal:fair=1 index=100".to_string()),
        "{lines:?}"
    );
    assert!(
        lines.contains(&"q2 on signal:fair=2 index=100".to_string()),
        "{lines:?}"
    );
    assert_eq!(reg.get::<u64>(u1, "signal:fair"), Some(1));
    assert_eq!(reg.get::<u64>(u2, "signal:fair"), Some(2));
    // Taking one bot down leaves the other running.
    reg.dispose(one);
    assert_eq!(state(&reg, u2), State::Active);
    assert_eq!(reg.provided(), 2);
    // Outside both realms, neither bot's signal is there.
    let outside = reg.mount(None, "outside", user("o", &["signal:fair"], &log), vec![]);
    assert_eq!(state(&reg, outside), State::Pending);
}

#[test]
fn a_failed_start_is_undone_and_tried_again_on_change() {
    // L-Raise: recover first, then record; nothing it did stays.
    let log = Log::default();
    let mut reg = Registry::default();
    let a = reg.mount(None, "a", provider("a", "k", 1, &log), vec![]);
    let watcher = reg.mount(None, "w", user("w", &["half"], &log), vec![]);
    let broken = reg.mount(
        None,
        "broken",
        Box::new(Broken { log: log.clone() }),
        vec![],
    );
    let s = reg.status(broken).unwrap();
    assert_eq!((s.state, s.error.as_deref()), (State::Failed, Some("no")));
    // Its half-provision started the watcher, and undoing it, last first,
    // stopped it again.
    assert_eq!(said(&log), ["a on", "w on half=1", "undone", "w off"]);
    assert_eq!(state(&reg, watcher), State::Inactive);
    // A new provider of what it injects: it tries again (and fails again).
    reg.update(a, provider("a", "k", 2, &log));
    assert_eq!(state(&reg, broken), State::Failed);
    assert_eq!(
        said(&log),
        ["a off", "a on", "w on half=1", "undone", "w off"]
    );
}

#[test]
fn effects_on_a_disposed_fiber_are_refused() {
    let log = Log::default();
    let mut reg = Registry::default();
    let a = reg.mount(None, "a", provider("a", "k", 1, &log), vec![]);
    reg.dispose(a);
    let ran = Arc::new(Mutex::new(false));
    let r = ran.clone();
    let result = reg.effect(a, move || {
        *r.lock().unwrap() = true;
        Ok(Box::new(|| {}))
    });
    assert_eq!(result, Err(INACTIVE_EFFECT.to_string()));
    assert!(!*ran.lock().unwrap());
}

#[test]
fn disposing_a_root_undoes_its_whole_tree_and_its_cleanup_runs_once() {
    // Thm. 61: ending a fiber removes exactly its contribution; the
    // "cancel" stands for what a sender registers (an emission's cleanup).
    let log = Log::default();
    let mut reg = Registry::default();
    reg.mount(None, "host", provider("host", "index", 100, &log), vec![]);
    let root = reg.mount(
        None,
        "bot",
        Box::new(Root),
        vec![("*".into(), "bot".into())],
    );
    let mut loader = Loader::default();
    let rows = [row("w", "weights", &[("x", "1")]), row("q", "quote", &[])];
    let built = build(&log);
    loader.reconcile(&mut reg, root, &rows, &built).unwrap();
    assert_eq!(reg.provided(), 2);
    said(&log);
    reg.dispose(root);
    assert_eq!(said(&log), ["q off", "cancel", "w off"]);
    assert_eq!(reg.provided(), 1);
}

fn row(id: &str, kind: &str, config: &[(&str, &str)]) -> Row {
    Row {
        id: id.into(),
        kind: kind.into(),
        disabled: false,
        config: config
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<BTreeMap<_, _>>(),
    }
}

/// `weights` provides its `x`; `quote` injects it and registers a cancel.
struct Quote {
    log: Log,
}

impl Component for Quote {
    fn inject(&self) -> BTreeSet<String> {
        keys(&["weights"])
    }
    fn apply(&mut self, ctx: &mut Ctx) -> Result<()> {
        let x = ctx.need::<u64>("weights")?;
        let log = self.log.clone();
        ctx.effect(move || Ok(Box::new(move || log.lock().unwrap().push("cancel".into()))))?;
        let log = self.log.clone();
        ctx.effect(move || {
            log.lock().unwrap().push(format!("q on x={x}"));
            Ok(Box::new(move || log.lock().unwrap().push("q off".into())))
        })
    }
}

fn build(log: &Log) -> impl Fn(&Row) -> Result<Box<dyn Component>> + '_ {
    move |r: &Row| -> Result<Box<dyn Component>> {
        match r.kind.as_str() {
            "weights" => {
                let x = r.config["x"].parse().map_err(|_| "x is not a number")?;
                Ok(Box::new(Provide {
                    name: "w",
                    key: "weights",
                    value: x,
                    inject: &[],
                    log: log.clone(),
                }))
            }
            "quote" => Ok(Box::new(Quote { log: log.clone() })),
            other => Err(format!("no kind {other}")),
        }
    }
}

#[test]
fn reconcile_restarts_only_what_changed_and_a_bad_list_changes_nothing() {
    let log = Log::default();
    let mut reg = Registry::default();
    let root = reg.mount(
        None,
        "bot",
        Box::new(Root),
        vec![("*".into(), "bot".into())],
    );
    let mut loader = Loader::default();
    let built = build(&log);
    let mut rows = vec![row("w", "weights", &[("x", "1")]), row("q", "quote", &[])];
    loader.reconcile(&mut reg, root, &rows, &built).unwrap();
    assert_eq!(said(&log), ["w on", "q on x=1"]);
    let fibers: Vec<FiberId> = loader.rows().map(|(_, f)| f).collect();
    // The same list: nothing restarts.
    loader.reconcile(&mut reg, root, &rows, &built).unwrap();
    assert!(said(&log).is_empty());
    // A new weight: its row restarts, and the quote that relied on it.
    rows[0] = row("w", "weights", &[("x", "2")]);
    loader.reconcile(&mut reg, root, &rows, &built).unwrap();
    assert_eq!(said(&log), ["q off", "cancel", "w off", "w on", "q on x=2"]);
    assert_eq!(loader.rows().map(|(_, f)| f).collect::<Vec<_>>(), fibers);
    // A row that does not build: nothing changes.
    rows[0] = row("w", "weights", &[("x", "lots")]);
    let error = loader.reconcile(&mut reg, root, &rows, &built).unwrap_err();
    assert_eq!(error, "w: x is not a number");
    assert!(said(&log).is_empty());
    assert_eq!(reg.get::<u64>(fibers[1], "weights"), Some(2));
    // Two rows of one id are refused, also before anything changes.
    let twice = [row("w", "weights", &[("x", "3")]), row("w", "quote", &[])];
    assert!(loader.reconcile(&mut reg, root, &twice, &built).is_err());
    assert!(said(&log).is_empty());
    // A row taken out is disposed; one disabled counts as taken out.
    rows[0] = row("w", "weights", &[("x", "2")]);
    rows[1].disabled = true;
    loader.reconcile(&mut reg, root, &rows, &built).unwrap();
    assert_eq!(said(&log), ["q off", "cancel"]);
    assert_eq!(loader.rows().count(), 1);
}
