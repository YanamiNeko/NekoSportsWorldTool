//! OBS 对象组装。
//!
//! 10 键对象，每值 gzip+base64；BD→GCJ 单次转换；28 键协议点集；
//! 10s 窗（speed/step_freq）；每 1000m 一圈；五点 fixed_point_json。
#![allow(non_snake_case)]

use chrono::{Local, TimeZone};
use flate2::write::GzEncoder;
use flate2::Compression;
use serde_json::{json, Value};
use std::io::Write;

use super::geom::round_to;
use super::model::{GenPoint, Track};

const X_PI: f64 = std::f64::consts::PI * 3000.0 / 180.0;

/// The run-area metadata returned by the point endpoint.
///
/// Older callers can keep using the default value (free-run semantics), while
/// campus runs pass through the server-provided area id and fence JSON so the
/// record detail page can draw the same green boundary as the official app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunAreaMeta {
    pub run_area_id: i64,
    pub geo_fences_json: String,
    pub freedom_show_fence: bool,
}

impl Default for RunAreaMeta {
    fn default() -> Self {
        Self {
            run_area_id: -1,
            geo_fences_json: "[]".into(),
            freedom_show_fence: false,
        }
    }
}

/// 保留服务端返回的有效围栏；`-1` 表示接口未提供区域 ID，不应抹掉真实围栏。
/// 未知围栏格式仍回退到默认值，避免把无效元数据写入详情对象。
fn payload_area(area: &RunAreaMeta) -> RunAreaMeta {
    let valid = area.run_area_id >= -1
        && area.freedom_show_fence
        && serde_json::from_str::<Value>(&area.geo_fences_json)
            .ok()
            .is_some_and(|value| matches!(value, Value::Array(ref items) if !items.is_empty()));
    if valid {
        area.clone()
    } else {
        RunAreaMeta::default()
    }
}

/// 百度 BD-09 → 高德 GCJ-02。
pub fn bd09_to_gcj02(bd_lat: f64, bd_lng: f64) -> (f64, f64) {
    let x = bd_lng - 0.0065;
    let y = bd_lat - 0.006;
    let z = (x * x + y * y).sqrt() - 0.00002 * (y * X_PI).sin();
    let theta = y.atan2(x) - 0.000003 * (x * X_PI).cos();
    (z * theta.sin(), z * theta.cos())
}

/// gzip + base64。
pub fn gz(data: &[u8]) -> String {
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(data).expect("gzip 写入内存失败");
    let out = enc.finish().expect("gzip 收尾失败");
    super::super::crypto::envelope::b64_encode(&out)
}

fn gz_json(v: &Value) -> String {
    gz(v.to_string().as_bytes())
}

fn gz_str(v: &str) -> String {
    gz(v.as_bytes())
}

/// 28 键协议点集（gen 点 → OBS 点；gLat/gLng 由 BD 转 GCJ）。
pub fn conv_point(p: &GenPoint, start_ms: i64) -> Value {
    let (glat, glng) = bd09_to_gcj02(p.gLat, p.gLng);
    json!({
        "avgSpeed": round_to(p.avgSpeed, 4),
        "bdA": round_to(p.bdA, 2),
        "bdD": round_to(p.bdD, 2),
        "bdG": p.bdG,
        "bdS": round_to(p.bdS, 4),
        "coorType": "gcj02",
        "count": p.count,
        "dtr": 0.0,
        "flag": start_ms,
        "gLat": round_to(glat, 7),
        "gLng": round_to(glng, 7),
        "gainTime": p.gainTime,
        "id": p.id,
        "lat": -1.0,
        "lng": -1.0,
        "locType": p.locType,
        "locationId": "",
        "queueNum": p.queueNum,
        "radius": round_to(p.radius, 2),
        "speed": round_to(p.speed, 4),
        "steps": p.steps,
        "state": p.state,
        "stepDistance": round_to(p.stepDistance, 4),
        "totalDis": round_to(p.totalDis, 4),
        "totalTime": p.totalTime,
        "type": p.ptype,
        "validDis": round_to(p.validDis, 4),
        "validTime": p.validTime,
    })
}

