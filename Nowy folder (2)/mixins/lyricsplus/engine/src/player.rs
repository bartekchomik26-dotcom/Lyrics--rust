//! An offline music player: the files in the music folder (MP3, M4A, AAC,
//! WAV, WMA, FLAC, OGG), their ID3 tags and covers, and `.lrc` files beside
//! them for lyrics with no connection at all.
//!
//! Horizon's lock has no MP3 decoder, so playback is the system's own:
//! Windows Media Player's engine (`WMPlayer.OCX`) in a hidden PowerShell
//! helper on Windows; `mpv`, `ffplay` or `afplay` elsewhere.

use crate::{Control, Engine, Origin, Sample};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const AUDIO: [&str; 8] = ["mp3", "m4a", "aac", "wav", "wma", "flac", "ogg", "opus"];

/// A song in the library.
#[derive(Clone, Debug, PartialEq)]
pub struct Song {
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Has a `.lrc` beside it.
    pub lrc: bool,
}

/// What the player is doing, for the menu.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerState {
    pub current: Option<usize>,
    pub playing: bool,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub backend: String,
}

#[derive(Default)]
struct Inner {
    songs: Vec<Song>,
    current: Option<usize>,
    playing: bool,
    /// Where playback was when `since` was taken (for backends that don't report).
    base_ms: u64,
    since: Option<Instant>,
    duration_ms: u64,
    reported: Option<Instant>,
    art: Option<Arc<Vec<u8>>>,
    lyrics: Option<String>,
    history: Vec<usize>,
    backend: String,
    scanning: bool,
    /// Set when the song finished on its own.
    ended: bool,
    /// When the last song was opened (the helper's reports lag a little).
    opened: Option<Instant>,
}

static PLAYER: Mutex<Option<Inner>> = Mutex::new(None);
static SEQ: AtomicU64 = AtomicU64::new(1);
static STARTED: AtomicBool = AtomicBool::new(false);
static HELPER: Mutex<Option<Child>> = Mutex::new(None);
/// Commands not yet read by the Windows helper: (seq, op, arg).
static QUEUE: Mutex<Vec<(u64, String, String)>> = Mutex::new(Vec::new());

fn with<R>(f: impl FnOnce(&mut Inner) -> R) -> R {
    let mut slot = PLAYER.lock().unwrap_or_else(|p| p.into_inner());
    f(slot.get_or_insert_with(Inner::default))
}

/// The music folder: the configured one, else `<data>/music`.
pub fn folder(engine: &Engine) -> PathBuf {
    engine.with_config(|c, d| {
        if c.music_folder.trim().is_empty() {
            d.join("music")
        } else {
            PathBuf::from(c.music_folder.trim())
        }
    })
}

fn walk(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if out.len() >= 5000 {
            return;
        }
        if p.is_dir() {
            if depth > 0 {
                walk(&p, depth - 1, out);
            }
        } else if p
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .is_some_and(|e| AUDIO.contains(&e.as_str()))
        {
            out.push(p);
        }
    }
}

/// Scans the folder again, in the background.
pub fn rescan(engine: &'static Engine) {
    if with(|p| std::mem::replace(&mut p.scanning, true)) {
        return;
    }
    let dir = folder(engine);
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::thread::Builder::new().name("lyricsplus-library".into()).spawn(move || {
        let mut files = Vec::new();
        walk(&dir, 4, &mut files);
        let songs: Vec<Song> = files.into_iter().map(|p| song(&p)).collect();
        let n = songs.len();
        with(|p| {
            let current = p.current.and_then(|i| p.songs.get(i)).map(|s| s.path.clone());
            p.songs = songs;
            p.current = current.and_then(|c| p.songs.iter().position(|s| s.path == c));
            p.scanning = false;
        });
        engine.message(format!("Music library: {n} songs in {}", dir.display()));
    });
}

/// How many songs the library has.
pub fn count() -> usize {
    with(|p| p.songs.len())
}

/// The playing song's title and artist.
pub fn current() -> Option<(String, String)> {
    with(|p| p.current.and_then(|i| p.songs.get(i)).map(|s| (s.title.clone(), s.artist.clone())))
}

/// The library as it is now.
pub fn songs() -> Vec<Song> {
    with(|p| p.songs.clone())
}

