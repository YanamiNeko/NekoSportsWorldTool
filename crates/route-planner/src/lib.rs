//! 真实道路拓扑路由 + 运动学物理仿真库。
//!
//! 三层解耦：
//!   1. 路由层（纯几何/拓扑）：OSM 解析 → 有向图 → 虚拟节点投影切分 → 闭环路线
//!   2. 动力学层（运动学 + 传感器）：曲率限速 / OU 配速 / AR(1) 噪声 + SDF / 生物力学
//!   3. 协议层（调用方适配）：由 `Route` + 速度序列组装最终轨迹
//!
//! 坐标约定：本 crate 内部以「度」存储（调用方已把 OSM 对齐到 BD 工作系），
//! 距离/曲率经本地等距投影转米制平面。

use petgraph::graph::NodeIndex;

pub mod biomech;
pub mod graph;
pub mod kinematics;
pub mod load;
pub mod noise;
pub mod project;
pub mod route;
pub mod sdf;
pub mod smooth;
pub mod speed;

pub use biomech::{cadence_for, fatigue_from_km, gait, ou_cadence_series, speed_from, stride_for};
pub use graph::{point_in_polygon, Coord, RoadGraph};
pub use kinematics::{pace_profile, speed_limit_ahead, KinParams};
pub use load::{load_osm, parse_osm};
pub use noise::{ar1_xy, GpsJitter};
pub use route::RouteOptions;
pub use sdf::Sdf;
pub use smooth::{turn_speed_limit, RoutePoint};

/// 一条规划好的空间路线（度系采样点 + 总长）。
#[derive(Clone, Debug)]
pub struct Route {
    pub points: Vec<RoutePoint>,
    pub length_m: f64,
}

/// 入口：解析路网 + 虚拟节点 + 闭环路由 + 曲率平滑。
///
/// `waypoints[0]` 为起点，`waypoints[1..]` 为必经打卡点（按顺序），
/// 路线自动闭合回起点。
pub fn plan_route(
    net: &RoadGraph,
    waypoints: &[Coord],
    target_len_m: f64,
    seed: u64,
    opts: &RouteOptions,
) -> Result<Route, String> {
    plan_route_split(net, waypoints, &waypoints[1..], target_len_m, seed, opts)
}

/// 入口（软/硬点分离）：`waypoints[0]` 为起点；`must` 为强制必经点（按序，不含
/// 起点，可为空）；`waypoints[1..]` 为软引导点（仅用于方向锚点，不必全经过）。
pub fn plan_route_split(
    net: &RoadGraph,
    waypoints: &[Coord],
    must: &[Coord],
    target_len_m: f64,
    seed: u64,
    opts: &RouteOptions,
) -> Result<Route, String> {
    if waypoints.is_empty() {
        return Err("无起点".into());
    }
    let mut g = net.clone();

    // 合并查询点（起点 + 必经点 + 软引导点），交由投影函数按坐标去重。
    let mut queries = vec![waypoints[0]];
    queries.extend(must.iter().copied());
    queries.extend(waypoints[1..].iter().copied());
    let vnodes = project::add_virtual_nodes(&mut g, &queries)?;

    let start = vnodes[0];
    let must_nodes: Vec<NodeIndex> = vnodes[1..1 + must.len()]
        .iter()
        .copied()
        .filter(|n| *n != start)
        .collect();
    let anchor_nodes: Vec<NodeIndex> = vnodes[1 + must.len()..].to_vec();

    let path = route::plan_loop(&g, start, &must_nodes, &anchor_nodes, target_len_m, seed, opts)?;
    let coords = route::path_coords(&g, &path);
    let points = smooth::smooth_and_sample(
        &g,
        &coords,
        opts.min_radius_m,
        opts.turn_threshold_deg,
        1.0,
    );
    if points.is_empty() {
        return Err("路线平滑后为空".into());
    }
    let length_m = points.last().map(|p| p.s).unwrap_or(0.0);
    Ok(Route { points, length_m })
}
