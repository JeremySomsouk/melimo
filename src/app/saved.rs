use super::{
    action::Action,
    state::{App, View},
};
use crate::{config::saved_queue, provider::Track};
use std::collections::VecDeque;
use tokio::sync::mpsc;

/// Capture the queue at the keystroke, then leave disk I/O off the terminal thread.
pub(super) fn start(app: &mut App, save: bool, demo: bool, tx: &mpsc::Sender<Action>) -> bool {
    let tx = tx.clone();
    if save {
        if app.queue.is_empty() || app.queue.len() > saved_queue::MAX_TRACKS {
            app.notice = Some(format!(
                "Save between 1 and {} upcoming tracks.",
                saved_queue::MAX_TRACKS
            ));
            return false;
        }
        let queue = app.queue.clone();
        app.notice = Some("Saving queue…".into());
        tokio::task::spawn_blocking(move || {
            let result = saved_queue::save(&queue, demo).map(|()| queue.len());
            let _ = tx.blocking_send(Action::QueueSaved(result));
        });
    } else {
        let generation = app.queue_generation;
        app.notice = Some("Opening saved queue…".into());
        tokio::task::spawn_blocking(move || {
            let _ = tx.blocking_send(Action::QueueLoaded {
                generation,
                result: saved_queue::load(demo),
            });
        });
    }
    true
}

pub(super) fn on_saved(app: &mut App, result: Result<usize, String>) {
    app.notice = Some(match result {
        Ok(1) => "Saved 1 upcoming track.".into(),
        Ok(count) => format!("Saved {count} upcoming tracks."),
        Err(message) => message,
    });
}

pub(super) fn on_loaded(app: &mut App, result: Result<VecDeque<Track>, String>) {
    app.notice = Some(match result {
        Err(message) => message,
        Ok(tracks) => append(app, tracks),
    });
}

fn append(app: &mut App, tracks: VecDeque<Track>) -> String {
    if let Some(item) = tracks
        .iter()
        .find(|track| !app.providers.contains(&track.provider))
    {
        return format!(
            "Enable {} to append this saved queue.",
            item.provider.name()
        );
    }
    if app.queue.len().saturating_add(tracks.len()) > saved_queue::MAX_TRACKS {
        return format!("Appending would exceed {} tracks.", saved_queue::MAX_TRACKS);
    }
    let count = tracks.len();
    app.queue.extend(tracks);
    if app.view == View::Queue && app.selected.is_none() && !app.queue.is_empty() {
        app.selected = Some(0);
    }
    match count {
        1 => "Appended 1 saved track.".into(),
        _ => format!("Appended {count} saved tracks."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ProviderId;

    fn track(id: &str, provider: ProviderId) -> Track {
        Track {
            provider,
            id: id.into(),
            title: id.into(),
            artist: String::new(),
            album: String::new(),
            duration_secs: 90,
        }
    }

    fn loaded(tracks: VecDeque<Track>) -> Action {
        Action::QueueLoaded {
            generation: 0,
            result: Ok(tracks),
        }
    }

    #[test]
    fn a_late_restore_cannot_refill_a_cleared_queue() {
        let mut app = App::default();
        app.providers.push(ProviderId::Deezer);
        let restore = loaded([track("1", ProviderId::Deezer)].into());
        app.update(Action::StopPlayback);
        app.update(restore);
        assert!(app.queue.is_empty());
        assert_eq!(
            app.notice.as_deref(),
            Some("Playback stopped. Queue cleared.")
        );
    }

    #[test]
    fn append_keeps_playback_order_selection_and_duplicates() {
        let mut app = App::default();
        let first = track("11", ProviderId::Deezer);
        let second = track("22", ProviderId::Deezer);
        app.providers.push(ProviderId::Deezer);
        app.view = View::Queue;
        app.opened = Some(first.clone());
        app.playback_id = 7;
        app.update(loaded([first.clone(), second.clone()].into()));
        assert_eq!(app.selected, Some(0));
        app.selected = Some(1);
        app.update(loaded([second.clone()].into()));
        assert_eq!(app.selected, Some(1));
        assert_eq!(
            app.queue,
            VecDeque::from([first.clone(), second.clone(), second])
        );
        assert_eq!(app.opened, Some(first));
        assert_eq!(app.playback_id, 7);
    }

    #[test]
    fn incompatible_or_oversized_restore_leaves_queue_intact() {
        let mut app = App::default();
        app.providers.push(ProviderId::Invidious);
        let item = track("abcdefghijk", ProviderId::Invidious);
        app.queue.push_back(item.clone());
        app.update(loaded(
            [item.clone(), track("22", ProviderId::Deezer)].into(),
        ));
        assert_eq!(app.queue.len(), 1);
        assert!(app.notice.as_deref().unwrap().contains("Deezer"));
        app.update(loaded(vec![item.clone(); saved_queue::MAX_TRACKS].into()));
        assert_eq!(app.queue.front(), Some(&item));
        assert_eq!(app.queue.len(), 1);
        assert!(app.notice.as_deref().unwrap().contains("1000"));
    }
}
