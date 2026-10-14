//! 自定义路线轨迹生成器（模式 C）。
//!
//! 几何源为高德步行路线（gcj02 → BD-09 对齐打卡点后走点）：
//! - 单程：A→B 一条腿；
//! - 绕圈：A→B→A 去程+返程合成自然闭环（返程取高德 B→A 路线，失败时回退去程反向）；
//! - 圈数：0 = 按目标距离自动（走点引擎自动绕环，含不满一圈的零头）；>0 = 精确圈数。
//!
//! 速度剖面 / 哨兵点 / 断崖 / 10s 窗 / 点位吸附与经典模式完全一致（复用 speed_profile）。
//! 起点固定为路线起点（不随机偏移弧长），保证「从哪里跑」即用户填的起点。

#![allow(non_snake_case)]

use super::generator::speed_profile;
use super::geom::{fmt_gain_time, ring_point_at, round_to, to_bd, Rng, MET_PER_DEG_LAT, MET_PER_DEG_LNG};
use super::model::{GenPoint, Segment, TenWindow, Track};
use super::postfix::apply_post_fixes;

/// 自定义路线参数（GUI 跑步页 / CLI run 共用）。
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CustomRouteSpec {
    /// 起点：地址文本或坐标（支持 经度,纬度 / 纬度,经度 自动识别）。
    pub origin: String,
    /// 终点：同上。
    pub destination: String,
    /// true = 绕圈（A→B→A 闭环）；false = 单程 A→B。
    pub loop_mode: bool,
    /// 圈数：0 = 按目标距离自动；>0 = 精确圈数（距离=圈数×闭环长度）。
    pub laps: i64,
}

impl Default for CustomRouteSpec {
    fn default() -> Self {
        Self {
            origin: String::new(),
            destination: String::new(),
            loop_mode: true,
            laps: 0,
        }
    }
}

impl CustomRouteSpec {
    pub fn is_valid(&self) -> bool {
        !self.origin.trim().is_empty() && !self.destination.trim().is_empty()
    }
}

/// 解析好的闭环路线（BD-09，与打卡点同系）。
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct CustomRing {
    /// 环点（BD-09）；绕圈模式为 A→B→A 闭环，单程模式为 A→B 开链。
    pub points_bd: Vec<(f64, f64)>,
    /// 去程 A→B 长度（米）。
    pub leg_length_m: f64,
    /// 闭环总长（米）：绕圈 = 去程+返程；单程 = 去程。
    pub total_length_m: f64,
    /// 是否绕圈闭环。
    pub loop_mode: bool,
}

/// 手绘路线存档（drawn_route.json，GCJ-02，与高德瓦片同基准）。
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DrawnRouteFile {
    /// 途经点（GCJ-02，lat, lng）。
    pub points_gcj: Vec<(f64, f64)>,
    /// 是否首尾闭合（绕圈）。
    pub auto_close: bool,
}

fn drawn_route_path() -> std::path::PathBuf {
    crate::platform::data_dir().join("drawn_route.json")
}

pub fn save_drawn_route(points_gcj: &[(f64, f64)], auto_close: bool) -> Result<(), String> {
    if points_gcj.len() < 2 {
        return Err("手绘路线至少需要 2 个点".into());
    }
    let doc = DrawnRouteFile {
        points_gcj: points_gcj.to_vec(),
        auto_close,
    };
    let json = serde_json::to_string_pretty(&doc).map_err(|e| format!("序列化失败: {e}"))?;
    std::fs::write(drawn_route_path(), json).map_err(|e| format!("写入失败: {e}"))
}

pub fn load_drawn_route() -> Option<DrawnRouteFile> {
    let text = std::fs::read_to_string(drawn_route_path()).ok()?;
    let doc: DrawnRouteFile = serde_json::from_str(&text).ok()?;
    if doc.points_gcj.len() < 2 {
        return None;
    }
    Some(doc)
}

