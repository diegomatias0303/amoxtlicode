//! AmoxliCode - pipeline del degradado del selector de color (cuadro
//! saturación/brillo + barra de tono).

use bytemuck::{Pod, Zeroable};
use wgpu::{Device, Queue, TextureFormat};

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct Vertex {
    position: [f32; 2],
    uv: [f32; 2],
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct GradientUniforms {
    hue: f32,
    mode: f32, // 0.0 = cuadro SV, 1.0 = barra de tono
    _pad: [f32; 2],
}

pub struct GradientPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
}

impl GradientPipeline {
    pub fn new(device: &Device, format: TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gradient-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("gradient.wgsl").into()),
        });

        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("gradient-bind-group-layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gradient-pipeline-layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gradient-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: std::mem::size_of::<[f32; 2]>() as u64,
                            shader_location: 1,
                        },
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        Self {
            pipeline,
            bind_group_layout,
        }
    }

    pub fn create_quad(&self, device: &Device) -> GradientQuad {
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gradient-vertex-buffer"),
            size: (std::mem::size_of::<Vertex>() * 6) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gradient-uniform-buffer"),
            size: std::mem::size_of::<GradientUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gradient-bind-group"),
            layout: &self.bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        GradientQuad {
            vertex_buffer,
            uniform_buffer,
            bind_group,
        }
    }
}

pub struct GradientQuad {
    vertex_buffer: wgpu::Buffer,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl GradientQuad {
    /// Actualiza posición, tamaño (en píxeles) y parámetros del degradado
    /// (tono actual + modo: cuadro SV o barra de tono).
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &self,
        queue: &Queue,
        window_width: u32,
        window_height: u32,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        hue: f32,
        is_hue_bar: bool,
    ) {
        let to_ndc_x = |px: f32| (px / window_width as f32) * 2.0 - 1.0;
        let to_ndc_y = |py: f32| 1.0 - (py / window_height as f32) * 2.0;

        let x0 = to_ndc_x(x);
        let x1 = to_ndc_x(x + w);
        let y0 = to_ndc_y(y);
        let y1 = to_ndc_y(y + h);

        let verts = [
            Vertex { position: [x0, y0], uv: [0.0, 0.0] },
            Vertex { position: [x1, y0], uv: [1.0, 0.0] },
            Vertex { position: [x0, y1], uv: [0.0, 1.0] },
            Vertex { position: [x1, y0], uv: [1.0, 0.0] },
            Vertex { position: [x1, y1], uv: [1.0, 1.0] },
            Vertex { position: [x0, y1], uv: [0.0, 1.0] },
        ];
        queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&verts));

        let uniforms = GradientUniforms {
            hue,
            mode: if is_hue_bar { 1.0 } else { 0.0 },
            _pad: [0.0, 0.0],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
    }

    pub fn draw<'pass>(&'pass self, pipeline: &'pass GradientPipeline, pass: &mut wgpu::RenderPass<'pass>) {
        pass.set_pipeline(&pipeline.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.draw(0..6, 0..1);
    }
}
