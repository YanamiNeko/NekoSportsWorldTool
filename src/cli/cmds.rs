//! CLI 子命令实现。

use super::{fmt_hms, get, jstr, logger, make_client, now_ms, parse_flags, parse_pace, print_rows};
use crate::api::ai::AiMode;
use crate::api::client::ApiClient;
use crate::api::model;

pub fn dispatch(args: Vec<String>) -> i32 {
    let cmd = args.first().map(|x| x.as_str()).unwrap_or("help");
    let rest = args.iter().skip(1).map(|x| x.as_str()).collect::<Vec<_>>();
    match cmd {
        "login" => cmd_login(&rest),
        "logout" => cmd_logout(),
        "run" => cmd_run(&rest),
        "ai" => cmd_ai(&rest),
        "ai-list" => cmd_ai_list(),
        "records" => cmd_records(),
        "ai-records" => cmd_ai_records(&rest),
        "ai-info" => cmd_ai_info(&rest),
        "semester" => cmd_semester(),
        "cheat" => cmd_cheat(&rest),
        "rank" => cmd_rank(&rest),
        "help" | "--help" | "-h" => {
            usage();
            0
        }
        _ => {
            eprintln!("未知命令: {cmd}");
            super::usage();
            1
        }
    }
}

fn usage() {
    super::usage();
}

fn cmd_login(rest: &[&str]) -> i32 {
    let flags = parse_flags(rest);
    let (Some(user), Some(pw)) = (get(&flags, "user"), get(&flags, "pass")) else {
        eprintln!("缺少 --user / --pass");
        return 1;
    };
    let identity = model::load_identity();
    let mut client = ApiClient::new(identity, None);
    let mut log = logger();
    match crate::api::login::login(&mut client, user, pw, &mut log) {
        Ok(s) => {
            if get(&flags, "remember").is_some() {
                let mut cfg = model::load_config();
                cfg.username = user.to_string();
                cfg.password = pw.to_string();
                cfg.remember = true;
                let _ = model::save_config(&cfg);
                println!("凭据已保存（会话失效时自动重登）");
            }
            println!("登录成功 uid={} unid={} name={}", s.uid, s.unid, s.name);
            0
        }
        Err(e) => {
            eprintln!("登录失败: {e}");
            1
        }
    }
}

fn cmd_logout() -> i32 {
    let identity = model::load_identity();
    let sess = model::load_session();
    let mut client = ApiClient::new(identity, sess.is_logged_in().then_some(sess));
    let mut log = logger();
    crate::api::login::logout(&mut client, &mut log);
    0
}

fn cmd_run(rest: &[&str]) -> i32 {
    let flags = parse_flags(rest);
    let mut client = match make_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let dist_km: f32 = get(&flags, "dist").and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let pace: f32 = get(&flags, "pace").map(parse_pace).unwrap_or(0.0);
    let ago_min: i64 = get(&flags, "ago").and_then(|v| v.parse().ok()).unwrap_or(0);
    let days_ago: i64 = get(&flags, "days-ago").and_then(|v| v.parse().ok()).unwrap_or(0).clamp(0, 3);
    let time_spec = get(&flags, "time").unwrap_or("");
    let face = get(&flags, "face").map(|v| v == "1" || v == "true").unwrap_or(true);
    let seed: u64 = get(&flags, "seed").and_then(|v| v.parse().ok()).unwrap_or(0);
    let seed = if seed == 0 { (now_ms() % 2_147_483_647) as u64 } else { seed };

    let dist = if dist_km > 0.0 {
        dist_km as f64 * 1000.0
    } else {
        (1.0 + rand::random::<f32>() * 0.5) as f64 * 1000.0
    };
    let pace_s = if pace > 0.0 { pace } else { 360.0 + rand::random::<f32>() * 120.0 };
    let dur = (dist as f32 / 1000.0 * pace_s) as i64;
    let start_ms = if !time_spec.is_empty() {
        // --time "HH:MM" 配合 --days-ago（0-3）
        let (h, m) = time_spec.split_once(':').unwrap_or((time_spec, "0"));
        let (h, m) = (h.parse::<u32>().unwrap_or(7) % 24, m.parse::<u32>().unwrap_or(0));
        let base = chrono::Local::now() - chrono::Duration::days(days_ago);
        use chrono::{Datelike, TimeZone};
        chrono::Local
            .with_ymd_and_hms(base.year(), base.month(), base.day(), h, m, 0)
            .single()
            .map(|x| x.timestamp_millis())
            .unwrap_or(now_ms())
    } else if ago_min > 0 {
        now_ms() - ago_min * 60_000
    } else {
        now_ms() - 30 * 60_000 - (rand::random::<f64>() * 270.0 * 60_000.0) as i64
    };

    println!(
        "参数：{:.0}m / {}s / 配速 {}:{:02}/km / 开始 {}",
        dist,
        dur,
        pace_s as i64 / 60,
        pace_s as i64 % 60,
        fmt_hms(start_ms)
    );

    let mut log = logger();
    let params = crate::api::flow::RunParams { dist, dur, start_ms, face_check: face as i64, seed };
    match crate::api::flow::run_full_flow(&mut client, &params, &mut log) {
        Ok(out) => {
            println!(
                "跑步提交成功 rrid={} uuid={} obs={}/2 verify={}",
                out.result.rrid,
                out.result.uuid,
                out.obs_ok,
                if out.detail_ok { "通过" } else { "未通过" }
            );
            0
        }
        Err(e) => {
            eprintln!("跑步提交失败: {e}");
            1
        }
    }
}