pub fn state() -> PlayerState {
    with(|p| PlayerState {
        current: p.current,
        playing: p.playing,
        position_ms: position(p),
        duration_ms: p.duration_ms,
        backend: p.backend.clone(),
    })
}

fn position(p: &Inner) -> u64 {
    match (p.playing, p.since) {
        (true, Some(t)) => p.base_ms + t.elapsed().as_millis() as u64,
        _ => p.base_ms,
    }
}

/// A song's tags (ID3) or its file name (`Artist - Title`).
fn song(path: &Path) -> Song {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let (mut artist, mut title) = match stem.split_once(" - ") {
        Some((a, t)) => (a.trim().to_owned(), t.trim().to_owned()),
        None => (String::new(), stem.trim().to_owned()),
    };
    let mut album = String::new();
    if let Some(tags) = id3(path, false) {
        if !tags.title.is_empty() {
            title = tags.title;
        }
        if !tags.artist.is_empty() {
            artist = tags.artist;
        }
        album = tags.album;
    }
    Song { lrc: path.with_extension("lrc").exists(), path: path.to_path_buf(), title, artist, album }
}

/// ID3 tags.
#[derive(Default, Debug, PartialEq)]
pub struct Tags {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub picture: Option<Vec<u8>>,
}

fn id3(path: &Path, picture: bool) -> Option<Tags> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut head = [0u8; 10];
    f.read_exact(&mut head).ok()?;
    if &head[..3] != b"ID3" {
        return None;
    }
    let size = synchsafe(&head[6..10]) as usize;
    if size > 64 * 1024 * 1024 {
        return None;
    }
    let mut body = vec![0u8; size];
    f.read_exact(&mut body).ok()?;
    Some(parse_id3(head[3], head[5], &body, picture))
}

fn synchsafe(b: &[u8]) -> u32 {
    b.iter().fold(0u32, |acc, &x| acc << 7 | u32::from(x & 0x7F))
}

/// The frames of an ID3v2.2/2.3/2.4 tag body.
pub fn parse_id3(version: u8, flags: u8, body: &[u8], picture: bool) -> Tags {
    let mut tags = Tags::default();
    let mut i = 0usize;
    // An extended header (2.3/2.4) is skipped.
    if flags & 0x40 != 0 && body.len() >= 4 {
        let ext = if version >= 4 { synchsafe(&body[..4]) as usize } else { u32::from_be_bytes([body[0], body[1], body[2], body[3]]) as usize + 4 };
        i = ext.min(body.len());
    }
    let (id_len, header) = if version <= 2 { (3, 6) } else { (4, 10) };
    while i + header <= body.len() {
        let id = &body[i..i + id_len];
        if id.iter().all(|&b| b == 0) {
            break;
        }
        let size = if version <= 2 {
            (usize::from(body[i + 3]) << 16) | (usize::from(body[i + 4]) << 8) | usize::from(body[i + 5])
        } else if version >= 4 {
            synchsafe(&body[i + 4..i + 8]) as usize
        } else {
            u32::from_be_bytes([body[i + 4], body[i + 5], body[i + 6], body[i + 7]]) as usize
        };
        let start = i + header;
        let end = start.saturating_add(size).min(body.len());
        let data = &body[start..end];
        match id {
            b"TIT2" | b"TT2" => tags.title = text_frame(data),
            b"TPE1" | b"TP1" => tags.artist = text_frame(data),
            b"TALB" | b"TAL" => tags.album = text_frame(data),
            b"APIC" if picture && tags.picture.is_none() => tags.picture = apic(data, false),
            b"PIC" if picture && tags.picture.is_none() => tags.picture = apic(data, true),
            _ => {}
        }
        if size == 0 {
            break;
        }
        i = end;
    }
    tags
}

fn decode_text(enc: u8, data: &[u8]) -> String {
    match enc {
        1 | 2 => {
            let (be, data) = match data {
                [0xFE, 0xFF, rest @ ..] => (true, rest),
                [0xFF, 0xFE, rest @ ..] => (false, rest),
                _ => (enc == 2, data),
            };
            let units: Vec<u16> = data
                .chunks_exact(2)
                .map(|c| if be { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) })
                .take_while(|&u| u != 0)
                .collect();
            String::from_utf16_lossy(&units)
        }
        3 => String::from_utf8_lossy(data.split(|&b| b == 0).next().unwrap_or(&[])).into_owned(),
        _ => data.iter().take_while(|&&b| b != 0).map(|&b| b as char).collect(),
    }
}

