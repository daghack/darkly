//! Three renders of one stroke must agree: the incremental live path
//! (checkpoint ring region save and restore), the live path forced through
//! a from-scratch re-render on every rewind, and `stroke_path`, which draws
//! the whole known path once.
//!
//! Every mid-stroke event rewinds to a checkpoint, restores it, and
//! replays the dabs after it. The ring copies only the region those dabs
//! dirtied, so a region that is one texel too small leaves a stale dab or
//! a hole. The
//! oracle is the engine's own full re-render path: `test_set_full_rerender`
//! clears the ring before every rewind, so each event renders the final
//! polyline from index 0 with the terminal's whole-scratch prologue.
//! That path and the incremental one must produce the same bytes.
//!
//! Incremental-vs-oracle cells run at `stabilize = 0`, where only the
//! synthetic tip correction rewinds and the two paths are exactly equal; a
//! reported divergence at `stabilize > 0` leaves the segment before it
//! drawn with a lookahead point that has since moved, so a from-scratch
//! render differs there regardless of the ring. `stroke_path` equals the
//! oracle at every strength: both render the final polyline from scratch.
//!
//! Run with: `cargo test -p darkly --test stroke_rewind --features testing -- --test-threads=1`

use std::path::PathBuf;

use darkly::brush::builtin_brushes;
use darkly::brush::gpu_context::MAX_DABS_PER_PHASE;
use darkly::brush::input_value::InputValue;
use darkly::config::ConfigValue;
use darkly::coord::CanvasRect;
use darkly::engine::types::StrokeOp;
use darkly::engine::DarklyEngine;
use darkly::format::stroke_recording::{replay, EventTiming, ReplayPacing, StrokeRecording};
use darkly::gpu::context::GpuContext;
use darkly::gpu::test_utils::test_device;
use darkly::layer::LayerId;

fn fixture(name: &str) -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "tests", "fixtures", name]
        .iter()
        .collect()
}

fn install_builtin(engine: &mut DarklyEngine, name: &str) {
    let brush = builtin_brushes::all()
        .into_iter()
        .find(|b| b.metadata.name == name)
        .unwrap_or_else(|| panic!("{name} is a builtin"));
    let json = serde_json::to_string(&brush.metadata.graph).expect("serialize brush graph");
    engine
        .set_brush_graph(&json)
        .unwrap_or_else(|e| panic!("{name} compiles: {e:?}"));
}

fn set_input(engine: &mut DarklyEngine, type_id: &str, port: &str, value: f32) {
    let id = engine
        .active_brush_graph()
        .nodes()
        .values()
        .find(|n| n.type_id == type_id)
        .unwrap_or_else(|| panic!("no '{type_id}' node in active graph"))
        .id
        .0
        .clone();
    engine
        .brush_graph_set_input(&id, port, InputValue::Scalar(value))
        .unwrap_or_else(|e| panic!("{type_id}.{port}: {e:?}"));
}

/// A headless engine with `brush` installed at the given stabilizer
/// strength.
fn new_engine(canvas: (u32, u32), brush: &str, stabilize: f32) -> DarklyEngine {
    let (device, queue) = test_device();
    let mut engine = DarklyEngine::new(GpuContext::new_headless(device, queue), canvas.0, canvas.1);
    install_builtin(&mut engine, brush);
    set_input(&mut engine, "brush_settings", "stabilize", stabilize);
    engine
}

fn event(x: f32, y: f32, t: f64) -> StrokeOp {
    pen_event(x, y, 1.0, t)
}

fn pen_event(x: f32, y: f32, pressure: f32, t: f64) -> StrokeOp {
    StrokeOp::BrushStroke {
        x,
        y,
        pressure,
        x_tilt: 0.0,
        y_tilt: 0.0,
        rotation: 0.0,
        tangential_pressure: 0.0,
        time_ms: t,
        cr: 0.0,
        cg: 0.0,
        cb: 0.0,
        ca: 1.0,
    }
}

