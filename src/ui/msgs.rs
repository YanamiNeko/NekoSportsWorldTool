//! 消息协议分发与弹窗构建。

use super::about::FinishAction;
use super::{
    App, PopupInfo, AI_DETAIL, AI_DONE, AI_LIST, AI_RECORDS, CHEAT, IP, LOGIN_DONE, POINTS_DONE,
    RANK, RECORDS, RUN_DETAIL, RUN_DONE, SEMESTER, UPDATE_CHK, UPDATE_DONE, UPDATE_PROG, USER,
};
use crate::api::model;
use chrono::TimeZone;

impl App {
    fn expire_road_session(&mut self) {
        self.session_epoch = self.session_epoch.wrapping_add(1);
        self.auth_guard
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.records_busy = false;
        self.data_busy = false;
        self.user_busy = false;
        self.ai_busy = self.ai_submit_active;
        self.records_page.detail_loading = false;
        self.records_page.ai_detail_loading = false;
        self.data_page.cheat = None;
        self.data_page.cheat_checking = false;
        self.data_page.cheat_error = Some("登录已失效，请重新登录后检查".into());
        self.data_page.semester = None;
        self.user_page.info = None;
        self.session = None;
        model::clear_session();
        self.auth_expired = true;
        self.osm_page.cancel.cancel();
        self.osm_page.request_id = self.osm_page.request_id.wrapping_add(1);
        self.osm_page.busy = false;
        self.run_page.preview_cancel.cancel();
        self.run_page.preview_inputs = None;
        self.run_page.preview = None;
        self.run_page.preview_stale = true;
        self.run_page.check = None;
        self.status = crate::api::client::SESSION_EXPIRED.into();
    }
    fn handle_fetch_error(&mut self, message: &str) -> bool {
        for (prefix, label) in [
            (RECORDS, "跑步记录"),
            (AI_LIST, "AI 项目"),
            (AI_RECORDS, "AI 记录"),
            (SEMESTER, "学期"),
            (CHEAT, "违规检查"),
            (RANK, "排行榜"),
        ] {
            let Some(payload) = message.strip_prefix(prefix) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) else {
                return false;
            };
            let Some(error) = value.get("fetchError").and_then(|v| v.as_str()) else {
                return false;
            };
            match prefix {
                RECORDS | AI_RECORDS => self.records_busy = false,
                AI_LIST => self.ai_busy = self.ai_submit_active,
                CHEAT => {
                    self.data_busy = false;
                    self.data_page.cheat = None;
                    self.data_page.cheat_checking = false;
                    self.data_page.cheat_error = Some(error.into());
                }
                SEMESTER => {
                    self.data_busy = false;
                    self.data_page.semester = None;
                }
                RANK => self.data_busy = false,
                _ => {}
            }
            self.status = format!("{label}刷新失败：{error}");
            return true;
        }
        false
    }

    pub(crate) fn poll_messages(&mut self) {
        self.poll_preview_results();
        let mut done_ip = None;
        let mut done_login = None;
        let mut done_run = None;
        let mut done_ai = None;
        let mut done_points = None;
        let mut records_json = None;
        let mut ai_list_json = None;
        let mut semester_json = None;
        let mut cheat_json = None;
        let mut rank_json = None;
        let mut user_json = None;
        let mut ai_records_json = None;
        let mut ai_detail_raw = None;
        let mut detail_raw = None;
        let mut update_chk = None;
        let mut update_prog = None;
        let mut update_done = None;
        while let Ok(msg) = self.rx.try_recv() {
            let msg = if let Some(wrapped) = msg.strip_prefix("__SESSION_JOB__") {
                let Some((epoch, message)) = wrapped.split_once('|') else {
                    continue;
                };
                if epoch.parse::<u64>().ok() != Some(self.session_epoch) {
                    continue;
                }
                message.to_string()
            } else {
                msg
            };
            if self.handle_fetch_error(&msg) {
                continue;
            }
            if let Some(v) = msg.strip_prefix("__ROAD_STAGE__") {
                if let Some((id, stage)) = v.split_once('|') {
                    if self.osm_page.busy
                        && id.parse::<u64>().ok() == Some(self.osm_page.request_id)
                    {
                        self.osm_page.status = stage.to_string();
                    }
                }
            } else if let Some(stage) = msg.strip_prefix("__RUN_STAGE__") {
                self.status = stage.to_string();
            } else if let Some(v) = msg.strip_prefix(IP) {
                done_ip = Some(v.to_string());
            } else if let Some(payload) = msg.strip_prefix(POINTS_DONE) {
                done_points = serde_json::from_str::<super::run::PreviewInputs>(payload).ok();
            } else if let Some(v) = msg.strip_prefix(LOGIN_DONE) {
                done_login = Some(v.to_string());
            } else if let Some(v) = msg.strip_prefix(RUN_DONE) {
                done_run = Some(v.to_string());
            } else if let Some(v) = msg.strip_prefix(AI_DONE) {
                done_ai = Some(v.to_string());
            } else if let Some(v) = msg.strip_prefix(RECORDS) {
                records_json = Some(v.to_string());
            } else if let Some(v) = msg.strip_prefix(AI_LIST) {
                ai_list_json = Some(v.to_string());
            } else if let Some(v) = msg.strip_prefix(SEMESTER) {
                semester_json = Some(v.to_string());
            } else if let Some(v) = msg.strip_prefix(CHEAT) {
                cheat_json = Some(v.to_string());
            } else if let Some(v) = msg.strip_prefix(RANK) {
                rank_json = Some(v.to_string());
            } else if let Some(v) = msg.strip_prefix(USER) {
                if v != "done" {
                    user_json = Some(v.to_string());
                } else {
                    self.user_busy = false;
                }
            } else if let Some(v) = msg.strip_prefix(AI_RECORDS) {
                ai_records_json = Some(v.to_string());
            } else if let Some(v) = msg.strip_prefix(RUN_DETAIL) {
                if let Ok(raw) = serde_json::from_str::<serde_json::Value>(v) {
                    // Filter before coalescing messages: a late reply must not
                    // discard the current record's reply in the same frame.
                    if raw.get("rrid").and_then(|v| v.as_i64()) == self.records_page.detail_rrid
                        && self.records_page.detail_rrid.is_some() {
                        detail_raw = Some(raw);
                    }
                }
            } else if let Some(v) = msg.strip_prefix(AI_DETAIL) {
                if let Ok(raw) = serde_json::from_str::<serde_json::Value>(v) {
                    ai_detail_raw = Some(raw);
                }
            } else if let Some(v) = msg.strip_prefix(UPDATE_CHK) {
                update_chk = Some(v.to_string());
            } else if let Some(v) = msg.strip_prefix(UPDATE_PROG) {
                update_prog = Some(v.to_string());
            } else if let Some(v) = msg.strip_prefix(UPDATE_DONE) {
                update_done = Some(v.to_string());
            } else {
                self.log.push(&msg);
            }
        }
        let got_semester = semester_json.is_some();
        let got_cheat = cheat_json.is_some();
        let got_rank = rank_json.is_some();

        // OSM 路网加载回传
        while let Ok(res) = self.net_rx.try_recv() {
            if res.request_id != self.osm_page.request_id {
                continue;
            }
            if let Some((uid, unid, anchor, token)) = &res.scope {
                // A delayed failure from an old token cannot invalidate a fresh login.
                if self
                    .session
                    .as_ref()
                    .is_some_and(|s| s.uid == *uid && s.token != *token)
                {
                    continue;
                }
                if !self
                    .session
                    .as_ref()
                    .map(|s| s.uid == *uid && s.unid == *unid)
                    .unwrap_or(false)
                    || self.identity.anchor_coordinate().ok() != *anchor
                {
                    self.osm_page.busy = false;
                    self.network = None;
                    self.run_page.preview = None;
                    self.run_page.preview_inputs = None;
                    self.osm_page.status = "学校或定位已变化，请重新获取路网".into();
                    continue;
                }
            }
            let refresh_manual_metadata = !res.pending
                && !res.offline
                && res.scope.is_none()
                && res.inputs.is_none()
                && res.result.is_ok()
                && !self.config.osm_path.is_empty()
                && self
                    .session
                    .as_ref()
                    .is_some_and(model::Session::is_logged_in)
                && self.run_page.route_mode == crate::track::generate_road::RouteMode::Road;
            self.osm_page.busy = res.pending;
            let expired = res.scope.is_some()
                && (crate::api::client::is_session_expired(&res.warning)
                    || res
                        .result
                        .as_ref()
                        .err()
                        .is_some_and(|e| crate::api::client::is_session_expired(e)));
            if !res.campuses.is_empty() {
                self.osm_page.campuses = res.campuses;
                self.osm_page.selected_key = res.selected_key;
                if let Some(session) = &self.session {
                    let keep = self
                        .config
                        .school_campus
                        .as_ref()
                        .map(|s| s.school == session.unid && s.key == self.osm_page.selected_key)
                        .unwrap_or(false);
                    if !keep {
                        self.config.school_campus = Some(crate::api::campus::Selection {
                            school: session.unid.clone(),
                            key: self.osm_page.selected_key.clone(),
                            confirmed: false,
                        });
                    }
                }
            }
            self.osm_page.offline = res.offline;
            if let Some(inputs) = res.inputs {
                self.run_page.preview_inputs = Some(inputs);
            }
            match res.result {
                Ok(net) => {
                    let nodes = net.graph.node_count();
                    let bld = net.buildings.len();
                    self.osm_page.fitted = false;
                    self.osm_page.status = format!(
                        "√ {}路网加载成功：{nodes} 节点 / {bld} 建筑",
                        if self.osm_page.offline { "离线" } else { "" }
                    );
                    if !res.warning.is_empty() {
                        self.osm_page.status.push_str(&format!("\n{}", res.warning));
                    }
                    self.log
                        .push(&format!("[osm] 路网加载成功：{nodes} 节点 / {bld} 建筑"));
                    self.network = Some(net);
                    self.run_page.preview_stale = true;
                    self.osm_page.cache_entries = crate::api::school_roads::entries();
                    self.analyze_gpx();
                }
                Err(e) => {
                    self.network = None;
                    self.run_page.preview = None;
                    self.run_page.preview_stale = true;
                    self.osm_page.status = crate::road_status::explain(&e);
                    self.log.push(&format!("[osm] 路网加载失败：{e}"));
                }
            }
            if expired {
                self.expire_road_session();
            } else if refresh_manual_metadata {
                self.refresh_points();
            }
        }
        if let Some(ip) = done_ip {
            self.ip = ip;
        }
        if let Some(inputs) = done_points {
            if self
                .session
                .as_ref()
                .map(|s| s.uid == inputs.uid && s.unid == inputs.unid)
                .unwrap_or(false)
                && self
                    .identity
                    .anchor_coordinate()
                    .map(|a| a == inputs.anchor)
                    .unwrap_or(false)
            {
                self.run_page.preview_inputs = Some(inputs);
                self.run_page.preview_stale = true;
            }
        }
        if let Some(v) = done_login {
            self.login_busy = false;
            let val: serde_json::Value = serde_json::from_str(&v).unwrap_or_default();
            if val.get("ok").and_then(|b| b.as_bool()).unwrap_or(false) {
                // 直接从消息构造会话（写盘可能被占用，不依赖回读）
                if let Some(sess) = val
                    .get("session")
                    .and_then(|s| serde_json::from_value::<model::Session>(s.clone()).ok())
                {
                    self.auth_expired = false;
                    self.session = Some(sess);
                    self.status = format!(
                        "已登录：{}",
                        self.session
                            .as_ref()
                            .map(|s| s.name.clone())
                            .unwrap_or_default()
                    );
                } else {
                    // 兜底：从磁盘读（老版本消息格式）
                    let sess = model::load_session();
                    if sess.is_logged_in() {
                        self.auth_expired = false;
                        self.session = Some(sess);
                        self.status = format!(
                            "已登录：{}",
                            self.session
                                .as_ref()
                                .map(|s| s.name.clone())
                                .unwrap_or_default()
                        );
                    } else {
                        self.status = "登录成功但本地会话缺失".into();
                    }
                }
                // 登录成功后补齐全套数据（首次登录时项目列表等均为空）
                self.refresh_data_page();
                self.refresh_records();
                self.refresh_user_page();
                self.refresh_ai_list();
                self.refresh_points();
            } else {
                self.status = format!(
                    "登录失败：{}",
                    val["message"].as_str().unwrap_or("未知错误")
                );
            }
        }
        if let Some(v) = done_run {
            self.run_busy = false;
            let val: serde_json::Value = serde_json::from_str(&v).unwrap_or_default();
            if val.get("ok").and_then(|b| b.as_bool()).unwrap_or(false) {
                self.status = format!(
                    "跑步提交成功 rrid={}（OBS {}/2）",
                    val["rrid"].as_i64().unwrap_or(0),
                    val["obs_ok"].as_i64().unwrap_or(0)
                );
                self.popup = Some(run_popup(&val));
                self.refresh_data_page();
                self.run_page.preview_stale = true;
            } else {
                self.status = format!("跑步操作未完成：{}", val["message"].as_str().unwrap_or(""));
            }
        }
        if let Some(v) = done_ai {
            self.ai_submit_active = false;
            self.ai_busy = false;
            let val: serde_json::Value = serde_json::from_str(&v).unwrap_or_default();
            if val.get("ok").and_then(|b| b.as_bool()).unwrap_or(false) {
                self.status = "AI 运动提交成功".into();
                self.popup = Some(ai_popup(&val));
            } else {
                self.status = format!("AI 提交失败：{}", val["message"].as_str().unwrap_or(""));
            }
        }
        if let Some(v) = records_json {
            self.records_busy = false;
            match serde_json::from_str::<Vec<crate::api::records::RecordRow>>(&v) {
                Ok(rows) => {
                    self.status = format!("已刷新 {} 条记录", rows.len());
                    self.records_page.rows = rows;
                }
                Err(_) => self.status = "记录数据解析失败".into(),
            }
        }
        if let Some(v) = ai_list_json {
            self.ai_busy = self.ai_submit_active;
            match serde_json::from_str::<Vec<crate::api::ai::AiSport>>(&v) {
                Ok(list) => {
                    self.status = format!("已拉取 {} 个 AI 项目", list.len());
                    self.ai_page.list = list;
                    self.ai_page.selected = 0;
                }
                Err(_) => self.status = "AI 列表解析失败".into(),
            }
        }
        if let Some(v) = semester_json {
            match serde_json::from_str::<crate::api::semester::SemesterSummary>(&v) {
                Ok(sm) => self.data_page.semester = Some(sm),
                Err(_) => self.status = "学期数据解析失败".into(),
            }
        }
        if let Some(v) = cheat_json {
            self.data_page.cheat_checking = false;
            match serde_json::from_str::<crate::api::cheat::CheatReport>(&v) {
                Ok(report) => {
                    self.data_page.cheat = Some(report);
                    self.data_page.cheat_error = None;
                }
                Err(_) => {
                    self.data_page.cheat = None;
                    self.data_page.cheat_error = Some("违规数据解析失败，无法判断".into());
                }
            }
        }
        if let Some(v) = rank_json {
            match serde_json::from_str::<Vec<crate::api::rank::RankRow>>(&v) {
                Ok(rows) => self.data_page.rank_rows = rows,
                Err(_) => self.status = "排行榜解析失败".into(),
            }
        }
        if let Some(v) = user_json {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&v) {
                self.user_page.info = Some(crate::api::user::MyInfo {
                    errors: serde_json::from_value(
                        val.get("errors").cloned().unwrap_or(serde_json::json!([])),
                    )
                    .unwrap_or_default(),
                    profile: val
                        .get("profile")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                    home_page: val
                        .get("home_page")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                    personal_semester: val
                        .get("personal_semester")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                    summary: val
                        .get("summary")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                    completed: val
                        .get("completed")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                });
            }
        }
        if let Some(raw) = ai_detail_raw {
            self.records_page.ai_detail_raw = Some(raw);
            self.records_page.ai_detail_loading = false;
        }
        if let Some(v) = ai_records_json {
            self.records_busy = false;
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&v) {
                self.records_page.ai_groups = val
                    .get("groups")
                    .and_then(|x| serde_json::from_value(x.clone()).ok())
                    .unwrap_or_default();
                self.records_page.ai_total = val.get("total").and_then(|x| x.as_i64()).unwrap_or(0);
            }
        }
        if let Some(raw) = detail_raw {
            self.records_page.detail_raw = Some(raw);
            self.records_page.detail_loading = false;
        }
        if let Some(v) = update_chk {
            self.update.checking = false;
            let val: serde_json::Value = serde_json::from_str(&v).unwrap_or_default();
            if val.get("ok").and_then(|b| b.as_bool()).unwrap_or(false) {
                if val.get("newer").and_then(|b| b.as_bool()).unwrap_or(false) {
                    if let Ok(rel) = serde_json::from_value::<crate::update::ReleaseInfo>(
                        val.get("release")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null),
                    ) {
                        self.status = format!("发现新版本 {}", rel.tag);
                        self.update.latest = Some(rel.clone());
                        self.update.up_to_date = false;
                        self.update.confirm = Some(rel);
                    }
                } else {
                    self.update.up_to_date = true;
                    self.status = "已是最新版本".into();
                }
            } else {
                let msg = val
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("未知错误");
                self.update.check_error = Some(msg.to_string());
                self.status = format!("检查更新失败：{msg}");
            }
        }
        if let Some(v) = update_prog {
            if let Some((done, total)) = v.split_once('/') {
                self.update.done = done.parse().unwrap_or(0);
                let t: u64 = total.parse().unwrap_or(0);
                self.update.total = (t > 0).then_some(t);
            }
        }
        if let Some(v) = update_done {
            self.update.downloading = false;
            let val: serde_json::Value = serde_json::from_str(&v).unwrap_or_default();
            let tag = val
                .get("tag")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            if val.get("ok").and_then(|b| b.as_bool()).unwrap_or(false) {
                self.status = format!("√ 已更新到 {tag}");
                #[cfg(target_os = "android")]
                {
                    self.update.finish = Some(FinishAction::InstallApk { tag });
                }
                #[cfg(not(target_os = "android"))]
                {
                    self.update.finish = Some(FinishAction::Restart { tag });
                }
            } else {
                let msg = val
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("未知错误");
                self.update.check_error = Some(msg.to_string());
                self.status = format!("× 更新失败：{msg}");
                self.popup = Some(PopupInfo {
                    title: "更新失败".into(),
                    lines: vec![msg.to_string()],
                });
            }
        }
        if got_semester || got_cheat || got_rank {
            self.data_busy = false;
        }
        while let Ok(failure) = self.auth_rx.try_recv() {
            if failure.epoch == self.session_epoch
                && self
                    .session
                    .as_ref()
                    .is_some_and(|s| s.uid == failure.uid && s.token == failure.token)
            {
                self.expire_road_session();
            }
        }
        if self.auth_expired
            && !self.login_busy
            && !self.status.starts_with("登录失败")
            && !self.status.starts_with("请填写")
        {
            self.status = crate::api::client::SESSION_EXPIRED.into();
        }
    }
}

