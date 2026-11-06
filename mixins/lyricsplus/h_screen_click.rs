// A click in the menu.
fn lyricsplus_screen_click(
    game: &crate::game::Game<'_>,
    screen: &horizon_client::mod_api::NativeScreenView<'_>,
    button: i32,
    canceled: &mut bool,
) {
    crate::game::lyricsplus::screen_click(game, screen, button, canceled);
}
