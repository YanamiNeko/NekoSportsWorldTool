//! Public-map boundaries are local planning constraints, never server metadata.
use crate::{location::Coordinate, task_control::Control};
use route_planner::campus::Campus;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Selection {
    pub school: String,
    pub id: String,
    pub revision: String,
}

fn legacy_revision(c: &Campus) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(c).unwrap_or_default())
    )
}

pub fn revision(c: &Campus) -> String {
    fn canonical(r: &[route_planner::Coord]) -> String {
        let mut points = r.to_vec();
        if points.first() == points.last() {
            points.pop();
        }
        fn rotate(points: &mut [route_planner::Coord]) {
            if let Some(index) = (0..points.len()).min_by(|&a, &b| {
                points[a]
                    .lon
                    .total_cmp(&points[b].lon)
                    .then(points[a].lat.total_cmp(&points[b].lat))
            }) {
                points.rotate_left(index);
            }
        }
        rotate(&mut points);
        let forward = serde_json::to_string(&points).unwrap_or_default();
        points.reverse();
        rotate(&mut points);
        let backward = serde_json::to_string(&points).unwrap_or_default();
        forward.min(backward)
    }
    let mut outer: Vec<_> = c.outer.iter().map(|r| canonical(r)).collect();
    outer.sort();
    let mut inner: Vec<_> = c.inner.iter().map(|r| canonical(r)).collect();
    inner.sort();
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(&c.id, &c.name, outer, inner)).unwrap_or_default())
    )
}

pub fn selected<'a>(
    items: &'a [Campus],
    choice: Option<&Selection>,
    school: &str,
) -> Option<&'a Campus> {
    let choice = choice.filter(|s| s.school == school)?;
    items.iter().find(|c| {
        c.id == choice.id
            && (revision(c) == choice.revision || legacy_revision(c) == choice.revision)
    })
}

pub fn planning_fences(
    official: &[Vec<(f64, f64)>],
    items: &[Campus],
    choice: Option<&Selection>,
    school: &str,
) -> Result<Vec<Vec<(f64, f64)>>, String> {
    if !official.is_empty() {
        return Ok(official.to_vec());
    }
    let Some(c) = selected(items, choice, school) else {
        return Ok(vec![]);
    };
    let mut aligned = c.clone();
    for p in aligned.outer.iter_mut().chain(&mut aligned.inner).flatten() {
        (p.lat, p.lon) = crate::track::geom::wgs84_to_bd09(p.lat, p.lon);
    }
    Ok(aligned
        .triangles()?
        .iter()
        .map(|r| r.iter().map(|c| (c.lat, c.lon)).collect())
        .collect())
}

