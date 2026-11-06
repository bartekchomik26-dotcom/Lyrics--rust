//! Settings, kept as `config.json` in the mod's data folder. Every field has
//! a default, so an older or hand-edited file still loads.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Declares a settings enum with its stored names and labels, cycled by the menu.
macro_rules! choice {
    ($name:ident { $($v:ident => $label:expr),+ $(,)? } default $d:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
        pub enum $name { $($v),+ }
        impl Default for $name { fn default() -> Self { Self::$d } }
        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$v),+];
            pub fn label(self) -> &'static str { match self { $(Self::$v => $label),+ } }
            pub fn index(self) -> usize { Self::ALL.iter().position(|v| *v == self).unwrap_or(0) }
            /// The next (or previous) choice, wrapping round.
            pub fn cycle(self, back: bool) -> Self {
                let n = Self::ALL.len();
                let i = self.index();
                Self::ALL[if back { (i + n - 1) % n } else { (i + 1) % n }]
            }
        }
    };
}

choice!(Display { Hud => "HUD (2D)", World => "World (3D)", Both => "2D + 3D" } default Hud);
choice!(Source {
    Auto => "Auto",
    Spotify => "Spotify Web API",
    System => "System media",
    Bridge => "Browser bridge",
    Local => "Offline player",
} default Auto);
choice!(LyricsPosition {
    Top => "Top",
    AboveCrosshair => "Above crosshair",
    Center => "Center",
    BelowCrosshair => "Below crosshair",
    Bottom => "Above hotbar",
    TopLeft => "Top left",
    TopRight => "Top right",
    Custom => "Custom",
} default AboveCrosshair);
choice!(Corner {
    TopLeft => "Top left",
    TopRight => "Top right",
    BottomLeft => "Bottom left",
    BottomRight => "Bottom right",
    Custom => "Custom",
} default BottomRight);
choice!(ToastCorner {
    TopLeft => "Top left",
    BottomLeft => "Bottom left",
    TopRight => "Top right",
} default TopLeft);
choice!(Align { Center => "Center", Left => "Left", Right => "Right" } default Center);
choice!(Animation {
    Fade => "Fade",
    Slide => "Slide up",
    Pop => "Pop",
    Typewriter => "Typewriter",
    Ascend => "Ascend",
    SlideSide => "Slide in",
} default Slide);
choice!(Highlight {
    Bounce => "Jump & pop",
    Word => "Current word pops",
    OneWord => "One word at a time",
    Sweep => "Karaoke sweep",
    Solid => "Solid",
    Glow => "Glow outline",
    Kick => "Kick (rocks right, then left)",
} default Bounce);
choice!(Arrangement {
    Line => "One line",
    Spread => "Separated",
    Scramble => "Scrambled",
    Split => "Split (centre clear)",
    Stack => "Stacked",
    Stairs => "Stairs",
    Arc => "Arc",
    Aura => "Aura (around you)",
} default Line);
choice!(Motion {
    Still => "Still",
    Wave => "Wavy",
    Float => "Floating",
    Breathe => "Breathing",
} default Still);
choice!(Vanish {
    Off => "Off",
    Sung => "Sung words vanish",
    Wipe => "Wipe left to right",
} default Off);
choice!(WidgetStyle { Full => "Full", Compact => "Compact", Mini => "Mini" } default Full);
choice!(Anchor {
    Stay => "Stays where it appears",
    Follow => "Follows you",
} default Stay);
choice!(Billboard { Yaw => "Turn only", Full => "Face camera", Fixed => "Fixed" } default Full);

