# Compute paint terminal: `paint` on one dispatch per dab

## Independent Review

Reviewer: a fresh agent given the repository instructions and this plan.
Working tree at `929928f9` (plan untracked). Every file, line, crate and
Krita reference was resolved against the tree,
`~/.cargo/registry/src/index.crates.io-*/{wgpu,wgpu-core,wgpu-types,naga}-29.0.4`
and `krita/`. Two shader claims were checked empirically with a scratch
naga 29.0.4 validator (`ValidationFlags::all()`, `Capabilities::all()`):
the plan's compute skeleton assembled over the real `shaders/lib/canvas.wgsl`,
`brush/_shape.wgsl`, `lib/fbm2d.wgsl` and `brush/_prelude.wgsl`, with group-3
graph textures, `textureSampleLevel` on the selection, and two
`texture_storage_2d<r32uint, read_write>` bindings loaded and stored from the
mid-dial body, validates; so does `composite.wgsl` with `commit_fragment`,
`fs_main` and `fs_packed` over float bindings 0/1 and uint binding 2 in one
module. Tint was not checked (no Dawn tree in the repo).

### What the plan gets right

- **`rgba8unorm` read-write is unreachable in production** (3.3). Verified:
  `wgpu-types-29.0.4/src/texture/format.rs:911-972` grants `R32Uint`
  `s_all` with `atomic` usages (`:933` for `atomic`, not `:930`) and
  `Rgba8Unorm` only `s_ro_wo`; the only widening is
  `TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES`, "a native only feature"
  (`features.rs:666-678`); no tier feature exists in 29 (grep); the
  workspace pins `wgpu = { version = "29.0", features = ["webgpu"] }`
  (`Cargo.toml:37`). Not gating on it is right.
- **Stage validation of the unreachable `textureSample`** (3.1). The rule
  holds but the citation is off: `naga-29.0.4/src/valid/function.rs:1025` is
  `S::Kill` (`discard`); implicit-LOD sampling is marked fragment-only at
  `valid/expression.rs:692` (`SampleLevel::Auto => ShaderStages::FRAGMENT`),
  folded into the function's `available_stages` (`function.rs:1905,1921`),
  and checked per entry point along the call graph
  (`valid/interface.rs:1353`). The scratch module above confirms it.
- **Two entry points, disjoint bindings, one module** (4.5). wgpu-core
  pushes an entry point's resources only for globals the entry point uses
  (`wgpu-core-29.0.4/src/validation.rs:1068-1072`) and `check_stage`
  iterates that list (`:1161`), so the float layout (bindings 0, 1) and the
  uint layout (binding 2) each match their entry.
- **Node bodies are compute-safe.** Grep over `nodes/`, `wgsl/`,
  `_shape.wgsl`, `_prelude.wgsl`, `fbm2d.wgsl`: no derivatives, no implicit
  `textureSample`, `discard` only in the terminal bodies the plan names, and
  the only `in.` is `watercolor.rs:1209-1210`.
- **Bind group budget** (3.2). Default limits at
  `wgpu-types-29.0.4/src/limits.rs:379-398`: `max_bind_groups: 4`,
  `max_storage_textures_per_shader_stage: 4` (ground + build = 2),
  `max_dynamic_uniform_buffers_per_pipeline_layout: 8` (two used),
  `min_uniform_buffer_offset_alignment: 256`,
  `max_compute_invocations_per_workgroup: 256` (16x16 is exactly the limit).
  `uniform_bgl` is `VERTEX_FRAGMENT | COMPUTE` (`pipeline.rs:322-334`), the
  selection layout `FRAGMENT | COMPUTE` (`selection.rs:852-876`), the
  graph-texture layout `FRAGMENT` only (`texture_registry.rs:180,187`).
  Group 1 for the ground is the right slot.
- **Two accumulators inside the dial** (4.6.3). The commit lays the wash
  slot through the ceiling and then the build slot over it
  (`composite.wgsl:150-157`); `both_overlap_sites_build_alike_at_every_dial_setting`
  (`brush_accumulation.rs:682-691`) and the Pencil (`pencil.yaml:130`,
  `buildup: 0.1`) pin that composition; and no single read-write texel can
  hold two RGBA8 accumulations (`Rg32Uint` is `s_ro_wo`, `format.rs`). The
  deviation from the handoff's "the split goes away" is correct and the
  plan says precisely which half of that sentence survives. Variant (a)'s
  rejection is derived; (b)'s is asserted ("changes the Pencil"), which is
  acceptable since (b) would be a semantics change needing its own plan.
- **The per-dab ceiling reproduces `Max` for one pigment** (4.6.1). The
  algebra checks: `d = (1 - a) R`, `t = (s2 - a) / (1 - a)`, new alpha `s2`,
  new colour `C s2`. The 8-bit idempotence argument is exact for black
  (`C = 0`, so the stored premultiplied colour is exactly `0` and `d` is
  exactly `1 - a'`), which is what `brush_accumulation.rs` paints
  (`:124-126`). See finding 7 for coloured pigments.
- **Framework plumbing.** `Scratch::new`/`create_write_texture`/`grow_write`/
  `clear_to_transparent`/`ensure_channels`/`build_channels`/`channel_view`
  (`scratch.rs:198-255, 780-805, 534-626, 389-399, 271-303, 656-713, 318-322`),
  the ring (`checkpoint_ring.rs:91-142, 316-333`), `StrokeBuffer::new`
  (`stroke_buffer.rs:73-137`), `runner.scratch_format()` (`eval.rs:818-827`)
  and `ensure_channels` at `eval.rs:1153` all resolve as described.
  `paint.rs`, `composite_pipeline.rs`, `paint_target_ext.rs`,
  `dab_record.rs`, `gpu_context.rs` citations all hold.
