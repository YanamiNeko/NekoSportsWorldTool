//! OBS 对象组装。
//!
//! 10 键对象，每值 gzip+base64；BD→GCJ 单次转换；28 键协议点集；
//! 10s 窗（speed/step_freq）；每 1000m 一圈；五点 fixed_point_json。
#![allow(non_snake_case)]

use flate2::write::GzEncoder;
use flate2::Compression;
use serde_json::{json, Value};
use std::io::Write;

use super::geom::round_to;
use super::model::{GenPoint, Track};

const X_PI: f64 = std::f64::consts::PI * 3000.0 / 180.0;

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
        "state": p.state,
        "steps": p.steps,
        "stepDistance": 0.0,
        "totalDis": round_to(p.totalDis, 4),
        "totalTime": p.totalTime,
        "type": p.ptype,
        "validDis": round_to(p.validDis, 4),
        "validTime": p.validTime,
    })
}

/// Metadata from the same live response; no made-up region identifiers.
#[derive(Clone, Debug)]
pub struct RunAreaMeta {
    pub run_area_id: i64,
    pub geo_fences: Vec<Value>,
    pub freedom_show_fence: bool,
}
impl Default for RunAreaMeta {
    fn default() -> Self {
        Self {
            run_area_id: -1,
            geo_fences: Vec::new(),
            freedom_show_fence: false,
        }
    }
}

/// Point checks use final, non-exception samples in BD-09, with the existing 40m tolerance.
pub fn passes_point(track: &Track, point: (f64, f64)) -> bool {
    track.locations.iter().filter(|p| p.ptype != -1).any(|p| {
        let dy = (p.gLat - point.0) * super::geom::MET_PER_DEG_LAT;
        let dx = (p.gLng - point.1) * 111_320.0 * point.0.to_radians().cos();
        dx * dx + dy * dy <= 40.0 * 40.0
    })
}

pub fn five_point_payload(points: &[Value], track: &Track) -> Vec<Value> {
    points
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let bd = crate::api::fence::point_xy(p);
            let (lat, lon) = bd.unwrap_or((0.0, 0.0));
            let (glat, glon) = bd09_to_gcj02(lat, lon);
            json!({
                "flag": track.startTime, "glat": glat, "glon": glon,
                "id": p.get("id").cloned().unwrap_or_else(|| json!(i + 1)),
                "isFixed": p.get("isFixed").cloned().unwrap_or_else(|| json!(0)),
                "isPass": bd.map(|xy| passes_point(track, xy)).unwrap_or(false),
                "lat": lat, "lon": lon,
                "pointName": p["pointName"].as_str().unwrap_or(""),
                "position": p.get("position").cloned().unwrap_or_else(|| json!(999)),
                "state": 0,
            })
        })
        .collect()
}

/// Construct once and reuse exactly for submit and OBS.
pub fn five_point_wrapper(points: &[Value], track: &Track, area: &RunAreaMeta) -> String {
    json!({
        "useZip": false,
        "fivePointJson": Value::Array(five_point_payload(points, track)).to_string(),
        "runAreaId": area.run_area_id,
        "geoFencesJson": Value::Array(area.geo_fences.clone()).to_string(),
        "freedomShowFence": area.freedom_show_fence,
    })
    .to_string()
}

/// 校验提交用五点轨迹仍是同一次点位请求产生的有效数据。
#[cfg(test)]
pub fn validate_five_point_wrapper(wrapper: &str) -> Result<(), String> {
    validate_five_point_wrapper_for(wrapper, false)
}

