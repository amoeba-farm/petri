use super::{
    input,
    model::{Action, App, Modal, Snapshot, View, safe, usdc},
    nav::Path,
    render,
};
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;

fn app() -> App {
    let mut app = App::new(
        "C:/Petri runtime/node.exe".into(),
        "C:/Petri runtime/research-runtime/agent.mjs".into(),
        "http://127.0.0.1:4185".into(),
    );
    let agent = json!({"id":"ab".repeat(32),"name":"cedar","harness":"Claude Code","payoutAuthority":"public-address","revokedAt":null});
    app.update(serde_json::from_value::<Snapshot>(json!({"agents":[agent],"pairings":[{"id":"12".repeat(16),"code":"ABCD1234","name":"flint","harness":"Codex","status":"pending","error":null}],
      "work":[{"id":"work-1","title":"RAMX sources","brief":"Find a public source with exact product evidence.","objective":{"targets":{"activeSources":3,"distinctHosts":2}},"availability":"available"}],"settings":{"payoutAuthority":"public-address"},
      "directives":[{"id":"directive-1","marketIds":[],"minActiveSources":3,"minDistinctHosts":2,"maxObservationAgeMs":600000,"enabled":true}],
      "selected":{"agent":agent,"events":[{"actor":"agent","kind":"note","message":"Researching a public listing.","referenceUrl":null,"createdAt":1800000000000_u64}],
      "candidates":[],"submissions":[],"validationError":null,"complete":true}})).unwrap());
    app.view = View::Connect;
    app
}
fn draw(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| render::draw(frame, app)).unwrap();
    let buffer = terminal.backend().buffer();
    for hit in &app.hits {
        assert!(
            hit.area.right() <= width && hit.area.bottom() <= height,
            "Out-of-bounds control: {hit:?}"
        );
    }
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
fn click(app: &mut App, action: &Action) {
    let hit = app
        .hits
        .iter()
        .find(|hit| &hit.action == action)
        .expect("Control must be visible")
        .clone();
    let event = Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: hit.area.x,
        row: hit.area.y,
        modifiers: KeyModifiers::NONE,
    });
    let actual = input::handle(app, event).expect("Mouse click must dispatch");
    assert_eq!(&actual, action);
    app.activate(actual);
}

#[test]
fn every_view_and_agent_can_be_selected_by_mouse() {
    let mut app = app();
    for view in View::ALL {
        draw(&mut app, 100, 32);
        click(&mut app, &Action::View(view));
        assert_eq!(app.view, view);
    }
    // Agents are listed beside the research views (not Markets or Fund).
    app.view = View::Connect;
    draw(&mut app, 100, 32);
    click(&mut app, &Action::Agent("ab".repeat(32)));
    assert_eq!(
        app.selected_agent.as_deref(),
        Some("abababababababababababababababababababababababababababababababab")
    );
}
#[test]
fn mouse_pair_approval_and_keyboard_activation_share_the_same_action() {
    let mut mouse = app();
    draw(&mut mouse, 100, 32);
    let action = Action::Approve("12".repeat(16));
    click(&mut mouse, &action);
    assert_eq!(
        mouse.pending.as_ref().unwrap().path,
        "/v1/owner/pairings/approve"
    );
    let mut keyboard = app();
    draw(&mut keyboard, 100, 32);
    keyboard.focus = keyboard
        .hits
        .iter()
        .position(|hit| hit.action == action)
        .unwrap();
    let value = input::handle(
        &mut keyboard,
        Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    )
    .unwrap();
    keyboard.activate(value);
    assert_eq!(keyboard.pending.unwrap().body, mouse.pending.unwrap().body);
}
#[test]
fn editable_forms_support_click_focus_paste_and_save() {
    let mut app = app();
    app.activate(Action::NewAgent);
    draw(&mut app, 100, 32);
    click(&mut app, &Action::Name);
    draw(&mut app, 100, 32);
    click(&mut app, &Action::FormFocus(0));
    for _ in 0.."researcher".len() {
        input::handle(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
        );
    }
    input::handle(&mut app, Event::Paste("scout".into()));
    draw(&mut app, 100, 32);
    click(&mut app, &Action::Save);
    assert_eq!(app.name, "scout");
    assert!(app.modal.is_none());
}

#[test]
fn gaps_read_as_market_month_and_product() {
    use crate::research::nav::short_title;
    use crate::research::render::gap_title;
    let work = |objective: serde_json::Value| {
        serde_json::from_value::<crate::research::model::Work>(
            json!({"id":"w","title":"RAMX · product b0bf2302bd",
            "brief":"","objective":objective}),
        )
        .unwrap()
    };
    let gap = json!({"marketId":"ramx","expiryId":"RAMX-202612","bucketId":"b0bf2302bd593a4b","bucketLabel":null});
    assert_eq!(
        gap_title(&work(gap.clone())),
        "RAMX · Dec 2026 · Unnamed product b0bf23"
    );
    assert_eq!(
        short_title(&gap, "x"),
        "RAMX Dec 2026 · Unnamed product b0bf23"
    );
    assert_eq!(
        gap_title(&work(
            json!({"marketId":"nandx","expiryId":"NANDX-202611","bucketId":"x","bucketLabel":"TLC 512Gb"})
        )),
        "NANDX · Nov 2026 · TLC 512Gb"
    );
    assert_eq!(
        gap_title(&work(
            json!({"marketId":"ramx","expiryId":"RAMX-202701","bucketId":null})
        )),
        "RAMX · Jan 2027 · All products"
    );
    // An update names its page's host.
    assert_eq!(
        short_title(
            &json!({"kind":"update","marketId":"ramx","expiryId":"RAMX-202612","bucketId":"5555",
                "bucketLabel":"DDR5 RDIMM 32GB","url":"https://www.memorymarket.com/price/rdimm"}),
            "x"
        ),
        "RAMX Dec 2026 · DDR5 RDIMM 32GB · memorymarket.com"
    );
    // No objective: the title it came with.
    assert_eq!(gap_title(&work(json!(null))), "RAMX · product b0bf2302bd");
    assert_eq!(short_title(&json!(null), "RAMX sources"), "RAMX sources");
}

/// Two gaps: a warm one, and a hot one the owner picked.
fn research_app() -> App {
    let mut app = app();
    app.view = View::Activity;
    app.snapshot.work = serde_json::from_value(json!([
        {"id":"w-a","title":"RAMX · product b0bf2302bd","brief":"For the agent.","availability":"available",
         "objective":{"marketId":"ramx","expiryId":"RAMX-202612","bucketId":"b0bf2302bd593a4b","bucketLabel":null,
           "activeSources":1,"distinctHosts":1,"targets":{"activeSources":3,"distinctHosts":2},
           "sources":[{"sourceId":"s","status":"Active","url":"https://example.com/ddr5"}]}},
        {"id":"w-b","title":"NANDX · TLC 512Gb","brief":"For the agent.","availability":"available","focused":true,
         "researchers":["ab".repeat(32)],
         "objective":{"marketId":"nandx","expiryId":"NANDX-202611","bucketId":"x","bucketLabel":"TLC 512Gb",
           "activeSources":0,"distinctHosts":0,"targets":{"activeSources":3,"distinctHosts":2},"sources":[]}}
    ]))
    .unwrap();
    app.snapshot.activity = serde_json::from_value(json!([
        {"agentId":"ab".repeat(32),"workId":"w-b","title":"NANDX · TLC 512Gb","startedAt":1800000000000_u64,
         "lastMessage":"Checked a distributor listing."}
    ]))
    .unwrap();
    // The oracle's months the work is in, both taking sources.
    app.fund.snap.months = [("ramx", "RAMX-202612"), ("nandx", "NANDX-202611")]
        .map(|(market, expiry)| crate::research::phase::Month {
            market: market.into(),
            expiry: expiry.into(),
            phase: "SourceSubmission".into(),
            ..Default::default()
        })
        .into();
    app
}

/// Research with its agent idle: the navigator on the products.
fn idle_research_app() -> App {
    let mut app = research_app();
    app.snapshot.activity.clear();
    for work in &mut app.snapshot.work {
        work.researchers.clear();
    }
    app
}

#[test]
fn research_opens_on_one_compact_row_per_product() {
    let mut app = idle_research_app();
    let screen = draw(&mut app, 120, 40);
    for text in [
        "[Research]",
        "Products",
        "Claude Code · cedar",
        "idle",
        "[Follow]",
    ] {
        assert!(screen.contains(text), "missing {text}:\n{screen}");
    }
    let nandx = screen.lines().nth(row_of(&screen, "NANDX")).unwrap();
    assert!(
        nandx.contains("1 month") && nandx.contains("1 task"),
        "{nandx}"
    );
    // One row each, nothing of their months or products yet.
    assert_eq!(
        row_of(&screen, "RAMX "),
        row_of(&screen, "NANDX") + 1,
        "{screen}"
    );
    for gone in [
        "[Products]",
        "[Agent activity]",
        "[Goals]",
        "Products to research",
        "potential earnings",
        "TLC 512Gb",
        "Unnamed product",
        "Dec 2026",
    ] {
        assert!(!screen.contains(gone), "still there: {gone}:\n{screen}");
    }
}

#[test]
fn products_and_months_are_the_oracles_alone() {
    // Withheld work of a month the oracle does not list (NANDX Oct 2026)
    // adds no month, and counts as no task.
    let mut app = idle_research_app();
    let mut gone: Vec<crate::research::model::Work> = serde_json::from_value(json!([
        {"id":"w-c","title":"NANDX · QLC 1Tb","brief":"For the agent.","availability":"unknown",
         "objective":{"marketId":"nandx","expiryId":"NANDX-202610","bucketId":"y","bucketLabel":"QLC 1Tb",
           "activeSources":0,"distinctHosts":0,"targets":{"activeSources":3,"distinctHosts":2},"sources":[]}}
    ]))
    .unwrap();
    app.snapshot.work.append(&mut gone);
    let screen = draw(&mut app, 120, 40);
    let nandx = screen.lines().nth(row_of(&screen, "NANDX")).unwrap();
    assert!(
        nandx.contains("1 month") && nandx.contains("1 task"),
        "{nandx}"
    );
    app.activate(Action::Nav(Path::product("nandx")));
    let screen = draw(&mut app, 120, 40);
    assert!(
        screen.contains("Nov 2026") && !screen.contains("Oct 2026"),
        "{screen}"
    );
    // The oracle stops listing NANDX: no product, though work names it.
    app.fund.snap.months.retain(|m| m.market != "nandx");
    app.activate(Action::Nav(Path::default()));
    let screen = draw(&mut app, 120, 40);
    assert!(!screen.contains("NANDX"), "{screen}");
    // An agent on such work is nowhere in the navigator, its feed's lines
    // lead nowhere, and following it stays put.
    let mut app = feed_app();
    let cedar = "ab".repeat(32);
    assert!(crate::research::nav::place_of(&app, &cedar).is_some());
    app.fund.snap.months.retain(|m| m.market != "nandx");
    assert!(crate::research::nav::place_of(&app, &cedar).is_none());
    assert!(
        crate::research::nav::feed(&app, &cedar)
            .iter()
            .all(|l| l.place.is_none())
    );
    app.activate(Action::Nav(Path::default()));
    app.activate(Action::Follow);
    assert!(app.nav().follow);
    assert_eq!(app.nav().path, Path::default());
}

#[test]
fn a_category_marks_the_agent_on_it_or_your_pick() {
    // Picked, but no agent has it: "○ pick".
    let mut app = idle_research_app();
    app.activate(Action::Nav(Path::month("nandx", "NANDX-202611")));
    let screen = draw(&mut app, 120, 40);
    let line = screen.lines().nth(row_of(&screen, "TLC 512Gb")).unwrap();
    assert!(line.contains("○ pick") && !line.contains("●"), "{line}");
    // An agent on it is named by its harness, at every level.
    let mut app = research_app();
    app.activate(Action::Nav(Path::default()));
    let screen = draw(&mut app, 120, 40);
    let line = screen.lines().nth(row_of(&screen, "NANDX")).unwrap();
    assert!(line.contains("● Claude Code"), "{line}");
    let line = screen.lines().nth(row_of(&screen, "RAMX")).unwrap();
    assert!(!line.contains("●"), "{line}");
    app.activate(Action::Nav(Path::month("nandx", "NANDX-202611")));
    let screen = draw(&mut app, 120, 40);
    let line = screen.lines().nth(row_of(&screen, "TLC 512Gb")).unwrap();
    assert!(
        line.contains("● Claude Code") && line.contains("0/3 src"),
        "{line}"
    );
    // Its detail says who.
    app.activate(Action::Nav(Path::category("nandx", "NANDX-202611", "x")));
    let screen = draw(&mut app, 120, 40);
    assert!(screen.contains("● Claude Code · cedar"), "{screen}");
}