/// 解析导入的轨迹 CSV：`时间, X, Y, Z`（X/Y/Z 为 ECEF 地心地固坐标，米）。
/// 转换为 GCJ-02（与高德瓦片同基准），并降采样到 max_pts 个点。
pub fn parse_ecef_csv(text: &str, max_pts: usize) -> Result<Vec<(f64, f64)>, String> {
    let mut pts: Vec<(f64, f64)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        // 支持 3 列（x,y,z）或 4 列（time,x,y,z）
        let (x, y, z) = if parts.len() >= 4 {
            (
                parts[1].parse::<f64>().map_err(|e| format!("X 解析失败: {e}"))?,
                parts[2].parse::<f64>().map_err(|e| format!("Y 解析失败: {e}"))?,
                parts[3].parse::<f64>().map_err(|e| format!("Z 解析失败: {e}"))?,
            )
        } else if parts.len() == 3 {
            (
                parts[0].parse::<f64>().map_err(|e| format!("X 解析失败: {e}"))?,
                parts[1].parse::<f64>().map_err(|e| format!("Y 解析失败: {e}"))?,
                parts[2].parse::<f64>().map_err(|e| format!("Z 解析失败: {e}"))?,
            )
        } else {
            continue;
        };
        if x.abs() > 1.0e7 || y.abs() > 1.0e7 || z.abs() > 1.0e7 {
            continue; // 明显越界行
        }
        let (lat, lon) = crate::track::geom::ecef_to_wgs84(x, y, z);
        let (glat, glon) = crate::track::geom::wgs84_to_gcj02(lat, lon);
        // 与上一点距离过近（<0.5 米）则跳过，避免重复点
        if let Some(&(pla, plo)) = pts.last() {
            let dlat = (glat - pla) * crate::track::geom::MET_PER_DEG_LAT;
            let dlng = (glon - plo) * crate::track::geom::MET_PER_DEG_LNG;
            if (dlat * dlat + dlng * dlng).sqrt() < 0.5 {
                continue;
            }
        }
        pts.push((glat, glon));
    }
    if pts.len() < 2 {
        return Err("有效轨迹点不足 2 个".into());
    }
    // 降采样
    if pts.len() > max_pts {
        let step = pts.len() as f64 / max_pts as f64;
        let mut out: Vec<(f64, f64)> = Vec::with_capacity(max_pts);
        let mut i = 0.0f64;
        while (i as usize) < pts.len() && out.len() < max_pts {
            out.push(pts[i as usize]);
            i += step;
        }
        if let Some(last) = pts.last() {
            if out.last() != Some(last) {
                out.push(*last);
            }
        }
        pts = out;
    }
    Ok(pts)
}

/// 由 GCJ-02 标定点构建平滑闭环（BD-09）：标定点经 Catmull-Rom 拟合成自然曲线；
/// auto_close 时首尾相接成环，开链按单程处理。
pub fn ring_from_gcj_polyline(
    points_gcj: &[(f64, f64)],
    auto_close: bool,
) -> Result<CustomRing, String> {
    if points_gcj.len() < 2 {
        return Err("手绘路线至少需要 2 个点".into());
    }
    let raw_bd: Vec<(f64, f64)> = points_gcj
        .iter()
        .map(|&(la, lo)| crate::track::geom::gcj02_to_bd09(la, lo))
        .collect();
    // 平滑：每段 12 采样，闭环时补首点闭合
    let mut pts = crate::track::geom::smooth_polyline(&raw_bd, 12, auto_close);
    if auto_close && pts.len() > 2 {
        let first = pts[0];
        let last = *pts.last().unwrap();
        if (first.0 - last.0).abs() > 1e-9 || (first.1 - last.1).abs() > 1e-9 {
            pts.push(first);
        }
    }
    let mut length = 0.0;
    for w in pts.windows(2) {
        let dlat = (w[0].0 - w[1].0) * crate::track::geom::MET_PER_DEG_LAT;
        let dlng = (w[0].1 - w[1].1) * crate::track::geom::MET_PER_DEG_LNG;
        length += (dlat * dlat + dlng * dlng).sqrt();
    }
    if length <= 0.0 {
        return Err("手绘路线长度无效（0 米）".into());
    }
    Ok(CustomRing {
        points_bd: pts,
        leg_length_m: length,
        total_length_m: length,
        loop_mode: auto_close,
    })
}

