//! The checkpoint ring's region save and restore, pinned against a
//! from-scratch render.
//!
//! Every mid-stroke event rewinds to a checkpoint, restores it, and
//! replays the dabs after it. The ring copies only the region those dabs
//! dirtied (plan `docs/plans/checkpoint-ring-delta-copies.md`), so a
//! region that is one texel too small leaves a stale dab or a hole. The
//! oracle is the engine's own full re-render path: `test_set_full_rerender`
//! clears the ring before every rewind, so each event renders the final
//! polyline from index 0 with the terminal's whole-scratch prologue.
//! That path and the incremental one must produce the same bytes.
//!
//! All cells run at `stabilize = 0`, where only the synthetic tip
//! correction rewinds and the two paths are exactly equal; a reported
//! divergence at `stabilize > 0` leaves the segment before it drawn with
//! a lookahead point that has since moved (`docs/plans/stabilized-rewind-
//! lookahead.md`), so a from-scratch render differs there regardless of
//! the ring.
//!
//! Run with: `cargo test -p darkly --test stroke_rewind --features testing -- --test-threads=1`

use std::path::PathBuf;

use darkly::brush::builtin_brushes;
use darkly::brush::input_value::InputValue;
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
    StrokeOp::BrushStroke {
        x,
        y,
        pressure: 1.0,
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
}

impl Cell<'_> {
    fn run(
        &self,
        full_rerender: bool,
        drive: impl FnOnce(&mut DarklyEngine, LayerId),
    ) -> (Vec<u8>, CanvasRect) {
        let mut engine = new_engine(self.canvas, self.brush, 0.0);
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
        let (incremental, bounds) = self.run(false, &drive);
        let (oracle, oracle_bounds) = self.run(true, &drive);
        assert_eq!(
            bounds, oracle_bounds,
            "the two runs grew the layer differently"
        );
        assert!(
            oracle.as_chunks::<4>().0.iter().any(|px| px[3] > 0),
            "the stroke left the layer empty"
        );
        if incremental != oracle {
            let differing: Vec<(i32, i32, [u8; 4], [u8; 4])> = incremental
                .as_chunks::<4>()
                .0
                .iter()
                .zip(oracle.as_chunks::<4>().0)
                .enumerate()
                .filter(|(_, (a, b))| a != b)
                .map(|(i, (a, b))| {
                    let x = bounds.origin.x + (i as u32 % bounds.width) as i32;
                    let y = bounds.origin.y + (i as u32 / bounds.width) as i32;
                    (x, y, *a, *b)
                })
                .collect();
            panic!(
                "incremental rewind differs from the full re-render in {} of {} pixels \
                 (buildup {:?}, layer {bounds:?}); first few (x, y, incremental, oracle): {:?}",
                differing.len(),
                oracle.len() / 4,
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
    engine.begin_stroke(layer).unwrap();
    let mut t = 0.0;
    let mut go = |x: f32, y: f32| {
        engine.stroke_to(event(x, y, t));
        t += 16.0;
    };
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
    engine.end_stroke();
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
fn lay_stripes(engine: &mut DarklyEngine, layer: LayerId, canvas: (u32, u32), brush: &str) {
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
    set_input(engine, "brush_settings", "stabilize", 0.0);
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
    };
    cell.assert_matches_oracle(|engine, layer| {
        lay_stripes(engine, layer, canvas, "Smudge");
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
        };
        let bounds = cell.assert_matches_oracle(grow_reverse_jump);
        assert!(
            bounds.origin.x < 0 && bounds.origin.y < 0,
            "the stroke must have grown the layer past the plane origin: {bounds:?}"
        );
    }
}