/// 五点实体（实时点位 → 跑完态 isPass=true）。
pub fn five_point_payload(points: &[Value], start_ms: i64) -> Vec<Value> {
    points
        .iter()
        .enumerate()
        .map(|(i, p)| {
            json!({
                "flag": start_ms,
                "glat": p["glat"].as_f64().unwrap_or(0.0),
                "glon": p["glon"].as_f64().unwrap_or(0.0),
                "id": i as i64 + 1,
                "isFixed": p["isFixed"].as_i64().unwrap_or(0),
                "isPass": true,
                "lat": p["lat"].as_f64().unwrap_or(0.0),
                "lon": p["lon"].as_f64().unwrap_or(0.0),
                "pointName": p["pointName"].as_str().unwrap_or(""),
                "position": 999,
                "state": 0,
            })
        })
        .collect()
}

/// 提交 body 的 fivePointJson 包装串。
pub fn five_point_wrapper(points: &[Value], start_ms: i64) -> String {
    five_point_wrapper_with_area(points, start_ms, &RunAreaMeta::default())
}

/// Build the fixed-point wrapper while preserving the server's campus area
/// and geofence metadata.
pub fn five_point_wrapper_with_area(points: &[Value], start_ms: i64, area: &RunAreaMeta) -> String {
    let five = five_point_payload(points, start_ms);
    let area = payload_area(area);
    json!({
        "useZip": false,
        "fivePointJson": Value::Array(five).to_string(),
        "runAreaId": area.run_area_id,
        "geoFencesJson": area.geo_fences_json,
        "freedomShowFence": area.freedom_show_fence,
    })
    .to_string()
}

/// 校验提交用五点轨迹仍是同一次点位请求产生的有效数据。
pub fn validate_five_point_wrapper(wrapper: &str) -> Result<(), String> {
    let outer: Value =
        serde_json::from_str(wrapper).map_err(|e| format!("五点轨迹 JSON 无效: {e}"))?;
    let raw = outer
        .get("fivePointJson")
        .and_then(Value::as_str)
        .ok_or("五点轨迹缺少 fivePointJson")?;
    let points: Vec<Value> =
        serde_json::from_str(raw).map_err(|e| format!("五点轨迹数组无效: {e}"))?;
    if points.is_empty() {
        return Err("五点轨迹不能为空".into());
    }
    for point in points {
        let lat = point.get("lat").and_then(Value::as_f64).unwrap_or(0.0);
        let lon = point.get("lon").and_then(Value::as_f64).unwrap_or(0.0);
        let glat = point.get("glat").and_then(Value::as_f64).unwrap_or(0.0);
        let glon = point.get("glon").and_then(Value::as_f64).unwrap_or(0.0);
        if (lat == 0.0 && lon == 0.0) || (glat == 0.0 && glon == 0.0) {
            return Err("五点轨迹包含缺失坐标".into());
        }
        crate::location::Coordinate::new(lat, lon, 0.0)?;
        crate::location::Coordinate::new(glat, glon, 0.0)?;
    }
    Ok(())
}