fn fmt_hms(ms: i64) -> String {
    chrono::Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default()
}

fn fmt_dur(sec: i64) -> String {
    format!("{}:{:02}:{:02}", sec / 3600, sec % 3600 / 60, sec % 60)
}

fn run_popup(v: &serde_json::Value) -> PopupInfo {
    let dist = v["dist"].as_f64().unwrap_or(0.0);
    let dur = v["dur"].as_i64().unwrap_or(0);
    let pace = if dist > 0.0 {
        dur as f64 / (dist / 1000.0)
    } else {
        0.0
    };
    let mut lines = vec![
        format!("记录号：{}", v["rrid"].as_i64().unwrap_or(0)),
        format!("UUID：{}", v["uuid"].as_str().unwrap_or("")),
        format!(
            "距离：{:.2} km（达标线 {} m）",
            dist / 1000.0,
            v["sel_distance"].as_i64().unwrap_or(0)
        ),
        format!(
            "时长：{} · 配速 {}:{:02}/km",
            fmt_dur(dur),
            (pace / 60.0) as i64,
            (pace as i64) % 60
        ),
        format!(
            "步数：{}（步频 {} spm）",
            v["steps"].as_i64().unwrap_or(0),
            v["avg_step_freq"].as_i64().unwrap_or(0)
        ),
        format!(
            "卡路里：{} kcal · 功率 {} W",
            v["calorie"].as_i64().unwrap_or(0),
            v["avg_power"].as_i64().unwrap_or(0)
        ),
        format!("开始时间：{}", fmt_hms(v["start"].as_i64().unwrap_or(0))),
        "记录提交：成功".into(),
        format!("OBS 上传：{}/2", v["obs_ok"].as_i64().unwrap_or(0)),
    ];
    if let Ok(verification) =
        serde_json::from_value::<crate::api::flow::Verification>(v["verification"].clone())
    {
        lines.push(format!(
            "详情读取：{}",
            if verification.detail.is_some() {
                "成功"
            } else {
                "失败"
            }
        ));
        lines.push(format!("服务端达标判定：{}", verification.label()));
        if let Some(error) = verification.error {
            lines.push(format!("详情错误：{error}"));
        }
        if let Some(list) = verification
            .detail
            .as_ref()
            .and_then(|d| d.get("reasonList"))
            .and_then(|v| v.as_array())
        {
            for r in list {
                let label = match r.get("complete").and_then(|v| v.as_bool()) {
                    Some(true) => "达标",
                    Some(false) => "未达标",
                    None => "未知",
                };
                lines.push(format!(
                    "判定：{} {}",
                    label,
                    r["reason"].as_str().unwrap_or("")
                ));
            }
        }
    } else {
        lines.push("详情读取：未知；服务端达标判定：未知".into());
    }
    PopupInfo {
        title: "跑步结果".into(),
        lines,
    }
}

