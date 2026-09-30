## Independent Review

Reviewed against the working tree at `d79ed0f5` (dirty), the Krita checkout under
`krita/`, and the bench output under `crates/darkly/bench-results/`. Every file
and line reference in the plan was resolved; the ones that are off are listed
under "Reference accuracy". Findings are ordered by weight.

### Verdict summary

The diagnosis is right, the architecture follows the brief's direction (a
canvas-sampling node feeding `paint`, with the `smudge` terminal slated for
deletion later), authority and ownership are clean (nothing new is document
state; the chained flag is compile-derived; the read region stays with the
module that already owns it), and the extraction from `read_mirror_terminal.rs`
is a real extraction as long as step 3 rewrites `rmt::evaluate_gpu` /
`rmt::flush_dabs` on top of the two helpers, which the plan commits to. The
equivalence claim in "What a dab does" is correct: with `stamp` premultiplying
(`crates/darkly/src/brush/nodes/stamp.rs:59-63`) and `paint` at `buildup = 1`
writing the scratch under `PREMULTIPLIED_SOURCE_OVER`
(`crates/darkly/src/brush/nodes/paint.rs:769-772`), `appearance_n = k * sampled
+ (1 - k) * appearance_{n-1}` with `k = tip * build_flow`, which is
`smudge.rs:146-149`'s `mix(bg, src, rate * mask)` on an opaque canvas.

What needs revising is one API that does not exist, two tests whose stated
bounds are wrong, one exposed port that produces a double image, and several
places where the plan gives a secondary reason where the structural one should
be stated. None of it changes the approach.

### 1. `wgpu::Texture::global_id()` does not exist in wgpu 29 (section 5, risk 2)

The chained flush rebuilds `@group(3)` when `stroke.scratch.read_mirror_texture()`
changes identity and keys that on `mirror.global_id()`. wgpu is pinned at 29
(`Cargo.toml:37`); `wgpu-29.0.4/src/api/texture.rs:11-19` declares `Texture` as
`#[derive(Debug, Clone)]` plus `impl_eq_ord_hash_proxy!(Texture => .inner)`.
There is no `global_id` (nothing in `crates/darkly/src` calls one either).
Compare with `==` against a cloned handle, or better, have
`for_each_mirrored_dab` report whether `prepare_dab_canvas_copy` grew the mirror
(`Scratch::sync_read_mirror`, `crates/darkly/src/brush/scratch.rs:452-454`,
already knows) and rebuild on that signal. The second shape also retires risk 2
("if a future `Scratch` change recycles the same texture object"), since the
grow is the event, not the identity.

### 2. Stroke opacity: do not expose `paint.opacity` on the Dry Smudge (section 7, open question 2)

The plan applies opacity at commit, so at 50% the layer shows
`mix(pre, smudged, 0.5)`: the un-displaced pigment and its displaced copy, both
at half strength. For a smear that is a double image, not a weaker smudge.
Krita applies opacity per dab (`krita/plugins/paintops/colorsmudge/kis_colorsmudgeop.cpp:204`,
`fpOpacity`, passed into the strategy at `:219`), and so does the `smudge`
terminal (`smudge.rs:148`): both read as "a shorter smear". So the prior art
supports the per-dab side of open question 2, not the plan's side. Commit-only
is still the right contract for `paint` ("Stroke-level opacity cap (applied at
commit)", `paint.rs:431`), and the plan should not special-case it; the fix is
in the YAML: drop `paint.opacity: {}` from `exposed_ports` so the only strength
control the artist sees is `build_flow` ("Strength"), and record in section 7
that opacity was deliberately not exposed because commit-time opacity ghosts a
smear. The equivalence test at opacity 1 remains meaningful either way: it pins
the chain, which is the feature.

### 3. Test 4's tolerance bound is derived wrongly (Tests, item 4)

"Both paths quantize the scratch to 8 bits once per dab ... three half-LSB
roundings per channel" undercounts the paint side. Its scratch stores
premultiplied `rgb` and `a`, both rounded, and the sampler reconstructs
`appearance = scratch.rgb + pre.rgb * (1 - scratch.a)`, so each dab contributes
up to one full LSB (half from `rgb`, half from `a * pre.rgb`), not half. Add
bilinear filtering on both sides and three dabs is 3 to 4 LSB before the
sampler slack. Either state the bound honestly and set the tolerance to 4 from
the start (the plan already anticipates widening), or assert only where
`tip >= 0.9` (dab interiors), where the premultiplied error is smallest. Do not
present "3 leaves one LSB of slack" as derived; it is not.

Also state that all three dabs must move by at least
`STATIONARY_THRESHOLD_PX`, because the two paths handle a stationary dab
differently (dropped before the queue vs a zero deposit), and both are identity
only on an opaque canvas.

### 4. Test 5's "bit-identical" on a transparent layer is not what the commit produces (Tests, item 5)

`shaders/source_over.wgsl:15-19` zeroes `rgb` wherever `out_a <= 0.001`, and
`composite.wgsl` divides through `out_a` for the rest. A zero deposit therefore
still rewrites transparent texels whose stored `rgb` was non-zero, and can flip
an LSB elsewhere through the float divide. Specify the fixture with `rgb = 0`
under `a = 0`, or compare alpha exactly and `rgb` only where `a > 0`. The test
does fail without the stationary rule (alpha rises under `app OVER app`), so it
is a valid regression test once the comparison is stated correctly.

### 5. The `buildup == 1` restriction: state the structural reason first (section 7)

The plan justifies the compile error with the documented wash-law caveat
(per-channel `Max` fringes on per-dab colour). That is true but secondary. The
primary reason is mechanical: inside the dial `paint` keeps its build half in a
separate channel (`BUILD_CHANNEL`, `paint.rs:59-64`, attached through
`scratch.color_attachments`), and `prepare_dab_canvas_copy` mirrors the
*scratch only* (`gpu_context.rs:736-743`). A live sampler under `buildup < 1`
would therefore read an appearance missing its build half regardless of any
fringing. Lead with that; it is the reason the restriction cannot be lifted
without a second mirror.

On the failure mode: a compile error has precedent (`paint.rs:748-751` rejects a
wired `buildup` the same way), and `buildup` is not exposed on the Dry Smudge,
so the picker slider cannot hit it. It can be hit from a Pencil-derived brush
(`pencil.yaml` exposes `paint.buildup` as "Strength") when an artist wires in a
live sampler. Acceptable, but the error message should name the exposed label
("Build-up") as well as the port, since that is what the artist sees.

### 6. Why not the `smudge` terminal: the stated reason is a limitation, and the real blocker is unstated (section "Why not extend the `smudge` terminal")

The plan rejects extending `smudge` because read-mirror terminals own
`@group(3)` (`read_mirror_terminal.rs:55-57`) and collide with baked `noise`
(`wgsl/mod.rs:544-553`). But this plan itself introduces
`LiveSource::StrokeMirror` and a framework `mirror_origin` dab field; with
those, the read-mirror terminals could request their mirror as an ordinary
`graph_sources` live slot, drop `SCRATCH_MIRROR_BINDINGS`, and the Dry Smudge
would be `smudge.yaml` with the grain chain wired into `smudge.mask`, at a
fraction of this plan's LOC. The reason that does not work today is real but
absent from the plan: liquify's scratch is `Rg32Float`, which is not
filterable, so its pipeline layout comes from
`canvas_copy_layout_for(target_format)` (`read_mirror_terminal.rs:708`,
`pipeline.rs:772-781`), while the registry layout is filterable-only. Record
that evidence in the section, and record that after this plan the same mirror
is bound two ways (`scratch_mirror_tex` via `canvas_copy_bgl` for the
read-mirror terminals, `graph_tex_N` via the registry for `paint`). That is
accepted duplication only if the follow-up that removes it (the brief's
"deleting the `smudge` terminal") is named as the owner.

### 7. Reference accuracy

- `kis_colorsmudgeop.cpp:137-147` for the subpixel note: the comment block is
  at `:143-152` and `disableSubpixelPrecision()` at `:153`. `:192-198` for
  `srcDabRect` / `m_firstRun` is right (`:192`, `:196-199`).
- `KisColorSmudgeStrategyBase.cpp:135-138`: `smearCompositeOp` is at
  `:136-139`. `:264-284` is right.
- `paint.rs:65-90` for the two accumulations: `BUILD_CHANNEL` and `shares` are
  at `:59-85`.
- `painting.rs:349-360`: the handler spans `:343-360`.
- Everything else resolves as cited, including `read_mirror_terminal.rs:431-461`,
  `:545-562`, `:572-600`, `:663-704`; `paint.rs:485-529`, `:531-676`,
  `:588-617`, `:596-600`, `:645-672`, `:736-789`; `eval.rs:790`, `:930-1002`,
  `:1035`, `:1307`; `texture_registry.rs:54-61`, `:250-253`;
  `checkpoint_ring.rs:472-550`; `stroke_engine.rs:249-256`, `:493-513`;
  `noise.rs:164-260`; `clone_source_cursor.ts:136`.
- The bench table reproduces
  `bench-results/stroke-replay-matrix-smudge-recorded_curvy_stroke-d79ed0f518.md`
  exactly. One inference does not follow: "the 1 px spacing floor makes the dab
  count per event constant across radius 1, 10 and 100". At radius 100 the
  Smudge's `spacing: 0.01` on a 200 px diameter is 2 px
  (`spacing.rs:38-43`), so the floor alone does not explain 321.6 dabs/ev at
  r = 100 matching 321.3 at r = 1. State the constancy as observed, not derived,
  or find the actual cause in the bench's size override before the follow-up
  perf plan builds on it.

### 8. Per-dab cost claim (Performance baseline)

Holds. Per dab the sampler issues one mirror read plus one pre-stroke read plus
`source_over`, against smudge's two mirror reads; the copy region is
`bbox + ceil|motion|` on both, plus the sampler's one extra texel; the pass and
copy count are identical; `paint`'s `LoadOp::Load` and hardware source-over are
what the instanced path already pays. What the plan leaves out: the Dry Smudge
also samples the baked grain tile per fragment (the benched Smudge has no
grain) and `paint`'s commit runs `composite.wgsl` full-layer (the ceiling branch
is skipped at `buildup = 1`) where smudge's is a blit. Neither is per dab.
"Inherits the table within noise" is a prediction the plan can make good on
cheaply: step 12 adds the topology, so step 5 should run it for at least the
1920x1080 r = 1/10/100 and 2560x1440 r = 2000 rows and put the numbers in the
perf doc next to the Smudge rows, rather than leaving the prediction unverified.

