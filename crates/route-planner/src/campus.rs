//! Campus areas are independent of roads and never inferred from a POI or building.
use crate::Coord;
use geo::{Area, Contains, Intersects, Line, LineString, Polygon, TriangulateEarcut};
use quick_xml::{events::Event, Reader};
use std::collections::HashMap;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Campus {
    pub id: String,
    pub name: String,
    pub outer: Vec<Vec<Coord>>,
    pub inner: Vec<Vec<Coord>>,
}

fn line(r: &[Coord]) -> LineString<f64> {
    let mut p: Vec<_> = r.iter().map(|c| (c.lon, c.lat)).collect();
    if p.first() != p.last() {
        p.push(p[0]);
    }
    LineString::from(p)
}

fn valid_ring(r: &[Coord]) -> bool {
    if r.len() < 4
        || r.first() != r.last()
        || r.len() > 10000
        || r.iter().any(|c| {
            !c.lat.is_finite() || !c.lon.is_finite() || c.lat.abs() > 85. || c.lon.abs() > 180.
        })
    {
        return false;
    }
    let ls = line(r);
    let edges: Vec<Line<f64>> = ls.lines().collect();
    for (i, a) in edges.iter().enumerate() {
        if a.start == a.end {
            return false;
        }
        for (j, b) in edges.iter().enumerate().skip(i + 2) {
            if i == 0 && j == edges.len() - 1 {
                continue;
            }
            if a.intersects(b) {
                return false;
            }
        }
    }
    Polygon::new(ls, vec![]).unsigned_area() > 1e-12
}

impl Campus {
    /// Triangulation preserves holes while reusing whole-segment union checks.
    pub fn triangles(&self) -> Result<Vec<Vec<Coord>>, String> {
        if self.outer.is_empty() || self.outer.iter().chain(&self.inner).any(|r| !valid_ring(r)) {
            return Err("校园轮廓不闭合或几何无效".into());
        }
        let mut polys: Vec<_> = self
            .outer
            .iter()
            .map(|r| Polygon::new(line(r), vec![]))
            .collect();
        for i in 0..polys.len() {
            for j in i + 1..polys.len() {
                if polys[i].intersects(&polys[j]) {
                    return Err("校园外环重叠".into());
                }
            }
        }
        let holes: Vec<_> = self
            .inner
            .iter()
            .map(|r| Polygon::new(line(r), vec![]))
            .collect();
        for i in 0..holes.len() {
            for j in i + 1..holes.len() {
                if holes[i].intersects(&holes[j]) {
                    return Err("校园排除区域重叠".into());
                }
            }
        }
        for hole in holes {
            let Some(parent) = polys
                .iter_mut()
                .find(|p| p.contains(&hole) && !p.exterior().intersects(hole.exterior()))
            else {
                return Err("校园排除区域不在外环内".into());
            };
            parent.interiors_push(hole.exterior().clone());
        }
        let expected: f64 = polys.iter().map(Area::unsigned_area).sum();
        let triangles: Vec<_> = polys.iter().flat_map(|p| p.earcut_triangles()).collect();
        let actual: f64 = triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.to_array();
                ((b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y)).abs() * 0.5
            })
            .sum();
        if (expected - actual).abs() > expected * 1e-7 {
            return Err("校园区域分解失败".into());
        }
        Ok(triangles
            .iter()
            .map(|t| t.to_array().iter().map(|p| Coord::new(p.x, p.y)).collect())
            .collect())
    }
}

fn rings(
    ids: &[u64],
    ways: &HashMap<u64, Vec<u64>>,
    nodes: &HashMap<u64, Coord>,
) -> Option<Vec<Vec<Coord>>> {
    let mut parts: Vec<Vec<u64>> = ids
        .iter()
        .map(|id| ways.get(id).cloned())
        .collect::<Option<_>>()?;
    let mut result = Vec::new();
    while let Some(mut ring) = parts.pop() {
        while ring.first() != ring.last() {
            let tail = *ring.last()?;
            let i = parts
                .iter()
                .position(|p| p.first() == Some(&tail) || p.last() == Some(&tail))?;
            let mut next = parts.remove(i);
            if next.first() != Some(&tail) {
                next.reverse();
            }
            ring.extend(next.into_iter().skip(1));
        }
        result.push(
            ring.iter()
                .map(|id| nodes.get(id).copied())
                .collect::<Option<_>>()?,
        );
    }
    Some(result)
}

