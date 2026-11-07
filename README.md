# LyricsPlus for Horizon (native mod)

A port of the Fabric mod **LyricsPlus 3.0.0** (by FRK, MIT) to [Horizon](https://playhorizon.dev), as a
native `.hnmod` (horizon-mixins bundle). It shows real-time synced lyrics and a Spotify-style now-playing
HUD, in 2D or floating in the world, with its own settings menu.

## Features

- **Synced lyrics** from LRCLIB (exact match, then ranked search), cached on disk; plain lyrics are spread
  over the song and marked "unsynced". Karaoke sweep, glow or solid highlight; 1–9 lines; smooth slide;
  instrumental countdown; offset in ms.
- **Player widget**: Full, Compact or Mini. Album art, scrolling title, artist, progress bar, times,
  equalizer, source. Goes in any corner or anywhere you drag it.
- **Now-playing toast**: slides in from the top-left corner (or bottom-left / top-right) on every new song,
  with the cover as a square, the title and the artist.
- **Adaptive colours**: accent and panel colours taken from the album art, easing between songs.
- **Works with shader packs**: 3D lyrics are drawn in Horizon's text-display pass (a custom RETURN hook on
  `text_display_geometry`), which shader packs render as text.
- **Word sync**: the word being sung pops in the accent colour ("Current word pops"), or only that word shows
  ("One word at a time"), or a karaoke sweep. Word timings come from enhanced LRC when LRCLIB has them,
  otherwise they're spread over the line by its letters.
- **Smooth fonts**: 10 built-in fonts (Playfair Display, Cinzel, Great Vibes, Dancing Script, Lora, Poppins,
  Quicksand, Bebas Neue, Pacifico, Caveat; SIL OFL, licences in `data/fonts`), plus your own: Fonts tab >
  Open fonts folder, drop `.ttf`/`.otf` files in, Reload fonts, click one. Fonts are rasterized with
  `ab_glyph` into the mod's own atlas image.
- **3D lyrics (default)**: one line at a time appears in the world ahead of where you look and stays there
  (long lines wrap onto two rows); the sung line fades out in place as the next appears. Or set
  "Follows you" in the 3D tab. Lines appear 12 blocks ahead at a random height (0–6 blocks of jitter),
  back away if you walk up to them, and have an optional outline. Six animations: fade, slide up, pop, typewriter, ascend, slide in.
- **Moving word styles** (Style tab > Word style): *Jump & pop* (each word hops up and swells as it's
  sung, default), current word pops, one word at a time, karaoke sweep, solid, glow. A burst of **sparks**
  fires as each line finishes (2D, island and 3D).
- **More motion**: *Kick* (the word being sung rocks right side up, then left, with a wave through its letters), and an idle motion for
  any style: *Wavy* (letters ride a travelling wave), *Floating* or *Breathing*. **Vanish**: sung words
  fade away, or each sung word wipes away letter by letter, left to right (only words already sung; the
  rest of the line and the next rows stay).
- **Layouts** (Style tab > Layout, with a Spread slider): one line, separated, scrambled, split (centre
  clear), stacked, stairs, arc, or an aura around you.
- **PvP mode** (Numpad 0, `/lyrics pvp`, or automatically while fighting): the lyrics follow you, closer and
  smaller, split left and right of the crosshair.
- **Tight spaces**: in a tunnel or a small room the 3D line comes in front of the wall (same size on screen),
  shrinks to fit between the walls, and when there's no room at all its words circle you like an aura.
- **Sparks from the last word** as it's sung, in the gradient's colours.
- **Gradient + colour wheel**: a two-colour gradient across the lyrics, taken from the album art or set on the
  colour wheel in the Style tab (the wheel also sets the lyrics colour and accent).
- **Scatter**: spreads the words of a line apart (slightly tilted and offset) while keeping them readable.
- **Island**: a rounded pill at the top middle of the screen with the cover, the current line with a sliding
  highlight on the sung word, the gradient and font, `m:ss / m:ss` and a progress bar. Player tab: size,
  width, height, roundness, distance from the top, background opacity and colour (solid, see-through or
  clear), outline opacity (25/50/75/100%) and colour, and whether it shows the cover, time and progress.
