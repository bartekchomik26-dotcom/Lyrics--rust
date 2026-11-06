//! Lyrics in time: LRC parsing and lookups by playback position.

/// One line, from `time_ms` until the next line starts. An empty `text` is
/// an instrumental gap (LRC's blank timed lines).
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub time_ms: u64,
    pub text: String,
    /// Word timings for enhanced LRC (`<mm:ss.xx>word`): each word's start.
    pub words: Vec<(u64, String)>,
}

/// A song's lyrics. `synced` is false when only plain lyrics were found and
/// their times are spread evenly over the song.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lyrics {
    pub lines: Vec<Line>,
    pub synced: bool,
    pub instrumental: bool,
    /// Where they came from: `LRCLIB`, `Bridge`, `Cache`…
    pub source: String,
}

impl Lyrics {
    /// The line playing at `pos_ms`: the last that started at or before it.
    pub fn index_at(&self, pos_ms: u64) -> Option<usize> {
        if self.lines.is_empty() || pos_ms < self.lines[0].time_ms {
            return None;
        }
        let i = self.lines.partition_point(|l| l.time_ms <= pos_ms);
        Some(i.saturating_sub(1))
    }
    /// When line `i` ends: the next line's start, else `duration_ms`
    /// (or five seconds on).
    pub fn end_of(&self, i: usize, duration_ms: u64) -> u64 {
        match self.lines.get(i + 1) {
            Some(next) => next.time_ms,
            None => {
                let start = self.lines.get(i).map_or(0, |l| l.time_ms);
                if duration_ms > start {
                    duration_ms
                } else {
                    start + 5000
                }
            }
        }
    }
    /// How far through line `i` playback is, 0 to 1.
    pub fn progress(&self, i: usize, pos_ms: u64, duration_ms: u64) -> f32 {
        let Some(line) = self.lines.get(i) else {
            return 0.0;
        };
        let end = self.end_of(i, duration_ms);
        // A line rarely sings until the next starts: a long gap is mostly silence.
        let span = (end.saturating_sub(line.time_ms)).clamp(1, 8000);
        (pos_ms.saturating_sub(line.time_ms) as f32 / span as f32).clamp(0.0, 1.0)
    }
    /// Line `i`'s words and when each is sung: the enhanced LRC timings,
    /// else spread over the line by their letters (a line is sung in about
    /// 110 ms a letter, never longer than until the next line).
    pub fn words(&self, i: usize, duration_ms: u64) -> Vec<(u64, String)> {
        let Some(line) = self.lines.get(i) else {
            return Vec::new();
        };
        if !line.words.is_empty() {
            return line.words.iter().map(|(t, w)| (*t, w.trim().to_owned())).collect();
        }
        let words: Vec<&str> = line.text.split_whitespace().collect();
        let letters: usize = words.iter().map(|w| w.chars().count()).sum();
        let gap = self.end_of(i, duration_ms).saturating_sub(line.time_ms).max(1);
        let span = gap.min(letters as u64 * 110 + 400).max(250);
        let mut before = 0usize;
        words
            .iter()
            .map(|w| {
                let t = line.time_ms + span * before as u64 / letters.max(1) as u64;
                before += w.chars().count();
                (t, (*w).to_owned())
            })
            .collect()
    }
    /// The word of line `i` being sung at `pos_ms` (its index), with how far through it is.
    pub fn word_at(&self, i: usize, pos_ms: u64, duration_ms: u64) -> Option<(usize, f32)> {
        let words = self.words(i, duration_ms);
        if words.is_empty() {
            return None;
        }
        let k = words.partition_point(|(t, _)| *t <= pos_ms).saturating_sub(1);
        let start = words[k].0;
        let end = words.get(k + 1).map_or_else(|| self.end_of(i, duration_ms).min(start + 1200), |w| w.0);
        let p = (pos_ms.saturating_sub(start) as f32 / end.saturating_sub(start).max(1) as f32).clamp(0.0, 1.0);
        Some((k, p))
    }
    /// Time until the first line, or until the next line after an empty one:
    /// the instrumental countdown's milliseconds, when the gap is long.
    pub fn gap_remaining(&self, pos_ms: u64) -> Option<(u64, u64)> {
        match self.index_at(pos_ms) {
            None => {
                let first = self.lines.first()?.time_ms;
                (first > 6000).then(|| (first - pos_ms, first))
            }
            Some(i) if self.lines[i].text.trim().is_empty() => {
                let next = self.lines.get(i + 1)?.time_ms;
                let total = next.saturating_sub(self.lines[i].time_ms);
                (total > 6000).then(|| (next.saturating_sub(pos_ms), total))
            }
            _ => None,
        }
    }
}

