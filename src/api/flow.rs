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
use crate::track::wire::{
    build_obs_object, five_point_wrapper, obs_keys, validate_five_point_wrapper,
};
use rand_distr::{Distribution, Normal};
use serde_json::{json, Value};

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
    pub detail_ok: bool,
}

/// 本地 dry-run 的摘要；不会读取会话、访问网络或上传任何数据。
#[derive(Debug, Clone, serde::Serialize)]
pub struct DryRunSummary {
    pub distance_m: f64,
    pub duration_s: i64,
    pub track_points: usize,
    pub obs_keys: usize,
    pub start_latitude: f64,
    pub start_longitude: f64,
    pub min_speed_mps: f64,
    pub max_speed_mps: f64,
}

fn dry_run_points(anchor: Coordinate) -> Vec<Value> {
    // 仅用于本地结构校验的四点小环；不会被当作实时点位或上传。
    let lat = anchor.latitude;
    let lon = anchor.longitude;
    [
        (lat - 0.0008, lon - 0.0010),
        (lat - 0.0008, lon + 0.0010),
        (lat + 0.0008, lon + 0.0010),
        (lat + 0.0008, lon - 0.0010),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (point_lat, point_lon))| {
        json!({
            "lat": point_lat,
            "lon": point_lon,
            "glat": point_lat,
            "glon": point_lon,
            "pointName": format!("dry-run-{i}"),
            "isFixed": 1,
        })
    })
    .collect()
}

