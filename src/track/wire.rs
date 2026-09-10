//! OBS 对象组装。
//!
//! 10 键对象，每值 gzip+base64；BD→GCJ 单次转换；27 键协议点集；
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

/// 27 键协议点集（gen 点 → OBS 点；gLat/gLng 由 BD 转 GCJ）。
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
        "stepDistance": 0.0,
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
    let five = five_point_payload(points, start_ms);
    json!({
        "useZip": false,
        "fivePointJson": Value::Array(five).to_string(),
        "runAreaId": -1,
        "geoFencesJson": "[]",
        "freedomShowFence": false,
    })
    .to_string()
}

/// 10s 时间窗，id=(rrid%100000)*1000+hi。
fn build_windows(track: &Track, rrid: i64) -> (Vec<Value>, Vec<Value>) {
    let start_ms = track.startTime;
    let total_time = track.totalTime;
    let mut sp = Vec::new();
    let mut stf = Vec::new();
    for (i, a) in track.speedPerTenSec.iter().enumerate() {
        let b = &track.stepsPerTenSec[i];
        let lo = (i * 10) as i64;
        let hi = (i * 10 + 10) as i64;
        let hi = hi.min(total_time);
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
            "stepsNum": b.value,
        }));
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
    let start_ms = track.startTime;
    let pts: Vec<Value> = track.locations.iter().map(|p| conv_point(p, start_ms)).collect();
    let run_wrap = json!({ "allLocJson": Value::Array(pts).to_string(), "useZip": false });
    let (sp, stf) = build_windows(track, rrid);
    let laps = build_laps(track, start_ms);
    let five = five_point_payload(live_points, start_ms);
    let fx = json!({
        "fivePointJson": Value::Array(five).to_string(),
        "freedomShowFence": false,
        "geoFencesJson": "[]",
        "runAreaId": -1,
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