### 9. Spacing 0.04 (The brush YAML)

The Performance and Risks sections say plainly that 0.04 cuts the dab count 4x
relative to the Smudge. The YAML notes do not mention it at all, so the value
reads as an unexplained default there. Say the same thing in both places, in
one sentence: 0.04 is inside the range Krita ships (its default brush spacing is
0.1, which `spacing.rs:4` and `:18` already cite) and was chosen with the bench in
view. That is honest and sufficient. A `comment:` on `brush_settings` in the
YAML would carry it to the editor.

### 10. `read_reach` hook (section 3)

Justified. The extent protocol is composed once at compile time
(`wgsl/extent.rs:121-164`) and `motion` is per dab; `read_half` is a
`ReadMirrorTerminal` method, and the sampler is not a terminal. One might argue
`motion` is bounded by the spacing step (dabs are placed `step` apart,
`stroke_engine.rs:415-424`, and `motion` is the delta between placed dabs), but
the step depends on `effective_diameter()` and the session's `SpacingConfig`,
neither of which is in the graph, so no compile-time bound exists. Composition
is consistent with the read-mirror terminals: `bbox_radius` already carries
`brush_extent_factor / extra_px` (`paint.rs:514`), the reach adds to it, and
`queue_mirrored_dab` clamps `read_half >= bbox_radius` per axis as
`read_mirror_terminal.rs:436-441` does today. `record_dab_footprint` keeps
taking the write radius, as it must.

Small: the hook runs for every CPU node every dab (`execute_cpu`,
`eval.rs:930-1002`); one virtual call per node per dab with an identity default
is noise, but say so in the hook doc so nobody later moves it to compile time
"for speed".

### 11. Smaller findings

- `center` stays visible in Live mode and does nothing. Give it
  `with_visible_when("source", [0])` like `mode` and `merged`.
- `dab_passes` on watercolor: `watercolor.rs:1043-1082` records two render
  passes per dab (pickup + composite). Counting `total_dabs` there is only right
  if the counter means "dab draws", so define it that way in the field doc, or
  count `2 * total_dabs`. Separately: the counter exists for test 6 alone and
  touches four files; test 3 already proves the chained path serializes. The
  unchained "exactly one pass" pin is worth keeping, but note that is the whole
  justification.
- `for_each_mirrored_dab`'s closure takes `&mut BrushGpuContext`, so `paint`'s
  prologue cannot hold `let scratch = &*stroke.scratch;` (`paint.rs:574`)
  across the loop; the attachments and write view must be re-borrowed per
  iteration as `read_mirror_terminal.rs:566-571` does. "The prologue is
  unchanged" is slightly optimistic; budget for it.
- `paint`'s `record_pass` closure and `rmt::flush_dabs`'s pass body
  (`read_mirror_terminal.rs:572-600`) remain two copies of "begin pass, viewport,
  four bind groups, draw". That duplication exists today between `paint.rs:645-672`
  and `rmt`; the plan neither adds to it nor removes it. Say so under DRY rather
  than leaving it implied.
- The framework `mirror_origin` field is pushed by `compile_brush_to_wgsl` after
  the walk. A tighter home is `CompileWgslCtx::request_live_texture`: it already
  owns `graph_sources` (`wgsl/context.rs:203`), and "a chained source carries
  the mirror origin" is a fact about the request, not about the walk. Either
  placement is defensible; the ctx one keeps the compiler ignorant of mirrors.
- `active_brush_needs_source` (`painting.rs:343-360`) is consumer-side matching
  on `type_id`; the plan deepens it (it will now also read the `source` port)
  without introducing it. `graph_capabilities` (`brush/mod.rs:242-251`) is the
  registration-driven home for graph-level facts, but this one depends on a port
  value, so it does not fit there cleanly either. Pre-existing; note it as such.
- `PickupAtlas` deletion: confirmed dead (`grep -rn PickupAtlas crates/darkly/src`
  hits only `texture_source.rs:80-90`). Of the comments that mention a pickup
  atlas, the ones about *watercolor's* atlas via `terminal_bindings`
  (`wgsl/mod.rs:276`, `:536`, `:958`, `eval.rs:391`, `dab_record.rs:93`) are
  true and stay; only the ones describing a `pickup` *node* publishing a live
  slot (`texture_source.rs:17`, `wgsl/mod.rs:136`, `:962`, `paint.rs:158-160`,
  `:585`, `:591`, `:663`, `context.rs:55`, `:301`) need correcting.
- `graph_smp` is Linear + Repeat (`texture_registry.rs:54-61`); the read-mirror
  sampler is Linear + ClampToEdge (`pipeline.rs:450-455`, default address
  mode). The plan's "Linear matches" is right and its "Repeat never matters"
  argument holds, but note that the mirror texture is lazily grown and usually
  larger than the copy, so `m_uv` inside `[0, 1]` is not by itself inside the
  copied region; it is `read_half >= bbox + |motion|` plus the extra texel that
  keeps every sampled texel inside the copy. Same reasoning smudge relies on;
  write it down once in the sampler's decls comment.
- Stale clone anchor: verified harmless. `set_clone_source_frame` runs every pen
  event for every stroke buffer (`painting.rs:1133-1142`), so the
  `debug_assert` in `place_dab` (`stroke_engine.rs:487-490`) cannot fire for a
  non-clone brush after a clone stroke, and the seeded `CloneState` packs keys
  no live-arm shader declares. The plan's section 10 is correct.
- Rewind, growth, preview, coordinate frames: all checked and correct as
  written. `restore_render_state` restores `last_dab_pos`
  (`stroke_engine.rs:254`); `restore_before` copies the checkpoint region into a
  cleared scratch (`checkpoint_ring.rs:472-550`); `target_pos` is a plane
  position in stroke mode (`wgsl/mod.rs:1023-1027`, `:1054`); `layer_offset` is
  `vec2<i32>` (`_prelude.wgsl:38`), so the `f32()` casts are needed; the floored
  `mirror_origin` matches `clamp_f32`'s floor (`gpu_context.rs:704-717`) and
  gpu lesson 7; the preview `DabRecord` carries `mirror_origin` because
  `dab_fields` are shared across both variants, and the live-arm helper is never
  called from the preview body.
- Picker icon (open question 3): `graph_capabilities` takes the first
  registration's `preview_staging.icon` (`brush/mod.rs:250-251`), so the Dry
  Smudge shows `fa6-solid:clone`. Accept it for this step and add the test row;
  a per-arm icon needs a method on the evaluator and is not worth it here.

### 12. LOC and scope

The per-file numbers are credible against the files named (the largest,
`clone_source.rs` at +130, is about 35 lines of WGSL, 15 of ports, 20 of
helpers and the rest docs). About 430 net production lines for one brush is
heavy; roughly 120 of it is the read-mirror extraction, which pays for itself,
and about 30 is the `dab_passes` counter, which does not (see 11). The pen-down
anchoring byproduct (section 12) should stay out; nothing in this feature
needs it.

### Verdict

`revise`. Fix 1 (the wgpu API), 2 (do not expose `paint.opacity` on the Dry
Smudge and say why), 3 and 4 (state the test bounds correctly), and put the
structural reason first in 5 and the missing evidence into 6. The rest are
wording and bookkeeping that the implementation step can carry.

## Revision (orchestrator response to review)

Every finding is accepted and folded into the plan below; the review above is
preserved verbatim. Where the plan text changed:

1. **wgpu API (finding 1):** the `@group(3)` rebuild keys on a grow signal.
   `Scratch::sync_read_mirror` returns whether it grew,
   `prepare_dab_canvas_copy` forwards it, and `for_each_mirrored_dab` hands
   it to the closure. No texture identity comparison; risk 2 rewritten.
2. **Opacity (finding 2):** `paint.opacity` is not exposed on the Dry Smudge.
   Section 7 records why (commit-time opacity ghosts a smear; Krita and the
   `smudge` terminal apply it per dab); `paint`'s contract is unchanged.
   Open question 2 closed.
3. **Test 4 (finding 3):** tolerance is 4 LSB with the derivation stated
   honestly (premultiplied `rgb` and `a` rounded separately, one LSB per dab,
   plus bilinear), all dabs required to move past the stationary threshold,
   and interiors-only as the fallback instead of widening.
4. **Test 5 (finding 4):** fixture authored with `rgb = 0` under `a = 0`;
   alpha compared exactly, `rgb` only where `a > 0`; the failing-without-fix
   mechanism (`app OVER app` raises alpha) is stated.
5. **Build-up restriction (finding 5):** the structural reason leads (the
   mirror copies the scratch only; the build half lives in `BUILD_CHANNEL`),
   the wash-law fringing follows, and the error names the "Build-up" label.
6. **Why not smudge (finding 6):** the section now records that this plan's
   own `StrokeMirror` and `mirror_origin` would let the read-mirror terminals
   bind through the registry, that liquify's non-filterable `Rg32Float`
   scratch is what blocks it, and that the resulting two-way binding of one
   mirror is owned by the `smudge`-deletion follow-up.
7. **References (finding 7):** the off-by-a-few citations are corrected in
   the body. The "1 px floor makes dab count constant" inference is restated
   as an observation with the radius-100 discrepancy flagged for the
   follow-up perf plan.
8. **Cost claim (finding 8):** the grain-tile sample and the full-layer
   composite commit are named as costs outside the Smudge rows, and step 5
   benches the `dry-smudge` topology on the named rows instead of predicting.
9. **Spacing (finding 9):** stated once in the YAML notes and carried into
   the editor by a `comment:` on `brush_settings`.
10. **`read_reach` (finding 10):** the hook doc records the per-node-per-dab
    cost and why it cannot move to compile time.
