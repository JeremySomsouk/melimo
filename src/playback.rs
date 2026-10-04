//! Bounded in-memory pipeline: HTTP -> audio decoder -> PCM -> audio device.
//! The device callback never waits on network I/O.
use crate::{
    app::{action::Action, state::PlaybackState},
    provider::{Track, router::Providers},
};
use rodio::{Decoder, OutputStream, OutputStreamBuilder, Sink, Source};
use std::{
    io::{self, Read, Seek, SeekFrom},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc as sync,
    },
    thread,
    time::Duration,
};
use tokio::{sync::mpsc, task::JoinHandle};

/// One lazy output device for the entire TUI session. Each track owns its sink.
#[derive(Default, Clone)]
pub struct AudioOutput(Arc<Mutex<Option<OutputDevice>>>);
struct OutputDevice {
    stream: OutputStream,
    failed: Arc<AtomicBool>,
    active: Weak<Sink>,
}
impl AudioOutput {
    fn stop(&self) {
        if let Ok(shared) = self.0.try_lock()
            && let Some(sink) = shared.as_ref().and_then(|d| d.active.upgrade())
        {
            sink.stop();
        }
    }
    fn sink(&self, cancel: &AtomicBool) -> Result<(Arc<Sink>, Arc<AtomicBool>), &'static str> {
        let mut shared = self.0.lock().map_err(|_| "Audio output lock failed.")?;
        if cancel.load(Ordering::Relaxed) {
            return Err("Playback cancelled.");
        }
        if shared
            .as_ref()
            .is_some_and(|device| device.failed.load(Ordering::Relaxed))
        {
            *shared = None;
        }
        if shared.is_none() {
            let failed = Arc::new(AtomicBool::new(false));
            let callback = failed.clone();
            let mut stream = OutputStreamBuilder::from_default_device()
                .map_err(|_| "No audio output device available.")?
                .with_error_callback(move |_| {
                    callback.store(true, Ordering::Relaxed);
                })
                .open_stream()
                .map_err(|_| "Cannot open the audio output device.")?;
            stream.log_on_drop(false);
            *shared = Some(OutputDevice {
                stream,
                failed,
                active: Weak::new(),
            });
        }
        if cancel.load(Ordering::Relaxed) {
            return Err("Playback cancelled.");
        }
        let device = shared.as_mut().unwrap();
        if let Some(previous) = device.active.upgrade() {
            previous.stop();
        }
        let sink = Arc::new(Sink::connect_new(device.stream.mixer()));
        sink.pause();
        device.active = Arc::downgrade(&sink);
        Ok((sink, device.failed.clone()))
    }
}

/// Bounded encoded prefix. Backpressure keeps the same HTTP response open;
/// promotion consumes the prefix and continues without a second request.
pub struct Prepared {
    track: Track,
    rx: Option<mpsc::Receiver<Vec<u8>>>,
    cancelled: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    network: JoinHandle<()>,
}
impl Prepared {
    fn new(provider: Arc<Providers>, track: Track) -> Self {
        let (tx, rx) = mpsc::channel(32); // Providers send at most 2048 bytes per chunk.
        let cancelled = Arc::new(AtomicBool::new(false));
        let error = Arc::new(Mutex::new(None));
        let failure = error.clone();
        let cancel = cancelled.clone();
        let item = track.clone();
        let network = tokio::spawn(async move {
            if let Err(message) = provider.stream(item, tx.clone()).await {
                *failure.lock().unwrap() = Some(message);
                cancel.store(true, Ordering::Relaxed);
            }
        });
        Self {
            track,
            rx: Some(rx),
            cancelled,
            error,
            network,
        }
    }
}
impl Drop for Prepared {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
        self.network.abort();
    }
}

