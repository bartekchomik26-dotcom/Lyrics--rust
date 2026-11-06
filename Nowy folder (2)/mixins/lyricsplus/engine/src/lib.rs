//! LyricsPlus for Horizon: everything that is not drawing. The game glue
//! (`mixins/lyricsplus/glue.rs`) asks [`engine`] for a [`Snapshot`] each
//! frame and draws it; the engine's threads keep it current:
//!
//! * media sources ([`Source`]): the Spotify Web API (the player's own app
//!   key, PKCE, no secret), the system's media session (Windows' media
//!   controls, MPRIS through `playerctl`, Spotify/Music through `osascript`)
//!   and a local HTTP bridge any player can post to (a userscript ships for
//!   SoundCloud, YouTube Music, Deezer, Tidal, Apple Music and the Spotify web
//!   player);
//! * synced lyrics from LRCLIB (or from the bridge), cached on disk;
//! * album art bytes, which the glue decodes and [`art::process`]es into a
//!   texture and a palette.

pub mod art;
mod bridge;
pub mod config;
pub mod glyphs;
mod http;
pub mod lrc;
mod lyrics;
pub mod player;
mod spotify;
mod system;
mod userscript;
pub mod util;

pub use spotify::{BUILT_IN_CLIENT_ID, open_browser};
pub use userscript::USERSCRIPT;

pub use config::*;
pub use lrc::{Line, Lyrics};

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

/// Where a track came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Spotify,
    System,
    Bridge,
    /// The built-in offline player.
    Local,
}

impl Origin {
    fn slot(self) -> usize {
        self as usize
    }
}

/// What a source last saw playing.
#[derive(Clone, Debug)]
pub struct Sample {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    pub position_ms: u64,
    pub playing: bool,
    pub art_url: Option<String>,
    /// Cover bytes the source has itself (the offline player's ID3 picture).
    pub art_bytes: Option<Arc<Vec<u8>>>,
    /// The player: `Spotify`, `SoundCloud`, `chrome.exe`…
    pub app: String,
    /// Lyrics the source supplied itself (the bridge), LRC or plain.
    pub lyrics: Option<String>,
    pub at: Instant,
}

impl Sample {
    fn key(&self) -> String {
        format!(
            "{}\u{1f}{}",
            util::normalize(&self.title),
            util::normalize(&util::primary_artist(&self.artist))
        )
    }
    fn valid(&self) -> bool {
        !self.title.trim().is_empty()
    }
}

/// The track being shown.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub key: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    pub origin: Origin,
    pub app: String,
    /// Changes with every new track (also the same song started again
    /// after another), for animations keyed on it.
    pub serial: u64,
}

/// Where the lyrics of the current track are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LyricsState {
    None,
    Loading,
    Found,
    NotFound,
    Instrumental,
    Failed,
}

/// The album art of the current track, as fetched.
#[derive(Clone, Debug)]
pub struct ArtBytes {
    /// The track's serial.
    pub serial: u64,
    pub bytes: Arc<Vec<u8>>,
}

/// Spotify's link, as the menu shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpotifyStatus {
    NoClientId,
    NotLinked,
    /// Waiting for the browser; the URL that was opened.
    Linking(String),
    Linked(String),
    Error(String),
}

/// A media control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    PlayPause,
    Next,
    Previous,
}

/// One frame's view of the engine.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub track: Option<Arc<Track>>,
    pub position_ms: u64,
    pub playing: bool,
    pub lyrics: Arc<Lyrics>,
    pub lyrics_state: LyricsState,
    pub art: Option<ArtBytes>,
    pub spotify: SpotifyStatus,
    /// Seconds since a browser last posted to the bridge.
    pub bridge_seen: Option<f32>,
    /// Whether the system media source is running.
    pub system_running: bool,
    pub track_age: Duration,
}

struct Current {
    track: Arc<Track>,
    base_ms: f64,
    base_at: Instant,
    playing: bool,
    started: Instant,
}

