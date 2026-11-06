// Keys in the menu: text fields and the layout editor.
fn lyricsplus_screen_key(game: &crate::game::Game<'_>, key: &str, modifiers: u32, canceled: &mut bool) {
    crate::game::lyricsplus::screen_key(game, key, modifiers, canceled);
}
