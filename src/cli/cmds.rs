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
        "template" => cmd_template(&rest),
        "ai" => cmd_ai(&rest),
        "ai-list" => cmd_ai_list(),
        "records" => cmd_records(),
        "records-raw" => cmd_records_raw(&rest),
        "record-info" => cmd_record_info(&rest),
        "obs-get" => cmd_obs_get(&rest),
        "obs-sample" => cmd_obs_sample(&rest),
        "ai-records" => cmd_ai_records(&rest),
        "ai-info" => cmd_ai_info(&rest),
        "semester" => cmd_semester(),
        "cheat" => cmd_cheat(&rest),
        "rank" => cmd_rank(&rest),
        "update" => cmd_update(&rest),
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
    let dist_km: f32 = get(&flags, "dist")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);
    let pace: f32 = get(&flags, "pace").map(parse_pace).unwrap_or(0.0);
    let ago_min: i64 = get(&flags, "ago").and_then(|v| v.parse().ok()).unwrap_or(0);
    let days_ago: i64 = get(&flags, "days-ago")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
        .clamp(0, 3);
    let time_spec = get(&flags, "time").unwrap_or("");
    let face = get(&flags, "face")
        .map(|v| v == "1" || v == "true")
        .unwrap_or(true);
    let (manual_altitude, manual_altitude_range) = match get(&flags, "altitude") {
        Some(value) => match crate::track::altitude::parse_spec(value) {
            Ok(Some(crate::track::altitude::AltitudeSpec::Single(value))) => (Some(value), None),
            Ok(Some(crate::track::altitude::AltitudeSpec::Range(range))) => (None, Some(range)),
            Ok(None) => (None, None),
            Err(e) => {
                eprintln!("--altitude {e}");
                return 1;
            }
        },
        None => (None, None),
    };
    let seed: u64 = get(&flags, "seed")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let seed = if seed == 0 {
        (now_ms() % 2_147_483_647) as u64
    } else {
        seed
    };

    let dist = if dist_km > 0.0 {
        dist_km as f64 * 1000.0
    } else {
        (1.0 + rand::random::<f32>() * 0.5) as f64 * 1000.0
    };
    let pace_s = if pace > 0.0 {
        pace
    } else {
        360.0 + rand::random::<f32>() * 120.0
    };
    let dur = (dist as f32 / 1000.0 * pace_s) as i64;
    let start_ms = if !time_spec.is_empty() {
        // --time "HH:MM" 配合 --days-ago（0-3）
        let (h, m) = time_spec.split_once(':').unwrap_or((time_spec, "0"));
        let (h, m) = (
            h.parse::<u32>().unwrap_or(7) % 24,
            m.parse::<u32>().unwrap_or(0),
        );
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
    let params = crate::api::flow::RunParams {
        dist,
        dur,
        start_ms,
        face_check: face as i64,
        manual_altitude,
        manual_altitude_range,
        seed,
    };
    match crate::api::flow::run_full_flow(&mut client, &params, &mut log) {
        Ok(out) => {
            println!(
                "跑步提交成功 rrid={} uuid={} obs_upload={}/2 obs_roundtrip={} detail_request={} detail_complete={} detail_checks_passed={}",
                out.result.rrid,
                out.result.uuid,
                out.obs_upload,
                out.obs_roundtrip,
                out.detail_request,
                out.detail_complete,
                out.detail_checks_passed,
            );
            0
        }
        Err(e) => {
            eprintln!("跑步提交失败: {e}");
            1
        }
    }
}

