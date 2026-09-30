# Compute dispatch-per-dab spike (perf-tracking option F, attempt #5)

## Independent Review

Reviewer: a fresh agent given only the repository instructions and this
plan. Working tree at `dev` (`9d824e90` plus the uncommitted section F in
`docs/paint-compute-perf-tracking.md`). Every file, line and crate
reference below was checked against the tree and
`~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/{wgpu,wgpu-core,wgpu-hal,wgpu-types,naga}-29.0.4`.

**Verdict: revise.**

The diagnosis is right, the wgpu citations hold (with the corrections under
4), and ground-as-channel is the smallest honest way to run the full
recorded stroke. But the plan builds the whole terminal (about 690
production lines) before measuring the one number the hypothesis rests on,
and about 150 of those lines are timestamp infrastructure a spike does not
need. Reshape it into two stages: a small per-dispatch cost harness first,
the terminal second and only if the harness passes.

### 1. A smaller first experiment exists and should be stage 1

Section F leaves exactly one quantity unmeasured: the all-in cost of one
`dispatch_workgroups` inside a compute pass against a `read_write`
`r32uint` storage texture, barrier included, at N of the order of 1000 per
pass. Nothing else in the spike is new information. The stabiliser,
checkpoint ring, layer growth and dab counts are terminal-independent (the
`paint` topology pays them too), the real dab counts are already in the #3
and #4 tables (`dabs/ev`), and the plan already isolates the unpack. A
harness that measures the quantity directly answers the premise in a
fraction of the lines, on the same device, with no framework change, and
proves the `r32uint` read-write bind group layout and the dynamic-offset
mechanism work on this driver before a terminal is written around them.

**Stage 1 builds:** `crates/darkly/src/bin/dispatch_cost_bench.rs` with
`required-features = ["testing"]` (the pattern `stroke_replay_bench` and
`stroke_replay_matrix` use, `crates/darkly/Cargo.toml:79-85`), on
`bench_device()` (`crates/darkly/src/gpu/test_utils.rs:44`), one inline
WGSL module. Three shapes over the same N scattered 3x3 px "dabs" (one 8x8
workgroup each; a second axis at radius 10, 3x3 workgroups each) on a
3840x2160 target:

- (a) one compute pass, N x (`set_bind_group(1, dab_bg, &[i * 256])` +
  `dispatch_workgroups`) doing unpack / source-over / pack on an `r32uint`
  `read_write` storage texture. This is the section F shape.
- (b) the same N dispatches with the texture bound read-only
  (`texture_2d<u32>`, an INCLUSIVE usage) and the store dropped: identical
  CPU encode, no barrier. (a) minus (b) is the barrier's cost, the part of
  premise 1 nobody has a number for.
- (c) N single-instance render passes on an `Rgba8Unorm` attachment: the
  #1 / smudge shape, so the 42 us/dab figure from section E is reproduced
  on this machine in the same harness instead of compared across benches.

Each shape uploads its per-dab records with one `write_buffer` per
iteration (using the layout under 3). Timing: wall clock from encode start
through `queue.submit` to `device.poll(Wait)` returning, which is legal
here (a bench binary under the `testing` feature, the escape hatch
`docs/lessons-learned/gpu-lessons-learned.md` section 5 names), plus the
pass's begin/end timestamps resolved and read back the same blocking way
when the adapter has `TIMESTAMP_QUERY` (about 30 lines, no accumulator).
N in {300, 900, 2000}, 20 iterations each, p50 per shape as total ms and
us per dab, printed as a markdown table with the adapter name. No stroke,
no engine.

**Stage 1 measures:** us per dispatch all-in (CPU encode + submit + GPU
completion) for (a); the barrier's share, (a) minus (b); and (c) as the
in-harness pass-per-dab reference.

**The number that gates stage 2:** at N = 900 (the 4K r=1 cell's 916 dabs
per event), shape (a) completes in at most 9 ms wall, i.e. 10 us per
dispatch. Derivation: the recording's cadence is 17.3 ms per event (3536
ms over 204 events); #4's non-dab per-event cost at 4K is about 3.5 ms
(`cpu p50` 3702 us at 4K r=1000 with 6.5 dabs per event,
`bench-results/stroke-replay-matrix-paint-recorded_curvy_stroke-cc26144cf2.md`);
the unpack is one full-layer 4K pass, budget 2 ms; about 2 ms is margin so
the terminal is not built to land exactly on the cadence. If (a) is at or
above 30 us per dispatch (the order of (c)), section F's answer is no and
the doc records the harness table as attempt #5 with nothing else built.
Between 10 and 30 us the door is open but the terminal will not land within
noise of #4 at 4K r=1; record that and decide with the user before stage 2.

**Stage 1 LOC:** about 220 lines in the bin (no CLI: the axes are
constants, like the matrix), about 50 lines of inline WGSL, 3 lines in
`Cargo.toml`. It stays after either outcome: it is also the harness that
answers section E's "what does one render pass per dab cost here", and the
place a later port would compare `set_bind_group` + `dispatch` against
`dispatch_workgroups_indirect`.

**Stage 2** is the plan's terminal, gated on that number, with the
revisions under 2, 3, 5 and 6 applied. Its gate stays section F's
(`behind_by_ms` within noise of a same-session `paint` run); the stage 1
per-dispatch number is the tie-breaker the plan currently wants GPU
timestamps for.

### 2. Instrumentation: drop the lifted timestamps

`b146df4220`'s `PaintComputeTimestamps` is about 170 lines
(`paint_compute.rs:63-230` in that commit) plus about 40 in its flush, and
the plan adds a `drain_gpu_timings` trait hook on `BrushPipelineEntry`, a
`gpu_timings` field on `BrushPerfDelta`, the `EventTiming` copy and the
bench percentiles on top (24 + 20 + 12 lines in the estimate). None of it
is needed to judge the gate:

- The gate is `behind_by_ms`, and the tie-breakers the bench already
  carries are `cpu p50` (wall around `stroke_to`: encode, submit and any
  back-pressure; `format/stroke_recording.rs:211`) and `submit p50`
  (`stroke_replay_matrix.rs:332-335`). A GPU bottleneck shows up in
  `submit` exactly as it did for #3 ("submit blocks for 28.8 ms").
- The per-dispatch GPU number comes from stage 1, cleaner than dividing a
  stroke's pass time by a stabiliser-dependent dab count.
- The unpack is measured by the A/B the plan already names as its
  fallback (section 3.7): one run with the unpack skipped, the difference
  in `cpu p50` and `behind`. Coarser, for a cost the port does not keep.

Cut section 3.7's GPU timestamps and the trait hook. Keep the `dispatches`
counter (12 lines) and the `flushes/ev` correction. While in
`engine/perf.rs`: its header (`perf.rs:9-16`) still tells new measurements
to "grow the GPU-timestamp slot pattern in `PaintComputeTimestamps`", a
struct removed in `39c5566b`; two lines to point it at the bench harness,
since the plan edits that file anyway. Savings: about 150 production lines
and the only generic-but-single-consumer hook the plan introduces.

### 3. Dab index: dynamic offset is right; the rejected variant is cheaper

Verified: `Features::IMMEDIATES` is "currently a proposal" for WebGPU
(`wgpu-types-29.0.4/src/features.rs:1753-1760`); the `webgpu` backend's
compute `set_immediates` panics (`wgpu-29.0.4/src/backend/webgpu.rs:3347-3348`);
default `max_immediate_size` is 0 (`limits.rs:408`);
`min_uniform_buffer_offset_alignment` and
`min_storage_buffer_offset_alignment` both default to 256
(`limits.rs:398-399`). `@builtin(num_workgroups)` carries the grid, not an
index; a `z = i` grid launches i layers; indirect dispatch carries no
index. A dynamic offset on `set_bind_group` is the only core-WebGPU way to
hand a dispatch a per-dispatch scalar without changing its work, so the
mechanism is right.

The plan's option 2 (a 4-byte index at stride 256, records tight in a
storage buffer) is rejected with "the same upload bytes (the slot is padded
to the alignment either way)". That is wrong: slot i of the index buffer
always holds i, so it is written once at pipeline build and never uploaded
again. Per flush the upload is then the tight record array, 32 x N bytes
(29 KB at N = 916) instead of 256 x N (234 KB). On the web every
`writeBuffer` byte crosses the JS to GPU-process boundary, which the plan's
own risk list names as the cost it cannot measure; an 8x smaller per-flush
upload is free. The shader reads `dabs[idx.i]`, one cached load. Use option
2 in both stages so they measure the same mechanism.

### 4. Barriers between dispatches: verified, one correction