impl Current {
    fn estimate(&self, now: Instant) -> f64 {
        let mut p = self.base_ms;
        if self.playing {
            p += now.saturating_duration_since(self.base_at).as_secs_f64() * 1000.0;
        }
        let d = self.track.duration_ms as f64;
        if d > 0.0 { p.min(d) } else { p }
    }
}

struct Shared {
    dir: PathBuf,
    config: Config,
    samples: [Option<Sample>; 4],
    current: Option<Current>,
    lyrics: Arc<Lyrics>,
    lyrics_state: LyricsState,
    art: Option<ArtBytes>,
    spotify: SpotifyStatus,
    bridge_seen: Option<Instant>,
    bridge_command: Option<Control>,
    system_running: bool,
    messages: Vec<String>,
    last_origin: Option<Origin>,
}

/// The engine: shared state and its threads.
pub struct Engine {
    shared: Mutex<Shared>,
    serial: AtomicU64,
    stop: AtomicBool,
    config_version: AtomicU64,
}

static ENGINE: OnceLock<Engine> = OnceLock::new();

/// The engine, started on first use with the mod's data folder `dir`
/// (later calls ignore `dir`).
pub fn engine(dir: impl FnOnce() -> PathBuf) -> &'static Engine {
    if let Some(e) = ENGINE.get() {
        return e;
    }
    let mut started = false;
    let e = ENGINE.get_or_init(|| {
        started = true;
        let dir = dir();
        let _ = std::fs::create_dir_all(&dir);
        let config = Config::load(&dir);
        let spotify = spotify::initial_status(&dir, &config);
        Engine {
            shared: Mutex::new(Shared {
                dir,
                config,
                samples: [None, None, None, None],
                current: None,
                lyrics: Arc::new(Lyrics::default()),
                lyrics_state: LyricsState::None,
                art: None,
                spotify,
                bridge_seen: None,
                bridge_command: None,
                system_running: false,
                messages: Vec::new(),
                last_origin: None,
            }),
            serial: AtomicU64::new(1),
            stop: AtomicBool::new(false),
            config_version: AtomicU64::new(1),
        }
    });
    if started {
        e.start();
    }
    e
}

/// The engine if it has started.
pub fn running() -> Option<&'static Engine> {
    ENGINE.get()
}

