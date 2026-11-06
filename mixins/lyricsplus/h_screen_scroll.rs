// The wheel in the menu scrolls its rows.
fn lyricsplus_screen_scroll(game: &crate::game::Game<'_>, scroll: [f64; 2], canceled: &mut bool) {
    crate::game::lyricsplus::screen_scroll(game, scroll, canceled);
}
