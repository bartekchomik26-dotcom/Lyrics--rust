// LyricsPlus for Horizon: the game side. Injected as `crate::game::lyricsplus`,
// so it reads the game's own fields and helpers (`native_hooks.rs`); all
// media, networking and settings are in the `lyricsplus-engine` crate. The
// hook helpers next to this file call the `pub(crate)` functions at the end.

use super::{Game, Screen};
use horizon_client::mesher::Vertex;
use lyricsplus_engine::glyphs;
use horizon_client::extras::Corner as HCorner;
use horizon_client::extras_hud::Corners;
use horizon_client::hud::{HudBatch, rgba};
use horizon_client::mod_api::{NativeMod, NativeScreenView};
use horizon_client::text::{Style, Text};
use lyricsplus_engine as engine;
use lyricsplus_engine::art::{self, Palette};
use lyricsplus_engine::{
    Align, Anchor, Animation, Arrangement, Motion, Vanish, Billboard, Config, Control, Corner, Display, Highlight, LyricsPosition,
    LyricsState, Snapshot, Source, SpotifyStatus, ToastCorner, WidgetStyle,
};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub(crate) const ID: &str = "lyricsplus";
pub(crate) const NAME: &str = "LyricsPlus";
pub(crate) const VERSION: &str = "3.0.0+horizon.1";
const DESCRIPTION: &str = "Real-time synced lyrics and a Spotify-style now-playing HUD, in 2D or floating in the world (elytra mode too). Spotify Web API, the system's media session, or any web player through the browser bridge. Press K for the menu.";

/// The album art's side in the mod image, and the white texels beside it
/// that world-space panels are drawn with.
const ART: u32 = glyphs::ART;

const KEY_MENU: (&str, &str, &str) = ("open_menu", "Open the LyricsPlus menu", "key.keyboard.k");
const KEY_TOGGLE: (&str, &str, &str) = ("toggle_hud", "Show / hide lyrics and player", "key.keyboard.j");
const KEY_PLAY: (&str, &str, &str) = ("play_pause", "Media play / pause", "key.keyboard.keypad.5");
const KEY_NEXT: (&str, &str, &str) = ("next_track", "Next track", "key.keyboard.keypad.6");
const KEY_PREV: (&str, &str, &str) = ("previous_track", "Previous track", "key.keyboard.keypad.4");
const KEY_PVP: (&str, &str, &str) = ("pvp_mode", "PvP lyrics mode (follow + split)", "key.keyboard.keypad.0");
const KEYS: [(&str, &str, &str); 6] = [KEY_MENU, KEY_TOGGLE, KEY_PLAY, KEY_NEXT, KEY_PREV, KEY_PVP];

const SPOTIFY_DASHBOARD: &str = "https://developer.spotify.com/dashboard";
const SPOTIFY_REDIRECT: &str = "http://127.0.0.1:8975/callback";