/// Every setting.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    // General
    pub enabled: bool,
    pub display: Display,
    pub source: Source,
    /// Added to the position lyrics are looked up at (ms; positive shows them earlier).
    pub offset_ms: i64,
    pub hide_when_paused: bool,
    pub hide_in_screens: bool,
    pub adaptive_colors: bool,
    pub accent: u32,
    pub combat_fade: bool,

    // Lyrics (2D)
    pub show_lyrics: bool,
    pub lyrics_position: LyricsPosition,
    pub lyrics_x: f32,
    pub lyrics_y: f32,
    pub lyrics_align: Align,
    pub lines: u32,
    pub lyrics_scale: f32,
    pub lyrics_background: f32,
    pub lyrics_font: String,
    pub active_color: u32,
    pub inactive_color: u32,
    pub highlight: Highlight,
    pub bold_active: bool,
    pub italic: bool,
    pub uppercase: bool,
    pub shadow: bool,
    pub smooth: bool,
    pub countdown: bool,
    pub show_status: bool,
    pub max_width: f32,

    // Player widget
    pub show_widget: bool,
    pub widget_style: WidgetStyle,
    pub widget_corner: Corner,
    pub widget_x: f32,
    pub widget_y: f32,
    pub widget_scale: f32,
    pub widget_alpha: f32,
    pub show_art: bool,
    pub show_progress: bool,
    pub show_time: bool,
    pub equalizer: bool,
    pub marquee: bool,
    pub title_font: String,

    // Toast
    pub show_toast: bool,
    pub toast_corner: ToastCorner,
    pub toast_seconds: f32,

    // 3D
    pub distance_3d: f32,
    pub height_3d: f32,
    pub scale_3d: f32,
    pub follow_3d: f32,
    pub anchor_3d: Anchor,
    pub billboard_3d: Billboard,
    pub two_sided_3d: bool,
    pub animation_3d: Animation,
    pub art_3d: bool,
    pub background_3d: f32,
    pub outline_3d: bool,
    /// Words spread apart (0 off, 1 wide), still readable.
    pub scatter: f32,
    /// A burst of sparks as a line finishes.
    pub sparks: bool,
    /// A colour gradient across the lyrics.
    pub gradient: bool,
    /// The gradient taken from the album art.
    pub gradient_album: bool,
    pub gradient_a: u32,
    pub gradient_b: u32,
    /// The island at the top middle of the screen.
    pub island: bool,
    pub island_scale: f32,
    /// Island: from the top edge (GUI pixels), width and height factors,
    /// roundness (0 square, 1 pill), background and outline opacity.
    pub island_y: f32,
    pub island_width: f32,
    pub island_height: f32,
    pub island_round: f32,
    pub island_bg: f32,
    pub island_bg_color: u32,
    pub island_outline: f32,
    pub island_outline_color: u32,
    pub island_cover: bool,
    pub island_time: bool,
    pub island_progress: bool,
    /// How the words of a line are placed, and how far (`scatter`, 0 to 1).
    pub arrangement: Arrangement,
    /// Idle movement of the text.
    pub motion: Motion,
    pub vanish: Vanish,
    /// PvP: lyrics follow you, split to the sides of the crosshair.
    pub pvp_mode: bool,
    /// PvP mode while you're fighting (attacking), off after.
    pub pvp_auto: bool,
    pub pvp_distance: f32,
    /// Lines fit into tight spaces (closer, smaller, round you when cramped).
    pub fit_tight: bool,
    // Player card
    pub widget_round: f32,
    pub art_round: f32,
    pub art_size: f32,
    pub widget_width: f32,
    pub show_title: bool,
    pub show_artist: bool,
    pub show_album: bool,
    pub show_source: bool,
    pub height_random_3d: f32,
    pub elytra: bool,
    pub elytra_lead: f32,
    pub elytra_scale: f32,

    // Sources
    pub spotify_client_id: String,
    pub spotify_poll_ms: u32,
    pub bridge: bool,
    pub bridge_port: u16,
    pub system_media: bool,

    // Offline player
    /// Empty: `<data>/music`.
    pub music_folder: String,
    pub volume: f32,
    pub shuffle: bool,
    pub repeat: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            display: Display::World,
            source: Source::Auto,
            offset_ms: 0,
            hide_when_paused: false,
            hide_in_screens: true,
            adaptive_colors: true,
            accent: 0x1DB954,
            combat_fade: true,

            show_lyrics: true,
            lyrics_position: LyricsPosition::AboveCrosshair,
            lyrics_x: 0.5,
            lyrics_y: 0.3,
            lyrics_align: Align::Center,
            lines: 1,
            lyrics_scale: 1.25,
            lyrics_background: 0.35,
            lyrics_font: "ttf:playfair".into(),
            active_color: 0xFFFFFF,
            inactive_color: 0xA0A0A0,
            highlight: Highlight::Bounce,
            bold_active: true,
            italic: false,
            uppercase: false,
            shadow: true,
            smooth: true,
            countdown: true,
            show_status: true,
            max_width: 0.7,

            show_widget: true,
            widget_style: WidgetStyle::Full,
            widget_corner: Corner::TopLeft,
            widget_x: 0.02,
            widget_y: 0.75,
            widget_scale: 1.0,
            widget_alpha: 0.75,
            show_art: true,
            show_progress: true,
            show_time: true,
            equalizer: true,
            marquee: true,
            title_font: "minecraft:default".into(),

            show_toast: true,
            toast_corner: ToastCorner::BottomLeft,
            toast_seconds: 4.0,

            distance_3d: 12.0,
            height_3d: 0.4,
            scale_3d: 3.5,
            follow_3d: 0.12,
            anchor_3d: Anchor::Stay,
            billboard_3d: Billboard::Full,
            two_sided_3d: true,
            animation_3d: Animation::Fade,
            art_3d: false,
            background_3d: 0.0,
            outline_3d: true,
            scatter: 0.0,
            sparks: true,
            gradient: true,
            gradient_album: true,
            gradient_a: 0x4FC3F7,
            gradient_b: 0xFF6FD8,
            island: false,
            island_scale: 1.0,
            island_y: 4.0,
            island_width: 1.0,
            island_height: 1.0,
            island_round: 1.0,
            island_bg: 0.88,
            island_bg_color: 0x000000,
            island_outline: 0.0,
            island_outline_color: 0xFFFFFF,
            island_cover: true,
            island_time: true,
            island_progress: true,
            arrangement: Arrangement::Line,
            motion: Motion::Still,
            vanish: Vanish::Off,
            pvp_mode: false,
            pvp_auto: false,
            pvp_distance: 5.5,
            fit_tight: true,
            widget_round: 0.25,
            art_round: 0.2,
            art_size: 1.0,
            widget_width: 1.0,
            show_title: true,
            show_artist: true,
            show_album: false,
            show_source: true,
            height_random_3d: 1.5,
            elytra: true,
            elytra_lead: 26.0,
            elytra_scale: 3.0,

            spotify_client_id: String::new(),
            spotify_poll_ms: 1000,
            bridge: true,
            bridge_port: 8974,
            system_media: true,
            music_folder: String::new(),
            volume: 0.8,
            shuffle: false,
            repeat: false,
        }
    }
}