#[test]
fn a_category_opens_in_place_and_sends_your_agents_there() {
    let mut app = idle_research_app();
    app.activate(Action::Nav(Path::category(
        "ramx",
        "RAMX-202612",
        "b0bf2302bd593a4b",
    )));
    let screen = draw(&mut app, 120, 40);
    for text in [
        "Products › RAMX › Dec 2026 › Unnamed product b0bf23",
        "sources 1/3 · sites 1/2",
        "warm",
        "example.com/ddr5",
        "active",
        "[Research this]",
        "[Directive]",
    ] {
        assert!(screen.contains(text), "missing {text}:\n{screen}");
    }
    // No pop-up, and nothing of the old one's sentences.
    assert!(app.modal.is_none());
    for gone in [
        "PRODUCT",
        "price page so far",
        "Already found",
        "Your directive",
        "Give your agent a directive",
        "Close",
        "For the agent.",
    ] {
        assert!(!screen.contains(gone), "still there: {gone}:\n{screen}");
    }
    click(&mut app, &Action::Focus("w-a".into(), true));
    let post = app.pending.take().expect("a request");
    assert_eq!(post.path, "/v1/owner/work/focus");
    assert_eq!(post.body, json!({"workId": "w-a", "focused": true}));
    // A picked one offers to stop, and shows your directive.
    app.snapshot.work[1].guidance =
        json!({"instruction":"Try the vendor's\nown page.","updatedAt":1});
    app.activate(Action::Nav(Path::category("nandx", "NANDX-202611", "x")));
    let screen = draw(&mut app, 120, 40);
    assert!(screen.contains("[Stop researching]"), "{screen}");
    assert!(
        screen.contains("you: Try the vendor's own page."),
        "{screen}"
    );
    click(&mut app, &Action::Focus("w-b".into(), false));
    assert_eq!(
        app.pending.take().unwrap().body,
        json!({"workId": "w-b", "focused": false})
    );
}

#[test]
fn work_refinement_is_clickable_and_keeps_the_exact_objective_id() {
    let mut app = idle_research_app();
    for (width, height) in [(60, 24), (80, 24), (100, 32)] {
        app.modal = None;
        app.activate(Action::Nav(Path::category(
            "ramx",
            "RAMX-202612",
            "b0bf2302bd593a4b",
        )));
        draw(&mut app, width, height);
        click(&mut app, &Action::Refine("w-a".into()));
        let screen = draw(&mut app, width, height);
        assert!(
            screen.contains("RAMX Dec 2026 · Unnamed product"),
            "{screen}"
        );
        assert!(!screen.contains("It reads it before it starts"), "{screen}");
        input::handle(
            &mut app,
            Event::Paste("Find manufacturer sources.\nMatch the region.".into()),
        );
        click(&mut app, &Action::Save);
        let post = app.pending.take().unwrap();
        assert_eq!(post.path, "/v1/owner/work/refine");
        assert_eq!(post.body["workId"], "w-a");
        assert_eq!(
            post.body["instruction"],
            "Find manufacturer sources.\nMatch the region."
        );
    }
}

#[test]
fn the_agent_page_says_in_one_line_what_it_is_on() {
    let mut app = research_app();
    for (width, height) in [(60, 24), (80, 24), (120, 40)] {
        let screen = draw(&mut app, width, height);
        let (_, line) = head(&screen, "Claude Code · cedar");
        assert!(line.contains("NANDX Nov 2026 · TLC"), "{screen}");
        for gone in ["Researching NANDX", "Last:", "Your directive", "for 0 min"] {
            assert!(!screen.contains(gone), "still there: {gone}:\n{screen}");
        }
    }
    // Idle, it says so in a word.
    app.snapshot.activity.clear();
    let screen = draw(&mut app, 120, 40);
    assert!(screen.contains("Claude Code · cedar  idle"), "{screen}");
    assert_eq!(screen.lines().filter(|l| l.contains("idle")).count(), 1);
    assert!(!screen.contains("Not researching right now"), "{screen}");
}

#[test]
fn narrow_layout_keeps_connection_approval_clickable() {
    let mut app = app();
    for (width, height) in [(60, 24), (80, 24), (100, 32), (140, 40)] {
        draw(&mut app, width, height);
        assert!(
            app.hits
                .iter()
                .any(|hit| matches!(hit.action, Action::Approve(_)))
        );
    }
}

#[test]
fn compact_connect_keeps_agent_disconnection_accessible() {
    let mut app = app();
    app.snapshot.pairings.clear();
    draw(&mut app, 80, 24);
    assert!(
        app.hits
            .iter()
            .any(|hit| matches!(hit.action, Action::Revoke(_)))
    );
}
#[test]
fn modal_blocks_clicks_on_background_actions_and_tiny_resize_does_not_panic() {
    let mut app = app();
    app.modal = Some(Modal::Payout(String::new()));
    draw(&mut app, 80, 24);
    assert!(
        !app.hits
            .iter()
            .any(|hit| matches!(hit.action, Action::Approve(_) | Action::View(_)))
    );
    for view in View::ALL {
        app.view = view;
        draw(&mut app, 20, 8);
    }
}
/// The whole frame, styles included.
fn frame(app: &mut App, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| render::draw(frame, app)).unwrap();
    terminal.backend().buffer().clone()
}

#[test]
fn the_pointer_highlights_nothing_behind_a_modal() {
    let mut app = app();
    draw(&mut app, 100, 32);
    let tab = app
        .hits
        .iter()
        .find(|hit| hit.action == Action::View(View::Evidence))
        .expect("the Evidence tab")
        .area;
    let at = |app: &mut App, pointer| {
        app.pointer = pointer;
        frame(app, 100, 32)
    };
    // Without a modal, the pointer highlights the tab.
    assert_ne!(at(&mut app, None), at(&mut app, Some((tab.x, tab.y))));
    // Behind one, the same pointer changes nothing.
    app.modal = Some(Modal::Payout(String::new()));
    assert_eq!(at(&mut app, None), at(&mut app, Some((tab.x, tab.y))));
}

/// A left click at (x, y), dispatched as the TUI does.
fn click_at(app: &mut App, x: u16, y: u16) -> Option<Action> {
    let action = input::handle(
        app,
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        }),
    );
    if let Some(action) = action.clone() {
        app.activate(action);
    }
    action
}

#[test]
fn a_click_outside_a_modal_closes_it_and_inside_does_not() {
    let mut app = app();
    app.modal = Some(Modal::Payout(String::new()));
    draw(&mut app, 100, 32);
    // The modal's box: what the closing areas around it leave out.
    let closing: Vec<_> = app
        .hits
        .iter()
        .filter(|hit| hit.action == Action::Cancel)
        .map(|hit| hit.area)
        .collect();
    let top = closing.iter().find(|a| a.y == 0).expect("above the box");
    let left = closing
        .iter()
        .find(|a| a.x == 0 && a.y == top.height)
        .expect("left of the box");
    // Its top-left corner is inside: no control there, and it stays open.
    assert_eq!(click_at(&mut app, left.width, top.height), None);
    assert!(app.modal.is_some());
    assert_eq!(click_at(&mut app, 0, 0), Some(Action::Cancel));
    assert!(app.modal.is_none());
}

#[test]
fn generated_setup_has_no_owner_credential_and_preserves_paths() {
    let mut app = app();
    app.harness = 1;
    let setup = app.agent_setup();
    // A message for the agent's terminal, with the exact login command in it.
    assert!(setup.starts_with("Connect to my Petri oracle research workspace"));
    assert!(setup.contains("C:/Petri runtime/node.exe"));
    assert!(setup.contains("C:/Petri runtime/research-runtime/agent.mjs"));
    assert!(setup.contains(&app.command()));
    assert!(setup.contains("Codex"));
    assert!(!setup.contains("OWNER_TOKEN"));
}

#[test]
fn a_connected_agents_page_shows_its_wallet_not_its_setup() {
    let mut app = app();
    // The compact layout and the full one.
    for (width, height) in [(80, 24), (100, 32), (120, 48)] {
        let screen = draw(&mut app, width, height);
        for text in [
            "Claude Code · cedar",
            "Connected · idle",
            "Active wallet  public-address",
            "Copy start message",
            "Disconnect agent",
            "flint / Codex",
            "[Approve]",
        ] {
            assert!(screen.contains(text), "missing {text}:\n{screen}");
        }
        for gone in [
            "Copy agent setup",
            "Connect wallet",
            "Rewards go to",
            "Agent: Claude Code",
            "Reward address",
            "Paste it into",
            "to get it researching",
            "Asking to connect",
            "not researching right now",
        ] {
            assert!(!screen.contains(gone), "still there: {gone}:\n{screen}");
        }
    }
    // Researching, it needs no start message.
    app.snapshot.activity = serde_json::from_value(json!([
        {"agentId":"ab".repeat(32),"workId":"work-1","title":"RAMX sources","startedAt":0,"lastMessage":null}
    ]))
    .unwrap();
    let screen = draw(&mut app, 120, 48);
    assert!(
        screen.contains("Connected · researching RAMX sources"),
        "{screen}"
    );
    assert!(!screen.contains("Copy start message"), "{screen}");
}

#[test]
fn the_start_message_gets_a_connected_agent_researching() {
    let app = app();
    let agent = app.snapshot.agents[0].clone();
    let message = app.start_message(&agent);
    assert!(message.contains("as the agent \"cedar\""), "{message}");
    assert!(message.contains("C:/Petri runtime/node.exe"), "{message}");
    assert!(
        message.contains("'work' 'next' '--agent' 'cedar'"),
        "{message}"
    );
    assert!(!message.contains("login"), "{message}");
    // Agents use browser use for signed-in pages; the person signs in.
    assert!(message.contains("use browser use"), "{message}");
    assert!(app.agent_setup().contains("sign in there myself"));
}

#[test]
fn new_agent_sets_up_afresh_from_the_sidebar() {
    let mut app = app();
    for (width, height) in [(80, 24), (100, 32), (120, 48)] {
        app.new_agent = false;
        app.view = View::Activity;
        draw(&mut app, width, height);
        click(&mut app, &Action::NewAgent);
        assert_eq!(app.view, View::Connect);
        let screen = draw(&mut app, width, height);
        assert!(
            screen
                .lines()
                .any(|l| l.contains("New agent") && !l.contains("+ New agent")),
            "{screen}"
        );
        for text in [
            "Agent: Claude Code",
            "Name: researcher",
            "Copy agent setup",
            "Active wallet  public-address  [Change]",
        ] {
            assert!(screen.contains(text), "missing {text}:\n{screen}");
        }
        for gone in [
            "Connect wallet",
            "Rewards go to",
            "Disconnect agent",
            "Connect a new agent",
            "Paste it into",
            "Approve its code here",
            "Asking to connect",
        ] {
            assert!(!screen.contains(gone), "still there: {gone}:\n{screen}");
        }
        // The requests to approve are on it too.
        assert!(
            app.hits
                .iter()
                .any(|hit| matches!(hit.action, Action::Approve(_)))
        );
    }
    // Picking an agent leaves the fresh setup.
    click(&mut app, &Action::Agent("ab".repeat(32)));
    assert!(!app.new_agent);
    // Without a wallet, it asks for one.
    app.activate(Action::NewAgent);
    app.snapshot.settings.payout_authority.clear();
    let screen = draw(&mut app, 120, 48);
    assert!(screen.contains("Connect wallet"), "{screen}");
    assert!(!screen.contains("Active wallet"), "{screen}");
    assert!(!screen.contains("before you approve"), "{screen}");
}

#[test]
fn the_agent_just_approved_is_shown_after_a_fresh_setup() {
    let mut app = app();
    app.activate(Action::NewAgent);
    let mut next = app.snapshot.clone();
    next.agents.push(
        serde_json::from_value(
            json!({"id":"cd".repeat(32),"name":"flint","harness":"Codex",
            "payoutAuthority":"public-address","revokedAt":null}),
        )
        .unwrap(),
    );
    next.pairings.clear();
    app.update(next);
    assert!(!app.new_agent);
    assert_eq!(
        app.selected_agent.as_deref(),
        Some("cd".repeat(32).as_str())
    );
}

#[test]
fn the_sidebar_shows_each_agent_as_one_block_harness_over_name() {
    let mut app = app();
    let screen = draw(&mut app, 100, 32);
    let lines: Vec<&str> = screen.lines().collect();
    let top = lines
        .iter()
        .position(|l| l.contains("▌Claude Code"))
        .unwrap_or_else(|| panic!("{screen}"));
    assert!(lines[top + 1].contains("▌cedar"), "{screen}");
    assert!(!screen.contains("> "), "{screen}");
    assert!(screen.contains("+ New agent"), "{screen}");
}

#[test]
fn copying_the_setup_shows_how_to_paste_it_into_the_agent() {
    let mut app = app();
    app.activate(Action::NewAgent);
    app.activate(Action::Harness(1));
    // mod.rs copies the setup, then shows the steps.
    app.pasting(None);
    for (width, height) in [(80, 24), (100, 32), (120, 48)] {
        let screen = draw(&mut app, width, height);
        for text in [
            "PASTE INTO YOUR AGENT",
            "Copied to your clipboard.",
            "1  Open a chat with Codex.",
            "2  Paste it (",
            "3  Codex shows a pairing code. Approve the same code here.",
            "[Done]",
            "[Copy again]",
        ] {
            assert!(screen.contains(text), "missing {text}:\n{screen}");
        }
    }
    // Copy again copies it again: the steps stay and say so.
    draw(&mut app, 100, 32);
    click(&mut app, &Action::CopyCommand);
    app.pasting(None);
    let screen = draw(&mut app, 100, 32);
    assert!(screen.contains("Copied again."), "{screen}");
    // Enter is Done, and what it said goes with it.
    let key = |code| Event::Key(KeyEvent::new(code, KeyModifiers::NONE));
    let action = input::handle(&mut app, key(KeyCode::Enter)).expect("closes");
    assert_eq!(action, Action::Cancel);
    app.activate(action);
    assert!(app.modal.is_none());
    assert!(app.status.is_empty());
    // Tab, then Enter, is Copy again.
    app.pasting(None);
    assert!(input::handle(&mut app, key(KeyCode::Tab)).is_none());
    assert_eq!(
        input::handle(&mut app, key(KeyCode::Enter)),
        Some(Action::CopyCommand)
    );
    // A click outside closes it.
    draw(&mut app, 100, 32);
    assert_eq!(click_at(&mut app, 0, 0), Some(Action::Cancel));
    assert!(app.modal.is_none());
}

