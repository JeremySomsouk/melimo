use super::{format_time, state};
use crate::{
    app::state::{App, PlaybackState},
    provider::Track,
    tui::theme::Theme,
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};

pub(super) fn full(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let Some(track) = &app.opened else {
        state(
            frame,
            area,
            theme,
            " Now playing ",
            "Nothing playing yet",
            "Press d to browse or / to search.",
            "Select a track and press Enter to play.",
        );
        return;
    };
    let wide = area.width >= 84 && area.height >= 9;
    let (main, aside) = if wide {
        let [main, aside] = Layout::horizontal([Constraint::Percentage(64), Constraint::Fill(1)])
            .spacing(1)
            .areas(area);
        (main, Some(aside))
    } else {
        (area, None)
    };

    let block = theme.panel(" Now playing ");
    let inner = block.inner(main);
    frame.render_widget(block, main);
    let [metadata, progress, controls, content] = Layout::vertical([
        Constraint::Length(4),
        Constraint::Length(1),
        Constraint::Length(if frame.area().height >= 18 { 2 } else { 0 }),
        Constraint::Min(0),
    ])
    .areas(inner);
    let status = playback_label(app);
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(status, theme.signal()),
            Line::styled(track.title.as_str(), theme.title()),
            Line::raw(track.artist.as_str()),
            Line::styled(
                format!("[{}]  {}", track.provider.label(), track.album),
                theme.secondary(),
            ),
        ]),
        metadata,
    );
    timeline(frame, progress, app, track, theme);
    if let PlaybackState::Error(reason) = &app.playback {
        // Use the listening area for long recovery messages from the provider.
        let recovery = if app.karaoke {
            controls
        } else {
            controls.union(content)
        };
        frame.render_widget(
            Paragraph::new(reason.as_str())
                .style(theme.signal())
                .wrap(Wrap { trim: true }),
            recovery,
        );
    } else {
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled(
                    format!("Space {} · ←/→ seek 10s", pause_verb(app)),
                    theme.accent(),
                ),
                Line::styled(
                    format!("+/- volume {} · m mute", volume_label(app)),
                    theme.secondary(),
                ),
            ]),
            controls,
        );
    }
    if app.karaoke {
        lyrics(frame, content, app, theme);
    } else if !matches!(app.playback, PlaybackState::Error(_)) && aside.is_none() {
        up_next(frame, content, app, theme);
    } else if !matches!(app.playback, PlaybackState::Error(_)) && content.height > 2 {
        frame.render_widget(
            Paragraph::new("l  Show lyrics").style(theme.secondary()),
            content,
        );
    }
    if let Some(aside) = aside {
        up_next(frame, aside, app, theme);
    }
}

fn up_next(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let block = theme.panel(format!(" Up next · {} ", app.queue.len()));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if app.queue.is_empty() {
        frame.render_widget(
            Paragraph::new("e  Add a track from search")
                .style(theme.secondary())
                .wrap(Wrap { trim: true }),
            inner,
        );
        return;
    }
    let [list, shortcut] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(u16::from(inner.height >= 4)),
    ])
    .areas(inner);
    let show_artist = list.height >= 5;
    let rows_per_track = if show_artist { 2 } else { 1 };
    let limit = usize::from(list.height.div_ceil(rows_per_track));
    let mut upcoming = Vec::new();
    for (i, track) in app.queue.iter().take(limit).enumerate() {
        upcoming.push(Line::from(vec![
            Span::styled(format!("{:>2}  ", i + 1), theme.secondary()),
            Span::styled(
                track.title.as_str(),
                if i == 0 { theme.accent() } else { theme.base() },
            ),
        ]));
        if show_artist {
            upcoming.push(Line::styled(
                format!("    {}", track.artist),
                theme.secondary(),
            ));
        }
    }
    frame.render_widget(Paragraph::new(upcoming), list);
    frame.render_widget(Line::styled("b  Open queue", theme.accent()), shortcut);
}

