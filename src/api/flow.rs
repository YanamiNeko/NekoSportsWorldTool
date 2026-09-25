//! 全链编排：policy → 实时点位 → 轨迹生成 → 提交 → OBS → 详情验证。
//! 由 UI 后台线程调用，log 闭包回传日志。

use super::client::ApiClient;
use super::model::Session;
use super::points;
use super::policy::fetch_policy;
use super::records::fetch_one_record;
use super::submit::{submit_record, SubmitParams, SubmitResult};
use crate::track::generator::build as gen_track;
use crate::track::wire::{build_obs_object_with_area, five_point_wrapper_with_area, obs_keys};
use crate::location::Coordinate;
use serde_json::Value;

#[derive(Clone, Copy)]
pub struct RunParams {
    /// 距离（米）与时长（秒）已由 UI 参数解析。
    pub dist: f64,
    pub dur: i64,
    /// 开始时间（毫秒）。
    pub start_ms: i64,
    pub face_check: i64,
    /// 用户手动填写的绝对海拔（米）；None 使用生成器海拔。
    pub manual_altitude: Option<f64>,
    /// 用户手动填写的海拔范围；与单值字段兼容，范围优先。
    pub manual_altitude_range: Option<crate::track::altitude::AltitudeRange>,
    pub seed: u64,
}

pub struct RunOutcome {
    pub result: SubmitResult,
    pub obs_ok: usize,
    pub detail_ok: bool,
}

fn sleep_secs(s: u64) {
    std::thread::sleep(std::time::Duration::from_secs(s));
}

