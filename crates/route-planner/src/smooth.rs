//! 曲率平滑：路口/转弯节点圆弧切角 + 曲率 k(s) 重采样。
//!
//! 弃用全局 Catmull-Rom；直线段保留，锐角处用等半径圆弧（回旋线近似，
//! 曲率恒定过渡）切角，得到连续折线 + 逐点曲率。

use crate::graph::{Coord, RoadGraph};

/// 平滑后的空间采样点（度系 + 弧长 + 曲率）。
#[derive(Clone, Copy, Debug)]
pub struct RoutePoint {
    pub lon: f64,
    pub lat: f64,
    /// 累计弧长（米）。
    pub s: f64,
    /// 曲率（1/米，直线为 0）。
    pub curvature: f64,
}

fn normalize(v: [f64; 2]) -> [f64; 2] {
    let n = (v[0] * v[0] + v[1] * v[1]).sqrt();
    if n <= 1e-12 {
        [0.0, 0.0]
    } else {
        [v[0] / n, v[1] / n]
    }
}

fn len2(a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    dx * dx + dy * dy
}

/// 圆弧切角：返回 (切点1, 切点2, 圆心, 有向转角)，None 表示转角过缓不切。
fn corner_fillet(
    a: [f64; 2],
    b: [f64; 2],
    c: [f64; 2],
    radius: f64,
    threshold_rad: f64,
) -> Option<([f64; 2], [f64; 2], [f64; 2], f64)> {
    let dir_in = normalize([b[0] - a[0], b[1] - a[1]]);
    let dir_out = normalize([c[0] - b[0], c[1] - b[1]]);
    if dir_in == [0.0, 0.0] || dir_out == [0.0, 0.0] {
        return None;
    }
    let cross = dir_in[0] * dir_out[1] - dir_in[1] * dir_out[0];
    let dot = dir_in[0] * dir_out[0] + dir_in[1] * dir_out[1];
    let delta = cross.atan2(dot);
    if delta.abs() < threshold_rad {
        return None;
    }
    let d = (radius * (delta.abs() / 2.0).tan())
        .min((len2(a, b).sqrt()) / 2.0)
        .min((len2(b, c).sqrt()) / 2.0);
    if d <= 0.0 {
        return None;
    }
    let cut1 = [b[0] - dir_in[0] * d, b[1] - dir_in[1] * d];
    let cut2 = [b[0] + dir_out[0] * d, b[1] + dir_out[1] * d];
    let sign = if delta >= 0.0 { 1.0 } else { -1.0 };
    // 内法线：dir_in 旋转 sign*90°
    let normal = [-dir_in[1] * sign, dir_in[0] * sign];
    let center = [cut1[0] + normal[0] * radius, cut1[1] + normal[1] * radius];
    Some((cut1, cut2, center, delta))
}

struct Smoothed {
    pts: Vec<[f64; 2]>,
    k: Vec<f64>,
}

fn push_line(out: &mut Smoothed, from: [f64; 2], to: [f64; 2]) {
    if from != to {
        out.pts.push(to);
        out.k.push(0.0);
    }
}

fn smooth_polyline(pts: &[[f64; 2]], radius: f64, threshold_rad: f64) -> Smoothed {
    let mut out = Smoothed { pts: vec![], k: vec![] };
    if pts.len() < 3 {
        for &p in pts {
            out.pts.push(p);
            out.k.push(0.0);
        }
        return out;
    }
    out.pts.push(pts[0]);
    out.k.push(0.0);
    let mut cursor = pts[0];
    let n = pts.len();
    for i in 1..n - 1 {
        let (a, b, c) = (pts[i - 1], pts[i], pts[i + 1]);
        match corner_fillet(a, b, c, radius, threshold_rad) {
            Some((cut1, cut2, center, delta)) => {
                push_line(&mut out, cursor, cut1);
                // 采样圆弧
                let a1 = (cut1[1] - center[1]).atan2(cut1[0] - center[0]);
                let steps = ((radius * delta.abs() / 0.5).ceil() as usize).max(2);
                for j in 1..=steps {
                    let ang = a1 + delta * (j as f64 / steps as f64);
                    let p = [center[0] + radius * ang.cos(), center[1] + radius * ang.sin()];
                    out.pts.push(p);
                    out.k.push(1.0 / radius);
                }
                cursor = cut2;
            }
            None => {
                push_line(&mut out, cursor, b);
                cursor = b;
            }
        }
    }
    push_line(&mut out, cursor, pts[n - 1]);
    out
}

/// 平滑 + 按步长重采样，返回度系 RoutePoint 序列。
pub fn smooth_and_sample(
    g: &RoadGraph,
    coords: &[Coord],
    radius: f64,
    threshold_deg: f64,
    step_m: f64,
) -> Vec<RoutePoint> {
    let pts_m: Vec<[f64; 2]> = coords.iter().map(|c| g.to_m(*c)).collect();
    let sm = smooth_polyline(&pts_m, radius, threshold_deg.to_radians());

    // 累计弧长
    let mut cum = vec![0.0f64];
    for w in sm.pts.windows(2) {
        let d = ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt();
        cum.push(cum[cum.len() - 1] + d);
    }
    let total = *cum.last().unwrap_or(&0.0);
    if total <= 0.0 {
        return Vec::new();
    }

    let mut out: Vec<RoutePoint> = Vec::new();
    let mut i = 0usize;
    let mut s = 0.0f64;
    while i + 1 < sm.pts.len() {
        let seg = cum[i + 1] - cum[i];
        let seg_end = cum[i + 1];
        // 在本段内以 step 前进
        while s < seg_end - 1e-9 {
            let t = if seg > 0.0 { (s - cum[i]) / seg } else { 0.0 };
            let x = sm.pts[i][0] + (sm.pts[i + 1][0] - sm.pts[i][0]) * t;
            let y = sm.pts[i][1] + (sm.pts[i + 1][1] - sm.pts[i][1]) * t;
            let k = sm.k[i] + (sm.k[i + 1] - sm.k[i]) * t;
            let c = g.to_deg([x, y]);
            out.push(RoutePoint { lon: c.lon, lat: c.lat, s, curvature: k });
            s += step_m;
            if s > total {
                s = total;
            }
        }
        i += 1;
    }
    // 终点
    let last_m = *sm.pts.last().unwrap();
    let c = g.to_deg(last_m);
    out.push(RoutePoint { lon: c.lon, lat: c.lat, s: total, curvature: 0.0 });
    out
}

/// 弯道限速：sqrt(a_c_max / k)，直线（k≈0）返回无穷大。
pub fn turn_speed_limit(a_c_max: f64, k: f64) -> f64 {
    if k <= 1e-9 {
        f64::INFINITY
    } else {
        (a_c_max / k).sqrt()
    }
}