// ---------------------------------------------------------------- state

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Field {
    ClientId,
    LyricsFont,
    TitleFont,
    Accent,
    Active,
    Offset,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Action {
    Done,
    Layout,
    SpotifyConnect,
    SpotifyDisconnect,
    SpotifyDashboard,
    CopyRedirect,
    InstallUserscript,
    CopyBridgeUrl,
    ReloadLyrics,
    OffsetMinus,
    OffsetPlus,
    Previous,
    PlayPause,
    Next,
    Reset,
    OpenFolder,
    OpenFonts,
    ReloadFonts,
    MusicPrevious,
    MusicToggle,
    MusicNext,
    MusicStop,
    OpenMusic,
    Rescan,
}

enum Pending {
    ReloadFonts,
    OpenScreen(u32),
    CloseScreen,
    Paste,
    Copy(String),
    Chat(String),
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Drag {
    Slider(usize),
    Wheel(usize),
    Lyrics([f32; 2]),
    Widget([f32; 2]),
}

struct Menu {
    tab: usize,
    scroll: f32,
    focus: Option<Field>,
    buffer: String,
    drag: Option<Drag>,
    layout: bool,
    size: [f32; 2],
    content_height: f32,
}

/// A line flown past in elytra mode, left where it was.
struct Passed {
    text: String,
    at: [f64; 3],
    since: Instant,
    yaw: f32,
}

/// Sparks in the world, from a word of a line as it was placed.
#[derive(Clone, Copy)]
struct WBurst {
    since: Instant,
    at: [f64; 3],
    yaw: f32,
    pitch: Option<f32>,
    px: f32,
    /// Where in the line (font pixels from its centre).
    origin: [f32; 2],
    seed: u64,
}

/// A cover decoded off the render thread: (serial, square RGBA, palette).
type Decoded = (u64, Vec<u8>, Palette);
static DECODED: Mutex<Option<Decoded>> = Mutex::new(None);

struct Ui {
    cfg: Config,
    cfg_version: u64,
    hidden: bool,
    started: Instant,
    last_frame: Instant,
    dt: f32,
    // art
    art_serial: u64,
    art_ready: bool,
    art_tried: u64,
    image_revision: Option<u64>,
    palette: Palette,
    accent: [f32; 3],
    panel: [f32; 3],
    // lyric animations (2D)
    line: Option<usize>,
    line_serial: u64,
    line_changed: Instant,
    prev_line: Option<usize>,
    previous_text: Option<String>,
    // toast
    toast_serial: u64,
    toast_start: Option<Instant>,
    combat: f32,
    // where the HUD drew, for the layout editor
    lyrics_rect: [f32; 4],
    widget_rect: [f32; 4],
    // 3D
    anchor: Option<[f64; 3]>,
    world_line: Option<usize>,
    world_serial: u64,
    world_changed: Instant,
    world_previous: Option<String>,
    passed: Vec<Passed>,
    flying: bool,
    fly_dir: [f64; 3],
    spawn_yaw: f32,
    world_frame: Instant,
    seed: u64,
    // the island
    island_w: f32,
    island_hl: [f32; 2],
    island_line: Option<usize>,
    island_serial: u64,
    island_changed: Instant,
    island_burst: Option<(Instant, [f32; 4], u64)>,
    island_fired: Option<(u64, Option<usize>)>,
    island_last: Option<[f32; 4]>,
    // 2D lyric sparks: from the last word, as it's sung (or as the line ends)
    hud_burst: Option<(Instant, [f32; 4], u64)>,
    hud_fired: Option<(u64, Option<usize>)>,
    hud_last: Option<[f32; 4]>,
    // 3D sparks
    wbursts: Vec<WBurst>,
    world_fired: Option<(u64, Option<usize>)>,
    world_last: Option<WBurst>,
    // tight spaces: distance and size factors, how much the aura shows
    fit_d: f32,
    fit_s: f32,
    aura: f32,
    // PvP
    pvp: bool,
    last_attack: Option<Instant>,
    // the fonts whose ASCII glyphs are in the atlas already
    warmed: Vec<String>,
    last_upload: Option<Instant>,
    // menu
    menu: Menu,
    pending: Vec<Pending>,
    /// Every font the menu offers: (id, name).
    fonts: Vec<(String, String)>,
    /// The offline player's library, refreshed while the Music tab shows.
    songs: Vec<engine::player::Song>,
    songs_at: Option<Instant>,
}

static UI: Mutex<Option<Ui>> = Mutex::new(None);
/// The mod image: the cover and the TrueType glyphs (locked after UI, never before).
static CANVAS: Mutex<Option<glyphs::Canvas>> = Mutex::new(None);

fn with_canvas<R>(f: impl FnOnce(&mut glyphs::Canvas) -> R) -> Option<R> {
    let mut slot = CANVAS.lock().unwrap_or_else(|p| p.into_inner());
    slot.as_mut().map(f)
}

/// The fonts built into the mod (SIL Open Font License, see data/fonts).
const BUILTIN_FONTS: [(&str, &str, &[u8]); 10] = [
    ("ttf:playfair", "Playfair Display", horizon_client::native_data!("lyricsplus", "fonts/playfair.ttf")),
    ("ttf:cinzel", "Cinzel", horizon_client::native_data!("lyricsplus", "fonts/cinzel.ttf")),
    ("ttf:greatvibes", "Great Vibes", horizon_client::native_data!("lyricsplus", "fonts/greatvibes.ttf")),
    ("ttf:dancingscript", "Dancing Script", horizon_client::native_data!("lyricsplus", "fonts/dancingscript.ttf")),
    ("ttf:lora", "Lora", horizon_client::native_data!("lyricsplus", "fonts/lora.ttf")),
    ("ttf:poppins", "Poppins", horizon_client::native_data!("lyricsplus", "fonts/poppins.ttf")),
    ("ttf:quicksand", "Quicksand", horizon_client::native_data!("lyricsplus", "fonts/quicksand.ttf")),
    ("ttf:bebasneue", "Bebas Neue", horizon_client::native_data!("lyricsplus", "fonts/bebasneue.ttf")),
    ("ttf:pacifico", "Pacifico", horizon_client::native_data!("lyricsplus", "fonts/pacifico.ttf")),
    ("ttf:caveat", "Caveat", horizon_client::native_data!("lyricsplus", "fonts/caveat.ttf")),
];

/// The player's font folder (`horizon-mods/.horizon-mod-data/lyricsplus/fonts`).
fn fonts_dir(e: &engine::Engine) -> std::path::PathBuf {
    e.data_dir().join("fonts")
}

/// (Re)loads the fonts into the canvas and lists them for the menu.
fn load_fonts(game: &Game<'_>, e: &engine::Engine) {
    let dir = fonts_dir(e);
    let _ = std::fs::create_dir_all(&dir);
    let readme = dir.join("PUT YOUR FONTS HERE.txt");
    if !readme.exists() {
        let _ = std::fs::write(&readme, "Drop .ttf or .otf font files in this folder, then press \"Reload fonts\" in the LyricsPlus menu (Fonts tab).\nEach file shows up in the font list under its file name.\n");
    }
    let builtin: Vec<(&str, &str, &'static [u8])> = BUILTIN_FONTS.iter().map(|(i, n, b)| (*i, *n, *b)).collect();
    let mut canvas = CANVAS.lock().unwrap_or_else(|p| p.into_inner());
    let c = canvas.get_or_insert_with(glyphs::Canvas::new);
    let problems = c.load_fonts(&builtin, &dir);
    let ttf: Vec<(String, String)> = c.fonts().into_iter().map(|f| (f.id, f.name)).collect();
    drop(canvas);
    for p in problems {
        e.message(format!("Couldn't load font {p}"));
    }
    let mut list: Vec<(String, String)> = ttf;
    for id in scan_fonts(game) {
        let name = font_label(&id);
        list.push((id, name));
    }
    with_ui(|ui| {
        ui.fonts = list;
        ui.warmed.clear();
    });
}

fn with_ui<R>(f: impl FnOnce(&mut Ui) -> R) -> R {
    let mut slot = UI.lock().unwrap_or_else(|p| p.into_inner());
    let ui = slot.get_or_insert_with(|| {
        let now = Instant::now();
        Ui {
            cfg: Config::default(),
            cfg_version: 0,
            hidden: false,
            started: now,
            last_frame: now,
            dt: 0.016,
            art_serial: 0,
            art_ready: false,
            art_tried: 0,
            image_revision: None,
            palette: Palette::default(),
            accent: unpack(0x1DB954),
            panel: unpack(0x121212),
            line: None,
            line_serial: 0,
            line_changed: now,
            prev_line: None,
            previous_text: None,
            toast_serial: 0,
            toast_start: None,
            combat: 0.0,
            lyrics_rect: [0.0; 4],
            widget_rect: [0.0; 4],
            anchor: None,
            world_line: None,
            world_serial: 0,
            world_changed: now,
            world_previous: None,
            passed: Vec::new(),
            flying: false,
            fly_dir: [0.0, 0.0, 1.0],
            spawn_yaw: 0.0,
            world_frame: now,
            island_w: 0.0,
            island_hl: [0.0, 0.0],
            island_line: None,
            island_serial: 0,
            island_changed: now,
            island_burst: None,
            island_fired: None,
            island_last: None,
            hud_burst: None,
            hud_fired: None,
            hud_last: None,
            wbursts: Vec::new(),
            world_fired: None,
            world_last: None,
            fit_d: 1.0,
            fit_s: 1.0,
            aura: 0.0,
            pvp: false,
            last_attack: None,
            warmed: Vec::new(),
            last_upload: None,
            seed: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(7, |d| d.as_nanos() as u64) | 1,
            menu: Menu {
                tab: 0,
                scroll: 0.0,
                focus: None,
                buffer: String::new(),
                drag: None,
                layout: false,
                size: [320.0, 240.0],
                content_height: 0.0,
            },
            pending: Vec::new(),
            fonts: Vec::new(),
            songs: Vec::new(),
            songs_at: None,
        }
    });
    f(ui)
}

fn eng(game: &Game<'_>) -> &'static engine::Engine {
    engine::engine(|| game.native_data_dir(ID))
}

impl Ui {
    fn refresh(&mut self, e: &engine::Engine) {
        let v = e.config_version();
        if v != self.cfg_version && self.menu.drag.is_none() {
            self.cfg = e.config();
            self.cfg_version = v;
        }
    }
    fn accent(&self) -> u32 {
        pack_rgb(self.accent)
    }
    fn panel(&self) -> u32 {
        pack_rgb(self.panel)
    }
    /// Eases the shown colours toward the art's (or the settings').
    fn ease_colors(&mut self) {
        let (accent, panel) = if self.cfg.adaptive_colors && self.art_ready {
            (self.palette.accent, self.palette.background)
        } else {
            (self.cfg.accent, 0x121212)
        };
        let t = 1.0 - (-self.dt * 6.0).exp();
        let (a, p) = (unpack(accent), unpack(panel));
        for i in 0..3 {
            self.accent[i] += (a[i] - self.accent[i]) * t;
            self.panel[i] += (p[i] - self.panel[i]) * t;
        }
    }
    /// PvP mode now: switched on, or automatic and fighting in the last few seconds.
    fn pvp_now(&self) -> bool {
        self.cfg.pvp_mode || (self.cfg.pvp_auto && self.last_attack.is_some_and(|t| t.elapsed() < Duration::from_secs(6)))
    }
    /// How words are placed now, and how far apart (PvP splits them).
    fn arrangement(&self) -> (Arrangement, f32) {
        if self.pvp_now() {
            (Arrangement::Split, self.cfg.scatter.max(0.45))
        } else {
            (self.cfg.arrangement, self.cfg.scatter)
        }
    }
    /// The lyrics' colour: with adaptive colours, a light tint of the cover's
    /// vivid colour (eased between songs), else the chosen one.
    fn active_color(&self) -> u32 {
        if self.cfg.adaptive_colors && self.art_ready {
            art::mix(self.accent(), 0xFFFFFF, 0.35)
        } else {
            self.cfg.active_color
        }
    }
}

// ---------------------------------------------------------------- small helpers

fn unpack(c: u32) -> [f32; 3] {
    [
        ((c >> 16) & 255) as f32 / 255.0,
        ((c >> 8) & 255) as f32 / 255.0,
        (c & 255) as f32 / 255.0,
    ]
}
fn pack_rgb(c: [f32; 3]) -> u32 {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    b(c[0]) << 16 | b(c[1]) << 8 | b(c[2])
}
/// 0xRRGGBB with an alpha of 0..1 as 0xAARRGGBB.
fn argb(rgb: u32, alpha: f32) -> u32 {
    ((alpha.clamp(0.0, 1.0) * 255.0).round() as u32) << 24 | (rgb & 0xFF_FFFF)
}
/// 0xAARRGGBB as the HUD's packed colour.
fn packed(c: u32) -> u32 {
    rgba((c >> 16) as u8, (c >> 8) as u8, c as u8, (c >> 24) as u8)
}
fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t) * (1.0 - t) * (1.0 - t)
}
fn ease_back(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let c = 1.70158;
    1.0 + (c + 1.0) * (t - 1.0).powi(3) + c * (t - 1.0).powi(2)
}
fn time(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}
fn secs(since: Instant) -> f32 {
    since.elapsed().as_secs_f32()
}

fn style(font: &str, bold: bool, italic: bool) -> Style {
    Style {
        font: (font != "minecraft:default" && !font.is_empty()).then(|| font.to_owned()),
        bold: Some(bold),
        italic: Some(italic),
        ..Style::default()
    }
}
fn text(s: &str, st: &Style) -> Text {
    Text::styled(s.to_owned(), st.clone())
}
fn plain(s: &str) -> Text {
    Text::literal(s.to_owned())
}
/// A TrueType font id (`ttf:…`) the canvas has, with the plain text.
fn ttf(t: &Text) -> Option<(&str, String)> {
    let id = t.spans.first()?.style.font.as_deref()?;
    id.starts_with("ttf:").then(|| (id, t.plain()))
}
/// GUI pixels a TrueType font's line spans (Minecraft's font: 9, caps 7).
const TTF_LINE: f32 = 11.0;

fn width(g: &Game<'_>, t: &Text) -> f32 {
    if let Some((id, s)) = ttf(t)
        && let Some(w) = with_canvas(|c| c.find(id).map(|f| c.width(f, &s) * TTF_LINE / c.line_height(f))).flatten()
    {
        return w;
    }
    g.gui.atlas.font.width_text(&mc_only(t))
}
/// `t` without a TrueType font (which the game's font doesn't know).
fn mc_only(t: &Text) -> Text {
    if ttf(t).is_none() {
        return t.clone();
    }
    let mut t = t.clone();
    for span in &mut t.spans {
        span.style.font = None;
    }
    t
}
/// Draws TrueType text from the canvas's glyphs; None when the font isn't loaded.
#[allow(clippy::too_many_arguments)]
fn draw_ttf(g: &Game<'_>, b: &mut HudBatch, id: &str, s: &str, x: f32, y: f32, scale: f32, color: u32, shadow: bool) -> Option<f32> {
    let sprite = g.gui.atlas.sprites.get(horizon_client::hud::MOD_IMAGE_SPRITE).copied()?;
    let (line, unit) = with_canvas(|c| {
        let f = c.find(id)?;
        let unit = TTF_LINE / c.line_height(f) * scale;
        Some((c.layout(f, s), unit))
    })??;
    let du = (sprite.u1 - sprite.u0) / sprite.w.max(1) as f32;
    let dv = (sprite.v1 - sprite.v0) / sprite.h.max(1) as f32;
    // The baseline where Minecraft's would be (its 7-pixel caps on a 9-pixel line).
    let baseline = y + 8.0 * scale - (line.ascent + line.descent) * unit * 0.12;
    let passes: &[(f32, u32)] = if shadow {
        &[(1.0, 0x3F), (0.0, 0xFF)]
    } else {
        &[(0.0, 0xFF)]
    };
    for &(off, shade) in passes {
        let c = if shade == 0xFF {
            color
        } else {
            let [a, r, gg, bb] = color.to_be_bytes();
            u32::from_be_bytes([a, (r as u32 * shade / 255) as u8, (gg as u32 * shade / 255) as u8, (bb as u32 * shade / 255) as u8])
        };
        for p in &line.glyphs {
            let Some(slot) = p.slot else { continue };
            let mut r = sprite;
            r.u0 = sprite.u0 + slot.x as f32 * du;
            r.v0 = sprite.v0 + slot.y as f32 * dv;
            r.u1 = sprite.u0 + (slot.x + slot.w) as f32 * du;
            r.v1 = sprite.v0 + (slot.y + slot.h) as f32 * dv;
            let gx = x + (p.x + slot.left) * unit + off * scale;
            let gy = baseline + slot.top * unit + off * scale;
            b.rect_uv(gx, gy, slot.w as f32 * unit, slot.h as f32 * unit, r, packed(c));
        }
    }
    Some(line.width * unit)
}
/// Draws `t` with its top left at `(x, y)`, scaled about that point; the
/// advance in GUI pixels.
fn draw(g: &Game<'_>, b: &mut HudBatch, t: &Text, x: f32, y: f32, scale: f32, color: u32, shadow: bool) -> f32 {
    if (color >> 24) < 6 {
        return width(g, t) * scale;
    }
    if let Some((id, s)) = ttf(t)
        && let Some(w) = draw_ttf(g, b, id, &s, x, y, scale, color, shadow)
    {
        return w;
    }
    let t = &mc_only(t);
    let start = b.vertices.len();
    let w = b.text_styled(&g.gui.atlas, t, x, y, packed(color), shadow);
    if (scale - 1.0).abs() > 0.001 {
        b.scale_since(start, x, y, scale, scale);
    }
    w * scale
}
fn fill(g: &Game<'_>, b: &mut HudBatch, x: f32, y: f32, w: f32, h: f32, color: u32) {
    if (color >> 24) == 0 || w <= 0.0 || h <= 0.0 {
        return;
    }
    g.native_fill(b, [x, y, w, h], color);
}
/// A rectangle with its corner pixels cut, as Spotify's panels look at GUI size.
fn panel(g: &Game<'_>, b: &mut HudBatch, x: f32, y: f32, w: f32, h: f32, color: u32) {
    if w < 3.0 || h < 3.0 {
        fill(g, b, x, y, w, h, color);
        return;
    }
    fill(g, b, x + 1.0, y, w - 2.0, h, color);
    fill(g, b, x, y + 1.0, 1.0, h - 2.0, color);
    fill(g, b, x + w - 1.0, y + 1.0, 1.0, h - 2.0, color);
}
fn outline(g: &Game<'_>, b: &mut HudBatch, x: f32, y: f32, w: f32, h: f32, color: u32) {
    fill(g, b, x, y, w, 1.0, color);
    fill(g, b, x, y + h - 1.0, w, 1.0, color);
    fill(g, b, x, y + 1.0, 1.0, h - 2.0, color);
    fill(g, b, x + w - 1.0, y + 1.0, 1.0, h - 2.0, color);
}
/// The album art from the mod image, or a gradient with a note.
fn cover(g: &Game<'_>, b: &mut HudBatch, ui: &Ui, x: f32, y: f32, side: f32, alpha: f32) {
    if ui.art_ready
        && let Some(mut r) = g.gui.atlas.sprites.get(horizon_client::hud::MOD_IMAGE_SPRITE).copied()
    {
        let du = (r.u1 - r.u0) / r.w.max(1) as f32;
        let dv = (r.v1 - r.v0) / r.h.max(1) as f32;
        r.u1 = r.u0 + ART as f32 * du;
        r.v1 = r.v0 + ART as f32 * dv;
        let a = (alpha.clamp(0.0, 1.0) * 255.0) as u8;
        b.rect_uv(x, y, side, side, r, rgba(255, 255, 255, a));
        return;
    }
    let accent = ui.accent();
    let top = argb(art::mix(accent, 0x101010, 0.3), alpha);
    let bottom = argb(art::mix(accent, 0x101010, 0.8), alpha);
    b.fill_gradient(&g.gui.atlas, (x, y, side, side), packed(top), packed(bottom));
    let note = plain("♪");
    let s = (side / 16.0).max(1.0);
    let w = width(g, &note) * s;
    draw(g, b, &note, x + (side - w) / 2.0, y + (side - 8.0 * s) / 2.0, s, argb(0xFFFFFF, alpha * 0.9), false);
}
/// Text clipped to `max` pixels, scrolling back and forth when longer.
fn marquee(g: &Game<'_>, b: &mut HudBatch, t: &Text, x: f32, y: f32, scale: f32, max: f32, color: u32, run: bool, clock: f32) {
    let w = width(g, t) * scale;
    if w <= max || !run {
        let shown = if w <= max { t.clone() } else { ellipsize(g, t, max / scale) };
        draw(g, b, &shown, x, y, scale, color, true);
        return;
    }
    let travel = w - max;
    // Hold, scroll, hold, scroll back: 30 GUI px a second.
    let leg = travel / 30.0;
    let period = 2.0 * (leg + 1.5);
    let p = clock % period;
    let off = if p < 1.5 {
        0.0
    } else if p < 1.5 + leg {
        (p - 1.5) * 30.0
    } else if p < 3.0 + leg {
        travel
    } else {
        travel - (p - 3.0 - leg) * 30.0
    };
    let start = b.vertices.len();
    draw(g, b, t, x - off, y, scale, color, true);
    b.clip_since(start, x, y - 2.0, x + max, y + 10.0 * scale + 2.0);
}
fn ellipsize(g: &Game<'_>, t: &Text, max: f32) -> Text {
    let s = t.plain();
    let st = t.spans.first().map(|sp| sp.style.clone()).unwrap_or_default();
    let mut out: String = s.clone();
    while !out.is_empty() && width(g, &text(&format!("{out}…"), &st)) > max {
        out.pop();
    }
    text(&format!("{}…", out.trim_end()), &st)
}
/// Words of `s` in lines no wider than `max` (at scale 1).
fn wrap(g: &Game<'_>, s: &str, st: &Style, max: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in s.split_whitespace() {
        let tryline = if cur.is_empty() { word.to_owned() } else { format!("{cur} {word}") };
        if !cur.is_empty() && width(g, &text(&tryline, st)) > max {
            lines.push(std::mem::take(&mut cur));
            cur = word.to_owned();
        } else {
            cur = tryline;
        }
    }
    if !cur.is_empty() || lines.is_empty() {
        lines.push(cur);
    }
    lines
}

fn lyric_text(cfg: &Config, s: &str) -> String {
    if cfg.uppercase { s.to_uppercase() } else { s.to_owned() }
}

/// The position lyrics are looked up at.
/// Lines start this much early, so their fade-in lands on the beat.
const LEAD_MS: i64 = 150;

fn lyric_pos(cfg: &Config, snap: &Snapshot) -> u64 {
    (snap.position_ms as i64 + cfg.offset_ms + LEAD_MS).max(0) as u64
}

fn visible(g: &Game<'_>, ui: &Ui) -> bool {
    if !ui.cfg.enabled || ui.hidden {
        return false;
    }
    let menu = g.native_mod_screen() == Some(ID);
    !(ui.cfg.hide_in_screens && g.screen != Screen::Playing && !menu)
}

// ---------------------------------------------------------------- hooks: list, settings, keys

pub(crate) fn listed(mods: &mut Vec<NativeMod>) {
    mods.push(NativeMod {
        id: ID.into(),
        title: format!("{NAME} {VERSION}"),
        description: DESCRIPTION.into(),
        keys: KEYS
            .iter()
            .map(|(n, l, d)| [(*n).to_owned(), (*l).to_owned(), (*d).to_owned()])
            .collect(),
    });
}

pub(crate) fn commands(list: &mut Vec<&'static str>) {
    list.push("lyrics");
}

/// The buttons of the mod's page in Options > Mods.
pub(crate) fn settings_page(game: &mut Game<'_>, mod_id: &str, buttons: &mut Vec<String>) {
    if mod_id != ID {
        return;
    }
    let e = eng(game);
    let c = e.config();
    let on = |b: bool| if b { "ON" } else { "OFF" };
    buttons.extend([
        format!("LyricsPlus: {}", on(c.enabled)),
        format!("Display: {}", c.display.label()),
        format!("Lyrics: {}", on(c.show_lyrics)),
        format!("Player widget: {}", on(c.show_widget)),
        format!("Now-playing toast: {}", on(c.show_toast)),
        format!("Source: {}", c.source.label()),
        format!("Adaptive colours: {}", on(c.adaptive_colors)),
        "Full menu: press K in game (or /lyrics)".to_owned(),
    ]);
}

pub(crate) fn settings_click(game: &mut Game<'_>, mod_id: &str, button: usize) {
    if mod_id != ID {
        return;
    }
    eng(game).update_config(|c| match button {
        0 => c.enabled = !c.enabled,
        1 => c.display = c.display.cycle(false),
        2 => c.show_lyrics = !c.show_lyrics,
        3 => c.show_widget = !c.show_widget,
        4 => c.show_toast = !c.show_toast,
        5 => c.source = c.source.cycle(false),
        6 => c.adaptive_colors = !c.adaptive_colors,
        _ => {}
    });
}

pub(crate) fn key(game: &mut Game<'_>, key: &str, pressed: bool) {
    if !pressed || !game.native_window() || !game.native_on(ID) {
        return;
    }
    let menu_open = game.native_mod_screen() == Some(ID);
    if bound(game, KEY_MENU, key) {
        let typing = with_ui(|ui| ui.menu.focus.is_some());
        if menu_open && !typing {
            game.native_mod_screen_close(ID);
        } else if game.native_playing() {
            open_menu(game);
        }
        return;
    }
    if !(game.native_playing() || menu_open) {
        return;
    }
    let e = eng(game);
    if bound(game, KEY_TOGGLE, key) && !menu_open {
        let hidden = with_ui(|ui| {
            ui.hidden = !ui.hidden;
            ui.hidden
        });
        game.native_chat(if hidden { "[LyricsPlus] Hidden (press again to show)." } else { "[LyricsPlus] Shown." });
    } else if bound(game, KEY_PLAY, key) {
        e.control(Control::PlayPause);
    } else if bound(game, KEY_NEXT, key) {
        e.control(Control::Next);
    } else if bound(game, KEY_PREV, key) {
        e.control(Control::Previous);
    } else if bound(game, KEY_PVP, key) {
        let mut on = false;
        e.update_config(|c| {
            c.pvp_mode = !c.pvp_mode;
            on = c.pvp_mode;
        });
        game.native_chat(if on { "[LyricsPlus] PvP mode: lyrics follow you, split to the sides." } else { "[LyricsPlus] PvP mode off." });
    }
}

fn bound(g: &Game<'_>, k: (&str, &str, &str), key: &str) -> bool {
    g.native_mod_key(ID, k.0, k.2) == key
}

fn say(g: &mut Game<'_>, s: &str) {
    g.native_chat(&format!("[LyricsPlus] {s}"));
}

fn open_menu(game: &mut Game<'_>) {
    with_ui(|ui| {
        ui.menu.focus = None;
        ui.menu.drag = None;
        ui.menu.layout = false;
    });
    if !game.native_mod_screen_open(ID) {
        with_ui(|ui| ui.pending.push(Pending::OpenScreen(20)));
    }
}

// ---------------------------------------------------------------- hook: frame

/// Once a frame: queued actions, chat lines, and a new cover into the mod image.
pub(crate) fn frame(game: &mut Game<'_>) {
    if !game.native_window() {
        return;
    }
    let e = eng(game);
    let pending = with_ui(|ui| std::mem::take(&mut ui.pending));
    for p in pending {
        match p {
            Pending::OpenScreen(tries) => {
                if !game.native_mod_screen_open(ID) && tries > 0 {
                    with_ui(|ui| ui.pending.push(Pending::OpenScreen(tries - 1)));
                }
            }
            Pending::CloseScreen => game.native_mod_screen_close(ID),
            Pending::ReloadFonts => {
                load_fonts(game, e);
                let n = with_ui(|ui| ui.fonts.len());
                game.native_chat(&format!("[LyricsPlus] {n} fonts loaded."));
            }
            Pending::Paste => {
                use horizon_client::clipboard::Clipboard;
                let t = game.clipboard.get();
                with_ui(|ui| {
                    if ui.menu.focus.is_some() {
                        let clean: String = t.chars().filter(|c| !c.is_control()).collect();
                        ui.menu.buffer.push_str(clean.trim());
                    }
                });
            }
            Pending::Copy(t) => {
                use horizon_client::clipboard::Clipboard;
                game.clipboard.set(&t);
            }
            Pending::Chat(t) => game.native_chat(&t),
        }
    }
    for m in e.drain_messages() {
        game.native_chat(&format!("[LyricsPlus] {m}"));
    }
    if !game.native_on(ID) {
        return;
    }
    let snap = e.snapshot();
    let revision = game.gui.atlas.revision;
    let (rebuilt, decode, serial, fonts) = with_ui(|ui| {
        let now = Instant::now();
        ui.dt = now.saturating_duration_since(ui.last_frame).as_secs_f32().clamp(0.0, 0.25);
        ui.last_frame = now;
        ui.refresh(e);
        let serial = snap.track.as_ref().map_or(0, |t| t.serial);
        if serial != ui.art_serial {
            ui.art_serial = serial;
            ui.art_ready = false;
            ui.palette = Palette::default();
        }
        // The atlas was rebuilt (a resource reload): its slot is empty again.
        let rebuilt = ui.image_revision != Some(revision);
        ui.image_revision = Some(revision);
        // A new cover is decoded on its own thread, not in the frame.
        let decode = snap.art.as_ref().filter(|a| a.serial == serial && ui.art_tried != serial).map(|a| a.bytes.clone());
        if decode.is_some() {
            ui.art_tried = serial;
        }
        let fonts: Vec<String> = [ui.cfg.lyrics_font.clone(), ui.cfg.title_font.clone()]
            .into_iter()
            .filter(|f| f.starts_with("ttf:") && !ui.warmed.contains(f))
            .collect();
        (rebuilt, decode, serial, fonts)
    });
    // The fonts, once: the built-in ones and the player's `fonts` folder.
    if CANVAS.lock().unwrap_or_else(|p| p.into_inner()).is_none() {
        load_fonts(game, e);
    }
    if let Some(bytes) = decode {
        let _ = std::thread::Builder::new().name("lyricsplus-cover".into()).spawn(move || {
            if let Ok((w, h, rgba)) = horizon_client::hud::decode_image_bytes(&bytes) {
                let square = art::square(w, h, &rgba, ART);
                let palette = art::palette(w, h, &rgba);
                *DECODED.lock().unwrap_or_else(|p| p.into_inner()) = Some((serial, square, palette));
            }
        });
    }
    let decoded = DECODED.lock().unwrap_or_else(|p| p.into_inner()).take();
    if let Some((s, square, palette)) = decoded {
        if s == serial {
            with_canvas(|c| c.set_art(&square));
            with_ui(|ui| {
                ui.palette = palette;
                ui.art_ready = true;
                ui.last_upload = None;
            });
        }
    }
    // The lyric fonts' common letters go in the atlas at once (one upload),
    // not one by one as lines bring them.
    if !fonts.is_empty() {
        with_canvas(|c| {
            for f in &fonts {
                if let Some(i) = c.find(f) {
                    let _ = c.layout(i, WARM_GLYPHS);
                }
            }
        });
        with_ui(|ui| ui.warmed.extend(fonts));
    }
    if rebuilt {
        with_canvas(|c| c.invalidate());
    }
    // New glyphs or a new cover: upload the rows that changed, at most a few
    // times a second (each upload is the image down to the lowest change).
    let due = rebuilt || with_ui(|ui| ui.last_upload.is_none_or(|t| t.elapsed() > Duration::from_millis(150)));
    let dirty = if due {
        with_canvas(|c| {
            c.take_dirty().map(|h| {
                let h = h.min(glyphs::SIZE);
                (h, c.pixels[..(glyphs::SIZE * h * 4) as usize].to_vec())
            })
        })
        .flatten()
    } else {
        None
    };
    if let Some((h, pixels)) = dirty {
        with_ui(|ui| ui.last_upload = Some(Instant::now()));
        if !game.native_mod_image(glyphs::SIZE, h, &pixels) {
            // No image slot in this build: covers fall back to the placeholder.
            with_ui(|ui| ui.art_ready = false);
        }
    }
    with_ui(|ui| ui.ease_colors());
}

/// Letters most lyrics use, put in the atlas when a font is chosen.
const WARM_GLYPHS: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 .,!?'\"-:;()&…’‘“”♪";

/// The game is closing: stop the media helper.
pub(crate) fn stopping() {
    if let Some(e) = engine::running() {
        e.shutdown();
    }
}

// ---------------------------------------------------------------- hook: HUD

pub(crate) fn hud(game: &mut Game<'_>, batch: &mut HudBatch, corners: &mut Corners) {
    if !game.native_window() || !game.native_on(ID) {
        return;
    }
    let attacking = game.native_clicks().0 > 0;
    let g: &Game<'_> = &*game;
    let e = eng(g);
    let snap = e.snapshot();
    with_ui(|ui| {
        ui.refresh(e);
        if attacking {
            ui.last_attack = Some(Instant::now());
        }
        let target = if attacking && ui.cfg.combat_fade && !ui.pvp_now() { 1.0 } else { 0.0 };
        let rate = if target > ui.combat { 12.0 } else { 1.5 };
        ui.combat += (target - ui.combat) * (1.0 - (-ui.dt * rate).exp());
        ui.lyrics_rect = [0.0; 4];
        ui.widget_rect = [0.0; 4];
        if !visible(g, ui) {
            return;
        }
        let shown = snap.track.is_some() && !(ui.cfg.hide_when_paused && !snap.playing);
        let fade = 1.0 - 0.6 * ui.combat;
        if shown && ui.cfg.show_widget {
            widget(g, batch, corners, ui, &snap, fade);
        }
        if shown && ui.cfg.show_lyrics && matches!(ui.cfg.display, Display::Hud | Display::Both) {
            lyrics_hud(g, batch, ui, &snap, fade);
        }
        if shown && ui.cfg.island {
            island(g, batch, ui, &snap);
        }
        if ui.cfg.show_toast {
            toast(g, batch, ui, &snap);
        }
        if ui.menu.layout && g.native_mod_screen() == Some(ID) {
            layout_overlay(g, batch, ui);
        }
    });
}

/// The now-playing card: what it shows, its size and roundness as set.
fn widget(g: &Game<'_>, b: &mut HudBatch, corners: &mut Corners, ui: &mut Ui, snap: &Snapshot, fade: f32) {
    let Some(track) = &snap.track else { return };
    let cfg = &ui.cfg;
    let s = cfg.widget_scale;
    let full = cfg.widget_style == WidgetStyle::Full;
    // What the card shows decides its height.
    let art = if cfg.show_art && cfg.widget_style != WidgetStyle::Mini { (if full { 42.0 } else { 24.0 }) * cfg.art_size } else { 0.0 };
    let pad = if full { 6.0 } else { 5.0 };
    let mut lines_h = 0.0;
    if cfg.show_title {
        lines_h += 11.0;
    }
    if cfg.show_artist {
        lines_h += if full { 11.0 } else { 9.5 };
    }
    if cfg.show_album {
        lines_h += 8.5;
    }
    let bar_h = if full && cfg.show_progress { 6.0 } else { 0.0 };
    let foot_h = if full && (cfg.show_time || cfg.show_source) { 9.0 } else { 0.0 };
    let bw = match cfg.widget_style {
        WidgetStyle::Full => 184.0,
        WidgetStyle::Compact => 150.0,
        WidgetStyle::Mini => 150.0,
    } * cfg.widget_width;
    let bh = match cfg.widget_style {
        WidgetStyle::Mini => 16.0,
        _ => (art + 2.0 * pad).max(lines_h + bar_h + foot_h + 2.0 * pad + 1.0).max(16.0),
    };
    let (w, h) = (bw * s, bh * s);
    let (x, y) = match cfg.widget_corner {
        Corner::Custom => (
            (cfg.widget_x * b.width).clamp(0.0, (b.width - w).max(0.0)),
            (cfg.widget_y * b.height).clamp(0.0, (b.height - h).max(0.0)),
        ),
        c => {
            let hc = match c {
                Corner::TopLeft => HCorner::TopLeft,
                Corner::TopRight => HCorner::TopRight,
                Corner::BottomLeft => HCorner::BottomLeft,
                _ => HCorner::BottomRight,
            };
            if !g.native_corner_shown(hc) {
                return;
            }
            let (px, py) = corners.place(hc, w + 4.0, h + 4.0);
            (px + 2.0, py + 2.0)
        }
    };
    ui.widget_rect = [x, y, w, h];
    let alpha = cfg.widget_alpha * fade;
    let start = b.vertices.len();
    // Drawn at scale 1 at (x, y), scaled at the end.
    let accent = ui.accent();
    let radius = cfg.widget_round * bh.min(40.0) / 2.0;
    round_rect(g, b, x, y, bw, bh, radius, argb(ui.panel(), alpha));
    // A flash of the accent as a new track starts.
    let flash = 1.0 - (snap.track_age.as_secs_f32() / 0.8).clamp(0.0, 1.0);
    if flash > 0.0 {
        round_outline(g, b, x, y, bw, bh, radius, argb(accent, flash * fade));
    }
    let clock = secs(ui.started);
    let title_style = style(&cfg.title_font, true, false);
    let artist_style = style(&cfg.title_font, false, false);
    let ink = argb(0xFFFFFF, fade);
    let dim = argb(0xB3B3B3, fade);
    let progress = if track.duration_ms > 0 { (snap.position_ms as f32 / track.duration_ms as f32).clamp(0.0, 1.0) } else { 0.0 };
    let inner = radius * 0.3;
    if cfg.widget_style == WidgetStyle::Mini {
        play_icon(g, b, x + 5.0 + inner, y + 4.5, snap.playing, argb(accent, fade));
        let line = match (cfg.show_title, cfg.show_artist) {
            (true, true) => format!("{} – {}", track.title, track.artist),
            (false, true) => track.artist.clone(),
            _ => track.title.clone(),
        };
        marquee(g, b, &text(&line, &title_style), x + 14.0 + inner, y + 4.0, 1.0, bw - 20.0 - 2.0 * inner, ink, cfg.marquee, clock);
        if cfg.show_progress {
            fill(g, b, x + 1.0 + inner, y + bh - 1.5, (bw - 2.0 - 2.0 * inner) * progress, 1.0, argb(accent, fade));
        }
    } else {
        let tx = if art > 0.0 {
            cover_round(g, b, ui, x + pad + inner * 0.5, y + (bh - art) / 2.0, art, fade, cfg.art_round);
            x + pad + inner * 0.5 + art + pad + 1.0
        } else {
            x + pad + 2.0 + inner
        };
        let eq_w = if cfg.equalizer && full { 14.0 } else { 0.0 };
        let right = x + bw - pad - inner * 0.5;
        let max = right - tx - eq_w;
        // The text block, centred in the height the card has.
        let block = lines_h + bar_h + foot_h;
        let mut ty = y + (bh - block) / 2.0 + 1.0;
        if cfg.equalizer && full {
            equalizer(g, b, right - 11.0, ty, snap.playing, accent, fade, clock);
        }
        if cfg.show_title {
            marquee(g, b, &text(&track.title, &title_style), tx, ty, 1.0, max, ink, cfg.marquee, clock);
            ty += 11.0;
        }
        if cfg.show_artist {
            let sc = if full { 1.0 } else { 0.85 };
            marquee(g, b, &text(&track.artist, &artist_style), tx, ty, sc, max + eq_w, dim, cfg.marquee, clock + 0.7);
            ty += if full { 11.0 } else { 9.5 };
        }
        if cfg.show_album {
            let album = if track.album.is_empty() { "—".to_owned() } else { track.album.clone() };
            marquee(g, b, &text(&album, &artist_style), tx, ty, 0.7, max + eq_w, argb(0x8A8A8A, fade), cfg.marquee, clock + 1.4);
            ty += 8.5;
        }
        if full {
            let bar_w = right - tx;
            if cfg.show_progress {
                fill(g, b, tx, ty + 1.0, bar_w, 2.0, argb(0x535353, alpha.max(0.6) * fade));
                fill(g, b, tx, ty + 1.0, bar_w * progress, 2.0, argb(accent, fade));
                fill(g, b, tx + bar_w * progress - 1.0, ty, 2.0, 4.0, argb(0xFFFFFF, fade));
                ty += bar_h;
            }
            let small = 0.75;
            if cfg.show_time {
                let left = time(snap.position_ms);
                let rt = if track.duration_ms > 0 { time(track.duration_ms) } else { "--:--".into() };
                draw(g, b, &plain(&left), tx, ty, small, dim, false);
                let rw = width(g, &plain(&rt)) * small;
                draw(g, b, &plain(&rt), right - rw, ty, small, dim, false);
            }
            if cfg.show_source {
                // Play state and source, in the middle.
                let source = if track.app.is_empty() { format!("{:?}", track.origin) } else { track.app.clone() };
                let mid = tx + bar_w / 2.0;
                let src = plain(&source);
                let sw = (width(g, &src) * small).min(bar_w * 0.45);
                play_icon(g, b, mid - (sw + 9.0) / 2.0, ty + 0.5, snap.playing, ink);
                draw(g, b, &ellipsize(g, &src, bar_w * 0.45 / small), mid - (sw + 9.0) / 2.0 + 9.0, ty, small, argb(accent, 0.9 * fade), false);
            }
        } else if cfg.show_progress {
            let r_in = radius.min(bh / 2.0);
            fill(g, b, x + 1.0 + r_in, y + bh - 2.0, bw - 2.0 - 2.0 * r_in, 1.5, argb(0x535353, fade));
            fill(g, b, x + 1.0 + r_in, y + bh - 2.0, (bw - 2.0 - 2.0 * r_in) * progress, 1.5, argb(accent, fade));
        }
    }
    if (s - 1.0).abs() > 0.001 {
        b.scale_since(start, x, y, s, s);
    }
}

fn play_icon(g: &Game<'_>, b: &mut HudBatch, x: f32, y: f32, playing: bool, color: u32) {
    if playing {
        fill(g, b, x, y, 2.0, 6.0, color);
        fill(g, b, x + 3.5, y, 2.0, 6.0, color);
    } else {
        g.native_quad(b, [[x, y], [x + 5.5, y + 3.0], [x + 5.5, y + 3.0], [x, y + 6.0]], color);
    }
}

fn equalizer(g: &Game<'_>, b: &mut HudBatch, x: f32, y: f32, playing: bool, accent: u32, fade: f32, clock: f32) {
    for i in 0..4 {
        let h = if playing {
            let f = [5.1, 7.3, 4.2, 6.6][i];
            2.0 + 6.0 * ((clock * f + i as f32 * 1.7).sin() * 0.5 + 0.5)
        } else {
            2.0
        };
        fill(g, b, x + i as f32 * 3.0, y + 8.0 - h, 2.0, h, argb(accent, fade));
    }
}

/// One drawn lyric row.
struct Row2 {
    text: String,
    style: Style,
    scale: f32,
    color: u32,
    active: bool,
    progress: f32,
    alpha: f32,
}

/// The lyrics, 2D.
fn lyrics_hud(g: &Game<'_>, b: &mut HudBatch, ui: &mut Ui, snap: &Snapshot, fade: f32) {
    let Some(track) = &snap.track else { return };
    let cfg = ui.cfg.clone();
    let pos = lyric_pos(&cfg, snap);
    let lyrics = &snap.lyrics;
    let index = lyrics.index_at(pos);
    let line_changed = index != ui.line && track.serial == ui.line_serial && ui.line.is_some();
    let grad = gradient(ui);
    let active_word = index.filter(|_| lyrics.synced).and_then(|i| lyrics.word_at(i, pos, track.duration_ms));
    if index != ui.line || track.serial != ui.line_serial {
        ui.prev_line = ui.line;
        ui.previous_text = ui
            .line
            .filter(|_| track.serial == ui.line_serial)
            .and_then(|i| lyrics.lines.get(i))
            .map(|l| l.text.clone());
        ui.line = index;
        ui.line_serial = track.serial;
        ui.line_changed = Instant::now();
    }
    let t = if cfg.smooth { ease_out(secs(ui.line_changed) / 0.28) } else { 1.0 };
    let base = cfg.lyrics_scale;
    let max_w = b.width * cfg.max_width;
    let active_col = ui.active_color();
    let mut rows: Vec<Row2> = Vec::new();
    let status = |s: &str, rows: &mut Vec<Row2>| {
        rows.push(Row2 {
            text: s.to_owned(),
            style: style(&cfg.lyrics_font, false, true),
            scale: base * 0.8,
            color: 0xB3B3B3,
            active: false,
            progress: 0.0,
            alpha: 0.9,
        })
    };
    let early = snap.track_age < Duration::from_secs(8);
    if lyrics.lines.is_empty() {
        if !cfg.show_status {
            return;
        }
        match snap.lyrics_state {
            LyricsState::Loading => status("♪ Looking for lyrics…", &mut rows),
            LyricsState::Instrumental => status("♪ Instrumental ♪", &mut rows),
            LyricsState::NotFound if early => status("No lyrics found for this song", &mut rows),
            LyricsState::Failed if early => status("Lyrics unavailable (offline?)", &mut rows),
            _ => return,
        }
    } else if let Some((left, total)) = lyrics.gap_remaining(pos).filter(|_| cfg.countdown) {
        // Instrumental countdown: three dots fill as the next line nears.
        let done = 1.0 - left as f32 / total.max(1) as f32;
        let dots: String = (0..3)
            .map(|i| if done * 3.0 > i as f32 + 0.5 { '●' } else { '○' })
            .collect::<Vec<char>>()
            .iter()
            .map(|c| format!("{c} "))
            .collect();
        let label = if left > 9000 { format!("{}  {}s", dots.trim_end(), left / 1000) } else { dots.trim_end().to_owned() };
        rows.push(Row2 {
            text: label,
            style: style(&cfg.lyrics_font, true, false),
            scale: base,
            color: ui.accent(),
            active: false,
            progress: 0.0,
            alpha: 0.95,
        });
        if let Some(next) = lyrics.lines.iter().find(|l| l.time_ms > pos && !l.text.trim().is_empty()) {
            rows.push(Row2 {
                text: lyric_text(&cfg, &next.text),
                style: style(&cfg.lyrics_font, false, cfg.italic),
                scale: base * 0.8,
                color: cfg.inactive_color,
                active: false,
                progress: 0.0,
                alpha: 0.6,
            });
        }
    } else {
        let n = cfg.lines as usize;
        let before = if n >= 3 { (n - 1) / 2 } else { 0 };
        let after = n - 1 - before;
        let cur = index.unwrap_or(0);
        let first = cur.saturating_sub(before);
        let has_current = index.is_some();
        let from = if has_current { first } else { 0 };
        let to = if has_current { (cur + after + 1).min(lyrics.lines.len()) } else { n.min(lyrics.lines.len()) };
        for i in from..to {
            let line = &lyrics.lines[i];
            let active = has_current && i == cur;
            let d = (i as i64 - cur as i64).unsigned_abs() as f32;
            let raw = if line.text.trim().is_empty() { "♪".to_owned() } else { lyric_text(&cfg, &line.text) };
            rows.push(Row2 {
                text: raw,
                style: style(&cfg.lyrics_font, active && cfg.bold_active, cfg.italic),
                scale: if active { base } else { base * 0.8 },
                color: if active { active_col } else { cfg.inactive_color },
                active,
                progress: if active { lyrics.progress(i, pos, track.duration_ms) } else { 0.0 },
                alpha: if active { 1.0 } else { (0.75 - 0.2 * (d - 1.0)).max(0.25) },
            });
        }
        if !lyrics.synced && !rows.is_empty() {
            rows.push(Row2 {
                text: "(unsynced lyrics)".into(),
                style: style("minecraft:default", false, true),
                scale: base * 0.6,
                color: 0x808080,
                active: false,
                progress: 0.0,
                alpha: 0.6,
            });
        }
    }
    if rows.is_empty() {
        return;
    }
    // The active line is placed word by word when it moves (the styles,
    // arrangements, motion and vanish); the others wrap as plain rows.
    let (arr, _) = ui.arrangement();
    let word_style = matches!(cfg.highlight, Highlight::Bounce | Highlight::Word | Highlight::OneWord | Highlight::Kick);
    let fancy = word_style || arr != Arrangement::Line || cfg.motion != Motion::Still || cfg.vanish != Vanish::Off;
    let accent = ui.accent();
    struct Laid {
        line: String,
        row: usize,
        w: f32,
        h: f32,
        placed: Option<Layout>,
    }
    let mut laid: Vec<Laid> = Vec::new();
    let mut last_word: Option<usize> = None;
    for (ri, r) in rows.iter().enumerate() {
        if r.active && fancy {
            let words: Vec<String> = r.text.split_whitespace().map(str::to_owned).collect();
            let a = r.alpha * fade * t.max(0.35);
            let base_c = if grad.is_some() { 0xFFFFFF } else { r.color };
            let hi = if grad.is_some() { 0xFFFFFF } else { accent };
            let seed = track.serial.wrapping_mul(31) ^ index.unwrap_or(0) as u64;
            let fx = LineFx::of(ui, active_word, r.progress, seed, max_w / r.scale);
            let lay = match active_word {
                Some((k, p)) if cfg.highlight == Highlight::OneWord => {
                    let one = vec![words.get(k).cloned().unwrap_or_default()];
                    let look = Look { color: argb(hi, a), scale: 1.25 + 0.2 * (1.0 - ease_out(p * 4.0)), lift: 3.0 * jump(p), underline: false };
                    last_word = Some(0);
                    place(g, &one, &r.style, &|_| look, &LineFx { arrangement: Arrangement::Line, sung: Some((0, p)), vanish: Vanish::Off, ..fx })
                }
                Some((k, p)) => {
                    last_word = Some(words.len().saturating_sub(1));
                    place(g, &words, &r.style, &|i| word_look(cfg.highlight, i, k, p, argb(base_c, a), argb(hi, a), argb(base_c, 0.45 * a)), &fx)
                }
                None => {
                    last_word = Some(words.len().saturating_sub(1));
                    place(g, &words, &r.style, &|_| Look::plain(argb(base_c, a)), &fx)
                }
            };
            let [x0, y0, x1, y1] = lay.bounds;
            laid.push(Laid { line: r.text.clone(), row: ri, w: (x1 - x0) * r.scale, h: (y1 - y0) * r.scale + 2.0, placed: Some(lay) });
            continue;
        }
        for part in wrap(g, &r.text, &r.style, max_w / r.scale) {
            let w = width(g, &text(&part, &r.style)) * r.scale;
            laid.push(Laid { line: part, row: ri, w, h: 10.0 * r.scale, placed: None });
        }
    }
    let block_w = laid.iter().map(|l| l.w).fold(0.0, f32::max);
    let block_h: f32 = laid.iter().map(|l| l.h).sum::<f32>() + 2.0;
    let (sw, sh) = (b.width, b.height);
    let pad = 4.0;
    let (cx, top, align) = match cfg.lyrics_position {
        LyricsPosition::Top => (sw / 2.0, 18.0, cfg.lyrics_align),
        LyricsPosition::Center => (sw / 2.0, (sh - block_h) / 2.0, cfg.lyrics_align),
        LyricsPosition::AboveCrosshair => (sw / 2.0, sh / 2.0 - 14.0 - block_h, cfg.lyrics_align),
        LyricsPosition::BelowCrosshair => (sw / 2.0, sh / 2.0 + 14.0, cfg.lyrics_align),
        LyricsPosition::Bottom => (sw / 2.0, sh - 64.0 - block_h, cfg.lyrics_align),
        LyricsPosition::TopLeft => (pad + 2.0 + block_w / 2.0, 6.0, Align::Left),
        LyricsPosition::TopRight => (sw - pad - 2.0 - block_w / 2.0, 6.0, Align::Right),
        LyricsPosition::Custom => (cfg.lyrics_x * sw, cfg.lyrics_y * sh - block_h / 2.0, cfg.lyrics_align),
    };
    // PvP: split round the crosshair, so the block sits on it.
    let top = if ui.pvp_now() && laid.len() == 1 && laid[0].placed.is_some() { sh / 2.0 - block_h / 2.0 } else { top };
    let left = cx - block_w / 2.0;
    ui.lyrics_rect = [left - pad, top - pad, block_w + 2.0 * pad, block_h + 2.0 * pad];
    if cfg.lyrics_background > 0.01 && !ui.pvp_now() {
        panel(g, b, left - pad, top - pad, block_w + 2.0 * pad, block_h + 2.0 * pad, argb(ui.panel(), cfg.lyrics_background * fade));
    }
    // The block slides up a line as a new line starts, inside its panel.
    let slide = if cfg.smooth { (1.0 - t) * 10.0 * base * 0.8 } else { 0.0 };
    let mut y = top + 1.0 + slide;
    let text_start = b.vertices.len();
    let mut spark_rect = None;
    for l in &laid {
        let r = &rows[l.row];
        let x = match align {
            Align::Left => left,
            Align::Right => left + block_w - l.w,
            Align::Center => cx - l.w / 2.0,
        };
        let a = r.alpha * fade * if r.active { t.max(0.35) } else { 1.0 };
        if let Some(lay) = &l.placed {
            let [x0, y0, x1, _] = lay.bounds;
            let ox = x - x0 * r.scale;
            let oy = y + 1.0 - y0 * r.scale;
            let _ = x1;
            draw_line_hud(g, b, lay, &r.style, ox, oy, r.scale, cfg.shadow, grad);
            if let Some(&[wx, wy]) = last_word.and_then(|k| lay.words.get(k)) {
                spark_rect = Some([ox + wx * r.scale - 5.0 * r.scale, oy + wy * r.scale - 4.0 * r.scale, 10.0 * r.scale, 8.0 * r.scale]);
            }
            y += l.h;
            continue;
        }
        let t_line = text(&l.line, &r.style);
        let row_start = b.vertices.len();
        if r.active {
            if cfg.highlight == Highlight::Glow {
                for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
                    draw(g, b, &t_line, x + dx, y + dy, r.scale, argb(accent, 0.35 * a), false);
                }
            }
            if cfg.highlight == Highlight::Sweep {
                // Sung part in the active colour, the rest dimmer: a sweep
                // through the wrapped parts in order.
                let parts: Vec<&Laid> = laid.iter().filter(|p| p.row == l.row).collect();
                let total: f32 = parts.iter().map(|p| p.w).sum();
                let before: f32 = parts.iter().take_while(|p| !std::ptr::eq(**p, l)).map(|p| p.w).sum();
                let sung = (r.progress * total - before).clamp(0.0, l.w);
                draw(g, b, &t_line, x, y, r.scale, argb(art::mix(r.color, 0x404040, 0.45), a), cfg.shadow);
                if sung > 0.0 {
                    let start = b.vertices.len();
                    draw(g, b, &t_line, x, y, r.scale, argb(r.color, a), false);
                    b.clip_since(start, x - 1.0, y - 2.0, x + sung, y + l.h + 2.0);
                }
            } else {
                draw(g, b, &t_line, x, y, r.scale, argb(r.color, a), cfg.shadow);
            }
            spark_rect = Some([x + l.w - 12.0 * r.scale, y, 12.0 * r.scale, l.h]);
        } else {
            draw(g, b, &t_line, x, y, r.scale, argb(r.color, a), cfg.shadow);
        }
        if let Some((c0, c1)) = grad {
            recolor(b, row_start, x, x + l.w, c0, c1);
        }
        y += l.h;
    }
    // Sparks from the last word as it's sung (or as the line ends).
    if cfg.sparks {
        let key = (track.serial, ui.line);
        let seed = track.serial ^ index.unwrap_or(0) as u64;
        if line_changed && ui.hud_fired != Some((track.serial, ui.prev_line)) && let Some(rect) = ui.hud_last {
            ui.hud_burst = Some((Instant::now(), rect, seed ^ 0x3C));
        }
        let last_sung = active_word.is_some_and(|(k, p)| {
            let n = index.and_then(|i| lyrics.lines.get(i)).map_or(0, |l| l.text.split_whitespace().count());
            k + 1 >= n && p > 0.8
        });
        if last_sung && ui.hud_fired != Some(key) && let Some(rect) = spark_rect {
            ui.hud_fired = Some(key);
            ui.hud_burst = Some((Instant::now(), rect, seed));
        }
        ui.hud_last = spark_rect;
        if let Some((at, rect, seed)) = ui.hud_burst {
            let age = secs(at);
            if age > 1.0 {
                ui.hud_burst = None;
            } else {
                hud_sparks(g, b, rect, age, seed, grad.unwrap_or((accent, ui.active_color())), base.max(0.8));
            }
        }
    }
    if slide > 0.5 {
        b.clip_since(text_start, left - pad, top - pad, left + block_w + pad, top + block_h + pad);
    }
}

