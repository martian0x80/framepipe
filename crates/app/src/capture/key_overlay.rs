use std::{
    collections::HashMap,
    fs,
    path::Path,
    time::{Duration, Instant},
};

use fontdue::{Font, FontSettings, Metrics};

use crate::shared::keyboard_ring::{KeyOverlayEvent, KeyboardOverlayRingBuffer};

pub const DEFAULT_DISPLAY_DURATION_MS: u64 = 1200;
pub const DEFAULT_FADE_DURATION_MS: u64 = 300;
pub const DEFAULT_DEBOUNCE_MS: u64 = 50;
pub const DEFAULT_SHOW_SINGLE_MODIFIERS: bool = false;

const FONT_SIZE: f32 = 28.0;
const TEXT_PAD_X: i32 = 20;
const TEXT_PAD_Y: i32 = 12;
const ATLAS_W: usize = 1024;
const ATLAS_H: usize = 1024;
const BITMAP_FONT_SCALE: i32 = 3;
const BITMAP_GLYPH_W: i32 = 5;
const BITMAP_GLYPH_H: i32 = 7;
const BITMAP_GLYPH_SPACING: i32 = 2;
const BITMAP_TEXT_PAD_X: i32 = 18;
const BITMAP_TEXT_PAD_Y: i32 = 12;

#[derive(Clone)]
pub struct OverlayBitmap {
    pub pixels: Vec<u8>,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Copy)]
pub struct KeyOverlayConfig {
    pub display_duration: Duration,
    pub fade_duration: Duration,
    pub debounce: Duration,
    pub show_single_modifiers: bool,
}

impl KeyOverlayConfig {
    pub fn from_millis(
        display_ms: u64,
        fade_ms: u64,
        debounce_ms: u64,
        show_single_modifiers: bool,
    ) -> Self {
        Self {
            display_duration: Duration::from_millis(display_ms.max(1)),
            fade_duration: Duration::from_millis(fade_ms.max(1)),
            debounce: Duration::from_millis(debounce_ms),
            show_single_modifiers,
        }
    }
}

impl Default for KeyOverlayConfig {
    fn default() -> Self {
        Self::from_millis(
            DEFAULT_DISPLAY_DURATION_MS,
            DEFAULT_FADE_DURATION_MS,
            DEFAULT_DEBOUNCE_MS,
            DEFAULT_SHOW_SINGLE_MODIFIERS,
        )
    }
}

pub struct KeyOverlayState {
    active: Vec<KeyOverlayItem>,
    last_seen_idx: u64,
    config: KeyOverlayConfig,
    last_label: Option<String>,
}

struct KeyOverlayItem {
    keycode: u32,
    label: &'static str,
    is_modifier: bool,
    pressed: bool,
    last_press_at: Instant,
    visible_until: Instant,
}

impl KeyOverlayState {
    pub fn new(config: KeyOverlayConfig) -> Self {
        Self {
            active: Vec::with_capacity(16),
            last_seen_idx: 0,
            config,
            last_label: None,
        }
    }

    pub fn ingest_ring(&mut self, ring: &KeyboardOverlayRingBuffer, now: Instant) {
        for event in ring.drain_since(self.last_seen_idx) {
            self.ingest(event, now);
        }
        self.last_seen_idx = ring.write_index();
    }

    pub fn ingest(&mut self, event: KeyOverlayEvent, now: Instant) {
        if event.pressed {
            if !self.active.iter().any(|item| item.pressed) {
                self.active.clear();
            }

            if let Some(item) = self.active.iter_mut().find(|i| i.keycode == event.keycode) {
                if item.pressed
                    && now.saturating_duration_since(item.last_press_at) < self.config.debounce
                {
                    return;
                }
                item.label = event.key_name;
                item.is_modifier = event.is_modifier;
                item.pressed = true;
                item.last_press_at = now;
                item.visible_until = now + self.config.display_duration;
                return;
            }

            self.active.push(KeyOverlayItem {
                keycode: event.keycode,
                label: event.key_name,
                is_modifier: event.is_modifier,
                pressed: true,
                last_press_at: now,
                visible_until: now + self.config.display_duration,
            });
        } else if let Some(item) = self.active.iter_mut().find(|i| i.keycode == event.keycode) {
            item.pressed = false;
            item.visible_until = now + self.config.display_duration;
        }
    }

