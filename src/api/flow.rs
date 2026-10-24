//! 全链编排：policy → 实时点位 → 轨迹生成 → 提交 → OBS → 详情验证。
//! 由 UI 后台线程调用，log 闭包回传日志。

use super::client::ApiClient;
use super::model::Session;
use super::points;
use super::policy::fetch_policy;
use super::records::fetch_one_record;
use super::submit::{submit_record, SubmitParams, SubmitResult};
use crate::location::Coordinate;
use crate::track::generate_road::RouteMode;
use crate::track::generator::build as gen_track;
use crate::track::wire::{build_obs_object, five_point_wrapper, obs_keys};
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
    /// 路线算法模式。
    pub route_mode: RouteMode,
}

pub struct RunOutcome {
    pub result: SubmitResult,
    pub obs_ok: usize,
    pub verification: Verification,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Verification {
    pub detail: Option<Value>,
    pub error: Option<String>,
    pub complete: Option<bool>,
}

impl Verification {
    pub fn from_result(result: Result<Value, String>) -> Self {
        match result {
            Ok(detail) => {
                // An explicit failure wins over a contradictory top-level flag.
                let failed = detail
                    .get("reasonList")
                    .and_then(Value::as_array)
                    .map(|rows| {
                        rows.iter()
                            .any(|r| r.get("complete").and_then(Value::as_bool) == Some(false))
                    })
                    .unwrap_or(false);
                let complete = if failed {
                    Some(false)
                } else {
                    detail.get("complete").and_then(Value::as_bool)
                };
                Self {
                    complete,
                    detail: Some(detail),
                    error: None,
                }
            }
            Err(error) => Self {
                detail: None,
                complete: None,
                error: Some(error),
            },
        }
    }
    pub fn label(&self) -> &'static str {
        match self.complete {
            Some(true) => "达标",
            Some(false) => "未达标",
            None => "未知",
        }
    }
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
    run_full_flow_cancellable(
        client,
        params,
        log,
        &crate::task_control::Control::default(),
    )
}

