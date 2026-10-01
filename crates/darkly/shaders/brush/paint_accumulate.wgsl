// The paint terminal's accumulation laws, applied per dab against the live
// stroke ground by the dispatch-per-dab skeleton. `src` is the dab's
// premultiplied deposit at the pixel (shape, flow, selection and the dial's
// share already folded in); `dst` is the ground texel, premultiplied.
//
// Emitted into a brush's module by `paint::compile_wgsl` through
// `NodeWgsl::decls`, after `lib/deposit_ceiling.wgsl`, which supplies
// `ceiling_t`. Only brushes whose terminal is `paint` carry it.

// Build-up: premultiplied source-over. Coverage accumulates as
// 1 - prod(1 - a_i); a pixel's density rises with every dab that lands.
fn accumulate_build(src: vec4f, dst: vec4f) -> vec4f {
    return src + dst * (1.0 - src.a);
}

// Wash: the deposit ceiling applied per dab against the live ground. For
// one pigment this is exactly the greatest coverage any dab laid on the
// pixel (docs/brush/architecture.md, "How dabs accumulate in the
// scratch"); for varying pigments it deposits whole colours instead of
// per-channel maxima. A pixel the ceiling leaves no room on is returned
// untouched, so a dab no stronger than what is already there changes
// nothing, bit for bit.
fn accumulate_wash(src: vec4f, dst: vec4f) -> vec4f {
    let t = ceiling_t(src, dst);
    if t <= 0.0 {
        return dst;
    }
    return vec4f(src.rgb / src.a * t, t) + dst * (1.0 - t);
}
