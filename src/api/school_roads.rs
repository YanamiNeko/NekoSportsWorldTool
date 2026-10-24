//! School-scoped public OSM download. No account credentials are sent to Overpass.
use route_planner::RoadGraph;
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path, time::Duration};

const MAX_BYTES: u64 = 24 * 1024 * 1024;
const TTL_MS: i64 = 7 * 24 * 60 * 60 * 1000;
const ENDPOINT: &str = "https://overpass-api.de/api/interpreter";

pub fn validate_fences(fences: &[Vec<(f64, f64)>]) -> Result<(), String> {
    if fences.is_empty() {
        return Err("当前学校没有有效围栏，无法保证校内路线；请先确认学校围栏".into());
    }
    for ring in fences {
        if ring.len() < 3
            || ring.iter().any(|&(lat, lon)| {
                !lat.is_finite() || !lon.is_finite() || lat.abs() > 85.0 || lon.abs() > 180.0
            })
        {
            return Err("学校围栏坐标无效".into());
        }
        let (oy, ox) = ring[0];
        let area: f64 = ring
            .iter()
            .zip(ring.iter().cycle().skip(1))
            .map(|(&(y, x), &(v, u))| (x - ox) * (v - oy) - (u - ox) * (y - oy))
            .sum();
        if area.abs() < 1e-12 {
            return Err("学校围栏面积无效".into());
        }
    }
    Ok(())
}

// Invert the same forward transform used to align roads, avoiding mixed coordinate systems.
pub fn to_wgs(lat: f64, lon: f64) -> (f64, f64) {
    let (mut y, mut x) = (lat, lon);
    for _ in 0..16 {
        let (by, bx) = crate::track::geom::wgs84_to_bd09(y, x);
        y += lat - by;
        x += lon - bx;
    }
    (y, x)
}

fn query(fences: &[Vec<(f64, f64)>]) -> Result<String, String> {
    validate_fences(fences)?;
    query_coordinates(&fences.iter().flatten().copied().collect::<Vec<_>>(), 0.001)
}

fn query_coordinates(points: &[(f64, f64)], margin: f64) -> Result<String, String> {
    if points.is_empty()
        || points
            .iter()
            .any(|&(a, b)| !a.is_finite() || !b.is_finite() || a.abs() > 85. || b.abs() > 180.)
    {
        return Err("缺少有效点位，不能确定路网下载范围".into());
    }
    let (mut south, mut west, mut north, mut east) = (90f64, 180f64, -90f64, -180f64);
    for &(lat, lon) in points {
        let (y, x) = to_wgs(lat, lon);
        south = south.min(y);
        north = north.max(y);
        west = west.min(x);
        east = east.max(x);
    }
    // Reject unexpectedly huge / geographically separate school areas before downloading.
    if north - south > 0.15 || east - west > 0.2 {
        return Err("路网下载范围跨度过大，请确认当前学校和点位后重试".into());
    }
    let bbox = format!(
        "{:.7},{:.7},{:.7},{:.7}",
        south - margin,
        west - margin,
        north + margin,
        east + margin
    );
    Ok(format!("[out:xml][timeout:25];(way[highway]({bbox});way[building]({bbox});relation[building]({bbox}););(._;>;);out body;"))
}

fn parse(bytes: &[u8]) -> Result<RoadGraph, String> {
    if bytes.len() as u64 > MAX_BYTES {
        return Err("学校路网响应超过 24 MiB".into());
    }
    let xml = std::str::from_utf8(bytes).map_err(|_| "学校路网不是有效 XML")?;
    if !xml.contains("<osm") || !xml.contains("</osm>") || xml.contains("<remark") {
        return Err("路网服务返回错误或不完整数据，请稍后重试".into());
    }
    let net = route_planner::load_osm(bytes)?;
    if net.graph.edge_count() == 0
        || net.graph.node_weights().any(|n| {
            !n.coord.lon.is_finite()
                || !n.coord.lat.is_finite()
                || n.coord.lon.abs() > 180.0
                || n.coord.lat.abs() > 90.0
        })
    {
        return Err("当前学校没有可用的 OSM 道路数据，可手动导入完整路网".into());
    }
    Ok(net)
}