#[test]
fn copying_the_start_message_shows_how_to_paste_it() {
    let mut app = app();
    let id = "ab".repeat(32);
    app.pasting(Some(id.clone()));
    let screen = draw(&mut app, 100, 32);
    for text in [
        "PASTE INTO YOUR AGENT",
        "1  Open your chat with cedar in Claude Code.",
        "2  Paste it (",
        "3  It starts researching.",
    ] {
        assert!(screen.contains(text), "missing {text}:\n{screen}");
    }
    assert!(!screen.contains("pairing code"), "{screen}");
    let key = |code| Event::Key(KeyEvent::new(code, KeyModifiers::NONE));
    assert!(input::handle(&mut app, key(KeyCode::Tab)).is_none());
    assert_eq!(
        input::handle(&mut app, key(KeyCode::Enter)),
        Some(Action::CopyStart(id))
    );
}

#[test]
fn the_agent_is_chosen_in_a_pop_up_by_click_or_keys() {
    let mut app = app();
    app.activate(Action::NewAgent);
    draw(&mut app, 100, 32);
    click(&mut app, &Action::ChooseHarness);
    assert!(matches!(app.modal, Some(Modal::Harness)));
    let screen = draw(&mut app, 100, 32);
    for harness in App::HARNESSES {
        assert!(screen.contains(harness), "missing {harness}:\n{screen}");
    }
    click(&mut app, &Action::Harness(3));
    assert_eq!(app.harness, 3);
    assert!(app.modal.is_none());
    // ↓ then Enter.
    draw(&mut app, 100, 32);
    click(&mut app, &Action::ChooseHarness);
    let key = |code| Event::Key(KeyEvent::new(code, KeyModifiers::NONE));
    assert!(input::handle(&mut app, key(KeyCode::Down)).is_none());
    assert_eq!(app.harness, 4);
    let action = input::handle(&mut app, key(KeyCode::Enter)).expect("closes");
    app.activate(action);
    assert!(app.modal.is_none());
    assert_eq!(app.harness, 4);
}

#[test]
fn connecting_a_wallet_offers_this_petris_wallet() {
    let mut app = app();
    app.snapshot.settings.payout_authority.clear();
    app.markets.snap.owner = Some("D9uvSy7TrPJFjEyhQ9ZfEKKvCi7qmTdywajBRo6YDVwv".into());
    app.activate(Action::Payout);
    assert!(matches!(
        &app.modal,
        Some(Modal::Payout(value)) if value == "D9uvSy7TrPJFjEyhQ9ZfEKKvCi7qmTdywajBRo6YDVwv"
    ));
    let screen = draw(&mut app, 100, 32);
    assert!(screen.contains("CONNECT WALLET"), "{screen}");
    assert!(screen.contains("Petri wallet"), "{screen}");
    for gone in [
        "This is your Petri wallet",
        "Rewards go to it",
        "Esc to cancel",
    ] {
        assert!(!screen.contains(gone), "still there: {gone}:\n{screen}");
    }
}

#[test]
fn coverage_gaps_show_their_potential_earnings_as_a_temperature() {
    use crate::research::render::Heat;
    let work = |objective: serde_json::Value, availability: &str| {
        serde_json::from_value::<crate::research::model::Work>(
            json!({"id":"w","title":"RAMX · DDR5",
            "brief":"","objective":objective,"availability":availability}),
        )
        .unwrap()
    };
    let gap = |sources: u64, hosts: serde_json::Value| {
        work(
            json!({"activeSources":sources,"distinctHosts":hosts,
            "targets":{"activeSources":3,"distinctHosts":2}}),
            "available",
        )
    };
    assert_eq!(Heat::of(&gap(0, json!(0))), Heat::Hot);
    assert_eq!(Heat::of(&gap(1, json!(1))), Heat::Warm);
    assert_eq!(Heat::of(&gap(2, json!(2))), Heat::Lukewarm);
    assert_eq!(Heat::of(&gap(3, json!(2))), Heat::Cold);
    // Unknown hosts leave the sources to decide.
    assert_eq!(Heat::of(&gap(2, json!(null))), Heat::Lukewarm);
    // A gap no longer open has nothing to earn.
    assert_eq!(
        Heat::of(&work(
            json!({"activeSources":0,"targets":{"activeSources":3}}),
            "resolved"
        )),
        Heat::Cold
    );
    // A category shows its hottest task's box; one with nothing to do none.
    let mut app = idle_research_app();
    app.activate(Action::Nav(Path::month("nandx", "NANDX-202611")));
    let screen = draw(&mut app, 120, 40);
    let line = screen.lines().nth(row_of(&screen, "TLC 512Gb")).unwrap();
    assert!(line.contains("0/3 src ██"), "{line}");
    app.snapshot.work[1].availability = "resolved".into();
    let screen = draw(&mut app, 120, 40);
    let line = screen.lines().nth(row_of(&screen, "TLC 512Gb")).unwrap();
    assert!(!line.contains("██"), "{line}");
}

#[test]
fn terminal_content_is_sanitized_and_money_is_formatted_exactly() {
    assert!(!safe("source\u{1b}[31m\u{202e}text").contains('\u{1b}'));
    assert_eq!(
        usdc(&json!("18446744073709551615")).unwrap(),
        "18446744073709.551615 USDC"
    );
    assert_eq!(usdc(&json!("4000000")).unwrap(), "4 USDC");
    assert_eq!(usdc(&json!("18446744073709551616")), None);
}

#[test]
fn workspace_opens_directly_and_controls_keep_spacing() {
    for (width, height) in [(45, 16), (60, 24), (80, 24), (110, 34)] {
        let mut app = app();
        let text = draw(&mut app, width, height);
        assert!(text.contains("PETRI"));
        assert!(!text.contains("S T A R T"));
        for view in View::ALL {
            assert!(app.hits.iter().any(|h| h.action == Action::View(view)));
        }
        let tabs: Vec<_> = app
            .hits
            .iter()
            .filter(|h| matches!(h.action, Action::View(_)))
            .collect();
        for pair in tabs.windows(2) {
            assert!(pair[1].area.x >= pair[0].area.right() + 2);
        }
        assert_eq!(
            input::handle(
                &mut app,
                Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
            ),
            Some(Action::View(View::Activity))
        );
    }
}

/// cedar's feed: one run on NANDX's TLC 512Gb, each kind of event in it.
fn feed_app() -> App {
    let mut app = research_app();
    let cedar = "ab".repeat(32);
    let event = |seq: u64, actor: &str, kind: &str, message: &str, reference: Option<&str>| {
        json!({"sequence":seq,"runId":"r1","actor":actor,"kind":kind,"message":message,
            "referenceUrl":reference,"createdAt":1_800_000_000_000_u64 + seq * 60_000})
    };
    let detail = app.details.get_mut(&cedar).unwrap();
    detail.events = serde_json::from_value(json!([
        event(
            1,
            "petri",
            "run_started",
            "Started: NANDX · TLC 512Gb",
            None
        ),
        event(2, "agent", "search", "tlc 512gb wafer spot price", None),
        event(
            3,
            "agent",
            "read",
            "TLC spot prices",
            Some("https://www.memorymarket.com/price/tlc")
        ),
        event(
            4,
            "agent",
            "value",
            "2.95 USD, TLC 512Gb",
            Some("https://www.memorymarket.com/price/tlc")
        ),
        event(
            5,
            "owner",
            "directive",
            "Check the vendor page first.",
            None
        ),
        event(
            6,
            "agent",
            "source_found",
            "Vendor listing matches.",
            Some("https://vendor.com/tlc")
        ),
        event(7, "agent", "submit", "sponsored 30d7e0f9", None),
        event(
            8,
            "backend",
            "candidate_received",
            "Source candidate received and attributed. Oracle acceptance is pending.",
            Some("https://vendor.com/tlc")
        ),
        event(9, "agent", "note", "Compared two\nlistings.", None),
        event(
            10,
            "agent",
            "thinking",
            "Weighing the second listing.",
            None
        ),
        event(11, "agent", "blocked", "The page needs a login.", None),
        event(12, "petri", "run_finished", "Submitted one source.", None),
    ]))
    .unwrap();
    detail.runs = serde_json::from_value(json!([
        {"id":"r1","workId":"w-b","title":"NANDX · TLC 512Gb","agentId":cedar,"createdAt":0,"finishedAt":null,
         "researchContext":{"objective":{"marketId":"nandx","expiryId":"NANDX-202611","bucketId":"x","bucketLabel":"TLC 512Gb"}}}
    ]))
    .unwrap();
    app
}

#[test]
fn the_feed_shows_each_step_as_one_line_by_kind_newest_last() {
    let mut app = feed_app();
    let screen = draw(&mut app, 120, 40);
    for text in [
        "▸ NANDX Nov 2026 · TLC 512Gb",
        "⌕ tlc 512gb wafer spot price",
        "↗ memorymarket.com/price/tlc",
        "$ 2.95 USD, TLC 512Gb",
        "you: Check the vendor page first.",
        "✓ vendor.com/tlc",
        "⇪ sponsored 30d7e0f9",
        "✓ received",
        "· Compared two listings.",
        "· Weighing the second",
        "! The page needs a login.",
        "■ Submitted one source.",
    ] {
        assert!(screen.contains(text), "missing {text}:\n{screen}");
    }
    for gone in [
        "TIME UTC",
        "Source candidate received",
        "Oracle acceptance is pending",
        "Started:",
        "Run started",
        "Agent activity",
    ] {
        assert!(!screen.contains(gone), "still there: {gone}:\n{screen}");
    }
    // Oldest at the top, newest at the bottom, one row each.
    let started = row_of(&screen, "▸ NANDX");
    assert_eq!(row_of(&screen, "■ Submitted"), started + 11, "{screen}");
    // A line of a run shows its work in the navigator.
    app.activate(Action::Nav(Path::default()));
    draw(&mut app, 120, 40);
    let line = app
        .hits
        .iter()
        .filter(|h| h.action == Action::Nav(Path::category("nandx", "NANDX-202611", "x")))
        .count();
    assert_eq!(line, 12, "each of the run's lines");
    let feed = app.feed_area;
    assert_eq!(
        click_at(&mut app, feed.x + 2, feed.y),
        Some(Action::Nav(Path::category("nandx", "NANDX-202611", "x")))
    );
    assert_eq!(app.nav().path, Path::category("nandx", "NANDX-202611", "x"));
    // The narrow layout keeps the feed under the navigator.
    let narrow = draw(&mut app, 80, 24);
    assert!(narrow.contains("■ Submitted one"), "{narrow}");
    assert!(
        row_of(&narrow, "■ Submitted") > row_of(&narrow, "Products ›"),
        "{narrow}"
    );
}

#[test]
fn the_feed_keeps_its_newest_in_view_unless_scrolled_up() {
    let mut app = feed_app();
    let cedar = "ab".repeat(32);
    let more: Vec<crate::research::model::Event> = (13..60)
        .map(|seq| {
            serde_json::from_value(json!({"sequence":seq,"runId":"r1","actor":"agent","kind":"note",
                "message":format!("step {seq}"),"referenceUrl":null,"createdAt":1_800_000_000_000_u64}))
            .unwrap()
        })
        .collect();
    app.details.get_mut(&cedar).unwrap().events.extend(more);
    let screen = draw(&mut app, 120, 40);
    assert!(screen.contains("step 59"), "{screen}");
    assert!(!screen.contains("▸ NANDX"), "{screen}");
    // Scrolled up, it stays where it is as events arrive.
    let wheel = |app: &mut App, kind| {
        let at = app.feed_area;
        input::handle(
            app,
            Event::Mouse(MouseEvent {
                kind,
                column: at.x + 1,
                row: at.y + 1,
                modifiers: KeyModifiers::NONE,
            }),
        );
    };
    for _ in 0..30 {
        wheel(&mut app, MouseEventKind::ScrollUp);
    }
    let screen = draw(&mut app, 120, 40);
    assert!(screen.contains("▸ NANDX"), "{screen}");
    assert!(!screen.contains("step 59"), "{screen}");
    app.details.get_mut(&cedar).unwrap().events.push(
        serde_json::from_value(
            json!({"sequence":60,"runId":"r1","actor":"agent","kind":"note",
            "message":"step 60","referenceUrl":null,"createdAt":1_800_000_000_000_u64}),
        )
        .unwrap(),
    );
    let screen = draw(&mut app, 120, 40);
    assert!(
        screen.contains("▸ NANDX") && !screen.contains("step 60"),
        "{screen}"
    );
    // "▾ newer" brings the newest back, and keeps it there.
    click(&mut app, &Action::Newest);
    let screen = draw(&mut app, 120, 40);
    assert!(screen.contains("step 60"), "{screen}");
    // So does scrolling back down.
    wheel(&mut app, MouseEventKind::ScrollUp);
    draw(&mut app, 120, 40);
    for _ in 0..5 {
        wheel(&mut app, MouseEventKind::ScrollDown);
    }
    assert_eq!(app.nav().feed_top, None);
}

