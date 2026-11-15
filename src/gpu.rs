//! Vẽ bằng wgpu (Vulkan trên Android, có GL dự phòng).

use std::{collections::HashMap, sync::Arc};

use wgpu::util::DeviceExt;
use winit::window::Window;

use crate::game::Mat4;
use crate::ui::{UiVertex, UI_MAX_VERTICES};
use crate::world::{MeshData, Vertex, OUTLINE_MAX_INDICES, OUTLINE_MAX_VERTICES};

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Màu trời, cũng là màu sương mù để chunk xa hoà vào nền.
pub const SKY: [f32; 3] = [0.30, 0.55, 0.92];

const SHADER: &str = r#"
struct Camera {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,   // xyz = vị trí mắt, w = khoảng cách bắt đầu có sương mù
    fog: vec4<f32>,   // rgb = màu sương mù, w = khoảng cách sương mù dày đặc
};
@group(0) @binding(0) var<uniform> camera: Camera;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) color: vec3<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) world: vec3<f32>,
};

@vertex
fn vs_main(v: VsIn) -> VsOut {
    var o: VsOut;
    o.clip = camera.view_proj * vec4<f32>(v.pos, 1.0);
    o.color = v.color;
    o.world = v.pos;
    return o;
}

@fragment
fn fs_main(i: VsOut) -> @location(0) vec4<f32> {
    let d = distance(i.world, camera.eye.xyz);
    let t = clamp((d - camera.eye.w) / (camera.fog.w - camera.eye.w), 0.0, 1.0);
    return vec4<f32>(mix(i.color, camera.fog.xyz, t), 1.0);
}

struct UiIn {
    @location(0) pos: vec2<f32>,
    @location(1) color: vec4<f32>,
};

struct UiOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn ui_vs(v: UiIn) -> UiOut {
    var o: UiOut;
    o.clip = vec4<f32>(v.pos, 0.0, 1.0);
    o.color = v.color;
    return o;
}

@fragment
fn ui_fs(i: UiOut) -> @location(0) vec4<f32> {
    return i.color;
}
"#;

const VERTEX_ATTRS: [wgpu::VertexAttribute; 2] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
const UI_ATTRS: [wgpu::VertexAttribute; 2] =
    wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4];

fn as_bytes<T>(s: &[T]) -> &[u8] {
    // Chỉ dùng cho kiểu #[repr(C)] gồm toàn f32/u32 (không có padding).
    unsafe { std::slice::from_raw_parts(s.as_ptr() as *const u8, std::mem::size_of_val(s)) }
}

/// Dữ liệu chung của cảnh, khớp với `struct Camera` trong shader (96 byte).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SceneUniform {
    view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    fog: [f32; 4],
}

impl SceneUniform {
    pub fn new(view_proj: Mat4, eye: [f32; 3], fog_start: f32, fog_end: f32) -> Self {
        Self {
            view_proj,
            eye: [eye[0], eye[1], eye[2], fog_start],
            fog: [SKY[0], SKY[1], SKY[2], fog_end],
        }
    }
}

struct GpuMesh {
    vb: wgpu::Buffer,
    ib: wgpu::Buffer,
    count: u32,
}

pub struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    scene_pipeline: wgpu::RenderPipeline,
    ui_pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    depth_view: wgpu::TextureView,
    chunks: HashMap<(i32, i32), GpuMesh>,
    sel_vb: wgpu::Buffer,
    sel_ib: wgpu::Buffer,
    ui_vb: wgpu::Buffer,
}