- **Krita** resolves with small line corrections: `kis_brushop.cpp:199`
  (`bltFixed(rc, state->dabsQueue)`); `kis_painter.cc:950-1021` is
  `bltFixed`, `readBytes` at `:980` (not `:975`);
  `kis_indirect_painting_support.cpp:202-205` ("Brushes don't apply the
  selection"), `writeMergeData` at `:230-232`.
- **Delete the spike** (4.10). The argument answers the handoff: once
  `paint` is the shape, the spike is a second copy with a hand-written disc
  and a visible node. `ChannelUse`, `channel_view`, `dispatches` and the
  harness stay. Right.
- **Dab index** stays the static index buffer with a dynamic offset, as the
  spike review settled.

### Findings

1. **(substantive) `tests/wgsl.rs` is not "unchanged beyond two asserts".**
   `paint_terminal_compiles_the_accumulation_dial_to_targets_and_a_body`
   (`:1838-1918`), `each_flow_scales_its_own_half` (`:1925-1983`) and
   `authored_buildup_picks_the_accumulation_shape` (`:2065-2121`) import
   `COVERAGE_CEILING` (deleted in step 6) and assert `dab_blend`, `FsOut`
   and the literal `return rgba * wash_flow * sel;` bodies of the fragment
   terminal. They stop compiling at step 6. Rewrite them to assert the
   compute shape (`dab_pass`, `storage_bindings()`, the `textureStore(ground,
   ...)` / `textureStore(build, ...)` lines, no `FsOut`) and fold the plan's
   test 4 into them instead of adding a parallel test. The LOC row for
   `tests/wgsl.rs` (+45/-2) is closer to +60/-60.

2. **(substantive) Golden fixtures: wrong device class, and a simpler gate
   exists.** CI runs the suite on lavapipe
   (`.github/workflows/ci.yml:89-102`: `mesa-vulkan-drivers`,
   `WGPU_BACKEND=vulkan`) while the goldens would be written on the dev
   machine's Intel iGPU (the only machine with history, 4.11). The
   4 LSB / 5%-beyond-1-LSB histogram then folds Intel-blend-unit versus
   llvmpipe rounding into the port's parity and can fail in CI for reasons
   that have nothing to do with the port. The repo also has no
   golden-compare mechanism to reuse (grep: `image::` only decodes
   generated PNGs, `brush_editor_preview.rs:159`, `docs_render.rs:385`), so
   the writer test, the env var, the sha header and three binaries are new
   machinery whose only reference is a code path the same PR deletes.
   Simpler, using what is in the tree: the spike stays through steps 6 to 8
   anyway, so move `spike_matches_paint_within_tolerance`
   (`tests/paint_dispatch_spike.rs:132-191`) to compare the ported `paint`
   against the spike, live, on whatever device runs the test. Both run the
   same pack/unpack/source-over per dab on the same disc coverage (the spike
   review's item 6 established the coverage equivalence), so the expected
   gap is 0 to 1 LSB and the test can say so; the spike's own distance from
   the fragment path is already recorded (4 LSB max; 1002 exact, 8291 at
   1 LSB, 264 at 2, 2 at 3 of 9559). The test goes with the spike in step
   11. Wash and mid-dial are pinned exactly by `brush_accumulation.rs` (3.7),
   which the proposed goldens duplicate at lower precision (the wash golden
   is a hard black disc under `Max`: a uniform band). If goldens are kept
   regardless: write them on lavapipe, say so in the test, and state each
   tolerance honestly (the black wash golden is exact in alpha by 4.6.1,
   not "within 2 LSB").

3. **(substantive) One fact, three homes: the write side is read-write
   storage in format X.** 4.1 infers `STORAGE_BINDING` from the format's
   `STORAGE_READ_WRITE` flag inside `create_write_texture`; 4.2's
   `storage_bindings()` hard-codes `PACKED_GROUND_FORMAT` for the `ground`
   declaration; the actual format is the registration's `scratch_format`
   (`paint.rs:389`, read through `runner.scratch_format()`,
   `eval.rs:818-827`). They agree by coincidence, and the module doc the
   plan promises ("a channel says the same thing explicitly ... which a
   format cannot") is a symptom. Open question 6 half-asks this; answer it:
   `dab_pass: DabPass` belongs on `BrushNodeRegistration` beside
   `scratch_format` and `lifecycle`. It is a per-type fact (paint always
   dispatches per dab; `dab_blend` was per brush only because `buildup`
   varied it). Expose it like `scratch_format()`; `StrokeBuffer::new` and
   `Scratch::new` take the pair, and the write side gets `STORAGE_BINDING`
   because the terminal declared `DispatchPerDab`, exactly as a channel
   declares `ChannelUse::Storage`; the assembler emits `ground` in the
   registration's format. `compile_brush_to_wgsl` then takes the terminal's
   registration (or the pair) from `compile_graph` (`brush/mod.rs:326-347`),
   and the `tests/wgsl.rs` call sites follow. `NodeWgsl::dab_pass` goes.

4. **(substantive) `_accumulate.wgsl` puts paint's laws in the framework
   prelude.** `accumulate_build` and `accumulate_wash` are the `paint`
   terminal's accumulation laws (the plan says so under Ownership), yet 4.6
   has `assemble_shader` prepend them to every brush, including watercolor,
   smudge and liquify. `NodeWgsl::decls` exists for node-owned helpers:
   emit them from `paint::compile_wgsl` (an `include_str!` of a paint-owned
   file, with `lib/deposit_ceiling.wgsl` included ahead of it; the commit
   shader includes the same lib, which is the DRY the plan wants).
   `pack_ground`/`unpack_ground` are the framework's (they are the packed
   ground format's) and belong in the compute skeleton. Then "the assembler
   knows two skeletons and nothing about `paint`" is literally true.

5. **(substantive) Step 3 is either unnecessary or mis-specified.** The
   footprint origin is `max(floor(pos - bbox), layer_x0)` per axis
   (`clamp_f32`, `coord.rs:183-191`, then `intersect`), and the shader
   already holds every input: `let origin = max(vec2<i32>(floor(d.pos -
   d.bbox_target_px)), u.intrinsic.layer_offset);` is the whole derivation.
   IEEE f32 subtraction and `floor` are correctly rounded on both sides, so
   the "two formulas that must agree" worry has no mechanism. Carrying it
   instead costs 8 to 16 bytes per dab on every terminal, a `vec2<i32>`
   variant of `WgslType` the plan does not list (`type_system.rs:22-28` has
   `F32, U32, I32, Vec2, Vec4`), and four packer edits. If the field is kept
   anyway, the call-site claim is wrong: `read_mirror_terminal.rs:412-418`
   and `watercolor.rs:870` already call `record_dab_footprint` before
   `queue_dab` and have the rect in hand; `prepare_dab_canvas_copy` runs at
   flush, after queueing, so no `DabFootprint::write_rect` is needed. The
   grid size still rides `meta_bytes` as planned.

6. **(substantive) The discrete-GPU session is a hardware dependency placed
   ahead of implementation.** Step 0 and 4.9 make `DAB_WORKGROUP` and the
   large-dab decision wait for a machine nobody in the repo has (open
   question 1), yet step 8 re-measures the ported terminal regardless, and
   `dispatch_cost_bench` is terminal-independent. Decouple: the port
   proceeds on the Intel gate (4.11), `DAB_WORKGROUP` is fixed from the
   Intel harness run with the two new variants, and the discrete session
   runs whenever a machine exists, against the ported terminal (better, no
   spike artefacts) with the harness plus the matrix. Keep the harness
   variants; drop "the spike must still be in the tree when it runs".

7. **(substantive) 4.6.1's 8-bit exactness holds for black, not for every
   pigment.** The induction uses `ground = C a + O (1 - a)` with the stored
   premultiplied colour exactly `C a`; stored it is `round(C_i a 255) / 255`,
   so `d` is perturbed by up to about `1/255` and `t` can come out slightly
   positive on a pixel the ideal law leaves alone, moving alpha by about one
   count once (after which `s - a'` outweighs the perturbation; `R >= 0.5`
   bounds it). The exact tests paint black (`:124-126`), so they are
   unaffected; `wash_saturates_per_pigment_not_globally` (`:412-460`) is the
   coloured one and asserts inequalities. Say this in 4.6.1 and in the
   `SITE_PARITY_TOL` risk, and have test 7 assert within 2 LSB, not 1.

8. **(minor) Nonexistent API in 4.1.** `sample_type` is a method on
   `TextureFormat` (`format.rs:1070-1074`, `format.sample_type(None, None)`),
   not on `TextureFormatFeatures`; the plan's
   `guaranteed_format_features(..).sample_type(None, None)` does not exist.

9. **(minor) `deposit_through_ceiling` is not "the same arithmetic as
   today".** Today a saturated pixel (`t == 0`) still goes through
   `source_over(pigment * 0, 0, bg)` (`composite.wgsl:122`), which divides
   by `bg.a` and zeroes rgb under `0.001`; the plan's `if (t <= 0.0) return
   bg` skips it. That is an improvement (an exact identity at saturation);
   state it as one. `wash_caps_coverage_across_strokes` compares alpha
   only, so no exact test moves.

10. **(minor) Citations and claims to correct.** `naga valid/function.rs:1025`
    is `discard`; the sampling rule is `valid/expression.rs:692`.
    `format.rs:930` for `atomic` is `:933`. `kis_painter.cc:975` is `:980`.
    Tests item 10: `tests/shader_compile.rs` validates a preamble only when
    an entry-point file under `shaders/` names one of its functions
    (`shader_compile.rs:18-34, 58-70`); nothing there will call
    `accumulate_*`, so `_accumulate.wgsl` (or its paint-owned successor) is
    covered by `tests/wgsl_validate.rs` through the builtin paint brushes,
    not by `shader_compile.rs`. 4.6.2's "87% within 1 LSB": the spike's
    histogram is 1002 exact plus 8291 at exactly 1 LSB of 9559, i.e. 97%
    within 1 LSB (the perf doc has the same slip).

11. **(minor) Files the step list misses.** `read_mirror_terminal.rs:708-716`
    and `paint.rs:826-836` each build a `BuildContext` from the
    `(bgl, sampler)` pair; the `CanvasCopyLayout` refactor changes
    `BuildContext` (`pipeline.rs:120-145`) and both sites. The `meta_bytes`
    doc (`gpu_context.rs:246-254`) says per-dab-feedback terminals only and
    needs a sentence. Test 4 compares against `BUILD_CHANNEL`, which is a
    private `const` in `paint.rs:65-71`; compare by name, kind and format.

12. **(minor) LOC bookkeeping.** The summary says +785 / -1090; the table
    sums to +821 / -1098. Fold in findings 1 (tests), 3 and 4 (about
    neutral), 5 (removes about 35 production lines) and the goldens decision
    (2). Step 0's 60 lines are bench tooling, not production.

13. **(minor) Say the live-canvas sampler is not foreclosed.** In the
    compute skeleton `ground` is in scope for upstream node bodies in stroke
    mode; a sampler node reads it with `textureLoad` and must emit a neutral
    preview body, the `clone_source` rule in
    `docs/brush-preview-and-overlays.md`. One sentence in 4.2 records that
    the paused plan's rewrite has its hook. Whichever name the write side
    gets in WGSL (`ground` is fine; the repo's prose says "write side"),
    document it on `NodeWgsl::body` (`context.rs:28-37`) beside `d`, `u`
    and `sel`.

Verified and accepted without change: the dispatch grid as the clamped
footprint from `record_dab_footprint` in plane pixels with the layer test
and the `bbox` reject making the written set identical to the fragment
path's (4.2, checked against `STROKE_VERTEX_STAGE_WGSL` and the rasterizer's
centre rule); selection sampled per dab inside the law (4.7); erase at
commit through `commit_fragment` (4.7); the hover preview untouched (4.8,
`cursor_preview_wgsl` stays a fragment module and `render_preview` never
reads `dab_blend`); checkpoint ring, clear and grow format-agnostic with
the ground as the write side (3.4); `dispatches == flushed_dabs` as the
counter contract.

Verdict: revise.

The diagnosis, the shape (ground as the write side, compute skeleton chosen
by the terminal, laws as shader code, packed commit) and the gate are
sound, and the wgpu and naga claims hold under direct validation; nothing
here asks for a rethink. What needs revising is scope and ownership around
the edges: a test file the plan says is untouched will not compile, the
parity gate is built on committed binaries from the wrong device when a
live comparison against the spike is already in the tree, the write side's
storage-ness and format are declared in three places, paint's laws are
prepended to every brush, a dab-record field is added for a value one
shader line derives, and the large-dab measurement is gated on hardware
the plan cannot schedule. Each is a small, concrete change; together they
shrink the port and make its remaining machinery sit where the principles
say it should.

## Revision (response to the review)

Every finding above was checked against the tree before being acted on.
What changed, by finding:

1. **Accepted.** `tests/wgsl.rs:1839-1918`, `:1925-1983` and `:2065-2121`
   import `COVERAGE_CEILING` and assert `dab_blend`, `FsOut` and the
   fragment bodies (verified by grep). The Tests section now rewrites those
   three tests to assert the compute shape and folds the former test 4 into
   them; the LOC row is +60 / -60.
2. **Accepted; goldens dropped.** CI runs the suite on lavapipe
   (`.github/workflows/ci.yml:89-102`, `WGPU_BACKEND: vulkan`), so PNGs
   written on the Intel iGPU would gate on blend-unit differences the port
   does not own, and the repo has no golden mechanism to reuse. The parity
   gate is now the existing live comparison
   `spike_matches_paint_within_tolerance` (`tests/paint_dispatch_spike.rs:132`),
   which after the port compares the ported `paint` against the spike on the
   same device, plus the exact `brush_accumulation.rs` suite for wash and
   mid-dial. Old step 5 is gone; open question 3 is resolved.
3. **Accepted.** `dab_pass: DabPass` is a field of `BrushNodeRegistration`
   beside `scratch_format` and `lifecycle` (`node.rs:55-78`); the write
   side's `STORAGE_BINDING` follows from the declared pass through
   `DabPass::write_side_usage()`, the assembler emits `ground` in the
   registration's `scratch_format`, and `NodeWgsl::dab_pass` and the
   format-inferred usage are gone. Non-terminals go through
   `BrushNodeRegistration::compute` (`node.rs:135-147`), so only the nine
   terminal literals gain a line. Open question 6 is resolved.
4. **Accepted.** The laws live in a paint-owned file emitted through
   `NodeWgsl::decls` from `paint::compile_wgsl`; the skeleton carries only
   `pack_ground` / `unpack_ground`. Section 4.5 rewritten.
5. **Accepted; old step 3 deleted.** `clamp_f32` (`src/coord.rs:183-191`)
   floors `position - bbox_radius` and intersects the extent; the shader
   holds `d.pos`, `d.bbox_target_px` and `u.intrinsic.layer_offset`, and
   IEEE f32 subtraction and `floor` are correctly rounded on both sides, so
   `max(floor(d.pos - bbox), layer_offset)` is the same integer. No record
   field, no `WgslType::IVec2`, no packer edits; the grid size still rides
   `meta_bytes`. The call-site claim about `prepare_dab_canvas_copy` was
   wrong and is withdrawn (`read_mirror_terminal.rs:420`, `watercolor.rs:874`
   already call `record_dab_footprint` before `queue_dab`).
6. **Accepted.** The discrete-GPU session is a follow-up against the ported
   terminal, run whenever a machine exists; it no longer gates
   implementation or requires the spike. `DAB_WORKGROUP` is fixed from the
   Intel harness run with the two new variants. Section 4.9 and step 0
   rewritten.
7. **Accepted.** 4.5.1 now states that 8-bit idempotence is exact for black
   (what the exact tests paint) and holds within one count for coloured
   pigments; test 7 asserts within 2 LSB; the `SITE_PARITY_TOL` risk names
   the mechanism.
8. to 13. **Accepted.** `TextureFormat::sample_type` (`format.rs:1070`);
   the saturated-pixel early return is recorded as an improvement; the naga,
   `format.rs` and Krita line numbers are corrected; the 87% figure is 97%
   (and the perf doc's slip is fixed in step 9); `shader_compile.rs` is no
   longer claimed to validate the laws file; `BuildContext`
   (`pipeline.rs:120-145`) and its two construction sites
   (`read_mirror_terminal.rs:710`, `paint.rs:826`) are in step 1; the
   `meta_bytes` doc is in step 6; the LOC table is re-summed and bench
   tooling has its own bucket; 4.2 records the live-canvas sampler's hook
   and the `ground` binding is documented on `NodeWgsl::body`.

Nothing was rejected.

## Implementation notes

What the implementation found that the plan did not say, recorded as the
plan asks. Nothing below changes the shape; each is a local decision made
where the tree disagreed with the text.

- **Steps 3 to 5 landed together.** Removing `NodeWgsl::dab_blend` (step
  3) leaves the instanced `paint` with no way to select its `Max` state,
  so there was no useful intermediate in which the assembler had the
  compute skeleton and `paint` was still instanced. Steps 1 and 2 were
  each gated by the full suite as planned; the port's own gate ran once
  after step 5.
- **`CanvasCopyLayouts`, not one layout per scratch.** The three layout
  classes live in one value (`pipeline.rs`, `for_format`) that
  `BrushPipelines`, `BuildContext` and every `Scratch` carry, and each
  side of a scratch binds through the layout for its own format. That is
  what lets a storage channel have a read bind group like any other
  channel (the mid-dial commit reads the `build` ground back through
  `fs_packed`), so `StrokeChannels::bind_groups` is no longer optional and
  `ChannelUse` affects usage only.
- **`CompiledBrush::dab_blend` is gone now, not later.** With
  `NodeWgsl::dab_blend` deleted nothing set it; `color_targets(format,
  blend)` takes the scratch's blend from the caller (watercolor passes
  its source-over, as it already did by overriding the field).
- **`compile_brush_to_wgsl` takes the terminal's `&BrushNodeRegistration`**
  rather than the `(pass, format)` pair: one argument, the home of both
  facts. `tests/wgsl.rs` resolves it with a `terminal_of(graph)` helper;
  the rejection test flips `dab_pass` on a clone of watercolor's
  registration, so no test evaluator was needed.
- **Test 6 is re-scoped.** No shipped node yields a per-dab pigment except
  a texture (`image`, `noise.color`, `clone_source`), the stroke colour is
  fixed at `begin_stroke` (`StrokeRecord::new(color)`), and no engine API
  registers a texture after init, so "two pigments in one stroke" is not
  reachable through the engine. In its place
  `coloured_wash_is_spacing_independent_within_two_lsb` pins the coloured
  per-dab wash arithmetic of 4.5.1 (review finding 7): the analytic disc
  under `Wash` with a coloured pigment at a 30x spacing spread agrees per
  interior pixel within 2 LSB in premultiplied space, with the chroma
  intact. The lifted restriction is recorded as behaviour in
  `docs/brush/architecture.md` and holds by the law's construction.
- **`docs/architecture-history.md` is untouched.** Its subject is the
  JS/Rust boundary; the brush terminal's history is the perf doc's
  attempts section, where attempt #6 records the port.
- **`dispatch_counter_is_per_dab_for_the_spike_and_per_flush_for_paint`**
  lost its `paint` half at step 5 (paint is per dab too) and went with the
  spike; `dispatches_count_one_per_dab` in `tests/paint_compute.rs` is its
  successor.
- **Harness result (step 0).** `bench-results/dispatch-cost-bench-929928f9ab.md`:
  8x8 wins every small-dab cell (1.5 px and 10 px at 300 to 2000 dabs:
  the 16x16 variant is 15 to 45% slower on wall, the four-row variant 30
  to 70% slower); at 1000 px the row loop gains about 20% of GPU time and
  16x16 gains nothing. By 4.8's rule the port ships 8x8 thread-per-pixel
  and the row loop stays deferred.
- **Parity (Tests, 1).** After the port, `spike_matches_paint_within_tolerance`
  measured 9559 painted pixels, max 1 LSB, 0 beyond 1 LSB, before the
  spike was deleted.
- **Baseline (step 0).** `stroke-replay-matrix-paint{,-rerun}-recorded_curvy_stroke-929928f9ab`
  agree with the `746570670c` reference within the noise band on every
  cell but 4K at 2000 px (+44 and +118 ms against +11), which is noisy on
  this machine.
- **Bench after (step 7) and the row loop.** The gate holds at every
  cell up to 500 px (within a few ms of the baseline, `dispatches/ev ==
  dabs/ev`); 4K at 1000 px is +30 / +11, 4K at 2000 px is +2986 / +1355
  against the spike's +2688, a cell with about ±800 ms of noise. Per 4.8's
  commitment the row loop was applied and the matrix run once: +1115 at
  4K 2000 px, +9 at 4K 1000 px, every other cell unchanged (CPU-bound, so
  the harness's small-dab GPU loss cannot show there). By 4.8's rule it
  stays deferred; the run is kept as
  `stroke-replay-matrix-paint-compute-after-four-rows-per-thread-recorded_curvy_stroke-929928f9ab.tsv`
  and the perf doc's attempt #6 records the decision.
- **Browser smoke (step 9)** was not run from this session, which has no
  browser; the native suite, naga validation of every builtin's compute
  module, `make wasm`, the frontend and desktop gates are what ran. The
  Tint question of 3.1 (an unreachable `textureSample` in a compute
  module) is therefore still open until a browser paints with the Ink Pen.
- **Lines, as landed** (`git diff --numstat` plus the new files):
  production about +1410 / -1380, of which `paint.rs` +280 / -155, the
  assembler +270 / -67, `scratch.rs` +168 / -90 (about 100 of the added
  are its unit tests), `pipeline.rs` +167 / -70 (`CanvasCopyLayout` and
  `CanvasCopyLayouts`), the commit shader and pipeline +128 / -121, the
  two new shader files +90, the harness variants +78 / -9, the spike
  -681 and its bench topology -28; tests about +680 / -420
  (`paint_compute.rs` +360, `wgsl.rs` rewrites, the spike test -254);
  docs about +250 / -100 plus the bench records. The plan estimated
  +780 / -1105 production: the overrun is the layout-class value the
  review asked for being carried by every scratch (which is what gave
  storage channels a read bind group), the assembler's skeleton split into
  three emitters with their docs, and the scratch unit tests living in
  `src`.

## Summary

Port the shipped `paint` terminal from one instanced fragment draw per flush
to one compute pass per flush with one `dispatch_workgroups` per dab over the
dab's layer-clamped footprint, one thread per pixel, reading and writing a
stroke-resident `r32uint` ground that holds packed premultiplied RGBA8. The
decision and the measurement are in `docs/paint-compute-perf-tracking.md`
("#5, stage 1" and "#5, stage 2") and
`notes/handoffs/handoff-compute-paint-terminal.md`; this plan settles how the
port is built, what it deletes, and how it is gated.

The shape, in one paragraph. A terminal's registration declares how its
per-dab pass writes the scratch (`dab_pass: DabPass`), beside the scratch
format it already declares. The WGSL assembler gains a second stroke
skeleton, a compute entry point, chosen by that declaration; every
non-terminal node body is spliced into it unchanged, and the hover preview
keeps the fragment skeleton it has today. The ground is the stroke scratch's
write side (paint's `scratch_format` becomes `R32Uint`), not a side channel,
so the checkpoint ring, the clear, the grow and the commit all act on one
texture, and the spike's unpack pass and doubled checkpoint copies disappear.
Each accumulation law becomes shader code owned by `paint` and applied per
dab against the live ground: build-up is premultiplied source-over, wash is
the commit's deposit ceiling applied per dab, which for one pigment is
exactly the `Max` blend (derivation in section 4.5.1). The commit reads the
packed ground directly through a second fragment entry point in
`composite.wgsl`. Selection stays in the per-dab pass; erase stays at commit.
The `paint_dispatch_spike` node, its shader, fixture, test and bench topology
are deleted once the real terminal passes the gate; `dispatch_cost_bench`
stays as the harness that sizes the large-dab mitigations.

Net production LOC is negative (about +785 / -1100): the port removes the
spike, the `Max` blend state and the fragment pipeline in `paint.rs`, and adds
the compute skeleton, the ceiling library, paint's law file, a
packed-foreground commit variant and a uint canvas-copy layout.

## Feature semantics

### What the artist sees

Nothing changes on purpose. Every shipped paint brush (`airbrush`,
`calligraphy`, `charcoal`, `clone`, `hair`, `ink_pen`, `pencil`, `rough_ink`,
`sponge`; the nine `type: paint` files under `crates/darkly/brushes/`) lays
down the same pixels within the rounding two 8-bit paths accumulate (section
4.5.2 and the tolerance in Tests). Erase, stroke opacity, selection feathering,
the accumulation dial, mid-stroke layer growth, stabiliser rewinds, the hover
preview and the picker thumbnails behave as today.

One improvement falls out and is recorded as a feature, not a goal: under
`Wash` a graph that varies dab colour per dab (`random` into `split_color`,
an `image` tip) no longer fringes. Today's `Max` runs per channel and takes
each channel's maximum from a different dab
(`docs/brush/architecture.md`, "What `Wash` requires"); the per-dab ceiling
deposits whole pigments, so the restriction "such brushes must stay on
`Build-up`" is lifted.

### What the engine does per flush

1. `evaluate_gpu` records the dab's layer-clamped footprint and queues one
   dab record per dab, as today, and pushes the footprint's size onto the
   batch's per-dab meta.
2. `flush_dabs` uploads the records once, opens one compute pass, binds the
   uniforms, the selection, the graph textures, and per dab sets the dab
   index through a static index buffer with a dynamic offset and dispatches
   `ceil(w / 8) x ceil(h / 8)` workgroups over the footprint.
3. Each thread owns one canvas pixel: it derives the footprint's origin from
   the record, rejects pixels off the layer and past the dab's bbox,
   evaluates the compiled graph at the pixel centre exactly as the fragment
   skeleton does, loads the ground texel, applies the brush's law, stores
   it. Dispatches in one pass are ordered and a dispatch's storage writes
   are visible to the next (wgpu-core inserts a barrier per dispatch;
   section 3.6), so dab `n + 1` reads dab `n`.
4. `commit` runs the unchanged `commit_brush_dab` law with the ground (and
   the build channel, inside the dial) as packed foregrounds.

## What was verified before designing (facts, with sources)

### 3.1 What the assembler emits today

`crates/darkly/src/brush/wgsl/mod.rs`:

- `compile_brush_to_wgsl` (`:237-609`) walks the plan, splices non-terminal
  bodies into `shared_body` and the terminal's `compile_wgsl` body into
  `stroke_terminal_body` (`:427-440`), collects `dab_fields`,
  `uniform_fields`, `terminal_bindings`, `channels` and `dab_blend`
  (`:492-503`), and calls `assemble_shader` twice: `ShaderMode::Stroke` with
  the terminal bindings and channels (`:564-573`) and
  `ShaderMode::CursorPreview` with neither (`:577-586`).
- `assemble_shader` (`:894-1080`) prepends `CANVAS_LIB`, `_shape.wgsl`,
  `fbm2d.wgsl`, `_prelude.wgsl` (`:907-916`); emits `DabRecord` and
  `Uniforms` (`:919-939`); binds group 0 uniforms and group 1 `dabs`
  (`:946-947`), group 2 selection in stroke mode plus the terminal bindings
  (`:948-957`), group 3 graph textures in both modes (`:979-988`); splices
  node decls; emits `STROKE_VERTEX_STAGE_WGSL` or the preview vertex stage
  (`:998-1001`); then `fs_main`, returning `vec4<f32>` or a generated
  `FsOut` with one field per attachment channel (`:1017-1035`), and the
  header every node body relies on: `d = dabs[in.dab_idx]`, `target_pos`,
  `local`, `local_dist_px` with the `discard` past `d.bbox_target_px`
  (`:1036-1046`), `local_uv`, `local_dist`, `theta` with `view_rotation`
  (`:1047-1058`), `canvas_size`, `canvas_origin` and `sel` sampled with
  `textureSampleLevel` through `plane_to_selection_uv` (`:1059-1075`).
- `STROKE_VERTEX_STAGE_WGSL` (`:1087-1137`) builds the instanced quad at
  `dab.pos +- dab.bbox_target_px` in layer NDC.

What node bodies may use is documented on `NodeWgsl::body`
(`wgsl/context.rs:28-37`): `d`, `u`, `local_uv`, `local_dist`, `theta`,
`target_pos`, and decls. Non-terminal bodies contain no `discard`, no
derivatives and no implicit-LOD `textureSample` (grep over
`crates/darkly/src/brush/nodes/` and `wgsl/`: the only `discard`s are in the
terminal bodies of `blur.rs:241`, `smudge.rs:164`, `watercolor.rs:1201-1260`
and `liquify.rs:301,330`, and the only `in.` reference is watercolor's
`in.dab_idx` at `watercolor.rs:1209-1210`). `sample_graph_texture`
(`wgsl/mod.rs:800-802`) emits `textureSampleLevel`, legal in a compute
shader. So every shipped non-terminal body is valid inside a compute entry
point with the same `let` bindings in scope.

One item to watch: `shaders/lib/canvas.wgsl:34-46` (`sample_mask_plane`)
calls `textureSample`, a fragment-only builtin, and `CANVAS_LIB` is
prepended to every assembled brush. naga marks implicit-LOD sampling as
fragment-only (`naga-29.0.4/src/valid/expression.rs:692`,
`SampleLevel::Auto => ShaderStages::FRAGMENT`), folds it into the function's
`available_stages` (`valid/function.rs:1905,1921`) and checks them only
along each entry point's call graph (`valid/interface.rs:1353`), so an
unreachable fragment-only helper does not fail a compute module under naga;
the reviewer confirmed this with a scratch naga 29.0.4 validator over the
real `canvas.wgsl`. Tint's rule has the same shape (stage restrictions are
checked per entry point along the call graph), but the port confirms it in
the browser (step 9); the fallback is to split `plane_to_selection_uv` from
the sampling helpers into their own file, which changes no shader semantics.

### 3.2 Bind group budget and visibilities

- WebGPU's default `max_bind_groups` is 4 (`wgpu-types-29.0.4/src/limits.rs:379-398`,
  which also gives `max_storage_textures_per_shader_stage: 4`,
  `max_dynamic_uniform_buffers_per_pipeline_layout: 8`,
  `min_uniform_buffer_offset_alignment: 256` and
  `max_compute_invocations_per_workgroup: 256`); paint today uses all four
  groups in a graph with textures: 0 uniforms, 1 dabs, 2 selection, 3 graph
  textures (`paint.rs:174-198`). Charcoal is `image` plus `paint`, so group 3
  is not free for the ground. The ground therefore joins group 1 (section
  4.2); the ground plus the build channel are two of the four storage
  textures a stage may bind.
- `uniform_bgl` is `VERTEX_FRAGMENT | COMPUTE` (`brush/pipeline.rs:322-334`).
- The selection layout is `FRAGMENT | COMPUTE` by design
  (`crates/darkly/src/gpu/selection.rs:852-876`).
- The graph-texture layout is `FRAGMENT` only
  (`crates/darkly/src/gpu/texture_registry.rs:172-204`, `:180` and `:187`):
  two one-word changes.
- The canvas-copy layouts are `FRAGMENT` only (`pipeline.rs:340-389`); the
  commit is a fragment pass, so they stay as they are, and the uint variant
  added in section 4.1 is `FRAGMENT` too.

### 3.3 Storage formats, and the `rgba8unorm` question

`~/.cargo/registry/src/index.crates.io-*/wgpu-types-29.0.4/src/texture/format.rs`:
`guaranteed_format_features` (`:911`) grants `R32Uint` `s_all`
(`STORAGE_READ_ONLY | STORAGE_WRITE_ONLY | STORAGE_READ_WRITE`, `:917-919`,
`:965`) with `RENDER_ATTACHMENT | STORAGE_BINDING | TEXTURE_BINDING | COPY_*`
usages (`atomic = attachment | storage | binding`, `:933`); `Rgba8Unorm` is
`s_ro_wo` (`:971`), no read-write. The only thing that widens this in wgpu 29
is `Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES`, documented "This is
a native only feature" (`wgpu-types-29.0.4/src/features.rs:666-678`); there
is no WebGPU tier feature in this wgpu (grep of `features.rs` for
`TEXTURE_FORMATS_TIER` finds nothing). Darkly's production path is the
`webgpu` backend everywhere: the workspace pins
`wgpu = { version = "29.0", features = ["webgpu"] }` (`Cargo.toml:37`), the
wasm bridge uses it (`frontend/wasm/Cargo.toml:21`), and the desktop is an
Electron host (`CLAUDE.md`, repo layout). Native wgpu runs only tests and
benches.

**Answer:** `rgba8unorm` read-write is not gated on in this port. It cannot
be reached from the backend the product ships on, and sizing its gain
(dropping the pack and unpack) would be a native-only number. The port keeps
the pack/unpack behind two one-line helpers in the compute skeleton so that
if a future wgpu exposes a web feature for it, the change is paint's
`scratch_format` and those helpers. Recorded under Open questions.

### 3.4 What `Scratch`, the ring, the clear and the grow do with an `r32uint` write side

- `Scratch::new` (`scratch.rs:198-255`) allocates the write side with
  `RENDER_ATTACHMENT | COPY_SRC | COPY_DST | TEXTURE_BINDING`
  (`create_write_texture`, `:780-805`) and builds `write_bind_group` and
  `read_mirror_bind_group` over the caller's canvas-copy layout
  (`canvas_copy_bind_group`, `:834-855`: texture at binding 0, sampler at
  binding 1). The layout comes from `canvas_copy_layout_for(format)`
  (`pipeline.rs:772-788`), which picks the filterable float pair or the
  non-filterable float pair by `FILTERABLE`. An `r32uint` view cannot satisfy
  either (`Float` sample type), so the port adds a `Uint` layout (section
  4.1), and the write side needs `STORAGE_BINDING`.
- `ensure_channels` (`:271-303`) and `build_channels` (`:656-713`) already
  add `STORAGE_BINDING` for `ChannelUse::Storage` and skip the canvas-copy
  bind group (`:675-677`, `:693-702`); `channel_view(name)` (`:318-322`)
  binds by name. `ChannelUse` is permanent (handoff).
- `clear_to_transparent` (`:389-399`) clears the write side and every channel
  as colour attachments with `Color::TRANSPARENT`; the spike's `r32uint`
  channel is cleared this way today and `tests/paint_dispatch_spike.rs`
  passes, so a uint attachment clear validates on this driver. `R32Uint` keeps
  `RENDER_ATTACHMENT` (3.3), so the write side clears the same way.
- `grow_write` (`:534-626`) reallocates and `copy_texture_to_texture`s the
  write side and each channel at the canvas-anchored offset; format-agnostic.
- The checkpoint ring allocates slots in `stroke.texture.format()` and each
  channel's `t.format()` (`checkpoint_ring.rs:316`, `:328-333`, `ensure_texture`
  `:91-142`, usage `COPY_SRC | COPY_DST` `:133`) and restores by copy
  (`:472-551`); format-agnostic. With the ground as the write side a
  `buildup = 1` brush snapshots one texture per checkpoint, as `paint` does
  today; the spike's doubled traffic (`docs/plans/compute-dispatch-per-dab-spike.md`
  section 3.5) is gone by construction.
- `StrokeBuffer::new` (`stroke_buffer.rs:73-137`) takes the terminal's
  `scratch_format` from the registration through `runner.scratch_format()`
  (`eval.rs:818-827`; callers `painting.rs:1020-1031`,
  `preview_renderer.rs:130-143`), and the pre-stroke snapshot and its bind
  group stay `Rgba8Unorm` over the float layout (`:91-126`). The `dab_pass`
  travels the same road (4.1).

### 3.5 The commit today

`commit_brush_dab` (`paint_target_ext.rs:52-62`, `:95-152`) binds two
foreground bind groups over the canvas-copy layout at groups 1 and 2 and the
pre-stroke snapshot at group 3 (`composite_pipeline.rs:77-93`), picks the
pipeline by destination format (`:142-147`), and `composite.wgsl` samples the
foregrounds with `textureSample` (`:134-135`), lays the wash slot through
`deposit_through_ceiling` (`:94-123`) and the build slot through
`source_over` (`:150-157`), or `destination_out` under erase (`:137-143`).
The spike's unpack pass (`paint_dispatch_spike.rs:492-539`) exists only
because this shader cannot read a uint texture.

### 3.6 Dispatch ordering

`wgpu-core-29.0.4/src/command/compute.rs:285-298`: compute passes have a
usage scope per dispatch and `drain_barriers` runs before each dispatch "if a
previous dispatch had a conflicting usage"; the spike review verified the
Vulkan transition for `STORAGE_READ_WRITE`
(`docs/plans/compute-dispatch-per-dab-spike.md`, review item 4) and stage 1
measured the barrier as free. On the `webgpu` backend the browser owns
ordering; `tests/paint_dispatch_spike.rs::spike_replay_is_deterministic_and_paints`
(moved to `paint_compute.rs` in step 10) is the determinism net on native,
and the browser smoke in step 9 is the only check the web gets until a
browser replay harness exists (Open questions).

### 3.7 What the existing accumulation tests demand

`crates/darkly/tests/brush_accumulation.rs` runs every paint brush law
through `tests/fixtures/analytic_disc.yaml` and asserts, among others:

- exact equality of wash density across a 30x spacing spread (`:187-208`),
  exact per-pixel equality when a wash stroke doubles back (`:221-250`),
  one-dab deposit equal to `pressure * 0.1 * 255` within 1 (`:261-288`);
- byte identity between an unwritten `buildup` and an explicit `1.0`
  (`:332-349`);
- exact equality across eight wash strokes along one path (`:379-409`,
  the commit's ceiling);
- the dial ladders: a within-stroke retrace adds exactly 0 at `Wash` and
  strictly more at each rung (`:654-673`), and a retrace and a second stroke
  agree within `SITE_PARITY_TOL = 2` at every rung (`:615`, `:682-691`),
  including the imported mid-dial fixture (`:884-942`).

The exact tests paint black (`:124-126`). The within-stroke exactness tests
are what the per-dab wash law must satisfy bit for bit; section 4.5.1 shows
it does. The mid-dial parity tests are what forces the port to keep two
accumulations inside the dial (section 4.5.3).

### 3.8 Prior art (Krita, read in the tree)

Only one design question here was genuinely open for prior art: whether a
read-modify-write of a stroke-resident buffer per dab, with the commit as a
separate step, is a sound stroke model. It is Krita's. `KisBrushOp::paintAt`
queues dabs and lands them with `state->painter->bltFixed(rc, state->dabsQueue)`
(`krita/plugins/paintops/defaultpaintops/brush/kis_brushop.cpp:199`);
`KisPainter::bltFixed` (`krita/libs/image/kis_painter.cc:950-1021`) reads
the destination region (`d->device->readBytes`, `:980`) and composites the
dab into it with the paintop's composite op, a per-dab read-modify-write of
the painter's device, which during a stroke is the indirect-painting
temporary target. At stroke end
`KisIndirectPaintingSupport::mergeToLayerImpl` blits that target onto the
layer (`writeMergeData` at
`krita/libs/image/kis_indirect_painting_support.cpp:230-232`). Krita applies
the selection at the merge ("Brushes don't apply the selection, we apply that
during the indirect painting merge operation", `:202-205`); Darkly applies it
per dab, and keeps doing so, because the wash law needs `sel` inside the
per-dab deposit for feathering to scale the mark rather than saturate
through it (`docs/brush/architecture.md`, "Canvas-space fields survive").
Everything else in this plan is answered by the tracking doc, the spike and
the code above, and the Prior Art Principle says to skip it.

## Architectural impact

- **Authority.** Nothing new is document state. The ground is the stroke
  scratch (session/compositor), the build channel is a stroke channel, the
  registration carries the dab-pass choice. No document field, no mirror.
- **Ownership.** The ground's lifecycle stays with `Scratch`; what is
  accumulated in it stays with `paint`. The accumulation laws move from a
  blend state in `node.rs` to WGSL in a paint-owned file emitted through
  `NodeWgsl::decls`, and the shared ceiling to `shaders/lib/deposit_ceiling.wgsl`,
  included by both paint's law file and the commit shader (DRY: one law, two
  compositing conventions). The pack and unpack of the ground format are the
  framework's and live in the compute skeleton.
- **Modularity.** A terminal's registration declares `dab_pass: DabPass`
  (`InstancedDraw` or `DispatchPerDab`) beside `scratch_format` and
  `lifecycle`; it is a per-type fact, like the format. The assembler knows
  two skeletons and nothing about `paint`; watercolor, smudge, blur and
  liquify declare `InstancedDraw` and compile as today. A future compute
  terminal declares `DispatchPerDab` and storage channels and gets the
  bindings without editing the assembler. The write side's storage usage
  follows from the declared pass through a method on `DabPass`, exactly as a
  channel's follows from `ChannelUse::Storage`. The commit picks its
  foreground fetch by the scratch format, through
  `CompositePipeline::pipeline(dest, foreground)`, not by terminal.
- **DRY.** `deposit_through_ceiling`'s room arithmetic becomes `ceiling_t` in
  one lib used by both the per-dab law and the commit. The canvas-copy
  layout kinds collapse into one `CanvasCopyLayout` value (`bgl`, optional
  sampler, texture binding) built once per kind and used by `Scratch`,
  `BuildContext` and the commit instead of a `(bgl, sampler)` tuple plus two
  parallel helpers.
- **What this port deletes.** `brush/nodes/paint_dispatch_spike.rs`,
  `shaders/brush/paint_dispatch_spike.wgsl`,
  `tests/fixtures/ink_pen_dispatch_spike.yaml`,
  `tests/paint_dispatch_spike.rs`, `Topology::PaintDispatchSpike` and its
  fixture include in `bin/stroke_replay_matrix.rs` (`:62-66`, `:101-107`,
  `:117`, `:129`, `:142`, `:153`, `:157-165`, `:201`, `:208`, `:214`);
  `node::COVERAGE_CEILING` (`node.rs:103-132`) and paint's `dab_blend`
  choice; paint's render pipeline, `FsOut` return and
  `Attachment { blend }` build channel. `nodes/mod.rs` regenerates.
  Bench-result files stay as records.
- **What is deleted later, not here** (handoff items 2 to 5): the read-mirror
  loop (`read_mirror_terminal.rs:472-600`, `Scratch`'s read mirror,
  `prepare_dab_canvas_copy`), the `smudge` and `blur` terminals, watercolor's
  atlas and group-3 `terminal_bindings`, `STROKE_VERTEX_STAGE_WGSL`,
  `ChannelUse::Attachment` and the generated `FsOut`, `CompiledBrush::dab_blend`
  and `color_targets`, `DabPass::InstancedDraw`, and the float foreground
  entry in `composite.wgsl`. Each goes when its last user is a graph on
  `paint`.
- **No blocking readbacks.** None added. The benches keep their test-only
  `poll(Wait)`.
- **JS/Rust boundary.** Unchanged. No protocol type changes; `make wasm` and
  the frontend gates run because the engine crate changes.

## Design

### 4.1 The registration declares the pass; the ground is the write side

`crates/darkly/src/brush/node.rs`:

```rust
/// How a terminal's per-dab pass writes the stroke scratch. Declared on
/// the registration beside `scratch_format`: a per-type fact the stroke
/// buffer, the assembler and the terminal's pipeline all read from one
/// place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DabPass {
    /// One instanced draw per flush: every dab is a quad, the blend unit
    /// folds them under `dab_blend`, channels are colour attachments.
    #[default]
    InstancedDraw,
    /// One compute dispatch per dab per flush over the dab's clamped
    /// footprint, one thread per pixel, reading and writing the scratch
    /// (the ground) and every storage channel; the law is the terminal's
    /// body.
    DispatchPerDab,
}

impl DabPass {
    /// Usage the write side needs beyond attachment, copy and sampling.
    pub fn write_side_usage(self) -> wgpu::TextureUsages {
        match self {
            Self::InstancedDraw => wgpu::TextureUsages::empty(),
            Self::DispatchPerDab => wgpu::TextureUsages::STORAGE_BINDING,
        }
    }
}

/// Scratch format for a terminal that accumulates through a compute dab
/// pass: premultiplied RGBA8 packed into one `u32` per texel with
/// `pack4x8unorm`, because core WebGPU allows read-write storage only on
/// 32-bit single-channel formats.
pub const PACKED_GROUND_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;
```

`BrushNodeRegistration` gains `pub dab_pass: DabPass` (`:55-78`);
`BrushNodeRegistration::compute` (`:135-147`) sets `InstancedDraw`; the nine
terminal literals (`liquify.rs:138`, `smudge.rs:46`, `stamp.rs:26`,
`paint.rs:389`, `blur.rs:72`, `circle.rs:47`, `polygon.rs:35`,
`paint_dispatch_spike.rs:113`, `watercolor.rs:738`) each name theirs;
`paint::register` sets `dab_pass: DabPass::DispatchPerDab, scratch_format:
PACKED_GROUND_FORMAT`.

`crates/darkly/src/brush/eval.rs`: `runner.dab_pass()` beside
`scratch_format()` (`:818-827`), read by `painting.rs:1020-1031` and
`preview_renderer.rs:130-143` and passed to `StrokeBuffer::new`
(`stroke_buffer.rs:73-137`), which passes it to `Scratch::new`.

`crates/darkly/src/brush/pipeline.rs`:

- New `pub struct CanvasCopyLayout { pub bgl: wgpu::BindGroupLayout, pub
  sampler: Option<wgpu::Sampler>, pub texture_binding: u32 }` with
  `fn bind(&self, device, label, view) -> wgpu::BindGroup` building the
  texture entry at `texture_binding` and the sampler entry at 1 when present.
- Three instances built in `BrushPipelines::new`: the filterable float pair
  (`:340-360`, binding 0 + sampler 1), the non-filterable float pair
  (`:368-389`, binding 0 + sampler 1), and a new `canvas_copy_uint` with one
  entry, `binding 2`, `Texture { sample_type: Uint }`, `FRAGMENT`, no sampler.
  Binding 2 rather than 0 so `composite.wgsl` can declare the packed and the
  float foregrounds in one module (section 4.5).
- `canvas_copy_layout_for(format) -> &CanvasCopyLayout` (`:772-788`) gains
  the uint arm: `format.sample_type(None, None) ==
  Some(TextureSampleType::Uint)` (`TextureFormat::sample_type`,
  `wgpu-types-29.0.4/src/texture/format.rs:1070`) takes it; otherwise the
  existing filterable test picks between the two float layouts.
- `BuildContext` (`:120-145`) carries `&CanvasCopyLayout` in place of the
  `(bgl, sampler)` pair; its two construction sites
  (`read_mirror_terminal.rs:710`, `paint.rs:826`) follow.

`crates/darkly/src/brush/scratch.rs`:

- `Scratch::new(device, w, h, layout: &CanvasCopyLayout, format, pass:
  DabPass)` replaces the `(bgl, sampler)` pair; the struct stores a clone of
  the layout instead of `canvas_copy_bgl`, `read_mirror_sampler` and
  `write_sampler` (`:83-90`), and `canvas_copy_bind_group` (`:834-855`)
  becomes `layout.bind(...)`. The write-side nearest sampler (`:206-211`) is
  kept only for the float layouts; for the uint layout nothing samples.
- `create_write_texture` (`:780-805`) ORs in `pass.write_side_usage()`.
  The scratch remembers the pass so `grow_write` reallocates with the same
  usage.
- `grow_write` and `build_channels` take the layout through `self`.

Checkpoint ring, clear, grow: no code change (3.4).

### 4.2 The assembler: a compute stroke skeleton

`crates/darkly/src/brush/wgsl/context.rs`: `ShaderMode` (`:310-313`)
becomes `Stroke(DabPass)` and `CursorPreview`; the three `ShaderMode::Stroke`
sites in `assemble_shader` (`:948`, `:999`, `:1071`) match on it. The
`NodeWgsl::body` doc (`:28-37`) gains the `ground` binding (and the storage
channels by name) as something a body in a `DispatchPerDab` stroke may read
with `textureLoad`; this is the hook the paused
`docs/plans/live-canvas-sampler.md` rewrite needs (a sampler node reads the
live ground and emits a neutral preview body under the `clone_source` rule in
`docs/brush-preview-and-overlays.md`), so the port forecloses nothing there.

`crates/darkly/src/brush/wgsl/mod.rs`: `compile_brush_to_wgsl` takes the
terminal's `DabPass` from `compile_graph` (`brush/mod.rs:323-347`, which has
the registration in hand) and the scratch format, and `CompiledBrush` gains
`pub dab_pass: DabPass`. The `tests/wgsl.rs` call sites follow.

`CompiledBrush::storage_bindings(&self) -> Vec<StorageBinding { binding: u32,
name: &'static str, format: TextureFormat }>`: binding 2 is the write side,
named `ground`, in the registration's `scratch_format`; bindings 3.. are the
declared `ChannelUse::Storage` channels in declaration order, named by their
`StrokeChannel::name`. This one list drives both the WGSL declarations the
skeleton emits and the bind-group layout the terminal builds (4.4), so the
two cannot disagree. Attachment channels are rejected under
`DispatchPerDab` with a `CompileError::NodeNotCompilable` naming the channel:
a compute pass has no colour targets.

`assemble_shader` for `Stroke(DispatchPerDab)` emits, in place of the group-1
line, the vertex stage and the fragment header:

```wgsl
struct DabSlot { i: u32, pad0: u32, pad1: u32, pad2: u32 };
@group(1) @binding(0) var<storage, read> dabs: array<DabRecord>;
@group(1) @binding(1) var<uniform> slot: DabSlot;
@group(1) @binding(2) var ground: texture_storage_2d<r32uint, read_write>;
// one line per storage channel, binding 3..:
@group(1) @binding(3) var build: texture_storage_2d<r32uint, read_write>;

fn unpack_ground(texel: vec4<u32>) -> vec4f { return unpack4x8unorm(texel.r); }
fn pack_ground(c: vec4f) -> vec4<u32> { return vec4<u32>(pack4x8unorm(c), 0u, 0u, 0u); }
```

(groups 0, 2 and 3 as today: `:946`, `:949-950`, `:979-988`), then after the
node decls:

```wgsl
@compute @workgroup_size(8, 8, 1)
fn cs_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let d = dabs[slot.i];
    // The grid covers the dab's layer-clamped footprint from its corner:
    // the same floor-and-clamp `record_dab_footprint` applied on the CPU
    // (`Rect::clamp_f32`), on the same f32 inputs. Threads past the
    // footprint's rounding or off the layer return here.
    let origin = max(vec2<i32>(floor(d.pos - vec2f(d.bbox_target_px))), u.intrinsic.layer_offset);
    let canvas_px = origin + vec2<i32>(gid.xy);
    let layer_px = canvas_px - u.intrinsic.layer_offset;
    if (any(layer_px < vec2<i32>(0)) || any(layer_px >= vec2<i32>(u.intrinsic.layer_size))) {
        return;
    }
    // Pixel centre: the fragment skeleton evaluates at (i + 0.5, j + 0.5).
    let target_pos = vec2<f32>(canvas_px) + vec2<f32>(0.5);
    let local = target_pos - d.pos;
    let local_dist_px = length(local);
    if (local_dist_px >= d.bbox_target_px) {
        return;
    }
    let local_uv = local * d.inv_radius_target_px;
    let local_dist = length(local_uv);
    let theta = atan2(local_uv.y, local_uv.x) - u.intrinsic.view_rotation;
    let canvas_size = ...;      // as today, `:1059-1066`
    let canvas_origin = ...;
    let sel = textureSampleLevel(sel_tex, sel_smp, plane_to_selection_uv(target_pos, canvas_origin, canvas_size), 0.0).r;
    <shared body>
    <terminal body>
}
```

Everything from `local_uv` down is the existing header text with `discard`
replaced by `return`; the emitter shares the lines between the two skeletons
rather than duplicating them (one `fn push_fragment_locals(out, mode)`).
`@workgroup_size(8, 8, 1)` is emitted from a `pub const DAB_WORKGROUP: u32 =
8` in `wgsl/mod.rs` that paint's flush reads for its grid arithmetic (one
constant, both sides).

Why the origin derivation agrees with the CPU: `record_dab_footprint`
(`gpu_context.rs:395-413`) calls `canvas_extent().clamp_f32(position -
bbox_radius, ...)`, and `clamp_f32` (`src/coord.rs:183-191`) floors the near
edge and intersects the extent, whose near corner is the layer's plane
offset. `d.pos` and `d.bbox_target_px` are the same f32 values `queue_dab`
packed from `position` and `bbox_radius`; IEEE f32 subtraction and `floor`
are correctly rounded on the CPU and in WGSL, so the shader's `origin` is the
rect's corner, and the grid size the flush reads from the batch meta is the
same rect's width and height.

Why the set of pixels written is identical to the fragment path: the quad
covers `pos +- bbox` and the rasterizer shades the pixels whose centres lie
inside it; the compute grid covers `floor(pos - bbox) .. ceil(pos + bbox)`
clamped to the layer, and every thread applies the same
`local_dist_px >= d.bbox_target_px` reject on the same centre. A centre inside
the grid but outside the quad has `|centre - pos| > bbox` on one axis and so
is rejected; a centre inside the quad is inside the grid. Off-layer pixels are
clipped by the viewport today and by the `layer_px` test here.

The preview skeleton is untouched: `cursor_preview_wgsl` stays a fragment
module for every terminal (`:577-586`), so `render_compiled_cursor_preview`
(`:698-784`) and the `CursorPreviewPipelineCache` (`pipeline.rs:977-1212`)
are not touched by this port. The dab record is unchanged.

### 4.3 `paint.rs`

**Registration.** `dab_pass: DispatchPerDab`, `scratch_format:
PACKED_GROUND_FORMAT`; ports, `supports_erase`, lifecycle unchanged
(`:384-472`).

**`BUILD_CHANNEL`** (`:65-71`) becomes `kind: ChannelUse::Storage, format:
PACKED_GROUND_FORMAT`. Declared only strictly inside the dial, as today
(`:773-781`).

**`compile_wgsl`** (`:739-784`). No `dab_blend`. `wgsl.decls` carries
paint's law file (4.6). The body keeps `rgba`, `wash_flow`, `build_flow` as
today and ends in stores instead of a `return`:

```wgsl
// build-up alone (buildup = 1, the registration default):
    let src = rgba * build_flow * sel;
    textureStore(ground, layer_px, pack_ground(accumulate_build(src, unpack_ground(textureLoad(ground, layer_px)))));
// wash alone (buildup = 0):
    let src = rgba * wash_flow * sel;
    textureStore(ground, layer_px, pack_ground(accumulate_wash(src, unpack_ground(textureLoad(ground, layer_px)))));
// inside the dial: the wash share to `ground`, the build share to `build`.
    let wash_src = rgba * wash_flow * sel * {wash_share:.6};
    let build_src = rgba * build_flow * sel * {build_share:.6};
    textureStore(ground, layer_px, pack_ground(accumulate_wash(wash_src, unpack_ground(textureLoad(ground, layer_px)))));
    textureStore(build, layer_px, pack_ground(accumulate_build(build_src, unpack_ground(textureLoad(build, layer_px)))));
```

`compile_cursor_preview_body` (`:794-808`) is unchanged: it returns a
`vec4<f32>` into the fragment preview skeleton.

**`PerBrushPipeline`** (`:99-312`) builds a `wgpu::ComputePipeline` from
`compiled.stroke_wgsl` with entry `cs_main`; its group-1 layout has binding 0
the dabs storage buffer (`COMPUTE`), binding 1 a uniform with
`has_dynamic_offset: true`, `min_binding_size: 16`, and one
`StorageTexture { access: ReadWrite, format, view_dimension: D2 }` per entry
of `compiled.storage_bindings()`. The pipeline layout is groups 0, 1, 2 and
optionally 3 as today (`:174-198`); the graph-texture layout needs the
visibility change of 3.2. The uniform ring, dabs buffer and graph-texture
cache stay (`:234-301`). `color_targets` is no longer called by paint.

**`PaintPipeline`** (`:320-371`) owns the static dab index buffer: `256 *
MAX_DABS_PER_PHASE` bytes (4 MiB, `gpu_context.rs:149`), slot `i` holding
`i`, written once in `build` (the spike's `:311-323`). The stride is
WebGPU's default `min_uniform_buffer_offset_alignment`, as the spike argued
(`paint_dispatch_spike.rs:74-77`).

**`evaluate_gpu`** (`:487-531`): after `record_dab_footprint` returns the
rect (`:519-525`), push `[rect.width, rect.height]` onto
`dab_batch.meta_bytes` (the terminal-private per-dab meta the framework
already provides, `gpu_context.rs:246-254`; its doc, which today names only
per-dab-feedback terminals, gains a sentence), so the flush has each dab's
grid size without re-deriving the clamp. `queue_dab` is unchanged.

**`flush_dabs`** (`:533-679`): the prologue (take, pipeline, uniforms, live
group 3) keeps its shape. Then: build the group-1 bind group over the dabs
buffer, the index buffer (size 16 at offset 0) and
`stroke.scratch.write_view()` plus `channel_view(name)` for each storage
binding; `begin_compute_pass`; set pipeline, group 0 with the ring offset,
group 2, group 3; for each dab `set_bind_group(1, &g1, &[i * 256])` and
`dispatch_workgroups(ceil(w / DAB_WORKGROUP), ceil(h / DAB_WORKGROUP), 1)` from
the meta; `record_dispatches(total_dabs)` (`:678` records 1 today). The
group-1 bind group is rebuilt per flush because `grow_write` and
`ensure_channels` can reallocate the views; that is the spike's measured
shape (about ten per event at `stabilize = 1.0`). If `cpu p50` ever shows it,
`Scratch` can expose a reallocation generation to key a cache on; not in this
port.

**`commit`** (`:681-715`): unchanged logic, through the `CommitForegrounds`
value of 4.4 with `format: stroke.scratch.format()`.

**`render_cursor_preview`** (`:723-731`): unchanged.

Module doc (`:1-33`) rewritten for the compute pass; the `PerBrushPipeline`
doc's erase note (`:100-108`) is kept, because it still holds: the per-dab
pass never branches on erase.

### 4.4 The commit reads the packed ground

`crates/darkly/shaders/lib/deposit_ceiling.wgsl` (new, no entry point, so
`tests/shader_compile.rs:18-34` treats it as a preamble and validates it
through `composite.wgsl`, which calls it):

```wgsl
fn chebyshev(a: vec3f, b: vec3f) -> f32 { ... }      // moved from composite.wgsl:72-75

// The effective coverage the deposit ceiling grants `fg` (premultiplied)
// over `bg` (premultiplied): 0 when there is nothing to deposit or no room.
// The law: a pass carrying pigment C at coverage s lands a fixed fraction of
// the way to C from the gamut corner opposite C; a pixel already closer than
// that takes nothing. See docs/brush/architecture.md, "How the commit decides".
fn ceiling_t(fg: vec4f, bg: vec4f) -> f32 {
    if (fg.a <= 0.0) { return 0.0; }
    let pigment = fg.rgb / fg.a;
    let origin = select(vec3f(0.0), vec3f(1.0), pigment < vec3f(0.5));
    let reach = chebyshev(origin, pigment);
    let ground = bg.rgb + origin * (1.0 - bg.a);
    let d = chebyshev(ground, pigment);
    if (d <= 0.0) { return 0.0; }
    return max(0.0, 1.0 - (1.0 - fg.a) * reach / d);
}
```

`composite.wgsl`'s `deposit_through_ceiling` (`:94-123`) becomes

```wgsl
fn deposit_through_ceiling(fg: vec4f, bg: vec4f) -> vec4f {
    let t = ceiling_t(fg, vec4f(bg.rgb * bg.a, bg.a));
    if (t <= 0.0) { return bg; }
    return source_over(fg.rgb / fg.a * t, t, bg);
}
```

Same room arithmetic as today with the straight background premultiplied
once. One deliberate difference: today a saturated pixel (`t == 0`) still
goes through `source_over(pigment * 0, 0, bg)` (`:122`), which divides by
`bg.a` and zeroes rgb under `0.001` (`:146-149`); the early return makes
saturation an exact identity. `wash_caps_coverage_across_strokes` compares
alpha only, so no exact test moves; the change is an improvement and is
recorded as one in the architecture doc (step 8).

`composite.wgsl` gains a second fragment entry point over packed foregrounds:

```wgsl
@group(1) @binding(2) var t_wash_packed: texture_2d<u32>;
@group(2) @binding(2) var t_build_packed: texture_2d<u32>;

@fragment fn fs_packed(in: VertexOutput) -> @location(0) vec4f {
    let px = vec2<i32>(in.position.xy);
    let wash = unpack4x8unorm(textureLoad(t_wash_packed, px, 0).r) * u.wash_opacity;
    let build = unpack4x8unorm(textureLoad(t_build_packed, px, 0).r) * u.build_opacity;
    return commit_fragment(in, wash, build);
}
```

with the body of today's `fs_main` (`:129-157`) moved into
`fn commit_fragment(in: VertexOutput, wash: vec4f, build: vec4f) -> vec4f`
that both entries call; `fs_main` keeps its `textureSample` fetches
(`:134-135`). One module, two entry points: wgpu-core collects an entry
point's resources only for the globals it uses
(`wgpu-core-29.0.4/src/validation.rs:1068-1072`) and `check_stage` walks
that list (`:1161`), so the float layout (bindings 0 and 1) and the uint
layout (binding 2) each match their entry with the other's declarations
unused; the reviewer validated the two-entry module under naga.
`textureLoad` at the fragment's own pixel is exact: the scratch is
layer-sized and the commit's quad is the layer
(`paint_target_ext.rs:113-128`), which is also why the nearest-sampled float
path reads the same texel today.

`crates/darkly/src/brush/composite_pipeline.rs`: the shader source is
`source_over.wgsl + lib/deposit_ceiling.wgsl + brush/composite.wgsl`
(`:61-76`); two pipeline layouts (float foregrounds over the float
canvas-copy layout, packed foregrounds over the uint layout; group 3 stays
float); four pipelines; `pipeline(dest: TextureFormat, foreground:
TextureFormat)` picks by the two formats, keeping the format branch in one
place as the module doc asks (`:13-14`).

`crates/darkly/src/brush/paint_target_ext.rs`: `commit_brush_dab` takes

```rust
pub struct CommitForegrounds<'a> {
    /// Texel format of both slots: the scratch's.
    pub format: wgpu::TextureFormat,
    pub wash: Option<&'a wgpu::BindGroup>,
    pub build: Option<&'a wgpu::BindGroup>,
}
```

in place of the two `Option`s (`:52-62`, `:95-152`), and selects
`composite.pipeline(self.format(), fg.format)`. Callers: `paint.rs:705-714`,
`watercolor.rs` (its commit), and the spike until it is deleted. The
`wash.or(build)` borrow for an absent slot (`:106-111`) is unchanged.

### 4.5 The laws, as shader code owned by `paint`

`crates/darkly/shaders/brush/paint_accumulate.wgsl` (new; `paint::compile_wgsl`
emits it through `NodeWgsl::decls` as
`concat!(include_str!("lib/deposit_ceiling.wgsl"), include_str!("brush/paint_accumulate.wgsl"))`,
so it reaches only brushes whose terminal is `paint`; `pack_ground` and
`unpack_ground` come from the skeleton):

```wgsl
// Build-up: premultiplied source-over. Coverage accumulates as
// 1 - prod(1 - a_i); a pixel's density rises with every dab that lands.
fn accumulate_build(src: vec4f, dst: vec4f) -> vec4f {
    return src + dst * (1.0 - src.a);
}

// Wash: the deposit ceiling applied per dab against the live ground. For
// one pigment this is exactly the greatest coverage any dab laid on the
// pixel (docs/brush/architecture.md, "How dabs accumulate in the
// scratch"); for varying pigments it deposits whole colours instead of
// per-channel maxima.
fn accumulate_wash(src: vec4f, dst: vec4f) -> vec4f {
    let t = ceiling_t(src, dst);
    if (t <= 0.0) { return dst; }
    return vec4f(src.rgb / src.a * t, t) + dst * (1.0 - t);
}
```

`tests/shader_compile.rs` parses the file standalone (every `.wgsl` under
`shaders/` must parse on its own); it is validated in context by
`tests/wgsl_validate.rs` through every builtin paint brush, because no
entry-point file under `shaders/` calls `accumulate_*`.

#### 4.5.1 The per-dab ceiling reproduces `Max` for one pigment

Let the ground start transparent and every dab carry pigment `C` at coverage
`s_i` (premultiplied `(C s_i, s_i)`). Write `O` for the corner opposite `C`
and `R = cheb(O, C)`.

Dab 1 on `bg = 0`: `ground = O`, `d = R`, `t = s_1`, the result is
`(C s_1, s_1)`.

Dab 2 on `bg = (C a, a)` with `a = s_1`: `ground = C a + O (1 - a)`, so
`d = cheb(C a + O (1 - a) - C) = (1 - a) cheb(O - C) = (1 - a) R`, and

```
t = max(0, 1 - (1 - s_2) R / ((1 - a) R)) = max(0, (s_2 - a) / (1 - a)).
```

If `s_2 <= a`, `t = 0` and the pixel is unchanged: alpha stays
`a = max(s_1, s_2)`. If `s_2 > a`: the new alpha is
`t + a (1 - t) = t (1 - a) + a = (s_2 - a) + a = s_2`, and the new
premultiplied colour is `C t + C a (1 - t) = C (t + a - a t) = C s_2`. So
the result is `(C max(s_1, s_2), max(s_1, s_2))` in both cases, and by
induction after `n` dabs it is `(C max_i s_i, max_i s_i)`: the `Max` blend.

**8-bit exactness** (what `brush_accumulation.rs` asserts). The stored alpha
after dab 1 is `a' = round(s_1 * 255) / 255`. A later identical dab (`s = s_1`)
sees `d = (1 - a') R` when the stored premultiplied colour is exactly `C a'`.
If `a' >= s`, `t = 0` and nothing is written. If `a' < s`,
`t = (s - a') / (1 - a')` and the new alpha is exactly `s` in float, which
packs back to `round(s * 255) / 255 = a'`. The stored value is therefore
idempotent under repeated identical dabs after the first, which is the
`Max`-of-equal-dabs property the spacing-independence (`:187-208`) and
double-back (`:221-250`) tests assert exactly, and a dab stronger than the
stored value re-targets the stored alpha to its own `round(s * 255)`, which
is what `wash_still_responds_to_pressure` (`:261-288`) and
`a_heavier_pass_still_darkens_through_the_ceiling` (`:564-583`, the commit
half) expect.

This is exact for black, which is what the exact tests paint (`:124-126`):
`C = 0`, the stored premultiplied colour is exactly `0`, and `d` is exactly
`(1 - a')`. For a coloured pigment the stored colour is
`round(C_i a' 255) / 255` per channel, which perturbs `d` by up to about
`1/255`, so `t` can come out slightly positive on a pixel the ideal law
leaves alone and move alpha by about one count, once (after that `s - a'`
outweighs the perturbation; `R >= 0.5` bounds it). The coloured wash test
`wash_saturates_per_pigment_not_globally` (`:412-460`) asserts inequalities
and is unaffected; test 7 below allows 2 LSB for the same reason.

#### 4.5.2 Build-up parity

`accumulate_build` is the blend unit's `One, OneMinusSrcAlpha` arithmetic on
the unpacked value. The spike measured the two paths at 4 LSB maximum in
premultiplied space with 97% of painted pixels within 1 LSB (1002 exact and
8291 at exactly 1 LSB of 9559; `tests/paint_dispatch_spike.rs:121-130`,
perf doc "#5, stage 2", Parity, whose "87%" is a slip corrected in step 8).
The rounding differs because `pack4x8unorm` rounds the shader's float and
the ROP rounds its own result once per dab; both are round-to-nearest, and
over a stack of about ten dabs per pixel the difference drifts by a few LSB.
The ported terminal and the spike run the same pack, unpack and source-over
per dab on the same disc coverage, so between those two the expected gap is
0 to 1 LSB (Tests, 1).

#### 4.5.3 Inside the dial: two accumulations stay

The commit takes two foregrounds and lays the wash slot through the ceiling
before compositing the build slot over it (`composite.wgsl:150-157`), and
architecture.md records why the order is load-bearing ("a build half laid
underneath would let a stroke's own build-up shrink its own wash"). A single
ground mixing both halves cannot be split at commit, no read-write storage
format holds two RGBA8 accumulations in one texel (`Rg32Uint` is `s_ro_wo`,
`format.rs`), and a per-dab ceiling reading a ground that already holds build
deposits changes the mid-dial within-stroke result that
`both_overlap_sites_build_alike_at_every_dial_setting`
(`brush_accumulation.rs:682-691`) and the Pencil (`brushes/pencil.yaml:130`,
`buildup: 0.1`) depend on. So inside the dial the port keeps two
accumulators: the ground takes the wash share under `accumulate_wash`, the
`build` storage channel takes the build share under `accumulate_build`, and
the commit is exactly today's. The handoff's "the wash and build split goes
away" is true of the mechanism (two blend states, two colour targets, `FsOut`)
and false of the two accumulations, which the commit's two laws require.
Recorded as a deviation from the handoff under Open questions; the reviewer
concurred.

Two rejected single-ground variants, for the record. (a) Apply the ceiling
per dab against the live appearance (`ground OVER pre_stroke`) and commit
with plain source-over: for one pigment at `buildup = 0` this reproduces
today's commit exactly (the same induction with `d_pre` in place of `R`),
but stroke opacity breaks: today's commit applies `CEIL(S * opacity, pre)`
and the per-dab form would have to fold opacity into each dab's coverage,
which is `Max`-homogeneous but not source-over-homogeneous, so build-up
opacity would change. (b) A single ground with both laws: a semantics change
to every mid-dial brush, needing its own plan.

### 4.6 Selection and erase

**Selection.** The compute skeleton samples `sel` exactly as the fragment
one does (4.2), from the window-anchored mask through
`plane_to_selection_uv(target_pos, canvas_origin, canvas_size)`
(`shaders/lib/canvas.wgsl:17-19`), with `target_pos` a plane position and the
selection layout already `COMPUTE`-visible (3.2). Every law multiplies by
`sel` inside the dab (4.3), which is what keeps feathering a multiplier
under the ceiling (architecture.md, "Canvas-space fields survive").

**Erase.** Unchanged: the per-dab pass never branches on it (the regression
recorded at `paint.rs:100-108`), and the commit routes `gpu.blend_mode` to
`destination_out` on both slots (`composite.wgsl:137-143`). The packed entry
calls the same `commit_fragment`. `docs/plans/graph-owned-erase.md`
(untracked) moves the erase decision onto the graph but keeps it commit-time,
so the two plans are independent; if it lands first, `CommitForegrounds` is
built with its `erase` value instead of `gpu.blend_mode`.

### 4.7 Hover preview, stroke preview, thumbnails

- **Hover preview.** `paint::render_cursor_preview` ->
  `render_compiled_cursor_preview` -> `BrushPipelines::render_preview` builds a
  fragment pipeline from `cursor_preview_wgsl` into the `Rgba8Unorm` preview
  mask (`pipeline.rs:845-876`, `:1066-1143`). `cursor_preview_wgsl` stays a
  fragment module (4.2) and the preview body is unchanged. `setOverlay` and
  the frontend are untouched. The brush-preview doc's "two shader variants"
  paragraph is updated to say the stroke variant is a compute module for a
  `DispatchPerDab` terminal.
- **Stroke and dab thumbnails** (`preview_renderer.rs`), node previews
  (`node_preview_subgraph.rs`) and README graphics (`docs_render`) run the
  real stroke path through a `StrokeBuffer` in the terminal's
  `scratch_format` and commit through `commit_brush_dab`: they read the
  packed ground through `fs_packed` with no code change beyond 4.4. This is
  the "stroke preview reads packed `r32uint` directly" of the handoff.

### 4.8 The large-dab regime

Measured on the Intel Raptor Lake-P iGPU only: at 4K with dabs of 1000 px
and more the spike is 0.7 s and 2.7 s behind over the 3.5 s stroke, of which
about 1 s is the thread-per-pixel shape's own cost (about 2x the blend unit
per pixel on a four-megapixel dab), the rest the unpack and the doubled
checkpoint copies the port removes (perf doc "#5, stage 2", "Where the
fragment path wins", and `bench-results/dispatch-cost-bench-746570670c.md`:
1.0 ms GPU per 1000 px dab for shape (a) against 0.48 ms for (c), 0.75 ms for
the read-only (b)).

**Size the knob on the machine we have.** `dispatch_cost_bench` is
terminal-independent. Step 0 adds two bench-only variants so the mitigations
are sized in one run: a `cs_rw_16` entry at `@workgroup_size(16, 16, 1)`
(exactly `max_compute_invocations_per_workgroup`) and a `cs_rw_rows` entry
where each thread walks four rows (`@workgroup_size(8, 2, 1)`, grid height
divided by 8), both on the 1.5 px and 1000 px cells. About 60 lines in
`dispatch_cost_bench.rs`, no shipped code. `DAB_WORKGROUP` is fixed from
that run on the Intel machine.

**The discrete-GPU session is a follow-up, not a gate.** Nothing in this
repo has run on a discrete GPU, and the plan cannot schedule one. When a
machine exists, the session is: the harness (with the variants) and the
matrix, `paint` twice, against the ported terminal, which is a better
vehicle than the spike (no unpack, no doubled checkpoints). Its result can
move `DAB_WORKGROUP` or adopt the row loop; neither needs the spike in the
tree.

**Candidates, with cost:**

| mitigation | production cost | what it buys | decision |
| --- | --- | --- | --- |
| larger workgroup (16x16) | 1 constant | better occupancy on wide GPUs; measured, not assumed | ship whichever of 8x8 / 16x16 the Intel harness favours; `DAB_WORKGROUP` is the one knob |
| per-thread row loop | about 20 lines in the skeleton (loop over `y` inside the thread, same rejects) | fewer threads, amortised record load; may lose on small dabs | adopt only if the harness shows it wins at 1000 px without losing at 1.5 px; otherwise defer |
| `rgba8unorm` read-write | format constant + the two pack helpers + a feature gate | drops pack/unpack: the (a) minus (b) share, about a quarter of the gap | deferred: unreachable on the `webgpu` backend in wgpu 29 (3.3) |
| size-gated fragment path for huge dabs | the whole instanced terminal kept alive beside the compute one | fixed-function blending where it wins | rejected: two terminals is what this port exists to remove; revisit only if a discrete-GPU run shows the regime is a real-world problem rather than an iGPU-at-4K corner |

**Commitment.** The port ships thread-per-pixel with the measured workgroup
shape. The gate (4.10) treats the 4K cells at `r >= 1000` as recorded, not
pass/fail; if the after-port matrix on the Intel machine shows them worse
than the spike's recorded gap (the port removes about 1.7 s of artefacts
there, so they should improve), the row loop is applied and re-measured
before the port is called done, and the fragment fallback is still not built.

### 4.9 The spike

Deleted in step 10, after the gate: node file, shader, fixture, test, bench
topology, and the `nodes/mod.rs` line `build.rs` regenerates. Two of its
tests move into `tests/paint_compute.rs` in the forms listed under Tests;
the third (parity) is the gate itself and goes with the spike. What stays:
`dispatch_cost_bench.rs` (the harness that answers the large-dab question
and any future "what does one dispatch cost here"), `ChannelUse`,
`Scratch::channel_view`, `BrushPerfCounters::dispatches` and the bench's
`flushes/ev` / `dispatches/ev` columns (all declared permanent by the
handoff), and the bench-result files. The perf doc's attempt #5 text is
history and stays; the port is recorded as attempt #6.

Argument for delete rather than keep: the spike's only purpose was to
measure a shape beside `paint`; once `paint` is that shape, the spike is a
second copy of the same terminal with a hand-written disc, and a registered
node that shows in the node editor. Its one remaining job, the live parity
reference for the port (Tests, 1), is done by step 10.

### 4.10 Bench and gate

Named commands, in order, same machine, same session, nothing else running:

1. **Baseline, before any production change** (the Intel machine, the only
   one with history): `paint` on the matrix, twice:

```bash
cargo run --release -p darkly --features testing --bin stroke_replay_matrix -- \
    --input crates/darkly/tests/fixtures/recorded_curvy_stroke.json --topology paint
cargo run --release -p darkly --features testing --bin stroke_replay_matrix -- \
    --input crates/darkly/tests/fixtures/recorded_curvy_stroke.json --topology paint --output <second run>
```

   The `746570670c` `paint` and `paint-rerun` files are the same cells on
   the same machine and are the reference if the baseline run agrees with
   them within the noise floor. Then the harness with the variants:

```bash
cargo run --release -p darkly --features testing --bin dispatch_cost_bench
```

2. **After the port:** `paint` twice. Pass: every cell at `radius <= 500`
   within +20 ms of the baseline's `behind` (the bench's noise band,
   perf doc "#5, stage 2"), with `dispatches/ev == dabs/ev`; the 4K cells at
   `radius >= 1000` recorded against the baseline and the spike's run and
   judged by 4.8.
3. **Parity and existing nets:**

```bash
cargo test -p darkly --test paint_dispatch_spike --features testing -- --test-threads=1
cargo test -p darkly --test brush_accumulation --features testing -- --test-threads=1
cargo test -p darkly --test paint_compute --features testing -- --test-threads=1
cargo test -p darkly --test wgsl --features testing -- --test-threads=1
cargo test -p darkly --test wgsl_validate --features testing -- --test-threads=1
cargo test --workspace --exclude darkly-wasm --features darkly/testing -- --test-threads=1
cargo test -p darkly --test protocol --features testing,ts-export -- --test-threads=1
```

4. The full `Lint / CI Checks` list from `CLAUDE.md`, including `make wasm`
   and the frontend and desktop gates (the engine crate changed), and a
   browser smoke: `cd frontend && npm run dev`, paint with the Ink Pen, the
   Pencil (mid-dial, baked noise) and Charcoal (image texture at group 3),
   erase, with a selection, across a layer edge, hover each.

## Coordinate frames

Per `docs/coordinate-systems.md`. Every value the compute skeleton and the
flush touch:

| value | frame | produced by | consumed as |
| --- | --- | --- | --- |
| `d.pos`, `d.bbox_target_px` | plane px (target = plane in stroke mode) | `queue_dab` from `pen_input.position` and the composed extent | `local = target_pos - d.pos`; `origin = max(floor(pos - bbox), layer_offset)` |
| footprint size (meta) | plane px, integer | `record_dab_footprint` -> `clamp_f32` (floor near, ceil far, intersect the paint target's `canvas_extent`); the rect's `width`, `height` | dispatch grid `ceil(size / 8)` |
| `u.intrinsic.layer_offset`, `layer_size` | plane rect of the paint target | `paint::flush_dabs` via `intrinsic_header` | the origin clamp; `layer_px = canvas_px - layer_offset`, the storage texel; bounds test |
| `target_pos` | plane px, pixel centre | skeleton: `canvas_px + 0.5` | every node body, `sel` |
| selection | window-local UV | `plane_to_selection_uv(target_pos, canvas_origin, canvas_size)` | `sel` |
| commit `px` | layer-local texel | `in.position.xy` of the layer-sized quad | `textureLoad(t_*_packed, px, 0)` |

Nothing here is authored in reference pixels; `dpi_factor` enters only
through `effective_radius` as today. Tests run once at `canvas_origin (0, 0)`
and once with a cropped canvas and an offset layer (Tests, 5).

## Implementation steps

No production code changes before step 5 of the workflow. Order for that
step; each line names the files it touches.

0. **Measure.** The baseline `paint` matrix on the Intel machine and the
   harness with the two new variants (`bin/dispatch_cost_bench.rs`). Record
   both in the perf doc under "#6: the port, measurements before" and fix
   `DAB_WORKGROUP` from the result. Nothing shipped changes.
1. **Pass declaration, ground format and layouts.** `node.rs` (`DabPass`,
   `write_side_usage`, `PACKED_GROUND_FORMAT`, the registration field,
   `compute()`); the nine terminal registrations (`InstancedDraw` on all but
   paint; paint stays `InstancedDraw` and `Rgba8Unorm` until step 5);
   `eval.rs` (`runner.dab_pass()`); `pipeline.rs` (`CanvasCopyLayout`, the
   uint layout, `canvas_copy_layout_for`, `BuildContext`);
   `read_mirror_terminal.rs:710` and `paint.rs:826` (`BuildContext` sites);
   `scratch.rs` (layout value, usage from the pass, `bind`);
   `stroke_buffer.rs`, `painting.rs`, `preview_renderer.rs` (route the
   pass). `cargo check --workspace`; full suite: no shipped brush's output
   changes.
2. **The commit.** `shaders/lib/deposit_ceiling.wgsl`; `composite.wgsl`
   (`commit_fragment`, `fs_packed`, packed bindings at 2);
   `composite_pipeline.rs` (four pipelines, `pipeline(dest, fg)`);
   `paint_target_ext.rs` (`CommitForegrounds`); `paint.rs:705-714`,
   `watercolor.rs` and the spike's commit calls. `tests/shader_compile.rs`
   and `tests/paint_target.rs` run.
3. **Assembler.** `wgsl/context.rs` (`ShaderMode::Stroke(DabPass)`, the
   `body` doc); `wgsl/mod.rs` (`compile_brush_to_wgsl` takes the pass and
   format, `CompiledBrush::dab_pass`, `storage_bindings`, `DAB_WORKGROUP`,
   the compute skeleton with the pack helpers, shared fragment-locals
   emitter, attachment-channel rejection under `DispatchPerDab`);
   `brush/mod.rs:323-347` (`compile_graph` passes them);
   `texture_registry.rs:180,187` (`FRAGMENT | COMPUTE`); `tests/wgsl.rs`
   call sites. No terminal selects the new pass yet; `tests/wgsl.rs` and
   `wgsl_validate.rs` pass unchanged in substance.
4. **Paint's law file.** `shaders/brush/paint_accumulate.wgsl`;
   `tests/shader_compile.rs` parses it. Not yet referenced.
5. **`paint.rs`.** Registration pass and format, `BUILD_CHANNEL` storage,
   `compile_wgsl` bodies and decls, `PerBrushPipeline` compute pipeline and
   group-1 layout, `PaintPipeline` index buffer, `evaluate_gpu` meta,
   `flush_dabs` dispatch loop, module docs; `gpu_context.rs:246-254`
   (`meta_bytes` doc). `node.rs`: delete `COVERAGE_CEILING` and its doc.
   `tests/wgsl.rs`: the three paint-shape tests rewritten (Tests, 8) and
   `rough_ink_brush_compiles_to_nonempty_wgsl` (`:56-57`) asserts
   `@compute` / `fn cs_main`. `tests/paint_dispatch_spike.rs::spike_matches_paint_within_tolerance`
   doc rewritten for what it now compares (Tests, 1).
6. **Tests** (`tests/paint_compute.rs`, `scratch.rs` unit tests,
   `tests/wgsl.rs` additions). Run the gate list of 4.10, item 3.
7. **Bench after.** 4.10 item 2. If the `r >= 1000` cells fail 4.8's
   decision, apply the row loop in the skeleton and re-run.
8. **Docs.** `docs/brush/architecture.md` ("How dabs accumulate in the
   scratch", "The accumulation dial", "How the commit decides" for the
   saturation identity, the `paint` terminal entry);
   `docs/brush-preview-and-overlays.md` ("Two shader variants");
   `docs/gpu-passes.md:135-140`; `docs/paint-compute-perf-tracking.md`
   (attempt #6, synthesis table row, cost-model row, section F status, the
   87% slip); `docs/architecture-history.md` (one paragraph: the four
   fragment attempts and why the compute terminal replaced #4).
   `cargo sync-docs` (no region lists brush nodes; the run confirms).
9. **Browser smoke** (4.10 item 4) and `make wasm`, frontend and desktop
   gates.
10. **Delete the spike** (4.9): node, shader, fixture, test, bench
    topology, help text; `nodes/mod.rs` regenerates. Full suite again.
11. **Lint / CI Checks** from `CLAUDE.md`, every line.

## Tests

All GPU tests under `--features darkly/testing -- --test-threads=1`. This
is a port with a parity gate, not a bug fix, so the tests below are feature
and parity tests; the regression net is the existing suites named at the
end, which the port must leave green.

Parity gate (exists today, reused):

1. **`spike_matches_paint_within_tolerance`**
   (`tests/paint_dispatch_spike.rs:132-191`), unchanged in mechanism: the
   builtin Ink Pen against the spike fixture on the 48-sample reversing
   stroke at 256x128, compared in premultiplied space. Before step 5 it
   compares the fragment `paint` against the compute spike and records the
   known gap (4 LSB max; 1002 exact, 8291 at 1 LSB, 264 at 2, 2 at 3 of
   9559). After step 5 both sides are compute with the same pack, unpack and
   source-over per dab on the same disc coverage (the spike plan's review
   item 6), so the expected gap is 0 to 1 LSB. The test keeps its bound (4
   LSB, no more than 5% beyond 1 LSB) as the hard gate; the after-port
   histogram is recorded in the perf doc, and if it is not within 1 LSB
   everywhere the difference is explained before step 10 deletes the test
   with the spike. The spike's own distance from the fragment path is the
   recorded bridge to today's pixels. Wash and mid-dial are pinned exactly
   by `brush_accumulation.rs` (3.7), on whatever device runs the suite.

New file `crates/darkly/tests/paint_compute.rs`:

2. **`replay_is_deterministic_through_checkpoints_and_paints`** (moved
   from `paint_dispatch_spike.rs:194-224`): the recorded curvy stroke at
   `stabilize = 1.0` through the Ink Pen, two fresh engines byte-identical,
   layer non-empty. The `r32uint` write side through the ring's rewinds.
3. **`dispatches_count_one_per_dab`** (from `:229-254`): after a stroke,
   `dispatches == flushed_dabs` for the Ink Pen.
4. **`selection_masks_the_dab_pass_under_a_cropped_canvas`:** a
   rectangular selection over the right half, a stroke across the whole
   width, pixels outside the selection untouched and inside painted; run at
   `canvas_origin (0, 0)` and after a crop that gives a non-zero origin with
   an offset layer, the way `tests/canvas_resize.rs::marquee_selection_masks_same_plane_pixels_after_crop`
   sets it up.
5. **`stroke_that_grows_the_layer_keeps_its_ground`:** start on the layer,
   stroke past its edge so `ensure_layer_covers_dab` grows it mid-stroke;
   the pixels painted before the grow are present afterwards and the stroke
   is continuous across the old edge (the `STORAGE_BINDING` write side
   through `grow_write`).
6. **`wash_with_two_pigments_deposits_both`** (the lifted restriction):
   two dabs of different pigments at one pixel under `Wash`; the result's
   alpha equals the second dab's coverage within 2 LSB (4.5.1, coloured
   pigments) and both pigments' dominant channels are present. Today's `Max`
   takes per-channel maxima; this pins the new behaviour.

Elsewhere:

7. `crates/darkly/src/brush/scratch.rs` unit tests:
   `dispatch_per_dab_write_side_has_storage_usage_and_a_uint_bind_group` (a
   `Scratch` in `R32Uint` under `DispatchPerDab` reports `STORAGE_BINDING`,
   `clear_to_transparent` and `grow_write` validate on the test device and
   the usage survives the grow; an `InstancedDraw` scratch has no
   `STORAGE_BINDING`).
8. `crates/darkly/tests/wgsl.rs`: the three tests that assert paint's
   fragment shape are rewritten for the compute shape.
   `paint_terminal_compiles_the_accumulation_dial_to_targets_and_a_body`
   (`:1838-1918`) asserts `dab_pass == DispatchPerDab`,
   `storage_bindings()` is `[ground@2]` for the default and wash brushes and
   `[ground@2, build@3]` mid-dial (the channel compared by name, kind and
   format, since `BUILD_CHANNEL` is private), the `textureStore(ground, ...)`
   and `textureStore(build, ...)` lines with the `0.500000` / `0.750000` /
   `0.250000` shares, no `FsOut` anywhere, and `cursor_preview_wgsl` still a
   `vec4<f32>` fragment. `each_flow_scales_its_own_half` (`:1925-1983`) and
   `authored_buildup_picks_the_accumulation_shape` (`:2065-2121`) assert the
   store lines in place of `dab_blend`.
   `rough_ink_brush_compiles_to_nonempty_wgsl` asserts `@compute` and
   `fn cs_main` in `stroke_wgsl` and `@fragment` / `fn fs_main` in
   `cursor_preview_wgsl`. New
   `watercolor_keeps_the_fragment_stroke_skeleton` asserts the Smooth
   Watercolor's `stroke_wgsl` still contains `@fragment` and `FsOut`; new
   `attachment_channel_under_dispatch_per_dab_is_rejected` (a test
   evaluator declaring an `Attachment` channel compiled with `DispatchPerDab`
   fails with a message naming the channel).
9. `tests/wgsl_validate.rs` needs no edits and validates the compute
   modules of every builtin paint brush, which is where
   `paint_accumulate.wgsl` is checked in context; `tests/shader_compile.rs`
   parses the two new files standalone.

Regression net, unchanged: `tests/brush_accumulation.rs` (every test, the
exactness requirements of 3.7), `tests/brush_erase.rs`, `tests/paint_basic.rs`,
`tests/preview_paint.rs` and `tests/cursor_preview_coverage_scale.rs` (hover
preview), `tests/builtin_brushes_stroke.rs::every_builtin_deposits_or_moves_pixels`,
`tests/brush_preserves_pixels_outside_dab.rs`, `tests/dab_footprint_ledger.rs`,
`tests/stroke.rs` (undo through the committed layer), `tests/stroke_replay.rs`,
`tests/clone.rs` (group 3 live slot beside the ground), `tests/watercolor.rs`,
`tests/smudge.rs`, `tests/blur.rs`, `tests/liquify.rs` (the fragment skeleton
survives), `tests/brush_editor_preview.rs` and `tests/picker_preview.rs`
(thumbnails through `fs_packed`), `tests/docs_render.rs`.

## Risks

- **The browser.** Every production pixel goes through the `webgpu` backend,
  where per-`setBindGroup` and per-`dispatchWorkgroups` cost is unmeasured
  (the spike plan's first risk). Two calls per dab at a thousand dabs per
  event is the shape; the smoke in step 9 is qualitative. A browser replay
  harness is the follow-up that makes it quantitative (Open questions).
- **Tint and `canvas.wgsl`.** If Tint rejects the unreachable `textureSample`
  in a compute module (3.1), split `canvas.wgsl`; no semantics change.
- **`SITE_PARITY_TOL`.** The mid-dial parity tests allow 2 counts and
  measure 0 / 1 / 0 / 0 / 0 today; the per-dab wash adds at most one
  rounding against `Max` for black and one count of colour-rounding
  perturbation for coloured pigments (4.5.1). The fixtures paint black. If a
  gap reaches 3, investigate the arithmetic before widening the tolerance.
- **Two storage textures inside the dial.** Mid-dial brushes (the Pencil)
  pay two loads and two stores per thread. The Pencil's dabs are small;
  the matrix has no mid-dial topology. If it matters, a `pencil` topology
  is a 15-line bench addition.
- **Bind group per flush.** About ten `create_bind_group` per event at
  `stabilize = 1.0`, as the spike paid; keyed caching on a `Scratch`
  reallocation generation is the fallback.
- **Uint attachment clear in the browser.** Native wgpu accepts
  `Color::TRANSPARENT` on an `r32uint` attachment (the spike's channel);
  Dawn converts clear values per format as well. If it does not, the
  fallback is a tiny clear dispatch over the ground inside
  `clear_to_transparent`, still framework-owned.
- **Workgroup shape chosen on one machine.** `DAB_WORKGROUP` is one
  constant; the matrix and the discrete-GPU follow-up are the way to revisit
  it.

## Open questions

1. **Discrete GPU.** A follow-up session (4.8) whenever a machine exists;
   it does not gate the port. Who has one?
2. **Two accumulations inside the dial** (4.5.3) against the handoff's
   "the wash and build split goes away". The plan keeps two accumulators
   because the commit's two-slot law and the Pencil require them; the
   mechanism (blend states, colour targets, `FsOut`) is what goes. The
   reviewer concurred. Confirm.
3. **`rgba8unorm` read-write.** Not reachable on the web in wgpu 29 (3.3).
   Revisit when wgpu exposes a WebGPU texture-format tier feature.
4. **A browser replay harness.** `engine/process_recording.rs` replays a
   recording inside the engine; a wasm entry that runs it with timing would
   turn the browser smoke into a number. Follow-up, not this port.

## LOC estimate

Lines added / removed, itemised per step. Production is net negative because
the spike (681 lines) and the `Max` blend path go.

| step | file | added | removed |
| --- | ---: | ---: | ---: |
| 1 | `brush/node.rs` (`DabPass`, `write_side_usage`, `PACKED_GROUND_FORMAT`, registration field, `compute()`) | 43 | 0 |
| 1 | nine terminal registrations (one line each) | 9 | 0 |
| 1 | `brush/eval.rs` (`runner.dab_pass()`) | 10 | 0 |
| 1 | `brush/pipeline.rs` (`CanvasCopyLayout`, uint layout, `canvas_copy_layout_for`, `BuildContext`) | 64 | 19 |
| 1 | `read_mirror_terminal.rs:710`, `paint.rs:826` (`BuildContext` sites) | 4 | 4 |
| 1 | `brush/scratch.rs` (layout value, usage from the pass, `bind`, docs) | 50 | 20 |
| 1 | `brush/stroke_buffer.rs`, `engine/painting.rs`, `brush/preview_renderer.rs` (route the pass) | 12 | 6 |
| 2 | `shaders/lib/deposit_ceiling.wgsl` (new) | 45 | 0 |
| 2 | `shaders/brush/composite.wgsl` (`commit_fragment`, `fs_packed`, moved ceiling) | 45 | 50 |
| 2 | `brush/composite_pipeline.rs` (four pipelines, source concat) | 45 | 15 |
| 2 | `brush/paint_target_ext.rs` (`CommitForegrounds`, docs) | 25 | 15 |
| 2 | `nodes/watercolor.rs`, `nodes/paint.rs`, spike commit calls | 10 | 8 |
| 3 | `brush/wgsl/context.rs` (`ShaderMode::Stroke(DabPass)`, `body` doc) | 15 | 3 |
| 3 | `brush/wgsl/mod.rs` (compute skeleton with pack helpers, `storage_bindings`, shared locals, rejection, signature, docs) | 110 | 20 |
| 3 | `brush/mod.rs` (`compile_graph` passes pass and format) | 4 | 2 |
| 3 | `gpu/texture_registry.rs` | 2 | 2 |
| 4 | `shaders/brush/paint_accumulate.wgsl` (new) | 30 | 0 |
| 5 | `nodes/paint.rs` (compile bodies and decls, compute pipeline, index buffer, flush loop, meta, docs) | 250 | 170 |
| 5 | `brush/node.rs` (delete `COVERAGE_CEILING` and docs) | 0 | 35 |
| 5 | `brush/gpu_context.rs` (`meta_bytes` doc), `brush/eval.rs` trait docs | 8 | 7 |
| 10 | `nodes/paint_dispatch_spike.rs` | 0 | 571 |
| 10 | `shaders/brush/paint_dispatch_spike.wgsl` | 0 | 110 |
| 10 | `bin/stroke_replay_matrix.rs` (topology, fixture, help) | 0 | 45 |
| 10 | `nodes/mod.rs` (generated) | 0 | 3 |
| **production** | | **~780** | **~1105** |
| 0 | `bin/dispatch_cost_bench.rs` (two bench-only variants, cells) | 60 | 0 |
| **bench tooling** | | **~60** | **0** |
| 5 | `tests/paint_dispatch_spike.rs` (parity test doc) | 10 | 5 |
| 5, 6 | `tests/wgsl.rs` (three tests rewritten, two asserts, two new tests, call sites) | 75 | 60 |
| 6 | `tests/paint_compute.rs` (tests 2 to 6) | 230 | 0 |
| 6 | `brush/scratch.rs` unit tests | 40 | 0 |
| 10 | `tests/paint_dispatch_spike.rs`, `tests/fixtures/ink_pen_dispatch_spike.yaml` | 0 | 294 |
| **tests** | | **~355** | **~360** |
| 8 | `docs/brush/architecture.md` | 60 | 40 |
| 8 | `docs/brush-preview-and-overlays.md`, `docs/gpu-passes.md` | 12 | 8 |
| 8 | `docs/paint-compute-perf-tracking.md` (attempt #6, tables, section F, the 87% slip) | 90 | 6 |
| 8 | `docs/architecture-history.md` | 12 | 0 |
| 0, 7 | `bench-results/*` (generated) | 120 | 0 |
| **docs / generated** | | **~295** | **~55** |

The two largest production items are `paint.rs` (about +250 / -170, a
rewrite of the pipeline and flush around the same evaluator shape) and the
assembler (+110). Steps 1 to 4 touch shipped code paths but change no
shipped brush's output (the full suite is run after each); step 5 is the
one behaviour-bearing change and is gated by the live parity test, the exact
accumulation suite and the matrix.
