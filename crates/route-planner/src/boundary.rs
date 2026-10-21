//! Inclusive polygon containment and whole-segment checks (not spaced sampling).
use crate::{point_in_polygon, Coord};

const EPS: f64 = 1e-10;
fn cross(a: (f64, f64), b: (f64, f64)) -> f64 {
    a.0 * b.1 - a.1 * b.0
}
fn delta(a: Coord, b: Coord) -> (f64, f64) {
    (b.lon - a.lon, b.lat - a.lat)
}
fn at(a: Coord, b: Coord, t: f64) -> Coord {
    Coord::new(a.lon + (b.lon - a.lon) * t, a.lat + (b.lat - a.lat) * t)
}
fn on_edge(a: Coord, b: Coord, p: Coord) -> bool {
    let d = delta(a, b);
    let q = delta(a, p);
    let length = d.0.hypot(d.1);
    if length < EPS {
        return q.0.hypot(q.1) < EPS;
    }
    cross(d, q).abs() <= EPS * length
        && p.lon >= a.lon.min(b.lon) - EPS
        && p.lon <= a.lon.max(b.lon) + EPS
        && p.lat >= a.lat.min(b.lat) - EPS
        && p.lat <= a.lat.max(b.lat) + EPS
}
pub fn contains(polys: &[Vec<Coord>], p: Coord) -> bool {
    p.lon.is_finite()
        && p.lat.is_finite()
        && polys.iter().any(|poly| {
            poly.len() >= 3
                && (point_in_polygon(poly, p)
                    || poly
                        .iter()
                        .zip(poly.iter().cycle().skip(1))
                        .any(|(&a, &b)| on_edge(a, b, p)))
        })
}
/// Split at every boundary intersection, then test every open interval.
/// This catches arbitrarily narrow notches and gaps between separate campuses.
pub fn segment_inside(polys: &[Vec<Coord>], a: Coord, b: Coord) -> bool {
    if !contains(polys, a) || !contains(polys, b) {
        return false;
    }
    let r = delta(a, b);
    let len2 = r.0 * r.0 + r.1 * r.1;
    if len2 < EPS * EPS {
        return true;
    }
    let mut cuts = vec![0.0, 1.0];
    for poly in polys {
        for (&c, &d) in poly.iter().zip(poly.iter().cycle().skip(1)) {
            let s = delta(c, d);
            let q = delta(a, c);
            let den = cross(r, s);
            if den.abs() > 1e-20 {
                let t = cross(q, s) / den;
                let u = cross(q, r) / den;
                if (0.0..=1.0).contains(&t) && (-EPS..=1.0 + EPS).contains(&u) {
                    cuts.push(t);
                }
            } else {
                for p in [c, d] {
                    if on_edge(a, b, p) {
                        let v = delta(a, p);
                        cuts.push(((v.0 * r.0 + v.1 * r.1) / len2).clamp(0.0, 1.0));
                    }
                }
            }
        }
    }
    cuts.sort_by(f64::total_cmp);
    cuts.windows(2)
        .all(|w| contains(polys, at(a, b, (w[0] + w[1]) * 0.5)))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn c(x: f64, y: f64) -> Coord {
        Coord::new(x, y)
    }
    #[test]
    fn catches_narrow_notch_and_includes_boundary() {
        let p = vec![vec![
            c(0., 0.),
            c(1., 0.),
            c(1., 1.),
            c(0.56, 1.),
            c(0.56, 0.2),
            c(0.55, 0.2),
            c(0.55, 1.),
            c(0., 1.),
        ]];
        assert!(!segment_inside(&p, c(0.1, 0.5), c(0.9, 0.5)));
        assert!(segment_inside(&p, c(0., 0.), c(1., 0.)));
        assert!(segment_inside(&p, c(0.1, 0.1), c(0.9, 0.1)));
    }
    #[test]
    fn disjoint_campuses_cannot_be_joined() {
        let p = vec![
            vec![c(0., 0.), c(1., 0.), c(1., 1.), c(0., 1.)],
            vec![c(2., 0.), c(3., 0.), c(3., 1.), c(2., 1.)],
        ];
        assert!(!segment_inside(&p, c(0.5, 0.5), c(2.5, 0.5)));
    }
}
