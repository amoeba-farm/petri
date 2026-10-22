use crate::research::model::{Action, App, Modal, View};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};

pub fn handle(app: &mut App, event: Event) -> Option<Action> {
    if let Event::Mouse(mouse) = &event {
        app.pointer = Some((mouse.column, mouse.row));
    }
    match event {
        Event::Mouse(mouse) => match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => return app.hit(mouse.column, mouse.row),
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let down = mouse.kind == MouseEventKind::ScrollDown;
                let at = (mouse.column, mouse.row).into();
                if app.modal.is_none() && app.sidebar.contains(at) {
                    app.agent_scroll = if down {
                        app.agent_scroll.saturating_add(1)
                    } else {
                        app.agent_scroll.saturating_sub(1)
                    };
                } else if app.modal.is_none()
                    && app.view == View::Evidence
                    && app.proofs.ticket.is_none()
                    && app.proofs.area.contains(at)
                {
                    // Your proofs' list scrolls on its own, a row at a time.
                    app.proofs.scroll = if down {
                        app.proofs.scroll.saturating_add(1)
                    } else {
                        app.proofs.scroll.saturating_sub(1)
                    };
                } else if app.modal.is_none()
                    && app.view == View::Activity
                    && app.feed_area.contains(at)
                {
                    // The feed keeps its newest in view until scrolled up.
                    let max = app.feed_max;
                    let nav = app.nav_mut();
                    let top = nav.feed_top.unwrap_or(max).min(max);
                    nav.feed_top = if down {
                        Some(top + 3).filter(|t| *t < max)
                    } else {
                        Some(top.saturating_sub(3)).filter(|_| max > 0)
                    };
                } else {
                    app.scroll = if down {
                        app.scroll.saturating_add(3)
                    } else {
                        app.scroll.saturating_sub(3)
                    };
                }
            }
            _ => {}
        },
        Event::Paste(value) if app.view == View::Evidence && app.proofs.ticket.is_some() => {
            app.proofs.paste(&value)
        }
        Event::Paste(value) => app.edit(&value, false),
        Event::Key(key) if key.kind != KeyEventKind::Release => {
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                return Some(Action::Quit);
            }
            if app.modal.is_some() {
                if matches!(app.modal, Some(Modal::Harness)) {
                    let n = App::HARNESSES.len();
                    match key.code {
                        KeyCode::Up | KeyCode::BackTab => app.harness = (app.harness + n - 1) % n,
                        KeyCode::Down | KeyCode::Tab => app.harness = (app.harness + 1) % n,
                        KeyCode::Enter | KeyCode::Esc => return Some(Action::Cancel),
                        _ => {}
                    }
                    return None;
                }
                if let Some(Modal::Find(find)) = &app.modal {
                    // Sponsor it (when it can be), Close.
                    let sponsor = app
                        .find_sponsorable(find)
                        .then(|| find.content_hash.clone());
                    let controls = if sponsor.is_some() { 2 } else { 1 };
                    match key.code {
                        KeyCode::Esc => return Some(Action::Cancel),
                        KeyCode::Tab => app.form_focus = (app.form_focus + 1) % controls,
                        KeyCode::BackTab => {
                            app.form_focus = (app.form_focus + controls - 1) % controls
                        }
                        KeyCode::Enter => {
                            return Some(match sponsor {
                                Some(hash) if app.form_focus == 0 => Action::Sponsor(hash),
                                _ => Action::Cancel,
                            });
                        }
                        _ => {}
                    }
                    return None;
                }
                if let Some(Modal::Paste(start)) = &app.modal {
                    // Done, Copy again.
                    let again = match start {
                        Some(id) => Action::CopyStart(id.clone()),
                        None => Action::CopyCommand,
                    };
                    match key.code {
                        KeyCode::Esc => return Some(Action::Cancel),
                        KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => {
                            app.form_focus = 1 - app.form_focus.min(1)
                        }
                        KeyCode::Enter => {
                            return Some(if app.form_focus == 1 {
                                again
                            } else {
                                Action::Cancel
                            });
                        }
                        _ => {}
                    }
                    return None;
                }
                if matches!(app.modal, Some(Modal::Revoke(_))) {
                    let controls = 2;
                    match key.code {
                        KeyCode::Esc => return Some(Action::Cancel),
                        KeyCode::Tab => app.form_focus = (app.form_focus + 1) % controls,
                        KeyCode::BackTab => {
                            app.form_focus = (app.form_focus + controls - 1) % controls
                        }
                        KeyCode::Enter => {
                            return Some(match &app.modal {
                                Some(Modal::Revoke(agent)) if app.form_focus == 0 => {
                                    Action::ConfirmRevoke(agent.id.clone())
                                }
                                _ => Action::Cancel,
                            });
                        }
                        _ => {}
                    }
                    return None;
                }
                let fields = app.form_fields();
                match key.code {
                    KeyCode::Esc => return Some(Action::Cancel),
                    KeyCode::Tab => app.form_focus = (app.form_focus + 1) % (fields + 2),
                    KeyCode::BackTab => {
                        app.form_focus = (app.form_focus + fields + 1) % (fields + 2)
                    }
                    KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Some(Action::Save);
                    }
                    KeyCode::Enter => {
                        if let Some(Modal::Revoke(agent)) = &app.modal {
                            return Some(if app.form_focus == 0 {
                                Action::ConfirmRevoke(agent.id.clone())
                            } else {
                                Action::Cancel
                            });
                        }
                        if app.form_focus == fields {
                            return Some(Action::Save);
                        }
                        if app.form_focus == fields + 1 {
                            return Some(Action::Cancel);
                        }
                        if matches!(app.modal, Some(Modal::Refine { .. })) {
                            app.edit("\n", false);
                        } else if fields == 1 {
                            return Some(Action::Save);
                        } else {
                            app.form_focus += 1;
                        }
                    }
                    KeyCode::Backspace => app.edit("", true),
                    KeyCode::Char(c)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        app.edit(&c.to_string(), false)
                    }
                    _ => {}
                }
                return None;
            }
            if app.view == View::Fund && app.fund.ticket.is_some() {
                app.fund.key(key);
                return None;
            }
            if app.view == View::Evidence && app.proofs.ticket.is_some() {
                let months = app.fund.snap.months.clone();
                app.proofs.key(key, &months);
                return None;
            }
            if app.view == View::Markets {
                if app.markets.ticket.is_some() {
                    if key.code == KeyCode::Char('q')
                        && !matches!(
                            app.markets.ticket.as_ref().map(|t| &t.stage),
                            Some(crate::research::markets::Stage::Edit)
                        )
                    {
                        return Some(Action::Quit);
                    }
                    app.markets.key(key);
                    return None;
                }
                if app.markets.key(key) {
                    return None;
                }
            }
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Some(Action::Quit),
                KeyCode::Char('m') => return Some(Action::View(View::Markets)),
                KeyCode::Char('c') => return Some(Action::View(View::Connect)),
                KeyCode::Char('a') => return Some(Action::View(View::Activity)),
                KeyCode::Char('e') => return Some(Action::View(View::Evidence)),
                KeyCode::Char('r') => return Some(Action::View(View::Rewards)),
                KeyCode::Char('f') => return Some(Action::View(View::Fund)),
                KeyCode::Char('n') => return Some(Action::NewAgent),
                KeyCode::Char('R') => return Some(Action::Refresh),
                // Research's navigator: one level up.
                KeyCode::Backspace if app.view == View::Activity => {
                    let path = app.nav().path;
                    if path.depth() > 0 {
                        return Some(Action::Nav(path.up()));
                    }
                }
                KeyCode::Tab if !app.hits.is_empty() => {
                    app.focus = (app.focus + 1) % app.hits.len()
                }
                KeyCode::BackTab if !app.hits.is_empty() => {
                    app.focus = (app.focus + app.hits.len() - 1) % app.hits.len()
                }
                KeyCode::Enter => return app.hits.get(app.focus).map(|h| h.action.clone()),
                KeyCode::Up | KeyCode::Down if !app.snapshot.agents.is_empty() => {
                    let length = app.snapshot.agents.len();
                    let current = app
                        .snapshot
                        .agents
                        .iter()
                        .position(|a| Some(&a.id) == app.selected_agent.as_ref())
                        .unwrap_or(0);
                    let next = if key.code == KeyCode::Down {
                        (current + 1) % length
                    } else {
                        (current + length - 1) % length
                    };
                    if next < app.agent_scroll {
                        app.agent_scroll = next;
                    }
                    return Some(Action::Agent(app.snapshot.agents[next].id.clone()));
                }
                KeyCode::PageDown => app.scroll = app.scroll.saturating_add(10),
                KeyCode::PageUp => app.scroll = app.scroll.saturating_sub(10),
                KeyCode::Home => app.scroll = 0,
                _ => {}
            }
        }
        _ => {}
    }
    None
}