    pub fn prune(&mut self, now: Instant) {
        self.active
            .retain(|item| item.pressed || item.visible_until > now);
    }

    pub fn label(&self) -> Option<String> {
        if self.active.is_empty()
            || (!self.config.show_single_modifiers
                && self.active.iter().all(|item| item.is_modifier))
        {
            return None;
        }

        Some(
            self.active
                .iter()
                .map(|item| normalize_label(item.label))
                .collect::<Vec<_>>()
                .join(" + "),
        )
    }

    pub fn alpha(&self, now: Instant) -> f32 {
        self.active
            .iter()
            .map(|item| {
                if item.pressed {
                    return 1.0;
                }

                let remaining = item.visible_until.saturating_duration_since(now);
                if remaining >= self.config.fade_duration {
                    1.0
                } else {
                    remaining.as_secs_f32() / self.config.fade_duration.as_secs_f32()
                }
            })
            .fold(0.0, f32::max)
            .clamp(0.0, 1.0)
    }

    pub fn take_dirty_label(&mut self) -> Option<Option<String>> {
        let label = self.label();
        if label == self.last_label {
            None
        } else {
            self.last_label.clone_from(&label);
            Some(label)
        }
    }
}

impl Default for KeyOverlayState {
    fn default() -> Self {
        Self::new(KeyOverlayConfig::default())
    }
}

pub struct KeyOverlayRenderer(KeyOverlayRendererInner);

enum KeyOverlayRendererInner {
    Bitmap {
        label_cache: HashMap<String, OverlayBitmap>,
    },
    Font(Box<FontOverlayRenderer>),
}

struct FontOverlayRenderer {
    font: Font,
    atlas: GlyphAtlas,
    label_cache: HashMap<String, OverlayBitmap>,
}

impl KeyOverlayRenderer {
    pub fn new(font_path: Option<&Path>) -> Result<Self, String> {
        if let Some(path) = font_path {
            let font = load_font(path)?;
            return Ok(Self(KeyOverlayRendererInner::Font(Box::new(
                FontOverlayRenderer {
                    font,
                    atlas: GlyphAtlas::new(ATLAS_W, ATLAS_H),
                    label_cache: HashMap::new(),
                },
            ))));
        }

        Ok(Self(KeyOverlayRendererInner::Bitmap {
            label_cache: HashMap::new(),
        }))
    }

    pub fn render(&mut self, label: &str) -> Result<OverlayBitmap, String> {
        match &mut self.0 {
            KeyOverlayRendererInner::Bitmap { label_cache } => {
                if let Some(bitmap) = label_cache.get(label) {
                    return Ok(bitmap.clone());
                }
                let bitmap = render_bitmap_overlay(label);
                label_cache.insert(label.to_string(), bitmap.clone());
                Ok(bitmap)
            }
            KeyOverlayRendererInner::Font(renderer) => {
                if let Some(bitmap) = renderer.label_cache.get(label) {
                    return Ok(bitmap.clone());
                }
                let bitmap = render_font_overlay(&renderer.font, &mut renderer.atlas, label)?;
                renderer
                    .label_cache
                    .insert(label.to_string(), bitmap.clone());
                Ok(bitmap)
            }
        }
    }
}

fn load_font(path: &Path) -> Result<Font, String> {
    let bytes =
        fs::read(path).map_err(|e| format!("failed to read font {}: {e}", path.display()))?;
    Font::from_bytes(bytes, FontSettings::default())
        .map_err(|e| format!("failed to parse font {}: {e}", path.display()))
}