pub fn parse(bytes: &[u8]) -> Result<Vec<Campus>, String> {
    if bytes.len() > 24 * 1024 * 1024 {
        return Err("校园地图超过 24 MiB".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "地图编码无效")?;
    if !text.contains("</osm>") || text.contains("<remark") {
        return Err("校园地图响应不完整".into());
    }
    let mut reader = Reader::from_reader(bytes);
    let mut nodes = HashMap::new();
    let mut ways = HashMap::new();
    let mut candidates = Vec::new();
    let mut kind = String::new();
    let mut id = 0;
    let mut refs = Vec::new();
    let mut members = Vec::new();
    let mut tags = HashMap::new();
    loop {
        match reader.read_event().map_err(|e| e.to_string())? {
            Event::Start(e) | Event::Empty(e) => {
                let mut a = HashMap::new();
                for attr in e.attributes() {
                    let attr = attr.map_err(|e| e.to_string())?;
                    a.insert(
                        String::from_utf8_lossy(attr.key.as_ref()).into_owned(),
                        attr.decode_and_unescape_value(reader.decoder())
                            .map_err(|e| e.to_string())?
                            .into_owned(),
                    );
                }
                let get = |k: &str| a.get(k).map(String::as_str).unwrap_or("");
                match e.name().as_ref() {
                    b"node" => {
                        if let (Ok(id), Ok(lat), Ok(lon)) = (
                            get("id").parse::<u64>(),
                            get("lat").parse::<f64>(),
                            get("lon").parse::<f64>(),
                        ) {
                            nodes.insert(id, Coord::new(lon, lat));
                        }
                    }
                    b"way" | b"relation" => {
                        kind = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                        id = get("id").parse().unwrap_or(0);
                        refs.clear();
                        members.clear();
                        tags.clear();
                    }
                    b"nd" if kind == "way" => {
                        refs.push(get("ref").parse::<u64>().unwrap_or(0));
                    }
                    b"member" if kind == "relation" => {
                        members.push((
                            get("type").to_owned(),
                            get("ref").parse::<u64>().unwrap_or(0),
                            get("role").to_owned(),
                        ));
                    }
                    b"tag" if !kind.is_empty() => {
                        tags.insert(get("k").to_owned(), get("v").to_owned());
                    }
                    _ => {}
                }
            }
            Event::End(e) if e.name().as_ref() == b"way" || e.name().as_ref() == b"relation" => {
                if kind == "way" {
                    ways.insert(id, refs.clone());
                }
                let educational = tags
                    .get("amenity")
                    .is_some_and(|v| matches!(v.as_str(), "university" | "college" | "school"))
                    || tags.get("landuse").is_some_and(|v| v == "education");
                let building = tags.get("building").is_some_and(|v| v != "no");
                if educational && !building && id > 0 {
                    let name = tags
                        .get("name:zh")
                        .or_else(|| tags.get("name"))
                        .or_else(|| tags.get("operator"))
                        .cloned()
                        .unwrap_or_else(|| "未命名校园".into());
                    if kind == "way" {
                        candidates.push((format!("way/{id}"), name, vec![id], vec![]));
                    } else if tags.get("type").is_some_and(|v| v == "multipolygon")
                        && members
                            .iter()
                            .all(|m| m.0 == "way" && matches!(m.2.as_str(), "outer" | "inner" | ""))
                    {
                        candidates.push((
                            format!("relation/{id}"),
                            name,
                            members
                                .iter()
                                .filter(|m| m.2 != "inner")
                                .map(|m| m.1)
                                .collect(),
                            members
                                .iter()
                                .filter(|m| m.2 == "inner")
                                .map(|m| m.1)
                                .collect(),
                        ));
                    }
                }
                kind.clear();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let mut out = Vec::new();
    for (id, name, outer, inner) in candidates {
        if let (Some(outer), Some(inner)) =
            (rings(&outer, &ways, &nodes), rings(&inner, &ways, &nodes))
        {
            let c = Campus {
                id,
                name,
                outer,
                inner,
            };
            if c.triangles().is_ok() {
                out.push(c);
            }
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn square(a: f64, b: f64) -> Vec<Coord> {
        vec![
            Coord::new(a, a),
            Coord::new(b, a),
            Coord::new(b, b),
            Coord::new(a, b),
            Coord::new(a, a),
        ]
    }
    #[test]
    fn holes_and_crossings_survive_triangulation() {
        let c = Campus {
            id: "x".into(),
            name: "x".into(),
            outer: vec![square(0., 4.)],
            inner: vec![square(1., 2.)],
        };
        let t = c.triangles().unwrap();
        assert!(crate::boundary::contains(&t, Coord::new(0.5, 1.5)));
        assert!(!crate::boundary::contains(&t, Coord::new(1.5, 1.5)));
        assert!(!crate::boundary::segment_inside(
            &t,
            Coord::new(0.5, 1.5),
            Coord::new(3., 1.5)
        ));
        assert!(crate::boundary::segment_inside(
            &t,
            Coord::new(0.5, 0.5),
            Coord::new(3., 0.5)
        ));
    }
    #[test]
    fn invalid_regions_are_rejected() {
        let mut c = Campus {
            id: "x".into(),
            name: "x".into(),
            outer: vec![square(0., 4.)],
            inner: vec![square(3., 5.)],
        };
        assert!(c.triangles().is_err());
        c.inner.clear();
        c.outer[0].pop();
        assert!(c.triangles().is_err());
    }
}

#[cfg(test)]
mod parser_tests {
    use super::*;
    const XML: &str = r#"<osm><node id="1" lon="1" lat="1"/><node id="2" lon="3" lat="1"/><node id="3" lon="3" lat="3"/><node id="4" lon="1" lat="3"/><node id="5" lon="1.5" lat="1.5"/><node id="6" lon="2" lat="1.5"/><node id="7" lon="2" lat="2"/><node id="8" lon="1.5" lat="2"/><way id="10"><nd ref="1"/><nd ref="2"/><nd ref="3"/></way><way id="11"><nd ref="1"/><nd ref="4"/><nd ref="3"/></way><way id="12"><nd ref="5"/><nd ref="6"/><nd ref="7"/><nd ref="8"/><nd ref="5"/></way><relation id="20"><member type="way" ref="10" role="outer"/><member type="way" ref="11" role="outer"/><member type="way" ref="12" role="inner"/><tag k="type" v="multipolygon"/><tag k="amenity" v="college"/><tag k="name" v="A &amp; B"/></relation></osm>"#;
    #[test]
    fn stitches_reversed_members_and_preserves_holes() {
        let c = parse(XML.as_bytes()).unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].name, "A & B");
        assert_eq!(c[0].outer.len(), 1);
        assert_eq!(c[0].inner.len(), 1);
        assert!(!crate::boundary::contains(
            &c[0].triangles().unwrap(),
            Coord::new(1.75, 1.75)
        ));
    }
    #[test]
    fn missing_members_open_rings_and_buildings_are_not_campuses() {
        assert!(parse(
            XML.replace("ref=\"11\" role", "ref=\"999\" role")
                .as_bytes()
        )
        .unwrap()
        .is_empty());
        assert!(parse(
            XML.replace(
                "<tag k=\"amenity\"",
                "<tag k=\"building\" v=\"yes\"/><tag k=\"amenity\""
            )
            .as_bytes()
        )
        .unwrap()
        .is_empty());
        assert!(parse(b"<osm><node id=\"1\" lat=\"1\" lon=\"1\"><tag k=\"amenity\" v=\"university\"/></node></osm>").unwrap().is_empty());
        assert!(parse(b"<osm>").is_err());
    }
}

#[cfg(test)]
mod real_map_tests {
    #[test]
    fn chengdu_neusoft_public_boundaries_parse_as_separate_candidates() {
        let campuses =
            super::parse(include_bytes!("../tests/fixtures/chengdu-neusoft.osm")).unwrap();
        assert_eq!(campuses.len(), 2);
        for c in campuses {
            assert!(!c.triangles().unwrap().is_empty());
            assert_eq!(c.name, "成都东软学院");
        }
    }
}
