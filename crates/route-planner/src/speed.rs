//! 奥恩斯坦-乌伦贝克（OU）过程配速模拟：均值回归随机微分方程。
//!
//!   dv_t = θ(μ − v_t)dt + σ dW_t
//!
//! Euler–Maruyama 离散化：v_{t+1} = v_t + θ(μ − v_t)Δt + σ·√Δt·Z，Z~N(0,1)。

use rand::rngs::StdRng;
use rand::SeedableRng;

#[derive(Clone, Copy, Debug)]
pub struct OuParams {
    /// 均值回归速率 θ（越小越持久）。
    pub theta: f64,
    /// 波动率 σ（m/s / √s）。
    pub sigma: f64,
}

impl Default for OuParams {
    fn default() -> Self {
        OuParams {
            theta: 0.12,
            sigma: 0.06,
        }
    }
}

/// 生成 n 步 OU 速度序列（m/s），首值为 v0，并钳制到 [floor, ceil]。
pub fn ou_series(
    mu: f64,
    v0: f64,
    dt: f64,
    n: usize,
    seed: u64,
    p: &OuParams,
    floor: f64,
    ceil: f64,
) -> Vec<f64> {
    use rand_distr::{Distribution, Normal};
    let mut rng = StdRng::seed_from_u64(seed);
    let normal = Normal::<f64>::new(0.0, 1.0).unwrap();
    let mut out = Vec::with_capacity(n);
    let mut v = v0;
    let sq = dt.sqrt();
    for _ in 0..n {
        let z = normal.sample(&mut rng);
        v = v + p.theta * (mu - v) * dt + p.sigma * sq * z;
        v = v.clamp(floor, ceil);
        out.push(v);
    }
    out
}