/// 跑步全链。
pub fn run_full_flow(
    client: &mut ApiClient,
    params: &RunParams,
    log: &mut dyn FnMut(&str),
) -> Result<RunOutcome, String> {
    let sess: Session = client.login.clone().ok_or("未登录")?;

    // ① policy
    log("[policy] 拉取跑步策略…");
    let pol = fetch_policy(client)?;
    log(&format!(
        "√ [policy] ts={} policy={} minDistance={} validTime={}",
        pol.timestamp, pol.policy, pol.min_distance, pol.valid_time
    ));
    sleep_secs(2);

    // ② 实时点位（拒绝本地样本兜底）
    log("[points] 拉取实时点位…");
    if client.identity.has_unconfigured_default_location() {
        return Err("请先在设备信息页填写本次跑步所在城市和定位锚点，不能使用大连默认配置".into());
    }
    let anchor: Coordinate = client.identity.anchor_coordinate()?;
    let requested_area_id = (pol.area.run_area_id >= 0).then(|| pol.area.run_area_id.to_string());
    let mut points_ctx = points::fetch_points_context_ext(client, anchor, requested_area_id, log)?;
    // 校园围栏可能随 runModePolicy 返回，而点位接口只给打卡点。
    // 优先保留策略中的真实区域；仅当策略缺失时使用点位中的区域。
    if pol.area.run_area_id >= 0 { points_ctx.area.run_area_id = pol.area.run_area_id; }
    if pol.area.geo_fences_json.trim() != "[]" {
        points_ctx.area.geo_fences_json = pol.area.geo_fences_json.clone();
        points_ctx.area.freedom_show_fence = pol.area.freedom_show_fence;
    }
    let pts = points_ctx.points.clone();
    if pts.is_empty() {
        return Err("实时点位为空 —— 拒绝本地样本兜底".into());
    }
    log(&format!(
        "√ [points] {} 个点位，runAreaId={}，绿色围栏={}（{} 字节）",
        pts.len(),
        points_ctx.area.run_area_id,
        points_ctx.area.freedom_show_fence,
        points_ctx.area.geo_fences_json.len(),
    ));
    for p in pts.iter().take(5) {
        log(&format!(
            "  [points] {} BD=({:.6},{:.6}) GCJ=({},{})",
            p.get("pointName").and_then(|v| v.as_str()).unwrap_or(""),
            p.get("lat").and_then(|v| v.as_f64()).unwrap_or(0.0),
            p.get("lon").and_then(|v| v.as_f64()).unwrap_or(0.0),
            p.get("glat").map(|v| v.to_string()).unwrap_or_default(),
            p.get("glon").map(|v| v.to_string()).unwrap_or_default(),
        ));
    }

    // ③ 轨迹生成（打卡点拟合环）
    let pts_bd = points::points_bd(&pts);
    // 平均配速须落在有效窗口内（否则逐点速度无法全窗内），越界时修正时长
    let mut params = *params;
    let avg = params.dist / params.dur as f64;
    let fixed_avg = avg.clamp(
        crate::track::generator::SPEED_FLOOR + 0.1,
        crate::track::generator::SPEED_CEIL - 0.1,
    );
    if (fixed_avg - avg).abs() > 1e-6 {
        let fixed_dur = (params.dist / fixed_avg).round() as i64;
        log(&format!(
            "[track] 平均配速 {} m/s 超出有效窗口，时长 {} -> {}s",
            (avg * 100.0).round() / 100.0,
            params.dur,
            fixed_dur
        ));
        params.dur = fixed_dur;
    }
    log(&format!(
        "[track] 生成轨迹 {:.0}m / {}s（{} 点位拟合环）…",
        params.dist,
        params.dur,
        pts_bd.len()
    ));
    // 随机 0-4 秒偏移（终端上报的 flag 与首点差 <5s），轨迹/提交/OBS/五点统一使用
    let start_ms = params.start_ms + (rand::random::<i64>() % 5) * 1000;
    let mut track = gen_track(params.dist, params.dur, params.seed, (anchor.latitude, anchor.longitude), start_ms, &pts_bd);
    if let Some(range) = params.manual_altitude_range {
        crate::track::altitude::override_bd_a_range(&mut track, range)?;
        log(&format!("√ [track] 已将海拔曲线映射到 {:.2}-{:.2}m，覆盖 {} 个点，爬升/圈数据将按覆盖值计算", range.min_m, range.max_m, track.locations.len()));
    } else if let Some(altitude_m) = params.manual_altitude {
        crate::track::altitude::override_bd_a(&mut track, altitude_m)?;
        log(&format!("√ [track] 已用手动海拔 {:.2}m 覆盖 {} 个点，爬升/圈数据将按覆盖值计算", altitude_m, track.locations.len()));
    }
    log(&format!(
        "√ [track] {} 点 totalDis={:.0}m steps={} 起点={}",
        track.locations.len(),
        track.totalDistance,
        track.totalSteps,
        chrono::Local.timestamp_millis_opt(params.start_ms).single()
            .map(|t| t.format("%H:%M:%S").to_string()).unwrap_or_default(),
    ));

    // ④ 五点 wrapper（跑完态）
    let five = five_point_wrapper_with_area(&pts, track.startTime, &points_ctx.area);
    let _ = &five;

    // ⑤ 提交（sportType=1）
    log("[record] 提交跑步记录（sportType=1）…");
    let sp = SubmitParams {
        track,
        uid: sess.uid,
        selected_unid: sess.unid.parse().unwrap_or(0),
        policy: pol.policy,
        policy_ts: pol.timestamp,
        min_distance: pol.min_distance,
        weight: if sess.weight > 0.0 { sess.weight } else { 68.0 },
        face_check: params.face_check,
        five_point_json: five,
        address: client.identity.city.clone(),
    };
    let result = submit_record(client, &sp, log)?;
    sleep_secs(1);

    // ⑥ OBS 上传（双 key）
    log("[obs] 上传 OBS 对象（gzip+base64，10 键）…");
    // 从提交结果回填 track.startTime（含随机秒偏移），保证 body/OBS/flag 全链一致
    let mut track_for_obs = sp.track.clone();
    track_for_obs.startTime = result.start_ms;
    let obj = build_obs_object_with_area(&track_for_obs, result.rrid, &result.uuid, sess.uid, &pts, &points_ctx.area);
    let expected_summary = super::obs::summarize_object(&obj).ok();
    if expected_summary.as_ref().map(|summary| !summary.is_expected()).unwrap_or(true) {
        log("⚠ [obs] 本地待上传对象缺少有效路线/区域/围栏数据");
    }
    let payload = obj.to_string().into_bytes();
    let keys = obs_keys(&track_for_obs, result.rrid, &result.uuid);
    let obs_ok = super::obs::upload_both_keys(client, &keys, &payload, log);
    let obs_content_ok = if obs_ok == 2 {
        log("√ [obs] 双 key 上传成功");
        sleep_secs(1);
        let mut all_valid = expected_summary.as_ref().is_some_and(|summary| summary.is_expected());
        for key in &keys {
            match super::obs::fetch_object(client, key, log)
                .and_then(|value| super::obs::summarize_object(&value))
            {
                Ok(summary) => {
                    let valid = expected_summary
                        .as_ref()
                        .is_some_and(|expected| summary.matches(expected));
                    all_valid &= valid;
                    log(&format!(
                        "{} [obs] 回读校验 key={} matches_upload={} route_points={} runAreaId={} 绿色围栏={} fence_points={}（{} 字节）",
                        if valid { "√" } else { "⚠" },
                        key.rsplit('/').next().unwrap_or(key),
                        valid,
                        summary.route_points,
                        summary.run_area_id,
                        summary.show_fence,
                        summary.fence_count,
                        summary.fence_bytes,
                    ));
                }
                Err(error) => {
                    all_valid = false;
                    log(&format!("⚠ [obs] 回读校验失败 key={}: {error}", key.rsplit('/').next().unwrap_or(key)));
                }
            }
        }
        if all_valid { log("√ [obs] 双 key 内容校验通过"); }
        else { log("⚠ [obs] 双 key 内容校验未通过"); }
        all_valid
    } else {
        log(&format!("⚠ [obs] 上传成功 {obs_ok}/2"));
        false
    };

    // ⑦ 详情验证
    sleep_secs(2);
    log("[verify] 拉取详情验证…");
    let record_ok = match fetch_one_record(client, result.rrid) {
        Ok(d) => {
            log(&format!(
                "√ [verify] rrid={} complete={:?} dis={:?} time={:?}",
                result.rrid,
                d.get("complete").and_then(|v| v.as_bool()),
                d.get("totalDis"),
                d.get("totalTime"),
            ));
            if let Ok(mut slot) = VERIFY_DETAIL.lock() {
                slot.replace(d.clone());
            }
            true
        }
        Err(e) => {
            log(&format!("⚠ [verify] 详情拉取失败（提交已成功 rrid={}）：{e}", result.rrid));
            false
        }
    };
    let detail_ok = obs_content_ok && record_ok;
    if !obs_content_ok {
        log("⚠ [verify] OBS 对象内容校验未通过");
    }
    Ok(RunOutcome { result, obs_ok, detail_ok })
}

