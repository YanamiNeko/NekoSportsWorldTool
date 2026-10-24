//! Optional, user-triggered Nominatim backup. Never used for periodic/automatic lookup.
use crate::task_control::Control;
use route_planner::{campus::Campus, Coord};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path, time::Duration};

pub fn default_endpoint() -> String {
    "https://nominatim.openstreetmap.org/search".into()
}

fn ring(value: &Value) -> Result<Vec<Coord>, String> {
    let points = value.as_array().ok_or("地图坐标环格式错误")?;
    if !(4..=10000).contains(&points.len()) {
        return Err("地图坐标环长度错误".into());
    }
    points
        .iter()
        .map(|point| {
            let pair = point
                .as_array()
                .filter(|p| p.len() == 2)
                .ok_or("地图坐标格式错误")?;
            Ok(Coord::new(
                pair[0].as_f64().ok_or("经度格式错误")?,
                pair[1].as_f64().ok_or("纬度格式错误")?,
            ))
        })
        .collect()
}

pub(crate) fn parse(bytes: &[u8]) -> Result<Vec<Campus>, String> {
    let data: Value =
        serde_json::from_slice(bytes).map_err(|e| format!("备用地图响应格式错误：{e}"))?;
    let entries = data.as_array().ok_or("备用地图响应不是结果列表")?;
    if entries.len() > 50 {
        return Err("备用地图结果超过上限".into());
    }
    let mut items = Vec::new();
    for entry in entries {
        let category = entry["category"]
            .as_str()
            .or_else(|| entry["class"].as_str())
            .unwrap_or("");
        let kind = entry["type"].as_str().unwrap_or("");
        if !(category == "amenity" && ["university", "college", "school"].contains(&kind)
            || category == "landuse" && kind == "education")
        {
            continue;
        }
        if entry["extratags"]["building"]
            .as_str()
            .is_some_and(|v| v != "no")
        {
            continue;
        }
        let object = entry["osm_type"].as_str().unwrap_or("");
        if !["way", "relation"].contains(&object) {
            continue;
        }
        let id = entry["osm_id"].as_u64().ok_or("校园地图对象编号缺失")?;
        let geometry = &entry["geojson"];
        let polygons: Vec<&Value> = match geometry["type"].as_str() {
            Some("Polygon") => vec![&geometry["coordinates"]],
            Some("MultiPolygon") => geometry["coordinates"]
                .as_array()
                .ok_or("地图多区域格式错误")?
                .iter()
                .collect(),
            _ => continue,
        };
        let mut campus = Campus {
            id: format!("{object}/{id}"),
            name: entry["name"].as_str().unwrap_or("未命名校园").to_owned(),
            outer: vec![],
            inner: vec![],
        };
        for polygon in polygons {
            let rings = polygon
                .as_array()
                .filter(|r| !r.is_empty())
                .ok_or("校园地图轮廓为空")?;
            campus.outer.push(ring(&rings[0])?);
            for hole in &rings[1..] {
                campus.inner.push(ring(hole)?);
            }
        }
        campus
            .triangles()
            .map_err(|e| format!("校园 {} 的轮廓无效：{e}", campus.name))?;
        if !items.iter().any(|c: &Campus| c.id == campus.id) {
            items.push(campus);
        }
    }
    Ok(items)
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Cached {
    loaded_ms: i64,
    bytes: Vec<u8>,
}
struct Guard(std::path::PathBuf);
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn cached_lookup(
    dir: &Path,
    name: &str,
    endpoint: &str,
    control: &Control,
    fetch: impl FnOnce(&str, &str) -> Result<Vec<u8>, String>,
) -> Result<Vec<Campus>, String> {
    control.check()?;
    super::map_boundary::name_query(name)?;
    let query = super::map_boundary::school_base(name);
    let path = dir.join(format!(
        "{:x}.json",
        Sha256::digest(format!("{endpoint}|{query}").as_bytes())
    ));
    let now = crate::crypto::envelope::now_ms();
    if std::fs::metadata(&path).is_ok_and(|m| m.len() < 8 * 1024 * 1024) {
        if let Ok(cached) = std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|b| serde_json::from_slice::<Cached>(&b).map_err(|e| e.to_string()))
        {
            if let Ok(items) = parse(&cached.bytes) {
                let ttl = if items.is_empty() { 60000 } else { 86400000 };
                if (0..ttl).contains(&(now - cached.loaded_ms)) {
                    return Ok(items);
                }
            }
        }
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("无法创建地图查询缓存：{e}"))?;
    let lock = dir.join("request.lock");
    // Recover only an old crashed-process lock; HTTP requests are capped at 20 seconds.
    if std::fs::metadata(&lock)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|a| a > Duration::from_secs(120))
    {
        let _ = std::fs::remove_file(&lock);
    }
    let _file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock)
        .map_err(|_| "备用地图已有查询进行中，请稍后再试")?;
    drop(_file);
    let _guard = Guard(lock);
    let last = dir.join("last-request.txt");
    if let Some(ms) = std::fs::read_to_string(&last)
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
    {
        if now - ms < 2000 {
            return Err("备用地图查询过于频繁，请等待 2 秒后重试".into());
        }
    }
    std::fs::write(&last, now.to_string()).map_err(|e| e.to_string())?;
    control.check()?;
    let bytes = fetch(endpoint, query)?;
    control.check()?;
    let items = parse(&bytes)?;
    let cached = Cached {
        loaded_ms: now,
        bytes,
    };
    std::fs::write(
        &path,
        serde_json::to_vec(&cached).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("地图结果缓存失败：{e}"))?;
    Ok(items)
}