fn lyrics(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let block = theme.panel(" Lyrics ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let mut lines = Vec::new();
    match &app.lyrics {
        None => lines.push(Line::styled("Loading lyrics…", theme.secondary())),
        Some(Err(_)) => lines.push(Line::styled("Lyrics unavailable", theme.secondary())),
        Some(Ok(lyrics)) if !lyrics.lines.is_empty() => {
            let active = lyrics.active_line(app.elapsed_ms);
            let credit_row = !lyrics.credits.is_empty() && inner.height > 2;
            let rows = usize::from(inner.height) - usize::from(credit_row);
            let start = active
                .unwrap_or(0)
                .saturating_sub(rows / 2)
                .min(lyrics.lines.len().saturating_sub(rows));
            for (i, line) in lyrics.lines.iter().enumerate().skip(start).take(rows) {
                lines.push(if Some(i) == active {
                    Line::styled(format!("› {}", line.text), theme.selected())
                } else {
                    Line::styled(format!("  {}", line.text), theme.secondary())
                });
            }
            if credit_row {
                lines.push(Line::styled(&lyrics.credits, theme.secondary()));
            }
        }
        Some(Ok(lyrics)) if !lyrics.plain.is_empty() => {
            frame.render_widget(
                Paragraph::new(format!(
                    "Unsynchronized lyrics\n{}\n{}",
                    lyrics.plain, lyrics.credits
                ))
                .wrap(Wrap { trim: false }),
                inner,
            );
            return;
        }
        Some(Ok(_)) => lines.push(Line::styled("No lyrics available", theme.secondary())),
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

pub(super) fn dock(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let Some(track) = &app.opened else {
        return;
    };
    if area.height == 0 {
        return;
    }
    let details = Line::from(vec![
        Span::styled(format!(" {} ", playback_label(app)), theme.selected()),
        Span::styled(format!("  {} ", track.title), theme.title()),
        Span::styled(format!(" / {}", track.artist), theme.secondary()),
    ]);
    if area.height == 1 {
        frame.render_widget(details, area);
        return;
    }
    let block = theme.panel(" Transport ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [identity, timeline_row] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);
    frame.render_widget(details, identity);
    let [bar, hint] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(if inner.width >= 50 { 20 } else { 0 }),
    ])
    .areas(timeline_row);
    if let PlaybackState::Error(error) = &app.playback {
        frame.render_widget(Line::styled(error, theme.signal()), bar);
    } else {
        timeline(frame, bar, app, track, theme);
    }
    let hint_text = if app.show_help {
        "Close help"
    } else if app.editing {
        "Esc leaves search"
    } else if matches!(app.playback, PlaybackState::Error(_)) {
        "n skips track"
    } else {
        "n next · p player"
    };
    frame.render_widget(
        Line::styled(hint_text, theme.secondary()).right_aligned(),
        hint,
    );
}

fn timeline(frame: &mut Frame, area: Rect, app: &App, track: &Track, theme: &Theme) {
    let times = format!(
        "{} / {}",
        format_time(app.elapsed),
        format_time(track.duration_secs)
    );
    let [rail, label] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(times.len() as u16)])
            .areas(area);
    let width = usize::from(rail.width.saturating_sub(1));
    let played = if track.duration_secs == 0 {
        0
    } else {
        (u128::from(app.elapsed.min(track.duration_secs)) * width as u128
            / u128::from(track.duration_secs)) as usize
    };
    frame.render_widget(
        Line::from(vec![
            Span::styled("━".repeat(played), theme.accent()),
            Span::styled("─".repeat(width - played), theme.border()),
        ]),
        rail,
    );
    frame.render_widget(Line::styled(times, theme.title()), label);
}

fn pause_verb(app: &App) -> &str {
    if app.pause_requested {
        "Resume"
    } else {
        "Pause"
    }
}

fn volume_label(app: &App) -> String {
    if app.volume.is_muted() {
        "muted".into()
    } else {
        format!("{}%", app.volume.percent())
    }
}

pub(super) fn playback_label(app: &App) -> &str {
    match &app.playback {
        PlaybackState::Stopped => "Stopped",
        PlaybackState::Loading => "Loading audio…",
        PlaybackState::Playing if app.buffering => "Buffering…",
        PlaybackState::Playing => "Playing",
        PlaybackState::Paused => "Paused",
        PlaybackState::Finished => "Finished",
        PlaybackState::Error(_) => "Playback unavailable",
    }
}