- `wgpu-core-29.0.4/src/command/compute.rs`: `flush_bindings` is at
  `:317-365` (its doc comment at `:285-316` says barriers "may be needed
  before each dispatch if a previous dispatch had a conflicting usage");
  `dispatch` at `:835-857` calls it, and it ends with
  `CommandEncoder::drain_barriers` (`:362`). The usage scope is per
  dispatch, as the plan says.
- `track/mod.rs:339-343`: `skip_barrier` is true only when the state is
  unchanged **and** the ordered mask contains it. `wgpu-types`
  `texture.rs:250` `INCLUSIVE = COPY_SRC | RESOURCE | DEPTH_STENCIL_READ |
  STORAGE_READ_ONLY`; `:253` `EXCLUSIVE` includes `STORAGE_READ_WRITE`.
  Vulkan's ordered mask is `INCLUSIVE`
  (`wgpu-hal-29.0.4/src/vulkan/adapter.rs:3079-3081`). The transition is
  pushed at `track/texture.rs:1317-1328`.
- What it costs: `vulkan/command.rs:234-280` turns each pending transition
  into a `vkCmdPipelineBarrier` whose stages come from
  `conv::map_texture_usage_to_barrier` (`vulkan/conv.rs:269-310`); for
  `STORAGE_READ_WRITE` that is all shader stages with `SHADER_READ |
  SHADER_WRITE`. Between every two dispatches the queue drains: dispatch
  n+1 starts only after dispatch n has retired and its writes are visible.
  Dispatches never overlap, so the per-dispatch GPU cost is at least the
  drain latency however few threads the dispatch has. The barrier covers
  the whole subresource range (the view is the whole texture), so it does
  not shrink with dab size.
- Correction: the plan says "every dispatch after the first gets a
  barrier". The first gets one too: the texture enters the pass in
  `COLOR_TARGET` (from the clear) or `STORAGE_READ_WRITE` (from the last
  flush), both EXCLUSIVE, so the transition into this pass's
  `STORAGE_READ_WRITE` is also a barrier. N dispatches, N barriers.

What the harness and the terminal must not do by accident: bind the ground
as a read-only usage and rely on dispatch order (the only barrier-free
state, and the one that does not carry writes forward), or run under wgpu
validation. `InstanceFlags::from_build_config`
(`wgpu-types-29.0.4/src/instance.rs:258-265`) enables `VALIDATION | DEBUG`
only under `debug_assertions`, so the `--release` in section 3.6 is
load-bearing: a debug run measures the validation layer, not the driver.

The plan's statement that the WebGPU spec orders dispatches within a pass
and makes their storage writes visible is consistent with wgpu-core's
stated intent (`compute.rs:295-303`) and with what stage 1 measures; I did
not check it against the spec text, and the plan should cite it as no more
than that.

### 5. `storage: bool` on `StrokeChannel`: smallest change, one missed cost, one shape nit

Callers checked: the struct is built at `paint.rs:65-69` and
`watercolor.rs:109-121` (the plan's `paint.rs:64-68` is one line off);
consumed by `build_channels` (`scratch.rs:616-664`), `ensure_channels`'s
equality test (`scratch.rs:246-250`; `derive(PartialEq)` picks the new
field up), `grow_write`'s rebuild (`scratch.rs:555-575`), `wgsl/mod.rs:279`
(gathered into `CompiledBrush.channels`) and `:891` (`assemble_shader`'s
`FsOut`), and the two `channel_bind_group` lookups at `paint.rs:698` and
`watercolor.rs:1019`, both on their own non-storage channel. With `storage:
false` on both existing declarations `build_channels` takes the same path
with the same usage flags and the same bind group, so no allocation, pass
or behaviour changes for shipped brushes. The rejected terminal-owned
texture is rightly rejected: `restore_before` copies only into the stroke
frame and the channel list (`painting.rs:1258-1262`). Making the ground the
write side instead (`scratch_format: R32Uint`) would need `Scratch::new` to
add `STORAGE_BINDING` by format and a uint canvas-copy layout
(`canvas_copy_layout_for`, `pipeline.rs:772-788`, only distinguishes
filterable from not), plus a spike-owned commit: more framework change, not
less. Channel-as-ground is the right call.

**Missed cost.** With the ground as a channel the checkpoint ring snapshots
and restores two 4-byte-per-pixel textures per checkpoint (write side plus
ground: `checkpoint_ring.rs:336-372` copies the cumulative save-point bbox
for the stroke frame and every `extra`), where `paint` at `buildup = 1`
snapshots one and a real port would snapshot one. The RGBA8 snapshot is
dead weight for the spike (section 3.5's "subtlety that is fine" says as
much). At 4K the cumulative bbox (`save_points.rs:64-66`) is most of the
canvas by mid-stroke, so this is tens of megabytes of extra copy per
checkpoint charged to the spike. Section 3.5's "what this leaves
unmeasured: nothing at the gate" is therefore not right. The bias runs
toward a false fail, never a false pass, so a pass is still trustworthy; a
marginal fail is not. Record it in the plan and in attempt #5, and if the
result is marginal, bound it from `paint`'s own checkpoint cost (the
`paint` cells at large radius, where dabs are few and checkpoints
dominate).

**Off by an order of magnitude.** Section 3.3 says "two `create_bind_group`
calls per event". Flushes are terminal-independent and the bench's current
`dispatches/ev` (flushes per event) is 9.3 to 10 at `stabilize = 1.0` on
this recording
(`bench-results/stroke-replay-matrix-smudge-recorded_curvy_stroke-d79ed0f518.md`),
so it is about twenty bind groups per event plus one per commit, and about
ten `write_buffer` uploads per event, not one. Neither changes the design;
both change what the plan tells the reader to expect in `cpu p50`.

**Shape nit.** A two-variant `bool` beside a `blend` field that is
meaningless for one variant is the shape the Modularity Principle asks to
avoid. `pub enum ChannelUse { Attachment { blend: wgpu::BlendState },
Storage }` in place of `blend` + `storage` makes the unused-field caveat the
plan wants to document impossible to express, at the same line count. For a
spike deleted on a fail either is acceptable; on a pass, the enum is what
should survive.

`clear_to_transparent` on an `r32uint` attachment: clear colours are
converted per attachment format in hal, and I found no wgpu-core validation
that rejects a float `Color` on a uint attachment, so the plan's fallback
should not be needed; test 3 catches it if it is.

### 6. Parity: the bench compares equivalent work

Checked against the shipped graph (`crates/darkly/brushes/ink_pen.yaml`):
`pen_input.pressure -> paint.size`, `pen_input.pressure ->
paint.build_flow`, `circle.softness: 0.1`, no `buildup` set. `paint.buildup`
defaults to 1.0 (`paint.rs:441-442`), so `shares` gives `(0, 1)`
(`paint.rs:88-91`): the compiled body is `return rgba * build_flow * sel`
(`paint.rs:768`), source-over onto the scratch, no channel allocated, and
`commit` runs `wash: None, build: Some(write side)` (`paint.rs:687-702`).
`circle.amplitude` defaults to 0.0 (`circle.rs:72-73`), so the Ink Pen's
sine circle is a plain disc and `brush_extent_factor = 1 + 0`
(`circle.rs:289-295`); the spike's `radius` bbox and `local_dist >= 1.0`
reject match the skeleton's `local_dist_px >= d.bbox_target_px` discard
(`wgsl/mod.rs:1029-1032`). `stamp` emits `a = color.a * mask; (rgb * a, a)`
(`stamp.rs:69-71`); the spike's `a = color.a * coverage * flow` is the same
product after `* build_flow`. Radius: `paint` and `watercolor` both
delegate to `read_mirror_terminal::effective_radius` (`paint.rs:475-477`,
`watercolor.rs:837-839`), so the plan's reference is the shared one.
`dab_size` is what the runner reads for spacing (`eval.rs:689-694`), so
with the same output the dab count and positions are identical. The
analytic-disc fixture (`tests/fixtures/analytic_disc.yaml`) also leaves
`buildup` unset and wires one `multiply.result` to both flows, so test 1
compares source-over against source-over.

The 4/255 and 2 % thresholds: at test 1's settings (flow 0.05, about 10
dabs per pixel, hard edge) each dab adds about 12/255 and both paths round
once per dab (the ROP and `pack4x8unorm`, both round-to-nearest in
practice), so 1 to 2 LSB of drift is expected and 4 is a reasonable first
bound. Fine as written, with the plan's own note that the number is
measured and then justified.

### 7. Removability: yes, once 2 is applied

With the timestamp hook gone, the delete is the node file, the shader, two
fixtures, one test file, the bench topology and its fixture-loading branch,
the `storage` flag (or enum) with its two declaration edits, and the
regenerated `nodes/mod.rs`. The `dispatches` counter and the `flushes/ev`
rename stay as a bench correction. `tests/docs_export.rs:271-289` reads
brush node registrations live and compares them with a fresh export, not a
committed file, so a node coming and going leaves no generated artifact.
Nothing else names the spike.

### 8. The bench: both claims verified

- `dispatches_per_event_avg` is `t.dab_flushes`
  (`stroke_replay_matrix.rs:314-315`): one per `flush_dabs` call, one draw
  for `paint`. The rename to `flushes/ev` is correct.
