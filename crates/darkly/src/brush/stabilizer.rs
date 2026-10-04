//! Stroke stabilizer: retroactive stroke reshaping with zero lag.
//!
//! The stabilizer processes the full stroke history before dabs are placed.
//! It operates outside the per-dab node graph: brushes configure which
//! algorithm to use and its parameters, and the engine constructs the
//! algorithm at stroke start.
//!
//! Follows the same modular registry pattern as veils (`gpu/veil.rs` +
//! `gpu/effects/*.rs`): each algorithm is a self-contained module that
//! declares its own params and factory.  A registry maps type_id →
//! registration.  New algorithms are added by dropping a `.rs` file in
//! `brush/stabilizers/`: no other files touched.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::paint_info::PaintInformation;
use super::resampler::{ResamplingStabilizer, RESAMPLE_SPACING_CSS_PX};
use crate::gpu::params::{ParamDef, ParamValue};

/// Result of pushing a new point through the stabilizer.
pub struct StabilizeResult {
    /// Earliest dab index that needs re-rendering (everything from here
    /// to the tip has changed).  `None` means nothing diverged: only
    /// new points were appended.
    pub divergence_index: Option<usize>,
}

/// Distance in CSS pixels below which a stabilized point is considered
/// unchanged since it was rendered. [`stroke_stabilizer_stack`] scales it to
/// canvas pixels at the stroke's zoom; a bare algorithm uses it unscaled.
pub const DIVERGENCE_EPSILON: f32 = 0.5;

/// Find the earliest rendered index whose position has moved, walking
/// backward from the tip until either an index lies within `epsilon` of its
/// rendered position or the influence bound `max_window` is hit.
///
/// `current` is this push's polyline, `rendered` the positions its vertices
/// were last rendered at. The walk is bounded to `max_window` indices behind
/// the tip (the caller's model of how far a perturbation can reach), so it
/// never reports divergence at indices the model says cannot have moved.
///
/// Returns `None` when every rendered index is unchanged, even if the
/// polyline grew: new indices have never been rendered, so appending them
/// needs no rewind. Callers go through [`DivergenceDiff`].
fn find_divergence(
    current: &[PaintInformation],
    rendered: &[[f32; 2]],
    max_window: usize,
    epsilon: f32,
) -> Option<usize> {
    walk_divergence(current, rendered, max_window, epsilon).filter(|&k| k < rendered.len())
}

/// The backward walk behind [`find_divergence`]: the index just above the
/// first unchanged index found walking down from the tip, which may name a
/// newly appended index.
fn walk_divergence(
    current: &[PaintInformation],
    rendered: &[[f32; 2]],
    max_window: usize,
    epsilon: f32,
) -> Option<usize> {
    let len = current.len();
    if len == 0 {
        return None;
    }

    // `earliest` is the lowest index whose position could possibly differ
    // since it was rendered given the influence bound. Walking past it is
    // wasted work and risks reporting spurious divergence.
    let earliest = len.saturating_sub(max_window + 1);
    let eps2 = epsilon * epsilon;

    // Squared position delta at index `i` (within bounds of both arrays here).
    let delta2 = |i: usize| -> f32 {
        let cur = current[i].pos;
        let prev = rendered[i];
        let dx = cur[0] - prev[0];
        let dy = cur[1] - prev[1];
        dx * dx + dy * dy
    };

    if rendered.len() == len {
        // Same length: walk backward from tip to `earliest`.
        for i in (earliest..len).rev() {
            if delta2(i) < eps2 {
                return if i + 1 < len { Some(i + 1) } else { None };
            }
        }
        Some(earliest)
    } else {
        // Polyline grew. New indices `[rendered.len(), len-1]` are by
        // definition new and cannot be compared. Existing indices that
        // overlap with `rendered` are in `[0, overlap_end)`: walk
        // those, descending, bounded below by `earliest`.
        let overlap_end = rendered.len().min(len);
        if earliest >= overlap_end {
            // No overlap to check (e.g., first push of a stroke). The
            // divergence index is `earliest` itself, which equals 0 when
            // there is nothing prior to compare against.
            return Some(earliest);
        }
        for i in (earliest..overlap_end).rev() {
            if delta2(i) < eps2 {
                return Some(i + 1);
            }
        }
        Some(earliest)
    }
}

