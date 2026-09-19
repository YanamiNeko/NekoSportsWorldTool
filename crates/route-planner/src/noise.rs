//! AR(1) 时间自相关红噪声：GPS 漂移。
//!
//!   e_t = α·e_{t-1} + η_t,  η~N(0, σ_t)
//!
//! σ_t 由建筑 SDF 自适应放大（高楼/遮挡边缘漂移增大）。

use rand::rngs::StdRng;
use rand::SeedableRng;

/// 生成 n 步二维 AR(1) 漂移（米），每步标准差由 `sigmas` 给出（SDF 调制后）。
pub fn ar1_xy(alpha: f64, sigmas: &[f64], n: usize, seed: u64) -> Vec<(f64, f64)> {
    use rand_distr::{Distribution, Normal};
    let mut rng = StdRng::seed_from_u64(seed);
    let normal = Normal::<f64>::new(0.0, 1.0).unwrap();
    let mut out = Vec::with_capacity(n);
    let (mut ex, mut ey) = (0.0f64, 0.0f64);
    for i in 0..n {
        let sigma = sigmas.get(i).copied().unwrap_or(0.0);
        ex = alpha * ex + sigma * normal.sample(&mut rng);
        ey = alpha * ey + sigma * normal.sample(&mut rng);
        out.push((ex, ey));
    }
    out
}

/// 顺序式 GPS 抖动：AR(1) 相关漂移 + 独立高斯测量噪声。
///
/// 适用于漂移状态需随正常点逐步推进、且方差随 SDF 逐点变化的场景。
/// `step` 的 `scale` 为当前点 SDF 方差放大系数，`z[4]` 为 4 个独立标准正态样本。
pub struct GpsJitter {
    /// AR(1) 自相关系数 α。
    pub alpha: f64,
    /// 相关漂移的基础标准差（米）。
    pub sigma_correlated: f64,
    /// 独立高斯测量噪声标准差（米）。
    pub sigma_meas: f64,
    ex: f64,
    ey: f64,
}

impl GpsJitter {
    pub fn new(alpha: f64, sigma_correlated: f64, sigma_meas: f64) -> Self {
        GpsJitter { alpha, sigma_correlated, sigma_meas, ex: 0.0, ey: 0.0 }
    }

    /// 当前相关漂移状态（供异常点复用上一次漂移）。
    pub fn state(&self) -> (f64, f64) {
        (self.ex, self.ey)
    }

    /// 前进一步，返回总抖动 (dx, dy)（相关漂移 + 测量噪声）。
    pub fn step(&mut self, scale: f64, z: [f64; 4]) -> (f64, f64) {
        let s = self.sigma_correlated * scale;
        self.ex = self.alpha * self.ex + s * z[0];
        self.ey = self.alpha * self.ey + s * z[1];
        (self.ex + self.sigma_meas * z[2], self.ey + self.sigma_meas * z[3])
    }
}