- The doc comment's `cargo run --release --bin stroke_replay_matrix`
  (`stroke_replay_matrix.rs:9-11`) omits `-p darkly --features testing`;
  `required-features = ["testing"]` is at `crates/darkly/Cargo.toml:83-85`.
  `stroke_replay_bench.rs:13-16` has the same omission; fix both in passing.
- The `--topology` panic message and `--help` (`stroke_replay_matrix.rs:167`,
  `:173`) already omit `smudge` and `liquify`; the plan's update should list
  every topology.
- The new topology follows the existing `Topology` pattern (`parse` /
  `slug` / `terminal_id` / `brush_name`). `write_markdown` and `main` print
  `brush_name()` (`:440`, `:504`), so the fixture topology needs a display
  name; `brush_graph_json` finds the settings node through
  `brush_settings::node_id` (`:204`), which the Ink Pen copy has.

### 9. LOC

The per-file numbers are plausible against the files named: `paint.rs` is
835 lines doing far more, and `b146df4220`'s timestamp code is about 170
lines, matching the "about 150 lifted" the plan admits. With 2 applied the
node file drops to roughly 250 and the production total to about 530. With
the two-stage shape the first commitment is about 270 lines of bench
tooling (stage 1) and no shipped code path; stage 2's roughly 530
production lines are spent only once a number says they are worth
spending. Redo the production / tests / generated split per stage.

### Required revisions

1. Two stages: stage 1 is the dispatch-cost harness under 1 with its
   numeric gate; stage 2 is the terminal, gated on it.
2. Drop the GPU timestamps and the `drain_gpu_timings` hook; keep the
   `dispatches` counter; fix the stale `perf.rs` header.
3. Static index buffer plus tight record array for the dab index, in both
   stages.
4. "Every dispatch after the first" becomes "every dispatch"; keep
   `--release` explicit and say why.
5. Record the doubled checkpoint traffic as a stated bias against the
   spike; fix "two bind groups per event"; prefer the enum over the bool if
   the flag outlives the spike.
6. Bench doc-comment and help-text fixes as listed under 8.


**Status:** stage 1 implemented and run; gate met (6.5 ms at N = 900, 7.2 us
per dispatch, barrier free). Recorded as "#5, stage 1" in
`docs/paint-compute-perf-tracking.md`. Stage 2 awaits approval.

A measurement spike, not a feature. It builds the smallest throwaway
terminal that has the shape section F of
[`docs/paint-compute-perf-tracking.md`](../paint-compute-perf-tracking.md)
describes, runs it on the existing stroke replay matrix next to the shipped
`paint` terminal, and records the result as attempt #5. Nothing shipped
changes behaviour; the whole spike is removable in one commit.

## Revision (orchestrator response to review)

All six required revisions are accepted. The review above is preserved
verbatim; the plan below is stage 2, edited in place where the review
asked, and this section defines stage 1.

1. **Two stages.** Stage 1 is the dispatch-cost harness below. Stage 2 is
   the terminal, built only if stage 1 passes its numeric gate. This is
   also what the user asked for: a basic experiment first, built on
   gradually.
2. **Timestamps dropped.** Section 3.7 keeps the 12-line `dispatches`
   counter, measures the unpack by A/B, and fixes the stale `perf.rs`
   header. About 150 lines gone.
3. **Static index buffer** plus tight record array, in both stages
   (section 3.2, revised choice).
4. **Every dispatch gets a barrier**, including the first; `--release` is
   stated as load-bearing (section 2.2).
5. **Doubled checkpoint traffic** recorded as a bias toward a false fail
   with its bound (section 3.5); "two bind groups per event" corrected to
   about twenty plus ten uploads (section 3.3); `ChannelUse` enum in place
   of `bool` + a meaningless `blend` (section 3.5), kept on either outcome.
6. **Bench text fixes** for both bins and the topology help (section 3.6).

### Stage 1: the dispatch-cost harness

**What it measures.** The one quantity section F of the perf doc leaves
unmeasured: the all-in cost of one `dispatch_workgroups` inside a compute
pass against a `read_write` `r32uint` storage texture, barrier included, at
N of the order of 1000 per pass. Everything else the terminal would add
(stabiliser, checkpoints, growth, dab counts) is terminal-independent and
already in the #3 and #4 tables. It also proves the `r32uint` read-write
bind group layout and the dynamic-offset index mechanism work on this
driver before a terminal is written around them.

**What it builds.** `crates/darkly/src/bin/dispatch_cost_bench.rs` with
`required-features = ["testing"]` (the pattern of `stroke_replay_bench` and
`stroke_replay_matrix`, `crates/darkly/Cargo.toml:79-85`), on
`bench_device()` (`crates/darkly/src/gpu/test_utils.rs:44`), one inline WGSL
module, no CLI (the axes are constants, like the matrix). Three shapes over
the same N scattered dabs on a 3840x2160 target, two dab sizes (3x3 px, one
8x8 workgroup each; radius 10, 3x3 workgroups each):

- **(a)** one compute pass, N x (`set_bind_group(1, &index_bg, &[i * 256])`
  + `dispatch_workgroups`) doing unpack / source-over / pack on an
  `r32uint` `read_write` storage texture. The section F shape.
- **(b)** the same N dispatches with the texture bound read-only
  (`texture_2d<u32>`, an INCLUSIVE usage) and the store dropped: identical
  CPU encode, no barrier. (a) minus (b) is the barrier's cost.
- **(c)** N single-instance render passes on an `Rgba8Unorm` attachment:
  the #1 / smudge shape, so section E's 42 us/dab figure is reproduced in
  the same harness on the same machine instead of compared across benches.

Each shape uploads its tight per-dab record array with one `write_buffer`
per iteration; the index buffer is static. Timing: wall clock from encode
start through `queue.submit` to `device.poll(Wait)` returning, legal here
(a bench binary under the `testing` feature, the escape hatch
`docs/lessons-learned/gpu-lessons-learned.md` section 5 names), plus the
pass's begin/end timestamps resolved and read back the same blocking way
when the adapter has `TIMESTAMP_QUERY` (about 30 lines, no accumulator).
N in {300, 900, 2000}, 20 iterations each, p50 per shape as total ms and
us per dab, printed as a markdown table with the adapter name and backend.
No stroke, no engine. Always `--release`.