fn text_frame(data: &[u8]) -> String {
    match data.split_first() {
        Some((&enc, rest)) => decode_text(enc, rest).trim().to_owned(),
        None => String::new(),
    }
}

/// An attached picture's bytes.
fn apic(data: &[u8], v22: bool) -> Option<Vec<u8>> {
    let (&enc, rest) = data.split_first()?;
    let rest = if v22 {
        rest.get(3..)?
    } else {
        let mime_end = rest.iter().position(|&b| b == 0)?;
        rest.get(mime_end + 1..)?
    };
    // Picture type, then a description ending in a terminator of the encoding.
    let rest = rest.get(1..)?;
    let wide = matches!(enc, 1 | 2);
    let mut k = 0;
    while k < rest.len() {
        if wide {
            if k + 1 < rest.len() && rest[k] == 0 && rest[k + 1] == 0 {
                k += 2;
                break;
            }
            k += 2;
        } else {
            if rest[k] == 0 {
                k += 1;
                break;
            }
            k += 1;
        }
    }
    let img = rest.get(k..)?;
    (img.len() > 64).then(|| img.to_vec())
}

// ----- playback

fn send(op: &str, arg: &str) {
    if cfg!(windows) {
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let mut q = QUEUE.lock().unwrap_or_else(|p| p.into_inner());
        q.push((seq, op.to_owned(), arg.to_owned()));
        if q.len() > 16 {
            let drop = q.len() - 16;
            q.drain(..drop);
        }
    }
}

fn kill_helper() {
    if let Some(mut c) = HELPER.lock().unwrap_or_else(|p| p.into_inner()).take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}

/// Stops everything (the game is closing).
pub fn shutdown() {
    send("stop", "");
    std::thread::sleep(Duration::from_millis(150));
    kill_helper();
}

/// Plays song `i` from `from_ms`.
pub fn play(engine: &'static Engine, i: usize) {
    start(engine);
    let Some(song) = with(|p| p.songs.get(i).cloned()) else { return };
    let tags = id3(&song.path, true);
    let lyrics = std::fs::read_to_string(song.path.with_extension("lrc")).ok();
    with(|p| {
        if let Some(c) = p.current
            && c != i
        {
            p.history.push(c);
            if p.history.len() > 200 {
                p.history.remove(0);
            }
        }
        p.current = Some(i);
        p.playing = true;
        p.base_ms = 0;
        p.since = Some(Instant::now());
        p.duration_ms = 0;
        p.ended = false;
        p.opened = Some(Instant::now());
        p.art = tags.and_then(|t| t.picture).map(Arc::new);
        p.lyrics = lyrics;
    });
    if cfg!(windows) {
        send("open", &song.path.to_string_lossy());
        let volume = engine.with_config(|c, _| (c.volume * 100.0).round() as u32);
        send("volume", &volume.to_string());
    } else {
        spawn_external(&song.path, 0);
    }
}

/// Play/pause, next, previous.
pub fn control(engine: &'static Engine, c: Control) {
    let (current, playing, n) = with(|p| (p.current, p.playing, p.songs.len()));
    match c {
        Control::PlayPause => match current {
            None if n > 0 => play(engine, 0),
            None => {}
            Some(i) if !playing => resume(engine, i),
            Some(_) => pause(),
        },
        Control::Next => {
            if let Some(i) = next_index(engine) {
                play(engine, i);
            }
        }
        Control::Previous => {
            // Back to the start when a few seconds in, else the song before.
            let pos = with(|p| position(p));
            match current {
                Some(i) if pos > 3000 => {
                    seek(i, 0);
                }
                _ => {
                    let prev = with(|p| p.history.pop()).or_else(|| current.map(|i| (i + n.max(1) - 1) % n.max(1)));
                    if let Some(i) = prev {
                        play(engine, i);
                    }
                }
            }
        }
    }
}