/// The positions each vertex of an output polyline was last rendered at, and
/// the diff of a new polyline against them.
///
/// Comparing against rendered positions rather than the previous push keeps
/// every rendered vertex within `epsilon` of where it now lies: a vertex that
/// drifts a little on every push is re-rendered once its accumulated drift
/// reaches `epsilon`, instead of never.
pub struct DivergenceDiff {
    rendered: Vec<[f32; 2]>,
    epsilon: f32,
}

impl DivergenceDiff {
    /// A diff that treats moves shorter than `epsilon` canvas px as unchanged.
    pub fn new(epsilon: f32) -> Self {
        Self {
            rendered: Vec::with_capacity(256),
            epsilon,
        }
    }

    /// Diff `current` against the rendered positions and report the first
    /// index to re-render, recording `current` as rendered from there on.
    pub fn update(&mut self, current: &[PaintInformation], max_window: usize) -> Option<usize> {
        let divergence = find_divergence(current, &self.rendered, max_window, self.epsilon);
        let from = divergence.unwrap_or(self.rendered.len());
        self.rendered.truncate(from);
        self.rendered.extend(current[from..].iter().map(|p| p.pos));
        divergence
    }

    /// Forget the rendered positions for a new stroke.
    pub fn clear(&mut self) {
        self.rendered.clear();
    }
}

/// The trait that all stabilizer algorithms implement.
pub trait StabilizerAlgorithm: Send {
    /// Append a raw input point, run the algorithm, and return the result.
    fn push(&mut self, point: PaintInformation) -> StabilizeResult;

    /// Forget the most recent point, so the next `push` replaces it. The
    /// polyline is only meaningful again after that push.
    fn retract_tip(&mut self);

    /// The current stabilized polyline (full stroke).
    fn stabilized(&self) -> &[PaintInformation];

    /// Number of points in the stabilized polyline.
    fn len(&self) -> usize {
        self.stabilized().len()
    }

    /// Whether the stabilized polyline is empty.
    fn is_empty(&self) -> bool {
        self.stabilized().is_empty()
    }

    /// Conservative upper bound on how far back from the tip divergence
    /// can reach (in vector indices). Used to space checkpoints so the
    /// oldest one is past the divergence boundary.
    fn max_divergence_window(&self) -> usize {
        0
    }

    /// Reset for a new stroke.
    fn clear(&mut self);
}

/// A pass-through "stabilizer" that does nothing: output equals input.
/// Used when no stabilization is configured (empty algorithm string).
pub struct PassThrough {
    points: Vec<PaintInformation>,
}

impl Default for PassThrough {
    fn default() -> Self {
        Self::new()
    }
}

impl PassThrough {
    pub fn new() -> Self {
        Self {
            points: Vec::with_capacity(256),
        }
    }
}

impl StabilizerAlgorithm for PassThrough {
    fn push(&mut self, point: PaintInformation) -> StabilizeResult {
        self.points.push(point);
        StabilizeResult {
            divergence_index: None,
        }
    }

    fn retract_tip(&mut self) {
        self.points.pop();
    }

    fn stabilized(&self) -> &[PaintInformation] {
        &self.points
    }

    fn clear(&mut self) {
        self.points.clear();
    }
}

/// Minimum real vertices before prediction engages: enough for a stable
/// heading and a measured speed. Below this the decorator is a pass-through
/// of the inner stabilizer's result.
const MIN_REAL_FOR_PREDICTION: usize = 3;

/// Predicted points appended past the real tip. Constant, so the divergence
/// window is static and the tail's density does not depend on input cadence.
const PREDICTED_POINTS: usize = 3;