/// One stroke configuration, run either incrementally (the ring's region
/// copies) or through the forced full re-render (the oracle).
struct Cell<'a> {
    brush: &'a str,
    /// `paint.buildup`; `Some` adds the `build` storage channel, so the
    /// ring snapshots and restores two grounds.
    buildup: Option<f32>,
    canvas: (u32, u32),
    /// Canvas window to crop to before the stroke, giving the window a
    /// non-zero plane origin.
    crop: Option<CanvasRect>,
    stabilize: f32,
}

/// Every op of a stroke through the live path.
fn live(engine: &mut DarklyEngine, layer: LayerId, ops: &[StrokeOp]) {
    engine.begin_stroke(layer).unwrap();
    for op in ops {
        engine.stroke_to(*op);
    }
    engine.end_stroke();
}

/// Differing RGBA pixels between two layer readbacks: `(x, y, a, b)`.
fn differing_pixels(a: &[u8], b: &[u8], bounds: CanvasRect) -> Vec<(i32, i32, [u8; 4], [u8; 4])> {
    a.as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, (a, b))| {
            let x = bounds.origin.x + (i as u32 % bounds.width) as i32;
            let y = bounds.origin.y + (i as u32 / bounds.width) as i32;
            (x, y, *a, *b)
        })
        .collect()
}

impl Cell<'_> {
    fn run(
        &self,
        full_rerender: bool,
        drive: impl FnOnce(&mut DarklyEngine, LayerId),
    ) -> (Vec<u8>, CanvasRect) {
        // Prediction draws a tail ahead of the pen that a from-scratch
        // render of the real samples never does. The pref is thread-global,
        // so pin it per run.
        darkly::config::set("input.predictionHorizon", ConfigValue::Float(0.0));
        let mut engine = new_engine(self.canvas, self.brush, self.stabilize);
        // Random-node brushes seed from the wall clock otherwise.
        engine.set_stroke_seed(Some(0x5eed));
        if let Some(b) = self.buildup {
            set_input(&mut engine, "paint", "buildup", b);
        }
        let layer = engine.add_raster_layer(None);
        if let Some(window) = self.crop {
            engine.resize_canvas(window);
        }
        engine.test_set_full_rerender(full_rerender);
        drive(&mut engine, layer);
        engine.test_flush_readbacks();
        if !full_rerender {
            assert_eq!(
                engine.test_stroke_full_rerender_events(),
                0,
                "the incremental run fell back to a full re-render, so this \
                 comparison would be the oracle against itself"
            );
        }
        let bounds = engine.layer_bounds(layer).expect("raster layer has bounds");
        (engine.test_readback_layer(layer), bounds)
    }

    /// Both runs of `drive`, compared byte for byte. Returns the layer
    /// bounds for the caller's structural assertions.
    fn assert_matches_oracle(&self, drive: impl Fn(&mut DarklyEngine, LayerId)) -> CanvasRect {
        self.assert_same(
            "incremental rewind",
            self.run(false, &drive),
            "full re-render",
            self.run(true, &drive),
        )
    }

    /// `stroke_path` of `ops` against the live path, forced through the full
    /// re-render (`oracle`) or not. `setup` runs first on both engines.
    fn assert_path_matches_live(
        &self,
        oracle: bool,
        setup: impl Fn(&mut DarklyEngine, LayerId),
        ops: &[StrokeOp],
    ) -> CanvasRect {
        let path = self.run(false, |engine, layer| {
            setup(engine, layer);
            engine.stroke_path(layer, ops).unwrap();
        });
        let live = self.run(oracle, |engine, layer| {
            setup(engine, layer);
            live(engine, layer, ops);
        });
        self.assert_same(
            "stroke_path",
            path,
            if oracle {
                "live full re-render"
            } else {
                "default live"
            },
            live,
        )
    }

    fn assert_same(
        &self,
        a_name: &str,
        (a, bounds): (Vec<u8>, CanvasRect),
        b_name: &str,
        (b, b_bounds): (Vec<u8>, CanvasRect),
    ) -> CanvasRect {
        assert_eq!(
            bounds, b_bounds,
            "{a_name} and {b_name} grew the layer differently ({})",
            self.brush
        );
        assert!(
            b.as_chunks::<4>().0.iter().any(|px| px[3] > 0),
            "the stroke left the layer empty ({})",
            self.brush
        );
        if a != b {
            let differing = differing_pixels(&a, &b, bounds);
            panic!(
                "{a_name} differs from {b_name} in {} of {} pixels ({} at stabilize {}, \
                 buildup {:?}, layer {bounds:?}); first few (x, y, {a_name}, {b_name}): {:?}",
                differing.len(),
                b.len() / 4,
                self.brush,
                self.stabilize,
                self.buildup,
                &differing[..differing.len().min(8)]
            );
        }
        bounds
    }
}

