//! 命令行入口：子命令分发 + 共享助手。
//! 子命令实现见 ；数据文件与 GUI 共用（exe 同目录）。
//!
//! 青龙定时任务：首次 `login --user .. --pass .. --remember`，之后定时 `run`
//! 即可（会话失效且已记住凭据时自动重登）。

mod cmds;

use crate::api::client::ApiClient;
use crate::api::model::{self, Session};
use crate::api::records::RecordRow;

pub fn main(args: Vec<String>) -> i32 {
    cmds::dispatch(args)
}

fn usage() {
    println!(
        r#"NekoSportsWorldTool CLI（full 构建无参数启动 GUI）

  login   --user <手机号> --pass <密码> [--remember]
                                           登录并保存会话；--remember 同时保存凭据供自动重登
  logout                                   登出并清理本地会话
  run    [--dist km] [--pace 秒/km] [--ago 分钟] [--days-ago 0-3 --time HH:MM] [--face 0|1] [--seed n]
                                           跑步全链：策略-点位-轨迹-提交-OBS-验证
  ai-list                                  AI 运动项目列表
  ai     --sport <id> [--mode min|count] [--score n]
                                           AI 运动：min 按分钟 1-30；count 按次 5-1000 步长 5
  records                                  跑步记录列表
  ai-records --sport <id>                  AI 运动记录（按天分组）
  semester                                 学期完成度
  cheat  [--page n]                        违规自查
  rank   main   --type 1|2|3 --sort 1|2 [--gender 0|1] [--date YYYY-MM-DD]
                                           排行榜（1个人 2班级 3院系；1日 2月）
  rank   indoor --range 1|2|3 [--gender 0|1]
                                           室内榜（1日 2周 3月）
  rank   history --sort 1|2 [--gender 0|1] 历史榜

当天榜单通常在有效里程产生后才有数据；run 默认随机 1.0~1.5 km / 6~8 分配速 / 30-300 分钟前。"#
    );
}

pub(crate) fn parse_flags(rest: &[&str]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        let k = rest[i].trim_start_matches('-').to_string();
        let v = rest
            .get(i + 1)
            .filter(|n| !n.starts_with('-') && *n != &"")
            .map(|s| s.to_string());
        match v {
            Some(v) => {
                out.push((k, v));
                i += 2;
            }
            None => {
                out.push((k, "1".into()));
                i += 1;
            }
        }
    }
    out
}

pub(crate) fn get<'a>(flags: &'a [(String, String)], name: &str) -> Option<&'a str> {
    flags.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

pub(crate) fn logger() -> impl FnMut(&str) {
    |s: &str| println!("{}", crate::textlog::clean(s))
}

pub(crate) fn silent() -> impl FnMut(&str) {
    |_s: &str| {}
}

/// 自动登录（GUI 记住密码 / login --remember 落盘的凭据）。
pub(crate) fn auto_login(identity: &model::HeaderIdentity) -> Result<Session, String> {
    let cfg = model::load_config();
    if cfg.remember && !cfg.username.is_empty() && !cfg.password.is_empty() {
        println!("[login] 使用已保存的账号自动登录…");
        let mut client = ApiClient::new(identity.clone(), None);
        return crate::api::login::login(&mut client, &cfg.username, &cfg.password, &mut silent());
    }
    Err("未登录：先执行 login 子命令，或在 GUI 勾选记住密码".into())
}

/// 构造已登录客户端；本地会话失效时自动重登一次。
pub(crate) fn make_client() -> Result<ApiClient, String> {
    let identity = model::load_identity();
    let client = {
        let sess = model::load_session();
        if sess.is_logged_in() {
            ApiClient::new(identity, Some(sess))
        } else {
            let s = auto_login(&identity)?;
            ApiClient::new(identity, Some(s))
        }
    };
    // 不做前置健康检查：业务错误（如新账号无学期数据）不等于会话过期，
    // 贸然重登会把好 session 删掉并触发 10121。会话真正过期时业务接口
    // 自然返回 401，由调用方处理。
    Ok(client)
}

pub(crate) fn now_ms() -> i64 {
    crate::crypto::envelope::now_ms()
}

pub(crate) fn fmt_hms(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default()
}


pub(crate) fn print_rows(rows: &[RecordRow]) {
    println!(
        "{:<16} {:>8} {:>8} {:>6} {:>6} {:>4} rrid",
        "时间", "距离m", "时长", "配速", "步频", "达标"
    );
    for r in rows {
        let t = fmt_hms(r.start_time)[5..].to_string();
        let pace = if r.total_dis > 0.0 && r.total_time > 0 {
            let p = r.total_time as f64 / (r.total_dis / 1000.0);
            format!("{}:{:02}", (p / 60.0) as i64, (p as i64) % 60)
        } else {
            "-".into()
        };
        println!(
            "{:<16} {:>8.0} {:>8} {:>6} {:>6} {:>4} {}",
            t,
            r.total_dis,
            format!("{}:{:02}:{:02}", r.total_time / 3600, r.total_time % 3600 / 60, r.total_time % 60),
            pace,
            r.avg_step_freq,
            if r.complete { "是" } else { "否" },
            r.rrid
        );
    }
}

pub(crate) fn jstr(v: &serde_json::Value, keys: &[&str]) -> String {
    for k in keys {
        match v.get(k) {
            Some(serde_json::Value::String(s)) => return s.clone(),
            Some(serde_json::Value::Number(n)) => return n.to_string(),
            _ => {}
        }
    }
    "-".into()
}

pub(crate) fn parse_pace(v: &str) -> f32 {
    if let Some((m, s)) = v.split_once(':') {
        m.parse::<f32>().unwrap_or(0.0) * 60.0 + s.parse::<f32>().unwrap_or(0.0)
    } else {
        v.parse().unwrap_or(0.0)
    }
}

