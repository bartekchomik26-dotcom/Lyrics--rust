//! Album art: fetching its bytes, and once the game has decoded them, a
//! square texture and the colours the HUD adapts to.

use crate::Track;
use crate::http;
use crate::util::{self, url_encode};

/// Art bigger than this is not art.
const LIMIT: u64 = 6 * 1024 * 1024;

/// The art's bytes: the source's own URL (Spotify, the bridge, MPRIS),
/// else the iTunes or Deezer cover of the song's best match.
pub fn fetch(track: &Track, url: Option<&str>) -> Option<Vec<u8>> {
    if let Some(url) = url.filter(|u| !u.trim().is_empty())
        && let Some(bytes) = fetch_url(url)
    {
        return Some(bytes);
    }
    let term = format!(
        "{} {}",
        util::primary_artist(&track.artist),
        util::clean_title(&track.title)
    );
    itunes(&term).or_else(|| deezer(&term))
}

fn fetch_url(url: &str) -> Option<Vec<u8>> {
    if let Some(path) = url.strip_prefix("file://") {
        // file:///C:/x.png on Windows, file:///tmp/x.png elsewhere.
        let path = util::url_decode(path);
        let path = if cfg!(windows) {
            path.trim_start_matches('/').to_owned()
        } else {
            path
        };
        return std::fs::read(path).ok().filter(|b| !b.is_empty());
    }
    if url.starts_with("https://") || url.starts_with("http://") {
        return http::get_bytes(url, LIMIT).ok().filter(|b| b.len() > 64);
    }
    None
}

fn itunes(term: &str) -> Option<Vec<u8>> {
    let url = format!(
        "https://itunes.apple.com/search?term={}&entity=song&limit=1",
        url_encode(term)
    );
    let (_, v) = http::get_json(&url, &[]).ok()?;
    let art = v["results"][0]["artworkUrl100"].as_str()?;
    fetch_url(&art.replace("100x100bb", "300x300bb"))
}

fn deezer(term: &str) -> Option<Vec<u8>> {
    let url = format!("https://api.deezer.com/search?q={}&limit=1", url_encode(term));
    let (_, v) = http::get_json(&url, &[]).ok()?;
    let art = v["data"][0]["album"]["cover_medium"].as_str()?;
    fetch_url(art)
}

/// Colours taken from the art (0xRRGGBB).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// The most vivid colour, light enough to read on `background`.
    pub accent: u32,
    /// The commonest colour, darkened: panels behind text.
    pub background: u32,
    /// The commonest colour as it is.
    pub dominant: u32,
    /// Text that reads on `background`.
    pub text: u32,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            accent: 0x1DB954,
            background: 0x121212,
            dominant: 0x282828,
            text: 0xFFFFFF,
        }
    }
}

fn rgb_of(c: [f32; 3]) -> u32 {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    b(c[0]) << 16 | b(c[1]) << 8 | b(c[2])
}
fn unpack(c: u32) -> [f32; 3] {
    [
        ((c >> 16) & 255) as f32 / 255.0,
        ((c >> 8) & 255) as f32 / 255.0,
        (c & 255) as f32 / 255.0,
    ]
}
/// Relative luminance (sRGB weights, no gamma: enough to pick text colour).
pub fn luminance(c: u32) -> f32 {
    let [r, g, b] = unpack(c);
    0.2126 * r + 0.7152 * g + 0.0722 * b
}
pub fn hsv(c: [f32; 3]) -> (f32, f32, f32) {
    let max = c[0].max(c[1]).max(c[2]);
    let min = c[0].min(c[1]).min(c[2]);
    let d = max - min;
    let sector = if d <= f32::EPSILON {
        0.0
    } else if max == c[0] {
        ((c[1] - c[2]) / d).rem_euclid(6.0)
    } else if max == c[1] {
        (c[2] - c[0]) / d + 2.0
    } else {
        (c[0] - c[1]) / d + 4.0
    };
    let h = sector * 60.0;
    let s = if max <= f32::EPSILON { 0.0 } else { d / max };
    (h, s, max)
}
pub fn from_hsv(h: f32, s: f32, v: f32) -> [f32; 3] {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h.rem_euclid(360.0) / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [r + m, g + m, b + m]
}