fn render_font_overlay(
    font: &Font,
    atlas: &mut GlyphAtlas,
    label: &str,
) -> Result<OverlayBitmap, String> {
    let mut glyphs = Vec::new();
    let mut pen_x = 0.0_f32;
    let mut min_y = 0_i32;
    let mut max_y = 0_i32;

    for ch in label.chars() {
        let glyph = atlas.glyph(font, ch)?;
        min_y = min_y.min(glyph.metrics.ymin);
        max_y = max_y.max(glyph.metrics.ymin + glyph.metrics.height as i32);
        glyphs.push((pen_x, glyph));
        pen_x += glyph.metrics.advance_width.max(1.0);
    }

    let text_w = pen_x.ceil().max(1.0) as i32;
    let text_h = (max_y - min_y).max(FONT_SIZE as i32);
    let width = (text_w + TEXT_PAD_X * 2).max(1);
    let height = (text_h + TEXT_PAD_Y * 2).max(1);
    let baseline = TEXT_PAD_Y - min_y;
    let mut pixels = vec![0u8; (width * height * 4) as usize];

    draw_rounded_pill(&mut pixels, width, height);

    for (pen, glyph) in glyphs {
        let dst_x = TEXT_PAD_X + pen.round() as i32 + glyph.metrics.xmin;
        let dst_y = baseline + glyph.metrics.ymin;
        atlas.blit_glyph(&mut pixels, width, height, glyph, dst_x, dst_y);
    }

    Ok(OverlayBitmap {
        pixels,
        width,
        height,
    })
}

#[derive(Clone, Copy)]
struct CachedGlyph {
    metrics: Metrics,
    atlas_x: usize,
    atlas_y: usize,
}

struct GlyphAtlas {
    pixels: Vec<u8>,
    width: usize,
    height: usize,
    next_x: usize,
    next_y: usize,
    row_h: usize,
    glyphs: HashMap<char, CachedGlyph>,
}

impl GlyphAtlas {
    fn new(width: usize, height: usize) -> Self {
        Self {
            pixels: vec![0; width * height],
            width,
            height,
            next_x: 1,
            next_y: 1,
            row_h: 0,
            glyphs: HashMap::new(),
        }
    }

    fn glyph(&mut self, font: &Font, ch: char) -> Result<CachedGlyph, String> {
        if let Some(glyph) = self.glyphs.get(&ch) {
            return Ok(*glyph);
        }

        let (metrics, bitmap) = font.rasterize(ch, FONT_SIZE);
        let w = metrics.width.max(1);
        let h = metrics.height.max(1);
        if w + 2 > self.width || h + 2 > self.height {
            return Err(format!("glyph '{ch}' does not fit keyboard overlay atlas"));
        }

        if self.next_x + w + 1 >= self.width {
            self.next_x = 1;
            self.next_y += self.row_h + 1;
            self.row_h = 0;
        }
        if self.next_y + h + 1 >= self.height {
            return Err("keyboard overlay glyph atlas is full".to_string());
        }

        let atlas_x = self.next_x;
        let atlas_y = self.next_y;
        for row in 0..metrics.height {
            let dst = (atlas_y + row) * self.width + atlas_x;
            let src = row * metrics.width;
            self.pixels[dst..dst + metrics.width]
                .copy_from_slice(&bitmap[src..src + metrics.width]);
        }

        self.next_x += w + 1;
        self.row_h = self.row_h.max(h);

        let glyph = CachedGlyph {
            metrics,
            atlas_x,
            atlas_y,
        };
        self.glyphs.insert(ch, glyph);
        Ok(glyph)
    }

    fn blit_glyph(
        &self,
        pixels: &mut [u8],
        width: i32,
        height: i32,
        glyph: CachedGlyph,
        dst_x: i32,
        dst_y: i32,
    ) {
        for y in 0..glyph.metrics.height {
            let py = dst_y + y as i32;
            if py < 0 || py >= height {
                continue;
            }
            for x in 0..glyph.metrics.width {
                let px = dst_x + x as i32;
                if px < 0 || px >= width {
                    continue;
                }
                let alpha = self.pixels[(glyph.atlas_y + y) * self.width + glyph.atlas_x + x];
                if alpha == 0 {
                    continue;
                }
                let idx = ((py * width + px) * 4) as usize;
                alpha_blend_pixel(pixels, idx, 245, 246, 248, alpha);
            }
        }
    }
}

