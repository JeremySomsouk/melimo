//! One explicitly saved snapshot of upcoming tracks, independent of playback and login.
use super::store;
use crate::provider::{ProviderId, Track};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    fs,
    io::{ErrorKind, Read, Write},
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

pub(crate) const MAX_TRACKS: usize = 1_000;
const MAX_BYTES: usize = 2 * 1024 * 1024;
const VERSION: u8 = 1;
const INVALID: &str = "The saved queue contains invalid track data.";
const TOO_LARGE: &str = "Saved queues support up to 1,000 tracks and 2 MiB of metadata.";

#[derive(Serialize, Deserialize)]
struct Snapshot<T> {
    version: u8,
    tracks: Vec<T>,
}

// Keep the on-disk contract explicit. New runtime fields must not implicitly enter the file.
#[derive(Serialize, Deserialize)]
#[serde(remote = "Track", deny_unknown_fields)]
struct StoredTrack {
    #[serde(with = "StoredProvider")]
    provider: ProviderId,
    id: String,
    title: String,
    artist: String,
    album: String,
    duration_secs: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "ProviderId", rename_all = "lowercase")]
enum StoredProvider {
    Deezer,
    Invidious,
    Mock,
}

#[derive(Serialize)]
#[serde(transparent)]
struct TrackRef<'a>(#[serde(with = "StoredTrack")] &'a Track);

