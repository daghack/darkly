## Independent Review

Reviewed against the working tree at `ce9622df`, the vendored
`wgpu-core-29.0.4` / `wgpu-types-29.0.4` sources, and the `krita/` checkout.
Every file and line reference in the plan was checked; the ones that are off
are listed under "Reference corrections". The diagnosis is sound, the
architecture is mostly right, and the size is honest for the design as
written. The findings below are what should change before implementation.

### Verified claims

- **The race, and why a pre-dab dispatch is the minimum.** A sampler
  reading the ground at `t - m` inside the dab's own dispatch races the
  thread writing `t - m` whenever `|m|` is smaller than the footprint, and
  WebGPU orders nothing inside a dispatch. I looked for a cheaper escape and
  found none that is correct: sampling k dabs back needs `|k m| > 2r` (about
  33 dabs at spacing 0.03) and changes the medium; writing the appearance
  from the dab's own dispatch into a mirror the same dispatch reads at an
  offset is the same race on the mirror; ping-ponging the grounds doubles
  stores, breaks the store-only-where-changed law and the checkpoint ring's
  one-texture contract; a `copy_texture_to_texture` cannot sit inside a
  compute pass, so it reintroduces a pass boundary per dab. A dispatch that
  only writes the mirror followed by a dispatch that only reads it is the
  smallest ordered shape. The plan's rejection is sound and should say the
  above in one sentence so the next reader does not re-derive it.
- **A render pass per dab for the snapshot is correctly rejected.** The
  commit composite (`composite.wgsl`, `fs_packed`) does compute the
  appearance over a sub-rect, but `docs/paint-compute-perf-tracking.md`
  "#5, stage 1" measured a single-instance render pass at about 27 us wall
  against about 1.5 us per dispatch; at the 1080p radius-1 row (491 dabs
  per event, attempt #6 table) that is 13 ms against 0.7 ms per event. The
  composite shader's background mapping also assumes the background's
  origin is `u.origin` (`composite.wgsl:116`), which a sub-rect quad would
  break. A compute snapshot is right, and extracting `commit_law` is the
  right DRY move regardless: test 4's tolerance argument depends on the
  snapshot and the commit sharing one law bit for bit.
- **The read mirror cannot be the snapshot target.** It is allocated in
  the scratch's own format (`scratch.rs:230-235`, `:663`:
  `create_read_mirror_texture(.., self.format)`, so `r32uint` for `paint`),
  with `COPY_DST | TEXTURE_BINDING` only (`:816`), bound through the uint
  layout that carries no sampler (`:89-92`), and addressed by its consumers
  through a per-dab `copy_origin` dab field
  (`read_mirror_terminal.rs:437-445`). Serving the snapshot would mean a
  second format, `STORAGE_BINDING`, a float layout, and the origin plumbed
  as an intrinsic dab field: the footprint-sized variant the plan already
  priced at about 60 more lines, built on a texture the perf doc lists for
  deletion with the read-mirror terminals. The layer-sized mirror is the
  better choice; its VRAM equals the dial's `build` channel. Accept.
- **wgpu.** `wgpu-core-29.0.4/src/command/compute.rs:279-298` (per-dispatch
  usage scopes), `:317-357` (`flush_bindings` merges only the active bind
  groups into a fresh scope and drains barriers), `:835-862` (`dispatch`
  calls it): verified. `wgpu-types-29.0.4/src/texture/format.rs:971`:
  `Rgba8Unorm` is `s_ro_wo`, so write-only storage is core: verified.
- **Krita.** `kis_colorsmudgeop.cpp:143-155`, `:192`, `:196-199`, `:204`;
  `KisColorSmudgeStrategyBase.cpp:135-138`, `:264-284`: all verified as
  cited.
- **`PickupAtlas`.** `grep -rn PickupAtlas crates/darkly/src` hits only
  `texture_source.rs:83` and `:90`. Removing it is correct.
- **Coordinate frames.** The table is right. `CanvasRect::clamp_f32`
  (`coord.rs:188-196`) floors the near edge, ceils the far edge and
  intersects the extent, as stated. `target_pos` is a pixel centre
  (`wgsl/mod.rs:1185`), so `uv = (src - lo) / lsz` lands on texel centres
  for integer motion and test 2's "a texel fetch" holds.
- **Checkpoints and growth.** Every texel a dab reads is rewritten by its
  own snapshot from the restored grounds, so the mirror needs no
  checkpoint, clear, or grow copy. Correct by construction, with the one
  exception in finding 2.
- **Hover preview.** Per `docs/brush-preview-and-overlays.md` a
  non-terminal cannot render differently at hover except through
  `compile_cursor_preview_body`; the shared grey fill
  (`clone_source.rs:253-262`) is right, and the declared-but-unread
  `StrokeAppearance` slot binds `_fallback` (`texture_registry.rs:214-233`).
  Accept.
- **The small generalizations.** `LiveSource::refreshed_per_dab`,
  `DabPass::can_refresh_between_dabs`, `CompiledBrush::reads_stroke_appearance`
  and the `read_reach` hook are each a one-line predicate or an
  identity-default hook, and the compile-time check asks the source and
  the pass rather than naming a node. Not over-generalized. Accept.

### Findings

1. **Opacity and blend mode do not belong in the snapshot (open question
   1). Resolve it the other way.** `paint.opacity` is "a stroke-level cap,
   applied at commit" (`paint.rs:530`) and paint-vs-erase is "a stroke
   decision applied at commit" (`paint.rs:142-149`). The plan's definition
   makes the chain depend on both: dab `n` samples an appearance already
   scaled by `o`, deposits it, and the commit scales the ground by `o`
   again. The plan's own analysis (section "What the snapshot holds",
   "Opacity") concludes this yields "a double image rather than a weaker
   smudge" and works around it by hiding the port, which a user can still
   expose in the editor. The consistent definition is the full-strength,
   paint-mode appearance: the chain runs at full strength under the
   source-over commit law, and the commit applies the cap and the blend
   mode exactly as it does for the Pencil. At `opacity = 0.5` the layer is
   then `mix(pre, full_smear, 0.5)`, which is what "opacity of the whole
   stroke" means for every other `paint` brush; under erase the commit
   removes coverage where the paint-mode smear would land, which is at
   least the same contract rather than a new, unanalysed chain under
   `destination_out`. Consequences, all simplifications: `SnapshotUniforms`
   shrinks to two slot-presence flags (both known to `paint` from the
   `buildup` shares at flush); the `opacity` read at flush and the
   `gather_from_slots = true` flip on `runner.flush_dabs`
   (`eval.rs:1228-1234`, listed under Risks) disappear; `paint.opacity` can
   be exposed on the Dry Smudge like every other paint brush; `commit_law`
   keeps its `blend_mode` argument for the commit's two entries only. Add
   one test: the Dry Smudge at `opacity = 0.5` equals the same stroke at
   `opacity = 1` mixed with the pre-stroke layer on the CPU within 1 LSB.
2. **The live arm reads stale mirror texels at the layer border: a real
   determinism defect.** `graph_smp` is `Repeat` in both axes
   (`texture_registry.rs:54-61`). The helper's bounds check admits
   `uv.x == 0.0001`, where a linear sample at texel 0 blends with texel
   `-1`, which `Repeat` maps to the opposite edge of the layer. The
   snapshot's read region is clamped to the layer (`clamp_f32`), so the
   `+ 1` margin does not exist past the border, and the opposite edge's
   mirror texels were never refreshed for this dab: zero (wgpu
   zero-initialises) at stroke start, and after a rewind the appearance of
   dabs the ring discarded. The plan's sentence "the mirror never reads
   stale texels" is therefore false at the border, and test 8's oracle
   (`recorded_stroke_rewinds_match_full_rerender`) can differ between the
   incremental and the full re-render when a stroke touches both
   opposite edges. The Snapshot arm has the same latent wrap on the
   pre-stroke snapshot (deterministic there, since that texture is frozen).
   Fix in the shared sample helper, for both arms: after the bounds check,
   clamp `uv` to `[0.5 / lsz, 1 - 0.5 / lsz]` so the filter never leaves
   the texture; or reject within half a texel of the border. Add a test:
   one dab whose read region straddles a layer edge, committed, compared
   against a CPU loop (the test 2 shape at the border), run at both
   origins.
3. **Halve the per-dab encoder traffic the plan lists as its top risk.**
   The loop in section 6 rebinds groups 0, 2 and 3 after every snapshot.
   wgpu-core keeps a bound group valid across a pipeline switch when the
   new layout's entry at that index is equal (`bind.rs:213-237`,
   `update_expectations`: `(Some, Some)` rebinds only when `!is_equal`;
   `(Some, None)` re-expects without rebinding), which is the WebGPU
   rule that bind groups persist across `setPipeline`. Give the snapshot
   pipeline the layout `[uniform_bgl, snapshot_g1]`: group 0 is the
   per-brush uniform bind group already bound (the snapshot declares the
   binding and reads nothing from it, or reads `u.intrinsic`), its two
   presence flags ride a small uniform in its own group 1, which is rebuilt
   per flush anyway. Then per dab the loop is `set_pipeline(snap)`,
   `set_bind_group(1, snap_g1, [i * INDEX_STRIDE])`, `dispatch`,
   `set_pipeline(paint)`, `set_bind_group(1, dabs, [i * INDEX_STRIDE])`,
   `dispatch`: two pipeline switches and two group binds, not two and
   five. State the per-dab call count in the performance section and have
   step 12 record it.
4. **Count every dispatch.** `BrushPerfCounters::dispatches`
   (`gpu_context.rs:75-78`) is the bench's per-event dispatch signal.
   Reading "into the scratch" so narrowly that the snapshot dispatches
   vanish from `dispatches/ev` hides exactly the number the risk section
   worries about. Count both, update the field's doc, keep
   `tests/paint_compute.rs::dispatches_count_one_per_dab` for the Ink Pen
   and add a `2 * dabs` assertion for the Dry Smudge. Open question 4
   closes.
5. **The size is honest, and the plan should say where it comes from.**
   The old draft was smaller because `buildup == 1` let a raw copy of one
   ground stand in for the appearance; the unrestricted dial the user
   requires means two grounds and the ceiling must be composited against
   the pre-stroke before a sampler can read them, so the copy becomes a
   dispatch and the dispatch needs a pipeline. Of the 855, about 200 is
   wgpu bind-group and pipeline boilerplate for a six-binding compute
   pipeline, and that is the irreducible new mechanism. Put that sentence
   in the summary. Trims that follow from the findings: findings 1 and 3
   remove about 25 lines; `scratch.rs` at 85 is high for a field, an
   `ensure`, a getter, a grow branch and a `create` function (about 60);
   `appearance_snapshot.rs` should use `BuildContext::make_uniform_ring`
   and `DynamicUniformRing` as `composite_pipeline.rs:145-148` does. A
   realistic total is 720 to 760 production lines. I also considered
   folding the snapshot into the per-brush module as a second entry point
   (`cs_snapshot` sharing `u`, `dabs`, `slot` and the grounds): it would
   save the separate file but needs stroke-only module-scope declarations
   (`NodeWgsl::decls` are shared with the preview module,
   `wgsl/mod.rs:514-519`), a per-binding storage access on
   `StorageBinding` (`wgsl/mod.rs:115-134`, read-write only today) and a
   way to bind the pre-stroke; roughly a wash in lines with more risk in
   the generic skeleton. Not recommended now.
6. **Erase is overclaimed.** Section 10 promises "a soft eraser that
   follows the grain" from a chain run under `destination_out`. Nothing in
   the plan analyses that chain, and Krita's smudge has no erase mode at
   all (`smudge.rs:96` likewise sets `supports_erase: false`). With finding
   1 the honest statement is: erase removes coverage where the paint-mode
   smear would land, under `paint`'s existing contract; the browser smoke
   in step 11 is where its feel is judged.