fn render_bitmap_overlay(label: &str) -> OverlayBitmap {
    let char_count = label.chars().count().max(1) as i32;
    let text_w = char_count * BITMAP_GLYPH_W * BITMAP_FONT_SCALE
        + (char_count - 1) * BITMAP_GLYPH_SPACING * BITMAP_FONT_SCALE;
    let text_h = BITMAP_GLYPH_H * BITMAP_FONT_SCALE;
    let width = (text_w + BITMAP_TEXT_PAD_X * 2).max(1);
    let height = (text_h + BITMAP_TEXT_PAD_Y * 2).max(1);
    let mut pixels = vec![0u8; (width * height * 4) as usize];

    draw_rounded_pill(&mut pixels, width, height);

    let mut x = BITMAP_TEXT_PAD_X;
    let y = BITMAP_TEXT_PAD_Y;
    for ch in label.chars() {
        draw_bitmap_char(&mut pixels, width, height, x, y, ch);
        x += (BITMAP_GLYPH_W + BITMAP_GLYPH_SPACING) * BITMAP_FONT_SCALE;
    }

    OverlayBitmap {
        pixels,
        width,
        height,
    }
}

fn draw_bitmap_char(pixels: &mut [u8], width: i32, height: i32, x0: i32, y0: i32, ch: char) {
    let rows = glyph_rows(ch);
    for (row, bits) in rows.iter().enumerate() {
        for col in 0..BITMAP_GLYPH_W {
            if (bits & (1 << (BITMAP_GLYPH_W - 1 - col))) == 0 {
                continue;
            }
            draw_scaled_bitmap_pixel(
                pixels,
                width,
                height,
                x0 + col * BITMAP_FONT_SCALE,
                y0 + row as i32 * BITMAP_FONT_SCALE,
            );
        }
    }
}

fn draw_scaled_bitmap_pixel(pixels: &mut [u8], width: i32, height: i32, x0: i32, y0: i32) {
    for y in y0..(y0 + BITMAP_FONT_SCALE) {
        for x in x0..(x0 + BITMAP_FONT_SCALE) {
            if x < 0 || y < 0 || x >= width || y >= height {
                continue;
            }
            let idx = ((y * width + x) * 4) as usize;
            pixels[idx] = 245;
            pixels[idx + 1] = 246;
            pixels[idx + 2] = 248;
            pixels[idx + 3] = 255;
        }
    }
}

fn glyph_rows(ch: char) -> [u8; 7] {
    match ch.to_ascii_uppercase() {
        'A' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'B' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ],
        'C' => [
            0b01111, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b01111,
        ],
        'D' => [
            0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110,
        ],
        'E' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ],
        'F' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'G' => [
            0b01111, 0b10000, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111,
        ],
        'H' => [
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'I' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111,
        ],
        'J' => [
            0b00111, 0b00010, 0b00010, 0b00010, 0b10010, 0b10010, 0b01100,
        ],
        'K' => [
            0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
        ],
        'L' => [
            0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
        ],
        'M' => [
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ],
        'N' => [
            0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001,
        ],
        'O' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'P' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'Q' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
        ],
        'R' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ],
        'S' => [
            0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        'T' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'U' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'V' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
        ],
        'W' => [
            0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b10101, 0b01010,
        ],
        'X' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
        ],
        'Y' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'Z' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
        ],
        '0' => [
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ],
        '1' => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        '2' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
        '3' => [
            0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        '4' => [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ],
        '5' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b00001, 0b00001, 0b11110,
        ],
        '6' => [
            0b01110, 0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ],
        '7' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        '8' => [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ],
        '9' => [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001, 0b01110,
        ],
        '+' => [
            0b00000, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0b00000,
        ],
        '-' => [
            0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000,
        ],
        ' ' => [0, 0, 0, 0, 0, 0, 0],
        _ => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b00000, 0b00100,
        ],
    }
}

