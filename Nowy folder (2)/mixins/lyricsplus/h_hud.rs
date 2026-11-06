// The HUD: lyrics, the player widget, the now-playing toast.
fn lyricsplus_hud(
    game: &mut crate::game::Game<'_>,
    batch: &mut horizon_client::hud::HudBatch,
    corners: &mut horizon_client::extras_hud::Corners,
) {
    crate::game::lyricsplus::hud(game, batch, corners);
}
