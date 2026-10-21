//! 电子围栏：POST /api/v1/getGeoFenceForRun。
//! 返回 data.geoFences[]，每个围栏 points[] 为坐标点（与打卡点同系，BD-09）。
//!
//! 服务端点结构可能只填充 GCJ 字段（glat/glon）而把 BD 字段（lat/lon）置 0，
//! 故优先取 lat/lon，若为 0/缺失则回退 glat/glon 并做 GCJ→BD 转换，避免缓存全 0。

use super::client::{get_field, ApiClient};
use crate::track::geom::gcj02_to_bd09;
use serde_json::{json, Value};

pub const FENCE_PATH: &str = "/api/v1/getGeoFenceForRun";

#[derive(Default)]
pub struct GeoFenceResponse {
    pub polygons: Vec<Vec<(f64, f64)>>,
    pub area: crate::track::wire::RunAreaMeta,
}

pub fn fetch_geo_fence_response(client: &mut ApiClient) -> Result<GeoFenceResponse, String> {
    let body = json!({ "geoFenceUpdateTime": 0 }).to_string();
    let biz = client.call("POST", FENCE_PATH, &body, &[])?;
    parse_response(&biz)
}

pub(crate) fn parse_response(biz: &Value) -> Result<GeoFenceResponse, String> {
    let biz = super::client::normalize_business_data(biz)?;
    let mut result = GeoFenceResponse::default();
    result.area.run_area_id = get_field(&biz, "runAreaId")
        .and_then(run_area_id)
        .unwrap_or(-1);
    let value = get_field(&biz, "geoFences")
        .or_else(|| biz.get("geoFences"))
        .ok_or("围栏响应缺少 geoFences 字段，不能确认学校边界")?;
    if value.is_null() {
        return Ok(result);
    }
    let fences = value.as_array().ok_or("围栏列表格式无效")?;
    let mut normalized = Vec::new();
    for fence in fences {
        let pts = fence
            .get("points")
            .and_then(Value::as_array)
            .ok_or("围栏缺少 points")?;
        let mut ring = Vec::new();
        let mut wire_points = Vec::new();
        for p in pts {
            let (lat, lon) = point_xy(p).ok_or("围栏坐标缺失或无效")?;
            let (glat, glng) = crate::track::wire::bd09_to_gcj02(lat, lon);
            ring.push((lat, lon));
            let mut wp = p.clone();
            wp["lat"] = json!(lat);
            wp["lng"] = json!(lon);
            wp["lon"] = json!(lon);
            wp["glat"] = json!(glat);
            wp["glng"] = json!(glng);
            wp["glon"] = json!(glng);
            if wp.get("latitude").is_some() {
                wp["latitude"] = json!(lat);
            }
            if wp.get("longitude").is_some() {
                wp["longitude"] = json!(lon);
            }
            wire_points.push(wp);
        }
        if ring.len() < 3 {
            return Err("围栏少于三个有效坐标".into());
        }
        let mut f = fence.clone();
        f["points"] = json!(wire_points);
        if let Some(id) = fence.get("runAreaId").and_then(run_area_id) {
            f["runAreaId"] = json!(id);
        }
        normalized.push(f);
        result.polygons.push(ring);
    }
    // A fence ID is not evidence of a run-area ID.
    result.area.freedom_show_fence = !normalized.is_empty();
    result.area.geo_fences = normalized;
    Ok(result)
}

/// 取一个点的 (lat, lon)（BD-09）。优先 lat/lon，缺失或全 0 时回退 glat/glon（GCJ→BD）。
/// 供围栏与 policy 必经点共用（两端点均实测存在「只填 GCJ、BD 置 0」的情况）。
pub(crate) fn point_xy(p: &Value) -> Option<(f64, f64)> {
    let lat = field_num(p, &["lat", "latitude"]);
    let lon = field_num(p, &["lon", "lng", "longitude"]);
    match (lat, lon) {
        (Some(a), Some(o)) if valid_xy(a, o) => Some((a, o)),
        _ => {
            let (ga, go) = (field_num(p, &["glat"]), field_num(p, &["glon", "glng"]));
            match (ga, go) {
                (Some(a), Some(o)) if valid_xy(a, o) => {
                    let point = gcj02_to_bd09(a, o);
                    valid_xy(point.0, point.1).then_some(point)
                }
                _ => None,
            }
        }
    }
}

fn valid_xy(lat: f64, lon: f64) -> bool {
    !(lat == 0.0 && lon == 0.0) && crate::location::Coordinate::new(lat, lon, 0.0).is_ok()
}

fn field_num(p: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|k| {
        p.get(*k).and_then(|v| {
            v.as_f64().or_else(|| v.as_str()?.trim().parse().ok())
                .filter(|n: &f64| n.is_finite())
        })
    })
}

pub(crate) fn run_area_id(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| value.as_str()?.trim().parse().ok())
        .filter(|id| *id >= 0)
}

#[cfg(test)]
mod tests {
    use super::point_xy;
    use serde_json::json;