/// Number of recent real segments the heading and speed are measured over.
/// Under resampling the last one is the partial segment to the pinned tip;
/// a speed (distance over elapsed time) is unaffected by its shorter length.
const HEADING_WINDOW: usize = 3;

/// Prediction decorator: wraps a real stabilizer and appends a short
/// extrapolated tail past the real tip, so ink appears ahead of the pen and
/// hides the residual pen-to-pixel latency.
///
/// The predicted points live in `stabilized()` **and** in the polyline
/// [`DivergenceDiff`] diffs, so the engine's existing rewind rewrites them
/// every frame: no separate render target, no parallel path. The predicted
/// count is constant once engaged, so the combined polyline grows as the
/// inner's does (by any number of vertices per push under resampling) and
/// reshapes: both cases the diff handles.
///
/// Only constructed when a real stabilizer is active (strength > 0) and a
/// look-ahead horizon is configured (> 0); see the engine's stroke-start path
/// and `docs/plans/stroke-prediction-stabilizer.md`.
pub struct PredictingStabilizer {
    inner: Box<dyn StabilizerAlgorithm>,
    /// Real + predicted polyline: what `stabilized()` returns.
    combined: Vec<PaintInformation>,
    /// Rendered positions of `combined` (divergence diff input).
    diff: DivergenceDiff,
    /// Look-ahead horizon in seconds (converted from the ms port value).
    horizon_secs: f32,
}

impl PredictingStabilizer {
    /// Wrap `inner` with prediction over a `horizon_ms` millisecond
    /// look-ahead, treating moves under `epsilon` canvas px as unchanged.
    pub fn new(inner: Box<dyn StabilizerAlgorithm>, horizon_ms: f32, epsilon: f32) -> Self {
        Self {
            inner,
            combined: Vec::with_capacity(256),
            diff: DivergenceDiff::new(epsilon),
            horizon_secs: (horizon_ms / 1000.0).max(0.0),
        }
    }

    /// Predicted points this decorator appends once engaged: none when the
    /// horizon is off.
    fn predicted_points(&self) -> usize {
        if self.horizon_secs > 0.0 {
            PREDICTED_POINTS
        } else {
            0
        }
    }

    /// Append `n` extrapolated points past the real tip. Reads the real
    /// prefix `self.combined[..real_len]` by value (all `Copy`) before pushing.
    fn append_prediction(&mut self, real_len: usize, n: usize) {
        let k = HEADING_WINDOW.min(real_len - 1);
        let tip = self.combined[real_len - 1];
        let base = self.combined[real_len - 1 - k];
        let dx = tip.pos[0] - base.pos[0];
        let dy = tip.pos[1] - base.pos[1];
        let dist = (dx * dx + dy * dy).sqrt();
        if dist < 1.0e-4 {
            // Stationary: no meaningful heading; collapse onto the tip.
            for _ in 0..n {
                self.combined.push(tip);
            }
            return;
        }
        let heading = [dx / dist, dy / dist];
        // The tail spans the horizon at the recent pen speed, so the
        // predicted distance is speed-proportional whatever the vertex
        // spacing or input cadence. Without a measurable elapsed time, fall
        // back to the mean per-vertex displacement.
        let dt = tip.time - base.time;
        let step = if dt > 0.0 {
            self.horizon_secs * (dist / dt) / n as f32
        } else {
            dist / k as f32
        };

        // Curvature/reversal damping pulls the tail toward the tip rather
        // than removing points: kills the reversal "whisker" and keeps the
        // point count constant.
        let damp = self.reversal_damp(real_len, heading, k);

        for j in 1..=n {
            let d = step * j as f32 * damp;
            let mut p = tip;
            p.pos = [tip.pos[0] + heading[0] * d, tip.pos[1] + heading[1] * d];
            self.combined.push(p);
        }
    }