fn next_index(engine: &Engine) -> Option<usize> {
    let shuffle = engine.with_config(|c, _| c.shuffle);
    with(|p| {
        let n = p.songs.len();
        if n == 0 {
            return None;
        }
        if shuffle && n > 1 {
            let bytes = crate::util::random_bytes(8);
            let seed = bytes.iter().fold(0u64, |a, &b| a << 8 | u64::from(b));
            let mut r = (seed % n as u64) as usize;
            if Some(r) == p.current {
                r = (r + 1) % n;
            }
            return Some(r);
        }
        Some(p.current.map_or(0, |c| (c + 1) % n))
    })
}

fn pause() {
    with(|p| {
        p.base_ms = position(p);
        p.playing = false;
        p.since = None;
    });
    if cfg!(windows) {
        send("pause", "");
    } else {
        kill_helper();
    }
}

fn resume(engine: &'static Engine, i: usize) {
    start(engine);
    let from = with(|p| {
        p.playing = true;
        p.since = Some(Instant::now());
        p.base_ms
    });
    if cfg!(windows) {
        send("play", "");
    } else if let Some(song) = with(|p| p.songs.get(i).cloned()) {
        spawn_external(&song.path, from);
    }
}

fn seek(i: usize, ms: u64) {
    let playing = with(|p| {
        p.base_ms = ms;
        p.since = p.playing.then(Instant::now);
        p.playing
    });
    if cfg!(windows) {
        send("seek", &ms.to_string());
    } else if playing && let Some(song) = with(|p| p.songs.get(i).cloned()) {
        spawn_external(&song.path, ms);
    }
}

/// Stops playback.
pub fn stop() {
    with(|p| {
        p.playing = false;
        p.base_ms = 0;
        p.since = None;
        p.current = None;
    });
    if cfg!(windows) {
        send("stop", "");
    } else {
        kill_helper();
    }
}

pub fn set_volume(v: f32) {
    send("volume", &((v.clamp(0.0, 1.0) * 100.0).round() as u32).to_string());
}

/// mpv, ffplay or afplay from `from_ms` (pausing stops it; resuming starts again there).
fn spawn_external(path: &Path, from_ms: u64) {
    kill_helper();
    let secs = format!("{:.3}", from_ms as f64 / 1000.0);
    let tries: Vec<(&str, Vec<String>)> = vec![
        ("mpv", vec!["--no-video".into(), "--really-quiet".into(), format!("--start={secs}"), path.to_string_lossy().into_owned()]),
        ("ffplay", vec!["-nodisp".into(), "-autoexit".into(), "-loglevel".into(), "quiet".into(), "-ss".into(), secs.clone(), path.to_string_lossy().into_owned()]),
        ("afplay", vec![path.to_string_lossy().into_owned()]),
    ];
    for (cmd, args) in tries {
        if let Ok(child) = Command::new(cmd).args(&args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            with(|p| p.backend = cmd.to_owned());
            *HELPER.lock().unwrap_or_else(|p| p.into_inner()) = Some(child);
            return;
        }
    }
    with(|p| {
        p.playing = false;
        p.backend = "none (install mpv or ffplay)".into();
    });
}

/// Starts the player's thread once.
fn start(engine: &'static Engine) {
    if STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    let _ = std::thread::Builder::new().name("lyricsplus-player".into()).spawn(move || run(engine));
}

