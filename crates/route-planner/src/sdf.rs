//! 建筑有向距离场（SDF）：到最近建筑墙面的距离，用于自适应放大 GPS 漂移方差。
//!
//!   σ(x) = σ_base · (1 + A·exp(−d(x)/d0))，d 为到最近建筑墙面的有向距离。

use rstar::{RTree, RTreeObject, AABB};

use crate::graph::Coord;

#[derive(Clone, Copy)]
struct WallSeg {
    a: [f64; 2],
    b: [f64; 2],
}

impl RTreeObject for WallSeg {
    type Envelope = AABB<[f64; 2]>;
    fn envelope(&self) -> Self::Envelope {
        AABB::from_corners(self.a, self.b)
    }
}

impl rstar::PointDistance for WallSeg {
    fn distance_2(&self, point: &[f64; 2]) -> f64 {
        let (proj, _) = crate::graph::project_to_segment(*point, self.a, self.b);
        let (dx, dy) = (proj[0] - point[0], proj[1] - point[1]);
        dx * dx + dy * dy
    }
}

pub struct Sdf {
    index: RTree<WallSeg>,
    /// 衰减参数 A（近墙放大倍数）。
    pub amp: f64,
    /// 衰减尺度 d0（米）。
    pub d0: f64,
}

impl Sdf {
    pub fn new(amp: f64, d0: f64) -> Self {
        Sdf { index: RTree::new(), amp, d0 }
    }

    /// 用建筑外环（度）构建 SDF。`to_m` 为度→米投影。
    pub fn from_buildings<F>(buildings: &[Vec<Coord>], to_m: F, amp: f64, d0: f64) -> Self
    where
        F: Fn(Coord) -> [f64; 2],
    {
        let mut segs: Vec<WallSeg> = Vec::new();
        for ring in buildings {
            let pts: Vec<[f64; 2]> = ring.iter().map(|c| to_m(*c)).collect();
            let n = pts.len();
            if n < 3 {
                continue;
            }
            for i in 0..n {
                let a = pts[i];
                let b = pts[(i + 1) % n];
                segs.push(WallSeg { a, b });
            }
        }
        let index = RTree::bulk_load(segs);
        Sdf { index, amp, d0 }
    }

    /// 到最近建筑墙面的距离（米，无符号；无建筑返回 f64::INFINITY）。
    pub fn distance(&self, p: [f64; 2]) -> f64 {
        let mut best = f64::INFINITY;
        for seg in self.index.locate_within_distance(p, 400.0f64 * 400.0) {
            let (proj, _t) = crate::graph::project_to_segment(p, seg.a, seg.b);
            let d = crate::graph::dist(p, proj);
            if d < best {
                best = d;
            }
        }
        best
    }

    /// 自适应漂移方差乘子：1 + A·exp(−d/d0)。
    pub fn sigma_scale(&self, p: [f64; 2]) -> f64 {
        let d = self.distance(p);
        if d.is_infinite() {
            1.0
        } else {
            1.0 + self.amp * (-d / self.d0.max(1e-9)).exp()
        }
    }
}