fn markets_app() -> App {
    let mut app = App::new(String::new(), String::new(), String::new());
    let market = json!({"market":"CgB3mTWzDVfTmYtgKjTXUwGSSPd39bPsCk7e7RbCxuqg","symbol":"RAMX","label":"RAMX Oct 26",
        "expiryTs":1793491200u64,"status":"open","tradable":true,"grid":200,"k":"10000","stepValue":"1",
        "valueDecimals":6,"lotUsdPerPoint":"0.01","index":{"value":"102.05","ts":1791794605},
        "lastEntryValue":"103.31","change24hValue":"-0.4","claimsOpen":12,
        "suggestion":{"lo":93,"hi":132,"lowValue":"93","highValue":"132"}});
    app.markets.snap.markets = vec![serde_json::from_value(market).unwrap()];
    app.markets.snap.book = Some(
        serde_json::from_value(json!({"lo":93,"hi":132,"loValue":"93","hiValue":"132",
        "asks":[{"premium":"103400","entryValue":"103.34","lots":10,"cumulativeLots":10}],
        "bids":[{"premium":"98000","entryValue":"102.8","lots":15,"cumulativeLots":15}]}))
        .unwrap(),
    );
    app.markets.snap.trades = serde_json::from_value(json!([{"signature":"sig","slot":5,"ts":1791794572,"side":"sell",
        "lo":93,"hi":132,"loValue":"93","hiValue":"132","lots":5,"premium":"490000","entryValue":"102.8",
        "sources":"ladder","taker":"t"}])).unwrap();
    app.markets.snap.closes = vec![103.3, 103.0, 102.8];
    app.markets.snap.owner = Some("owner".into());
    app.markets.snap.account = Some(serde_json::from_value(json!({"owner":"owner","usdc":"9000000000","markets":[{
        "market":"CgB3mTWzDVfTmYtgKjTXUwGSSPd39bPsCk7e7RbCxuqg","symbol":"RAMX","label":"RAMX Oct 26","status":"open",
        "tradable":true,"markValue":"102.05","trader":{"slot":3,"free":"5000000","reserved":"2040000","paidOut":"0"},
        "orders":[{"slot":0,"seq":"9","side":"short","lo":93,"hi":132,"loValue":"93","hiValue":"132","lots":2,
          "placedLots":2,"price":"110000","entryValue":"104","escrow":"580000","close":false,"claim":null,"placedTs":1}],
        "claims":[{"slot":4,"seq":"17","side":"long","lo":92,"hi":142,"loValue":"92","hiValue":"142","lots":3,
          "cost":"246000","reserved":0,"entryValue":"100.2","openedTs":1791794572,"maxPayout":"1500000",
          "payoutAtIndex":"301500","pnlAtIndex":"-1200000",
          "exit":{"lots":3,"premium":"290000","net":"290000","entryValue":"101.67"}}]}]})).unwrap());
    app.markets.loaded = true;
    app
}

#[test]
fn markets_view_shows_book_trades_positions_and_orders() {
    let mut app = markets_app();
    assert_eq!(app.view, View::Markets);
    let screen = draw(&mut app, 120, 40);
    for text in [
        "Capped futures",
        "MARKETS",
        "RAMX Oct 26",
        "price",
        "103.31",
        "index",
        "102.05",
        "BOOK",
        "103.34",
        "102.80",
        "93–132",
        "TRADES",
        "RANGE",
        "POSITIONS",
        "free $5.00",
        "wallet $9,000.00",
        "#4",
        "12x",
        "P&L -$1.20",
        "close $0.29",
        "ORDERS",
        "#0",
        "holds $0.58",
        "b/s",
        "Long/Short",
    ] {
        assert!(screen.contains(text), "missing {text}:\n{screen}");
    }
}

/// Three markets: the fixture's, then two more under other names.
fn three_markets() -> App {
    let mut app = markets_app();
    for label in ["NANDX Oct 26", "RAMX Nov 26"] {
        let mut market = app.markets.snap.markets[0].clone();
        market.label = label.into();
        app.markets.snap.markets.push(market);
    }
    app
}

#[test]
fn a_click_on_a_market_in_the_list_shows_it() {
    let mut app = three_markets();
    draw(&mut app, 120, 40);
    click(&mut app, &Action::Market(2));
    assert_eq!(app.markets.selected, 2);
    let screen = draw(&mut app, 120, 40);
    assert!(screen.contains("RAMX Nov 26  open"), "{screen}");
    click(&mut app, &Action::Market(1));
    assert_eq!(app.markets.selected, 1);
}

#[test]
fn the_price_chart_fills_its_rows_with_the_high_and_low_beside_it() {
    let mut app = markets_app();
    app.markets.snap.closes = (0..40).map(|i| 100.0 + f64::from(i % 7)).collect();
    let screen = draw(&mut app, 120, 40);
    assert!(screen.contains("106.00"), "the high:\n{screen}");
    assert!(screen.contains("100.00"), "the low:\n{screen}");
    let chart_rows = screen.lines().filter(|line| line.contains('█')).count();
    assert!(chart_rows >= 3, "a chart several rows high:\n{screen}");
}

const UNREACHABLE: &str = "futures API http://127.0.0.1:4500 is unreachable (/v1/markets): \
    error sending request for url (http://127.0.0.1:4500/v1/markets)";

fn failed_read() -> crate::research::markets::Snapshot {
    crate::research::markets::Snapshot {
        error: Some(UNREACHABLE.into()),
        ..Default::default()
    }
}

#[test]
fn an_unreachable_api_keeps_the_markets_shown_and_says_only_offline() {
    let mut app = markets_app();
    let good = app.markets.snap.clone();
    app.markets.take_snapshot(failed_read());
    let screen = draw(&mut app, 120, 40);
    assert!(
        screen.contains("RAMX Oct 26") && screen.contains("103.31"),
        "{screen}"
    );
    assert!(screen.contains("offline"), "{screen}");
    assert!(
        !screen.contains("unreachable") && !screen.contains("127.0.0.1"),
        "{screen}"
    );
    app.markets.take_snapshot(good);
    assert!(!draw(&mut app, 120, 40).contains("offline"));
}

#[test]
fn markets_never_read_show_no_loading_text_then_a_plain_line() {
    let mut app = App::new(String::new(), String::new(), String::new());
    let screen = draw(&mut app, 120, 40);
    assert!(!screen.contains("Loading"), "{screen}");
    app.markets.take_snapshot(failed_read());
    let screen = draw(&mut app, 120, 40);
    assert!(screen.contains("offline"), "{screen}");
    assert!(!screen.contains("aren't available"), "{screen}");
    assert!(
        !screen.contains("unreachable") && !screen.contains("127.0.0.1"),
        "{screen}"
    );
}

#[test]
fn an_unreachable_api_or_rpc_reads_as_one_plain_sentence() {
    use crate::research::markets::plain;
    let plain_line = "Couldn't reach Amoeba.";
    assert_eq!(plain(UNREACHABLE.into()), plain_line);
    assert_eq!(
        plain("RPC http://127.0.0.1:8899 is unreachable: connection refused".into()),
        plain_line
    );
    assert_eq!(
        plain("slippage: the price moved".into()),
        "slippage: the price moved"
    );
}

#[test]
fn a_click_outside_the_ticket_closes_it() {
    let mut app = three_markets();
    draw(&mut app, 120, 40);
    let key = |code| Event::Key(KeyEvent::new(code, KeyModifiers::NONE));
    assert!(input::handle(&mut app, key(KeyCode::Char('b'))).is_none());
    draw(&mut app, 120, 40);
    // A market in the list is outside the ticket: a click there closes the
    // ticket and selects nothing.
    let list = app
        .hits
        .iter()
        .find(|hit| hit.action == Action::Market(2))
        .expect("the list")
        .area;
    assert_eq!(
        click_at(&mut app, list.x, list.y),
        Some(Action::CloseTicket)
    );
    assert!(app.markets.ticket.is_none());
    assert_eq!(app.markets.selected, 0);
}

/// The Fund view with one find (already on the oracle), a balance, and one
/// source this wallet listed.
fn fund_app() -> App {
    use crate::research::fund::snapshot_from;
    let mut app = app();
    app.view = View::Fund;
    let candidates = json!({"candidates":[{"contentHash":"c".repeat(64),"agentId":"ab".repeat(32),
        "canonicalLocatorHash":"AA11","createdAt":1_000_000u64,"expiresAt":2_000_000u64,
        "details":{"marketId":"ramx","expiryId":"RAMX-202611","bucketId":"a165995eac5c","canonicalUrl":"https://www.example.com/ddr5",
          "productDefinition":"DDR5 16Gb module","priceField":"spot","region":"US","currency":"USD","evidence":"Listed price"}}]});
    let oracle = json!({"sourceLinks":[{"marketId":"nandx","expiryId":"NANDX-202610","sources":[
        {"canonicalLocator":{"url":"https://example.com/ddr5","hash":"aa11"},"proposer":"other","status":"Candidate","bucketId":"x"},
        {"canonicalLocator":{"url":"https://vendor.com/tlc","hash":"bb22"},"proposer":"me","status":"Active","bucketId":"y","bucketLabel":"TLC 512Gb"}]}]});
    let mut snap = snapshot_from(&candidates, &oracle, Some("me"));
    snap.balance = Some((12_000_000, 1_000_000));
    app.fund.take_snapshot(snap);
    app
}

#[test]
fn finds_read_plainly_and_know_when_they_are_on_the_oracle() {
    use crate::research::fund::{ago, dollars, host};
    let app = fund_app();
    let find = &app.fund.snap.finds[0];
    assert_eq!(find.title, "RAMX · Nov 2026 · Unnamed product a16599");
    assert_eq!(find.price, "spot · US · USD");
    assert_eq!(find.on_oracle.as_deref(), Some("Candidate"));
    assert_eq!(app.fund.snap.mine.len(), 1);
    assert_eq!(app.fund.snap.mine[0].title, "NANDX · Oct 2026 · TLC 512Gb");
    assert_eq!(dollars(12_345_678), "$12.34");
    assert_eq!(ago(10 * 60_000, 0), "10 min ago");
    assert_eq!(ago(3 * 86_400_000, 0), "3 d ago");
    assert_eq!(host("https://www.example.com/a?b"), "example.com");
}

#[test]
fn the_fund_view_shows_the_balance_the_finds_and_your_sources() {
    let mut app = fund_app();
    let screen = draw(&mut app, 120, 40);
    for text in [
        "[Fund]",
        "Oracle balance $12.00 · $1.00 locked   [Add funds]  [Withdraw]",
        "New finds",
        "RAMX · Nov 2026 · Unnamed product a16599 · example.com",
        "● candidate",
        "Your sources",
        "NANDX · Oct 2026 · TLC 512Gb · vendor.com",
    ] {
        assert!(screen.contains(text), "missing {text}:\n{screen}");
    }
    for gone in [
        "Sponsor a find to put it on the oracle",
        "held in bonds",
        "New finds from researchers",
        "show here",
    ] {
        assert!(!screen.contains(gone), "still there: {gone}:\n{screen}");
    }
    // f opens it from elsewhere.
    app.view = View::Connect;
    draw(&mut app, 120, 40);
    let key = |code| Event::Key(KeyEvent::new(code, KeyModifiers::NONE));
    assert_eq!(
        input::handle(&mut app, key(KeyCode::Char('f'))),
        Some(Action::View(View::Fund))
    );
}

#[test]
fn a_find_opens_what_it_is_and_who_found_it() {
    let mut app = fund_app();
    draw(&mut app, 120, 40);
    click(&mut app, &Action::ShowFind(0));
    let screen = draw(&mut app, 120, 40);
    for text in [
        "FOUND BY A RESEARCHER",
        "https://www.example.com/ddr5",
        "DDR5 16Gb module",
        "spot · US · USD",
        "your agent cedar",
        "Oracle     candidate",
        "Close",
    ] {
        assert!(screen.contains(text), "missing {text}:\n{screen}");
    }
    assert!(!screen.contains("Save"), "{screen}");
    assert_eq!(click_at(&mut app, 0, 0), Some(Action::Cancel));
    assert!(app.modal.is_none());
}

