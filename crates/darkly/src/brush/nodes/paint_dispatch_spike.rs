//! Paint terminal, dispatch-per-dab spike: a measurement vehicle, not a
//! brush anyone paints with.
//!
//! Attempt #5 of `docs/paint-compute-perf-tracking.md` asks whether a
//! paint terminal that issues **one compute dispatch per dab inside a
//! single compute pass**, against a stroke-resident read-write `r32uint`
//! storage texture, keeps up with the shipped instanced fragment terminal
//! on the stroke replay matrix. Stage 1 measured the dispatch in
//! isolation (`dispatch_cost_bench`); this terminal is stage 2, the same
//! shape on the real stroke path: stabiliser rewinds, checkpoints, layer
//! growth, and the bench's dab counts. The plan is
//! `docs/plans/compute-dispatch-per-dab-spike.md`.
//!
//! ## Shape
//!
//! - **The ground** is a stroke channel of [`ChannelUse::Storage`] kind,
//!   `r32uint`, holding premultiplied RGBA8 packed with `pack4x8unorm`.
//!   Declaring it as a channel is what makes the framework allocate it
//!   with the scratch, clear it at stroke start and at every rewind
//!   boundary, checkpoint and restore it, and grow it with the layer, with
//!   no spike-specific code in the engine.
//! - **Per flush**, one compute pass. Each queued dab is one
//!   `dispatch_workgroups` sized to its bounding box, one thread per
//!   pixel, reading and writing the ground under premultiplied source-over
//!   (the law `paint` runs at build-up 100%). Dispatches in a pass are
//!   ordered and their storage writes are visible to the next, so every
//!   dab sees the previous dab's output with no copy and no pass boundary.
//! - **The dab index** reaches a dispatch through a static index buffer
//!   (slot `i` holds `i`, written once at build) bound with a dynamic
//!   offset; the dab records stay a tight 32-byte array uploaded once per
//!   flush. Immediates are unavailable on the `webgpu` backend, and this
//!   is the cheapest legal mechanism there.
//! - **At commit**, an unpack pass rewrites the RGBA8 stroke scratch from
//!   the ground, and the ordinary `commit_brush_dab` lays it on the layer
//!   exactly as `paint` does. The unpack is one full-layer pass per event
//!   that a real port would fold into the commit shader; the bench
//!   measures it by A/B.
//!
//! ## What is deliberately not here
//!
//! The disc is hand-written in the compute shader from the dab's
//! position, radius and the `softness` port, not compiled from the brush
//! graph. The graph still compiles (the `rgba` port keeps the Ink Pen's
//! shape, and `compile_wgsl` returns a valid body), but this terminal
//! never builds a pipeline from that shader. Selection is not sampled and
//! erase is not offered. The hover preview is a no-op.

use std::any::Any;
use std::num::NonZeroU64;

use crate::brush::eval::{BrushNodeEvaluator, EvalContext};
use crate::brush::gpu_context::{BrushGpuContext, MAX_DABS_PER_PHASE};
use crate::brush::node::BrushNodeRegistration;
use crate::brush::paint_target_ext::BrushPaintTargetExt;
use crate::brush::pipeline::{BrushPipelineEntry, BrushPipelineRegistration, BuildContext};
use crate::brush::read_mirror_terminal::effective_radius;
use crate::brush::scratch::{ChannelUse, StrokeChannel};
use crate::brush::wgsl::{CompileWgslCtx, InputBinding, NodeWgsl};
use crate::brush::wire::{BrushWireType, ScalarValue};
use crate::nodegraph::{NodeRegistration, PortDef, UnitType};

pub const TYPE_ID: &str = "paint_dispatch_spike";
const PIPELINE_ID: &str = "paint_dispatch_spike";

/// The stroke-resident ground the dispatches read and write.
const GROUND: StrokeChannel = StrokeChannel {
    name: "ground",
    format: wgpu::TextureFormat::R32Uint,
    kind: ChannelUse::Storage,
};

/// Threads per workgroup side; the shader's `@workgroup_size`.
const WORKGROUP: u32 = 8;
/// Stride of the static index buffer: WebGPU's default
/// `min_uniform_buffer_offset_alignment`, so the native run pays what the
/// web would.
const INDEX_STRIDE: u64 = 256;

const SHADER: &str = include_str!("../../../shaders/brush/paint_dispatch_spike.wgsl");

