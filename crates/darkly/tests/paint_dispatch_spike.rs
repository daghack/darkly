//! Tests for the dispatch-per-dab spike terminal
//! (`crates/darkly/src/brush/nodes/paint_dispatch_spike.rs`), the stage 2
//! measurement vehicle of `docs/plans/compute-dispatch-per-dab-spike.md`.
//!
//! The bench compares the spike against `paint` on the replay matrix, so
//! the load-bearing property is that the two do equivalent work: the same
//! stroke through the Ink Pen and through the Ink Pen with its terminal
//! swapped must land the same pixels. The other two tests pin what the
//! bench relies on: that the spike survives the real stroke path
//! (stabiliser rewinds, checkpoints) deterministically, and that the
//! `dispatches` counter means one per dab for the spike and one per flush
//! for `paint`.
//!
//! Run with: `cargo test -p darkly --test paint_dispatch_spike --features testing -- --test-threads=1`

use std::path::PathBuf;

use darkly::brush::builtin_brushes;
use darkly::brush::input_value::InputValue;
use darkly::brush::portable::PortableBrush;
use darkly::engine::types::StrokeOp;
use darkly::engine::DarklyEngine;
use darkly::format::stroke_recording::{replay, ReplayPacing, StrokeRecording};
use darkly::gpu::context::GpuContext;
use darkly::gpu::test_utils::test_device;
use darkly::layer::LayerId;

const W: u32 = 256;
const H: u32 = 128;

const SPIKE_FIXTURE: &str = include_str!("fixtures/ink_pen_dispatch_spike.yaml");

fn test_engine(canvas: (u32, u32)) -> DarklyEngine {
    let (device, queue) = test_device();
    DarklyEngine::new(GpuContext::new_headless(device, queue), canvas.0, canvas.1)
}

fn fixture(name: &str) -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "tests", "fixtures", name]
        .iter()
        .collect()
}

fn install_spike(engine: &mut DarklyEngine) {
    let portable: PortableBrush = serde_yaml_ng::from_str(SPIKE_FIXTURE).expect("fixture parses");
    let brush = portable
        .into_brush(darkly::brush::registry(), "spike")
        .expect("fixture builds");
    let json = serde_json::to_string(&brush.metadata.graph).expect("serialize brush graph");
    engine
        .set_brush_graph(&json)
        .unwrap_or_else(|e| panic!("spike fixture compiles: {e:?}"));
}

fn install_ink_pen(engine: &mut DarklyEngine) {
    let brush = builtin_brushes::all()
        .into_iter()
        .find(|b| b.metadata.name == "Ink Pen")
        .expect("Ink Pen is a builtin");
    let json = serde_json::to_string(&brush.metadata.graph).expect("serialize brush graph");
    engine
        .set_brush_graph(&json)
        .unwrap_or_else(|e| panic!("Ink Pen compiles: {e:?}"));
}

fn find_node_id(engine: &DarklyEngine, type_id: &str) -> String {
    engine
        .active_brush_graph()
        .nodes()
        .values()
        .find(|n| n.type_id == type_id)
        .unwrap_or_else(|| panic!("no '{type_id}' node in active graph"))
        .id
        .0
        .clone()
}

fn set_stabilize(engine: &mut DarklyEngine, value: f32) {
    let bs = find_node_id(engine, "brush_settings");
    engine
        .brush_graph_set_input(&bs, "stabilize", InputValue::Scalar(value))
        .expect("brush_settings stabilize port");
}

/// A stroke that crosses the canvas with a reversal in the middle, so the
/// stabiliser's rewind path runs, at a pressure that gives the Ink Pen's
/// pressure-driven size and flow something to do.
fn stroke(engine: &mut DarklyEngine, layer: LayerId) {
    const SAMPLES: u32 = 48;
    engine.begin_stroke(layer).unwrap();
    let mut t = 0.0f64;
    for i in 0..SAMPLES {
        let u = i as f32 / (SAMPLES - 1) as f32;
        // Out and partly back, with a gentle vertical wave.
        let x = if u < 0.7 {
            12.0 + u / 0.7 * (W as f32 - 24.0)
        } else {
            (W as f32 - 12.0) - (u - 0.7) / 0.3 * 60.0
        };
        let y = (H / 2) as f32 + 18.0 * (u * 9.0).sin();
        engine.stroke_to(StrokeOp::BrushStroke {
            x,
            y,
            pressure: 0.6,
            x_tilt: 0.0,
            y_tilt: 0.0,
            rotation: 0.0,
            tangential_pressure: 0.0,
            time_ms: t,
            cr: 0.1,
            cg: 0.4,
            cb: 0.9,
            ca: 1.0,
        });
        t += 16.0;
    }
    engine.end_stroke();
    engine.test_flush_readbacks();
}

