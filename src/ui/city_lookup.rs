//! City lookup against a cached national boundary dataset. No coordinate is sent online.
use crate::location::Coordinate;
use route_planner::{graph::point_in_polygon, Coord};
use std::{
    io::Read,
    sync::mpsc,
    time::{Duration, Instant},
};

const DATA_URL: &str = "https://geo.datav.aliyun.com/areas_v3/bound/100000_full_city.json";
const MAX_BYTES: usize = 8 * 1024 * 1024;
#[derive(Default)]
pub(super) struct CityLookup {
    coordinate: Option<(f64, f64)>,
    queued: Option<Instant>,
    receiver: Option<mpsc::Receiver<Result<String, String>>>,
    requested: Option<(f64, f64)>,
    pub status: String,
}
impl CityLookup {
    pub fn busy(&self) -> bool {
        self.queued.is_some() || self.receiver.is_some()
    }
    pub fn request(&mut self, coordinate: (f64, f64)) {
        self.coordinate = Some(coordinate);
        self.queued = Some(Instant::now());
        self.status = "正在识别城市（首次下载全国边界数据，坐标在本地匹配）…".into();
    }
    pub fn poll(&mut self, coordinate: (f64, f64), city: &mut String) {
        if self.coordinate != Some(coordinate) {
            let first = self.coordinate.is_none();
            self.coordinate = Some(coordinate);
            let valid = Coordinate::new(coordinate.0, coordinate.1, 0.0);
            if valid.is_ok()
                && (!first
                    || ((city.trim().is_empty() || city.trim() == "大连市")
                        && !valid.unwrap().is_default_dalian()))
            {
                city.clear();
                self.request(coordinate);
            } else {
                self.queued = None;
            }
        }
        if let Some(receiver) = &self.receiver {
            let result = match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("城市识别线程已退出，请重试".into()))
                }
                Err(mpsc::TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                self.receiver = None;
                if self.requested == Some(coordinate) && self.queued.is_none() {
                    match result {
                        Ok(name) if city.trim().is_empty() => {
                            *city = name;
                            self.status = "已按行政边界自动识别城市，请核对并保存".into();
                        }
                        Ok(_) => self.status = "保留手动填写的城市，请核对并保存".into(),
                        Err(error) => {
                            self.status = format!("城市识别失败：{error}；请手动填写城市")
                        }
                    }
                }
            }
        }
        if self.receiver.is_none()
            && self
                .queued
                .is_some_and(|time| time.elapsed() >= Duration::from_millis(400))
        {
            self.queued = None;
            self.requested = Some(coordinate);
            let (sender, receiver) = mpsc::channel();
            self.receiver = Some(receiver);
            std::thread::spawn(move || {
                let _ = sender.send(lookup(coordinate));
            });
        }
    }
}

// A point on a shared boundary has no unique city. Ask for manual confirmation.
fn on_ring_boundary(ring: &[Coord], point: Coord) -> bool {
    if ring.len() < 3 {
        return false;
    }
    ring.iter().zip(ring.iter().cycle().skip(1)).any(|(a, b)| {
        let dx = b.lon - a.lon;
        let dy = b.lat - a.lat;
        let length_sq = dx * dx + dy * dy;
        if length_sq == 0.0 {
            return false;
        }
        let t = (((point.lon - a.lon) * dx + (point.lat - a.lat) * dy) / length_sq).clamp(0.0, 1.0);
        (point.lon - a.lon - t * dx).hypot(point.lat - a.lat - t * dy) <= 1e-9
    })
}

fn containing_city(data: &serde_json::Value, coordinate: (f64, f64)) -> Result<String, String> {
    Coordinate::new(coordinate.0, coordinate.1, 0.0)?;
    let point = Coord::new(coordinate.1, coordinate.0);
    for feature in data["features"].as_array().ok_or("城市边界格式无效")? {
        let level = feature["properties"]["level"].as_str().unwrap_or("");
        // Includes directly administered municipalities; ordinary counties are not cities.
        let name = feature["properties"]["name"].as_str().unwrap_or("");
        if level != "city" && !["北京市", "上海市", "天津市", "重庆市"].contains(&name)
        {
            continue;
        }
        let geometry = &feature["geometry"];
        let polygons = match geometry["type"].as_str() {
            Some("MultiPolygon") => geometry["coordinates"]
                .as_array()
                .cloned()
                .unwrap_or_default(),
            Some("Polygon") => vec![geometry["coordinates"].clone()],
            _ => continue,
        };
        for polygon in polygons {
            let Some(rings) = polygon.as_array() else {
                continue;
            };
            let ring = |value: &serde_json::Value| -> Vec<Coord> {
                value
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|pair| Some(Coord::new(pair[0].as_f64()?, pair[1].as_f64()?)))
                    .collect()
            };
            if rings
                .iter()
                .any(|value| on_ring_boundary(&ring(value), point))
            {
                return Err("位置位于城市边界，请手动确认城市".into());
            }
            if rings
                .first()
                .is_some_and(|outer| point_in_polygon(&ring(outer), point))
                && !rings
                    .iter()
                    .skip(1)
                    .any(|hole| point_in_polygon(&ring(hole), point))
            {
                return Ok(name.to_owned());
            }
        }
    }
    Err("未找到中国城市边界内的位置，或位置在边界附近".into())
}