fn cmd_template(rest: &[&str]) -> i32 {
    let flags = parse_flags(rest);
    let Some(path) = get(&flags, "file") else {
        eprintln!("缺少 --file <GPX/JSON>");
        return 1;
    };
    match crate::template::load(path) {
        Ok(samples) => {
            let summary = crate::template::summarize(path, &samples);
            println!("本地模板分析（仅预览，不上传）");
            println!("文件：{}", summary.source);
            println!("采样点：{}", summary.samples);
            println!("海拔范围：{:.1}–{:.1} m", summary.min_m, summary.max_m);
            println!(
                "累计上升：{:.1} m；累计下降：{:.1} m",
                summary.gain_m, summary.loss_m
            );
            0
        }
        Err(e) => {
            eprintln!("模板读取失败：{e}");
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
            AiMode::Count {
                reps: (reps / 5) * 5,
            }
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

/// 记录列表原始探测：支持自定义 body，打印条数与日期范围。
fn cmd_records_raw(rest: &[&str]) -> i32 {
    let flags = parse_flags(rest);
    let body = get(&flags, "body").unwrap_or("{}").to_string();
    let mut client = match make_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    match client.call("POST", crate::api::records::RECORDS_PATH, &body, &[]) {
        Ok(biz) => {
            let data = crate::api::client::parse_data_field(&biz);
            let arr = data.as_array().cloned().unwrap_or_default();
            let n = arr.len();
            println!("条数: {n}");
            if let Some(out) = get(&flags, "out") {
                std::fs::write(out, serde_json::to_string_pretty(&arr).unwrap_or_default())
                    .map_err(|e| eprintln!("写入失败: {e}"))
                    .ok();
                println!("已保存 {out}");
            } else if n > 0 {
                println!(
                    "首条: {}",
                    &arr[0].to_string()[..arr[0].to_string().len().min(300)]
                );
                println!(
                    "末条: {}",
                    &arr[n - 1].to_string()[..arr[n - 1].to_string().len().min(300)]
                );
            } else {
                println!(
                    "data: {}",
                    data.to_string()[..data.to_string().len().min(500)].to_string()
                );
            }
            0
        }
        Err(e) => {
            eprintln!("拉取失败: {e}");
            1
        }
    }
}

/// 拉取 OBS 对象（签名 GET），解码 gzip+base64 字段后保存。
fn cmd_obs_get(rest: &[&str]) -> i32 {
    let flags = parse_flags(rest);
    let Some(key) = get(&flags, "key") else {
        eprintln!("缺少 --key");
        return 1;
    };
    let mut client = match make_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let mut log = |s: &str| eprintln!("{s}");
    match crate::api::obs::fetch_object(&mut client, key, &mut log) {
        Ok(v) => {
            let out = get(&flags, "out").unwrap_or("obs_real.json");
            // 解码 gzip+base64 值便于直接阅读
            let decoded = decode_gz_fields(&v);
            std::fs::write(
                out,
                serde_json::to_string_pretty(&decoded).unwrap_or_default(),
            )
            .map_err(|e| eprintln!("写入失败: {e}"))
            .ok();
            println!("已保存 {out}");
            0
        }
        Err(e) => {
            eprintln!("拉取失败: {e}");
            1
        }
    }
}

/// 解码 OBS 对象中 gzip+base64 的字段值（尽力而为）。
fn decode_gz_fields(v: &serde_json::Value) -> serde_json::Value {
    use base64::Engine;
    let mut out = v.clone();
    if let Some(obj) = out.as_object_mut() {
        for (_k, val) in obj.iter_mut() {
            if let Some(s) = val.as_str() {
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(s)
                    .ok()
                    .and_then(|raw| {
                        let mut dec = flate2::read::GzDecoder::new(raw.as_slice());
                        use std::io::Read;
                        let mut txt = String::new();
                        dec.read_to_string(&mut txt).ok()?;
                        Some(txt)
                    });
                if let Some(txt) = decoded {
                    let parsed = serde_json::from_str::<serde_json::Value>(&txt)
                        .map(|x| x.to_string())
                        .unwrap_or(txt);
                    *val = serde_json::Value::String(parsed);
                }
            }
        }
    }
    out
}

/// 本地生成一份 OBS 对象样本（不提交；结构对照用）。
fn cmd_obs_sample(rest: &[&str]) -> i32 {
    let flags = parse_flags(rest);
    let dist = get(&flags, "dist")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(1050.0);
    let dur = get(&flags, "dur")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(480);
    let rrid = get(&flags, "rrid")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(1320000000);
    let seed = get(&flags, "seed")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(7);
    let mut client = match make_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let mut log = |s: &str| eprintln!("{s}");
    let anchor = match client.identity.anchor_coordinate() {
        Ok(value) => value,
        Err(e) => {
            eprintln!("定位锚点无效: {e}");
            return 1;
        }
    };
    let pts = match crate::api::points::fetch_points(&mut client, anchor, &mut log) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("点位拉取失败: {e}");
            return 1;
        }
    };
    let pts_bd = crate::api::points::points_bd(&pts);
    let start_ms = crate::crypto::envelope::now_ms() - dur * 1000;
    let track = crate::track::generator::build(
        dist,
        dur,
        seed,
        (anchor.latitude, anchor.longitude),
        start_ms,
        &pts_bd,
    );
    let sess = client.login.clone().unwrap_or_default();
    let uuid = uuid::Uuid::new_v4().to_string().to_uppercase();
    let obj = crate::track::wire::build_obs_object(&track, rrid, &uuid, sess.uid, &pts);
    let out = get(&flags, "out").unwrap_or("obs_ours.json");
    let decoded = decode_gz_fields(&obj);
    std::fs::write(
        out,
        serde_json::to_string_pretty(&decoded).unwrap_or_default(),
    )
    .map_err(|e| eprintln!("写入失败: {e}"))
    .ok();
    println!("已保存 {out}");
    0
}

/// 单条跑步记录原始详情。
fn cmd_record_info(rest: &[&str]) -> i32 {
    let flags = parse_flags(rest);
    let Some(rrid) = get(&flags, "rrid").and_then(|v| v.parse::<i64>().ok()) else {
        eprintln!("缺少 --rrid");
        return 1;
    };
    let mut client = match make_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    match crate::api::records::fetch_one_record(&mut client, rrid) {
        Ok(v) => println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default()),
        Err(e) => {
            eprintln!("拉取失败: {e}");
            return 1;
        }
    }
    0
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
    let page: i64 = get(&flags, "page")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
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
            let range: i64 = get(&flags, "range")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1);
            crate::api::rank::indoor_rank(&mut client, range, -1)
        }
        "history" => {
            let sort: i64 = get(&flags, "sort")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1);
            crate::api::rank::history_rank(&mut client, sort, -1)
        }
        _ => {
            let rtype: i64 = get(&flags, "type")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1);
            let sort: i64 = get(&flags, "sort")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1);
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

