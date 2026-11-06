// Typing into the menu's text fields.
fn lyricsplus_screen_char(game: &crate::game::Game<'_>, c: char, modifiers: u32, canceled: &mut bool) {
    crate::game::lyricsplus::screen_char(game, c, modifiers, canceled);
}