pub fn run_full_flow_cancellable(
    client: &mut ApiClient,
    params: &RunParams,
    log: &mut dyn FnMut(&str),
    control: &crate::task_control::Control,
) -> Result<RunOutcome, String> {
    control.check()?;
    if !params.dist.is_finite() || params.dist <= 0.0 || params.dur <= 0 {
        return Err("距离和时长必须为正数".into());
    }
    let sess: Session = client.login.clone().ok_or("未登录")?;

    super::submission_guard::preflight(client, params.start_ms, params.dur, log)?;

    // ① policy
    log("[policy] 拉取跑步策略…");
    let pol = fetch_policy(client)?;
    log(&format!(
        "√ [policy] ts={} policy={} minDistance={} validTime={}",
        pol.timestamp, pol.policy, pol.min_distance, pol.valid_time
    ));
    sleep_secs(2);

    control.check()?;
    log("[road] 获取并核对当前校区围栏…");
    let fences = super::fence::fetch_geo_fence_response(client)
        .map_err(|e| format!("无法确认本次围栏：{e}"))?;
    let fences = if params.route_mode == RouteMode::Road && !fences.polygons.is_empty() {
        super::campus::selected_response(
            fences,
            super::model::load_config().school_campus.as_ref(),
            &sess.unid,
        )?
    } else {
        fences
    };

    // ② 实时点位（拒绝本地样本兜底）
    log("[points] 拉取实时点位…");
    if client.identity.has_unconfigured_default_location() {
        return Err("请先在设备信息页填写本次跑步所在城市和定位锚点，不能使用大连默认配置".into());
    }
    let anchor: Coordinate = client.identity.anchor_coordinate()?;
    let area_id = (fences.area.run_area_id >= 0).then(|| fences.area.run_area_id.to_string());
    let point_response = points::fetch_point_response(client, anchor, area_id, log)?;
    log(point_response.label());
    let pts = point_response.points();
    if pts.is_empty() && params.route_mode == RouteMode::Legacy {
        return Err("学校没有实时点位，请使用真实道路路由配置起点".into());
    }
    log(&format!("√ [points] {} 个点位", pts.len()));
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

    // ③ 轨迹生成（必经点 + 打卡点）
    if points::points_bd(&pts).len() != pts.len() {
        return Err("实时点位包含无效坐标".into());
    }
    let route_pts = super::map_boundary::route_inputs(&pol.must_points, pts, anchor);
    log(&format!(
        "[points] 去重后 {} 个路线点位（包含全部策略必经点）",
        route_pts.len()
    ));
    // Keep the user's location stable so preview and submission refer to the same campus.
    let mut params = *params;
    params.dur = crate::track::generate_road::effective_duration(params.dist, params.dur);
    control.check()?;
    log(&format!(
        "[track] 生成轨迹 {:.0}m / {}s（{} 点位）…",
        params.dist,
        params.dur,
        route_pts.len()
    ));
    // 随机 0-4 秒偏移（终端上报的 flag 与首点差 <5s），轨迹/提交/OBS/五点统一使用
    let start_ms = params.start_ms + (rand::random::<u32>() % 5) as i64 * 1000;
    let mut track = match params.route_mode {
        RouteMode::Road => {
            if !fences.polygons.is_empty() {
                super::school_roads::validate_fences(&fences.polygons)?;
            }
            let config = super::model::load_config();
            let mut items = if !config.osm_path.is_empty() {
                crate::track::generate_road::load_network_path(&config.osm_path)?.campuses
            } else { Vec::new() };
            if fences.polygons.is_empty() && (items.is_empty() || config.map_boundary.as_ref().is_some_and(|s| s.school == sess.unid && super::map_boundary::selected(&items, Some(s), &sess.unid).is_none())) {
                let name = super::map_boundary::school_name(&sess, &config);
                match super::map_boundary::discover(&sess.unid, &name, anchor, false, control) {
                    Ok(found) => items = found,
                    Err(e) => { control.check()?; log(&format!("[map] {e}；校园范围未校验")); }
                }
            }
            // A previously confirmed selection may not silently disappear at submission.
            if fences.polygons.is_empty() && config.map_boundary.as_ref().is_some_and(|s|s.school==sess.unid)
                && super::map_boundary::selected(&items,config.map_boundary.as_ref(),&sess.unid).is_none() {
                return Err("已确认的地图边界不可用或已变化，请刷新并重新确认；如需无边界运行，请在路网页明确取消地图边界".into());
            }
            let planning_fences = super::map_boundary::planning_fences(&fences.polygons,&items,config.map_boundary.as_ref(),&sess.unid)?;
            let mut net = super::school_roads::load_for_route_cancellable(
                &config.osm_path,
                &sess.unid,
                &planning_fences,
                &route_pts,
                false,
                control,
                log,
            )?;
            crate::track::generate_road::align_network(&mut net);
            let filtered = crate::track::generate_road::apply_fences(&net, &planning_fences);
            log(if planning_fences.is_empty() {
                "[road] 校园范围未校验；沿道路闭环重复补足距离"
            } else {
                "[road] 在学校围栏内沿点位闭环重复补足距离"
            });
            crate::track::generate_road::build_road(
                params.dist,
                params.dur,
                params.seed,
                start_ms,
                &route_pts,
                &route_pts,
                &filtered,
            )
            .map_err(|e| format!("道路路由失败：{e}；请修复路网或主动切换经典模式"))?
        }
        RouteMode::Legacy => gen_track(
            params.dist,
            params.dur,
            params.seed,
            (anchor.latitude, anchor.longitude),
            start_ms,
            &route_pts,
        ),
    };
    control.check()?;
    if let Some(range) = params.manual_altitude_range {
        crate::track::altitude::override_bd_a_range(&mut track, range)?;
        log(&format!(
            "√ [track] 已将海拔曲线映射到 {:.2}-{:.2}m，覆盖 {} 个点，爬升/圈数据将按覆盖值计算",
            range.min_m,
            range.max_m,
            track.locations.len()
        ));
    } else if let Some(altitude_m) = params.manual_altitude {
        crate::track::altitude::override_bd_a(&mut track, altitude_m)?;
        log(&format!(
            "√ [track] 已用手动海拔 {:.2}m 覆盖 {} 个点，爬升/圈数据将按覆盖值计算",
            altitude_m,
            track.locations.len()
        ));
    }
    log(&format!(
        "√ [track] {} 点 totalDis={:.0}m steps={} 起点={}",
        track.locations.len(),
        track.totalDistance,
        track.totalSteps,
        crate::track::geom::fmt_cst(track.startTime, "%H:%M:%S"),
    ));

    // ④ 五点 wrapper（跑完态）
    track.validate_consistency()?;
    for point in &pol.must_points {
        if !crate::track::wire::passes_point(&track, *point) {
            return Err("最终轨迹未经过策略必经点，请调整路线".into());
        }
    }
    let five = five_point_wrapper(&pts, &track, &fences.area);

    // ⑤ 提交（sportType=1）
    control.begin_submit()?;
    log("[record] 提交跑步记录（已进入提交，不能撤回）…");
    let sp = SubmitParams {
        allow_empty_points: point_response.allows_empty(),
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
    let obj = build_obs_object(
        &track_for_obs,
        result.rrid,
        &result.uuid,
        sess.uid,
        &sp.five_point_json,
    );
    let payload = obj.to_string().into_bytes();
    let keys = obs_keys(&track_for_obs, result.rrid, &result.uuid);
    let obs_ok = super::obs::upload_both_keys(client, &keys, &payload, log);
    if obs_ok == 2 {
        log("√ [obs] 双 key 上传成功");
    } else {
        log(&format!("⚠ [obs] 上传成功 {obs_ok}/2"));
    }

    // ⑦ 详情验证
    sleep_secs(2);
    log("[verify] 拉取详情验证…");
    let verification = Verification::from_result(fetch_one_record(client, result.rrid));
    log(&format!(
        "[verify] 详情读取={}；服务端达标判定={}",
        if verification.detail.is_some() {
            "成功"
        } else {
            "失败"
        },
        verification.label()
    ));
    if let Some(error) = &verification.error {
        log(&format!("⚠ [verify] {error}"));
    }
    Ok(RunOutcome {
        result,
        obs_ok,
        verification,
    })
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
pub fn run_ai_list(
    client: &mut ApiClient,
    log: &mut dyn FnMut(&str),
) -> Result<Vec<super::ai::AiSport>, String> {
    log("[ai] 拉取项目列表…");
    let list = super::ai::fetch_list(client)?;
    log(&format!("√ [ai] {} 个项目", list.len()));
    Ok(list)
}

/// 记录列表（UI 线程用）。
pub fn run_records(
    client: &mut ApiClient,
    log: &mut dyn FnMut(&str),
) -> Result<Vec<super::records::RecordRow>, String> {
    log("[records] 拉取跑步记录…");
    let rows = super::records::fetch_records(client)?;
    if let Some(session) = &client.login {
        let reconciliation = (|| {
            let mut guard = super::submission_guard::Guard::open(session.uid, session.unid.parse().map_err(|_| "学校编号无效")?)?;
            let found = guard.reconcile(&rows)?;
            for id in found { log(&format!("[record] 已核对先前提交的记录 #{id}；未重复提交")); }
            if guard.has_pending() { log("[record] 上次提交结果仍未知；本次列表未找到匹配记录，重复提交保护继续生效"); }
            Ok::<_, String>(())
        })();
        if let Err(error) = reconciliation { log(&format!("[record] 提交保护核对未完成：{error}")); }
    }
    log(&format!("√ [records] {} 条记录", rows.len()));
    Ok(rows)
}

#[cfg(test)]
mod verification_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn reading_a_detail_is_not_a_successful_grade() {
        let failed = Verification::from_result(Ok(json!({"complete": false})));
        assert!(failed.detail.is_some());
        assert_eq!(failed.label(), "未达标");
        let unknown = Verification::from_result(Ok(json!({"reasonList": []})));
        assert_eq!(unknown.complete, None);
        assert_eq!(unknown.label(), "未知");
        let contradictory = Verification::from_result(Ok(
            json!({"complete": true, "reasonList": [{"complete": false}]}),
        ));
        assert_eq!(contradictory.complete, Some(false));
    }
    #[test]
    fn consecutive_results_do_not_share_details() {
        let first = Verification::from_result(Ok(json!({"complete": true, "rrid": 1})));
        let second = Verification::from_result(Err("offline".into()));
        assert_eq!(first.complete, Some(true));
        assert!(second.detail.is_none());
        assert_eq!(second.complete, None);
        assert_eq!(second.error.as_deref(), Some("offline"));
    }
}