fn cmd_update(rest: &[&str]) -> i32 {
    let flags = parse_flags(rest);
    let check_only = get(&flags, "check").is_some();
    crate::update::cleanup_residue();
    let mut log = logger();
    let release = match crate::update::check_latest(&mut log) {
        Ok(r) => r,
        Err(e) => {
            println!("× 检查更新失败: {e}");
            return 1;
        }
    };
    let Some(rel) = release else {
        println!("√ 已是最新版本（v{}）", crate::update::current_version());
        return 0;
    };
    println!(
        "发现新版本 {}（当前 v{}，{:.1} MB）",
        rel.tag,
        crate::update::current_version(),
        rel.asset_size as f64 / 1024.0 / 1024.0
    );
    if !rel.notes.is_empty() {
        for line in rel.notes.lines().take(8) {
            println!("  {line}");
        }
    }
    if check_only {
        println!("仅检查（--check），未下载。下载地址：{}", rel.asset_url);
        return 0;
    }
    #[cfg(target_os = "android")]
    {
        println!("Android 版请在界面（关于页）中下载并安装更新");
        0
    }
    #[cfg(not(target_os = "android"))]
    {
        println!("开始下载 {}…", rel.asset_name);
        let mut last_pct = u64::MAX;
        let bytes = match crate::update::download(&rel.asset_url, |done, total| {
            if let Some(t) = total.filter(|t| *t > 0) {
                let pct = done * 100 / t;
                if pct != last_pct && pct % 5 == 0 {
                    println!("  {pct}%");
                    last_pct = pct;
                }
            }
        }) {
            Ok(b) => b,
            Err(e) => {
                println!("× 下载失败: {e}");
                return 1;
            }
        };
        println!("下载完成（{} 字节），解包替换…", bytes.len());
        let bin = match crate::update::extract(&rel.asset_name, &bytes) {
            Ok(b) => b,
            Err(e) => {
                println!("× {e}");
                return 1;
            }
        };
        match crate::update::apply(&bin) {
            Ok(()) => {
                println!("√ 已更新到 {}，请重新运行命令", rel.tag);
                0
            }
            Err(e) => {
                println!("× 替换失败: {e}");
                1
            }
        }
    }
}