11. **Smaller findings (11):** `center` hidden in Live mode; the counter is
    `dab_draws` (draw calls into the scratch), defined so watercolor's count
    is right, with test 6 named as its sole justification; the per-iteration
    re-borrow in `paint`'s flush is budgeted; the duplicated pass body is
    recorded under Risks with the `smudge` deletion as owner; the
    `mirror_origin` field is pushed by whichever node or wrapper requests a
    chained source, with a by-name dedupe in the compiler's `dab_fields`
    aggregation (the compiler stays ignorant of mirrors); the
    `active_brush_needs_source` `type_id` match is noted as pre-existing;
    the `PickupAtlas` comment list is the reviewer's; the sampler-address-mode
    argument is corrected (the reach, not the UV range, keeps reads inside
    the copy). Picker icon accepted for this step; pen-down anchoring stays
    out; `PickupAtlas` removed. Open questions 3, 4 and 5 closed.
12. **LOC (finding 12):** production about 590 added / 145 removed (was
    570 / 140), the delta being the grow signal and the re-borrow.

# Live canvas sampler: a dry-media smudge through `paint`

Status: reviewed and revised, then **paused before approval**. The `buildup == 1` restriction (section 7) exposed that a sampler under the fragment path cannot see the whole stroke, one more instance of the pattern recorded in section F of `docs/paint-compute-perf-tracking.md`. The dispatch-per-dab spike (`docs/plans/compute-dispatch-per-dab-spike.md`) decides whether this plan is rewritten onto a compute paint terminal or resumed with a full-appearance mirror. No production code has changed.

## Summary

A finger smudge for pencil and charcoal, built as a *canvas-sampling node
feeding the `paint` terminal* rather than as a terminal of its own. The node
samples the stroke in progress at `target_pos - motion` (the point under this
fragment one dab ago), feeds that colour into `stamp.color`, and `stamp.dab`
feeds `paint.rgba`. Paper grain comes from wiring the pencil's canvas-space
grain chain into `stamp.tip`. `paint` supplies flow, opacity, pressure size,
the hover preview, the commit, and undo for free.

Two mechanisms are new, both generic:

1. **The sampler** is `clone_source` generalized with a `source` port:
   `Snapshot` (frozen at stroke start, what it does today) or `Live` (the
   stroke in progress). Both are "sample the canvas at an offset"; only the
   texture, its frame and the offset formula differ, and they are selected at
   compile time the way `noise` selects its baked/live and canvas/dab arms.
