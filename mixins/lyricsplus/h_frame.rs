// Once a frame: queued actions, chat lines, the album art upload.
fn lyricsplus_frame(game: &mut crate::game::Game<'_>) {
    crate::game::lyricsplus::frame(game);
}
