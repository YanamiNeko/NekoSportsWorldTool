//! 轨迹几何与随机工具。
//!
//! round 封装、RNG、打卡点 Catmull-Rom 拟合环 + 弧长表 + 弧长插值。

use rand::rngs::StdRng;
use rand::SeedableRng;
use rand_distr::{Distribution, Normal};

pub const MET_PER_DEG_LAT: f64 = 111_132.0;
pub const MET_PER_DEG_LNG: f64 = 86_600.0;

/// round(x, n)：按精确二进制值四舍五入。
pub fn round_to(x: f64, n: usize) -> f64 {
    let s = format!("{x:.n$}");
    s.parse().unwrap_or(x)
}

/// 可播种 RNG 封装。
pub struct Rng {
    inner: StdRng,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self {
            inner: StdRng::seed_from_u64(seed),
        }
    }
    pub fn random(&mut self) -> f64 {
        rand::Rng::gen_range(&mut self.inner, 0.0..1.0)
    }
    pub fn uniform(&mut self, a: f64, b: f64) -> f64 {
        rand::Rng::gen_range(&mut self.inner, a..=b)
    }
    pub fn randint(&mut self, a: i64, b: i64) -> i64 {
        rand::Rng::gen_range(&mut self.inner, a..=b)
    }
    pub fn gauss(&mut self, mu: f64, sigma: f64) -> f64 {
        Normal::new(mu, sigma).unwrap().sample(&mut self.inner)
    }
    pub fn choice<T>(&mut self, items: &[T]) -> T
    where
        T: Copy,
    {
        items[rand::Rng::gen_range(&mut self.inner, 0..items.len())]
    }
    pub fn weighted<T>(&mut self, items: &[(T, u32)]) -> T
    where
        T: Copy,
    {
        let total: u32 = items.iter().map(|(_, w)| *w).sum();
        let mut u = self.uniform(0.0, total as f64);
        for (item, w) in items {
            u -= *w as f64;
            if u < 0.0 {
                return *item;
            }
        }
        items[items.len() - 1].0
    }
}

/// 打卡点 BD 系 (lat, lng) → 闭合平面路径 + 弧长表 + 中心。
pub type PointRing = (Vec<(f64, f64)>, Vec<f64>, (f64, f64));
pub fn make_point_ring(bd_points: &[(f64, f64)]) -> PointRing {
    let n = bd_points.len();
    let cx = bd_points.iter().map(|q| q.0).sum::<f64>() / n as f64;
    let cy = bd_points.iter().map(|q| q.1).sum::<f64>() / n as f64;
    let mut ordered = bd_points.to_vec();
    ordered.sort_by(|a, b| {
        let ka = (a.0 - cx).atan2(a.1 - cy);
        let kb = (b.0 - cx).atan2(b.1 - cy);
        ka.partial_cmp(&kb).unwrap()
    });
    let plane: Vec<(f64, f64)> = ordered
        .iter()
        .map(|q| ((q.1 - cy) * MET_PER_DEG_LNG, (q.0 - cx) * MET_PER_DEG_LAT))
        .collect();
    let samples = 18usize;
    let mut dense = Vec::with_capacity(n * samples);
    for i in 0..n {
        let p0 = plane[(i + n - 1) % n];
        let p1 = plane[i];
        let p2 = plane[(i + 1) % n];
        let p3 = plane[(i + 2) % n];
        for j in 0..samples {
            let t = j as f64 / samples as f64;
            let (t2, t3) = (t * t, t * t * t);
            let x = 0.5
                * ((2.0 * p1.0)
                    + (-p0.0 + p2.0) * t
                    + (2.0 * p0.0 - 5.0 * p1.0 + 4.0 * p2.0 - p3.0) * t2
                    + (-p0.0 + 3.0 * p1.0 - 3.0 * p2.0 + p3.0) * t3);
            let y = 0.5
                * ((2.0 * p1.1)
                    + (-p0.1 + p2.1) * t
                    + (2.0 * p0.1 - 5.0 * p1.1 + 4.0 * p2.1 - p3.1) * t2
                    + (-p0.1 + 3.0 * p1.1 - 3.0 * p2.1 + p3.1) * t3);
            dense.push((x, y));
        }
    }
    let mut arcs = vec![0.0f64];
    for i in 1..=dense.len() {
        let a = dense[i - 1];
        let b = dense[i % dense.len()];
        arcs.push(arcs[i - 1] + ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt());
    }
    (dense, arcs, (cx, cy))
}