7. **Open question 2 (`buildup: 0.1`).** Accept the Pencil's value as the
   default: it is the brief. Note in the plan that the wash law's exact
   idempotence (`compute-paint-terminal.md` 4.5.1) is for an identical
   pigment, while a smudge's sampled pigment changes under every dab, so
   test 5 pins the law with a stable source (same position, same motion,
   `|m|` beyond the footprint) and the smear's quality at 0.1 is judged by
   the smoke, not a test. Also state that test 5 runs at runner level with
   explicit `motion`, so the stationary rule does not fire on the repeated
   dab.
8. **Stationary dabs.** The rule in the sampler's WGSL and
   `paint_info::STATIONARY_MOTION_PX` shared with `smudge.rs:37` are right;
   with finding 1 the identity argument on an opaque canvas still holds and
   the transparent-pixel alpha rise still needs the rule. Accept.
9. **Engine gate.** `active_brush_needs_source` (`painting.rs:349-360`)
   and `clone_source_port_default` (`:367-378`) name
   `clone_source::TYPE_ID` twice. Rather than deepening that from
   `painting.rs`, add `pub fn graph_needs_source(graph) -> bool` beside
   `source_default_is_live` in `clone_source.rs` and have `painting.rs`
   call it; the type id is then named only in the node's own file, which
   is as far as this step should go before the rename follow-up.
10. **Tests.** Test 2's "why this fails without the snapshot" describes a
    variant nobody will build; it is a feature test (exact three-pixel
    shift) and is fine as one, so drop the regression framing. Test 4's
    tolerance derivation is sound. Test 6 fails without the rule as
    claimed (`app OVER app` raises alpha on `0 < a < 1`). Test 8's row in
    `tests/stroke_rewind.rs` must lay the stripe down first (the `Cell`
    harness at `:95-133` starts from a blank raster layer). Add the two
    tests from findings 1 and 2.

### Reference corrections

- `composite.wgsl:124-140` for the body of `commit_fragment`: the
  function runs `:112-140`; `:124` is inside the erase branch.
- `nodes/noise.rs:118-127` for the `space` enum arm: `:122-126`.
- `kis_colorsmudgeop.cpp:143-155`: the block runs to `:155`.
- Section 4 says `grow_write` "reallocates it at the new size with no
  copy"; `grow_write` (`scratch.rs:576-657`) is where that branch goes,
  and `StrokeBuffer::grow_preserving` (`stroke_buffer.rs:288-313`) is the
  only caller, so no other site needs touching. Say so.

### Verdict

`revise`. The approach stands: a per-dab appearance snapshot dispatch is
the smallest ordered shape WebGPU allows, the layer-sized mirror is the
right home, and the generalizations are small and type-owned. Before
implementation: resolve open question 1 to a full-strength, paint-mode
appearance and drop the opacity and blend-mode plumbing (finding 1); fix
the `Repeat` wrap at the layer border in the shared sampler helper and
test it (finding 2); share group 0 with the per-brush pipeline so the loop
rebinds only group 1 (finding 3); count snapshot dispatches (finding 4);
update the LOC estimate and the summary's account of where the size comes
from (finding 5); correct the erase claim (finding 6).

## Revision (orchestrator response to review)

Every finding is accepted and folded into the plan below; the review above
is preserved verbatim. Where the plan text changed:

1. **Opacity and blend mode (finding 1):** the snapshot is the
   full-strength, paint-mode appearance. `SnapshotUniforms` is two
   presence flags; the opacity read at flush and the `gather_from_slots`
   flip are gone; `paint.opacity` is exposed on the Dry Smudge; test 11
   pins `opacity = 0.5` against a CPU mix. Open question 1 closed.
2. **Border wrap (finding 2):** the shared sample helper clamps `uv` to
   texel centres after the bounds check, for both arms; the "never reads
   stale texels" claim is restated with the border case; test 10 pins it
   at both origins.
3. **Encoder traffic (finding 3):** the snapshot pipeline's layout is
   `[uniform_bgl, snapshot_g1]` and its module declares nothing in group
   0, so per dab the loop is two `set_pipeline`, two `set_bind_group(1)`,
   two dispatches. The call count is stated in the performance section
   and step 12 records it.
4. **Dispatch count (finding 4):** every dispatch is counted; the
   counter's doc changes to "per flush"; the Ink Pen test keeps one per
   dab and a sibling pins the Dry Smudge at two. Open question 4 closed.
5. **Size (finding 5):** the summary says where the size comes from (the
   unrestricted dial turns a copy into a composite dispatch, and about
   200 lines are wgpu boilerplate); `scratch.rs` is budgeted at 60 and
   the snapshot pipeline at 205 on `make_uniform_ring`; the production
   total is about 760 added / 125 removed. The second-entry-point
   alternative is noted as considered and not taken.
6. **Erase (finding 6):** the claim is reduced to paint's existing
   contract, with the smoke as the judge and Krita's lack of an erase
   mode noted.
7. **Build-up default (finding 7):** 0.1 kept; the YAML notes record what
   test 5 does and does not pin and that it runs at runner level.
8. **Stationary dabs (finding 8):** unchanged.
9. **Engine gate (finding 9):** `clone_source::graph_needs_source` is the
   predicate; `painting.rs` calls it and names the type id nowhere.
10. **Tests (finding 10):** test 2 is framed as a feature test; test 8
    lays the stripe first; the "why a snapshot" section now lists the
    rejected alternatives in one place; the four reference corrections
    are applied.

# Live canvas sampler: a dry-media smudge through `paint`

Status: implemented. The sections below are the approved plan; where the
implementation departed from it, "Implementation notes" says how and why.

## Implementation notes

Material discoveries made while implementing, in the order they arose.

1. **The Live arm filters by hand, not through `graph_smp`.** The rewind
   oracle (`tests/stroke_rewind.rs::live_sampler_rewinds_match_full_rerender`)
   differed from the incremental run by one alpha LSB in 11 pixels. A
   texel-load experiment made it pass, which isolated the cause: the
   hardware bilinear filter's fixed-point weights depend on the
   normalized `uv`, so a dab rendered before a mid-stroke layer grow and
   the same dab re-rendered after it (the full re-render path) read
   different LSBs. The Live helper now loads four texels and mixes them
   with a weight taken from `motion` alone (`fract(-motion)`), on an
   integer base texel from the pixel's layer-local texel. That is
   independent of the frame by construction, and its taps clamp to the
   layer's edge, which also replaces the half-texel `uv` clamp for this
   arm (finding 2; test 10 still fails when the far tap is allowed to
   wrap). The Snapshot arm keeps the sampler and gains the half-texel
   clamp as planned. Test 1 checks for `textureLoad(graph_tex_` in place
   of `textureSampleLevel`.
2. **The snapshot reads the grounds as `texture_2d<u32>`**, not as
   read-only storage: the ground already carries `TEXTURE_BINDING`,
   read-only storage textures need a WGSL language feature, and the
   snapshot is left with one storage texture (the mirror). The pre-stroke
   binds as unfilterable float, read with `textureLoad`.
3. **`@workgroup_size` comes from a WGSL `override`** set to
   `DAB_WORKGROUP` in the pipeline's `compilation_options`, so
   `appearance_snapshot.wgsl` parses standalone in `tests/shader_compile.rs`.
   That test's preamble resolution became transitive, because
   `commit_law.wgsl` is a preamble that needs two others.
