mod browse;
mod player;

use super::theme::Theme;
use crate::app::state::{App, View};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Paragraph, TableState, Wrap},
};

pub fn render(frame: &mut Frame, app: &App, provider: &str, table: &mut TableState) {
    let theme = Theme::from_env();
    let area = frame.area();
    frame.render_widget(Block::default().style(theme.base()), area);
    let full_player = app.view == View::NowPlaying && !app.show_help;
    let dock_height = if full_player || app.opened.is_none() {
        0
    } else if area.height >= 18 {
        4
    } else {
        1
    };
    let compact = area.width < 72;
    let [header, second_row, body, notice, footer, dock] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(u16::from(compact && area.height >= 11)),
        Constraint::Min(0),
        Constraint::Length(u16::from(app.notice.is_some())),
        Constraint::Length(1),
        Constraint::Length(dock_height),
    ])
    .areas(area);

    let view = match app.view {
        View::Discover => "Discover",
        View::Search => "Search",
        View::Queue => "Queue",
        View::NowPlaying => "Player",
    };
    let provider_width = provider.chars().count() as u16 + 1;
    let [brand, main_nav, source] = Layout::horizontal([
        if compact {
            Constraint::Fill(1)
        } else {
            Constraint::Length(10)
        },
        if compact {
            Constraint::Length(0)
        } else {
            Constraint::Fill(1)
        },
        Constraint::Length(provider_width.min(area.width / 2)),
    ])
    .areas(header);
    let mut identity = vec![Span::styled(" Mélimo ", theme.selected())];
    if compact {
        identity.push(Span::styled(
            format!(" / {}", if app.show_help { "Keys" } else { view }),
            theme.title(),
        ));
    }
    frame.render_widget(Line::from(identity), brand);
    frame.render_widget(
        Line::styled(format!("{provider} "), theme.secondary()).right_aligned(),
        source,
    );
    let mut nav_items = Vec::new();
    for (name, key) in [
        ("Discover", "d"),
        ("Search", "/"),
        ("Queue", "b"),
        ("Player", "p"),
    ] {
        if name == "Discover" && app.search_provider == crate::provider::ProviderId::Invidious {
            continue;
        }
        let label = format!(" {key} {name} ");
        nav_items.push(Span::styled(
            label,
            if name == view && !app.show_help {
                theme.accent().underlined()
            } else {
                theme.secondary()
            },
        ));
    }
    frame.render_widget(
        Line::from(nav_items),
        if compact { second_row } else { main_nav },
    );

    if app.show_help {
        render_help(frame, body, &theme);
    } else {
        match app.view {
            View::Discover => browse::discover(frame, body, app, &theme, table),
            View::Search => browse::search(frame, body, app, &theme, table),
            View::Queue => browse::queue(frame, body, app, &theme, table),
            View::NowPlaying => player::full(frame, body, app, &theme),
        }
    }
    if let Some(message) = &app.notice {
        frame.render_widget(Line::styled(format!(" {message}"), theme.signal()), notice);
    }

    let keys: &[(&str, &str)] = if app.show_help {
        &[("Esc", "close help"), ("Ctrl+C", "quit")]
    } else if app.editing {
        &[("Enter", "search"), ("Esc", "cancel"), ("Ctrl+U", "clear")]
    } else if app.view == View::Queue && app.queue.is_empty() {
        &[("/", "search"), (":", "append saved queue")]
    } else if app.view == View::Queue {
        &[
            ("J/K", "move"),
            ("E", "to front"),
            ("Del", "remove"),
            ("Enter", "play"),
            ("r", "shuffle"),
        ]
    } else if app.view == View::Discover {
        &[("Enter", "browse"), ("←/→", "group"), ("/", "search")]
    } else if app.view == View::NowPlaying {
        &[
            ("l", "lyrics"),
            ("b", "queue"),
            ("[", "previous"),
            ("n", "next"),
        ]
    } else if app.selected.is_none() {
        &[("/", "search"), ("b", "queue")]
    } else if app.showing_playlists {
        &[("Enter", "inspect"), ("a", "play"), ("r", "shuffle")]
    } else {
        &[
            ("Enter", "play"),
            ("E", "play next"),
            ("e", "enqueue"),
            ("a", "play all"),
        ]
    };
    let palette_width = if app.show_help || app.editing {
        0
    } else if area.width < 65 {
        8
    } else {
        20
    };
    let [hints, commands] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(palette_width)]).areas(footer);
    frame.render_widget(key_line(keys, hints.width, &theme), hints);
    frame.render_widget(
        Line::styled(
            if palette_width == 8 {
                ": cmds "
            } else {
                ": commands   ? help "
            },
            theme.accent(),
        )
        .right_aligned(),
        commands,
    );
    player::dock(frame, dock, app, &theme);
}

