//! 跑步提交：POST /api/v70260/runnings/save/record。
//! Android 身份 + runes/runef 头；body 31+ 字段 + signature/originalSign。

use super::client::{check_business, ApiClient};
use crate::crypto::decrypt::{decrypt_response, derive_paes_key};
use crate::crypto::envelope::{build_envelope, build_envelope_ts, rsa_public_key, OuterOrder};
use crate::crypto::header::{build_android_header, HeaderIdentity, UA_ANDROID};
use crate::crypto::sign::{original_sign, signature};
use crate::track::calorie::{avg_power, official_kcal};
use crate::track::geom::round_to;
use crate::track::model::{GenPoint, Track};
use crate::track::wire::validate_five_point_wrapper;
use serde_json::{Map, Value};

pub const RECORD_PATH: &str = "/api/v70260/runnings/save/record";

/// Record pace uses thousandths of a minute/km; OBS laps use seconds/km.
fn pace_speed_value(seconds: i64, meters: f64) -> i64 {
    let distance = (meters * 100.0).ceil() / 100.0;
    (seconds as f64 / 60.0 / (distance / 1000.0) * 1000.0).round() as i64
}

fn submit_windows(track: &Track) -> (Vec<Value>, Vec<Value>) {
    let (mut speed, mut steps) = crate::track::wire::build_windows(track, 0);
    // Submit IDs and queue numbers differ from the OBS wire format; metrics do not.
    for values in [&mut speed, &mut steps] {
        for (i, value) in values.iter_mut().enumerate() {
            value["id"] = Value::from(60000 + i as i64);
            value["queueNum"] = Value::from(i);
        }
    }
    (speed, steps)
}

/// bdA 正差分累计。
pub fn total_ascent(locs: &[GenPoint]) -> f64 {
    let mut ascent = 0.0;
    for i in 1..locs.len() {
        let d = locs[i].bdA - locs[i - 1].bdA;
        if d > 0.0 {
            ascent += d;
        }
    }
    ascent
}

pub struct SubmitParams {
    pub track: Track,
    pub uid: i64,
    pub selected_unid: i64,
    pub policy: i64,
    pub policy_ts: i64,
    pub min_distance: i64,
    pub weight: f64,
    pub face_check: i64,
    pub five_point_json: String,
    pub address: String,
}

#[allow(dead_code)]
pub struct SubmitResult {
    pub rrid: i64,
    pub uuid: String,
    pub start_ms: i64,
    pub complete: Option<bool>,
    pub total_dis: f64,
    pub total_time: i64,
    pub total_steps: i64,
    pub avg_step_freq: i64,
    pub calorie: i64,
    pub avg_power: i64,
    pub sel_distance: i64,
}

/// 提交跑步记录（sportType=1 自由跑 + 实时五点）。
pub fn submit_record(
    client: &mut ApiClient,
    p: &SubmitParams,
    log: &mut dyn FnMut(&str),
) -> Result<SubmitResult, String> {
    client.ensure_session_valid()?;
    let session = client.login.as_ref().ok_or("未登录")?;
    if session.uid != p.uid || session.unid.parse::<i64>().ok() != Some(p.selected_unid) {
        return Err("提交账号或学校与当前会话不一致".into());
    }
    p.track.validate_consistency()?;
    validate_five_point_wrapper(&p.five_point_json)?;
    let stop_ms = p.track.totalTime.checked_mul(1000)
        .and_then(|n| p.track.startTime.checked_add(n)).ok_or("时间范围溢出")?;
    let mut guard = super::submission_guard::Guard::open(p.uid, p.selected_unid)?;
    if guard.has_pending() {
        match super::records::fetch_records(client) {
            Ok(rows) => {
                for id in guard.reconcile(&rows)? { log(&format!("[record] 已核对先前记录 #{id}")); }
            }
            Err(e) => log(&format!("[record] 只读核对失败：{e}")),
        }
    }
    let run_uuid = uuid::Uuid::new_v4().to_string().to_uppercase();
    guard.begin(&run_uuid, p.track.startTime, stop_ms)?;
    match send_record(client, p, run_uuid.clone(), log) {
        Ok(result) => {
            if let Err(e) = guard.accepted(&run_uuid, result.rrid) {
                log(&format!("[record] 服务端已接收记录 #{}，但本地回执保存失败：{e}。请勿重复提交", result.rrid));
            }
            Ok(result)
        }
        Err(SubmitFailure::Rejected(error)) => {
            guard.rejected(&run_uuid)?;
            Err(format!("服务端明确拒绝本次提交：{error}"))
        }
        Err(SubmitFailure::Unknown(error)) => {
            log("[record] 提交响应不确定，正在只读核对记录；不会再次发送提交请求");
            match super::records::fetch_records(client) {
                Ok(rows) => {
                    let found = guard.reconcile(&rows)?;
                    if let Some(id) = found.first() {
                        return Err(format!("响应不确定，但已找到本次记录 #{id}。未重复提交，未补传 OBS；请到运动记录查看详情"));
                    }
                }
                Err(e) => log(&format!("[record] 只读核对失败：{e}")),
            }
            Err(format!("提交结果未知：{error}。已保留本地请求回执并阻止再次提交；请到运动记录刷新核对，未查到不代表未入账"))
        }
    }
}