/// 环线弧长 → 坐标（线性插值）。
pub fn ring_point_at(dense: &[(f64, f64)], arcs: &[f64], s: f64) -> (f64, f64) {
    let total = *arcs.last().unwrap_or(&1.0);
    let s = s.rem_euclid(total);
    let mut lo = 0usize;
    let mut hi = arcs.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        if arcs[mid] < s {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    let i = lo.max(1);
    let a = dense[(i - 1) % dense.len()];
    let b = dense[i % dense.len()];
    let seg = arcs[i] - arcs[i - 1];
    let t = if seg > 0.0 {
        (s - arcs[i - 1]) / seg
    } else {
        0.0
    };
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

pub fn to_bd(x: f64, y: f64, c_lat: f64, c_lng: f64) -> (f64, f64) {
    (c_lat + y / MET_PER_DEG_LAT, c_lng + x / MET_PER_DEG_LNG)
}

// ── 坐标基准转换：WGS84 ↔ GCJ-02 ↔ BD-09 ─────────────────────────
// 打卡点来自服务端为 BD-09；OSM 路网为 WGS84。真实道路路由需先把路网
// 逐节点转成 BD-09，与经典模式（直接在 BD-09 上拟合打卡点环）保持一致。

const X_PI: f64 = std::f64::consts::PI * 3000.0 / 180.0;
const WGS_A: f64 = 6378245.0;
const WGS_EE: f64 = 0.00669342162296594323;

fn out_of_china(lat: f64, lng: f64) -> bool {
    lng < 72.004 || lng > 137.8347 || lat < 0.8293 || lat > 55.8271
}

fn transform_lat(x: f64, y: f64) -> f64 {
    let pi = std::f64::consts::PI;
    let mut ret = -100.0 + 2.0 * x + 3.0 * y + 0.2 * y * y + 0.1 * x * y + 0.2 * x.abs().sqrt();
    ret += (20.0 * (6.0 * x * pi).sin() + 20.0 * (2.0 * x * pi).sin()) * 2.0 / 3.0;
    ret += (20.0 * (y * pi).sin() + 40.0 * (y / 3.0 * pi).sin()) * 2.0 / 3.0;
    ret += (160.0 * (y / 12.0 * pi).sin() + 320.0 * (y * pi / 30.0).sin()) * 2.0 / 3.0;
    ret
}

fn transform_lng(x: f64, y: f64) -> f64 {
    let pi = std::f64::consts::PI;
    let mut ret = 300.0 + x + 2.0 * y + 0.1 * x * x + 0.1 * x * y + 0.1 * x.abs().sqrt();
    ret += (20.0 * (6.0 * x * pi).sin() + 20.0 * (2.0 * x * pi).sin()) * 2.0 / 3.0;
    ret += (20.0 * (x * pi).sin() + 40.0 * (x / 3.0 * pi).sin()) * 2.0 / 3.0;
    ret += (150.0 * (x / 12.0 * pi).sin() + 300.0 * (x / 30.0 * pi).sin()) * 2.0 / 3.0;
    ret
}

/// WGS84 → GCJ-02（火星坐标，境内偏移）。
pub fn wgs84_to_gcj02(lat: f64, lng: f64) -> (f64, f64) {
    if out_of_china(lat, lng) {
        return (lat, lng);
    }
    let pi = std::f64::consts::PI;
    let mut dlat = transform_lat(lng - 105.0, lat - 35.0);
    let mut dlng = transform_lng(lng - 105.0, lat - 35.0);
    let radlat = lat / 180.0 * pi;
    let mut magic = radlat.sin();
    magic = 1.0 - WGS_EE * magic * magic;
    let sqrtmagic = magic.sqrt();
    dlat = (dlat * 180.0) / ((WGS_A * (1.0 - WGS_EE)) / (magic * sqrtmagic) * pi);
    dlng = (dlng * 180.0) / (WGS_A / sqrtmagic * radlat.cos() * pi);
    (lat + dlat, lng + dlng)
}

/// GCJ-02 → BD-09（`wire::bd09_to_gcj02` 的逆变换）。
pub fn gcj02_to_bd09(gcj_lat: f64, gcj_lng: f64) -> (f64, f64) {
    let z = (gcj_lng * gcj_lng + gcj_lat * gcj_lat).sqrt() + 0.00002 * (gcj_lat * X_PI).sin();
    let theta = gcj_lat.atan2(gcj_lng) + 0.000003 * (gcj_lng * X_PI).cos();
    (z * theta.sin() + 0.006, z * theta.cos() + 0.0065)
}

/// WGS84 → BD-09（OSM 路网对齐打卡点用）。
pub fn wgs84_to_bd09(lat: f64, lng: f64) -> (f64, f64) {
    let (g_lat, g_lng) = wgs84_to_gcj02(lat, lng);
    gcj02_to_bd09(g_lat, g_lng)
}

/// 北京时间相对 UTC 的**分钟**偏移。
///
/// ## 为什么不能依赖 `chrono::Local`
///
/// `Local` 取的是**跑这个程序的机器**的时区。桌面上它是用户时区（北京时间），
/// 所以看不出问题；但只要跑在**时区不是 UTC+8 的环境**里（服务器 / 容器 /
/// CI / 用户把系统时区设成 UTC），就会整体偏 8 小时 —— 而且**不报错、静默写错**。
///
/// 这不是假想：本工具被跑到服务器上时实测到（2026-10，容器 `Etc/UTC`）——
///
/// ```text
///   App 自己跑的 120 个点：gainTime 相对 flag 偏差中位 +0.09 小时   ✅
///   本工具生成的 126 个点：中位 −7.92 小时，100% 命中                ❌
/// ```
///
/// 也就是同一个点的 `flag` 是 08:30、`gainTime` 字符串却写 00:30 ——
/// **同一条记录里两个时间自相矛盾**。真机写的是北京时间，所以这里固定 +8。
pub const CST_OFFSET_MIN: i64 = 8 * 60;

/// epoch 毫秒 → **北京时间**字符串，格式由 `fmt` 给（如 `"%Y%m%d%H"`）。
///
/// 所有"要给人看/要给服务端对"的时间都必须走这里，不要再直接用 `chrono::Local`。
pub fn fmt_cst(ms: i64, fmt: &str) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|t| {
            (t + chrono::Duration::minutes(CST_OFFSET_MIN))
                .format(fmt)
                .to_string()
        })
        .unwrap_or_default()
}