/// AI 提交流（UI 线程用）。
pub fn run_ai_submit(
    client: &mut ApiClient,
    sport_id: i64,
    mode: super::ai::AiMode,
    log: &mut dyn FnMut(&str),
) -> Result<Value, String> {
    log(&format!("[ai] 提交 sportId={sport_id} mode={mode:?}…"));
    let biz = super::ai::upload(client, sport_id, mode, None)?;
    log("√ [ai] 提交成功");
    Ok(biz)
}

/// AI 列表（UI 线程用）。
pub fn run_ai_list(client: &mut ApiClient, log: &mut dyn FnMut(&str)) -> Result<Vec<super::ai::AiSport>, String> {
    log("[ai] 拉取项目列表…");
    let list = super::ai::fetch_list(client)?;
    log(&format!("√ [ai] {} 个项目", list.len()));
    Ok(list)
}

/// 记录列表（UI 线程用）。
pub fn run_records(client: &mut ApiClient, log: &mut dyn FnMut(&str)) -> Result<Vec<super::records::RecordRow>, String> {
    log("[records] 拉取跑步记录…");
    let rows = super::records::fetch_records(client)?;
    log(&format!("√ [records] {} 条记录", rows.len()));
    Ok(rows)
}

use chrono::TimeZone as _;

/// 最近一次详情验证的完整响应（达标判定明细在 reasonList）。
pub static VERIFY_DETAIL: std::sync::Mutex<Option<Value>> = std::sync::Mutex::new(None);
