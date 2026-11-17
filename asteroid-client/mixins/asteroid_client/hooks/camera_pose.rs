fn asteroid_camera_pose(game: &crate::game::Game<'_>, position: &mut [f64; 3], yaw: &mut f32, pitch: &mut f32) {
    crate::game::asteroid_movement::camera_pose(game, position, yaw, pitch);
}
