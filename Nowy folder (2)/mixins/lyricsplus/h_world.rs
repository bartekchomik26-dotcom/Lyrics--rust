// Lyrics in the world (3D and elytra mode), after the text displays.
fn lyricsplus_world(
    value: (),
    game: &mut crate::game::Game<'_>,
    camera: glam::DVec3,
    normal: &mut Vec<horizon_client::mesher::Vertex>,
) {
    let () = value;
    crate::game::lyricsplus::world(&*game, camera, normal);
}