    #[test]
    fn point_xy_prefers_bd_then_gcj_fallback() {
        // BD lat/lon 正常
        let p = json!({ "lat": 38.9, "lon": 121.5 });
        assert_eq!(point_xy(&p), Some((38.9, 121.5)));
        // 经纬度字段用 lng
        let p = json!({ "lat": 38.9, "lng": 121.5 });
        assert_eq!(point_xy(&p), Some((38.9, 121.5)));
        // BD 全 0，回退 glat/glon（GCJ）并转 BD
        let p =
            json!({ "lat": 0.0, "lon": 0.0, "glat": 38.8956025774013, "glon": 121.5337497718317 });
        let (lat, lon) = point_xy(&p).unwrap();
        assert!((lat - 38.901678).abs() < 1e-6, "lat={lat}");
        assert!((lon - 121.540241).abs() < 1e-6, "lon={lon}");
        // 全缺
        let p = json!({ "foo": 1 });
        assert_eq!(point_xy(&p), None);
    }
}

#[cfg(test)]
mod metadata_tests {
    use super::*;
    #[test]
    fn json_text_fences_and_numeric_strings_preserve_area_and_geometry() {
        let data = json!({"runAreaId":"42","geoFences":[{"id":7,"runAreaId":"43","points":[
            {"lat":"39.9","lon":"116.4"},
            {"lat":"39.901","lon":"116.4"},
            {"lat":"39.9","lon":"116.401"}
        ]}]});
        let object = parse_response(&json!({"error":10000,"data":data})).unwrap();
        let encoded = parse_response(&json!({"error":10000,"data":data.to_string()})).unwrap();
        assert_eq!(encoded.polygons, object.polygons);
        assert_eq!(encoded.area.geo_fences, object.area.geo_fences);
        assert_eq!(encoded.area.run_area_id, 42);
        assert_eq!(encoded.area.geo_fences[0]["runAreaId"], 43);
        assert_eq!(encoded.polygons[0][0], (39.9, 116.4));
        assert_eq!(run_area_id(&json!("invalid")), None);
        assert_eq!(run_area_id(&json!("-1")), None);
    }

    #[test]
    fn missing_invalid_and_explicitly_empty_fences_remain_distinct() {
        for data in [json!({"geoFences":null}), json!({"geoFences":[]})] {
            let response = parse_response(&json!({"data":data.to_string()})).unwrap();
            assert!(response.polygons.is_empty());
            assert!(!response.area.freedom_show_fence);
        }
        let missing = parse_response(&json!({"data":"{}"})).err().unwrap();
        assert!(missing.contains("缺少 geoFences"));
        assert!(parse_response(&json!({"data":"{invalid"})).err().unwrap().contains("data JSON"));
        for point in [json!({"lat":"NaN","lon":"116.4"}), json!({"lat":"91","lon":"116.4"}), json!({"lat":"39.9","lon":"inf"})] {
            assert_eq!(point_xy(&point), None);
        }
        let gcj = json!({"lat":"0","lon":"0","glat":"39.9","glng":"116.4"});
        let (lat, lon) = point_xy(&gcj).unwrap();
        assert!(valid_xy(lat, lon));
        assert_ne!((lat, lon), (39.9, 116.4));
    }

    #[test]
    fn normalized_coordinates_have_consistent_aliases() {
        let p = json!({"lat":0.0,"lon":0.0,"lng":0.0,"latitude":0.0,"longitude":0.0,"glat":39.9,"glon":116.4,"glng":0.0});
        let r = parse_response(&json!({"geoFences":[{"points":[p.clone(),p.clone(),p]}]})).unwrap();
        let wire = &r.area.geo_fences[0]["points"][0];
        assert_eq!(point_xy(wire), Some(r.polygons[0][0]));
        assert_eq!(wire["lon"], wire["lng"]);
        assert_eq!(wire["glon"], wire["glng"]);
        assert_eq!(wire["longitude"], wire["lon"]);
        assert_eq!(wire["latitude"], wire["lat"]);
    }
    #[test]
    fn explicit_area_id_survives_absent_fences() {
        for value in [
            json!({"runAreaId":42,"geoFences":null}),
            json!({"data":{"runAreaId":42,"geoFences":null}}),
        ] {
            let r = parse_response(&value).unwrap();
            assert_eq!(r.area.run_area_id, 42);
            assert!(r.polygons.is_empty());
        }
    }
    #[test]
    fn optional_metadata_does_not_invent_ids() {
        for v in [
            json!({"geoFences":null}),
            json!({"geoFences":[]}),
        ] {
            let r = parse_response(&v).unwrap();
            assert!(r.polygons.is_empty());
            assert_eq!(r.area.run_area_id, -1);
        }
        let f = json!({"id": 42, "points": [{"lat":39.0,"lon":116.0},{"lat":39.1,"lon":116.0},{"lat":39.0,"lon":116.1}]});
        let r = parse_response(&json!({"data":{"geoFences":[f.clone()]}})).unwrap();
        assert_eq!(r.area.run_area_id, -1);
        assert!(r.area.freedom_show_fence);
        assert_eq!(r.area.geo_fences[0]["points"][0]["lng"], 116.0);
        let multiple = parse_response(&json!({"geoFences":[f.clone(), f]})).unwrap();
        assert_eq!(multiple.area.run_area_id, -1);
        assert!(parse_response(&json!({"geoFences":[{"points":[]}]})).is_err());
        assert!(parse_response(&json!({"geoFences":"bad"})).is_err());
        assert!(parse_response(&json!({})).is_err());
    }
}