/// 按绕圈/单程/圈数计算本次实际距离与时长，返回 (距离, 时长, 说明日志)。
///
/// 平均速度沿用调用方已校正到有效配速窗口内的 dist/dur 比值，
/// 只对距离做裁剪/圈数换算，时长等比换算。
pub fn effective_dist_dur(
    spec: &CustomRouteSpec,
    ring: &CustomRing,
    dist: f64,
    dur: i64,
) -> Result<(f64, i64, String), String> {
    if ring.total_length_m <= 0.0 {
        return Err("路线长度无效（0 米）".into());
    }
    let avg_v = dist / (dur.max(1) as f64);
    if avg_v <= 0.0 {
        return Err("平均速度无效".into());
    }
    let (d, note) = if spec.loop_mode {
        if spec.laps > 0 {
            (
                spec.laps as f64 * ring.total_length_m,
                format!(
                    "绕圈 {:.0} m × {} 圈 = {:.0} m",
                    ring.total_length_m,
                    spec.laps,
                    spec.laps as f64 * ring.total_length_m
                ),
            )
        } else {
            (
                dist,
                format!(
                    "绕圈自动（{:.0} m/圈，目标 {:.0} m ≈ {:.1} 圈）",
                    ring.total_length_m,
                    dist,
                    dist / ring.total_length_m
                ),
            )
        }
    } else if dist > ring.total_length_m {
        (
            ring.total_length_m,
            format!(
                "单程路线 {:.0} m < 目标 {:.0} m，按路线全长提交",
                ring.total_length_m, dist
            ),
        )
    } else {
        (dist, format!("单程 {:.0} m", dist))
    };
    let dur_new = (d / avg_v).round() as i64;
    Ok((d, dur_new, note))
}

