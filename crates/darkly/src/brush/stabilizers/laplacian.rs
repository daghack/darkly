//! Laplacian relaxation stabilizer: iterative smoothing with zero lag.
//!
//! Maintains a polyline of all input positions.  On each new input:
//! 1. Append to polyline
//! 2. Run N sweeps of Laplacian smoothing on interior points (first + last
//!    pinned), each moving a point onto the chord between its neighbours
//! 3. Sensor values (pressure, tilt, etc.) smoothed the same way
//! 4. Diff against the positions last rendered → find divergence point
//!
//! A point lands on the chord at its own arc-length proportion between its
//! neighbours (the midpoint when they are equally spaced), measured on the
//! raw polyline. Smoothing therefore changes shape only and never slides
//! points along the stroke, so an unevenly spaced segment, such as the
//! short one to a provisional tip, does not pull its neighbours after it.
//!
//! Each point is pulled onto that chord in proportion to the pen's speed as
//! it passed, up to [`REFERENCE_SPEED_CSS_PX_PER_S`]. Fast motion is smoothed
//! in full; a slow pivot keeps its corner; a stopped pen is exact.
//!
//! The tip is always pinned at the cursor (zero lag).  The stroke behind
//! the pen continuously reshapes as direction changes: the "taffy" feel.
//!
//! Repeated neighbour averaging is a diffusion, so the smoothing radius
//! grows with the square root of the sweep count. The sweep count is
//! therefore quadratic in strength, which makes the visible smoothing grow
//! about linearly with the slider.

use crate::brush::paint_info::PaintInformation;
use crate::brush::stabilizer::{
    DivergenceDiff, StabilizeResult, StabilizerAlgorithm, StabilizerRegistration,
    DIVERGENCE_EPSILON,
};
use crate::gpu::params::{ParamDef, ParamValue};

const PARAMS: &[ParamDef] = &[ParamDef::float("strength", 0.0, 1.0, 0.5)
    .with_label("Strength")
    .with_description(
        "How firmly the stroke is smoothed as you draw; higher lags further behind the cursor.",
    )];

pub fn register() -> StabilizerRegistration {
    StabilizerRegistration {
        type_id: "laplacian",
        display_name: "Laplacian Relaxation",
        params: PARAMS,
        from_params: |params, canvas_per_css_px| {
            let strength = match params.first() {
                Some(ParamValue::Float(v)) => *v,
                _ => 0.5,
            };
            Box::new(LaplacianStabilizer::new(strength, canvas_per_css_px))
        },
    }
}

/// Sweeps at strength 1. With vertices one resample spacing apart this
/// smooths over about ten spacings.
const MAX_SWEEPS: f32 = 160.0;

/// Pen speed at and above which a point is pulled fully onto the chord
/// between its neighbours each sweep. Below it the pull falls off linearly
/// with speed.
pub const REFERENCE_SPEED_CSS_PX_PER_S: f32 = 500.0;

pub struct LaplacianStabilizer {
    raw_points: Vec<PaintInformation>,
    stabilized: Vec<PaintInformation>,
    /// Per interior point, its arc-length proportion between its raw
    /// neighbours: where on their chord relaxation places it.
    chord_t: Vec<f32>,
    /// Per interior point, how far toward that chord each sweep moves it:
    /// the pen's speed past it over the reference speed, at most 1.
    pull: Vec<f32>,
    diff: DivergenceDiff,
    sweeps: u32,
    /// Reference speed in canvas px/s at this stroke's view.
    reference_speed: f32,
}

impl LaplacianStabilizer {
    /// `canvas_per_css_px` is the stroke's view scale, which puts the
    /// reference speed into canvas units.
    pub fn new(strength: f32, canvas_per_css_px: f32) -> Self {
        let strength = strength.clamp(0.0, 1.0);
        Self {
            raw_points: Vec::with_capacity(256),
            stabilized: Vec::with_capacity(256),
            chord_t: Vec::with_capacity(256),
            pull: Vec::with_capacity(256),
            diff: DivergenceDiff::new(DIVERGENCE_EPSILON),
            sweeps: (strength * strength * MAX_SWEEPS).ceil() as u32,
            reference_speed: REFERENCE_SPEED_CSS_PX_PER_S * canvas_per_css_px,
        }
    }