#[test]
fn a_find_not_yet_on_the_oracle_can_be_sponsored_after_a_review() {
    use crate::research::fund::{Kind, Reply, Sponsored, Stage};
    let mut app = fund_app();
    app.fund.snap.finds[0].on_oracle = None;
    draw(&mut app, 120, 40);
    click(&mut app, &Action::ShowFind(0));
    let screen = draw(&mut app, 120, 40);
    assert!(
        screen.contains("Sponsor it") && screen.contains("Oracle     not yet"),
        "{screen}"
    );
    assert!(!screen.contains("posts its bond"), "{screen}");
    // The keyboard reaches it too: Tab moves to Close and back, Enter sponsors.
    let hash = app.fund.snap.finds[0].content_hash.clone();
    let key = |code| Event::Key(KeyEvent::new(code, KeyModifiers::NONE));
    assert_eq!(input::handle(&mut app, key(KeyCode::Tab)), None);
    assert_eq!(app.form_focus, 1);
    assert_eq!(input::handle(&mut app, key(KeyCode::Tab)), None);
    assert_eq!(
        input::handle(&mut app, key(KeyCode::Enter)),
        Some(Action::Sponsor(hash.clone()))
    );
    click(&mut app, &Action::Sponsor(hash.clone()));
    assert!(app.modal.is_none());
    let ticket = app.fund.ticket.clone().expect("a ticket");
    assert_eq!(ticket.kind, Kind::Sponsor(hash.clone()));
    // No amount to type: the product's bond is exact, so the review comes first.
    assert_eq!(ticket.stage, Stage::Working);
    // The whole review shows, however its lines wrap.
    app.fund.reply(Reply::Prepared(Ok(vec![
        "Sponsor DDR5 for RAMX-202611: a 1 USDC bond".into(),
        "RAMX-202611 (RAMX Nov 26, period forming, month 71M9UmvwyueA9YTKGmQ4mX73oryTtoB7zPRzP2aS12j2, phase SourceSubmission)".into(),
        "Page https://www.example-retailer.com/p/ddr5-16gb-module".into(),
        "Found by agent dad8c9cc (pays ovPY16HeRpECKKjkVQhBJF4Z27doQwAdNSXKqEykTDN)".into(),
        "The bond comes from your oracle collateral and comes back when the source settles, unless a challenger proves it wrong".into(),
        "If it goes live, the researcher gets half its reward (a fifth of that held in their reserve) and you the rest".into(),
        "Evidence: locator HMVgceSJmkhvJhHbXUNM9KZh1ymZMH6sfNLNYkzPFUGm, definition 7BdRVMRx4mEisG1YEjyHwxvTAPXWJ3XdHPHLmLsFBEyb".into(),
    ])));
    let screen = draw(&mut app, 120, 40);
    assert!(
        screen.contains("SPONSOR")
            && screen.contains("a 1 USDC bond")
            && screen.contains("and you the rest")
            && screen.contains("7BdRVMRx4mEisG1YEjyHwxvTAPXWJ3XdHPHLmLsFBEyb")
            && screen.contains("y/Enter send")
            && !screen.contains("signs it on this computer"),
        "{screen}"
    );
    // Your sponsorships show how they are doing.
    app.fund.ticket = None;
    app.fund.snap.sponsored = vec![Sponsored {
        title: "RAMX · Nov 2026 · DDR5".into(),
        bond: 1_000_000,
        outcome: "waiting".into(),
        reward_paid: 0,
    }];
    let screen = draw(&mut app, 120, 40);
    assert!(
        screen.contains("Your sponsorships")
            && screen.contains("RAMX · Nov 2026 · DDR5  $1.00 bond · waiting"),
        "{screen}"
    );
    // A find this wallet sponsored is on the oracle, even in a month the API does not list.
    crate::research::fund::mark_sponsored(&mut app.fund.snap.finds, &[hash]);
    assert_eq!(
        app.fund.snap.finds[0].on_oracle.as_deref(),
        Some("Sponsored by you")
    );
    click(&mut app, &Action::ShowFind(0));
    let screen = draw(&mut app, 120, 40);
    assert!(
        screen.contains("Oracle     sponsored by you") && !screen.contains("Sponsor it"),
        "{screen}"
    );
}

#[test]
fn adding_funds_reviews_before_the_wallet_signs() {
    use crate::research::fund::{Kind, Reply, Stage};
    let mut app = fund_app();
    draw(&mut app, 120, 40);
    click(&mut app, &Action::FundTicket(Kind::Deposit));
    let key = |code| Event::Key(KeyEvent::new(code, KeyModifiers::NONE));
    for c in ['1', '2', '.', '5', 'x'] {
        input::handle(&mut app, key(KeyCode::Char(c)));
    }
    assert_eq!(app.fund.ticket.as_ref().unwrap().amount, "12.5");
    let screen = draw(&mut app, 120, 40);
    assert!(screen.contains("Add $12.5"), "{screen}");
    assert!(
        !screen.contains("Bonds and stakes come from it"),
        "{screen}"
    );
    input::handle(&mut app, key(KeyCode::Enter));
    assert_eq!(app.fund.ticket.as_ref().unwrap().stage, Stage::Working);
    app.fund.reply(Reply::Prepared(Ok(vec![
        "Deposit 12.5 USDC into oracle collateral".into(),
    ])));
    let screen = draw(&mut app, 120, 40);
    assert!(
        screen.contains("Deposit 12.5 USDC") && screen.contains("y/Enter send"),
        "{screen}"
    );
    input::handle(&mut app, key(KeyCode::Char('y')));
    assert_eq!(app.fund.ticket.as_ref().unwrap().stage, Stage::Working);
    app.fund.reply(Reply::Sent(Ok(vec![
        "Deposit 5.00 USDC into oracle collateral".into(),
        "Added.".into(),
    ])));
    assert!(draw(&mut app, 120, 40).contains("Added."));
    input::handle(&mut app, key(KeyCode::Char(' ')));
    assert!(app.fund.ticket.is_none());
    // Withdraw: a for everything; a click outside closes it.
    draw(&mut app, 120, 40);
    click(&mut app, &Action::FundTicket(Kind::Withdraw));
    input::handle(&mut app, key(KeyCode::Char('a')));
    assert_eq!(app.fund.ticket.as_ref().unwrap().amount, "all");
    draw(&mut app, 120, 40);
    assert_eq!(click_at(&mut app, 0, 0), Some(Action::CloseFundTicket));
    assert!(app.fund.ticket.is_none());
}

#[test]
fn markets_keys_open_a_ticket_and_never_send_without_a_quote() {
    let mut app = markets_app();
    draw(&mut app, 120, 40);
    let key = |code| Event::Key(KeyEvent::new(code, KeyModifiers::NONE));
    assert!(input::handle(&mut app, key(KeyCode::Char('b'))).is_none());
    let ticket = app.markets.ticket.clone().expect("long ticket");
    // The site's default leverage, 10x, fits a long from 103.31.
    assert!(matches!(
        ticket.kind,
        crate::research::markets::Kind::Take {
            lots: 1,
            leverage: 10,
            ..
        }
    ));
    input::handle(&mut app, key(KeyCode::Up));
    input::handle(&mut app, key(KeyCode::Right));
    let ticket = app.markets.ticket.clone().unwrap();
    assert!(matches!(
        ticket.kind,
        crate::research::markets::Kind::Take {
            lots: 2,
            leverage: 20,
            ..
        }
    ));
    // Tab switches to a short; 20x fits it too.
    input::handle(&mut app, key(KeyCode::Tab));
    assert!(matches!(
        app.markets.ticket.as_ref().unwrap().kind,
        crate::research::markets::Kind::Take {
            side: crate::futures::args::SideArg::Short,
            leverage: 20,
            ..
        }
    ));
    input::handle(&mut app, key(KeyCode::Tab));
    // `y` does nothing before a quote; Esc closes the ticket.
    input::handle(&mut app, key(KeyCode::Char('y')));
    assert_eq!(
        app.markets.ticket.as_ref().unwrap().stage,
        crate::research::markets::Stage::Edit
    );
    let screen = draw(&mut app, 120, 40);
    assert!(
        screen.contains("LONG") && screen.contains("Enter quote"),
        "{screen}"
    );
    for gone in ["nothing is sent yet", "edit to quote again", "Quoting"] {
        assert!(!screen.contains(gone), "still there: {gone}:\n{screen}");
    }
    input::handle(&mut app, key(KeyCode::Esc));
    assert!(app.markets.ticket.is_none());
    // x opens a close for the selected position; d a cancel for the selected order.
    input::handle(&mut app, key(KeyCode::Char('x')));
    assert!(matches!(
        app.markets.ticket.as_ref().unwrap().kind,
        crate::research::markets::Kind::Close {
            slot: 4,
            lots: 3,
            max: 3
        }
    ));
    input::handle(&mut app, key(KeyCode::Up));
    assert!(matches!(
        app.markets.ticket.as_ref().unwrap().kind,
        crate::research::markets::Kind::Close { lots: 3, .. }
    ));
    input::handle(&mut app, key(KeyCode::Esc));
    input::handle(&mut app, key(KeyCode::Char('d')));
    assert!(matches!(
        app.markets.ticket.as_ref().unwrap().kind,
        crate::research::markets::Kind::Cancel { slot: 0, .. }
    ));
}

#[test]
fn rewards_chart_each_month_and_name_each_sponsored_find() {
    use crate::research::rewards::{Earning, by_month, researcher_share};
    // Half the bounty is the researcher's; its reserve keeps a fifth of that.
    assert_eq!(researcher_share(10_000_000, 5_000, 2_000), 5_000_000);
    let mut app = app();
    app.view = View::Rewards;
    // Before the first read, no figure at all.
    let screen = draw(&mut app, 120, 40);
    assert!(!screen.contains("Earned"), "{screen}");
    assert!(!screen.contains("once its finds pay out"), "{screen}");
    app.rewards.by_agent.insert("ab".repeat(32), Vec::new());
    let screen = draw(&mut app, 120, 40);
    for text in [
        "Claude Code · cedar",
        "Earned  0 USDC   0 finds · 0 sponsored",
    ] {
        assert!(screen.contains(text), "missing {text}:\n{screen}");
    }
    // Nothing earned yet: no chart, no list, no sentence about them.
    for gone in [
        "Rewards go to",
        "USDC per month",
        "once its finds pay out",
        "None yet",
        "Sponsored finds",
    ] {
        assert!(!screen.contains(gone), "still there: {gone}:\n{screen}");
    }
    let earning = |title: &str, expiry: &str, outcome: &str, earned| Earning {
        title: title.into(),
        expiry: expiry.into(),
        outcome: outcome.into(),
        earned,
    };
    let earnings = vec![
        earning(
            "RAMX · Nov 2026 · DDR5 16Gb",
            "RAMX-202611",
            "live, reward paid",
            2_500_000,
        ),
        earning("NANDX · Dec 2026 · TLC 512Gb", "NANDX-202612", "waiting", 0),
        earning(
            "RAMX · Nov 2026 · DDR4 8Gb",
            "RAMX-202611",
            "live, reward paid",
            1_000_000,
        ),
    ];
    assert_eq!(
        by_month(&earnings),
        vec![("Nov 2026".into(), 3_500_000), ("Dec 2026".into(), 0)]
    );
    app.rewards.by_agent.insert("ab".repeat(32), earnings);
    let screen = draw(&mut app, 120, 40);
    for text in [
        "Earned  3.5 USDC",
        "USDC per month",
        "Nov 2026",
        "Dec 2026",
        "3.5",
        "Sponsored finds",
        "RAMX · Nov 2026 · DDR5 16Gb  · live, reward paid  · 2.5 USDC",
        "NANDX · Dec 2026 · TLC 512Gb  · waiting",
        "3 sponsored",
    ] {
        assert!(screen.contains(text), "missing {text}:\n{screen}");
    }
    // A failed read keeps what is shown, tagged offline on the heading.
    app.rewards.offline = true;
    let screen = draw(&mut app, 120, 40);
    assert!(screen.contains("Claude Code · cedar  offline"), "{screen}");
    assert!(!screen.contains("showing the last read"), "{screen}");
    // The list keeps room under the chart in a small window.
    assert!(draw(&mut app, 80, 24).contains("Sponsored finds"));
    draw(&mut app, 60, 24);
}

#[test]
fn a_market_read_before_shows_at_once_and_another_never_shows_its_data() {
    let mut app = three_markets();
    for (i, m) in app.markets.snap.markets.iter_mut().enumerate() {
        m.market = format!("market-{i}");
    }
    let read = |app: &App, market: &str, closes: Vec<f64>| {
        let mut snap = app.markets.snap.clone();
        snap.detail_for = Some(market.into());
        snap.closes = closes;
        snap
    };
    let shown = read(&app, "market-0", vec![100.0, 101.0]);
    app.markets.take_snapshot(shown);
    // A background read of another market leaves the shown one as it is.
    let other = read(&app, "market-2", vec![7.0, 8.0, 9.0]);
    app.markets.take_snapshot(other);
    assert_eq!(app.markets.snap.closes, vec![100.0, 101.0]);
    // Clicking it shows what was read, before any new read.
    app.markets.show(2);
    assert_eq!(app.markets.snap.closes, vec![7.0, 8.0, 9.0]);
    // One never read shows nothing of another's.
    app.markets.show(1);
    assert!(app.markets.snap.closes.is_empty());
    assert!(app.markets.snap.book.is_none());
    // A failed read keeps the last good one.
    let mut failed = read(&app, "market-2", vec![]);
    failed.book = None;
    app.markets.take_snapshot(failed);
    app.markets.show(2);
    assert_eq!(app.markets.snap.closes, vec![7.0, 8.0, 9.0]);
}

#[test]
fn switching_agents_shows_each_ones_last_read_at_once() {
    let mut app = app();
    let mut next = app.snapshot.clone();
    let birch: crate::research::model::Agent = serde_json::from_value(
        json!({"id":"cd".repeat(32),"name":"birch","harness":"Codex","payoutAuthority":"public-address","revokedAt":null}),
    )
    .unwrap();
    next.agents.push(birch.clone());
    // birch (not the one asking to connect), read in the background while cedar is shown.
    let mut detail = app.details[&"ab".repeat(32)].clone();
    detail.agent = birch;
    detail.events[0].message = "Birch checked a distributor.".into();
    next.selected = Some(detail);
    app.update(next);
    assert_eq!(
        app.selected_agent.as_deref(),
        Some("ab".repeat(32).as_str())
    );
    app.view = View::Activity;
    draw(&mut app, 120, 40);
    click(&mut app, &Action::Agent("cd".repeat(32)));
    let screen = draw(&mut app, 120, 40);
    assert!(screen.contains("Birch checked a distributor."), "{screen}");
}

