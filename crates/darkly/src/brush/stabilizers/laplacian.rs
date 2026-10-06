//! Laplacian relaxation stabilizer: iterative smoothing with zero lag.
//!
//! Maintains a polyline of all input positions.  On each new input:
//! 1. Append to polyline
//! 2. Run N sweeps of Laplacian smoothing on interior points (first + last
//!    pinned), each moving a point onto the chord between its neighbours
//! 3. Sensor values (pressure, tilt, etc.) smoothed the same way
//!
//! The result is defined as relaxing the whole polyline from its raw points
//! on every push, but only the last `N + 2` vertices can differ from the
//! previous push (see [`LaplacianStabilizer::max_divergence_window`]), so
//! that is all a push computes. In a forward Gauss-Seidel sweep, vertex `i`
//! after sweep `k` depends only on raw points `0..=i + k`, so the vertex just
//! left of the window takes, sweep for sweep, exactly the values it took when
//! it was the left edge of an earlier push's window. Those per-sweep values
//! are recorded then and replayed now as the window's left neighbour, which
//! makes the windowed result bit-identical to the from-scratch one at a cost
//! of `N x (N + 1)` vertex updates per push, whatever the stroke's length.
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
use crate::brush::stabilizer::{StabilizerAlgorithm, StabilizerRegistration};
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

/// A vertex's value after each sweep of the push that last relaxed it as the
/// left edge of its window.
struct EdgeTrajectory {
    index: usize,
    sweeps: Vec<PaintInformation>,
}

