// The settings menu, drawn over the world while it is open.
fn lyricsplus_screen_draw(
    game: &crate::game::Game<'_>,
    batch: &mut horizon_client::hud::HudBatch,
    screen: &horizon_client::mod_api::NativeScreenView<'_>,
) {
    crate::game::lyricsplus::screen_draw(game, batch, screen);
}