#[test]
fn research_views_draw_nothing_before_the_first_read() {
    let mut app = App::new(String::new(), String::new(), String::new());
    for view in [View::Connect, View::Activity, View::Evidence, View::Rewards] {
        app.view = view;
        let screen = draw(&mut app, 100, 32);
        for wrong in [
            "Connect your agent",
            "No agents yet",
            "Connect an agent",
            "Select an agent",
            "+ New agent",
        ] {
            assert!(
                !screen.contains(wrong),
                "{wrong} before any read:\n{screen}"
            );
        }
    }
}

/// RAMX Jan 2027 taking sources with $520 of rewards (two of its 13
/// products), RAMX Dec 2026 in Game with none (two sources); a Jan 2027
/// product to research and two Dec 2026 updates.
fn phases_app() -> App {
    use crate::research::phase::{Month, Sku, Source};
    let mut app = research_app();
    let day = 86_400;
    let now = 1_792_300_000;
    app.fund.snap.chain_now = Some(now);
    let source = |bucket: &str, label: &str, url: &str| Source {
        bucket: bucket.into(),
        label: Some(label.into()),
        url: Some(url.into()),
        status: "Active".into(),
        ..Default::default()
    };
    app.fund.snap.months = vec![
        Month {
            market: "ramx".into(),
            expiry: "RAMX-202612".into(),
            phase: "Game".into(),
            scramble_start: Some(1_790_121_600),
            sources: vec![
                source(
                    "5555",
                    "DDR4 UDIMM 8GB 3200",
                    "https://www.memorymarket.com/price/udimm",
                ),
                source(
                    "6666",
                    "DDR5 RDIMM 32GB",
                    "https://www.memorymarket.com/price/rdimm",
                ),
            ],
            ..Default::default()
        },
        Month {
            market: "ramx".into(),
            expiry: "RAMX-202701".into(),
            phase: "SourceSubmission".into(),
            scramble_start: Some(now + 2 * day),
            rewards: Some((520_000_000, 520_000_000)),
            skus: vec![
                Sku {
                    bucket: "4444".into(),
                    label: Some("DDR5 RDIMM 64GB".into()),
                },
                Sku {
                    bucket: "3333".into(),
                    label: Some("DDR4 SODIMM 16GB".into()),
                },
            ],
            ..Default::default()
        },
    ];
    app.snapshot.work = serde_json::from_value(json!([
        {"id":"w-d","title":"RAMX · DDR5 RDIMM 64GB","brief":"For the agent.","availability":"available",
         "objective":{"kind":"source","marketId":"ramx","expiryId":"RAMX-202701","bucketId":"4444","bucketLabel":"DDR5 RDIMM 64GB",
           "activeSources":0,"distinctHosts":0,"targets":{"activeSources":3,"distinctHosts":2},"sources":[]}},
        {"id":"w-u1","title":"RAMX · update","brief":"For the agent.","availability":"available",
         "objective":{"kind":"update","marketId":"ramx","expiryId":"RAMX-202612","bucketId":"5555","bucketLabel":"DDR4 UDIMM 8GB 3200",
           "url":"https://www.memorymarket.com/price/udimm","sourceAddress":"Src1","rewardsRemainingAtomic":"0","targets":{"activeSources":3,"distinctHosts":2}}},
        {"id":"w-u2","title":"RAMX · update","brief":"For the agent.","availability":"available",
         "objective":{"kind":"update","marketId":"ramx","expiryId":"RAMX-202612","bucketId":"6666","bucketLabel":"DDR5 RDIMM 32GB",
           "url":"https://www.memorymarket.com/price/rdimm","sourceAddress":"Src2","rewardsRemainingAtomic":"0","targets":{"activeSources":3,"distinctHosts":2}}}
    ]))
    .unwrap();
    app.snapshot.activity.clear();
    app
}

#[test]
fn a_products_months_show_their_phase_as_a_strip_and_their_rewards() {
    let mut app = phases_app();
    let screen = draw(&mut app, 120, 40);
    let ramx = screen.lines().nth(row_of(&screen, "RAMX ")).unwrap();
    assert!(
        ramx.contains("2 months") && ramx.contains("3 tasks"),
        "{ramx}"
    );
    click(&mut app, &Action::Nav(Path::product("ramx")));
    let screen = draw(&mut app, 120, 40);
    let line = screen.lines().nth(row_of(&screen, "Jan 2027  ")).unwrap();
    for text in ["◉○○○○ Sources  $520", "1 task"] {
        assert!(line.contains(text), "missing {text}: {line}");
    }
    let line = screen.lines().nth(row_of(&screen, "Dec 2026  ")).unwrap();
    assert!(
        line.contains("●●●◉○ Game") && line.contains("2 tasks"),
        "{line}"
    );
    assert!(!line.contains('$'), "{line}");
    for gone in [
        "Taking sources ─",
        "New price pages go on the oracle",
        "Sources are set; each update is staked",
        "no rewards",
        "in rewards",
    ] {
        assert!(!screen.contains(gone), "still there: {gone}:\n{screen}");
    }
}

#[test]
fn a_months_categories_are_its_products_each_on_one_row() {
    let mut app = phases_app();
    // Taking sources: every product it lists, the one with work lit up.
    app.activate(Action::Nav(Path::month("ramx", "RAMX-202701")));
    let screen = draw(&mut app, 120, 40);
    assert!(screen.contains("Products › RAMX › Jan 2027"), "{screen}");
    let line = screen
        .lines()
        .nth(row_of(&screen, "DDR5 RDIMM 64GB"))
        .unwrap();
    assert!(line.contains("0/3 src ██"), "{line}");
    let quiet = screen
        .lines()
        .nth(row_of(&screen, "DDR4 SODIMM 16GB"))
        .unwrap();
    assert!(!quiet.contains("██"), "{quiet}");
    assert!(
        row_of(&screen, "DDR4 SODIMM") < row_of(&screen, "DDR5 RDIMM"),
        "{screen}"
    );
    // Game: a product per source, its updates inside.
    app.activate(Action::Nav(Path::month("ramx", "RAMX-202612")));
    let screen = draw(&mut app, 120, 40);
    for text in ["DDR4 UDIMM 8GB 3200", "DDR5 RDIMM 32GB", "1 src ██"] {
        assert!(screen.contains(text), "missing {text}:\n{screen}");
    }
    for gone in [
        "Show 2 sources",
        "live sources take updates",
        "update memorymarket.com",
    ] {
        assert!(!screen.contains(gone), "still there: {gone}:\n{screen}");
    }
    click(
        &mut app,
        &Action::Nav(Path::category("ramx", "RAMX-202612", "6666")),
    );
    let screen = draw(&mut app, 120, 40);
    for text in [
        "Products › RAMX › Dec 2026 › DDR5 RDIMM 32GB",
        "updates · 1 source",
        "cold",
        "memorymarket.com/price/rdimm",
        "[Research this]",
    ] {
        assert!(screen.contains(text), "missing {text}:\n{screen}");
    }
    for gone in [
        "A live source in Game",
        "an update earns nothing",
        "Already found",
    ] {
        assert!(!screen.contains(gone), "still there: {gone}:\n{screen}");
    }
    // Its task is what the buttons act on.
    click(&mut app, &Action::Focus("w-u2".into(), true));
    assert_eq!(app.pending.take().unwrap().body["workId"], "w-u2");
}

#[test]
fn opening_work_shows_under_its_opening_month_a_task_per_source() {
    use crate::research::phase::Month;
    let mut app = phases_app();
    let listing = 1_792_972_800; // Oct 26 2026 00:00 UTC
    app.fund.snap.months.push(Month {
        market: "nandx".into(),
        expiry: "NANDX-202612".into(),
        phase: "Opening".into(),
        scramble_start: Some(listing - 8 * 86_400),
        rewards: Some((90_000_000, 90_000_000)),
        ..Default::default()
    });
    let mut work: Vec<crate::research::model::Work> = serde_json::from_value(json!([
        {"id":"w-o1","title":"NANDX · opening","brief":"For the agent.","availability":"available",
         "objective":{"kind":"opening","marketId":"nandx","expiryId":"NANDX-202612","bucketId":"7777","bucketLabel":"TLC 512Gb",
           "url":"https://www.dramexchange.com/nand/tlc","sourceAddress":"Src3","openingBondAtomic":"2500000",
           "openingBudgetAtomic":"30000000","listingUtc":"2026-10-26T00:00:00.000Z","targets":{"activeSources":3,"distinctHosts":2}}},
        {"id":"w-o2","title":"NANDX · opening","brief":"For the agent.","availability":"available",
         "objective":{"kind":"opening","marketId":"nandx","expiryId":"NANDX-202612","bucketId":"8888","bucketLabel":"QLC 1Tb",
           "url":"https://www.nandprices.com/qlc","sourceAddress":"Src4","openingBondAtomic":"2500000",
           "openingBudgetAtomic":"0","listingUtc":"2026-10-26T00:00:00.000Z","targets":{"activeSources":3,"distinctHosts":2}}}
    ]))
    .unwrap();
    app.snapshot.work.append(&mut work);
    app.activate(Action::Nav(Path::product("nandx")));
    let screen = draw(&mut app, 140, 50);
    let line = screen.lines().nth(row_of(&screen, "Dec 2026  ")).unwrap();
    assert!(line.contains("●●◉○○ Opening  $90"), "{line}");
    app.activate(Action::Nav(Path::category("nandx", "NANDX-202612", "7777")));
    let screen = draw(&mut app, 140, 50);
    for text in [
        "Products › NANDX › Dec 2026 › TLC 512Gb",
        "openings · 1 source · bond $2.50 · budget $30",
        "██ warm",
        "dramexchange.com/nand/tlc",
    ] {
        assert!(screen.contains(text), "missing {text}:\n{screen}");
    }
    app.activate(Action::Nav(Path::category("nandx", "NANDX-202612", "8888")));
    let screen = draw(&mut app, 140, 50);
    assert!(
        screen.contains("openings · 1 source · bond $2.50") && !screen.contains("budget"),
        "{screen}"
    );
    for gone in [
        "A frozen source in Opening",
        "Its opening value is the price",
        "sits out the month",
        "This month pays nothing",
    ] {
        assert!(!screen.contains(gone), "still there: {gone}:\n{screen}");
    }
}

/// The agent page's head: its first row (the one naming `who`) and the
/// next, where what it is on goes when the two do not fit one.
fn head(screen: &str, who: &str) -> (usize, String) {
    let lines: Vec<&str> = screen.lines().collect();
    let row = lines
        .iter()
        .position(|l| {
            l.match_indices(who)
                .any(|(at, _)| !l[..at].ends_with("● ") && !l[at..].contains("next →"))
        })
        .unwrap_or_else(|| panic!("missing {who}:\n{screen}"));
    (row, format!("{} {}", lines[row], lines[row + 1]))
}

fn row_of(screen: &str, text: &str) -> usize {
    screen
        .lines()
        .position(|l| l.contains(text))
        .unwrap_or_else(|| panic!("missing {text}:\n{screen}"))
}

#[test]
fn a_months_rewards_read_as_what_is_left_and_its_products_and_sources_are_read() {
    use crate::research::phase::{Month, months};
    let rewards = |total: &str, left: &str| {
        months(&json!({"state": {"months": [{"marketId": "ramx", "expiryId": "RAMX-202612", "phase": "Game",
            "rewards": {"phase": "Funded", "totalAtomic": total, "remainingAtomic": left}}]}}))[0]
            .rewards_left()
    };
    assert_eq!(rewards("520000000", "520000000").as_deref(), Some("$520"));
    assert_eq!(rewards("520000000", "2500000").as_deref(), Some("$2.50"));
    assert_eq!(rewards("520000000", "0"), None);
    // A schedule of 0 (dev's Game months) pays nothing.
    assert_eq!(rewards("0", "0"), None);
    assert_eq!(Month::default().rewards_left(), None);
    let read = months(&json!({
        "state": {"months": [{"marketId": "ramx", "expiryId": "RAMX-202701", "phase": "SourceSubmission",
            "skus": [{"bucketId": "4444", "bucketLabel": "DDR5 RDIMM 64GB"}, {"bucketId": "3333", "bucketLabel": null}]}]},
        "sourceLinks": [
            {"marketId": "ramx", "expiryId": "RAMX-202701", "sources": [
                {"bucketId": "4444", "bucketLabel": "DDR5 RDIMM 64GB", "status": "Candidate",
                 "canonicalLocator": {"url": "https://example.com/ddr5"}}]},
            {"marketId": "ramx", "expiryId": "RAMX-202612", "sources": [{"bucketId": "9", "status": "Active"}]}]
    }));
    assert_eq!(read[0].skus.len(), 2);
    assert_eq!(read[0].skus[1].label, None);
    assert_eq!(read[0].sources.len(), 1);
    assert_eq!(
        read[0].sources[0].url.as_deref(),
        Some("https://example.com/ddr5")
    );
}

