//! The Spotify Web API with the player's own app (Client ID from
//! developer.spotify.com): Authorization Code with PKCE, so no client secret
//! is kept. The redirect URI to register on the app is [`REDIRECT`].
//!
//! Spotify's public API has no lyrics: it gives the exact track, position and
//! controls; the lyrics come from LRCLIB (or the bridge) like any source's.

use crate::http::{self, HttpError};
use crate::util::{self, url_encode};
use crate::{Config, Control, Engine, Origin, Sample, Source, SpotifyStatus};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A Client ID compiled into the mod, so players can press Connect with no
/// setup: create one app at developer.spotify.com (redirect URI [`REDIRECT`]),
/// put its Client ID here and rebuild. Empty: each player sets their own.
/// PKCE needs no secret, so a Client ID is safe to ship. Spotify keeps a new
/// app in development mode (its owner adds up to 25 users) until it is
/// granted extended quota.
pub const BUILT_IN_CLIENT_ID: &str = "";

/// The Client ID in use: the player's own, else the built-in one.
pub fn client_id(config: &Config) -> String {
    if config.spotify_client_id.is_empty() {
        BUILT_IN_CLIENT_ID.to_owned()
    } else {
        config.spotify_client_id.clone()
    }
}

pub const PORT: u16 = 8975;
pub const REDIRECT: &str = "http://127.0.0.1:8975/callback";
const SCOPES: &str = "user-read-currently-playing user-read-playback-state user-modify-playback-state";
const ACCOUNTS: &str = "https://accounts.spotify.com";
const API: &str = "https://api.spotify.com/v1";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Tokens {
    client_id: String,
    refresh_token: String,
    access_token: String,
    /// Unix seconds.
    expires_at: u64,
    user: String,
}

static TOKENS: Mutex<Option<Tokens>> = Mutex::new(None);

fn token_path(dir: &Path) -> PathBuf {
    dir.join("spotify.json")
}

fn load(dir: &Path) -> Option<Tokens> {
    let mut slot = TOKENS.lock().unwrap_or_else(|p| p.into_inner());
    if slot.is_none() {
        *slot = std::fs::read_to_string(token_path(dir))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok());
    }
    slot.clone()
}

fn store(dir: &Path, t: Option<Tokens>) {
    match &t {
        Some(t) => {
            if let Ok(text) = serde_json::to_string_pretty(t) {
                let _ = std::fs::create_dir_all(dir);
                let _ = std::fs::write(token_path(dir), text);
            }
        }
        None => {
            let _ = std::fs::remove_file(token_path(dir));
        }
    }
    *TOKENS.lock().unwrap_or_else(|p| p.into_inner()) = t;
}

/// Forgets the link.
pub fn forget(dir: &Path) {
    store(dir, None);
}

pub fn initial_status(dir: &Path, config: &Config) -> SpotifyStatus {
    let id = client_id(config);
    if id.is_empty() {
        return SpotifyStatus::NoClientId;
    }
    match load(dir) {
        Some(t) if !t.refresh_token.is_empty() && t.client_id == id => {
            SpotifyStatus::Linked(if t.user.is_empty() {
                "Spotify".into()
            } else {
                t.user
            })
        }
        _ => SpotifyStatus::NotLinked,
    }
}

