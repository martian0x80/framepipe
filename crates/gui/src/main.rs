use std::sync::Arc;
use std::time::Duration;
use iced::widget::shader::{self, Pipeline, Primitive, Shader as ShaderWidget};
use iced::widget::{column, container, row, text, toggler, slider};
use iced::{Alignment, Element, Length, Rectangle, Subscription};
use iced::wgpu;

use framepipe::drm_kms::types::{LiveSettings, LiveSettingsMailbox};

#[derive(Debug, Clone)]
pub enum Message {
    Tick,
    FpsChanged(u32),
    CursorSmoothToggled(bool),
    CursorSmearToggled(bool),
}

const PREVIEW_WGSL: &str = r#"
struct VSOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2)
var<uniform> aspect: vec4<f32>;
// aspect.x = tex_aspect  (frame width / frame height)
// aspect.y = view_aspect (widget width / widget height)

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VSOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>( 3.0,  1.0),
        vec2<f32>(-1.0,  1.0),
    );

    var uvs = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 2.0),
        vec2<f32>(2.0, 0.0),
        vec2<f32>(0.0, 0.0),
    );

    var out: VSOut;
    out.pos = vec4<f32>(positions[i], 0.0, 1.0);
    out.uv = uvs[i];
    return out;
}

@fragment
fn fs_main(in: VSOut) -> @location(0) vec4<f32> {
    var uv = in.uv;
    let tex_aspect = aspect.x;
    let view_aspect = aspect.y;

    if (view_aspect > tex_aspect) {
        // View is wider than the texture -> pillarbox (black bars left/right).
        // Expand the UV-x range so values outside [0,1] are discarded as black.
        let scale = tex_aspect / view_aspect;   // < 1
        let new_x = (uv.x - 0.5) / scale + 0.5;
        if (new_x < 0.0 || new_x > 1.0) {
            return vec4<f32>(0.0, 0.0, 0.0, 1.0);
        }
        uv.x = new_x;
    } else {
        // View is taller than the texture -> letterbox (black bars top/bottom).
        let scale = view_aspect / tex_aspect;   // < 1
        let new_y = (uv.y - 0.5) / scale + 0.5;
        if (new_y < 0.0 || new_y > 1.0) {
            return vec4<f32>(0.0, 0.0, 0.0, 1.0);
        }
        uv.y = new_y;
    }

    return textureSample(tex, samp, uv);
}
"#;


#[derive(Clone)]
pub struct PreviewProgram {
    latest: common::types::PreviewMailbox,
}

impl PreviewProgram {
    pub fn new(latest: common::types::PreviewMailbox) -> Self {
        Self { latest }
    }
}

pub struct PreviewState;

impl Default for PreviewState {
    fn default() -> Self {
        Self
    }
}

#[derive(Debug)]
pub struct PreviewPrimitive {
    frame: Arc<common::types::PreviewFrame>,
}

pub struct PreviewPipeline {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
    /// Uniform buffer carrying (tex_aspect, view_aspect, pad, pad).
    aspect_buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    last_t_ns: u64,
}

impl PreviewPipeline {
    fn recreate_texture(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        self.texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("preview_texture"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        self.view = self
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        self.bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("preview_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.aspect_buffer.as_entire_binding(),
                },
            ],
        });

        self.width = width.max(1);
        self.height = height.max(1);
    }
}

impl Pipeline for PreviewPipeline {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("preview_texture"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("preview_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let aspect_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("preview_aspect_buffer"),
            size: std::mem::size_of::<[f32; 4]>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("preview_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("preview_bind_group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: aspect_buffer.as_entire_binding(),
                },
            ],
        });

        let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("preview_shader"),
            source: wgpu::ShaderSource::Wgsl(PREVIEW_WGSL.into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("preview_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("preview_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader_module,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader_module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        Self {
            texture,
            view,
            sampler,
            bind_group_layout,
            bind_group,
            pipeline,
            aspect_buffer,
            width: 1,
            height: 1,
            last_t_ns: 0,
        }
    }
}

impl Primitive for PreviewPrimitive {
    type Pipeline = PreviewPipeline;

