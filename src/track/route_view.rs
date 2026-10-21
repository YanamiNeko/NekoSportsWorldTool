use route_planner::Route;
/// Laps are returns to the route start; a final partial lap is retained.
pub fn laps(route: &Route, target: f64) -> Vec<f64> {
    if route.length_m <= 0. || target <= 0. {
        return Vec::new();
    }
    let start = route.points[0];
    let mut cuts = vec![0.];
    let mut base = 0.;
    while base < target {
        for p in route.points.iter().skip(1) {
            let s = base + p.s;
            if s >= target {
                break;
            }
            if (p.lat - start.lat).abs() < 1e-9
                && (p.lon - start.lon).abs() < 1e-9
                && s > *cuts.last().unwrap() + 0.01
            {
                cuts.push(s);
            }
        }
        base += route.length_m;
    }
    cuts.push(target);
    cuts.windows(2).map(|w| w[1] - w[0]).collect()
}
pub fn position(points: &[(f64, f64)], arcs: &[f64], distance: f64) -> Option<(f64, f64)> {
    let total = *arcs.last()?;
    if total <= 0. || points.len() != arcs.len() {
        return None;
    }
    let s = distance.rem_euclid(total);
    let i = arcs.partition_point(|a| *a < s).max(1).min(arcs.len() - 1);
    let f = (s - arcs[i - 1]) / (arcs[i] - arcs[i - 1]).max(1e-12);
    Some((
        points[i - 1].0 + (points[i].0 - points[i - 1].0) * f,
        points[i - 1].1 + (points[i].1 - points[i - 1].1) * f,
    ))
}
#[cfg(test)]
mod tests {
    #[test]
    fn full_and_partial_laps_sum_to_requested_distance() {
        let route = route_planner::Route {
            length_m: 100.,
            points: vec![
                route_planner::RoutePoint {
                    lat: 0.,
                    lon: 0.,
                    s: 0.,
                    curvature: 0.,
                },
                route_planner::RoutePoint {
                    lat: 1.,
                    lon: 1.,
                    s: 50.,
                    curvature: 0.,
                },
                route_planner::RoutePoint {
                    lat: 0.,
                    lon: 0.,
                    s: 100.,
                    curvature: 0.,
                },
            ],
        };
        assert_eq!(super::laps(&route, 250.), vec![100., 100., 50.]);
        assert_eq!(
            super::position(&[(0., 0.), (1., 1.), (0., 0.)], &[0., 50., 100.], 125.),
            Some((0.5, 0.5))
        );
    }
}
