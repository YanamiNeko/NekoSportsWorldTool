//! 虚拟节点：起点/途经点垂直投影到最近道路边，落在路段中间时动态切分。

use petgraph::graph::NodeIndex;

use crate::graph::{Coord, EdgeData, RoadGraph};

const EPS: f64 = 1e-3;

/// 将查询点投影到路网，落在路段中间时切分边，返回虚拟节点。
/// `unique`：对相同坐标去重（起点与终点重合时共用同一虚拟节点）。
pub fn add_virtual_nodes(g: &mut RoadGraph, queries: &[Coord]) -> Result<Vec<NodeIndex>, String> {
    let mut out = Vec::with_capacity(queries.len());
    let mut cache: std::collections::HashMap<(i64, i64), NodeIndex> =
        std::collections::HashMap::new();

    for q in queries {
        // 量化去重（约 1e-6 度）。
        let key = ((q.lon * 1e6).round() as i64, (q.lat * 1e6).round() as i64);
        if let Some(&n) = cache.get(&key) {
            out.push(n);
            continue;
        }

        let p = g.to_m(*q);
        let (edge, t, proj, _dist) = g
            .nearest_edge(p)
            .ok_or_else(|| format!("点 ({:.6},{:.6}) 附近找不到道路", q.lat, q.lon))?;

        // 投影落在端点附近：直接复用端点，无需切分。
        if t <= EPS {
            let n = g.graph.edge_endpoints(edge).map(|(s, _)| s).unwrap();
            cache.insert(key, n);
            out.push(n);
            continue;
        }
        if t >= 1.0 - EPS {
            let n = g.graph.edge_endpoints(edge).map(|(_, t)| t).unwrap();
            cache.insert(key, n);
            out.push(n);
            continue;
        }

        // 落在中间：切分（含反向边）。
        let (s, tg) = g.graph.edge_endpoints(edge).unwrap();
        let data = g.graph[edge].clone();
        let coord = g.to_deg(proj);
        let v = g.add_node(coord);

        // 前向边 s→tg 切分为 s→v、v→tg。
        g.graph.remove_edge(edge);
        let len_sv = crate::graph::dist(g.to_m(g.node_coord(s)), proj);
        let len_vt = crate::graph::dist(proj, g.to_m(g.node_coord(tg)));
        g.add_edge(
            s,
            v,
            EdgeData {
                length_m: len_sv,
                ..data.clone()
            },
        );
        g.index_insert_edge(s, v);
        g.add_edge(
            v,
            tg,
            EdgeData {
                length_m: len_vt,
                ..data.clone()
            },
        );
        g.index_insert_edge(v, tg);

        // 反向边（若非单向）：tg→s 切分为 tg→v、v→s。
        if !data.oneway {
            if let Some(rev) = g.graph.find_edge(tg, s) {
                let rdata = g.graph[rev].clone();
                g.graph.remove_edge(rev);
                let len_tgv = crate::graph::dist(g.to_m(g.node_coord(tg)), proj);
                let len_vs = crate::graph::dist(proj, g.to_m(g.node_coord(s)));
                g.add_edge(
                    tg,
                    v,
                    EdgeData {
                        length_m: len_tgv,
                        ..rdata.clone()
                    },
                );
                g.index_insert_edge(tg, v);
                g.add_edge(
                    v,
                    s,
                    EdgeData {
                        length_m: len_vs,
                        ..rdata
                    },
                );
                g.index_insert_edge(v, s);
            }
        }

        cache.insert(key, v);
        out.push(v);
    }

    g.rebuild_index();
    Ok(out)
}
