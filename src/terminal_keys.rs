use crossterm::event::KeyEventKind;

pub(crate) fn is_actionable_key_event(kind: KeyEventKind) -> bool {
    matches!(kind, KeyEventKind::Press | KeyEventKind::Repeat)
}