pub fn validate_five_point_wrapper_for(wrapper: &str, allow_empty: bool) -> Result<(), String> {
    let outer: Value =
        serde_json::from_str(wrapper).map_err(|e| format!("五点轨迹 JSON 无效: {e}"))?;
    let raw = outer
        .get("fivePointJson")
        .and_then(Value::as_str)
        .ok_or("五点轨迹缺少 fivePointJson")?;
    let points: Vec<Value> =
        serde_json::from_str(raw).map_err(|e| format!("五点轨迹数组无效: {e}"))?;
    if points.is_empty() && !allow_empty {
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
}

/// 10s 时间窗，id=(rrid%100000)*1000+窗口序秒（6 个真人样本跨 9 月记录验证一致；
/// 旧版 App 样本为全局序号，不适用当前版本）。
pub(crate) fn build_windows(track: &Track, rrid: i64) -> (Vec<Value>, Vec<Value>) {
    let start_ms = track.startTime;
    let total_time = track.totalTime;
    let mut sp = Vec::new();
    let mut stf = Vec::new();
    let mut lo = 0;
    for (i, a) in track.speedPerTenSec.iter().enumerate() {
        let b = &track.stepsPerTenSec[i];
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

/// 圈（每 1000m 一圈，末圈 isFullLap=false）。
/// avgPace 按秒/公里写入，供客户端格式化为分:秒；avgStride 单位厘米。
fn build_laps(track: &Track, start_ms: i64) -> Vec<Value> {
    let mut laps = Vec::new();
    let locs = &track.locations;
    let Some(last) = locs.last() else { return laps };
    let (mut prev_d, mut prev_t, mut prev_steps, mut prev_gain) = (0.0f64, 0i64, 0i64, 0.0f64);
    let alt0 = locs.first().map(|p| p.bdA).unwrap_or(0.0);
    while prev_d < last.totalDis {
        let d_now = (prev_d + 1000.0).min(last.totalDis);
        let mut gain = 0.0;
        let (mut t_now, mut steps, mut altitude) = (last.totalTime, last.steps, last.bdA);
        for pair in locs.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            let climb = (b.bdA - a.bdA).max(0.0);
            // The final boundary uses the actual last sample, including any stop.
            if d_now < last.totalDis && b.totalDis >= d_now && b.totalDis > a.totalDis {
                let k = (d_now - a.totalDis) / (b.totalDis - a.totalDis);
                t_now =
                    (a.totalTime as f64 + (b.totalTime - a.totalTime) as f64 * k).round() as i64;
                steps = (a.steps as f64 + (b.steps - a.steps) as f64 * k).round() as i64;
                altitude = a.bdA + (b.bdA - a.bdA) * k;
                gain += climb * k;
                break;
            }
            gain += climb;
        }
        let lap_d = d_now - prev_d;
        let lap_t = t_now - prev_t;
        let lap_steps = steps - prev_steps;
        laps.push(json!({
                "avgCadence": if lap_t > 0 { json!(round_to(lap_steps as f64 / (lap_t as f64 / 60.0), 2)) } else { Value::Null },
                "avgPace": if lap_t > 0 { json!(round_to(lap_t as f64 / (lap_d / 1000.0), 2)) } else { Value::Null },
                "avgStride": if lap_steps > 0 { json!(round_to(lap_d / lap_steps as f64 * 100.0, 2)) } else { Value::Null },
                "cumulativeDuration": t_now,
                "distance": round_to(lap_d, 4),
                "duration": lap_t,
                "elevationGain": round_to(gain - prev_gain, 2),
                "endAltAbs": round_to(altitude, 2),
                "endAltRel": round_to(altitude - alt0, 2),
                "flag": start_ms,
                "id": laps.len() as i64 + 1,
                "isFullLap": lap_d >= 1000.0,
                "lapIndex": laps.len() as i64 + 1,
                "step": lap_steps,
            }));
        prev_d = d_now;
        prev_t = t_now;
        prev_steps = steps;
        prev_gain = gain;
    }
    laps
}

/// 组装 10 键 OBS 对象（值均 gzip+base64；segment_json/runFaceCheck 为空串 gzip）。
pub fn build_obs_object(
    track: &Track,
    rrid: i64,
    uuid: &str,
    uid: i64,
    five_point_json: &str,
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
    json!({
        "rrid": gz_str(&rrid.to_string()),
        "uuid": gz_str(uuid),
        "uid": gz_str(&uid.to_string()),
        "run_data": gz_json(&run_wrap),
        "fixed_point_json": gz_str(five_point_json),
        "segment_json": gz_str(""),
        "speed_json": gz_json(&Value::Array(sp)),
        "step_freq_json": gz_json(&Value::Array(stf)),
        "laps_json": gz_json(&Value::Array(laps)),
        "runFaceCheck": gz_str(""),
    })
}

/// OBS objectKey（两个都要传）。
pub fn obs_keys(track: &Track, rrid: i64, uuid: &str) -> Vec<String> {
    let t0 = super::geom::fmt_cst(track.startTime, "%Y%m%d%H");
    vec![
        format!("run_data/{t0}/{uuid}.json"),
        format!("run_data/{}/{}.json", rrid / 1000000, rrid),
    ]
}

#[cfg(test)]
mod regression_tests {
    use super::*;
    fn linear_track(distance: f64, seconds: i64, steps: i64) -> Track {
        let mut track = crate::track::generator::build(
            1200.0,
            600,
            1,
            (39.9, 116.4),
            0,
            &[(39.9, 116.4), (39.901, 116.401)],
        );
        let mut a = track.locations[0].clone();
        a.gLat = 39.9;
        a.gLng = 116.4;
        a.totalTime = 0;
        a.totalDis = 0.0;
        a.steps = 0;
        let mut b = a.clone();
        b.totalTime = seconds;
        b.totalDis = distance;
        b.steps = steps;
        track.startLatitude = a.gLat;
        track.startLongitude = a.gLng;
        track.locations = vec![a, b];
        crate::track::stats::rebuild(&mut track);
        track
    }
    #[test]
    fn lap_units_partial_laps_and_zero_steps() {
        let track = linear_track(1000.0, 600, 1200);
        let laps = build_laps(&track, 0);
        assert_eq!(laps.len(), 1);
        assert_eq!(laps[0]["avgPace"], 600.0);
        assert_eq!(laps[0]["avgStride"], 83.33);
        assert_eq!(laps[0]["isFullLap"], true);
        let partial = build_laps(&linear_track(500.0, 300, 600), 0);
        assert_eq!(partial[0]["isFullLap"], false);
        assert_eq!(partial[0]["avgPace"], 600.0);
        assert!(build_laps(&linear_track(1000.0, 600, 0), 0)[0]["avgStride"].is_null());
        assert!(build_laps(&linear_track(0.0, 600, 0), 0).is_empty());
        let laps = build_laps(&linear_track(2500.0, 1500, 3000), 0);
        assert_eq!(laps.len(), 3);
        let tiny_tail = build_laps(&linear_track(1000.001, 600, 1200), 0);
        assert_eq!(
            tiny_tail
                .iter()
                .map(|l| l["duration"].as_i64().unwrap())
                .sum::<i64>(),
            600
        );
        assert!(tiny_tail.last().unwrap()["avgPace"].is_null());
        assert_eq!(
            laps.iter()
                .map(|l| l["distance"].as_f64().unwrap())
                .collect::<Vec<_>>(),
            vec![1000.0, 1000.0, 500.0]
        );
        assert_eq!(
            laps.iter()
                .map(|l| l["duration"].as_i64().unwrap())
                .sum::<i64>(),
            1500
        );
        assert_eq!(
            laps.iter()
                .map(|l| l["step"].as_i64().unwrap())
                .sum::<i64>(),
            3000
        );
    }
    #[test]
    fn lap_pace_extrema_are_ordered_in_seconds_per_kilometre() {
        let mut track = linear_track(3000.0, 1164, 3600);
        let first = track.locations[0].clone();
        track.locations = [(0., 0, 0), (1000., 410, 1200),
            (2000., 799, 2400), (3000., 1164, 3600)].into_iter().map(|(d,t,s)| {
                let mut p=first.clone(); p.totalDis=d; p.totalTime=t; p.steps=s; p
            }).collect();
        crate::track::stats::rebuild(&mut track);
        let laps=build_laps(&track,0);
        let paces: Vec<_>=laps.iter().map(|l| l["avgPace"].as_f64().unwrap()).collect();
        assert_eq!(paces, vec![410.,389.,365.]);
        let fastest=paces.iter().copied().fold(f64::INFINITY,f64::min);
        let slowest=paces.iter().copied().fold(0.,f64::max);
        let average=track.totalTime as f64/(track.totalDistance/1000.);
        assert_eq!(fastest,365.);
        assert!(fastest <= average && average <= slowest);
        // There are no maxPace/minPace fields in the observed lap contract.
        assert!(laps.iter().all(|l| l.get("maxPace").is_none() && l.get("minPace").is_none()));
    }
    #[test]
    fn encoded_obs_lap_pace_uses_seconds_not_decimal_minutes() {
        for (distance, seconds, pace) in [
            (1000.0, 408, 408.0),
            (1000.0, 419, 419.0),
            (500.0, 204, 408.0),
            (10.0, 9, 900.0),
        ] {
            let track = linear_track(distance, seconds, 1200);
            let obs = build_obs_object(&track, 1, "fixture", 1, "{}");
            let bytes = crate::crypto::envelope::b64_decode(obs["laps_json"].as_str().unwrap()).unwrap();
            let mut decoded = String::new();
            use std::io::Read;
            flate2::read::GzDecoder::new(&bytes[..]).read_to_string(&mut decoded).unwrap();
            let laps: Vec<Value> = serde_json::from_str(&decoded).unwrap();
            assert_eq!(laps.len(), 1);
            assert_eq!(laps[0]["avgPace"], pace);
            assert_eq!(laps[0]["duration"], seconds);
            assert_eq!(laps[0]["cumulativeDuration"], seconds);
            assert_eq!(laps[0]["distance"], distance);
        }
        // Crossing a kilometre boundary must not leak rounding into the total.
        let laps = build_laps(&linear_track(1010.0, 408, 1200), 0);
        assert_eq!(laps[0]["avgPace"], 404.0);
        assert_eq!(laps[1]["avgPace"], 400.0);
        assert_eq!(laps.iter().map(|l| l["duration"].as_i64().unwrap()).sum::<i64>(), 408);
    }

    #[test]
    fn point_status_and_metadata_are_identical_in_obs() {
        let track = linear_track(1000.0, 600, 1200);
        let points = vec![
            json!({"id":17,"lat":39.9,"lon":116.4,"isFixed":0}),
            json!({"id":18,"lat":39.91,"lon":116.41,"isFixed":1}),
        ];
        let payload = five_point_payload(&points, &track);
        assert_eq!(payload[0]["isPass"], true);
        assert_eq!(payload[1]["isPass"], false);
        assert_eq!(payload[0]["isFixed"], 0);
        assert_eq!(payload[1]["isFixed"], 1);
        assert_eq!(payload[0]["id"], 17);
        let area = RunAreaMeta {
            run_area_id: 42,
            geo_fences: vec![json!({"id":42})],
            freedom_show_fence: true,
        };
        let wrapper = five_point_wrapper(&points, &track, &area);
        let obj = build_obs_object(&track, 1, "test", 1, &wrapper);
        let bytes =
            crate::crypto::envelope::b64_decode(obj["fixed_point_json"].as_str().unwrap()).unwrap();
        let mut decoded = String::new();
        use std::io::Read;
        flate2::read::GzDecoder::new(&bytes[..])
            .read_to_string(&mut decoded)
            .unwrap();
        assert_eq!(decoded, wrapper);
        let (speed, steps) = build_windows(&linear_track(1100.0, 603, 1201), 60);
        assert_eq!(speed.last().unwrap()["endTime"], 603000);
        assert_eq!(speed.last().unwrap()["beginTime"], 600000);
        assert_eq!(
            steps
                .iter()
                .map(|v| v["stepsNum"].as_i64().unwrap())
                .sum::<i64>(),
            1201
        );
    }
    #[test]
    fn protocol_times_use_beijing_even_on_utc_hosts() {
        assert_eq!(crate::track::geom::fmt_gain_time(0), "1970-01-01 08:00:00");
        let mut track = linear_track(1000.0, 600, 1200);
        track.startTime = 16 * 3600 * 1000;
        assert_eq!(
            obs_keys(&track, 1, "test")[0],
            "run_data/1970010200/test.json"
        );
    }
}
