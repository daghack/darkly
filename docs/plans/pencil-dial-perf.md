# The mid-dial Pencil regression on the compute paint terminal

## Independent Review

Reviewed against `b4a1c545` on `compute-brushes` with the uncommitted bench
additions in the working tree, by reading `paint.rs::compile_wgsl`,
`paint_accumulate.wgsl`, `deposit_ceiling.wgsl`, `wgsl/mod.rs`
(`push_compute_entry`, `push_pixel_locals`, `pack_ground`), `circle.rs`,
`stamp.rs`, `levels.rs`, `wgsl/extent.rs`, `_shape.wgsl`, the three test
files, the matrix diff, the browser harness, and the port plan's open
questions. `npx tsc --noEmit` was run in `frontend/`.

### Verified

- **Empty-source early-out is exact (at `== 0`).** `stamp.rs:70-73` emits
  `a = color.a * mask; vec4(color.rgb * a, a)`, so a zero `a` is an all-zero
  `rgba` and every `src` in the body is exactly zero. `accumulate_build(0,
  dst)` is `dst` (`paint_accumulate.wgsl:12-14`), and `accumulate_wash`
  returns `dst` because `ceiling_t` guards `fg.a <= 0.0`
  (`deposit_ceiling.wgsl`). The pack/unpack round-trip identity is the one
  the refusal path already stores through, and
  `wash_stroke_does_not_darken_when_it_crosses_itself` pins it in bytes on
  real hardware. So returning before the loads produces the same texels.
- **Unchanged-texel guard is exact** by construction: one thread per texel
  per dispatch, dispatches ordered, the word compared is the word at the
  texel. Nothing to argue.
- **Uniformity.** The plan's argument is sound, and the tree already
  contains stronger evidence than it cites: `push_pixel_locals` emits the
  non-uniform bbox `return` (`wgsl/mod.rs:1250-1252`) *before* the `sel`
  `textureSampleLevel` (`:1282`), so every builtin module already has a
  divergent return ahead of an explicit-LOD sample and validates under
  naga with every flag (`tests/wgsl_validate.rs:55-56`). Say so instead of
  arguing from the position of the terminal.
- **Zero-coverage fraction.** `extent.rs:95-96`: a wired port returns its
  `natural_range` max, and `circle`'s `amplitude` is `0.0..=0.5`
  (`circle.rs:74-75`), so the Pencil (amplitude wired from `user_input`)
  compiles to `extent = 1.5` whatever the knob says; its knob is also 0.5,
  so the realised swing matches the bound. The bbox disc is 2.25x the mean
  silhouette (plus `brush_extent_extra_px`, `paint.rs:605`). The Contrast
  curve (`pencil.yaml`, points `(0.267, 0)`) zeroes more of the interior.
  The plan's "about 56%" is a lower bound on empty body threads; fine.
- **Owner.** `paint.rs` is right. The skeleton owns the texel convention
  (`pack_ground`, `wgsl/mod.rs:1129-1134`) because every dispatch-per-dab
  body shares it; "my laws are identities at zero source" is law knowledge
  the skeleton cannot assert for a terminal it has not seen. A generic
  "empty source" hook would need the terminal to declare its identity
  element, which is a second mechanism for one user. Keep the closure.
- **Divergence.** No realistic way for this to be slower: the compare is
  one integer op on a word already in a register, the return removes
  loads in spatially coherent regions (whole 8x8 groups outside the
  silhouette), and an exec-masked store dirties fewer lines. The plan's
  expectation is grounded in the handoff's measured gap but see B for what
  that gap also contains.
