// The stroke commit's law over two premultiplied, opacity-scaled
// foregrounds and a straight-alpha background. Called by the commit
// (`brush/composite.wgsl`, one fragment per layer pixel) and by the
// appearance snapshot (`brush/appearance_snapshot.wgsl`, one thread per
// pixel of a dab's read region), so the stroke a sampler reads mid-stroke
// is the stroke the commit will show.
//
// Two slots, each with a fixed law, where an opacity of zero marks an
// absent slot:
//   wash:  committed through the per-pigment deposit ceiling
//   build: composited on top with plain Porter-Duff source-over
//
// Requires `source_over.wgsl` and `lib/deposit_ceiling.wgsl`.

// The deposit ceiling: lay `fg` (premultiplied) onto `bg` (straight alpha),
// depositing only what the pixel can still take; the room is `ceiling_t`'s.
// A saturated pixel (`t == 0`) is left exactly as it is, where a
// `source_over` at zero alpha would re-derive it through a division.
fn deposit_through_ceiling(fg: vec4f, bg: vec4f) -> vec4f {
    let t = ceiling_t(fg, vec4f(bg.rgb * bg.a, bg.a));
    if t <= 0.0 {
        return bg;
    }
    return source_over(fg.rgb / fg.a * t, t, bg);
}

fn commit_law(wash: vec4f, build: vec4f, bg: vec4f, blend_mode: u32,
              wash_opacity: f32, build_opacity: f32) -> vec4f {
    if blend_mode == 1u {
        // Erase: each slot removes its own coverage, composing to a removal
        // of `1 - (1 - wash.a) * (1 - build.a)`. Removal never goes through
        // the ceiling, so an eraser can always reach zero. No gate needed:
        // `destination_out(0, x)` is exactly `x`.
        return destination_out(build.a, destination_out(wash.a, bg));
    }

    // The wash slot first: the ceiling reads the ground to find room, and a
    // build slot laid under it would let a stroke's own build-up shrink its
    // own wash. An absent slot is skipped rather than composited at zero
    // alpha, because `source_over` at zero alpha is not an exact identity
    // (it divides `bg.a * bg.rgb` by `bg.a`, and zeroes rgb under 0.001).
    var out = bg;
    if wash_opacity > 0.0 {
        out = deposit_through_ceiling(wash, out);
    }
    if build_opacity > 0.0 {
        out = source_over(build.rgb, build.a, out);
    }
    return out;
}
