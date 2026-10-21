//! Inspect reachability and the minimum closed route in the supplied point order.
use crate::{boundary, project, route, Coord, RoadGraph};
#[derive(Clone, Debug)]
pub struct Issue {
    pub message: String,
    pub point: Option<usize>,
    pub position: Option<Coord>,
}

#[cfg(test)]
mod tests {
    use crate::{graph::EdgeData, Coord, RoadGraph};
    #[test]
    fn viewport_query_only_returns_nearby_roads_without_mutating_graph() {
        let mut g = RoadGraph::new();
        for i in 0..10000 {
            let x = 116. + i as f64 * 0.0001;
            let a = g.add_node(Coord::new(x, 39.));
            let b = g.add_node(Coord::new(x, 39.0001));
            g.add_edge(
                a,
                b,
                EdgeData {
                    way_id: i,
                    oneway: false,
                    highway: "footway".into(),
                    maxspeed_kmh: None,
                    length_m: 11.,
                },
            );
        }
        g.compute_anchor();
        g.rebuild_index();
        let before = g.graph.edge_count();
        let visible = g.visible_edges(Coord::new(116., 38.99), Coord::new(116.001, 39.001));
        assert!(!visible.is_empty() && visible.len() < 20);
        assert_eq!(g.graph.edge_count(), before);
    }
}
#[derive(Clone, Debug, Default)]
pub struct Check {
    pub issues: Vec<Issue>,
    pub minimum_m: Option<f64>,
    pub components: usize,
}
impl Check {
    pub fn error(&self) -> Option<String> {
        self.issues.first().map(|i| i.message.clone())
    }
}
pub fn inspect(net: &RoadGraph, points: &[Coord], target: f64) -> Check {
    let mut out = Check::default();
    let mut seen = std::collections::HashSet::new();
    for n in net.graph.node_indices() {
        if seen.insert(n) {
            out.components += 1;
            let mut stack = vec![n];
            while let Some(n) = stack.pop() {
                for m in net.graph.neighbors_undirected(n) {
                    if seen.insert(m) {
                        stack.push(m);
                    }
                }
            }
        }
    }
    if points.is_empty() {
        out.issues.push(Issue {
            message: "没有路线点位，请刷新当前校区点位".into(),
            point: None,
            position: None,
        });
        return out;
    }
    for (i, &p) in points.iter().enumerate() {
        let issue = if !net.boundaries.is_empty() && !boundary::contains(&net.boundaries, p) {
            Some("点位在所选围栏外，请检查校区和设备定位")
        } else if net
            .nearest_edge(net.to_m(p))
            .map(|v| v.3 > 40.0)
            .unwrap_or(true)
        {
            Some("点位距离可用道路超过 40 米，请更新或导入完整路网")
        } else {
            None
        };
        if let Some(reason) = issue {
            out.issues.push(Issue {
                message: format!("点位 {}：{reason}", i + 1),
                point: Some(i),
                position: Some(p),
            });
        }
    }
    if !out.issues.is_empty() {
        return out;
    }
    let mut g = net.clone();
    let nodes = match project::add_virtual_nodes(&mut g, points) {
        Ok(n) => n,
        Err(e) => {
            out.issues.push(Issue {
                message: e,
                point: None,
                position: None,
            });
            return out;
        }
    };
    let mut length = 0.0;
    for i in 0..nodes.len() {
        let next = (i + 1) % nodes.len();
        match route::shortest_path(&g, nodes[i], nodes[next]) {
            Some(path) => {
                for w in path.windows(2) {
                    let a = g.to_m(g.node_coord(w[0]));
                    let b = g.to_m(g.node_coord(w[1]));
                    length += (a[0] - b[0]).hypot(a[1] - b[1]);
                }
            }
            None => out.issues.push(Issue {
                message: format!(
                    "点位 {} 到 {} 的道路不连通或方向受限，请检查校区/路网",
                    i + 1,
                    next + 1
                ),
                point: Some(next),
                position: Some(points[next]),
            }),
        }
    }
    if out.issues.is_empty() {
        out.minimum_m = Some(length);
        if target + 0.01 < length {
            out.issues.push(Issue{message:format!("目标 {:.0} 米不足：按当前顺序经过全部道路点并返回起点至少需要 {:.0} 米，请增加距离",target,length.ceil()),point:None,position:Some(points[0])});
        }
    }
    out
}
