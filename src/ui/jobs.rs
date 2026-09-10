//! 后台任务与消息处理：IP 获取 / 登录 / 刷新 / 消息协议分发。

use super::{App, AI_RECORDS, AI_LIST, CHEAT, IP, LOGIN_DONE, RANK, RECORDS, RUN_DETAIL, SEMESTER, USER};
use crate::api::model;

impl App {
    pub(crate) fn fetch_ip(&mut self) {
        self.spawn_job(|tx| {
            let agent = crate::api::client::make_agent();
            let mut log = App::logger(tx.clone());
            let mut ip = String::new();
            for url in ["https://api.ipify.org?format=json", "https://httpbin.org/ip"] {
                if let Ok(resp) = agent.get(url).call() {
                    if let Ok(text) = resp.into_string() {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                            let got = v
                                .get("ip")
                                .or_else(|| v.get("origin"))
                                .and_then(|x| x.as_str())
                                .unwrap_or("")
                                .to_string();
                            if !got.is_empty() {
                                ip = got;
                                break;
                            }
                        }
                    }
                }
            }
            if ip.is_empty() {
                log("公网 IP 获取失败");
                tx.send(format!("{IP}失败")).ok();
            } else {
                log(&format!("公网 IP：{ip}"));
                tx.send(format!("{IP}{ip}")).ok();
            }
        });
    }

    pub(crate) fn do_login(&mut self) {
        if self.username.is_empty() || self.password.is_empty() {
            self.status = "请填写账号和密码".into();
            return;
        }
        let username = self.username.trim().to_string();
        let password = self.password.clone();
        let identity = self.identity.clone();
        let remember = self.remember;
        self.config.username = username.clone();
        self.config.password = if remember { password.clone() } else { String::new() };
        self.config.remember = remember;
        let _ = model::save_config(&self.config);
        self.login_busy = true;
        self.status = format!("登录中（{username}）…");
        self.spawn_job(move |tx| {
            let mut log = App::logger(tx.clone());
            let mut client = crate::api::client::ApiClient::new(identity, None);
            let msg = match crate::api::login::login(&mut client, &username, &password, &mut log) {
                Ok(sess) => {
                    log(&format!(
                        "√ 登录成功 uid={} unid={} name={}",
                        sess.uid, sess.unid, sess.name
                    ));
                    let payload = serde_json::json!({
                        "ok": true,
                        "session": {
                            "uid": sess.uid, "token": sess.token,
                            "unid": sess.unid, "name": sess.name,
                            "weight": sess.weight, "username": username,
                            "device_id": sess.device_id,
                            "profile": sess.profile,
                        },
                    });
                    format!("{LOGIN_DONE}{payload}")
                }
                Err(e) => {
                    log(&format!("× 登录失败: {e}"));
                    format!("{LOGIN_DONE}{}", serde_json::json!({"ok": false, "message": e}))
                }
            };
            tx.send(msg).ok();
        });
    }

    pub(crate) fn do_logout(&mut self) {
        let identity = self.identity.clone();
        let session = self.session.clone();
        self.session = None;
        self.spawn_job(move |tx| {
            let mut log = App::logger(tx.clone());
            if let Some(sess) = session {
                let mut client = crate::api::client::ApiClient::new(identity, Some(sess));
                crate::api::login::logout(&mut client, &mut log);
            }
        });
        self.status = "已登出".into();
    }

    pub fn refresh_ai_list(&mut self) {
        let Some(session) = self.session.clone() else {
            self.status = "请先登录".into();
            return;
        };
        let identity = self.identity.clone();
        self.ai_busy = true;
        self.status = "AI 列表拉取中…".into();
        self.spawn_job(move |tx| {
            let mut log = App::logger(tx.clone());
            let mut client = crate::api::client::ApiClient::new(identity, Some(session));
            let payload = match crate::api::flow::run_ai_list(&mut client, &mut log) {
                Ok(list) => serde_json::to_string(&list).unwrap_or_else(|_| "[]".into()),
                Err(e) => {
                    log(&format!("× AI 列表拉取失败: {e}"));
                    "[]".to_string()
                }
            };
            tx.send(format!("{AI_LIST}{payload}")).ok();
        });
    }

    pub fn refresh_records(&mut self) {
        let Some(session) = self.session.clone() else {
            self.status = "请先登录".into();
            return;
        };
        let identity = self.identity.clone();
        self.records_busy = true;
        self.status = "记录拉取中…".into();
        self.spawn_job(move |tx| {
            let mut log = App::logger(tx.clone());
            let mut client = crate::api::client::ApiClient::new(identity, Some(session));
            let payload = match crate::api::flow::run_records(&mut client, &mut log) {
                Ok(rows) => serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into()),
                Err(e) => {
                    log(&format!("× 记录拉取失败: {e}"));
                    "[]".to_string()
                }
            };
            tx.send(format!("{RECORDS}{payload}")).ok();
        });
    }

    /// 我的页数据聚合。
    pub fn refresh_user_page(&mut self) {
        let Some(session) = self.session.clone() else {
            self.status = "请先登录".into();
            return;
        };
        let identity = self.identity.clone();
        self.user_busy = true;
        self.spawn_job(move |tx| {
            let mut log = App::logger(tx.clone());
            let mut client = crate::api::client::ApiClient::new(identity, Some(session));
            let info = crate::api::user::fetch_my_info(&mut client, &mut log);
            let payload = serde_json::json!({
                "profile": info.profile,
                "home_page": info.home_page,
                "personal_semester": info.personal_semester,
                "summary": info.summary,
                "completed": info.completed,
            });
            log("√ [user] 我的页数据已更新");
            tx.send(format!("{USER}{payload}")).ok();
            tx.send(format!("{USER}done")).ok();
        });
    }

    /// 拉取单条跑步详情。
    pub fn fetch_run_detail(&mut self, rrid: i64) {
        let Some(session) = self.session.clone() else { return };
        let identity = self.identity.clone();
        self.records_page.detail_loading = true;
        self.spawn_job(move |tx| {
            let mut client = crate::api::client::ApiClient::new(identity, Some(session));
            match crate::api::records::fetch_detail(&mut client, rrid) {
                Ok(d) => {
                    let payload = serde_json::to_string(&d).unwrap_or_default();
                    tx.send(format!("{RUN_DETAIL}{payload}")).ok();
                }
                Err(e) => {
                    let payload = serde_json::json!({ "statusInfo": format!("详情拉取失败：{e}") });
                    tx.send(format!("{RUN_DETAIL}{payload}")).ok();
                }
            }
        });
    }

    /// AI 记录：遍历全部项目合并（按天排序）。
    pub fn refresh_ai_records(&mut self) {
        let Some(session) = self.session.clone() else {
            self.status = "请先登录".into();
            return;
        };
        let identity = self.identity.clone();
        self.records_busy = true;
        self.spawn_job(move |tx| {
            let mut log = App::logger(tx.clone());
            let mut client = crate::api::client::ApiClient::new(identity, Some(session));
            // 先拉项目列表
            let list = match crate::api::ai::fetch_list(&mut client) {
                Ok(l) => l,
                Err(e) => {
                    log(&format!("× [ai] 项目列表拉取失败: {e}"));
                    tx.send(format!("{AI_RECORDS}{{\"groups\":[],\"total\":0}}")).ok();
                    return;
                }
            };
            let mut all_groups: Vec<crate::api::ai::AiRecordGroup> = Vec::new();
            let mut total = 0i64;
            for sport in &list {
                match crate::api::ai::fetch_records(&mut client, sport.id, 50) {
                    Ok(page) => {
                        for mut g in page.groups {
                            for r in &mut g.records {
                                r.name = format!("{} {}", sport.name, r.name);
                            }
                            all_groups.push(g);
                        }
                        total += page.total_count;
                    }
                    Err(e) => log(&format!("× [ai] {} 拉取失败: {e}", sport.name)),
                }
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
            all_groups.sort_by(|a, b| b.score_date.cmp(&a.score_date));
            let count: usize = all_groups.iter().map(|g| g.records.len()).sum();
            log(&format!("√ [ai] {} 个项目共 {} 条记录", list.len(), count));
            let payload = serde_json::json!({ "groups": all_groups, "total": total });
            tx.send(format!("{AI_RECORDS}{payload}")).ok();
            tx.send("__AI_RECORDS_DONE__".to_string()).ok();
        });
    }

    /// 学期 + 违规自查（登录 / 跑步提交成功后自动调用）。
    pub fn refresh_data_page(&mut self) {
        let Some(session) = self.session.clone() else { return };
        let identity = self.identity.clone();
        self.data_busy = true;
        self.spawn_job(move |tx| {
            let mut log = App::logger(tx.clone());
            let mut client = crate::api::client::ApiClient::new(identity, Some(session));
            match crate::api::semester::query(&mut client, &mut log) {
                Ok(r) => {
                    if let Some(s) = r.summary {
                        log(&format!(
                            "√ [semester] {} 有效 {}/{} 次，有效里程 {:.2} km",
                            s.sname, s.semester_valid_count, s.semester_count,
                            s.semester_valid_dis / 1000.0
                        ));
                        let payload = serde_json::to_string(&s).unwrap_or_default();
                        tx.send(format!("{SEMESTER}{payload}")).ok();
                    } else {
                        tx.send(format!("{SEMESTER}{{}}")).ok();
                    }
                }
                Err(e) => {
                    log(&format!("× [semester] 拉取失败: {e}"));
                    tx.send(format!("{SEMESTER}{{}}")).ok();
                }
            }
            match crate::api::cheat::query(&mut client, 1, &mut log) {
                Ok(r) => {
                    let payload = serde_json::json!({ "self": r.self_info, "list": r.list });
                    tx.send(format!("{CHEAT}{}", payload)).ok();
                }
                Err(e) => {
                    log(&format!("× [cheat] 检查失败: {e}"));
                    tx.send(format!("{CHEAT}{{\"self\":null,\"list\":[]}}")).ok();
                }
            }
        });
    }

    pub fn refresh_cheat_only(&mut self) {
        let Some(session) = self.session.clone() else {
            self.status = "请先登录".into();
            return;
        };
        let identity = self.identity.clone();
        self.data_busy = true;
        self.spawn_job(move |tx| {
            let mut log = App::logger(tx.clone());
            let mut client = crate::api::client::ApiClient::new(identity, Some(session));
            match crate::api::cheat::query(&mut client, 1, &mut log) {
                Ok(r) => {
                    let payload = serde_json::json!({ "self": r.self_info, "list": r.list });
                    tx.send(format!("{CHEAT}{}", payload)).ok();
                }
                Err(e) => {
                    log(&format!("× [cheat] 检查失败: {e}"));
                    tx.send(format!("{CHEAT}{{\"self\":null,\"list\":[]}}")).ok();
                }
            }
        });
    }

    /// 榜单查询：kind=main|indoor|history。
    pub fn refresh_rank(&mut self, kind: &str, subtype: i64) {
        let Some(session) = self.session.clone() else {
            self.status = "请先登录".into();
            return;
        };
        let identity = self.identity.clone();
        let kind = kind.to_string();
        self.data_busy = true;
        self.spawn_job(move |tx| {
            let mut log = App::logger(tx.clone());
            let mut client = crate::api::client::ApiClient::new(identity, Some(session));
            let res: Result<Vec<crate::api::rank::RankRow>, String> = (|| {
                match kind.as_str() {
                    "main" => {
                        // 当天无数据时自动回退最近 3 天（当日累计榜凌晨常为空）
                        let (rtype, sort_type) = (subtype / 10, subtype % 10);
                        let mut rows =
                            crate::api::rank::main_rank(&mut client, rtype, sort_type, None, None)?;
                        let mut back = 0;
                        while rows.is_empty() && back < 3 {
                            back += 1;
                            let d = (chrono::Local::now() - chrono::Duration::days(back))
                                .format("%Y-%m-%d")
                                .to_string();
                            log(&format!("[rank] 当天暂无数据，回退查询 {d}"));
                            rows = crate::api::rank::main_rank(
                                &mut client,
                                rtype,
                                sort_type,
                                None,
                                Some(d),
                            )?;
                        }
                        Ok(rows)
                    }
                    "indoor" => crate::api::rank::indoor_rank(&mut client, subtype, -1),
                    _ => crate::api::rank::history_rank(&mut client, subtype, -1),
                }
            })();
            let rows = match res {
                Ok(r) => r,
                Err(e) => {
                    log(&format!("× [rank] 查询失败: {e}"));
                    Vec::new()
                }
            };
            log(&format!("[rank] {} 行", rows.len()));
            tx.send(format!(
                "{RANK}{}",
                serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into())
            )).ok();
        });
    }
}
