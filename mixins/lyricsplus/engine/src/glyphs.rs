//! Smooth TrueType/OpenType text: fonts (the built-in ones and the player's
//! own from the `fonts` folder) rasterized with `ab_glyph` into the mod's
//! image, which the game uploads into its HUD atlas. The cover art sits in the
//! top-left corner of the same image; glyphs are packed in shelves below it.

use ab_glyph::{Font, FontArc, PxScale, ScaleFont};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// The image's side (Horizon's `MOD_IMAGE_SIZE`).
pub const SIZE: u32 = 1024;
/// The cover art's side, at the image's top left.
pub const ART: u32 = 128;
/// White texels right of the cover, for solid quads in the world.
pub const WHITE: [u32; 4] = [ART + 2, 2, ART + 6, 6];
/// Glyphs start below the cover.
const TOP: u32 = ART + 4;
/// Glyphs are rasterized this many pixels high (smooth in the world, sharp in the HUD).
pub const PX: f32 = 64.0;
const PAD: u32 = 2;

/// A font the player can pick.
#[derive(Clone, Debug, PartialEq)]
pub struct FontInfo {
    /// `ttf:playfair`, `ttf:my font`.
    pub id: String,
    /// "Playfair Display".
    pub name: String,
    pub builtin: bool,
}

/// A glyph in the image: its pixels, and where it sits from the pen on the
/// baseline (pixels at [`PX`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub left: f32,
    pub top: f32,
}

/// A glyph placed on a line: its slot (None for a blank such as a space) and pen x.
#[derive(Clone, Copy, Debug)]
pub struct Placed {
    pub slot: Option<Slot>,
    pub x: f32,
    pub ch: char,
}

/// A laid-out run of text, in pixels at [`PX`].
#[derive(Clone, Debug, Default)]
pub struct Line {
    pub glyphs: Vec<Placed>,
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
}

/// The fonts and the image.
pub struct Canvas {
    pub pixels: Vec<u8>,
    fonts: Vec<(FontInfo, FontArc)>,
    cache: HashMap<(usize, char), Option<Slot>>,
    shelf_x: u32,
    shelf_y: u32,
    shelf_h: u32,
    /// Rows changed since the last upload (0: none).
    dirty: u32,
    pub generation: u64,
    /// Lines already laid out (cleared with the glyphs): drawing the same
    /// words frame after frame costs a lookup.
    lines: HashMap<usize, HashMap<String, Arc<Line>>>,
}

impl Default for Canvas {
    fn default() -> Self {
        Self::new()
    }
}

