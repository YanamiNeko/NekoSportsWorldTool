//! OSM `.osm`（XML）解析：节点 / 道路 way / 建筑 way+multipolygon relation。
//!
//! 输入为 openstreetmap.org 导出的标准 OSM XML（`<node>`/`<way>`/`<relation>`）。
//! 只抽取：`highway` 路径（剔除机动车道）、`building` 轮廓。

use std::collections::HashMap;

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::graph::{Coord, EdgeData, RoadGraph};

/// 一条道路 way。
pub struct OsmRoad {
    pub id: u64,
    pub node_refs: Vec<u64>,
    pub oneway: bool,
    pub oneway_reversed: bool,
    pub highway: String,
    pub maxspeed_kmh: Option<f64>,
}

/// 一个建筑：外环为节点 id 序列。
pub struct OsmBuilding {
    pub outer: Vec<Vec<u64>>,
}

pub struct ParsedOsm {
    pub nodes: HashMap<u64, Coord>,
    pub roads: Vec<OsmRoad>,
    pub buildings: Vec<OsmBuilding>,
}

/// 不应被当作可跑步道路的 highway 值。
fn is_motorized(h: &str) -> bool {
    matches!(
        h,
        "motorway"
            | "motorway_link"
            | "trunk"
            | "trunk_link"
            | "primary"
            | "primary_link"
            | "secondary"
            | "secondary_link"
            | "construction"
            | "proposed"
    )
}

fn parse_maxspeed(v: &str) -> Option<f64> {
    let digits: String = v.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let n: f64 = digits.parse().ok()?;
    if v.to_ascii_lowercase().contains("mph") {
        Some(n * 1.60934)
    } else {
        Some(n)
    }
}

struct Attrs {
    map: HashMap<String, String>,
}

impl Attrs {
    fn new() -> Self {
        Attrs { map: HashMap::new() }
    }
    fn insert(&mut self, k: &[u8], v: &[u8]) {
        self.map.insert(
            String::from_utf8_lossy(k).into_owned(),
            String::from_utf8_lossy(v).into_owned(),
        );
    }
    fn get(&self, k: &str) -> Option<&str> {
        self.map.get(k).map(|s| s.as_str())
    }
    fn id(&self) -> Option<u64> {
        self.get("id").and_then(|v| v.parse().ok())
    }
}