fn fetch(endpoint: &str, query: &str) -> Result<Vec<u8>, String> {
    if !endpoint.starts_with("https://") {
        return Err("备用地图地址必须使用 HTTPS".into());
    }
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(20))
        .build();
    let response=agent.get(endpoint)
        .set("User-Agent","NekoSportsWorldTool/0.3.0 (school-boundary-test; https://github.com/GAN1112233131/NekoSportsWorldTool)")
        .query("q",query).query("format","jsonv2").query("polygon_geojson","1")
        .query("extratags","1").query("limit","10").call().map_err(|e|format!("备用地图查询失败：{e}"))?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("备用地图响应超过 2 MiB".into());
    }
    Ok(bytes)
}

pub fn lookup(name: &str, endpoint: &str, control: &Control) -> Result<Vec<Campus>, String> {
    cached_lookup(
        &crate::platform::data_dir().join("map-search-backup"),
        name,
        endpoint,
        control,
        fetch,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    const REAL: &[u8] =
        include_bytes!("../../crates/route-planner/tests/fixtures/zhaoqing-search.json");
    #[test]
    fn real_search_contains_dinghu_and_keeps_three_campuses_separate() {
        let items = parse(REAL).unwrap();
        assert_eq!(items.len(), 3);
        let dinghu = items.iter().find(|c| c.id == "way/689151593").unwrap();
        assert_eq!(dinghu.name, "肇庆医学院(鼎湖校区)");
        assert_eq!(dinghu.outer.len(), 1);
        let raw = route_planner::campus::parse(include_bytes!(
            "../../crates/route-planner/tests/fixtures/dinghu.osm"
        ))
        .unwrap();
        assert_eq!(
            crate::api::map_boundary::revision(dinghu),
            crate::api::map_boundary::revision(&raw[0])
        );
    }
    #[test]
    fn query_normalization_covers_other_schools_and_no_space_suffixes() {
        for (input, expected) in [
            ("肇庆医学院鼎湖校区", "肇庆医学院"),
            ("肇庆医学院 鼎湖校区", "肇庆医学院"),
            ("成都东软学院", "成都东软学院"),
            ("某某大学（东校区）", "某某大学"),
            ("某某职业学校新校区", "某某职业学校"),
        ] {
            assert_eq!(super::super::map_boundary::school_base(input), expected);
        }
    }
    #[test]
    fn rejects_points_buildings_errors_and_bad_geometry_preserves_holes() {
        let mut entries: Vec<Value> = serde_json::from_slice(REAL).unwrap();
        entries.truncate(1);
        entries[0]["extratags"]["building"] = Value::String("yes".into());
        assert!(parse(&serde_json::to_vec(&entries).unwrap())
            .unwrap()
            .is_empty());
        entries[0]["extratags"] = serde_json::json!({});
        entries[0]["osm_type"] = Value::String("node".into());
        assert!(parse(&serde_json::to_vec(&entries).unwrap())
            .unwrap()
            .is_empty());
        entries[0]["osm_type"] = Value::String("way".into());
        entries[0]["geojson"] = serde_json::json!({"type":"MultiPolygon","coordinates":[[[[0.,0.],[3.,0.],[3.,3.],[0.,3.],[0.,0.]],[[1.,1.],[1.,2.],[2.,2.],[2.,1.],[1.,1.]]],[[[4.,0.],[5.,0.],[5.,1.],[4.,1.],[4.,0.]]]]});
        let items = parse(&serde_json::to_vec(&entries).unwrap()).unwrap();
        assert_eq!(items[0].outer.len(), 2);
        assert_eq!(items[0].inner.len(), 1);
        entries[0]["geojson"]["coordinates"][0][0][0] = serde_json::json!([999, 999]);
        assert!(parse(&serde_json::to_vec(&entries).unwrap()).is_err());
        assert!(parse(br#"{"error":"rate limited"}"#).is_err());
    }
    #[test]
    fn backup_cache_rate_limit_and_errors_are_independent_of_school_identity() {
        let dir = std::env::temp_dir().join(format!("map-backup-{}", uuid::Uuid::new_v4()));
        let c = Control::default();
        let a = cached_lookup(
            &dir,
            "肇庆医学院鼎湖校区",
            "fixture",
            &c,
            |_, q| {
                assert_eq!(q, "肇庆医学院");
                Ok(REAL.to_vec())
            },
        )
        .unwrap();
        assert_eq!(a.len(), 3);
        assert_eq!(
            cached_lookup(
                &dir,
                "肇庆医学院 端州校区",
                "fixture",
                &c,
                |_, _| panic!("same base must use cache")
            )
            .unwrap()
            .len(),
            3
        );
        assert!(
            cached_lookup(&dir, "其他学校", "fixture", &c, |_, _| panic!(
                "rate limit must stop request"
            ))
            .unwrap_err()
            .contains("频繁")
        );
        c.cancel();
        assert!(cached_lookup(&dir, "肇庆医学院", "fixture", &c, |_, _| panic!()).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    #[ignore = "explicit public read-only verification, never part of offline tests"]
    fn live_dinghu_backup_query() {
        let control=Control::default();
        let items=crate::api::map_boundary::lookup_user_requested("live-public-verification", "肇庆医学院鼎湖校区", &default_endpoint(), &control).unwrap();
        let cached=crate::api::map_boundary::lookup_named("live-public-verification", "肇庆医学院鼎湖校区", false, &control).unwrap();
        assert_eq!(items.len(),cached.len());
        assert!(items
            .iter()
            .any(|c| c.id == "way/689151593" && c.name.contains("鼎湖")));
        println!(
            "verified public query: {} campuses; Dinghu way/689151593",
            items.len()
        );
    }
}