/// Sparks on the HUD, bursting from the box `rect` `age` seconds ago.
fn hud_sparks(g: &Game<'_>, b: &mut HudBatch, rect: [f32; 4], age: f32, seed: u64, colors: (u32, u32), scale: f32) {
    let life = 0.8;
    if age >= life {
        return;
    }
    let t = age / life;
    for i in 0..22u64 {
        let r = |k: u64| hash01(seed, i * 7 + k);
        let (sx, sy) = (rect[0] + r(0) * rect[2], rect[1] + r(1) * rect[3]);
        let (dy, dx) = (r(2) * std::f32::consts::TAU).sin_cos();
        let travel = (14.0 + r(3) * 26.0) * scale * (1.0 - (1.0 - t).powi(2));
        let (x, y) = (sx + dx * travel, sy + dy * travel - 6.0 * t * scale);
        let size = (1.0 + r(4) * 1.2) * scale * (1.0 - t);
        let c = art::mix(colors.0, colors.1, r(5));
        let a = (1.0 - t).powf(0.7);
        g.native_quad(b, [[x, y - size], [x + size, y], [x, y + size], [x - size, y]], argb(c, a));
        fill(g, b, x - size * 1.7, y - 0.25, size * 3.4, 0.5, argb(0xFFFFFF, a * 0.6));
    }
}

/// The inset of row `i` (of a box `h` high) at corners of radius `r`.
fn corner_inset(i: f32, h: f32, r: f32) -> f32 {
    if r <= 0.0 {
        return 0.0;
    }
    let d = if i < r { r - i } else if i > h - r { i - (h - r) } else { 0.0 };
    if d <= 0.0 { 0.0 } else { r - (r * r - d * d).max(0.0).sqrt() }
}

/// A box with round corners of radius `r`: the middle in one piece, the
/// corner rows inset one GUI pixel at a time.
fn round_rect(g: &Game<'_>, b: &mut HudBatch, x: f32, y: f32, w: f32, h: f32, r: f32, color: u32) {
    let r = r.clamp(0.0, (w.min(h) / 2.0).max(0.0));
    if r < 1.0 {
        fill(g, b, x, y, w, h, color);
        return;
    }
    let band = r.ceil();
    for i in 0..band as i32 {
        let yy = i as f32;
        let inset = corner_inset(yy + 0.5, h, r);
        fill(g, b, x + inset, y + yy, w - 2.0 * inset, 1.0, color);
        let inset = corner_inset(h - yy - 0.5, h, r);
        fill(g, b, x + inset, y + h - yy - 1.0, w - 2.0 * inset, 1.0, color);
    }
    fill(g, b, x, y + band, w, h - 2.0 * band, color);
}

/// The edge of a `round_rect`, one GUI pixel wide.
fn round_outline(g: &Game<'_>, b: &mut HudBatch, x: f32, y: f32, w: f32, h: f32, r: f32, color: u32) {
    let r = r.clamp(0.0, (w.min(h) / 2.0).max(0.0));
    let rows = h.ceil() as i32;
    let mut prev = corner_inset(0.5, h, r);
    fill(g, b, x + prev, y, w - 2.0 * prev, 1.0, color);
    fill(g, b, x + corner_inset(h - 0.5, h, r), y + h - 1.0, w - 2.0 * corner_inset(h - 0.5, h, r), 1.0, color);
    for i in 1..rows - 1 {
        let inset = corner_inset(i as f32 + 0.5, h, r);
        let next = corner_inset(i as f32 + 1.5, h, r);
        // Wide enough to join the rows above and below.
        let span = (prev - inset).abs().max((next - inset).abs()).max(1.0);
        let lx = x + inset.min(prev.min(next));
        fill(g, b, lx, y + i as f32, span, 1.0, color);
        fill(g, b, x + w - lx + x - span, y + i as f32, span, 1.0, color);
        prev = inset;
    }
}

/// The cover with round corners: strips of the image, each inset as the corners curve.
fn cover_round(g: &Game<'_>, b: &mut HudBatch, ui: &Ui, x: f32, y: f32, side: f32, alpha: f32, round: f32) {
    let r = round.clamp(0.0, 1.0) * side / 2.0;
    if r < 1.0 {
        cover(g, b, ui, x, y, side, alpha);
        return;
    }
    let strips = (side.ceil() as usize).clamp(8, 48);
    let sh = side / strips as f32;
    let image = if ui.art_ready { g.gui.atlas.sprites.get(horizon_client::hud::MOD_IMAGE_SPRITE).copied() } else { None };
    let accent = ui.accent();
    for i in 0..strips {
        let yy = i as f32 * sh;
        let inset = corner_inset(yy + sh / 2.0, side, r);
        let (sx, sw) = (x + inset, side - 2.0 * inset);
        if sw <= 0.0 {
            continue;
        }
        match image {
            Some(full) => {
                let du = (full.u1 - full.u0) / full.w.max(1) as f32;
                let dv = (full.v1 - full.v0) / full.h.max(1) as f32;
                let px = ART as f32 / side;
                let mut part = full;
                part.u0 = full.u0 + inset * px * du;
                part.u1 = full.u0 + (side - inset) * px * du;
                part.v0 = full.v0 + yy * px * dv;
                part.v1 = full.v0 + (yy + sh) * px * dv;
                b.rect_uv(sx, y + yy, sw, sh, part, rgba(255, 255, 255, (alpha.clamp(0.0, 1.0) * 255.0) as u8));
            }
            None => {
                let c = art::mix(art::mix(accent, 0x101010, 0.3), art::mix(accent, 0x101010, 0.8), yy / side);
                fill(g, b, sx, y + yy, sw, sh + 0.02, argb(c, alpha));
            }
        }
    }
    if image.is_none() {
        let note = plain("♪");
        let s = (side / 16.0).max(1.0);
        let w = width(g, &note) * s;
        draw(g, b, &note, x + (side - w) / 2.0, y + (side - 8.0 * s) / 2.0, s, argb(0xFFFFFF, alpha * 0.9), false);
    }
}

/// A rounded pill: `round_rect` with the ends fully round.
fn pill(g: &Game<'_>, b: &mut HudBatch, x: f32, y: f32, w: f32, h: f32, color: u32) {
    round_rect(g, b, x, y, w, h, h / 2.0, color);
}

/// The island: a pill at the top middle with the cover, the line (the sung
/// word tracked by a sliding highlight) and the time, all as set.
fn island(g: &Game<'_>, b: &mut HudBatch, ui: &mut Ui, snap: &Snapshot) {
    let Some(track) = &snap.track else { return };
    let cfg = ui.cfg.clone();
    let s = cfg.island_scale;
    let pos = lyric_pos(&cfg, snap);
    let lyrics = &snap.lyrics;
    let index = lyrics.index_at(pos);
    if index != ui.island_line || track.serial != ui.island_serial {
        // A line that ended before its last word's sparks bursts now.
        if cfg.sparks
            && ui.island_line.is_some()
            && track.serial == ui.island_serial
            && ui.island_fired != Some((ui.island_serial, ui.island_line))
            && let Some(rect) = ui.island_last
        {
            ui.island_burst = Some((Instant::now(), rect, ui.island_serial ^ ui.island_line.unwrap_or(0) as u64));
        }
        ui.island_line = index;
        ui.island_serial = track.serial;
        ui.island_changed = Instant::now();
    }
    let since = secs(ui.island_changed);
    let st = style(&cfg.lyrics_font, cfg.bold_active, cfg.italic);
    let (words, synced): (Vec<String>, bool) = match index.and_then(|i| lyrics.lines.get(i)) {
        Some(l) if !l.text.trim().is_empty() => (lyric_text(&cfg, &l.text).split_whitespace().map(str::to_owned).collect(), true),
        _ => (format!("{} – {}", track.title, track.artist).split_whitespace().map(str::to_owned).collect(), false),
    };
    let text_scale = 0.9 * s;
    let text_w = words.iter().map(|w| width(g, &text(w, &st))).sum::<f32>() * text_scale
        + width(g, &text(" ", &st)) * text_scale * words.len().saturating_sub(1) as f32;
    let h = 22.0 * s * cfg.island_height;
    let radius = cfg.island_round * h / 2.0;
    let cover_side = if cfg.island_cover { 14.0 * s * cfg.island_height.sqrt() } else { 0.0 };
    let time = format!("{} / {}", time(snap.position_ms), if track.duration_ms > 0 { self::time(track.duration_ms) } else { "--:--".into() });
    let time_scale = 0.6 * s;
    let time_w = if cfg.island_time { width(g, &plain(&time)) * time_scale } else { 0.0 };
    let edge = radius.max(6.0 * s);
    let target = ((text_w * 1.12 + cover_side + time_w + 2.0 * edge + 12.0 * s) * cfg.island_width).clamp(80.0 * s, b.width * 0.95);
    // The pill grows and shrinks smoothly with the line.
    let k = 1.0 - (-ui.dt * 9.0).exp();
    ui.island_w = if ui.island_w <= 0.0 { target } else { ui.island_w + (target - ui.island_w) * k };
    let w = ui.island_w;
    let (x, y) = ((b.width - w) / 2.0, cfg.island_y.min(b.height - h));
    // Solid, see-through or clear, with an outline if set.
    if cfg.island_bg > 0.01 {
        round_rect(g, b, x, y, w, h, radius, argb(cfg.island_bg_color, cfg.island_bg));
    }
    if cfg.island_outline > 0.01 {
        round_outline(g, b, x, y, w, h, radius, argb(cfg.island_outline_color, cfg.island_outline));
    }
    let accent = ui.accent();
    let grad = gradient(ui);
    let inner = edge.min(h / 2.0).max(4.0 * s);
    let mut left = x + inner;
    if cfg.island_cover {
        let cy = y + (h - cover_side) / 2.0;
        cover_round(g, b, ui, left, cy, cover_side, 1.0, cfg.art_round.max(cfg.island_round * 0.6));
        left += cover_side + 6.0 * s;
    }
    let mut right = x + w - inner;
    if cfg.island_time {
        let tx = right - time_w;
        draw(g, b, &plain(&time), tx, y + (h - 8.0 * time_scale) / 2.0, time_scale, argb(0xB3B3B3, 1.0), false);
        right = tx - 6.0 * s;
    }
    // Progress along the bottom, inside the round ends.
    if cfg.island_progress && track.duration_ms > 0 {
        let p = (snap.position_ms as f32 / track.duration_ms as f32).clamp(0.0, 1.0);
        let (bx, bw) = (x + inner, w - 2.0 * inner);
        fill(g, b, bx, y + h - 2.5 * s, bw, 1.0 * s, argb(0x333333, 0.8));
        let start = b.vertices.len();
        fill(g, b, bx, y + h - 2.5 * s, bw * p, 1.0 * s, argb(if grad.is_some() { 0xFFFFFF } else { accent }, 1.0));
        if let Some((c0, c1)) = grad {
            recolor(b, start, bx, bx + bw, c0, c1);
        }
    }
    // The line, sliding in, the sung word tracked.
    let room = (right - left).max(10.0);
    let fit = (room / text_w.max(1.0)).min(1.0);
    let tscale = text_scale * fit.max(0.55);
    let t = ease_out(since / 0.3);
    let sung = index.filter(|_| synced).and_then(|i| lyrics.word_at(i, pos, track.duration_ms));
    let progress = index.filter(|_| synced).map_or(0.0, |i| lyrics.progress(i, pos, track.duration_ms));
    let base = if grad.is_some() { 0xFFFFFF } else { ui.active_color() };
    let (sung_c, hi, dim) = (argb(base, t), argb(if grad.is_some() { 0xFFFFFF } else { accent }, t), argb(base, 0.45 * t));
    let style_ = if cfg.highlight == Highlight::OneWord { Highlight::Bounce } else { cfg.highlight };
    // One row in the island: its motion and vanish, not the arrangement.
    let mut fx = LineFx::of(ui, sung, progress, track.serial ^ index.unwrap_or(0) as u64, f32::MAX);
    fx.arrangement = Arrangement::Line;
    let lay = match sung {
        Some((k, p)) => place(g, &words, &st, &|i| word_look(style_, i, k, p, sung_c, hi, dim), &fx),
        None => place(g, &words, &st, &|_| Look::plain(sung_c), &fx),
    };
    let ox = (left + right) / 2.0 - (lay.bounds[0] + lay.bounds[2]) / 2.0 * tscale;
    let oy = y + h / 2.0 - 0.5 * s + (1.0 - t) * 6.0 * s;
    let start = b.vertices.len();
    // The highlight under the sung word, sliding to it.
    if let Some((k, _)) = sung {
        let (mut wx0, mut wx1) = (f32::MAX, f32::MIN);
        for u in lay.units.iter().filter(|u| u.word == k) {
            wx0 = wx0.min(u.cx - u.w * u.scale / 2.0);
            wx1 = wx1.max(u.cx + u.w * u.scale / 2.0);
        }
        if wx1 > wx0 {
            let (rx, rw) = (ox + wx0 * tscale, (wx1 - wx0) * tscale);
            let k2 = 1.0 - (-ui.dt * 14.0).exp();
            let [hx, hw] = ui.island_hl;
            ui.island_hl = if hw <= 0.0 { [rx, rw] } else { [hx + (rx - hx) * k2, hw + (rw - hw) * k2] };
            let [hx, hw] = ui.island_hl;
            let hh = (h - 8.0 * s).max(6.0);
            round_rect(g, b, hx - 2.0 * s, y + (h - hh) / 2.0, hw + 4.0 * s, hh, hh * 0.3 * cfg.island_round.max(0.3), argb(0xFFFFFF, 0.14 * t));
        }
    }
    draw_line_hud(g, b, &lay, &st, ox, oy, tscale, false, grad);
    b.clip_since(start, left - 2.0, y, right + 2.0, y + h);
    // Sparks from the last word as it's sung.
    let last = words.len().saturating_sub(1);
    let rect = lay.words.get(last).map(|&[wx, wy]| [ox + wx * tscale - 5.0 * s, oy + wy * tscale - 4.0 * s, 10.0 * s, 8.0 * s]);
    if cfg.sparks && synced {
        let key = (track.serial, index);
        if ui.island_fired != Some(key)
            && sung.is_some_and(|(k, p)| k >= last && p > 0.8)
            && let Some(r) = rect
        {
            ui.island_fired = Some(key);
            ui.island_burst = Some((Instant::now(), r, track.serial ^ index.unwrap_or(0) as u64));
        }
    }
    ui.island_last = rect;
    if cfg.sparks
        && let Some((burst, r, seed)) = ui.island_burst
    {
        let age = secs(burst);
        if age > 1.0 {
            ui.island_burst = None;
        } else {
            hud_sparks(g, b, r, age, seed, grad.unwrap_or((accent, ui.active_color())), s);
        }
    }
}

