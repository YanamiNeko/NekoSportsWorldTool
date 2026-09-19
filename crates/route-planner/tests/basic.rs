//! 集成测试：路网解析 + 闭环路由 + 平滑。

use route_planner::{plan_route, Coord, RoadGraph, RouteOptions};

/// 构建 4x4 网格路网（双向边，节点间距约 111m）。
fn grid() -> RoadGraph {
    let mut g = RoadGraph::new();
    let mut nodes = vec![];
    for j in 0..4i64 {
        for i in 0..4i64 {
            nodes.push(g.add_node(Coord::new(i as f64 * 0.001, j as f64 * 0.001)));
        }
    }
    let mut wid = 1u64;
    for j in 0..4usize {
        for i in 0..3usize {
            let a = nodes[j * 4 + i];
            let b = nodes[j * 4 + i + 1];
            let len = 111.0;
            let data = route_planner::graph::EdgeData {
                way_id: wid,
                oneway: false,
                highway: "residential".into(),
                maxspeed_kmh: None,
                length_m: len,
            };
            g.add_edge(a, b, data.clone());
            g.add_edge(b, a, data);
            wid += 1;
        }
    }
    for i in 0..4usize {
        for j in 0..3usize {
            let a = nodes[j * 4 + i];
            let b = nodes[(j + 1) * 4 + i];
            let data = route_planner::graph::EdgeData {
                way_id: wid,
                oneway: false,
                highway: "residential".into(),
                maxspeed_kmh: None,
                length_m: 111.0,
            };
            g.add_edge(a, b, data.clone());
            g.add_edge(b, a, data);
            wid += 1;
        }
    }
    g.compute_anchor();
    g.rebuild_index();
    g
}

#[test]
fn test_plan_route_closes_loop() {
    let g = grid();
    let start = Coord::new(0.0005, 0.0); // 道路中段
    let wp = vec![start, Coord::new(0.003, 0.003)];
    let route = plan_route(&g, &wp, 1200.0, 42, &RouteOptions::default()).unwrap();
    assert!(route.points.len() > 10, "采样点过少");
    assert!(route.length_m > 100.0, "长度异常 {}", route.length_m);
    // 闭合：首末点接近
    let f = &route.points[0];
    let l = route.points.last().unwrap();
    let dx = (f.lon - l.lon) * 111_320.0;
    let dy = (f.lat - l.lat) * 111_132.0;
    let d = (dx * dx + dy * dy).sqrt();
    assert!(d < 120.0, "未闭合: {d}m");
}

#[test]
fn test_virtual_node_projection_mid_edge() {
    let mut g = grid();
    // 起点落在路段中间：应能切分并返回虚拟节点
    let q = Coord::new(0.0005, 0.0);
    let v = route_planner::project::add_virtual_nodes(&mut g, &[q]).unwrap();
    assert_eq!(v.len(), 1);
    let n = v[0];
    let c = g.node_coord(n);
    assert!((c.lon - 0.0005).abs() < 1e-3, "投影点偏移");
}

#[test]
fn test_route_visits_checkpoints() {
    let g = grid();
    let wps = vec![
        Coord::new(0.0, 0.0),
        Coord::new(0.003, 0.0),
        Coord::new(0.003, 0.003),
        Coord::new(0.0, 0.003),
    ];
    let route = plan_route(&g, &wps, 2000.0, 42, &RouteOptions::default()).unwrap();
    for c in &wps[1..] {
        let min_d = route
            .points
            .iter()
            .map(|p| {
                let dx = (p.lon - c.lon) * 111_320.0;
                let dy = (p.lat - c.lat) * 111_132.0;
                (dx * dx + dy * dy).sqrt()
            })
            .fold(f64::INFINITY, f64::min);
        assert!(min_d < 40.0, "checkpoint ({},{}) missed by {}m", c.lat, c.lon, min_d);
    }
}

#[test]
fn test_point_in_polygon_and_fence() {
    let poly = vec![
        Coord::new(0.0, 0.0),
        Coord::new(0.002, 0.0),
        Coord::new(0.002, 0.002),
        Coord::new(0.0, 0.002),
    ];
    assert!(route_planner::point_in_polygon(&poly, Coord::new(0.001, 0.001)));
    assert!(!route_planner::point_in_polygon(&poly, Coord::new(0.005, 0.005)));

    let mut g = grid();
    let before = g.graph.node_count();
    g.retain_inside_any(&[poly]);
    let after = g.graph.node_count();
    assert!(after < before, "过滤后节点应减少: {after} vs {before}");
    assert!(g.graph.edge_count() > 0, "过滤后仍有道路");
}

#[test]
fn test_plan_route_respects_start_and_order() {
    let g = grid();
    let start = Coord::new(0.0, 0.0);
    let mid = Coord::new(0.003, 0.0);
    let end = Coord::new(0.003, 0.003);
    let wps = vec![start, mid, end];
    let route = plan_route(&g, &wps, 1500.0, 7, &RouteOptions::default()).unwrap();
    // 起点 = waypoints[0]（网格节点精确落位）
    let f = &route.points[0];
    let dx = (f.lon - start.lon) * 111_320.0;
    let dy = (f.lat - start.lat) * 111_132.0;
    assert!((dx * dx + dy * dy).sqrt() < 1.0, "起点偏移过大");
    // 必经点按顺序到达：mid 在 end 之前出现在路线上
    let idx_of = |c: Coord| -> usize {
        route
            .points
            .iter()
            .position(|p| {
                let dx = (p.lon - c.lon) * 111_320.0;
                let dy = (p.lat - c.lat) * 111_132.0;
                (dx * dx + dy * dy).sqrt() < 40.0
            })
            .unwrap_or_else(|| panic!("必经点 ({},{}) 未达", c.lat, c.lon))
    };
    assert!(idx_of(mid) < idx_of(end), "必经点顺序被破坏");
}

#[test]
fn test_osm_parse_minimal() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<osm version="0.6">
  <node id="1" lat="39.0000" lon="121.0000"/>
  <node id="2" lat="39.0010" lon="121.0010"/>
  <node id="3" lat="39.0020" lon="121.0020"/>
  <way id="101">
    <nd ref="1"/><nd ref="2"/><nd ref="3"/>
    <tag k="highway" v="footway"/>
  </way>
  <node id="10" lat="39.0005" lon="121.0005"/>
  <node id="11" lat="39.0005" lon="121.0015"/>
  <node id="12" lat="39.0015" lon="121.0015"/>
  <node id="13" lat="39.0015" lon="121.0005"/>
  <way id="201">
    <nd ref="10"/><nd ref="11"/><nd ref="12"/><nd ref="13"/><nd ref="10"/>
    <tag k="building" v="yes"/>
  </way>
</osm>"#;
    let g = route_planner::load_osm(xml.as_bytes()).unwrap();
    assert!(g.graph.node_count() >= 3);
    assert!(g.graph.edge_count() >= 2, "双向道路边");
    assert_eq!(g.buildings.len(), 1, "应解析出 1 个建筑");
}
