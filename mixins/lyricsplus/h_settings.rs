// The mod's page in Options > Mods: quick switches.
fn lyricsplus_settings(game: &mut crate::game::Game<'_>, mod_id: &str, buttons: &mut Vec<String>) {
    crate::game::lyricsplus::settings_page(game, mod_id, buttons);
}