fn lookup(coordinate: (f64, f64)) -> Result<String, String> {
    Coordinate::new(coordinate.0, coordinate.1, 0.0)?;
    let cache = crate::platform::data_dir().join("city_boundaries.json");
    if let Ok(metadata) = std::fs::metadata(&cache) {
        if metadata.len() <= MAX_BYTES as u64 {
            if let Ok(bytes) = std::fs::read(&cache) {
                if let Ok(data) = serde_json::from_slice(&bytes) {
                    return containing_city(&data, coordinate);
                }
            }
        }
    }
    let response = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .build()
        .get(DATA_URL)
        .call()
        .map_err(|error| format!("下载全国城市边界失败：{error}"))?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err("城市边界数据过大".into());
    }
    let data = serde_json::from_slice(&bytes).map_err(|_| "城市边界数据无效")?;
    let result = containing_city(&data, coordinate);
    if data["features"].is_array() {
        let _ = std::fs::write(cache, bytes);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn city_boundary_vertices_holes_and_unknown_regions_require_confirmation() {
        let data = serde_json::json!({"features":[{"properties":{"level":"city","name":"测试城市"},"geometry":{"type":"Polygon","coordinates":[[[0,0],[4,0],[4,4],[0,4],[0,0]],[[1,1],[2,1],[2,2],[1,2],[1,1]]]}}]});
        for point in [
            (0.0, 2.0),
            (4.0, 2.0),
            (2.0, 0.0),
            (2.0, 4.0),
            (0.0, 0.0),
            (4.0, 4.0),
            (1.0, 1.5),
            (1.5, 1.5),
            (5.0, 5.0),
        ] {
            assert!(containing_city(&data, point).is_err(), "{point:?}");
        }
        assert_eq!(containing_city(&data, (3.0, 3.0)).unwrap(), "测试城市");
        assert!(containing_city(&data, (f64::NAN, 0.0)).is_err());
        assert!(containing_city(&serde_json::json!({}), (0.0, 0.0)).is_err());
    }
    #[test]
    fn city_polygons_respect_holes_and_districts_are_not_cities() {
        let data = serde_json::json!({"features":[{"properties":{"level":"district","name":"天河区"},"geometry":{"type":"Polygon","coordinates":[[[0,0],[4,0],[4,4],[0,4],[0,0]]]}},{"properties":{"level":"city","name":"广州市"},"geometry":{"type":"MultiPolygon","coordinates":[[[[0,0],[4,0],[4,4],[0,4],[0,0]],[[1,1],[2,1],[2,2],[1,2],[1,1]]]]}}]});
        assert_eq!(containing_city(&data, (3.0, 3.0)).unwrap(), "广州市");
        assert!(containing_city(&data, (1.5, 1.5)).is_err());
    }
    #[test]
    fn old_lookup_cannot_overwrite_new_coordinate_or_manual_city() {
        let mut lookup = CityLookup::default();
        let mut city = "广州市".to_owned();
        let coordinate = (23.14, 113.32);
        lookup.poll(coordinate, &mut city);
        let (sender, receiver) = mpsc::channel();
        lookup.receiver = Some(receiver);
        lookup.requested = Some(coordinate);
        sender.send(Ok("北京市".into())).unwrap();
        lookup.poll(coordinate, &mut city);
        assert_eq!(city, "广州市");
        let (sender, receiver) = mpsc::channel();
        lookup.receiver = Some(receiver);
        lookup.requested = Some(coordinate);
        sender.send(Ok("广州市".into())).unwrap();
        lookup.poll((22.5, 114.0), &mut city);
        assert!(city.is_empty() && lookup.queued.is_some());
    }
    #[test]
    #[ignore = "explicit public national dataset smoke test; coordinates are matched locally"]
    fn public_guangzhou_city() {
        assert_eq!(lookup((23.1398, 113.3235)).unwrap(), "广州市");
    }
}