#[test]
fn scramble_says_which_of_its_seven_days_it_is() {
    use crate::research::phase::Month;
    let day = 86_400;
    let month = |start: i64| Month {
        market: "ramx".into(),
        expiry: "RAMX-202701".into(),
        phase: "Scramble".into(),
        scramble_start: Some(start),
        ..Default::default()
    };
    let now = 1_792_300_000;
    let text = |spans: Vec<ratatui::text::Span>| {
        spans
            .iter()
            .map(|s| s.content.to_string())
            .collect::<String>()
    };
    assert!(text(month(now - day).map(now).spans).contains("Scramble: placing, day 2 of 7"));
    assert!(text(month(now - 5 * day).map(now).spans).contains("Scramble: challenges, day 6 of 7"));
    assert!(text(month(now - 6 * day - 10).map(now).spans).contains("Scramble: freezing, day 7"));
    // Research's strip: in a word or two.
    assert_eq!(text(month(now - day).strip(now)), "●◉○○○ Scramble day 2");
    assert_eq!(
        crate::research::phase::expiry_id("ramx", 1_798_761_600),
        "RAMX-202612"
    );
}

#[test]
fn a_market_shows_where_its_oracle_month_is() {
    use crate::research::phase::Month;
    let mut app = markets_app();
    app.view = View::Markets;
    app.fund.snap.chain_now = Some(1_792_300_000);
    app.fund.snap.months = vec![Month {
        market: "ramx".into(),
        expiry: "RAMX-202610".into(),
        phase: "Game".into(),
        ..Default::default()
    }];
    let screen = draw(&mut app, 120, 40);
    let line = screen
        .lines()
        .find(|l| l.contains("Oracle  "))
        .unwrap_or_else(|| panic!("{screen}"));
    for step in [
        "Taking sources",
        "Scramble",
        "Opening",
        "Game",
        "Settled",
        "settles Nov 1",
    ] {
        assert!(line.contains(step), "{line}");
    }
    // The map alone: no rule under it.
    assert!(!screen.contains("each update is staked"), "{screen}");
}

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

#[test]
fn the_navigator_drills_down_by_click_and_back_up_by_crumb_or_backspace() {
    let mut app = idle_research_app();
    draw(&mut app, 120, 40);
    click(&mut app, &Action::Nav(Path::product("ramx")));
    let screen = draw(&mut app, 120, 40);
    assert!(screen.contains("Products › RAMX"), "{screen}");
    assert!(screen.contains("Dec 2026"), "{screen}");
    assert!(!screen.contains("NANDX"), "{screen}");
    click(&mut app, &Action::Nav(Path::month("ramx", "RAMX-202612")));
    let screen = draw(&mut app, 120, 40);
    assert!(screen.contains("Products › RAMX › Dec 2026"), "{screen}");
    let line = screen
        .lines()
        .nth(row_of(&screen, "Unnamed product b0bf23"))
        .unwrap();
    assert!(line.contains("1/3 src ██"), "{line}");
    let category = Path::category("ramx", "RAMX-202612", "b0bf2302bd593a4b");
    click(&mut app, &Action::Nav(category.clone()));
    draw(&mut app, 120, 40);
    assert_eq!(app.nav().path, category);
    // A crumb goes back up: the month's (the first control for it).
    click(&mut app, &Action::Nav(Path::month("ramx", "RAMX-202612")));
    assert_eq!(app.nav().path, Path::month("ramx", "RAMX-202612"));
    // Backspace goes up one level at a time, and does nothing at the top.
    for depth in [1, 0] {
        draw(&mut app, 120, 40);
        let action = input::handle(&mut app, key(KeyCode::Backspace)).expect("up");
        app.activate(action);
        assert_eq!(app.nav().path.depth(), depth);
    }
    draw(&mut app, 120, 40);
    assert_eq!(input::handle(&mut app, key(KeyCode::Backspace)), None);
    // The keyboard reaches every row too.
    let row = app
        .hits
        .iter()
        .position(|h| h.action == Action::Nav(Path::product("nandx")))
        .unwrap();
    app.focus = row;
    let action = input::handle(&mut app, key(KeyCode::Enter)).unwrap();
    app.activate(action);
    assert_eq!(app.nav().path, Path::product("nandx"));
}

#[test]
fn the_navigator_follows_the_agents_work_until_a_click_moves_it() {
    let cedar = "ab".repeat(32);
    let mut app = research_app();
    let screen = draw(&mut app, 120, 40);
    // Selected, cedar is followed to its work, lit.
    let nandx = Path::category("nandx", "NANDX-202611", "x");
    assert_eq!(app.nav().path, nandx);
    assert!(app.nav().follow);
    assert!(
        screen.contains("Products › NANDX › Nov 2026 › TLC 512Gb"),
        "{screen}"
    );
    // Its work moves: so does the navigator.
    let move_to = |app: &mut App, work: &str| {
        let mut next = app.snapshot.clone();
        next.activity[0].work_id = work.into();
        for w in &mut next.work {
            w.researchers = if w.id == work {
                vec![cedar.clone()]
            } else {
                vec![]
            };
        }
        app.update(next);
    };
    move_to(&mut app, "w-a");
    let screen = draw(&mut app, 120, 40);
    let ramx = Path::category("ramx", "RAMX-202612", "b0bf2302bd593a4b");
    assert_eq!(app.nav().path, ramx);
    assert!(
        screen.contains("Products › RAMX › Dec 2026 › Unnamed product b0bf23"),
        "{screen}"
    );
    // Up a level, its row is lit (the bar beside it).
    click(&mut app, &Action::Nav(Path::month("ramx", "RAMX-202612")));
    let screen = draw(&mut app, 120, 40);
    let line = screen
        .lines()
        .nth(row_of(&screen, "Unnamed product b0bf23"))
        .unwrap();
    assert!(line.contains("▌Unnamed product"), "{line}");
    // That click stopped following: the agent moves, the navigator stays.
    assert!(!app.nav().follow);
    move_to(&mut app, "w-b");
    draw(&mut app, 120, 40);
    assert_eq!(app.nav().path, Path::month("ramx", "RAMX-202612"));
    // Follow takes it back to the agent.
    click(&mut app, &Action::Follow);
    draw(&mut app, 120, 40);
    assert_eq!(app.nav().path, nandx);
    assert!(app.nav().follow);
}

#[test]
fn each_agent_has_its_own_page() {
    let (cedar, birch) = ("ab".repeat(32), "cd".repeat(32));
    let mut app = research_app();
    let mut next = app.snapshot.clone();
    next.agents.push(
        serde_json::from_value(json!({"id":birch,"name":"birch","harness":"Codex",
            "payoutAuthority":"public-address","revokedAt":null}))
        .unwrap(),
    );
    next.activity.push(
        serde_json::from_value(
            json!({"agentId":birch,"workId":"w-a","title":"RAMX","lastMessage":null}),
        )
        .unwrap(),
    );
    next.work[0].researchers = vec![birch.clone()];
    let mut detail = app.details[&cedar].clone();
    detail.agent = next.agents[1].clone();
    detail.events[0].message = "Birch read a distributor.".into();
    next.selected = Some(detail);
    app.update(next);
    let screen = draw(&mut app, 120, 40);
    assert!(
        head(&screen, "Claude Code · cedar").1.contains("NANDX"),
        "{screen}"
    );
    assert!(!screen.contains("Birch read"), "{screen}");
    // cedar's page was moved by hand.
    click(&mut app, &Action::Nav(Path::default()));
    let screen = draw(&mut app, 120, 40);
    let line = screen.lines().nth(row_of(&screen, "RAMX")).unwrap();
    assert!(line.contains("● Codex"), "{line}");
    // birch's page follows birch, with its own feed.
    click(&mut app, &Action::Agent(birch.clone()));
    let screen = draw(&mut app, 120, 40);
    assert_eq!(
        app.nav().path,
        Path::category("ramx", "RAMX-202612", "b0bf2302bd593a4b")
    );
    assert!(
        head(&screen, "Codex · birch").1.contains("RAMX Dec 2026"),
        "{screen}"
    );
    assert!(screen.contains("Birch read a distributor."), "{screen}");
    // Back on cedar, its page follows cedar again.
    click(&mut app, &Action::Agent(cedar));
    assert_eq!(app.nav().path, Path::category("nandx", "NANDX-202611", "x"));
}

#[test]
fn research_says_nothing_but_names_and_numbers() {
    let sentences = [
        "Products to research",
        "potential earnings",
        "Each month's phase sets the work",
        "Pick one to send your agents there first",
        "New price pages go on the oracle",
        "once the sponsor posts their bond",
        "once a sponsor posts their bond",
        "Sources are placed and supported",
        "Sources are set; each update",
        "Each source gets its opening value",
        "earn no rewards this month",
        "earns nothing this month",
        "Show 1 source",
        "Show 2 sources",
        "Your directive",
        "Already found",
        "Researching now",
        "Not researching right now",
        "This agent has not started",
        "Select an agent",
        "Connect an agent to see",
        "Nothing needs research right now",
        "Petri couldn't check",
        "TIME UTC",
        "[Agent activity]",
        "[Goals]",
    ];
    let mut screens = Vec::new();
    for mut app in [
        research_app(),
        idle_research_app(),
        phases_app(),
        feed_app(),
    ] {
        for path in [
            Path::default(),
            Path::product("ramx"),
            Path::product("nandx"),
            Path::month("ramx", "RAMX-202612"),
            Path::month("ramx", "RAMX-202701"),
            Path::category("ramx", "RAMX-202612", "6666"),
            Path::category("ramx", "RAMX-202701", "4444"),
            Path::category("ramx", "RAMX-202612", "b0bf2302bd593a4b"),
        ] {
            app.activate(Action::Nav(path));
            for (width, height) in [(120, 40), (80, 24)] {
                screens.push(draw(&mut app, width, height));
            }
        }
    }
    // Nothing read yet, nothing found: no sentence either.
    let mut empty = app();
    empty.view = View::Activity;
    empty.snapshot.work.clear();
    for discovery in [
        json!(null),
        json!({"assessedAt": 1}),
        json!({"error": "boom"}),
    ] {
        empty.snapshot.discovery = discovery;
        screens.push(draw(&mut empty, 120, 40));
    }
    for screen in &screens {
        for gone in sentences {
            assert!(!screen.contains(gone), "still there: {gone}:\n{screen}");
        }
        assert!(
            !screen.contains("boom") && !screen.contains("Loading"),
            "{screen}"
        );
    }
}

#[test]
fn the_navigator_takes_the_width_without_agents_and_stacks_when_narrow() {
    // No agents: no page beside it.
    let mut app = idle_research_app();
    app.snapshot.agents.clear();
    app.details.clear();
    app.selected_agent = None;
    let screen = draw(&mut app, 120, 40);
    assert!(!screen.contains("[Follow]"), "{screen}");
    let row = |app: &App| {
        app.hits
            .iter()
            .find(|h| h.action == Action::Nav(Path::product("ramx")))
            .unwrap()
            .area
    };
    assert!(row(&app).width > 80, "{:?}", row(&app));
    // Side by side from 100 columns, stacked below.
    let mut app = research_app();
    app.activate(Action::Nav(Path::default()));
    let wide = draw(&mut app, 120, 40);
    assert!(row(&app).width < 45, "{:?}", row(&app));
    assert_eq!(
        head(&wide, "Claude Code · cedar").0,
        row_of(&wide, "Products  ") + 1,
        "{wide}"
    );
    let narrow = draw(&mut app, 80, 24);
    let (top, line) = head(&narrow, "Claude Code · cedar");
    assert!(line.contains("NANDX"), "{narrow}");
    assert!(top > row_of(&narrow, "Products") + 2, "{narrow}");
    let rule = narrow.lines().nth(top - 1).unwrap();
    assert!(rule.contains("─────") && !rule.contains('┌'), "{narrow}");
    assert!(row(&app).width > 40, "{:?}", row(&app));
    for (width, height) in [(45, 16), (60, 24), (99, 30), (100, 30), (200, 60)] {
        draw(&mut app, width, height);
    }
}

#[test]
fn an_owner_action_while_another_is_out_is_quietly_ignored() {
    let mut app = App::new(String::new(), String::new(), String::new());
    app.post(
        "/v1/owner/work/focus",
        json!({"workId": "w1", "focused": true}),
    );
    assert!(app.pending.is_some());
    // A second one while the first waits: no sentence, the first stays.
    app.post(
        "/v1/owner/work/focus",
        json!({"workId": "w2", "focused": true}),
    );
    assert_eq!(app.status, "");
    assert_eq!(app.pending.as_ref().unwrap().body["workId"], "w1");
    // Sent and not answered yet: the same.
    app.pending = None;
    app.mutation_inflight = true;
    app.post(
        "/v1/owner/work/focus",
        json!({"workId": "w3", "focused": true}),
    );
    assert_eq!(app.status, "");
    assert!(app.pending.is_none());
}

#[test]
fn render_snapshots_show_the_intro_only_when_asked() {
    use super::{intro, shows_intro};
    assert!(shows_intro(false, None));
    assert!(shows_intro(false, Some("markets")));
    assert!(shows_intro(true, Some("intro")));
    assert!(shows_intro(true, Some("Intro")));
    assert!(!shows_intro(true, None));
    assert!(!shows_intro(true, Some("markets")));
    let mut app = app();
    for view in View::ALL {
        app.view = view;
        assert!(!draw(&mut app, 100, 30).contains(intro::HINT), "{view:?}");
    }
}