- **Player card**: roundness, width, cover size and cover roundness, and which of title, artist, album,
  progress, times, source and equalizer it shows (the toast takes the same roundness).
- **Offline music player** (Music tab): plays MP3, M4A, AAC, WAV, WMA, FLAC and OGG from a music folder
  (Open music folder, Rescan), with prev/play/pause/next/stop, volume, shuffle and repeat. Titles, artists,
  albums and covers come from ID3 tags (else `Artist - Title.mp3`); a `.lrc` file with the same name gives
  lyrics without internet. It's a source like the others (top priority in Auto; media keys control it).
  Playback uses the system: Windows Media Player's engine on Windows, `mpv`/`ffplay`/`afplay` elsewhere.
- **Elytra mode**: while gliding, each line appears far ahead along your flight path, then hangs in place
  so you fly through it.
- **Fonts**: pick the lyrics font and the song-title font: default, uniform (Unicode), alt (enchanting),
  illageralt, or any font a resource pack adds (`namespace:name`). The menu previews the playing title in it.
- **Menu** on a key (default **K**), with eight tabs (General, Style, Lyrics, Player, 3D, Sources, Fonts, Music), a drag-to-move HUD layout editor, and `/lyrics` commands.
- **Sources**
  - **Spotify Web API** with your own app key (Client ID; PKCE, no secret stored).
  - **System media session**: anything in your OS media controls. On Windows that's the Spotify app,
    SoundCloud or YouTube Music in a browser, Apple Music, TIDAL, foobar2000 and others. Linux uses
    `playerctl`; macOS reads Spotify and Music.
  - **Browser bridge** on `127.0.0.1:8974`, compatible with the original mod's bridge. A userscript covers
    SoundCloud, YouTube Music, YouTube, Deezer, TIDAL, Apple Music, Bandcamp, Amazon Music, Mixcloud,
    Audiomack and the Spotify web player. Any app can post to it (and send lyrics); `GET /now` serves the
    current line as JSON for OBS overlays.

## Install and build (in the launcher)

1. Copy `lyricsplus-3.0.0.hnmod` into your instance's `horizon-native-mods` folder, and turn it on in the
   Mods page (or `mod.lyricsplus=true` in `horizon-mods.txt`).
2. In the launcher's Mods page, press **Build** and confirm "I trust these mods". The first native build
   downloads Rust and takes 10–30 minutes; later builds take a few minutes.
3. Play the **Native mods** instance. Press **K** in game.

The bundle pins Horizon Nightly 2026-11-05 (`f319701`) and horizon-mixins `e8cd0aa` (the launcher's own
revision for that Nightly). The 3D lyrics hook into `native_world_submit`, which isn't in the named-hook
catalog yet, so the mod pins a commit instead of declaring a hooks API.

## Spotify

No setup is needed for most players: the Spotify desktop app (and SoundCloud/YouTube Music in a browser)
shows in Windows' media controls, which the mod reads directly. Linking the Web API is optional and gives
exact timing and album art from Spotify.

To ship the mod with a built-in key, set `BUILT_IN_CLIENT_ID` in `mixins/lyricsplus/engine/src/spotify.rs`
to your app's Client ID and repackage: everyone then just presses Connect. Spotify limits a new app to 25
users you add in its dashboard until it's granted extended quota.

### Linking your own app

1. Go to <https://developer.spotify.com/dashboard> and create an app (Web API).
2. Add the redirect URI `http://127.0.0.1:8975/callback`.
3. Copy the app's **Client ID** into the Sources tab (click the field, Ctrl+V), or run `/lyrics spotify <id>`.
4. Press **Connect** (or `/lyrics connect`) and allow access in the browser.

Spotify's public API has no lyrics. It provides the exact track, position and controls; the lyrics come from
LRCLIB. Play/pause/skip through the API need Premium; otherwise the mod falls back to the system media
controls.

## Browser bridge