const BACKUP_ENDPOINT: &str = "https://api.openstreetmap.org/api/0.6/map";

// Convert only our bounded road queries; arbitrary/name queries must use
// the dedicated school-search fallback rather than inventing an extent.
fn official_bbox(query: &str) -> Result<String, String> {
    let raw = query.split_once("way[highway](").and_then(|(_, tail)| tail.split_once(')').map(|v| v.0))
        .ok_or("此查询需使用校名备用搜索")?;
    let values: Vec<f64> = raw.split(',').map(str::parse).collect::<Result<_, _>>()
        .map_err(|_| "无效路网范围")?;
    if values.len() != 4 || values.iter().any(|v| !v.is_finite()) { return Err("无效路网范围".into()); }
    let (s, w, n, e) = (values[0], values[1], values[2], values[3]);
    if s < -85. || n > 85. || w < -180. || e > 180. || s >= n || w >= e
        || (n-s)*(e-w) > 0.01 { return Err("备用地图仅支持小范围校园查询".into()); }
    Ok(format!("{w},{s},{e},{n}"))
}

// Only transport/server failures are retryable; do not evade rate limits or
// repeat invalid queries. Each endpoint is tried at most once, sequentially.
fn with_backup<T>(mut fetch: impl FnMut(&str) -> Result<T, (String, bool)>) -> Result<T, String> {
    match fetch(ENDPOINT) {
        Ok(value) => Ok(value),
        Err((primary, false)) => Err(primary),
        Err((primary, true)) => {
            eprintln!("[map-service] primary failed, switching to backup: {primary}");
            fetch(BACKUP_ENDPOINT)
                .map_err(|(backup, _)| format!("主地图服务：{primary}；备用地图服务：{backup}"))
        },
    }
}

pub(crate) fn download(query: &str) -> Result<Vec<u8>, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(8))
        .timeout(Duration::from_secs(35))
        .build();
    with_backup(|endpoint| {
        eprintln!("[map-service] requesting {endpoint}");
        let request = if endpoint == BACKUP_ENDPOINT {
            let bbox = official_bbox(query).map_err(|e| (e, false))?;
            agent.get(endpoint).query("bbox", &bbox)
                .set("User-Agent", "NekoSportsWorldTool/0.3.0 (school-road-map)").call()
        } else {
            agent.post(endpoint).set("User-Agent", "NekoSportsWorldTool/0.3.0 (school-road-map)")
                .send_form(&[("data", query)])
        };
        let response = request
            .map_err(|e| {
                let retry = match &e {
                    ureq::Error::Transport(_) => true,
                    ureq::Error::Status(code, _) => matches!(code, 408 | 500 | 502 | 503 | 504),
                };
                (format!("自动获取地图失败：{e}"), retry)
            })?;
        let mut bytes = Vec::new();
        response.into_reader().take(MAX_BYTES + 1).read_to_end(&mut bytes)
            .map_err(|e| (format!("读取地图失败：{e}"), true))?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(("地图响应超过 24 MiB".into(), false));
        }
        eprintln!("[map-service] received {} bytes from {endpoint}", bytes.len());
        Ok(bytes)
    })
}