fn proof_entry() -> crate::evidence::store::Entry {
    crate::evidence::store::Entry {
        digest: "ab".repeat(32),
        origin: "proved".into(),
        locator: "https://portal.example.com/api/prices".into(),
        price: "41.25 USD".into(),
        product: "DDR5-16G-4800".into(),
        ..Default::default()
    }
}

#[test]
fn the_evidence_view_lets_you_prove_and_submit_by_hand() {
    use super::proofs::{Click, Kind, Stage};
    let mut app = app();
    app.view = View::Evidence;
    app.loaded = true;
    app.proofs.list = vec![proof_entry()];
    let screen = draw(&mut app, 110, 32);
    assert!(screen.contains("Your proofs"), "{screen}");
    for button in ["[Sign in to a site]", "[Prove a page]", "[Import a proof]"] {
        assert!(screen.contains(button), "{screen}");
    }
    assert!(screen.contains("41.25 USD · portal.example.com/api/prices · not submitted"));
    // The agent's finds stay below.
    assert!(screen.contains("Your agent's finds"));

    click(&mut app, &Action::ProofTicket(Kind::Prove));
    let screen = draw(&mut app, 110, 32);
    assert!(
        screen.contains("PROVE A PAGE")
            && screen.contains("JSON address")
            && screen.contains("Manifest file"),
        "{screen}"
    );
    // Typing and pasting fill the focused field; Tab moves on.
    input::handle(
        &mut app,
        Event::Paste("https://portal.example.com/api/prices".into()),
    );
    input::handle(&mut app, Event::Key(KeyEvent::from(KeyCode::Tab)));
    for c in "m.json".chars() {
        input::handle(&mut app, Event::Key(KeyEvent::from(KeyCode::Char(c))));
    }
    let t = app.proofs.ticket.clone().unwrap();
    assert_eq!(
        t.fields,
        vec![
            "https://portal.example.com/api/prices".to_string(),
            "m.json".to_string()
        ]
    );
    // No loading text while it works.
    app.proofs.ticket.as_mut().unwrap().stage = Stage::Working;
    let screen = draw(&mut app, 110, 32);
    for word in ["Loading", "Proving", "Working", "Preparing", "…"] {
        assert!(!screen.contains(word), "{word}: {screen}");
    }

    // A preview of a proof that verifies offers Submit; Submit asks which
    // product of the months taking sources it is for.
    app.proofs.ticket.as_mut().unwrap().stage = Stage::Preview(
        vec![ratatui::text::Line::raw(
            "Web proof  portal.example.com/api/prices",
        )],
        Some("ab".repeat(32)),
    );
    app.fund.snap.months = vec![super::phase::Month {
        market: "ramx".into(),
        expiry: "RAMX-202612".into(),
        phase: "SourceSubmission".into(),
        skus: vec![super::phase::Sku {
            bucket: "cd".repeat(32),
            label: Some("DDR5 16Gb".into()),
        }],
        ..Default::default()
    }];
    let screen = draw(&mut app, 110, 32);
    assert!(screen.contains("[ Submit as a source ]"), "{screen}");
    assert!(screen.contains("[ Submit as an update ]"), "{screen}");
    click(&mut app, &Action::ProofClick(Click::Submit));
    let screen = draw(&mut app, 110, 32);
    assert!(
        screen.contains("Which product is this a source for?"),
        "{screen}"
    );
    assert!(screen.contains("RAMX · Dec 2026 · DDR5 16Gb"), "{screen}");
    assert!(matches!(
        app.proofs.ticket.as_ref().unwrap().stage,
        Stage::Choose(..)
    ));

    // A click inside the pop-up on plain text does nothing behind it; a
    // click outside closes it.
    let r = app
        .hits
        .iter()
        .find(|h| h.action == Action::ProofClick(Click::Nothing))
        .unwrap()
        .area;
    assert_eq!(
        click_at(&mut app, r.x + 1, r.y),
        Some(Action::ProofClick(Click::Nothing))
    );
    assert_eq!(click_at(&mut app, 0, 0), Some(Action::CloseProof));
}

#[test]
fn a_proof_that_does_not_verify_offers_no_submit() {
    use super::proofs::{Kind, Stage};
    let mut app = app();
    app.view = View::Evidence;
    app.proofs.open(Kind::Import);
    app.proofs.ticket.as_mut().unwrap().stage = Stage::Preview(
        vec![ratatui::text::Line::raw("Proof: the proof does not verify")],
        None,
    );
    let screen = draw(&mut app, 110, 32);
    assert!(!screen.contains("Submit as a source"), "{screen}");
    assert!(screen.contains("any key closes"));
}

/// A complete published assessment of DDR5 16Gb chips: it fits its bucket.
fn chip_series(id: &str, row: &str) -> ameba_commodity_spec::SeriesDefinition {
    let bucket = ameba_commodity_spec::Product::Ramx
        .bucket("DDR5_16GBIT")
        .unwrap();
    let mut v = ameba_commodity_spec::series_template(bucket, false);
    v["seriesId"] = json!(id);
    v["origin"] = json!({"publisher":"Example Research","locator":"https://example.com/price/dram","row":row});
    v["nativeUnit"] = json!("USD per chip");
    v["statistic"] = json!("average");
    v["procurementSegment"] = json!({"state":"documented_mix","value":"OEM contract buyers"});
    v["geography"] = json!({"state":"documented_mix","value":"global"});
    v["referencePeriod"] = json!("month");
    v["cadence"] = json!("monthly");
    v["revisionPolicy"] = json!("corrections replace the release they correct");
    v["missingDataPolicy"] = json!("carry the last accepted value forward");
    v["dram"]["dieCount"] = json!(1);
    v["dram"]["condition"] = json!("new_unused");
    v["dram"]["grade"] = json!("standard_production");
    v["dram"]["organization"] = json!({"state":"exact","value":"x8"});
    v["dram"]["speed"] = json!({"state":"documented_mix","value":"4800 and 5600 MT/s"});
    serde_json::from_value(v).unwrap()
}

fn chip_bucket() -> String {
    crate::oracle::skus::hex(
        &ameba_commodity_spec::Product::Ramx
            .bucket("DDR5_16GBIT")
            .unwrap()
            .id32(),
    )
}

#[test]
fn a_chip_months_sources_say_whether_they_count_from_their_definitions_on_chain() {
    use crate::research::fund::{Definitions, route_months};
    use crate::research::phase::{Month, Source};
    let bucket = chip_bucket();
    let mut app = research_app();
    let source = |id: &str, address: &str, url: &str| Source {
        bucket: bucket.clone(),
        label: Some("DDR5_16GBIT".into()),
        url: Some(url.into()),
        status: "Active".into(),
        id: id.repeat(32),
        address: Some(address.into()),
        checked: None,
    };
    app.fund.snap.months = vec![Month {
        market: "ramx".into(),
        expiry: "RAMX-202612".into(),
        phase: "Game".into(),
        sources: vec![
            source("aa", "SrcA", "https://example.com/price/dram"),
            source("bb", "SrcB", "https://other.example/ddr5"),
            source("cc", "SrcC", "https://example.com/price/dram?copy"),
            // A dev bootstrap source: its locator is linked, its definition not.
            source("dd", "SrcD", "https://bootstrap.example/ddr5"),
        ],
        ..Default::default()
    }];
    let series = chip_series("example-ddr5-16gb", "DDR5 16Gb (2Gx8) 4800/5600");
    let mut known = Definitions::new();
    known.insert("SrcA".into(), Some(series.definition_text()));
    known.insert("SrcB".into(), Some("Product: DDR5 16GB module".into()));
    // Not read yet: nothing is routed, since a repeat is judged against the others.
    route_months(&mut app.fund.snap.months, &known);
    assert!(
        app.fund.snap.months[0]
            .sources
            .iter()
            .all(|s| s.checked.is_none())
    );
    known.insert("SrcC".into(), Some(series.definition_text()));
    known.insert("SrcD".into(), None);
    route_months(&mut app.fund.snap.months, &known);
    let words: Vec<Option<String>> = app.fund.snap.months[0]
        .sources
        .iter()
        .map(|s| s.checked.as_ref().map(|c| c.words()))
        .collect();
    assert_eq!(
        words,
        [
            Some("counts this month".to_string()),
            Some("missing details".to_string()),
            Some("already counted".to_string()),
            // No definition on chain: no routing at all.
            None,
        ]
    );
    assert_eq!(
        app.fund.snap.months[0].sources[1]
            .checked
            .as_ref()
            .unwrap()
            .reason
            .as_deref(),
        Some("not a comparison series")
    );
    // The category reads by the bucket's name, each source with whether it counts.
    app.activate(Action::Nav(Path::category("ramx", "RAMX-202612", &bucket)));
    let screen = draw(&mut app, 140, 40);
    for text in [
        "Products › RAMX › Dec 2026 › DDR5 16Gb",
        "example.com/price/dram",
        "active · counts this month",
        "active · missing details",
        "active · already counted",
        "bootstrap.example/ddr5",
    ] {
        assert!(screen.contains(text), "missing {text}:\n{screen}");
    }
    assert!(!screen.contains("DDR5_16GBIT"), "{screen}");
    // Its row: the page, then its status and nothing after it.
    let line = screen
        .lines()
        .find(|l| l.contains("bootstrap.example/ddr5"))
        .unwrap();
    let after = line.split("bootstrap.example/ddr5").nth(1).unwrap();
    assert_eq!(after.split('│').next().unwrap().trim(), "active", "{line}");
}

#[test]
fn chip_finds_and_your_chip_sources_read_by_name_with_the_apis_routing() {
    use crate::research::fund::{check_mine, snapshot_from};
    let bucket = chip_bucket();
    let mut app = app();
    app.view = View::Fund;
    let mut template = ameba_commodity_spec::series_template(
        ameba_commodity_spec::Product::Ramx
            .bucket("DDR5_16GBIT")
            .unwrap(),
        false,
    );
    template["origin"]["locator"] = json!("https://example.com/price/dram");
    let candidates = json!({"candidates":[
        {"contentHash":"c".repeat(64),"agentId":"ab".repeat(32),"canonicalLocatorHash":"aa11","createdAt":1_000_000u64,
         "routing":{"status":"NEEDS_METADATA","reasons":[{"status":"NEEDS_METADATA","field":"statistic","message":"the price statistic is not set"}]},
         "details":{"marketId":"ramx","expiryId":"RAMX-202612","bucketId":bucket,"canonicalUrl":"https://www.example.com/dram"}},
        {"contentHash":"d".repeat(64),"agentId":"ab".repeat(32),"canonicalLocatorHash":"bb22","createdAt":1_000_000u64,
         "series":chip_series("example-ddr5-16gb", "DDR5 16Gb"),
         "details":{"marketId":"ramx","expiryId":"RAMX-202612","bucketId":bucket,"canonicalUrl":"https://www.trendforce.example/dram"}},
        {"contentHash":"e".repeat(64),"agentId":"ab".repeat(32),"canonicalLocatorHash":"cc33","createdAt":1_000_000u64,
         "series":template,
         "details":{"marketId":"ramx","expiryId":"RAMX-202612","bucketId":bucket,"canonicalUrl":"https://www.example.org/dram"}}]});
    let oracle = json!({"sourceLinks":[{"marketId":"ramx","expiryId":"RAMX-202612","sources":[
        {"canonicalLocator":{"url":"https://vendor.com/ddr5","hash":"dd44"},"proposer":"me","status":"Active",
         "sourceId":"ee".repeat(32),"bucketId":bucket,"bucketLabel":"DDR5_16GBIT","bucketName":"DDR5 16Gb",
         "routing":{"status":"ELIGIBLE_FOR_CONTRACT","reasons":[]}}]}]});
    let mut snap = snapshot_from(&candidates, &oracle, Some("me"));
    snap.balance = Some((12_000_000, 0));
    // The month's routing reaches your sources (fetch does this after its own).
    snap.mine[0].checked = None;
    snap.months = crate::research::phase::months(&json!({"state":{"months":[]}}));
    check_mine(&mut snap.mine, &snap.months);
    assert!(snap.mine[0].checked.is_none());
    let mut snap = snapshot_from(&candidates, &oracle, Some("me"));
    snap.balance = Some((12_000_000, 0));
    assert_eq!(snap.finds[0].title, "RAMX · Dec 2026 · DDR5 16Gb");
    assert_eq!(snap.mine[0].title, "RAMX · Dec 2026 · DDR5 16Gb");
    // The API's routing, else the series' own.
    let words: Vec<String> = snap
        .finds
        .iter()
        .map(|f| f.checked.as_ref().unwrap().words())
        .collect();
    assert_eq!(
        words,
        ["missing details", "fits the bucket", "missing details"]
    );
    app.fund.take_snapshot(snap);
    let screen = draw(&mut app, 120, 40);
    for text in [
        "RAMX · Dec 2026 · DDR5 16Gb · example.com",
        "– missing details",
        "– fits the bucket",
        "RAMX · Dec 2026 · DDR5 16Gb · vendor.com  active · counts this month",
    ] {
        assert!(screen.contains(text), "missing {text}:\n{screen}");
    }
    click(&mut app, &Action::ShowFind(0));
    let screen = draw(&mut app, 120, 40);
    assert!(
        screen.contains("Checked    Missing details: the price statistic is not set"),
        "{screen}"
    );
}
