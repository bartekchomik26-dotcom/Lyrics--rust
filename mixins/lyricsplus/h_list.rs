// Options > Mods: the mod's row and its keys.
fn lyricsplus_list(mods: &mut Vec<horizon_client::mod_api::NativeMod>) {
    crate::game::lyricsplus::listed(mods);
}