/// Per-flush constants; matches the WGSL `FlushUniforms`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FlushUniforms {
    layer_offset: [i32; 2],
    layer_size: [u32; 2],
    softness: f32,
    _pad: [f32; 3],
}

/// One dab, 48 bytes; matches the WGSL `SpikeDab`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SpikeDab {
    pos: [f32; 2],
    radius: f32,
    flow: f32,
    color: [f32; 4],
    /// The footprint clamped to the layer, in canvas pixels, from the same
    /// ledger entry `paint` publishes; the dispatch grid covers it.
    origin: [i32; 2],
    size: [u32; 2],
}

pub fn register() -> BrushNodeRegistration {
    BrushNodeRegistration {
        pipelines: vec![BrushPipelineRegistration {
            id: PIPELINE_ID,
            build: |ctx| Box::new(SpikePipeline::build(ctx)),
        }],
        evaluator: || Box::new(SpikeEvaluator),
        lifecycle: crate::brush::node::Lifecycle::ClearScratchToTransparent,
        scratch_format: crate::brush::node::COLOR_SCRATCH_FORMAT,
        node: NodeRegistration {
            type_id: TYPE_ID,
            category: "output",
            display_name: "Paint (dispatch spike)",
            description: "Measurement spike: paint with one compute dispatch per dab. Not for painting; see docs/paint-compute-perf-tracking.md attempt #5.",
            ports: vec![
                PortDef::input("position", BrushWireType::Vec2)
                    .with_description("Canvas-pixel pen tip for this dab"),
                PortDef::input("size", BrushWireType::Scalar)
                    .with_range(0.0, 1.0, 1.0)
                    .with_natural_range(0.0, 1.0)
                    .with_label("Size")
                    .with_unit(UnitType::Percent)
                    .with_description("Per-touch size multiplier, as on paint"),
                PortDef::input("flow", BrushWireType::Scalar)
                    .with_range(0.0, 1.0, 1.0)
                    .with_natural_range(0.0, 1.0)
                    .with_label("Flow")
                    .with_unit(UnitType::Percent)
                    .with_description("Per-dab alpha scale, as paint's build flow"),
                PortDef::input("color", BrushWireType::Vec4)
                    .with_description("Brush colour (wire Paint Color)"),
                PortDef::input("softness", BrushWireType::Scalar)
                    .with_range(0.0, 1.0, 0.1)
                    .with_natural_range(0.0, 1.0)
                    .with_label("Softness")
                    .with_unit(UnitType::Percent)
                    .stroke_constant()
                    .with_description("The disc's feather band as a fraction of the radius"),
                PortDef::input("rgba", BrushWireType::Vec4).with_description(
                    "Accepted so the graph keeps paint's shape; the spike draws its own disc and never reads it",
                ),
                PortDef::output("dab_size", BrushWireType::Vec2)
                    .with_description("Brush mark size in canvas pixels"),
            ],
            is_gpu: true,
            is_terminal: true,
            supports_erase: false,
            preview_staging: None,
        },
    }
}

// ── Pipeline entry ───────────────────────────────────────────────────────

/// Built once at engine init; nothing here depends on the brush.
struct SpikePipeline {
    compute: wgpu::ComputePipeline,
    unpack: wgpu::RenderPipeline,
    /// `@group(0)`: the flush uniform and the record array.
    flush_bind_group: wgpu::BindGroup,
    /// `@group(1)`: the static index buffer, bound with a dynamic offset.
    index_bind_group: wgpu::BindGroup,
    /// `@group(2)`: the ground as a read-write storage texture. Built per
    /// flush because the channel can be reallocated on growth.
    ground_bgl: wgpu::BindGroupLayout,
    /// The unpack pass's view of the ground as a `u32` texture.
    unpack_bgl: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    records: wgpu::Buffer,
}