fn cmd_ai(rest: &[&str]) -> i32 {
    let flags = parse_flags(rest);
    let Some(sport) = get(&flags, "sport").and_then(|v| v.parse().ok()) else {
        eprintln!("缺少 --sport");
        return 1;
    };
    // 按分钟（--minutes 1-30）或按次（--count 5-1000 步长 5）
    let mode = match get(&flags, "mode").unwrap_or("min") {
        "count" => {
            let reps = get(&flags, "score")
                .or_else(|| get(&flags, "count"))
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(5)
                .clamp(5, 1000);
            AiMode::Count { reps: (reps / 5) * 5 }
        }
        _ => {
            let minutes = get(&flags, "score")
                .or_else(|| get(&flags, "minutes"))
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(1)
                .clamp(1, 30);
            AiMode::Minutes { minutes }
        }
    };
    let mut client = match make_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let mut log = logger();
    match crate::api::flow::run_ai_submit(&mut client, sport, mode, &mut log) {
        Ok(biz) => {
            println!(
                "AI 提交成功 服务器={} {}",
                biz.get("error").and_then(|e| e.as_i64()).unwrap_or(0),
                biz.get("message").and_then(|m| m.as_str()).unwrap_or("")
            );
            0
        }
        Err(e) => {
            eprintln!("AI 提交失败: {e}");
            1
        }
    }
}

fn cmd_ai_list() -> i32 {
    let mut client = match make_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let mut log = logger();
    match crate::api::flow::run_ai_list(&mut client, &mut log) {
        Ok(list) => {
            for s in list {
                println!("id={:<4} {}", s.id, s.name);
            }
            0
        }
        Err(e) => {
            eprintln!("拉取失败: {e}");
            1
        }
    }
}

fn cmd_records() -> i32 {
    let mut client = match make_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let mut log = logger();
    match crate::api::flow::run_records(&mut client, &mut log) {
        Ok(rows) => {
            print_rows(&rows);
            0
        }
        Err(e) => {
            eprintln!("拉取失败: {e}");
            1
        }
    }
}

/// 单条 AI 记录全量详情。
fn cmd_ai_info(rest: &[&str]) -> i32 {
    let flags = parse_flags(rest);
    let Some(id) = get(&flags, "id").and_then(|v| v.parse::<i64>().ok()) else {
        eprintln!("缺少 --id");
        return 1;
    };
    let mut client = match make_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    match crate::api::ai::fetch_record_detail(&mut client, id) {
        Ok(v) => println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default()),
        Err(e) => {
            eprintln!("拉取失败: {e}");
            return 1;
        }
    }
    0
}

