use super::{
    input,
    model::{Action, App, Modal, Snapshot, View, safe, usdc},
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
    app.view = View::Activity;
    draw(&mut app, 100, 32);
    click(&mut app, &Action::ActivitySection(2));
    draw(&mut app, 100, 32);
    click(&mut app, &Action::NewDirective);
    draw(&mut app, 100, 32);
    click(&mut app, &Action::FormFocus(0));
    input::handle(&mut app, Event::Paste("ramx, nandx".into()));
    draw(&mut app, 100, 32);
    click(&mut app, &Action::FormFocus(1));
    input::handle(
        &mut app,
        Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
    );
    input::handle(&mut app, Event::Paste("4".into()));
    draw(&mut app, 100, 32);
    click(&mut app, &Action::Save);
    assert_eq!(
        app.pending.as_ref().unwrap().body["marketIds"],
        json!(["ramx", "nandx"])
    );
    assert_eq!(app.pending.as_ref().unwrap().body["minActiveSources"], 4);
    assert!(app.pending.as_ref().unwrap().body.get("title").is_none());
    assert!(app.pending.as_ref().unwrap().body.get("brief").is_none());
}

#[test]
fn work_refinement_is_clickable_and_keeps_the_exact_objective_id() {
    let mut app = app();
    app.view = View::Activity;
    for (width, height) in [(60, 24), (80, 24), (100, 32)] {
        app.modal = None;
        app.activity_section = 0;
        draw(&mut app, width, height);
        click(&mut app, &Action::ShowWork(0));
        draw(&mut app, width, height);
        click(&mut app, &Action::Refine("work-1".into()));
        draw(&mut app, width, height);
        input::handle(
            &mut app,
            Event::Paste("Find manufacturer sources.\nMatch the region.".into()),
        );
        click(&mut app, &Action::Save);
        let post = app.pending.take().unwrap();
        assert_eq!(post.path, "/v1/owner/work/refine");
        assert_eq!(post.body["workId"], "work-1");
        assert_eq!(
            post.body["instruction"],
            "Find manufacturer sources.\nMatch the region."
        );
    }
}

#[test]
fn directives_remain_clickable_at_terminal_sizes_without_assignment_fields() {
    let mut app = app();
    app.view = View::Activity;
    app.activity_section = 2;
    for (width, height) in [(60, 24), (80, 24), (100, 32)] {
        app.modal = None;
        let rendered = draw(&mut app, width, height);
        assert!(!rendered.contains("New assignment"));
        click(&mut app, &Action::EditDirective("directive-1".into()));
        draw(&mut app, width, height);
        for field in 0..3 {
            assert!(
                app.hits
                    .iter()
                    .any(|h| h.action == Action::FormFocus(field))
            );
        }
        assert!(app.hits.iter().any(|h| h.action == Action::Save));
    }
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
#[test]
fn generated_setup_has_no_owner_credential_and_preserves_paths() {
    let mut app = app();
    app.harness = 1;
    let codex = app.command(true);
    assert!(codex.contains("mcp_servers.petri_research"));
    assert!(codex.contains("C:/Petri runtime/node.exe"));
    assert!(codex.contains("C:/Petri runtime/research-runtime/agent.mjs"));
    assert!(!codex.contains("OWNER_TOKEN"));
    app.harness = 0;
    let json: serde_json::Value = serde_json::from_str(&app.command(true)).unwrap();
    assert_eq!(
        json["mcpServers"]["petri-research"]["command"],
        "C:/Petri runtime/node.exe"
    );
    assert_eq!(
        json["mcpServers"]["petri-research"]["args"][0],
        "C:/Petri runtime/research-runtime/agent.mjs"
    );
    assert_eq!(json["mcpServers"]["petri-research"]["args"][1], "mcp");
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

#[test]
fn activity_feed_keeps_long_reports_and_references_readable() {
    let mut app = app();
    app.view = View::Activity;
    app.activity_section = 1;
    let event = &mut app.snapshot.selected.as_mut().unwrap().events[0];
    event.kind = "source_found".into();
    event.message = "Manufacturer listing checked. ".repeat(4) + "REPORT_END";
    event.reference_url =
        Some("https://example.com/".to_string() + &"reference/".repeat(4) + "URL_END");
    let text = draw(&mut app, 110, 40);
    assert!(text.contains("TIME UTC"));
    assert!(text.contains("Source found"));
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(compact.contains("REPORT_END"));
    assert!(compact.contains("URL_END"));
    let narrow = draw(&mut app, 60, 32);
    assert!(narrow.contains("Source found"));
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
        screen.contains("LONG") && screen.contains("Enter quotes this trade"),
        "{screen}"
    );
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
