use super::palette::Palette;
use crate::app::{
    action::Action,
    state::{App, View},
};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::{io, time::Duration};

pub fn read_action(app: &App, palette: &mut Option<Palette>) -> io::Result<Option<Action>> {
    if !event::poll(Duration::from_millis(50))? {
        return Ok(None);
    }
    match event::read()? {
        Event::Key(key) => Ok(dispatch_key(key, app, palette)),
        Event::Resize(..) => Ok(Some(Action::Redraw)),
        _ => Ok(None),
    }
}

fn dispatch_key(key: KeyEvent, app: &App, palette: &mut Option<Palette>) -> Option<Action> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Some(Action::Quit);
    }
    if palette.is_some() {
        return Palette::handle_key(palette, key, app);
    }
    if !app.show_help
        && !app.editing
        && key.code == KeyCode::Char(':')
        && key.modifiers.difference(KeyModifiers::SHIFT).is_empty()
    {
        *palette = Some(Palette::default());
        return Some(Action::Redraw);
    }
    map_key(key, app)
}

fn map_key(key: KeyEvent, app: &App) -> Option<Action> {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('u') if app.editing && !app.show_help => Some(Action::ClearQuery),
            _ => None,
        };
    }
    if app.show_help {
        return match key.code {
            KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Esc => Some(Action::Back),
            _ => None,
        };
    }
    if app.editing {
        return match key.code {
            KeyCode::Esc => Some(Action::Back),
            KeyCode::Enter => Some(Action::SubmitSearch),
            KeyCode::Backspace => Some(Action::Backspace),
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::ALT) => {
                Some(Action::Insert(c))
            }
            _ => None,
        };
    }
    shortcut(key.code, app)
}