With the bridge on, open `http://127.0.0.1:8974/lyricsplus.user.js` in a browser that has Tampermonkey or
Violentmonkey (the menu's **Install userscript** button does this). The same script is in
`extras/lyricsplus-bridge.user.js`.

`POST /update` JSON: `title, artist, album, durationMs, positionMs, isPlaying, artUrl, source, lyrics`
(LRC or plain). The answer includes `command` (`toggle`, `next`, `previous`) when a media key was pressed in
game. Other endpoints: `POST /lyrics {title, lyrics}`, `POST /clear`, `GET /now`, `GET /status`.

## Keys and commands

Keys (changeable in Options > Mods > LyricsPlus): menu **K**, show/hide **J**, play/pause **Numpad 5**,
next **Numpad 6**, previous **Numpad 4**, PvP mode **Numpad 0**.

`/lyrics` opens the menu. Subcommands: `on|off|toggle`, `mode hud|3d|both`,
`source auto|spotify|system|bridge|local`, `spotify <client id>`, `connect`, `disconnect`, `offset <ms|+ms|-ms>`,
`font <id>`, `titlefont <id>`, `reload`, `next|prev|pause`, `pvp`, `sync <+ms|-ms|reset>`, `bridge`, `status`.

Settings are stored in `horizon-mods/.horizon-mod-data/lyricsplus/config.json`.

## Layout

```text
mixins/lyricsplus/
  horizon-mixin.toml     17 HEAD hooks, the module, the engine crate
  glue.rs                game side: HUD, toast, 3D/elytra, menu, keys, commands (crate::game::lyricsplus)
  h_*.rs                 one small helper per hook
  engine/                bundle crate: sources, Spotify, bridge, offline player, LRCLIB, LRC, art palette, settings
extras/lyricsplus-bridge.user.js
tools/package.py         writes target/lyricsplus-3.0.0.hnmod
```

`cargo test --workspace` runs the engine's unit tests.

## What it does on your computer

Native mods run with your rights, so here is everything LyricsPlus does outside the game:

- **Network**: lyrics from `lrclib.net`; covers from the player's own URL, iTunes Search or Deezer; Spotify's
  Web API only when you link your own app key. Nothing else, and no data about you is sent.
- **Local ports, on 127.0.0.1 only**: 8974, the browser bridge (posts are taken only from apps, browser
  extensions and the listed music sites, never from other web pages), and 8975, Spotify's sign-in callback while
  you link.
- **Helper processes**: on Windows, PowerShell scripts it writes to its data folder read the system's media
  session and play your offline music (Windows Media Player's engine), in a hidden window; they stop when the
  game closes. On Linux `playerctl`, on macOS `osascript`; `mpv`, `ffplay` or `afplay` for offline music.
  The browser opens only when you press a button that says so.
- **Files**: only its own data folder (`horizon-mods/.horizon-mod-data/lyricsplus`: settings, caches, fonts,
  music) and the music folder you choose.

## Sync

The lyrics follow the player's position (Spotify's API, the system's media session or the bridge) and
start each line 150 ms early so its fade lands on time. The right version of a song's lyrics is picked by
its length: LyricsPlus waits up to 2 s for a player that reports the length late, and looks again when a
cached lyric belongs to a version of another length. Word timing comes from the lyrics when they have it
(enhanced LRC), otherwise it's estimated from the letters. One song early or late (a video with an intro,
a live take): General tab > Song ±250 ms, or `/lyrics sync +300`; it's remembered for that song.

## Performance

Measured with the test harness (release build, the mod's hooks only): 0.03 to 0.07 ms of CPU a frame,
depending on what's shown (the island with every effect is the most), and no image uploads after the first
second. Covers decode on their own thread; a lyric font's common letters go into the atlas in one upload when
it's chosen, and later uploads are batched to at most one every 150 ms. The Windows media helper reads every
session's title about once a second and only the playing one's position in between.

To compare in game: `horizon-client --benchmark standard --render-distance 16` with the mod on, then off
(Options > Mods), and compare the two reports in `%APPDATA%\horizon\benchmarks` (FPS, 1% and 0.1% lows).
`HORIZON_SPIKES=20` prints what made any frame slower than 20 ms.