    /// Damping factor in [0, 1] from heading alignment: 1 when the stroke
    /// continues straight, 0 when it reverses onto itself.
    fn reversal_damp(&self, real_len: usize, heading: [f32; 2], k: usize) -> f32 {
        // Need a preceding segment of the same span to compare against.
        if real_len < 2 * k + 1 {
            return 1.0;
        }
        let a = self.combined[real_len - 1 - k];
        let b = self.combined[real_len - 1 - 2 * k];
        let dx = a.pos[0] - b.pos[0];
        let dy = a.pos[1] - b.pos[1];
        let dist = (dx * dx + dy * dy).sqrt();
        if dist < 1.0e-4 {
            return 1.0;
        }
        let prev_heading = [dx / dist, dy / dist];
        let align = heading[0] * prev_heading[0] + heading[1] * prev_heading[1];
        align.clamp(0.0, 1.0)
    }
}

impl StabilizerAlgorithm for PredictingStabilizer {
    fn push(&mut self, point: PaintInformation) -> StabilizeResult {
        // Advance the inner (real) stabilizer, then rebuild the combined
        // polyline from its relaxed output.
        self.inner.push(point);
        self.combined.clear();
        self.combined.extend_from_slice(self.inner.stabilized());
        let real_len = self.combined.len();

        // Append the predicted extension once enough real vertices exist.
        let n = self.predicted_points();
        if n > 0 && real_len >= MIN_REAL_FOR_PREDICTION {
            self.append_prediction(real_len, n);
        }

        // Divergence over the FULL combined polyline (not the inner's
        // real-only result), with the widened window, which is what makes the
        // existing rewind rewrite the predicted tail every frame.
        let window = self.max_divergence_window();
        let divergence_index = self.diff.update(&self.combined, window);
        StabilizeResult { divergence_index }
    }

    fn retract_tip(&mut self) {
        self.inner.retract_tip();
    }

    fn stabilized(&self) -> &[PaintInformation] {
        &self.combined
    }

    /// The inner window widened by the constant predicted count, so the
    /// checkpoint ring spaces its snapshots deep enough to rewind over the
    /// predicted region and the engine's coverage assert still holds.
    fn max_divergence_window(&self) -> usize {
        self.inner.max_divergence_window() + self.predicted_points()
    }

    fn clear(&mut self) {
        self.inner.clear();
        self.combined.clear();
        self.diff.clear();
    }
}

/// What each stabilizer module returns from its `register()` function.
pub struct StabilizerRegistration {
    pub type_id: &'static str,
    pub display_name: &'static str,
    pub params: &'static [ParamDef],
    pub from_params: fn(&[ParamValue]) -> Box<dyn StabilizerAlgorithm>,
}

/// Auto-discovered stabilizer registry.
pub struct StabilizerRegistry {
    entries: HashMap<&'static str, StabilizerRegistration>,
}

impl Default for StabilizerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl StabilizerRegistry {
    pub fn new() -> Self {
        let mut entries = HashMap::new();
        for reg in super::stabilizers::registrations() {
            entries.insert(reg.type_id, reg);
        }
        StabilizerRegistry { entries }
    }

    /// Return all registered stabilizer type IDs with their parameter definitions.
    pub fn types(&self) -> Vec<(&'static str, &'static str, &'static [ParamDef])> {
        let mut types: Vec<_> = self
            .entries
            .iter()
            .map(|(&id, reg)| (id, reg.display_name, reg.params))
            .collect();
        types.sort_by_key(|(id, _, _)| *id);
        types
    }

    /// Get the static parameter definitions for a stabilizer type.
    pub fn param_defs(&self, type_id: &str) -> &'static [ParamDef] {
        self.entries.get(type_id).map(|e| e.params).unwrap_or(&[])
    }

    /// Create a stabilizer algorithm instance from a type string and parameters.
    /// Returns `None` if the type_id is not found.
    pub fn create(
        &self,
        type_id: &str,
        params: &[ParamValue],
    ) -> Option<Box<dyn StabilizerAlgorithm>> {
        self.entries
            .get(type_id)
            .map(|reg| (reg.from_params)(params))
    }

    /// Create a stabilizer from a `StabilizerConfig`.
    /// Returns a pass-through if the config has no algorithm set.
    pub fn create_from_config(&self, config: &StabilizerConfig) -> Box<dyn StabilizerAlgorithm> {
        if config.algorithm.is_empty() || config.algorithm == "none" {
            return Box::new(PassThrough::new());
        }
        self.create(&config.algorithm, &config.params)
            .unwrap_or_else(|| {
                log::warn!(
                    "unknown stabilizer algorithm '{}', using pass-through",
                    config.algorithm
                );
                Box::new(PassThrough::new())
            })
    }
}