impl SpikePipeline {
    fn build(ctx: &BuildContext) -> Self {
        let device = ctx.device;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("paint-dispatch-spike"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let flush_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("spike-flush-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(
                            std::mem::size_of::<FlushUniforms>() as u64
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let index_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("spike-index-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: NonZeroU64::new(16),
                },
                count: None,
            }],
        });
        let ground_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("spike-ground-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::ReadWrite,
                    format: GROUND.format,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            }],
        });
        let unpack_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("spike-unpack-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Uint,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });

        let compute_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("spike-compute-layout"),
            bind_group_layouts: &[Some(&flush_bgl), Some(&index_bgl), Some(&ground_bgl)],
            immediate_size: 0,
        });
        let compute = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("spike-compute"),
            layout: Some(&compute_layout),
            module: &shader,
            entry_point: Some("cs_main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let unpack_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("spike-unpack-layout"),
            bind_group_layouts: &[Some(&unpack_bgl)],
            immediate_size: 0,
        });
        let unpack = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("spike-unpack"),
            layout: Some(&unpack_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_unpack"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_unpack"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: crate::brush::node::COLOR_SCRATCH_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("spike-flush-uniforms"),
            size: std::mem::size_of::<FlushUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let records = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("spike-dab-records"),
            size: std::mem::size_of::<SpikeDab>() as u64 * MAX_DABS_PER_PHASE as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // Slot `i` holds `i`, once, for the life of the pipeline.
        let mut index_bytes = vec![0u8; (INDEX_STRIDE * MAX_DABS_PER_PHASE as u64) as usize];
        for i in 0..MAX_DABS_PER_PHASE {
            let at = (i as u64 * INDEX_STRIDE) as usize;
            index_bytes[at..at + 4].copy_from_slice(&i.to_le_bytes());
        }
        let index = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("spike-dab-index"),
            size: index_bytes.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        ctx.queue.write_buffer(&index, 0, &index_bytes);

        let flush_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("spike-flush-bg"),
            layout: &flush_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: records.as_entire_binding(),
                },
            ],
        });
        let index_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("spike-index-bg"),
            layout: &index_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &index,
                    offset: 0,
                    size: NonZeroU64::new(16),
                }),
            }],
        });

        Self {
            compute,
            unpack,
            flush_bind_group,
            index_bind_group,
            ground_bgl,
            unpack_bgl,
            uniforms,
            records,
        }
    }
}

impl BrushPipelineEntry for SpikePipeline {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ── Evaluator ────────────────────────────────────────────────────────────

pub struct SpikeEvaluator;

impl BrushNodeEvaluator for SpikeEvaluator {
    fn evaluate_cpu(&self, _ctx: &EvalContext) -> Vec<(String, ScalarValue)> {
        vec![]
    }

    /// Queue one dab: the footprint the framework's ledger needs, then the
    /// spike's own record. The record layout is this terminal's, so it does
    /// not go through `queue_dab`, whose layout is the compiled brush's.
    fn evaluate_gpu(
        &self,
        ctx: &EvalContext,
        gpu: &mut BrushGpuContext,
    ) -> Vec<(String, ScalarValue)> {
        let Some(compiled) = gpu.dab_batch.compiled_brush.clone() else {
            debug_assert!(false, "spike requires compiled_brush on gpu_context");
            return vec![];
        };
        let Some(stroke) = gpu.stroke.as_ref() else {
            return vec![];
        };
        let position = ctx.input("position").as_vec2();
        let radius = effective_radius(ctx);
        let diameter = radius * 2.0;
        let dab_size = || vec![("dab_size".into(), ScalarValue::Vec2([diameter, diameter]))];
        if diameter <= 0.0 || gpu.dab_batch.count >= MAX_DABS_PER_PHASE {
            return dab_size();
        }
        // The same footprint `paint` publishes, so the save-point ledger and
        // the union bbox match the instanced path dab for dab.
        let bbox_radius = radius * compiled.brush_extent_factor + compiled.brush_extent_extra_px;
        let Some(footprint) =
            gpu.dab_batch
                .record_dab_footprint(&stroke.paint_target, position, bbox_radius)
        else {
            return dab_size();
        };
        let record = SpikeDab {
            pos: position,
            radius,
            flow: ctx.input_f32("flow").clamp(0.0, 1.0),
            color: ctx.input("color").as_color(),
            origin: [footprint.x0(), footprint.y0()],
            size: [footprint.width, footprint.height],
        };
        gpu.dab_batch
            .bytes
            .extend_from_slice(bytemuck::bytes_of(&record));
        gpu.dab_batch.count += 1;
        dab_size()
    }