/// A `[mm:ss.xx]` tag's milliseconds (also `[m:ss]`, `[mm:ss:xx]`, `[mm:ss.xxx]`).
fn timestamp(tag: &str) -> Option<u64> {
    let (min, rest) = tag.split_once(':')?;
    let min: u64 = min.trim().parse().ok()?;
    let (sec, frac) = match rest.find(['.', ':']) {
        Some(at) => (&rest[..at], &rest[at + 1..]),
        None => (rest, ""),
    };
    let sec: u64 = sec.trim().parse().ok()?;
    if sec >= 60 && !frac.is_empty() {
        return None;
    }
    let frac = frac.trim();
    if !frac.is_ascii() {
        return None;
    }
    let ms = match frac.len() {
        0 => 0,
        1 => frac.parse::<u64>().ok()? * 100,
        2 => frac.parse::<u64>().ok()? * 10,
        _ => frac[..3].parse::<u64>().ok()?,
    };
    Some(min * 60_000 + sec * 1000 + ms)
}

/// Parses LRC text. Lines with several time tags repeat; `[offset:±ms]`
/// shifts everything (positive is earlier, as the format says); metadata
/// tags are skipped; enhanced word tags give word timings.
pub fn parse(text: &str) -> Vec<Line> {
    let mut offset: i64 = 0;
    let mut out = Vec::new();
    for raw in text.lines() {
        let mut rest = raw.trim();
        let mut times = Vec::new();
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(end) = stripped.find(']') else {
                break;
            };
            let tag = &stripped[..end];
            if let Some(t) = timestamp(tag) {
                times.push(t);
            } else if let Some(v) = tag.strip_prefix("offset:") {
                offset = v.trim().parse().unwrap_or(0);
            }
            rest = stripped[end + 1..].trim_start();
        }
        if times.is_empty() {
            continue;
        }
        let (plain, words) = words(rest);
        for t in times {
            let shift = |t: u64| (t as i64 - offset).max(0) as u64;
            out.push(Line {
                time_ms: shift(t),
                text: plain.clone(),
                words: words.iter().map(|(wt, w)| (shift(*wt), w.clone())).collect(),
            });
        }
    }
    out.sort_by_key(|l| l.time_ms);
    out
}

/// Strips enhanced LRC word tags, returning the text and the timed words.
fn words(line: &str) -> (String, Vec<(u64, String)>) {
    if !line.contains('<') {
        return (line.trim().to_owned(), Vec::new());
    }
    let mut plain = String::new();
    let mut words = Vec::new();
    let mut current: Option<(u64, String)> = None;
    let mut rest = line;
    while !rest.is_empty() {
        if let Some(stripped) = rest.strip_prefix('<')
            && let Some(end) = stripped.find('>')
            && let Some(t) = timestamp(&stripped[..end])
        {
            if let Some(w) = current.take()
                && !w.1.trim().is_empty()
            {
                words.push(w);
            }
            current = Some((t, String::new()));
            rest = &stripped[end + 1..];
            continue;
        }
        let c = rest.chars().next().unwrap_or(' ');
        plain.push(c);
        if let Some(w) = current.as_mut() {
            w.1.push(c);
        }
        rest = &rest[c.len_utf8()..];
    }
    if let Some(w) = current
        && !w.1.trim().is_empty()
    {
        words.push(w);
    }
    (plain.split_whitespace().collect::<Vec<_>>().join(" "), words)
}