    /// Each interior point's arc-length proportion between its raw
    /// neighbours (0.5 where a neighbour coincides with it) and its pull,
    /// from the pen's speed across those neighbours. Without timing (equal
    /// timestamps) the pull is full.
    fn compute_weights(&mut self) {
        self.chord_t.clear();
        self.pull.clear();
        let raw = &self.raw_points;
        self.chord_t.push(0.5);
        self.pull.push(1.0);
        for i in 1..raw.len().saturating_sub(1) {
            let before =
                (raw[i].pos[0] - raw[i - 1].pos[0]).hypot(raw[i].pos[1] - raw[i - 1].pos[1]);
            let after =
                (raw[i + 1].pos[0] - raw[i].pos[0]).hypot(raw[i + 1].pos[1] - raw[i].pos[1]);
            let total = before + after;
            self.chord_t
                .push(if total > 0.0 { before / total } else { 0.5 });
            let dt = raw[i + 1].time - raw[i - 1].time;
            self.pull.push(if dt > 0.0 {
                (total / dt / self.reference_speed).min(1.0)
            } else {
                1.0
            });
        }
    }

    /// Run Laplacian relaxation on the stabilized polyline.
    /// First and last points are pinned (never move).
    fn relax(&mut self) {
        let len = self.stabilized.len();
        if len < 3 {
            return;
        }

        for _ in 0..self.sweeps {
            for i in 1..len - 1 {
                let prev = self.stabilized[i - 1];
                let next = self.stabilized[i + 1];
                let t = self.chord_t[i];
                let pull = self.pull[i];
                let cur = &mut self.stabilized[i];

                // Position and every continuous sensor move toward the chord
                // between their neighbours, at this point's own proportion,
                // by this point's pull.
                macro_rules! smooth_field {
                    ($field:ident) => {
                        let target = prev.$field + (next.$field - prev.$field) * t;
                        cur.$field += (target - cur.$field) * pull;
                    };
                }

                let target = [
                    prev.pos[0] + (next.pos[0] - prev.pos[0]) * t,
                    prev.pos[1] + (next.pos[1] - prev.pos[1]) * t,
                ];
                cur.pos[0] += (target[0] - cur.pos[0]) * pull;
                cur.pos[1] += (target[1] - cur.pos[1]) * pull;
                smooth_field!(pressure);
                smooth_field!(x_tilt);
                smooth_field!(y_tilt);
                smooth_field!(rotation);
                smooth_field!(tangential_pressure);
                smooth_field!(speed);
                smooth_field!(tilt_magnitude);
                smooth_field!(tilt_direction);
            }
        }
    }
}

impl StabilizerAlgorithm for LaplacianStabilizer {
    fn push(&mut self, point: PaintInformation) -> StabilizeResult {
        // Append raw point.
        self.raw_points.push(point);

        // Copy raw → stabilized (fresh copy each frame for correct relaxation).
        self.stabilized.clear();
        self.stabilized.extend_from_slice(&self.raw_points);

        // Run relaxation.
        self.compute_weights();
        self.relax();

        // Find divergence. The walk is bounded by `max_divergence_window`,
        // which shares the relaxation's influence model, so the bound is
        // enforced by construction rather than by clamping a wider scan.
        let divergence_index = if self.sweeps == 0 {
            None
        } else {
            let window = self.max_divergence_window();
            self.diff.update(&self.stabilized, window)
        };

        StabilizeResult { divergence_index }
    }

    fn retract_tip(&mut self) {
        self.raw_points.pop();
        self.stabilized.pop();
    }

    fn stabilized(&self) -> &[PaintInformation] {
        &self.stabilized
    }

    /// Conservative upper bound on `tip_vi - find_divergence().unwrap()`.
    ///
    /// Each `push()` re-runs `N` Gauss-Seidel Laplacian sweeps from scratch
    /// on the raw polyline. Between frames, the only
    /// inputs that differ are the new tip and the now-interior previous tip
    /// (formerly pinned). Both perturbations sit at indices `>= len - 2`.
    ///
    /// Per sweep, the *backward* influence radius of a Gauss-Seidel Laplacian
    /// pass is exactly 1: forward in-sweep updates do not move information
    /// backward, so a change at index `i` can only affect index `i-1` in the
    /// *next* sweep. After `N` sweeps the backward reach is `N`. The earliest
    /// index that can possibly differ between frames is `len - 2 - N`, so the
    /// distance from the tip (`len - 1`) is at most `N + 1`.
    fn max_divergence_window(&self) -> usize {
        if self.sweeps == 0 {
            return 0;
        }
        self.sweeps as usize + 1
    }

