// Paint terminal, dispatch-per-dab spike: one compute dispatch per dab
// against a stroke-resident read-write `r32uint` ground holding packed
// premultiplied RGBA8, plus the unpack that turns the ground into the
// RGBA8 stroke scratch at commit.
//
// A measurement vehicle for attempt #5 of
// docs/paint-compute-perf-tracking.md (see
// docs/plans/compute-dispatch-per-dab-spike.md). The disc is written by
// hand rather than compiled from the brush graph: what the bench measures
// is the cost of the dispatch, and the Ink Pen's tip is a plain soft disc.

struct FlushUniforms {
    // Plane-space origin of the paint target, so a canvas pixel maps to a
    // layer-local texel.
    layer_offset: vec2<i32>,
    layer_size: vec2<u32>,
    // The disc's feather band as a fraction of the radius.
    softness: f32,
    pad0: f32,
    pad1: f32,
    pad2: f32,
}

// One dab, 48 bytes, tight; the CPU uploads the array once per flush.
struct SpikeDab {
    pos: vec2<f32>,
    radius: f32,
    flow: f32,
    // Straight alpha; premultiplied below, as `stamp` does.
    color: vec4<f32>,
    // The dab's footprint clamped to the layer, in canvas pixels: the
    // dispatch grid covers exactly this, as the rasterizer clips the
    // fragment path's quad to the viewport for free.
    origin: vec2<i32>,
    size: vec2<u32>,
}

// The static index buffer: slot `i` holds `i`, and a dynamic offset on
// the bind group picks the slot for this dispatch.
struct Slot {
    i: u32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
}

@group(0) @binding(0) var<uniform> u: FlushUniforms;
@group(0) @binding(1) var<storage, read> dabs: array<SpikeDab>;
// The ground bound for the unpack pass only.
@group(0) @binding(2) var ground_u: texture_2d<u32>;
@group(1) @binding(0) var<uniform> slot: Slot;
@group(2) @binding(0) var ground: texture_storage_2d<r32uint, read_write>;

// The `circle` node's coverage for a plain disc: a smoothstep over the
// feather band inside the edge, with the same floor on the band.
fn disc_coverage(local_dist: f32, softness: f32) -> f32 {
    return smoothstep(0.0, max(softness, 0.004), 1.0 - local_dist);
}

@compute @workgroup_size(8, 8, 1)
fn cs_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dab = dabs[slot.i];
    // The dispatch grid starts at the clamped footprint's corner; each
    // thread owns one canvas pixel of it, and the grid's rounding past the
    // footprint returns here.
    if (any(gid.xy >= dab.size)) {
        return;
    }
    let canvas_px = dab.origin + vec2<i32>(gid.xy);
    let layer_px = canvas_px - u.layer_offset;
    if (any(layer_px < vec2<i32>(0)) || any(layer_px >= vec2<i32>(u.layer_size))) {
        return;
    }
    // Pixel-centre convention: the fragment path evaluates at (i + 0.5,
    // j + 0.5), so does this.
    let centre = vec2<f32>(canvas_px) + vec2<f32>(0.5);
    let local_dist = length(centre - dab.pos) / dab.radius;
    if (local_dist >= 1.0) {
        return;
    }
    let coverage = disc_coverage(local_dist, u.softness);
    let a = dab.color.a * coverage * dab.flow;
    let src = vec4<f32>(dab.color.rgb * a, a);
    let dst = unpack4x8unorm(textureLoad(ground, layer_px).r);
    // Premultiplied source-over: the law `paint` runs at build-up 100%.
    let out = src + dst * (1.0 - src.a);
    textureStore(ground, layer_px, vec4<u32>(pack4x8unorm(out), 0u, 0u, 0u));
}

// Unpack: a fullscreen triangle that rewrites the RGBA8 scratch from the
// ground, one texel each, no blending.
struct UnpackOut {
    @builtin(position) position: vec4<f32>,
}

@vertex
fn vs_unpack(@builtin(vertex_index) vi: u32) -> UnpackOut {
    let corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0),
    );
    var out: UnpackOut;
    out.position = vec4<f32>(corners[vi], 0.0, 1.0);
    return out;
}

@fragment
fn fs_unpack(in: UnpackOut) -> @location(0) vec4<f32> {
    let px = vec2<i32>(in.position.xy);
    return unpack4x8unorm(textureLoad(ground_u, px, 0).r);
}