/// Plain (unsynced) lyrics spread over the song: an intro of 8% then one
/// line every equal share, blank lines kept as short gaps.
pub fn spread(plain: &str, duration_ms: u64) -> Vec<Line> {
    let lines: Vec<&str> = plain.lines().map(str::trim).collect();
    let count = lines.iter().filter(|l| !l.is_empty()).count();
    if count == 0 {
        return Vec::new();
    }
    let duration = if duration_ms > 10_000 { duration_ms } else { 180_000 };
    let start = duration * 8 / 100;
    let step = (duration - start) * 90 / 100 / count as u64;
    let mut t = start;
    let mut out = Vec::new();
    for l in lines {
        if l.is_empty() {
            continue;
        }
        out.push(Line {
            time_ms: t,
            text: l.to_owned(),
            words: Vec::new(),
        });
        t += step;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tags_offsets_and_repeats() {
        let lrc = "[ti:Song]\n[offset:+500]\n[00:01.00]one\n[00:03.50][00:10.00]two\n[00:05.123]\n";
        let lines = parse(lrc);
        let got: Vec<(u64, &str)> = lines.iter().map(|l| (l.time_ms, l.text.as_str())).collect();
        assert_eq!(
            got,
            [(500, "one"), (3000, "two"), (4623, ""), (9500, "two")]
        );
    }

    #[test]
    fn odd_tags_do_not_panic() {
        assert!(parse("[00:01.é€x]a\n[ar:Ö]\n[xx]b").is_empty());
    }

    #[test]
    fn enhanced_words_are_timed() {
        let lines = parse("[00:01.00]<00:01.00>Hello <00:01.50>world");
        assert_eq!(lines[0].text, "Hello world");
        assert_eq!(lines[0].words.len(), 2);
        assert_eq!(lines[0].words[1].0, 1500);
    }

    #[test]
    fn looks_up_by_position() {
        let lyrics = Lyrics {
            lines: parse("[00:10.00]a\n[00:20.00]\n[00:40.00]b"),
            synced: true,
            ..Default::default()
        };
        assert_eq!(lyrics.index_at(5_000), None);
        assert_eq!(lyrics.index_at(10_000), Some(0));
        assert_eq!(lyrics.index_at(39_999), Some(1));
        assert_eq!(lyrics.index_at(99_000), Some(2));
        assert_eq!(lyrics.gap_remaining(5_000), Some((5_000, 10_000)));
        assert_eq!(lyrics.gap_remaining(30_000), Some((10_000, 20_000)));
        assert_eq!(lyrics.gap_remaining(12_000), None);
        assert!((lyrics.progress(0, 14_000, 60_000) - 0.5).abs() < 0.01);
    }

    #[test]
    fn words_are_timed_through_the_line() {
        let lyrics = Lyrics { lines: parse("[00:10.00]one two three\n[00:20.00]x"), synced: true, ..Default::default() };
        let w = lyrics.words(0, 60_000);
        assert_eq!(w.len(), 3);
        assert_eq!(w[0].0, 10_000);
        assert!(w[1].0 > 10_000 && w[2].0 > w[1].0 && w[2].0 < 20_000);
        assert_eq!(lyrics.word_at(0, 10_050, 60_000).unwrap().0, 0);
        assert_eq!(lyrics.word_at(0, 19_000, 60_000).unwrap().0, 2);
    }

    #[test]
    fn spreads_plain_lyrics() {
        let lines = spread("a\n\nb\nc", 100_000);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].time_ms, 8_000);
        assert!(lines[1].time_ms > lines[0].time_ms);
    }
}