/// The now-playing toast: slides in with the cover, holds, slides out.
fn toast(g: &Game<'_>, b: &mut HudBatch, ui: &mut Ui, snap: &Snapshot) {
    let Some(track) = &snap.track else {
        return;
    };
    if track.serial != ui.toast_serial {
        ui.toast_serial = track.serial;
        ui.toast_start = Some(Instant::now());
    }
    let Some(start) = ui.toast_start else { return };
    let t = secs(start);
    let hold = ui.cfg.toast_seconds;
    if t > hold + 0.6 {
        ui.toast_start = None;
        return;
    }
    let shown = if t < 0.35 {
        ease_out(t / 0.35)
    } else if t > hold + 0.25 {
        1.0 - ease_out((t - hold - 0.25) / 0.35)
    } else {
        1.0
    };
    let (w, h) = (168.0, 36.0);
    let (bx, by) = match ui.cfg.toast_corner {
        ToastCorner::TopLeft => (4.0, 4.0),
        ToastCorner::BottomLeft => (4.0, b.height - h - 4.0),
        ToastCorner::TopRight => (b.width - w - 4.0, 4.0),
    };
    let from_left = ui.cfg.toast_corner != ToastCorner::TopRight;
    let x = if from_left { bx - (w + 8.0) * (1.0 - shown) } else { bx + (w + 8.0) * (1.0 - shown) };
    let y = by;
    let a = shown;
    let accent = ui.accent();
    let radius = ui.cfg.widget_round * h / 2.0;
    round_rect(g, b, x, y, w, h, radius, argb(ui.panel(), 0.92 * a));
    if radius < 4.0 {
        fill(g, b, x, y + 1.0, 2.0, h - 2.0, argb(accent, a));
    } else {
        round_outline(g, b, x, y, w, h, radius, argb(accent, 0.6 * a));
    }
    cover_round(g, b, ui, x + 4.0 + radius * 0.25, y + 4.0, 28.0, a, ui.cfg.art_round);
    let tx = x + 39.0 + radius * 0.25;
    let max = w - 44.0 - radius * 0.5;
    draw(g, b, &text("NOW PLAYING", &style("minecraft:default", true, false)), tx, y + 4.0, 0.6, argb(accent, a), false);
    let clock = t;
    marquee(g, b, &text(&track.title, &style(&ui.cfg.title_font, true, false)), tx, y + 11.0, 1.0, max, argb(0xFFFFFF, a), true, clock);
    marquee(g, b, &text(&track.artist, &style(&ui.cfg.title_font, false, false)), tx, y + 22.0, 0.85, max, argb(0xB3B3B3, a), true, clock);
}

/// The layout editor's frames and hints, over the HUD.
fn layout_overlay(g: &Game<'_>, b: &mut HudBatch, ui: &Ui) {
    let accent = ui.accent();
    for (r, label) in [(ui.lyrics_rect, "Lyrics"), (ui.widget_rect, "Player")] {
        if r[2] > 0.0 {
            outline(g, b, r[0] - 1.0, r[1] - 1.0, r[2] + 2.0, r[3] + 2.0, argb(accent, 1.0));
            draw(g, b, &plain(label), r[0], r[1] - 10.0, 0.75, argb(accent, 1.0), true);
        }
    }
}

// ---------------------------------------------------------------- hook: world (3D, elytra)

fn forward(yaw: f32, pitch: f32) -> [f64; 3] {
    let (y, p) = ((yaw as f64).to_radians(), (pitch as f64).to_radians());
    [-y.sin() * p.cos(), -p.sin(), y.cos() * p.cos()]
}
fn add(a: [f64; 3], b: [f64; 3], s: f64) -> [f64; 3] {
    [a[0] + b[0] * s, a[1] + b[1] * s, a[2] + b[2] * s]
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn len(a: [f64; 3]) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}
fn lerp3(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    add(a, sub(b, a), t)
}

/// Full sky and block light: lyrics glow as text displays with glowing text do.
const FULL_LIGHT: u32 = 240 | 240 << 8;

/// `batch` (font pixels, y down) into world vertices under `pose`, as
/// Horizon's own text displays are built, so shader packs draw them as text.
fn emit(out: &mut Vec<Vertex>, pose: glam::Mat4, batch: &HudBatch, z: f32) {
    if !pose.is_finite() {
        return;
    }
    for quad in batch.vertices.chunks_exact(6) {
        for i in horizon_client::text_display::CORNERS {
            let v = quad[i];
            let p = pose.transform_point3(glam::Vec3::new(v.pos[0], v.pos[1], v.depth + z));
            out.push(Vertex {
                pos: p.to_array(),
                uv: v.uv,
                layer: horizon_client::gfx::TEXT_POLYGON_OFFSET_OVERLAY,
                color: v.color,
                light: FULL_LIGHT,
                normal: [0; 4],
                glint_uv: [0.0, 0.0],
            });
        }
    }
}

/// How one word of a world line looks this frame.
#[derive(Clone, Copy)]
struct Look {
    /// ARGB.
    color: u32,
    scale: f32,
    /// Font pixels up (a jump).
    lift: f32,
    /// A bar under it: the word being sung.
    underline: bool,
}

impl Look {
    fn plain(color: u32) -> Self {
        Self { color, scale: 1.0, lift: 0.0, underline: false }
    }
}

/// A number in 0..1 from a seed and an index: the same each frame.
fn hash01(seed: u64, i: u64) -> f32 {
    let mut x = seed ^ i.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    x = x.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    x ^= x >> 33;
    (x >> 40) as f32 / (1u64 << 24) as f32
}

/// The gradient's colours: the album's (light, readable) or the chosen pair.
fn gradient(ui: &Ui) -> Option<(u32, u32)> {
    let cfg = &ui.cfg;
    if !cfg.gradient {
        return None;
    }
    if cfg.gradient_album && ui.art_ready {
        let a = art::mix(ui.palette.accent, 0xFFFFFF, 0.2);
        let (h, s, v) = art::to_hsv(ui.palette.dominant);
        // The cover's second colour, kept bright; a hue turn when it's grey.
        let b = if s < 0.2 {
            let (ha, sa, _) = art::to_hsv(ui.palette.accent);
            art::hsv_rgb(ha + 45.0, sa.max(0.5), 0.95)
        } else {
            art::hsv_rgb(h, s.max(0.45), v.max(0.85))
        };
        return Some((a, b));
    }
    Some((cfg.gradient_a, cfg.gradient_b))
}

/// Multiplies the colours of `b`'s vertices from `start` by the gradient from
/// `c0` at `x0` to `c1` at `x1`, keeping their alpha (outlines stay dark).
fn recolor(b: &mut HudBatch, start: usize, x0: f32, x1: f32, c0: u32, c1: u32) {
    let span = (x1 - x0).max(0.001);
    for v in &mut b.vertices[start..] {
        let t = ((v.pos[0] - x0) / span).clamp(0.0, 1.0);
        let g = art::mix(c0, c1, t);
        let [r, gg, bl] = [(g >> 16) & 255, (g >> 8) & 255, g & 255];
        let c = v.color;
        let (vr, vg, vb, va) = (c & 255, (c >> 8) & 255, (c >> 16) & 255, c >> 24);
        v.color = va << 24 | (vb * bl / 255) << 16 | (vg * gg / 255) << 8 | (vr * r / 255);
    }
}

/// The pose of a line `at` (camera-relative), turned to `yaw`/`pitch`, `px` blocks a font pixel.
fn line_pose(camera: [f64; 3], at: [f64; 3], yaw: f32, pitch: Option<f32>, px: f32) -> (glam::Mat4, [f64; 3]) {
    let offset = sub(at, camera);
    let mut pose = glam::Mat4::from_translation(glam::Vec3::new(offset[0] as f32, offset[1] as f32, offset[2] as f32))
        * glam::Mat4::from_quat(glam::Quat::from_axis_angle(glam::Vec3::Y, (180.0 - yaw).to_radians()));
    if let Some(p) = pitch {
        pose *= glam::Mat4::from_quat(glam::Quat::from_axis_angle(glam::Vec3::X, (-p).to_radians()));
    }
    (pose * glam::Mat4::from_scale(glam::Vec3::new(px, -px, px)), offset)
}

/// `pose` turned round when the camera is behind it (nothing culls these
/// quads, so the far face would show mirrored); None when hidden.
fn facing_side(pose: glam::Mat4, offset: [f64; 3], two_sided: bool) -> Option<glam::Mat4> {
    let normal = pose.transform_vector3(glam::Vec3::Z);
    let behind = normal.dot(glam::Vec3::new(-offset[0] as f32, -offset[1] as f32, -offset[2] as f32)) < 0.0;
    match (behind, two_sided) {
        (false, _) => Some(pose),
        (true, true) => Some(pose * glam::Mat4::from_quat(glam::Quat::from_axis_angle(glam::Vec3::Y, std::f32::consts::PI))),
        (true, false) => None,
    }
}

/// How a line's words are laid out and move this frame.
#[derive(Clone, Copy)]
struct LineFx {
    arrangement: Arrangement,
    /// 0 to 1: how far the arrangement goes.
    amount: f32,
    motion: Motion,
    vanish: Vanish,
    /// The word being sung and how far through it.
    sung: Option<(usize, f32)>,
    highlight: Highlight,
    /// How far through the line (the wipe).
    progress: f32,
    clock: f32,
    /// Keeps a line's scramble the same from frame to frame.
    seed: u64,
    /// Wrap width, font pixels.
    max_w: f32,
    /// Gaps between words (1 as set; less in tight spaces).
    squeeze: f32,
}

impl LineFx {
    fn plain(clock: f32, max_w: f32) -> Self {
        Self {
            arrangement: Arrangement::Line,
            amount: 0.0,
            motion: Motion::Still,
            vanish: Vanish::Off,
            sung: None,
            highlight: Highlight::Solid,
            progress: 0.0,
            clock,
            seed: 0,
            max_w,
            squeeze: 1.0,
        }
    }
    /// The settings' word effects for a line.
    fn of(ui: &Ui, sung: Option<(usize, f32)>, progress: f32, seed: u64, max_w: f32) -> Self {
        let (arrangement, amount) = ui.arrangement();
        Self {
            arrangement,
            amount,
            motion: ui.cfg.motion,
            vanish: ui.cfg.vanish,
            sung,
            highlight: ui.cfg.highlight,
            progress,
            clock: secs(ui.started),
            seed,
            max_w,
            squeeze: 1.0,
        }
    }
}

/// One drawn piece of a line (a word, or a letter when the text waves),
/// placed: its centre in font pixels from the line's centre (y down), its
/// natural width, size and tilt (degrees, clockwise).
struct Placed {
    text: String,
    word: usize,
    cx: f32,
    cy: f32,
    w: f32,
    scale: f32,
    tilt: f32,
    color: u32,
    underline: bool,
}

/// A placed line: its pieces, their bounds `[x0, y0, x1, y1]` and each word's centre.
struct Layout {
    units: Vec<Placed>,
    bounds: [f32; 4],
    words: Vec<[f32; 2]>,
}

/// Rows of words (by index) no wider than `max`.
fn wrap_rows(ww: &[f32], gap: f32, max: f32) -> Vec<std::ops::Range<usize>> {
    let mut rows = Vec::new();
    let (mut start, mut w) = (0usize, 0.0f32);
    for (i, &x) in ww.iter().enumerate() {
        if i > start && w + gap + x > max {
            rows.push(start..i);
            start = i;
            w = x;
        } else {
            w += if i > start { gap + x } else { x };
        }
    }
    if start < ww.len() || rows.is_empty() {
        rows.push(start..ww.len());
    }
    rows
}

fn rotate(p: [f32; 2], deg: f32) -> [f32; 2] {
    let (s, c) = deg.to_radians().sin_cos();
    [p[0] * c - p[1] * s, p[0] * s + p[1] * c]
}

fn with_alpha(c: u32, f: f32) -> u32 {
    let a = ((c >> 24) as f32 * f.clamp(0.0, 1.0)).round() as u32;
    a << 24 | (c & 0xFF_FFFF)
}

/// Lays out `words`: the arrangement, the kick, the motion and the vanish.
fn place(g: &Game<'_>, words: &[String], st: &Style, look: &dyn Fn(usize) -> Look, fx: &LineFx) -> Layout {
    let n = words.len();
    let a = fx.amount.clamp(0.0, 1.0);
    let looks: Vec<Look> = (0..n).map(look).collect();
    let nat: Vec<f32> = words.iter().map(|w| width(g, &text(w, st))).collect();
    // A word drawn bigger takes its bigger width, so the gaps stay even.
    let ww: Vec<f32> = nat.iter().zip(&looks).map(|(w, l)| w * l.scale.max(1.0)).collect();
    let space = width(g, &text(" ", st)).max(2.0);
    let gap = space
        * fx.squeeze
        * match fx.arrangement {
            Arrangement::Spread => 1.8 + 3.5 * a,
            Arrangement::Scramble => 1.0 + 2.5 * a,
            Arrangement::Split | Arrangement::Arc => 1.0 + a,
            _ => 1.0,
        };
    let row_w = |r: &std::ops::Range<usize>| ww[r.clone()].iter().sum::<f32>() + gap * r.len().saturating_sub(1) as f32;
    let flow = |r: std::ops::Range<usize>, left: f32, y: f32, centre: &mut [[f32; 2]]| {
        let mut x = left;
        for i in r {
            centre[i] = [x + ww[i] / 2.0, y];
            x += ww[i] + gap;
        }
    };
    let h = |i: usize, k: u64| hash01(fx.seed, i as u64 * 5 + k) * 2.0 - 1.0;
    let mut centre = vec![[0.0f32; 2]; n];
    let mut tilt = vec![0.0f32; n];
    let row_h = 12.0;
    match fx.arrangement {
        Arrangement::Line | Arrangement::Spread | Arrangement::Scramble | Arrangement::Stairs => {
            let grow = match fx.arrangement {
                Arrangement::Scramble => 1.0 + 0.4 * a,
                Arrangement::Spread => 1.0 + 0.3 * a,
                _ => 1.0,
            };
            let rows = wrap_rows(&ww, gap, fx.max_w / grow);
            let k = rows.len() as f32;
            let rh = row_h * if fx.arrangement == Arrangement::Scramble { 1.0 + 0.6 * a } else { 1.0 };
            for (ri, r) in rows.into_iter().enumerate() {
                let y = (ri as f32 - (k - 1.0) / 2.0) * rh;
                let w = row_w(&r);
                if fx.arrangement == Arrangement::Stairs {
                    let m = r.len() as f32;
                    let first = r.start;
                    flow(r.clone(), -w / 2.0, y, &mut centre);
                    for i in r {
                        centre[i][1] += ((i - first) as f32 - (m - 1.0) / 2.0) * (3.0 + 7.0 * a);
                    }
                } else {
                    flow(r, -w / 2.0, y, &mut centre);
                }
            }
            if fx.arrangement == Arrangement::Scramble {
                for i in 0..n {
                    centre[i][0] += h(i, 0) * 6.0 * a;
                    centre[i][1] += h(i, 1) * 9.0 * a;
                    tilt[i] = h(i, 2) * 16.0 * a;
                }
            }
        }
        Arrangement::Split => {
            // Two groups either side of a clear middle (the crosshair).
            let mid = 50.0 + 110.0 * a;
            if n == 1 {
                centre[0] = [0.0, -16.0 - 10.0 * a];
            } else {
                let cut = n / 2;
                for (range, left_side) in [(0..cut, true), (cut..n, false)] {
                    let sub: Vec<f32> = ww[range.clone()].to_vec();
                    let rows = wrap_rows(&sub, gap, fx.max_w * 0.55);
                    let k = rows.len() as f32;
                    for (ri, r) in rows.into_iter().enumerate() {
                        let r = r.start + range.start..r.end + range.start;
                        let y = (ri as f32 - (k - 1.0) / 2.0) * row_h;
                        let w = row_w(&r);
                        let left = if left_side { -mid / 2.0 - w } else { mid / 2.0 };
                        flow(r.clone(), left, y, &mut centre);
                        for i in r {
                            tilt[i] = if left_side { 3.0 } else { -3.0 };
                        }
                    }
                }
            }
        }
        Arrangement::Stack => {
            for i in 0..n {
                let side = if n > 1 { (i % 2) as f32 * 2.0 - 1.0 } else { 0.0 };
                centre[i] = [side * a * 14.0, (i as f32 - (n as f32 - 1.0) / 2.0) * row_h * (1.0 + 0.25 * a)];
                tilt[i] = side * a * 5.0;
            }
        }
        Arrangement::Arc => {
            let all = 0..n;
            let total = row_w(&all);
            flow(all, -total / 2.0, 0.0, &mut centre);
            let radius = total.max(60.0) / (1.0 + 2.2 * a);
            let sag = radius * (1.0 - (total / 2.0 / radius).min(std::f32::consts::FRAC_PI_2 * 1.4).cos());
            for i in 0..n {
                let th = (centre[i][0] / radius).clamp(-2.2, 2.2);
                centre[i] = [radius * th.sin(), -radius * (1.0 - th.cos()) + sag / 2.0];
                tilt[i] = -th.to_degrees();
            }
        }
        Arrangement::Aura => {
            // Round you: an ellipse wide enough for the words, turning slowly.
            let total = row_w(&(0..n));
            let mut rx = 58.0 + 70.0 * a;
            let around = |rx: f32| std::f32::consts::TAU * ((rx * rx + (rx * 0.55).powi(2)) / 2.0).sqrt();
            while around(rx) < total * 1.15 && rx < 400.0 {
                rx *= 1.15;
            }
            let ry = rx * 0.55;
            for i in 0..n {
                let th = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * i as f32 / n.max(1) as f32 + fx.clock * 0.15;
                centre[i] = [rx * th.cos(), ry * th.sin()];
            }
        }
    }
    // The kick: the line rocks, right side up, then left, word by word.
    if fx.highlight == Highlight::Kick
        && let Some((k, p)) = fx.sung
    {
        let angle = if k % 2 == 0 { -9.0 } else { 9.0 } * jump(p);
        for i in 0..n {
            centre[i] = rotate(centre[i], angle);
            tilt[i] += angle;
        }
    }
    let mut units = Vec::with_capacity(n);
    let mut word_centres = Vec::with_capacity(n);
    for i in 0..n {
        let l = looks[i];
        let mut sc = l.scale;
        let [mut cx, mut cy] = centre[i];
        match fx.motion {
            Motion::Float => {
                cy += (fx.clock * 1.7 + i as f32 * 1.1).sin() * 1.6;
                cx += (fx.clock * 1.3 + i as f32 * 0.7).cos() * 0.8;
            }
            Motion::Breathe => sc *= 1.0 + 0.05 * (fx.clock * 2.4 + i as f32 * 0.6).sin(),
            _ => {}
        }
        cy -= l.lift;
        word_centres.push([cx, cy]);
        if fx.motion == Motion::Wave {
            // Letter by letter, along the word's own tilt, on a travelling wave.
            let mut off = -nat[i] / 2.0;
            for ch in words[i].chars() {
                let cs = ch.to_string();
                let cw = width(g, &text(&cs, st));
                let o = rotate([(off + cw / 2.0) * sc, 0.0], tilt[i]);
                let (x, y) = (cx + o[0], cy + o[1]);
                let phase = fx.clock * 4.2 - x * 0.09;
                units.push(Placed { text: cs, word: i, cx: x, cy: y + phase.sin() * 2.2, w: cw, scale: sc, tilt: tilt[i] + phase.cos() * 6.0, color: l.color, underline: l.underline });
                off += cw;
            }
        } else {
            units.push(Placed { text: words[i].clone(), word: i, cx, cy, w: nat[i], scale: sc, tilt: tilt[i], color: l.color, underline: l.underline });
        }
    }
    let mut bounds = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for u in &units {
        let (hw, hh) = (u.w * u.scale / 2.0, 4.5 * u.scale);
        bounds = [bounds[0].min(u.cx - hw), bounds[1].min(u.cy - hh), bounds[2].max(u.cx + hw), bounds[3].max(u.cy + hh + 1.0)];
    }
    if units.is_empty() {
        bounds = [0.0, -4.5, 0.0, 4.5];
    }
    match fx.vanish {
        Vanish::Off => {}
        Vanish::Sung => {
            if let Some((k, p)) = fx.sung {
                for u in &mut units {
                    if u.word < k {
                        let f = if u.word + 1 == k { 1.0 - ease_out((p * 3.0).min(1.0)) } else { 0.0 };
                        u.color = with_alpha(u.color, f);
                    }
                }
            }
        }
        Vanish::Wipe => {
            let span = (bounds[2] - bounds[0]).max(1.0);
            let cut = fx.progress * 1.2 - 0.1;
            for u in &mut units {
                let xn = (u.cx - bounds[0]) / span;
                u.color = with_alpha(u.color, ((xn - cut) / 0.12 + 0.5).clamp(0.0, 1.0));
            }
        }
    }
    Layout { units, bounds, words: word_centres }
}