/// 模式 C：自定义路线轨迹生成。
///
/// `snap_points_bd` 为打卡点（BD-09），与路线距离 <40m 的打卡点会被精确吸附。
/// 走点引擎与经典模式一致：起点固定 s=0（用户填的起点），方向固定 +1。
pub fn build_custom(
    dist: f64,
    dur: i64,
    seed: u64,
    start_ms: i64,
    ring: &CustomRing,
    snap_points_bd: &[(f64, f64)],
) -> Track {
    let mut rng = Rng::new(seed);
    let dur_f = dur as f64;

    // 环 → 平面折线 + 弧长表（闭合：末点补回首点，供 ring_point_at 无缝绕环）
    let n_pts = ring.points_bd.len().max(1);
    let (c_lat, c_lng) = (
        ring.points_bd.iter().map(|q| q.0).sum::<f64>() / n_pts as f64,
        ring.points_bd.iter().map(|q| q.1).sum::<f64>() / n_pts as f64,
    );
    let mut dense: Vec<(f64, f64)> = ring
        .points_bd
        .iter()
        .map(|&(la, lo)| ((lo - c_lng) * MET_PER_DEG_LNG, (la - c_lat) * MET_PER_DEG_LAT))
        .collect();
    let closed = dense.len() < 2
        || ((dense[0].0 - dense[dense.len() - 1].0).abs() < 1e-9
            && (dense[0].1 - dense[dense.len() - 1].1).abs() < 1e-9);
    if dense.len() >= 2 && !closed {
        dense.push(dense[0]);
    }
    let mut arcs = vec![0.0f64];
    for i in 1..dense.len() {
        let a = dense[i - 1];
        let b = dense[i];
        arcs.push(arcs[i - 1] + ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt());
    }
    let s0 = 0.0f64;
    let direction = 1.0f64;

    let (times, dts, w) = speed_profile(dist, dur, &mut rng);
    let n = times.len();
    let seg_dist: Vec<f64> = (0..n).map(|i| w[i] * dts[i]).collect();
    let speeds: Vec<f64> = w.clone();
    let phase_l = rng.uniform(0.0, std::f64::consts::TAU);

    // 点位类型（同经典模式）
    let tl_w = [((-1i64, 4i64), 54u32), ((-1, 1), 27), ((-1, 12), 13), ((-1, 5), 3), ((-1, 6), 2)];
    let mut kinds: Vec<(i64, i64)> = Vec::with_capacity(n);
    for _ in 0..n {
        let u = rng.random();
        if u < 0.39 {
            kinds.push((3, 1));
        } else if u < 0.93 {
            kinds.push((0, 1));
        } else if u < 0.96 {
            kinds.push(rng.weighted(&tl_w));
        } else {
            kinds.push((rng.choice(&[1, 1, 1, 1, 2, 2]), 1));
        }
    }
    for i in 1..kinds.len() {
        let prev_drift = (-1 == kinds[i - 1].0) || kinds[i - 1].0 == 5 || kinds[i - 1].0 == 6;
        if kinds[i].0 != -1 && prev_drift && rng.random() < 0.08 {
            kinds[i] = (if rng.random() < 0.75 { 7 } else { 8 }, 1);
        }
    }
    for i in 1..n {
        if kinds[i].0 == -1 && kinds[i - 1].0 == -1 {
            kinds[i] = (rng.choice(&[3, 0]), 1);
        }
    }
    let normal_idx: Vec<usize> = (0..n).filter(|&i| kinds[i].0 != -1 && i > 1).collect();
    let share: f64 = normal_idx.iter().map(|&i| seg_dist[i]).sum();
    let share = if share == 0.0 { 1.0 } else { share };
    let mut disp_of = vec![0.0f64; n];
    for &i in &normal_idx {
        disp_of[i] = seg_dist[i] * (dist / share);
    }

    let mut locs: Vec<GenPoint> = Vec::with_capacity(n);
    let mut s = s0;
    let mut t_acc = 0.0f64;
    let mut dist_acc = 0.0f64;
    let mut steps_acc = 0.0f64;
    let mut jx = 0.0f64;
    let mut jy = 0.0f64;
    let mut alt = 82.0 + rng.uniform(-1.0, 1.0);
    let n_est = 1.max((dur_f / 5.0) as i64);
    let alt_sigma = rng.uniform(3.8, 6.2) / (0.40 * n_est as f64);
    let mut ten_t = 0.0f64;
    let mut ten_d = 0.0f64;
    let mut ten_st = 0.0f64;
    let mut ten_speed: Vec<TenWindow> = Vec::new();
    let mut ten_steps: Vec<TenWindow> = Vec::new();

    for i in 0..n {
        let dt = dts[i];
        let (typ, lt) = kinds[i];
        t_acc += dt;
        let pos = |ss: f64| ring_point_at(&dense, &arcs, ss);
        let mut d_step = 0.0f64;
        let px;
        let py;
        let x;
        let y;
        let rad;
        let state;
        if typ != -1 {
            d_step = disp_of[i];
            s += direction * d_step;
            let (bx, by) = pos(s);
            x = bx;
            y = by;
            jx = 0.72 * jx + rng.gauss(0.0, 0.75);
            jy = 0.72 * jy + rng.gauss(0.0, 0.75);
            px = bx + jx;
            py = by + jy;
            rad = round_to(
                if typ == 3 { rng.uniform(1.4, 5.1) } else { rng.uniform(1.4, 2.4) },
                2,
            );
            state = if typ == 0 {
                rng.weighted(&[(1, 145), (2, 45), (3, 164)])
            } else {
                rng.weighted(&[(1, 145), (2, 256), (3, 151)])
            };
        } else {
            match lt {
                4 => {
                    if rng.random() >= 0.68 {
                        d_step = if rng.random() < 0.95 { rng.uniform(2.0, 60.0) } else { rng.uniform(60.0, 250.0) };
                    }
                }
                1 => {
                    if rng.random() >= 0.83 {
                        d_step = rng.uniform(0.5, 36.0);
                    }
                }
                12 => {
                    d_step = if rng.random() < 0.9 { rng.uniform(5.0, 80.0) } else { rng.uniform(80.0, 220.0) };
                }
                5 => d_step = rng.uniform(5.0, 60.0),
                _ => d_step = rng.uniform(100.0, 300.0),
            }
            let (bx, by) = pos(s);
            x = bx;
            y = by;
            if d_step > 0.0 {
                let ang = rng.uniform(0.0, std::f64::consts::TAU);
                px = x + d_step * ang.sin();
                py = y + d_step * ang.cos();
            } else if let Some(last) = locs.last() {
                // 零位移：精确复制上一点坐标（BD→平面）
                py = (last.gLat - c_lat) * MET_PER_DEG_LAT;
                px = (last.gLng - c_lng) * MET_PER_DEG_LNG;
            } else {
                px = x + jx;
                py = y + jy;
            }
            rad = if lt == 4 {
                round_to(if rng.random() < 0.75 { rng.uniform(30.0, 100.0) } else { rng.uniform(100.0, 550.0) }, 2)
            } else if lt == 1 {
                round_to(if rng.random() < 0.75 { rng.uniform(1.6, 12.0) } else { rng.uniform(12.0, 95.0) }, 2)
            } else if lt == 12 {
                round_to(rng.uniform(30.0, 125.0), 2)
            } else if lt == 5 {
                round_to(rng.uniform(25.0, 300.0), 2)
            } else {
                550.0
            };
            state = rng.weighted(&[(1, 102), (2, 124), (3, 136)]);
        }
        if typ != -1 {
            dist_acc += d_step; // 异常点漂移不计入累计距离
        }
        let (lat, lng) = to_bd(px, py, c_lat, c_lng);
        alt += 0.04 * (82.0 - alt) + rng.gauss(0.0, alt_sigma);
        let v_now = dist / dur_f;
        let stride_target = 0.62 + 0.17 * v_now
            + 0.03 * (std::f64::consts::TAU * t_acc / 200.0 + phase_l).sin()
            + rng.gauss(0.0, 0.008);
        let v_cad = v_now * (1.0 + 0.03 * (std::f64::consts::TAU * t_acc / 70.0 + phase_l).sin());
        let cad = (v_cad / stride_target * 60.0).clamp(100.0, 200.0);
        steps_acc += cad / 60.0 * dt;
        let nxt = pos(s + direction * 2.0);
        let brg = ((nxt.0 - x).atan2(nxt.1 - y).to_degrees() + rng.gauss(0.0, 35.0)).rem_euclid(360.0);
        // 异常点（-1）：avgSpeed 为累计均值；GPS 瞬时速度多为低速，偶发漂移尖峰
        let (avg_sp, gps_speed) = if typ == -1 {
            let avg = round_to(dist_acc / t_acc.max(1.0), 4);
            let gps = if rng.random() < 0.12 {
                rng.uniform(15.0, 46.0)
            } else {
                rng.uniform(0.5, 6.0)
            };
            (avg, round_to(gps, 4))
        } else {
            let avg = round_to(d_step / dt, 4);
            let kmh = avg * 3.6;
            let sigma = (kmh * 0.08).max(0.05);
            let gps = if rng.random() < 0.20 {
                0.0
            } else {
                round_to((kmh + rng.gauss(0.0, sigma)).max(0.0), 4)
            };
            (avg, gps)
        };

        ten_t += dt;
        ten_d += speeds[i] * dt;
        ten_st += cad / 60.0 * dt;
        while ten_t >= 10.0 {
            let k = 10.0 / ten_t; // 只取前 10s 的量，余量留给下一窗
            let (out_d, out_st) = (ten_d * k, ten_st * k);
            ten_speed.push(TenWindow { time: 10, value: round_to(out_d, 2) });
            ten_steps.push(TenWindow { time: 10, value: round_to(out_st, 0) });
            ten_t -= 10.0;
            ten_d -= out_d;
            ten_st -= out_st;
        }
        locs.push(GenPoint {
            id: i as i64 + 1,
            flag: start_ms,
            lat: -1.0,
            lng: -1.0,
            gLat: round_to(lat, 7),
            gLng: round_to(lng, 7),
            speed: round_to(gps_speed, 4),
            avgSpeed: avg_sp,
            radius: rad,
            accuracy: rad,
            ptype: typ,
            locType: lt,
            hasAltitude: true,
            totalTime: round_to(t_acc, 0) as i64,
            totalDis: round_to(dist_acc, 4),
            validDis: round_to(dist_acc, 4),
            validTime: round_to(t_acc, 0) as i64,
            steps: steps_acc as i64,
            stepDistance: 0.0,
            gainTime: fmt_gain_time(start_ms + (t_acc * 1000.0) as i64),
            gainTimeMs: start_ms + (t_acc * 1000.0) as i64,
            queueNum: 0,
            coorType: "gcj02".into(),
            bdA: round_to(alt, 2),
            bdD: round_to(brg, 2),
            bdS: round_to((avg_sp * rng.uniform(0.6, 0.95)).max(0.0), 3),
            bdG: rng.choice(&[1, 1, 1, -1]),
            count: rng.randint(20, 88),
            dtr: 0.0,
            state,
            locationId: String::new(),
        });
    }
    if ten_t > 1.0 {
        // 尾窗：限制放大倍数 ≤1.4
        let k = (10.0 / ten_t).min(1.4);
        ten_speed.push(TenWindow { time: 10, value: round_to(ten_d * k, 2) });
        ten_steps.push(TenWindow { time: 10, value: round_to(ten_st * k, 0) });
    }

    let mut segments: Vec<Segment> = Vec::new();
    let (mut seg_t, mut seg_d) = (0.0f64, 0.0f64);
    let mut seg_v: Vec<f64> = Vec::new();
    for i in 0..n {
        seg_t += dts[i];
        seg_d += seg_dist[i];
        seg_v.push(speeds[i]);
        if seg_t >= 60.0 || i == n - 1 {
            segments.push(Segment {
                totalTime: round_to(seg_t, 0) as i64,
                distance: round_to(seg_d, 0) as i64,
                startTime: round_to((times[i] - seg_t) * 1000.0, 0) as i64,
                endTime: round_to(times[i] * 1000.0, 0) as i64,
                avgSpeed: round_to(seg_v.iter().sum::<f64>() / seg_v.len() as f64, 3),
                avgStep: round_to(steps_acc / 1.0f64.max(t_acc) * 60.0, 0) as i64,
                state: 0,
            });
            seg_t = 0.0;
            seg_d = 0.0;
            seg_v.clear();
        }
    }

    apply_post_fixes(&mut locs, &mut rng, start_ms);

    // 点位吸附：<40m 精确落位（路线经过的打卡点才会被吸附，其余保持不变）
    for pl in snap_points_bd {
        let mut best_i = None;
        let mut best_d = 1e18f64;
        for (i, q) in locs.iter().enumerate() {
            let dd = ((q.gLat - pl.0) * MET_PER_DEG_LAT).powi(2)
                + ((q.gLng - pl.1) * MET_PER_DEG_LNG).powi(2);
            if dd < best_d {
                best_d = dd;
                best_i = Some(i);
            }
        }
        if let Some(i) = best_i {
            if best_d < 40.0 * 40.0 {
                locs[i].gLat = round_to(pl.0, 7);
                locs[i].gLng = round_to(pl.1, 7);
            }
        }
    }

    let total_dis = round_to(dist, 3);
    Track {
        totalTime: round_to(t_acc, 0) as i64,
        totalDistance: total_dis,
        validDistance: total_dis,
        validTime: round_to(t_acc, 0) as i64,
        startTime: start_ms,
        startLatitude: locs[0].gLat,
        startLongitude: locs[0].gLng,
        totalSteps: steps_acc as i64,
        locations: locs,
        speedPerTenSec: ten_speed,
        stepsPerTenSec: ten_steps,
        segments,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ECEF 反解应落在海口区域（CSV 首行真实值）。
    #[test]
    fn ecef_csv_first_point_is_haikou() {
        let (lat, lon) = crate::track::geom::ecef_to_wgs84(
            -2081798.439,
            5620358.051,
            2174073.353,
        );
        // 海口/海南大学一带：约 20.0°N, 110.3°E
        assert!((lat - 20.0).abs() < 1.0, "lat={lat}");
        assert!((lon - 110.3).abs() < 1.0, "lon={lon}");
    }

    /// 解析 ECEF CSV 并转 GCJ-02：点数合理、坐标连续无跳变。
    #[test]
    fn parse_ecef_csv_works() {
        let text = "\
0.0,-2081798.439, 5620358.051, 2174073.353
0.1,-2081798.222, 5620358.131, 2174073.354
0.2,-2081798.004, 5620358.211, 2174073.356
0.3,-2081797.787, 5620358.291, 2174073.357
";
        let pts = parse_ecef_csv(text, 600).unwrap();
        assert!(pts.len() >= 2);
        // 相邻点距离应远小于 1km（平滑轨迹）
        for w in pts.windows(2) {
            let dlat = (w[1].0 - w[0].0) * crate::track::geom::MET_PER_DEG_LAT;
            let dlng = (w[1].1 - w[0].1) * crate::track::geom::MET_PER_DEG_LNG;
            let d = (dlat * dlat + dlng * dlng).sqrt();
            assert!(d < 1000.0, "相邻点距离 {d}m 过大");
        }
    }

    fn sample_ring() -> CustomRing {
        // 校园内一个 1km 左右的闭环（BD-09，与打卡点同系）
        let c = (38.90f64, 121.54f64);
        let mut pts = Vec::new();
        for k in 0..40 {
            let a = k as f64 / 40.0 * std::f64::consts::TAU;
            pts.push((
                c.0 + 0.0045 * a.sin(),
                c.1 + 0.0045 * a.cos(),
            ));
        }
        CustomRing {
            leg_length_m: 500.0,
            total_length_m: 1000.0,
            points_bd: pts,
            loop_mode: true,
        }
    }

    fn sample_checkpoints() -> Vec<(f64, f64)> {
        vec![(38.900678, 121.540241), (38.902564, 121.541233)]
    }

    /// 距离精确、结构合法（哨兵/断崖/10s 窗），闭环总距与目标一致。
    #[test]
    fn test_build_custom_structure() {
        let ring = sample_ring();
        let track = build_custom(2200.0, 900, 7, 1_788_958_186_123, &ring, &sample_checkpoints());
        assert!((track.totalDistance - 2200.0).abs() < 2.0);
        assert_eq!(track.totalTime, 900);
        assert!(!track.locations.is_empty());
        assert!([0, 7].contains(&track.locations[0].ptype));
        assert_eq!(track.locations[1].ptype, 5);
        assert_eq!(track.locations.last().unwrap().ptype, 6);
        assert!(track.totalSteps > 500);
        // 累计距离单调不减
        let mut prev = 0.0;
        for p in &track.locations {
            assert!(p.totalDis >= prev - 1e-6);
            prev = p.totalDis;
        }
    }

    /// 起点必须是环首点（用户填的起点），不能随机偏移。
    #[test]
    fn test_start_is_ring_origin() {
        let ring = sample_ring();
        let track = build_custom(1200.0, 480, 3, 1_788_958_186_123, &ring, &[]);
        let first = &track.locations[0];
        let (want_lat, want_lng) = ring.points_bd[0];
        let d = ((first.gLat - want_lat) * MET_PER_DEG_LAT).powi(2)
            + ((first.gLng - want_lng) * MET_PER_DEG_LNG).powi(2);
        // 首点带 GPS 抖动（±1m 量级），放宽到 30m 断言
        assert!(d.sqrt() < 30.0, "起点偏离路线起点 {:.1} m", d.sqrt());
    }

    /// 圈数换算：精确圈数 / 单程裁剪 / 自动圈数。
    #[test]
    fn test_effective_dist_dur() {
        let ring = sample_ring();
        // 绕圈自动
        let (d, _, _) = effective_dist_dur(
            &CustomRouteSpec { loop_mode: true, laps: 0, ..Default::default() },
            &ring, 2500.0, 1000,
        ).unwrap();
        assert_eq!(d, 2500.0);
        // 绕圈 2 圈
        let (d, dur, _) = effective_dist_dur(
            &CustomRouteSpec { loop_mode: true, laps: 2, ..Default::default() },
            &ring, 2500.0, 1000,
        ).unwrap();
        assert!((d - 2000.0).abs() < 0.01);
        assert!((dur - 800).abs() <= 1);
        // 单程超长裁剪
        let (d, _, _) = effective_dist_dur(
            &CustomRouteSpec { loop_mode: false, laps: 0, ..Default::default() },
            &ring, 2500.0, 1000,
        ).unwrap();
        assert!((d - 1000.0).abs() < 0.01);
    }

    /// 10 秒窗配速全部落在有效窗口内（与经典模式同一判定规则）。
    #[test]
    fn test_custom_speeds_within_window() {
        let ring = sample_ring();
        for seed in 0..8u64 {
            let t = build_custom(2100.0, 900, seed, 1_788_958_186_123, &ring, &[]);
            for w in &t.speedPerTenSec {
                let pace = 1000.0 / (w.value / 10.0);
                assert!((141.0..=600.0).contains(&pace), "seed={seed} 配速越界 {pace}");
            }
        }
    }

    /// 平滑折线：闭环补首点、开链端点不动、采样数正确。
    #[test]
    fn test_smooth_polyline_geometry() {
        let pts = vec![(38.90, 121.54), (38.91, 121.55), (38.905, 121.545)];
        let sm = crate::track::geom::smooth_polyline(&pts, 12, true);
        assert_eq!(sm.len(), 3 * 12 + 1, "闭环 = 段数×采样 + 闭合点");
        assert_eq!(sm[0], sm[sm.len() - 1]);
        let sm2 = crate::track::geom::smooth_polyline(&pts, 12, false);
        assert_eq!(sm2.len(), 2 * 12);
        assert!((sm2[0].0 - pts[0].0).abs() < 1e-9 && (sm2[0].1 - pts[0].1).abs() < 1e-9);
        let last = sm2[sm2.len() - 1];
        assert!((last.0 - pts[2].0).abs() < 1e-9 && (last.1 - pts[2].1).abs() < 1e-9);
        // 平滑环长度应大于两点直线距离的粗糙下界（非退化）
        let ring = ring_from_gcj_polyline(&pts, true).unwrap();
        assert!(ring.total_length_m > 1000.0, "len={}", ring.total_length_m);
        assert!(ring.points_bd.len() >= 37);
    }
}
