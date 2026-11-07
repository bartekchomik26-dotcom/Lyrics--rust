//! A local HTTP bridge on `127.0.0.1` (port 8974, the original mod's), so
//! any player can drive the lyrics: the bundled userscript for web players
//! (SoundCloud, YouTube Music, Deezer, Tidal, Apple Music, Spotify's web
//! player, Bandcamp…), a desktop player's plugin, a script.
//!
//! * `POST /update` `{title, artist, album?, durationMs?, positionMs?,
//!   isPlaying?, artUrl?, source?, lyrics?}`: what is playing (the original
//!   bridge's fields, plus `source` and LRC or plain `lyrics`). The answer
//!   carries a pending control: `{"success": true, "command": "next"}`.
//! * `POST /lyrics` `{title, lyrics}`: lyrics for the track playing.
//! * `POST /clear`: nothing plays any more.
//! * `GET /now`: the track and the line showing, for overlays (OBS) and apps.
//! * `GET /status`: `{"status": "ok", "mod": "LyricsPlus"}`.
//!
//! Only loopback connections are served, and only small bodies read.

use crate::{Control, Engine, Origin, Sample};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

const MAX_BODY: usize = 512 * 1024;

pub fn run(engine: &'static Engine) {
    let mut bound: Option<(u16, TcpListener)> = None;
    let mut warned = false;
    while !engine.stopped() {
        let (on, port) = engine.with_config(|c, _| (c.bridge, c.bridge_port));
        if !on {
            bound = None;
            std::thread::sleep(Duration::from_secs(1));
            continue;
        }
        if bound.as_ref().map(|b| b.0) != Some(port) {
            match TcpListener::bind(("127.0.0.1", port)) {
                Ok(l) => {
                    let _ = l.set_nonblocking(true);
                    bound = Some((port, l));
                    warned = false;
                }
                Err(e) => {
                    if !warned {
                        engine.message(format!("LyricsPlus bridge: port {port} is taken ({e}); retrying."));
                        warned = true;
                    }
                    bound = None;
                    std::thread::sleep(Duration::from_secs(5));
                    continue;
                }
            }
        }
        let Some((_, listener)) = &bound else { continue };
        match listener.accept() {
            Ok((stream, addr)) => {
                if addr.ip().is_loopback() {
                    let _ = handle(engine, stream, port);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(250)),
        }
    }
}

fn respond(stream: &mut TcpStream, status: &str, body: &str) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\n\
         Content-Type: application/json\r\n\
         Access-Control-Allow-Origin: *\r\n\
         Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
         Access-Control-Allow-Headers: Content-Type\r\n\
         Access-Control-Allow-Private-Network: true\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    )
}