/// Turns the vertices from `start` by `deg` (clockwise) about (cx, cy).
fn rotate_since(b: &mut HudBatch, start: usize, cx: f32, cy: f32, deg: f32) {
    let (s, c) = deg.to_radians().sin_cos();
    for v in &mut b.vertices[start..] {
        let (x, y) = (v.pos[0] - cx, v.pos[1] - cy);
        v.pos[0] = cx + x * c - y * s;
        v.pos[1] = cy + x * s + y * c;
    }
}

/// A placed line on the HUD, its centre at (ox, oy), `scale` GUI pixels a font pixel.
#[allow(clippy::too_many_arguments)]
fn draw_line_hud(g: &Game<'_>, b: &mut HudBatch, lay: &Layout, st: &Style, ox: f32, oy: f32, scale: f32, shadow: bool, grad: Option<(u32, u32)>) {
    let start = b.vertices.len();
    for u in &lay.units {
        if (u.color >> 24) < 6 {
            continue;
        }
        let s = scale * u.scale;
        let (cx, cy) = (ox + u.cx * scale, oy + u.cy * scale);
        let from = b.vertices.len();
        draw(g, b, &text(&u.text, st), cx - u.w * s / 2.0, cy - 4.5 * s, s, u.color, shadow);
        if u.underline {
            fill(g, b, cx - u.w * s / 2.0, cy + 5.5 * s, u.w * s, (0.8 * s).max(1.0), argb(0xFFFFFF, (u.color >> 24) as f32 / 255.0));
        }
        if u.tilt.abs() > 0.05 {
            rotate_since(b, from, cx, cy, u.tilt);
        }
    }
    if let Some((c0, c1)) = grad {
        recolor(b, start, ox + lay.bounds[0] * scale, ox + lay.bounds[2] * scale, c0, c1);
    }
}

/// A placed line in the world at `at` (the line's centre).
#[allow(clippy::too_many_arguments)]
fn draw_line_world(
    g: &Game<'_>,
    out: &mut Vec<Vertex>,
    ui: &Ui,
    camera: [f64; 3],
    at: [f64; 3],
    yaw: f32,
    pitch: Option<f32>,
    px: f32,
    st: &Style,
    lay: &Layout,
    art: Option<f32>,
    grad: Option<(u32, u32)>,
) {
    let cfg = &ui.cfg;
    let (pose, offset) = line_pose(camera, at, yaw, pitch, px);
    let Some(side) = facing_side(pose, offset, cfg.two_sided_3d) else { return };
    let [x0, y0, x1, y1] = lay.bounds;
    let art_side = 22.0;
    let mid_y = (y0 + y1) / 2.0;
    let left = if art.is_some() { x0 - art_side - 5.0 } else { x0 };
    if let Some(alpha) = art {
        let mut b = HudBatch::new([1, 1]);
        cover(g, &mut b, ui, left, mid_y - art_side / 2.0, art_side, alpha);
        emit(out, side, &b, -0.02);
    }
    if cfg.background_3d > 0.01 {
        let mut b = HudBatch::new([1, 1]);
        let h = (y1 - y0).max(if art.is_some() { art_side } else { 9.0 });
        fill_white(g, &mut b, left - 3.0, mid_y - h / 2.0 - 2.0, x1 - left + 6.0, h + 4.0, argb(0x101010, cfg.background_3d));
        emit(out, side, &b, -0.08);
    }
    let outline = cfg.outline_3d || cfg.highlight == Highlight::Glow;
    for u in &lay.units {
        if (u.color >> 24) < 6 || u.text.is_empty() {
            continue;
        }
        let m = side
            * glam::Mat4::from_translation(glam::Vec3::new(u.cx, u.cy, 0.0))
            * glam::Mat4::from_quat(glam::Quat::from_axis_angle(glam::Vec3::Z, u.tilt.to_radians()))
            * glam::Mat4::from_scale(glam::Vec3::new(u.scale, u.scale, 1.0))
            * glam::Mat4::from_translation(glam::Vec3::new(-u.w / 2.0, -4.5, 0.0));
        let t = text(&u.text, st);
        let alpha = (u.color >> 24) as f32 / 255.0;
        if outline {
            let ink = if cfg.highlight == Highlight::Glow { argb(ui.accent(), 0.85 * alpha) } else { argb(0x000000, 0.7 * alpha) };
            let mut b = HudBatch::new([1, 1]);
            for (ox, oy) in [(-0.75, 0.0), (0.75, 0.0), (0.0, -0.75), (0.0, 0.75)] {
                draw(g, &mut b, &t, ox, oy, 1.0, ink, false);
            }
            emit(out, m, &b, -0.05);
        }
        let mut b = HudBatch::new([1, 1]);
        draw(g, &mut b, &t, 0.0, 0.0, 1.0, u.color, false);
        if u.underline {
            fill_white(g, &mut b, 0.0, 10.0, u.w, 0.9, argb(0xFFFFFF, alpha));
        }
        if let Some((c0, c1)) = grad {
            // Across the whole line, so the colours flow from word to word.
            let shift = u.cx - u.w / 2.0;
            recolor(&mut b, 0, x0 - shift, x1 - shift, c0, c1);
        }
        emit(out, m, &b, 0.0);
    }
}

/// Sparks bursting from `origin` (font pixels in the line at `at`), `age` seconds old.
#[allow(clippy::too_many_arguments)]
fn world_sparks(g: &Game<'_>, out: &mut Vec<Vertex>, camera: [f64; 3], burst: &WBurst, age: f32, colors: (u32, u32), two_sided: bool) {
    let life = 0.9;
    if age >= life {
        return;
    }
    let (pose, offset) = line_pose(camera, burst.at, burst.yaw, burst.pitch, burst.px);
    let Some(side) = facing_side(pose, offset, two_sided) else { return };
    let t = age / life;
    let seed = burst.seed;
    for i in 0..30u64 {
        let r = |k: u64| hash01(seed, i * 7 + k);
        // From the word, outward and a little up, slowing down.
        let (sx, sy) = (burst.origin[0] + (r(0) * 2.0 - 1.0) * 7.0, burst.origin[1] + (r(1) * 2.0 - 1.0) * 4.0);
        let angle = (r(2) * std::f32::consts::TAU).sin_cos();
        let speed = 22.0 + r(3) * 50.0;
        let travel = speed * (1.0 - (1.0 - t).powi(2)) * life;
        let (x, y) = (sx + angle.1 * travel, sy + angle.0 * travel - 12.0 * t);
        let size = (1.6 + r(4) * 1.4) * (1.0 - t);
        // The gradient's colours, a few sparks near white.
        let c = if r(7) < 0.2 { art::mix(colors.1, 0xFFFFFF, 0.6) } else { art::mix(colors.0, colors.1, r(5)) };
        let a = (1.0 - t).powf(0.7) * if (age * 18.0 + r(6) * 6.0).sin() > -0.3 { 1.0 } else { 0.35 };
        let mut b = HudBatch::new([1, 1]);
        fill_white(g, &mut b, -size / 2.0, -size / 2.0, size, size, argb(c, a));
        // A diamond: the square turned 45°, and a thin cross for the twinkle.
        let m = side
            * glam::Mat4::from_translation(glam::Vec3::new(x, y, 0.1))
            * glam::Mat4::from_quat(glam::Quat::from_axis_angle(glam::Vec3::Z, std::f32::consts::FRAC_PI_4));
        emit(out, m, &b, 0.0);
        let mut cross = HudBatch::new([1, 1]);
        fill_white(g, &mut cross, -size * 1.6, -0.15, size * 3.2, 0.3, argb(c, a * 0.8));
        fill_white(g, &mut cross, -0.15, -size * 1.6, 0.3, size * 3.2, argb(c, a * 0.8));
        emit(out, side * glam::Mat4::from_translation(glam::Vec3::new(x, y, 0.12)), &cross, 0.0);
    }
}

/// A plain rectangle from the white texels beside the cover.
fn fill_white(g: &Game<'_>, b: &mut HudBatch, x: f32, y: f32, w: f32, h: f32, color: u32) {
    if let Some(mut r) = g.gui.atlas.sprites.get(horizon_client::hud::MOD_IMAGE_SPRITE).copied() {
        let du = (r.u1 - r.u0) / r.w.max(1) as f32;
        let dv = (r.v1 - r.v0) / r.h.max(1) as f32;
        let [x0, y0, x1, y1] = glyphs::WHITE;
        let (u0, v0) = (r.u0, r.v0);
        r.u0 = u0 + x0 as f32 * du;
        r.v0 = v0 + y0 as f32 * dv;
        r.u1 = u0 + x1 as f32 * du;
        r.v1 = v0 + y1 as f32 * dv;
        b.rect_uv(x, y, w, h, r, packed(color));
    }
}

/// How far into a word's moment `p` (0 to 1) the jump is: up fast, down soft.
fn jump(p: f32) -> f32 {
    let p = (p * 2.2).clamp(0.0, 1.0);
    (p * std::f32::consts::PI).sin() * (1.0 - p * 0.35)
}

/// Each word's look for a highlight style: `k` the word sung, `p` how far
/// through it, the sung colour, the highlight colour and the dim colour.
fn word_look(style: Highlight, i: usize, k: usize, p: f32, sung: u32, hi: u32, dim: u32) -> Look {
    use std::cmp::Ordering::*;
    match (style, i.cmp(&k)) {
        (Highlight::Kick, Equal) => Look { color: hi, scale: 1.0 + 0.22 * (1.0 - ease_out(p * 2.5)) + 0.06, lift: 0.0, underline: false },
        (Highlight::Kick, Less) => Look::plain(sung),
        (Highlight::Kick, Greater) => Look::plain(dim),
        (Highlight::Bounce, Equal) => Look { color: hi, scale: 1.0 + 0.3 * (1.0 - ease_out(p * 2.5)) + 0.08, lift: 4.0 * jump(p), underline: false },
        (Highlight::Bounce, Less) => Look::plain(sung),
        (Highlight::Bounce, Greater) => Look::plain(dim),
        (Highlight::Word | Highlight::OneWord, Equal) => Look { color: hi, scale: 1.0 + 0.28 * (1.0 - ease_out(p * 3.0)) + 0.1, lift: 0.0, underline: true },
        (Highlight::Word | Highlight::OneWord, Less) => Look::plain(sung),
        (Highlight::Word | Highlight::OneWord, Greater) => Look::plain(dim),
        (Highlight::Sweep, Less | Equal) => Look::plain(sung),
        (Highlight::Sweep, Greater) => Look::plain(dim),
        _ => Look::plain(sung),
    }
}