fn run(engine: &'static Engine) {
    let mut last_seq_written = 0u64;
    let mut failures = 0usize;
    while !engine.stopped() {
        if cfg!(windows) {
            // The helper: started on demand, fed commands through a file.
            let dir = engine.data_dir();
            let cmd = dir.join("player-commands.txt");
            let alive = HELPER.lock().unwrap_or_else(|p| p.into_inner()).as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(None)));
            if !alive {
                let script = dir.join("player.ps1");
                if std::fs::read_to_string(&script).ok().as_deref() != Some(WINDOWS_PLAYER) {
                    let _ = std::fs::write(&script, WINDOWS_PLAYER);
                }
                let _ = std::fs::write(&cmd, "");
                let mut c = Command::new("powershell.exe");
                c.args(["-NoProfile", "-NonInteractive", "-STA", "-ExecutionPolicy", "Bypass", "-File"])
                    .arg(&script)
                    .arg("-Cmd")
                    .arg(&cmd)
                    .arg("-ParentPid")
                    .arg(std::process::id().to_string())
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null());
                crate::system::hidden(&mut c);
                match c.spawn() {
                    Ok(mut child) => {
                        if let Some(out) = child.stdout.take() {
                            let _ = std::thread::Builder::new().name("lyricsplus-player-out".into()).spawn(move || read_helper(out));
                        }
                        *HELPER.lock().unwrap_or_else(|p| p.into_inner()) = Some(child);
                        with(|p| p.backend = "Windows Media Player".into());
                        last_seq_written = 0;
                    }
                    Err(_) => {
                        with(|p| {
                            p.playing = false;
                            p.backend = "unavailable (PowerShell)".into();
                        });
                        std::thread::sleep(Duration::from_secs(5));
                        continue;
                    }
                }
            }
            // Pending commands into the file (the helper runs those it hasn't).
            let q = QUEUE.lock().unwrap_or_else(|p| p.into_inner()).clone();
            if let Some(&(top, _, _)) = q.last()
                && top != last_seq_written
            {
                let text: String = q.iter().map(|(s, o, a)| format!("{s}\t{o}\t{a}\n")).collect();
                let tmp = cmd.with_extension("tmp");
                if std::fs::write(&tmp, text).is_ok() && std::fs::rename(&tmp, &cmd).is_ok() {
                    last_seq_written = top;
                }
            }
        } else {
            // The external player ending means the song did.
            let ended = {
                let mut h = HELPER.lock().unwrap_or_else(|p| p.into_inner());
                matches!(h.as_mut().map(|c| c.try_wait()), Some(Ok(Some(_))))
                    && {
                        *h = None;
                        true
                    }
            };
            if ended && with(|p| p.playing) {
                with(|p| p.ended = true);
            }
        }
        // A song that ended plays the next (or the same, on repeat).
        if let Some(quick) = with(|p| std::mem::take(&mut p.ended).then(|| p.opened.is_some_and(|t| t.elapsed() < Duration::from_secs(2)))) {
            // Songs that end the moment they start can't be played: skip a
            // few, then stop rather than racing through the whole folder.
            failures = if quick { failures + 1 } else { 0 };
            if failures >= with(|p| p.songs.len()).clamp(1, 5) {
                failures = 0;
                stop();
                engine.message("The music player couldn't play those files (Windows: Media Player features; elsewhere: install mpv or ffmpeg).");
                report(engine);
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            let repeat = engine.with_config(|c, _| c.repeat);
            let next = if repeat { with(|p| p.current) } else { next_index(engine) };
            match next {
                Some(i) => play(engine, i),
                None => stop(),
            }
        }
        report(engine);
        std::thread::sleep(Duration::from_millis(100));
    }
    shutdown();
}

/// The Windows helper's lines: `PL␉state␉positionMs␉durationMs`.
fn read_helper(out: std::process::ChildStdout) {
    let mut was_playing = false;
    for line in BufReader::new(out).lines() {
        let Ok(line) = line else { break };
        let f: Vec<&str> = line.trim().split('\t').collect();
        if f.len() < 4 || f[0] != "PL" {
            continue;
        }
        let state: i32 = f[1].parse().unwrap_or(0);
        let pos: u64 = f[2].parse::<f64>().unwrap_or(0.0).max(0.0) as u64;
        let dur: u64 = f[3].parse::<f64>().unwrap_or(0.0).max(0.0) as u64;
        with(|p| {
            if p.current.is_none() {
                return;
            }
            // Just after an open, reports may still be about the song before.
            if p.opened.is_some_and(|t| t.elapsed() < Duration::from_millis(1500)) {
                if state == 3 && pos < 3000 {
                    p.base_ms = pos;
                    p.since = Some(Instant::now());
                    was_playing = true;
                }
                return;
            }
            // WMP: 3 playing, 2 paused, 1 stopped, 8 ended, 9/10 opening.
            match state {
                3 => {
                    p.base_ms = pos;
                    p.since = Some(Instant::now());
                    was_playing = true;
                }
                8 => {
                    if was_playing && p.playing {
                        p.ended = true;
                    }
                    was_playing = false;
                }
                1 if was_playing && p.playing => {
                    p.ended = true;
                    was_playing = false;
                }
                _ => {}
            }
            if dur > 0 {
                p.duration_ms = dur;
            }
            p.reported = Some(Instant::now());
        });
    }
}

