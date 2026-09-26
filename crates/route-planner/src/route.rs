//! 闭环路线生成：方向锚点搜索 + 已通过 physical edge（way_id）重复权值惩罚。
//!
//! 贪心游走 + 目标引导 + 回家闭合，配合多次重试择优，抑制折返。

use std::collections::HashMap;

use petgraph::algo::astar;
use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;
use petgraph::Direction;
use rand::rngs::StdRng;
use rand::SeedableRng;

use crate::graph::{angle_diff, bearing, dist, Coord, RoadGraph};

#[derive(Clone, Debug)]
pub struct RouteOptions {
    /// 最大向心加速度 m/s²（弯道限速）。
    pub a_c_max: f64,
    /// 转弯切角阈值（度）。
    pub turn_threshold_deg: f64,
    /// 最小转弯半径（米）。
    pub min_radius_m: f64,
    /// 重复边惩罚系数（已走 way 长度乘子）。
    pub repeat_penalty: f64,
    /// 判定「到达目标」的半径（米）。
    pub snap_m: f64,
    /// 转弯惩罚（等效米 / 弧度）。
    pub turn_weight_m: f64,
}

impl Default for RouteOptions {
    fn default() -> Self {
        RouteOptions {
            a_c_max: 2.5,
            turn_threshold_deg: 30.0,
            min_radius_m: 6.0,
            repeat_penalty: 1.5,
            snap_m: 30.0,
            turn_weight_m: 25.0,
        }
    }
}

const MAX_ITERS: usize = 4000;
const RETRIES: usize = 4;

/// 两点最短路径（节点序列，几何长度）。
pub fn shortest_path(g: &RoadGraph, from: NodeIndex, to: NodeIndex) -> Option<Vec<NodeIndex>> {
    let res = astar(
        &g.graph,
        from,
        |n| n == to,
        |e| e.weight().length_m,
        |_| 0.0,
    );
    res.map(|(_cost, path)| path)
}

fn dist_nodes(g: &RoadGraph, a: NodeIndex, b: NodeIndex) -> f64 {
    dist(g.to_m(g.node_coord(a)), g.to_m(g.node_coord(b)))
}

fn shortest_len(g: &RoadGraph, from: NodeIndex, to: NodeIndex) -> f64 {
    shortest_path(g, from, to)
        .map(|p| path_len(g, &p))
        .unwrap_or(f64::INFINITY)
}

fn path_len(g: &RoadGraph, path: &[NodeIndex]) -> f64 {
    path.windows(2).map(|w| dist_nodes(g, w[0], w[1])).sum()
}

/// 一次方向锚点游走，返回节点路径（首尾为 start，闭合）。
///
/// 两阶段：
///   1) 最短路径依次经过所有必经点/打卡点（**保证必达**）；
///   2) 方向锚点游走补足目标距离，再最短路径回家闭合。
fn directional_walk(
    g: &RoadGraph,
    start: NodeIndex,
    must_pass: &[NodeIndex],
    target_len_m: f64,
    anchor_deg: f64,
    rng: &mut StdRng,
    opts: &RouteOptions,
) -> Vec<NodeIndex> {
    let mut path = vec![start];
    let mut used: HashMap<u64, u32> = HashMap::new();
    let mut length = 0.0f64;
    let mut current = start;
    let mut prev_dir: Option<f64> = None;

    // —— 阶段一：最短路径依次经过所有必经点（打卡点必达）——
    for &t in must_pass {
        if let Some(seg) = shortest_path(g, current, t) {
            for &n in seg.iter().skip(1) {
                if let Some(e) = g.graph.find_edge(current, n) {
                    let wid = g.graph[e].way_id;
                    *used.entry(wid).or_insert(0) += 1;
                    length += g.graph[e].length_m;
                }
                prev_dir = Some(bearing(
                    g.to_m(g.node_coord(current)),
                    g.to_m(g.node_coord(n)),
                ));
                current = n;
                path.push(n);
            }
        }
    }

    // —— 阶段二：游走补足距离并回家闭合 ——
    let anchor = anchor_deg.to_radians();
    let mut desired = prev_dir.unwrap_or(anchor);
    let mut prev_node = if path.len() >= 2 {
        Some(path[path.len() - 2])
    } else {
        None
    };

    for _ in 0..MAX_ITERS {
        let d_home = shortest_len(g, current, start);
        if length + d_home >= target_len_m {
            // 收口：最短路径回家
            if let Some(home) = shortest_path(g, current, start) {
                if home.len() > 1 {
                    path.extend(home.into_iter().skip(1));
                }
            }
            break;
        }

        // 候选边
        let mut candidates: Vec<(NodeIndex, f64)> = Vec::new();
        for e in g.graph.edges_directed(current, Direction::Outgoing) {
            let m = e.target();
            if Some(m) == prev_node {
                continue; // 禁止立即折返
            }
            let ed = e.weight();
            let dir_e = bearing(g.to_m(g.node_coord(current)), g.to_m(g.node_coord(m)));
            let used_n = used.get(&ed.way_id).copied().unwrap_or(0) as f64;
            let turn = match prev_dir {
                Some(pd) => opts.turn_weight_m * angle_diff(pd, dir_e),
                None => 0.0,
            };
            let mut score = ed.length_m * (1.0 + opts.repeat_penalty * used_n) + turn;
            // 方向偏置：对齐 desired 减分（更优）
            score -= ed.length_m * 0.6 * (dir_e - desired).cos();
            candidates.push((m, score));
        }
        if candidates.is_empty() {
            // 死胡同：允许折返一次
            if let Some(prev) = prev_node {
                candidates.push((prev, 1e6));
            }
        }
        if candidates.is_empty() {
            break;
        }
        candidates.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        // 前 3 名 soft-max 加权随机
        let k = candidates.len().min(3);
        let min_score = candidates[0].1;
        let weights: Vec<f64> = candidates[..k]
            .iter()
            .map(|c| (-(c.1 - min_score) / 40.0).exp())
            .collect();
        let total: f64 = weights.iter().sum();
        let mut u: f64 = rand::Rng::gen_range(rng, 0.0..total.max(1e-9));
        let mut pick = 0usize;
        for (i, w) in weights.iter().enumerate() {
            u -= *w;
            if u <= 0.0 {
                pick = i;
                break;
            }
        }
        if pick >= weights.len() {
            pick = weights.len() - 1;
        }
        let next = candidates[pick].0;

        // 移动
        let dir_now = bearing(g.to_m(g.node_coord(current)), g.to_m(g.node_coord(next)));
        if let Some(e) = g.graph.find_edge(current, next) {
            let wid = g.graph[e].way_id;
            *used.entry(wid).or_insert(0) += 1;
            length += g.graph[e].length_m;
        }
        desired = dir_now; // 继续沿当前朝向探索
        prev_dir = Some(dir_now);
        prev_node = Some(current);
        current = next;
        path.push(next);

        // 提前闭合（回到起点且长度足够）
        if current == start && length >= target_len_m {
            break;
        }
    }

    // 保证闭合
    if *path.last().unwrap() != start {
        if let Some(home) = shortest_path(g, current, start) {
            if home.len() > 1 {
                path.extend(home.into_iter().skip(1));
            }
        }
    }
    path
}