fn normalize_label(label: &'static str) -> &'static str {
    match label {
        "LeftCtrl" | "RightCtrl" => "Ctrl",
        "LeftShift" | "RightShift" => "Shift",
        "LeftAlt" | "RightAlt" => "Alt",
        "LeftMeta" | "RightMeta" => "Meta",
        other => other,
    }
}

fn draw_rounded_pill(pixels: &mut [u8], width: i32, height: i32) {
    let radius = (height as f32 * 0.5).max(1.0);
    let left_cx = radius;
    let right_cx = width as f32 - radius;
    let cy = height as f32 * 0.5;

    for y in 0..height {
        for x in 0..width {
            let xf = x as f32 + 0.5;
            let yf = y as f32 + 0.5;
            let inside_center = xf >= left_cx && xf <= right_cx;
            let dx = if xf < left_cx {
                xf - left_cx
            } else if xf > right_cx {
                xf - right_cx
            } else {
                0.0
            };
            let dy = yf - cy;
            let inside = inside_center || (dx * dx + dy * dy) <= radius * radius;
            if inside {
                let idx = ((y * width + x) * 4) as usize;
                pixels[idx] = 18;
                pixels[idx + 1] = 20;
                pixels[idx + 2] = 24;
                pixels[idx + 3] = 215;
            }
        }
    }
}

fn alpha_blend_pixel(pixels: &mut [u8], idx: usize, r: u8, g: u8, b: u8, a: u8) {
    let src_a = a as f32 / 255.0;
    let dst_a = pixels[idx + 3] as f32 / 255.0;
    let out_a = src_a + dst_a * (1.0 - src_a);
    if out_a <= f32::EPSILON {
        return;
    }

    let blend = |src: u8, dst: u8| -> u8 {
        (((src as f32 * src_a) + (dst as f32 * dst_a * (1.0 - src_a))) / out_a)
            .round()
            .clamp(0.0, 255.0) as u8
    };

    pixels[idx] = blend(r, pixels[idx]);
    pixels[idx + 1] = blend(g, pixels[idx + 1]);
    pixels[idx + 2] = blend(b, pixels[idx + 2]);
    pixels[idx + 3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(
        keycode: u32,
        key_name: &'static str,
        pressed: bool,
        is_modifier: bool,
    ) -> KeyOverlayEvent {
        KeyOverlayEvent {
            t_ns: 0,
            keycode,
            key_name,
            is_modifier,
            pressed,
        }
    }

    #[test]
    fn new_press_after_released_chord_replaces_lingering_keys() {
        let now = Instant::now();
        let mut state = KeyOverlayState::new(KeyOverlayConfig::default());

        state.ingest(key(125, "LeftMeta", true, true), now);
        state.ingest(key(57, "Space", true, false), now);
        assert_eq!(state.label().as_deref(), Some("Meta + Space"));

        state.ingest(key(57, "Space", false, false), now);
        state.ingest(key(125, "LeftMeta", false, true), now);
        assert_eq!(state.label().as_deref(), Some("Meta + Space"));

        state.ingest(key(1, "Esc", true, false), now + Duration::from_millis(100));
        assert_eq!(state.label().as_deref(), Some("Esc"));
    }

    #[test]
    fn modifier_only_keys_are_hidden_by_default() {
        let now = Instant::now();
        let mut state = KeyOverlayState::new(KeyOverlayConfig::default());

        state.ingest(key(29, "LeftCtrl", true, true), now);
        state.ingest(key(42, "LeftShift", true, true), now);
        assert_eq!(state.label(), None);

        state.ingest(key(30, "A", true, false), now);
        assert_eq!(state.label().as_deref(), Some("Ctrl + Shift + A"));
    }

    #[test]
    fn modifier_only_keys_can_be_shown() {
        let now = Instant::now();
        let config = KeyOverlayConfig { show_single_modifiers: true, ..Default::default() };
        let mut state = KeyOverlayState::new(config);

        state.ingest(key(29, "LeftCtrl", true, true), now);
        state.ingest(key(42, "LeftShift", true, true), now);
        assert_eq!(state.label().as_deref(), Some("Ctrl + Shift"));
    }
}