/// 构造并校验一条完整本地记录，不创建 ApiClient，也不触发任何网络请求。
pub fn run_dry_run(params: &RunParams, anchor: Coordinate) -> Result<DryRunSummary, String> {
    anchor.validate()?;
    if params.dist <= 0.0 || !params.dist.is_finite() {
        return Err("dry-run 距离必须是正数".into());
    }
    if params.dur <= 0 {
        return Err("dry-run 时长必须是正数".into());
    }
    if params.start_ms <= 0 {
        return Err("dry-run 开始时间无效".into());
    }

    let live_points = dry_run_points(anchor);
    let point_coords: Vec<(f64, f64)> = live_points
        .iter()
        .map(|p| {
            let lat = p["lat"].as_f64().ok_or("dry-run 点位缺纬度")?;
            let lon = p["lon"].as_f64().ok_or("dry-run 点位缺经度")?;
            Coordinate::new(lat, lon, 0.0)?;
            Ok((lat, lon))
        })
        .collect::<Result<_, String>>()?;
    let mut track = gen_track(
        params.dist,
        params.dur,
        params.seed,
        (anchor.latitude, anchor.longitude),
        params.start_ms,
        &point_coords,
    );
    if let Some(range) = params.manual_altitude_range {
        crate::track::altitude::override_bd_a_range(&mut track, range)?;
    } else if let Some(altitude_m) = params.manual_altitude {
        crate::track::altitude::override_bd_a(&mut track, altitude_m)?;
    }
    track.validate_consistency()?;

    let five = five_point_wrapper(&live_points, track.startTime);
    validate_five_point_wrapper(&five)?;
    let obj = build_obs_object(&track, 0, "dry-run", 0, &live_points);
    let keys = obs_keys(&track, 0, "dry-run");
    if keys.len() != 2 {
        return Err(format!("dry-run OBS key 数量异常: {}", keys.len()));
    }
    for key in [
        "rrid",
        "uuid",
        "uid",
        "run_data",
        "fixed_point_json",
        "segment_json",
        "speed_json",
        "step_freq_json",
        "laps_json",
        "runFaceCheck",
    ] {
        if obj
            .get(key)
            .and_then(Value::as_str)
            .map(|value| value.is_empty())
            .unwrap_or(true)
        {
            return Err(format!("dry-run OBS 字段为空: {key}"));
        }
    }

    let speed_samples: Vec<f64> = track
        .locations
        .iter()
        // avgSpeed 是轨迹生成器约束后的运动速度；speed 是协议中的 GPS
        // 瞬时速度，允许为 0 或短时漂移尖峰，不能用它判断有效速度窗。
        .map(|p| p.avgSpeed)
        .filter(|speed| speed.is_finite() && *speed > 0.0)
        .collect();
    if speed_samples.is_empty() {
        return Err("dry-run 没有可校验的有效速度样本".into());
    }
    let (min_speed_mps, max_speed_mps) = speed_samples
        .into_iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), speed| {
            (min.min(speed), max.max(speed))
        });
    if min_speed_mps < crate::track::generator::SPEED_FLOOR
        || max_speed_mps > crate::track::generator::SPEED_CEIL
    {
        return Err(format!(
            "dry-run 速度超出窗口: {:.3}..{:.3} m/s",
            min_speed_mps, max_speed_mps
        ));
    }

    Ok(DryRunSummary {
        distance_m: track.totalDistance,
        duration_s: track.totalTime,
        track_points: track.locations.len(),
        obs_keys: keys.len(),
        start_latitude: track.startLatitude,
        start_longitude: track.startLongitude,
        min_speed_mps,
        max_speed_mps,
    })
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
    let pts = points::fetch_points(client, anchor, log)?;
    if pts.is_empty() {
        return Err("实时点位为空 —— 拒绝本地样本兜底".into());
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
    let pts_bd = points::points_bd(&pts);
    // 必经点保持策略顺序置于前端（waypoints[0] 即起点），剩余打卡点去重后按质心角
    // 排序，使环序自然且不破坏必经点顺序。
    let mut route_pts: Vec<(f64, f64)> = pol.must_points.clone();
    let mut free: Vec<(f64, f64)> = Vec::new();
    for p in &pts_bd {
        if !route_pts
            .iter()
            .any(|q| (q.0 - p.0).abs() < 1e-6 && (q.1 - p.1).abs() < 1e-6)
        {
            free.push(*p);
        }
    }
    if !free.is_empty() {
        route_pts.extend(crate::track::generate_road::radial_order(&free));
    }
    if !pol.must_points.is_empty() {
        log(&format!(
            "√ [policy] 必经点 {} 个（保持顺序），合并后路线 waypoint 共 {} 个",
            pol.must_points.len(),
            route_pts.len()
        ));
        // 防御性日志：must_points 的「首个=起点」语义未经证实（提交时 skip(1) 依赖此假设），
        // 逐点打印便于抓真实响应核对，避免首个必经点被静默丢弃。
        for (i, &(mlat, mlon)) in pol.must_points.iter().enumerate() {
            log(&format!(
                "  [policy] must_points[{i}] BD=({mlat:.6},{mlon:.6}){}",
                if i == 0 {
                    "（假设为起点）"
                } else {
                    "（必经点）"
                }
            ));
        }
    } else {
        log(&format!(
            "[policy] 响应未含必经点列表，仅用打卡点 {} 个",
            route_pts.len()
        ));
    }
    if let Some(&(slat, slon)) = route_pts.first() {
        log(&format!("√ [track] 起点 BD=({slat:.6},{slon:.6})"));
    }
    // 用打卡点随机偏移更新锚点并持久化：下次拉点位即学校真实坐标，摆脱写死的默认值
    if !pts_bd.is_empty() {
        let idx = (rand::random::<f64>() * pts_bd.len() as f64) as usize;
        let (clat, clng) = pts_bd[idx];
        let mut rng = rand::thread_rng();
        let normal = Normal::<f64>::new(0.0, 120.0).unwrap();
        let dlat =
            normal.sample(&mut rng).clamp(-200.0, 200.0) / crate::track::geom::MET_PER_DEG_LAT;
        let dlng =
            normal.sample(&mut rng).clamp(-200.0, 200.0) / crate::track::geom::MET_PER_DEG_LNG;
        client.identity.anchor_lat = clat + dlat;
        client.identity.anchor_lon = clng + dlng;
        if let Err(e) = super::model::save_identity(&client.identity) {
            log(&format!("⚠ 锚点持久化失败: {e}"));
        }
        // 用漂移后的新锚点重存点位缓存，使缓存锚点与持久化锚点一致，避免预览锚点失配。
        if let Ok(new_anchor) = client.identity.anchor_coordinate() {
            let _ = super::model::save_points_cache(new_anchor, &pts);
        }
    }
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
        "[track] 生成轨迹 {:.0}m / {}s（{} 点位）…",
        params.dist,
        params.dur,
        route_pts.len()
    ));
    // 随机 0-4 秒偏移（终端上报的 flag 与首点差 <5s），轨迹/提交/OBS/五点统一使用
    let start_ms = params.start_ms + (rand::random::<i64>() % 5) * 1000;
    let mut track = match params.route_mode {
        RouteMode::Road => {
            let cfg = crate::api::model::load_config();
            if cfg.osm_path.is_empty() {
                log("⚠ [track] 未配置 OSM 路网，回退经典算法");
                gen_track(
                    params.dist,
                    params.dur,
                    params.seed,
                    (anchor.latitude, anchor.longitude),
                    start_ms,
                    &pts_bd,
                )
            } else {
                match crate::track::generate_road::load_network_path(&cfg.osm_path) {
                    Ok(mut net) => {
                        crate::track::generate_road::align_network(&mut net);
                        // 电子围栏：裁剪到围栏内道路（失败/无围栏则跳过）
                        let fences = match crate::api::fence::fetch_geo_fence(client) {
                            Ok(f) => {
                                let _ = crate::api::model::save_fence_cache(&f);
                                log(&format!("√ [track] 电子围栏 {} 个", f.len()));
                                f
                            }
                            Err(e) => {
                                log(&format!("⚠ [track] 围栏获取失败，回退缓存: {e}"));
                                crate::api::model::load_fence_cache().unwrap_or_default()
                            }
                        };
                        let filtered = crate::track::generate_road::apply_fences(&net, &fences);
                        // 强制必经点（不含起点）：其余打卡点仅软引导 + <40m 吸附
                        let must_bd: Vec<(f64, f64)> =
                            pol.must_points.iter().skip(1).copied().collect();
                        match crate::track::generate_road::build_road(
                            params.dist,
                            params.dur,
                            params.seed,
                            start_ms,
                            &route_pts,
                            &must_bd,
                            &filtered,
                        ) {
                            Ok(t) => {
                                log(&format!(
                                    "√ [track] 真实道路路由 {} 点 / {} 建筑 / {} 围栏",
                                    t.locations.len(),
                                    filtered.buildings.len(),
                                    fences.len()
                                ));
                                t
                            }
                            Err(e) => {
                                log(&format!("⚠ [track] 道路路由失败，回退经典算法: {e}"));
                                gen_track(
                                    params.dist,
                                    params.dur,
                                    params.seed,
                                    (anchor.latitude, anchor.longitude),
                                    start_ms,
                                    &pts_bd,
                                )
                            }
                        }
                    }
                    Err(e) => {
                        log(&format!("⚠ [track] 路网加载失败，回退经典算法: {e}"));
                        gen_track(
                            params.dist,
                            params.dur,
                            params.seed,
                            (anchor.latitude, anchor.longitude),
                            start_ms,
                            &pts_bd,
                        )
                    }
                }
            }
        }
        RouteMode::Legacy => gen_track(
            params.dist,
            params.dur,
            params.seed,
            (anchor.latitude, anchor.longitude),
            start_ms,
            &pts_bd,
        ),
    };
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
        chrono::Local
            .timestamp_millis_opt(params.start_ms)
            .single()
            .map(|t| t.format("%H:%M:%S").to_string())
            .unwrap_or_default(),
    ));

    // ④ 五点 wrapper（跑完态）
    let five = five_point_wrapper(&pts, track.startTime);
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
    let obj = build_obs_object(&track_for_obs, result.rrid, &result.uuid, sess.uid, &pts);
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
    let detail_ok = match fetch_one_record(client, result.rrid) {
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
            log(&format!(
                "⚠ [verify] 详情拉取失败（提交已成功 rrid={}）：{e}",
                result.rrid
            ));
            false
        }
    };
    Ok(RunOutcome {
        result,
        obs_ok,
        detail_ok,
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

#[cfg(test)]
mod tests {
    use super::{run_dry_run, RouteMode, RunParams};
    use crate::location::Coordinate;

    fn params() -> RunParams {
        RunParams {
            dist: 2_000.0,
            dur: 800,
            start_ms: 1_700_000_000_000,
            face_check: 1,
            manual_altitude: None,
            manual_altitude_range: None,
            seed: 7,
            route_mode: RouteMode::Legacy,
        }
    }

    #[test]
    fn dry_run_builds_and_validates_protocol_payloads() {
        let summary = run_dry_run(&params(), Coordinate::new(39.9, 116.4, 0.0).unwrap()).unwrap();
        assert_eq!(summary.duration_s, 800);
        assert_eq!(summary.obs_keys, 2);
        assert!(summary.track_points > 10);
        assert!(summary.distance_m > 1_999.0 && summary.distance_m < 2_001.0);
    }

    #[test]
    fn dry_run_rejects_invalid_parameters() {
        let mut invalid = params();
        invalid.dur = 0;
        let error = run_dry_run(&invalid, Coordinate::new(39.9, 116.4, 0.0).unwrap()).unwrap_err();
        assert!(error.contains("时长"));
    }
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
    log(&format!("√ [records] {} 条记录", rows.len()));
    Ok(rows)
}

use chrono::TimeZone as _;

/// 最近一次详情验证的完整响应（达标判定明细在 reasonList）。
pub static VERIFY_DETAIL: std::sync::Mutex<Option<Value>> = std::sync::Mutex::new(None);
