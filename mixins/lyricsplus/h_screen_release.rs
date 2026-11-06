// A mouse button up in the menu: a slider or a dragged HUD element let go.
fn lyricsplus_screen_release(
    game: &crate::game::Game<'_>,
    screen: &horizon_client::mod_api::NativeScreenView<'_>,
    canceled: &mut bool,
) {
    crate::game::lyricsplus::screen_release(game, screen, canceled);
}