impl Canvas {
    pub fn new() -> Self {
        let mut c = Self {
            pixels: vec![0; (SIZE * SIZE * 4) as usize],
            fonts: Vec::new(),
            cache: HashMap::new(),
            shelf_x: 0,
            shelf_y: TOP,
            shelf_h: 0,
            dirty: SIZE,
            generation: 0,
            lines: HashMap::new(),
        };
        c.white();
        c
    }
    fn white(&mut self) {
        for y in 0..8 {
            for x in ART..ART + 8 {
                let i = ((y * SIZE + x) * 4) as usize;
                self.pixels[i..i + 4].copy_from_slice(&[255; 4]);
            }
        }
        self.dirty = self.dirty.max(8);
    }
    /// Loads the built-in fonts and every `.ttf`/`.otf`/`.ttc` in `dir`
    /// (the player's, which replace built-ins of the same name).
    pub fn load_fonts(&mut self, builtin: &[(&str, &str, &'static [u8])], dir: &Path) -> Vec<String> {
        let mut problems = Vec::new();
        self.fonts.clear();
        for (id, name, bytes) in builtin {
            if let Ok(f) = FontArc::try_from_slice(bytes) {
                self.fonts.push((FontInfo { id: (*id).into(), name: (*name).into(), builtin: true }, f));
            }
        }
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .map(|d| d.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        files.sort();
        for path in files.into_iter().take(200) {
            let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            if !matches!(ext.as_str(), "ttf" | "otf" | "ttc") {
                continue;
            }
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            match std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| {
                FontArc::try_from_vec(b).map_err(|e| e.to_string())
            }) {
                Ok(f) => {
                    let id = format!("ttf:{}", stem.to_lowercase());
                    let info = FontInfo { id: id.clone(), name: stem.replace(['_', '-'], " "), builtin: false };
                    self.fonts.retain(|(i, _)| i.id != id);
                    self.fonts.push((info, f));
                }
                Err(e) => problems.push(format!("{}: {e}", path.display())),
            }
        }
        self.clear_glyphs();
        problems
    }
    pub fn fonts(&self) -> Vec<FontInfo> {
        self.fonts.iter().map(|(i, _)| i.clone()).collect()
    }
    /// The index of font `id`.
    pub fn find(&self, id: &str) -> Option<usize> {
        self.fonts.iter().position(|(i, _)| i.id == id)
    }
    /// Empties the glyph shelves (full, or the fonts changed).
    fn clear_glyphs(&mut self) {
        let from = (TOP * SIZE * 4) as usize;
        self.pixels[from..].fill(0);
        self.cache.clear();
        self.shelf_x = 0;
        self.shelf_y = TOP;
        self.shelf_h = 0;
        self.dirty = SIZE;
        self.generation += 1;
        self.lines.clear();
    }
    /// The cover art (`ART` x `ART` RGBA) into its corner.
    pub fn set_art(&mut self, rgba: &[u8]) {
        if rgba.len() < (ART * ART * 4) as usize {
            return;
        }
        for y in 0..ART as usize {
            let from = y * ART as usize * 4;
            let to = y * SIZE as usize * 4;
            self.pixels[to..to + ART as usize * 4].copy_from_slice(&rgba[from..from + ART as usize * 4]);
        }
        self.dirty = self.dirty.max(ART);
    }
    /// The rows to upload, once: Some(height) when anything changed.
    pub fn take_dirty(&mut self) -> Option<u32> {
        let d = std::mem::take(&mut self.dirty);
        (d > 0).then_some(d)
    }
    /// Marks everything for upload again (the game's atlas was rebuilt).
    pub fn invalidate(&mut self) {
        self.dirty = SIZE;
    }

    fn rasterize(&mut self, font: usize, ch: char) -> Option<Slot> {
        let (_, f) = self.fonts.get(font)?;
        let f = f.clone();
        let scaled = f.as_scaled(PxScale::from(PX));
        let id = f.glyph_id(ch);
        if id.0 == 0 && !ch.is_whitespace() {
            return None;
        }
        let glyph = id.with_scale_and_position(PxScale::from(PX), ab_glyph::point(0.0, scaled.ascent()));
        let outlined = f.outline_glyph(glyph)?;
        let b = outlined.px_bounds();
        let (w, h) = (b.width().ceil() as u32, b.height().ceil() as u32);
        if w == 0 || h == 0 || w + 2 * PAD > SIZE || h + 2 * PAD > SIZE - TOP {
            return None;
        }
        if self.shelf_x + w + 2 * PAD > SIZE {
            self.shelf_x = 0;
            self.shelf_y += self.shelf_h;
            self.shelf_h = 0;
        }
        if self.shelf_y + h + 2 * PAD > SIZE {
            // Full: start over; what is still needed comes back as it is drawn.
            self.clear_glyphs();
        }
        let (x0, y0) = (self.shelf_x + PAD, self.shelf_y + PAD);
        let pixels = &mut self.pixels;
        outlined.draw(|gx, gy, c| {
            let (x, y) = (x0 + gx, y0 + gy);
            if x < SIZE && y < SIZE {
                let i = ((y * SIZE + x) * 4) as usize;
                pixels[i..i + 4].copy_from_slice(&[255, 255, 255, (c.clamp(0.0, 1.0) * 255.0) as u8]);
            }
        });
        self.shelf_x += w + 2 * PAD;
        self.shelf_h = self.shelf_h.max(h + 2 * PAD);
        self.dirty = self.dirty.max(y0 + h + PAD);
        Some(Slot {
            x: x0,
            y: y0,
            w,
            h,
            left: b.min.x,
            // From the baseline (the glyph was placed with its baseline at `ascent`).
            top: b.min.y - scaled.ascent(),
        })
    }

    /// `text` in font `font`, laid out with kerning, its glyphs rasterized as needed.
    pub fn layout(&mut self, font: usize, text: &str) -> Arc<Line> {
        if let Some(l) = self.lines.get(&font).and_then(|m| m.get(text)) {
            return l.clone();
        }
        let first = self.layout_once(font, text);
        let line = Arc::new(match first {
            Some(l) => l,
            // The atlas was emptied midway: once more into the fresh shelves
            // (a line too big for the whole atlas keeps what fit).
            None => self.layout_once(font, text).unwrap_or_default(),
        });
        let m = self.lines.entry(font).or_default();
        if m.len() >= 4096 {
            m.clear();
        }
        m.insert(text.to_owned(), line.clone());
        line
    }
    fn layout_once(&mut self, font: usize, text: &str) -> Option<Line> {
        let Some((_, f)) = self.fonts.get(font) else {
            return Some(Line::default());
        };
        let f = f.clone();
        let scaled = f.as_scaled(PxScale::from(PX));
        let mut line = Line { ascent: scaled.ascent(), descent: scaled.descent(), ..Line::default() };
        let mut pen = 0.0f32;
        let mut previous = None;
        let generation = self.generation;
        for ch in text.chars() {
            let id = scaled.glyph_id(ch);
            if let Some(p) = previous {
                pen += scaled.kern(p, id);
            }
            let slot = match self.cache.get(&(font, ch)) {
                Some(s) => *s,
                None => {
                    let s = if ch.is_whitespace() { None } else { self.rasterize(font, ch) };
                    self.cache.insert((font, ch), s);
                    s
                }
            };
            line.glyphs.push(Placed { slot, x: pen, ch });
            pen += scaled.h_advance(id);
            previous = Some(id);
        }
        line.width = pen;
        (self.generation == generation).then_some(line)
    }
    /// The width of `text`, in pixels at [`PX`].
    pub fn width(&mut self, font: usize, text: &str) -> f32 {
        self.layout(font, text).width
    }
    /// The line height (ascent − descent) of `font`, in pixels at [`PX`].
    pub fn line_height(&self, font: usize) -> f32 {
        self.fonts.get(font).map_or(PX, |(_, f)| {
            let s = f.as_scaled(PxScale::from(PX));
            s.ascent() - s.descent()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FONT: &[u8] = include_bytes!("../../data/fonts/poppins.ttf");

    #[test]
    fn lays_out_and_packs_glyphs() {
        let mut c = Canvas::new();
        let dir = std::env::temp_dir().join("lp-no-fonts");
        let problems = c.load_fonts(&[("ttf:poppins", "Poppins", FONT)], &dir);
        assert!(problems.is_empty());
        assert_eq!(c.find("ttf:poppins"), Some(0));
        let _ = c.take_dirty();
        let l = c.layout(0, "Hello, World");
        assert_eq!(l.glyphs.len(), 12);
        assert!(l.width > 200.0 && l.width < 600.0, "{}", l.width);
        assert!(l.glyphs[5].slot.is_some() && l.glyphs[6].slot.is_none());
        let h = l.glyphs[0].slot.unwrap();
        assert!(h.top < 0.0 && h.y >= TOP);
        assert!(c.take_dirty().is_some());
        // Cached: nothing new to upload.
        c.layout(0, "Hello");
        assert!(c.take_dirty().is_none());
        // Every glyph the font has fits; emptying the atlas lays out again.
        let all: String = (0x21u32..0x3000).filter_map(char::from_u32).collect();
        assert!(c.layout(0, &all).width > 0.0);
        c.clear_glyphs();
        assert!(c.layout(0, "Hello").glyphs[0].slot.is_some());
    }
}