pub(super) fn shortcut(code: KeyCode, app: &App) -> Option<Action> {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => Some(Action::Back),
        KeyCode::Char('?') => Some(Action::ToggleHelp),
        KeyCode::Char('/') => Some(Action::FocusSearch),
        KeyCode::Left if app.view == View::Discover => Some(Action::DiscoverGroup(-1)),
        KeyCode::Right if app.view == View::Discover => Some(Action::DiscoverGroup(1)),
        KeyCode::Left if app.view == View::NowPlaying => Some(Action::SeekRelative(-10)),
        KeyCode::Right if app.view == View::NowPlaying => Some(Action::SeekRelative(10)),
        KeyCode::Char(' ') => Some(Action::TogglePause),
        KeyCode::Char('+') | KeyCode::Char('=') => Some(Action::VolumeUp),
        KeyCode::Char('-') | KeyCode::Char('_') => Some(Action::VolumeDown),
        KeyCode::Char('m') => Some(Action::ToggleMute),
        KeyCode::Char('s') => Some(Action::StopPlayback),
        KeyCode::Char('e') => Some(Action::Enqueue),
        KeyCode::Char('E') if matches!(app.view, View::Search | View::Queue) => {
            Some(Action::QueueNext)
        }
        KeyCode::Char('J') if app.view == View::Queue => Some(Action::MoveQueued(1)),
        KeyCode::Char('K') if app.view == View::Queue => Some(Action::MoveQueued(-1)),
        KeyCode::Char('[') => Some(Action::PreviousTrack),
        KeyCode::Delete if app.view == View::Queue => Some(Action::RemoveQueued),
        KeyCode::Char('l') => Some(Action::ToggleLyrics),
        KeyCode::Char('p') => Some(Action::ShowPlayer),
        KeyCode::Char('P') => Some(Action::CycleProvider),
        KeyCode::Char('b') => Some(Action::ShowQueue),
        KeyCode::Char('r') => Some(Action::ShufflePlay),
        KeyCode::Char('f') => Some(Action::ToggleFavorite),
        KeyCode::Char('L') => Some(Action::Login),
        KeyCode::Char('d') => Some(Action::Discover),
        KeyCode::Char('n') => Some(Action::NextTrack),
        KeyCode::Tab => Some(Action::ToggleSearchKind),
        KeyCode::Char('a') if app.view == View::Search => Some(Action::PlayAll),
        _ if app.view == View::NowPlaying => None,
        KeyCode::Char('j') | KeyCode::Down => Some(Action::MoveDown),
        KeyCode::Char('k') | KeyCode::Up => Some(Action::MoveUp),
        KeyCode::Char('g') | KeyCode::Home => Some(Action::First),
        KeyCode::Char('G') | KeyCode::End => Some(Action::Last),
        KeyCode::Enter => Some(Action::OpenTrack),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn press(code: KeyCode, app: &App, palette: &mut Option<Palette>) -> Option<Action> {
        dispatch_key(KeyEvent::new(code, KeyModifiers::NONE), app, palette)
    }

    #[test]
    fn palette_consumes_text_and_runs_the_selected_command() {
        let app = App::default();
        let mut palette = None;
        assert!(matches!(
            press(KeyCode::Char(':'), &app, &mut palette),
            Some(Action::Redraw)
        ));
        for ch in "OPEN quEuEé".chars() {
            assert!(matches!(
                press(KeyCode::Char(ch), &app, &mut palette),
                Some(Action::Redraw)
            ));
        }
        assert!(press(KeyCode::Enter, &app, &mut palette).is_none());
        assert!(palette.is_some());
        press(KeyCode::Backspace, &app, &mut palette);
        assert!(matches!(
            press(KeyCode::Enter, &app, &mut palette),
            Some(Action::ShowQueue)
        ));
        assert!(palette.is_none());
        assert!(app.query.is_empty());
    }

    #[test]
    fn palette_closes_without_changing_the_view_and_leaves_other_modes_alone() {
        let mut app = App::default();
        let mut palette = None;
        press(KeyCode::Char(':'), &app, &mut palette);
        press(KeyCode::Char('q'), &app, &mut palette);
        assert!(palette.is_some());
        assert!(matches!(
            press(KeyCode::Esc, &app, &mut palette),
            Some(Action::Redraw)
        ));
        assert!(palette.is_none());
        app.update(Action::FocusSearch);
        assert!(matches!(
            press(KeyCode::Char(':'), &app, &mut palette),
            Some(Action::Insert(':'))
        ));
        app.update(Action::Back);
        app.update(Action::ToggleHelp);
        assert!(press(KeyCode::Char(':'), &app, &mut palette).is_none());
        assert!(palette.is_none());
        app.update(Action::ToggleHelp);
        press(KeyCode::Char(':'), &app, &mut palette);
        assert!(matches!(
            dispatch_key(
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                &app,
                &mut palette
            ),
            Some(Action::Quit)
        ));
    }

    #[test]
    fn palette_only_commands_and_navigation_use_the_actual_queue() {
        let mut app = App::default();
        let track = crate::provider::Track {
            provider: crate::provider::ProviderId::Mock,
            id: "1".into(),
            title: "Track".into(),
            artist: String::new(),
            album: String::new(),
            duration_secs: 90,
        };
        app.queue = [track.clone(), track].into();
        app.update(Action::ShowQueue);
        let mut palette = None;
        press(KeyCode::Char(':'), &app, &mut palette);
        for ch in "session".chars() {
            press(KeyCode::Char(ch), &app, &mut palette);
        }
        press(KeyCode::Down, &app, &mut palette);
        assert!(matches!(
            press(KeyCode::Enter, &app, &mut palette),
            Some(Action::RestoreQueue)
        ));
        press(KeyCode::Char(':'), &app, &mut palette);
        for ch in "session".chars() {
            press(KeyCode::Char(ch), &app, &mut palette);
        }
        assert!(matches!(
            press(KeyCode::Enter, &app, &mut palette),
            Some(Action::SaveQueue)
        ));
        for (key, direction) in [('J', 1), ('K', -1)] {
            assert!(
                matches!(press(KeyCode::Char(key), &app, &mut palette), Some(Action::MoveQueued(d)) if d == direction)
            );
        }
        assert!(matches!(
            press(KeyCode::Char('E'), &app, &mut palette),
            Some(Action::QueueNext)
        ));
        assert!(matches!(
            press(KeyCode::Char('['), &app, &mut palette),
            Some(Action::PreviousTrack)
        ));
    }

    #[test]
    fn volume_shortcuts_are_global_but_not_in_text_entry() {
        let app = App::default();
        for (key, action) in [
            (KeyCode::Char('+'), Action::VolumeUp),
            (KeyCode::Char('='), Action::VolumeUp),
            (KeyCode::Char('-'), Action::VolumeDown),
            (KeyCode::Char('_'), Action::VolumeDown),
            (KeyCode::Char('m'), Action::ToggleMute),
        ] {
            let got = map_key(KeyEvent::new(key, KeyModifiers::NONE), &app);
            assert!(
                matches!((&got, &action), (Some(a), b) if std::mem::discriminant(a) == std::mem::discriminant(b)),
                "{key:?} should map to its volume action"
            );
        }
        // While editing a query, the same keys must stay literal text.
        let mut editing = App::default();
        editing.update(Action::FocusSearch);
        assert!(matches!(
            map_key(
                KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE),
                &editing
            ),
            Some(Action::Insert('m'))
        ));
    }

    #[test]
    fn player_transport_shortcuts() {
        let mut app = App::default();
        app.update(Action::ShowPlayer);
        app.view = View::NowPlaying;
        for (key, delta) in [(KeyCode::Left, -10), (KeyCode::Right, 10)] {
            assert!(
                matches!(map_key(KeyEvent::new(key, KeyModifiers::NONE), &app), Some(Action::SeekRelative(d)) if d == delta)
            );
        }
        assert!(matches!(
            map_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE), &app),
            Some(Action::TogglePause)
        ));
        assert!(matches!(
            map_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE), &app),
            Some(Action::ToggleLyrics)
        ));
    }
    #[test]
    fn text_entry_does_not_trigger_shortcuts() {
        let mut app = App::default();
        app.update(Action::FocusSearch);
        for c in ['q', '/', '?', 'j', 'é', 'l', 'P', ' '] {
            assert!(
                matches!(map_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE), &app), Some(Action::Insert(value)) if value == c)
            );
        }
        assert!(matches!(
            dispatch_key(
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                &app,
                &mut None,
            ),
            Some(Action::Quit)
        ));
        app.editing = false;
        assert!(matches!(
            map_key(KeyEvent::new(KeyCode::Char('P'), KeyModifiers::NONE), &app),
            Some(Action::CycleProvider)
        ));
    }
}