fn replay_recording(
    engine: &mut DarklyEngine,
    layer: LayerId,
    canvas: (u32, u32),
) -> Vec<EventTiming> {
    let recording =
        StrokeRecording::load(&fixture("recorded_curvy_stroke.json")).expect("fixture parses");
    let timings = replay(
        engine,
        &recording,
        layer,
        canvas,
        ReplayPacing::AsFastAsPossible,
        None,
    );
    assert_eq!(timings.len(), recording.events.len());
    timings
}

/// A checkpoint save never submits on its own: it is recorded into the
/// submission of the segment it snapshots. At `stabilize = 0` the
/// synthetic tip correction rewinds to the checkpoint two indices below
/// the tip and replays exactly two segments, so an event is at most four
/// submissions: the stroke prologue or the rewind, the two segments
/// (one may place no dab and still submits for its save), and the
/// commit. A save in a submission of its own makes it six. Checked per
/// event rather than as a stroke total, which could not tell one event
/// over from another under.
#[test]
fn checkpoint_saves_share_their_segment_submission() {
    let canvas = (1024, 512);
    let mut engine = new_engine(canvas, "Ink Pen", 0.0);
    let layer = engine.add_raster_layer(None);
    let timings = replay_recording(&mut engine, layer, canvas);
    engine.test_flush_readbacks();
    assert_eq!(engine.test_stroke_full_rerender_events(), 0);
    let over: Vec<(usize, u32)> = timings
        .iter()
        .filter(|t| t.submits > 4)
        .map(|t| (t.index, t.submits))
        .collect();
    assert!(
        over.is_empty(),
        "events with more submissions than rewind + two segments + commit (index, submits): \
         {over:?}"
    );
}

/// A stroke that walks inside the window, jumps off its left and top edges
/// in one event (growing the layer to a negative origin), walks on
/// outside, reverses, and ends with a 90 px jump back inside, so a
/// rewind's dirtied region lands wholly outside the restored slot's frame.
///
/// Every edge crossing is a single jump longer than the dab's reach. A dab
/// whose footprint merely crosses the layer edge is clipped by design
/// (`StrokeOp::required_coverage` grows the layer only once a dab centre
/// escapes it) and is re-rendered unclipped only if a later rewind reaches
/// it; that history is not something a from-scratch render can reproduce,
/// and it is not the ring's doing.
fn grow_reverse_jump(engine: &mut DarklyEngine, layer: LayerId) {
    live(engine, layer, &grow_reverse_jump_ops());
}

fn grow_reverse_jump_ops() -> Vec<StrokeOp> {
    let mut ops = Vec::new();
    let mut go = |x: f32, y: f32| ops.push(event(x, y, ops.len() as f64 * 16.0));
    let (mut x, mut y) = (150.0, 120.0);
    for _ in 0..5 {
        go(x, y);
        x -= 12.0;
        y -= 10.0;
    }
    (x, y) = (-40.0, -40.0);
    go(x, y);
    for _ in 0..4 {
        x -= 12.0;
        y -= 10.0;
        go(x, y);
    }
    for _ in 0..6 {
        x += 12.0;
        y += 10.0;
        go(x, y);
    }
    go(x + 90.0, y + 90.0);
    ops
}

