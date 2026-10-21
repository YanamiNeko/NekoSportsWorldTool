//! Local GPX analysis only. Never feeds the record submission pipeline.
use route_planner::{boundary, Coord};
#[derive(Clone, Debug)]
pub struct Analysis {
    pub segments: Vec<Vec<(f64, f64)>>,
    pub distance_m: f64,
    pub duration_s: Option<f64>,
    pub pace_min_km: Option<f64>,
    pub coverage: Vec<bool>,
    pub outside: Vec<(f64, f64)>,
    pub crossing: Vec<Vec<(f64, f64)>>,
    pub warnings: Vec<String>,
}
pub fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    let dy = (b.0 - a.0).to_radians();
    let dx = (b.1 - a.1).to_radians();
    let h = (dy / 2.).sin().powi(2)
        + a.0.to_radians().cos() * b.0.to_radians().cos() * (dx / 2.).sin().powi(2);
    6371000. * 2. * h.sqrt().clamp(0., 1.).asin()
}
pub fn analyze(
    text: &str,
    fences: &[Vec<(f64, f64)>],
    checkpoints: &[(f64, f64)],
) -> Result<Analysis, String> {
    let samples = route_planner::gpx::parse(text)?;
    let polys: Vec<Vec<Coord>> = fences
        .iter()
        .map(|p| p.iter().map(|&(a, b)| Coord::new(b, a)).collect())
        .collect();
    let mut out = Analysis {
        segments: Vec::new(),
        distance_m: 0.,
        duration_s: None,
        pace_min_km: None,
        coverage: vec![false; checkpoints.len()],
        outside: Vec::new(),
        crossing: Vec::new(),
        warnings: Vec::new(),
    };
    let (mut time_ok, mut total_ms) = (true, 0i64);
    let bd: Vec<_> = samples
        .iter()
        .map(|s| super::geom::wgs84_to_bd09(s.coordinate.lat, s.coordinate.lon))
        .collect();
    for (i, s) in samples.iter().enumerate() {
        let p = (s.coordinate.lat, s.coordinate.lon);
        if i == 0 || samples[i - 1].segment != s.segment {
            out.segments.push(Vec::new());
        }
        out.segments.last_mut().unwrap().push(p);
        if !polys.is_empty() && !boundary::contains(&polys, Coord::new(bd[i].1, bd[i].0)) {
            out.outside.push(p);
        }
        for (j, &cp) in checkpoints.iter().enumerate() {
            if distance(bd[i], cp) <= 40. {
                out.coverage[j] = true;
            }
        }
        if i == 0 || samples[i - 1].segment != s.segment {
            continue;
        }
        let prev = &samples[i - 1];
        let a = (prev.coordinate.lat, prev.coordinate.lon);
        out.distance_m += distance(a, p);
        let parse = |s: &Option<String>| {
            s.as_ref()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|t| t.timestamp_millis())
        };
        match (parse(&prev.time), parse(&s.time)) {
            (Some(x), Some(y)) if y > x => total_ms += y - x,
            _ => time_ok = false,
        }
        if !polys.is_empty()
            && !boundary::segment_inside(
                &polys,
                Coord::new(bd[i - 1].1, bd[i - 1].0),
                Coord::new(bd[i].1, bd[i].0),
            )
        {
            out.crossing.push(vec![a, p]);
        }
        for (j, &cp) in checkpoints.iter().enumerate() {
            let xy = |q: (f64, f64)| {
                [
                    (q.1 - cp.1) * 111320. * cp.0.to_radians().cos(),
                    (q.0 - cp.0) * 111132.,
                ]
            };
            let (q, _) =
                route_planner::graph::project_to_segment([0., 0.], xy(bd[i - 1]), xy(bd[i]));
            if q[0].hypot(q[1]) <= 40. {
                out.coverage[j] = true;
            }
        }
    }
    if time_ok && total_ms > 0 {
        let t = total_ms as f64 / 1000.;
        out.duration_s = Some(t);
        if out.distance_m > 0. {
            out.pace_min_km = Some(t / 60. / (out.distance_m / 1000.));
        }
    } else {
        out.warnings
            .push("时间缺失、重复或倒退，无法计算可信配速".into());
    }
    if polys.is_empty() {
        out.warnings.push("未载入校区围栏，越界状态未知".into());
    }
    if checkpoints.is_empty() {
        out.warnings.push("未载入点位，覆盖状态未知".into());
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gpx_distance_time_segments_and_boundary_are_analyzed_without_connecting_gaps() {
        let xml = r#"<gpx><trk><trkseg><trkpt lat="39.9" lon="116.4"><time>2026-01-01T00:00:00Z</time></trkpt><trkpt lat="39.901" lon="116.4"><time>2026-01-01T00:01:00Z</time></trkpt></trkseg><trkseg><trkpt lat="40" lon="117"/><trkpt lat="40.001" lon="117"/></trkseg></trk></gpx>"#;
        let a = analyze(xml, &[], &[]).unwrap();
        assert_eq!(a.segments.len(), 2);
        assert!((a.distance_m - 222.39).abs() < 0.2);
        assert!(a.pace_min_km.is_none());
        let good = xml.split("</trkseg>").next().unwrap().to_string() + "</trkseg></trk></gpx>";
        let cp = super::super::geom::wgs84_to_bd09(39.9005, 116.4);
        let f = vec![vec![
            (39.899, 116.399),
            (39.899, 116.401),
            (39.9006, 116.401),
            (39.9006, 116.399),
        ]
        .into_iter()
        .map(|p| super::super::geom::wgs84_to_bd09(p.0, p.1))
        .collect()];
        let a = analyze(&good, &f, &[cp]).unwrap();
        assert_eq!(a.duration_s, Some(60.));
        assert!(a.pace_min_km.unwrap() > 8.9);
        assert_eq!(a.coverage, vec![true]);
        assert_eq!(a.outside.len(), 1);
        assert_eq!(a.crossing.len(), 1);
        assert!(analyze("<gpx><trkpt lat='NaN' lon='1'/></gpx>", &[], &[]).is_err());
        assert!(analyze("<gpx><trkpt lat='1' lon='1'>&bad;", &[], &[]).is_err());
    }
}