/// The playing song as a source sample.
fn report(engine: &Engine) {
    let sample = with(|p| {
        let song = p.current.and_then(|i| p.songs.get(i))?;
        Some(Sample {
            title: song.title.clone(),
            artist: song.artist.clone(),
            album: song.album.clone(),
            duration_ms: p.duration_ms,
            position_ms: position(p),
            playing: p.playing,
            art_url: None,
            art_bytes: p.art.clone(),
            app: "LyricsPlus Player".into(),
            lyrics: p.lyrics.clone(),
            at: Instant::now(),
        })
    });
    engine.report(Origin::Local, sample);
}

/// Windows Media Player's engine, driven by the command file.
const WINDOWS_PLAYER: &str = r#"param([string]$Cmd, [int]$ParentPid = 0)
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$w = New-Object -ComObject WMPlayer.OCX
$w.settings.autoStart = $true
$w.settings.volume = 80
$last = 0
$n = 0
while ($true) {
    $n++
    if ($ParentPid -gt 0 -and ($n % 25) -eq 0 -and -not (Get-Process -Id $ParentPid -ErrorAction SilentlyContinue)) { $w.controls.stop(); exit 0 }
    try {
        if (Test-Path -LiteralPath $Cmd) {
            $lines = [System.IO.File]::ReadAllLines($Cmd, [System.Text.Encoding]::UTF8)
            foreach ($line in $lines) {
                $p = $line.Split("`t")
                if ($p.Count -lt 3) { continue }
                $seq = [long]$p[0]
                if ($seq -le $last) { continue }
                $last = $seq
                $arg = $p[2]
                switch ($p[1]) {
                    'open' { $w.URL = $arg; $w.controls.play() }
                    'play' { $w.controls.play() }
                    'pause' { $w.controls.pause() }
                    'stop' { $w.controls.stop() }
                    'seek' { $w.controls.currentPosition = [double]$arg / 1000.0 }
                    'volume' { $w.settings.volume = [int]$arg }
                }
            }
        }
        $state = [int]$w.playState
        $pos = [long]($w.controls.currentPosition * 1000)
        $dur = 0
        if ($w.currentMedia) { $dur = [long]($w.currentMedia.duration * 1000) }
        [Console]::Out.WriteLine("PL`t$state`t$pos`t$dur")
        [Console]::Out.Flush()
    } catch {}
    Start-Sleep -Milliseconds 120
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(id: &[u8], data: &[u8]) -> Vec<u8> {
        let mut f = id.to_vec();
        f.extend_from_slice(&(data.len() as u32).to_be_bytes());
        f.extend_from_slice(&[0, 0]);
        f.extend_from_slice(data);
        f
    }

    #[test]
    fn reads_id3v23_text_and_pictures() {
        let mut body = Vec::new();
        body.extend(frame(b"TIT2", b"\x03Song \xc3\xa9\x00"));
        let utf16: Vec<u8> = [0xFF, 0xFE].into_iter().chain("Band".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        body.extend(frame(b"TPE1", &[&[1u8][..], &utf16].concat()));
        body.extend(frame(b"TALB", b"\x00Album"));
        let mut apic = b"\x00image/jpeg\x00\x03cover\x00".to_vec();
        apic.extend(std::iter::repeat_n(7u8, 100));
        body.extend(frame(b"APIC", &apic));
        body.extend([0u8; 16]);
        let t = parse_id3(3, 0, &body, true);
        assert_eq!((t.title.as_str(), t.artist.as_str(), t.album.as_str()), ("Song é", "Band", "Album"));
        assert_eq!(t.picture.unwrap().len(), 100);
        assert_eq!(synchsafe(&[0, 0, 2, 1]), 257);
    }

    #[test]
    fn names_come_from_files_without_tags() {
        let dir = std::env::temp_dir().join("lp-music-test");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("Some Artist - Some Song.mp3");
        std::fs::write(&p, b"not an mp3").unwrap();
        std::fs::write(dir.join("Some Artist - Some Song.lrc"), "[00:01.00]hi").unwrap();
        let s = song(&p);
        assert_eq!((s.artist.as_str(), s.title.as_str(), s.lrc), ("Some Artist", "Some Song", true));
        let mut found = Vec::new();
        walk(&dir, 2, &mut found);
        assert!(found.contains(&p));
    }
}
