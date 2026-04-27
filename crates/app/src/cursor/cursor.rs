use glow::HasContext;
use std::{
    borrow::Cow,
    path::{Path, PathBuf},
    sync::{Arc, atomic::Ordering},
    thread,
};

#[derive(Default, Clone, Copy)]
pub struct CursorSmoother {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    tx: f32,
    ty: f32,
    initialized: bool,
}

impl CursorSmoother {
    pub fn update(
        &mut self,
        target_x: f32,
        target_y: f32,
        dt: f32,
        k: f32,
        d: f32,
        max_speed: f32,
        snap_px: f32,
        smooth_ms: f32,
        deadzone_px: f32,
    ) -> (f32, f32) {
        if !self.initialized {
            self.x = target_x;
            self.y = target_y;
            self.vx = 0.0;
            self.vy = 0.0;
            self.tx = target_x;
            self.ty = target_y;
            self.initialized = true;
            return (self.x, self.y);
        }

        let (tx, ty) = if smooth_ms > 0.0 {
            let tau = (smooth_ms / 1000.0).max(0.001);
            let alpha = 1.0 - (-dt / tau).exp();
            self.tx = self.tx + (target_x - self.tx) * alpha;
            self.ty = self.ty + (target_y - self.ty) * alpha;
            (self.tx, self.ty)
        } else {
            (target_x, target_y)
        };

        let dx = tx - self.x;
        let dy = ty - self.y;
        let dist_sq = dx * dx + dy * dy;
        if deadzone_px > 0.0 && dist_sq <= deadzone_px * deadzone_px {
            // Kill micro jitter while keeping velocity under control.
            self.vx *= 0.25;
            self.vy *= 0.25;
            return (self.x, self.y);
        }
        if snap_px > 0.0 && dist_sq > snap_px * snap_px {
            self.x = tx;
            self.y = ty;
            self.vx = 0.0;
            self.vy = 0.0;
            return (self.x, self.y);
        }

        let fx = k * dx - d * self.vx;
        let fy = k * dy - d * self.vy;
        self.vx += fx * dt;
        self.vy += fy * dt;

        let speed_sq = self.vx * self.vx + self.vy * self.vy;
        if max_speed > 0.0 && speed_sq > max_speed * max_speed {
            let inv = max_speed / speed_sq.sqrt();
            self.vx *= inv;
            self.vy *= inv;
        }

        self.x += self.vx * dt;
        self.y += self.vy * dt;
        (self.x, self.y)
    }
}

pub struct MouseTrackingWorker {
    pub stop_requested: Arc<std::sync::atomic::AtomicBool>,
    pub handle: Option<thread::JoinHandle<()>>,
}

/// A cursor texture can be loaded from a file or from memory.
pub enum CursorTextureSource<'a> {
    File(Cow<'a, Path>),
    Memory(Cow<'a, [u8]>),
}

impl<'a> From<&'a PathBuf> for CursorTextureSource<'a> {
    fn from(path: &'a PathBuf) -> Self {
        CursorTextureSource::File(Cow::Borrowed(path.as_path()))
    }
}

impl<'a> From<&'a Path> for CursorTextureSource<'a> {
    fn from(path: &'a Path) -> Self {
        CursorTextureSource::File(Cow::Borrowed(path))
    }
}

impl<'a> From<&'a [u8]> for CursorTextureSource<'a> {
    fn from(data: &'a [u8]) -> Self {
        CursorTextureSource::Memory(Cow::Borrowed(data))
    }
}

/// Load a PNG or JPEG image from `path` and upload it as an RGBA8 GL texture.
/// Returns `(texture, width_f32, height_f32)` on success.
pub fn load_rgba_texture<'a>(
    gl: &glow::Context,
    source: impl Into<CursorTextureSource<'a>>,
) -> Result<(glow::NativeTexture, f32, f32), String> {
    let source = source.into();
    let img = match source {
        CursorTextureSource::File(path) => image::open(&path)
            .map_err(|e| format!("failed to load image {}: {e}", path.display()))?,
        CursorTextureSource::Memory(data) => image::load_from_memory(&data)
            .map_err(|e| format!("failed to load image from memory: {e}"))?,
    }
    .to_rgba8();
    let w = img.width() as i32;
    let h = img.height() as i32;
    if w <= 0 || h <= 0 {
        return Err(format!(
            "invalid cursor sprite dimensions for image ({}x{})",
            w, h
        ));
    }

    unsafe {
        let tex = gl.create_texture().map_err(|e| e.to_string())?;
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            w,
            h,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(img.as_raw())),
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_S,
            glow::CLAMP_TO_EDGE as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_T,
            glow::CLAMP_TO_EDGE as i32,
        );
        Ok((tex, w as f32, h as f32))
    }
}

impl Drop for MouseTrackingWorker {
    fn drop(&mut self) {
        self.stop_requested.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            log::info!("Waiting for mouse tracking thread to stop");
            match handle.join() {
                Ok(()) => log::debug!("Mouse tracking thread stopped"),
                Err(_) => log::warn!("Mouse tracking thread join failed"),
            }
        }
    }
}