fn key_line(keys: &[(&str, &str)], width: u16, theme: &Theme) -> Line<'static> {
    use unicode_width::UnicodeWidthStr;
    let mut spans = Vec::new();
    let mut remaining = usize::from(width);
    for &(key, description) in keys {
        let length = 3 + key.width() + description.width();
        if length > remaining {
            break;
        }
        remaining -= length;
        spans.push(Span::styled(format!(" {key} "), theme.accent()));
        spans.push(Span::styled(format!("{description} "), theme.base()));
    }
    Line::from(spans)
}

fn state(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    title: &str,
    heading: &str,
    hint: &str,
    detail: &str,
) {
    let block = theme.panel(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [_, content] = Layout::vertical([
        Constraint::Length(inner.height.saturating_sub(5).min(3)),
        Constraint::Min(0),
    ])
    .areas(inner);
    let mut lines = vec![
        Line::styled(heading, theme.title()),
        Line::styled(hint, theme.accent()),
    ];
    if !detail.is_empty() {
        lines.push(Line::styled(detail, theme.secondary()));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), content);
}

fn render_help(frame: &mut Frame, area: Rect, theme: &Theme) {
    let block = theme.panel("Keyboard shortcuts");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let navigation = [
        ("Navigation", ""),
        (":", "Find an action"),
        ("/ · Enter", "Edit / submit search"),
        ("Ctrl+U · ⌫", "Clear / erase query"),
        ("↑/↓ · j/k", "Select a row"),
        ("g/G", "First / last row"),
        ("←/→", "Change Discover group"),
        ("d · p · b", "Discover / player / queue"),
        ("Tab · P", "Search type / provider"),
        ("L", "Refresh Deezer login"),
        ("q/Esc", "Back, then quit"),
        ("Ctrl+C", "Quit immediately"),
    ];
    let playback = [
        ("Playback & queue", ""),
        ("Enter", "Play track / open playlist"),
        ("e · E", "Queue last / play next"),
        ("J/K · Del", "Move down/up / remove"),
        ("Space", "Pause / resume"),
        ("[ · n", "Previous / next track"),
        ("←/→", "Seek 10 seconds in player"),
        ("+/- · m", "Volume / mute"),
        ("a · r", "Play all / shuffle"),
        ("l · f", "Lyrics / Deezer favorite"),
        ("s", "Stop and clear queue"),
        (":", "Save / append a saved queue"),
    ];
    let section = |items: &[(&str, &str)]| -> Vec<Line<'static>> {
        items
            .iter()
            .map(|&(key, desc)| {
                if desc.is_empty() {
                    Line::styled(key.to_owned(), theme.signal())
                } else {
                    Line::from(vec![
                        Span::styled(format!("{key:<12}"), theme.accent()),
                        Span::raw(desc.to_owned()),
                    ])
                }
            })
            .collect()
    };
    if inner.width >= 82 {
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(48), Constraint::Fill(1)]).areas(inner);
        frame.render_widget(Paragraph::new(section(&navigation)), left);
        frame.render_widget(Paragraph::new(section(&playback)), right);
    } else {
        // The searchable command menu also exposes these controls on a short screen.
        let compact = [
            (":", "Find any action by name"),
            ("/ · Enter", "Edit / submit search"),
            ("j/k · g/G", "Select / first or last"),
            ("d / p / b", "Discover / player / queue"),
            ("e / E", "Queue last / play next"),
            ("J/K · Del", "Move down/up / remove"),
            ("[ / n", "Previous / next"),
            ("Space · ←/→", "Pause / seek"),
            ("+/- · m", "Volume / mute"),
            ("a/r · l/f", "Play/shuffle · lyrics/favorite"),
            ("Tab · P", "Search type / provider"),
            ("s · L", "Stop & clear / refresh login"),
            (":", "Save / append a saved queue"),
            ("Esc · Ctrl+C", "Back / quit"),
        ];
        frame.render_widget(Paragraph::new(section(&compact)), inner);
    }
}

