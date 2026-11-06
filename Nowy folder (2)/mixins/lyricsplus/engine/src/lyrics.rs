//! Finding a track's lyrics: what its source supplied, the disk cache, then
//! LRCLIB (`/api/get`, then `/api/search` ranked as the original mod ranked it).

use crate::http::{self, HttpError};
use crate::lrc::{self, Lyrics};
use crate::util::{self, url_encode};
use crate::{LyricsState, Track};
use serde_json::Value;
use std::path::{Path, PathBuf};

const API: &str = "https://lrclib.net/api";
/// A miss is asked again after a day (lyrics get added).
const MISS_TTL: u64 = 24 * 3600;

fn cache_path(dir: &Path, track: &Track) -> PathBuf {
    dir.join("cache")
        .join("lyrics")
        .join(format!("{}.json", util::file_key(&track.key)))
}

/// Drops the cached lyrics of `track`.
pub fn forget(dir: &Path, track: &Track) {
    let _ = std::fs::remove_file(cache_path(dir, track));
}

/// Lyrics from LRC or plain text; None for empty text.
pub fn from_text(text: &str, duration_ms: u64, source: &str) -> Option<Lyrics> {
    let synced = lrc::parse(text);
    if !synced.is_empty() {
        return Some(Lyrics {
            lines: synced,
            synced: true,
            instrumental: false,
            source: source.into(),
        });
    }
    let plain = lrc::spread(text, duration_ms);
    (!plain.is_empty()).then(|| Lyrics {
        lines: plain,
        synced: false,
        instrumental: false,
        source: source.into(),
    })
}

fn instrumental(source: &str) -> Lyrics {
    Lyrics {
        lines: Vec::new(),
        synced: true,
        instrumental: true,
        source: source.into(),
    }
}

/// An LRCLIB record as lyrics.
fn from_record(v: &Value, duration_ms: u64, source: &str) -> Option<Lyrics> {
    if v["instrumental"].as_bool() == Some(true) {
        return Some(instrumental(source));
    }
    let synced = v["syncedLyrics"].as_str().unwrap_or("");
    if let Some(l) = from_text(synced, duration_ms, source).filter(|l| l.synced) {
        return Some(l);
    }
    from_text(v["plainLyrics"].as_str().unwrap_or(""), duration_ms, source)
}

/// The track's lyrics and how the search went.
pub fn find(dir: &Path, track: &Track, supplied: Option<&str>) -> (Lyrics, LyricsState) {
    if let Some(text) = supplied
        && let Some(l) = from_text(text, track.duration_ms, "Bridge")
    {
        return (l, LyricsState::Found);
    }
    let path = cache_path(dir, track);
    if let Some(cached) = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
    {
        if cached["found"].as_bool() == Some(true) {
            if let Some(l) = from_record(&cached, track.duration_ms, "LRCLIB (cached)") {
                let state = if l.instrumental {
                    LyricsState::Instrumental
                } else {
                    LyricsState::Found
                };
                return (l, state);
            }
        } else if util::unix_now().saturating_sub(cached["at"].as_u64().unwrap_or(0)) < MISS_TTL {
            return (Lyrics::default(), LyricsState::NotFound);
        }
    }
    match lrclib(track) {
        Ok(Some(record)) => {
            let found = from_record(&record, track.duration_ms, "LRCLIB");
            let mut keep = serde_json::json!({
                "found": found.is_some(),
                "at": util::unix_now(),
                "syncedLyrics": record["syncedLyrics"],
                "plainLyrics": record["plainLyrics"],
                "instrumental": record["instrumental"],
            });
            if found.is_none() {
                keep["syncedLyrics"] = Value::Null;
            }
            save(&path, &keep);
            match found {
                Some(l) if l.instrumental => (l, LyricsState::Instrumental),
                Some(l) => (l, LyricsState::Found),
                None => (Lyrics::default(), LyricsState::NotFound),
            }
        }
        Ok(None) => {
            save(&path, &serde_json::json!({ "found": false, "at": util::unix_now() }));
            (Lyrics::default(), LyricsState::NotFound)
        }
        Err(_) => (Lyrics::default(), LyricsState::Failed),
    }
}

fn save(path: &Path, v: &Value) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, v.to_string());
}

