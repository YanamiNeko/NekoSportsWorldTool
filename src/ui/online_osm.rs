//! Explicit, bounded Overpass downloads reused as ordinary local OSM files.
use crate::location::Coordinate;
use route_planner::RoadGraph;
use std::{io::Read, path::PathBuf, time::Duration};

const MAX_BYTES: usize = 16 * 1024 * 1024;
const CACHE_AGE: Duration = Duration::from_secs(7 * 24 * 3600);

// Do not echo request URLs or server response bodies into user-facing logs.
pub(super) fn request_error(error: ureq::Error) -> String {
    match error {
        ureq::Error::Status(status, response) => {
            let wait = response
                .header("Retry-After")
                .and_then(|value| value.trim().parse::<u64>().ok())
                .map(|seconds| format!("请至少等待 {seconds} 秒后手动重试"))
                .unwrap_or_else(|| "请稍后手动重试".into());
            match status {
                429 => format!("路网服务请求过多（HTTP 429）；{wait}"),
                503 | 504 => format!("路网服务繁忙或超时（HTTP {status}）；{wait}，或缩小查询范围"),
                _ => format!("路网服务请求失败（HTTP {status}），请稍后重试"),
            }
        }
        ureq::Error::Transport(_) => "路网服务连接失败或超时，请检查网络后手动重试".into(),
    }
}
pub(super) struct Download {
    pub network: RoadGraph,
    pub path: PathBuf,
    pub cached: bool,
}

fn query(center: Coordinate, radius: u32) -> Result<String, String> {
    center.validate()?;
    if !(200..=2000).contains(&radius) {
        return Err("在线下载半径需在 200–2000 米之间".into());
    }
    let area = format!(
        "around:{radius},{:.7},{:.7}",
        center.latitude, center.longitude
    );
    Ok(format!("[out:xml][timeout:45][maxsize:{MAX_BYTES}];(way[\"highway\"]({area});way[\"leisure\"=\"track\"]({area});way[\"building\"]({area});relation[\"building\"]({area}););(._;>;);out body;"))
}