/// The recorded curvy stroke (204 events with reversals) at the downlevel
/// limit: slots are reused across indices, outgrown and reallocated, and
/// the stroke crosses frame boundaries many times.
#[test]
fn recorded_stroke_rewinds_match_full_rerender() {
    // Under the headless device's downlevel limits, as in
    // `tests/stroke_replay.rs`.
    let canvas = (1024, 512);
    let cell = Cell {
        brush: "Ink Pen",
        buildup: None,
        canvas,
        crop: None,
        stabilize: 0.0,
    };
    cell.assert_matches_oracle(|engine, layer| {
        replay_recording(engine, layer, canvas);
    });
}

/// Black vertical bars across the layer in the Ink Pen, then `brush`
/// back at `stabilize = 0`: something for a brush that moves existing
/// pigment to move. The bars stay a dab's reach inside the layer: pigment
/// at the border would let the smear cross the edge at a dab clipped
/// before the layer grows, the history `grow_reverse_jump`'s note says a
/// from-scratch render cannot reproduce.
fn lay_stripes(
    engine: &mut DarklyEngine,
    layer: LayerId,
    canvas: (u32, u32),
    brush: &str,
    stabilize: f32,
) {
    install_builtin(engine, "Ink Pen");
    set_input(engine, "brush_settings", "stabilize", 0.0);
    const INSET: u32 = 64;
    for x in (INSET..canvas.0 - INSET).step_by(48) {
        engine.begin_stroke(layer).unwrap();
        for i in 0..=8 {
            let y = INSET as f32 + (canvas.1 - 2 * INSET) as f32 * i as f32 / 8.0;
            engine.stroke_to(event(x as f32, y, i as f64 * 16.0));
        }
        engine.end_stroke();
    }
    install_builtin(engine, brush);
    set_input(engine, "brush_settings", "stabilize", stabilize);
}

/// The recorded stroke through the Smudge over a striped layer: every
/// rewind restores the grounds and the next dab's appearance snapshot
/// reads them back, and the appearance mirror itself is never
/// checkpointed, so a mirror texel a dab read without refreshing it shows
/// here as a difference from the full re-render.
#[test]
fn live_sampler_rewinds_match_full_rerender() {
    let canvas = (1024, 512);
    let cell = Cell {
        brush: "Smudge",
        buildup: None,
        canvas,
        crop: None,
        stabilize: 0.0,
    };
    cell.assert_matches_oracle(|engine, layer| {
        lay_stripes(engine, layer, canvas, "Smudge", 0.0);
        replay_recording(engine, layer, canvas);
    });
}

/// The same stroke with the `build` channel declared: the ring snapshots
/// and restores two `r32uint` grounds with one set of rects, and the
/// commit reads both, so a channel the restore missed shows in the layer.
#[test]
fn two_grounds_rewind_together() {
    let canvas = (1024, 512);
    let cell = Cell {
        brush: "Ink Pen",
        buildup: Some(0.5),
        canvas,
        crop: None,
        stabilize: 0.0,
    };
    cell.assert_matches_oracle(|engine, layer| {
        replay_recording(engine, layer, canvas);
    });
}