pub struct LaplacianStabilizer {
    raw_points: Vec<PaintInformation>,
    stabilized: Vec<PaintInformation>,
    /// Per interior point, its arc-length proportion between its raw
    /// neighbours: where on their chord relaxation places it.
    chord_t: Vec<f32>,
    /// Per interior point, how far toward that chord each sweep moves it:
    /// the pen's speed past it over the reference speed, at most 1.
    pull: Vec<f32>,
    /// Per-sweep values of the left edges of the two most recent relaxation
    /// windows: the boundary the next window, or a window that replaces the
    /// tip, reads in place of the vertex it no longer recomputes.
    edges: Vec<EdgeTrajectory>,
    sweeps: u32,
    /// Reference speed in canvas px/s at this stroke's view.
    reference_speed: f32,
    /// Vertex updates performed by relaxation, for tests that bound it.
    #[cfg(test)]
    relaxed_vertex_updates: u64,
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
            edges: Vec::with_capacity(2),
            sweeps: (strength * strength * MAX_SWEEPS).ceil() as u32,
            reference_speed: REFERENCE_SPEED_CSS_PX_PER_S * canvas_per_css_px,
            #[cfg(test)]
            relaxed_vertex_updates: 0,
        }
    }

    /// Each interior point's arc-length proportion between its raw
    /// neighbours (0.5 where a neighbour coincides with it) and its pull,
    /// from the pen's speed across those neighbours. Without timing (equal
    /// timestamps) the pull is full.
    ///
    /// An interior point's weights read only its raw neighbours, so after a
    /// push only the point that became interior, or whose right neighbour
    /// replaced a retracted tip, needs computing.
    fn update_weights(&mut self) {
        let raw = &self.raw_points;
        let settled = raw.len().saturating_sub(2).max(1);
        self.chord_t.truncate(settled);
        self.pull.truncate(settled);
        if self.chord_t.is_empty() {
            // Index 0 is pinned and carries no weight.
            self.chord_t.push(0.5);
            self.pull.push(1.0);
        }
        for i in self.chord_t.len()..raw.len().saturating_sub(1) {
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

    /// Relax interior vertices `from..len - 1` over every sweep, with the
    /// first and last points pinned. `boundary`, when given, is vertex
    /// `from - 1`'s value after each sweep, read in its place; without it
    /// `from` is 1 and the pinned origin is the boundary. Records vertex
    /// `edge`'s per-sweep values for the windows that follow.
    fn relax(&mut self, from: usize, edge: usize, boundary: Option<EdgeTrajectory>) {
        let len = self.stabilized.len();
        // Reuse a stale trajectory's allocation for the one recorded now.
        let mut trajectory = self.edges.pop().map(|e| e.sweeps).unwrap_or_default();
        self.edges.clear();
        trajectory.clear();
        if len < 3 {
            return;
        }

        for k in 0..self.sweeps as usize {
            for i in from..len - 1 {
                let prev = match &boundary {
                    Some(b) if i == from => b.sweeps[k],
                    _ => self.stabilized[i - 1],
                };
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
                #[cfg(test)]
                {
                    self.relaxed_vertex_updates += 1;
                }
                if i == edge {
                    trajectory.push(self.stabilized[i]);
                }
            }
        }

        self.edges.extend(boundary);
        self.edges.push(EdgeTrajectory {
            index: edge,
            sweeps: trajectory,
        });
    }
}

impl StabilizerAlgorithm for LaplacianStabilizer {
    fn push(&mut self, point: PaintInformation) {
        self.raw_points.push(point);
        self.update_weights();

        // The window: the vertices this push can move. Its left neighbour's
        // per-sweep values were recorded when that vertex was an earlier
        // window's left edge; without them (stroke start) the window reaches
        // back to the first interior vertex, which relaxes from scratch.
        let len = self.raw_points.len();
        let edge = len.saturating_sub(self.sweeps as usize + 2).max(1);
        let boundary = self
            .edges
            .iter()
            .position(|e| edge >= 2 && e.index == edge - 1)
            .map(|p| self.edges.swap_remove(p));
        let from = if boundary.is_some() { edge } else { 1 };

        // Vertices before the window keep their settled values; the window
        // restarts from raw, as the from-scratch relaxation does.
        let keep = from.min(self.stabilized.len());
        self.stabilized.truncate(keep);
        self.stabilized.extend_from_slice(&self.raw_points[keep..]);
        self.relax(from, edge, boundary);
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
    /// Each `push()` yields the result of `N` Gauss-Seidel Laplacian sweeps
    /// from scratch on the raw polyline. Between frames, the only
    /// inputs that differ are the new tip and the now-interior previous tip
    /// (formerly pinned). Both perturbations sit at indices `>= len - 2`.
    ///
    /// Per sweep, the *backward* influence radius of a Gauss-Seidel Laplacian
    /// pass is exactly 1: forward in-sweep updates do not move information
    /// backward, so a change at index `i` can only affect index `i-1` in the
    /// *next* sweep. After `N` sweeps the backward reach is `N`. The earliest
    /// index that can possibly differ between frames is `len - 2 - N`, so the
    /// distance from the tip (`len - 1`) is at most `N + 1`. (The bound is
    /// conservative by one: the previous tip's first sweep reads only
    /// unchanged inputs, so `len - 1 - N` is the tight index.) `push` relaxes
    /// exactly that range, so nothing outside it can move.
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
        self.edges.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brush::stroke_engine::{DivergenceDiff, DIVERGENCE_EPSILON};

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
            stab.push(*pt);
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

        let mut diff = DivergenceDiff::new(DIVERGENCE_EPSILON);
        let window = stab.max_divergence_window();

        // Build a straight stroke much longer than the smoothing reach.
        for i in 0..200 {
            stab.push(make_point(i as f32 * 10.0, 0.0));
            diff.update(stab.stabilized(), window);
        }

        // Add a sharp turn, which should cause divergence near the end, not at the beginning.
        stab.push(make_point(1990.0, 30.0));
        if let Some(div) = diff.update(stab.stabilized(), window) {
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

    /// Regression for the prior "find_divergence can return `Some(0)`" bug,
    /// stated as the bound itself: no push moves a vertex further behind the
    /// previous tip than `max_divergence_window`, so a diff that walks only
    /// that window misses nothing.
    #[test]
    fn pushes_move_nothing_behind_the_window() {
        let mut stab = LaplacianStabilizer::new(1.0, 1.0);
        let window = stab.max_divergence_window();
        let mut previous: Vec<PaintInformation> = Vec::new();

        // A long stroke with curvature, so every push reshapes the vertices
        // near the tip.
        for i in 0..400 {
            let t = i as f32;
            stab.push(make_point(t * 4.0, (t * 0.15).sin() * 30.0));
            let settled = previous.len().saturating_sub(window + 1);
            assert_eq!(
                &stab.stabilized()[..settled],
                &previous[..settled],
                "push {i} moved a vertex more than {window} behind the previous tip"
            );
            previous = stab.stabilized().to_vec();
        }
    }

    /// Regression: a push relaxes only the vertices its influence bound says
    /// can move, so its cost does not grow with the stroke. Relaxing the
    /// whole polyline on every push made a long stroke at high strength lag
    /// further the longer it got.
    #[test]
    fn per_push_relaxation_cost_is_independent_of_stroke_length() {
        let mut stab = LaplacianStabilizer::new(0.5, 1.0);
        let sweeps = stab.sweeps as u64;
        let mut costs = Vec::new();
        for i in 0..400 {
            let before = stab.relaxed_vertex_updates;
            stab.push(make_point(i as f32 * 6.0, (i as f32 * 0.05).sin() * 40.0));
            if i == 99 || i == 399 {
                costs.push(stab.relaxed_vertex_updates - before);
            }
        }
        assert_eq!(
            costs[0], costs[1],
            "push 100 cost {} vertex updates, push 400 cost {}",
            costs[0], costs[1]
        );
        assert!(
            costs[1] <= sweeps * (sweeps + 1),
            "a push relaxed {} vertex updates, more than the {} its window holds",
            costs[1],
            sweeps * (sweeps + 1)
        );
    }

    /// Test oracle: the from-scratch relaxation, kept as a deliberate copy of
    /// the production update rule. Every interior point is reset to its raw
    /// value and relaxed over all `sweeps`, with the weights recomputed over
    /// the whole polyline, which is the definition the windowed relaxation
    /// must reproduce bit for bit.
    fn relax_from_scratch(
        raw: &[PaintInformation],
        sweeps: u32,
        reference_speed: f32,
    ) -> Vec<PaintInformation> {
        let mut out = raw.to_vec();
        let len = out.len();
        if len < 3 {
            return out;
        }
        let mut chord_t = vec![0.5f32; len];
        let mut pull = vec![1.0f32; len];
        for i in 1..len - 1 {
            let before =
                (raw[i].pos[0] - raw[i - 1].pos[0]).hypot(raw[i].pos[1] - raw[i - 1].pos[1]);
            let after =
                (raw[i + 1].pos[0] - raw[i].pos[0]).hypot(raw[i + 1].pos[1] - raw[i].pos[1]);
            let total = before + after;
            chord_t[i] = if total > 0.0 { before / total } else { 0.5 };
            let dt = raw[i + 1].time - raw[i - 1].time;
            pull[i] = if dt > 0.0 {
                (total / dt / reference_speed).min(1.0)
            } else {
                1.0
            };
        }
        for _ in 0..sweeps {
            for i in 1..len - 1 {
                let prev = out[i - 1];
                let next = out[i + 1];
                let (t, p) = (chord_t[i], pull[i]);
                let cur = &mut out[i];
                macro_rules! smooth_field {
                    ($field:ident) => {
                        let target = prev.$field + (next.$field - prev.$field) * t;
                        cur.$field += (target - cur.$field) * p;
                    };
                }
                let target = [
                    prev.pos[0] + (next.pos[0] - prev.pos[0]) * t,
                    prev.pos[1] + (next.pos[1] - prev.pos[1]) * t,
                ];
                cur.pos[0] += (target[0] - cur.pos[0]) * p;
                cur.pos[1] += (target[1] - cur.pos[1]) * p;
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
        out
    }

    /// The relaxed polyline is bit-identical to relaxing the whole stroke
    /// from its raw points after every push, including pushes that replace a
    /// provisional tip, with the pen's speed varying so the pull is partial
    /// in places and every relaxed sensor carrying its own signal. Every push
    /// stays within its window's cost, so the exactness is the windowed
    /// relaxation's and not a from-scratch fallback's.
    #[test]
    fn relaxation_is_bit_exact_with_from_scratch() {
        for (strength, vertices) in [(1.0f32, 340usize), (0.3, 200), (0.1, 60)] {
            let mut stab = LaplacianStabilizer::new(strength, 1.0);
            let window_cost = stab.sweeps as u64 * (stab.sweeps as u64 + 1);
            let mut raw: Vec<PaintInformation> = Vec::new();
            let mut t = 0.0f32;
            let sample = |i: usize, wobble: f32, t: f32| {
                let s = i as f32 + wobble;
                PaintInformation {
                    pos: [s * 6.0, (s * 0.07).sin() * 50.0 + (s * 0.31).cos() * 4.0],
                    pressure: 0.5 + 0.4 * (s * 0.11).sin(),
                    x_tilt: (s * 0.05).cos(),
                    y_tilt: (s * 0.09).sin(),
                    rotation: s * 0.01,
                    tangential_pressure: (s * 0.2).sin(),
                    time: t,
                    speed: (s * 0.13).cos(),
                    tilt_magnitude: 0.3 + 0.2 * (s * 0.17).sin(),
                    tilt_direction: (s * 0.03).sin(),
                    ..Default::default()
                }
            };
            for i in 0..vertices {
                // Slow near multiples of 40 vertices, fast elsewhere.
                let speed = if i % 40 < 6 { 60.0 } else { 900.0 };
                t += 6.0 / speed;
                // A provisional tip, replaced in place, then the committed
                // vertex, as the resampler feeds the algorithm.
                if i % 3 == 0 && i > 0 {
                    let tip = sample(i, -0.4, t - 2.0 / speed);
                    let before = stab.relaxed_vertex_updates;
                    stab.push(tip);
                    assert!(
                        stab.relaxed_vertex_updates - before <= window_cost,
                        "strength {strength}: provisional tip at {i} relaxed past its window"
                    );
                    raw.push(tip);
                    assert_eq!(
                        stab.stabilized(),
                        relax_from_scratch(&raw, stab.sweeps, stab.reference_speed).as_slice(),
                        "strength {strength}: provisional tip at {i}"
                    );
                    stab.retract_tip();
                    raw.pop();
                }
                let p = sample(i, 0.0, t);
                let before = stab.relaxed_vertex_updates;
                stab.push(p);
                assert!(
                    stab.relaxed_vertex_updates - before <= window_cost,
                    "strength {strength}: vertex {i} relaxed past its window"
                );
                raw.push(p);
                assert_eq!(
                    stab.stabilized(),
                    relax_from_scratch(&raw, stab.sweeps, stab.reference_speed).as_slice(),
                    "strength {strength}: vertex {i}"
                );
            }
        }
    }
}
