use std::fs::File;
use std::io::Write;

#[derive(Debug, thiserror::Error)]
pub(crate) enum DebugEglError {
    #[error("EGL error: {0}")]
    Egl(#[source] khronos_egl::Error),
    #[error("Unknown error")]
    Unknown,
}

fn save_ppm_rgb(path: &str, w: i32, h: i32, rgba: &[u8]) -> Result<(), DebugEglError> {
    let mut f = File::create(path).map_err(|_| DebugEglError::Unknown)?;
    // P6 PPM: RGB only
    write!(f, "P6\n{} {}\n255\n", w, h).map_err(|_| DebugEglError::Unknown)?;
    let mut row = vec![0u8; (w * 3) as usize];
    for y in 0..h {
        let src_y = y;
        let base = (src_y * w * 4) as usize;
        for x in 0..w as usize {
            row[x * 3] = rgba[base + x * 4];
            row[x * 3 + 1] = rgba[base + x * 4 + 1];
            row[x * 3 + 2] = rgba[base + x * 4 + 2];
        }
        f.write_all(&row).map_err(|_| DebugEglError::Unknown)?;
    }
    Ok(())
}

pub(crate) fn debug_dump_texture_ppm(
    egl: &khronos_egl::Instance<khronos_egl::Static>,
    src_tex: u32,
    w: i32,
    h: i32,
    out_path: &str,
) -> Result<(), DebugEglError> {
    use glow::HasContext;

    let gl = unsafe {
        glow::Context::from_loader_function(|s| {
            egl.get_proc_address(s)
                .map(|p| p as *const _)
                .unwrap_or(std::ptr::null())
        })
    };

    let vs = r#"#version 300 es
        precision mediump float;
        layout(location=0) in vec2 a_pos;
        layout(location=1) in vec2 a_uv;
        out vec2 v_uv;
        void main() {
            v_uv = a_uv;
            gl_Position = vec4(a_pos, 0.0, 1.0);
        }"#;

    let fs = r#"#version 300 es
        precision mediump float;
        in vec2 v_uv;
        uniform sampler2D u_tex;
        out vec4 o_color;
        void main() {
            o_color = texture(u_tex, v_uv);
        }"#;

    unsafe {
        let program = gl.create_program().map_err(|_| DebugEglError::Unknown)?;
        let vsh = gl
            .create_shader(glow::VERTEX_SHADER)
            .map_err(|_| DebugEglError::Unknown)?;
        gl.shader_source(vsh, vs);
        gl.compile_shader(vsh);
        if !gl.get_shader_compile_status(vsh) {
            log::error!("VS compile: {}", gl.get_shader_info_log(vsh));
            return Err(DebugEglError::Unknown);
        }

        let fsh = gl
            .create_shader(glow::FRAGMENT_SHADER)
            .map_err(|_| DebugEglError::Unknown)?;
        gl.shader_source(fsh, fs);
        gl.compile_shader(fsh);
        if !gl.get_shader_compile_status(fsh) {
            log::error!("FS compile: {}", gl.get_shader_info_log(fsh));
            return Err(DebugEglError::Unknown);
        }

        gl.attach_shader(program, vsh);
        gl.attach_shader(program, fsh);
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            log::error!("Program link: {}", gl.get_program_info_log(program));
            return Err(DebugEglError::Unknown);
        }
        gl.delete_shader(vsh);
        gl.delete_shader(fsh);

        // Fullscreen quad (2 triangles): pos.xy, uv.xy
        let verts: [f32; 24] = [
            -1.0, -1.0, 0.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, -1.0, -1.0, 0.0, 0.0,
            1.0, 1.0, 1.0, 1.0, -1.0, 1.0, 0.0, 1.0,
        ];

        let vao = gl.create_vertex_array().map_err(|_| DebugEglError::Unknown)?;
        let vbo = gl.create_buffer().map_err(|_| DebugEglError::Unknown)?;
        gl.bind_vertex_array(Some(vao));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        let bytes =
            std::slice::from_raw_parts(verts.as_ptr() as *const u8, std::mem::size_of_val(&verts));
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STATIC_DRAW);

        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 16, 0);
        gl.enable_vertex_attrib_array(1);
        gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, 16, 8);

        // Render target texture + FBO
        let out_tex = gl.create_texture().map_err(|_| DebugEglError::Unknown)?;
        gl.bind_texture(glow::TEXTURE_2D, Some(out_tex));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            w,
            h,
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

        let fbo = gl.create_framebuffer().map_err(|_| DebugEglError::Unknown)?;
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::TEXTURE_2D,
            Some(out_tex),
            0,
        );
        if gl.check_framebuffer_status(glow::FRAMEBUFFER) != glow::FRAMEBUFFER_COMPLETE {
            return Err(DebugEglError::Unknown);
        }

        gl.viewport(0, 0, w, h);
        gl.clear_color(1.0, 0.0, 1.0, 1.0); // magenta background for sanity
        gl.clear(glow::COLOR_BUFFER_BIT);

        gl.use_program(Some(program));
        let u = gl.get_uniform_location(program, "u_tex");
        gl.uniform_1_i32(u.as_ref(), 0);

        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(std::mem::transmute(src_tex)));
        gl.bind_vertex_array(Some(vao));
        gl.draw_arrays(glow::TRIANGLES, 0, 6);
        gl.finish();

        let mut rgba = vec![0u8; (w * h * 4) as usize];
        gl.read_pixels(
            0,
            0,
            w,
            h,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut rgba)),
        );

        save_ppm_rgb(out_path, w, h, &rgba)?;

        gl.delete_framebuffer(fbo);
        gl.delete_texture(out_tex);
        gl.delete_buffer(vbo);
        gl.delete_vertex_array(vao);
        gl.delete_program(program);
    }

    Ok(())
}