/// A cropped window with a non-zero plane origin, a mid-stroke layer grow
/// to a negative origin, a reversal and a jump: slots saved under the old
/// extent are restored under the new one, and the first save after the
/// grow reallocates the slot. One ground and two.
#[test]
fn growth_and_crop_rewind_match_full_rerender() {
    for buildup in [None, Some(0.5)] {
        let cell = Cell {
            brush: "Ink Pen",
            buildup,
            canvas: (256, 192),
            crop: Some(CanvasRect::from_xywh(16, 8, 224, 176)),
            stabilize: 0.0,
        };
        let bounds = cell.assert_matches_oracle(grow_reverse_jump);
        assert!(
            bounds.origin.x < 0 && bounds.origin.y < 0,
            "the stroke must have grown the layer past the plane origin: {bounds:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// `stroke_path`: the whole known path, drawn once.
// ---------------------------------------------------------------------------

/// Canvas for the striped `stroke_path` cells: tall enough that
/// `lay_stripes`' 64 px inset leaves real bars.
const STRIPED: (u32, u32) = (384, 256);

/// ~40 events across the striped layer, with a reversal and pressure that
/// varies along the curve (so pressure-wired commit values differ between
/// dabs). Every dab centre stays well inside the layer, so nothing clips
/// before a growth.
fn striped_curve() -> Vec<StrokeOp> {
    let mut ops = Vec::new();
    for i in 0..=24 {
        let t = i as f32 / 24.0;
        let x = 100.0 + 180.0 * t;
        let y = 128.0 + 40.0 * (t * std::f32::consts::TAU).sin();
        ops.push(pen_event(x, y, 0.3 + 0.7 * t, ops.len() as f64 * 16.0));
    }
    for i in 1..=16 {
        let t = i as f32 / 16.0;
        let x = 280.0 - 120.0 * t;
        let y = 128.0 + 30.0 * t;
        ops.push(pen_event(x, y, 1.0 - 0.6 * t, ops.len() as f64 * 16.0));
    }
    ops
}

/// The stripes under `brush` at `stabilize`, with a clone source set so the
/// Clone brush has something to copy (every other brush ignores it).
fn striped_setup(brush: &str, stabilize: f32) -> impl Fn(&mut DarklyEngine, LayerId) + '_ {
    move |engine, layer| {
        lay_stripes(engine, layer, STRIPED, brush, stabilize);
        engine.set_clone_source(40.0, 40.0, None);
    }
}

fn striped_cell(brush: &str, stabilize: f32) -> Cell<'_> {
    Cell {
        brush,
        buildup: None,
        canvas: STRIPED,
        crop: None,
        stabilize,
    }
}

/// Every builtin brush, over stripes so the brushes that move pigment have
/// something to move: `stroke_path` equals the live full re-render at
/// strength 0 and at 0.6, and the default live path at 0. No name list, so
/// a new builtin is covered without an edit.
#[test]
fn stroke_path_matches_oracle_for_every_builtin() {
    let ops = striped_curve();
    for brush in builtin_brushes::all() {
        let name = brush.metadata.name.as_str();
        for stabilize in [0.0, 0.6] {
            striped_cell(name, stabilize).assert_path_matches_live(
                true,
                striped_setup(name, stabilize),
                &ops,
            );
        }
        // A failure here with the oracle arm above passing is a live-ring
        // bug for this brush (the incremental path disagreeing with its own
        // full re-render), not a `stroke_path` defect.
        striped_cell(name, 0.0).assert_path_matches_live(false, striped_setup(name, 0.0), &ops);
    }
}

fn recorded_ops(canvas: (u32, u32)) -> Vec<StrokeOp> {
    let recording =
        StrokeRecording::load(&fixture("recorded_curvy_stroke.json")).expect("fixture parses");
    let scale = (
        canvas.0 as f32 / recording.canvas_width as f32,
        canvas.1 as f32 / recording.canvas_height as f32,
    );
    recording
        .events
        .iter()
        .map(|e| e.to_stroke_op(scale))
        .collect()
}

/// The recorded curvy stroke (204 events, real reversals) through the Ink
/// Pen at its shipped strength: the stabilizer-divergence workload.
#[test]
fn stroke_path_matches_oracle_on_recorded_stroke() {
    let canvas = (1024, 512);
    let cell = Cell {
        brush: "Ink Pen",
        buildup: None,
        canvas,
        crop: None,
        stabilize: 0.6,
    };
    cell.assert_path_matches_live(true, |_, _| {}, &recorded_ops(canvas));
}

/// A cropped window, growth past the plane origin, a reversal and a jump:
/// `stroke_path` grows the layer to the same bounds and paints the same
/// bytes as the live path, with one ground and two.
#[test]
fn stroke_path_grows_like_live() {
    let ops = grow_reverse_jump_ops();
    for buildup in [None, Some(0.5)] {
        let cell = Cell {
            brush: "Ink Pen",
            buildup,
            canvas: (256, 192),
            crop: Some(CanvasRect::from_xywh(16, 8, 224, 176)),
            stabilize: 0.0,
        };
        let bounds = cell.assert_path_matches_live(false, |_, _| {}, &ops);
        assert!(
            bounds.origin.x < 0 && bounds.origin.y < 0,
            "the stroke must have grown the layer past the plane origin: {bounds:?}"
        );
    }
}

/// Erase is the commit's blend mode, read when the context is built, so a
/// whole-path stroke erases exactly as a live one.
#[test]
fn stroke_path_erases_like_live() {
    striped_cell("Ink Pen", 0.0).assert_path_matches_live(
        false,
        |engine, layer| {
            striped_setup("Ink Pen", 0.0)(engine, layer);
            engine.set_brush_blend_mode(1);
        },
        &striped_curve(),
    );
}

/// A boustrophedon over `canvas`, one event every 8 px.
fn zig_zag_ops(canvas: (u32, u32)) -> Vec<StrokeOp> {
    let mut ops = Vec::new();
    let (x0, x1) = (16.0, canvas.0 as f32 - 16.0);
    let mut y = 16.0;
    let mut forward = true;
    while y <= canvas.1 as f32 - 16.0 {
        let mut x = if forward { x0 } else { x1 };
        while (x0..=x1).contains(&x) {
            ops.push(event(x, y, ops.len() as f64 * 4.0));
            x += if forward { 8.0 } else { -8.0 };
        }
        forward = !forward;
        y += 8.0;
    }
    ops
}

/// A small Ink Pen at the 1 px spacing floor over a long zig-zag places more
/// dabs than one phase holds. `stroke_path` draws it in one phase, split at
/// the cap, and matches the live path, whose per-event segments stay far
/// below it.
#[test]
fn stroke_path_splits_phases_at_the_dab_cap() {
    let canvas = (1024, 256);
    let small_dense = |engine: &mut DarklyEngine, _: LayerId| {
        set_input(engine, "brush_settings", "size", 0.02);
        set_input(engine, "brush_settings", "spacing", 0.0);
    };
    let ops = zig_zag_ops(canvas);
    let cell = Cell {
        brush: "Ink Pen",
        buildup: None,
        canvas,
        crop: None,
        stabilize: 0.0,
    };
    let (path, bounds) = cell.run(false, |engine, layer| {
        small_dense(engine, layer);
        engine.stroke_path(layer, &ops).unwrap();
        assert!(
            engine.test_stroke_total_dabs() > MAX_DABS_PER_PHASE as u64,
            "the path must place more dabs than one phase holds: {}",
            engine.test_stroke_total_dabs()
        );
    });
    let live_run = cell.run(false, |engine, layer| {
        small_dense(engine, layer);
        live(engine, layer, &ops);
    });
    cell.assert_same("stroke_path", (path, bounds), "default live", live_run);
}

/// The point of the feature, pinned structurally rather than by timing: the
/// recorded stroke (which the live path draws in three to four submissions
/// per event) is one submission per dab phase.
#[test]
fn stroke_path_submits_once_per_dab_phase() {
    let canvas = (1024, 512);
    darkly::config::set("input.predictionHorizon", ConfigValue::Float(0.0));
    let mut engine = new_engine(canvas, "Ink Pen", 0.6);
    let layer = engine.add_raster_layer(None);
    engine.stroke_path(layer, &recorded_ops(canvas)).unwrap();
    let dabs = engine.test_stroke_total_dabs();
    let submits = engine.drain_brush_perf_delta().submits as u64;
    assert!(dabs > 0);
    assert!(
        submits <= 1 + dabs / MAX_DABS_PER_PHASE as u64,
        "{submits} submissions for {dabs} dabs"
    );
}

/// A non-brush op, a locked layer, or an open stroke refuse before anything
/// changes, and leave no stroke open or disturbed.
#[test]
fn stroke_path_refuses_without_side_effects() {
    let canvas = (256, 128);
    let fill = StrokeOp::FloodFill {
        x: 10.0,
        y: 10.0,
        r: 255,
        g: 0,
        b: 0,
        a: 255,
        tolerance: 0,
    };

    let mut engine = new_engine(canvas, "Ink Pen", 0.0);
    let layer = engine.add_raster_layer(None);
    let empty = engine.test_readback_layer(layer);
    let err = engine
        .stroke_path(layer, &[event(20.0, 20.0, 0.0), fill])
        .unwrap_err();
    assert!(err.contains("op 1"), "the refusal names the op: {err}");
    // No stroke was left open: a stray event paints nothing.
    engine.stroke_to(event(30.0, 30.0, 16.0));
    engine.test_flush_readbacks();
    assert_eq!(engine.test_readback_layer(layer), empty);

    engine.set_node_locked(layer, true);
    let err = engine
        .stroke_path(layer, &[event(20.0, 20.0, 0.0)])
        .unwrap_err();
    assert!(err.contains("locked"), "{err}");
    engine.test_flush_readbacks();
    assert_eq!(engine.test_readback_layer(layer), empty);

    // Mid-stroke: refused, and the open stroke finishes as if the call had
    // never been made.
    let run = |attempt: bool| {
        let mut engine = new_engine(canvas, "Ink Pen", 0.0);
        let layer = engine.add_raster_layer(None);
        engine.begin_stroke(layer).unwrap();
        for i in 0..6 {
            engine.stroke_to(event(40.0 + 10.0 * i as f32, 60.0, i as f64 * 16.0));
            if attempt && i == 2 {
                assert!(engine
                    .stroke_path(layer, &[event(200.0, 100.0, 0.0)])
                    .is_err());
            }
        }
        engine.end_stroke();
        engine.test_flush_readbacks();
        engine.test_readback_layer(layer)
    };
    assert!(
        run(false) == run(true),
        "the refused call disturbed the open stroke"
    );
}

/// A mask filter target takes the R8 commit path; `stroke_path` paints it
/// exactly as the live path does.
#[test]
fn stroke_path_paints_a_mask_like_live() {
    let canvas = STRIPED;
    let ops: Vec<StrokeOp> = striped_curve()
        .into_iter()
        .map(|op| match op {
            StrokeOp::BrushStroke {
                x,
                y,
                pressure,
                time_ms,
                ..
            } => {
                // Black on the white mask, so the dabs show.
                StrokeOp::BrushStroke {
                    x,
                    y,
                    pressure,
                    x_tilt: 0.0,
                    y_tilt: 0.0,
                    rotation: 0.0,
                    tangential_pressure: 0.0,
                    time_ms,
                    cr: 0.0,
                    cg: 0.0,
                    cb: 0.0,
                    ca: 1.0,
                }
            }
            other => other,
        })
        .collect();
    let run = |whole: bool| {
        let mut engine = new_engine(canvas, "Ink Pen", 0.0);
        let host = engine.add_raster_layer(None);
        engine.add_mask(host).expect("add mask");
        let mask = engine.host_mask_id(host).expect("host has a mask");
        if whole {
            engine.stroke_path(mask, &ops).unwrap();
        } else {
            live(&mut engine, mask, &ops);
        }
        engine.test_flush_readbacks();
        engine.test_readback_layer(mask)
    };
    let (path, live_run) = (run(true), run(false));
    assert!(
        live_run.iter().any(|&v| v < 128),
        "the stroke left the mask white"
    );
    let differing = path.iter().zip(&live_run).filter(|(a, b)| a != b).count();
    assert_eq!(
        differing, 0,
        "stroke_path differs from live on the mask in {differing} pixels"
    );
}