/// A small, fast random number in 0..1 (the spawn height's jitter).
fn random(ui: &mut Ui) -> f64 {
    let mut x = ui.seed | 1;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    ui.seed = x;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

/// How far the view is open from `eye` along the unit `dir`, up to `max` blocks.
fn open(game: &Game<'_>, eye: [f64; 3], dir: [f64; 3], max: f64) -> f64 {
    game.clip_ray(eye, dir, max).map_or(max, |h| len(sub(h.point, eye)))
}

/// The lyrics in the world, appended to the text displays' vertices (the
/// hook at the end of `text_display_geometry`).
pub(crate) fn world(game: &Game<'_>, camera: glam::DVec3, out: &mut Vec<Vertex>) {
    if !game.native_window() || !game.native_on(ID) {
        return;
    }
    let e = eng(game);
    let snap = e.snapshot();
    let eye = [camera.x, camera.y, camera.z];
    with_ui(|ui| {
        ui.refresh(e);
        let now = Instant::now();
        let dt = now.saturating_duration_since(ui.world_frame).as_secs_f32().clamp(0.0, 0.25);
        ui.world_frame = now;
        let cfg = ui.cfg.clone();
        let flying = cfg.elytra && game.player.as_ref().is_some_and(|p| p.fall_flying);
        let wants = matches!(cfg.display, Display::World | Display::Both) || flying;
        if !wants || !cfg.enabled || ui.hidden || !cfg.show_lyrics || snap.track.is_none() {
            ui.anchor = None;
            ui.passed.clear();
            ui.wbursts.clear();
            return;
        }
        let Some(track) = snap.track.clone() else { return };
        if cfg.hide_when_paused && !snap.playing {
            return;
        }
        let (yaw, pitch) = game.native_camera().map_or((game.yaw, game.pitch), |c| (c.yaw, c.pitch));
        let pvp = ui.pvp_now() && !flying;
        if pvp != ui.pvp {
            // Into or out of PvP: the line comes to you (or goes back out).
            ui.pvp = pvp;
            ui.anchor = None;
            ui.passed.clear();
        }
        let stay = cfg.anchor_3d == Anchor::Stay && !flying && !pvp;
        let distance = if pvp { cfg.pvp_distance } else { cfg.distance_3d } as f64;
        let pos = lyric_pos(&cfg, &snap);
        let lyrics = &snap.lyrics;
        let index = lyrics.index_at(pos);
        if ui.flying != flying {
            ui.flying = flying;
            ui.anchor = None;
            ui.passed.clear();
        }
        if flying {
            let v = game.player.as_ref().map(|p| [p.velocity.x, p.velocity.y, p.velocity.z]).unwrap_or([0.0; 3]);
            let speed = len(v);
            let dir = if speed > 0.15 { [v[0] / speed, v[1] / speed, v[2] / speed] } else { forward(yaw, pitch) };
            ui.fly_dir = lerp3(ui.fly_dir, dir, 1.0 - (-dt as f64 * 3.0).exp());
            let n = len(ui.fly_dir).max(1e-6);
            ui.fly_dir = [ui.fly_dir[0] / n, ui.fly_dir[1] / n, ui.fly_dir[2] / n];
        }
        let up = [0.0, 1.0, 0.0];
        let changed = index != ui.world_line || track.serial != ui.world_serial;
        if changed {
            let key = (ui.world_serial, ui.world_line);
            // The line ended before its last word's sparks: they burst now.
            if cfg.sparks
                && ui.world_fired != Some(key)
                && ui.world_line.is_some()
                && let Some(mut b) = ui.world_last.take()
            {
                b.since = now;
                ui.wbursts.push(b);
            }
            ui.world_last = None;
            let previous = ui
                .world_line
                .filter(|_| track.serial == ui.world_serial)
                .and_then(|i| lyrics.lines.get(i))
                .map(|l| l.text.clone())
                .filter(|s| !s.trim().is_empty());
            if (stay || flying)
                && let (Some(prev), Some(at)) = (previous.clone(), ui.anchor)
            {
                ui.passed.push(Passed { text: prev, at, since: now, yaw: ui.spawn_yaw });
                if ui.passed.len() > 6 {
                    ui.passed.remove(0);
                }
            }
            ui.world_previous = previous;
            ui.world_line = index;
            ui.world_serial = track.serial;
            ui.world_changed = now;
            ui.anchor = None;
        }
        // A new line appears far ahead of where you look, at a random height.
        let anchor = match ui.anchor {
            Some(a) if len(sub(a, eye)) < 128.0 => a,
            _ => {
                let mut jitter = if pvp { 0.0 } else { (random(ui) * 2.0 - 1.0) * cfg.height_random_3d as f64 };
                // Never right on top of the line fading out: at least 1.3 blocks above or below it.
                if let Some(prev) = ui.passed.last().map(|p| p.at[1] - (eye[1] + cfg.height_3d as f64)) {
                    let gap = jitter - prev;
                    if gap.abs() < 1.3 {
                        jitter = prev + if gap >= 0.0 || prev < -1.0 { 1.3 } else { -1.3 };
                    }
                }
                let a = if flying {
                    add(add(eye, ui.fly_dir, cfg.elytra_lead as f64 * 2.4), up, cfg.height_3d as f64 + 1.0 + jitter)
                } else {
                    let look = forward(yaw, 0.0);
                    add(add(eye, look, distance), up, cfg.height_3d as f64 + jitter)
                };
                ui.spawn_yaw = yaw;
                a
            }
        };
        let anchor = if stay {
            // Fixed in the world, but never walked into: it backs away to keep its distance.
            let d = sub(anchor, eye);
            let flat = (d[0] * d[0] + d[2] * d[2]).sqrt();
            let keep = distance * 0.6;
            if flat < keep && flat > 1e-3 {
                let pushed = [eye[0] + d[0] / flat * keep, anchor[1], eye[2] + d[2] / flat * keep];
                lerp3(anchor, pushed, 1.0 - (-dt as f64 * 8.0).exp())
            } else {
                anchor
            }
        } else if flying {
            let target = add(add(eye, ui.fly_dir, cfg.elytra_lead as f64), up, cfg.height_3d as f64 + 1.0);
            lerp3(anchor, target, 1.0 - (-dt as f64 * 4.0).exp())
        } else {
            let target = add(add(eye, forward(yaw, 0.0), distance), up, cfg.height_3d as f64);
            // PvP keeps up with fast turns.
            let k = if pvp { 1.0 - (-dt as f64 * 14.0).exp() } else { 1.0 - (1.0 - cfg.follow_3d as f64).powf((dt as f64 * 60.0).max(0.0)) };
            lerp3(anchor, target, k)
        };
        ui.anchor = Some(anchor);
        // Fixed lines face where you stood when they appeared; following ones face you.
        let facing = |at: [f64; 3], spawned: f32| -> (f32, Option<f32>) {
            if stay || cfg.billboard_3d == Billboard::Fixed {
                return (spawned, None);
            }
            let to = sub(eye, at);
            let flat = (to[0] * to[0] + to[2] * to[2]).sqrt();
            let y = (-(to[0]).atan2(to[2])).to_degrees() as f32 + 180.0;
            let p = (to[1].atan2(flat)).to_degrees() as f32;
            (y, (cfg.billboard_3d == Billboard::Full).then_some(p))
        };
        // PvP lines come close, so they're drawn smaller: a bit under the usual size on screen.
        let scale = 0.025 * if flying { cfg.elytra_scale } else { cfg.scale_3d } * if pvp { 0.8 * (distance / cfg.distance_3d.max(2.0) as f64).min(1.0) as f32 } else { 1.0 };
        let accent = ui.accent();
        let active = ui.active_color();
        let since = now.saturating_duration_since(ui.world_changed).as_secs_f32();
        let t = ease_out(since / 0.45);
        let st = style(&cfg.lyrics_font, cfg.bold_active, cfg.italic);
        let split = |s: &str| -> Vec<String> { lyric_text(&cfg, s).split_whitespace().map(str::to_owned).collect() };
        let grad = gradient(ui);
        let spark_colors = grad.unwrap_or((accent, active));
        // Lines already sung fade where they hang.
        let fade_for: f32 = if flying { 4.0 } else { 0.55 };
        ui.passed.retain(|p| now.saturating_duration_since(p.since).as_secs_f32() < fade_for);
        for p in &ui.passed {
            let age = now.saturating_duration_since(p.since).as_secs_f32();
            let alpha = (1.0 - age / fade_for).clamp(0.0, 1.0);
            let at = add(p.at, up, if flying { 0.0 } else { 0.9 * ease_out(age / fade_for) as f64 });
            let (y, pt) = facing(at, p.yaw);
            let c = argb(if grad.is_some() { 0xFFFFFF } else { active }, alpha * 0.9);
            let seed = p.since.duration_since(ui.started).as_nanos() as u64;
            let mut fx = LineFx::of(ui, None, 1.0, seed ^ 0x51, 190.0);
            fx.vanish = Vanish::Off;
            let lay = place(game, &split(&p.text), &st, &|_| Look::plain(c), &fx);
            draw_line_world(game, out, ui, eye, at, y, pt, scale as f32, &st, &lay, None, grad);
        }
        // The current line.
        let (words, synced) = match index.and_then(|i| lyrics.lines.get(i)) {
            Some(l) if !l.text.trim().is_empty() => (split(&l.text), true),
            Some(_) => (vec!["♪".to_owned()], false),
            None if !lyrics.lines.is_empty() => (vec!["♪".to_owned()], false),
            None => match snap.lyrics_state {
                LyricsState::Loading => (split("♪ …"), false),
                LyricsState::Instrumental => (split("♪ Instrumental ♪"), false),
                _ => (split(&format!("♪ {}", track.title)), false),
            },
        };
        let mut alpha = 1.0f32;
        let mut px = scale as f32;
        let mut at = anchor;
        match cfg.animation_3d {
            Animation::Fade => alpha = t,
            Animation::Slide => {
                at = add(at, up, -0.4 * (1.0 - t as f64));
                alpha = t;
            }
            Animation::Pop => px *= 0.55 + 0.45 * ease_back(since / 0.4),
            Animation::Typewriter | Animation::Ascend => {
                at = add(at, up, -0.7 * (1.0 - t as f64));
                alpha = t;
            }
            Animation::SlideSide => {
                at = add(at, forward(ui.spawn_yaw - 90.0, 0.0), -1.0 * (1.0 - t as f64));
                alpha = t;
            }
        }
        // Which word is sung now, and how far through the line.
        let sung = index.filter(|_| synced).and_then(|i| lyrics.word_at(i, pos, track.duration_ms));
        let progress = index.filter(|_| synced).map_or(0.0, |i| lyrics.progress(i, pos, track.duration_ms));
        // Tight spaces: closer (same size on screen), smaller when the sides
        // are close, and round you like an aura when there's no room at all.
        let (mut fit_d, mut fit_s, mut aura) = (1.0f32, 1.0f32, 0.0f32);
        if cfg.fit_tight && !flying {
            let v = sub(at, eye);
            let d = len(v);
            if d > 0.5 {
                let dir = [v[0] / d, v[1] / d, v[2] / d];
                let free = open(game, eye, dir, d + 0.3);
                let reach = (free - 0.35).clamp(0.6, d);
                fit_d = (reach / d) as f32;
                let line_w = words.iter().map(|w| width(game, &text(w, &st))).sum::<f32>().min(190.0) + 6.0 * words.len() as f32;
                let half = (line_w * px * fit_d / 2.0) as f64;
                let flat = (dir[0] * dir[0] + dir[2] * dir[2]).sqrt().max(1e-6);
                let right = [-dir[2] / flat, 0.0, dir[0] / flat];
                let mid = add(eye, dir, reach);
                for s in [-1.0, 1.0] {
                    let edge = add(mid, right, s * half);
                    let ev = sub(edge, eye);
                    let el = len(ev).max(1e-6);
                    let o = open(game, eye, [ev[0] / el, ev[1] / el, ev[2] / el], el + 0.2);
                    if o < el {
                        fit_s = fit_s.min((((o - 0.3) / el) as f32).max(0.3));
                    }
                }
                aura = (((1.6 - free) / 1.0) as f32).clamp(0.0, 1.0).max(((0.75 - fit_s) * 2.5).clamp(0.0, 1.0));
            }
        }
        let k = 1.0 - (-dt * 6.0).exp();
        ui.fit_d += (fit_d - ui.fit_d) * k;
        ui.fit_s += (fit_s - ui.fit_s) * k;
        ui.aura += (aura - ui.aura) * k;
        if cfg.fit_tight && !flying {
            at = lerp3(eye, at, ui.fit_d as f64);
            px *= ui.fit_d * ui.fit_s.max(0.3);
        }
        let (y, pt) = facing(at, ui.spawn_yaw);
        let show_art = (cfg.art_3d && !flying && ui.aura < 0.5).then_some(alpha);
        // With a gradient the words are white and the gradient colours them.
        let base_color = if grad.is_some() { 0xFFFFFF } else { active };
        let sung_c = argb(base_color, alpha);
        let hi = argb(if grad.is_some() { 0xFFFFFF } else { accent }, alpha);
        let dim = argb(base_color, 0.42 * alpha);
        let seed = track.serial.wrapping_mul(1_000_003) ^ index.unwrap_or(0) as u64;
        let mut fx = LineFx::of(ui, sung, progress, seed, 190.0);
        if ui.aura > 0.5 {
            fx.arrangement = Arrangement::Aura;
            fx.amount = 0.15;
        }
        fx.squeeze = 1.0 - 0.5 * (1.0 - ui.fit_s).clamp(0.0, 1.0);
        let lay = match (cfg.highlight, sung) {
            (Highlight::OneWord, Some((k, p))) => {
                // Only the word being sung, big, popping in.
                let one = vec![words.get(k).cloned().unwrap_or_default()];
                let pop = 1.35 + 0.25 * (1.0 - ease_out(p * 4.0));
                let look = Look { color: hi, scale: pop, lift: 3.0 * jump(p), underline: false };
                let fx = LineFx { arrangement: Arrangement::Line, sung: Some((0, p)), vanish: Vanish::Off, ..fx };
                place(game, &one, &st, &|_| look, &fx)
            }
            (style, Some((k, p))) => place(game, &words, &st, &|i| word_look(style, i, k, p, sung_c, hi, dim), &fx),
            _ => place(game, &words, &st, &|_| Look::plain(sung_c), &fx),
        };
        draw_line_world(game, out, ui, eye, at, y, pt, px, &st, &lay, show_art, grad);
        // Sparks from the last word, as it's sung.
        let last = if cfg.highlight == Highlight::OneWord && sung.is_some() { 0 } else { words.len().saturating_sub(1) };
        if let Some(&origin) = lay.words.get(last) {
            let spec = WBurst { since: now, at, yaw: y, pitch: pt, px, origin, seed: seed ^ 0xA5 };
            ui.world_last = Some(spec);
            let key = (track.serial, index);
            if cfg.sparks
                && synced
                && ui.world_fired != Some(key)
                && sung.is_some_and(|(k, p)| k + 1 >= words.len() && p > 0.8)
            {
                ui.world_fired = Some(key);
                ui.wbursts.push(spec);
            }
        }
        ui.wbursts.retain(|b| now.saturating_duration_since(b.since).as_secs_f32() < 0.9);
        if cfg.sparks {
            for b in &ui.wbursts {
                world_sparks(game, out, eye, b, now.saturating_duration_since(b.since).as_secs_f32(), spark_colors, cfg.two_sided_3d);
            }
        }
        // Follow mode with more lines: the next one waits below.
        if !stay && !flying && !pvp && cfg.lines > 1 && synced
            && let Some(next) = index.map(|i| i + 1).and_then(|i| lyrics.lines.get(i))
            && !next.text.trim().is_empty()
        {
            let c = argb(cfg.inactive_color, 0.55);
            let below = add(at, up, -(lay.bounds[3] - lay.bounds[1] + 6.0) as f64 * px as f64);
            let fx = LineFx { sung: None, vanish: Vanish::Off, ..LineFx::of(ui, None, 0.0, seed ^ 7, 190.0) };
            let next_lay = place(game, &split(&next.text), &st, &|_| Look::plain(c), &fx);
            draw_line_world(game, out, ui, eye, below, y, pt, px * 0.75, &st, &next_lay, None, None);
        }
    });
}

// ---------------------------------------------------------------- menu

const TABS: [&str; 8] = ["General", "Style", "Lyrics", "Player", "3D", "Sources", "Fonts", "Music"];
const MUSIC_TAB: usize = 7;

#[derive(Clone, Copy)]
enum Row {
    Header(&'static str),
    Toggle(&'static str, fn(&Config) -> bool, fn(&mut Config, bool)),
    Cycle(&'static str, fn(&Config) -> String, fn(&mut Config, bool)),
    Slider(&'static str, fn(&Config) -> f32, fn(&mut Config, f32), f32, f32, f32, fn(f32) -> String),
    Swatch(&'static str, fn(&Config) -> u32, fn(&mut Config, u32), Field),
    Input(&'static str, Field),
    Buttons(&'static [(&'static str, Action)]),
    Info(fn(&Snapshot, &Config) -> String),
    FontPick(&'static str, bool),
    FontItem(usize),
    SongItem(usize),
    Wheel,
}

const COLOR_TARGETS: [&str; 4] = ["Gradient start", "Gradient end", "Lyrics colour", "Accent"];
static COLOR_TARGET: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
fn color_target(_: &Config) -> usize {
    COLOR_TARGET.load(std::sync::atomic::Ordering::Relaxed)
}
fn target_color(c: &Config, t: usize) -> u32 {
    match t {
        0 => c.gradient_a,
        1 => c.gradient_b,
        2 => c.active_color,
        _ => c.accent,
    }
}
fn set_target_color(c: &mut Config, t: usize, v: u32) {
    match t {
        0 => c.gradient_a = v,
        1 => c.gradient_b = v,
        2 => c.active_color = v,
        _ => c.accent = v,
    }
}
/// The wheel's disc and brightness strip in a row's rectangle.
fn wheel_geometry(r: [f32; 4]) -> ([f32; 2], f32, [f32; 4]) {
    let radius = (r[3] / 2.0 - 3.0).max(8.0);
    let centre = [r[0] + 8.0 + radius, r[1] + r[3] / 2.0];
    let strip = [centre[0] + radius + 10.0, r[1] + 3.0, 9.0, r[3] - 6.0];
    (centre, radius, strip)
}

const FONT_TARGETS: [&str; 3] = ["Lyrics", "Song title", "Both"];

/// What a click in the font list sets (kept in the menu, not the settings).
fn font_target(_: &Config) -> usize {
    FONT_TARGET.load(std::sync::atomic::Ordering::Relaxed)
}
static FONT_TARGET: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

const SWATCHES: [u32; 12] = [
    0xFFFFFF, 0x1DB954, 0x000000, 0xFF5500, 0xFF3B5C, 0xF7D046, 0x4FC3F7, 0x7C4DFF, 0xFF8AD8, 0x00E5C3,
    0xB3B3B3, 0xFF9E40,
];

fn pct(v: f32) -> String {
    format!("{:.0}%", v * 100.0)
}
fn times(v: f32) -> String {
    format!("{v:.2}×")
}
fn blocks(v: f32) -> String {
    format!("{v:.1} blocks")
}
fn seconds(v: f32) -> String {
    format!("{v:.1} s")
}
fn count(v: f32) -> String {
    format!("{v:.0}")
}

fn rows(tab: usize, fonts: usize, songs: usize) -> Vec<Row> {
    use Row::*;
    match tab {
        0 => vec![
            Info(|s, _| match &s.track {
                Some(t) => format!("Now: {} – {}  ({}{})", t.title, t.artist, if t.app.is_empty() { "?" } else { t.app.as_str() }, if s.playing { "" } else { ", paused" }),
                None => "Nothing playing. Start music in Spotify, a browser or any player.".into(),
            }),
            Buttons(&[("<< Prev", Action::Previous), ("Play / Pause", Action::PlayPause), ("Next >>", Action::Next), ("Reload lyrics", Action::ReloadLyrics)]),
            Header("General"),
            Toggle("LyricsPlus on", |c| c.enabled, |c, v| c.enabled = v),
            Cycle("Display", |c| c.display.label().into(), |c, b| c.display = c.display.cycle(b)),
            Toggle("Hide when paused", |c| c.hide_when_paused, |c, v| c.hide_when_paused = v),
            Toggle("Hide behind other screens", |c| c.hide_in_screens, |c, v| c.hide_in_screens = v),
            Toggle("Fade while fighting", |c| c.combat_fade, |c, v| c.combat_fade = v),
            Toggle("Adaptive colours (album art)", |c| c.adaptive_colors, |c, v| c.adaptive_colors = v),
            Swatch("Accent colour", |c| c.accent, |c, v| c.accent = v, Field::Accent),
            Header("Timing"),
            Info(|_, c| format!("Lyrics offset: {:+} ms (positive shows lines earlier)", c.offset_ms)),
            Buttons(&[("-250 ms", Action::OffsetMinus), ("+250 ms", Action::OffsetPlus)]),
            Input("Offset (ms)", Field::Offset),
            Header("Layout"),
            Buttons(&[("Edit HUD layout (drag)", Action::Layout), ("Reset all settings", Action::Reset)]),
        ],
        2 => vec![
            Header("Lyrics on screen"),
            Toggle("Show lyrics", |c| c.show_lyrics, |c, v| c.show_lyrics = v),
            Cycle("Position", |c| c.lyrics_position.label().into(), |c, b| c.lyrics_position = c.lyrics_position.cycle(b)),
            Cycle("Alignment", |c| c.lyrics_align.label().into(), |c, b| c.lyrics_align = c.lyrics_align.cycle(b)),
            Slider("Lines", |c| c.lines as f32, |c, v| c.lines = v.round() as u32, 1.0, 9.0, 1.0, count),
            Slider("Size", |c| c.lyrics_scale, |c, v| c.lyrics_scale = v, 0.5, 4.0, 0.05, times),
            Slider("Max width", |c| c.max_width, |c, v| c.max_width = v, 0.2, 1.0, 0.05, pct),
            Slider("Background", |c| c.lyrics_background, |c, v| c.lyrics_background = v, 0.0, 1.0, 0.05, pct),
            Cycle("Active line", |c| c.highlight.label().into(), |c, b| c.highlight = c.highlight.cycle(b)),
            Swatch("Active colour", |c| c.active_color, |c, v| c.active_color = v, Field::Active),
            Swatch("Other lines", |c| c.inactive_color, |c, v| c.inactive_color = v, Field::Active),
            Toggle("Bold active line", |c| c.bold_active, |c, v| c.bold_active = v),
            Toggle("Italic", |c| c.italic, |c, v| c.italic = v),
            Toggle("UPPERCASE", |c| c.uppercase, |c, v| c.uppercase = v),
            Toggle("Text shadow", |c| c.shadow, |c, v| c.shadow = v),
            Toggle("Smooth transitions", |c| c.smooth, |c, v| c.smooth = v),
            Toggle("Instrumental countdown", |c| c.countdown, |c, v| c.countdown = v),
            Toggle("Show search status", |c| c.show_status, |c, v| c.show_status = v),
        ],
        3 => vec![
            Header("Player widget"),
            Toggle("Show player", |c| c.show_widget, |c, v| c.show_widget = v),
            Cycle("Style", |c| c.widget_style.label().into(), |c, b| c.widget_style = c.widget_style.cycle(b)),
            Cycle("Corner", |c| c.widget_corner.label().into(), |c, b| c.widget_corner = c.widget_corner.cycle(b)),
            Slider("Size", |c| c.widget_scale, |c, v| c.widget_scale = v, 0.5, 3.0, 0.05, times),
            Slider("Opacity", |c| c.widget_alpha, |c, v| c.widget_alpha = v, 0.0, 1.0, 0.05, pct),
            Toggle("Album art", |c| c.show_art, |c, v| c.show_art = v),
            Toggle("Progress bar", |c| c.show_progress, |c, v| c.show_progress = v),
            Toggle("Times", |c| c.show_time, |c, v| c.show_time = v),
            Toggle("Equalizer", |c| c.equalizer, |c, v| c.equalizer = v),
            Toggle("Scroll long titles", |c| c.marquee, |c, v| c.marquee = v),
            Toggle("Title", |c| c.show_title, |c, v| c.show_title = v),
            Toggle("Artist", |c| c.show_artist, |c, v| c.show_artist = v),
            Toggle("Album", |c| c.show_album, |c, v| c.show_album = v),
            Toggle("Source and play state", |c| c.show_source, |c, v| c.show_source = v),
            Slider("Width", |c| c.widget_width, |c, v| c.widget_width = v, 0.7, 1.8, 0.05, times),
            Slider("Roundness", |c| c.widget_round, |c, v| c.widget_round = v, 0.0, 1.0, 0.05, pct),
            Slider("Cover size", |c| c.art_size, |c, v| c.art_size = v, 0.5, 1.8, 0.05, times),
            Slider("Cover roundness", |c| c.art_round, |c, v| c.art_round = v, 0.0, 1.0, 0.05, pct),
            Header("Island (top middle)"),
            Toggle("Show the island", |c| c.island, |c, v| c.island = v),
            Slider("Size", |c| c.island_scale, |c, v| c.island_scale = v, 0.5, 3.0, 0.05, times),
            Slider("Width", |c| c.island_width, |c, v| c.island_width = v, 0.5, 2.0, 0.05, times),
            Slider("Height", |c| c.island_height, |c, v| c.island_height = v, 0.7, 2.5, 0.05, times),
            Slider("Roundness", |c| c.island_round, |c, v| c.island_round = v, 0.0, 1.0, 0.05, pct),
            Slider("From the top", |c| c.island_y, |c, v| c.island_y = v, 0.0, 200.0, 1.0, count),
            Slider("Background", |c| c.island_bg, |c, v| c.island_bg = v, 0.0, 1.0, 0.05, pct),
            Swatch("Background colour", |c| c.island_bg_color, |c, v| c.island_bg_color = v, Field::Accent),
            Slider("Outline", |c| c.island_outline, |c, v| c.island_outline = v, 0.0, 1.0, 0.25, pct),
            Swatch("Outline colour", |c| c.island_outline_color, |c, v| c.island_outline_color = v, Field::Accent),
            Toggle("Cover", |c| c.island_cover, |c, v| c.island_cover = v),
            Toggle("Time", |c| c.island_time, |c, v| c.island_time = v),
            Toggle("Progress bar", |c| c.island_progress, |c, v| c.island_progress = v),
            Header("Now-playing toast"),
            Toggle("Show toast on new song", |c| c.show_toast, |c, v| c.show_toast = v),
            Cycle("Toast corner", |c| c.toast_corner.label().into(), |c, b| c.toast_corner = c.toast_corner.cycle(b)),
            Slider("Toast time", |c| c.toast_seconds, |c, v| c.toast_seconds = v, 1.0, 15.0, 0.5, seconds),
        ],
        4 => vec![
            Header("Lyrics in the world"),
            Cycle("Display", |c| c.display.label().into(), |c, b| c.display = c.display.cycle(b)),
            Cycle("Lyrics", |c| c.anchor_3d.label().into(), |c, b| c.anchor_3d = c.anchor_3d.cycle(b)),
            Slider("Distance", |c| c.distance_3d, |c, v| c.distance_3d = v, 2.0, 40.0, 0.5, blocks),
            Slider("Height", |c| c.height_3d, |c, v| c.height_3d = v, -3.0, 4.0, 0.1, blocks),
            Slider("Size", |c| c.scale_3d, |c, v| c.scale_3d = v, 0.25, 12.0, 0.05, times),
            Slider("Follow speed", |c| c.follow_3d, |c, v| c.follow_3d = v, 0.01, 1.0, 0.01, pct),
            Cycle("Facing", |c| c.billboard_3d.label().into(), |c, b| c.billboard_3d = c.billboard_3d.cycle(b)),
            Toggle("Two-sided", |c| c.two_sided_3d, |c, v| c.two_sided_3d = v),
            Toggle("Outline", |c| c.outline_3d, |c, v| c.outline_3d = v),
            Slider("Random height", |c| c.height_random_3d, |c, v| c.height_random_3d = v, 0.0, 6.0, 0.25, blocks),
            Cycle("Word sync", |c| c.highlight.label().into(), |c, b| c.highlight = c.highlight.cycle(b)),
            Cycle("Animation", |c| c.animation_3d.label().into(), |c, b| c.animation_3d = c.animation_3d.cycle(b)),
            Toggle("Album art beside", |c| c.art_3d, |c, v| c.art_3d = v),
            Slider("Background", |c| c.background_3d, |c, v| c.background_3d = v, 0.0, 1.0, 0.05, pct),
            Header("Elytra mode"),
            Toggle("Lyrics ahead while gliding", |c| c.elytra, |c, v| c.elytra = v),
            Slider("Lead distance", |c| c.elytra_lead, |c, v| c.elytra_lead = v, 6.0, 80.0, 1.0, blocks),
            Slider("Elytra size", |c| c.elytra_scale, |c, v| c.elytra_scale = v, 0.5, 10.0, 0.1, times),
            Info(|_, _| "Lines appear far ahead along your flight, then hang in place for you to fly through.".into()),
        ],
        5 => vec![
            Cycle("Source", |c| c.source.label().into(), |c, b| c.source = c.source.cycle(b)),
            Header("Spotify Web API (your own app key)"),
            Info(|s, _| match &s.spotify {
                SpotifyStatus::NoClientId => "Optional: the Spotify app already works through System media below.".into(),
                SpotifyStatus::NotLinked => "Client ID set. Press Connect and allow access in the browser.".into(),
                SpotifyStatus::Linking(_) => "Waiting for the browser… (approve the LyricsPlus app)".into(),
                SpotifyStatus::Linked(u) => format!("✔ Linked as {u}"),
                SpotifyStatus::Error(e) => format!("✖ {e}"),
            }),
            Info(|_, _| format!("For exact timing: an app at developer.spotify.com, redirect {SPOTIFY_REDIRECT}")),
            Input("Client ID", Field::ClientId),
            Buttons(&[("Connect", Action::SpotifyConnect), ("Disconnect", Action::SpotifyDisconnect), ("Dashboard", Action::SpotifyDashboard), ("Copy redirect URI", Action::CopyRedirect)]),
            Slider("Poll every", |c| c.spotify_poll_ms as f32 / 1000.0, |c, v| c.spotify_poll_ms = (v * 1000.0) as u32, 0.5, 5.0, 0.25, seconds),
            Header("System media (any app in your OS media controls)"),
            Toggle("Use system media session", |c| c.system_media, |c, v| c.system_media = v),
            Info(|s, _| if s.system_running { "Running: SoundCloud/YouTube in a browser, Spotify app, Apple Music, TIDAL…".into() } else { "Not running (Windows: PowerShell; Linux: playerctl; macOS: Spotify/Music)".into() }),
            Header("Browser bridge (SoundCloud, YouTube Music, Deezer, TIDAL…)"),
            Toggle("Bridge on 127.0.0.1:8974", |c| c.bridge, |c, v| c.bridge = v),
            Info(|s, _| match s.bridge_seen {
                Some(t) if t < 5.0 => "✔ A browser is posting to the bridge".into(),
                Some(t) => format!("Last post {t:.0} s ago"),
                None => "Install the userscript (Tampermonkey/Violentmonkey) to connect web players".into(),
            }),
            Buttons(&[("Install userscript", Action::InstallUserscript), ("Copy bridge URL", Action::CopyBridgeUrl), ("Open data folder", Action::OpenFolder)]),
        ],
        1 => vec![
            Header("How the words move"),
            Cycle("Word style", |c| c.highlight.label().into(), |c, b| c.highlight = c.highlight.cycle(b)),
            Cycle("Idle motion", |c| c.motion.label().into(), |c, b| c.motion = c.motion.cycle(b)),
            Cycle("Vanish", |c| c.vanish.label().into(), |c, b| c.vanish = c.vanish.cycle(b)),
            Toggle("Sparks from the last word", |c| c.sparks, |c, v| c.sparks = v),
            Header("How the words are placed"),
            Cycle("Layout", |c| c.arrangement.label().into(), |c, b| c.arrangement = c.arrangement.cycle(b)),
            Slider("Spread", |c| c.scatter, |c, v| c.scatter = v, 0.0, 1.0, 0.05, pct),
            Toggle("Fit into tight spaces (3D)", |c| c.fit_tight, |c, v| c.fit_tight = v),
            Header("PvP mode (key: Numpad 0)"),
            Toggle("PvP mode", |c| c.pvp_mode, |c, v| c.pvp_mode = v),
            Toggle("Turn on while fighting", |c| c.pvp_auto, |c, v| c.pvp_auto = v),
            Slider("PvP distance", |c| c.pvp_distance, |c, v| c.pvp_distance = v, 2.0, 16.0, 0.5, blocks),
            Info(|_, _| "Lyrics follow you, split left and right so the crosshair stays clear.".into()),
            Header("Colours"),
            Toggle("Gradient on the lyrics", |c| c.gradient, |c, v| c.gradient = v),
            Toggle("Gradient from the album art", |c| c.gradient_album, |c, v| c.gradient_album = v),
            Toggle("Adaptive colours (album art)", |c| c.adaptive_colors, |c, v| c.adaptive_colors = v),
            Cycle("Colour wheel sets", |c| COLOR_TARGETS[color_target(c)].into(), |_, _| {}),
            Wheel,
        ],
        MUSIC_TAB => {
            let mut v = vec![
                Header("Offline music player"),
                Info(|_, _| {
                    let st = engine::player::state();
                    match engine::player::current() {
                        Some((title, artist)) => format!(
                            "{} {}{}  {} / {}",
                            if st.playing { "▶" } else { "❚❚" },
                            title,
                            if artist.is_empty() { String::new() } else { format!(" – {artist}") },
                            time(st.position_ms),
                            time(st.duration_ms)
                        ),
                        None => format!("Nothing playing · {} songs in your music folder", engine::player::count()),
                    }
                }),
                Buttons(&[("<< Prev", Action::MusicPrevious), ("Play / Pause", Action::MusicToggle), ("Next >>", Action::MusicNext), ("Stop", Action::MusicStop)]),
                Slider("Volume", |c| c.volume, |c, v| {
                    c.volume = v;
                    engine::player::set_volume(v);
                }, 0.0, 1.0, 0.05, pct),
                Toggle("Shuffle", |c| c.shuffle, |c, v| c.shuffle = v),
                Toggle("Repeat the song", |c| c.repeat, |c, v| c.repeat = v),
                Buttons(&[("Open music folder", Action::OpenMusic), ("Rescan", Action::Rescan)]),
                Header("Library (click a song to play it)"),
            ];
            if songs == 0 {
                v.push(Info(|_, _| "Empty. Press Open music folder, drop songs in, then Rescan.".into()));
            }
            v.extend((0..songs).map(SongItem));
            v.push(Header("About"));
            v.push(Info(|_, _| "MP3, M4A, WAV, FLAC, WMA, OGG. A .lrc file with the same name = offline lyrics.".into()));
            v.push(Info(|_, _| {
                let b = engine::player::state().backend;
                if b.is_empty() { "Plays through your system: no internet needed.".into() } else { format!("Playing through: {b}") }
            }));
            v
        }
        _ => {
            let mut v = vec![
                Header("Fonts"),
                Cycle("Pick a font for", |c| FONT_TARGETS[font_target(c)].into(), |_, _| {}),
                Buttons(&[("Open fonts folder", Action::OpenFonts), ("Reload fonts", Action::ReloadFonts)]),
                Info(|_, _| "Drop .ttf/.otf files in the fonts folder, press Reload, then click one below.".into()),
            ];
            v.extend((0..fonts).map(FontItem));
            v
        }    }
}

const ROW_H: f32 = 16.0;

struct Frame {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    content_top: f32,
    content_bottom: f32,
}

fn frame_of(size: [f32; 2]) -> Frame {
    let w = (size[0] - 16.0).min(372.0).max(220.0);
    let h = (size[1] - 12.0).min(260.0).max(150.0);
    let x = ((size[0] - w) / 2.0).round();
    let y = ((size[1] - h) / 2.0).round();
    Frame { x, y, w, h, content_top: y + 44.0, content_bottom: y + h - 20.0 }
}

/// Every row's rectangle in the content area, scrolled.
fn row_rects(f: &Frame, rows: &[Row], scroll: f32) -> Vec<[f32; 4]> {
    let mut y = f.content_top - scroll;
    rows.iter()
        .map(|r| {
            let h = match r {
                Row::Header(_) => 13.0,
                Row::Swatch(..) => 18.0,
                Row::FontPick(..) => 26.0,
                Row::FontItem(_) => 18.0,
                Row::SongItem(_) => 20.0,
                Row::Wheel => 74.0,
                _ => ROW_H,
            };
            let rect = [f.x + 8.0, y, f.w - 16.0, h];
            y += h;
            rect
        })
        .collect()
}

fn control_rect(row: [f32; 4]) -> [f32; 4] {
    let w = (row[2] * 0.48).min(170.0);
    [row[0] + row[2] - w, row[1] + 1.0, w, row[3] - 3.0]
}
fn inside(p: [f32; 2], r: [f32; 4]) -> bool {
    p[0] >= r[0] && p[0] < r[0] + r[2] && p[1] >= r[1] && p[1] < r[1] + r[3]
}
fn buttons_rects(row: [f32; 4], n: usize) -> Vec<[f32; 4]> {
    let gap = 4.0;
    let w = (row[2] - gap * (n as f32 - 1.0)) / n as f32;
    (0..n).map(|i| [row[0] + i as f32 * (w + gap), row[1] + 1.0, w, row[3] - 3.0]).collect()
}
fn tab_rects(f: &Frame) -> Vec<[f32; 4]> {
    let gap = 2.0;
    let n = TABS.len() as f32;
    let w = (f.w - 16.0 - gap * (n - 1.0)) / n;
    (0..TABS.len()).map(|i| [f.x + 8.0 + i as f32 * (w + gap), f.y + 26.0, w, 13.0]).collect()
}
fn footer_rects(f: &Frame) -> [[f32; 4]; 2] {
    [
        [f.x + f.w - 64.0, f.y + f.h - 17.0, 56.0, 13.0],
        [f.x + 8.0, f.y + f.h - 17.0, 110.0, 13.0],
    ]
}

fn field_value(c: &Config, field: Field) -> String {
    match field {
        Field::ClientId => c.spotify_client_id.clone(),
        Field::LyricsFont => c.lyrics_font.clone(),
        Field::TitleFont => c.title_font.clone(),
        Field::Accent => format!("{:06X}", c.accent),
        Field::Active => format!("{:06X}", c.active_color),
        Field::Offset => c.offset_ms.to_string(),
    }
}

fn commit(e: &engine::Engine, field: Field, value: &str) {
    let v = value.trim().to_owned();
    e.update_config(|c| match field {
        Field::ClientId => c.spotify_client_id = v.chars().filter(|c| c.is_ascii_alphanumeric()).collect(),
        Field::LyricsFont => c.lyrics_font = if v.is_empty() { "minecraft:default".into() } else { full_id(&v) },
        Field::TitleFont => c.title_font = if v.is_empty() { "minecraft:default".into() } else { full_id(&v) },
        Field::Accent => {
            if let Ok(x) = u32::from_str_radix(v.trim_start_matches('#'), 16) {
                c.accent = x;
            }
        }
        Field::Active => {
            if let Ok(x) = u32::from_str_radix(v.trim_start_matches('#'), 16) {
                c.active_color = x;
            }
        }
        Field::Offset => {
            if let Ok(x) = v.trim_start_matches('+').parse::<i64>() {
                c.offset_ms = x;
            }
        }
    });
}

fn full_id(s: &str) -> String {
    if s.contains(':') { s.to_lowercase() } else { format!("minecraft:{}", s.to_lowercase()) }
}

/// Fonts to offer: vanilla's four and every font file in the resources and
/// the instance's resource packs (folders and zips).
fn scan_fonts(g: &Game<'_>) -> Vec<String> {
    let mut out: Vec<String> = ["minecraft:default", "minecraft:uniform", "minecraft:alt", "minecraft:illageralt"]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    let mut add = |ns: &str, file: &str| {
        if let Some(stem) = file.strip_suffix(".json")
            && !stem.contains('/')
            && !stem.is_empty()
        {
            let id = format!("{ns}:{stem}");
            if !out.contains(&id) {
                out.push(id);
            }
        }
    };
    let mut roots = vec![g.options.resources.clone()];
    if let Ok(dir) = std::fs::read_dir(g.native_instance_dir().join("resourcepacks")) {
        for entry in dir.flatten().take(64) {
            let p = entry.path();
            if p.is_dir() {
                roots.push(p);
            }
        }
    }
    for root in roots {
        let Ok(namespaces) = std::fs::read_dir(root.join("assets")) else { continue };
        for ns in namespaces.flatten() {
            let ns_name = ns.file_name().to_string_lossy().into_owned();
            if let Ok(files) = std::fs::read_dir(ns.path().join("font")) {
                for f in files.flatten() {
                    add(&ns_name, &f.file_name().to_string_lossy());
                }
            }
        }
    }
    out
}

fn font_label(id: &str) -> String {
    match id {
        "minecraft:default" => "Default".into(),
        "minecraft:uniform" => "Uniform (Unicode)".into(),
        "minecraft:alt" => "Enchanting (alt)".into(),
        "minecraft:illageralt" => "Illager (illageralt)".into(),
        other => other.into(),
    }
}

fn mine(game: &Game<'_>) -> bool {
    game.native_mod_screen() == Some(ID)
}

/// Draws the menu over the world.
pub(crate) fn screen_draw(game: &Game<'_>, b: &mut HudBatch, screen: &NativeScreenView<'_>) {
    if !mine(game) {
        return;
    }
    let e = eng(game);
    let snap = e.snapshot();
    with_ui(|ui| {
        ui.refresh(e);
        ui.menu.size = [b.width, b.height];
        if ui.menu.layout {
            layout_hints(game, b, ui);
            return;
        }
        menu(game, b, ui, &snap, screen.mouse);
    });
}

fn layout_hints(g: &Game<'_>, b: &mut HudBatch, ui: &Ui) {
    let lines = [
        "Drag the lyrics or the player to move them.",
        "Right-click one to put it back.  Esc or Enter: done.",
    ];
    let w = 250.0;
    let x = (b.width - w) / 2.0;
    panel(g, b, x, 6.0, w, 26.0, argb(0x101014, 0.85));
    for (i, l) in lines.iter().enumerate() {
        let t = plain(l);
        let tw = width(g, &t) * 0.85;
        draw(g, b, &t, (b.width - tw) / 2.0, 9.0 + i as f32 * 10.0, 0.85, argb(if i == 0 { ui.accent() } else { 0xD0D0D0 }, 1.0), false);
    }
}

fn menu(g: &Game<'_>, b: &mut HudBatch, ui: &mut Ui, snap: &Snapshot, mouse: [f32; 2]) {
    let f = frame_of(ui.menu.size);
    let accent = ui.accent();
    let cfg = ui.cfg.clone();
    // Dim the world a little, then the panel.
    fill(g, b, 0.0, 0.0, b.width, b.height, argb(0x000000, 0.35));
    panel(g, b, f.x, f.y, f.w, f.h, argb(0x0E0E12, 0.94));
    fill(g, b, f.x + 1.0, f.y, f.w - 2.0, 1.0, argb(accent, 1.0));
    // Header: the name and what plays.
    draw(g, b, &text("♪ LyricsPlus", &style("minecraft:default", true, false)), f.x + 8.0, f.y + 7.0, 1.2, argb(accent, 1.0), true);
    if let Some(t) = &snap.track {
        let side = 18.0;
        cover(g, b, ui, f.x + f.w - 8.0 - side, f.y + 4.0, side, 1.0);
        let label = text(&t.title, &style(&cfg.title_font, true, false));
        let max = 150.0f32.min(f.w - 120.0);
        let lw = width(g, &label).min(max);
        marquee(g, b, &label, f.x + f.w - 12.0 - side - lw, f.y + 5.0, 1.0, max, argb(0xFFFFFF, 1.0), true, secs(ui.started));
        let artist = text(&t.artist, &style(&cfg.title_font, false, false));
        let aw = (width(g, &artist) * 0.75).min(max);
        marquee(g, b, &artist, f.x + f.w - 12.0 - side - aw, f.y + 15.0, 0.75, max, argb(0xB3B3B3, 1.0), true, secs(ui.started));
    }
    // Tabs.
    for (i, r) in tab_rects(&f).iter().enumerate() {
        let on = i == ui.menu.tab;
        let hover = inside(mouse, *r);
        panel(g, b, r[0], r[1], r[2], r[3], argb(if on { accent } else { 0x26262C }, if on { 0.95 } else if hover { 1.0 } else { 0.85 }));
        let t = text(TABS[i], &style("minecraft:default", on, false));
        let tw = width(g, &t) * 0.85;
        draw(g, b, &t, r[0] + (r[2] - tw) / 2.0, r[1] + 3.0, 0.85, argb(if on { 0x0E0E12 } else { 0xE0E0E0 }, 1.0), false);
    }
    // Rows.
    if ui.menu.tab == MUSIC_TAB && ui.songs_at.is_none_or(|t| t.elapsed() > Duration::from_millis(700)) {
        ui.songs = engine::player::songs();
        ui.songs_at = Some(Instant::now());
    }
    let rows = rows(ui.menu.tab, ui.fonts.len(), ui.songs.len());
    let rects = row_rects(&f, &rows, ui.menu.scroll);
    ui.menu.content_height = rects.last().map_or(0.0, |r| r[1] + r[3] + ui.menu.scroll - f.content_top);
    let start = b.vertices.len();
    for (row, r) in rows.iter().zip(&rects) {
        if r[1] + r[3] < f.content_top - 2.0 || r[1] > f.content_bottom + 2.0 {
            continue;
        }
        draw_row(g, b, ui, snap, &cfg, *row, *r, mouse);
    }
    b.clip_since(start, f.x + 1.0, f.content_top, f.x + f.w - 1.0, f.content_bottom);
    // A scroll bar when the rows don't fit.
    let view = f.content_bottom - f.content_top;
    if ui.menu.content_height > view {
        let track_h = view;
        let bar = (view / ui.menu.content_height * track_h).max(12.0);
        let off = ui.menu.scroll / (ui.menu.content_height - view) * (track_h - bar);
        fill(g, b, f.x + f.w - 4.0, f.content_top, 2.0, track_h, argb(0x26262C, 1.0));
        fill(g, b, f.x + f.w - 4.0, f.content_top + off, 2.0, bar, argb(accent, 1.0));
    }
    // Footer.
    let [done, layout] = footer_rects(&f);
    button(g, b, done, "Done", inside(mouse, done), accent, true);
    button(g, b, layout, "Edit HUD layout", inside(mouse, layout), accent, false);
    let hint = plain("Right-click: back · Wheel: scroll");
    draw(g, b, &hint, layout[0] + layout[2] + 6.0, layout[1] + 3.5, 0.7, argb(0x808080, 1.0), false);
}

fn button(g: &Game<'_>, b: &mut HudBatch, r: [f32; 4], label: &str, hover: bool, accent: u32, primary: bool) {
    let bg = if primary { accent } else { 0x2E2E36 };
    panel(g, b, r[0], r[1], r[2], r[3], argb(if hover { art::mix(bg, 0xFFFFFF, 0.18) } else { bg }, 1.0));
    let t = plain(label);
    let s = 0.85;
    let tw = width(g, &t) * s;
    let shown = if tw > r[2] - 4.0 { ellipsize(g, &t, (r[2] - 4.0) / s) } else { t };
    let tw = width(g, &shown) * s;
    draw(g, b, &shown, r[0] + (r[2] - tw) / 2.0, r[1] + (r[3] - 8.0 * s) / 2.0, s, argb(if primary { 0x0E0E12 } else { 0xF0F0F0 }, 1.0), false);
}

#[allow(clippy::too_many_arguments)]
fn draw_row(g: &Game<'_>, b: &mut HudBatch, ui: &Ui, snap: &Snapshot, cfg: &Config, row: Row, r: [f32; 4], mouse: [f32; 2]) {
    let accent = ui.accent();
    let label = |b: &mut HudBatch, s: &str| {
        draw(g, b, &plain(s), r[0] + 2.0, r[1] + (r[3] - 8.0) / 2.0, 1.0, argb(0xE6E6E6, 1.0), false);
    };
    let hover = inside(mouse, r);
    if hover && !matches!(row, Row::Header(_) | Row::Info(_)) {
        fill(g, b, r[0], r[1], r[2], r[3] - 1.0, argb(0xFFFFFF, 0.04));
    }
    match row {
        Row::Header(s) => {
            draw(g, b, &text(s, &style("minecraft:default", true, false)), r[0], r[1] + 3.0, 0.85, argb(accent, 1.0), false);
            fill(g, b, r[0], r[1] + r[3] - 2.0, r[2], 1.0, argb(accent, 0.3));
        }
        Row::Toggle(name, get, _) => {
            label(b, name);
            let on = get(cfg);
            let c = control_rect(r);
            let sw = [c[0] + c[2] - 22.0, c[1] + 1.0, 22.0, c[3] - 2.0];
            panel(g, b, sw[0], sw[1], sw[2], sw[3], argb(if on { accent } else { 0x3A3A44 }, 1.0));
            let knob = if on { sw[0] + sw[2] - sw[3] } else { sw[0] };
            panel(g, b, knob + 1.0, sw[1] + 1.0, sw[3] - 2.0, sw[3] - 2.0, argb(0xFFFFFF, 1.0));
            let state = plain(if on { "ON" } else { "OFF" });
            let tw = width(g, &state) * 0.75;
            draw(g, b, &state, sw[0] - tw - 4.0, sw[1] + 2.0, 0.75, argb(if on { accent } else { 0x909090 }, 1.0), false);
        }
        Row::Cycle(name, get, _) => {
            label(b, name);
            let c = control_rect(r);
            panel(g, b, c[0], c[1], c[2], c[3], argb(if inside(mouse, c) { 0x34343E } else { 0x26262C }, 1.0));
            let v = plain(&get(cfg));
            let s = 0.85;
            let shown = if width(g, &v) * s > c[2] - 20.0 { ellipsize(g, &v, (c[2] - 20.0) / s) } else { v };
            let tw = width(g, &shown) * s;
            draw(g, b, &shown, c[0] + (c[2] - tw) / 2.0, c[1] + (c[3] - 8.0 * s) / 2.0, s, argb(0xFFFFFF, 1.0), false);
            draw(g, b, &plain("‹"), c[0] + 3.0, c[1] + 2.0, 1.0, argb(accent, 1.0), false);
            draw(g, b, &plain("›"), c[0] + c[2] - 7.0, c[1] + 2.0, 1.0, argb(accent, 1.0), false);
        }
        Row::Slider(name, get, _, lo, hi, _, fmt) => {
            label(b, name);
            let c = control_rect(r);
            let v = get(cfg);
            let t = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
            let bar_y = c[1] + c[3] / 2.0 - 1.0;
            fill(g, b, c[0], bar_y, c[2], 2.0, argb(0x3A3A44, 1.0));
            fill(g, b, c[0], bar_y, c[2] * t, 2.0, argb(accent, 1.0));
            panel(g, b, c[0] + c[2] * t - 2.5, c[1] + 1.0, 5.0, c[3] - 2.0, argb(0xFFFFFF, 1.0));
            let vt = plain(&fmt(v));
            let tw = width(g, &vt) * 0.75;
            draw(g, b, &vt, c[0] - tw - 5.0, c[1] + 2.5, 0.75, argb(0xC8C8C8, 1.0), false);
        }
        Row::Swatch(name, get, _, _) => {
            label(b, name);
            let cur = get(cfg);
            let c = control_rect(r);
            let n = SWATCHES.len() as f32;
            let side = ((c[2] - (n - 1.0)) / n).min(12.0);
            for (i, sw) in SWATCHES.iter().enumerate() {
                let x = c[0] + c[2] - (n - i as f32) * (side + 1.0);
                let rr = [x, c[1] + 1.0, side, side];
                if *sw == cur {
                    outline(g, b, rr[0] - 1.0, rr[1] - 1.0, rr[2] + 2.0, rr[3] + 2.0, argb(0xFFFFFF, 1.0));
                }
                fill(g, b, rr[0], rr[1], rr[2], rr[3], argb(*sw, 1.0));
            }
            if !SWATCHES.contains(&cur) {
                let t = plain(&format!("#{cur:06X}"));
                draw(g, b, &t, c[0] - 40.0, c[1] + 3.0, 0.75, argb(cur, 1.0), false);
            }
        }
        Row::Input(name, field) => {
            label(b, name);
            let c = control_rect(r);
            let focused = ui.menu.focus == Some(field);
            panel(g, b, c[0], c[1], c[2], c[3], argb(if focused { 0x1A1A20 } else { 0x26262C }, 1.0));
            if focused {
                outline(g, b, c[0], c[1], c[2], c[3], argb(accent, 1.0));
            }
            let value = if focused { ui.menu.buffer.clone() } else { field_value(cfg, field) };
            let shown = if field == Field::ClientId && !focused && value.len() > 8 {
                format!("{}…{}", &value[..4], &value[value.len() - 4..])
            } else {
                value
            };
            let caret = if focused && (secs(ui.started) * 2.0) as u32 % 2 == 0 { "_" } else { "" };
            let t = plain(&format!("{shown}{caret}"));
            let s = 0.85;
            let start = b.vertices.len();
            let tw = width(g, &t) * s;
            let x = if tw > c[2] - 6.0 { c[0] + c[2] - 3.0 - tw } else { c[0] + 3.0 };
            draw(g, b, &t, x, c[1] + (c[3] - 8.0 * s) / 2.0, s, argb(if shown_empty(&t) { 0x707070 } else { 0xFFFFFF }, 1.0), false);
            b.clip_since(start, c[0] + 1.0, c[1], c[0] + c[2] - 1.0, c[1] + c[3]);
            if !focused && field_value(cfg, field).is_empty() {
                draw(g, b, &plain("click, then type or Ctrl+V"), c[0] + 3.0, c[1] + 3.0, 0.75, argb(0x707070, 1.0), false);
            }
        }
        Row::Buttons(list) => {
            for ((name, _), br) in list.iter().zip(buttons_rects(r, list.len())) {
                button(g, b, br, name, inside(mouse, br), accent, false);
            }
        }
        Row::Info(f) => {
            let s = f(snap, cfg);
            let t = plain(&s);
            let sc = 0.8;
            let shown = if width(g, &t) * sc > r[2] - 4.0 { ellipsize(g, &t, (r[2] - 4.0) / sc) } else { t };
            draw(g, b, &shown, r[0] + 2.0, r[1] + (r[3] - 8.0 * sc) / 2.0, sc, argb(0xA8A8B4, 1.0), false);
        }
        Row::Wheel => {
            let target = color_target(cfg);
            let cur = target_color(cfg, target);
            let (ch, cs, cv) = art::to_hsv(cur);
            let (c, radius, strip) = wheel_geometry(r);
            // The disc: hue round it, saturation out from the middle, at the colour's brightness.
            let (rings, segs) = (5, 36);
            for ring in 0..rings {
                let (r0, r1) = (radius * ring as f32 / rings as f32, radius * (ring + 1) as f32 / rings as f32);
                for k in 0..segs {
                    let (a0, a1) = (k as f32 / segs as f32 * std::f32::consts::TAU, (k + 1) as f32 / segs as f32 * std::f32::consts::TAU);
                    let p = |rad: f32, a: f32| [c[0] + rad * a.cos(), c[1] + rad * a.sin()];
                    let hue = (k as f32 + 0.5) / segs as f32 * 360.0;
                    let sat = (ring as f32 + 0.5) / rings as f32;
                    let col = art::hsv_rgb(hue, sat, cv.max(0.25));
                    g.native_quad(b, [p(r0, a0), p(r1, a0), p(r1, a1), p(r0, a1)], argb(col, 1.0));
                }
            }
            // Where the colour is on it.
            let (mx, my) = (c[0] + cs * radius * ch.to_radians().cos(), c[1] + cs * radius * ch.to_radians().sin());
            outline(g, b, mx - 2.5, my - 2.5, 5.0, 5.0, argb(0xFFFFFF, 1.0));
            outline(g, b, mx - 3.5, my - 3.5, 7.0, 7.0, argb(0x000000, 1.0));
            // Brightness, top bright to bottom dark.
            let steps = 16;
            for i in 0..steps {
                let v = 1.0 - i as f32 / steps as f32;
                fill(g, b, strip[0], strip[1] + strip[3] * i as f32 / steps as f32, strip[2], strip[3] / steps as f32 + 0.5, argb(art::hsv_rgb(ch, cs, v), 1.0));
            }
            let vy = strip[1] + (1.0 - cv) * strip[3];
            fill(g, b, strip[0] - 2.0, vy - 0.5, strip[2] + 4.0, 1.5, argb(0xFFFFFF, 1.0));
            // What it sets, and the gradient as it is.
            let px = strip[0] + strip[2] + 12.0;
            let label = format!("{}: #{cur:06X}", COLOR_TARGETS[target]);
            draw(g, b, &plain(&label), px, r[1] + 6.0, 0.8, argb(0xE6E6E6, 1.0), false);
            fill(g, b, px, r[1] + 17.0, 14.0, 10.0, argb(cur, 1.0));
            let bar = [px, r[1] + 34.0, (r[0] + r[2] - px - 6.0).max(20.0), 10.0];
            let n = 32;
            for i in 0..n {
                let t = i as f32 / (n - 1) as f32;
                fill(g, b, bar[0] + bar[2] * i as f32 / n as f32, bar[1], bar[2] / n as f32 + 0.5, bar[3], argb(art::mix(cfg.gradient_a, cfg.gradient_b, t), 1.0));
            }
            let sample = text("Lyrics look like this", &style(&cfg.lyrics_font, true, false));
            let start = b.vertices.len();
            let sw = draw(g, b, &sample, px, r[1] + 50.0, 1.0, argb(0xFFFFFF, 1.0), false);
            recolor(b, start, px, px + sw, cfg.gradient_a, cfg.gradient_b);
        }
        Row::FontItem(i) => {
            let Some((id, name)) = ui.fonts.get(i) else { return };
            let lyr = cfg.lyrics_font == *id;
            let tit = cfg.title_font == *id;
            if lyr || tit {
                panel(g, b, r[0], r[1], r[2], r[3] - 1.0, argb(accent, 0.18));
            }
            let sample = text(name, &style(id, false, false));
            let start = b.vertices.len();
            draw(g, b, &sample, r[0] + 6.0, r[1] + 4.0, 1.15, argb(0xFFFFFF, 1.0), false);
            b.clip_since(start, r[0], r[1], r[0] + r[2] - 70.0, r[1] + r[3]);
            let tag = match (lyr, tit) {
                (true, true) => "lyrics + title",
                (true, false) => "lyrics",
                (false, true) => "title",
                _ => if id.starts_with("ttf:") { "" } else { "game font" },
            };
            let t = plain(tag);
            let tw = width(g, &t) * 0.7;
            draw(g, b, &t, r[0] + r[2] - tw - 4.0, r[1] + 6.0, 0.7, argb(if lyr || tit { accent } else { 0x808080 }, 1.0), false);
        }
        Row::SongItem(i) => {
            let Some(song) = ui.songs.get(i) else { return };
            let st = engine::player::state();
            let on = st.current == Some(i);
            let hover = inside(mouse, r);
            if on || hover {
                panel(g, b, r[0], r[1], r[2], r[3] - 1.0, argb(accent, if on { 0.22 } else { 0.10 }));
            }
            let mark = if on { if st.playing { "▶" } else { "❚❚" } } else { "" };
            if !mark.is_empty() {
                draw(g, b, &plain(mark), r[0] + 3.0, r[1] + 6.0, 0.8, argb(accent, 1.0), false);
            }
            let start = b.vertices.len();
            let title = text(&song.title, &style(&cfg.title_font, on, false));
            draw(g, b, &title, r[0] + 14.0, r[1] + 2.0, 0.9, argb(0xFFFFFF, 1.0), false);
            let sub = if song.album.is_empty() { song.artist.clone() } else { format!("{} · {}", song.artist, song.album) };
            draw(g, b, &plain(&sub), r[0] + 14.0, r[1] + 11.5, 0.65, argb(0x9A9AA6, 1.0), false);
            b.clip_since(start, r[0], r[1], r[0] + r[2] - 30.0, r[1] + r[3]);
            if song.lrc {
                let t = plain("LRC");
                let tw = width(g, &t) * 0.65;
                draw(g, b, &t, r[0] + r[2] - tw - 4.0, r[1] + 7.0, 0.65, argb(accent, 1.0), false);
            }
        }
        Row::FontPick(name, lyrics) => {
            let id = if lyrics { cfg.lyrics_font.clone() } else { cfg.title_font.clone() };
            draw(g, b, &plain(name), r[0] + 2.0, r[1] + 2.0, 1.0, argb(0xE6E6E6, 1.0), false);
            let c = control_rect([r[0], r[1], r[2], 15.0]);
            panel(g, b, c[0], c[1], c[2], c[3], argb(if inside(mouse, c) { 0x34343E } else { 0x26262C }, 1.0));
            let v = plain(&font_label(&id));
            let tw = (width(g, &v) * 0.85).min(c[2] - 20.0);
            draw(g, b, &v, c[0] + (c[2] - tw) / 2.0, c[1] + 3.0, 0.85, argb(0xFFFFFF, 1.0), false);
            draw(g, b, &plain("‹"), c[0] + 3.0, c[1] + 2.0, 1.0, argb(accent, 1.0), false);
            draw(g, b, &plain("›"), c[0] + c[2] - 7.0, c[1] + 2.0, 1.0, argb(accent, 1.0), false);
            // A preview of the song's title (or a sample) in that font.
            let sample = snap.track.as_ref().map_or("The quick brown fox ♪".to_owned(), |t| t.title.clone());
            let st = style(&id, lyrics && cfg.bold_active, lyrics && cfg.italic);
            let p = text(&sample, &st);
            let start = b.vertices.len();
            draw(g, b, &p, r[0] + 12.0, r[1] + 15.0, 1.0, argb(if lyrics { ui.active_color() } else { 0xFFFFFF }, 1.0), true);
            b.clip_since(start, r[0], r[1] + 13.0, r[0] + r[2], r[1] + r[3]);
        }
    }
}

fn shown_empty(t: &Text) -> bool {
    t.plain().trim_end_matches('_').is_empty()
}

fn act(game: &Game<'_>, ui: &mut Ui, action: Action) {
    let e = eng(game);
    match action {
        Action::Done => {
            finish_field(e, ui);
            ui.pending.push(Pending::CloseScreen);
        }
        Action::Layout => {
            finish_field(e, ui);
            ui.menu.layout = true;
        }
        Action::SpotifyConnect => {
            finish_field(e, ui);
            match e.spotify_connect() {
                Ok(url) => {
                    ui.pending.push(Pending::Copy(url));
                    ui.pending.push(Pending::Chat("[LyricsPlus] Opening Spotify in your browser (the link is also on your clipboard)…".into()));
                }
                Err(msg) => ui.pending.push(Pending::Chat(format!("[LyricsPlus] {msg}"))),
            }
        }
        Action::SpotifyDisconnect => e.spotify_disconnect(),
        Action::SpotifyDashboard => {
            engine::open_browser(SPOTIFY_DASHBOARD);
        }
        Action::CopyRedirect => {
            ui.pending.push(Pending::Copy(SPOTIFY_REDIRECT.into()));
            ui.pending.push(Pending::Chat(format!("[LyricsPlus] Copied {SPOTIFY_REDIRECT}")));
        }
        Action::InstallUserscript => {
            let url = format!("http://127.0.0.1:{}/lyricsplus.user.js", ui.cfg.bridge_port);
            if !engine::open_browser(&url) {
                ui.pending.push(Pending::Copy(url.clone()));
            }
            ui.pending.push(Pending::Chat(format!("[LyricsPlus] Opened {url}: your userscript manager offers to install it.")));
        }
        Action::CopyBridgeUrl => {
            let url = format!("http://127.0.0.1:{}/update", ui.cfg.bridge_port);
            ui.pending.push(Pending::Copy(url.clone()));
            ui.pending.push(Pending::Chat(format!("[LyricsPlus] Copied {url} (POST JSON: title, artist, positionMs, durationMs, isPlaying, artUrl, lyrics)")));
        }
        Action::OpenFolder => {
            let dir = e.data_dir();
            let _ = std::fs::create_dir_all(&dir);
            let url = format!("file:///{}", dir.display().to_string().replace('\\', "/").trim_start_matches('/'));
            engine::open_browser(&url);
        }
        Action::OpenFonts => {
            let dir = fonts_dir(e);
            let _ = std::fs::create_dir_all(&dir);
            let url = format!("file:///{}", dir.display().to_string().replace('\\', "/").trim_start_matches('/'));
            engine::open_browser(&url);
            ui.pending.push(Pending::Chat(format!("[LyricsPlus] Fonts folder: {}", dir.display())));
        }
        Action::ReloadFonts => ui.pending.push(Pending::ReloadFonts),
        Action::MusicPrevious => engine::player::control(e, Control::Previous),
        Action::MusicToggle => engine::player::control(e, Control::PlayPause),
        Action::MusicNext => engine::player::control(e, Control::Next),
        Action::MusicStop => engine::player::stop(),
        Action::OpenMusic => {
            let dir = engine::player::folder(e);
            let _ = std::fs::create_dir_all(&dir);
            let url = format!("file:///{}", dir.display().to_string().replace('\\', "/").trim_start_matches('/'));
            engine::open_browser(&url);
            ui.pending.push(Pending::Chat(format!("[LyricsPlus] Music folder: {}", dir.display())));
        }
        Action::Rescan => {
            engine::player::rescan(e);
            ui.songs_at = None;
        }
        Action::ReloadLyrics => e.reload_lyrics(),
        Action::OffsetMinus => e.update_config(|c| c.offset_ms -= 250),
        Action::OffsetPlus => e.update_config(|c| c.offset_ms += 250),
        Action::Previous => e.control(Control::Previous),
        Action::PlayPause => e.control(Control::PlayPause),
        Action::Next => e.control(Control::Next),
        Action::Reset => e.reset_config(),
    }
}

fn finish_field(e: &engine::Engine, ui: &mut Ui) {
    if let Some(field) = ui.menu.focus.take() {
        let v = std::mem::take(&mut ui.menu.buffer);
        commit(e, field, &v);
    }
}

/// A click or drag on the colour wheel (the disc picks hue and saturation,
/// the strip brightness) sets the chosen colour.
fn wheel_pick(e: &engine::Engine, r: [f32; 4], m: [f32; 2]) {
    let (c, radius, strip) = wheel_geometry(r);
    let target = COLOR_TARGET.load(std::sync::atomic::Ordering::Relaxed);
    e.update_config(|cfg| {
        let (h, s, v) = art::to_hsv(target_color(cfg, target));
        let (dx, dy) = (m[0] - c[0], m[1] - c[1]);
        let d = (dx * dx + dy * dy).sqrt();
        let picked = if d <= radius + 3.0 {
            let hue = dy.atan2(dx).to_degrees().rem_euclid(360.0);
            Some(art::hsv_rgb(hue, (d / radius).clamp(0.0, 1.0), if v < 0.2 { 1.0 } else { v }))
        } else if m[0] >= strip[0] - 3.0 && m[0] <= strip[0] + strip[2] + 3.0 && m[1] >= strip[1] - 3.0 && m[1] <= strip[1] + strip[3] + 3.0 {
            Some(art::hsv_rgb(h, s, 1.0 - ((m[1] - strip[1]) / strip[3]).clamp(0.0, 1.0)))
        } else {
            None
        };
        if let Some(col) = picked {
            set_target_color(cfg, target, col);
        }
    });
}

fn slider_set(e: &engine::Engine, row: Row, rect: [f32; 4], x: f32) {
    if let Row::Slider(_, _, set, lo, hi, step, _) = row {
        let c = control_rect(rect);
        let t = ((x - c[0]) / c[2]).clamp(0.0, 1.0);
        let mut v = lo + (hi - lo) * t;
        if step > 0.0 {
            v = (v / step).round() * step;
        }
        e.update_config(|cfg| set(cfg, v.clamp(lo, hi)));
    }
}

/// A click in the menu (or the layout editor). Always taken while it is open.
pub(crate) fn screen_click(game: &Game<'_>, screen: &NativeScreenView<'_>, button: i32, canceled: &mut bool) {
    if !mine(game) {
        return;
    }
    *canceled = true;
    let e = eng(game);
    let m = screen.mouse;
    with_ui(|ui| {
        if ui.menu.layout {
            let (lr, wr) = (ui.lyrics_rect, ui.widget_rect);
            if inside(m, lr) {
                if button == 1 {
                    e.update_config(|c| c.lyrics_position = LyricsPosition::AboveCrosshair);
                } else {
                    let center = [lr[0] + lr[2] / 2.0, lr[1] + lr[3] / 2.0];
                    ui.menu.drag = Some(Drag::Lyrics([m[0] - center[0], m[1] - center[1]]));
                }
            } else if inside(m, wr) {
                if button == 1 {
                    e.update_config(|c| c.widget_corner = Corner::BottomRight);
                } else {
                    ui.menu.drag = Some(Drag::Widget([m[0] - wr[0], m[1] - wr[1]]));
                }
            } else if button == 1 {
                ui.menu.layout = false;
            }
            return;
        }
        let f = frame_of(ui.menu.size);
        if button == 1 && !inside(m, [f.x, f.content_top, f.w, f.content_bottom - f.content_top]) {
            ui.pending.push(Pending::CloseScreen);
            return;
        }
        for (i, r) in tab_rects(&f).iter().enumerate() {
            if inside(m, *r) {
                finish_field(e, ui);
                ui.menu.tab = i;
                ui.menu.scroll = 0.0;
                return;
            }
        }
        let [done, layout] = footer_rects(&f);
        if inside(m, done) {
            act(game, ui, Action::Done);
            return;
        }
        if inside(m, layout) {
            act(game, ui, Action::Layout);
            return;
        }
        if m[1] < f.content_top || m[1] > f.content_bottom {
            finish_field(e, ui);
            return;
        }
        let rows = rows(ui.menu.tab, ui.fonts.len(), ui.songs.len());
        let rects = row_rects(&f, &rows, ui.menu.scroll);
        let back = button == 1;
        let mut clicked_field = None;
        for (i, (row, r)) in rows.iter().zip(&rects).enumerate() {
            if !inside(m, *r) {
                continue;
            }
            match *row {
                Row::Toggle(_, get, set) => e.update_config(|c| {
                    let v = get(c);
                    set(c, !v)
                }),
                Row::Cycle(name, _, cycle) => {
                    if name == "Colour wheel sets" {
                        let n = COLOR_TARGETS.len();
                        let cur = COLOR_TARGET.load(std::sync::atomic::Ordering::Relaxed);
                        COLOR_TARGET.store(if back { (cur + n - 1) % n } else { (cur + 1) % n }, std::sync::atomic::Ordering::Relaxed);
                    } else if name == "Pick a font for" {
                        let n = FONT_TARGETS.len();
                        let cur = FONT_TARGET.load(std::sync::atomic::Ordering::Relaxed);
                        let next = if back { (cur + n - 1) % n } else { (cur + 1) % n };
                        FONT_TARGET.store(next, std::sync::atomic::Ordering::Relaxed);
                    } else {
                        e.update_config(|c| cycle(c, back));
                    }
                }
                Row::Wheel => {
                    wheel_pick(e, *r, m);
                    ui.menu.drag = Some(Drag::Wheel(i));
                }
                Row::FontItem(k) => {
                    if let Some((id, _)) = ui.fonts.get(k).cloned() {
                        let target = FONT_TARGET.load(std::sync::atomic::Ordering::Relaxed);
                        e.update_config(|c| {
                            if target != 1 {
                                c.lyrics_font = id.clone();
                            }
                            if target != 0 {
                                c.title_font = id.clone();
                            }
                        });
                    }
                }
                Row::SongItem(k) => engine::player::play(e, k),
                Row::Slider(..) => {
                    if inside(m, control_rect(*r)) {
                        slider_set(e, *row, *r, m[0]);
                        ui.menu.drag = Some(Drag::Slider(i));
                    }
                }
                Row::Swatch(_, _, set, field) => {
                    let c = control_rect(*r);
                    let n = SWATCHES.len() as f32;
                    let side = ((c[2] - (n - 1.0)) / n).min(12.0);
                    for (k, sw) in SWATCHES.iter().enumerate() {
                        let x = c[0] + c[2] - (n - k as f32) * (side + 1.0);
                        if inside(m, [x, c[1], side + 1.0, side + 2.0]) {
                            let v = *sw;
                            e.update_config(|cfg| set(cfg, v));
                        }
                    }
                    let _ = field;
                }
                Row::Input(_, field) => {
                    if inside(m, control_rect(*r)) {
                        clicked_field = Some(field);
                    }
                }
                Row::Buttons(list) => {
                    for ((_, a), br) in list.iter().zip(buttons_rects(*r, list.len())) {
                        if inside(m, br) {
                            act(game, ui, *a);
                        }
                    }
                }
                Row::FontPick(_, lyrics) => {
                    let fonts: Vec<String> = ui.fonts.iter().map(|f| f.0.clone()).collect();
                    if fonts.is_empty() {
                        continue;
                    }
                    e.update_config(|c| {
                        let cur = if lyrics { &c.lyrics_font } else { &c.title_font };
                        let at = fonts.iter().position(|f| f == cur).unwrap_or(0);
                        let n = fonts.len();
                        let next = fonts[if back { (at + n - 1) % n } else { (at + 1) % n }].clone();
                        if lyrics {
                            c.lyrics_font = next;
                        } else {
                            c.title_font = next;
                        }
                    });
                }
                Row::Header(_) | Row::Info(_) => {}
            }
        }
        if clicked_field != ui.menu.focus {
            finish_field(e, ui);
            if let Some(field) = clicked_field {
                ui.menu.focus = Some(field);
                ui.menu.buffer = field_value(&e.config(), field);
            }
        }
    });
}

pub(crate) fn screen_release(game: &Game<'_>, screen: &NativeScreenView<'_>, canceled: &mut bool) {
    if !mine(game) {
        return;
    }
    *canceled = true;
    let e = eng(game);
    let m = screen.mouse;
    with_ui(|ui| {
        let drag = ui.menu.drag.take();
        let (w, h) = (ui.menu.size[0].max(1.0), ui.menu.size[1].max(1.0));
        match drag {
            Some(Drag::Lyrics(off)) => {
                let (x, y) = (((m[0] - off[0]) / w).clamp(0.0, 1.0), ((m[1] - off[1]) / h).clamp(0.0, 1.0));
                e.update_config(|c| {
                    c.lyrics_position = LyricsPosition::Custom;
                    c.lyrics_x = x;
                    c.lyrics_y = y;
                });
            }
            Some(Drag::Widget(off)) => {
                let (x, y) = (((m[0] - off[0]) / w).clamp(0.0, 1.0), ((m[1] - off[1]) / h).clamp(0.0, 1.0));
                e.update_config(|c| {
                    c.widget_corner = Corner::Custom;
                    c.widget_x = x;
                    c.widget_y = y;
                });
            }
            _ => {}
        }
        ui.cfg = e.config();
        ui.cfg_version = e.config_version();
    });
}

pub(crate) fn screen_drag(game: &Game<'_>, screen: &NativeScreenView<'_>, canceled: &mut bool) {
    if !mine(game) {
        return;
    }
    *canceled = true;
    let e = eng(game);
    let m = screen.mouse;
    with_ui(|ui| {
        let (w, h) = (ui.menu.size[0].max(1.0), ui.menu.size[1].max(1.0));
        match ui.menu.drag {
            Some(Drag::Wheel(i)) => {
                let f = frame_of(ui.menu.size);
                let rows = rows(ui.menu.tab, ui.fonts.len(), ui.songs.len());
                if let Some(r) = row_rects(&f, &rows, ui.menu.scroll).get(i) {
                    wheel_pick(e, *r, m);
                }
            }
            Some(Drag::Slider(i)) => {
                let f = frame_of(ui.menu.size);
                let rows = rows(ui.menu.tab, ui.fonts.len(), ui.songs.len());
                let rects = row_rects(&f, &rows, ui.menu.scroll);
                if let (Some(row), Some(r)) = (rows.get(i), rects.get(i)) {
                    slider_set(e, *row, *r, m[0]);
                }
            }
            // Moving: only the shown copy changes until the button goes up.
            Some(Drag::Lyrics(off)) => {
                ui.cfg.lyrics_position = LyricsPosition::Custom;
                ui.cfg.lyrics_x = ((m[0] - off[0]) / w).clamp(0.0, 1.0);
                ui.cfg.lyrics_y = ((m[1] - off[1]) / h).clamp(0.0, 1.0);
            }
            Some(Drag::Widget(off)) => {
                ui.cfg.widget_corner = Corner::Custom;
                ui.cfg.widget_x = ((m[0] - off[0]) / w).clamp(0.0, 1.0);
                ui.cfg.widget_y = ((m[1] - off[1]) / h).clamp(0.0, 1.0);
            }
            None => {}
        }
    });
}

pub(crate) fn screen_scroll(game: &Game<'_>, scroll: [f64; 2], canceled: &mut bool) {
    if !mine(game) {
        return;
    }
    *canceled = true;
    with_ui(|ui| {
        let f = frame_of(ui.menu.size);
        let view = f.content_bottom - f.content_top;
        let max = (ui.menu.content_height - view).max(0.0);
        ui.menu.scroll = (ui.menu.scroll - scroll[1] as f32 * 16.0).clamp(0.0, max);
    });
}

/// Keys in the menu: editing a field, leaving the layout editor. Escape
/// closes the menu (the game's own handling) unless a field or the layout
/// editor takes it.
pub(crate) fn screen_key(game: &Game<'_>, key: &str, modifiers: u32, canceled: &mut bool) {
    if !mine(game) {
        return;
    }
    let e = eng(game);
    with_ui(|ui| {
        let ctrl = modifiers & (2 | 8) != 0;
        if ui.menu.layout {
            if key == "key.keyboard.escape" || key == "key.keyboard.enter" {
                ui.menu.layout = false;
                *canceled = true;
            }
            return;
        }
        if ui.menu.focus.is_none() {
            return;
        }
        *canceled = true;
        match key {
            "key.keyboard.escape" => {
                ui.menu.focus = None;
                ui.menu.buffer.clear();
            }
            "key.keyboard.enter" | "key.keyboard.keypad.enter" | "key.keyboard.tab" => finish_field(e, ui),
            "key.keyboard.backspace" => {
                if ctrl {
                    ui.menu.buffer.clear();
                } else {
                    ui.menu.buffer.pop();
                }
            }
            "key.keyboard.v" if ctrl => ui.pending.push(Pending::Paste),
            "key.keyboard.c" if ctrl => ui.pending.push(Pending::Copy(ui.menu.buffer.clone())),
            _ => {}
        }
    });
}

pub(crate) fn screen_char(game: &Game<'_>, c: char, modifiers: u32, canceled: &mut bool) {
    if !mine(game) {
        return;
    }
    with_ui(|ui| {
        if ui.menu.focus.is_none() || modifiers & (2 | 8) != 0 || c.is_control() {
            return;
        }
        *canceled = true;
        if ui.menu.buffer.chars().count() < 96 {
            ui.menu.buffer.push(c);
        }
    });
}

// ---------------------------------------------------------------- command

pub(crate) fn command(game: &mut Game<'_>, command: &str, ran: &mut bool) {
    let mut words = command.split_whitespace();
    let Some(head) = words.next() else { return };
    if !(head.eq_ignore_ascii_case("lyrics") || head.eq_ignore_ascii_case("lyricsplus")) {
        return;
    }
    *ran = true;
    let e = eng(game);
    let sub = words.next().unwrap_or("").to_lowercase();
    let rest: Vec<&str> = words.collect();
    let arg = rest.join(" ");
    match sub.as_str() {
        "" | "menu" | "config" | "settings" => {
            with_ui(|ui| {
                ui.menu.focus = None;
                ui.menu.layout = false;
                ui.pending.push(Pending::OpenScreen(40));
            });
        }
        "on" => e.update_config(|c| c.enabled = true),
        "off" => e.update_config(|c| c.enabled = false),
        "toggle" | "hide" | "show" => {
            let hidden = with_ui(|ui| {
                ui.hidden = sub == "hide" || (sub == "toggle" && !ui.hidden);
                ui.hidden
            });
            say(game, if hidden { "Hidden." } else { "Shown." });
        }
        "mode" | "display" => {
            let d = match arg.to_lowercase().as_str() {
                "hud" | "2d" => Some(Display::Hud),
                "3d" | "world" => Some(Display::World),
                "both" => Some(Display::Both),
                _ => None,
            };
            match d {
                Some(d) => e.update_config(|c| c.display = d),
                None => say(game, "Usage: /lyrics mode <hud|3d|both>"),
            }
        }
        "source" => {
            let s = match arg.to_lowercase().as_str() {
                "auto" => Some(Source::Auto),
                "spotify" => Some(Source::Spotify),
                "system" | "media" => Some(Source::System),
                "bridge" | "browser" => Some(Source::Bridge),
                "local" | "offline" | "mp3" | "player" => Some(Source::Local),
                _ => None,
            };
            match s {
                Some(s) => e.update_config(|c| c.source = s),
                None => say(game, "Usage: /lyrics source <auto|spotify|system|bridge|local>"),
            }
        }
        "spotify" | "clientid" => {
            if arg.is_empty() {
                say(game, "Usage: /lyrics spotify <Client ID>  (then /lyrics connect)");
            } else {
                commit(e, Field::ClientId, &arg);
                say(game, "Client ID saved. Now: /lyrics connect");
            }
        }
        "connect" | "link" => match e.spotify_connect() {
            Ok(url) => {
                use horizon_client::clipboard::Clipboard;
                game.clipboard.set(&url);
                say(game, "Opening Spotify in your browser (the link is also on your clipboard)…");
            }
            Err(m) => say(game, &m),
        },
        "disconnect" | "unlink" => {
            e.spotify_disconnect();
            say(game, "Spotify unlinked.");
        }
        "offset" => {
            let n = arg.trim_start_matches('+').trim_end_matches("ms").parse::<i64>();
            match n {
                Ok(v) if arg.starts_with('+') || arg.starts_with('-') => e.update_config(|c| c.offset_ms += v),
                Ok(v) => e.update_config(|c| c.offset_ms = v),
                Err(_) => say(game, "Usage: /lyrics offset <ms | +ms | -ms>"),
            }
            let o = e.config().offset_ms;
            say(game, &format!("Offset: {o:+} ms"));
        }
        "font" | "lyricsfont" => {
            if arg.is_empty() {
                say(game, "Usage: /lyrics font <id> (default, uniform, alt, illageralt, or a pack's namespace:name)");
            } else {
                commit(e, Field::LyricsFont, &arg);
            }
        }
        "titlefont" => {
            if !arg.is_empty() {
                commit(e, Field::TitleFont, &arg);
            }
        }
        "reload" => {
            e.reload_lyrics();
            say(game, "Fetching lyrics again…");
        }
        "next" | "skip" => e.control(Control::Next),
        "pvp" => {
            let mut on = false;
            e.update_config(|c| {
                c.pvp_mode = !c.pvp_mode;
                on = c.pvp_mode;
            });
            say(game, if on { "PvP mode on: lyrics follow you, split to the sides." } else { "PvP mode off." });
        }
        "prev" | "previous" | "back" => e.control(Control::Previous),
        "pause" | "play" => e.control(Control::PlayPause),
        "bridge" => {
            let port = e.config().bridge_port;
            say(game, &format!("Browser bridge: http://127.0.0.1:{port}/update  ·  userscript: http://127.0.0.1:{port}/lyricsplus.user.js  ·  now playing JSON: http://127.0.0.1:{port}/now"));
        }
        "status" => {
            let s = e.snapshot();
            let c = e.config();
            let track = s.track.as_ref().map_or("nothing".to_owned(), |t| format!("{} – {} via {:?} ({})", t.title, t.artist, t.origin, t.app));
            say(game, &format!("Playing: {track}; lyrics: {:?} ({} lines{}); source setting: {}; Spotify: {:?}", s.lyrics_state, s.lyrics.lines.len(), if s.lyrics.synced { ", synced" } else { "" }, c.source.label(), s.spotify));
        }
        _ => say(game, "/lyrics [menu|on|off|toggle|mode <hud|3d|both>|source <auto|spotify|system|bridge|local>|spotify <client id>|connect|disconnect|offset <ms>|font <id>|titlefont <id>|reload|next|prev|pause|pvp|bridge|status]"),
    }
}