fn make_depth_view(device: &wgpu::Device, w: u32, h: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

impl Gpu {
    pub fn new(window: Arc<Window>) -> Result<Self, String> {
        let size = window.inner_size();
        let (w, h) = (size.width.max(1), size.height.max(1));

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::GL,
            ..Default::default()
        });
        let surface = instance
            .create_surface(window)
            .map_err(|e| format!("create_surface: {e}"))?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
        }))
        .ok_or("không tìm thấy GPU phù hợp")?;
        log::info!("GPU: {:?}", adapter.get_info());

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("device"),
                required_limits: adapter.limits(),
                ..Default::default()
            },
            None,
        ))
        .map_err(|e| format!("request_device: {e}"))?;

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: w,
            height: h,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&device, &config);

        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera"),
            size: std::mem::size_of::<SceneUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera_bind"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pipeline_layout"),
            bind_group_layouts: &[&bgl],
            push_constant_ranges: &[],
        });

        let scene_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scene_pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &VERTEX_ATTRS,
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        // Giao diện: vẽ sau cùng, đè lên mọi thứ, có trong suốt.
        let ui_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui_pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "ui_vs",
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<UiVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &UI_ATTRS,
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "ui_fs",
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let depth_view = make_depth_view(&device, w, h);

        let sel_vb = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("selection_vb"),
            size: (OUTLINE_MAX_VERTICES * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sel_ib = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("selection_ib"),
            size: (OUTLINE_MAX_INDICES * std::mem::size_of::<u32>()) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let ui_vb = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui_vb"),
            size: (UI_MAX_VERTICES * std::mem::size_of::<UiVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Self {
            surface,
            device,
            queue,
            config,
            scene_pipeline,
            ui_pipeline,
            uniform,
            bind_group,
            depth_view,
            chunks: HashMap::new(),
            sel_vb,
            sel_ib,
            ui_vb,
        })
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        if w == 0 || h == 0 {
            return;
        }
        self.config.width = w;
        self.config.height = h;
        self.surface.configure(&self.device, &self.config);
        self.depth_view = make_depth_view(&self.device, w, h);
    }

    pub fn aspect(&self) -> f32 {
        self.config.width as f32 / self.config.height as f32
    }

    /// Thay (hoặc bỏ, nếu `mesh` là None hoặc rỗng) mesh của một chunk.
    pub fn set_chunk(&mut self, key: (i32, i32), mesh: Option<&MeshData>) {
        match mesh {
            Some(m) if !m.indices.is_empty() => {
                let vb = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("chunk_vb"),
                    contents: as_bytes(&m.vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
                let ib = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("chunk_ib"),
                    contents: as_bytes(&m.indices),
                    usage: wgpu::BufferUsages::INDEX,
                });
                self.chunks
                    .insert(key, GpuMesh { vb, ib, count: m.indices.len() as u32 });
            }
            _ => {
                self.chunks.remove(&key);
            }
        }
    }

    pub fn render(&mut self, scene: &SceneUniform, selection: Option<&MeshData>, ui: &[UiVertex]) {
        let frame = match self.surface.get_current_texture() {
            Ok(f) => f,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            Err(e) => {
                log::warn!("get_current_texture: {e}");
                return;
            }
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());

        self.queue
            .write_buffer(&self.uniform, 0, as_bytes(std::slice::from_ref(scene)));

        let mut sel_count = 0u32;
        if let Some(m) = selection {
            if !m.indices.is_empty()
                && m.vertices.len() <= OUTLINE_MAX_VERTICES
                && m.indices.len() <= OUTLINE_MAX_INDICES
            {
                self.queue.write_buffer(&self.sel_vb, 0, as_bytes(&m.vertices));
                self.queue.write_buffer(&self.sel_ib, 0, as_bytes(&m.indices));
                sel_count = m.indices.len() as u32;
            }
        }

        let ui_count = ui.len().min(UI_MAX_VERTICES);
        if ui_count > 0 {
            self.queue.write_buffer(&self.ui_vb, 0, as_bytes(&ui[..ui_count]));
        }

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: SKY[0] as f64,
                            g: SKY[1] as f64,
                            b: SKY[2] as f64,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            pass.set_pipeline(&self.scene_pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            for m in self.chunks.values() {
                pass.set_vertex_buffer(0, m.vb.slice(..));
                pass.set_index_buffer(m.ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..m.count, 0, 0..1);
            }
            if sel_count > 0 {
                pass.set_vertex_buffer(0, self.sel_vb.slice(..));
                pass.set_index_buffer(self.sel_ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..sel_count, 0, 0..1);
            }

            if ui_count > 0 {
                pass.set_pipeline(&self.ui_pipeline);
                pass.set_vertex_buffer(0, self.ui_vb.slice(..));
                pass.draw(0..ui_count as u32, 0..1);
            }
        }
        self.queue.submit(Some(encoder.finish()));
        frame.present();
    }
}
