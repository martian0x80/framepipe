use std::path::PathBuf;

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum MouseEffect {
    None,
    Sprite,
    Zoom,
    Spotlight,
}

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum BlendMode {
    Alpha,
    Add,
    Multiply,
}

#[derive(Debug, Clone)]
pub struct MouseEffectConfig {
    pub effect: MouseEffect,
    pub sprite_path: Option<PathBuf>,
    pub blend_mode: BlendMode,
    pub opacity: f32,
    pub scale: f32,
    pub hotspot_x: i32,
    pub hotspot_y: i32,
    pub smoothing_ms: u32,
    pub zoom_factor: f32,
    pub zoom_radius_px: f32,
    pub spotlight_radius_px: f32,
    pub spotlight_softness: f32,
}