#[derive(Deserialize)]
#[serde(transparent)]
struct OwnedTrack(#[serde(with = "StoredTrack")] Track);

fn slot(demo: bool) -> Result<PathBuf, String> {
    if store::disabled() {
        return Err("Queue storage is disabled for this session.".into());
    }
    let name = if demo {
        "queue-demo.json"
    } else {
        "queue.json"
    };
    store::session_path()
        .map(|path| path.with_file_name(name))
        .ok_or_else(|| "Cannot locate the saved queue directory.".into())
}

pub fn save(queue: &VecDeque<Track>, demo: bool) -> Result<(), String> {
    save_at(&slot(demo)?, queue)
}

pub fn load(demo: bool) -> Result<VecDeque<Track>, String> {
    load_at(&slot(demo)?)
}

fn valid_track(track: &Track) -> bool {
    if track.id.len() > 32 {
        return false;
    }
    let valid_id = match track.provider {
        ProviderId::Deezer => {
            !track.id.is_empty()
                && track.id.bytes().all(|b| b.is_ascii_digit())
                && track.id.parse::<u64>().is_ok()
        }
        ProviderId::Invidious => {
            track.id.len() == 11
                && track
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        }
        ProviderId::Mock => track.id.strip_prefix("mock:").is_some_and(|n| {
            !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && n.parse::<usize>().is_ok()
        }),
    };
    valid_id
        && [&track.title, &track.artist, &track.album]
            .iter()
            .all(|s| s.len() <= 2048 && !s.chars().any(char::is_control))
}

fn save_at(path: &Path, queue: &VecDeque<Track>) -> Result<(), String> {
    if queue.is_empty() {
        return Err("Add a track before saving the queue.".into());
    }
    if queue.len() > MAX_TRACKS {
        return Err(TOO_LARGE.into());
    }
    if !queue.iter().all(valid_track) {
        return Err(INVALID.into());
    }
    let snapshot = Snapshot {
        version: VERSION,
        tracks: queue.iter().map(TrackRef).collect(),
    };
    let bytes = serde_json::to_vec(&snapshot).map_err(|_| INVALID)?;
    if bytes.len() > MAX_BYTES {
        return Err(TOO_LARGE.into());
    }
    let dir = path
        .parent()
        .ok_or("Cannot locate the saved queue directory.")?;
    store::ensure_private_dir(dir).map_err(|_| "Cannot use the private queue directory.")?;
    if fs::symlink_metadata(path).is_ok_and(|meta| !meta.is_file()) {
        return Err("The saved queue must be a regular file.".into());
    }
    let temporary = dir.join(format!(".queue-{:016x}.tmp", rand::random::<u64>()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&temporary)
        .map_err(|_| "Cannot create the saved queue.")?;
    let written = file.write_all(&bytes).and_then(|()| file.sync_all());
    drop(file);
    written
        .and_then(|()| fs::rename(&temporary, path))
        .map_err(|_| {
            let _ = fs::remove_file(&temporary);
            "Cannot write the saved queue.".into()
        })
}

fn load_at(path: &Path) -> Result<VecDeque<Track>, String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if !meta.is_file() => return Err("The saved queue must be a regular file.".into()),
        Ok(_) => {}
        Err(err) if err.kind() == ErrorKind::NotFound => return Err("No saved queue yet.".into()),
        Err(_) => return Err("Cannot inspect the saved queue.".into()),
    }
    let dir = path
        .parent()
        .ok_or("Cannot locate the saved queue directory.")?;
    store::validate_private_dir(dir).map_err(|_| "Cannot use the private queue directory.")?;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    let file = options
        .open(path)
        .map_err(|_| "Cannot open the saved queue.")?;
    let meta = file
        .metadata()
        .map_err(|_| "Cannot inspect the saved queue.")?;
    if !meta.is_file() {
        return Err("The saved queue must be a regular file.".into());
    }
    #[cfg(unix)]
    if meta.permissions().mode() & 0o077 != 0
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.nlink() != 1
    {
        return Err("The saved queue must be private and owned by you.".into());
    }
    if meta.len() > MAX_BYTES as u64 {
        return Err(TOO_LARGE.into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read the saved queue.")?;
    if bytes.len() > MAX_BYTES {
        return Err(TOO_LARGE.into());
    }
    let snapshot: Snapshot<OwnedTrack> = serde_json::from_slice(&bytes).map_err(|_| INVALID)?;
    if snapshot.version != VERSION {
        return Err("This saved queue uses an unsupported format version.".into());
    }
    if snapshot.tracks.len() > MAX_TRACKS {
        return Err(TOO_LARGE.into());
    }
    if snapshot.tracks.is_empty() || !snapshot.tracks.iter().all(|entry| valid_track(&entry.0)) {
        return Err(INVALID.into());
    }
    Ok(snapshot.tracks.into_iter().map(|entry| entry.0).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempSlot(PathBuf);

    impl TempSlot {
        fn new() -> Self {
            Self(
                std::env::temp_dir()
                    .join(format!("melimo-saved-queue-{:016x}", rand::random::<u64>()))
                    .join("queue.json"),
            )
        }
    }

    impl Drop for TempSlot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(self.0.parent().unwrap());
        }
    }

    fn track(provider: ProviderId, id: &str) -> Track {
        Track {
            provider,
            id: id.into(),
            title: "Lumière".into(),
            artist: "Écho".into(),
            album: String::new(),
            duration_secs: 195,
        }
    }

    #[test]
    fn saves_order_duplicates_and_provider_identity() {
        let path = TempSlot::new();
        let one = track(ProviderId::Deezer, "12345678901");
        let two = track(ProviderId::Invidious, "12345678901");
        let queue = VecDeque::from([one.clone(), two, one]);
        save_at(&path.0, &queue).unwrap();
        assert_eq!(load_at(&path.0).unwrap(), queue);

        let replacement = VecDeque::from([track(ProviderId::Mock, "mock:1")]);
        save_at(&path.0, &replacement).unwrap();
        assert_eq!(load_at(&path.0).unwrap(), replacement);
        #[cfg(unix)]
        assert_eq!(fs::metadata(&path.0).unwrap().mode() & 0o777, 0o600);
    }

    #[test]
    fn rejected_saves_preserve_the_previous_queue() {
        let path = TempSlot::new();
        let item = track(ProviderId::Deezer, "42");
        let queue = VecDeque::from([item.clone()]);
        save_at(&path.0, &queue).unwrap();
        assert!(save_at(&path.0, &VecDeque::new()).is_err());
        assert!(save_at(&path.0, &vec![item.clone(); MAX_TRACKS + 1].into()).is_err());
        let mut oversized = item;
        oversized.title = "x".repeat(2048);
        assert!(save_at(&path.0, &vec![oversized; MAX_TRACKS].into()).is_err());
        assert_eq!(load_at(&path.0).unwrap(), queue);
    }

    #[cfg(unix)]
    #[test]
    fn refuses_an_insecure_file_or_redirected_slot() {
        let path = TempSlot::new();
        let queue = VecDeque::from([track(ProviderId::Mock, "mock:2")]);
        save_at(&path.0, &queue).unwrap();
        fs::set_permissions(&path.0, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(load_at(&path.0).is_err());
        let target = path.0.with_file_name("other");
        fs::rename(&path.0, &target).unwrap();
        std::os::unix::fs::symlink(&target, &path.0).unwrap();
        assert!(load_at(&path.0).is_err());
        assert!(save_at(&path.0, &queue).is_err());
        assert!(fs::symlink_metadata(&path.0).unwrap().is_symlink());
    }

    #[test]
    fn rejects_missing_damaged_unsupported_and_unsafe_snapshots() {
        let path = TempSlot::new();
        assert_eq!(load_at(&path.0).unwrap_err(), "No saved queue yet.");
        let queue = VecDeque::from([track(ProviderId::Mock, "mock:2")]);
        save_at(&path.0, &queue).unwrap();
        let original = fs::read_to_string(&path.0).unwrap();
        for value in [
            "broken".into(),
            original.replace("\"version\":1", "\"version\":2"),
            original.replace("mock:2", "https://invalid/"),
            original.replace("Lumière", "unsafe\\u001b"),
            "x".repeat(MAX_BYTES + 1),
        ] {
            fs::write(&path.0, value).unwrap();
            assert!(load_at(&path.0).is_err());
        }
    }
}