fn ai_popup(v: &serde_json::Value) -> PopupInfo {
    if v["batch"].as_bool().unwrap_or(false) {
        return PopupInfo {
            title: "AI 批量补签结果".into(),
            lines: vec![
                format!(
                    "成功：{}/{}",
                    v["success"].as_i64().unwrap_or(0),
                    v["total"].as_i64().unwrap_or(0)
                ),
                format!(
                    "覆盖：{} 天 × 每天 {} 次 × {} 个项目",
                    v["days"].as_i64().unwrap_or(0),
                    v["per_day"].as_i64().unwrap_or(0),
                    v["sports"].as_i64().unwrap_or(0)
                ),
            ],
        };
    }
    PopupInfo {
        title: "AI 运动提交结果".into(),
        lines: vec![
            format!("项目 sportId：{}", v["sport_id"].as_i64().unwrap_or(0)),
            format!("模式：{}", v["mode"].as_str().unwrap_or("")),
            format!("成绩：{}", v["score"].as_str().unwrap_or("")),
            format!(
                "用时：{:.1} 秒 · 速度 {}/分 · 消耗 {:.1}",
                v["secs"].as_f64().unwrap_or(0.0),
                v["per_min"].as_i64().unwrap_or(0),
                v["consume"].as_f64().unwrap_or(0.0),
            ),
            format!(
                "服务器：{} {}",
                v["resp_error"].as_i64().unwrap_or(0),
                v["resp_msg"].as_str().unwrap_or("")
            ),
        ],
    }
}

#[cfg(test)]
mod result_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn popup_separates_partial_upload_reading_and_grade() {
        let failed = crate::api::flow::Verification::from_result(Ok(
            json!({"complete":false,"reasonList":[{"complete":false,"reason":"点位未经过"}]}),
        ));
        let first = run_popup(&json!({"rrid":1,"obs_ok":1,"verification":failed}))
            .lines
            .join("\n");
        assert!(first.contains("OBS 上传：1/2"));
        assert!(first.contains("详情读取：成功"));
        assert!(first.contains("服务端达标判定：未达标"));
        let next = crate::api::flow::Verification::from_result(Err("offline".into()));
        let second = run_popup(&json!({"rrid":2,"obs_ok":2,"verification":next}))
            .lines
            .join("\n");
        assert!(second.contains("详情读取：失败"));
        assert!(second.contains("服务端达标判定：未知"));
        assert!(!second.contains("点位未经过"));
    }
}