2. **A chained flush in `paint`**: a compiled brush whose graph requests a
   live *chained* source (one that depends on the pass's own output) makes
   `paint::flush_dabs` draw one dab per render pass with a
   `copy_texture_to_texture` mirror refresh between passes, reusing the
   per-dab loop that `read_mirror_terminal.rs` already runs for smudge, blur
   and liquify. Brushes without such a source keep the single instanced draw,
   byte for byte.

Ships with `crates/darkly/brushes/dry_smudge.yaml`. `smudge.yaml` and the
`smudge` terminal are untouched.

## Performance baseline

Read first, because it bounds what this plan can promise.

Bench: `cargo run --release --features testing --bin stroke_replay_matrix --
--topology smudge --input crates/darkly/tests/fixtures/recorded_curvy_stroke.json`
on `dev` at `d79ed0f5`, run for this plan. Full tables written by the bench to
`crates/darkly/bench-results/stroke-replay-matrix-smudge-recorded_curvy_stroke-d79ed0f518.{md,tsv}`.

Hardware: Intel(R) Graphics (RPL-U) integrated GPU, Vulkan, Linux 7.2.6. This is
not the machine the earlier matrices in `docs/paint-compute-perf-tracking.md`
were taken on (that doc does not record its hardware), so compare shapes, not
absolute numbers. The Smudge brush authors `spacing: 0.01`. Observed, not
derived: the dab count per event is the same across radius 1, 10 and 100
(the three rows per canvas below differ only in bbox). The 1 px spacing floor
(`spacing::ABSOLUTE_MIN_SPACING_PX`) explains radius 1 and 10 but not 100,
where 0.01 of a 200 px diameter is 2 px (`spacing.rs:38-43`); the bench's
size override is the likely cause and the follow-up perf plan must pin it
down before building on these rows. Either way the small-radius rows measure
the 1 to 2 px spacing regime section E of the perf doc warns about.

`behind_by_ms` (positive = the engine fell behind the recorded 3536 ms stroke):

| canvas | radius_px | behind (ms) | worst-frame (ms) | cpu p50 (us) | submit p50 (us) | dabs/ev | passes+copies per dab |
|---|---:|---:|---:|---:|---:|---:|---:|
| 1280x720 | 1 | +44 | 32.1 | 13337 | 10891 | 321 | 1+1 |
| 1280x720 | 10 | +28 | 24.9 | 12871 | 10582 | 321 | 1+1 |
| 1280x720 | 100 | +52 | 26.5 | 12982 | 10443 | 322 | 1+1 |
| 1280x720 | 250 | +9 | 17.4 | 8241 | 6221 | 129 | 1+1 |
| 1280x720 | 500 | +7 | 19.7 | 7065 | 5105 | 64 | 1+1 |
| 1280x720 | 1000 | +7 | 19.3 | 5697 | 3951 | 32 | 1+1 |
| 1280x720 | 2000 | +6 | 23.2 | 4071 | 2661 | 16 | 1+1 |
| 1920x1080 | 1 | **+1089** | 71.9 | 19789 | 16829 | 491 | 1+1 |
| 1920x1080 | 10 | **+1121** | 63.5 | 19949 | 16563 | 491 | 1+1 |
| 1920x1080 | 100 | **+1136** | 67.8 | 20114 | 16537 | 491 | 1+1 |
| 1920x1080 | 250 | +8 | 33.4 | 9607 | 7477 | 197 | 1+1 |
| 1920x1080 | 500 | +9 | 21.6 | 8119 | 6089 | 98 | 1+1 |
| 1920x1080 | 1000 | +6 | 28.5 | 6870 | 4798 | 49 | 1+1 |
| 1920x1080 | 2000 | +242 | 30.2 | 8669 | 3639 | 25 | 1+1 |
| 2560x1440 | 1 | **+2615** | 67.7 | 27055 | 22409 | 667 | 1+1 |
| 2560x1440 | 10 | **+2673** | 62.9 | 27588 | 22528 | 667 | 1+1 |
| 2560x1440 | 100 | **+2718** | 73.6 | 27159 | 22813 | 667 | 1+1 |
| 2560x1440 | 250 | +10 | 28.2 | 11675 | 9265 | 267 | 1+1 |
| 2560x1440 | 500 | +9 | 40.1 | 8457 | 6387 | 133 | 1+1 |
| 2560x1440 | 1000 | +513 | 41.0 | 15655 | 5723 | 67 | 1+1 |
| 2560x1440 | 2000 | **+1976** | 50.1 | 26818 | 19898 | 33 | 1+1 |
| 3840x2160 | 1 | **+5851** | 119.4 | 42908 | 35299 | 1021 | 1+1 |
| 3840x2160 | 10 | **+5938** | 121.7 | 42783 | 35534 | 1021 | 1+1 |
| 3840x2160 | 100 | **+5969** | 123.0 | 44185 | 36743 | 1022 | 1+1 |
| 3840x2160 | 250 | +552 | 49.0 | 17205 | 13807 | 409 | 1+1 |
| 3840x2160 | 500 | +381 | 50.4 | 11647 | 8335 | 204 | 1+1 |
| 3840x2160 | 1000 | **+2405** | 64.6 | 26966 | 8119 | 102 | 1+1 |
| 3840x2160 | 2000 | **+6265** | 199.4 | 47541 | 28799 | 51 | 1+1 |

**Verdict, stated plainly: the serialized path does not keep up.** At 1 px
spacing it falls behind on every canvas at or above 1080p: about 40 us per dab
(one `begin_render_pass` + one `copy_texture_to_texture` + bind + draw) times
491 to 1021 dabs per event is 20 to 44 ms per event against a 17 ms budget.
At the other end, large dabs on large canvases lose on copy bytes: the mirror
refresh copies `(2r + 2|motion|)^2` texels per dab, and at 4K with r = 2000 px
that is up to 64 MB per dab, 51 times per event. Only 1280x720 keeps up
everywhere, and the 250 to 500 px band keeps up on every canvas. The smudge
brush as shipped lags today; this is a pre-existing condition of the
read-mirror path, not something this plan introduces.

**Expected cost of the new path relative to the current smudge, per dab:** the
same. One render pass with one instance (`draw(0..6, i..i+1)`), one
`copy_texture_to_texture` of the same region (`bbox + |motion|` per axis, the
formula smudge's `read_half` uses today), two texture samples in the fragment
(smudge reads the mirror twice; the sampler reads the mirror once and the
pre-stroke snapshot once, plus one `source_over`), and the hardware blend that
the instanced path already pays. One extra: the sampler's `@group(3)` bind
group is rebuilt when the read mirror is reallocated, which happens a handful
of times per stroke as it lazy-grows, never per dab. Per event, `paint`'s
commit is a full-layer composite pass where smudge's is a full-layer copy;
comparable. Not per dab, but not in the smudge rows either: the Dry Smudge samples the
baked grain tile per fragment, and `paint`'s commit runs `composite.wgsl`
over the layer where smudge's is a blit. So "inherits the table within
noise" is a prediction, and step 5 verifies it: with the `dry-smudge` bench
topology from implementation step 12, run at least the 1920x1080 rows at
radius 1, 10 and 100 and the 2560x1440 row at 2000, and record the numbers in
`docs/paint-compute-perf-tracking.md` next to the Smudge rows. The shipped
YAML authors `spacing: 0.04` (pencil uses 0.03; the Smudge uses 0.01), which
cuts the per-event dab count by roughly 4x relative to the Smudge rows before
any engine work.

**Not in this plan:** the hybrid compute path or any other change to the
serialized flush's cost curve. Section E of `docs/paint-compute-perf-tracking.md`
already frames that question for the *shared* serialized path (smudge, blur,
liquify, and now chained `paint`); it deserves its own perf plan, and the
extraction in this plan (one per-dab loop shared by every chained terminal)
is what makes such a plan land in one place. Follow-up: "serialized flush
perf" plan, taking the table above as its baseline.

## Feature semantics

### What a dab does

Per dab `n` at centre `p_n` with motion `m_n = p_n - p_{n-1}`, for every
fragment at plane position `t` inside the dab footprint:

```
sampled  = appearance_{n-1}(t - m_n)             // straight RGBA, the stroke so far
dab      = premul(sampled) * tip(t) * build_flow // what stamp + paint emit
scratch_n = dab OVER scratch_{n-1}                // paint's PREMULTIPLIED_SOURCE_OVER
appearance_n = scratch_n OVER pre_stroke          // what the commit shows
```

where `appearance_{n-1} = scratch_{n-1} OVER pre_stroke` is computed by the
sampler in the fragment shader from the pre-stroke snapshot and the scratch
read mirror. On an opaque canvas with `sampled.a = 1` this collapses per dab
to `appearance_n(t) = mix(appearance_{n-1}(t), appearance_{n-1}(t - m), tip * flow)`,
which is the `smudge` terminal's `mix(bg, src, rate * mask)` with
`rate = build_flow` (`crates/darkly/src/brush/nodes/smudge.rs:142-148`). That
equality is the equivalence test below.

### Prior art (Krita, verified in source)

`krita/plugins/paintops/colorsmudge/kis_colorsmudgeop.cpp:192-198`: the source
rect is the destination rect translated back to the previous dab's centre
(`srcDabRect = m_dstDabRect.translated((m_lastPaintPos - newCenterPos).toPoint())`),
and the very first dab of a stroke paints nothing (`if (m_firstRun) { m_firstRun = false; return spacingInfo; }`).
Krita reads from the aligned image (it disables subpixel precision in smearing
mode, lines 137-147) so the source is the previous dab's *centre*, which is the
same `t - motion` this plan samples at.

`KisColorSmudgeStrategyBase.cpp:136-139`: the smear composite op is `COPY` when
smearing alpha and `OVER` otherwise; `264-284` (`blendInBackgroundWithSmearing`):
the destination is read, the source rect is read, and the source is composited
over the destination at the smudge-rate opacity. Every access to the source is
`readBytes`; nothing writes it back. Krita never depletes the source: it only
reads. This plan does the same: sampling the live stroke deposits at the
destination and leaves the source pixels alone.

The plan follows Krita on the first-dab rule (a stationary dab deposits
nothing, see "Stationary dabs" below) and on the read-then-write-at-offset
shape. It differs in that Krita's op reads the layer directly between dabs,
where Darkly reads through a per-dab mirror copy because WebGPU forbids
sampling the render target.

### Why not extend the `smudge` terminal

Read-mirror terminals own `@group(3)` for the scratch mirror
(`read_mirror_terminal.rs:55`, `SCRATCH_MIRROR_BINDINGS`), and `image` /
baked `noise` textures also bind at `@group(3)`, so `compile_brush_to_wgsl`
rejects the combination (`crates/darkly/src/brush/wgsl/mod.rs:544`). Under
`paint`, live slots already coexist with named and baked textures in the
graph-texture bind group: the shipped Clone brush proves `clone_source` +
`@group(3)` compiles, and `paint.rs:588-617` builds that group from published
views per flush. Everything paper grain needs is already wired there.

That limitation is not by itself decisive, because this plan introduces the
two things that would lift it: `LiveSource::StrokeMirror` and a framework
`mirror_origin` dab field. With those, the read-mirror terminals could
request their mirror as an ordinary `graph_sources` live slot, drop
`SCRATCH_MIRROR_BINDINGS`, and the Dry Smudge would be `smudge.yaml` with the
grain chain wired into `smudge.mask`. What blocks that today is liquify: its
scratch is `Rg32Float`, which is not filterable, so its pipeline layout comes
from `canvas_copy_layout_for(target_format)` (`read_mirror_terminal.rs:708`,
`pipeline.rs:772-781`), while the registry's `@group(3)` layout is
filterable-only. Rehoming the read-mirror terminals onto the registry means
giving the registry a non-filterable slot kind first.

Recorded consequence: after this plan the same mirror is bound two ways,
`scratch_mirror_tex` through `canvas_copy_bgl` for smudge, blur and liquify,
and `graph_tex_N` through the registry for chained `paint`. That is accepted
duplication with a named owner: the follow-up that deletes the `smudge`
terminal (the brief's later step) rehomes blur and liquify onto the registry
slot, or records the liquify format as the reason they stay, and removes one
of the two bindings.

## Architectural impact

- **Authority.** Nothing new is document state. The read mirror, the
  pre-stroke snapshot and the compiled brush are stroke resources
  (`StrokeBuffer`, session); the chained flag is compile-derived from the
  compiled brush; the per-dab read reach is per-dab transient data on
  `DabBatch`. No document field, no compositor mirror of a document fact.
- **Ownership.** The read-region math and the per-dab loop stay in
  `read_mirror_terminal.rs`, which already owns them; `paint` calls them.
  The sampler owns its own reach (`read_reach`) and its own WGSL; `paint`
  owns the pass. No terminal learns any node's `type_id`.
- **Modularity.** The serialization flag is a property of a source
  (`LiveSource::is_chained`), composed onto `CompiledBrush::dabs_are_chained()`;
  `paint` asks the compiled brush, never the graph. The read reach is a trait
  hook with an identity default (`extent`'s per-dab sibling), composed by the
  runner. Adding a second chained source later touches `texture_source.rs`
  and the new node only.
- **DRY.** `rmt::evaluate_gpu`'s "compute copy origin, insert it, queue the
  dab, push the meta" and `rmt::flush_dabs`'s "reset cache, refresh mirror,
  draw one instance" are extracted into two free functions the three
  read-mirror terminals and `paint` share. `STATIONARY_THRESHOLD_PX` moves to
  the shared module and smudge imports it. `source_over.wgsl` is prepended to
  every compiled brush rather than re-derived in the sampler.
- **JS/Rust boundary.** Unchanged. The frontend arms the set-source gesture
  through `engine.api.activeBrushNeedsSource()`
  (`frontend/src/tools/clone_source_cursor.ts:136`), an engine query; the
  engine side of that query is what changes.

## Design

### 1. The sampler: `clone_source` with a `source` port

Recommendation: generalize `clone_source`. Both modes are
`color = sample(texture, target_pos + offset)` with a bounds check and a
neutral preview body; what differs (texture, frame, offset) is exactly the
kind of compile-time arm `noise` already switches on (`nodes/noise.rs:164-260`
picks baked vs live and canvas vs dab and emits only the chosen arm). A second
node would duplicate the port surface (`center`, `color`), the preview
override, the CPU placeholder and the sample helper, and would leave "the
canvas at an offset" split across two files that must agree on frames. A new
node would be justified only if the live arm needed a different *evaluator
lifecycle* (it does not: it stays `is_gpu: false`, non-terminal, per-fragment)
or a different terminal (it does not: `paint` both ways).

`type_id` stays `clone_source` in this step: the engine's structural queries
(`painting.rs:349-403`), `clone.yaml` and the frontend's gesture arming key on
it, and a rename is mechanical churn that belongs in its own commit. Display
name becomes "Canvas Sampler" and the description is rewritten to cover both
modes. (Open question 1 records the rename.)

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
                       Live mode samples one dab behind along this vector."),
```

`mode`, `merged` and `center` gain `.with_visible_when("source", [0])`
(`center` feeds the anchored offset only; the live arm ignores it). Enum ports are
non-wirable, so `source` is always a compile-time literal, read by a new
`pub fn source_default_is_live(v: f32) -> bool` beside `mode_default_is_anchored`
(same "one threshold, shared by compile and engine queries" rule as
`MODE_ANCHORED_THRESHOLD`, lines 128-142).

`compile_wgsl` branches once on `source`:

- `Snapshot`: the existing body, unchanged, requesting
  `LiveSource::StrokeSnapshot`.
- `Live`: the body in section 6, requesting `LiveSource::PreStroke` and
  `LiveSource::StrokeMirror`.

`compile_cursor_preview_body` is shared: the neutral grey fill already there
(lines 253-262) is correct for both.

New trait hook on the node (section 3): `read_reach` returns
`[|motion.x|.ceil() + 1, |motion.y|.ceil() + 1]` in `Live` mode and `[0, 0]`
otherwise.

`evaluate_cpu` keeps returning the grey placeholder.

### 2. Live sources and the chained flag

`crates/darkly/src/brush/texture_source.rs:75-90`, `LiveSource` becomes:

```rust
pub enum LiveSource {
    /// The stroke's frozen clone source: the cross-layer / merged snapshot
    /// when one was captured, else the pre-stroke snapshot. Never changes
    /// during the stroke.
    StrokeSnapshot,
    /// The painted layer as it was at stroke start, always
    /// `StrokeResources::pre_stroke_texture`, never the clone override.
    /// Never changes during the stroke.
    PreStroke,
    /// The scratch read mirror: a per-dab snapshot of the stroke scratch
    /// under the dab's read region. Republished by the terminal before
    /// every dab, because it holds the previous dab's output.
    StrokeMirror,
}

impl LiveSource {
    /// Whether this source holds the output of the pass that samples it.
    /// A graph with a chained source cannot batch its dabs into one draw:
    /// instances in one draw cannot see each other's writes, so the
    /// terminal draws one dab per pass and refreshes the source between.
    pub fn is_chained(&self) -> bool { matches!(self, Self::StrokeMirror) }
}
```

`ResolvedSource::is_chained()` forwards; `CompiledBrush::dabs_are_chained()`
(`wgsl/mod.rs`, next to `color_targets`) is
`self.graph_sources.iter().any(|s| s.is_chained())`. That is the serialization
flag: derived from the sources, never stored twice, named after the
dependency, not the node.

`PickupAtlas` has no producer anywhere in the tree (`grep -rn PickupAtlas
crates/darkly/src` hits only `texture_source.rs`; there is no `pickup` node,
watercolor binds its atlas through `terminal_bindings`). Remove it in the same
edit. Of the comments that mention a pickup atlas, the ones about
*watercolor's* atlas via `terminal_bindings` (`wgsl/mod.rs:276`, `:536`,
`:958`, `eval.rs:391`, `dab_record.rs:93`) are true and stay; only the ones
describing a `pickup` *node* publishing a live slot (`texture_source.rs:17`,
`wgsl/mod.rs:136`, `:962`, `paint.rs:158-160`, `:585`, `:591`, `:663`,
`context.rs:55`, `:301`) are corrected.

Who publishes: both new sources are stroke resources, so `paint::flush_dabs`
publishes them exactly where it publishes `StrokeSnapshot` today
(`paint.rs:596-600`): `PreStroke` once per flush from
`stroke.pre_stroke_texture`, `StrokeMirror` inside the per-dab loop from
`stroke.scratch.read_mirror_texture()` whenever that texture changed (see
section 5).

`BrushGraphRunner::samples_source()` (`eval.rs:790`) keeps matching
`StrokeSnapshot` only, so the engine's clone no-op gate
(`painting.rs:943`) and source-snapshot capture (`painting.rs:1050`) do not
fire for a live sampler. That is the reason `PreStroke` is a distinct variant
from `StrokeSnapshot` even though they bind the same texture on a same-layer
clone: they answer different questions ("what is the clone source" vs "what
was the painted layer before this stroke"), and one of them gates a gesture.

### 3. Per-dab read reach

The sampler reads outside the dab footprint by `|motion|` per axis. The extent
protocol (`wgsl/extent.rs`) is the wrong tool: it sizes the *write* footprint
(`bbox_target_px`, the rasterized quad, the save-point bbox), is composed at
compile time, and `motion` is per dab and unbounded. The read region is
different data with a different lifetime, so it gets the per-dab sibling of
`extent`:

`crates/darkly/src/brush/eval.rs`, on `BrushNodeEvaluator` after `extent`:

```rust
/// Canvas pixels this node reads beyond the dab's write footprint, per
/// axis, for the dab being evaluated. The per-dab counterpart of
/// [`extent`]: `extent` bounds what a dab *writes* and is composed once
/// at compile time; this bounds what it *reads* and is composed per dab,
/// because a read offset such as the stroke's motion is per-dab data with
/// no compile-time bound. The runner takes the per-axis maximum over the
/// graph and hands it to the terminal, which sizes its read-mirror copy
/// from it. Default: nothing beyond the footprint.
fn read_reach(&self, _ctx: &EvalContext) -> [f32; 2] { [0.0, 0.0] }
```

`BrushGraphRunner` gains `dab_read_reach: [f32; 2]`, reset in `clear_slots`
(`eval.rs:1307`), and `execute_cpu` (`eval.rs:930-1002`) calls
`evaluator.read_reach(&ctx)` right after `evaluate_cpu` and folds it in with a
per-axis `max`. `dispatch_gpu` (`eval.rs:1035`) copies it onto
`gpu.dab_batch.read_reach` beside `compiled_brush` and `slot_outputs`. The hook
runs only for CPU-evaluated steps, which every sampler is (`is_gpu: false`);
a future GPU-typed node with a reach would need the same one-line call in
`dispatch_gpu`, noted in the hook's doc. The hook is one virtual call per
node per dab with an identity default; that is noise next to the dab's GPU
work, and the doc says so, because the value has no compile-time bound
(`motion` is the delta between placed dabs, and the step depends on the
session's spacing config, which the graph does not see), so it cannot move
to compile time "for speed".

`DabBatch` (`gpu_context.rs:218`) gains `pub read_reach: [f32; 2]`, documented
as per-dab, reset with the batch's other per-dab fields.

### 4. The mirror origin dab field

The sampler's shader needs the mirror's plane-space origin per dab to map
`target_pos` into mirror UVs, exactly what the read-mirror terminals carry as
`n{id}_copy_origin` today (`read_mirror_terminal.rs:663-704`). That field is a
framework fact ("where this dab's mirror snapshot starts"), not a node's, so it
becomes one:

`read_mirror_terminal.rs`:

```rust
/// Dab-record field carrying the plane-space top-left of the read-mirror
/// snapshot taken for this dab. Declared once per compiled brush by
/// whichever side needs it (the compiler for a chained graph, the
/// read-mirror wrapper for its terminals); read by any shader that samples
/// the mirror.
pub const MIRROR_ORIGIN_FIELD: &str = "mirror_origin";
pub fn mirror_origin_dab_field() -> DabField { .. }   // packs outputs[MIRROR_ORIGIN_FIELD]
```

- `rmt::compile_wgsl` pushes `mirror_origin_dab_field()` instead of its
  per-node `copy_origin`, and passes `MIRROR_ORIGIN_FIELD` to `compile_body`
  (the variants already take the field name as a parameter, so smudge, blur
  and liquify bodies change by zero lines).
- The sampler node pushes the same `mirror_origin_dab_field()` from its own
  `compile_wgsl` when it requests `StrokeMirror`, so "a chained source
  carries the mirror origin" is stated by the requester, and the compiler
  stays ignorant of mirrors. The aggregation in `compile_brush_to_wgsl`
  (`wgsl/mod.rs:482`, `dab_fields.extend(result.dab_fields)`) gains a dedupe
  by name: a second field with a name already present is dropped after a
  `debug_assert` that its type matches. Today every field name is
  node-prefixed, so this is the first shared field and the first time the
  dedupe fires.
- The sampler's WGSL references `d.mirror_origin`.

### 5. `paint`'s serialized flush and the shared helpers

Two extractions from `read_mirror_terminal.rs`, then `paint` calls them.

**`queue_mirrored_dab`** (from `rmt::evaluate_gpu`, lines 431-461):

```rust
/// Queue one dab that will read the scratch through the mirror: compute
/// the mirror origin with the formula `prepare_dab_canvas_copy` uses at
/// flush, publish it under [`MIRROR_ORIGIN_FIELD`], pack the record, and
/// push the CPU-side meta the flush loop walks in lockstep.
pub fn queue_mirrored_dab(
    gpu: &mut BrushGpuContext, compiled: &CompiledBrush,
    position: [f32; 2], bbox_radius: f32, radius: f32, read_half: [f32; 2],
)
```

It clamps `read_half` up to `bbox_radius` per axis (the existing
read-encloses-write rule), computes `copy_origin` against the paint target's
near edge (lines 438-446), inserts it under `MIRROR_ORIGIN_FIELD`, calls
`queue_dab`, and pushes `ReadMirrorDabMeta`. `rmt::evaluate_gpu` becomes:
geometry, `read_half`, `record_dab_footprint`, `pack_extra`,
`queue_mirrored_dab`. `ReadMirrorDabMeta` becomes `pub(crate)`.

**`for_each_mirrored_dab`** (from `rmt::flush_dabs`, lines 545-562):

```rust
/// Walk the queued mirrored dabs in order. Before each, invalidate the
/// per-dab read cache and refresh the mirror for that dab's read region
/// (`prepare_dab_canvas_copy`, whose copy is the barrier that lets the next
/// pass see the previous one's output); then hand the dab's index to
/// `draw`, which records exactly one pass for it.
pub fn for_each_mirrored_dab(
    gpu: &mut BrushGpuContext, meta_bytes: &[u8],
    mut draw: impl FnMut(&mut BrushGpuContext, u32, bool),
)
```

The `bool` is whether this dab's refresh reallocated the mirror. The lazy
grow lives in `Scratch::sync_read_mirror` (`scratch.rs:452-454`), which
returns nothing today; it returns `true` when it grew,
`prepare_dab_canvas_copy` forwards that, and the loop hands it to `draw`.
The grow is the event a `@group(3)` consumer cares about (a new texture has
a new view), so no texture-identity comparison is needed, and none is
possible anyway: wgpu 29's `Texture` has no `global_id`
(`wgpu-29.0.4/src/api/texture.rs:11-19`).

`rmt::flush_dabs` keeps its prologue and calls this with a closure that begins
its pass and draws `ii..ii + 1` (lines 572-600 move into the closure
unchanged).

**`paint::evaluate_gpu`** (`paint.rs:485-529`): after `record_dab_footprint`,

```rust
if compiled.dabs_are_chained() {
    let reach = gpu.dab_batch.read_reach;
    rmt::queue_mirrored_dab(gpu, &compiled, position, bbox_radius, radius,
        [bbox_radius + reach[0], bbox_radius + reach[1]]);
} else {
    gpu.dab_batch.queue_dab(&compiled, position, bbox_radius, radius);
}
```

**`paint::flush_dabs`** (`paint.rs:531-676`): the prologue (take, pipeline,
uniforms, dab upload) keeps its shape, with one adjustment: `for_each_mirrored_dab`
hands the closure `&mut BrushGpuContext`, so `let scratch = &*stroke.scratch;`
(`paint.rs:574`) cannot be held across the loop; the attachments and write
view are re-borrowed per call inside the pass closure, exactly as
`read_mirror_terminal.rs:566-571` does. The pass recording (lines 645-672)
moves into a local closure `record_pass(gpu, group3: Option<&BindGroup>,
instances: Range<u32>)`. Then:

```rust
if compiled.dabs_are_chained() {
    let metas = gpu.dab_batch.take_meta();
    // Published once: the pre-stroke snapshot does not change mid-stroke.
    publish PreStroke;
    let mut group3 = None;
    rmt::for_each_mirrored_dab(gpu, &metas, |gpu, i, mirror_grew| {
        // A fresh allocation has a fresh view; nothing else invalidates it.
        if group3.is_none() || mirror_grew {
            publish StrokeMirror from a fresh view; group3 = Some(build_group3(gpu));
        }
        record_pass(gpu, group3.as_ref(), i..i + 1);
    });
} else {
    let group3 = live-or-cached group as today;
    record_pass(gpu, group3, 0..total_dabs);
}
```

`build_group3` is the existing `make_bind_group` call over
`compiled.graph_sources` and the published views, lifted into a closure so
both branches use it. A brush without a chained source runs the `else` arm,
which is the current code with the pass body moved into a closure and called
once: zero behaviour change, and a test pins the pass count (section 9).

The "one accumulation" invariant makes this simple: `for_each_mirrored_dab`
refreshes only the write side's mirror, and section 7 guarantees a chained
graph has no other accumulation to mirror.

`BrushPerfCounters` (`gpu_context.rs:63`) gains `pub dab_draws: u32`: the
number of draw calls a flush issued into the stroke scratch, bumped by
`record_dab_draws(n)` from every flush (`rmt::flush_dabs`: `total_dabs`,
watercolor: `total_dabs`, since its pickup probes draw into the atlas and
only the composites draw into the scratch, chained `paint`: `total_dabs`,
instanced `paint`: `1`), folded in `AddAssign`. It is a count, not a timing,
so it stays inside `engine/perf.rs`'s "keep `BrushPerfCounters` small" rule.
Its whole justification is test 6's pin that an unchained graph still issues
exactly one draw; test 3 already proves the chained path serializes. The
bench gains a column only if the follow-up perf plan wants it.

### 6. The sampler's WGSL (live arm)

Emitted into `decls` once per node instance, called from the body:

```wgsl
fn sample_live_{id}(tp: vec2<f32>, motion: vec2<f32>) -> vec4<f32> {
    // Krita's first-run rule: a dab that has not moved smears nothing.
    if (abs(motion.x) < STATIONARY && abs(motion.y) < STATIONARY) {
        return vec4<f32>(0.0);
    }
    let src = tp - motion;                                   // plane px
    // Pre-stroke snapshot: the painted layer's own frame, which is what
    // the stroke skeleton packs into the intrinsic header.
    let lo  = vec2<f32>(f32(u.intrinsic.layer_offset.x), f32(u.intrinsic.layer_offset.y));
    let lsz = vec2<f32>(f32(u.intrinsic.layer_size.x),   f32(u.intrinsic.layer_size.y));
    let pre_uv = (src - lo) / lsz;
    if (pre_uv.x < 0.0 || pre_uv.x > 1.0 || pre_uv.y < 0.0 || pre_uv.y > 1.0) {
        return vec4<f32>(0.0);                               // off the layer
    }
    let pre = textureSampleLevel(graph_tex_{pre}, graph_smp, pre_uv, 0.0);          // straight
    // Read mirror: origin is the floored copy origin (see gpu-lessons-learned #7).
    let mdims = vec2<f32>(textureDimensions(graph_tex_{mirror}));
    let m_uv  = (src - d.mirror_origin) / mdims;
    let acc   = textureSampleLevel(graph_tex_{mirror}, graph_smp, m_uv, 0.0);       // premultiplied
    return source_over(acc.rgb, acc.a, pre);                  // straight appearance
}
```

Body: `let clone_c_{id} = sample_live_{id}(target_pos, {motion_expr});` with
`motion_expr = cctx.input("motion").as_vec2()` (a `d.n{pen}_motion` dab field
when wired, a literal when not). The output name stays `clone_c_{id}` so the
preview body's substitution keeps working.

`STATIONARY` is the literal of `read_mirror_terminal::STATIONARY_THRESHOLD_PX`
(moved there from `smudge.rs:37`; smudge imports it), so the CPU early-out in
smudge and the GPU rule here share one number.

`source_over` comes from `crates/darkly/shaders/source_over.wgsl`, which its
header declares the single source of truth for straight-alpha compositing.
`assemble_shader` (`wgsl/mod.rs`) prepends it after `fbm2d.wgsl`; it is
dead-stripped in every brush that does not call it, and no node declares a
function of that name (checked: `grep -rn "fn source_over" crates/darkly/src/brush`
is empty). This is the same composite `composite.wgsl` performs at commit
under `build_opacity` (section 7 explains why opacity is not applied here).

Why the composite is in the sampler and not in the mirror: decision 3 of the
brief. The mirror stays a plain `copy_texture_to_texture` of the scratch (the
cost in the baseline table), and the sampler pays one extra texture read and
one `source_over` per fragment to see the same picture the commit would show.

Sampler details: `graph_smp` is Linear + Repeat
(`gpu/texture_registry.rs:54-61`); the read-mirror sampler is Linear +
ClampToEdge (`pipeline.rs:450-455`). Linear matches, so sub-pixel motion
interpolates the same way. Repeat never matters, but the reason is not that
`m_uv` stays in `[0, 1]`: the mirror texture is lazily grown and usually
larger than the copied region, so a UV inside the texture is not by itself
inside the copy. What keeps every sampled texel inside the copy is
`read_half >= bbox + |motion|` plus the one extra texel of reach for the
bilinear half-texel, the same argument smudge relies on. It is written once,
in the sampler's decls comment. `pre_uv` is bounds-checked separately.

Selection: untouched here. `paint`'s terminal body multiplies by `sel`
(`paint.rs:736-789`), so a feathered selection scales the deposit as it does
for every paint brush and as `sel` scales smudge's `amount`.

Out of source: transparent, as the snapshot arm does. Through `stamp` that is a
zero-alpha dab: nothing deposited where the source lies off the layer.

### 7. Build-up: the live arm requires `buildup == 1`

The reason is mechanical before it is aesthetic. Inside the dial `paint`
keeps two accumulations: the wash half in the scratch under the coverage
ceiling and the build half in `BUILD_CHANNEL`, a separate colour attachment
(`paint.rs:59-85`). The read mirror copies the *scratch only*
(`prepare_dab_canvas_copy`, `gpu_context.rs:736-743`). A live sampler under
`buildup < 1` would therefore read an appearance missing its build half,
whatever the blend law did with the rest. Lifting the restriction means a
second mirror for the channel, and nothing in this feature wants one.

The wash law is also documented as wrong for this graph, which is why the
second mirror is not worth building. `docs/brush/architecture.md`, "What
`Wash` requires": `Max` runs per channel and only works when every dab of a
stroke carries one chroma; "a graph that varies dab colour per dab ... would
take per-channel maxima from different dabs and would fringe; such brushes
must stay on `Build-up`. This is not checked automatically." A live sampler
varies dab colour per dab by definition. This plan makes the existing rule
checked for the one case where it is structural.

`PaintEvaluator::compile_wgsl` (`paint.rs:736`), after reading `buildup`:

```rust
if cctx.graph_sources.borrow().iter().any(|s| s.is_chained()) && build_share < 1.0 {
    return Err("a live canvas sampler reads the stroke through the scratch \
                mirror, which cannot see the build channel that a Build-up \
                below 100% keeps; set paint.buildup (Build-up) to 100%".into());
}
```

The terminal compiles last in topological order, so `graph_sources` is
complete at that point. The check asks the sources a capability question
(`is_chained`), not the graph what nodes it holds. A compile error is the
established failure mode for a `buildup` the pass cannot honour
(`paint.rs:748-751` rejects a wired dial the same way); `buildup` is not
exposed on the Dry Smudge, so the picker cannot hit it, and a Pencil-derived
brush (which exposes it as "Build-up") gets a message naming the label.

Consequences that fall out: one accumulation means one mirror; the sampler's
composite is plain `source_over` with no ceiling to reproduce; and `paint`'s
commit for the dry smudge is the `build` slot alone, `source_over(build *
opacity, pre)`.

Stroke opacity is *not* applied in the sampler. `paint.opacity` is a
"stroke-level cap (applied at commit)" for every paint brush, and the sampler
sees the stroke at full strength so the chain is self-consistent (each dab
reads what the previous one deposited), and the commit scales the whole smear
by `opacity`. That is `mix(pre, smudged, opacity)`, a half-strength smudge at
50%. The `smudge` terminal instead folds opacity into every dab
(`smudge.rs:142-148`), so the two agree only at opacity 1; the equivalence test
fixes opacity at 1 and this paragraph is the recorded reason.

It follows that `paint.opacity` is **not exposed** on the Dry Smudge. At 50%
the commit would show `mix(pre, smudged, 0.5)`: the undisplaced pigment and
its displaced copy, each at half strength, a double image rather than a
weaker smudge. Krita applies opacity per dab
(`kis_colorsmudgeop.cpp:204`, `fpOpacity`, handed to the strategy at `:219`)
and so does the `smudge` terminal; both read as a shorter smear. `paint`'s
contract ("stroke-level cap, applied at commit") is right for every other
paint brush and is not special-cased here; the artist's one strength control
on the Dry Smudge is `build_flow`.

### 8. Cursor preview

Per `docs/brush-preview-and-overlays.md`: the preview pipeline binds the
registry `_fallback` tile to every unpublished live slot
(`texture_registry.rs:250-253`), and a non-terminal cannot render differently
at hover except through `compile_cursor_preview_body`. `clone_source` already
overrides it with a neutral grey fill that samples nothing
(`clone_source.rs:253-262`); the live arm shares that body. Both new slots are
declared in the preview shader (the compiler declares `@group(3)` from the
shared `graph_sources` list for both variants) and never read. The dry smudge
hover shows the tip shape in grey through the grain, which is what Krita's
smudge outline conveys. Test in section 9.

### 9. Erase, growth, rewind

**Erase.** `paint`'s commit under erase runs `destination_out(build.a, bg)`
(`composite.wgsl`), so a dry smudge in eraser mode removes coverage along the
stroke, weighted by the sampled alpha times tip times flow: a soft eraser that
follows the grain. Meaningful enough to leave `supports_erase: true` on the
node (it is already). Known limit, recorded: the sampler composites the
appearance *as if painting* regardless of `gpu.blend_mode`, so mid-stroke the
chain sees deposit the erase commit will never show. Making the sampler
erase-aware would mean threading `blend_mode` into the intrinsic uniforms;
not worth it for an eraser nobody asked for.

**Mid-stroke layer growth.** `gpu_stroke_to` grows the layer before any dab of
the event renders (`painting.rs:504-508`), and `StrokeBuffer::grow_preserving`
rebases the scratch and the pre-stroke snapshot together, so within a phase
the paint target's extent is stable: the `copy_origin` computed at
`evaluate_gpu` and the one `prepare_dab_canvas_copy` recomputes at flush agree
(the same guarantee the read-mirror terminals rely on today). The intrinsic
header (`layer_offset`, `layer_size`) is packed per flush from the current
extent, so the pre-stroke frame in the shader tracks growth. The mirror is
re-copied per dab and needs no rebase.

**Rewind.** A rewind runs `begin_stroke` (clear) then `restore_before`, which
copies the checkpoint region back into the scratch (`checkpoint_ring.rs:472-550`);
the next dab's mirror copy reads the restored scratch, so the chain resumes
from the checkpoint's pixels. `RenderCheckpoint::last_dab_pos` restores the
motion tracker (`stroke_engine.rs:249-256`), so the first replayed dab's motion
is the true delta from the checkpoint's last dab, not zero; only a full reset
zeroes it (`motion_resets_to_zero_after_rewind`), and that is the stroke's
first dab, which the stationary rule already handles. No checkpoint change:
the ring snapshots the scratch and its channels, and a chained brush has
exactly the scratch.

### 10. Engine gates

`DarklyEngine::active_brush_needs_source` (`painting.rs:343-360`) currently
answers "is there a `clone_source` node". It becomes "is there one whose
`source` is not `Live`", via the existing `clone_source_port_default` reader
(`painting.rs:367`) and the new `source_default_is_live`. This is
consumer-side matching on a `type_id`, and pre-existing: the plan deepens it
by one port read without introducing it. `graph_capabilities`
(`brush/mod.rs:242-251`) is the registration-driven home for graph facts, but
this one depends on a port value, so it does not fit there either; it goes
with the `clone_source` rename follow-up. The frontend keeps
calling `activeBrushNeedsSource()`; nothing crosses the boundary differently.
The compile-time gate (`runner.samples_source()`, `painting.rs:942-946`) is
already correct because `PreStroke` is not `StrokeSnapshot`.

`CloneState` seeding in `place_dab` (`stroke_engine.rs:493-513`) is unchanged:
it runs only when the engine holds a source anchor, and the live arm emits no
anchor uniforms, so a stale anchor from an earlier clone stroke injects keys
nobody packs.

### 11. Stationary dabs

Decided above: the live arm emits transparent below
`STATIONARY_THRESHOLD_PX` (shader side), and the terminal still queues the
dab. Reasons, in order: (a) on an opaque canvas a stationary sample deposited
over itself is an exact identity in colour, but on a partially transparent
pixel it raises alpha (`app OVER app`), a visible dot at every stroke start,
which is precisely why Krita's first dab paints nothing; (b) the sampler
cannot drop the *terminal's* dab without also dropping whatever else the
graph deposits in it (a future colour-rate mix), so it zeroes its own
contribution instead; (c) stationary dabs are rare under the stabilizer (the
first dab of a stroke, a full-reset replay), so the wasted pass and copy cost
nothing measurable, and the smudge terminal's CPU early-out stays where it is.

### 12. Pen-down anchoring (byproduct, not required)

"Pickup at pen-down" is the snapshot arm with `offset = dest_anchor - center`
(anchored, but the anchor is the first dab rather than the gesture). It falls
out as a third `mode` option, `Anchored at pen-down`, whose WGSL offset is
`(u.{da_field} - center)`, plus one change in `place_dab`: seed `CloneState`
whenever the runner has a `clone_source` node in the snapshot arm, with
`source_anchor = dest_anchor` when the engine holds no gesture anchor, and
`samples_source` returning false for that mode so the no-op gate does not
fire. About 25 production lines and one test. Not in this step's estimate;
listed under open questions so the reviewer can pull it in or leave it.

## Coordinate frames

Per `docs/coordinate-systems.md`. Every value the live arm touches:

| Value | Frame | Where it is produced | Converted by |
|---|---|---|---|
| `pen_input.position`, `motion` | plane px (sensed) | `stroke_engine::place_dab` | never; the sampler uses them as plane offsets |
| `target_pos` | target px, which is plane px in stroke mode | skeleton, `wgsl/mod.rs` `assemble_shader` | none in stroke mode; the preview body never samples |
| `src = target_pos - motion` | plane px | sampler WGSL | bounds-checked against the layer frame |
| `u.intrinsic.layer_offset/size` | plane rect of the paint target = the pre-stroke frame | `paint::flush_dabs` via `intrinsic_header` | `pre_uv = (src - offset) / size` |
| `d.mirror_origin` | plane px, floored | `queue_mirrored_dab` (CPU), same formula as `prepare_dab_canvas_copy` | `m_uv = (src - origin) / mirror_dims` |
| read region | plane rect | `prepare_dab_canvas_copy`, `clamp_f32` (floor near, ceil far) | translated to layer-local for the copy inside the same function |
| `read_reach` | plane px | sampler `read_reach` from `motion` | added to `bbox_radius`, which already crossed the reference boundary in `effective_radius` |
| selection | window-local | `plane_to_selection_uv` in the skeleton | unchanged |

Nothing in the live arm is authored in reference pixels: `motion` is sensed
and the reach is raster padding, so `dpi_factor` does not enter. The grain
chain's `noise.scale` is reference pixels and converts inside
`frame_sample_coord_expr` as it does for the pencil.

Test with a non-zero `canvas_origin`: the serialization test below runs once
at origin `(0, 0)` and once at `(-37, 19)` with the layer offset to match,
the way `tests/clone.rs::clone_copies_under_nonzero_origin_and_offset_layer`
does.

## Implementation steps

1. `crates/darkly/src/brush/texture_source.rs`: add `PreStroke` and
   `StrokeMirror`, remove `PickupAtlas`, add `LiveSource::is_chained` and
   `ResolvedSource::is_chained`, labels, docs.
2. `crates/darkly/src/brush/wgsl/mod.rs`: `CompiledBrush::dabs_are_chained()`;
   prepend `shaders/source_over.wgsl` in `assemble_shader`; dedupe
   `dab_fields` by name when aggregating. Update the `graph_sources` doc
   comment.
3. `crates/darkly/src/brush/read_mirror_terminal.rs`: `pub const
   STATIONARY_THRESHOLD_PX`, `MIRROR_ORIGIN_FIELD`, `mirror_origin_dab_field`,
   `queue_mirrored_dab`, `for_each_mirrored_dab` (with the grow signal
   threaded from `sync_read_mirror` through `prepare_dab_canvas_copy`);
   `ReadMirrorDabMeta` to `pub(crate)`; rewrite `evaluate_gpu`, `flush_dabs`,
   `compile_wgsl` on top of them; `record_dab_draws`.
4. `crates/darkly/src/brush/nodes/smudge.rs`: import the threshold constant
   (delete the local one). No other change.
5. `crates/darkly/src/brush/eval.rs`: `read_reach` hook; runner field, reset,
   fold in `execute_cpu`, hand-off in `dispatch_gpu`.
6. `crates/darkly/src/brush/gpu_context.rs`: `DabBatch::read_reach`;
   `BrushPerfCounters::dab_draws` + `record_dab_draws` + `AddAssign`;
   `prepare_dab_canvas_copy` forwards `sync_read_mirror`'s grow signal.
   `scratch.rs`: `sync_read_mirror` returns whether it grew.
7. `crates/darkly/src/brush/nodes/paint.rs`: chained branch in
   `evaluate_gpu`; `flush_dabs` restructured around `record_pass` /
   `build_group3`; the `buildup == 1` check in `compile_wgsl`; module docs.
8. `crates/darkly/src/brush/nodes/clone_source.rs`: `source` and `motion`
   ports, `visible_when` on `mode`/`merged`, `source_default_is_live`,
   the live arm in `compile_wgsl`, `read_reach`, display name and docs.
9. `crates/darkly/src/engine/painting.rs`: `active_brush_needs_source`
   consults `source`.
10. `crates/darkly/src/brush/nodes/watercolor.rs`: `record_dab_draws` in its
    loop (one line) so the counter means the same thing everywhere.
11. `crates/darkly/brushes/dry_smudge.yaml` and `crates/darkly/packs/dry_media.yaml`
    (`members: [pencil, charcoal, dry_smudge, hair, sponge]`).
12. `crates/darkly/src/bin/stroke_replay_matrix.rs`: a `dry-smudge` topology
    (brush name, terminal `paint`), so the follow-up perf plan can bench it
    against the smudge rows above.
13. Docs: `docs/brush/architecture.md` (paint terminal: the chained flush;
    terminals list: the sampler's live arm), `docs/paint-compute-perf-tracking.md`
    section E (link the smudge baseline, note that chained `paint` shares the
    path), `README.md` roadmap unchanged (Smudge is already ticked).
14. `cargo sync-docs`; no region lists nodes or brushes today
    (`grep -rn "darkly:" *.md docs` hits only the README's effects graphic),
    and the brush catalog has no committed stills, so no binary lands.

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
    comment: Spacing 0.04 sits inside the range Krita ships (default 0.1) and was chosen with the serialized-flush bench in view; each dab is a render pass
    inputs:
      size: 0.1
      spacing: 0.04
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
      buildup: 1.0
      build_flow: 0.6
connections:
- pen_input.position -> paint.position
- pen_input.position -> sampler.center
- pen_input.motion -> sampler.motion
- pen_input.pressure -> curve.input
- curve.output -> paint.size
- noise.value -> levels.input
- levels.output -> multiply.a
- circle.mask -> multiply.b
- multiply.result -> stamp.tip
- sampler.color -> stamp.color
- stamp.dab -> paint.rgba
exposed_ports:
  brush_settings.size: {}
  brush_settings.stabilize: {}
  paint.build_flow:
    label: Strength
    description: How much of what is under the finger moves with each touch
    icon: mdi:gesture-swipe
  levels.in_high:
    label: Grain
    description: How much the paper's tooth breaks up the smear
    icon: tabler:grain
    invert: true
```

Notes on the choices: the grain chain is the pencil's `noise_2 -> levels ->
multiply(circle) -> stamp.tip` core (`pencil.yaml`), without the pencil's
pressure-and-roughness curves feeding `levels`, since the smudge exposes grain
as one dial. `buildup: 1.0` is required by section 7 and stated so the file
documents it. `build_flow: 0.6` matches the Smudge's default `rate`.
`spacing: 0.04` is four times the Smudge's 0.01 and inside the range Krita
ships (its default brush spacing is 0.1, which `spacing.rs:4` and `:18`
cite); it was chosen with the performance baseline in view, since every dab of
a chained brush is a render pass, and the `comment:` on `brush_settings`
carries that to the editor. `paint.opacity` is deliberately not exposed
(section 7). The
`sampler` node id is deliberately not `clone_source` so the node's role reads
correctly in the editor; nothing keys on instance ids.

The `content_dependent_brushes_get_preview_icons` unit test in
`builtin_brushes.rs` lists expected picker-fallback icons; the new brush gets
the sampler's `preview_staging` icon (`fa6-solid:clone`) unless the
registration's icon is changed. Open question 3.

## Tests

All GPU tests run with `--features darkly/testing -- --test-threads=1`.

New file `crates/darkly/tests/live_sampler.rs`, harness modelled on
`tests/smudge.rs:66-190` (runner-level: explicit dabs, one `flush_dabs`,
`commit`, readback) with a `two_tone_canvas` and a graph builder that loads
`dry_smudge.yaml` from `builtin_brushes::all()`.

1. `live_sampler_compiles_beside_baked_noise_and_image` (no GPU): take the
   dry smudge graph, add an `image` node wired into `multiply.b` through a
   second `multiply`, `compile_graph` succeeds, and
   `compiled.graph_sources` holds a `Baked`, a `Named`, `Live(PreStroke)` and
   `Live(StrokeMirror)`; `dabs_are_chained()` is true. Companion: the Clone
   brush's `dabs_are_chained()` is false and the analytic disc fixture's is
   false.
2. `live_sampler_rejects_the_wash_law` (no GPU): same graph with
   `paint.buildup = 0.5` and again `0.0` fails `compile_graph` with an error
   mentioning `buildup`.
3. `second_dab_reads_first_dabs_deposit` (serialization): two overlapping
   dabs in one flush on the two-tone canvas, motion `(+30, 0)`, placed so dab
   2's sample point lies inside dab 1's write footprint; the pixel under dab 2
   carries red that a control render (dab 2 alone) does not. The same shape
   as `tests/smudge.rs::smudge_dab2_reads_dab1_deposit_not_pre_stroke`, and
   the regression net for collapsing the chained flush back into one draw.
   Run at origin `(0, 0)` and at a non-zero `canvas_origin` with an offset
   layer.
4. `matches_the_smudge_terminal_at_full_buildup` (equivalence): the same
   three-dab sequence through the shipped Smudge (`rate = 0.6`,
   `opacity = 1`, `circle.softness = 0.4`, pressure 1) and through a sampler
   graph with the identical `circle`, no grain, `build_flow = 0.6`,
   `buildup = 1`, on an opaque two-tone canvas, every dab moving by at least
   `STATIONARY_THRESHOLD_PX` (the two paths treat a stationary dab
   differently: dropped before the queue versus a zero deposit). Every pixel
   agrees within `+-4/255` per channel. Tolerance, derived: the smudge side
   rounds a straight-alpha scratch once per dab (half an LSB); the paint
   side rounds premultiplied `rgb` and `a` separately and the sampler
   reconstructs `rgb + pre.rgb * (1 - a)`, so each dab contributes up to one
   full LSB; three dabs plus bilinear filtering on both sides is 3 to 4 LSB.
   `4` is that bound, not a guess with slack; if a driver's rounding exceeds
   it, assert only on dab interiors (`tip >= 0.9`) rather than widening.
5. `stationary_dab_deposits_nothing`: one dab with `motion = (0, 0)` on a
   transparent layer holding a soft-edged mark, authored with `rgb = 0`
   wherever `a = 0`. Alpha is compared exactly and `rgb` only where `a > 0`:
   the commit's `source_over.wgsl:15-19` zeroes `rgb` under `a <= 0.001` and
   divides through `a` elsewhere, so "bit-identical" is not what a zero
   deposit produces on a layer with junk colour under transparent texels.
   Without the stationary rule alpha rises (`app OVER app`), so the test
   fails against the unfixed code. Regression for the first-dab alpha dot.
6. `unchained_graph_issues_one_pass_per_flush`: the analytic disc fixture
   with 8 dabs in one flush reports `ctx.perf.dab_draws == 1`; the dry
   smudge with 8 dabs reports `8`. Read from the `BrushGpuContext` before
   submit.
7. `hover_preview_is_a_neutral_footprint`: modelled on
   `tests/preview_smudge.rs`; the dry smudge preview has non-zero alpha
   inside the tip, grey RGB, and never NaN.
8. Engine level, `tests/live_sampler_engine.rs` (or in the same file with a
   `test_engine`): `replay_is_deterministic_and_moves_pixels`: replay
   `tests/fixtures/recorded_curvy_stroke.json` (stabilize 1.0, so the
   tip-divergence rewind runs every event) through two fresh engines over a
   striped layer laid down the way `tests/builtin_brushes_stroke.rs::stripe`
   does; the two readbacks are identical and differ from the pre-stroke
   layer. Exercises mirror + checkpoint restore + layer growth in one run.
9. Existing suites as regression nets: `tests/clone.rs` (the snapshot arm is
   untouched), `tests/smudge.rs`, `tests/blur.rs`, `tests/liquify.rs`,
   `tests/dab_footprint_ledger.rs` (the read-mirror extraction),
   `tests/brush_accumulation.rs` (paint's instanced path),
   `tests/builtin_brushes_stroke.rs::every_builtin_deposits_or_moves_pixels`
   (picks up the new YAML automatically), `builtin_brushes::tests::*`
   (catalog covers the new stem; the icon row for "Dry Smudge" is added).
10. Unit tests in `clone_source.rs`: `registration_shape` updated for the two
    new ports; `source_default_is_live` at 0, 0.5, 1; `read_reach` is
    `ceil(|motion|) + 1` per axis in live mode and zero in snapshot mode.

## Risks

- **Performance.** The baseline says the serialized path lags at 1 px spacing
  on 1080p and up, and at very large dabs on 1440p and up. The dry smudge
  ships at spacing 0.04, which keeps dab counts a quarter of the Smudge's,
  but a painter who drags the spacing down gets the Smudge's curve. The plan
  does not fix this and says so; the extraction concentrates the cost in one
  loop for the follow-up.
- **Bind-group rebuild timing.** `@group(3)` is rebuilt on the grow signal
  `sync_read_mirror` reports, not on texture identity, so a `Scratch` that
  recycles or reallocates is covered either way. Test 3 at large motion
  covers a grow inside one flush.
- **Two copies of "begin pass, viewport, four bind groups, draw".** `paint`'s
  `record_pass` closure and `rmt::flush_dabs`'s pass body
  (`read_mirror_terminal.rs:572-600`) stay separate. That duplication exists
  today between `paint.rs:645-672` and `rmt`; this plan neither adds to it nor
  removes it. It goes with the `smudge` terminal deletion, when the
  read-mirror pass body is the only one left to fold.
- **Two half-LSB paths in the equivalence test.** If the tolerance proves too
  tight on a driver with different rounding, widen to 4 and record why; do
  not relax the structural assertions (test 3) to match.
- **`source_over.wgsl` prepended to every brush.** Adds a few lines of source
  to every compiled brush; naga strips unused functions, so no runtime cost.
  A node that later declares its own `source_over` would collide at compile
  time, which is the desired failure.
- **Wash-law compile error is new user-facing behaviour** for anyone who wires
  a live sampler into an authored pencil (`buildup: 0.1`). The message names
  the port and the fix.

## Open questions

1. Rename `clone_source` to `canvas_sampler` (type id, file, engine query
   names, frontend gesture module)? Mechanical, touches the frontend, and
   `clone.yaml`. Recommended as its own follow-up commit, not this step.
2. Resolved by review: `paint.opacity` stays a commit-time cap and is not
   exposed on the Dry Smudge (section 7).
3. Resolved by review: the Dry Smudge thumbnail fallback shows the sampler's
   `fa6-solid:clone` icon for this step; the icon test row is added. A
   per-arm icon needs a method on the evaluator and is not worth it here.
4. Resolved by review: pen-down anchoring (section 12) stays out; nothing in
   this feature needs it.
5. Resolved by review: `LiveSource::PickupAtlas` is removed here; confirmed
   dead.

## LOC estimate

Lines added or removed, honest ranges.

| Area | Added | Removed | Files |
|---|---:|---:|---|
| `texture_source.rs` (variants, `is_chained`, docs) | 30 | 12 | 1 |
| `wgsl/mod.rs` (`dabs_are_chained`, prepend, mirror field, docs) | 30 | 4 | 1 |
| `read_mirror_terminal.rs` (extractions, constants, pass counter) | 110 | 70 | 1 |
| `smudge.rs` (import the shared constant) | 2 | 6 | 1 |
| `eval.rs` (hook, runner field, fold, hand-off) | 35 | 0 | 1 |
| `gpu_context.rs` + `scratch.rs` (`read_reach`, `dab_draws`, grow signal) | 35 | 2 | 2 |
| `paint.rs` (chained branch, flush restructure with per-iteration re-borrow, build-up check, docs) | 120 | 30 | 1 |
| `clone_source.rs` (ports, live arm, reach, docs) | 130 | 15 | 1 |
| `painting.rs` (`active_brush_needs_source`) | 8 | 2 | 1 |
| `watercolor.rs` (pass counter) | 2 | 0 | 1 |
| `dry_smudge.yaml`, `dry_media.yaml` | 75 | 1 | 2 |
| `stroke_replay_matrix.rs` (topology) | 12 | 0 | 1 |
| **Production total** | **~590** | **~145** | 14 |
| `tests/live_sampler.rs` (harness + tests 1 to 7) | 420 | 0 | 1 |
| engine-level replay test (8) | 70 | 0 | 1 |
| unit tests in `clone_source.rs`, `builtin_brushes.rs` row | 40 | 4 | 2 |
| **Tests total** | **~530** | **~4** | 4 |
| `docs/brush/architecture.md`, perf doc section E | 45 | 5 | 2 |
| bench results (generated by the bench, already on disk) | 60 | 0 | 2 |
| **Generated / docs total** | **~105** | **~5** | 4 |
| **Grand total** | **~1220** | **~155** | |

Roughly 445 net production lines, of which about 120 are the read-mirror
extraction that the three existing terminals then share and about 30 are
the `dab_draws` counter, whose only job is test 6. The single largest
production file change is `clone_source.rs`'s live arm.

## Out of scope (later steps, per the brief)

A per-fragment `displacement` port scaling the motion vector; a "lift" channel
and a displace/duplicate dial; deleting the `smudge` terminal; the hybrid
compute path or any change to the serialized flush's cost; anything
watercolor; the `clone_source` rename.