4. **The two presence flags are a plain uniform buffer written per flush**,
   like the records and like `paint`'s own dab records (one flush per
   submission), rather than a `DynamicUniformRing`: a ring would put a
   second dynamic offset into group 1 for no gain. `DAB_SLOT_STRIDE` (the
   index buffer's stride) moved to `wgsl/mod.rs` beside `DAB_WORKGROUP`.
5. **`dab_read_reach` resets at the top of `execute_cpu`**, not in
   `clear_slots`, since some runner-level callers do not clear slots
   between dabs and the reach must not carry over.
6. **The engine gate**: `clone_source::snapshot_sampler(graph)` returns
   the first sampler on the Snapshot source; `graph_needs_source` and
   `painting.rs`'s port-default reads both go through it, so the type id
   is named only in the node's file.
7. **YAML**: the "Grain" knob is not `invert: true`. `levels.in_high`
   raises the grain as it rises (`levels` maps `[in_low, in_high]` onto
   `[0, 1]`, so a higher white point leaves more of the noise below
   full), so the mirrored control would have run backwards. YAML node ids
   are file-local; the loader reassigns ids by type, so the sampler is
   `clone_source` and the strength dial `user_input` inside the graph.
8. **Test 8's stripes stay a dab's reach inside the layer.** Stripes at
   the border let the smear cross the edge at a dab clipped before the
   layer grows, the history `grow_reverse_jump`'s note already says a
   from-scratch render cannot reproduce (the Ink Pen over the same
   stripes diverges identically). The determinism replay lives in
   `tests/paint_compute.rs` beside the Ink Pen's.
9. **Bench**: `paint`, `pencil` and `dry-smudge` were run in one session;
   the Pencil is the fair baseline (same dial and spacing). Results are in
   `docs/paint-compute-perf-tracking.md` attempt #9.

This plan replaces an earlier draft of the same name whose design (a
serialized one-dab-per-render-pass flush inside `paint`, with a
`copy_texture_to_texture` mirror refresh between passes and a `buildup == 1`
restriction) was written against the instanced fragment `paint` terminal.
That terminal no longer exists: `paint` is a compute terminal (commit
`b4a1c545`, `docs/plans/compute-paint-terminal.md`,
`docs/paint-compute-perf-tracking.md` attempt #6), one compute pass per
flush and one `dispatch_workgroups` per dab, with dispatches ordered and
each dispatch's stores visible to the next. Everything below is built on
that. The old draft's brush YAML, the sampler's port design, its test
ideas and its Krita citations are carried forward; its serialized flush is
not.

## Summary

A finger smudge for pencil and charcoal, built as a *canvas-sampling node
feeding the `paint` terminal* rather than as a terminal of its own. The
node's `color` output samples the stroke in progress at
`target_pos - motion` (what was under this pixel one dab ago), feeds
`stamp.color`, and `stamp.dab` feeds `paint.rgba`. Paper grain comes from
wiring the pencil's canvas-space grain chain into `stamp.tip`. `paint`
supplies flow, the build-up dial, pressure size, the hover preview, the
commit, and undo. The user's hard requirement is honoured: the smudge
accumulates under `paint`'s own laws at **every** setting of the dial,
with no restriction on it.

Two mechanisms are new, both generic:

1. **The sampler** is `clone_source` generalized with a `source` port:
   `Snapshot` (the canvas frozen at stroke start, what it does today) or
   `Live` (the stroke in progress), plus a `motion` input the live arm
   samples one dab behind along.
2. **The stroke appearance mirror**: a brush whose graph requests the
   live stroke (`LiveSource::StrokeAppearance`) gets, before each dab's
   dispatch, a second small dispatch that renders the stroke's current
   appearance (the pre-stroke snapshot with the wash ground laid through
   the ceiling and the build ground over it, at stroke opacity, under the
   stroke's blend mode: exactly what the commit would show) into a
   layer-sized `rgba8unorm` mirror under the dab's read region. The dab's
   own dispatch samples the mirror. Dispatch ordering inside the pass makes
   the read exact; the sampler never learns how many grounds exist. The
   capability is named after the dependency (a graph reads the stroke's
   appearance at other pixels), owned by the framework and the terminal,
   and is what later lets blur and watercolor's pickup move onto `paint`.

Ships with `crates/darkly/brushes/dry_smudge.yaml`. `smudge.yaml` and the
`smudge` terminal are untouched; deleting `smudge` and `blur` once this
capability exists is the follow-up the perf doc already lists.

Where the size comes from: an earlier draft of this plan was smaller
because a `buildup == 1` restriction let a raw copy of one ground stand in
for the appearance. The unrestricted dial the user requires means two
grounds and the ceiling must be composited against the pre-stroke before a
sampler can read them, so the copy becomes a dispatch and the dispatch
needs a pipeline; about 200 of the production lines are the wgpu
bind-group and pipeline boilerplate of that one new mechanism.

## Feature semantics

### What a dab does

Per dab `n` at centre `p_n` with motion `m_n = p_n - p_{n-1}`, for every
pixel `t` in the dab's footprint:

```
appearance_{n-1}  = commit_law(wash_{n-1}, build_{n-1}, pre)   // what the commit would show now
sampled           = appearance_{n-1}(t - m_n)                  // straight RGBA, bilinear
dab               = premul(sampled) * tip(t) * flow            // stamp, then paint's per-half flow
grounds_n         = paint's laws applied to `dab` against grounds_{n-1}
```

`paint`'s laws are unchanged shader code (`shaders/brush/paint_accumulate.wgsl`):
at `buildup = 1` the ground takes `dab` under premultiplied source-over, at
`buildup = 0` under the deposit ceiling, and between, each half under its
own law. On an opaque canvas at `buildup = 1` this collapses to
`appearance_n(t) = mix(appearance_{n-1}(t), appearance_{n-1}(t - m), tip * flow)`,
which is the `smudge` terminal's `mix(bg, src, rate * mask)` with
`rate = build_flow` (`crates/darkly/src/brush/nodes/smudge.rs:139-150`).
That equality is the equivalence test below. Under the ceiling the smear
takes, per pixel, the strongest dab that lands there and refuses a
retrace at the same pressure, which is the Pencil's law and the user's
stated intent: the smudge is the pencil's medium, so it accumulates like
the pencil.

### Prior art (Krita, verified in source)

`krita/plugins/paintops/colorsmudge/kis_colorsmudgeop.cpp`:

- `:143-153`: in smearing mode the op disables sub-pixel precision so it
  reads from the aligned image; the source is the previous dab's centre.
- `:192`: `srcDabRect = m_dstDabRect.translated((m_lastPaintPos - newCenterPos).toPoint())`:
  the source rect is the destination rect translated back to the previous
  dab's centre, which is the same `t - motion` this plan samples at.
- `:196-199`: `if (m_firstRun) { m_firstRun = false; return spacingInfo; }`:
  the first dab of a stroke paints nothing.
- `:204`, `:219`: `fpOpacity = m_opacityOption.apply(info)` is handed to
  the strategy per dab, so Krita's smudge opacity is per dab.

`krita/plugins/paintops/colorsmudge/KisColorSmudgeStrategyBase.cpp`:

- `:135-138` (`smearCompositeOp`): `COPY` when smearing alpha, else `OVER`.
- `:264-284` (`blendInBackgroundWithSmearing`): the source rect and the
  destination rect are both `readBytes`, and the source is composited over
  the destination at the smudge-rate opacity. Nothing writes the source:
  Krita never depletes it.

This plan follows Krita on the first-dab rule and on the read-then-write-
at-offset shape, and reads a per-dab refreshed mirror rather than the
layer directly because WebGPU gives no intra-dispatch ordering.

### Why a snapshot per dab, and not a direct read of the ground

In the compute skeleton `ground` is in scope for every node body, so a
sampler could `textureLoad(ground, ...)` at `t - m` inside the dab's own
dispatch. That read races: the thread owning texel `t - m` is in the same
dispatch when `|m|` is smaller than the footprint, which at smudge spacings
it always is (spacing 0.03 of the diameter puts `|m|` at 6% of the radius),
and WebGPU orders nothing inside a dispatch. The result would depend on
scheduling. What is ordered is dispatch against dispatch, so the read must
come from something written by an earlier dispatch: a per-dab snapshot
dispatch over the read region, then the dab. This is the one real engine
piece.

Cheaper escapes were looked for and none is correct. Sampling `k` dabs
back so the read never overlaps the writing dab needs `|k m| > 2r`, about
33 dabs at spacing 0.03, and changes the medium. Writing the appearance
from the dab's own dispatch into a mirror the same dispatch reads at an
offset is the same race on the mirror. Ping-ponging the grounds doubles
the stores, breaks the store-only-where-changed law and the checkpoint
ring's one-texture contract. A `copy_texture_to_texture` cannot sit
inside a compute pass, so it brings back a pass boundary per dab. A render
pass per dab for the snapshot (the commit composite over a sub-rect) costs
about 27 us against 1.5 us per dispatch (`docs/paint-compute-perf-tracking.md`,
"#5, stage 1"): 13 ms against 0.7 ms per event on the 1080p radius-1 row,
and the composite's background mapping assumes `u.origin`
(`composite.wgsl:116`), which a sub-rect quad breaks. A dispatch that only
writes the mirror followed by a dispatch that only reads it is the
smallest ordered shape.

### What the snapshot holds, and why

The mirror holds the stroke's *appearance*: the pre-stroke snapshot with
the wash ground deposited through the ceiling and the build ground over it,
at full strength, in paint mode. That is `composite.wgsl`'s
`commit_fragment` law with the opacity cap and the blend mode left to the
commit, run under the dab's read region into the mirror instead of across
the layer into the paint target. Three consequences:

- The sampler reads one straight-alpha texture and knows nothing about
  grounds, laws, channels or opacity. A terminal that accumulates
  differently (watercolor's build slot alone) renders its own appearance
  the same way, with its own slot mapping.
- The dial is unrestricted. Inside the dial `paint` keeps two grounds; the
  snapshot composites both exactly as the commit will, so a live sampler
  under `buildup = 0.1` sees the Pencil's picture.
- The law is shared, not copied: `commit_fragment`'s body moves into
  `shaders/lib/commit_law.wgsl` as a pure function both the commit's
  fragment entries and the snapshot's compute entry call.

**Opacity and blend mode stay at the commit.** `paint.opacity` is "a
stroke-level cap, applied at commit" (`paint.rs:530`) and paint-vs-erase is
"a stroke decision applied at commit" (`paint.rs:142-149`). If the
snapshot applied them too, the chain would depend on both: dab `n` would
sample an appearance already scaled by the opacity, deposit it, and the
commit would scale the ground by it again, which reads as a double image.
So the chain runs at full strength under the paint-mode law, and the
commit applies the cap and the blend mode once, as it does for the Pencil.
At `opacity = 0.5` the layer is `mix(pre, full_smear, 0.5)`, which is what
"opacity of the whole stroke" means for every other `paint` brush, and
`paint.opacity` is exposed on the Dry Smudge like on every other paint
brush. Krita (`kis_colorsmudgeop.cpp:204`) and the `smudge` terminal
(`smudge.rs:149`) apply opacity per dab instead and read as a shorter
smear; that is a different control, and the Dry Smudge's per-dab strength
is its flow. Test 11 pins the definition.

## Architectural impact

- **Authority.** Nothing new is document state. The appearance mirror is
  a stroke resource on `Scratch` (session), derived per dab from the
  grounds and the pre-stroke snapshot and never checkpointed; the per-dab
  read region is transient data on `DabBatch`; the source kind is
  compile-derived from the graph. No document field, no second home for
  any fact.
- **Ownership.** `Scratch` owns the mirror's allocation and growth, as it
  owns the channels. The snapshot pipeline (framework, `brush/appearance_snapshot.rs`)
  owns "render two packed grounds and a background through the commit law
  into a region of the mirror", the compute twin of `commit_brush_dab`.
  `paint` owns the mapping of its grounds onto the law's slots and the
  decision to run the snapshot before each dab, as it owns the commit's
  slot mapping today. The sampler owns its reach and its WGSL. The engine
  learns nothing.
- **Modularity / type-owned dispatch.** Whether a brush needs the snapshot
  is a property of a *source* (`LiveSource::StrokeAppearance`,
  `LiveSource::refreshed_per_dab()`), composed onto
  `CompiledBrush::reads_stroke_appearance()`; `paint` asks the compiled
  brush, never the graph, and no code anywhere asks whether a node is a
  `clone_source`. Whether a terminal *can* supply the snapshot is a
  property of its pass (`DabPass::can_refresh_between_dabs()`); the
  compiler rejects the combination that cannot, naming both. A second
  node that samples the live stroke (blur on `paint`) requests the same
  source and gets the mirror with no edits outside its own file.
- **DRY.** One commit law in `lib/commit_law.wgsl`, called by `fs_main`,
  `fs_packed` and the snapshot's `cs_main`. The sampler's two arms share
  the port surface, the preview body, the bounds check and the sample
  helper; only the texture, its frame and the offset differ. The
  stationary threshold has one home (`paint_info::STATIONARY_MOTION_PX`),
  imported by `smudge.rs` and emitted into the sampler's WGSL.
- **What this does not change.** The instanced skeleton, the read-mirror
  terminals, the checkpoint ring, the commit pipeline's entries, the
  engine's stroke loop, the JS/Rust boundary. The frontend keeps calling
  `activeBrushNeedsSource()`.

## What was verified before designing (facts, with sources)

### Storage write in one dispatch, filtered sample in the next, one pass

The WebGPU usage-scope rule for compute passes is per dispatch, and wgpu
29 implements it that way: `wgpu-core-29.0.4/src/command/compute.rs:280-298`
("compute passes have a separate usage scope for each dispatch ... we call
`drain_barriers` here, because barriers may be needed before each dispatch
if a previous dispatch had a conflicting usage"), with `flush_bindings`
merging only the bind groups the current pipeline uses into a fresh scope
and `dispatch` (`:835-862`) calling it before every dispatch. So a texture
bound as write-only storage in dispatch `k` and as a filtered
`texture_2d<f32>` in dispatch `k + 1` of the same pass is legal, and the
transition is the barrier that makes the write visible. The paint port
measured that barrier as free (perf doc "#5, stage 1": shape (b) is not
cheaper than (a)).

Format: `rgba8unorm` is `s_ro_wo` in `wgpu-types-29.0.4/src/texture/format.rs:971`,
so write-only storage is in core WebGPU, and it is filterable, so the
graph-texture layout (`gpu/texture_registry.rs:181-195`,
`Float { filterable: true }`, visibility `FRAGMENT | COMPUTE`) binds it
unchanged and the registry's linear sampler filters it. The mirror is
therefore `Rgba8Unorm` with `STORAGE_BINDING | TEXTURE_BINDING`, written
with `textureStore` from the snapshot dispatch and read with
`textureSampleLevel` through `graph_tex_N` by the sampler, which is what
`sample_graph_texture` emits already (`wgsl/mod.rs:919-921`). No packed
`r32uint` mirror and no manual bilinear are needed. Limits: the snapshot
dispatch binds three storage textures (wash, build, mirror) against
`max_storage_textures_per_shader_stage: 4`; the dab dispatch binds its two
grounds and samples the mirror as an ordinary texture.

Tint is not in the tree; the browser smoke in step 11 is where this is
confirmed on the web, as it was for the compute skeleton.

### What the compute skeleton gives a node body

`wgsl/mod.rs:1159-1189` (`push_compute_entry`): `d`, `origin`,
`canvas_px`, `layer_px`, `target_pos` (pixel centre), then
`push_pixel_locals` (`local`, `local_uv`, `theta`, `canvas_origin`, `sel`),
then the bodies. `u.intrinsic.layer_offset` / `layer_size` are the paint
target's plane rect (`paint.rs:674-683`, `intrinsic_header`), which is the
frame the layer-sized mirror shares with the grounds. The `@group(3)`
textures are declared in both variants from the shared `graph_sources`
list (`:1115-1124`), so a live slot binds the mirror in stroke mode and
`_fallback` in preview mode with no special case
(`texture_registry.rs:214-233`).

`NodeWgsl::decls` are shared by both shader variants (`wgsl/mod.rs:514-519`,
`:550-556`); a decl may reference only what both modules declare (`u`,
`graph_tex_N`, `graph_smp`), never `ground` or a stroke-only binding. The
live arm's helper below obeys that.

### How paint flushes today

`paint.rs:634-793`: one compute pass; group 0 uniforms, group 2 selection,
group 3 graph textures (rebuilt per flush when any source is live,
`:698-731`, publishing `StrokeSnapshot` from `stroke.source_texture()`),
then per dab `set_bind_group(1, ..., &[i * INDEX_STRIDE])` and
`dispatch_workgroups` over the grid the batch meta carries
(`DabGrid`, `:106-113`, pushed at `:621-626`). `evaluate_gpu` computes the
footprint through `record_dab_footprint` (`gpu_context.rs:397-415`,
`clamp_f32`: floor near, ceil far, intersect the extent). The group-1 bind
group is rebuilt per flush because a grow can reallocate the ground
(`:749-756`).

### Scratch, channels, grow, checkpoints

`scratch.rs`: channels are layer-sized, allocated by `ensure_channels`
(`:273-305`) from the compiled brush's declaration in
`BrushGraphRunner::begin_stroke` (`eval.rs:1189-1199`), freed when a brush
declares none, rebased by `grow_write` (`:576-657`). The checkpoint ring
snapshots the write side and the channels (`checkpoint_ring.rs:73-80`,
`StrokeResources::channel_textures`). A derived texture that is rewritten
under every dab's read region before any read needs none of the copy,
clear or checkpoint traffic: the appearance mirror is allocated and grown
like a channel and otherwise ignored by the ring and the lifecycle.

### Motion, rewinds, stationary dabs

`stroke_engine.rs:697-708`: `motion` is the delta from the previous
*emitted* dab, `[0, 0]` when there is none (stroke start, after
`reset_render_state`). `restore_render_state` (`:255-263`) restores
`last_dab_pos`, so the first dab after a partial rewind carries its true
delta. The `smudge` terminal drops a dab below `STATIONARY_THRESHOLD_PX = 0.5`
(`smudge.rs:34-37`, `:111-125`).

### Engine gates that key on `clone_source`

`painting.rs:349-360` (`active_brush_needs_source`, a `type_id` match on
the graph), `:367-378` (`clone_source_port_default`), `:942-946` (the
clone no-op gate, through `runner.samples_source()`, which matches
`StrokeSnapshot` only, `eval.rs:811-818`), `:1054-1093` (source snapshot
capture, also under `samples_source`). `CloneState` seeding runs only when
the engine holds an anchor (`stroke_engine.rs:488-510`).

## Design

### 1. The sampler: `clone_source` with a `source` port

Recommendation: generalize `clone_source`. Both arms are
`color = sample(texture, target_pos + offset)` with a bounds check and a
neutral preview; what differs (the texture, its frame, the offset) is the
kind of compile-time arm `noise` already switches on
(`nodes/noise.rs:122-126`, the `space` enum picks the emitted arm). A
second node would duplicate the port surface, the preview override, the
CPU placeholder and the sample helper, and leave "the canvas at an offset"
split across two files that must agree on frames. A new node would be
justified only if the live arm needed a different evaluator lifecycle (it
does not: `is_gpu: false`, non-terminal, per-pixel) or terminal (it does
not: `paint` both ways).

`type_id` stays `clone_source`; the rename is a follow-up (open question
5). Display name becomes "Canvas Sampler"; the description covers both
arms.

Ports added to `register()` in `crates/darkly/src/brush/nodes/clone_source.rs`:

```rust
PortDef::input("source", BrushWireType::Enum)
    .with_enum_options(["Snapshot", "Live"])
    .with_value(InputValue::Int(0))
    .with_label("Source")
    .with_description("Snapshot: the canvas frozen at stroke start (clone). \
                       Live: the stroke as it is being painted (smudge)."),
PortDef::input("motion", BrushWireType::Vec2)
    .with_visible_when("source", [1])
    .with_description("Per-dab motion in canvas pixels (wire Pen Input -> Motion). \
                       Live samples one dab behind along this vector."),
```

`center`, `mode` and `merged` gain `.with_visible_when("source", [0])`.
Enum ports are non-wirable, so `source` is always a compile-time literal,
read by `pub fn source_default_is_live(v: f32) -> bool` beside
`mode_default_is_anchored` (one threshold shared by the compile-time bake
and the engine's structural query, the existing rule at
`clone_source.rs:122-144`).

`compile_wgsl` branches once on `source`:

- `Snapshot`: the existing body, unchanged, requesting
  `LiveSource::StrokeSnapshot` and declaring the anchor and frame uniforms.
- `Live`: the body in section 6, requesting `LiveSource::StrokeAppearance`
  and declaring no uniforms.

`compile_cursor_preview_body` is shared: the neutral grey fill at
`clone_source.rs:253-262` is right for both arms.

New trait hook (section 3): `read_reach` returns
`[|m.x|.ceil() + 1, |m.y|.ceil() + 1]` in `Live` mode (the `+ 1` covers
the bilinear filter's half-texel reach past `t - m`, the same margin
`smudge.rs:119-124` adds) and `[0, 0]` otherwise.

`evaluate_cpu` keeps returning the grey placeholder.

### 2. The live source and the pass that can supply it

`crates/darkly/src/brush/texture_source.rs:74-93`, `LiveSource` becomes:

```rust
pub enum LiveSource {
    /// The stroke's frozen source snapshot: the cross-layer / merged
    /// snapshot when one was captured, else the pre-stroke snapshot.
    /// Never changes during the stroke. Requested by `clone_source`'s
    /// Snapshot arm.
    StrokeSnapshot,
    /// The stroke as the commit would show it right now: the pre-stroke
    /// snapshot with every accumulation laid on it under the commit law,
    /// at stroke opacity, under the stroke's blend mode. Layer-sized,
    /// in the paint target's frame. Depends on the pass's own output,
    /// so the terminal refreshes it under each dab's read region before
    /// that dab's dispatch; a pass that cannot order a refresh between
    /// dabs cannot host it.
    StrokeAppearance,
}

impl LiveSource {
    /// Whether this source holds the output of the pass that samples it
    /// and must be refreshed between consecutive dabs.
    pub fn refreshed_per_dab(&self) -> bool { matches!(self, Self::StrokeAppearance) }
}
```

`PickupAtlas` is removed in the same edit: nothing produces it
(`grep -rn PickupAtlas crates/darkly/src` hits only `texture_source.rs:83,90`;
watercolor binds its atlas through `terminal_bindings`). The comments that
describe a `pickup` *node* publishing a live slot (`texture_source.rs:17`,
`wgsl/mod.rs:216-217`, `:1108`, `paint.rs:232-234`, `:694-695`, `:772-773`,
`context.rs:63-68` is about watercolor and stays, `gpu_context.rs:271-280`)
are corrected to name the two sources that exist.

`ResolvedSource::refreshed_per_dab()` forwards;
`CompiledBrush::reads_stroke_appearance()` (`wgsl/mod.rs`, beside
`storage_bindings`) is `self.graph_sources.iter().any(|s| s.refreshed_per_dab())`.

`crates/darkly/src/brush/node.rs`, on `DabPass`:

```rust
/// Whether a texture written for one dab can be refreshed before the
/// next dab of the same flush reads it. A dispatch per dab orders its
/// dispatches and a dispatch's stores are visible to the next; the
/// instances of one draw cannot see each other's writes, and nothing
/// can run between them.
pub fn can_refresh_between_dabs(self) -> bool {
    matches!(self, Self::DispatchPerDab)
}
```

`compile_brush_to_wgsl` (`wgsl/mod.rs:654-679`, with the other
terminal-against-graph checks) rejects a graph whose sources include one
that is refreshed per dab under a terminal whose pass cannot refresh:

```
`<live stroke appearance>` is refreshed between dabs, which a terminal
that draws its dabs in one instanced pass (`smudge`) cannot do; end the
graph in `paint`
```

The check asks the source and the pass capability questions; it names no
node.

`BrushGraphRunner::samples_source()` (`eval.rs:811-818`) keeps matching
`StrokeSnapshot` only, so the clone no-op gate and the source-snapshot
capture do not fire for a live sampler.

### 3. Per-dab read reach

The sampler reads `|motion|` beyond the dab footprint. The extent protocol
(`wgsl/extent.rs`) sizes the *write* footprint, is composed once at compile
time, and `motion` is per dab and unbounded (the step depends on
`effective_diameter()` and the session's spacing config, neither in the
graph), so the read region is different data with a different lifetime and
gets the per-dab sibling of `extent`:

`crates/darkly/src/brush/eval.rs`, on `BrushNodeEvaluator` after `extent`:

```rust
/// Canvas pixels this node reads beyond the dab's write footprint, per
/// axis, for the dab being evaluated. The per-dab counterpart of
/// [`extent`]: `extent` bounds what a dab writes and is composed once at
/// compile time; this bounds what it reads and is composed per dab,
/// because a read offset such as the stroke's motion is per-dab data
/// with no compile-time bound. The runner takes the per-axis maximum
/// over the graph and hands it to the terminal, which sizes the region
/// it refreshes the stroke appearance under. One virtual call per node
/// per dab with an identity default; it cannot move to compile time.
fn read_reach(&self, _ctx: &EvalContext) -> [f32; 2] { [0.0, 0.0] }
```

`BrushGraphRunner` gains `dab_read_reach: [f32; 2]`, reset in
`clear_slots` (`eval.rs:1352`), folded with a per-axis `max` right after
each `evaluate_cpu` in `execute_cpu` (`:1016`) and in `dispatch_gpu`'s
promoted-node `evaluate_cpu` (`:1137`), and copied onto
`gpu.dab_batch.read_reach` beside `compiled_brush` and `slot_outputs`
(`:1084-1087`). `DabBatch` (`gpu_context.rs:229`) gains
`pub read_reach: [f32; 2]`, documented as per-dab.

### 4. The appearance mirror on `Scratch`

`crates/darkly/src/brush/scratch.rs`: a third read path for the in-flight
stroke, beside the read mirror and the direct read (module doc `:27-40`).

```rust
/// The stroke's appearance under the dabs placed so far, rendered by the
/// terminal under each dab's read region before that dab runs, for
/// graphs that sample the live stroke at other pixels. Layer-sized in the
/// write side's frame so a sampler addresses it through the paint
/// target's extent alone; written as storage by the terminal's snapshot
/// dispatch, sampled as an ordinary graph texture by the dab's dispatch.
/// Derived: never cleared, checkpointed or restored, because every texel
/// a dab reads was rewritten for that dab. `None` until a brush asks.
appearance: Option<(wgpu::Texture, wgpu::TextureView)>,
```

- `ensure_appearance_mirror(&mut self, device: &wgpu::Device, wanted: bool)`:
  allocate at `(write_w, write_h)` in `Rgba8Unorm` with
  `STORAGE_BINDING | TEXTURE_BINDING` when wanted and absent; free when not
  wanted (a `Scratch` outlives one brush: the preview renderer keeps one
  across brushes, the same reason `ensure_channels` frees). Idempotent.
- `appearance_view(&self) -> Option<&wgpu::TextureView>`.
- `grow_write` (`scratch.rs:576-657`) reallocates it at the new size with
  no copy: contents are rewritten before any read. Its only caller is
  `StrokeBuffer::grow_preserving` (`stroke_buffer.rs:288-313`), so no
  other site needs touching.

`BrushGraphRunner::begin_stroke` (`eval.rs:1189-1199`) calls it beside
`ensure_channels`, with `compiled.reads_stroke_appearance()`.

Memory: one RGBA8 texture the size of the layer, allocated only for a
brush that samples the live stroke (a 4K layer is 33 MB, the same as the
dial's `build` channel). The footprint-sized alternative (a lazily grown
mirror with a per-dab origin, the shape the read-mirror terminals use)
was rejected for this plan: it needs the origin carried to the sampler
through a per-dab record or buffer, a bind group that changes on grow, and
origin arithmetic in two places; the layer-sized mirror needs none of
that, and its frame is the one the sampler already has in
`u.intrinsic`. Recorded as open question 3.

### 5. The snapshot pipeline (framework) and the commit law (shared)

**`shaders/lib/commit_law.wgsl`** (new): the body of `commit_fragment`
(`composite.wgsl:112-140`) as a pure function:

```wgsl
// The stroke commit's law over two premultiplied, opacity-scaled
// foregrounds and a straight-alpha background. Called by the commit
// (`brush/composite.wgsl`, one fragment per layer pixel) and by the
// appearance snapshot (`brush/appearance_snapshot.wgsl`, one thread per
// pixel of a dab's read region), so the stroke a sampler reads mid-stroke
// is the stroke the commit will show. An opacity of zero marks an absent
// slot, which is skipped rather than composited at zero alpha.
fn commit_law(wash: vec4f, build: vec4f, bg: vec4f, blend_mode: u32,
              wash_opacity: f32, build_opacity: f32) -> vec4f {
    if blend_mode == 1u {
        return destination_out(build.a, destination_out(wash.a, bg));
    }
    var out = bg;
    if wash_opacity > 0.0 { out = deposit_through_ceiling(wash, out); }
    if build_opacity > 0.0 { out = source_over(build.rgb, build.a, out); }
    return out;
}
```

`deposit_through_ceiling` moves with it. `composite.wgsl` keeps the
vertex stage, the two fragment entries and the background sample, and
calls `commit_law`. `composite_pipeline.rs:72-81` includes the new lib
after `deposit_ceiling.wgsl`. `tests/shader_compile.rs` validates a
preamble through the entry-point file that calls it, which `composite.wgsl`
does.

**`shaders/brush/appearance_snapshot.wgsl`** (new, prepended with
`source_over.wgsl`, `lib/deposit_ceiling.wgsl`, `lib/commit_law.wgsl`):

```wgsl
struct SnapshotUniforms {
    has_wash: u32,       // 0 = slot absent
    has_build: u32,      // 0 = slot absent
    _pad0: u32,
    _pad1: u32,
};
// One per dab, in lockstep with the dab records: the read region, in
// write-side (layer-local) texels, already clamped to the layer.
struct SnapshotRecord { origin: vec2<u32>, size: vec2<u32> };
struct DabSlot { i: u32, pad0: u32, pad1: u32, pad2: u32 };

// Group 0 is the per-brush uniform group (`uniform_bgl`) the brush's own
// pipeline has bound. This module declares nothing in it, so the group
// stays bound across the pipeline switch and only group 1 is rebound per
// dab (wgpu-core `bind.rs:213-237`: an equal layout entry is kept).
@group(1) @binding(0) var<uniform> u: SnapshotUniforms;
@group(1) @binding(1) var<storage, read> records: array<SnapshotRecord>;
@group(1) @binding(2) var<uniform> slot: DabSlot;
@group(1) @binding(3) var wash: texture_storage_2d<r32uint, read>;
@group(1) @binding(4) var build: texture_storage_2d<r32uint, read>;
@group(1) @binding(5) var appearance: texture_storage_2d<rgba8unorm, write>;
@group(1) @binding(6) var pre_stroke: texture_2d<f32>;

@compute @workgroup_size(8, 8, 1)
fn cs_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let r = records[slot.i];
    if (any(gid.xy >= r.size)) { return; }
    let px = vec2<i32>(r.origin + gid.xy);
    let wash_px = unpack4x8unorm(textureLoad(wash, px).r);
    let build_px = unpack4x8unorm(textureLoad(build, px).r);
    let bg = textureLoad(pre_stroke, px, 0);
    // Full strength, paint mode: the opacity cap and the blend mode are
    // the commit's, applied once, as for every paint brush.
    textureStore(appearance, px, commit_law(wash_px, build_px, bg, 0u,
                                            f32(u.has_wash), f32(u.has_build)));
}
```

`@workgroup_size` is emitted from `DAB_WORKGROUP` (`wgsl/mod.rs:102`) by
a `format!` at module build, as the skeleton does, so the two grids divide
by one constant.

**`crates/darkly/src/brush/appearance_snapshot.rs`** (new): the compute
twin of `commit_brush_dab`, registered through `plumbing_registrations()`
(`pipeline.rs:319-323`) beside the composite and the warp-field resolve.

```rust
/// Renders a terminal's accumulations through the commit law into the
/// scratch's appearance mirror, one dispatch per dab over that dab's read
/// region, so a graph that samples the stroke at other pixels reads what
/// the commit would show. Format-fixed: packed `r32uint` grounds, the
/// only kind a dispatch-per-dab terminal accumulates.
pub struct AppearanceSnapshotPipeline {
    /// Layout `[uniform_bgl, bgl]`: group 0 is the per-brush uniform group
    /// the brush pipeline already has bound and this module never declares,
    /// so it stays bound across the pipeline switch.
    pipeline: wgpu::ComputePipeline,
    /// Group 1: flags, records, slot, the two grounds, the mirror, the
    /// pre-stroke snapshot. Rebuilt per flush (a grow reallocates views).
    bgl: wgpu::BindGroupLayout,
    /// The two presence flags, one entry per flush, through
    /// `BuildContext::make_uniform_ring` as `composite_pipeline.rs:145-148`.
    flags: DynamicUniformRing,
    /// One `SnapshotRecord` per queued dab, uploaded per flush.
    records: wgpu::Buffer,               // MAX_DABS_PER_PHASE * 16 bytes
}

/// What one flush's snapshots read and write. The terminal maps its
/// accumulations onto the two slots exactly as it does for the commit;
/// an absent slot borrows the present one and is switched off by its
/// opacity, the `commit_brush_dab` convention.
pub struct SnapshotSources<'a> {
    pub wash: Option<&'a wgpu::TextureView>,
    pub build: Option<&'a wgpu::TextureView>,
    pub pre_stroke: &'a wgpu::TextureView,
    pub appearance: &'a wgpu::TextureView,
}

impl AppearanceSnapshotPipeline {
    /// Upload this flush's records, build the group-1 bind group (per
    /// flush: a grow reallocates every view in it) and write the
    /// uniforms. Returns what `dispatch` needs per dab.
    pub fn begin_flush(&self, device, queue, index_buffer: &wgpu::Buffer,
                       records: &[SnapshotRecord], sources: SnapshotSources<'_>) -> FlushSnapshots;
    /// Record dab `i`'s snapshot into an open compute pass: set the
    /// pipeline and group 1 at the dab's slot offset (group 0 stays
    /// bound), dispatch over the record's size.
    pub fn dispatch(&self, pass: &mut wgpu::ComputePass<'_>, flush: &FlushSnapshots, i: u32);
}
```

The dab slot is paint's static index buffer (`PaintPipeline::index_buffer`,
`paint.rs:399-425`), passed in: the snapshot for dab `i` binds the slot at
`i * INDEX_STRIDE` exactly as the dab's own dispatch does, so both read
`slot.i == i`. `INDEX_STRIDE` moves to `appearance_snapshot.rs` or
`wgsl/mod.rs` as a shared constant; either is fine.

### 6. `paint`: the read region, the per-dab snapshot, the publish

**Per-dab meta** (`paint.rs:106-113`): `DabGrid` becomes

```rust
/// Per-dab meta, in lockstep with the dab records: the footprint's size
/// (the dab's dispatch grid) and the read region (the appearance
/// snapshot's dispatch grid and origin, write-side texels), clamped to
/// the layer. The read region is the footprint expanded by the graph's
/// per-dab read reach; for a graph that reads nothing beyond its
/// footprint it equals the footprint and is never used.
struct PaintDabMeta { grid: [u32; 2], read: SnapshotRecord }
```

Sixteen more CPU bytes per dab for every paint brush, read by nothing
unless the brush samples the live stroke; stated here so nobody wonders.

**`evaluate_gpu`** (`paint.rs:582-632`): after `record_dab_footprint`,
compute the read region with the same clamp:

```rust
let reach = gpu.dab_batch.read_reach;
let read = paint_target.canvas_extent().clamp_f32(
    position[0] - bbox_radius - reach[0], position[1] - bbox_radius - reach[1],
    position[0] + bbox_radius + reach[0], position[1] + bbox_radius + reach[1],
).expect("the read region encloses a footprint that overlaps the layer");
// translated to write-side texels, as `prepare_dab_canvas_copy` does
```

**`flush_dabs`** (`paint.rs:634-793`): the prologue keeps its shape. After
`ensure_per_brush_pipeline`:

```rust
let snapshots = compiled.reads_stroke_appearance().then(|| {
    let (wash_share, build_share) = shares(buildup);   // the same mapping `commit` uses
    let snap = gpu.pipelines.get::<AppearanceSnapshotPipeline>(APPEARANCE_SNAPSHOT_ID);
    snap.begin_flush(gpu.device, gpu.queue, &pipeline_ref.index_buffer,
        &metas.iter().map(|m| m.read).collect::<Vec<_>>(),
        SnapshotSources { wash: ..., build: ..., pre_stroke: &pre_stroke_view,
                          appearance: scratch.appearance_view().expect("begin_stroke allocated it") })
});
```

The slot mapping (`wash` = the ground when `wash_share > 0`, `build` = the
ground at `buildup = 1` or the `build` channel inside the dial) is lifted
out of `commit` (`:804-818`) into a small `fn slots(&self, ctx, scratch)
-> (Option<&TextureView>, Option<&TextureView>)` both call, so the
snapshot and the commit cannot map differently. The snapshot reads nothing
per stroke beyond the two presence flags, which follow from `buildup`'s
shares exactly as `commit`'s slots do; `runner.flush_dabs` keeps gathering
nothing from the slot table.

The group-3 publish (`:698-731`) adds, beside `StrokeSnapshot`:

```rust
if let Some(view) = scratch.appearance_view() {
    gpu.dab_batch.publish_live_texture(LiveSource::StrokeAppearance, view.clone());
}
```

The pass body:

```rust
for (i, meta) in metas.iter().enumerate() {
    if let Some(flush) = &snapshots {
        // Its pipeline and its group 1. Group 0 is an equal layout entry
        // in both pipelines, so wgpu keeps it bound across the switch
        // (`bind.rs:213-237`); groups 2 and 3 are outside the snapshot's
        // layout and are re-expected, not rebound, when the brush
        // pipeline returns.
        snap.dispatch(&mut pass, flush, i as u32);
        pass.set_pipeline(&per_brush.pipeline);
    }
    pass.set_bind_group(1, &dabs_bind_group, &[(i as u64 * INDEX_STRIDE) as u32]);
    pass.dispatch_workgroups(groups(meta.grid[0]), groups(meta.grid[1]), 1);
}

Per dab for a live-sampling brush: two `set_pipeline`, two
`set_bind_group(1)`, two dispatches. The performance section states this
count and step 12 records it.
```

A brush that does not read the live stroke runs the loop exactly as
today. `record_dispatches` counts every dispatch the flush issues, snapshots
included: `2 * total_dabs` for a live-sampling brush, `total_dabs`
otherwise. The field's doc (`gpu_context.rs:75-78`) changes from "into
the scratch" to "per flush", so the bench's `dispatches/ev` shows the
doubling the risk section cares about rather than hiding it.
`tests/paint_compute.rs::dispatches_count_one_per_dab` keeps pinning the
Ink Pen at one per dab, and a sibling pins the Dry Smudge at two.

**`commit`**: unchanged, through the shared slot mapping.

### 7. The sampler's WGSL (live arm)

Emitted into `decls` once per node instance. It references only `u` and
the group-3 texture, both declared in the preview module too, so the
shared-decls rule holds; the preview body simply never calls it.

```wgsl
fn sample_live_{id}(tp: vec2<f32>, motion: vec2<f32>) -> vec4<f32> {
    // A dab that has not moved has no "one dab ago" to read: Krita's
    // first-run rule (kis_colorsmudgeop.cpp:196-199).
    if (abs(motion.x) < STATIONARY && abs(motion.y) < STATIONARY) {
        return vec4<f32>(0.0);
    }
    // The appearance mirror is layer-sized in the paint target's frame,
    // the frame the intrinsic header carries.
    let src = tp - motion;
    let lo = vec2<f32>(u.intrinsic.layer_offset);
    let lsz = vec2<f32>(u.intrinsic.layer_size);
    let uv = (src - lo) / lsz;
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
        return vec4<f32>(0.0);                              // off the layer
    }
    // `graph_smp` repeats. Within half a texel of the border the filter
    // would wrap to the opposite edge, whose mirror texels this dab never
    // refreshed; clamp to texel centres so it never leaves the texture.
    // Shared with the Snapshot arm, which had the same latent wrap on the
    // frozen pre-stroke snapshot.
    let half = vec2<f32>(0.5) / lsz;
    let uv_c = clamp(uv, half, vec2<f32>(1.0) - half);
    return textureSampleLevel(graph_tex_{slot}, graph_smp, uv_c, 0.0);  // straight RGBA
}
```

`STATIONARY` is the `{:.6}` literal of
`crate::brush::paint_info::STATIONARY_MOTION_PX` (new, `0.5`, beside the
`motion` field it qualifies; `smudge.rs:37` imports it and drops its own
copy), so the CPU early-out in smudge and the GPU rule here share one
number. Body: `let clone_c_{id} = sample_live_{id}(target_pos, {motion_expr});`
with `motion_expr = cctx.input("motion").as_vec2()` (a `d.n{pen}_motion`
dab field when wired, a literal when not). The output name stays
`clone_c_{id}` so the preview body's substitution keeps working.

Inside the layer, every texel the filter touches lies inside the region
the snapshot refreshed for this dab: the read region is
`pos +- (bbox + |m| + 1)` and `src` is within `pos +- (bbox + |m|)`. At
the layer border the read region is clamped to the layer, so that margin
does not exist there, and `Repeat` on `graph_smp` would reach the opposite
edge, where the mirror holds whatever this dab did not refresh: zero at
stroke start (wgpu zero-initialises), a discarded dab's appearance after a
rewind. The clamp to texel centres in the helper is what keeps that from
ever being read, and test 10 pins it at both origins. Written once, in
the helper's comment. The sample is straight alpha (the commit's output) and
is filtered as straight alpha, the same convention the Snapshot arm uses
on the pre-stroke snapshot; `stamp` premultiplies it.

Selection: untouched. `paint`'s body multiplies by `sel`, so a feathered
selection scales the deposit as it does for every paint brush.

Out of layer: transparent, as the Snapshot arm does. Through `stamp` that
is a zero-alpha dab and `paint`'s body returns before touching a ground.

### 8. Stationary dabs

The rule lives in the sampler's shader (above), and the terminal still
queues and snapshots the dab. Reasons: (a) the sampler defines what "the
stroke one dab ago under this pixel" means when there is no previous dab,
and Krita puts the rule in the same place (the op, not the compositor);
(b) the sampler cannot drop the *terminal's* dab without also dropping
whatever else the graph deposits in it (a future colour-rate mix), so it
zeroes its own contribution and `paint`'s existing
`if (rgba.a * sel == 0.0) { return; }` (`paint.rs:883`) leaves the grounds
untouched; (c) what a stationary live sample would otherwise do: on an
opaque canvas, depositing the appearance over itself is an identity under
source-over and, at commit, under the ceiling (`d == 0` for a pigment
equal to the pixel, so `t == 0`), but on a partially transparent pixel the
build law raises alpha by `k * (1 - pre.a)` per dab, a visible dot at
every stroke start; (d) stationary dabs are rare (the first dab of a
stroke, a full-reset replay), so the wasted snapshot dispatch costs
nothing measurable, and the `smudge` terminal's CPU early-out stays where
it is.

### 9. Cursor preview

Per `docs/brush-preview-and-overlays.md`: the preview pipeline binds the
registry `_fallback` tile to every unpublished live slot
(`texture_registry.rs:214-233`), and a non-terminal cannot render
differently at hover except through `compile_cursor_preview_body`.
`clone_source` already overrides it with a neutral grey fill that samples
nothing (`clone_source.rs:253-262`); the live arm shares that body. The
`StrokeAppearance` slot is declared in the preview module and never read.
The Dry Smudge hover shows the tip shape in grey through the grain, which
is what Krita's smudge outline conveys. Test 7.

### 10. Erase, growth, rewind

**Erase.** The snapshot is the paint-mode appearance whatever
`gpu.blend_mode` says, and the commit removes coverage where the
paint-mode smear would land, through `destination_out`: `paint`'s existing
contract, nothing new, and nothing in this plan analyses how that chain
feels. Krita's smudge has no erase mode and neither does the `smudge`
terminal (`smudge.rs:96`); the browser smoke in step 11 is where the feel
is judged. `supports_erase: true` stays because the contract is paint's.

**Mid-stroke layer growth.** `gpu_stroke_to` grows the layer before any
dab of the event renders (`painting.rs:504-508`), `StrokeBuffer::grow_preserving`
rebases the scratch and the pre-stroke snapshot and `Scratch::grow_write`
reallocates the appearance mirror at the new size; within a flush the
paint target's extent is stable, the intrinsic header is packed per flush
from it, and every read region is translated against the same extent the
grid is. The mirror needs no copy: each dab's reads were refreshed for
that dab.

**Rewind.** A rewind restores the grounds through the ring; the next
dab's snapshot reads the restored grounds, so the chain resumes from the
checkpoint's pixels. `RenderCheckpoint::last_dab_pos` restores the motion
tracker, so the first replayed dab's motion is the true delta; only a
full reset zeroes it, which is the stroke's first dab, which the
stationary rule handles. The ring snapshots the scratch and its channels
and never sees the mirror. Test 8 pins determinism and the rewind oracle.

### 11. Engine gates

`DarklyEngine::active_brush_needs_source` (`painting.rs:349-360`) and
`clone_source_port_default` (`:367-378`) both name `clone_source::TYPE_ID`
today. Rather than deepen that from `painting.rs`, `clone_source.rs` gains
`pub fn graph_needs_source(graph: &Graph) -> bool` beside
`source_default_is_live` ("is there a sampler whose `source` is
Snapshot"), and `painting.rs` calls it. The type id is then named only in
the node's own file, which is as far as this step goes before the rename
follow-up (open question 5). The frontend keeps calling
`activeBrushNeedsSource()`. `samples_source()` is already correct because
`StrokeAppearance` is not `StrokeSnapshot`. `CloneState` seeding is
unchanged: the live arm declares no anchor uniforms, so a stale anchor
from an earlier clone stroke injects keys no packer declares.

## Coordinate frames

Per `docs/coordinate-systems.md`. Every value the live arm and the
snapshot touch:

| Value | Frame | Produced by | Consumed as |
|---|---|---|---|
| `pen_input.position`, `motion` | plane px (sensed) | `stroke_engine::place_dab` | the sampler uses `motion` as a plane offset, never converted |
| `target_pos` | plane px, pixel centre | compute skeleton | `src = target_pos - motion` |
| `u.intrinsic.layer_offset`, `layer_size` | plane rect of the paint target | `paint::flush_dabs` via `intrinsic_header` | `uv = (src - offset) / size` into the layer-sized mirror; the off-layer test |
| `read_reach` | plane px (raster padding) | sampler `read_reach` from `motion` | added to `bbox_radius`, which already crossed the reference boundary in `effective_radius` |
| read region | plane rect, then write-side texels | `paint::evaluate_gpu` via `clamp_f32` (floor near, ceil far, intersect the extent), translated by the extent's origin | `SnapshotRecord { origin, size }`: the snapshot's grid and the texel it writes |
| snapshot `px` | write-side texel | `records[slot.i].origin + gid.xy` | `textureLoad` of both grounds and the pre-stroke, `textureStore` of the mirror |
| selection | window-local UV | `plane_to_selection_uv` in the skeleton | `sel`, unchanged |

Nothing here is authored in reference pixels: `motion` is sensed and the
reach is raster padding, so `dpi_factor` does not enter. The grain chain's
`noise.scale` is reference pixels and converts inside
`frame_sample_coord_expr` as it does for the Pencil. Tests run once at
`canvas_origin (0, 0)` and once with a cropped canvas and an offset layer
(test 3), the way `tests/clone.rs::clone_copies_under_nonzero_origin_and_offset_layer`
does.

## Implementation steps

No production code changes before step 5 of the workflow. Order for that
step; each line names the files it touches.

1. `shaders/lib/commit_law.wgsl` (new), `shaders/brush/composite.wgsl`
   (call it), `brush/composite_pipeline.rs` (include it). Full suite: no
   pixel moves (`tests/brush_accumulation.rs`, `tests/paint_target.rs`).
2. `brush/node.rs` (`DabPass::can_refresh_between_dabs`);
   `brush/texture_source.rs` (`StrokeAppearance`, `refreshed_per_dab`,
   remove `PickupAtlas`, labels, docs); `brush/wgsl/mod.rs`
   (`reads_stroke_appearance`, the compile-time check, docs);
   `brush/wgsl/context.rs` (body doc: the appearance mirror as a graph
   texture); comment corrections listed in section 2.
3. `brush/paint_info.rs` (`STATIONARY_MOTION_PX`); `nodes/smudge.rs`
   (import it, delete the local constant).
4. `brush/eval.rs` (`read_reach` hook; runner fold, reset, hand-off;
   `begin_stroke` ensures the mirror; `flush_dabs` gathers from slots);
   `brush/gpu_context.rs` (`DabBatch::read_reach`, docs).
5. `brush/scratch.rs` (the appearance mirror: field, ensure, view, grow,
   docs); unit test beside the existing scratch tests.
6. `brush/appearance_snapshot.rs` (new) and
   `shaders/brush/appearance_snapshot.wgsl` (new); `brush/pipeline.rs`
   (`plumbing_registrations`). `tests/shader_compile.rs` parses the file.
7. `nodes/paint.rs` (`PaintDabMeta`, the read region in `evaluate_gpu`,
   `slots`, the snapshot in `flush_dabs`, the publish, module docs).
8. `nodes/clone_source.rs` (ports, visibility, `source_default_is_live`,
   the live arm, `read_reach`, display name, docs, unit tests).
9. `engine/painting.rs` (`active_brush_needs_source` consults `source`).
10. `brushes/dry_smudge.yaml`, `packs/dry_media.yaml`
    (`members: [pencil, charcoal, dry_smudge, hair, sponge]`);
    `builtin_brushes.rs` icon row.
11. Tests (section below), then `make wasm`, the frontend and desktop
    gates, and a browser smoke: paint with the Dry Smudge over a Pencil
    mark, erase with it, under a selection, across a layer edge, hover.
12. `bin/stroke_replay_matrix.rs`: a `dry-smudge` topology (brush "Dry
    Smudge", terminal `paint`). Bench per the performance section and
    record the rows in the perf doc.
13. Docs: `docs/brush/architecture.md` (`paint` terminal: the appearance
    mirror; terminals list; "Warp brushes" note that a live sampler on
    `paint` is the shape to prefer over a new read-mirror terminal),
    `docs/brush-preview-and-overlays.md` (one sentence: the sampler's live
    arm follows the clone rule), `docs/paint-compute-perf-tracking.md`
    (attempt #9; section E rewritten: chained reads on `paint` go through
    the appearance mirror, and the `smudge` / `blur` deletion is the
    follow-up), `README.md` roadmap unchanged (Smudge is ticked).
14. `cargo sync-docs` (no region lists nodes or brushes; the run confirms),
    then the full Lint / CI Checks list from `CLAUDE.md`.

## The brush YAML

`crates/darkly/brushes/dry_smudge.yaml`:

```yaml
name: Dry Smudge
description: "A fingertip rubbed through graphite and charcoal. Pulls the pigment already on the paper along the stroke, through the same tooth the pencil laid it down with, and leaves nothing of its own."
nodes:
  pen_input:
    type: pen_input
  brush_settings:
    type: brush_settings
    comment: The Pencil's cadence. Each dab moves the pigment by exactly its own step, so the smear is continuous at any spacing; 0.03 keeps the dab count of the medium it smudges
    inputs:
      size: 0.1
      spacing: 0.03
      stabilize: 0.4
  circle:
    type: circle
    name: Fingertip
    inputs:
      softness: 0.6
  noise:
    type: noise
    name: Paper grain texture
    comment: Canvas space, so the smear is broken by the sheet's tooth and not by a per-dab pattern
    inputs:
      roughness: 0.548843264579773
      scale: 2.5
  levels:
    type: levels
    name: Grain intensity
    inputs:
      in_high: 0.40818285942077637
  multiply:
    type: multiply
    name: Combine grain with tip
  curve:
    type: curve
    name: Squash pen pressure
    comment: A light touch still smears; pressure mostly widens the fingertip
    inputs:
      curve:
      - - 0.0
        - 0.35
      - - 1.0
        - 1.0
  strength:
    type: user_input
    name: Strength
    comment: One dial for both halves of the dial, the way the Pencil wires pressure into both flows
    inputs:
      value: 0.6
  sampler:
    type: clone_source
    name: Live canvas
    inputs:
      source: 1
  stamp:
    type: stamp
  paint:
    type: paint
    inputs:
      buildup: 0.1
connections:
- pen_input.position -> paint.position
- pen_input.motion -> sampler.motion
- pen_input.pressure -> curve.input
- curve.output -> paint.size
- noise.value -> levels.input
- levels.output -> multiply.a
- circle.mask -> multiply.b
- multiply.result -> stamp.tip
- sampler.color -> stamp.color
- stamp.dab -> paint.rgba
- strength.value -> paint.wash_flow
- strength.value -> paint.build_flow
exposed_ports:
  brush_settings.size: {}
  brush_settings.stabilize: {}
  strength.value:
    label: Strength
    description: How much of what is under the finger moves with each touch
    icon: mdi:gesture-swipe
  levels.in_high:
    label: Grain
    description: How much the paper's tooth breaks up the smear
    icon: tabler:grain
    invert: true
  paint.buildup:
    label: Build-up
    description: How much rubbing the same spot keeps moving pigment, 0% = one pass moves it once, 100% = every touch moves more
    icon: fa6-solid:layer-group
  paint.opacity: {}
```

Notes on the choices:

- The grain chain is the Pencil's `noise_2 -> levels -> multiply(circle) -> stamp.tip`
  core (`pencil.yaml`), without the Pencil's pressure-and-roughness curves
  feeding `levels`, since the smudge exposes grain as one dial.
- `buildup: 0.1` matches the Pencil so the two read as one medium: a pass
  moves the pigment once, rubbing the same spot at the same pressure moves
  little more, pressing harder moves more. The dial is exposed because the
  user's requirement is that it be free; at 100% the smudge compounds per
  dab like Krita's and like the `smudge` terminal. The wash law's exact
  idempotence (`compute-paint-terminal.md` 4.5.1) holds for an identical
  pigment, and a smudge's sampled pigment changes under every dab, so
  test 5 pins the law with a stable source and the smear's quality at 10%
  is judged by the browser smoke, not by a test. Open question 2.
- `strength` is a `user_input` fanned into both flows (`docs/brush/node-system.md`,
  "Fan-out"), default 0.6, the `smudge` terminal's default `rate`.
- `spacing: 0.03` is the Pencil's. The old draft chose 0.04 to cut the
  dab count of a serialized render-pass path; a dab is now two dispatches
  at about 1.5 us marginal each on the test iGPU, so the cadence follows
  the medium instead. The smear is continuous at any spacing, because each
  dab displaces by exactly its own step; what spacing changes is how many
  dabs stack on a pixel, which the dial governs.
- `paint.opacity` is exposed like every other paint brush: the snapshot is
  the full-strength appearance and the commit applies the cap once, so 50%
  is `mix(pre, smear, 0.5)`, the meaning it has on the Pencil (section
  "What the snapshot holds"). `sampler.center` is not wired: the live arm
  ignores it.
- The node id `sampler` is deliberately not `clone_source` so the node's
  role reads correctly in the editor; nothing keys on instance ids.
- `content_dependent_brushes_get_preview_icons` (`builtin_brushes.rs:253-277`)
  gets a row: `graph_capabilities` takes the first staged registration's
  icon, so the Dry Smudge shows `fa6-solid:clone` until the rename
  follow-up gives the sampler a per-arm icon. Accepted for this step.

## Tests

All GPU tests under `--features darkly/testing -- --test-threads=1`.

New file `crates/darkly/tests/live_sampler.rs`, harness modelled on
`tests/smudge.rs:66-190` (runner level: explicit dabs with explicit
`motion`, one `flush_dabs`, `commit`, readback) with a `two_tone_canvas`
and a builder that loads `dry_smudge.yaml` from `builtin_brushes::all()`
and can strip the grain chain (wire `circle.mask` straight into
`stamp.tip`) for the exact tests.

1. **`live_sampler_compiles_beside_baked_noise_and_image`** (no GPU): the
   Dry Smudge graph plus an `image` node wired into a second `multiply`
   on the tip compiles; `compiled.graph_sources` holds a `Baked`, a
   `Named` and `Live(StrokeAppearance)`; `reads_stroke_appearance()` is
   true; `stroke_wgsl` contains `textureSampleLevel(graph_tex_` for the
   live slot and `cursor_preview_wgsl` does not call the live helper.
   Companions: the Clone brush and the analytic disc fixture report
   `false`. A copy of the graph re-terminated in `smudge` fails
   `compile_graph` with an error naming the live stroke appearance and
   the instanced pass.
2. **`sampler_reads_through_the_snapshot_not_the_racing_ground`** (the
   hazard test): an opaque pre-stroke canvas holding a horizontal ramp
   (`r = x`), a hard-edged disc (`circle.softness = 0`), `buildup = 1`,
   `build_flow = 1`, one dab at `(64, 64)` with motion `(3, 0)` and radius
   20. With `sampled.a = 1` and `tip = 1` the deposit replaces the ground
   outright, so the committed layer inside the disc must equal the ramp
   shifted right by exactly three pixels, bit for bit against a CPU loop
   over the disc (integer motion makes the bilinear filter a texel fetch;
   no shape math is involved). A second dab at `(70, 64)` with the same
   motion then reads inside the first dab's footprint and the overlap
   must show a six-pixel shift where both dabs covered it. This is the
   feature test for the snapshot's ordering (an exact shift is only
   possible if every read saw the pre-dab ground); it is not a regression
   test, since no variant without the snapshot is built. Run at origin
   `(0, 0)` and at a non-zero `canvas_origin` with an offset layer.
3. **`second_dab_reads_first_dabs_deposit`**: two overlapping dabs in one
   flush on the two-tone canvas, motion `(+30, 0)`, dab 2's sample point
   inside dab 1's write footprint; the pixel under dab 2 carries red that
   a control render (dab 2 alone) does not. The same shape as
   `tests/smudge.rs::smudge_dab2_reads_dab1_deposit_not_pre_stroke`.
4. **`matches_the_smudge_terminal_at_full_buildup`** (equivalence): the
   same three-dab sequence through the shipped Smudge (`rate = 0.6`,
   `opacity = 1`, `circle.softness = 0.4`, pressure 1) and through a
   sampler graph with the identical `circle`, no grain, both flows 0.6,
   `buildup = 1`, on an opaque two-tone canvas, every dab moving by at
   least `STATIONARY_MOTION_PX` (the two paths treat a stationary dab
   differently: dropped before the queue versus a zero deposit). Every
   pixel agrees within `+-6/255` per channel. Tolerance, derived: the
   smudge side rounds a straight-alpha RGBA8 scratch once per dab (half an
   LSB); the paint side rounds the appearance to RGBA8 in the snapshot
   (half an LSB) and the premultiplied `rgb` and `a` of the ground
   separately (up to one LSB between them when the ground is partially
   covered), so each dab contributes up to 1.5 LSB against 0.5, a
   differential of one LSB per dab; three dabs, bilinear filtering of
   differently rounded values on both sides, and the commit's one rounding
   bound it at five, and six is that bound rounded up. If a driver's
   rounding exceeds it, assert on dab interiors (`tip >= 0.9`) rather than
   widening.
5. **`wash_refuses_a_repeated_smear_like_the_pencil`** (the dial): a white
   canvas with a black bar; two dabs at the same position with the same
   motion, `|m|` larger than the footprint so both sample the untouched bar
   and carry the same pigment at the same coverage. At `buildup = 0` the
   second dab changes no byte of the committed layer (the idempotence
   argument of `docs/plans/compute-paint-terminal.md` 4.5.1, exact for a
   black pigment); at `buildup = 1` some pixel's alpha-weighted darkness
   strictly rises. The regression net for "the smudge obeys paint's laws".
   Runner level with explicit `motion`, so the stationary rule does not
   fire on the repeated dab.
6. **`stationary_dab_deposits_nothing`**: one dab with `motion = (0, 0)`
   at `buildup = 1` on a transparent layer holding a soft-edged mark,
   authored with `rgb = 0` wherever `a = 0`. Alpha is compared exactly and
   `rgb` only where `a > 0`: the commit's `source_over.wgsl:15-19` zeroes
   `rgb` under `a <= 0.001` and divides through `a` elsewhere, so
   "bit-identical" is not what a zero deposit produces on a layer with
   junk colour under transparent texels. Without the rule alpha rises
   (`app OVER app`), so the test fails against a sampler that samples at
   zero motion. Regression for the first-dab alpha dot.
7. **`hover_preview_is_a_neutral_footprint`**: modelled on
   `tests/preview_smudge.rs`; the Dry Smudge preview has non-zero alpha
   inside the tip, grey RGB, and never NaN. `tests/wgsl_validate.rs`
   validates both modules of the new builtin under naga without edits.
8. Engine level, in `tests/stroke_rewind.rs` and `tests/paint_compute.rs`:
   a `Cell { brush: "Dry Smudge", .. }` row in
   `recorded_stroke_rewinds_match_full_rerender`'s oracle, with the
   stripe laid down first (the `Cell` harness at `:95-133` starts from a
   blank raster layer; `tests/builtin_brushes_stroke.rs::stripe` shows
   how), so the appearance mirror through checkpoint restores and layer
   growth agrees with a full re-render; and a two-engine replay of
   `recorded_curvy_stroke.json` at `stabilize = 1.0` whose readbacks are
   byte-identical and differ from the pre-stroke layer, the shape of
   `replay_is_deterministic_through_checkpoints_and_paints`. In
   `tests/paint_compute.rs`, a sibling of `dispatches_count_one_per_dab`
   pins the Dry Smudge at two dispatches per dab.
9. Unit tests: `clone_source.rs` (`registration_shape` for the two new
   ports and their visibility; `source_default_is_live` at 0, 0.5, 1;
   `read_reach` is `ceil(|m|) + 1` per axis in live mode and zero in
   snapshot mode); `scratch.rs` (`ensure_appearance_mirror` allocates a
   layer-sized `Rgba8Unorm` with `STORAGE_BINDING | TEXTURE_BINDING`,
   frees when unwanted, and survives `grow_write` at the new size);
   `texture_source.rs` (`refreshed_per_dab` per variant);
   `builtin_brushes.rs` (the icon row).
10. **`border_dab_never_reads_the_opposite_edge`**: one dab whose read
    region straddles a layer edge, on a layer whose opposite edge holds a
    saturated colour the dab's side does not, committed and compared
    against a CPU loop (the test 2 shape at the border), at both origins.
    Without the texel-centre clamp the bilinear filter wraps through
    `Repeat` and the edge pixels pick up the opposite edge's colour.
11. **`opacity_is_a_commit_time_cap`**: the Dry Smudge at `opacity = 0.5`
    equals the same stroke at `opacity = 1` mixed with the pre-stroke
    layer on the CPU within 1 LSB, the contract every paint brush has. A
    snapshot that applied opacity would fail it (the chain would be
    scaled twice).
12. Existing suites as regression nets: `tests/clone.rs` (the Snapshot
    arm is untouched), `tests/smudge.rs`, `tests/blur.rs`,
    `tests/liquify.rs`, `tests/brush_accumulation.rs` (paint's laws and
    the commit through the extracted `commit_law`), `tests/paint_compute.rs`,
    `tests/brush_erase.rs`, `tests/builtin_brushes_stroke.rs::every_builtin_deposits_or_moves_pixels`
    (picks up the new YAML automatically), `tests/brush_packs.rs` (the
    pack lists every member), `builtin_brushes::tests::*` (the catalog
    covers the new stem), `tests/wgsl.rs` (no paint shape changes).

## Risks

- **The browser's per-call cost.** The dispatch count doubles and every
  dab now switches pipelines twice and rebinds group 1 twice (six encoder
  calls per dab against two today); at a thousand dabs per event that is
  several thousand encoder calls per event through the browser's GPU
  process, a cost the paint port's own risk list already names as
  unmeasured. The native bench in the performance section is
  what this plan can measure; a browser replay harness remains the
  follow-up the paint port listed.
- **Per-pixel cost of the snapshot.** Each read-region thread loads two
  grounds and the pre-stroke, runs the ceiling arithmetic and stores:
  about the dab's own cost again, over a slightly larger region. Small
  dabs are CPU-bound and will not show it; the 2560x1440 row at 2000 px
  is where it shows, and that regime is already the compute terminal's
  weak one on this iGPU.
- **VRAM.** One layer-sized RGBA8 mirror per stroke of a live-sampling
  brush, freed with the stroke buffer. Open question 3 if it matters.
- **Tint.** Write-only `rgba8unorm` storage in a compute module and a
  storage-then-sample sequence inside one pass are core WebGPU; the smoke
  in step 11 is where the web confirms it, as it did for the compute
  skeleton. Fallback if a browser objects to the in-pass sequence: close
  and reopen the compute pass per dab (two passes per dab, no other
  change), and bench that.
- **Straight-alpha filtering.** The mirror is sampled bilinearly as
  straight alpha, so at a transparent layer's edges the filter mixes
  colour from zero-alpha texels, the same fringe the Snapshot arm has on
  the pre-stroke snapshot. `commit_law` zeroes `rgb` under `a <= 0.001`,
  which keeps the fringe dark rather than random. Recorded; not fixed
  here.
- **Test 4's tolerance.** If it proves tight on a driver with different
  rounding, assert on interiors as the test says; do not relax test 2.

## Open questions

1. Resolved by review: the snapshot is the full-strength, paint-mode
   appearance; opacity and blend mode stay at the commit (section "What
   the snapshot holds"), and `paint.opacity` is exposed on the Dry Smudge.
2. **The shipped dial value.** `buildup: 0.1` to match the Pencil
   (rubbing the same spot at the same pressure moves little more) versus
   `1.0` to compound per dab like Krita's smudge and the `smudge`
   terminal. The dial is exposed either way; this is only the default.
3. **A layer-sized mirror.** Chosen for simplicity (no per-dab origin,
   no bind group that changes on grow, the sampler's frame is the
   intrinsic one). Costs a layer-sized RGBA8 texture per live-sampling
   stroke. If that is too much on large documents, the footprint-sized
   variant with a `SnapshotRecord` origin the sampler also reads is the
   fallback, at roughly 60 more production lines.
4. Resolved by review: every dispatch is counted, so `dispatches/ev` is
   `2 * dabs/ev` for the Dry Smudge (section 6).
5. **Rename `clone_source` to `canvas_sampler`** (type id, file, engine
   query names, the frontend's gesture module, `clone.yaml`) and give the
   live arm its own picker icon. Mechanical; its own follow-up.

## Performance

Read `docs/paint-compute-perf-tracking.md` first: attempt #6 is the
compute `paint` terminal this plan builds on, "#5, stage 1" is where a
dispatch inside a pass was measured, section E is the old framing of
chained reads (one render pass plus one copy per dab, about 40 us), and
section F is the shape that replaced it. The old draft's smudge baseline
(`bench-results/stroke-replay-matrix-smudge-recorded_curvy_stroke-d79ed0f518.md`,
the read-mirror `smudge` terminal, this machine) fell behind at every
canvas at or above 1080p at 1 to 10 px and at the largest dabs on 1440p
and up; it is not re-run.

**Expected per-dab cost of the new path, against `paint`'s measured rows
(`bench-results/stroke-replay-matrix-paint-compute-after-recorded_curvy_stroke-929928f9ab.md`):**

- Two dispatches per dab instead of one. Stage 1 measured about 1.5 us
  wall and 1.5 us GPU marginal per dispatch, with the inter-dispatch
  barrier free; so about 3 us of dispatch overhead per dab on this
  machine, plus two `set_pipeline` and one extra `set_bind_group` per dab
  (group 0 is shared across the switch, groups 2 and 3 are re-expected),
  unmeasured in any harness but of the same order. Six encoder calls per
  dab against two today; step 12 records the count.
- The snapshot's grid is the read region, `(2r + 2|m| + 2)^2` texels
  against the footprint's `(2r)^2`; at spacing 0.03 that is a few percent
  more threads than the dab's own, each doing two packed loads, one
  background load, the ceiling arithmetic and one store: about the dab's
  own per-pixel work again.
- The sampler adds one bilinear texture read and a branch per footprint
  thread.
- Per event, the commit is unchanged. Not per dab and not in `paint`'s
  rows: the Dry Smudge samples the baked grain tile per thread, as the
  Pencil does.

So at small radii, where `paint` is CPU-bound at 6 to 7 ms `cpu p50` for
490 dabs per event at 1080p, the prediction is that the Dry Smudge lands
within the bench's noise band or a few milliseconds above it, from the
doubled encoder traffic; at the 2560x1440 row at 2000 px, where the
compute terminal is already bandwidth-bound, roughly twice `paint`'s GPU
time per dab.

**The browser is unmeasured.** Every production pixel goes through the
`webgpu` backend, where a `setBindGroup`, `setPipeline` or
`dispatchWorkgroups` costs whatever the browser's GPU process charges; the
paint port recorded that as unmeasured, and this plan doubles the dispatch
count and adds pipeline switches on top. The native bench cannot answer
it; the browser replay harness the paint port listed as a follow-up is
where it would be answered.

**Step 12 benches, rather than predicts:** with the `dry-smudge` topology
in `stroke_replay_matrix`, run at least the 1920x1080 rows at radius 1,
10 and 100 and the 2560x1440 row at 2000, same machine and session as a
fresh `paint` run, and record them in the perf doc as attempt #9 next to
`paint`'s rows, with `dispatches/ev == dabs/ev` and the snapshot count
stated as equal to it.

```bash
cargo run --release -p darkly --features testing --bin stroke_replay_matrix -- \
    --input crates/darkly/tests/fixtures/recorded_curvy_stroke.json --topology paint
cargo run --release -p darkly --features testing --bin stroke_replay_matrix -- \
    --input crates/darkly/tests/fixtures/recorded_curvy_stroke.json --topology dry-smudge
```

No hybrid, no shader-shape work: the row loop and the workgroup shape are
`paint`'s knobs and stay where attempt #6 left them.

## LOC estimate

Lines added or removed, honest ranges.

| Area | Added | Removed | Files |
|---|---:|---:|---|
| `shaders/lib/commit_law.wgsl` (new), `composite.wgsl`, `composite_pipeline.rs` | 60 | 40 | 3 |
| `node.rs` (`can_refresh_between_dabs`) | 12 | 0 | 1 |
| `texture_source.rs` (variant, `refreshed_per_dab`, remove `PickupAtlas`, docs) | 25 | 12 | 1 |
| `wgsl/mod.rs`, `wgsl/context.rs` (`reads_stroke_appearance`, the check, docs) | 45 | 8 | 2 |
| `paint_info.rs` (threshold), `smudge.rs` (import) | 10 | 6 | 2 |
| `eval.rs` (hook, fold, hand-off, mirror ensure) | 35 | 3 | 1 |
| `gpu_context.rs` (`read_reach`, docs) | 12 | 4 | 1 |
| `scratch.rs` (appearance mirror: field, ensure, getter, grow branch, create) | 60 | 2 | 1 |
| `appearance_snapshot.rs` (new, on `make_uniform_ring` / `DynamicUniformRing`), `appearance_snapshot.wgsl` (new), `pipeline.rs` | 205 | 0 | 3 |
| `paint.rs` (meta, read region, slots, snapshot loop, publish, dispatch count, docs) | 90 | 25 | 1 |
| `clone_source.rs` (ports, live arm with the border clamp, reach, `graph_needs_source`, docs) | 125 | 15 | 1 |
| `painting.rs` (`active_brush_needs_source` calls the predicate) | 4 | 6 | 1 |
| `dry_smudge.yaml`, `dry_media.yaml` | 85 | 1 | 2 |
| `stroke_replay_matrix.rs` (topology) | 12 | 0 | 1 |
| **Production total** | **~760** | **~125** | 21 |
| `tests/live_sampler.rs` (harness + tests 1 to 7, 10, 11) | 530 | 0 | 1 |
| `tests/stroke_rewind.rs`, `tests/paint_compute.rs` (test 8, dispatch count) | 70 | 0 | 2 |
| unit tests (`clone_source.rs`, `scratch.rs`, `texture_source.rs`, `builtin_brushes.rs`) | 70 | 4 | 4 |
| **Tests total** | **~670** | **~4** | 7 |
| `docs/brush/architecture.md`, `docs/brush-preview-and-overlays.md`, perf doc | 120 | 15 | 3 |
| bench results (generated by the bench) | 60 | 0 | 2 |
| **Generated / docs total** | **~180** | **~15** | 5 |
| **Grand total** | **~1610** | **~145** | |

About 635 net production lines, of which the snapshot pipeline and its
shader are about 205 (the one genuinely new mechanism, reusable by every
future live reader, and mostly wgpu boilerplate), the sampler's live arm
about 110, and the mirror on `Scratch` about 60. Nothing is extracted from the read-mirror terminals,
which stay as they are until their deletion.

## Out of scope (later steps, per the brief)

A per-fragment `displacement` port scaling the motion vector; a "lift"
channel and a displace/duplicate dial; deleting the `smudge` and `blur`
terminals (the follow-up this capability enables, already in the perf
doc's "deleted later" list); anything watercolor; the `clone_source`
rename; a browser replay harness.