- **Simpler alternatives considered and rejected.** Tightening
  `circle::extent` (no: it is a worst-case bound by design, and perlin
  genuinely reaches `1 + amplitude` at some angle); one accumulation
  inside the dial (rejected in the port plan, 4.5.3); moving the guard
  into the laws (they are pure `(src, dst) -> result` functions shared with
  the commit; the store is the body's). The plan's fix is the minimal one.

### Findings

1. **(substantive) Two existing tests assert the unguarded store text
   verbatim and fail after the fix; the plan counts "65 added, 0 removed"
   and edits neither.** `tests/wgsl.rs:1926-1934` (`build_store`,
   `wash_store`, `ground_store` in
   `paint_terminal_compiles_the_accumulation_dial_to_storage_and_stores`)
   and `:2250-2254` (the `stores` helper in
   `authored_buildup_picks_the_accumulation_shape`, prefix
   `textureStore({target}, layer_px, pack_ground({law}(`). Do not add a
   third dial walk on a hand-built graph: update the three closures in the
   first test to the guarded text and add the early-return assertion
   (present, before the first `textureLoad(` in the `cs_main` slice) there;
   it already compiles a real brush at all three dial shapes and is the
   natural regression test, failing on the unfixed tree by construction.
   Change the second test's helper to match on the law line
   (`let {target}_now = pack_ground({law}(`). Tests LOC becomes about 25
   modified, not 65 added.

2. **(substantive) The gate is stated against the wrong reference and
   quotes numbers from another session.** The regression is "port mid-dial
   costs 20 to 37% more than pre-port mid-dial"; the plan's primary
   criterion is "within noise of the fixed tree's own build-only cell",
   and the pre-port figures are copied from the handoff, which the plan
   itself calls "the shape of the problem, not the band". Two problems.
   (a) Mid-dial carries work this fix does not touch: the second ground's
   checkpoint snapshot and restore (`checkpoint_ring.rs:85-89`,
   `extra_formats`), its clear (0.24 ms per the handoff), and the commit's
   second foreground. "Within 0.5 ms of build-only" may be structurally
   unreachable, and a miss would then say nothing about whether the fix
   worked. (b) The handoff's table shows port build-only (10.1 ms) already
   above pre-port mid-dial (9.8 ms) at 250 px, so the two criteria nearly
   coincide there, which is an argument for stating the real one. Revise
   the gate to: a pre-port worktree (`929928f9ab` with the same bench
   patches, as the handoff's session did) measured in the same session;
   pass is port mid-dial `event_sync_ms.p50` within the band of pre-port
   mid-dial at 250 and 500 px, native `cpu p50` likewise, and
   `long_frames_over_33ms` equal. Keep the build-only and wash-only cells
   as the diagnostic reference that attributes any residual. The
   instrument is right: `pace=sync` gives a per-event cost that can be
   differenced across cells, `pace=realtime` gives the symptom.

3. **(minor) `<= 0.0` changes the build law for negative coverage; `==
   0.0` is exact for both laws.** Nothing clamps `stamp`'s mask
   (`stamp.rs:70-73`), so a graph with a subtracting node upstream can
   produce `a < 0`; today build computes `src + dst * (1 - src.a)` there,
   after the change it deposits nothing. Unreachable for the Pencil
   (`levels` clamps, `levels.rs:92`; the curve LUT is in `[0, 1]`) and
   arguably better, but it is an untested behaviour change under a plan
   whose selling point is "no law changes". Prefer `== 0.0` (`-0.0 ==
   0.0`, so negative zero is covered); the Risks entry and open question 2
   then disappear. If `<=` is kept, drop the unqualified exactness claim
   and state the build law now agrees with the ceiling.

4. **(minor) Tooling: agree with the keep/trim/delete split, with
   adjustments.**
   - The CDP driver is not in the tree (session scratchpad), so "otherwise
     as is" cannot be reviewed. It lands in step 6 with its Chromium flags
     named in the perf doc (headless WebGPU needs explicit enabling; the
     dev server is https, so certificate handling too). Playwright is
     already a devDependency of `desktop/` (`desktop/package.json:27,31`);
     a built-ins-only script in `frontend/scripts` is lighter than adding
     it to the frontend, which is fine, but record the choice.
   - Confirmed: `npx tsc --noEmit` fails today only at
     `src/bench/pencil.ts(216,91)` (TS2339 on `texturesCreatedLog`), the
     line the trim removes; `vite.config.ts` has no `rollupOptions.input`;
     vitest's default include does not reach `src/bench`.
   - Drop `mult` (the event-rate multiplier). No gate cell uses it.
   - The port plan's open question 4 names `engine/process_recording.rs`,
     which is the timelapse recorder (`process_recording.rs:1-9`), not a
     replay; the in-engine replay is `format/stroke_recording.rs::replay`.
     The TS harness reimplements its scale and pacing (about 15 lines),
     which is acceptable only because what is measured (transport post
     cost, `onSubmittedWorkDone` latency, frame intervals) exists only on
     the JS side; the perf doc paragraph should say that is why it is a
     page and not a wasm entry, and correct the reference.
   - `replay()`'s `after_event: Option<&mut dyn FnMut()>`
     (`stroke_recording.rs:188`) and the three `None` call sites are the
     right shape; nothing to change.

5. **(minor) Matrix help and module doc.** The diff adds `--stabilize` to
   the `--help` line but not `--gpu-sync`, `--buildup`, `--only`, and the
   module doc (`stroke_replay_matrix.rs:15-20`) still describes a run
   without them. Fix alongside the `gpu_sync=` / `buildup=` header fields.

6. **(minor) LOC and docs.** Production 22/6 is plausible (closure +5/-3,
   return +1, two comments). Tests per finding 1. The 80-line perf doc
   section is generous: the before/after tables, the ruled-out list and
   which early-out bought what are the content; keep the narrative to a
   paragraph. Committing the pencil `bench-results` pair follows precedent
   (`stroke-replay-matrix-paint-compute-after-*`). Deleting
   `zz_dump_pencil.rs`, `microbench.html` and `browser-profile/` is right.
   Comments in the diff and the comments the plan proposes describe the
   code, not the plan. `paint.rs:844-850` for the `compile_wgsl` doc is
   `836-843` on this tree; not material.

**Verdict: revise.** The diagnosis and the fix are right and minimal;
findings 1 and 2 are the substantive ones (the test accounting is wrong,
and the gate should be the regression's own definition, measured in one
session).

## Revision (response to the review)

Every finding is taken; the sections below are rewritten accordingly.

1. **Tests.** No third dial walk. The regression test is the existing
   `paint_terminal_compiles_the_accumulation_dial_to_storage_and_stores`
   with its three store closures changed to the guarded text and the
   early-return assertion added; `authored_buildup_picks_the_accumulation_shape`'s
   `stores` helper matches on the law line. Both fail on the unfixed tree
   by construction. Tests LOC is about 25 modified lines.
2. **Gate.** Stated against the regression's own definition: a pre-port
   worktree of `929928f9ab` carrying the same bench patches, measured in
   the same session as the fixed tree. Pass is port mid-dial within the
   band of pre-port mid-dial at both cells (browser `event_sync_ms.p50`,
   native `cpu p50`) and an equal long-frame count at 500 px realtime.
   Build-only and wash-only stay as the diagnostic reference that
   attributes any residual.
3. **Predicate.** `== 0.0`, not `<= 0.0`: exact for both laws with no
   behaviour change for a negative mask; `-0.0 == 0.0` holds. The Risks
   entry and the open question are gone.
4. **Tooling.** `mult` is dropped. The driver lands in step 6 under
   `frontend/scripts/bench-drive.mjs` with its Chromium flags named in the
   perf doc; built-ins only, chosen over Playwright (a `desktop/`
   devDependency) because the frontend has no such dependency and the
   script is 55 lines. The perf doc paragraph says why the replay is a page
   and not a wasm entry (the measured quantities exist only on the JS
   side) and corrects the port plan's open question 4 reference
   (`format/stroke_recording.rs::replay`, not `engine/process_recording.rs`).
5. **Matrix help and module doc** are updated for all four flags, with the
   `gpu_sync=` and `buildup=` header fields.
6. **Docs.** The perf doc section is the tables, the ruled-out list and
   which early-out bought what, with a paragraph of narrative: about 40
   lines, not 80.

A performance regression introduced by the compute paint terminal
(`b4a1c545`, plan `docs/plans/compute-paint-terminal.md`, attempt #6 of
`docs/paint-compute-perf-tracking.md`), fully diagnosed before this plan was
written. This plan fixes it with two bit-exact early-outs in the WGSL that
`paint::compile_wgsl` emits, keeps the bench additions that reproduce it as
the regression gate, and records the structural options it defers.

## Summary

The Pencil (`crates/darkly/brushes/pencil.yaml`: `paint.buildup: 0.1`,
`brush_settings.spacing: 0.03`, a perlin disc with `softness: 1.0`) costs 20
to 37% more GPU time per event than it did on the instanced fragment
terminal. Inside the dial every thread of every dab loads and stores two
packed grounds (the wash ground under `accumulate_wash`, the `build` channel
under `accumulate_build`), whether or not the dab deposits anything at the
pixel and whether or not the law changes the texel. At 3% spacing most dabs
are refused by the deposit ceiling on most pixels, and for a perlin tip
more than half of the threads that reach the body sit outside the
silhouette, so most of that traffic writes back what was already there.

The fix: the terminal's body returns before either load when the dab's
coverage at the pixel is zero, and stores a ground only when the packed
texel it computed differs from the one it loaded. Both are identities at
the texel (storing an unchanged word is a no-op), so the exact accumulation
suite stays bit-identical and no law changes. Production change is about
twenty lines in `crates/darkly/src/brush/nodes/paint.rs`.

The gate is the bench cell that found the bug: the Pencil at the artist's
settings in headless Chromium, mid-dial against its own build-only cell.

## Root cause

### What was measured

Per-event cost with CPU and GPU serialized (`pace=sync` in the browser
harness), 1920x1080, `stabilize = 0.6`, the recorded curvy stroke, Intel
Raptor Lake-P iGPU, headless Chromium 153 (Dawn, Vulkan):

| cell | pre-port mid-dial | port mid-dial | port build-only | port wash-only |
|---|---:|---:|---:|---:|
| 250 px column (about 135 px dabs) | 9.8 ms | 13.4 ms | 10.1 ms | - |
| 500 px column (about 270 px dabs) | 13.6 ms | 16.4 ms | 14.0 ms | 14.2 ms |

Real-time replay at the 500 px cell: pre-port drops no frames, the port
drops 8 to 15 of 235 with a 24 to 45 ms median submit-to-completion
latency. The same tree natively (`stroke_replay_matrix --topology pencil
--stabilize 0.6 --gpu-sync --only 1920x1080:500`, `cpu p50`): port
mid-dial 14.4 ms, port build-only 11.8 ms, port wash-only 12.1 ms, pre-port
mid-dial 11.2 ms. Every native cell is under budget, which is why the
port's gate (Ink Pen, `stabilize = 1.0`, no GPU wait) did not see it.

Ruled out by measurement (handoff, "Findings, 2026-10-01"): per-dispatch
overhead in Dawn (`strokeTo` posts in 0.2 ms, `dispatches == dabs`, the Ink
Pen at 1000 px is smooth), the two `noise` nodes (both bake to tiles), the
checkpoint ring's copies (a 1800x1000 `copyTextureToTexture` is 0.2 to
0.4 ms whatever the format, and the pre-port tree issues the same 2806
copies per stroke), and the clears as such (an `r32uint` storage attachment
clears in 0.24 ms against 0.07 ms for `rgba8unorm`, two per event).

### Where the time goes

The emitted body for the Pencil ends like this (the session's dump of the
compiled module; `paint.rs:881-906` is the emitter):

```wgsl
    let rgba = stamp_stamp(..., u.npaint_color_color);
    let wash_flow = clamp(curve_curve_5(d.npen_input_pressure), 0.0, 1.0);
    let build_flow = clamp(curve_curve_5(d.npen_input_pressure), 0.0, 1.0);
    let wash_src = rgba * wash_flow * sel * 0.900000;
    let build_src = rgba * build_flow * sel * 0.100000;
    textureStore(ground, layer_px, pack_ground(accumulate_wash(wash_src, unpack_ground(textureLoad(ground, layer_px)))));
    textureStore(build, layer_px, pack_ground(accumulate_build(build_src, unpack_ground(textureLoad(build, layer_px)))));
```

Two unconditional read-modify-writes per thread. Two things make most of
them redundant for this brush:

1. **Most threads deposit nothing.** The skeleton rejects threads past the
   dab's bbox disc (`wgsl/mod.rs:1243-1250`, `local_dist_px >=
   d.bbox_target_px`), but the bbox is the worst case of the silhouette:
   `circle::extent` (`nodes/circle.rs:290-321`) multiplies the radius by
   `1 + amplitude` for a perlin disc, and the Pencil wires `amplitude =
   0.5`, so the bbox disc has 2.25x the area of the mean silhouette and
   about 56% of the threads that reach the body sit outside it, where
   `shape_coverage` (`shaders/brush/_shape.wgsl:177-185`) is exactly `0.0`
   (`smoothstep` of a negative perpendicular distance). On top of that the
   "Contrast pen tip" curve maps tip values below 0.267 to exactly `0.0`
   (`nodes/curve.rs:71-96`, a LUT with linear interpolation between two
   zero entries), and `stamp` (`nodes/stamp.rs:70-73`) emits
   `vec4(color.rgb * a, a)` with `a = color.a * mask`, so a zero mask is a
   zero `rgba`. Every one of those threads loads both grounds, runs both
   laws on `src = 0`, and stores both grounds back unchanged.
2. **Most wash stores are refusals.** At `spacing: 0.03` a pixel under the
   stroke sees a dab every 3% of the diameter of travel, tens of dabs per
   pass. `accumulate_wash` (`shaders/brush/paint_accumulate.wgsl:23-29`)
   returns `dst` untouched whenever `ceiling_t <= 0`, which for one pigment
   is every dab no stronger than what the pixel already holds (plan
   4.5.1): with a slowly varying pressure that is nearly all of them. Each
   refusal still packs and stores the loaded word.

The fragment terminal paid the same per-pixel shading but through the
blend unit, which the dispatch-cost harness measured at about half the
cost of a storage read-modify-write; and it had hardware early-out on a
zero-coverage fragment only in the sense that a blend with `src = 0` is
cheap. The compute terminal has neither for free; it has to say so in the
shader.

### Why both early-outs are exact

- **Empty source.** `accumulate_build(0, dst) = 0 + dst * (1 - 0) = dst`.
  `accumulate_wash(src, dst)` with `src.a <= 0` returns `dst` because
  `ceiling_t` (`shaders/lib/deposit_ceiling.wgsl`, the `fg.a <= 0.0` guard)
  returns `0.0`. And `pack_ground(unpack_ground(w)) == w` for every packed
  word: `unpack4x8unorm` yields `byte / 255` and `pack4x8unorm` rounds
  `255 * (byte / 255)` back to `byte`. The existing law already depends on
  that identity ("changes nothing, bit for bit" in the wash doc comment, and
  `wash_stroke_does_not_darken_when_it_crosses_itself` asserts it in
  pixels). So returning before the loads produces the same texels as
  loading, computing and storing.
- **Unchanged texel.** Storing a word equal to the one at the texel is a
  no-op by definition, whatever rounding produced it. Guarding the store on
  `now.r != was.r` cannot change any pixel; it only removes stores.
  It is a superset of "the ceiling refused" (that case returns `dst`
  exactly, so the words are equal) and also covers a build-up dab whose
  contribution rounds to the same byte.

`sel` is folded into the source (`src = rgba * flow * sel`), so a zero
selection is a zero source and the first early-out covers it. Erase is a
commit decision (`gpu.blend_mode` in `commit_brush_dab`, see the
`PerBrushPipeline::pipeline` doc comment at `paint.rs:135-144`), so the
per-dab pass is the same under erase and both early-outs apply unchanged.

One deliberate edge: the predicate is `== 0.0`, not `<= 0.0`. A source with
negative alpha cannot come from a well-formed premultiplied graph, but
nothing clamps `stamp`'s mask, and today the build law computes
`src + dst * (1 + |a|)` for one; `<= 0.0` would silently change that to a
no-op. The equality keeps both laws exactly as they are for every input
and still catches negative zero.

## Architectural impact

None to the model. The change is confined to the text `paint::compile_wgsl`
emits for the terminal's own body (`crates/darkly/src/brush/nodes/paint.rs`,
the `store` closure at `:881-885` and the body assembly at `:886-906`). The
laws in `shaders/brush/paint_accumulate.wgsl` and
`shaders/lib/deposit_ceiling.wgsl` are untouched, as are the compute
skeleton (`wgsl/mod.rs`, `push_compute_entry`), the registration, the
scratch, the ring and the commit. Document, session and compositor state
are not involved: the ground is bulk pixel data on the GPU and stays so.

The early return is the skeleton's own idiom: `push_pixel_locals` returns a
thread past the bbox before any node body runs (`wgsl/mod.rs:1243-1250`),
and the terminal's body is the last thing in `cs_main` (the terminal is the
plan's final step, `compile_brush_to_wgsl`'s doc at `wgsl/mod.rs:322-330`), so
a return there skips nothing that any node needs. Every `textureSampleLevel`
in the module (the selection mask and the baked noise tiles) is explicit-LOD
and is evaluated before the terminal's body; compute has no derivatives, no
`workgroupBarrier` is emitted anywhere, so the new non-uniform return has no
uniformity consequence. `tests/wgsl_validate.rs` runs naga with every
validation flag over every builtin's stroke module and is the check that
this stays true.

The guarded read-modify-write stays a closure inside `paint.rs` rather than
a skeleton helper: `paint` is the only `DispatchPerDab` terminal, and the
closure is already the single emitter for all three dial shapes (wash only,
build only, both). If a second compute terminal with a ground appears, the
closure is what moves into `wgsl/mod.rs` next to `pack_ground`; not before.

Prior art: not researched. This is an additive change with in-repo
precedent (the skeleton's early returns, and `deposit_through_ceiling` in
`shaders/brush/composite.wgsl:84-90`, which returns `bg` untouched on
refusal for the same reason), not an open design question.

## Design

### The emitted body, after

The `store` closure becomes a guarded read-modify-write and the body gains
one early return after `rgba`. For the Pencil the tail of `cs_main` reads:

```wgsl
    let rgba = stamp_stamp(..., u.npaint_color_color);
    if (rgba.a * sel == 0.0) {
        return;
    }
    let wash_flow = clamp(curve_curve_5(d.npen_input_pressure), 0.0, 1.0);
    let build_flow = clamp(curve_curve_5(d.npen_input_pressure), 0.0, 1.0);
    let wash_src = rgba * wash_flow * sel * 0.900000;
    let build_src = rgba * build_flow * sel * 0.100000;
    let ground_was = textureLoad(ground, layer_px);
    let ground_now = pack_ground(accumulate_wash(wash_src, unpack_ground(ground_was)));
    if (ground_now.r != ground_was.r) {
        textureStore(ground, layer_px, ground_now);
    }
    let build_was = textureLoad(build, layer_px);
    let build_now = pack_ground(accumulate_build(build_src, unpack_ground(build_was)));
    if (build_now.r != build_was.r) {
        textureStore(build, layer_px, build_now);
    }
```

The early-out tests the dab's coverage at the pixel (`rgba.a * sel`) rather
than a per-half `src.a`, so it is one predicate for all three dial shapes
and does not depend on the flows. It is an equality, not `<= 0.0`: a zero
source is an identity under both laws, while a negative mask (nothing
clamps `stamp`'s input) is a behaviour the laws define today and this plan
does not change; `-0.0 == 0.0`, so negative zero is covered; a half whose flow is zero still runs its
law and is caught by the store guard (its word is unchanged). The flows are
evaluated after the return so an empty thread skips them too.

In `compile_wgsl`:

```rust
let store = |target: &str, law: &str, src: &str| {
    format!(
        "    let {target}_was = textureLoad({target}, layer_px);\n\
         \x20   let {target}_now = pack_ground({law}({src}, unpack_ground({target}_was)));\n\
         \x20   if ({target}_now.r != {target}_was.r) {{\n\
         \x20       textureStore({target}, layer_px, {target}_now);\n\
         \x20   }}\n"
    )
};
```

and, straight after `let rgba = ...;`:

```rust
body.push_str("    if (rgba.a * sel == 0.0) {\n        return;\n    }\n");
```

The `_was` / `_now` identifiers are suffixes on the binding names the
skeleton already declares (`ground`, and each storage channel's `name`), in
the same namespace those names already occupy; node identifiers carry a
node-id suffix (`CompileWgslCtx::ident`) and dab fields an `n<id>_` prefix,
so there is no new collision class.

### What is not in this plan

Evaluated and deferred, each for a reason the implementation can revisit
with numbers:

- **One wider texel for both halves.** `Rg32Uint` is read-only or
  write-only storage in core WebGPU (`format.rs`, the `s_ro_wo` table the
  port plan cites in 4.5.3); a read-write two-channel ground needs a
  feature the web does not expose. Not available.
- **A single accumulation inside the dial.** The commit's two-slot law
  (wash through the ceiling, build over it) needs two grounds; a single
  ground is a semantics change to every mid-dial brush (port plan 4.5.3,
  both rejected variants). Its own plan if ever.
- **Two alpha-only grounds packed as `u16` pairs in one `r32uint`** for a
  stroke-constant pigment (the chroma then lives in `u.paint_color` and
  both laws reduce to alpha arithmetic). It would halve the dial's
  bandwidth and raise precision, but it is a third ground format with its
  own commit entry point and a compile-time "pigment is stroke-constant"
  fact, and it does not help per-pixel pigments. Worth a plan only if the
  gate below is still missed after this fix, because the residual would
  then be the wash half's load, which only a single texel removes.
- **Specialising `accumulate_wash` to `max(src.a, dst.a)` for a
  stroke-constant pigment** (handoff candidate). Removes a division and
  two Chebyshev distances per thread; the diagnosis says the cost is
  bandwidth, not ALU. Deferred until a measurement says otherwise.

## Implementation steps

Order per the Testing Principle: the regression test first, failing, then
the fix.

1. **Shape test, failing.** Change the two `tests/wgsl.rs` tests as under
   Tests (1). Run `cargo test -p darkly --test wgsl --features testing --
   --test-threads=1`; both must fail on the unfixed tree because the body
   has no `rgba.a * sel` return and its stores are unguarded.
2. **Baseline the gate** (same machine, same session as step 5), on the
   unfixed tree and on a pre-port worktree (`git worktree add <dir>
   929928f9ab`, the bench patches applied to it, its own `make wasm` and
   dev server on a second port), so before, reference and after come from
   one environment:
   - native: `cargo run --release -p darkly --features testing --bin
     stroke_replay_matrix -- --input
     crates/darkly/tests/fixtures/recorded_curvy_stroke.json --topology
     pencil --stabilize 0.6 --gpu-sync --only 1920x1080:250 --only
     1920x1080:500`, once per dial (`--buildup 1.0`, `--buildup 0.0`, and
     the brush's own 0.1), each three times; keep the medians of `cpu p50`.
   - browser: `make wasm` (or `npm run start`), then the driver over the
     harness for the same three dials at 250 and 500 px with `pace=sync`,
     and `pace=realtime` at 500 px for the long-frame count. Three runs
     each, medians of `event_sync_ms.p50`. The pre-port worktree runs the
     mid-dial cells only (its ends are not the reference).
3. **The fix** in `paint.rs::compile_wgsl` as under Design: the early
   return after `rgba`, the guarded `store` closure. Update the module
   header's ground paragraph (`paint.rs:17-24`, "Each thread loads its
   texel, applies the brush's accumulation law ... and stores it") and the
   `compile_wgsl` doc comment (`:836-843`) to describe the two early-outs.
4. **Correctness gates**, all unchanged in expectation:
   ```bash
   cargo test -p darkly --test wgsl --features testing -- --test-threads=1
   cargo test -p darkly --test wgsl_validate --features testing -- --test-threads=1
   cargo test -p darkly --test brush_accumulation --features testing -- --test-threads=1
   cargo test -p darkly --test paint_compute --features testing -- --test-threads=1
   ```
   `brush_accumulation` is the bit-exactness gate; any pixel moving is a
   bug in this plan's reasoning, not tolerance to widen.
5. **Measure after**, the same commands as step 2 on the fixed tree. Two
   builds are worth the extra ten minutes: the early return alone, then
   both early-outs, so the perf doc can say which bought what. Pass
   criteria under Tests (3).
6. **Bench tooling**: keep, trim and delete per the recommendation below,
   including the matrix's `--help` text, module doc and markdown header
   for all four flags. The harness must pass `npx tsc --noEmit` (today
   `src/bench/pencil.ts:216` fails: `before.texturesCreatedLog` is a
   number, not an array, a line the trim removes anyway) and `npm run
   check`.
7. **Docs**: the perf doc section and the architecture sentences under
   Docs. Commit the matrix's `md` and `tsv` for the pencil cells
   (`bench-results/stroke-replay-matrix-pencil-{before,after}-recorded_curvy_stroke-<hash>.{md,tsv}`,
   the file naming the existing results use) and the browser numbers as a
   table in the perf doc.
8. The full `Lint / CI Checks` list from `CLAUDE.md`. `make wasm` and the
   frontend gates are required (the engine crate and `frontend/src` both
   change).

## Tests

### 1. Regression test: the emitted shape (`tests/wgsl.rs`)

Two existing tests already pin the emitted store text verbatim, so they
are the regression test once their expectations say what the fixed body
emits; no new test and no hand-built graph.

- `paint_terminal_compiles_the_accumulation_dial_to_storage_and_stores`
  (`tests/wgsl.rs:1926-1934`): it compiles a real brush at all three dial
  shapes. Its `build_store`, `wash_store` and `ground_store` closures change
  to the guarded text,
  `    let <t>_was = textureLoad(<t>, layer_px);\n    let <t>_now = pack_ground(<law>(<src>, unpack_ground(<t>_was)));\n    if (<t>_now.r != <t>_was.r) {\n        textureStore(<t>, layer_px, <t>_now);\n    }\n`,
  and it gains, on the `fn cs_main` slice of each module: the early return
  `if (rgba.a * sel == 0.0) {\n        return;\n    }` is present and its
  index is below that of the first `textureLoad(`. `naga_validate` on the
  module (`:1327`) certifies the non-uniform return.
- `authored_buildup_picks_the_accumulation_shape` (`:2250-2254`): its
  `stores` helper counts the law line, `let <t>_now = pack_ground(<law>(`,
  instead of the unguarded `textureStore(` prefix.

Both fail on the unfixed tree (the old text is gone) and pass after. This
is the regression test for the mechanism; the bench cell is the gate for
the symptom.

### 2. Pixel exactness: the existing suite, no new test

`tests/brush_accumulation.rs` (17 tests, exact equality for black on the
analytic disc) is the pixel gate and must pass unchanged. It already
asserts both facts this plan relies on more strongly than a new test could:
`wash_stroke_does_not_darken_when_it_crosses_itself` (`:221`) and
`wash_density_is_independent_of_spacing` (`:187`) are "a refused dab
changes nothing, bit for bit", and `selection_masks_the_dab_pass_under_a_cropped_canvas`
(`tests/paint_compute.rs:176`) is "a zero selection deposits nothing". A
"zero flow leaves the ground bit-identical" test in `paint_compute.rs`
would pass on both trees, so it would be a guard, not a regression test, and
it would restate what the suite already pins. Not added.

### 3. The gate: the bench cell

Same machine, same session, medians of three runs, three trees: the
unfixed port (before), the fixed port (after), and a pre-port worktree of
`929928f9ab` with the same bench patches (the reference the regression is
defined against). The browser instrument is `pace=sync`, a per-event cost
that differences across cells; `pace=realtime` gives the symptom.

- **Browser, `pace=sync`, 1920x1080, `stab=0.6`, mid-dial Pencil:** the
  fixed tree's `event_sync_ms.p50` within the band of the pre-port tree's
  at `radius=250` and `radius=500`, where the band is the spread of the
  pre-port tree's three runs (about 0.5 ms on this iGPU). Last session's
  numbers, the shape of the problem and not the band: 13.4 against 9.8 ms
  at 250 px, 16.4 against 13.6 ms at 500 px.
- **Browser, `pace=realtime`, 500 px:** `long_frames_over_33ms` equal to
  the pre-port tree's (0 of 235 last session; the port drops 8 to 15).
- **Native, `--gpu-sync`, `1920x1080:500`:** the fixed tree's mid-dial
  `cpu p50` within 1 ms of the pre-port tree's (14.4 against 11.2 ms last
  session).
- `dispatches/ev == dabs/ev` still, on every cell run.

The fixed tree's `buildup=1.0` and `buildup=0.0` cells are run as the
diagnostic reference, not as a criterion: mid-dial carries work this fix
does not touch (the second ground's checkpoint snapshot and restore, its
clear, the commit's second foreground), so "within noise of build-only" may
be structurally out of reach while the gate above is met.

What the two early-outs are expected to buy, so a miss is diagnosable: the
return removes two loads and two stores on the majority of body threads
(both grounds, so the ends improve as well); the guard removes the wash
store on every refused dab, leaving the wash half a load on live threads.
If after the fix the pre-port band is still missed, the residual is the
wash load on live threads plus the second ground's copies, and the
alpha-only packed pair under "What is not in this plan" is the next step,
with its own plan.

## Bench tooling: what to keep

The uncommitted bench additions are what found the bug after the port's
gate had passed; that is the argument for keeping them. Minimalism argues
for keeping only what the gate needs and nothing diagnostic.

**Keep and commit** (`crates/darkly/src/bin/stroke_replay_matrix.rs`,
`crates/darkly/src/format/stroke_recording.rs`, the two `None` call sites in
`tests/paint_compute.rs` and `tests/stroke_replay.rs`, and
`bin/stroke_replay_bench.rs`):

- `Topology::Pencil`: the cell that reproduces the regression natively
  (the port plan's Risks named it as "a 15-line bench addition if it
  matters"; it did).
- `--gpu-sync`: blocks on the device after every event through `replay`'s
  new `after_event` hook, which is what makes `cpu p50` a GPU number. The
  matrix never waited on the GPU before, which is the single reason the
  port's gate missed a 20 to 37% GPU regression. The wait is
  `device.poll(wait_indefinitely)` inside a `required-features =
  ["testing"]` binary, native only: the test-only escape hatch of the No
  Blocking GPU Readbacks rule (`docs/lessons-learned/gpu-lessons-learned.md`
  §5). It must not migrate anywhere the engine runs.
- `--buildup`: gives the build-only and wash-only reference cells the gate
  compares against.
- `--stabilize`: the artist's 0.6 is where the rewinds are realistic; 1.0
  stays the default for the full matrix.
- `--only WxH:R`: a two-cell run takes seconds instead of the 28-cell
  matrix.
- One small addition: the markdown header the matrix writes (`write_markdown`,
  which prints `stabilize=`) should also print `gpu_sync=` and `buildup=`
  so a results file describes its own run; the `--help` text and the
  module doc (`stroke_replay_matrix.rs:15-20`) name all four flags.

**Keep, renamed and trimmed:** the browser harness. The port plan's open
question 4 asked for exactly this (a replay inside the browser that turns
the smoke into a number) and this regression was invisible natively.

- `frontend/bench-pencil.html` -> `frontend/stroke-replay.html`, and
  `frontend/src/bench/pencil.ts` -> `frontend/src/bench/stroke_replay.ts`:
  it is parameterised by `brush`, so the name should not say Pencil.
- Keep: the parameters (`brush`, `stab`, `radius`, `buildup`, `pace`,
  `w`, `h`; `mult`, the event-rate multiplier, goes, no gate cell uses it), the `GPUQueue.submit` patch (submit-to-done latency is
  the saturation signal), the `dispatchWorkgroups` count (the `dispatches
  == dabs` check), the frame loop with intervals and `long_frames_over_33ms`,
  the `strokeTo` post cost, the `pace=sync` loop with `event_sync_ms`, and
  the `window.__result` + `document.title = 'done'` handshake. About 120
  lines.
- Remove: the write-buffer, bind-group, draw, pass, copy, texture create
  and destroy, `__fmt`, cleared-attachment and visibility instrumentation.
  Those were one-off diagnostics whose findings are recorded (handoff,
  "What it is not") and are what makes the file fail `tsc` today.
- The page is dev-server-only: Vite's build input is `index.html` (no
  `rollupOptions.input` in `frontend/vite.config.ts`), so it never reaches
  `dist/` or the PWA precache. The fixture import crosses the frontend
  root through `server.fs.allow: ['..']` (`vite.config.ts:98-99`), already
  there. `tsconfig.json` includes `src/**/*.ts`, so the file is under both
  TS gates and must pass them.

**Commit the driver** as `frontend/scripts/bench-drive.mjs` (the directory
already holds the repo's `.mjs` tooling). It is not in the tree today (the
session scratchpad holds it) and lands in step 6: a Node 22 built-ins only
script (`fetch`, `WebSocket`; `package.json` `engines.node >= 22`) that
launches a Chromium with `--remote-debugging-port`, `--headless=new`,
`--enable-unsafe-webgpu`, `--use-angle=vulkan`, `--enable-features=Vulkan`
and `--ignore-certificate-errors` (the dev server is https), navigates each
URL, waits for `document.title === 'done'` and prints `window.__result` as
one JSON line. `PROFILE` env for the user-data dir (default under
`os.tmpdir()`), `CHROME` for the binary (default `chromium`). Playwright is
a `desktop/` devDependency (`desktop/package.json:27,31`) and could drive
this too; the built-ins script is chosen because the frontend has no such
dependency and 55 lines do not justify adding one. The perf doc names the
flags. Without it the harness is a page
someone has to click through; with it the gate is one command:

```bash
HEADLESS=1 node frontend/scripts/bench-drive.mjs \
  'https://localhost:5173/stroke-replay.html?brush=Pencil&stab=0.6&radius=500&pace=sync' \
  'https://localhost:5173/stroke-replay.html?brush=Pencil&stab=0.6&radius=500&buildup=100&pace=sync'
```

**Delete:** `frontend/microbench.html` (a raw-WebGPU copy and clear
micro-bench; its numbers go into the perf doc section and it is not a
gate), `crates/darkly/tests/zz_dump_pencil.rs` (a WGSL dump through a test
and an env var; the shape test supersedes its purpose and
`tests/wgsl_validate.rs` prints a module's source on failure), and the
untracked `browser-profile/` directory at the repo root (a Chromium
profile).

## Docs

- `docs/paint-compute-perf-tracking.md`: a new subsection after "#6: the
  compute paint terminal (shipped)", titled "#6, follow-up: the mid-dial
  Pencil and the store guards": the before and after tables (three trees,
  native and browser), the ruled-out list with the micro-bench's copy and
  clear numbers, which early-out bought what, and a paragraph of narrative
  (the fix, why it is exact, the deferred structural options). Plus a
  paragraph under "Instrumentation status" for the pencil topology and
  flags, the browser harness and the driver with the command above and the
  Chromium flags, saying why the replay is a page and not a wasm entry
  (what it measures, transport post cost, `onSubmittedWorkDone` latency
  and frame intervals, exists only on the JS side) and that the in-engine
  replay the port plan's open question 4 meant is
  `format/stroke_recording.rs::replay` (not `engine/process_recording.rs`,
  the timelapse recorder). About 40 lines.
- `docs/brush/architecture.md`, "How dabs accumulate in the scratch"
  (`:121-135`): after "each thread loads its texel, folds the dab in, and
  stores it", two sentences: a thread whose dab has no coverage at its
  pixel returns before touching either ground, and a thread stores a
  ground only when the law changed its texel, so a dab the ceiling refuses
  costs a load and no store. And the `paint` terminal's `flush_dabs` bullet
  (`:362-368`) gains "where the law changed the texel". About 5 lines.
- `docs/plans/compute-paint-terminal.md`: untouched; its open question 4 is
  answered by the harness and the perf doc records that.

## Risks

- **The gate may not close to the pre-port band.** The wash half still
  pays a load on every live thread, the second ground's copies and clear
  are untouched, and this iGPU's load/store cost split is unmeasured. The
  plan states the expected mechanism so a miss points at the residual; the
  alpha-only pair is the named next step.
- **Uniformity under Tint.** naga with all validation flags accepts the
  module (`tests/wgsl_validate.rs`); Tint applies the same uniformity rules
  to explicit-LOD sampling, and every sample precedes the return anyway.
  The browser smoke in step 8 is the check.
- **Noise on the measurement.** A shared iGPU with compositing; medians of
  three, same session, nothing else running. The handoff's single numbers
  are the shape of the problem, not the band.
- **The TS gates.** `svelte-check` and `tsc` walk `src/bench/`; the trimmed
  harness has to be strict-clean, including the `any` casts the WebGPU
  prototype patch needs (`@webgpu/types` is in `tsconfig.json` `types`, so
  `GPUQueue.prototype` is typed and the casts can mostly go).
- **`--gpu-sync` drifting into production.** It lives in a bench binary
  behind `required-features = ["testing"]`; the perf doc paragraph says so
  in as many words.

## Open questions

1. Should the guarded read-modify-write move into the skeleton now
   (`wgsl/mod.rs`, beside `pack_ground`) rather than when a second compute
   terminal needs it? The plan keeps it in `paint.rs` as the existing
   closure; the reviewer concurred (a generic hook needs a second mechanism
   for one user).
2. File names for the harness (`stroke-replay.html`,
   `src/bench/stroke_replay.ts`, `scripts/bench-drive.mjs`).

## LOC estimate

Lines added / removed. Bench tooling is counted separately from production
because none of it ships; the uncommitted working-tree additions are
counted as added here since this is the change that commits them.

| area | file | added | removed |
| --- | --- | ---: | ---: |
| production | `crates/darkly/src/brush/nodes/paint.rs` (early return, guarded `store` closure, two doc comments) | 22 | 6 |
| **production** | | **22** | **6** |
| tests | `crates/darkly/tests/wgsl.rs` (two tests' store expectations, early-return assertion) | 25 | 12 |
| tests | `tests/paint_compute.rs`, `tests/stroke_replay.rs` (`replay` call sites, in tree) | 2 | 0 |
| **tests** | | **27** | **12** |
| bench | `crates/darkly/src/bin/stroke_replay_matrix.rs` (Pencil topology, four flags, help, module doc, header fields) | 125 | 18 |
| bench | `crates/darkly/src/format/stroke_recording.rs` (`after_event` hook), `bin/stroke_replay_bench.rs` | 5 | 0 |
| bench | `frontend/stroke-replay.html`, `frontend/src/bench/stroke_replay.ts` (trimmed from 222, no `mult`) | 115 | 0 |
| bench | `frontend/scripts/bench-drive.mjs` | 55 | 0 |
| bench | delete `frontend/microbench.html`, `tests/zz_dump_pencil.rs`, `browser-profile/` (untracked, 0 in git) | 0 | 0 |
| **bench tooling** | | **300** | **18** |
| docs | `docs/paint-compute-perf-tracking.md` (follow-up section, instrumentation paragraph) | 40 | 0 |
| docs | `docs/brush/architecture.md` | 5 | 2 |
| generated | `crates/darkly/bench-results/stroke-replay-matrix-pencil-*` (md and tsv, before and after) | 40 | 0 |
| **docs / generated** | | **85** | **2** |