/// 轨迹点的 `gainTime`（真机格式 `YYYY-MM-DD HH:MM:SS`，**北京时间**）。
pub fn fmt_gain_time(ms: i64) -> String {
    fmt_cst(ms, "%Y-%m-%d %H:%M:%S")
}

#[cfg(test)]
mod cst_tests {
    use super::*;

    /// 向量取自**真机**记录（2026-10 实测对比 App 的 OBS 对象），不是自造的。
    #[test]
    fn gain_time_is_beijing_regardless_of_host_timezone() {
        // App 自己跑的那条：flag=1789600250000。该 epoch 对应的北京时间是 07:10:50
        // （真机那条的 gainTime 写的是 "2026-09-17 07:10:55" —— 那是"点的采集时刻"，
        //  比 flag 晚几秒，属于同一个口径）
        assert_eq!(fmt_gain_time(1789600250000), "2026-09-17 07:10:50");
        // 本工具生成的那条：flag=1791937828401 → 北京 08:30:28
        assert_eq!(fmt_gain_time(1791937828401), "2026-10-14 08:30:28");
        // **关键回归**：绝不能退化成 UTC（差 8 小时就是那个 bug）
        assert_ne!(fmt_gain_time(1791937828401), "2026-10-14 00:30:28");
    }

    /// OBS 的「小时目录」也必须按北京时间（真机就是按北京时间分目录的）。
    #[test]
    fn obs_hour_key_is_beijing() {
        assert_eq!(fmt_cst(1791937828401, "%Y%m%d%H"), "2026101408");
        // 北京 00:30（UTC 还是前一天 16:30）—— 用 UTC 会落到前一天
        let midnightish = 1791937800000 - CST_OFFSET_MIN * 60_000;
        assert_eq!(fmt_cst(midnightish, "%Y%m%d%H"), "2026101400");
    }
}