/// Opens `url` in the default browser.
pub fn open_browser(url: &str) -> bool {
    use std::process::{Command, Stdio};
    let mut cmd = if cfg!(windows) {
        // `start` would split the URL at each `&`; the URL handler does not.
        let mut c = Command::new("rundll32.exe");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(url);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
}

/// Starts linking: opens the consent page and waits for its redirect.
pub fn connect(engine: &'static Engine) -> Result<String, String> {
    let (client_id, dir) = engine.with_config(|c, d| (client_id(c), d.clone()));
    if client_id.is_empty() {
        return Err("Set your Spotify Client ID first (developer.spotify.com > Dashboard > your app).".into());
    }
    let listener = TcpListener::bind(("127.0.0.1", PORT))
        .map_err(|e| format!("Can't listen on 127.0.0.1:{PORT} for Spotify's redirect: {e}"))?;
    let verifier = util::base64url(&util::random_bytes(48));
    let challenge = util::base64url(&util::sha256(verifier.as_bytes()));
    let state = util::base64url(&util::random_bytes(16));
    let url = format!(
        "{ACCOUNTS}/authorize?client_id={}&response_type=code&redirect_uri={}&code_challenge_method=S256&code_challenge={challenge}&state={state}&scope={}",
        url_encode(&client_id),
        url_encode(REDIRECT),
        url_encode(SCOPES)
    );
    engine.set_spotify(SpotifyStatus::Linking(url.clone()));
    let opened = open_browser(&url);
    if !opened {
        engine.message("Couldn't open a browser: paste the copied Spotify link into one.");
    }
    let _ = std::thread::Builder::new()
        .name("lyricsplus-spotify-link".into())
        .spawn(move || {
            let result = wait_for_code(engine, &listener, &state)
                .and_then(|code| exchange(&client_id, &code, &verifier));
            match result {
                Ok(mut t) => {
                    t.user = me(&t.access_token).unwrap_or_default();
                    let name = if t.user.is_empty() { "Spotify".to_owned() } else { t.user.clone() };
                    store(&dir, Some(t));
                    engine.set_spotify(SpotifyStatus::Linked(name.clone()));
                    engine.message(format!("Spotify linked ({name})."));
                }
                Err(e) => {
                    engine.set_spotify(SpotifyStatus::Error(e.clone()));
                    engine.message(format!("Spotify link failed: {e}"));
                }
            }
        });
    Ok(url)
}

fn wait_for_code(engine: &Engine, listener: &TcpListener, state: &str) -> Result<String, String> {
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(300);
    while Instant::now() < deadline && !engine.stopped() {
        match listener.accept() {
            Ok((stream, _)) => {
                if let Some(result) = callback(stream, state) {
                    return result;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("timed out waiting for the browser (5 minutes)".into())
}

/// One request to the redirect listener: the code, an error, or None for
/// something else (a favicon).
fn callback(mut stream: TcpStream, state: &str) -> Option<Result<String, String>> {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line).ok()?;
    let target = line.split_whitespace().nth(1)?.to_owned();
    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    if path != "/callback" {
        let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return None;
    }
    let result = if let Some(err) = util::query_param(query, "error") {
        Err(format!("Spotify said: {err}"))
    } else if util::query_param(query, "state").as_deref() != Some(state) {
        Err("the answer's state didn't match (try again)".into())
    } else {
        util::query_param(query, "code").ok_or_else(|| "no code in Spotify's answer".to_owned())
    };
    let (title, text) = match &result {
        Ok(_) => ("Linked", "LyricsPlus is linked to Spotify. You can close this tab and go back to the game."),
        Err(_) => ("Not linked", "LyricsPlus couldn't link Spotify. Go back to the game and try again."),
    };
    let body = format!(
        "<!doctype html><meta charset=utf-8><title>LyricsPlus – {title}</title>\
         <body style=\"background:#121212;color:#fff;font:16px system-ui;display:grid;place-items:center;height:90vh\">\
         <div style=\"text-align:center\"><h1 style=\"color:#1db954\">♪ LyricsPlus</h1><p>{text}</p></div>"
    );
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    Some(result)
}

fn tokens_from(v: &serde_json::Value, client_id: &str, old_refresh: &str) -> Result<Tokens, String> {
    let access = v["access_token"].as_str().ok_or("no access token in Spotify's answer")?;
    Ok(Tokens {
        client_id: client_id.to_owned(),
        refresh_token: v["refresh_token"].as_str().unwrap_or(old_refresh).to_owned(),
        access_token: access.to_owned(),
        expires_at: util::unix_now() + v["expires_in"].as_u64().unwrap_or(3600),
        user: String::new(),
    })
}

fn exchange(client_id: &str, code: &str, verifier: &str) -> Result<Tokens, String> {
    let v = http::post_form(
        &format!("{ACCOUNTS}/api/token"),
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", REDIRECT),
            ("client_id", client_id),
            ("code_verifier", verifier),
        ],
    )
    .map_err(|e| e.to_string())?;
    tokens_from(&v, client_id, "")
}

fn me(token: &str) -> Option<String> {
    let auth = format!("Bearer {token}");
    let (_, v) = http::get_json(&format!("{API}/me"), &[("Authorization", &auth)]).ok()?;
    v["display_name"]
        .as_str()
        .or(v["id"].as_str())
        .map(str::to_owned)
}

/// A current access token, refreshed when it is about to expire.
fn access(engine: &Engine) -> Option<String> {
    let (client_id, dir) = engine.with_config(|c, d| (client_id(c), d.clone()));
    let t = load(&dir)?;
    if t.refresh_token.is_empty() || t.client_id != client_id {
        return None;
    }
    if !t.access_token.is_empty() && t.expires_at > util::unix_now() + 60 {
        return Some(t.access_token);
    }
    match http::post_form(
        &format!("{ACCOUNTS}/api/token"),
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &t.refresh_token),
            ("client_id", &client_id),
        ],
    ) {
        Ok(v) => {
            let mut fresh = tokens_from(&v, &client_id, &t.refresh_token).ok()?;
            fresh.user = t.user;
            let token = fresh.access_token.clone();
            store(&dir, Some(fresh));
            Some(token)
        }
        Err(HttpError::Status(400, ..)) | Err(HttpError::Status(401, ..)) => {
            // Revoked or rotated away: the player links again.
            forget(&dir);
            engine.set_spotify(SpotifyStatus::Error("the link expired: connect again".into()));
            engine.message("Spotify's link expired or was revoked: connect again from the LyricsPlus menu.");
            None
        }
        Err(_) => None,
    }
}

fn invalidate(dir: &Path) {
    if let Some(mut t) = load(dir) {
        t.expires_at = 0;
        store(dir, Some(t));
    }
}

/// The best image of `images` for a ~300 px cover.
fn image(images: &serde_json::Value) -> Option<String> {
    images
        .as_array()?
        .iter()
        .filter_map(|i| Some((i["url"].as_str()?, i["width"].as_i64().unwrap_or(300))))
        .min_by_key(|(_, w)| (w - 300).abs())
        .map(|(u, _)| u.to_owned())
}

/// A `currently-playing` answer as a sample.
fn sample(v: &serde_json::Value, at: Instant) -> Option<Sample> {
    let item = &v["item"];
    let title = item["name"].as_str()?.to_owned();
    let episode = item["type"].as_str() == Some("episode");
    let artist = if episode {
        item["show"]["name"].as_str().unwrap_or("").to_owned()
    } else {
        item["artists"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x["name"].as_str()).collect::<Vec<_>>().join(", "))
            .unwrap_or_default()
    };
    let album = item["album"]["name"].as_str().unwrap_or("").to_owned();
    let art = image(&item["album"]["images"]).or_else(|| image(&item["images"]));
    Some(Sample {
        title,
        artist,
        album,
        duration_ms: item["duration_ms"].as_u64().unwrap_or(0),
        position_ms: v["progress_ms"].as_u64().unwrap_or(0),
        playing: v["is_playing"].as_bool().unwrap_or(false),
        art_url: art,
        art_bytes: None,
        app: "Spotify".into(),
        lyrics: None,
        at,
    })
}

