// Dragging in the menu: sliders and the layout editor.
fn lyricsplus_screen_drag(
    game: &crate::game::Game<'_>,
    screen: &horizon_client::mod_api::NativeScreenView<'_>,
    canceled: &mut bool,
) {
    crate::game::lyricsplus::screen_drag(game, screen, canceled);
}