    fn clear(&mut self) {
        self.raw_points.clear();
        self.stabilized.clear();
        self.chord_t.clear();
        self.pull.clear();
        self.diff.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_point(x: f32, y: f32) -> PaintInformation {
        PaintInformation {
            pos: [x, y],
            pressure: 0.5,
            ..Default::default()
        }
    }

    fn make_point_with_pressure(x: f32, y: f32, pressure: f32) -> PaintInformation {
        PaintInformation {
            pos: [x, y],
            pressure,
            ..Default::default()
        }
    }

    #[test]
    fn straight_line_stays_straight() {
        let mut stab = LaplacianStabilizer::new(0.8, 1.0);
        for i in 0..10 {
            stab.push(make_point(i as f32 * 10.0, 0.0));
        }

        // All points should be on y=0 (straight line is already smooth).
        for pt in stab.stabilized() {
            assert!(pt.pos[1].abs() < 1e-3, "y={} should be ~0", pt.pos[1]);
        }
    }

    #[test]
    fn sharp_turn_is_smoothed() {
        let mut stab = LaplacianStabilizer::new(0.8, 1.0);

        // Straight right, then sharp turn down.
        for i in 0..5 {
            stab.push(make_point(i as f32 * 10.0, 0.0));
        }
        for i in 1..5 {
            stab.push(make_point(40.0, i as f32 * 10.0));
        }

        // The corner point (40, 0) should be pulled inward by smoothing.
        let corner = &stab.stabilized()[4];
        // It should have moved: either x decreased or y increased.
        let moved = corner.pos[0] < 40.0 - 0.1 || corner.pos[1] > 0.1;
        assert!(
            moved,
            "corner at {:?} should be smoothed away from (40, 0)",
            corner.pos
        );
    }

    #[test]
    fn strength_zero_is_pass_through() {
        let mut stab = LaplacianStabilizer::new(0.0, 1.0);
        let points: Vec<_> = (0..5)
            .map(|i| make_point(i as f32 * 10.0, (i as f32).sin() * 5.0))
            .collect();

        for pt in &points {
            let result = stab.push(*pt);
            assert!(result.divergence_index.is_none());
        }

        // Output should exactly match input.
        for (orig, stab_pt) in points.iter().zip(stab.stabilized()) {
            assert!((orig.pos[0] - stab_pt.pos[0]).abs() < 1e-6);
            assert!((orig.pos[1] - stab_pt.pos[1]).abs() < 1e-6);
        }
    }

    #[test]
    fn first_and_last_pinned() {
        let mut stab = LaplacianStabilizer::new(1.0, 1.0);

        // Zigzag pattern.
        stab.push(make_point(0.0, 0.0));
        stab.push(make_point(10.0, 20.0));
        stab.push(make_point(20.0, -20.0));
        stab.push(make_point(30.0, 20.0));
        stab.push(make_point(40.0, 0.0));

        let s = stab.stabilized();
        assert!(
            (s[0].pos[0] - 0.0).abs() < 1e-6,
            "first point must be pinned"
        );
        assert!(
            (s[0].pos[1] - 0.0).abs() < 1e-6,
            "first point must be pinned"
        );
        let last = s.last().unwrap();
        assert!(
            (last.pos[0] - 40.0).abs() < 1e-6,
            "last point must be pinned"
        );
        assert!(
            (last.pos[1] - 0.0).abs() < 1e-6,
            "last point must be pinned"
        );
    }

    #[test]
    fn divergence_detected_near_turn() {
        let mut stab = LaplacianStabilizer::new(0.5, 1.0);

        // Build a straight stroke much longer than the smoothing reach.
        for i in 0..200 {
            stab.push(make_point(i as f32 * 10.0, 0.0));
        }

        // Add a sharp turn, which should cause divergence near the end, not at the beginning.
        let result = stab.push(make_point(1990.0, 30.0));
        if let Some(div) = result.divergence_index {
            assert!(
                div > 100,
                "divergence at {div} should be near the turn, not at the start"
            );
        }
    }

    #[test]
    fn sensor_values_smoothed() {
        // Gentle enough that a five-point stroke does not fully converge to
        // a straight ramp between its pinned ends.
        let mut stab = LaplacianStabilizer::new(0.1, 1.0);

        // Pressure spike in the middle.
        stab.push(make_point_with_pressure(0.0, 0.0, 0.3));
        stab.push(make_point_with_pressure(10.0, 0.0, 0.3));
        stab.push(make_point_with_pressure(20.0, 0.0, 1.0)); // spike
        stab.push(make_point_with_pressure(30.0, 0.0, 0.3));
        stab.push(make_point_with_pressure(40.0, 0.0, 0.3));

        let s = stab.stabilized();
        // The spike should be smoothed down.
        assert!(
            s[2].pressure < 0.95,
            "pressure spike at {} should be smoothed",
            s[2].pressure
        );
        // Neighbors should be pulled up slightly.
        assert!(
            s[1].pressure > 0.3,
            "pressure {} should be pulled toward spike",
            s[1].pressure
        );
        assert!(
            s[3].pressure > 0.3,
            "pressure {} should be pulled toward spike",
            s[3].pressure
        );
    }

    #[test]
    fn higher_strength_smooths_more() {
        fn corner_displacement(strength: f32) -> f32 {
            let mut stab = LaplacianStabilizer::new(strength, 1.0);
            for i in 0..5 {
                stab.push(make_point(i as f32 * 10.0, 0.0));
            }
            for i in 1..5 {
                stab.push(make_point(40.0, i as f32 * 10.0));
            }
            let corner = &stab.stabilized()[4];
            let dx = corner.pos[0] - 40.0;
            let dy = corner.pos[1] - 0.0;
            (dx * dx + dy * dy).sqrt()
        }

        let low = corner_displacement(0.2);
        let high = corner_displacement(0.8);
        assert!(
            high > low,
            "higher strength ({high}) should displace corner more than lower ({low})"
        );
    }

    /// Regression: relaxation changes shape only. A straight line with one
    /// short segment at the end (a provisional tip just past the last
    /// committed vertex) must not slide its points along the line, which a
    /// midpoint rule does, and which made the stroke lurch at every commit.
    #[test]
    fn uneven_spacing_on_a_line_does_not_slide_points() {
        let mut stab = LaplacianStabilizer::new(1.0, 1.0);
        for i in 0..20 {
            stab.push(make_point(i as f32 * 6.0, 0.0));
        }
        stab.push(make_point(19.0 * 6.0 + 0.4, 0.0));
        for (i, p) in stab.stabilized().iter().enumerate().take(20) {
            let expected = i as f32 * 6.0;
            assert!(
                (p.pos[0] - expected).abs() < 1e-3,
                "vertex {i} slid to {} (expected {expected})",
                p.pos[0]
            );
        }
    }

    /// Regression: smoothing follows the pen's speed. A corner the pen slowed
    /// into keeps its shape, while the same corner taken at speed is cut.
    #[test]
    fn slow_corner_keeps_its_shape() {
        fn corner_cut(slow_near_corner: bool) -> f32 {
            let mut stab = LaplacianStabilizer::new(1.0, 1.0);
            let mut t = 0.0f32;
            let pts: Vec<[f32; 2]> = (0..=20)
                .map(|i| [i as f32 * 6.0, 0.0])
                .chain((1..=20).map(|i| [120.0, i as f32 * 6.0]))
                .collect();
            for (i, p) in pts.iter().enumerate() {
                let near_corner = (i as i32 - 20).abs() <= 3;
                let speed = if slow_near_corner && near_corner {
                    40.0
                } else {
                    1000.0
                };
                t += 6.0 / speed;
                stab.push(PaintInformation {
                    pos: *p,
                    time: t,
                    ..Default::default()
                });
            }
            stab.stabilized()
                .iter()
                .map(|p| (p.pos[0] - 120.0).hypot(p.pos[1]))
                .fold(f32::INFINITY, f32::min)
        }
        let fast = corner_cut(false);
        let slow = corner_cut(true);
        assert!(
            fast > 20.0,
            "a corner taken at speed is smoothed: cut {fast}"
        );
        assert!(
            slow < fast / 4.0,
            "a corner the pen slowed into keeps its shape: cut {slow} vs {fast} at speed"
        );
    }

    /// `max_divergence_window` is the contract the checkpoint ring relies on
    /// for coverage. Lock the value to the influence-radius derivation so any
    /// future change is forced through this test (and the documentation).
    #[test]
    fn max_divergence_window_matches_influence_radius() {
        // strength = 0 → pass-through, no relaxation, no divergence window.
        assert_eq!(
            LaplacianStabilizer::new(0.0, 1.0).max_divergence_window(),
            0
        );

        // sweeps = ceil(strength^2 * 160); window = sweeps + 1.
        for (strength, expected_iters) in [
            (0.1f32, 2u32),
            (0.25, 10),
            (0.5, 40),
            (0.8, 103),
            (1.0, 160),
        ] {
            let stab = LaplacianStabilizer::new(strength, 1.0);
            assert_eq!(
                stab.max_divergence_window(),
                expected_iters as usize + 1,
                "strength={strength}: window should be iterations+1"
            );
        }
    }

    /// Regression for the prior "find_divergence can return `Some(0)`" bug.
    /// The detector and the bound are co-derived now; this test enforces that
    /// `find_divergence` never returns an index further from the tip than
    /// `max_divergence_window` indices behind it.
    #[test]
    fn find_divergence_respects_max_window() {
        let mut stab = LaplacianStabilizer::new(1.0, 1.0);
        let max_back = stab.max_divergence_window();

        // Build a long stroke with curvature so divergence is detected on
        // most pushes (every interior point shifts slightly when a new tip
        // is pinned somewhere else).
        for i in 0..200 {
            let t = i as f32;
            let x = t * 4.0;
            let y = (t * 0.15).sin() * 30.0;
            let result = stab.push(make_point(x, y));

            let len = stab.stabilized().len();
            let earliest = len.saturating_sub(max_back + 1);
            if let Some(div) = result.divergence_index {
                assert!(
                    div >= earliest,
                    "find_divergence returned {div} at len={len}, earliest allowed = {earliest} \
                     (max_divergence_window = {max_back})"
                );
            }
        }
    }
}