/// Build the stabilizer stack a stroke runs through: the configured
/// algorithm fed by a [`ResamplingStabilizer`], wrapped in prediction when a
/// look-ahead horizon is set.
///
/// `canvas_per_css_px` is the canvas pixels one CSS pixel of pen travel spans
/// at the stroke's view (`device_pixel_ratio / zoom`). The resample spacing
/// and the divergence epsilon are both scaled by it, so the smoothing reach
/// and what counts as a move are the same on screen at every zoom and pixel
/// ratio. Decorators engage only over an algorithm that can reshape rendered
/// vertices (`max_divergence_window() > 0`).
pub fn stroke_stabilizer_stack(
    registry: &StabilizerRegistry,
    config: &StabilizerConfig,
    prediction_horizon_ms: f32,
    canvas_per_css_px: f32,
) -> Box<dyn StabilizerAlgorithm> {
    let inner = registry.create_from_config(config);
    if inner.max_divergence_window() == 0 {
        return inner;
    }
    let epsilon = DIVERGENCE_EPSILON * canvas_per_css_px;
    let inner = Box::new(ResamplingStabilizer::new(
        inner,
        RESAMPLE_SPACING_CSS_PX * canvas_per_css_px,
        epsilon,
    ));
    if prediction_horizon_ms > 0.0 {
        Box::new(PredictingStabilizer::new(
            inner,
            prediction_horizon_ms,
            epsilon,
        ))
    } else {
        inner
    }
}

