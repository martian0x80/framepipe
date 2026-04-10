use glow::HasContext;

const MAX_CURSOR_TAPS: usize = 8;

#[derive(Clone)]
pub struct CursorState {
    pub tex: Option<glow::NativeTexture>,
    pub w: f32,
    pub h: f32,
    pub taps: Vec<[f32; 3]>, // x, y, alpha in output pixel space
    pub dir_x: f32,
    pub dir_y: f32,
    pub stretch: f32,
    pub squash: f32,
}

impl CursorState {
    pub fn empty() -> Self {
        Self {
            tex: None,
            w: 0.0,
            h: 0.0,
            taps: Vec::new(),
            dir_x: 1.0,
            dir_y: 0.0,
            stretch: 1.0,
            squash: 1.0,
        }
    }

    pub fn with_position(tex: glow::NativeTexture, x: f32, y: f32, w: f32, h: f32) -> Self {
        Self {
            tex: Some(tex),
            w,
            h,
            taps: vec![[x, y, 1.0]],
            dir_x: 1.0,
            dir_y: 0.0,
            stretch: 1.0,
            squash: 1.0,
        }
    }

    pub fn with_blur_samples(
        tex: glow::NativeTexture,
        w: f32,
        h: f32,
        mut taps: Vec<[f32; 3]>,
        dir_x: f32,
        dir_y: f32,
        stretch: f32,
        squash: f32,
    ) -> Self {
        if taps.len() > MAX_CURSOR_TAPS {
            taps.truncate(MAX_CURSOR_TAPS);
        }
        Self {
            tex: Some(tex),
            w,
            h,
            taps,
            dir_x,
            dir_y,
            stretch,
            squash,
        }
    }
}

// todo: update the default texture
pub fn create_default_cursor_texture(gl: &glow::Context) -> Result<glow::NativeTexture, String> {
    let size = 24i32;
    let mut pixels: Vec<u8> = vec![0u8; (size * size * 4) as usize];

    for y in 0..size {
        for x in 0..size {
            let idx = ((y * size + x) * 4) as usize;
            let dx = x as f32 - 2.0;
            let dy = y as f32 - 2.0;
            let dist = (dx * dx + dy * dy).sqrt();

            if x < 4 && y >= x && y < size - x {
                let alpha = if y < 12 { 255 } else { 180 };
                pixels[idx] = 255;
                pixels[idx + 1] = 255;
                pixels[idx + 2] = 255;
                pixels[idx + 3] = alpha;
            } else if x >= 4 && y >= x - 4 && y < size - x + 4 && x < size - 8 {
                let alpha = if y < x + 8 { 255 } else { 180 };
                pixels[idx] = 0;
                pixels[idx + 1] = 0;
                pixels[idx + 2] = 0;
                pixels[idx + 3] = alpha;
            } else if dist < 4.0 {
                let alpha = ((1.0 - dist / 4.0) * 200.0) as u8;
                pixels[idx] = 255;
                pixels[idx + 1] = 255;
                pixels[idx + 2] = 255;
                pixels[idx + 3] = alpha;
            }
        }
    }

    unsafe {
        let tex = gl.create_texture().map_err(|e| e.to_string())?;
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            size,
            size,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(&pixels)),
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
        Ok(tex)
    }
}

pub struct GpuPipeline {
    pub gl: glow::Context,
    prog: glow::NativeProgram,
    prog_external: Option<glow::NativeProgram>,
    vao: glow::NativeVertexArray,
    vbo: glow::NativeBuffer,
    fbo: glow::NativeFramebuffer,
    out_tex: glow::NativeTexture,
    out_w: i32,
    out_h: i32,
}

