//! AmoxliCode - pipeline de rectángulos sólidos.
//!
//! Dibuja todos los elementos de UI planos (barra de herramientas,
//! pestañas, fondo del panel, selección de texto, cursor, etc.) en un
//! solo lote: se suben TODOS los rectángulos del cuadro a la vez a un
//! solo buffer y se dibujan con una sola llamada. Antes se hacía un
//! `set_rect` + `draw` por cada rectángulo, reutilizando el mismo buffer
//! — eso causaba parpadeos, porque la GPU podía alcanzar a sobreescribir
//! un rectángulo antes de que el anterior terminara de dibujarse.

use bytemuck::{Pod, Zeroable};
use wgpu::{Device, Queue, TextureFormat};

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct Vertex {
    position: [f32; 2],
    color: [f32; 4],
}

/// Un rectángulo a dibujar: posición y tamaño en píxeles (origen arriba a
/// la izquierda), color y opacidad.
#[derive(Clone, Copy)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub color: [f32; 3],
    pub alpha: f32,
}

pub struct SolidQuadPipeline {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    capacity: usize, // cuántos rectángulos caben en el buffer actual
    count: usize,    // cuántos hay subidos ahora mismo
}

impl SolidQuadPipeline {
    pub fn new(device: &Device, format: TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("solid-quad-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("solid_quad.wgsl").into()),
        });

        let capacity = 64;
        let vertex_buffer = Self::make_buffer(device, capacity);

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("solid-quad-pipeline-layout"),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("solid-quad-pipeline"),
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
                            format: wgpu::VertexFormat::Float32x4,
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
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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
            vertex_buffer,
            capacity,
            count: 0,
        }
    }

    fn make_buffer(device: &Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("solid-quad-vertex-buffer"),
            size: (std::mem::size_of::<Vertex>() * 6 * capacity) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Sube TODOS los rectángulos de este cuadro a la vez. Debe llamarse
    /// una sola vez por frame (agrupando todo lo que haya que dibujar),
    /// seguido de UN solo `draw()`.
    pub fn upload(&mut self, device: &Device, queue: &Queue, window_width: u32, window_height: u32, rects: &[Rect]) {
        self.count = rects.len();
        if rects.is_empty() {
            return;
        }
        if rects.len() > self.capacity {
            self.capacity = rects.len().next_power_of_two().max(64);
            self.vertex_buffer = Self::make_buffer(device, self.capacity);
        }

        let to_ndc_x = |px: f32| (px / window_width as f32) * 2.0 - 1.0;
        let to_ndc_y = |py: f32| 1.0 - (py / window_height as f32) * 2.0;

        let mut verts = Vec::with_capacity(rects.len() * 6);
        for r in rects {
            let x0 = to_ndc_x(r.x);
            let x1 = to_ndc_x(r.x + r.w);
            let y0 = to_ndc_y(r.y);
            let y1 = to_ndc_y(r.y + r.h);
            let c = [r.color[0], r.color[1], r.color[2], r.alpha];
            verts.push(Vertex { position: [x0, y0], color: c });
            verts.push(Vertex { position: [x1, y0], color: c });
            verts.push(Vertex { position: [x0, y1], color: c });
            verts.push(Vertex { position: [x1, y0], color: c });
            verts.push(Vertex { position: [x1, y1], color: c });
            verts.push(Vertex { position: [x0, y1], color: c });
        }
        queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&verts));
    }

    pub fn draw<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>) {
        if self.count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.draw(0..(self.count as u32 * 6), 0..1);
    }
}