fn cmd_ai_records(rest: &[&str]) -> i32 {
    let flags = parse_flags(rest);
    let Some(sport) = get(&flags, "sport").and_then(|v| v.parse().ok()) else {
        eprintln!("缺少 --sport");
        return 1;
    };
    let mut client = match make_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    match crate::api::ai::fetch_records(&mut client, sport, 50) {
        Ok(page) => {
            use chrono::TimeZone;
            for g in page.groups {
                let date = chrono::Local
                    .timestamp_millis_opt(g.score_date)
                    .single()
                    .map(|t| t.format("%Y-%m-%d").to_string())
                    .unwrap_or_default();
                println!("{date} ×{}：", g.frequency);
                for r in g.records {
                    let grade = if r.rtype == 2 {
                        format!("{:.1} 秒", r.score.parse::<f64>().unwrap_or(0.0) / 1000.0)
                    } else {
                        format!("{} 个", r.score)
                    };
                    let finish = chrono::Local
                        .timestamp_millis_opt(r.score_date)
                        .single()
                        .map(|t| t.format("%H:%M:%S").to_string())
                        .unwrap_or_default();
                    let video = if r.has_video { "有" } else { "-" };
                    println!("  {} {} 完成 {finish} 视频 {video}", r.name, grade);
                }
            }
            0
        }
        Err(e) => {
            eprintln!("拉取失败: {e}");
            1
        }
    }
}

fn cmd_semester() -> i32 {
    let mut client = match make_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let mut log = logger();
    match crate::api::semester::query(&mut client, &mut log) {
        Ok(r) => {
            if let Some(s) = r.summary {
                println!(
                    "学期：{}  有效次数：{}/{}  有效里程：{:.2} km（总 {:.2} km）",
                    s.sname,
                    s.semester_valid_count,
                    s.semester_count,
                    s.semester_valid_dis / 1000.0,
                    s.semester_dis / 1000.0
                );
            }
            if !r.personal_raw.is_null() {
                println!("个人完成度：{}", r.personal_raw);
            }
            0
        }
        Err(e) => {
            eprintln!("拉取失败: {e}");
            1
        }
    }
}

fn cmd_cheat(rest: &[&str]) -> i32 {
    let flags = parse_flags(rest);
    let page: i64 = get(&flags, "page").and_then(|v| v.parse().ok()).unwrap_or(1);
    let mut client = match make_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let mut log = logger();
    match crate::api::cheat::query(&mut client, page, &mut log) {
        Ok(r) => {
            if r.is_clean() {
                println!("自查：干净（self=null）");
            } else {
                println!("已被标记：{}", r.self_brief());
            }
            println!("全校违规 {} 条", r.list.len());
            for item in r.list.iter().take(20) {
                println!(
                    "  {} | {} | {}",
                    jstr(item, &["name", "userName"]),
                    jstr(item, &["reason", "punishReason", "cause"]),
                    jstr(item, &["createTime", "time", "date"]),
                );
            }
            0
        }
        Err(e) => {
            eprintln!("检查失败: {e}");
            1
        }
    }
}

fn cmd_rank(rest: &[&str]) -> i32 {
    let kind = rest.first().copied().unwrap_or("main");
    let flags = parse_flags(&rest.iter().skip(1).copied().collect::<Vec<_>>());
    let mut client = match make_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let res = match kind {
        "indoor" => {
            let range: i64 = get(&flags, "range").and_then(|v| v.parse().ok()).unwrap_or(1);
            crate::api::rank::indoor_rank(&mut client, range, -1)
        }
        "history" => {
            let sort: i64 = get(&flags, "sort").and_then(|v| v.parse().ok()).unwrap_or(1);
            crate::api::rank::history_rank(&mut client, sort, -1)
        }
        _ => {
            let rtype: i64 = get(&flags, "type").and_then(|v| v.parse().ok()).unwrap_or(1);
            let sort: i64 = get(&flags, "sort").and_then(|v| v.parse().ok()).unwrap_or(1);
            let gender = get(&flags, "gender").and_then(|v| v.parse().ok());
            let date = get(&flags, "date").map(|v| v.to_string());
            crate::api::rank::main_rank(&mut client, rtype, sort, gender, date)
        }
    };
    match res {
        Ok(rows) => {
            for r in rows {
                println!("{:<4} {}  {:.2} km", r.sort, r.name, r.length / 1000.0);
            }
            0
        }
        Err(e) => {
            eprintln!("查询失败: {e}");
            1
        }
    }
}