    /// One compute pass, one dispatch per queued dab.
    fn flush_dabs(&self, ctx: &EvalContext, gpu: &mut BrushGpuContext) {
        if gpu.dab_batch.count == 0 {
            return;
        }
        let (union_w, union_h) = gpu.dab_batch.batch_extent();
        let (bytes, count) = gpu.dab_batch.take();
        if count == 0 {
            return;
        }
        gpu.perf.record_dab_flush_workload(count, union_w, union_h);
        gpu.perf.record_dab_flush(count);
        gpu.perf.record_dispatches(count);

        let pipe = gpu.pipelines.get::<SpikePipeline>(PIPELINE_ID);
        let stroke = gpu
            .stroke
            .as_ref()
            .expect("spike flush_dabs requires stroke resources");
        let canvas_ext = stroke.paint_target.canvas_extent();
        let uniforms = FlushUniforms {
            layer_offset: [canvas_ext.x0(), canvas_ext.y0()],
            layer_size: [canvas_ext.width, canvas_ext.height],
            softness: ctx.input_f32("softness").clamp(0.0, 1.0),
            _pad: [0.0; 3],
        };
        gpu.queue
            .write_buffer(&pipe.uniforms, 0, bytemuck::bytes_of(&uniforms));
        gpu.queue.write_buffer(&pipe.records, 0, &bytes);

        let ground_view = stroke
            .scratch
            .channel_view(GROUND.name)
            .expect("spike declares its ground channel");
        let ground_bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("spike-ground-bg"),
            layout: &pipe.ground_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(ground_view),
            }],
        });

        let dabs: &[SpikeDab] = bytemuck::cast_slice(&bytes);
        let mut pass = gpu
            .encoder
            .begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("spike-dispatch"),
                timestamp_writes: None,
            });
        pass.set_pipeline(&pipe.compute);
        pass.set_bind_group(0, &pipe.flush_bind_group, &[]);
        pass.set_bind_group(2, &ground_bind_group, &[]);
        for (i, dab) in dabs.iter().enumerate() {
            // The grid covers the clamped footprint; the shader rejects
            // pixels outside the disc.
            let groups = |px: u32| px.div_ceil(WORKGROUP).max(1);
            pass.set_bind_group(
                1,
                &pipe.index_bind_group,
                &[(i as u64 * INDEX_STRIDE) as u32],
            );
            pass.dispatch_workgroups(groups(dab.size[0]), groups(dab.size[1]), 1);
        }
    }

    /// Unpack the ground into the RGBA8 scratch, then commit it the way
    /// `paint` does at build-up 100%.
    fn commit(&self, _ctx: &EvalContext, gpu: &mut BrushGpuContext) {
        let Some(stroke) = gpu.stroke.as_ref() else {
            return;
        };
        let pipe = gpu.pipelines.get::<SpikePipeline>(PIPELINE_ID);
        let ground_view = stroke
            .scratch
            .channel_view(GROUND.name)
            .expect("spike declares its ground channel");
        let unpack_bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("spike-unpack-bg"),
            layout: &pipe.unpack_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(ground_view),
            }],
        });
        {
            let mut pass = gpu.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("spike-unpack"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: stroke.scratch.write_view(),
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&pipe.unpack);
            pass.set_bind_group(0, &unpack_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        stroke.paint_target.commit_brush_dab(
            &mut gpu.encoder,
            gpu.pipelines,
            gpu.queue,
            None,
            Some(stroke.scratch.write_bind_group()),
            stroke.pre_stroke_bind_group,
            1.0,
            gpu.blend_mode,
        );
    }

    /// No hover preview: the spike is driven by the bench, never by a
    /// cursor.
    fn render_cursor_preview(
        &self,
        _ctx: &EvalContext,
        _gpu: &mut BrushGpuContext,
    ) -> Vec<(String, ScalarValue)> {
        vec![]
    }

    /// A valid body so the graph compiles like any paint graph, plus the
    /// ground channel declaration. The body is never built into a pipeline
    /// by this terminal.
    fn compile_wgsl(&self, cctx: &CompileWgslCtx) -> Result<NodeWgsl, String> {
        let mut wgsl = NodeWgsl::default();
        let rgba = match cctx.inputs.get("rgba") {
            Some(InputBinding::Wired(expr)) => expr.clone(),
            _ => "vec4<f32>(0.0)".into(),
        };
        wgsl.body = format!("    return ({rgba}) * sel;\n");
        wgsl.channels = vec![GROUND];
        Ok(wgsl)
    }

    fn compile_cursor_preview_body(&self, _cctx: &CompileWgslCtx) -> Result<NodeWgsl, String> {
        Ok(NodeWgsl {
            body: "    return vec4<f32>(0.0);\n".into(),
            ..Default::default()
        })
    }
}