/// Per-brush stabilizer configuration: stored in `BrushMetadata`.
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct StabilizerConfig {
    /// Algorithm type_id.  Empty string or "none" = pass-through.
    #[serde(default)]
    pub algorithm: String,
    /// Algorithm-specific parameter values.
    #[serde(default)]
    pub params: Vec<ParamValue>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pass_through_identity() {
        let mut stab = PassThrough::new();
        for i in 0..5 {
            let pt = PaintInformation {
                pos: [i as f32 * 10.0, 0.0],
                pressure: 0.5,
                ..Default::default()
            };
            let result = stab.push(pt);
            assert!(result.divergence_index.is_none());
        }
        assert_eq!(stab.len(), 5);
        // Points are unchanged (no smoothing).
        assert!((stab.stabilized()[2].pos[0] - 20.0).abs() < 1e-6);
    }

    #[test]
    fn pass_through_clear() {
        let mut stab = PassThrough::new();
        stab.push(PaintInformation::default());
        assert_eq!(stab.len(), 1);
        stab.clear();
        assert_eq!(stab.len(), 0);
    }

    #[test]
    fn stabilizer_config_default_is_pass_through() {
        let config = StabilizerConfig::default();
        assert!(config.algorithm.is_empty());
        assert!(config.params.is_empty());
    }

    #[test]
    fn stabilizer_config_serde_round_trip() {
        let config = StabilizerConfig {
            algorithm: "laplacian".into(),
            params: vec![ParamValue::Float(0.6)],
        };
        let json = serde_json::to_string(&config).unwrap();
        let loaded: StabilizerConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.algorithm, "laplacian");
        assert_eq!(loaded.params.len(), 1);
    }

    #[test]
    fn stabilizer_config_missing_fields_default() {
        let json = "{}";
        let config: StabilizerConfig = serde_json::from_str(json).unwrap();
        assert!(config.algorithm.is_empty());
        assert!(config.params.is_empty());
    }

    #[test]
    fn registry_creates_from_config() {
        let registry = StabilizerRegistry::new();

        // Empty config → pass-through.
        let config = StabilizerConfig::default();
        let stab = registry.create_from_config(&config);
        assert_eq!(stab.len(), 0);

        // "none" → pass-through.
        let config = StabilizerConfig {
            algorithm: "none".into(),
            params: vec![],
        };
        let stab = registry.create_from_config(&config);
        assert_eq!(stab.len(), 0);

        // Known algorithm.
        let config = StabilizerConfig {
            algorithm: "laplacian".into(),
            params: vec![ParamValue::Float(0.5)],
        };
        let mut stab = registry.create_from_config(&config);
        stab.push(PaintInformation::default());
        assert_eq!(stab.len(), 1);
    }

    #[test]
    fn registry_discovers_algorithms() {
        let registry = StabilizerRegistry::new();
        let types = registry.types();
        assert!(
            !types.is_empty(),
            "registry should discover at least one algorithm"
        );
        assert!(types.iter().any(|(id, _, _)| *id == "laplacian"));
    }

    // ── Stroke stabilizer stack ─────────────────────────────────────────

    fn laplacian_config(strength: f32) -> StabilizerConfig {
        StabilizerConfig {
            algorithm: "laplacian".into(),
            params: vec![ParamValue::Float(strength)],
        }
    }

    /// Distance from the raw corner (40, 0) of an L path to the nearest
    /// stabilized vertex, with the path sampled every `step` px.
    fn l_corner_cut(step: f32) -> f32 {
        let mut stab =
            stroke_stabilizer_stack(&StabilizerRegistry::new(), &laplacian_config(0.8), 0.0, 1.0);
        let n = (40.0 / step).round() as usize;
        for i in 0..=n {
            stab.push(mk(i as f32 * step, 0.0, 0.0));
        }
        for i in 1..=n {
            stab.push(mk(40.0, i as f32 * step, 0.0));
        }
        stab.stabilized()
            .iter()
            .map(|p| dist(p.pos, [40.0, 0.0]))
            .fold(f32::INFINITY, f32::min)
    }

    /// Regression: the same path smooths the same whatever the pointer event
    /// rate. The Laplacian's reach is a count of vertices, so dense raw input
    /// (a high event rate, or a slow pen) used to shrink it to almost nothing.
    #[test]
    fn same_path_at_1x_and_8x_density_gives_the_same_geometry() {
        let sparse = l_corner_cut(10.0);
        let dense = l_corner_cut(1.25);
        assert!(
            sparse > 3.0 && dense > 3.0,
            "the corner must be smoothed at both densities: sparse {sparse}, dense {dense}"
        );
        assert!(
            (sparse - dense).abs() < 0.05,
            "corner cut must not depend on sample density: sparse {sparse}, dense {dense}"
        );
    }

    // ── PredictingStabilizer ────────────────────────────────────────────

    /// A pen sample at position `(x, y)` and timestamp `t` (seconds).
    fn mk(x: f32, y: f32, t: f32) -> PaintInformation {
        PaintInformation {
            pos: [x, y],
            pressure: 0.5,
            time: t,
            ..Default::default()
        }
    }

    /// A laplacian inner stabilizer at the given strength, via the registry
    /// (avoids depending on the generated module path).
    fn laplacian_inner(strength: f32) -> Box<dyn StabilizerAlgorithm> {
        StabilizerRegistry::new()
            .create("laplacian", &[ParamValue::Float(strength)])
            .expect("laplacian registered")
    }

    fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
        let dx = a[0] - b[0];
        let dy = a[1] - b[1];
        (dx * dx + dy * dy).sqrt()
    }

    /// T-B: with prediction on and a straight stroke, `stabilized()` extends
    /// past the last real point along its heading by ~the horizon (fixed
    /// point count).
    #[test]
    fn prediction_extends_tip_on_straight_stroke() {
        // 30ms horizon, samples 10px / 10ms apart ⇒ N = round(30/10) = 3.
        let mut stab = PredictingStabilizer::new(laplacian_inner(0.5), 30.0, DIVERGENCE_EPSILON);
        for i in 0..8 {
            stab.push(mk(i as f32 * 10.0, 0.0, i as f32 * 0.01));
        }
        let n = 3;
        let pts = stab.stabilized();
        assert_eq!(pts.len(), 8 + n, "combined = 8 real + {n} predicted");

        // The last real point of a straight stroke stays pinned at x = 70.
        let real_tip_x = pts[7].pos[0];
        assert!((real_tip_x - 70.0).abs() < 1e-3);
        for pred in &pts[8..8 + n] {
            assert!(
                pred.pos[0] > real_tip_x,
                "predicted x {} should extend past the tip",
                pred.pos[0]
            );
            assert!(pred.pos[1].abs() < 1e-2, "straight stroke stays on y=0");
        }
        // Last predicted point ≈ horizon ahead: 3 steps × 10px = 30px past tip.
        assert!((pts[10].pos[0] - (real_tip_x + 30.0)).abs() < 2.0);
    }

    /// T-C: after a turn, the combined-polyline divergence lands at/inside the
    /// prediction boundary and stays within the widened window, and the
    /// predicted tail is rewritten to follow the turn (self-correction).
    #[test]
    fn prediction_self_corrects_via_combined_divergence() {
        let mut stab = PredictingStabilizer::new(laplacian_inner(0.5), 30.0, DIVERGENCE_EPSILON);
        for i in 0..8 {
            stab.push(mk(i as f32 * 10.0, 0.0, i as f32 * 0.01));
        }
        // Straight-stroke prediction runs along +x (y≈0).
        assert!(stab.stabilized().last().unwrap().pos[1].abs() < 1e-2);

        // A turn downward.
        let r = stab.push(mk(80.0, 30.0, 0.08));
        let real_len = 9; // 9 real points, N = 3 predicted
        let tip_vi = stab.stabilized().len() - 1;
        let window = stab.max_divergence_window();

        let div = r.divergence_index.expect("a turn must diverge");
        assert!(
            div <= real_len,
            "divergence {div} must cover the predicted tail (<= real_len {real_len})"
        );
        assert!(
            div >= tip_vi.saturating_sub(window),
            "divergence {div} must stay within the widened window \
             (tip_vi {tip_vi}, window {window})"
        );

        // The predicted tail now heads into the turn (y grew from ~0).
        let last_pred_y = stab.stabilized().last().unwrap().pos[1];
        assert!(
            last_pred_y > 5.0,
            "predicted tail should follow the turn, got y={last_pred_y}"
        );
    }

    /// T-D: a sharp reversal collapses the predicted extension toward the real
    /// tip (no overshoot whisker) while keeping the point count constant.
    #[test]
    fn reversal_collapses_predicted_tail() {
        let mut stab = PredictingStabilizer::new(laplacian_inner(0.5), 30.0, DIVERGENCE_EPSILON);
        // Rightward…
        for i in 0..6 {
            stab.push(mk(i as f32 * 10.0, 0.0, i as f32 * 0.01));
        }
        // …then reverse back leftward.
        stab.push(mk(40.0, 0.0, 0.06));
        stab.push(mk(30.0, 0.0, 0.07));
        stab.push(mk(20.0, 0.0, 0.08));

        let n = 3;
        let pts = stab.stabilized();
        let real_len = pts.len() - n;
        assert_eq!(pts.len(), real_len + n, "point count stays constant at N");

        let tip = pts[real_len - 1].pos;
        // A straight extension would place the far predicted point ~N×step
        // (≈30px) away; damping pulls it in to well under one step.
        let far = dist(pts[pts.len() - 1].pos, tip);
        assert!(
            far < 10.0,
            "reversed predicted tail should collapse toward the tip, got {far}px"
        );
    }

    /// T-E: horizon 0 ⇒ the decorator is transparent: `stabilized()` and the
    /// divergence result match a bare inner stabilizer, frame for frame.
    #[test]
    fn horizon_zero_is_transparent() {
        let mut pred = PredictingStabilizer::new(laplacian_inner(0.5), 0.0, DIVERGENCE_EPSILON);
        let mut bare = laplacian_inner(0.5);
        for i in 0..8 {
            let p = mk(i as f32 * 10.0, (i as f32).sin() * 5.0, i as f32 * 0.01);
            let rp = pred.push(p);
            let rb = bare.push(p);
            assert_eq!(rp.divergence_index, rb.divergence_index, "step {i}");
        }
        assert_eq!(pred.max_divergence_window(), bare.max_divergence_window());
        assert_eq!(pred.stabilized().len(), bare.stabilized().len());
        for (a, b) in pred.stabilized().iter().zip(bare.stabilized()) {
            assert!(dist(a.pos, b.pos) < 1e-6, "positions must match bare inner");
        }
    }

    /// Regression: on fixed-spacing input the predicted tail spans the
    /// horizon at the current pen speed, with a constant point count. A
    /// cadence-derived count and a per-vertex step would freeze the tail at
    /// `count x spacing` whatever the speed.
    #[test]
    fn prediction_on_fixed_spacing_spans_horizon_at_current_speed() {
        let mut stab = PredictingStabilizer::new(laplacian_inner(0.5), 30.0, DIVERGENCE_EPSILON);
        // 6 px vertices: 500 px/s for 12 vertices, then 2000 px/s.
        let (mut x, mut t) = (0.0f32, 0.0f32);
        let mut tail_at = |stab: &mut PredictingStabilizer, speed: f32, n: usize| {
            let mut tail = 0.0;
            for _ in 0..n {
                x += 6.0;
                t += 6.0 / speed;
                stab.push(mk(x, 0.0, t));
                let pts = stab.stabilized();
                let real = (x / 6.0).round() as usize;
                if real >= MIN_REAL_FOR_PREDICTION {
                    assert_eq!(
                        pts.len(),
                        real + PREDICTED_POINTS,
                        "constant predicted count"
                    );
                }
                tail = pts.last().unwrap().pos[0] - x;
            }
            tail
        };
        let slow = tail_at(&mut stab, 500.0, 12);
        let fast = tail_at(&mut stab, 2000.0, 12);
        assert!(
            (slow - 15.0).abs() < 3.0,
            "slow tail {slow}, expected ~15 px"
        );
        assert!(
            (fast - 60.0).abs() < 12.0,
            "fast tail {fast}, expected ~60 px"
        );
    }

    /// T-I: at stroke start the first < 3 real samples emit no prediction;
    /// once engaged, the polyline grows by exactly one per push and the
    /// divergence never reports outside the (ramping) window.
    #[test]
    fn stroke_start_ramps_without_breaking_growth() {
        let mut stab = PredictingStabilizer::new(laplacian_inner(0.5), 30.0, DIVERGENCE_EPSILON);
        let mut prev_len = 0usize;
        for i in 0..12 {
            let r = stab.push(mk(i as f32 * 10.0, 0.0, i as f32 * 0.01));
            let len = stab.stabilized().len();
            let tip_vi = len.saturating_sub(1);
            let window = stab.max_divergence_window();

            if let Some(div) = r.divergence_index {
                assert!(
                    div >= tip_vi.saturating_sub(window),
                    "step {i}: div {div} outside window (tip_vi {tip_vi}, window {window})"
                );
            }

            // Below MIN_REAL_FOR_PREDICTION real samples: no predicted tail.
            if i < MIN_REAL_FOR_PREDICTION - 1 {
                assert_eq!(len, i + 1, "step {i}: no prediction before ramp");
            } else if i > MIN_REAL_FOR_PREDICTION - 1 {
                // Past the one-time engage jump, growth is exactly one/push.
                assert_eq!(len, prev_len + 1, "step {i}: should grow by one");
            }
            prev_len = len;
        }
    }
}