/// 解析 OSM XML 字节流。
pub fn parse_osm(bytes: &[u8]) -> Result<ParsedOsm, String> {
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(true);

    let mut nodes: HashMap<u64, Coord> = HashMap::new();
    let mut roads: Vec<OsmRoad> = Vec::new();
    let mut buildings: Vec<OsmBuilding> = Vec::new();

    // 所有 way 的节点序列（供 relation 建筑外环回填）。
    let mut way_nodes: HashMap<u64, Vec<u64>> = HashMap::new();
    // relation 建筑：收集 outer way id。
    let mut rel_buildings: Vec<Vec<u64>> = Vec::new();

    let mut cur_way: Option<(u64, Vec<u64>, Vec<(String, String)>)> = None;
    let mut cur_rel: Option<Vec<(String, String, String)>> = None;
    let mut cur_rel_tags: Vec<(String, String)> = Vec::new();

    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let local = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                let mut attrs = Attrs::new();
                for a in e.attributes().with_checks(false) {
                    let a = a.map_err(|e| format!("属性解析失败: {e}"))?;
                    attrs.insert(a.key.as_ref(), a.value.as_ref());
                }
                match local.as_str() {
                    "node" => {
                        if let (Some(id), Some(lat), Some(lon)) = (
                            attrs.id(),
                            attrs.get("lat").and_then(|v| v.parse().ok()),
                            attrs.get("lon").and_then(|v| v.parse().ok()),
                        ) {
                            nodes.insert(id, Coord::new(lon, lat));
                        }
                    }
                    "way" => {
                        cur_way = Some((attrs.id().unwrap_or(0), Vec::new(), Vec::new()));
                    }
                    "relation" => {
                        cur_rel = Some(Vec::new());
                        cur_rel_tags = Vec::new();
                    }
                    "nd" => {
                        if let Some((_, refs, _)) = cur_way.as_mut() {
                            if let Some(r) = attrs.get("ref").and_then(|v| v.parse().ok()) {
                                refs.push(r);
                            }
                        }
                    }
                    "tag" => {
                        let k = attrs.get("k").unwrap_or("").to_string();
                        let v = attrs.get("v").unwrap_or("").to_string();
                        if let Some((_, _, tags)) = cur_way.as_mut() {
                            tags.push((k.clone(), v.clone()));
                        }
                        if cur_rel.is_some() {
                            cur_rel_tags.push((k, v));
                        }
                    }
                    "member" => {
                        if let Some(members) = cur_rel.as_mut() {
                            let mtype = attrs.get("type").unwrap_or("").to_string();
                            let mref = attrs.get("ref").unwrap_or("").to_string();
                            let role = attrs.get("role").unwrap_or("").to_string();
                            members.push((mtype, mref, role));
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::End(e)) => {
                let local = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                match local.as_str() {
                    "way" => {
                        if let Some((id, refs, tags)) = cur_way.take() {
                            way_nodes.insert(id, refs.clone());
                            let is_building =
                                tags.iter().any(|(k, v)| k == "building" && v != "no");
                            if is_building && refs.len() >= 3 {
                                buildings.push(OsmBuilding { outer: vec![refs] });
                            } else if let Some(h) = tags
                                .iter()
                                .find(|(k, _)| k == "highway")
                                .map(|(_, v)| v.clone())
                            {
                                if !is_motorized(&h) && refs.len() >= 2 {
                                    let oneway = tags.iter().any(|(k, v)| {
                                        k == "oneway" && matches!(v.as_str(), "yes" | "true" | "1")
                                    });
                                    let oneway_reversed =
                                        tags.iter().any(|(k, v)| k == "oneway" && v == "-1");
                                    let maxspeed = tags
                                        .iter()
                                        .find(|(k, _)| k == "maxspeed")
                                        .and_then(|(_, v)| parse_maxspeed(v));
                                    roads.push(OsmRoad {
                                        id,
                                        node_refs: refs,
                                        oneway,
                                        oneway_reversed,
                                        highway: h,
                                        maxspeed_kmh: maxspeed,
                                    });
                                }
                            }
                        }
                    }
                    "relation" => {
                        if let Some(members) = cur_rel.take() {
                            let is_building = cur_rel_tags
                                .iter()
                                .any(|(k, v)| k == "building" && v != "no");
                            if is_building {
                                let outer: Vec<u64> = members
                                    .iter()
                                    .filter(|(mtype, _r, role)| mtype == "way" && role == "outer")
                                    .filter_map(|(_, r, _)| r.parse().ok())
                                    .collect();
                                if !outer.is_empty() {
                                    rel_buildings.push(outer);
                                }
                            }
                        }
                        cur_rel_tags = Vec::new();
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("OSM 解析失败: {e}")),
            _ => {}
        }
    }

    // 回填 relation 建筑外环（outer way 的节点序列）。
    for outer_ways in rel_buildings {
        let mut rings: Vec<Vec<u64>> = Vec::new();
        for wid in outer_ways {
            if let Some(refs) = way_nodes.get(&wid) {
                if refs.len() >= 3 {
                    rings.push(refs.clone());
                }
            }
        }
        if !rings.is_empty() {
            buildings.push(OsmBuilding { outer: rings });
        }
    }

    Ok(ParsedOsm { nodes, roads, buildings })
}

/// 用道路 way 构建路网图。
pub fn build_graph(parsed: &ParsedOsm) -> RoadGraph {
    let mut g = RoadGraph::new();
    let mut node_map: HashMap<u64, petgraph::graph::NodeIndex> = HashMap::new();

    for r in &parsed.roads {
        // 先解析节点 id → NodeIndex（避免闭包跨迭代借用）。
        let idxs: Vec<petgraph::graph::NodeIndex> = r
            .node_refs
            .iter()
            .filter_map(|id| {
                let c = *parsed.nodes.get(id)?;
                if let Some(&n) = node_map.get(id) {
                    Some(n)
                } else {
                    let n = g.add_node(c);
                    node_map.insert(*id, n);
                    Some(n)
                }
            })
            .collect();
        for w in idxs.windows(2) {
            let (p, next) = (w[0], w[1]);
            let a = g.node_coord(p);
            let b = g.node_coord(next);
            let len = geo_haversine(a, b);
            if !r.oneway_reversed {
                g.add_edge(p, next, EdgeData {
                    way_id: r.id,
                    oneway: r.oneway,
                    highway: r.highway.clone(),
                    maxspeed_kmh: r.maxspeed_kmh,
                    length_m: len,
                });
            }
            if !r.oneway {
                g.add_edge(next, p, EdgeData {
                    way_id: r.id,
                    oneway: r.oneway,
                    highway: r.highway.clone(),
                    maxspeed_kmh: r.maxspeed_kmh,
                    length_m: len,
                });
            }
        }
    }

    let mut bld: Vec<Vec<Coord>> = Vec::new();
    for b in &parsed.buildings {
        for ring in &b.outer {
            let coords: Vec<Coord> =
                ring.iter().filter_map(|id| parsed.nodes.get(id).copied()).collect();
            if coords.len() >= 3 {
                bld.push(coords);
            }
        }
    }
    g.buildings = bld;

    g.compute_anchor();
    g.rebuild_index();
    g
}

/// 完整流程：解析 OSM → 构建路网。
pub fn load_osm(bytes: &[u8]) -> Result<RoadGraph, String> {
    let parsed = parse_osm(bytes)?;
    if parsed.roads.is_empty() {
        return Err("OSM 中未找到可用的 highway 道路".into());
    }
    Ok(build_graph(&parsed))
}

/// 球面近似距离（米）。
fn geo_haversine(a: Coord, b: Coord) -> f64 {
    use geo::HaversineDistance;
    let pa = geo::Point::new(a.lon, a.lat);
    let pb = geo::Point::new(b.lon, b.lat);
    pa.haversine_distance(&pb)
}