impl Engine {
    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|p| p.into_inner())
    }
    fn start(&'static self) {
        let spawn = |name: &str, f: fn(&'static Engine)| {
            let _ = std::thread::Builder::new()
                .name(format!("lyricsplus-{name}"))
                .spawn(move || f(self));
        };
        spawn("select", Engine::select_loop);
        spawn("spotify", spotify::run);
        spawn("system", system::run);
        spawn("bridge", bridge::run);
        player::rescan(self);
    }
    pub(crate) fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
    /// Stops the threads and the media helper process (the game is closing).
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Relaxed);
        system::kill();
        player::shutdown();
    }
    pub fn data_dir(&self) -> PathBuf {
        self.lock().dir.clone()
    }

    // ----- settings -----

    pub fn config(&self) -> Config {
        self.lock().config.clone()
    }
    /// Bumped on every change, so the glue re-reads only then.
    pub fn config_version(&self) -> u64 {
        self.config_version.load(Ordering::Relaxed)
    }
    /// Changes the settings and saves them.
    pub fn update_config(&self, f: impl FnOnce(&mut Config)) {
        let mut s = self.lock();
        let before = s.config.clone();
        f(&mut s.config);
        s.config = s.config.clone().sanitized();
        if s.config != before {
            s.config.save(&s.dir);
            if s.config.spotify_client_id != before.spotify_client_id {
                s.spotify = spotify::initial_status(&s.dir, &s.config);
            }
            self.config_version.fetch_add(1, Ordering::Relaxed);
        }
    }
    pub fn reset_config(&self) {
        self.update_config(|c| {
            let id = std::mem::take(&mut c.spotify_client_id);
            *c = Config::default();
            c.spotify_client_id = id;
        });
    }

    // ----- what the glue reads -----

    pub fn snapshot(&self) -> Snapshot {
        let s = self.lock();
        let now = Instant::now();
        let (track, position_ms, playing, track_age) = match &s.current {
            Some(c) => (
                Some(c.track.clone()),
                c.estimate(now).max(0.0) as u64,
                c.playing,
                now.saturating_duration_since(c.started),
            ),
            None => (None, 0, false, Duration::ZERO),
        };
        Snapshot {
            track,
            position_ms,
            playing,
            lyrics: s.lyrics.clone(),
            lyrics_state: s.lyrics_state,
            art: s.art.clone(),
            spotify: s.spotify.clone(),
            bridge_seen: s.bridge_seen.map(|t| now.saturating_duration_since(t).as_secs_f32()),
            system_running: s.system_running,
            track_age,
        }
    }
    /// Lines for chat since the last call.
    pub fn drain_messages(&self) -> Vec<String> {
        std::mem::take(&mut self.lock().messages)
    }
    pub fn message(&self, text: impl Into<String>) {
        let mut s = self.lock();
        if s.messages.len() < 32 {
            s.messages.push(text.into());
        }
    }

    // ----- actions -----

    /// Play/pause, next or previous on the source showing the track.
    pub fn control(&'static self, c: Control) {
        let (origin, playing) = {
            let s = self.lock();
            match &s.current {
                Some(cur) => (Some(cur.track.origin), cur.playing),
                None => (s.last_origin, false),
            }
        };
        // Answer at once: a pause shows before the next poll confirms it.
        if c == Control::PlayPause && origin != Some(Origin::Local) {
            let mut s = self.lock();
            let now = Instant::now();
            if let Some(cur) = s.current.as_mut() {
                cur.base_ms = cur.estimate(now);
                cur.base_at = now;
                cur.playing = !cur.playing;
            }
        }
        let _ = std::thread::Builder::new()
            .name("lyricsplus-control".into())
            .spawn(move || match origin {
                Some(Origin::Spotify) => spotify::control(self, c, playing),
                Some(Origin::Bridge) => self.lock().bridge_command = Some(c),
                Some(Origin::Local) => player::control(self, c),
                _ => system::control(c),
            });
    }
    /// Opens Spotify's consent page; the link completes in the background.
    pub fn spotify_connect(&'static self) -> Result<String, String> {
        spotify::connect(self)
    }
    pub fn spotify_disconnect(&self) {
        let mut s = self.lock();
        spotify::forget(&s.dir);
        s.spotify = spotify::initial_status(&s.dir, &s.config);
    }
    /// Forgets cached lyrics for the current track and fetches them again.
    pub fn reload_lyrics(&'static self) {
        let track = self.lock().current.as_ref().map(|c| c.track.clone());
        if let Some(t) = track {
            let dir = self.data_dir();
            lyrics::forget(&dir, &t);
            self.fetch_for(t, None);
        }
    }
    /// Seeks the shown position by `delta_ms` (for a source that can't seek,
    /// only until its next report).
    pub fn nudge(&self, delta_ms: i64) {
        let mut s = self.lock();
        if let Some(c) = s.current.as_mut() {
            c.base_ms = (c.base_ms + delta_ms as f64).max(0.0);
        }
    }

    // ----- sources report here -----

    pub(crate) fn report(&self, origin: Origin, sample: Option<Sample>) {
        let mut s = self.lock();
        if origin == Origin::Bridge && sample.is_some() {
            s.bridge_seen = Some(Instant::now());
        }
        s.samples[origin.slot()] = sample.filter(Sample::valid);
    }
    pub(crate) fn set_system_running(&self, on: bool) {
        self.lock().system_running = on;
    }
    pub(crate) fn take_bridge_command(&self) -> Option<Control> {
        self.lock().bridge_command.take()
    }
    pub(crate) fn set_spotify(&self, status: SpotifyStatus) {
        self.lock().spotify = status;
    }
    pub(crate) fn with_config<R>(&self, f: impl FnOnce(&Config, &PathBuf) -> R) -> R {
        let s = self.lock();
        f(&s.config, &s.dir)
    }

    // ----- choosing what shows -----

    fn select_loop(&'static self) {
        while !self.stopped() {
            self.select();
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Picks the sample to show and follows it.
    fn select(&'static self) {
        let now = Instant::now();
        let fetch = {
            let mut s = self.lock();
            let source = s.config.source;
            let fresh = |o: Origin, s: &Shared| {
                let limit = match o {
                    Origin::Spotify => Duration::from_secs(12),
                    Origin::System => Duration::from_secs(4),
                    Origin::Bridge => Duration::from_secs(5),
                    Origin::Local => Duration::from_secs(2),
                };
                s.samples[o.slot()]
                    .as_ref()
                    .filter(|x| now.saturating_duration_since(x.at) < limit)
                    .cloned()
            };
            let order: &[Origin] = match source {
                Source::Spotify => &[Origin::Spotify],
                Source::System => &[Origin::System],
                Source::Bridge => &[Origin::Bridge],
                Source::Local => &[Origin::Local],
                // The bridge knows the web player exactly; Spotify's API
                // is exact too; the system session is the catch-all.
                Source::Auto => &[Origin::Local, Origin::Spotify, Origin::Bridge, Origin::System],
            };
            let sticky = s.current.as_ref().map(|c| c.track.origin);
            let candidates: Vec<(Origin, Sample)> =
                order.iter().filter_map(|&o| fresh(o, &s).map(|x| (o, x))).collect();
            // Something playing wins; the source already shown wins a tie.
            let pick = candidates
                .iter()
                .find(|(o, x)| x.playing && Some(*o) == sticky)
                .or_else(|| candidates.iter().find(|(_, x)| x.playing))
                .or_else(|| candidates.iter().find(|(o, _)| Some(*o) == sticky))
                .or_else(|| candidates.first())
                .cloned();
            match pick {
                None => {
                    if s.current.is_some() {
                        s.last_origin = s.current.as_ref().map(|c| c.track.origin);
                        s.current = None;
                        s.lyrics = Arc::new(Lyrics::default());
                        s.lyrics_state = LyricsState::None;
                        s.art = None;
                    }
                    None
                }
                Some((origin, sample)) => self.follow(&mut s, origin, sample, now),
            }
        };
        if let Some((track, art, supplied)) = fetch {
            self.fetch_for(track.clone(), supplied);
            self.fetch_art(track, art);
        }
    }

    /// Updates the current track from `sample`; a new track comes back with
    /// its art URL and supplied lyrics, to fetch.
    #[allow(clippy::type_complexity)]
    fn follow(
        &self,
        s: &mut Shared,
        origin: Origin,
        sample: Sample,
        now: Instant,
    ) -> Option<(Arc<Track>, (Option<String>, Option<Arc<Vec<u8>>>), Option<String>)> {
        let key = sample.key();
        // Where the sample says playback is now.
        let mut reported = sample.position_ms as f64;
        if sample.playing {
            reported += now.saturating_duration_since(sample.at).as_secs_f64() * 1000.0;
        }
        if let Some(c) = s.current.as_mut()
            && c.track.key == key
            && c.track.origin == origin
        {
            let est = c.estimate(now);
            let drift = reported - est;
            if c.playing != sample.playing || drift.abs() > 1500.0 {
                c.base_ms = reported;
                c.base_at = now;
                c.playing = sample.playing;
            } else if sample.playing {
                // Small differences are network jitter: ease toward them.
                c.base_ms = est + drift * 0.15;
                c.base_at = now;
            }
            if sample.duration_ms > 0 && c.track.duration_ms != sample.duration_ms {
                let mut t = (*c.track).clone();
                t.duration_ms = sample.duration_ms;
                c.track = Arc::new(t);
            }
            return None;
        }
        // Same song from another source: keep its lyrics and art.
        if let Some(c) = s.current.as_mut()
            && c.track.key == key
        {
            let mut t = (*c.track).clone();
            t.origin = origin;
            t.app = sample.app.clone();
            c.track = Arc::new(t);
            c.base_ms = reported;
            c.base_at = now;
            c.playing = sample.playing;
            return None;
        }
        let serial = self.serial.fetch_add(1, Ordering::Relaxed);
        let track = Arc::new(Track {
            key,
            title: sample.title.trim().to_owned(),
            artist: sample.artist.trim().to_owned(),
            album: sample.album.trim().to_owned(),
            duration_ms: sample.duration_ms,
            origin,
            app: sample.app.clone(),
            serial,
        });
        s.current = Some(Current {
            track: track.clone(),
            base_ms: reported,
            base_at: now,
            playing: sample.playing,
            started: now,
        });
        s.last_origin = Some(origin);
        s.lyrics = Arc::new(Lyrics::default());
        s.lyrics_state = LyricsState::Loading;
        s.art = None;
        Some((track, (sample.art_url.clone(), sample.art_bytes.clone()), sample.lyrics.clone()))
    }

    fn fetch_for(&'static self, track: Arc<Track>, supplied: Option<String>) {
        {
            let mut s = self.lock();
            if s.current.as_ref().is_some_and(|c| c.track.serial == track.serial) {
                s.lyrics_state = LyricsState::Loading;
            }
        }
        let _ = std::thread::Builder::new()
            .name("lyricsplus-lyrics".into())
            .spawn(move || {
                let dir = self.data_dir();
                let (lyrics, state) = lyrics::find(&dir, &track, supplied.as_deref());
                let mut s = self.lock();
                if s.current.as_ref().is_some_and(|c| c.track.serial == track.serial) {
                    s.lyrics = Arc::new(lyrics);
                    s.lyrics_state = state;
                }
            });
    }

    fn fetch_art(&'static self, track: Arc<Track>, (url, have): (Option<String>, Option<Arc<Vec<u8>>>)) {
        let _ = std::thread::Builder::new()
            .name("lyricsplus-art".into())
            .spawn(move || {
                let bytes = match have {
                    Some(b) => (*b).clone(),
                    None => match art::fetch(&track, url.as_deref()) {
                        Some(b) => b,
                        None => return,
                    },
                };
                let mut s = self.lock();
                if s.current.as_ref().is_some_and(|c| c.track.serial == track.serial) {
                    s.art = Some(ArtBytes {
                        serial: track.serial,
                        bytes: Arc::new(bytes),
                    });
                }
            });
    }
    /// Lyrics a source (the bridge) sends after the track began.
    pub(crate) fn supply_lyrics(&self, key_title: &str, text: &str) {
        let mut s = self.lock();
        let Some(c) = &s.current else {
            return;
        };
        if util::normalize(&c.track.title) != util::normalize(key_title) {
            return;
        }
        let duration = c.track.duration_ms;
        if let Some(l) = lyrics::from_text(text, duration, "Bridge") {
            s.lyrics = Arc::new(l);
            s.lyrics_state = LyricsState::Found;
        }
    }
    /// The bridge's `/now`: what is showing, for overlays (OBS) and other apps.
    pub(crate) fn now_json(&self) -> serde_json::Value {
        let snap = self.snapshot();
        let offset = self.lock().config.offset_ms;
        let pos = (snap.position_ms as i64 + offset).max(0) as u64;
        let line = snap.lyrics.index_at(pos);
        let text = |i: Option<usize>| {
            i.and_then(|i| snap.lyrics.lines.get(i))
                .map(|l| l.text.clone())
                .unwrap_or_default()
        };
        match &snap.track {
            None => serde_json::json!({ "playing": false, "track": null }),
            Some(t) => serde_json::json!({
                "playing": snap.playing,
                "track": {
                    "title": t.title, "artist": t.artist, "album": t.album,
                    "durationMs": t.duration_ms, "source": format!("{:?}", t.origin), "app": t.app,
                },
                "positionMs": snap.position_ms,
                "synced": snap.lyrics.synced,
                "line": text(line),
                "next": text(line.map_or(Some(0), |i| Some(i + 1))),
                "lineIndex": line,
            }),
        }
    }
}
