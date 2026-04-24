use std::sync::Arc;

use iced::Rectangle;
use iced::wgpu;
use iced::widget::shader::{self, Pipeline, Primitive};

const PREVIEW_WGSL: &str = r#"
struct VSOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> aspect: vec4<f32>;

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
        let scale = tex_aspect / view_aspect;
        let new_x = (uv.x - 0.5) / scale + 0.5;
        if (new_x < 0.0 || new_x > 1.0) { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }
        uv.x = new_x;
    } else {
        let scale = view_aspect / tex_aspect;
        let new_y = (uv.y - 0.5) / scale + 0.5;
        if (new_y < 0.0 || new_y > 1.0) { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }
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
    frame: Option<Arc<common::types::PreviewFrame>>,
}

pub struct PreviewPipeline {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
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
        let Some(frame) = &self.frame else {
            return;
        };

        let width = frame.width.max(1);
        let height = frame.height.max(1);

        if pipeline.width != width || pipeline.height != height {
            pipeline.recreate_texture(device, width, height);
            pipeline.last_t_ns = 0;
        }

        let tex_aspect = width as f32 / height as f32;
        let view_aspect = if bounds.height > 0.0 {
            bounds.width / bounds.height
        } else {
            tex_aspect
        };

        let aspect_data: [f32; 4] = [tex_aspect, view_aspect, 0.0, 0.0];
        queue.write_buffer(
            &pipeline.aspect_buffer,
            0,
            bytemuck::cast_slice(&aspect_data),
        );

        if pipeline.last_t_ns != frame.t_ns {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &pipeline.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                frame.rgba.as_ref(),
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
            pipeline.last_t_ns = frame.t_ns;
        }
    }

    fn draw(&self, pipeline: &Self::Pipeline, render_pass: &mut wgpu::RenderPass<'_>) -> bool {
        if self.frame.is_none() {
            return false;
        }

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
        PreviewPrimitive {
            frame: self.latest.get_frame(),
        }
    }
}
