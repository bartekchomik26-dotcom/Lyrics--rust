// The mod's keys: menu, show/hide, media controls.
fn lyricsplus_key(game: &mut crate::game::Game<'_>, key: &str, pressed: bool) {
    crate::game::lyricsplus::key(game, key, pressed);
}
