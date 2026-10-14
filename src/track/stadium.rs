//! Standard athletics-track geometry fitted to the live server checkpoints.

use serde::{Deserialize, Serialize};

use super::geom::{PointRing, MET_PER_DEG_LAT, MET_PER_DEG_LNG};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrackSpec {
    #[serde(rename = "auto", alias = "Auto")]
    Auto,
    #[serde(rename = "200m", alias = "200", alias = "m200", alias = "M200")]
    M200,
    #[serde(rename = "300m", alias = "300", alias = "m300", alias = "M300")]
    M300,
    #[serde(rename = "400m", alias = "400", alias = "m400", alias = "M400")]
    M400,
    #[serde(rename = "custom")]
    Custom { total_m: u32 },
}

impl Default for TrackSpec {
    fn default() -> Self {
        Self::Auto
    }
}

impl TrackSpec {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Auto => "自动（服务器点位）",
            Self::M200 => "200 米标准跑道",
            Self::M300 => "300 米标准跑道",
            Self::M400 => "400 米标准跑道",
            Self::Custom { .. } => "自定义跑道",
        }
    }

    fn dimensions(self) -> Option<TrackDimensions> {
        let total_m = match self {
            Self::Auto => return None,
            Self::M200 => 200,
            Self::M300 => 300,
            Self::M400 => 400,
            Self::Custom { total_m } if (100..=1000).contains(&total_m) => total_m,
            Self::Custom { .. } => return None,
        };
        let base = TrackDimensions {
            straight_m: 84.39,
            radius_m: 36.80,
        };
        let scale = total_m as f64 / base.length_m();
        Some(TrackDimensions {
            straight_m: base.straight_m * scale,
            radius_m: base.radius_m * scale,
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct TrackDimensions {
    straight_m: f64,
    radius_m: f64,
}

impl TrackDimensions {
    fn length_m(self) -> f64 {
        2.0 * self.straight_m + 2.0 * std::f64::consts::PI * self.radius_m
    }
}

/// Fit a standard oval around the live checkpoints and return it in the same
/// local metre plane used by the regular track generator.
pub fn make_stadium_ring(points: &[(f64, f64)], spec: TrackSpec) -> Result<PointRing, String> {
    let dimensions = spec
        .dimensions()
        .ok_or_else(|| "请选择有效的操场规格".to_string())?;
    if points.len() < 2 {
        return Err("服务器打卡点不足，无法定位标准跑道".into());
    }
    if points
        .iter()
        .any(|(lat, lng)| !lat.is_finite() || !lng.is_finite())
    {
        return Err("服务器打卡点包含非有限坐标".into());
    }

    let center_lat = points.iter().map(|point| point.0).sum::<f64>() / points.len() as f64;
    let center_lng = points.iter().map(|point| point.1).sum::<f64>() / points.len() as f64;
    let local: Vec<_> = points
        .iter()
        .map(|(lat, lng)| {
            (
                (lng - center_lng) * MET_PER_DEG_LNG,
                (lat - center_lat) * MET_PER_DEG_LAT,
            )
        })
        .collect();
    let (angle, offset) = fit_pose(&local, dimensions);
    let mut dense: Vec<_> = rounded_rectangle(dimensions)
        .into_iter()
        .map(|point| rotate((point.0 + offset.0, point.1 + offset.1), angle))
        .collect();

    // Refuse a preset that clearly does not describe the checkpoint geometry.
    let tolerance = dimensions.radius_m * 0.30 + 3.0;
    let furthest = local
        .iter()
        .map(|&point| {
            let aligned = rotate(point, -angle);
            distance_to_centerline(
                (aligned.0 - offset.0, aligned.1 - offset.1),
                dimensions,
            )
        })
        .fold(0.0, f64::max);
    if furthest > tolerance {
        return Err(format!(
            "服务器打卡点与{}跑道不匹配：最大偏差 {:.1}m（允许 {:.1}m）",
            spec.label(),
            furthest,
            tolerance
        ));
    }

    // Put close server checkpoints directly on the route so their markers do
    // not depend on a later sample accidentally landing at the same location.
    let mut used = std::collections::HashSet::new();
    for &(lat, lng) in points {
        let target = (
            (lng - center_lng) * MET_PER_DEG_LNG,
            (lat - center_lat) * MET_PER_DEG_LAT,
        );
        let nearest = dense
            .iter()
            .enumerate()
            .map(|(index, &point)| (index, distance(point, target)))
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((index, distance_to_ring)) = nearest {
            if distance_to_ring <= 3.0 {
                let index = nearest_unused(index, dense.len(), &used);
                used.insert(index);
                dense[index] = target;
            }
        }
    }

    let mut arcs = vec![0.0];
    for index in 1..=dense.len() {
        arcs.push(arcs[index - 1] + distance(dense[index - 1], dense[index % dense.len()]));
    }
    if (arcs[dense.len()] - dimensions.length_m()).abs() > 4.0 {
        return Err("标准跑道采样长度偏离设计值过大".into());
    }
    Ok((dense, arcs, (center_lat, center_lng)))
}

fn nearest_unused(
    preferred: usize,
    count: usize,
    used: &std::collections::HashSet<usize>,
) -> usize {
    (0..count)
        .map(|offset| (preferred + offset) % count)
        .find(|index| !used.contains(index))
        .unwrap_or(preferred)
}

fn fit_pose(points: &[(f64, f64)], dimensions: TrackDimensions) -> (f64, (f64, f64)) {
    let mut best_angle = 0.0;
    let mut best_error = f64::INFINITY;
    let mut best_offset = bbox_center(points);
    for index in 0..=180 {
        let angle = -std::f64::consts::FRAC_PI_2
            + index as f64 * std::f64::consts::PI / 180.0;
        let rotated: Vec<_> = points.iter().map(|&point| rotate(point, -angle)).collect();
        let offset = bbox_center(&rotated);
        let error = rotated
            .iter()
            .map(|&(x, y)| {
                let d = distance_to_centerline((x - offset.0, y - offset.1), dimensions);
                d * d
            })
            .sum::<f64>();
        if error < best_error {
            best_error = error;
            best_angle = angle;
            best_offset = offset;
        }
    }
    (best_angle, best_offset)
}

fn rounded_rectangle(dimensions: TrackDimensions) -> Vec<(f64, f64)> {
    let straight = dimensions.straight_m;
    let radius = dimensions.radius_m;
    let samples = |length: f64| (length / 0.75).ceil().max(8.0) as usize;
    let straight_count = samples(straight);
    let arc_count = samples(std::f64::consts::PI * radius);
    let mut points = Vec::with_capacity(straight_count * 2 + arc_count * 2);
    for index in 0..straight_count {
        let t = index as f64 / straight_count as f64;
        points.push((-straight / 2.0 + straight * t, radius));
    }
    for index in 0..arc_count {
        let angle = std::f64::consts::FRAC_PI_2
            - std::f64::consts::PI * index as f64 / arc_count as f64;
        points.push((straight / 2.0 + radius * angle.cos(), radius * angle.sin()));
    }
    for index in 0..straight_count {
        let t = index as f64 / straight_count as f64;
        points.push((straight / 2.0 - straight * t, -radius));
    }
    for index in 0..arc_count {
        let angle = -std::f64::consts::FRAC_PI_2
            - std::f64::consts::PI * index as f64 / arc_count as f64;
        points.push((-straight / 2.0 + radius * angle.cos(), radius * angle.sin()));
    }
    points
}

fn bbox_center(points: &[(f64, f64)]) -> (f64, f64) {
    let (mut min_x, mut max_x, mut min_y, mut max_y) =
        (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
    for &(x, y) in points {
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    ((min_x + max_x) / 2.0, (min_y + max_y) / 2.0)
}

fn rotate(point: (f64, f64), angle: f64) -> (f64, f64) {
    let (sin, cos) = angle.sin_cos();
    (point.0 * cos - point.1 * sin, point.0 * sin + point.1 * cos)
}

fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

fn distance_to_centerline(point: (f64, f64), dimensions: TrackDimensions) -> f64 {
    let half_straight = dimensions.straight_m / 2.0;
    let radius = dimensions.radius_m;
    if point.0.abs() <= half_straight {
        (point.1.abs() - radius).abs()
    } else {
        let center_x = if point.0.is_sign_negative() {
            -half_straight
        } else {
            half_straight
        };
        ((point.0 - center_x).hypot(point.1) - radius).abs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checkpoints(spec: TrackSpec, lat: f64, lng: f64, angle: f64) -> Vec<(f64, f64)> {
        let dimensions = spec.dimensions().unwrap();
        rounded_rectangle(dimensions)
            .iter()
            .step_by((rounded_rectangle(dimensions).len() / 8).max(1))
            .map(|&(x, y)| {
                let point = rotate((x, y), angle);
                (
                    lat + point.1 / MET_PER_DEG_LAT,
                    lng + point.0 / MET_PER_DEG_LNG,
                )
            })
            .collect()
    }

    #[test]
    fn presets_have_expected_nominal_lengths() {
        for (spec, expected) in [
            (TrackSpec::M200, 200.0),
            (TrackSpec::M300, 300.0),
            (TrackSpec::M400, 400.0),
        ] {
            assert!((spec.dimensions().unwrap().length_m() - expected).abs() < 0.01);
        }
        assert!(TrackSpec::Custom { total_m: 99 }.dimensions().is_none());
    }

    #[test]
    fn fitted_ring_keeps_nominal_length_and_checkpoint_orientation() {
        let spec = TrackSpec::M400;
        let points = checkpoints(spec, 38.9, 121.54, 0.35);
        let (ring, arcs, _) = make_stadium_ring(&points, spec).unwrap();
        assert!((arcs.last().unwrap() - 400.0).abs() < 4.0);
        assert!(ring.iter().any(|point| point.0.abs() > 70.0));
    }

    #[test]
    fn generated_stadium_track_keeps_totals_consistent() {
        let spec = TrackSpec::M400;
        let points = checkpoints(spec, 38.9, 121.54, 0.35);
        let track = crate::track::generator::build_stadium(
            1200.0,
            600,
            7,
            (38.9, 121.54),
            1_700_000_000_000,
            &points,
            spec,
        )
        .unwrap();
        assert!(track.validate_consistency().is_ok());
    }

    #[test]
    fn rejects_points_that_do_not_fit_the_selected_preset() {
        let points = [(38.9, 121.54), (38.92, 121.54), (38.9, 121.56)];
        assert!(make_stadium_ring(&points, TrackSpec::M400).is_err());
    }
}