fn handle(engine: &Engine, mut stream: TcpStream, port: u16) -> std::io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request = String::new();
    reader.read_line(&mut request)?;
    let mut parts = request.split_whitespace();
    let method = parts.next().unwrap_or("").to_owned();
    let path = parts.next().unwrap_or("").split('?').next().unwrap_or("").to_owned();
    let mut length = 0usize;
    let mut origin: Option<String> = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            break;
        }
        let header = header.trim();
        if header.is_empty() {
            break;
        }
        if let Some((k, v)) = header.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                length = v.trim().parse().unwrap_or(0);
            } else if k.trim().eq_ignore_ascii_case("origin") {
                origin = Some(v.trim().to_ascii_lowercase());
            }
        }
    }
    if length > MAX_BODY {
        return respond(&mut stream, "413 Payload Too Large", r#"{"success":false}"#);
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body)?;
    let json = || serde_json::from_slice::<serde_json::Value>(&body).unwrap_or_default();
    // Only players, the userscript and local apps may change what shows: a web
    // page from anywhere else can't post to the bridge from the browser.
    if method == "POST" && !trusted(origin.as_deref()) {
        return respond(&mut stream, "403 Forbidden", r#"{"success":false}"#);
    }
    match (method.as_str(), path.as_str()) {
        ("OPTIONS", _) => respond(&mut stream, "204 No Content", ""),
        ("GET", "/status") | ("GET", "/") => respond(
            &mut stream,
            "200 OK",
            &serde_json::json!({
                "status": "ok", "mod": "LyricsPlus", "platform": "horizon",
                "version": env!("CARGO_PKG_VERSION"),
            })
            .to_string(),
        ),
        ("GET", "/now") => respond(&mut stream, "200 OK", &engine.now_json().to_string()),
        ("GET", "/lyricsplus.user.js") => {
            // Userscript managers offer to install a `.user.js` they are sent.
            let body = crate::USERSCRIPT.replace("__PORT__", &port.to_string());
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/javascript; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
        }
        ("POST", "/update") => {
            let v = json();
            let sample = sample(&v);
            let lyrics = sample.as_ref().and_then(|s| s.lyrics.clone());
            let title = sample.as_ref().map(|s| s.title.clone());
            engine.report(Origin::Bridge, sample);
            if let (Some(t), Some(l)) = (title, lyrics) {
                engine.supply_lyrics(&t, &l);
            }
            let command = engine.take_bridge_command().map(|c| match c {
                Control::PlayPause => "toggle",
                Control::Next => "next",
                Control::Previous => "previous",
            });
            respond(
                &mut stream,
                "200 OK",
                &serde_json::json!({ "success": true, "command": command }).to_string(),
            )
        }
        ("POST", "/lyrics") => {
            let v = json();
            if let (Some(t), Some(l)) = (v["title"].as_str(), v["lyrics"].as_str()) {
                engine.supply_lyrics(t, l);
            }
            respond(&mut stream, "200 OK", r#"{"success":true}"#)
        }
        ("POST", "/clear") => {
            engine.report(Origin::Bridge, None);
            respond(&mut stream, "200 OK", r#"{"success":true}"#)
        }
        _ => respond(&mut stream, "404 Not Found", r#"{"success":false}"#),
    }
}

/// Music sites the userscript covers, whose pages may post to the bridge.
const SITES: [&str; 11] = [
    "soundcloud.com",
    "music.youtube.com",
    "www.youtube.com",
    "open.spotify.com",
    "www.deezer.com",
    "listen.tidal.com",
    "music.apple.com",
    "bandcamp.com",
    "music.amazon.com",
    "www.mixcloud.com",
    "audiomack.com",
];

/// Whether a request's `Origin` may post: none (apps, OBS, userscript managers'
/// own requests), a browser extension, or one of the music sites.
fn trusted(origin: Option<&str>) -> bool {
    let Some(o) = origin else { return true };
    if ["chrome-extension://", "moz-extension://", "safari-web-extension://"].iter().any(|p| o.starts_with(p)) {
        return true;
    }
    let host = o.strip_prefix("https://").or_else(|| o.strip_prefix("http://")).unwrap_or("");
    let host = host.split(':').next().unwrap_or("");
    SITES.iter().any(|s| host == *s || host.ends_with(&format!(".{s}")))
}

/// An `/update` body as a sample (None when nothing plays).
fn sample(v: &serde_json::Value) -> Option<Sample> {
    let title = v["title"].as_str()?.trim().to_owned();
    if title.is_empty() {
        return None;
    }
    let ms = |k: &str| v[k].as_f64().unwrap_or(0.0).max(0.0) as u64;
    let text = |k: &str| v[k].as_str().unwrap_or("").trim().to_owned();
    Some(Sample {
        title,
        artist: text("artist"),
        album: text("album"),
        duration_ms: ms("durationMs"),
        position_ms: ms("positionMs"),
        playing: v["isPlaying"].as_bool().unwrap_or(true),
        art_url: Some(text("artUrl")).filter(|s| !s.is_empty()),
        art_bytes: None,
        app: Some(text("source")).filter(|s| !s.is_empty()).unwrap_or_else(|| "Browser".into()),
        lyrics: Some(text("lyrics")).filter(|s| !s.is_empty()),
        at: Instant::now(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_music_sites_post() {
        assert!(trusted(None));
        assert!(trusted(Some("https://soundcloud.com")));
        assert!(trusted(Some("https://artist.bandcamp.com")));
        assert!(trusted(Some("chrome-extension://abc")));
        assert!(!trusted(Some("https://evil.example")));
        assert!(!trusted(Some("https://soundcloud.com.evil.example")));
        assert!(!trusted(Some("null")));
    }

    #[test]
    fn reads_the_original_bridge_fields() {
        let v = serde_json::json!({
            "title": "Song", "artist": "Band", "durationMs": 1000.0, "positionMs": 10,
            "isPlaying": false, "artUrl": "", "source": "SoundCloud"
        });
        let s = sample(&v).unwrap();
        assert_eq!(s.app, "SoundCloud");
        assert!(!s.playing);
        assert!(s.art_url.is_none());
        assert!(sample(&serde_json::json!({ "title": "" })).is_none());
    }
}