#[cfg(test)]
mod fallback_tests {
    use super::*;
    #[test]
    fn bounded_query_converts_coordinate_order_and_rejects_large_or_named_queries() {
        assert_eq!(official_bbox("way[highway](23.20,112.57,23.22,112.59)").unwrap(), "112.57,23.2,112.59,23.22");
        for query in ["way[name=test]", "way[highway](0,0,5,5)", "way[highway](NaN,0,1,1)", "way[highway](2,0,1,1)"] {
            assert!(official_bbox(query).is_err());
        }
    }
    #[test]
    fn primary_success_never_calls_backup() {
        let mut calls = vec![];
        assert_eq!(with_backup(|endpoint| { calls.push(endpoint.to_owned()); Ok(42) }).unwrap(), 42);
        assert_eq!(calls, vec![ENDPOINT]);
    }
    #[test]
    fn timeout_uses_backup_once_for_the_same_query() {
        let mut calls = vec![];
        let result = with_backup(|endpoint| {
            calls.push(endpoint.to_owned());
            if endpoint == ENDPOINT { Err(("timeout".into(), true)) } else { Ok(b"<osm/>".to_vec()) }
        });
        assert_eq!(result.unwrap(), b"<osm/>");
        assert_eq!(calls, vec![ENDPOINT, BACKUP_ENDPOINT]);
    }
    #[test]
    fn both_failures_are_reported_without_retry_loop() {
        let mut calls = 0;
        let error = with_backup::<()>(|endpoint| { calls += 1; Err((endpoint.into(), true)) }).unwrap_err();
        assert_eq!(calls, 2);
        assert!(error.contains(ENDPOINT) && error.contains(BACKUP_ENDPOINT));
    }
    #[test]
    fn rate_limit_does_not_switch_servers() {
        let mut calls = 0;
        assert!(with_backup::<()>(|_| { calls += 1; Err(("HTTP 429".into(), false)) }).is_err());
        assert_eq!(calls, 1);
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Cache {
    key: String,
    loaded_ms: i64,
    xml: String,
    #[serde(default)]
    school: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    fences: Vec<Vec<(f64, f64)>>,
    #[serde(default)]
    points: Vec<(f64, f64)>,
}

fn cached_network(
    dir: &Path,
    school: &str,
    fences: &[Vec<(f64, f64)>],
    refresh: bool,
    now: i64,
    fetch: impl FnOnce(&str) -> Result<Vec<u8>, String>,
    log: &mut dyn FnMut(&str),
) -> Result<RoadGraph, String> {
    cached_route_network(dir, school, fences, &[], refresh, now, fetch, log)
}

fn cached_route_network(
    dir: &Path,
    school: &str,
    fences: &[Vec<(f64, f64)>],
    points: &[(f64, f64)],
    refresh: bool,
    now: i64,
    fetch: impl FnOnce(&str) -> Result<Vec<u8>, String>,
    log: &mut dyn FnMut(&str),
) -> Result<RoadGraph, String> {
    if school.trim().is_empty() {
        return Err("登录信息缺少学校编号，请重新登录".into());
    }
    let q = if fences.is_empty() {
        query_coordinates(points, 0.002)?
    } else {
        query(fences)?
    };
    let key = route_scope_key(school, fences, points);
    let file = dir.join(format!("{key}.json"));
    if !refresh
        && std::fs::metadata(&file)
            .map(|m| m.len() < MAX_BYTES * 2)
            .unwrap_or(false)
    {
        if let Ok(text) = std::fs::read_to_string(&file) {
            if let Ok(cache) = serde_json::from_str::<Cache>(&text) {
                if cache.key == key && (0..TTL_MS).contains(&(now - cache.loaded_ms)) {
                    if let Ok(net) = parse(cache.xml.as_bytes()) {
                        log("[road] 使用当前学校及下载范围对应的路网缓存");
                        return Ok(net);
                    }
                }
            }
        }
    }
    log("[road] 正在下载当前学校区域的 OpenStreetMap 路网…");
    let bytes = fetch(&q)?;
    let net = parse(&bytes)?;
    // Reject empty in-school coverage before accepting a download into the cache.
    let mut aligned = net.clone();
    crate::track::generate_road::align_network(&mut aligned);
    if crate::track::generate_road::apply_fences(&aligned, fences)
        .graph
        .edge_count()
        == 0
    {
        return Err("下载的路网在学校围栏内没有可用道路，不能向校外补足距离".into());
    }
    let cache = Cache {
        key,
        loaded_ms: now,
        xml: String::from_utf8(bytes).map_err(|e| e.to_string())?,
        school: school.into(),
        name: String::new(),
        fences: fences.to_vec(),
        points: points.to_vec(),
    };
    // Cache is optional: read-only installation directories must not block a valid download.
    let saved = (|| -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        // Interrupted/concurrent writes are rejected by the parser on the next read.
        std::fs::write(
            &file,
            serde_json::to_vec(&cache).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })();
    if let Err(e) = saved {
        log(&format!("[road] 本次路网可用，但缓存未保存：{e}"));
    }
    Ok(net)
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub key: String,
    pub school: String,
    pub name: String,
    pub loaded_ms: i64,
    pub bytes: u64,
    pub fences: Vec<Vec<(f64, f64)>>,
    pub points: Vec<(f64, f64)>,
}
pub fn scope_key(school: &str, fences: &[Vec<(f64, f64)>]) -> String {
    // About centimetre precision: stable across JSON floating-point round trips.
    let normalized: Vec<Vec<(i64, i64)>> = fences
        .iter()
        .map(|f| {
            f.iter()
                .map(|&(a, b)| ((a * 1e7).round() as i64, (b * 1e7).round() as i64))
                .collect()
        })
        .collect();
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(school, normalized)).unwrap())
    )
}