impl Config {
    pub fn load(dir: &Path) -> Self {
        std::fs::read_to_string(dir.join("config.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Config>(&t).ok())
            .map(Config::sanitized)
            .unwrap_or_default()
    }
    pub fn save(&self, dir: &Path) {
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::create_dir_all(dir);
            let tmp = dir.join("config.json.tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, dir.join("config.json"));
            }
        }
    }
    /// Values a hand edit could put out of range, brought back in.
    pub fn sanitized(mut self) -> Self {
        let clamp = |v: f32, lo: f32, hi: f32, d: f32| if v.is_finite() { v.clamp(lo, hi) } else { d };
        self.lines = self.lines.clamp(1, 9);
        self.lyrics_scale = clamp(self.lyrics_scale, 0.5, 4.0, 1.25);
        self.lyrics_background = clamp(self.lyrics_background, 0.0, 1.0, 0.35);
        self.lyrics_x = clamp(self.lyrics_x, 0.0, 1.0, 0.5);
        self.lyrics_y = clamp(self.lyrics_y, 0.0, 1.0, 0.3);
        self.max_width = clamp(self.max_width, 0.2, 1.0, 0.7);
        self.widget_x = clamp(self.widget_x, 0.0, 1.0, 0.02);
        self.widget_y = clamp(self.widget_y, 0.0, 1.0, 0.75);
        self.widget_scale = clamp(self.widget_scale, 0.5, 3.0, 1.0);
        self.widget_alpha = clamp(self.widget_alpha, 0.0, 1.0, 0.75);
        self.toast_seconds = clamp(self.toast_seconds, 1.0, 15.0, 4.0);
        self.distance_3d = clamp(self.distance_3d, 2.0, 40.0, 12.0);
        self.height_3d = clamp(self.height_3d, -3.0, 4.0, 0.6);
        self.scale_3d = clamp(self.scale_3d, 0.25, 12.0, 3.5);
        self.follow_3d = clamp(self.follow_3d, 0.01, 1.0, 0.12);
        self.background_3d = clamp(self.background_3d, 0.0, 1.0, 0.0);
        self.scatter = clamp(self.scatter, 0.0, 1.0, 0.0);
        self.island_scale = clamp(self.island_scale, 0.5, 3.0, 1.0);
        self.island_y = clamp(self.island_y, 0.0, 200.0, 4.0);
        self.island_width = clamp(self.island_width, 0.5, 2.0, 1.0);
        self.island_height = clamp(self.island_height, 0.7, 2.5, 1.0);
        self.island_round = clamp(self.island_round, 0.0, 1.0, 1.0);
        self.island_bg = clamp(self.island_bg, 0.0, 1.0, 0.88);
        self.island_outline = clamp(self.island_outline, 0.0, 1.0, 0.0);
        self.island_bg_color &= 0xFF_FFFF;
        self.island_outline_color &= 0xFF_FFFF;
        self.pvp_distance = clamp(self.pvp_distance, 2.0, 16.0, 5.5);
        self.widget_round = clamp(self.widget_round, 0.0, 1.0, 0.25);
        self.art_round = clamp(self.art_round, 0.0, 1.0, 0.2);
        self.art_size = clamp(self.art_size, 0.5, 1.8, 1.0);
        self.widget_width = clamp(self.widget_width, 0.7, 1.8, 1.0);
        self.gradient_a &= 0xFF_FFFF;
        self.gradient_b &= 0xFF_FFFF;
        self.height_random_3d = clamp(self.height_random_3d, 0.0, 6.0, 1.5);
        self.elytra_lead = clamp(self.elytra_lead, 6.0, 80.0, 26.0);
        self.elytra_scale = clamp(self.elytra_scale, 0.5, 10.0, 3.0);
        self.spotify_poll_ms = self.spotify_poll_ms.clamp(500, 10_000);
        self.offset_ms = self.offset_ms.clamp(-30_000, 30_000);
        self.accent &= 0xFF_FFFF;
        self.volume = clamp(self.volume, 0.0, 1.0, 0.8);
        self.active_color &= 0xFF_FFFF;
        self.inactive_color &= 0xFF_FFFF;
        if self.lyrics_font.trim().is_empty() {
            self.lyrics_font = "minecraft:default".into();
        }
        if self.title_font.trim().is_empty() {
            self.title_font = "minecraft:default".into();
        }
        self.spotify_client_id = self.spotify_client_id.trim().to_owned();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_files_keep_defaults_and_choices_cycle() {
        let c: Config = serde_json::from_str(r#"{"lines": 5, "display": "Hud"}"#).unwrap();
        assert_eq!(c.lines, 5);
        assert_eq!(c.display, Display::Hud);
        assert_eq!(c.anchor_3d, Anchor::Stay);
        assert!(c.show_widget);
        assert_eq!(Display::Hud.cycle(false), Display::World);
        assert_eq!(Display::Hud.cycle(true), Display::Both);
        let bad: Config = serde_json::from_str(r#"{"lines": 99, "lyrics_scale": -3}"#).unwrap();
        let bad = bad.sanitized();
        assert_eq!(bad.lines, 9);
        assert_eq!(bad.lyrics_scale, 0.5);
    }
}