fn parse(bytes: &[u8]) -> Result<RoadGraph, String> {
    if bytes.len() > MAX_BYTES {
        return Err("路网响应超过 16 MB，请缩小下载半径".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "路网响应不是有效的 OSM XML")?;
    if !text.contains("<osm") || text.contains("<remark") {
        return Err("路网服务未返回完整数据，可能超时或限流，请稍后重试".into());
    }
    let network = route_planner::load_osm(bytes)?;
    if network.graph.edge_count() == 0 {
        return Err("该范围内没有连通道路，请换选点或扩大半径".into());
    }
    Ok(network)
}

pub(super) fn load_cache(path: PathBuf) -> Result<Download, String> {
    let cache = crate::platform::data_dir()
        .join("online_roads")
        .canonicalize()
        .map_err(|_| "未找到在线路网缓存，请重新下载")?;
    super::disk_cache::prepare(&cache, super::disk_cache::ROAD_LIMIT)
        .map_err(|_| "无法整理路网缓存，请检查磁盘空间与写入权限")?;
    let resolved = path
        .canonicalize()
        .map_err(|_| "路网缓存已不存在，请重新下载")?;
    if resolved.parent() != Some(cache.as_path()) {
        return Err("该文件不是已下载路网缓存，请使用在线下载".into());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(&resolved)
        .map_err(|error| format!("打开路网缓存失败：{error}"))?
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("读取路网缓存失败：{error}"))?;
    Ok(Download {
        network: parse(&bytes)?,
        path: resolved,
        cached: true,
    })
}

pub(super) fn download(center: Coordinate, radius: u32) -> Result<Download, String> {
    let query = query(center, radius)?;
    let cache = crate::platform::data_dir().join("online_roads");
    super::disk_cache::prepare(&cache, super::disk_cache::ROAD_LIMIT)
        .map_err(|_| "无法整理路网缓存，请检查磁盘空间与写入权限")?;
    let path = cache.join(format!(
        "v2-{:.7}-{:.7}-{radius}.osm",
        center.latitude, center.longitude
    ));
    if std::fs::metadata(&path).ok().is_some_and(|metadata| {
        metadata.len() <= MAX_BYTES as u64
            && metadata
                .modified()
                .ok()
                .and_then(|time| time.elapsed().ok())
                .is_some_and(|age| age < CACHE_AGE)
    }) {
        if let Ok(bytes) = std::fs::read(&path) {
            if let Ok(network) = parse(&bytes) {
                return Ok(Download {
                    network,
                    path,
                    cached: true,
                });
            }
        }
    }
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(75))
        .build();
    let response = agent
        .post("https://overpass-api.de/api/interpreter")
        .set("User-Agent", super::MAP_USER_AGENT)
        .send_form(&[("data", &query)])
        .map_err(request_error)?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("读取路网失败：{error}"))?;
    let network = parse(&bytes)?;
    super::disk_cache::store(&cache, super::disk_cache::ROAD_LIMIT, &[(&path, &bytes)])
        .map_err(|_| "保存路网失败：请检查磁盘空间与写入权限")?;
    Ok(Download {
        network,
        path,
        cached: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limit_and_service_failures_offer_bounded_manual_retry() {
        for status in [429, 503, 504] {
            let response: ureq::Response =
                format!("HTTP/1.1 {status} Busy\r\nRetry-After: 120\r\n\r\nprivate response")
                    .parse()
                    .unwrap();
            let message = request_error(ureq::Error::Status(status, response));
            assert!(message.contains(&status.to_string()));
            assert!(message.contains("120 秒"));
            assert!(!message.contains("private response"));
        }
        for header in ["", "Retry-After: invalid\r\n", "Retry-After: -1\r\n"] {
            let response = format!("HTTP/1.1 429 Busy\r\n{header}\r\n")
                .parse()
                .unwrap();
            assert!(request_error(ureq::Error::Status(429, response)).contains("稍后手动重试"));
        }
    }

    #[test]
    fn restores_downloaded_cache_and_rejects_external_imports() {
        let directory =
            std::env::temp_dir().join(format!("neko-roads-cache-{}", uuid::Uuid::new_v4()));
        crate::platform::TEST_DATA_DIR.with(|path| *path.borrow_mut() = Some(directory.clone()));
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                crate::platform::TEST_DATA_DIR.with(|path| *path.borrow_mut() = None);
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory.clone());
        std::fs::create_dir_all(directory.join("online_roads")).unwrap();
        let xml = br#"<osm version="0.6"><node id="1" lat="39.9" lon="116.4"/><node id="2" lat="39.9001" lon="116.4"/><way id="10"><nd ref="1"/><nd ref="2"/><tag k="highway" v="footway"/></way></osm>"#;
        let cached = directory.join("online_roads/downloaded.osm");
        std::fs::write(&cached, xml).unwrap();
        let loaded = load_cache(cached).unwrap();
        assert!(loaded.cached);
        assert_eq!(loaded.network.graph.edge_count(), 2);
        let external = directory.join("imported.osm");
        std::fs::write(&external, xml).unwrap();
        assert!(load_cache(external).is_err());
    }
    #[test]
    #[ignore = "explicit small public-area Overpass download and cache smoke test"]
    fn public_area_download_and_cache() {
        let directory =
            std::env::temp_dir().join(format!("neko-roads-test-{}", uuid::Uuid::new_v4()));
        crate::platform::TEST_DATA_DIR.with(|path| *path.borrow_mut() = Some(directory.clone()));
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                crate::platform::TEST_DATA_DIR.with(|path| *path.borrow_mut() = None);
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory);
        let center = Coordinate::new(48.8584, 2.2945, 0.0).unwrap();
        let downloaded = download(center, 200).unwrap();
        assert!(!downloaded.cached && downloaded.network.graph.edge_count() > 0);
        let cached = download(center, 200).unwrap();
        assert!(cached.cached);
        assert_eq!(
            cached.network.graph.edge_count(),
            downloaded.network.graph.edge_count()
        );
        assert_eq!(cached.path, downloaded.path);
        assert_eq!(
            load_cache(downloaded.path)
                .unwrap()
                .network
                .graph
                .edge_count(),
            cached.network.graph.edge_count()
        );
    }
    #[test]
    fn rejects_unbounded_or_invalid_requests_and_incomplete_responses() {
        let center = Coordinate::new(39.9, 116.4, 0.0).unwrap();
        assert!(query(center, 2001).is_err());
        assert!(query(
            Coordinate {
                latitude: f64::NAN,
                ..center
            },
            1000
        )
        .is_err());
        assert!(query(center, 500)
            .unwrap()
            .contains("around:500,39.9000000,116.4000000"));
        assert!(parse(b"<html>busy</html>").is_err());
        assert!(parse(b"<osm><remark>timeout</remark></osm>").is_err());
        assert!(parse(b"<osm/>").is_err());
    }

    #[test]
    fn legacy_import_config_is_preserved() {
        let config: crate::api::model::Config = serde_json::from_str(
            r#"{"osm_path":"old-local.osm","username":"test","route_mode":"road"}"#,
        )
        .unwrap();
        assert!(config.road_cache_path.is_empty());
        assert_eq!(config.osm_path, "old-local.osm");
        assert_eq!(config.username, "test");
        assert_eq!(config.route_mode, "road");
    }
    #[test]
    fn online_xml_preserves_topology_for_existing_planner() {
        let network = parse(br#"<osm version="0.6"><node id="1" lat="39.9" lon="116.4"/><node id="2" lat="39.9001" lon="116.4"/><way id="10"><nd ref="1"/><nd ref="2"/><tag k="highway" v="footway"/></way></osm>"#).unwrap();
        assert_eq!(network.graph.node_count(), 2);
        assert_eq!(network.graph.edge_count(), 2);
    }
}