impl GpuPipeline {
    pub unsafe fn new(
        egl: &khronos_egl::Instance<khronos_egl::Static>,
        out_w: i32,
        out_h: i32,
    ) -> Result<Self, String> {
        let gl = unsafe {
            glow::Context::from_loader_function(|s| {
                egl.get_proc_address(s)
                    .map(|p| p as *const _)
                    .unwrap_or(std::ptr::null())
            })
        };
        log::trace!("GL Version: {}", unsafe {
            gl.get_parameter_string(glow::VERSION)
        });

        let vs = r#"#version 300 es
            precision mediump float;
            layout(location=0) in vec2 a_pos;
            layout(location=1) in vec2 a_uv;
            out vec2 v_uv;
            void main() { v_uv = a_uv; gl_Position = vec4(a_pos,0,1); }"#;

        let fs = r#"#version 300 es
            precision mediump float;
            in vec2 v_uv;
            uniform sampler2D u_src;
            uniform sampler2D u_cursor;
            uniform int u_cursor_tap_count;
            uniform vec3 u_cursor_taps[8]; // x,y,alpha in output pixel space
            uniform vec2 u_cursor_size_px; // w,h
            uniform vec2 u_cursor_dir;
            uniform float u_cursor_stretch;
            uniform float u_cursor_squash;
            uniform vec2 u_out_size;
            out vec4 o;

            void main() {
                vec2 uv = v_uv;
                vec4 base = texture(u_src, uv);
                vec2 p = uv * u_out_size; // output pixel space

                for (int i = 0; i < 8; i++) {
                    if (i >= u_cursor_tap_count) {
                        break;
                    }
                    vec2 cmin = u_cursor_taps[i].xy;
                    vec2 cmax = cmin + u_cursor_size_px;
                    if (p.x >= cmin.x && p.y >= cmin.y && p.x < cmax.x && p.y < cmax.y) {
                        vec2 center = cmin + 0.5 * u_cursor_size_px;
                        vec2 local = p - center;
                        vec2 dir = normalize(u_cursor_dir);
                        vec2 perp = vec2(-dir.y, dir.x);
                        float a = dot(local, dir);
                        float b = dot(local, perp);
                        float stretch = max(u_cursor_stretch, 0.001);
                        float squash = max(u_cursor_squash, 0.001);
                        vec2 deformed = dir * (a / stretch) + perp * (b / squash);
                        vec2 sample_p = center + deformed;
                        vec2 cuv = (sample_p - cmin) / u_cursor_size_px;
                        vec4 c = texture(u_cursor, cuv);
                        float alpha = clamp(c.a * u_cursor_taps[i].z, 0.0, 1.0);
                        base.rgb = c.rgb * alpha + base.rgb * (1.0 - alpha);
                        base.a = 1.0;
                    }
                }
                o = base;
            }"#;
        let fs_external = r#"#version 300 es
            #extension GL_OES_EGL_image_external_essl3 : require
            precision mediump float;
            in vec2 v_uv;
            uniform samplerExternalOES u_src;
            uniform sampler2D u_cursor;
            uniform int u_cursor_tap_count;
            uniform vec3 u_cursor_taps[8]; // x,y,alpha in output pixel space
            uniform vec2 u_cursor_size_px; // w,h
            uniform vec2 u_cursor_dir;
            uniform float u_cursor_stretch;
            uniform float u_cursor_squash;
            uniform vec2 u_out_size;
            out vec4 o;

            void main() {
                vec2 uv = v_uv;
                vec4 base = texture(u_src, uv);
                vec2 p = uv * u_out_size; // output pixel space

                for (int i = 0; i < 8; i++) {
                    if (i >= u_cursor_tap_count) {
                        break;
                    }
                    vec2 cmin = u_cursor_taps[i].xy;
                    vec2 cmax = cmin + u_cursor_size_px;
                    if (p.x >= cmin.x && p.y >= cmin.y && p.x < cmax.x && p.y < cmax.y) {
                        vec2 center = cmin + 0.5 * u_cursor_size_px;
                        vec2 local = p - center;
                        vec2 dir = normalize(u_cursor_dir);
                        vec2 perp = vec2(-dir.y, dir.x);
                        float a = dot(local, dir);
                        float b = dot(local, perp);
                        float stretch = max(u_cursor_stretch, 0.001);
                        float squash = max(u_cursor_squash, 0.001);
                        vec2 deformed = dir * (a / stretch) + perp * (b / squash);
                        vec2 sample_p = center + deformed;
                        vec2 cuv = (sample_p - cmin) / u_cursor_size_px;
                        vec4 c = texture(u_cursor, cuv);
                        float alpha = clamp(c.a * u_cursor_taps[i].z, 0.0, 1.0);
                        base.rgb = c.rgb * alpha + base.rgb * (1.0 - alpha);
                        base.a = 1.0;
                    }
                }
                o = base;
            }"#;
        unsafe {
            let prog = create_program(&gl, vs, fs)?;
            let prog_external = match create_program(&gl, vs, fs_external) {
                Ok(p) => Some(p),
                Err(e) => {
                    log::trace!("External texture shader unavailable: {}", e);
                    None
                }
            };
            log::trace!("Shader program compiled and linked successfully");
            let vao = gl.create_vertex_array().map_err(|e| e.to_string())?;
            let vbo = gl.create_buffer().map_err(|e| e.to_string())?;
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            log::trace!("VAO and VBO created and bound");

            let quad: [f32; 24] = [
                -1.0, -1.0, 0.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, -1.0, -1.0, 0.0,
                0.0, 1.0, 1.0, 1.0, 1.0, -1.0, 1.0, 0.0, 1.0,
            ];
            let bytes = std::slice::from_raw_parts(
                quad.as_ptr() as *const u8,
                std::mem::size_of_val(&quad),
            );
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STATIC_DRAW);
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 16, 0);
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, 16, 8);
            log::trace!("Quad vertex data uploaded and attribute pointers set");

            let out_tex = gl.create_texture().map_err(|e| e.to_string())?;
            gl.bind_texture(glow::TEXTURE_2D, Some(out_tex));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                out_w,
                out_h,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
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
            log::trace!("Output texture created and configured");

            let fbo = gl.create_framebuffer().map_err(|e| e.to_string())?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(out_tex),
                0,
            );
            if gl.check_framebuffer_status(glow::FRAMEBUFFER) != glow::FRAMEBUFFER_COMPLETE {
                return Err("FBO incomplete".into());
            }
            log::trace!("Framebuffer created and output texture attached successfully");

            Ok(Self {
                gl,
                prog,
                prog_external,
                vao,
                vbo,
                fbo,
                out_tex,
                out_w,
                out_h,
            })
        }
    }

    // 1) render source texture -> encoder input target (RGBA here)
    // 2) cursor blend in same pass
    // 3) insert GL fence
    pub unsafe fn render_with_cursor(
        &self,
        src_tex: glow::NativeTexture,
        use_external_texture: bool,
        cursor: &CursorState,
    ) -> Result<glow::NativeFence, String> {
        unsafe {
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            log::trace!("Framebuffer bound for rendering");
            self.gl.viewport(0, 0, self.out_w, self.out_h);
            log::trace!(
                "Viewport set to output texture size: {}x{}",
                self.out_w,
                self.out_h
            );
            const GL_TEXTURE_2D: u32 = 0x0DE1;
            const GL_TEXTURE_EXTERNAL_OES: u32 = 0x8D65;
            let program = if use_external_texture {
                self.prog_external
                    .ok_or_else(|| "external texture shader not available".to_string())?
            } else {
                self.prog
            };
            self.gl.use_program(Some(program));
            log::trace!("Shader program in use for rendering");

            self.gl.active_texture(glow::TEXTURE0);
            let src_target = if use_external_texture {
                GL_TEXTURE_EXTERNAL_OES
            } else {
                GL_TEXTURE_2D
            };
            self.gl.bind_texture(src_target, Some(src_tex));
            self.gl
                .uniform_1_i32(self.gl.get_uniform_location(program, "u_src").as_ref(), 0);
            log::trace!("Source texture bound and uniform set");

            let tap_count = if cursor.tex.is_some() {
                cursor.taps.len().min(MAX_CURSOR_TAPS) as i32
            } else {
                0
            };
            self.gl.uniform_1_i32(
                self.gl
                    .get_uniform_location(program, "u_cursor_tap_count")
                    .as_ref(),
                tap_count,
            );
            self.gl.uniform_2_f32(
                self.gl
                    .get_uniform_location(program, "u_out_size")
                    .as_ref(),
                self.out_w as f32,
                self.out_h as f32,
            );
            self.gl.uniform_2_f32(
                self.gl
                    .get_uniform_location(program, "u_cursor_size_px")
                    .as_ref(),
                cursor.w,
                cursor.h,
            );
            self.gl.uniform_2_f32(
                self.gl
                    .get_uniform_location(program, "u_cursor_dir")
                    .as_ref(),
                cursor.dir_x,
                cursor.dir_y,
            );
            self.gl.uniform_1_f32(
                self.gl
                    .get_uniform_location(program, "u_cursor_stretch")
                    .as_ref(),
                cursor.stretch,
            );
            self.gl.uniform_1_f32(
                self.gl
                    .get_uniform_location(program, "u_cursor_squash")
                    .as_ref(),
                cursor.squash,
            );

            let mut taps_flat = [0.0f32; MAX_CURSOR_TAPS * 3];
            for (i, tap) in cursor.taps.iter().take(MAX_CURSOR_TAPS).enumerate() {
                let base = i * 3;
                taps_flat[base] = tap[0];
                taps_flat[base + 1] = tap[1];
                taps_flat[base + 2] = tap[2];
            }
            self.gl.uniform_3_f32_slice(
                self.gl
                    .get_uniform_location(program, "u_cursor_taps")
                    .as_ref(),
                &taps_flat,
            );

            if let Some(ctex) = cursor.tex {
                self.gl.active_texture(glow::TEXTURE1);
                self.gl.bind_texture(glow::TEXTURE_2D, Some(ctex));
                self.gl.uniform_1_i32(
                    self.gl.get_uniform_location(program, "u_cursor").as_ref(),
                    1,
                );
            }

            self.gl.bind_vertex_array(Some(self.vao));
            self.gl.draw_arrays(glow::TRIANGLES, 0, 6);

            // Fence after rendering; wait/import on encoder side.
            let fence = self
                .gl
                .fence_sync(glow::SYNC_GPU_COMMANDS_COMPLETE, 0)
                .map_err(|e| e.to_string())?;
            self.gl.flush();
            log::trace!("Rendering complete, GL fence created and flushed");

            Ok(fence)
        }
    }

    pub fn output_texture(&self) -> glow::NativeTexture {
        self.out_tex
    }
}

unsafe fn create_program(
    gl: &glow::Context,
    vs: &str,
    fs: &str,
) -> Result<glow::NativeProgram, String> {
    unsafe {
        let p = gl.create_program().map_err(|e| e.to_string())?;
        let v = gl
            .create_shader(glow::VERTEX_SHADER)
            .map_err(|e| e.to_string())?;
        gl.shader_source(v, vs);
        gl.compile_shader(v);
        if !gl.get_shader_compile_status(v) {
            return Err(gl.get_shader_info_log(v));
        }

        let f = gl
            .create_shader(glow::FRAGMENT_SHADER)
            .map_err(|e| e.to_string())?;
        gl.shader_source(f, fs);
        gl.compile_shader(f);
        if !gl.get_shader_compile_status(f) {
            return Err(gl.get_shader_info_log(f));
        }

        gl.attach_shader(p, v);
        gl.attach_shader(p, f);
        gl.link_program(p);
        if !gl.get_program_link_status(p) {
            return Err(gl.get_program_info_log(p));
        }

        gl.delete_shader(v);
        gl.delete_shader(f);
        Ok(p)
    }
}
