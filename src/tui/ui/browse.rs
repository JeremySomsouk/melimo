use super::{format_time, state};
use crate::{
    app::state::{App, DISCOVER, DISCOVER_GROUPS},
    provider::{ProviderId, Track},
    tui::theme::Theme,
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::{Line, Span, Text},
    widgets::{Block, Cell, Paragraph, Row, Table, TableState},
};
use unicode_width::UnicodeWidthStr;

pub(super) fn discover(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    theme: &Theme,
    table: &mut TableState,
) {
    if area.width < 90 {
        table.select(app.selected);
        let selected_group = DISCOVER_GROUPS
            .iter()
            .find(|(_, items)| app.selected.is_some_and(|i| items.contains(&i)));
        let title = selected_group
            .map(|(group, _)| format!("Discover  /  {group}"))
            .unwrap_or_else(|| "Discover".into());
        frame.render_stateful_widget(
            Table::new(
                DISCOVER.iter().map(|(label, _)| Row::new([*label])),
                [Constraint::Fill(1)],
            )
            .block(theme.panel(title))
            .row_highlight_style(theme.selected())
            .highlight_symbol("› "),
            area,
            table,
        );
        return;
    }
    let columns = Layout::horizontal([
        Constraint::Percentage(43),
        Constraint::Percentage(24),
        Constraint::Fill(1),
    ])
    .spacing(1)
    .split(area);
    for ((title, indices), &column) in DISCOVER_GROUPS.iter().zip(columns.iter()) {
        let mut selection = TableState::default().with_selected(
            app.selected
                .and_then(|selected| indices.iter().position(|&i| i == selected)),
        );
        let block =
            theme
                .panel(format!(" {title} "))
                .title_style(if selection.selected().is_some() {
                    theme.signal()
                } else {
                    theme.secondary()
                });
        let inner = block.inner(column);
        let spacious = inner.height >= 2 * indices.len() as u16;
        let rows = indices.iter().map(|&index| {
            let label = DISCOVER[index]
                .0
                .strip_prefix("Genre · ")
                .unwrap_or(DISCOVER[index].0);
            Row::new([label]).bottom_margin(u16::from(spacious))
        });
        frame.render_stateful_widget(
            Table::new(rows, [Constraint::Fill(1)])
                .block(block)
                .row_highlight_style(theme.selected())
                .highlight_symbol("› "),
            column,
            &mut selection,
        );
    }
}

pub(super) fn search(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    theme: &Theme,
    table: &mut TableState,
) {
    let [input, results] =
        Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).areas(area);
    let kind = if app.playlist_search {
        "playlists"
    } else {
        "tracks"
    };
    let label = if app.editing {
        format!(" Search {kind} ")
    } else {
        format!(" /  Search {kind} ")
    };
    let block = theme.panel(label).border_style(if app.editing {
        theme.accent()
    } else {
        theme.border()
    });
    let inner = block.inner(input);
    frame.render_widget(block, input);
    let prompt = if app.editing {
        format!("{}▏", app.query)
    } else if app.query.is_empty() {
        (if app.playlist_search {
            "Playlist name or keyword"
        } else {
            "Track, artist or album"
        })
        .to_string()
    } else {
        app.query.clone()
    };
    let scroll = if app.editing {
        prompt
            .width()
            .saturating_sub(usize::from(inner.width))
            .min(u16::MAX as usize) as u16
    } else {
        0
    };
    frame.render_widget(
        Paragraph::new(prompt)
            .style(if app.editing {
                theme.title()
            } else {
                theme.secondary()
            })
            .scroll((0, scroll)),
        inner,
    );
    let title = app
        .collection
        .as_deref()
        .unwrap_or(if app.showing_playlists {
            "Playlists"
        } else {
            "Tracks"
        });
    let message = if app.loading {
        Some(("Finding your music…", "Press d to return to browsing.", ""))
    } else if let Some(error) = &app.error {
        Some((
            "Could not load music",
            "Press / to try a new search.",
            error.as_str(),
        ))
    } else if !app.searched {
        Some((
            "Find your next song",
            "Press / to enter a search.",
            if app.search_provider == ProviderId::Invidious {
                ""
            } else {
                "Tab switches between tracks and playlists."
            },
        ))
    } else if (app.showing_playlists && app.playlists.is_empty())
        || (!app.showing_playlists && app.tracks.is_empty())
    {
        Some(("No music found", "Try other keywords with /.", ""))
    } else {
        None
    };
    if let Some((heading, hint, detail)) = message {
        state(
            frame,
            results,
            theme,
            title,
            heading,
            if app.editing {
                "Enter submits your search."
            } else {
                hint
            },
            detail,
        );
        return;
    }
    table.select(app.selected);
    if app.showing_playlists {
        let rows = app.playlists.iter().map(|p| {
            Row::new([
                Cell::from(p.title.as_str()),
                Cell::from(if p.tracks > 0 {
                    p.tracks.to_string()
                } else {
                    "—".into()
                }),
            ])
        });
        frame.render_stateful_widget(
            Table::new(rows, [Constraint::Fill(1), Constraint::Length(8)])
                .header(Row::new(["Playlist", "Tracks"]).style(theme.secondary()))
                .block(theme.panel(format!(" {title} · {} ", app.playlists.len())))
                .row_highlight_style(theme.selected())
                .highlight_symbol("› "),
            results,
            table,
        );
    } else {
        tracks(
            frame,
            results,
            theme,
            table,
            theme.panel(format!(" {title} · {} ", app.tracks.len())),
            app.tracks.iter(),
            false,
        );
    }
}