/// Polls `currently-playing` while linked.
pub fn run(engine: &'static Engine) {
    let mut wait = Duration::from_millis(500);
    while !engine.stopped() {
        std::thread::sleep(wait);
        let (wanted, poll, dir) = engine.with_config(|c, d| {
            (
                matches!(c.source, Source::Auto | Source::Spotify) && !client_id(c).is_empty(),
                c.spotify_poll_ms,
                d.clone(),
            )
        });
        if !wanted {
            engine.report(Origin::Spotify, None);
            wait = Duration::from_secs(1);
            continue;
        }
        let Some(token) = access(engine) else {
            wait = Duration::from_secs(2);
            continue;
        };
        let auth = format!("Bearer {token}");
        let sent = Instant::now();
        match http::get_json(
            &format!("{API}/me/player/currently-playing?additional_types=episode"),
            &[("Authorization", &auth)],
        ) {
            Ok((200, v)) => {
                let rtt = sent.elapsed();
                let at = Instant::now().checked_sub(rtt / 2).unwrap_or_else(Instant::now);
                let s = sample(&v, at);
                let playing = s.as_ref().is_some_and(|s| s.playing);
                engine.report(Origin::Spotify, s);
                wait = Duration::from_millis(if playing { poll as u64 } else { 2500 });
            }
            Ok(_) => {
                // 204: nothing playing.
                engine.report(Origin::Spotify, None);
                wait = Duration::from_millis(3000);
            }
            Err(HttpError::Status(401, ..)) => {
                invalidate(&dir);
                wait = Duration::from_millis(200);
            }
            Err(HttpError::Status(429, retry, _)) => {
                wait = Duration::from_secs(retry.unwrap_or(5).clamp(1, 120));
            }
            Err(_) => wait = Duration::from_secs(3),
        }
    }
}

/// A player control through the API (Premium), else the system's media keys.
pub fn control(engine: &'static Engine, c: Control, playing: bool) {
    let Some(token) = access(engine) else {
        crate::system::control(c);
        return;
    };
    let (method, path) = match c {
        Control::Next => ("POST", "next"),
        Control::Previous => ("POST", "previous"),
        Control::PlayPause if playing => ("PUT", "pause"),
        Control::PlayPause => ("PUT", "play"),
    };
    match http::send_empty(method, &format!("{API}/me/player/{path}"), &token) {
        Ok(_) => {}
        Err(HttpError::Status(403, ..)) | Err(HttpError::Status(404, ..)) => {
            // Free accounts can't control playback through the API.
            crate::system::control(c);
        }
        Err(e) => engine.message(format!("Spotify control failed: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_currently_playing_answer() {
        let v = serde_json::json!({
            "is_playing": true, "progress_ms": 61000,
            "item": {
                "type": "track", "name": "Song", "duration_ms": 200000,
                "artists": [{"name": "A"}, {"name": "B"}],
                "album": {"name": "Album", "images": [
                    {"url": "big", "width": 640}, {"url": "mid", "width": 300}, {"url": "small", "width": 64}
                ]}
            }
        });
        let s = sample(&v, Instant::now()).unwrap();
        assert_eq!(s.artist, "A, B");
        assert_eq!(s.art_url.as_deref(), Some("mid"));
        assert_eq!(s.position_ms, 61000);
        assert!(s.playing);
        assert!(sample(&serde_json::json!({"item": null}), Instant::now()).is_none());
    }
}
