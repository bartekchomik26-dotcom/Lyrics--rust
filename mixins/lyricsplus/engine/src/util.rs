//! Small helpers with no dependencies: SHA-256, base64url, percent-encoding, randomness.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since the Unix epoch.
pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// SHA-256 of `data` (FIPS 180-4).
pub fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());
    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, v) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(v);
        }
    }
    let mut out = [0u8; 32];
    for (i, v) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

/// Lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Base64 with the URL-safe alphabet and no padding (RFC 4648 §5), as PKCE wants.
pub fn base64url(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = match chunk.len() {
            3 => u32::from(chunk[0]) << 16 | u32::from(chunk[1]) << 8 | u32::from(chunk[2]),
            2 => u32::from(chunk[0]) << 16 | u32::from(chunk[1]) << 8,
            _ => u32::from(chunk[0]) << 16,
        };
        let chars = chunk.len() + 1;
        for i in 0..chars {
            out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

/// Percent-encodes everything but RFC 3986's unreserved characters.
pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Decodes `%XX` and `+` (form encoding).
pub fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let h = std::str::from_utf8(&bytes[i + 1..i + 3])
                    .ok()
                    .and_then(|h| u8::from_str_radix(h, 16).ok());
                match h {
                    Some(v) => {
                        out.push(v);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The value of `key` in a query string (`a=1&b=2`).
pub fn query_param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (url_decode(k) == key).then(|| url_decode(v))
    })
}

/// `n` unpredictable bytes. The standard library's hasher keys come from the
/// operating system's random source; mixed with the clock and a counter and
/// run through SHA-256 they are good enough for a PKCE verifier and state.
pub fn random_bytes(n: usize) -> Vec<u8> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut out = Vec::with_capacity(n + 32);
    while out.len() < n {
        let mut seed = Vec::with_capacity(64);
        for _ in 0..4 {
            let mut h = RandomState::new().build_hasher();
            h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            h.write_u128(nanos);
            h.write_usize(&seed as *const _ as usize);
            seed.extend_from_slice(&h.finish().to_le_bytes());
        }
        out.extend_from_slice(&sha256(&seed));
    }
    out.truncate(n);
    out
}

/// Lowercase letters and digits only, words separated by single spaces:
/// what two titles are compared on.
pub fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            if space && !out.is_empty() {
                out.push(' ');
            }
            space = false;
            out.push(c);
        } else {
            space = true;
        }
    }
    out
}

/// A title without its decorations: `(feat. X)`, `[Official Video]`,
/// `- Remastered 2011`, `| Lyrics`… as the original mod's `cleanTitle`.
pub fn clean_title(title: &str) -> String {
    const NOISE: [&str; 16] = [
        "official", "video", "audio", "lyric", "lyrics", "visualizer", "hd", "4k", "feat", "ft.",
        "remaster", "live", "explicit", "clean", "mv", "version",
    ];
    let mut out = String::with_capacity(title.len());
    let mut depth = 0usize;
    let mut group = String::new();
    for c in title.chars() {
        match c {
            '(' | '[' | '{' => {
                if depth == 0 {
                    group.clear();
                }
                depth += 1;
                if depth > 1 {
                    group.push(c);
                }
            }
            ')' | ']' | '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    let lower = group.to_lowercase();
                    if !NOISE.iter().any(|n| lower.contains(n)) {
                        out.push('(');
                        out.push_str(&group);
                        out.push(')');
                    }
                } else {
                    group.push(c);
                }
            }
            _ if depth > 0 => group.push(c),
            _ => out.push(c),
        }
    }
    let mut s = out.trim().to_owned();
    for sep in [" | ", " - ", " – ", " — "] {
        if let Some(at) = s.find(sep) {
            let tail = s[at + sep.len()..].to_lowercase();
            if NOISE.iter().any(|n| tail.contains(n)) || tail.starts_with("remaster") {
                s.truncate(at);
            }
        }
    }
    // ASCII lowering keeps byte offsets, so they index `s` safely.
    let lower = s.to_ascii_lowercase();
    for marker in [" feat. ", " ft. ", " feat "] {
        if let Some(at) = lower.find(marker) {
            s.truncate(at);
            break;
        }
    }
    s.trim().to_owned()
}

/// The first artist of `a, b & c` / `a feat. b` / `a; b`.
pub fn primary_artist(artist: &str) -> String {
    let lower = artist.to_ascii_lowercase();
    let mut cut = artist.len();
    for sep in [", ", " & ", " feat. ", " ft. ", " x ", "; ", " / ", " and "] {
        if let Some(at) = lower.find(sep) {
            cut = cut.min(at);
        }
    }
    artist[..cut].trim().to_owned()
}

/// A safe file name from any text.
pub fn file_key(s: &str) -> String {
    hex(&sha256(s.as_bytes())[..12])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_known_vectors() {
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let long = vec![b'a'; 1000];
        assert_eq!(
            hex(&sha256(&long)),
            "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3"
        );
    }

    #[test]
    fn pkce_challenge_matches_rfc7636() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            base64url(&sha256(verifier.as_bytes())),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn urls_round_trip() {
        assert_eq!(url_encode("a b&c/é"), "a%20b%26c%2F%C3%A9");
        assert_eq!(url_decode("a%20b+c%26"), "a b c&");
        assert_eq!(query_param("code=AB%2FC&state=x", "code").as_deref(), Some("AB/C"));
        assert_eq!(query_param("code=1", "state"), None);
    }

    #[test]
    fn titles_lose_their_decorations() {
        assert_eq!(clean_title("Song (Official Video)"), "Song");
        assert_eq!(clean_title("Song [feat. Someone]"), "Song");
        assert_eq!(clean_title("Song - Remastered 2011"), "Song");
        assert_eq!(clean_title("Song (Acoustic)"), "Song (Acoustic)");
        assert_eq!(clean_title("Song feat. Someone"), "Song");
        assert_eq!(primary_artist("A, B & C"), "A");
        assert_eq!(normalize("Hello,  World!"), "hello world");
        // Lowercasing that changes byte lengths must not split a character.
        assert_eq!(clean_title("İİİ Song feat. X"), "İİİ Song");
        assert_eq!(primary_artist("İstanbul & B"), "İstanbul");
    }

    #[test]
    fn random_bytes_differ() {
        let a = random_bytes(32);
        let b = random_bytes(32);
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
    }
}
