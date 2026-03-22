use std::path::PathBuf;

use crate::postfx::types::MouseEffectConfig;

#[derive(Debug, Clone)]
pub struct PostFxSetup {
    pub input: PathBuf,
    pub output: PathBuf,
    pub mouse_track: PathBuf,
    pub mouse: MouseEffectConfig,
}