fn format_time(seconds: u64) -> String {
    let mut buffer = std::fmt::NumBuffer::new();
    let minutes = (seconds / 60).format_into(&mut buffer);
    let mut text = String::with_capacity(minutes.len() + 3);
    text.push_str(minutes);
    text.push(':');
    let remainder = seconds % 60;
    if remainder < 10 {
        text.push('0');
    }
    text.push_str(remainder.format_into(&mut buffer));
    text
}

#[cfg(test)]
mod tests {
    use super::player::playback_label;
    use super::*;
    use crate::app::state::{DISCOVER, DISCOVER_GROUPS, PlaybackState};
    use ratatui::{Terminal, backend::TestBackend};
    #[test]
    fn provider_labels_follow_results_queue_and_player() {
        use crate::provider::{ProviderId, Track};
        let track = Track {
            provider: ProviderId::Invidious,
            id: "abcdefghijk".into(),
            title: "Synthetic video".into(),
            artist: "Channel".into(),
            album: String::new(),
            duration_secs: 120,
        };
        let mut app = App::default();
        app.search_provider = ProviderId::Invidious;
        app.tracks = vec![track.clone()];
        app.queue = vec![track.clone()].into();
        app.opened = Some(track);
        app.selected = Some(0);
        app.searched = true;
        for width in [40, 60, 100] {
            for view in [View::Search, View::Queue, View::NowPlaying] {
                app.view = view;
                let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
                terminal
                    .draw(|f| {
                        render(
                            f,
                            &app,
                            app.search_provider.name(),
                            &mut TableState::default(),
                        )
                    })
                    .unwrap();
                let text: String = terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .map(|c| c.symbol())
                    .collect();
                assert!(text.contains("[INV]"));
                assert!(text.contains("Channel"));
            }
        }
    }
    #[test]
    fn renders_small_and_normal_terminals() {
        for (width, height) in [(1, 1), (20, 5), (80, 24)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let mut app = App::default();
            app.update(crate::app::action::Action::FocusSearch);
            for c in "Écho 日本".repeat(30).chars() {
                app.update(crate::app::action::Action::Insert(c));
            }
            terminal
                .draw(|frame| render(frame, &app, "Mock", &mut TableState::default()))
                .unwrap();
        }
    }
    #[test]
    fn player_renders_progress_controls_and_active_lyric_on_all_sizes() {
        use crate::provider::{LyricLine, Lyrics, Track};
        let mut app = App::default();
        app.update(crate::app::action::Action::ToggleLyrics);
        app.opened = Some(Track {
            provider: crate::provider::ProviderId::Mock,
            id: "demo".into(),
            title: "Demo".into(),
            artist: "Demo".into(),
            album: "Demo".into(),
            duration_secs: 120,
        });
        app.view = View::NowPlaying;
        app.elapsed = 30;
        app.elapsed_ms = 30500;
        app.playback = PlaybackState::Paused;
        app.pause_requested = true;
        app.lyrics = Some(Ok(Lyrics {
            lines: vec![
                LyricLine {
                    at_ms: 0,
                    text: "Opening demo line".into(),
                },
                LyricLine {
                    at_ms: 30250,
                    text: "Active demo line".into(),
                },
                LyricLine {
                    at_ms: 40000,
                    text: "Next demo line".into(),
                },
            ],
            ..Default::default()
        }));
        for (width, height) in [(1, 1), (20, 5), (40, 18), (60, 24), (100, 24)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| render(frame, &app, "Mock", &mut TableState::default()))
                .unwrap();
            if width >= 40 {
                let text: String = terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .map(|c| c.symbol())
                    .collect();
                assert!(text.contains("0:30 / 2:00"));
                assert!(text.contains("Resume"));
                assert!(text.contains("m mute"));
                let buffer = terminal.backend().buffer();
                assert!(buffer.content.iter().any(|cell| cell.symbol() == "D"
                    && cell.modifier.contains(ratatui::style::Modifier::BOLD)));
                assert!(text.contains("› Active demo line"));
            }
        }
    }

    #[test]
    fn bottom_player_persists_across_views_and_input_modes() {
        let mut app = App::default();
        app.opened = Some(crate::provider::Track {
            provider: crate::provider::ProviderId::Mock,
            id: "demo".into(),
            title: "Sunrise Drive".into(),
            artist: "Lofi Keys".into(),
            album: "Demo".into(),
            duration_secs: 236,
        });
        app.elapsed = 84;
        for width in [60, 100] {
            for view in [View::Discover, View::Search, View::Queue] {
                app.view = view;
                for state in [
                    PlaybackState::Playing,
                    PlaybackState::Paused,
                    PlaybackState::Loading,
                    PlaybackState::Error("Unavailable".into()),
                ] {
                    app.playback = state;
                    for (editing, help) in [(false, false), (true, false), (false, true)] {
                        app.editing = editing;
                        app.show_help = help;
                        let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
                        terminal
                            .draw(|frame| render(frame, &app, "Mock", &mut TableState::default()))
                            .unwrap();
                        let dock: String = terminal
                            .backend()
                            .buffer()
                            .content
                            .iter()
                            .skip(usize::from(width) * 18)
                            .map(|cell| cell.symbol())
                            .collect();
                        assert!(dock.contains("Sunrise Drive"));
                        assert!(dock.contains("Lofi Keys"));
                        if matches!(app.playback, PlaybackState::Error(_)) {
                            assert!(dock.contains("Unavailable"));
                        } else {
                            assert!(dock.contains("1:24 / 3:56"));
                        }
                        assert!(dock.contains(playback_label(&app)));
                        assert!(dock.contains(if editing {
                            "Esc leaves search"
                        } else if help {
                            "Close help"
                        } else if matches!(app.playback, PlaybackState::Error(_)) {
                            "n skips"
                        } else {
                            "n next"
                        }));
                    }
                }
            }
        }
    }

    #[test]
    fn discovery_columns_keep_each_selected_item_visible() {
        let mut app = App::default();
        app.view = View::Discover;
        for (width, height) in [(60, 18), (100, 18), (120, 24)] {
            for (index, (label, _)) in DISCOVER.iter().enumerate() {
                app.selected = Some(index);
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal
                    .draw(|frame| render(frame, &app, "Mock", &mut TableState::default()))
                    .unwrap();
                let text: String = terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect();
                assert!(
                    text.contains(label.strip_prefix("Genre · ").unwrap_or(label)),
                    "missing {label} at {width}x{height}"
                );
                if width >= 90 {
                    for (title, _) in DISCOVER_GROUPS {
                        assert!(text.contains(title));
                    }
                }
            }
        }
    }

    #[test]
    fn state_panels_keep_recovery_visible_on_narrow_terminals() {
        for (width, height) in [(40, 16), (60, 24), (100, 24)] {
            for (view, loading, searched, error, heading, hint) in [
                (
                    View::Queue,
                    false,
                    false,
                    None,
                    "Your queue is empty",
                    "Browse with d",
                ),
                (
                    View::NowPlaying,
                    false,
                    false,
                    None,
                    "Nothing playing yet",
                    "Press d",
                ),
                (
                    View::Search,
                    true,
                    false,
                    None,
                    "Finding your music",
                    "Press d",
                ),
                (
                    View::Search,
                    false,
                    false,
                    None,
                    "Find your next song",
                    "Press /",
                ),
                (
                    View::Search,
                    false,
                    true,
                    None,
                    "No music found",
                    "Try other keywords",
                ),
                (
                    View::Search,
                    false,
                    true,
                    Some("Synthetic failure".into()),
                    "Could not load music",
                    "Press /",
                ),
            ] {
                let mut app = App::default();
                app.view = view;
                app.loading = loading;
                app.searched = searched;
                app.error = error;
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal
                    .draw(|frame| render(frame, &app, "Mock", &mut TableState::default()))
                    .unwrap();
                let text: String = terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect();
                assert!(text.contains(heading), "{width}x{height}: {heading}");
                assert!(text.contains(hint), "{width}x{height}: {hint}");
            }
        }
    }

    #[test]
    fn short_player_hides_transport_hints() {
        let mut app = App::default();
        app.opened = Some(crate::provider::Track {
            provider: crate::provider::ProviderId::Mock,
            id: "demo".into(),
            title: "Demo".into(),
            artist: "Artist".into(),
            album: "Album".into(),
            duration_secs: 0,
        });
        app.view = View::NowPlaying;
        let mut terminal = Terminal::new(TestBackend::new(60, 16)).unwrap();
        terminal
            .draw(|frame| render(frame, &app, "Mock", &mut TableState::default()))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("Demo"));
        assert!(!text.contains("Space Pause"));
        assert!(!text.contains("m mute"));
    }

    fn snapshot(app: &App, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                render(
                    frame,
                    app,
                    app.search_provider.name(),
                    &mut TableState::default(),
                )
            })
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .chunks(usize::from(width))
            .map(|row| row.iter().map(|cell| cell.symbol()).collect())
            .collect()
    }

    #[test]
    fn listening_desk_reflows_without_repeating_the_track() {
        let mut app = App::default();
        let song = crate::provider::Track {
            provider: crate::provider::ProviderId::Deezer,
            id: "1".into(),
            title: "Départ".into(),
            artist: "Orchestra".into(),
            album: "Night".into(),
            duration_secs: 180,
        };
        app.opened = Some(song.clone());
        app.queue.push_back(crate::provider::Track {
            title: "Morning".into(),
            ..song
        });
        app.view = View::NowPlaying;
        let wide = snapshot(&app, 100, 24);
        assert!(
            wide.iter()
                .any(|row| row.contains("Now playing") && row.contains("Up next"))
        );
        let narrow = snapshot(&app, 40, 18);
        let player = narrow
            .iter()
            .position(|row| row.contains("Départ"))
            .unwrap();
        let next = narrow
            .iter()
            .position(|row| row.contains("Morning"))
            .unwrap();
        assert!(next > player);
        for rows in [wide, narrow] {
            let text = rows.join("\n");
            assert_eq!(text.matches("Départ").count(), 1);
            assert!(text.contains("0:00 / 3:00"));
        }
    }

    #[test]
    fn queue_reports_total_and_surfaces_reordering_and_commands() {
        let mut app = App::default();
        app.view = View::Queue;
        for (id, seconds) in [("one", 70), ("two", 125)] {
            app.queue.push_back(crate::provider::Track {
                provider: crate::provider::ProviderId::Deezer,
                id: id.into(),
                title: id.into(),
                artist: "Mix".into(),
                album: "Night".into(),
                duration_secs: seconds,
            });
        }
        app.selected = Some(1);
        let text = snapshot(&app, 100, 20).join("\n");
        for expected in [
            "Up next · 2 · 3:15 total",
            "1:10 queued before selection",
            "J/K move",
            "E to front",
            ": commands",
            "Album",
        ] {
            assert!(text.contains(expected), "{expected}");
        }
        app.queue[0].title = "Night Train to Sendai".into();
        app.queue[0].duration_secs = 3600;
        let small = snapshot(&app, 40, 14).join("\n");
        assert!(small.contains("1h 02m total"));
        assert!(small.contains("Night Train to Sendai"));
        assert!(small.contains("[DZR] Mix"));
        assert!(small.contains(": cmds"));
        assert!(!small.contains("Album"));
    }

    #[test]
    fn last_and_first_track_remain_visible_when_a_long_queue_reflows() {
        let mut app = App::default();
        app.view = View::Queue;
        app.queue = (0..1000)
            .map(|i| crate::provider::Track {
                provider: crate::provider::ProviderId::Deezer,
                id: i.to_string(),
                title: format!("Song {i:03}"),
                artist: "Group".into(),
                album: String::new(),
                duration_secs: 60,
            })
            .collect();
        let mut table = TableState::default();
        for (index, width, height) in [(999, 40, 13), (998, 100, 25), (2, 40, 13), (0, 60, 20)] {
            app.selected = Some(index);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| render(frame, &app, "Demo", &mut table))
                .unwrap();
            let text: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(text.contains(&format!("Song {index:03}")));
            assert_eq!(table.selected(), Some(index));
        }
    }

    #[test]
    fn time_formatting() {
        assert_eq!(format_time(65), "1:05");
        assert_eq!(format_time(0), "0:00");
        for seconds in [9, 10, 59, 60, 599, 600, 3599, 3600, u64::MAX] {
            assert_eq!(
                format_time(seconds),
                format!("{}:{:02}", seconds / 60, seconds % 60)
            );
        }
    }
}