/// The best LRCLIB record for `track`, None when nothing matches.
fn lrclib(track: &Track) -> Result<Option<Value>, HttpError> {
    let title = util::clean_title(&track.title);
    let artist = util::primary_artist(&track.artist);
    let secs = track.duration_ms / 1000;
    // Exact: title, artist, album and duration (LRCLIB allows ±2 s).
    if secs > 0 {
        let mut url = format!(
            "{API}/get?track_name={}&artist_name={}&duration={secs}",
            url_encode(&title),
            url_encode(&artist)
        );
        if !track.album.is_empty() {
            url.push_str(&format!("&album_name={}", url_encode(&track.album)));
        }
        match http::get_json(&url, &[]) {
            Ok((200, v)) if v.is_object() => {
                if has_lyrics(&v) {
                    return Ok(Some(v));
                }
            }
            Ok(_) | Err(HttpError::Status(404, ..)) => {}
            Err(e @ HttpError::Transport(_)) => return Err(e),
            Err(_) => {}
        }
    }
    // Ranked search: by fields, then free text (web titles often carry the
    // artist: "Artist - Song").
    let mut best: Option<(f64, Value)> = None;
    let queries = [
        format!(
            "{API}/search?track_name={}&artist_name={}",
            url_encode(&title),
            url_encode(&artist)
        ),
        format!("{API}/search?q={}", url_encode(&format!("{artist} {title}"))),
    ];
    for url in queries {
        let Ok((200, Value::Array(results))) = http::get_json(&url, &[]) else {
            continue;
        };
        for r in results {
            let score = rank(&r, &title, &artist, track.duration_ms);
            if score > best.as_ref().map_or(f64::MIN, |b| b.0) {
                best = Some((score, r));
            }
        }
        if best.as_ref().is_some_and(|b| b.0 >= 60.0) {
            break;
        }
    }
    Ok(best.filter(|b| b.0 >= 25.0).map(|b| b.1))
}

fn has_lyrics(v: &Value) -> bool {
    v["instrumental"].as_bool() == Some(true)
        || v["syncedLyrics"].as_str().is_some_and(|s| !s.trim().is_empty())
        || v["plainLyrics"].as_str().is_some_and(|s| !s.trim().is_empty())
}

/// How well a search result fits: synced lyrics, the title, the artist and
/// the duration (the original mod's weights).
fn rank(r: &Value, title: &str, artist: &str, duration_ms: u64) -> f64 {
    if !has_lyrics(r) {
        return f64::MIN;
    }
    let mut score = 0.0;
    if r["syncedLyrics"].as_str().is_some_and(|s| !s.trim().is_empty()) {
        score += 30.0;
    }
    let rt = util::normalize(&util::clean_title(r["trackName"].as_str().unwrap_or("")));
    let ra = util::normalize(r["artistName"].as_str().unwrap_or(""));
    let (t, a) = (util::normalize(title), util::normalize(artist));
    if rt == t {
        score += 40.0;
    } else if !t.is_empty() && (rt.contains(&t) || t.contains(&rt)) {
        score += 20.0;
    } else {
        score -= 30.0;
    }
    if !a.is_empty() && (ra.contains(&a) || a.contains(&ra)) {
        score += 25.0;
    }
    if duration_ms > 0 {
        let d = (r["duration"].as_f64().unwrap_or(0.0) - duration_ms as f64 / 1000.0).abs();
        score += if d <= 2.0 {
            20.0
        } else if d <= 5.0 {
            8.0
        } else if d <= 15.0 {
            -10.0
        } else {
            -40.0
        };
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranking_prefers_the_matching_synced_record() {
        let good = serde_json::json!({
            "trackName": "Song", "artistName": "Band", "duration": 200.0,
            "syncedLyrics": "[00:01.00]hi", "plainLyrics": "hi"
        });
        let other = serde_json::json!({
            "trackName": "Song (Live)", "artistName": "Band", "duration": 260.0,
            "plainLyrics": "hi"
        });
        assert!(rank(&good, "Song", "Band", 200_500) > rank(&other, "Song", "Band", 200_500));
        assert!(rank(&good, "Song", "Band", 200_500) >= 60.0);
        let none = serde_json::json!({ "trackName": "Song" });
        assert_eq!(rank(&none, "Song", "Band", 0), f64::MIN);
    }

    #[test]
    fn records_become_lyrics() {
        let v = serde_json::json!({ "syncedLyrics": "[00:02.00]a\n[00:04.00]b" });
        let l = from_record(&v, 10_000, "x").unwrap();
        assert!(l.synced);
        assert_eq!(l.lines.len(), 2);
        let v = serde_json::json!({ "plainLyrics": "a\nb" });
        assert!(!from_record(&v, 10_000, "x").unwrap().synced);
        let v = serde_json::json!({ "instrumental": true });
        assert!(from_record(&v, 10_000, "x").unwrap().instrumental);
    }
}
