use super::{event, theme::Theme};
use crate::{
    app::{
        action::Action,
        state::{App, PlaybackState, View},
    },
    provider::ProviderId,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Margin},
    text::Line,
    widgets::{Cell, Clear, Paragraph, Row, Table, TableState},
};
use unicode_width::UnicodeWidthStr;

#[derive(Default)]
pub struct Palette {
    query: String,
    selected: Option<&'static str>,
}

struct Command {
    label: &'static str,
    aliases: &'static str,
    binding: Binding,
}

enum Binding {
    Key(KeyCode),
    Direct(Action),
}

impl Command {
    fn run(self, app: &App) -> Option<Action> {
        match self.binding {
            Binding::Key(code) => event::shortcut(code, app),
            Binding::Direct(action) => Some(action),
        }
    }

    fn matches(&self, terms: &str) -> bool {
        let label = self.label.to_ascii_lowercase();
        terms
            .split_whitespace()
            .all(|term| label.contains(term) || self.aliases.contains(term))
    }
}

fn commands(app: &App) -> Vec<Command> {
    use KeyCode::{Char, Delete, Enter, Tab};
    let mut items = Vec::new();
    let mut add = |key, label, aliases| {
        items.push(Command {
            label,
            aliases,
            binding: Binding::Key(key),
        })
    };

    let selected_track = match app.view {
        View::Search if !app.showing_playlists => app.selected.and_then(|i| app.tracks.get(i)),
        View::Queue => app.selected.and_then(|i| app.queue.get(i)),
        View::NowPlaying => app.opened.as_ref(),
        _ => None,
    };
    match app.view {
        View::Search
            if app.showing_playlists
                && app.selected.and_then(|i| app.playlists.get(i)).is_some() =>
        {
            add(Enter, "Open selected playlist", "inspect browse");
            add(Char('a'), "Play selected playlist", "play all");
            add(Char('r'), "Shuffle selected playlist", "random play");
        }
        View::Search if selected_track.is_some() => {
            add(Enter, "Play selected track", "listen now song");
            add(Char('E'), "Play next in queue", "enqueue add front song");
            add(
                Char('e'),
                "Add to end of queue",
                "enqueue append track song",
            );
            add(Char('a'), "Play all results", "tracks list");
            add(Char('r'), "Shuffle all results", "random play");
        }
        View::Queue if selected_track.is_some() => {
            add(Enter, "Play from selected track", "skip queue song");
            let index = app.selected.unwrap_or_default();
            if index > 0 {
                add(Char('E'), "Move to front of queue", "promote play next");
                add(Char('K'), "Move up in queue", "reorder earlier");
            }
            if index + 1 < app.queue.len() {
                add(Char('J'), "Move down in queue", "reorder later");
            }
            add(Delete, "Remove from queue", "delete item song");
            add(Char('r'), "Shuffle queue and play", "random");
        }
        View::Discover if app.selected.is_some() => {
            add(Enter, "Open selected collection", "browse discover")
        }
        _ => {}
    }
    if selected_track.is_some_and(|track| track.provider == ProviderId::Deezer) {
        add(
            Char('f'),
            "Toggle Deezer favorite",
            "like favourite library",
        );
    }

    add(Char('/'), "Search music", "find tracks playlists songs");
    if app.view != View::Queue {
        add(Char('b'), "Open queue", "up next list");
    }
    if app.view != View::NowPlaying && app.opened.is_some() {
        add(Char('p'), "Open player", "now playing current track");
    }
    if app.search_provider != ProviderId::Invidious {
        if app.view != View::Discover {
            add(Char('d'), "Open Discover", "browse music genres moods flow");
        }
        add(
            Tab,
            if app.playlist_search {
                "Switch to track search"
            } else {
                "Switch to playlist search"
            },
            "change search type",
        );
    }
    if app.providers.len() > 1 {
        add(
            Char('P'),
            "Switch search provider",
            "change source deezer invidious",
        );
    }
    let active_playback = matches!(
        app.playback,
        PlaybackState::Playing | PlaybackState::Paused | PlaybackState::Loading
    );
    if active_playback {
        add(
            Char(' '),
            if app.pause_requested || app.playback == PlaybackState::Paused {
                "Resume playback"
            } else {
                "Pause playback"
            },
            "play pause resume",
        );
    }
    if app.view == View::NowPlaying
        && app.opened.is_some()
        && !matches!(
            app.playback,
            PlaybackState::Stopped | PlaybackState::Error(_)
        )
    {
        add(KeyCode::Left, "Seek back 10 seconds", "rewind earlier");
        add(KeyCode::Right, "Seek forward 10 seconds", "skip ahead");
    }
    if app.has_previous() {
        add(Char('['), "Play previous track", "back history song");
    }
    if !app.queue.is_empty() {
        add(Char('n'), "Play next queued track", "skip forward song");
    }
    if app
        .opened
        .as_ref()
        .is_some_and(|track| track.provider == ProviderId::Deezer)
    {
        add(
            Char('l'),
            if app.karaoke {
                "Hide lyrics"
            } else {
                "Show lyrics"
            },
            "karaoke words",
        );
    }
    if !app.queue.is_empty() || active_playback {
        add(Char('s'), "Stop and clear queue", "end playback empty");
    }
    add(
        Char('m'),
        if app.volume.is_muted() {
            "Unmute audio"
        } else {
            "Mute audio"
        },
        "volume sound",
    );
    add(Char('+'), "Increase volume", "louder gain up");
    add(Char('-'), "Decrease volume", "quieter gain down");
    add(Char('?'), "Show keyboard shortcuts", "help controls keys");
    if app.providers.contains(&ProviderId::Deezer) {
        add(
            Char('L'),
            "Refresh login and clear queue",
            "sign in reconnect deezer",
        );
    }
    if !app.queue.is_empty() {
        items.push(Command {
            label: "Save queue",
            aliases: "store snapshot session",
            binding: Binding::Direct(Action::SaveQueue),
        });
    }
    items.push(Command {
        label: "Append saved queue",
        aliases: "restore load session snapshot",
        binding: Binding::Direct(Action::RestoreQueue),
    });
    items
}

