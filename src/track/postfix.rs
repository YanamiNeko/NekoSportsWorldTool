//! 轨迹后处理：
//! 结尾断崖（最后正常点压速 1.2-2.6km/h、时间延伸 6-14s）、起终点哨兵
//! （索引1 type=5 全零 / 末点 type=6 / 首点 type∈{0,7}）、后半段 type=1/2 散布、
//! 索引2 avgSpeed 边界修正、首点速度取首个非零值。

use super::geom::{fmt_gain_time, round_to, Rng};
use super::model::GenPoint;

pub fn apply_post_fixes(locs: &mut [GenPoint], rng: &mut Rng, start_ms: i64) {
    // 结尾断崖：最后正常点压速 + 时间自然延伸 6-14s
    if locs.len() >= 4 {
        let mut ci = locs.len() - 1;
        while ci > 0 && locs[ci].ptype == -1 {
            ci -= 1;
        }
        if ci > 1 {
            let stop_t = locs[ci].totalTime + rng.randint(6, 14);
            locs[ci].totalTime = stop_t;
            locs[ci].validTime = stop_t;
            let stop_kmh = round_to(rng.uniform(1.2, 2.6), 4);
            locs[ci].speed = stop_kmh;
            locs[ci].avgSpeed = round_to(stop_kmh / 3.6, 4);
            locs[ci].bdS = round_to(rng.uniform(0.05, 0.15), 3);
            locs[ci].gainTime = fmt_gain_time(start_ms + stop_t * 1000);
            locs[ci].gainTimeMs = start_ms + stop_t * 1000;
            // totalDis 保持（App 过滤条件 totalDis>0）
        }
    }
    if locs.len() > 2 {
        // 起点哨兵（type=5→setStartPoint）：全零累计 + state=1
        let sp = &mut locs[1];
        sp.ptype = 5;
        sp.state = 1;
        sp.locType = 1;
        sp.radius = round_to(rng.uniform(2.0, 3.5), 2);
        sp.speed = 0.0;
        sp.avgSpeed = 0.0;
        sp.bdS = 0.0;
        sp.totalDis = 0.0;
        sp.validDis = 0.0;
        sp.totalTime = 0;
        sp.validTime = 0;
        sp.steps = 0;
        // 终点哨兵
        let last = locs.len() - 1;
        let e = &mut locs[last];
        e.ptype = 6;
        e.locType = 1;
        e.radius = round_to(rng.uniform(1.5, 3.0), 2);
        // 后半段散布 type=1/2（67-99% 行程区间）
        let from = (locs.len() as f64 * 0.67) as usize;
        for i in from..locs.len().saturating_sub(2) {
            if rng.random() < 0.06 {
                locs[i].ptype = rng.choice(&[1, 1, 2]);
                locs[i].locType = 1;
            }
        }
    }
    // 边界修正①：索引2 首个真实点 avgSpeed 不跨哨兵计算
    if locs.len() > 3 {
        let p2 = &mut locs[2];
        p2.avgSpeed = round_to(p2.totalDis / 1.0f64.max(p2.totalTime as f64), 4);
        p2.speed = round_to(p2.avgSpeed * 3.6, 4);
    }
    // 首点 = 起点：totalTime=0、type∈{0,7}、速度取首个非零值
    if !locs.is_empty() {
        locs[0].totalTime = 0;
        locs[0].validTime = 0;
        locs[0].state = 1;
        locs[0].ptype = rng.choice(&[0, 0, 0, 7]);
        locs[0].locType = 1;
        locs[0].radius = round_to(rng.uniform(1.5, 3.0), 2);
        for j in 1..locs.len() {
            if locs[j].avgSpeed > 0.0 {
                locs[0].avgSpeed = locs[j].avgSpeed;
                locs[0].bdS = locs[j].bdS;
                break;
            }
        }
        if locs[0].speed == 0.0 {
            locs[0].speed = round_to(rng.uniform(6.0, 14.0), 4);
        }
    }
}