fn route_scope_key(school: &str, fences: &[Vec<(f64, f64)>], points: &[(f64, f64)]) -> String {
    if !fences.is_empty() {
        return scope_key(school, fences);
    }
    let normalized: Vec<_> = points
        .iter()
        .map(|&(a, b)| ((a * 1e7).round() as i64, (b * 1e7).round() as i64))
        .collect();
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&("points-v1", school, normalized)).unwrap())
    )
}
fn cache_dir() -> std::path::PathBuf {
    crate::platform::data_dir().join("school_roads")
}
fn cache_path(key: &str) -> Result<std::path::PathBuf, String> {
    if key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("无效缓存标识".into());
    }
    Ok(cache_dir().join(format!("{key}.json")))
}
pub fn entries() -> Vec<Entry> {
    let Ok(files) = std::fs::read_dir(cache_dir()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for f in files.flatten() {
        let Some(key) = f
            .path()
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_owned)
        else {
            continue;
        };
        if let Ok((cache, bytes)) = read_cache_record(&key) {
            out.push(cache_entry(&cache, bytes));
        }
    }
    out.sort_by(|a, b| b.loaded_ms.cmp(&a.loaded_ms));
    out
}
fn read_cache_record(key: &str) -> Result<(Cache, u64), String> {
    let path = cache_path(key)?;
    let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    if meta.len() > MAX_BYTES * 2 {
        return Err("缓存过大".into());
    }
    let cache: Cache = serde_json::from_slice(&std::fs::read(&path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if cache.key != key {
        return Err("缓存标识不一致".into());
    }
    Ok((cache, meta.len()))
}
fn cache_entry(c: &Cache, bytes: u64) -> Entry {
    Entry {
        key: c.key.clone(),
        school: c.school.clone(),
        name: c.name.clone(),
        loaded_ms: c.loaded_ms,
        bytes,
        fences: c.fences.clone(),
        points: c.points.clone(),
    }
}
pub fn read_cache(key: &str) -> Result<(RoadGraph, Entry), String> {
    let (c, bytes) = read_cache_record(key)?;
    Ok((parse(c.xml.as_bytes())?, cache_entry(&c, bytes)))
}
pub fn clear_cache(key: &str) -> Result<(), String> {
    std::fs::remove_file(cache_path(key)?).map_err(|e| e.to_string())
}
pub fn save_context(school: &str, name: &str, fences: &[Vec<(f64, f64)>], points: &[(f64, f64)]) {
    let key = route_scope_key(school, fences, points);
    let Ok(path) = cache_path(&key) else {
        return;
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return;
    };
    if let Ok(mut cache) = serde_json::from_slice::<Cache>(&bytes) {
        cache.name = name.into();
        cache.points = points.to_vec();
        if let Ok(bytes) = serde_json::to_vec(&cache) {
            let _ = std::fs::write(path, bytes);
        }
    }
}

pub fn load_for_school_cancellable(
    path: &str,
    school: &str,
    fences: &[Vec<(f64, f64)>],
    refresh: bool,
    control: &crate::task_control::Control,
    log: &mut dyn FnMut(&str),
) -> Result<RoadGraph, String> {
    control.check()?;
    if !path.trim().is_empty() {
        let net = crate::track::generate_road::load_network_path(path)?;
        control.check()?;
        return Ok(net);
    }
    validate_fences(fences)?;
    cached_network(
        &cache_dir(),
        school,
        fences,
        refresh,
        crate::crypto::envelope::now_ms(),
        |q| {
            control.check()?;
            let bytes = download(q)?;
            control.check()?;
            Ok(bytes)
        },
        log,
    )
}

/// With no server boundary, download near real school checkpoints. This extent
/// is only a public-map query, never a campus fence or a route constraint.
pub fn load_for_route_cancellable(
    path: &str,
    school: &str,
    fences: &[Vec<(f64, f64)>],
    points: &[(f64, f64)],
    refresh: bool,
    control: &crate::task_control::Control,
    log: &mut dyn FnMut(&str),
) -> Result<RoadGraph, String> {
    if !path.trim().is_empty() || !fences.is_empty() {
        return load_for_school_cancellable(path, school, fences, refresh, control, log);
    }
    control.check()?;
    log("[road] 无学校围栏：按当前学校真实点位获取附近道路；下载范围不是学校边界");
    cached_route_network(
        &cache_dir(),
        school,
        fences,
        points,
        refresh,
        crate::crypto::envelope::now_ms(),
        |q| {
            control.check()?;
            let bytes = download(q)?;
            control.check()?;
            Ok(bytes)
        },
        log,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Vec<Vec<(f64, f64)>>, Vec<u8>) {
        let fences = vec![vec![
            (39.899, 116.399),
            (39.899, 116.403),
            (39.903, 116.403),
            (39.903, 116.399),
        ]
        .into_iter()
        .map(|(a, b)| crate::track::geom::wgs84_to_bd09(a, b))
        .collect()];
        let xml=br#"<osm version="0.6"><node id="1" lat="39.9" lon="116.4"/><node id="2" lat="39.901" lon="116.401"/><way id="1"><nd ref="1"/><nd ref="2"/><tag k="highway" v="footway"/></way></osm>"#.to_vec();
        (fences, xml)
    }
    #[test]
    fn coordinate_round_trip_and_query_scope() {
        for p in [(39.9, 116.4), (22.5, 113.9), (31.2, 121.5)] {
            let bd = crate::track::geom::wgs84_to_bd09(p.0, p.1);
            let w = to_wgs(bd.0, bd.1);
            assert!((w.0 - p.0).abs() < 1e-8 && (w.1 - p.1).abs() < 1e-8);
        }
        let q = query(&fixture().0).unwrap();
        assert!(q.contains("39.8980000,116.3980000,39.9040000,116.4040000"));
        assert!(query(&[]).is_err());
        assert!(validate_fences(&[vec![(0., 0.); 3]]).is_err());
    }
    #[test]
    fn cache_is_school_and_fence_scoped_expires_and_propagates_failure() {
        let dir = std::env::temp_dir().join(format!("school-roads-{}", uuid::Uuid::new_v4()));
        let (f, xml) = fixture();
        let get =
            |school: &str, fences: &[Vec<(f64, f64)>], refresh, now, bytes: Option<Vec<u8>>| {
                cached_network(
                    &dir,
                    school,
                    fences,
                    refresh,
                    now,
                    |_| bytes.ok_or("offline".into()),
                    &mut |_| {},
                )
            };
        assert!(get("a", &f, false, 1000, Some(xml.clone())).is_ok());
        assert!(get("a", &f, false, 1001, None).is_ok());
        assert!(get("b", &f, false, 1001, None).is_err());
        let mut changed = f.clone();
        changed[0][0].0 -= 0.00001;
        assert!(get("a", &changed, false, 1001, None).is_err());
        assert!(get("a", &f, false, 1000 + TTL_MS, None).is_err());
        assert!(get("a", &f, false, 999, None).is_err());
        assert!(get("a", &f, true, 1001, None).is_err());
        assert!(get("a", &f, true, 1002, Some(xml)).is_ok());
        assert!(get("a", &f, false, 1002 + TTL_MS - 1, None).is_ok());
        assert!(parse(b"<osm></osm>").is_err());
        assert!(parse(b"<osm><remark>timeout</remark></osm>").is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn no_boundary_download_is_point_scoped_without_inventing_fences() {
        let dir = std::env::temp_dir().join(format!("point-roads-{}", uuid::Uuid::new_v4()));
        let (_, xml) = fixture();
        let points = [(39.9, 116.4), (39.901, 116.401)]
            .map(|(a, b)| crate::track::geom::wgs84_to_bd09(a, b));
        let net = cached_route_network(
            &dir,
            "a",
            &[],
            &points,
            false,
            1000,
            |q| {
                assert!(q.contains("39.8980000,116.3980000,39.9030000,116.4030000"));
                Ok(xml)
            },
            &mut |_| {},
        )
        .unwrap();
        assert!(net.graph.edge_count() > 0);
        assert!(net.boundaries.is_empty());
        let key = route_scope_key("a", &[], &points);
        let cache: Cache =
            serde_json::from_slice(&std::fs::read(dir.join(format!("{key}.json"))).unwrap())
                .unwrap();
        assert!(cache.fences.is_empty());
        assert_eq!(cache.points.len(), 2);
        let cached = |school, pts: &[(f64, f64)], now| {
            cached_route_network(
                &dir,
                school,
                &[],
                pts,
                false,
                now,
                |_| Err("offline".into()),
                &mut |_| {},
            )
        };
        assert!(cached("a", &points, 1001).is_ok());
        assert!(cached("b", &points, 1001).is_err());
        assert!(cached("a", &points[..1], 1001).is_err());
        assert!(cached("a", &points, 1000 + TTL_MS).is_err());
        assert!(query_coordinates(&[], 0.002).is_err());
        assert!(query_coordinates(&[(f64::NAN, 116.4)], 0.002).is_err());
        assert!(query_coordinates(&[(39., 116.), (40., 116.)], 0.002).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn manual_map_loading_needs_neither_boundary_nor_school_id() {
        let path = std::env::temp_dir().join(format!("manual-road-{}.osm", uuid::Uuid::new_v4()));
        std::fs::write(&path, fixture().1).unwrap();
        let net = load_for_route_cancellable(
            path.to_str().unwrap(),
            "",
            &[],
            &[],
            false,
            &crate::task_control::Control::default(),
            &mut |_| {},
        )
        .unwrap();
        assert!(net.graph.edge_count() > 0);
        assert!(net.boundaries.is_empty());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn saved_maps_can_be_listed_loaded_offline_and_deleted_safely() {
        let root = std::env::temp_dir().join(format!("school-manager-{}", uuid::Uuid::new_v4()));
        crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = Some(root.clone()));
        let (fences, xml) = fixture();
        cached_network(
            &root.join("school_roads"),
            "test-school",
            &fences,
            false,
            1000,
            |_| Ok(xml),
            &mut |_| {},
        )
        .unwrap();
        save_context("test-school", "接口校区名", &fences, &[(39.9, 116.4)]);
        let list = entries();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "接口校区名");
        assert!(list[0].bytes > 0);
        let (_, entry) = read_cache(&list[0].key).unwrap();
        assert_eq!(scope_key("", &entry.fences), scope_key("", &fences));
        assert_eq!(entry.points.len(), 1);
        assert!(clear_cache("../other").is_err());
        clear_cache(&list[0].key).unwrap();
        assert!(entries().is_empty());
        crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = None);
        std::fs::remove_dir_all(root).unwrap();
    }
}