**The gate for stage 2, written before the run.** At N = 900 (the 4K r = 1
cell's 916 dabs per event), shape (a) completes in at most **9 ms wall**,
i.e. **10 us per dispatch**. Derivation: the recording's cadence is 17.3 ms
per event (3536 ms over 204 events); #4's non-dab per-event cost at 4K is
about 3.5 ms (`cpu p50` 3702 us at 4K r = 1000 with 6.5 dabs per event,
`bench-results/stroke-replay-matrix-paint-recorded_curvy_stroke-cc26144cf2.md`);
the unpack is one full-layer 4K pass, budget 2 ms; about 2 ms is margin so
the terminal is not built to land exactly on the cadence.

- **At or under 10 us:** stage 2 proceeds as planned below.
- **At or above 30 us** (the order of shape (c)): section F's answer is no.
  The harness table is recorded as attempt #5 and nothing else is built.
- **Between:** the door is open but the terminal will not land within noise
  of #4 at 4K r = 1. Record the numbers and decide with the user before
  stage 2; the fallback shape is section E's hybrid.

**Tests.** The harness is a bench, not a test; its correctness check is
that shape (a)'s final texture, unpacked, equals shape (c)'s attachment
within 1 LSB per channel on the painted pixels (both are source-over of the
same premultiplied discs), asserted at the end of each run so the timings
are of equivalent work. `cargo check`, clippy and the dash check cover the
rest.

**Recording.** The table goes into `docs/paint-compute-perf-tracking.md`
under a new "Attempt #5, stage 1" heading with the adapter name, the three
shapes, the barrier share, and the gate decision, following the doc's
Working agreement. The harness stays after either outcome: it is also the
harness that answers section E's "what does one render pass per dab cost
here", and where a later port compares `set_bind_group` + `dispatch`
against `dispatch_workgroups_indirect`.

**Stage 1 LOC:** about 220 lines in the bin, about 50 lines of inline WGSL,
3 lines in `Cargo.toml`; no tests beyond the in-run parity assert; about 40
lines of docs. No shipped path touched.

## 1. Hypothesis and what the gate decides

**Hypothesis (section F, restated).** One compute pass per flush, one
`dispatch_workgroups` per dab sized to that dab's bounding box, one thread
per pixel, writing a stroke-resident `r32uint` read-write storage texture
(RGBA8 premultiplied packed with `pack4x8unorm`), can keep up with the
shipped instanced fragment terminal (`paint`, attempt #4) on every cell of
the matrix, including the many-dabs-per-event cells where a
pass-per-dab path (#1, and the smudge baseline in section E) collapses.

The two premises under B.1's earlier dismissal that were never measured:

1. that a dispatch inside one compute pass costs what a render pass costs;
2. that the scratch has to be a buffer and pay #3's texture-to-buffer round
   trip over the union bbox.

The spike removes premise 2 by construction (the scratch is a storage
texture: no `sync_in`, no `sync_out`, nothing is copied) and measures
premise 1 directly.

**Decision gate (from section F, unchanged).** `behind_by_ms` for the spike
within bench noise of #4 on:

- 3840x2160 at radius 1 (about 916 dabs per event),
- 1920x1080 at radius 1 and radius 10.

Bench noise is about +/- 20 ms over the 3.5 s stroke (section "#4: Where
#4 doesn't close a gap"). The #4 numbers those cells will be compared
against, from the synthesis table and the `cc26144cf2` run:

| cell | #4 behind (ms) | #4 cpu p50 (us) | #4 worst-frame (ms) | dabs/ev |
|---|---:|---:|---:|---:|
| 3840x2160, r=1 | +15 | 5224 | 33.2 | 916.5 |
| 1920x1080, r=1 | +16 | 4203 | 23.8 | 438.5 |
| 1920x1080, r=10 | +14 | 3855 | 24.9 | 308.8 |

Those numbers were taken on unrecorded hardware. The section E smudge
baseline (`d79ed0f518`, uncommitted) was taken on an Intel Raptor Lake-P
iGPU, which is also this machine (`lspci`: "Intel Corporation Raptor Lake-P
[UHD Graphics]"). The gate is therefore judged against a **fresh
`--topology paint` run on the same machine in the same session**, with the
`cc26144cf2` numbers as the historical reference only. The bench will print
the adapter name so attempt #5 records the hardware (step 8).

**A falsifiable prediction, written before the run.** At 4K + 1 px the
engine has about 17 ms per event and #4 spends about 5 ms of it. For the
spike to sit within noise of that, the all-in per-dispatch cost (CPU encode
of `set_bind_group` + `dispatch_workgroups`, plus the GPU-side barrier wgpu
inserts between dispatches, section 2.2) has to be under roughly 13 us
across 916 dispatches. The smudge baseline's pass-per-dab cost is about
42 us per dab (4K + 1 px: cpu p50 42908 us over 1021 dabs). If a dispatch
costs a third of a render pass or less, the spike passes; if it costs the
same, it fails by the same margin #1 did.

**Two stages (review finding 1).** The terminal below is stage 2. It is
gated on a stage 1 harness that measures the one unmeasured quantity
directly, in about 270 lines and no framework change; see the Revision
section for its design, its numeric gate (10 us per dispatch at N = 900)
and its outcomes.

**Outcomes.**

- **Pass:** the spike stays in the tree behind its bench topology, attempt
  #5 records the table, and a follow-up plan ports `paint` to this shape
  and consolidates the chained terminals (smudge, blur, liquify,
  watercolor) onto it. The spike itself is not the port: it is deleted by
  that follow-up once the real terminal exists.
- **Fail:** attempt #5 records the numbers that closed the door, and the
  same PR deletes the spike (node, shader, fixtures, test, bench topology).
  `ChannelUse` (section 3.5), the `flushes/ev` column rename and the
  `dispatches` counter (section 3.7) stay, since they are a shape
  improvement and a bench correction independent of the outcome.

## 2. What was verified before designing (facts, with sources)

### 2.1 wgpu 29 capabilities, on the `webgpu` target and native

The workspace pins `wgpu = "29.0"` with the `webgpu` feature
(`Cargo.toml:37`); the resolved crates are `wgpu-29.0.4`, `wgpu-core-29.0.4`,
`wgpu-types-29.0.4`, `wgpu-hal-29.0.4`, `naga-29.0.4` under
`~/.cargo/registry/src/index.crates.io-*/`.

- **`r32uint` read-write storage is core, no feature required.**
  `wgpu-types-29.0.4/src/texture/format.rs:965`:
  `Self::R32Uint => (s_all, atomic)`, where `s_all = STORAGE_READ_ONLY |
  STORAGE_WRITE_ONLY | STORAGE_READ_WRITE` (`:917-919`) and `atomic =
  attachment | storage | binding` (`:930`), so the guaranteed usages
  include `RENDER_ATTACHMENT`, `STORAGE_BINDING`, `TEXTURE_BINDING`,
  `COPY_SRC` and `COPY_DST` together. `Rgba8Unorm` is `s_ro_wo` only
  (`:972`): no read-write, which is why the pack is needed. Bind-group
  layout validation for `StorageTextureAccess::ReadWrite` requires no
  feature (`wgpu-core-29.0.4/src/device/resource.rs:2713-2722`; only
  `Atomic` adds `TEXTURE_ATOMIC`). The `webgpu` backend maps it to
  `GpuStorageTextureAccess::ReadWrite` (`wgpu-29.0.4/src/backend/webgpu.rs:2026-2027`).
- **`pack4x8unorm` / `unpack4x8unorm` are WGSL core in naga**
  (`naga-29.0.4/src/front/wgsl/parse/conv.rs:341,351`).
- **Immediates (push constants) are not an option on the web.**
  `Features::IMMEDIATES` says "WebGPU support is currently a proposal and
  will be available in browsers in the future"
  (`wgpu-types-29.0.4/src/features.rs:1742-1769`); the `webgpu` backend's
  `set_immediates` panics (`wgpu-29.0.4/src/backend/webgpu.rs:3347`), and
  the default `max_immediate_size` is 0 (`limits.rs:408`). The shipped
  pipelines already pass `immediate_size: 0` (`paint.rs:189,201`).
- **Pass-level timestamps need only `TIMESTAMP_QUERY`**
  (`wgpu-29.0.4/src/api/compute_pass.rs:189-196`), which `bench_device`
  requests when the adapter offers it
  (`crates/darkly/src/gpu/test_utils.rs:44-65`). The frontend's device never
  requests it, so the instrumentation is inert in production.

### 2.2 wgpu inserts a barrier between dispatches that share a read-write storage texture

- Compute passes have a usage scope **per dispatch**: `dispatch()` calls
  `flush_bindings`, which merges the active bind groups into the scope and
  ends with `CommandEncoder::drain_barriers`
  (`wgpu-core-29.0.4/src/command/compute.rs:285-365, 835-842`; the doc
  comment at `:295-298` says this is exactly for "barriers ... before each
  dispatch if a previous dispatch had a conflicting usage").
- A barrier is skipped only when the state is unchanged **and** inside the
  adapter's ordered mask (`wgpu-core-29.0.4/src/track/mod.rs:339-343`).
  Vulkan reports `TextureUses::INCLUSIVE` as ordered
  (`wgpu-hal-29.0.4/src/vulkan/adapter.rs:3080-3082`), which is `COPY_SRC |
  RESOURCE | DEPTH_STENCIL_READ | STORAGE_READ_ONLY`
  (`wgpu-types-29.0.4/src/texture.rs:250`). `STORAGE_READ_WRITE` is
  `EXCLUSIVE` (`:253`), so **every** dispatch gets a `vkCmdPipelineBarrier`:
  the first transitions in from the clear's `COLOR_TARGET` or the previous
  flush's usage, each later one from the previous dispatch's read-write
  (transition pushed at `track/texture.rs:1317-1328`, realised with all
  shader stages and `SHADER_READ | SHADER_WRITE` in
  `vulkan/command.rs:234-280`). N dispatches, N barriers. That barrier is
  part of the per-dispatch cost the gate measures, and it is what makes dab
  `n+1` see dab `n`'s writes. Every run is `--release`, and that is
  load-bearing: `InstanceFlags::from_build_config` enables validation under
  `debug_assertions`, which would charge validation to the dispatch.
- On the `webgpu` backend wgpu forwards `dispatchWorkgroups`
  (`wgpu-29.0.4/src/backend/webgpu.rs:3379-3382`) and the browser owns
  synchronisation; the WebGPU spec orders dispatches within a pass and
  makes a dispatch's storage writes visible to later dispatches in the same
  pass. The spike runs natively only, so the browser's per-dispatch cost is
  an open question (section 7).

### 2.3 The framework hooks the spike plugs into (nothing new is invented)

- Terminal hooks: `BrushNodeEvaluator::{evaluate_gpu, flush_dabs, commit,
  begin_stroke, render_cursor_preview, compile_wgsl,
  compile_cursor_preview_body}` (`crates/darkly/src/brush/eval.rs:296-420`).
  A terminal registers with `is_terminal: true`, and the runner then skips
  every upstream GPU node's `evaluate_gpu` (`eval.rs:1059-1068`), so only
  the terminal queues dabs. The runner looks up the terminal's `dab_size`
  output for spacing (`eval.rs:685-689`); the spike must declare it.
- Flush cadence: `flush_dabs` runs once at the end of every rendering phase
  (`stroke_engine.rs:439, 608, 655`), and each phase has its own command
  encoder and `submit_final` (`engine/painting.rs:1310-1314, 1348-1350,
  1354-1356`). `paint` already relies on "one `queue.write_buffer` per
  flush into one buffer" being safe under that cadence (`paint.rs:600-601`).
- `begin_stroke` runs at stroke start and at **every** rewind boundary
  (`painting.rs:1231-1245`); the framework prologue for
  `Lifecycle::ClearScratchToTransparent` clears the write side **and every
  declared channel** in one clear pass (`scratch.rs:349-360`,
  `eval.rs:1132-1160`, `eval.rs:252-264`).
- Checkpoints snapshot and restore the write side **plus
  `Scratch::channel_textures()`** (`painting.rs:1257-1260, 1324-1328,
  1372-1376`; `checkpoint_ring.rs:288-410` save, `:472-540` restore; slot
  textures are allocated per channel format with `COPY_SRC | COPY_DST`,
  `:85-141`). Channels grow in lockstep with the write side
  (`scratch.rs:555-575`).
- What a channel is today: an extra colour attachment on the terminal's
  instanced draw (`scratch.rs:105-116`), allocated by `build_channels` with
  `RENDER_ATTACHMENT | COPY_SRC | COPY_DST | TEXTURE_BINDING` **and a
  canvas-copy bind group whose layout samples as
  `Float { filterable: true }`** (`scratch.rs:614-660`,
  `pipeline.rs:340-360`). An `r32uint` view cannot satisfy that entry, so a
  channel in that format fails at bind-group creation today. This is the
  one framework generalisation the spike needs (section 3.5).
- The bench: `Topology` enum with a per-topology brush and terminal id
  (`stroke_replay_matrix.rs:64-125`), per-cell engine build through
  `engine.set_brush_graph(json)` (`:250-258`), per-event `BrushPerfDelta`
  drain in `format/stroke_recording.rs:206-225`, and `BrushPerfCounters` on
  `BrushGpuContext` (`gpu_context.rs:63-129`). The current markdown column
  headed `dispatches/ev` is `dab_flushes` (`stroke_replay_matrix.rs:308-310`):
  one per `flush_dabs` call, which for `paint` is one draw. It does not
  count dispatches.

### 2.4 Prior art

Skipped, per the Prior Art Principle: this is a measurement of a shape
already described in-repo (attempts #2 and #3 were compute terminals, and
`git show b146df4220:crates/darkly/src/brush/nodes/paint_compute.rs` still
holds a working compute flush with pass-level GPU timestamps), and the
design questions are all answered by the framework and the wgpu source
above. Krita's and GIMP's brush engines are CPU-side and say nothing about
per-dispatch cost on a GPU queue.

## 3. Design

### 3.1 The terminal node

`crates/darkly/src/brush/nodes/paint_dispatch_spike.rs`, `type_id =
"paint_dispatch_spike"`, display name "Paint (dispatch spike)", category
`output`, description stating it is a measurement spike. `build.rs`
discovers it; `nodes/mod.rs` is regenerated, nothing else registers it.

Registration: `is_gpu: true`, `is_terminal: true`, `supports_erase:
false`, `preview_staging: None`, `lifecycle:
Lifecycle::ClearScratchToTransparent`, `scratch_format:
COLOR_SCRATCH_FORMAT`, one pipeline registration (section 3.3).

Ports, chosen so the Ink Pen graph drives it with one node swapped:

| port | wire | notes |
|---|---|---|
| `position` in | Vec2 | as `paint.position` |
| `size` in | Scalar, 0..1, default 1 | per-touch modulation, as `paint.size`; radius via the shared `read_mirror_terminal::effective_radius` (`read_mirror_terminal.rs:353-356`) |
| `flow` in | Scalar, 0..1, default 1 | per-dab alpha scale; Ink Pen wires pressure here, as it does to `paint.build_flow` |
| `color` in | Vec4 | wired from `paint_color.color`. `paint_color` is a CPU node (`paint_color.rs:26`) seeded per dab (`eval.rs:908-911`), so `ctx.input("color").as_color()` is live in `evaluate_gpu`. This is how the spike gets its colour without the compiled uniform |
| `softness` in | Scalar, 0..1, default 0.1, `stroke_constant()` | the disc's feather band; 0.1 matches the Ink Pen's `circle.softness`, 0 matches the analytic disc |
| `rgba` in | Vec4 | accepted and **unused**. It keeps the `stamp.dab -> terminal.rgba` edge so the graph is shaped like the Ink Pen's and the compiled shader is the Ink Pen's, but the spike never instantiates that shader (below) |
| `dab_size` out | Vec2 | required by the runner for spacing |

**How the terminal treats the upstream compiled expression.** The
framework compiles every terminal's graph to a fragment shader
(`brush/mod.rs:341-350`), so the spike's `compile_wgsl` must return a
valid body. It declares the ground channel (section 3.5) and returns
`FsOut(rgba * sel, vec4<f32>(0.0))`; `compile_cursor_preview_body` returns
`vec4<f32>(0.0)`. Neither body is ever turned into a pipeline: the spike's
pipeline entry holds no per-brush state and never reads
`compiled.stroke_wgsl`, and `render_cursor_preview` is a no-op override,
so the shared preview cache (`pipeline.rs:819-830`) never sees the spike.
The disc is hand-written in the compute shader from the dab record's
position, radius and the `softness` constant. Justification: per-brush
WGSL assembly for compute is the expensive part of a real port and is not
what the gate measures; the Ink Pen is a plain soft disc, so a hand-written
disc is the same work.

**`evaluate_gpu`.** Mirrors `paint.rs:487-527`: read position, compute
`radius = effective_radius(ctx)`, `bbox_radius = radius *
compiled.brush_extent_factor + compiled.brush_extent_extra_px` (1.0 and 0
for the Ink Pen's sine circle, so footprints match `paint` exactly),
`record_dab_footprint` (`gpu_context.rs:366-388`; publishes the
save-point bbox and the union), then push the spike's own record onto
`dab_batch.bytes` and bump `count`:

```rust
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SpikeDab {
    pos: [f32; 2],      // canvas px
    radius: f32,        // canvas px; the disc's edge and the dispatch bbox
    flow: f32,
    color: [f32; 4],    // straight alpha; the shader premultiplies
}
```

`DabBatch` already exists for terminal-owned record layouts
("each terminal reinterprets via `bytemuck::cast_slice` against its own
record type", `gpu_context.rs:214-222`); the spike does not call
`queue_dab`, whose layout is the compiled one.

### 3.2 Compute shader and the per-dispatch dab index

`crates/darkly/shaders/brush/paint_dispatch_spike.wgsl`, one module with a
compute entry and the unpack vertex/fragment entries.

**Passing the dab index: a dynamic offset on the dab-record binding.**
Each dispatch must know which dab it is. Options considered:

1. **Immediates / push constants:** unavailable on the `webgpu` backend
   (section 2.1). Measuring with them natively would measure something the
   web cannot have.
2. **A per-dab uniform slot holding only a `u32` index, plus the record in
   a storage buffer:** works, but is the same `set_bind_group` with a
   dynamic offset as option 3 plus one more indirection in the shader, and
   the same upload bytes (the slot is padded to the alignment either way).
3. **The dab record itself in a uniform buffer bound with a dynamic
   offset, one slot per dab (chosen).** Per dab the encoder does
   `set_bind_group(1, &dab_bg, &[i * stride])` then
   `dispatch_workgroups(gx, gy, 1)`. Two commands per dab; the shader
   reads `var<uniform> dab: SpikeDab` with no index at all. The records
   for a flush are expanded to `stride` bytes each into a CPU `Vec<u8>` and
   uploaded with **one** `queue.write_buffer` per flush (916 records at 4K +
   1 px is 234 KB at a 256-byte stride). The buffer is `MAX_DABS_PER_PHASE
   * stride` = 4 MiB (`gpu_context.rs:138`), `UNIFORM | COPY_DST`, binding
   size `size_of::<SpikeDab>()`. The shipped `DynamicUniformRing` is not
   reused: its capacity is fixed at 256 slots (`pipeline.rs:56`) and it
   issues one `write_buffer` per slot (`:85-92`), which at 916 dabs per
   event would itself be a measurable CPU cost that has nothing to do with
   the hypothesis.
4. **One dispatch with N workgroups:** unordered, the "fatal as-stated"
   B above. **Indirect dispatch:** carries no index. **Encoding the index
   in the grid (a 3D grid with `z = i`):** launches `i` layers of work.

**Stride:** `stride = max(device.limits().min_uniform_buffer_offset_alignment,
256)`. Intel's Vulkan driver reports 64, but WebGPU's default limit is 256
and the gate models the web, so the native run pays the web's upload bytes.

**Workgroup size:** `@workgroup_size(8, 8, 1)`, 64 threads, the tile the
earlier compute attempts used, and the smallest size that fills a wave on
every vendor (AMD wave64 is the widest). For a 1 px dab the bbox is 3x3
and any size gives one mostly idle workgroup; the per-dispatch overhead is
the quantity under test, so a larger tile changes nothing at the gate
cells. Noted as a knob for the large-dab cells.

**Dispatch grid per dab (CPU, at flush):** `x0 = floor(pos.x - r)`, `x1 =
ceil(pos.x + r)`, same for y; `gx = ceil((x1 - x0) / 8)`, `gy = ceil((y1 -
y0) / 8)`. Both fit `max_compute_workgroups_per_dimension` (65535) by
orders of magnitude at 2000 px.

**Compute entry (sketch, final math in the file):**

```wgsl
struct FlushUniforms { layer_offset: vec2<i32>, layer_size: vec2<u32>, softness: f32, _pad: vec3<f32> }
struct SpikeDab { pos: vec2<f32>, radius: f32, flow: f32, color: vec4<f32> }
@group(0) @binding(0) var<uniform> u: FlushUniforms;
@group(1) @binding(0) var<uniform> dab: SpikeDab;        // dynamic offset picks the record
@group(2) @binding(0) var ground: texture_storage_2d<r32uint, read_write>;

@compute @workgroup_size(8, 8, 1)
fn cs_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let bbox_min = vec2<i32>(floor(dab.pos - dab.radius));
    let canvas_px = bbox_min + vec2<i32>(gid.xy);
    let layer_px = canvas_px - u.layer_offset;
    if (any(layer_px < vec2<i32>(0)) || any(layer_px >= vec2<i32>(u.layer_size))) { return; }
    // Pixel-centre convention (gpu-lessons-learned section 4): the fragment
    // path evaluates at (i + 0.5, j + 0.5); so does this.
    let local_dist = length(vec2<f32>(canvas_px) + 0.5 - dab.pos) / dab.radius;
    if (local_dist >= 1.0) { return; }
    // `shape_coverage` for r(theta) = 1: grad = 1, perp = 1 - local_dist,
    // same 0.004 floor as circle.rs.
    let coverage = smoothstep(0.0, max(u.softness, 0.004), 1.0 - local_dist);
    let a = dab.color.a * coverage * dab.flow;
    let src = vec4<f32>(dab.color.rgb * a, a);                    // stamp's premultiply
    let dst = unpack4x8unorm(textureLoad(ground, layer_px).r);
    let out = src + dst * (1.0 - src.a);                          // PREMULTIPLIED_SOURCE_OVER
    textureStore(ground, layer_px, vec4<u32>(pack4x8unorm(out), 0u, 0u, 0u));
}
```

The coverage is the `circle` node's `shape_coverage` (`_shape.wgsl:177-185`)
specialised to a plain disc, with the same band floor (`circle.rs:273`) and
the same `local_dist_px >= bbox` reject the skeleton emits
(`wgsl/mod.rs:1029-1033`), so the spike deposits what `paint` deposits
for the Ink Pen and the analytic disc. The selection mask is not sampled
(the bench has no selection; a real port must sample it, section 7).

**Unpack entries:** a fullscreen triangle whose fragment does
`unpack4x8unorm(textureLoad(ground_u, vec2<i32>(pos.xy), 0).r)` into the
RGBA8 write side with `blend: None` (replace). Ground and write side share
dimensions (`build_channels` allocates at `write_w x write_h`,
`scratch.rs:614-624`), so no viewport is needed. Binding:
`texture_2d<u32>`, `TextureSampleType::Uint`, no sampler.

**Revised choice (review finding 3).** The dab index goes through a
**static index buffer**: slot `i` at stride 256 holds the `u32` `i`,
written once at pipeline build and never uploaded again; the dab records
stay a tight `32 x N` byte storage array uploaded once per flush (29 KB at
N = 916 instead of 234 KB when the record itself is padded to the stride).
Per dab: `set_bind_group(1, &index_bg, &[i * 256])` then
`dispatch_workgroups`; the shader reads `dabs[idx.i]`, one cached load. The
earlier rejection of this shape ("the same upload bytes") was wrong, since
the padded buffer is static. Stage 1 uses the same mechanism so the two
stages measure the same thing, and on the web every uploaded byte crosses
the JS to GPU-process boundary, so the 8x smaller per-flush upload is free.

### 3.3 The pipeline entry

`PaintDispatchSpikePipeline: BrushPipelineEntry` (`pipeline.rs:185-206`),
registered through the node's `pipelines` and built once at engine init:
the compute pipeline, the unpack render pipeline, three bind-group layouts
(flush uniform; dab uniform with `has_dynamic_offset: true`; ground as
`StorageTexture { access: ReadWrite, format: R32Uint }`), the unpack layout
(`Texture { sample_type: Uint }`), the flush uniform buffer, the 4 MiB dab
buffer, and the static index buffer. No
`RefCell`, no per-brush cache: nothing here depends on the brush. `rings()`
returns empty.

Bind groups over the ground view and the write view are built per flush
and per commit rather than cached, because `ensure_channels` and
`grow_write` can reallocate the channel. Flushes are terminal-independent
and run 9 to 10 per event at `stabilize = 1.0` on this recording
(`bench-results/stroke-replay-matrix-smudge-recorded_curvy_stroke-d79ed0f518.md`),
so that is about twenty bind groups and ten record uploads per event plus
one bind group per commit: noise next to 900 dispatches, and the same
shape `paint` has for live graph textures (`paint.rs:623-660`), but the
reader should expect it in `cpu p50`.

### 3.4 Flush and commit

**`flush_dabs`** (per rendering phase):

1. Early-out on an empty queue; `take()` the bytes and count;
   `record_dab_flush_workload(count, union_w, union_h)`,
   `record_dab_flush(count)`, and the new `record_dispatches(count)`.
2. Write the flush uniform (`layer_offset`, `layer_size` from
   `paint_target.canvas_extent()` as `paint` does, `softness` from
   `ctx.input_f32("softness")`).
3. Expand the records to `stride` and `write_buffer` them once.
4. `begin_compute_pass` with `timestamp_writes` on slots 0 and 1 when the
   feature is present; `set_pipeline`; bind groups 0 and 2 once; then the
   per-dab loop: `set_bind_group(1, &dab_bg, &[i * stride])`,
   `dispatch_workgroups(gx_i, gy_i, 1)`.
5. If timestamps are on: resolve, copy to a fresh 32-byte readback buffer,
   register it as pending (the `b146df4220` pattern).

**`commit`** (per pen event): a render pass with `timestamp_writes` on
slots 2 and 3 that runs the unpack into `scratch.write_view()`, then the
unchanged `commit_brush_dab(encoder, pipelines, queue, wash: None, build:
Some(scratch.write_bind_group()), pre_stroke_bg, opacity: 1.0,
gpu.blend_mode)` (`paint_target_ext.rs:52-62`): exactly what `paint` does
at `buildup = 1.0`. The unpack is the one full-layer pass per event that a
real port would fold into `composite.wgsl`; it is timed separately
(section 3.7) so the gate is judged on the dispatch pass with the unpack
cost stated beside it.

**Erase.** `supports_erase: false`. The commit already routes
`gpu.blend_mode` to destination-out, so erase would work for free; it is
left out to keep the spike's surface at the gate's, and noted for the port.

### 3.5 Where the ground lives: a storage-only stroke channel

The `r32uint` ground must survive the stroke, be cleared at stroke start
and at every rewind boundary, be restored by checkpoints, and grow with
the layer. Everything on that list already happens to a **stroke channel**
(section 2.3). The spike therefore declares its ground as a channel from
`compile_wgsl` (`NodeWgsl::channels`, exactly as `paint` declares its build
channel at `paint.rs:64-68`), and the framework allocates, clears, snapshots,
restores and grows it with no spike-specific code in the engine.

The blocker (section 2.3) is that `build_channels` assumes every channel is
a float-sampleable colour attachment. The generalisation, in
`crates/darkly/src/brush/scratch.rs`:

- `StrokeChannel.blend` becomes `pub use: ChannelUse` with
  `enum ChannelUse { Attachment { blend: wgpu::BlendState }, Storage }`.
  The two existing declarations (`paint.rs:65-69`, `watercolor.rs:109-121`)
  become `Attachment { blend: <what they had> }`, which changes nothing
  about them; `derive(PartialEq)` on the struct picks the enum up for
  `ensure_channels`'s equality test (`scratch.rs:246-250`). An enum rather
  than a `storage: bool` beside a `blend` that would be meaningless for a
  storage channel: the unused-field caveat becomes impossible to express.
- `build_channels`: for `Storage`, the texture usage adds
  `STORAGE_BINDING` (keeping `RENDER_ATTACHMENT` so the framework's clear
  pass still clears it: `R32Uint` allows both, section 2.1) and **no**
  canvas-copy bind group is built. `bind_groups` becomes
  `Vec<Option<wgpu::BindGroup>>`; `channel_bind_group(name)` returns `None`
  for a storage channel and is otherwise unchanged (`paint.rs:566`'s
  `expect` is on a non-storage channel).
- A new `Scratch::channel_view(name) -> Option<&TextureView>` so the spike
  can bind the ground by name, never by position.

Verified consequences: `clear_to_transparent` and `clear_channel_views`
clear it as a colour attachment with `Color::TRANSPARENT` (0), legal for a
uint attachment with no pipeline bound; `grow_write` copies it with
`copy_texture_to_texture`; the checkpoint ring's `extra_formats` come from
`t.format()` so its slots are `r32uint`. The generated `FsOut` field for a
storage channel is never compiled (section 3.1); documented on the variant.

**A subtlety that is fine.** Checkpoints are saved before the event's
commit (`painting.rs:1316-1345, 1358-1386`), so a slot's RGBA8 snapshot
holds the previous event's unpack while its ground snapshot is current.
That is harmless: the RGBA8 write side is a derived mirror of the ground,
overwritten in full by the unpack at every commit, and nothing reads it
between a restore and the next commit. State this in the file.

**Rejected alternative: a terminal-owned texture.** Zero framework change,
but the spike would not see checkpoint restores (`begin_stroke` runs
before `restore_before`, `painting.rs:1243-1260`, and there is no
post-restore hook), so it would have to re-pack the restored RGBA8 scratch
into the ground at the first flush after every `begin_stroke`: a full-layer
pass per rewind event, which at `stabilize = 1.0` is every event, and which
the real port would never pay. It would contaminate the very number the
gate reads.

**What this leaves unmeasured, and a bias the reader must know.** With the
ground as a channel the spike runs the full recorded stroke through the
real stabiliser, checkpoint ring and rewinds at `stabilize = 1.0`, with
nothing bypassed. Two costs a port would not pay are charged to the spike:
the unpack (measured by A/B, section 3.7), and **doubled checkpoint
traffic**: the ring snapshots and restores both the RGBA8 write side and
the ground (`checkpoint_ring.rs:336-372`, over the cumulative save-point
bbox from `save_points.rs:64-66`), where `paint` at `buildup = 1` and a real
port each snapshot one texture. At 4K the cumulative bbox is most of the
canvas by mid-stroke, so this is tens of megabytes of extra copy per
checkpoint. The bias runs toward a false fail, never a false pass: a pass is
trustworthy, a marginal fail is not, and in that case the extra cost is
bounded from `paint`'s own large-radius cells, where dabs are few and
checkpoints dominate. Recorded in attempt #5 either way.

### 3.6 The bench

`crates/darkly/src/bin/stroke_replay_matrix.rs`:

- `Topology::PaintDispatchSpike`, slug `paint-dispatch-spike`, parsed from
  `--topology paint-dispatch-spike`; terminal id `paint_dispatch_spike`.
- The brush comes from a fixture instead of `builtin_brushes::all()`:
  `Topology::graph()` returns the Ink Pen graph for the paint-family
  topologies and, for the spike, the graph of
  `crates/darkly/tests/fixtures/ink_pen_dispatch_spike.yaml` loaded through
  `PortableBrush::into_brush(registry, id)` (`portable.rs:292`), the same
  route `tests/brush_accumulation.rs:71-80` uses. The fixture is the Ink
  Pen with `paint` replaced by `paint_dispatch_spike` (connections:
  `pen_input.position -> spike.position`, `pen_input.pressure ->
  spike.size`, `pen_input.pressure -> spike.flow`, `paint_color.color ->
  spike.color`, `stamp.dab -> spike.rgba`; `spike.softness: 0.1`). The
  `pen_input.size` / `stabilize` overrides are unchanged. Not a builtin,
  not shipped.
- Columns: rename the existing `dispatches/ev` to `flushes/ev` (that is
  what it counts: `dispatches_per_event_avg = t.dab_flushes`,
  `stroke_replay_matrix.rs:314-315`) and add a real `dispatches/ev` from
  the new counter (section 3.7). The TSV gains the same columns.
- `parse_args` help and the panic message list the new topology, and the
  `smudge` and `liquify` topologies they already omit. The doc comments of
  this bin and `stroke_replay_bench.rs` gain the `-p darkly --features
  testing` their `required-features` demand.
- `bench_device` prints `adapter.get_info().name` and backend once, so a
  results file names its hardware.

**Exact invocation, in this order, same machine, same session, nothing else
running:**

```bash
cargo run --release -p darkly --features testing --bin stroke_replay_matrix -- \
    --input crates/darkly/tests/fixtures/recorded_curvy_stroke.json --topology paint
cargo run --release -p darkly --features testing --bin stroke_replay_matrix -- \
    --input crates/darkly/tests/fixtures/recorded_curvy_stroke.json --topology paint-dispatch-spike
cargo run --release -p darkly --features testing --bin stroke_replay_matrix -- \
    --input crates/darkly/tests/fixtures/recorded_curvy_stroke.json --topology paint
```

The second `paint` run bounds thermal and background drift; if the two
`paint` runs disagree by more than the noise floor on any gate cell, the
session is repeated rather than interpreted. Output lands under
`crates/darkly/bench-results/stroke-replay-matrix-{paint,paint-dispatch-spike}-recorded_curvy_stroke-<sha>.{md,tsv}`.
The bin's `required-features = ["testing"]` (`crates/darkly/Cargo.toml:83-85`)
is why `--features testing` is on the command line; the bench's own doc
comment omits it and should be corrected in passing.

### 3.7 Instrumentation

- **`BrushPerfCounters.dispatches: u32`** (`gpu_context.rs:63-83`), with
  `record_dispatches(n)`, folded in `AddAssign`, exposed on
  `BrushPerfDelta` and `EventTiming`. `paint` records 1 per flush (one
  instanced draw), the spike records `count`. This is the `dispatches/ev`
  column, about 12 lines end to end.
- **No GPU timestamps.** The draft lifted `PaintComputeTimestamps` from
  `b146df4220` (about 170 lines) plus a generic `drain_gpu_timings` hook on
  `BrushPipelineEntry` and delta plumbing. Cut: `cpu p50` and `submit p50`
  already expose a GPU-bound flush (they did for #3), stage 1 supplies the
  per-dispatch GPU number directly, and the unpack is measured by **A/B**:
  one spike run as specified and one with the unpack pass disabled by a
  local edit that is not committed, the difference in `cpu p50` and
  `behind_by_ms` being the unpack's cost. Stated beside the gate number.
- `engine/perf.rs:9-16` still names the removed `PaintComputeTimestamps`;
  its header is corrected in passing.

## 4. Implementation steps

No production code changes before step 5 of the workflow; this is the
order for that step.

0. **Stage 1 first** (see the Revision section): the dispatch-cost
   harness, its table in the perf doc, and the gate decision with the
   user. Nothing below starts unless stage 1 passes.
1. `scratch.rs`: `ChannelUse`, usage, `Option` bind groups,
   `channel_view(name)`; `paint.rs` and `watercolor.rs` declarations move
   their blend into `Attachment { blend }`. `cargo check --workspace`.
2. `gpu_context.rs`, `engine/perf.rs`, `format/stroke_recording.rs`: the
   `dispatches` counter end to end; `paint.rs` records 1 per flush; the
   stale `perf.rs` header.
3. (removed: no timestamp plumbing.)
4. `shaders/brush/paint_dispatch_spike.wgsl`: compute + unpack entries.
5. `nodes/paint_dispatch_spike.rs`: registration, evaluator, pipeline
   entry, static index buffer. Build; `nodes/mod.rs` regenerates.
6. Fixtures: `tests/fixtures/ink_pen_dispatch_spike.yaml` and
   `tests/fixtures/analytic_disc_dispatch_spike.yaml` (the analytic disc
   with the terminal swapped: `multiply.result -> spike.flow`,
   `spike.softness: 0.0`).
7. `tests/paint_dispatch_spike.rs` (section 5); run it and the full
   suite with `--test-threads=1`.
8. Bench: topology, fixture loading, columns, adapter name, help text and
   doc comment fixes (both bins). Run the three invocations from 3.6, then
   the unpack A/B run.
9. Record attempt #5 in `docs/paint-compute-perf-tracking.md` under
   "Attempts", following the doc's Working agreement: shape, files, what it
   bought, why kept or removed, the bench table (all 28 cells, with
   `flushes/ev` and `dispatches/ev` beside `behind`), the same-session
   `paint` table for comparison, the unpack A/B, the doubled checkpoint
   traffic (section 3.5), the adapter name, and stage 1's per-dispatch
   number next to the 42 us/dab pass-per-dab figure from section E. Add the spike column
   to the "Bench synthesis" table and the row to the "Architectural cost
   model" table. Flip section F's "spike pending" to the outcome and link
   the attempt.
10. Apply the outcome (section 1): keep behind the topology, or delete the
    spike files in the same PR. `ChannelUse` and the `dispatches` counter
    stay either way (a shape improvement and a bench correction).
11. Full check suite from CONTRIBUTING.md, including
    `./scripts/check-dashes.sh`, `cargo sync-docs` (no region lists brush
    nodes, so no change is expected; the run confirms it), and the
    `ts-export` protocol test (no protocol type changes; confirms it).

## 5. Tests

All in `crates/darkly/tests/paint_dispatch_spike.rs`, GPU integration
tests on the shared test device, run with `--test-threads=1`. Not
regression tests: nothing here fixes a bug.

1. **`spike_deposits_what_paint_deposits`** (equivalent work). Two
   engines at 256x128. One installs `tests/fixtures/analytic_disc.yaml`
   and strokes the centreline at pressure 0.5, spacing 0.1, one pass
   (`brush_accumulation.rs:108-135` is the shape); the other installs
   `analytic_disc_dispatch_spike.yaml` and does the same. Compare
   `test_readback_layer` byte-for-byte: every channel within 4/255
   everywhere, fewer than 2 % of painted pixels differing by more than
   1/255, and peak alpha above 0 on both. The tolerance exists because
   `pack4x8unorm` and the ROP round independently per dab and the disc
   stacks about 10 dabs per pixel; the thresholds are initial and are
   tightened or justified in the file from the measured distribution
   (section 7).
2. **`spike_rewinds_through_checkpoints_like_paint`** (the ground survives
   restore). Replay `recorded_curvy_stroke.json` at 400x200 with
   `ReplayPacing::Fast` and `stabilize = 1.0` through the Ink Pen and
   through `ink_pen_dispatch_spike.yaml`; compare layers under the same
   tolerance. A ground that was not restored on rewind would re-deposit
   discarded dabs and blow the tolerance by a wide margin. Also assert the
   spike's own replay is byte-identical across two runs (determinism).
3. **`storage_channel_has_no_sampled_bind_group`**. Build a `Scratch`,
   `ensure_channels` with one `storage: true` `R32Uint` channel and one
   ordinary `Rgba8Unorm` channel: the first has `channel_view` and no
   `channel_bind_group`, the second has both; `clear_to_transparent` and
   `grow_write` run without validation errors.
4. **Counters.** In test 2, assert per event `dispatches == dabs_total`
   for the spike and `dispatches == dab_flushes` for `paint`.

Existing suites that enumerate registrations (`docs_export.rs`,
`wgsl.rs`, `port_ranges.rs`) pick the node up automatically and need no
edits; `docs_md.rs` is unaffected because no region lists brush nodes
(`README.md:52` and `docs/generated-markdown.md:19` are effects catalogs).

## 6. Architectural impact

- **Shipped behaviour:** unchanged. `paint`, `watercolor`, `smudge`,
  `liquify`, `blur` run the same passes with the same textures; the two
  existing channels are declared `storage: false` and are allocated as
  before. The frontend node palette lists the spike while it exists
  ("Nothing filters on [category]; every registered node appears in the
  palette", `nodegraph/registration.rs:22-26`); its description says what
  it is, and it is gone or replaced in the same PR.
- **Removable in one commit:** the node file, the shader, two fixtures, one
  test, the bench topology, and the `storage` flag. The counter and the
  timing hook are bench instrumentation with no spike-specific names.
- **Principles:** modular (one file, `register()`, no consumer edits);
  type-owned dispatch (the engine never names the spike; timings come
  through the pipeline-entry trait); ownership (the ground is stroke state
  and lives on the stroke's `Scratch` with the rest of it); document
  authority untouched (all of this is compositor-side stroke state).
- **No blocking readbacks:** the timestamp drain is the `b146df4220`
  non-blocking pattern behind a feature the frontend never enables.

## 7. Risks

- **Per-dispatch cost on WebGPU versus Vulkan.** The native number is a
  lower bound. In a browser every `setBindGroup` and `dispatchWorkgroups`
  crosses the JS to GPU-process boundary and is re-validated by the
  implementation; two calls per dab at 916 dabs per event may cost more
  than the Vulkan encode does. The spike cannot measure this. If the gate
  passes natively, a browser replay of the same recording through the
  spike (the node is registered, `set_brush_graph_yaml` loads the fixture,
  `engine/process_recording.rs` replays) is the first step of the port
  plan, before any consolidation work.
- **wgpu barrier behaviour.** Verified (section 2.2) that a barrier is
  inserted per dispatch on Vulkan; its cost is inside `gpu dispatch p50`.
  On the web the browser decides; same open question as above.
- **`r32uint` in the preview path.** Not reachable: `render_cursor_preview`
  is a no-op and the preview cache never sees the spike's shader. Hovering
  with the spike brush shows the generic ring. If a real port keeps a
  packed ground, the cursor preview keeps rendering through the fragment
  preview skeleton, which is separate from the stroke path.
- **8-bit packing precision.** Same 8 bits as the `Rgba8Unorm` scratch, but
  `pack4x8unorm` rounds the shader's float and the ROP rounds its blend
  result; per-dab rounding differs by up to 1 LSB and drifts across
  stacked dabs. Test 1 measures the drift. If it exceeds the tolerance the
  spike is still measuring equivalent work (same pixels touched, same law),
  but the port would have to choose between accepting it and a two-texel
  `rgba16` packing that doubles ground bandwidth.
- **Uint attachment clear.** `LoadOp::Clear(Color::TRANSPARENT)` on an
  `r32uint` attachment with no draw is expected to validate; if wgpu
  rejects the mixed float/uint clear list, the fallback is a `storage`
  channel clear through `clear_texture` in `clear_channel_views` (still
  framework-owned, still one place).
- **Bench noise and the +/- 20 ms floor.** A gate cell that lands within
  noise but consistently above `paint` is reported as such, not rounded to
  a pass; the per-event `gpu dispatch p50` and `cpu p50` columns are the
  tie-breakers and are recorded either way.
- **Bind groups per flush.** Two per event; if profiling shows them in
  `cpu p50`, cache by `Texture::global_id()`. Not expected to matter.

## 8. Open questions

1. **Does the native per-dispatch number transfer to browsers?** The gate
   is native; the product runs on WebGPU. A pass here buys a browser
   measurement, not the port. (Section 7, first bullet.)
2. **Is "a storage-only channel" the right home for the ground in the real
   port, or does the ground replace the write side?** In a port the packed
   ground would likely *be* the scratch (`scratch_format` on the
   registration, commit reading `texture_2d<u32>` directly) and the RGBA8
   write side would go, taking `commit_brush_dab`'s float sampling with
   it. The spike deliberately does not decide this; the `storage` flag is
   the smallest thing that makes the bench honest.
3. **How much pack rounding drift is acceptable?** Test 1 puts a number on
   it; the threshold in the test is a first guess, not a spec.
4. **Selection.** The spike ignores the selection mask. The layout is
   already `COMPUTE`-visible (`gpu/selection.rs:854-876`), so a port
   samples it with `textureSampleLevel` at no architectural cost; the
   bench does not exercise it either way.

## 9. LOC estimate (lines added or removed)

| bucket | file | est. |
|---|---|---:|
| production | `brush/nodes/paint_dispatch_spike.rs` (registration, evaluator, flush, commit, pipeline entry, static index buffer) | 300 |
| production | `shaders/brush/paint_dispatch_spike.wgsl` | 90 |
| production | `brush/scratch.rs` (`ChannelUse`, usage, `Option` bind groups, `channel_view`) | 30 |
| production | `brush/nodes/paint.rs`, `brush/nodes/watercolor.rs` (`Attachment { blend }`, `record_dispatches(1)`) | 6 |
| production | `brush/gpu_context.rs` (`dispatches` counter) | 12 |
| production | `engine/perf.rs`, `format/stroke_recording.rs` (delta field, `EventTiming` field, header fix) | 12 |
| production | `bin/stroke_replay_matrix.rs`, `bin/stroke_replay_bench.rs` (topology, fixture graph, columns, help, doc comments) | 80 |
| production | `gpu/test_utils.rs` (adapter name) | 4 |
| **stage 2 production total** | | **~535** |
| production (stage 1) | `bin/dispatch_cost_bench.rs` + inline WGSL + `Cargo.toml` | 275 |
| tests | `tests/paint_dispatch_spike.rs` | 180 |
| tests | `tests/fixtures/ink_pen_dispatch_spike.yaml`, `analytic_disc_dispatch_spike.yaml` | 95 |
| **tests total** | | **~275** |
| docs / generated | `docs/paint-compute-perf-tracking.md` attempt #5 + synthesis rows + section F status | 110 |
| docs / generated | `bench-results/*-paint-*` and `*-paint-dispatch-spike-*` (`.md` + `.tsv`, generated) | 140 |
| docs / generated | `brush/nodes/mod.rs` (generated) | 3 |
| **docs / generated total** | | **~255** |

The first commitment is stage 1: about 275 lines of bench tooling touching
no shipped path. Stage 2's roughly 535 production lines are built only if
stage 1 passes its gate. The 36 lines in `scratch.rs` and the two
declarations are the only stage 2 lines that touch shipped code paths, and
they change no allocation or pass for existing brushes. If the door closes
at stage 2, the delete removes about 780 of the lines above and keeps about
60 (`ChannelUse`, the counter, the column rename) plus the stage 1 harness.