impl Palette {
    fn matches(&self, app: &App) -> Vec<Command> {
        let terms = self.query.to_ascii_lowercase();
        commands(app)
            .into_iter()
            .filter(|cmd| cmd.matches(&terms))
            .collect()
    }

    // Playback and search can update the available commands while the palette is open.
    // Keep a deliberately highlighted command attached to its identity until it is run.
    fn selected_index(&self, matches: &[Command]) -> Option<usize> {
        match self.selected {
            Some(label) => matches.iter().position(|command| command.label == label),
            None => (!matches.is_empty()).then_some(0),
        }
    }

    pub(super) fn handle_key(slot: &mut Option<Self>, key: KeyEvent, app: &App) -> Option<Action> {
        let palette = slot.as_mut()?;
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            if key.code == KeyCode::Char('u') {
                palette.query.clear();
                palette.selected = None;
                return Some(Action::Redraw);
            }
            return None;
        }
        if !key.modifiers.difference(KeyModifiers::SHIFT).is_empty() {
            return None;
        }
        match key.code {
            KeyCode::Esc => *slot = None,
            KeyCode::Enter => {
                let matches = palette.matches(app);
                let index = palette.selected_index(&matches)?;
                let command = matches.into_iter().nth(index)?;
                *slot = None;
                return command.run(app);
            }
            KeyCode::Down | KeyCode::Up | KeyCode::Home | KeyCode::End => {
                let matches = palette.matches(app);
                let last = matches.len().checked_sub(1)?;
                let selected = palette.selected_index(&matches);
                let index = match key.code {
                    KeyCode::End => last,
                    KeyCode::Down => selected.map_or(0, |i| (i + 1).min(last)),
                    KeyCode::Up => selected.unwrap_or(0).saturating_sub(1),
                    _ => 0,
                };
                palette.selected = Some(matches[index].label);
            }
            KeyCode::Char(ch) if !ch.is_control() => {
                palette.query.push(ch);
                palette.selected = None;
            }
            KeyCode::Backspace => {
                palette.query.pop();
                palette.selected = None;
            }
            _ => return None,
        }
        Some(Action::Redraw)
    }

    pub fn render(&self, frame: &mut Frame, app: &App) {
        let theme = Theme::from_env();
        let size = frame.area();
        let area = size.centered(Constraint::Length(58), Constraint::Length(13));
        let matches = self.matches(app);
        let selected = self.selected_index(&matches);
        let mut panel = theme.panel(" Commands ");
        if !matches.is_empty() && area.width >= 26 {
            panel = panel.title_bottom(
                Line::styled(
                    format!(" {} / {} ", selected.map_or(0, |i| i + 1), matches.len()),
                    theme.secondary(),
                )
                .right_aligned(),
            );
        }
        let inner = panel.inner(area).inner(Margin::new(1, 0));
        frame.render_widget(Clear, area);
        frame.render_widget(panel, area);
        if inner.is_empty() {
            return;
        }
        let [search, list, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(u16::from(inner.height >= 4)),
        ])
        .areas(inner);
        let [prompt, input] =
            Layout::horizontal([Constraint::Length(2), Constraint::Fill(1)]).areas(search);
        frame.render_widget(Paragraph::new("›").style(theme.accent()), prompt);
        if !input.is_empty() {
            let query_width = UnicodeWidthStr::width(self.query.as_str());
            let offset = query_width.saturating_sub(usize::from(input.width - 1));
            let query = if self.query.is_empty() {
                Paragraph::new("Find a command…").style(theme.secondary())
            } else {
                Paragraph::new(self.query.as_str())
                    .style(theme.title())
                    .scroll((0, offset.min(u16::MAX as usize) as u16))
            };
            frame.render_widget(query, input);
            frame.set_cursor_position((input.x + (query_width - offset) as u16, input.y));
        }

        if matches.is_empty() {
            frame.render_widget(
                Paragraph::new("No matching commands").style(theme.secondary()),
                list,
            );
        } else {
            let rows = matches.iter().map(|command| {
                let key = match command.binding {
                    Binding::Key(code) => code.to_string(),
                    Binding::Direct(_) => String::new(),
                };
                Row::new([
                    Cell::from(command.label),
                    Cell::from(Line::from(key).right_aligned()),
                ])
            });
            let table = Table::new(rows, [Constraint::Fill(1), Constraint::Length(8)])
                .highlight_symbol("› ")
                .row_highlight_style(theme.selected())
                .style(theme.base());
            frame.render_stateful_widget(
                table,
                list,
                &mut TableState::default().with_selected(selected),
            );
        }
        frame.render_widget(
            Paragraph::new("↑↓ Select   Enter Run   Esc Close").style(theme.secondary()),
            footer,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn rendered(palette: &Palette, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| palette.render(frame, &App::default()))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn compact_palette_displays_matching_commands_and_empty_state() {
        let mut palette = Palette {
            query: "load session".into(),
            ..Default::default()
        };
        let screen = rendered(&palette, 40, 10);
        for text in [
            "Commands",
            "load session",
            "Append saved queue",
            "Esc Close",
            "1 / 1",
        ] {
            assert!(screen.contains(text), "missing {text} in {screen}");
        }
        palette.query = "界".repeat(60);
        assert!(rendered(&palette, 40, 6).contains("No matching commands"));
        for (w, h) in [(20, 4), (4, 4), (1, 1)] {
            rendered(&palette, w, h);
        }
    }

    #[test]
    fn retiring_a_highlighted_command_does_not_run_its_new_neighbor() {
        let mut app = App::default();
        app.playback = PlaybackState::Playing;
        let mut slot = Some(Palette {
            query: "playback".into(),
            ..Default::default()
        });
        let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        Palette::handle_key(
            &mut slot,
            KeyEvent::new(KeyCode::Home, KeyModifiers::NONE),
            &app,
        );
        assert_eq!(slot.as_ref().unwrap().selected, Some("Pause playback"));
        app.playback = PlaybackState::Paused;
        assert!(Palette::handle_key(&mut slot, enter, &app).is_none());
        assert!(slot.is_some());
        Palette::handle_key(
            &mut slot,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            &app,
        );
        assert!(matches!(
            Palette::handle_key(&mut slot, enter, &app),
            Some(Action::TogglePause)
        ));
    }

    #[test]
    fn commands_only_offer_applicable_controls() {
        let mut app = App::default();
        app.providers = vec![ProviderId::Invidious];
        app.search_provider = ProviderId::Invidious;
        let available: Vec<_> = commands(&app).into_iter().map(|c| c.label).collect();
        for unavailable in [
            "Save queue",
            "Open Discover",
            "Switch search provider",
            "Switch to playlist search",
            "Move down in queue",
            "Pause playback",
            "Play previous track",
        ] {
            assert!(!available.contains(&unavailable), "{unavailable}");
        }
        assert!(available.contains(&"Append saved queue"));
    }
}