#[cfg(test)]
mod validation_tests {
    use super::*;
    #[test]
    fn rejects_empty_or_malformed_five_point_payload() {
        assert!(validate_five_point_wrapper("{}").is_err());
        assert!(validate_five_point_wrapper(r#"{"fivePointJson":"[]"}"#).is_err());
    }

    #[test]
    fn area_metadata_is_written_to_fixed_point_wrapper() {
        let area = RunAreaMeta {
            run_area_id: 42,
            geo_fences_json: "[{\"x\":1}]".into(),
            freedom_show_fence: true,
        };
        let wrapper = five_point_wrapper_with_area(
            &[json!({"lat": 39.9, "lon": 116.4, "glat": 39.9, "glon": 116.4})],
            1_700_000_000_000,
            &area,
        );
        let value: Value = serde_json::from_str(&wrapper).unwrap();
        assert_eq!(value["runAreaId"], 42);
        assert_eq!(value["geoFencesJson"], "[{\"x\":1}]");
        assert_eq!(value["freedomShowFence"], true);
    }

    #[test]
    fn malformed_area_falls_back_to_route_compatible_defaults() {
        let area = RunAreaMeta {
            run_area_id: 42,
            geo_fences_json: "not-json".into(),
            freedom_show_fence: true,
        };
        let value: Value = serde_json::from_str(&five_point_wrapper_with_area(
            &[json!({"lat": 39.9, "lon": 116.4, "glat": 39.9, "glon": 116.4})],
            1_700_000_000_000,
            &area,
        ))
        .unwrap();
        assert_eq!(value["runAreaId"], -1);
        assert_eq!(value["geoFencesJson"], "[]");
        assert_eq!(value["freedomShowFence"], false);
    }

    #[test]
    fn server_fence_without_area_id_is_preserved() {
        let area = RunAreaMeta {
            run_area_id: -1,
            geo_fences_json: "[{\"lat\":39.4,\"lon\":116.2}]".into(),
            freedom_show_fence: true,
        };
        let value: Value = serde_json::from_str(&five_point_wrapper_with_area(
            &[json!({"lat": 39.9, "lon": 116.4, "glat": 39.9, "glon": 116.4})],
            1_700_000_000_000,
            &area,
        ))
        .unwrap();
        assert_eq!(value["runAreaId"], -1);
        assert_eq!(value["geoFencesJson"], area.geo_fences_json);
        assert_eq!(value["freedomShowFence"], true);
    }
    #[test]
    fn laps_are_rebuilt_from_overridden_altitude() {
        let points = vec![(38.901678, 121.540241), (38.902564, 121.541233)];
        let mut track = crate::track::generator::build(
            1200.0,
            600,
            7,
            (38.9, 121.54),
            1_700_000_000_000,
            &points,
        );
        crate::track::altitude::override_bd_a(&mut track, 36.75).unwrap();
        let laps = build_laps(&track, track.startTime);
        assert!(!laps.is_empty());
        assert!(laps.iter().all(|lap| lap["elevationGain"] == 0.0));
        assert!(laps.iter().all(|lap| lap["endAltAbs"] == 36.75));
        assert!(laps.iter().all(|lap| lap["endAltRel"] == 0.0));
    }

    #[test]
    fn lap_stride_uses_distance_and_conserved_step_count() {
        let track = crate::track::generator::build(
            1000.0,
            390,
            42,
            (38.9, 121.54),
            1_700_000_000_000,
            &[
                (38.901678, 121.540241),
                (38.902564, 121.541233),
                (38.900921, 121.542310),
                (38.899823, 121.541010),
                (38.900455, 121.539512),
            ],
        );
        let laps = build_laps(&track, track.startTime);
        let lap = laps.last().expect("1000m route must have a lap summary");
        let distance = lap["distance"].as_f64().unwrap();
        let steps = lap["step"].as_i64().unwrap();
        assert!(distance > 0.0);
        assert!(steps > 0);
        assert_eq!(steps, track.totalSteps);
        assert_eq!(
            lap["avgStride"].as_f64().unwrap(),
            round_to(distance / steps as f64 * 100.0, 2)
        );
    }
}

/// 10s 时间窗，id=(rrid%100000)*1000+窗口序秒（6 个真人样本跨 9 月记录验证一致；
/// 旧版 App 样本为全局序号，不适用当前版本）。
fn build_windows(track: &Track, rrid: i64) -> (Vec<Value>, Vec<Value>) {
    let start_ms = track.startTime;
    let total_time = track.totalTime;
    let mut sp = Vec::new();
    let mut stf = Vec::new();
    let (speed_windows, step_windows) = track.ten_second_windows();
    let mut lo = 0i64;
    for (a, b) in speed_windows.iter().zip(&step_windows) {
        let hi = (lo + a.time).min(total_time);
        let id = (rrid % 100000) * 1000 + hi;
        sp.push(json!({
            "beginTime": start_ms + lo * 1000,
            "distance": a.value,
            "endTime": start_ms + hi * 1000,
            "flag": start_ms,
            "id": id,
            "queueNum": 0,
            "state": 0,
        }));
        stf.push(json!({
            "avgDiff": 0.0,
            "beginTime": start_ms + lo * 1000,
            "endTime": start_ms + hi * 1000,
            "flag": start_ms,
            "id": id,
            "maxDiff": 0.0,
            "minDiff": 1000.0,
            "queueNum": 0,
            "state": 0,
            "stepsNum": b.value as i64,
        }));
        lo = hi;
    }
    (sp, stf)
}

/// 圈（每 1000m 一圈，末圈 isFullLap=false；avgStride 单位厘米）。
fn build_laps(track: &Track, start_ms: i64) -> Vec<Value> {
    let mut laps = Vec::new();
    let locs = &track.locations;
    let (mut prev_d, mut prev_t, mut prev_steps, mut gain) = (0.0f64, 0i64, 0i64, 0.0f64);
    let alt0 = locs.first().map(|p| p.bdA).unwrap_or(0.0);
    for (i, pt) in locs.iter().enumerate() {
        if i > 0 {
            let dd = pt.bdA - locs[i - 1].bdA;
            if dd > 0.0 {
                gain += dd;
            }
        }
        let d_now = pt.totalDis;
        let t_now = pt.totalTime;
        let last = i == locs.len() - 1;
        if d_now - prev_d >= 1000.0 || last {
            let lap_d = d_now - prev_d;
            let lap_t = 1i64.max(t_now - prev_t);
            let lap_steps = pt.steps - prev_steps;
            laps.push(json!({
                "avgCadence": round_to(lap_steps as f64 / (lap_t as f64 / 60.0), 2),
                "avgPace": round_to((lap_t as f64 / 60.0) / (lap_d / 1000.0).max(0.001), 2),
                "avgStride": round_to(lap_d / 1.max(lap_steps) as f64 * 100.0, 2),
                "cumulativeDuration": t_now,
                "distance": round_to(lap_d, 4),
                "duration": lap_t,
                "elevationGain": round_to(gain, 2),
                "endAltAbs": round_to(pt.bdA, 2),
                "endAltRel": round_to(pt.bdA - alt0, 2),
                "flag": start_ms,
                "id": laps.len() as i64 + 1,
                "isFullLap": lap_d >= 1000.0,
                "lapIndex": laps.len() as i64 + 1,
                "step": lap_steps,
            }));
            prev_d = d_now;
            prev_t = t_now;
            prev_steps = pt.steps;
            gain = 0.0;
        }
    }
    if let Some(target) = track.altitude_gain_override {
        let natural = laps
            .iter()
            .map(|lap| lap["elevationGain"].as_f64().unwrap_or(0.0))
            .sum::<f64>();
        if natural > f64::EPSILON {
            let scale = target / natural;
            for lap in &mut laps {
                let gain = lap["elevationGain"].as_f64().unwrap_or(0.0);
                lap["elevationGain"] = Value::from(round_to(gain * scale, 2));
            }
        } else if let Some(last) = laps.last_mut() {
            last["elevationGain"] = Value::from(round_to(target, 2));
        }
    }
    laps
}

/// 组装 10 键 OBS 对象（值均 gzip+base64；segment_json/runFaceCheck 为空串 gzip）。
pub fn build_obs_object(
    track: &Track,
    rrid: i64,
    uuid: &str,
    uid: i64,
    live_points: &[Value],
) -> Value {
    build_obs_object_with_area(track, rrid, uuid, uid, live_points, &RunAreaMeta::default())
}

/// Build the OBS payload with the same area metadata used by submission.
pub fn build_obs_object_with_area(
    track: &Track,
    rrid: i64,
    uuid: &str,
    uid: i64,
    live_points: &[Value],
    area: &RunAreaMeta,
) -> Value {
    let start_ms = track.startTime;
    let pts: Vec<Value> = track
        .locations
        .iter()
        .map(|p| conv_point(p, start_ms))
        .collect();
    let run_wrap = json!({ "allLocJson": Value::Array(pts).to_string(), "useZip": false });
    let (sp, stf) = build_windows(track, rrid);
    let laps = build_laps(track, start_ms);
    let five = five_point_payload(live_points, start_ms);
    let area = payload_area(area);
    let fx = json!({
        "fivePointJson": Value::Array(five).to_string(),
        "freedomShowFence": area.freedom_show_fence,
        "geoFencesJson": area.geo_fences_json,
        "runAreaId": area.run_area_id,
        "useZip": false,
    });
    json!({
        "rrid": gz_str(&rrid.to_string()),
        "uuid": gz_str(uuid),
        "uid": gz_str(&uid.to_string()),
        "run_data": gz_json(&run_wrap),
        "fixed_point_json": gz_json(&fx),
        "segment_json": gz_str(""),
        "speed_json": gz_json(&Value::Array(sp)),
        "step_freq_json": gz_json(&Value::Array(stf)),
        "laps_json": gz_json(&Value::Array(laps)),
        "runFaceCheck": gz_str(""),
    })
}

/// OBS objectKey（两个都要传）。
pub fn obs_keys(track: &Track, rrid: i64, uuid: &str) -> Vec<String> {
    let t0 = Local
        .timestamp_millis_opt(track.startTime)
        .single()
        .map(|t| t.format("%Y%m%d%H").to_string())
        .unwrap_or_default();
    vec![
        format!("run_data/{t0}/{uuid}.json"),
        format!("run_data/{}/{}.json", rrid / 1000000, rrid),
    ]
}