/// The start is a routing input only; it never enters the server checkpoint list.
pub fn route_inputs(
    required: &[(f64, f64)],
    live: &[serde_json::Value],
    anchor: Coordinate,
) -> Vec<(f64, f64)> {
    let mut points = super::points::route_points(required, live);
    if points.is_empty() {
        points.push((anchor.latitude, anchor.longitude));
    }
    points
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Cache {
    loaded_ms: i64,
    items: Vec<Campus>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SearchName {
    pub school: String,
    pub name: String,
}

pub fn school_name(session: &super::model::Session, config: &super::model::Config) -> String {
    config
        .map_search_name
        .as_ref()
        .filter(|s| s.school == session.unid)
        .map(|s| s.name.clone())
        .or_else(|| {
            ["campusName", "schoolName", "universityName"]
                .iter()
                .find_map(|k| {
                    session
                        .profile
                        .get(k)
                        .and_then(|v| v.as_str())
                        .filter(|n| !n.trim().is_empty())
                        .map(str::to_owned)
                })
        })
        .unwrap_or_default()
}

fn regex_literal(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if ".^$*+?()[]{}|\\".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

pub(crate) fn school_base(name: &str) -> &str {
    let base = name
        .trim()
        .split(|c: char| c.is_whitespace() || "·（(".contains(c))
        .next()
        .unwrap_or(name);
    if base.ends_with("校区") || base.ends_with("分校") {
        if let Some(end) = ["大学", "学院", "学校"]
            .iter()
            .filter_map(|suffix| base.rfind(suffix).map(|i| i + suffix.len()))
            .max()
        {
            if end < base.len() {
                return &base[..end];
            }
        }
    }
    base
}

pub(crate) fn name_query(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.chars().count() < 2 || name.chars().count() > 100 || name.chars().any(char::is_control)
    {
        return Err("请输入 2—100 字的学校名称".into());
    }
    let mut names = vec![name.to_owned()];
    // Account school labels often append a campus after whitespace or parentheses.
    let base = school_base(name);
    if base != name && base.chars().count() >= 2 {
        names.push(base.to_owned());
    }
    // Search a sufficiently specific stem as well, to discover older map names.
    for suffix in ["学院", "大学"] {
        if let Some(stem) = base.strip_suffix(suffix) {
            if stem.chars().count() >= 3 {
                names.push(stem.to_owned());
            }
        }
    }
    names.sort();
    names.dedup();
    let pattern = format!(
        "({})",
        names
            .iter()
            .map(|s| regex_literal(s))
            .collect::<Vec<_>>()
            .join("|")
    );
    let quoted = serde_json::to_string(&pattern).map_err(|e| e.to_string())?;
    let mut query = String::from("[out:xml][timeout:25];(");
    for kind in ["way", "relation"] {
        for tag in [
            "[amenity~\"^(university|college|school)$\"]",
            "[landuse=education]",
        ] {
            query.push_str(&format!(
                "{kind}{tag}[~\"^(name|old_name|alt_name|official_name)(:.*)?$\"~{quoted}];"
            ));
        }
    }
    query.push_str(");(._;>>;);out body;");
    Ok(query)
}

fn cache_path(key: &str) -> std::path::PathBuf {
    crate::platform::data_dir()
        .join("map-boundaries")
        .join(format!("{:x}.json", Sha256::digest(key.as_bytes())))
}
fn read_cache(path: &std::path::Path, force: bool) -> Option<Vec<Campus>> {
    if force || !std::fs::metadata(path).is_ok_and(|m| m.len() < 24 * 1024 * 1024) {
        return None;
    }
    let cache: Cache = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    let age = crate::crypto::envelope::now_ms() - cache.loaded_ms;
    if (0..7 * 86400 * 1000).contains(&age)
        && !cache.items.is_empty()
        && cache.items.iter().all(|c| c.triangles().is_ok())
    {
        Some(cache.items)
    } else {
        None
    }
}
fn save_cache(path: &std::path::Path, items: &[Campus]) {
    if items.is_empty() {
        return;
    }
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    if let Ok(bytes) = serde_json::to_vec(&Cache {
        loaded_ms: crate::crypto::envelope::now_ms(),
        items: items.to_vec(),
    }) {
        let _ = std::fs::write(path, bytes);
    }
}
fn fetch_map(query: &str) -> Result<Vec<u8>, String> {
    #[cfg(test)]
    if crate::platform::TEST_DATA_DIR.with(|p| p.borrow().is_some()) {
        return Ok(b"<osm></osm>".to_vec());
    }
    super::school_roads::download(query)
}
fn named_with_fetch(
    name: &str,
    control: &Control,
    fetch: impl FnOnce(&str) -> Result<Vec<u8>, String>,
) -> Result<Vec<Campus>, String> {
    control.check()?;
    let query = name_query(name)?;
    let data = fetch(&query)?;
    control.check()?;
    route_planner::campus::parse(&data)
}

pub fn lookup_named(
    school: &str,
    name: &str,
    force: bool,
    control: &Control,
) -> Result<Vec<Campus>, String> {
    control.check()?;
    name_query(name)?;
    let path = cache_path(&format!("named-v2|{school}|{}", name.trim()));
    if let Some(items) = read_cache(&path, force) {
        return Ok(items);
    }
    let items = named_with_fetch(name, control, fetch_map)?;
    save_cache(&path, &items);
    Ok(items)
}

pub fn lookup_user_requested(
    school: &str,
    name: &str,
    endpoint: &str,
    control: &Control,
) -> Result<Vec<Campus>, String> {
    let primary = lookup_named(school, name, true, control);
    if primary.as_ref().is_ok_and(|items| !items.is_empty()) {
        return primary;
    }
    control.check()?;
    if endpoint.trim().is_empty() {
        return primary;
    }
    #[cfg(test)]
    if crate::platform::TEST_DATA_DIR.with(|p| p.borrow().is_some()) {
        return primary;
    }
    match super::map_search::lookup(name, endpoint, control) {
        Ok(items) if !items.is_empty() => {
            save_cache(
                &cache_path(&format!("named-v2|{school}|{}", name.trim())),
                &items,
            );
            Ok(items)
        }
        Ok(_) => primary,
        Err(e) => Err(format!(
            "主查询：{}；备用查询：{e}",
            primary.err().unwrap_or_else(|| "未找到校园区域".into())
        )),
    }
}

/// Called by the user's automatic-map refresh. Reuses the configured,
/// throttled school-search service only after the primary discovery fails.
pub fn discover_with_backup(school: &str, name: &str, anchor: Coordinate, force: bool,
    endpoint: &str, control: &Control) -> Result<Vec<Campus>, String> {
    let primary = discover(school, name, anchor, force, control);
    if primary.as_ref().is_ok_and(|items| !items.is_empty()) || name.trim().is_empty() || endpoint.trim().is_empty() {
        return primary;
    }
    control.check()?;
    #[cfg(test)]
    if crate::platform::TEST_DATA_DIR.with(|p| p.borrow().is_some()) { return primary; }
    eprintln!("[map-service] campus search switching to configured backup");
    let items = super::map_search::lookup(name, endpoint, control)
        .map_err(|e| format!("主查询：{}；备用校名搜索：{e}", primary.as_ref().err().map(String::as_str).unwrap_or("未找到边界")))?;
    control.check()?;
    if items.is_empty() { return primary; }
    save_cache(&cache_path(&format!("named-v2|{school}|{}", name.trim())), &items);
    eprintln!("[map-service] campus backup returned {} candidates", items.len());
    Ok(items)
}

pub fn discover(
    school: &str,
    name: &str,
    anchor: Coordinate,
    force: bool,
    control: &Control,
) -> Result<Vec<Campus>, String> {
    control.check()?;
    anchor.validate()?;
    let (lat, lon) = super::school_roads::to_wgs(anchor.latitude, anchor.longitude);
    let path = cache_path(&format!(
        "nearby-v2|{school}|{}|{lat:.5}|{lon:.5}",
        name.trim()
    ));
    if let Some(items) = read_cache(&path, force) {
        return Ok(items);
    }
    if !name.trim().is_empty() {
        let items = lookup_named(school, name, force, control)?;
        if !items.is_empty() {
            return Ok(items);
        }
    }
    let q=format!("[out:xml][timeout:25];(way[amenity~\"^(university|college|school)$\"](around:5000,{lat:.7},{lon:.7});relation[amenity~\"^(university|college|school)$\"](around:5000,{lat:.7},{lon:.7});way[landuse=education](around:5000,{lat:.7},{lon:.7});relation[landuse=education](around:5000,{lat:.7},{lon:.7}););(._;>>;);out body;");
    let bytes = fetch_map(&q)?;
    control.check()?;
    let items = route_planner::campus::parse(&bytes)?;
    save_cache(&path, &items);
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn campus() -> Campus {
        let c = route_planner::Coord::new;
        Campus {
            id: "way/1".into(),
            name: "测试校区".into(),
            outer: vec![vec![
                c(116., 39.),
                c(116.01, 39.),
                c(116.01, 39.01),
                c(116., 39.01),
                c(116., 39.),
            ]],
            inner: vec![],
        }
    }
    #[test]
    fn selection_is_bound_to_school_and_geometry() {
        let c = campus();
        let s = Selection {
            school: "a".into(),
            id: c.id.clone(),
            revision: revision(&c),
        };
        assert!(selected(&[c.clone()], Some(&s), "a").is_some());
        assert!(selected(&[c.clone()], Some(&s), "b").is_none());
        let mut changed = c.clone();
        changed.outer[0][1].lon += 0.001;
        assert!(selected(&[changed], Some(&s), "a").is_none());
        let official = vec![vec![(1., 1.), (1., 2.), (2., 1.)]];
        assert_eq!(
            planning_fences(&official, &[c], Some(&s), "a").unwrap(),
            official
        );
    }
    #[test]
    fn absent_points_only_supply_a_local_start() {
        let a = Coordinate::new(39., 116., 0.).unwrap();
        assert_eq!(route_inputs(&[], &[], a), vec![(39., 116.)]);
        assert_eq!(route_inputs(&[(40., 117.)], &[], a), vec![(40., 117.)]);
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    #[test]
    fn cache_is_scoped_to_school_location_and_age() {
        let dir = std::env::temp_dir().join(format!("boundary-cache-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("map-boundaries")).unwrap();
        crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = Some(dir.clone()));
        let anchor = Coordinate::new(39., 116., 0.).unwrap();
        let (lat, lon) = super::super::school_roads::to_wgs(39., 116.);
        let key = format!(
            "{:x}",
            Sha256::digest(format!("nearby-v2|a||{lat:.5}|{lon:.5}").as_bytes())
        );
        let path = dir.join("map-boundaries").join(format!("{key}.json"));
        let items = route_planner::campus::parse(include_bytes!(
            "../../crates/route-planner/tests/fixtures/chengdu-neusoft.osm"
        ))
        .unwrap();
        let mut cache = Cache {
            loaded_ms: crate::crypto::envelope::now_ms(),
            items,
        };
        std::fs::write(&path, serde_json::to_vec(&cache).unwrap()).unwrap();
        let control = Control::default();
        assert_eq!(discover("a", "", anchor, false, &control).unwrap().len(), 2);
        assert!(discover("b", "", anchor, false, &control)
            .unwrap()
            .is_empty());
        assert!(discover(
            "a",
            "",
            Coordinate::new(40., 116., 0.).unwrap(),
            false,
            &control
        )
        .unwrap()
        .is_empty());
        assert!(discover("a", "", anchor, true, &control)
            .unwrap()
            .is_empty());
        cache.loaded_ms -= 8 * 86400 * 1000;
        std::fs::write(&path, serde_json::to_vec(&cache).unwrap()).unwrap();
        assert!(discover("a", "", anchor, false, &control)
            .unwrap()
            .is_empty());
        control.cancel();
        assert!(discover("a", "", anchor, false, &control).is_err());
        crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = None);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod lookup_tests {
    use super::*;
    #[test]
    fn school_label_searches_full_name_stem_and_aliases_without_anchor_radius() {
        let q = name_query("肇庆医学院 鼎湖校区").unwrap();
        assert!(q.contains("肇庆医学院 鼎湖校区"));
        assert!(q.contains("肇庆医|"));
        assert!(q.contains("old_name|alt_name"));
        assert!(!q.contains("around:"));
        assert!(!q.contains("node["));
        assert!(name_query("x").is_err());
        assert!(name_query("学校\n注入").is_err());
        let escaped = name_query("A.*大学").unwrap();
        assert!(escaped.contains(r"A\\.\\*大学"));
    }
    #[test]
    fn named_lookup_preserves_errors_and_validates_the_response() {
        let control = Control::default();
        assert_eq!(
            named_with_fetch("测试大学", &control, |_| Err("timeout".into())).unwrap_err(),
            "timeout"
        );
        assert!(named_with_fetch("测试大学", &control, |_| Ok(
            b"<osm><remark>timeout</remark></osm>".to_vec()
        ))
        .is_err());
        assert!(
            named_with_fetch("测试大学", &control, |_| Ok(b"<osm></osm>".to_vec()))
                .unwrap()
                .is_empty()
        );
        let items = named_with_fetch("成都东软学院", &control, |q| {
            assert!(!q.contains("around:"));
            Ok(
                include_bytes!("../../crates/route-planner/tests/fixtures/chengdu-neusoft.osm")
                    .to_vec(),
            )
        })
        .unwrap();
        assert_eq!(items.len(), 2);
        control.cancel();
        assert!(named_with_fetch("测试大学", &control, |_| panic!(
            "cancelled query must not fetch"
        ))
        .is_err());
    }
    #[test]
    fn empty_cache_never_masks_a_new_search() {
        let dir = std::env::temp_dir().join(format!("empty-campus-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cache.json");
        save_cache(&path, &[]);
        assert!(!path.exists());
        std::fs::write(
            &path,
            serde_json::to_vec(&Cache {
                loaded_ms: crate::crypto::envelope::now_ms(),
                items: vec![],
            })
            .unwrap(),
        )
        .unwrap();
        assert!(read_cache(&path, false).is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
