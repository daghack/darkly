# Stabilized rewind: re-render the segment whose lookahead point moved

Follow-up deferred from `checkpoint-ring-delta-copies.md` (open question 2, decided 2026-10-02: keep separate). Not yet drafted through the planning workflow; this file records the problem so it is not forgotten.

## Problem

When the stabilizer reports divergence at polyline index `k`, `painting.rs` (the `div_idx` derivation near the `tip_div` comment) restores the newest checkpoint with `vi < k` and re-renders segments `k..=tip`. Segment `k - 1` is left as it was, but it was drawn as a Catmull-Rom curve whose trailing control point `p3` is `stabilized[k]` (`stroke_engine.rs`, `render_from_stabilized_range_to`, `p3_pt = next_neighbor`), the point that just moved. The scratch therefore differs, sub-pixel, from a from-scratch render of the final polyline at any `stabilize > 0`. At `stabilize = 0` the synthetic tip correction already rewinds one segment further, so that setting is exact.

## Fix

In the `div_idx` match: `Some(k) => Some(k.saturating_sub(1).min(tip_div))`. The ring's window then becomes `max_div + 1` everywhere `painting.rs` passes `max_div` (`CheckpointRing::spacing`, `pick_slot` via `save`, `has_anchor`, `compute_segment_boundaries`), since the deepest reachable rewind is one index lower. Derive it once in `brush_stroke_to` and document the `+ 1` beside the stabilizer's own bound in `docs/brush/stabilization.md`. Cost: one more segment per event.

## Regression test

`tests/stroke_rewind.rs` (from the delta-copies plan) gains cells at `stabilize = 0.6` and `1.0`: the incremental path must equal the forced full re-render byte for byte. These fail on the tree before this fix and pass after it.