    fn prepare(
        &self,
        pipeline: &mut Self::Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: &Rectangle,
        _viewport: &shader::Viewport,
    ) {
        let width = self.frame.width.max(1);
        let height = self.frame.height.max(1);

        if pipeline.width != width || pipeline.height != height {
            pipeline.recreate_texture(device, width, height);
            pipeline.last_t_ns = 0;
        }

        // Write aspect uniforms unconditionally so window resizes are reflected
        // even when no new frame has arrived (t_ns unchanged).
        let tex_aspect = width as f32 / height as f32;
        let view_aspect = if bounds.height > 0.0 {
            bounds.width / bounds.height
        } else {
            tex_aspect
        };
        // 16 bytes alignment
        let aspect_data: [f32; 4] = [tex_aspect, view_aspect, 0.0, 0.0];
        queue.write_buffer(
            &pipeline.aspect_buffer,
            0,
            bytemuck::cast_slice(&aspect_data),
        );

        if pipeline.last_t_ns != self.frame.t_ns {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &pipeline.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                self.frame.rgba.as_ref(),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * 4),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );

            pipeline.last_t_ns = self.frame.t_ns;
        }
    }

    fn draw(
        &self,
        pipeline: &Self::Pipeline,
        render_pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        render_pass.set_pipeline(&pipeline.pipeline);
        render_pass.set_bind_group(0, &pipeline.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
        true
    }
}

impl<Message> shader::Program<Message> for PreviewProgram {
    type State = PreviewState;
    type Primitive = PreviewPrimitive;

    fn draw(
        &self,
        _state: &Self::State,
        _cursor: iced::advanced::mouse::Cursor,
        _bounds: Rectangle,
    ) -> Self::Primitive {
        let frame = self
            .latest
            .get_frame()
            .expect("preview frame unavailable");

        PreviewPrimitive { frame }
    }
}

pub struct App {
    _preview_session: Option<framepipe::embedded_preview::EmbeddedPreviewSession>,
    preview_program: Option<PreviewProgram>,
    live_settings: Option<LiveSettingsMailbox>,
    /// GUI-side mirror of the current live settings.  Mutated by update()
    /// and published to the recording loop via live_settings.update().
    current_live: LiveSettings,
    status: String,
}

impl App {
    pub fn new() -> Self {
        let _ = env_logger::try_init();

        match framepipe::embedded_preview::start_embedded_preview_default() {
            Ok(session) => {
                let preview_mailbox = session.mailbox();
                let live_settings = session.live_settings();
                let current_live = live_settings.get().as_ref().clone();
                Self {
                    _preview_session: Some(session),
                    preview_program: Some(PreviewProgram::new(preview_mailbox)),
                    live_settings: Some(live_settings),
                    current_live,
                    status: "Embedded preview started".to_string(),
                }
            }
            Err(e) => Self {
                _preview_session: None,
                preview_program: None,
                live_settings: None,
                current_live: LiveSettings::default(),
                status: format!("Failed to start embedded preview: {e}"),
            },
        }
    }

    fn push_live_settings(&self) {
        if let Some(mb) = &self.live_settings {
            mb.update(self.current_live.clone());
        }
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::Tick => {
                // Nothing to do on tick — the shader program reads the mailbox
                // directly via PreviewMailbox.
            }
            Message::FpsChanged(fps) => {
                self.current_live.fps = fps.clamp(1, 240);
                self.push_live_settings();
            }
            Message::CursorSmoothToggled(enabled) => {
                self.current_live.cursor_smooth = enabled;
                self.push_live_settings();
            }
            Message::CursorSmearToggled(enabled) => {
                self.current_live.cursor_smear = enabled;
                self.push_live_settings();
            }
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        iced::time::every(Duration::from_millis(16)).map(|_| Message::Tick)
    }

    fn view(&self) -> Element<'_, Message> {
        let preview: Element<'_, Message> = if let Some(program) = &self.preview_program {
            ShaderWidget::new(program.clone())
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            container(text("No preview available"))
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        };

        let fps_label = text(format!("FPS: {}", self.current_live.fps));
        let fps_slider = slider(1..=240, self.current_live.fps, Message::FpsChanged)
            .width(Length::Fixed(200.0));

        let smooth_toggle = toggler(self.current_live.cursor_smooth)
            .label("Cursor Smooth")
            .on_toggle(Message::CursorSmoothToggled);

        let smear_toggle = toggler(self.current_live.cursor_smear)
            .label("Cursor Smear")
            .on_toggle(Message::CursorSmearToggled);

        let settings_row = row![
            fps_label,
            fps_slider,
            smooth_toggle,
            smear_toggle,
        ]
        .spacing(16)
        .align_y(Alignment::Center);

        container(
            column![
                text(&self.status),
                settings_row,
                preview,
            ]
            .align_x(Alignment::Start)
            .spacing(8),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }
}

fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title("Framepipe GUI")
        .subscription(App::subscription)
        .run()
}