enum SubmitFailure { Rejected(String), Unknown(String) }
impl From<String> for SubmitFailure {
    fn from(error: String) -> Self { Self::Unknown(error) }
}
impl From<&str> for SubmitFailure {
    fn from(error: &str) -> Self { Self::Unknown(error.into()) }
}

fn send_record(
    client: &mut ApiClient,
    p: &SubmitParams,
    run_uuid: String,
    log: &mut dyn FnMut(&str),
) -> Result<SubmitResult, SubmitFailure> {
    let track = &p.track;
    let start_coordinate = track.validate_consistency()?;
    if p.five_point_json.is_empty() {
        return Err("五点轨迹不能为空".into());
    }
    validate_five_point_wrapper(&p.five_point_json)?;
    let (speed_windows, step_windows) = submit_windows(track);
    let total_time = track.totalTime;
    let total_dis = track.totalDistance;
    let total_steps = track.totalSteps;
    let start_ms = track.startTime;
    let stop_ms = start_ms + total_time * 1000;
    let ascent = total_ascent(&track.locations);
    let power = avg_power(p.weight, total_dis, total_time);
    let kcal = official_kcal(p.weight, total_time, total_dis);

    let unid = p.selected_unid;

    let speed = pace_speed_value(total_time, total_dis);
    let avg_step_freq = 1i64.max(round_to(total_steps as f64 / total_time as f64 * 60.0, 0) as i64);

    let mut body = Map::new();
    body.insert("allLocJson".into(), Value::String(String::new()));
    body.insert("sportType".into(), Value::from(1));
    body.insert("policy".into(), Value::from(p.policy));
    body.insert("totalTime".into(), Value::from(total_time));
    body.insert("startTime".into(), Value::from(start_ms));
    body.insert("stopTime".into(), Value::from(stop_ms));
    body.insert("getPrize".into(), Value::Bool(false));
    body.insert("status".into(), Value::from(0));
    body.insert("uuid".into(), Value::String(run_uuid.clone()));
    body.insert("uid".into(), Value::from(p.uid));
    body.insert("selectedUnid".into(), Value::from(unid));
    body.insert("selRunTime".into(), Value::from(total_time));
    body.insert("selDistance".into(), Value::from(p.min_distance));
    body.insert(
        "totalDis".into(),
        Value::from(round_to(total_dis, 0) as i64),
    );
    body.insert("speed".into(), Value::from(speed));
    body.insert(
        "validDis".into(),
        Value::from(round_to(track.validDistance, 0) as i64),
    );
    body.insert("validTime".into(), Value::from(track.validTime));
    body.insert("complete".into(), Value::Bool(true));
    body.insert("unCompleteReason".into(), Value::from(0));
    body.insert("calorie".into(), Value::from(kcal));
    body.insert("totalSteps".into(), Value::from(total_steps));
    body.insert("avgStepFreq".into(), Value::from(avg_step_freq));
    body.insert("useMobilityTools".into(), Value::from(0));
    body.insert("faceCheck".into(), Value::from(p.face_check));
    body.insert(
        "totalAscent".into(),
        Value::from(round_to(ascent, 0) as i64),
    );
    body.insert("avgPower".into(), Value::from(power));
    body.insert("speedPerTenSec".into(), Value::Array(speed_windows));
    body.insert("stepsPerTenSec".into(), Value::Array(step_windows));
    body.insert("isUpload".into(), Value::Bool(false));
    body.insert("more".into(), Value::Bool(false));
    body.insert("latitude".into(), Value::from(start_coordinate.latitude));
    body.insert("longitude".into(), Value::from(start_coordinate.longitude));
    body.insert("maxRunTime".into(), Value::from(0));
    body.insert("minSteps".into(), Value::from(0));
    if !p.five_point_json.is_empty() {
        body.insert(
            "fivePointJson".into(),
            Value::String(p.five_point_json.clone()),
        );
    }
    // Android 必现扩展字段
    body.insert("errorCode".into(), Value::from(0));
    body.insert("geeToken".into(), Value::String(String::new()));
    body.insert("unauthorized".into(), Value::from(0));
    body.insert("themeId".into(), Value::from(0));
    body.insert("goalId".into(), Value::Null);
    body.insert(
        "address".into(),
        Value::String(p.address.trim().to_string()),
    );

    let body_val = Value::Object(body.clone());
    let sig = signature(&body_val, false);
    let orig = original_sign(&body_val, false);
    body.insert("signature".into(), Value::String(sig));
    body.insert("originalSign".into(), Value::String(orig));
    let body_plain = Value::Object(body).to_string();

    let device_id = if client.identity.device_id.is_empty() {
        uuid::Uuid::new_v4().to_string().to_uppercase()
    } else {
        client.identity.device_id.clone()
    };
    let android_identity = HeaderIdentity {
        platform: "android".into(),
        device_id: device_id.clone(),
        os_version: "14".into(),
        device_name: "22081212C".into(),
        ..client.identity.clone()
    };
    let (hp, hp_extra) =
        build_android_header(&android_identity, p.uid, &client_token(client), None);
    let header_env = build_envelope(&mut client.session, &hp, OuterOrder::Observed);
    let now = crate::crypto::envelope::now_ms();
    let body_env = build_envelope_ts(
        &mut client.session,
        &body_plain,
        OuterOrder::Insert,
        now + 1,
    );

    let runes = format!("{}{}", p.policy_ts, p.uid);
    let runef = format!("{}{}", run_uuid, start_ms);
    client.ensure_session_valid()?;
    let mut req = client.agent.post(&client.record_url());
    req = req
        .set("Content-Type", "application/json; charset=utf-8")
        .set("User-Agent", UA_ANDROID)
        .set("appVersion", "7.3.40")
        .set("headerSign", &header_env.json)
        .set("runes", &runes)
        .set("runef", &runef);
    for (k, v) in &hp_extra {
        req = req.set(k, v);
    }
    let resp = req.send_string(&body_env.json).map_err(|error| {
        let rejected = matches!(&error, ureq::Error::Status(401, _));
        let message = client.handle_request_error(error);
        if rejected { SubmitFailure::Rejected(message) } else { SubmitFailure::Unknown(message) }
    })?;
    let status = resp.status();
    let raw = resp.into_string().unwrap_or_default();
    log(&format!(
        "[record] sportType=1 HTTP {status} len={}",
        raw.len()
    ));

    let key = derive_paes_key(
        &body_env.key_data[0],
        &body_env.key_data[1],
        &body_env.key_data[2],
        &body_env.key_data[3],
    );
    let dec = decrypt_response(raw.as_bytes(), &key, &rsa_public_key())
        .map_err(|e| format!("提交响应解密失败: {e}"))?;
    let biz = check_business(&dec.business).map_err(|error| {
        if dec.business.get("error").and_then(Value::as_i64).is_some_and(|code| code != 10000) {
            SubmitFailure::Rejected(error)
        } else { SubmitFailure::Unknown(error) }
    })?;
    let rrid = super::client::get_field(&biz, "rrid")
        .and_then(|v| v.as_i64().or_else(|| v.as_str()?.parse().ok()))
        .unwrap_or(0);
    if rrid <= 0 {
        return Err(format!("提交未返回 rrid: {}", truncate_json(&biz)).into());
    }
    log(&format!("√ 提交成功 rrid={rrid} uuid={run_uuid}"));
    Ok(SubmitResult {
        rrid,
        uuid: run_uuid,
        start_ms,
        complete: super::client::get_field(&biz, "complete").and_then(|v| v.as_bool()),
        total_dis,
        total_time,
        total_steps,
        avg_step_freq,
        calorie: kcal,
        avg_power: power,
        sel_distance: p.min_distance,
    })
}

fn client_token(client: &ApiClient) -> String {
    client
        .login
        .as_ref()
        .map(|s| s.token.clone())
        .unwrap_or_default()
}

fn truncate_json(v: &Value) -> String {
    let s = v.to_string();
    s.chars().take(240).collect()
}

#[cfg(test)]
#[path = "submission_tests.rs"]
mod submission_tests;

#[cfg(test)]
mod metric_tests {
    use super::*;
    #[test]
    fn record_pace_has_decimal_protocol_scale() {
        assert_eq!(pace_speed_value(600, 1000.0), 10000);
        assert_eq!(pace_speed_value(720, 2000.0), 6000);
    }
}
