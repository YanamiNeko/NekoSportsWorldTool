//! 官方卡路里/功率公式。
//!
//! MET(pace 分/km) 分段：p<10 → 33/p+4.1；10≤p<12 → 102/p−3.7；p≥12 → 33.6/p+1；
//! 无位移 12。calorie = 体重 × MET × 时长(h)。体重默认 68。
//! avgPower = 1.1 × 体重 × v(m/s)（RunHistoryDetailPImpl.enrichRunOverviewFromTrail）。

/// 官方展示 kcal。
pub fn official_kcal(weight: f64, total_time: i64, total_dis: f64) -> i64 {
    let km = total_dis / 1000.0;
    let mut met = if km <= 0.0 {
        12.0
    } else {
        let pace = (total_time as f64 / 60.0) / km;
        if pace < 10.0 {
            33.0 / pace + 4.1
        } else if pace < 12.0 {
            102.0 / pace - 3.7
        } else {
            33.6 / pace + 1.0
        }
    };
    met = (met * 10.0).round() / 10.0; // 官方按 0.1 粒度取整
    (weight * met * (total_time as f64 / 3600.0)).round() as i64
}

/// 官方兜底功率（瓦）。
pub fn avg_power(weight: f64, total_dis: f64, total_time: i64) -> i64 {
    if total_time <= 0 {
        return 0;
    }
    (1.1 * weight * (total_dis / total_time as f64)).round() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 实测向量。
    #[test]
    fn test_official_kcal_vectors() {
        assert_eq!(official_kcal(68.0, 1220, 3300.0), 219);
        assert_eq!(official_kcal(68.0, 600, 1000.0), 74);
    }

    /// 功率向量：round(1.1*68*(3300/1220)) = 202。
    #[test]
    fn test_avg_power_vector() {
        assert_eq!(avg_power(68.0, 3300.0, 1220), 202);
    }
}