pub(super) fn queue(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    theme: &Theme,
    table: &mut TableState,
) {
    if app.queue.is_empty() {
        state(
            frame,
            area,
            theme,
            " Up next ",
            "Your queue is empty",
            "Browse with d or search with /.",
            "Press e on a track to add it here.",
        );
        return;
    }
    let selection = app.selected.unwrap_or(0);
    let duration = |count| {
        let mut known = 0u64;
        let mut incomplete = false;
        for track in app.queue.iter().take(count) {
            known = known.saturating_add(track.duration_secs);
            incomplete |= track.duration_secs == 0;
        }
        let mut label = if known >= 3600 {
            format!("{}h {:02}m", known / 3600, known / 60 % 60)
        } else {
            format_time(known)
        };
        if incomplete {
            label.push('+');
        }
        label
    };
    table.select(app.selected);
    let title = format!(
        " Up next · {} · {} total ",
        app.queue.len(),
        duration(app.queue.len())
    );
    let mut panel = theme.panel(title);
    if selection > 0 {
        panel = panel.title_bottom(Line::styled(
            format!(" {} queued before selection ", duration(selection)),
            theme.secondary(),
        ));
    }
    tracks(frame, area, theme, table, panel, app.queue.iter(), true);
}

fn tracks<'a>(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    table: &mut TableState,
    panel: Block,
    tracks: impl ExactSizeIterator<Item = &'a Track>,
    ordered: bool,
) {
    let wide = area.width >= 88;
    let stacked = area.width < 56 && area.height >= 6;
    let mut constraints = Vec::new();
    let mut headers = Vec::new();
    if ordered {
        constraints.push(Constraint::Length(4));
        headers.push("#");
    }
    constraints.push(Constraint::Fill(3));
    if stacked {
        headers.push("Track / artist");
    } else {
        headers.push("Title");
        constraints.push(Constraint::Fill(2));
        headers.push("Artist");
    }
    if wide {
        constraints.push(Constraint::Fill(2));
        headers.push("Album");
    }
    constraints.push(Constraint::Length(6));
    headers.push("Time");
    // Table eagerly collects its rows. Only format the visible tracks, even for a long queue.
    let row_height = if stacked { 2 } else { 1 };
    let capacity = usize::from(panel.inner(area).height.saturating_sub(1) / row_height).max(1);
    let max_start = tracks.len().saturating_sub(capacity);
    let mut start = table.offset().min(max_start);
    if let Some(selected) = table.selected() {
        start = start
            .clamp(selected.saturating_sub(capacity - 1), selected)
            .min(max_start);
    }
    *table.offset_mut() = start;
    let mut window =
        TableState::default().with_selected(table.selected().and_then(|i| i.checked_sub(start)));
    let rows = tracks
        .enumerate()
        .skip(start)
        .take(capacity)
        .map(|(index, t)| {
            let mut cells = Vec::new();
            if ordered {
                cells.push(Cell::from((index + 1).to_string()));
            }
            if stacked {
                cells.push(Cell::from(Text::from(vec![
                    Line::raw(t.title.as_str()),
                    Line::from(vec![
                        Span::raw(format!("[{}] ", t.provider.label())),
                        Span::raw(t.artist.as_str()),
                    ]),
                ])));
            } else {
                cells.push(Cell::from(Line::from(vec![
                    Span::raw(format!("[{}] ", t.provider.label())),
                    Span::raw(t.title.as_str()),
                ])));
                cells.push(Cell::from(t.artist.as_str()));
            }
            if wide {
                cells.push(Cell::from(t.album.as_str()));
            }
            cells.push(Cell::from(format_time(t.duration_secs)));
            Row::new(cells).height(row_height)
        });
    frame.render_stateful_widget(
        Table::new(rows, constraints)
            .header(Row::new(headers).style(theme.secondary()))
            .block(panel)
            .row_highlight_style(theme.selected())
            .highlight_symbol("› "),
        area,
        &mut window,
    );
}
