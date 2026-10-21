//! Preserve road corners between timed observations so connecting lines stay on campus.
use super::{geom::fmt_gain_time, model::GenPoint};
use route_planner::{boundary, Coord, Route};

pub(super) fn preserve_corners(
    points: Vec<GenPoint>,
    positions: &[f64],
    route: &Route,
    fences: &[Vec<Coord>],
    start_ms: i64,
) -> Result<Vec<GenPoint>, String> {
    let length = route.length_m;
    if !length.is_finite() || length <= 0.0 || points.len() != positions.len() {
        return Err("学校路网无法形成有效路线".into());
    }
    let mut out = Vec::with_capacity(points.len() + route.points.len());
    for (i, p) in points.iter().enumerate() {
        if i > 0 && positions[i] > positions[i - 1] {
            let a = &points[i - 1];
            let from = positions[i - 1];
            let to = positions[i];
            let first = (from / length).floor() as usize;
            let last = (to / length).floor() as usize;
            for lap in first..=last {
                for corner in &route.points {
                    // The closing vertex of the preceding lap already represents s=0.
                    if corner.s == 0.0 {
                        continue;
                    }
                    let s = lap as f64 * length + corner.s;
                    if s <= from + 1e-8 || s >= to - 1e-8 {
                        continue;
                    }
                    let f = (s - from) / (to - from);
                    let lerp = |x: f64, y: f64| x + (y - x) * f;
                    let mut q = p.clone();
                    q.gLat = corner.lat;
                    q.gLng = corner.lon;
                    q.totalDis = lerp(a.totalDis, p.totalDis);
                    q.validDis = lerp(a.validDis, p.validDis);
                    q.totalTime = lerp(a.totalTime as f64, p.totalTime as f64).round() as i64;
                    q.validTime = lerp(a.validTime as f64, p.validTime as f64).round() as i64;
                    q.steps = lerp(a.steps as f64, p.steps as f64).round() as i64;
                    q.gainTimeMs = start_ms + q.totalTime * 1000;
                    q.gainTime = fmt_gain_time(q.gainTimeMs);
                    q.ptype = 0;
                    q.locType = 1;
                    out.push(q);
                }
            }
        }
        out.push(p.clone());
    }
    for (i, p) in out.iter_mut().enumerate() {
        p.id = i as i64 + 1;
    }
    let coord = |p: &GenPoint| Coord::new(p.gLng, p.gLat);
    if !out.iter().all(|p| boundary::contains(fences, coord(p)))
        || !out
            .windows(2)
            .all(|p| boundary::segment_inside(fences, coord(&p[0]), coord(&p[1])))
    {
        return Err("最终轨迹或连线越出学校围栏，已停止生成".into());
    }
    Ok(out)
}