/// The spike lands the same stroke `paint` lands, within the rounding two
/// 8-bit paths accumulate: the spike packs and unpacks per dab, `paint`
/// rounds in the blend unit. Compared in premultiplied space, because the
/// layer readback is straight alpha and the commit's un-premultiply turns
/// a 1 LSB difference under an alpha of 1/255 into a full-range colour
/// difference that says nothing about the deposit. Per channel within 4
/// LSB everywhere, and no more than 5% of painted pixels further than
/// 1 LSB apart. Measured when written: of 9559 painted pixels, 1002 exact,
/// 8291 at 1 LSB (the pack's rounding against the blend unit's), 264 at
/// 2, 2 at 3, none above.
#[test]
fn spike_matches_paint_within_tolerance() {
    let mut a = test_engine((W, H));
    install_ink_pen(&mut a);
    let layer_a = a.add_raster_layer(None);
    stroke(&mut a, layer_a);
    let paint_px = a.test_readback_layer(layer_a);

    let mut b = test_engine((W, H));
    install_spike(&mut b);
    let layer_b = b.add_raster_layer(None);
    stroke(&mut b, layer_b);
    let spike_px = b.test_readback_layer(layer_b);

    assert_eq!(paint_px.len(), spike_px.len());
    let (paint_px, _) = paint_px.as_chunks::<4>();
    let (spike_px, _) = spike_px.as_chunks::<4>();
    let mut painted = 0u32;
    let mut max_diff = 0u8;
    let mut off_by_more_than_one = 0u32;
    for (p, s) in paint_px.iter().zip(spike_px) {
        if p[3] == 0 && s[3] == 0 {
            continue;
        }
        painted += 1;
        let premul = |px: &[u8; 4]| {
            let a = px[3] as u32;
            [
                ((px[0] as u32 * a + 127) / 255) as u8,
                ((px[1] as u32 * a + 127) / 255) as u8,
                ((px[2] as u32 * a + 127) / 255) as u8,
                px[3],
            ]
        };
        let diff = premul(p)
            .iter()
            .zip(premul(s))
            .map(|(x, y)| x.abs_diff(y))
            .max()
            .unwrap_or(0);
        max_diff = max_diff.max(diff);
        if diff > 1 {
            off_by_more_than_one += 1;
        }
    }
    assert!(painted > 500, "the stroke painted only {painted} pixels");
    assert!(
        max_diff <= 4,
        "spike and paint differ by up to {max_diff} LSB on painted pixels"
    );
    let share = off_by_more_than_one as f64 / painted as f64;
    assert!(
        share <= 0.05,
        "{off_by_more_than_one} of {painted} painted pixels ({:.1}%) differ by more than 1 LSB",
        share * 100.0
    );
}

/// The real stroke path: the recorded curvy stroke at full stabilisation,
/// so every event rewinds and restores through the checkpoint ring, whose
/// slots now carry the `r32uint` ground beside the scratch. Two fresh
/// engines must agree byte for byte, and the layer must hold paint.
#[test]
fn spike_replay_is_deterministic_and_paints() {
    let recording =
        StrokeRecording::load(&fixture("recorded_curvy_stroke.json")).expect("fixture parses");
    // Under the headless device's downlevel limits, as in
    // `tests/stroke_replay.rs`.
    let canvas = (1024, 512);

    let run = || {
        let mut engine = test_engine(canvas);
        install_spike(&mut engine);
        set_stabilize(&mut engine, 1.0);
        let layer = engine.add_raster_layer(None);
        let timings = replay(
            &mut engine,
            &recording,
            layer,
            canvas,
            ReplayPacing::AsFastAsPossible,
        );
        assert_eq!(timings.len(), recording.events.len());
        engine.test_flush_readbacks();
        engine.test_readback_layer(layer)
    };
    let first = run();
    let second = run();
    assert!(
        first.as_chunks::<4>().0.iter().any(|px| px[3] > 0),
        "the replay left the layer empty"
    );
    assert!(first == second, "two replays of the same recording differ");
}

/// `dispatches` counts one per dab for the spike and one per flush for the
/// instanced `paint` terminal, which is what the bench's `dispatches/ev`
/// column reads.
#[test]
fn dispatch_counter_is_per_dab_for_the_spike_and_per_flush_for_paint() {
    let mut a = test_engine((W, H));
    install_ink_pen(&mut a);
    let layer_a = a.add_raster_layer(None);
    let _ = a.drain_brush_perf_delta();
    stroke(&mut a, layer_a);
    let paint = a.drain_brush_perf_delta();
    assert!(paint.dab_flushes > 0, "paint issued no flushes");
    assert_eq!(
        paint.dispatches, paint.dab_flushes,
        "paint should issue one instanced draw per flush"
    );

    let mut b = test_engine((W, H));
    install_spike(&mut b);
    let layer_b = b.add_raster_layer(None);
    let _ = b.drain_brush_perf_delta();
    stroke(&mut b, layer_b);
    let spike = b.drain_brush_perf_delta();
    assert!(spike.flushed_dabs > 0, "the spike flushed no dabs");
    assert_eq!(
        spike.dispatches as u64, spike.flushed_dabs,
        "the spike should issue one dispatch per dab"
    );
}