fn repeat_cost(g: &RoadGraph, path: &[NodeIndex]) -> (u32, f64) {
    let mut used: HashMap<u64, u32> = HashMap::new();
    let mut repeats = 0u32;
    for w in path.windows(2) {
        if let Some(e) = g.graph.find_edge(w[0], w[1]) {
            let wid = g.graph[e].way_id;
            let c = used.entry(wid).or_insert(0);
            if *c > 0 {
                repeats += 1;
            }
            *c += 1;
        }
    }
    (repeats, path_len(g, path))
}

/// 生成闭合路线（节点序列，首尾一致）。
///
/// `must_pass` 为强制必经点（按序）；`anchor_pts` 为软引导点，仅用于方向锚点
/// （不必全部经过，只引导游走朝向，最终由调用方在 <40m 内吸附）。
pub fn plan_loop(
    g: &RoadGraph,
    start: NodeIndex,
    must_pass: &[NodeIndex],
    anchor_pts: &[NodeIndex],
    target_len_m: f64,
    seed: u64,
    opts: &RouteOptions,
) -> Result<Vec<NodeIndex>, String> {
    // 方向锚点：指向最远软引导点，否则最远必经点，否则 0（随机多角度重试兜底）。
    let anchor_deg = anchor_pts
        .iter()
        .chain(must_pass.iter())
        .max_by(|a, b| {
            dist_nodes(g, start, **a)
                .partial_cmp(&dist_nodes(g, start, **b))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|far| bearing(g.to_m(g.node_coord(start)), g.to_m(g.node_coord(*far))).to_degrees())
        .unwrap_or(0.0);

    // 预检查：必经点必须依次可达（围栏裁剪可能把路网切成多个连通分量，落在被裁掉
    // 分量上的必经点会静默丢失，破坏「必达」承诺）。不可达直接报错，而非静默跳过。
    let mut cur = start;
    for &t in must_pass {
        if shortest_path(g, cur, t).is_none() {
            return Err(format!(
                "必经点 ({:.6},{:.6}) 不可达：可能落在围栏外或路网不连通",
                g.node_coord(t).lat,
                g.node_coord(t).lon
            ));
        }
        cur = t;
    }

    let mut best: Option<Vec<NodeIndex>> = None;
    let mut best_key: Option<(f64, u32)> = None;
    for i in 0..RETRIES {
        let mut rng = StdRng::seed_from_u64(seed.wrapping_add(i as u64));
        let path = directional_walk(
            g,
            start,
            must_pass,
            target_len_m,
            anchor_deg + i as f64 * 90.0,
            &mut rng,
            opts,
        );
        if path.len() < 2 {
            continue;
        }
        let (repeats, len) = repeat_cost(g, &path);
        let len_err = (len - target_len_m).abs();
        // 评分：优先长度贴合，其次少折返
        let key = (len_err + repeats as f64 * 60.0, repeats);
        let better = match best_key {
            Some((bk, _)) => key.0 < bk,
            None => true,
        };
        if better {
            best = Some(path);
            best_key = Some(key);
        }
    }

    let path = best.ok_or("无法生成路线".to_string())?;
    Ok(path)
}

/// 节点路径 → 度系折线坐标（供 UI/平滑使用）。
pub fn path_coords(g: &RoadGraph, path: &[NodeIndex]) -> Vec<Coord> {
    path.iter().map(|&n| g.node_coord(n)).collect()
}