/// 0xRRGGBB's hue (degrees), saturation and value (0 to 1).
pub fn to_hsv(c: u32) -> (f32, f32, f32) {
    hsv(unpack(c))
}
/// Hue (degrees), saturation and value as 0xRRGGBB.
pub fn hsv_rgb(h: f32, s: f32, v: f32) -> u32 {
    rgb_of(from_hsv(h, s.clamp(0.0, 1.0), v.clamp(0.0, 1.0)))
}

/// Mixes `a` toward `b` by `t` (0 is `a`).
pub fn mix(a: u32, b: u32, t: f32) -> u32 {
    let (a, b) = (unpack(a), unpack(b));
    let t = t.clamp(0.0, 1.0);
    rgb_of([
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ])
}

/// The palette of an RGBA image.
pub fn palette(width: u32, height: u32, rgba: &[u8]) -> Palette {
    // 4 bits a channel: 4096 buckets of summed colour and count.
    let mut sums = vec![[0f32; 4]; 4096];
    let n = (width as usize) * (height as usize);
    let step = (n / 6000).max(1);
    for px in rgba.chunks_exact(4).step_by(step) {
        if px[3] < 128 {
            continue;
        }
        let i = (px[0] as usize >> 4) << 8 | (px[1] as usize >> 4) << 4 | px[2] as usize >> 4;
        let s = &mut sums[i];
        s[0] += px[0] as f32 / 255.0;
        s[1] += px[1] as f32 / 255.0;
        s[2] += px[2] as f32 / 255.0;
        s[3] += 1.0;
    }
    let buckets: Vec<([f32; 3], f32)> = sums
        .iter()
        .filter(|s| s[3] > 0.0)
        .map(|s| ([s[0] / s[3], s[1] / s[3], s[2] / s[3]], s[3]))
        .collect();
    if buckets.is_empty() {
        return Palette::default();
    }
    let dominant = buckets
        .iter()
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|b| b.0)
        .unwrap_or([0.1; 3]);
    // Vivid: saturated, not too dark, and common enough to belong.
    let vivid = buckets
        .iter()
        .map(|(c, count)| {
            let (_, s, v) = hsv(*c);
            let bright = if v < 0.25 { v * 2.0 } else { 1.0 - (v - 0.85).max(0.0) };
            (c, count.sqrt() * s * s * bright)
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .filter(|(_, score)| *score > 0.05)
        .map(|(c, _)| *c);
    let (dh, ds, _) = hsv(dominant);
    let background = from_hsv(dh, (ds * 0.8).min(0.6), 0.16);
    let accent = match vivid {
        Some(c) => {
            let (h, s, v) = hsv(c);
            // Light enough to read on the dark panel.
            from_hsv(h, s.clamp(0.45, 0.9), v.max(0.75))
        }
        None => {
            // A grey cover: a light tint of its own colour.
            from_hsv(dh, ds.min(0.25), 0.85)
        }
    };
    let background = rgb_of(background);
    Palette {
        accent: rgb_of(accent),
        background,
        dominant: rgb_of(dominant),
        text: if luminance(background) > 0.55 { 0x101010 } else { 0xFFFFFF },
    }
}

/// `rgba` (`width` x `height`) cropped to its centre square and resampled
/// to `size` x `size` (area average down, bilinear up).
pub fn square(width: u32, height: u32, rgba: &[u8], size: u32) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let side = w.min(h).max(1);
    let (ox, oy) = ((w - side.min(w)) / 2, (h - side.min(h)) / 2);
    let size_u = size as usize;
    let mut out = vec![0u8; size_u * size_u * 4];
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return out;
    }
    let scale = side as f32 / size as f32;
    for y in 0..size_u {
        for x in 0..size_u {
            let mut acc = [0f32; 4];
            if scale >= 1.0 {
                let x0 = ox + (x as f32 * scale) as usize;
                let y0 = oy + (y as f32 * scale) as usize;
                let x1 = (ox + ((x + 1) as f32 * scale) as usize).max(x0 + 1).min(ox + side);
                let y1 = (oy + ((y + 1) as f32 * scale) as usize).max(y0 + 1).min(oy + side);
                let mut count = 0f32;
                for sy in y0..y1 {
                    for sx in x0..x1 {
                        let i = (sy * w + sx) * 4;
                        for (c, a) in acc.iter_mut().enumerate() {
                            *a += rgba[i + c] as f32;
                        }
                        count += 1.0;
                    }
                }
                for a in &mut acc {
                    *a /= count.max(1.0);
                }
            } else {
                let fx = (x as f32 + 0.5) * scale - 0.5;
                let fy = (y as f32 + 0.5) * scale - 0.5;
                let (x0, y0) = (fx.floor().max(0.0) as usize, fy.floor().max(0.0) as usize);
                let (x1, y1) = ((x0 + 1).min(side - 1), (y0 + 1).min(side - 1));
                let (tx, ty) = ((fx - x0 as f32).clamp(0.0, 1.0), (fy - y0 as f32).clamp(0.0, 1.0));
                let at = |sx: usize, sy: usize, c: usize| rgba[((oy + sy) * w + ox + sx) * 4 + c] as f32;
                for (c, a) in acc.iter_mut().enumerate() {
                    let top = at(x0, y0, c) * (1.0 - tx) + at(x1, y0, c) * tx;
                    let bottom = at(x0, y1, c) * (1.0 - tx) + at(x1, y1, c) * tx;
                    *a = top * (1.0 - ty) + bottom * ty;
                }
            }
            let o = (y * size_u + x) * 4;
            for c in 0..4 {
                out[o + c] = acc[c].round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// A placeholder cover: a diagonal gradient of the accent colour.
pub fn placeholder(size: u32, accent: u32) -> Vec<u8> {
    let size = size as usize;
    let dark = mix(accent, 0x101010, 0.75);
    let light = mix(accent, 0x101010, 0.25);
    let mut out = Vec::with_capacity(size * size * 4);
    for y in 0..size {
        for x in 0..size {
            let t = (x + y) as f32 / (2 * size.max(1)) as f32;
            let c = mix(light, dark, t);
            out.extend_from_slice(&[(c >> 16) as u8, (c >> 8) as u8, c as u8, 255]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_finds_the_vivid_colour_and_a_dark_panel() {
        // Mostly dark grey with a red patch.
        let mut img = Vec::new();
        for i in 0..100 * 100 {
            if i % 10 < 3 {
                img.extend_from_slice(&[220, 30, 40, 255]);
            } else {
                img.extend_from_slice(&[40, 40, 44, 255]);
            }
        }
        let p = palette(100, 100, &img);
        let [r, g, _] = unpack(p.accent);
        assert!(r > 0.7 && g < 0.4, "accent {:06x}", p.accent);
        assert!(luminance(p.background) < 0.25);
        assert_eq!(p.text, 0xFFFFFF);
    }

    #[test]
    fn hsv_round_trips() {
        for c in [0xFF0000, 0x00FF00, 0x4FC3F7, 0x808080] {
            let (h, s, v) = to_hsv(c);
            let back = hsv_rgb(h, s, v);
            let d = |a: u32, b: u32, sh: u32| ((a >> sh) & 255).abs_diff((b >> sh) & 255);
            assert!(d(c, back, 16) <= 1 && d(c, back, 8) <= 1 && d(c, back, 0) <= 1, "{c:06x} {back:06x}");
        }
    }

    #[test]
    fn squares_crop_and_resample() {
        let img: Vec<u8> = (0..4 * 2).flat_map(|i| [i as u8 * 10, 0, 0, 255]).collect();
        let out = square(4, 2, &img, 2);
        assert_eq!(out.len(), 16);
        let up = square(4, 2, &img, 8);
        assert_eq!(up.len(), 8 * 8 * 4);
        assert_eq!(placeholder(4, 0x1DB954).len(), 64);
    }
}