#[derive(Default)]
pub struct Preparation {
    next: Option<Prepared>,
}
impl Preparation {
    pub fn update(&mut self, provider: &Arc<Providers>, track: Option<&Track>) {
        let track = track.filter(|t| t.provider == crate::provider::ProviderId::Deezer);
        if self.next.as_ref().map(|p| &p.track) != track {
            self.next = track.map(|t| Prepared::new(provider.clone(), t.clone()));
        }
    }
    fn take(&mut self, track: &Track) -> Option<Prepared> {
        let prepared = self.next.take()?;
        (prepared.track == *track && !prepared.cancelled.load(Ordering::Relaxed))
            .then_some(prepared)
    }
}

pub struct Playback {
    prepared: Prepared,
    output: AudioOutput,
    paused: Arc<AtomicBool>,
    volume: Arc<AtomicU32>,
}
impl Playback {
    pub fn toggle_pause(&self) {
        self.paused.fetch_xor(true, Ordering::Relaxed);
    }
    pub fn set_volume(&self, level: f32) {
        self.volume
            .store(level.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        provider: Arc<Providers>,
        track: Track,
        id: u64,
        tx: mpsc::Sender<Action>,
        offset: u64,
        initially_paused: bool,
        volume: f32,
        output: AudioOutput,
        preparation: &mut Preparation,
    ) -> Self {
        let mut prepared = preparation
            .take(&track)
            .unwrap_or_else(|| Prepared::new(provider, track));
        let audio_rx = prepared.rx.take().unwrap();
        let paused = Arc::new(AtomicBool::new(initially_paused));
        let volume = Arc::new(AtomicU32::new(volume.clamp(0.0, 1.0).to_bits()));
        let cancel = prepared.cancelled.clone();
        let error = prepared.error.clone();
        let pause = paused.clone();
        let gain = volume.clone();
        let spawn_tx = tx.clone();
        let session_output = output.clone();
        if thread::Builder::new()
            .name("melimo-audio".into())
            .spawn(move || {
                let result = play(
                    audio_rx,
                    cancel.clone(),
                    pause,
                    id,
                    &tx,
                    offset,
                    gain,
                    output,
                );
                let failure = error.lock().unwrap().take();
                if let Some(message) = failure.or_else(|| {
                    (!cancel.load(Ordering::Relaxed))
                        .then(|| result.err())
                        .flatten()
                        .map(str::to_owned)
                }) {
                    let _ = tx.blocking_send(Action::PlaybackUpdate {
                        id,
                        state: PlaybackState::Error(message),
                        elapsed_ms: 0,
                        buffering: false,
                    });
                }
                cancel.store(true, Ordering::Relaxed);
            })
            .is_err()
        {
            prepared.cancelled.store(true, Ordering::Relaxed);
            prepared.network.abort();
            let _ = spawn_tx.try_send(Action::PlaybackUpdate {
                id,
                state: PlaybackState::Error("Cannot start audio worker.".into()),
                elapsed_ms: 0,
                buffering: false,
            });
        }
        Self {
            output: session_output,
            prepared,
            paused,
            volume,
        }
    }
}
impl Drop for Playback {
    fn drop(&mut self) {
        self.prepared.cancelled.store(true, Ordering::Relaxed);
        self.output.stop();
    }
}

struct AudioReader {
    rx: mpsc::Receiver<Vec<u8>>,
    current: io::Cursor<Vec<u8>>,
    cancelled: Arc<AtomicBool>,
}
impl Read for AudioReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        loop {
            if self.cancelled.load(Ordering::Relaxed) {
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            let n = self.current.read(out)?;
            if n > 0 {
                return Ok(n);
            }
            match self.rx.try_recv() {
                Ok(bytes) => self.current = io::Cursor::new(bytes),
                Err(mpsc::error::TryRecvError::Disconnected) => return Ok(0),
                Err(mpsc::error::TryRecvError::Empty) => thread::sleep(Duration::from_millis(10)),
            }
        }
    }
}
impl Seek for AudioReader {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

struct Pcm {
    rx: sync::Receiver<Vec<f32>>,
    current: Vec<f32>,
    position: usize,
    recycle: sync::SyncSender<Vec<f32>>,
    channels: u16,
    rate: u32,
    samples: Arc<AtomicU64>,
    buffering: Arc<AtomicBool>,
    silence_remaining: u16,
}
impl Iterator for Pcm {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.silence_remaining > 0 {
            self.silence_remaining -= 1;
            return Some(0.0);
        }
        if let Some(&sample) = self.current.get(self.position) {
            self.position += 1;
            self.samples.fetch_add(1, Ordering::Relaxed);
            return Some(sample);
        }
        match self.rx.try_recv() {
            Ok(chunk) => {
                let old = std::mem::replace(&mut self.current, chunk);
                let _ = self.recycle.try_send(old);
                self.position = 0;
                self.buffering.store(false, Ordering::Relaxed);
                self.next()
            }
            Err(sync::TryRecvError::Disconnected) => None,
            Err(sync::TryRecvError::Empty) => {
                self.buffering.store(true, Ordering::Relaxed);
                self.silence_remaining = self.channels.saturating_sub(1);
                Some(0.0)
            }
        }
    }
}
impl Source for Pcm {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        self.channels
    }
    fn sample_rate(&self) -> u32 {
        self.rate
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

fn discard_samples(
    decoder: &mut impl Iterator<Item = f32>,
    count: u64,
    cancel: &AtomicBool,
) -> bool {
    for _ in 0..count {
        if cancel.load(Ordering::Relaxed) || decoder.next().is_none() {
            return false;
        }
    }
    true
}

#[allow(clippy::too_many_arguments)]
fn play(
    rx: mpsc::Receiver<Vec<u8>>,
    cancel: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    id: u64,
    tx: &mpsc::Sender<Action>,
    offset: u64,
    volume: Arc<AtomicU32>,
    output: AudioOutput,
) -> Result<(), &'static str> {
    let reader = AudioReader {
        rx,
        current: io::Cursor::new(Vec::new()),
        cancelled: cancel.clone(),
    };
    let mut decoder = Decoder::builder()
        .with_data(reader)
        .with_seekable(false)
        .build()
        .map_err(|_| "Cannot decode this audio stream. Try another track.")?;
    if cancel.load(Ordering::Relaxed) {
        return Ok(());
    }
    let channels = decoder.channels();
    let rate = decoder.sample_rate();
    let (sink, device_failed) = output.sink(&cancel)?;
    let (pcm_tx, pcm_rx) = sync::sync_channel(8); // At most 128 KiB of queued PCM; recycled buffers are separately bounded.
    let (recycle, recycled) = sync::sync_channel::<Vec<f32>>(9);
    let decode_cancel = cancel.clone();
    thread::Builder::new()
        .name("melimo-decode".into())
        .spawn(move || {
            // Reopen and decode forward for seeks: bounded memory, no disk cache.
            let skip = offset
                .saturating_mul(u64::from(rate))
                .saturating_mul(u64::from(channels));
            if !discard_samples(&mut decoder, skip, &decode_cancel) {
                return;
            }
            while !decode_cancel.load(Ordering::Relaxed) {
                let mut chunk = recycled
                    .try_recv()
                    .unwrap_or_else(|_| Vec::with_capacity(4096));
                chunk.clear();
                chunk.extend(decoder.by_ref().take(4096));
                if chunk.is_empty() {
                    break;
                }
                loop {
                    if decode_cancel.load(Ordering::Relaxed) {
                        return;
                    }
                    match pcm_tx.try_send(chunk) {
                        Ok(()) => break,
                        Err(sync::TrySendError::Disconnected(_)) => return,
                        Err(sync::TrySendError::Full(value)) => {
                            chunk = value;
                            thread::sleep(Duration::from_millis(10));
                        }
                    }
                }
            }
        })
        .map_err(|_| "Cannot start audio decoder.")?;
    let samples = Arc::new(AtomicU64::new(0));
    let buffering = Arc::new(AtomicBool::new(true));
    // Install the source paused and with its initial gain already applied.
    // A cancelled worker must never briefly play at full volume.
    let initial_volume = f32::from_bits(volume.load(Ordering::Relaxed));
    sink.set_volume(initial_volume);
    sink.append(Pcm {
        rx: pcm_rx,
        current: Vec::new(),
        position: 0,
        recycle,
        channels,
        rate,
        samples: samples.clone(),
        buffering: buffering.clone(),
        silence_remaining: 0,
    });
    let mut applied_volume = initial_volume;
    loop {
        if cancel.load(Ordering::Relaxed) {
            sink.stop();
            return Ok(());
        }
        if device_failed.load(Ordering::Relaxed) {
            return Err("Audio output device disconnected or failed.");
        }
        // Apply gain changes from the UI thread without touching the sink from afar.
        let level = f32::from_bits(volume.load(Ordering::Relaxed));
        if level != applied_volume {
            sink.set_volume(level);
            applied_volume = level;
        }
        let elapsed_ms = offset.saturating_mul(1000)
            + samples.load(Ordering::Relaxed).saturating_mul(1000)
                / u64::from(channels)
                / u64::from(rate);
        let state = if sink.empty() {
            PlaybackState::Finished
        } else if pause.load(Ordering::Relaxed) {
            sink.pause();
            PlaybackState::Paused
        } else {
            sink.play();
            PlaybackState::Playing
        };
        let finished = state == PlaybackState::Finished;
        let action = Action::PlaybackUpdate {
            id,
            state,
            elapsed_ms,
            buffering: buffering.load(Ordering::Relaxed),
        };
        if finished {
            let _ = tx.blocking_send(action);
            break;
        }
        let _ = tx.try_send(action);
        thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn synthetic_prepared(track: Track, error: Option<String>) -> Prepared {
        let (tx, rx) = mpsc::channel(32);
        let cancelled = Arc::new(AtomicBool::new(error.is_some()));
        let network = tokio::spawn(async move {
            for i in 0..100u8 {
                if tx.send(vec![i; 2048]).await.is_err() {
                    break;
                }
            }
        });
        Prepared {
            track,
            rx: Some(rx),
            cancelled,
            error: Arc::new(Mutex::new(error)),
            network,
        }
    }
    fn synthetic_track(id: &str) -> Track {
        Track {
            provider: crate::provider::ProviderId::Deezer,
            id: id.into(),
            title: "Synthetic".into(),
            artist: "Test".into(),
            album: String::new(),
            duration_secs: 120,
        }
    }
    #[tokio::test]
    async fn prepared_prefix_is_bounded_and_promoted_without_losing_bytes() {
        let track = synthetic_track("1");
        let mut preparation = Preparation {
            next: Some(synthetic_prepared(track.clone(), None)),
        };
        tokio::time::timeout(Duration::from_secs(2), async {
            while preparation
                .next
                .as_ref()
                .unwrap()
                .rx
                .as_ref()
                .unwrap()
                .len()
                < 32
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!preparation.next.as_ref().unwrap().network.is_finished());
        let mut promoted = preparation.take(&track).unwrap();
        let rx = promoted.rx.as_mut().unwrap();
        for i in 0..100u8 {
            let bytes = tokio::time::timeout(Duration::from_secs(2), rx.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(bytes, vec![i; 2048]);
        }
        assert!(rx.recv().await.is_none());
        assert!(preparation.next.is_none());
    }
    #[tokio::test]
    async fn preparation_cancels_replacements_and_rejects_failed_or_wrong_tracks() {
        let track = synthetic_track("1");
        let providers = Arc::new(Providers::default());
        let mut preparation = Preparation {
            next: Some(synthetic_prepared(track.clone(), None)),
        };
        let cancel = preparation.next.as_ref().unwrap().cancelled.clone();
        preparation.update(&providers, Some(&track));
        assert!(!cancel.load(Ordering::Relaxed));
        preparation.update(&providers, None);
        assert!(cancel.load(Ordering::Relaxed));
        preparation.next = Some(synthetic_prepared(track.clone(), None));
        let cancel = preparation.next.as_ref().unwrap().cancelled.clone();
        assert!(preparation.take(&synthetic_track("2")).is_none());
        assert!(cancel.load(Ordering::Relaxed));
        preparation.next = Some(synthetic_prepared(
            track.clone(),
            Some("unavailable".into()),
        ));
        assert!(preparation.take(&track).is_none());
        let mut invidious = track;
        invidious.provider = crate::provider::ProviderId::Invidious;
        preparation.update(&providers, Some(&invidious));
        assert!(preparation.next.is_none());
    }
    #[test]
    fn pcm_recycles_buffers_without_blocking_or_changing_samples() {
        let (tx, rx) = sync::sync_channel(2);
        let (recycle, recycled) = sync::sync_channel(1);
        tx.send(vec![0.1, 0.2]).unwrap();
        tx.send(vec![0.3, 0.4]).unwrap();
        drop(tx);
        let mut pcm = Pcm {
            rx,
            current: Vec::with_capacity(4096),
            position: 0,
            recycle,
            channels: 2,
            rate: 44100,
            samples: Arc::new(AtomicU64::new(0)),
            buffering: Arc::new(AtomicBool::new(false)),
            silence_remaining: 0,
        };
        assert_eq!(pcm.by_ref().collect::<Vec<_>>(), vec![0.1, 0.2, 0.3, 0.4]);
        assert_eq!(pcm.samples.load(Ordering::Relaxed), 4);
        assert_eq!(recycled.try_recv().unwrap().capacity(), 4096);
    }

    #[test]
    fn decodes_and_seeks_aac_m4a_from_nonseekable_chunks() {
        let bytes = include_bytes!("../tests/fixtures/tone.m4a");
        let (tx, rx) = mpsc::channel(2);
        let feed = thread::spawn(move || {
            for chunk in bytes.chunks(127) {
                if tx.blocking_send(chunk.to_vec()).is_err() {
                    break;
                }
            }
        });
        let cancel = Arc::new(AtomicBool::new(false));
        let mut decoder = Decoder::builder()
            .with_data(AudioReader {
                rx,
                current: io::Cursor::new(Vec::new()),
                cancelled: cancel.clone(),
            })
            .with_seekable(false)
            .build()
            .unwrap();
        assert_eq!(decoder.channels(), 2);
        assert_eq!(decoder.sample_rate(), 44100);
        assert!(discard_samples(&mut decoder, 8820, &cancel));
        let remainder: Vec<_> = decoder.collect();
        assert!(remainder.len() >= 13230);
        assert!(remainder.iter().any(|s| s.abs() > 0.01));
        feed.join().unwrap();
    }

    #[tokio::test]
    async fn provider_failure_reaches_ui_without_decoder_error_race() {
        let providers = Arc::new(Providers::default());
        let (tx, mut rx) = mpsc::channel(8);
        let player = Playback::start(
            providers,
            Track {
                provider: crate::provider::ProviderId::Invidious,
                id: "abcdefghijk".into(),
                title: "Synthetic".into(),
                artist: "Channel".into(),
                album: String::new(),
                duration_secs: 120,
            },
            7,
            tx,
            0,
            false,
            1.0,
            AudioOutput::default(),
            &mut Preparation::default(),
        );
        let action = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(action, Action::PlaybackUpdate { id: 7, state: PlaybackState::Error(e), .. } if e.contains("Invidious is disabled"))
        );
        drop(player);
    }
    #[tokio::test]
    #[ignore = "requires an audio output device; plays a generated MP3 tone"]
    async fn audio_device_promotes_prepared_audio_and_preserves_pause_and_gain() {
        let track = synthetic_track("prepared");
        let (feed, bytes) = mpsc::channel(32);
        let network = tokio::spawn(async move {
            for chunk in include_bytes!("../tests/fixtures/tone.mp3").chunks(2048) {
                if feed.send(chunk.to_vec()).await.is_err() {
                    break;
                }
            }
        });
        let mut preparation = Preparation {
            next: Some(Prepared {
                track: track.clone(),
                rx: Some(bytes),
                cancelled: Arc::new(AtomicBool::new(false)),
                error: Arc::new(Mutex::new(None)),
                network,
            }),
        };
        let output = AudioOutput::default();
        let (tx, mut rx) = mpsc::channel(32);
        // No provider is connected: any second stream request would fail this test.
        let player = Playback::start(
            Arc::new(Providers::default()),
            track,
            42,
            tx,
            0,
            true,
            0.0,
            output.clone(),
            &mut preparation,
        );
        let paused = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            paused,
            Action::PlaybackUpdate {
                id: 42,
                state: PlaybackState::Paused,
                elapsed_ms: 0,
                ..
            }
        ));
        {
            let shared = output.0.lock().unwrap();
            let sink = shared.as_ref().unwrap().active.upgrade().unwrap();
            assert_eq!(sink.volume(), 0.0);
            assert!(sink.is_paused());
        }
        player.toggle_pause();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match rx.recv().await.unwrap() {
                    Action::PlaybackUpdate {
                        state: PlaybackState::Finished,
                        ..
                    } => break,
                    Action::PlaybackUpdate {
                        state: PlaybackState::Error(e),
                        ..
                    } => panic!("{e}"),
                    _ => {}
                }
            }
        })
        .await
        .unwrap();
        assert!(preparation.next.is_none());
        drop(player);
    }

    #[test]
    #[ignore = "requires an audio output device"]
    fn audio_device_reuses_output_and_recovers_after_failure() {
        let output = AudioOutput::default();
        let cancel = AtomicBool::new(false);
        let (first, original_failure) = output.sink(&cancel).unwrap();
        let (second, same_failure) = output.sink(&cancel).unwrap();
        assert!(Arc::ptr_eq(&original_failure, &same_failure));
        assert!(!Arc::ptr_eq(&first, &second));
        original_failure.store(true, Ordering::Relaxed);
        let (_, recovered_failure) = output.sink(&cancel).unwrap();
        assert!(!Arc::ptr_eq(&original_failure, &recovered_failure));
        assert!(!recovered_failure.load(Ordering::Relaxed));
        cancel.store(true, Ordering::Relaxed);
        assert!(output.sink(&cancel).is_err());
    }

    #[test]
    #[ignore = "requires an audio device; plays a generated quarter-second tone"]
    fn audio_device_smoke() {
        audio_device_fixture(include_bytes!("../tests/fixtures/tone.mp3"));
    }
    #[test]
    #[ignore = "requires an audio device; plays a generated quarter-second AAC tone"]
    fn audio_device_m4a_smoke() {
        audio_device_fixture(include_bytes!("../tests/fixtures/tone.m4a"));
    }
    fn audio_device_fixture(bytes: &'static [u8]) {
        let (tx, rx) = mpsc::channel(32);
        for chunk in bytes.chunks(2048) {
            tx.try_send(chunk.to_vec()).unwrap();
        }
        drop(tx);
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let (notify, mut events) = mpsc::channel(64);
        let worker = thread::spawn(move || {
            play(
                rx,
                stop,
                Arc::new(AtomicBool::new(false)),
                1,
                &notify,
                0,
                Arc::new(AtomicU32::new(1.0f32.to_bits())),
                AudioOutput::default(),
            )
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut finished = false;
        while std::time::Instant::now() < deadline {
            if matches!(
                events.try_recv(),
                Ok(Action::PlaybackUpdate {
                    state: PlaybackState::Finished,
                    ..
                })
            ) {
                finished = true;
                break;
            }
            if worker.is_finished() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        while let Ok(Action::PlaybackUpdate { state, .. }) = events.try_recv() {
            finished |= state == PlaybackState::Finished;
        }
        cancel.store(true, Ordering::Relaxed);
        worker.join().unwrap().unwrap();
        assert!(finished, "audio did not reach completion");
    }
    #[test]
    fn decodes_generated_mp3_from_small_stream_chunks_without_seeking() {
        let (tx, rx) = mpsc::channel(2);
        let bytes = include_bytes!("../tests/fixtures/tone.mp3");
        let feed = thread::spawn(move || {
            for chunk in bytes.chunks(127) {
                tx.blocking_send(chunk.to_vec()).unwrap();
            }
        });
        let reader = AudioReader {
            rx,
            current: io::Cursor::new(Vec::new()),
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        let decoder = Decoder::builder()
            .with_data(reader)
            .with_seekable(false)
            .build()
            .unwrap();
        assert_eq!(decoder.channels(), 2);
        assert_eq!(decoder.sample_rate(), 44100);
        let samples: Vec<f32> = decoder.collect();
        assert!(samples.len() >= 22050);
        assert!(samples.iter().any(|sample| sample.abs() > 0.01));
        feed.join().unwrap();
        let mut seek_decoder = Decoder::builder()
            .with_data(io::Cursor::new(bytes.to_vec()))
            .with_seekable(false)
            .build()
            .unwrap();
        let cancel = AtomicBool::new(false);
        assert!(discard_samples(&mut seek_decoder, 8820, &cancel));
        let remainder: Vec<_> = seek_decoder.by_ref().collect();
        assert_eq!(remainder, samples[8820..]);
        assert!(!discard_samples(&mut seek_decoder, 1, &cancel));
        cancel.store(true, Ordering::Relaxed);
        assert!(!discard_samples(&mut std::iter::repeat(0.0), 100, &cancel));
    }
    #[test]
    fn reader_handles_boundaries_eof_and_cancellation() {
        let (tx, rx) = mpsc::channel(2);
        tx.try_send(vec![1, 2]).unwrap();
        tx.try_send(vec![3]).unwrap();
        drop(tx);
        let cancel = Arc::new(AtomicBool::new(false));
        let mut reader = AudioReader {
            rx,
            current: io::Cursor::new(Vec::new()),
            cancelled: cancel.clone(),
        };
        let mut data = Vec::new();
        reader.read_to_end(&mut data).unwrap();
        assert_eq!(data, [1, 2, 3]);
        cancel.store(true, Ordering::Relaxed);
        assert!(reader.read(&mut [0]).is_err());
    }
    #[test]
    fn pcm_starvation_is_silent_and_does_not_advance_track_time() {
        let (tx, rx) = sync::sync_channel(1);
        let samples = Arc::new(AtomicU64::new(0));
        let (recycle, _recycled) = sync::sync_channel(9);
        let mut pcm = Pcm {
            rx,
            current: Vec::new(),
            position: 0,
            recycle,
            channels: 2,
            rate: 44100,
            samples: samples.clone(),
            buffering: Arc::new(AtomicBool::new(false)),
            silence_remaining: 0,
        };
        assert_eq!(pcm.next(), Some(0.0));
        assert_eq!(pcm.next(), Some(0.0)); // Preserve stereo frame alignment.
        assert_eq!(samples.load(Ordering::Relaxed), 0);
        tx.send(vec![0.5, -0.5]).unwrap();
        drop(tx);
        assert_eq!(pcm.collect::<Vec<_>>(), [0.5, -0.5]);
        assert_eq!(samples.load(Ordering::Relaxed), 2);
    }
}
