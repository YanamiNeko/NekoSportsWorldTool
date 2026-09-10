//! 登录链。
//!
//! ① GT4 滑块验证→ 四凭证
//! ② POST /api/v70270/security/geevalidate（信封 body + headerSign）
//! ③ POST /api/v70100/login（Authorization: Basic + 信封 body）→ uid/token/unid

use super::client::{check_business, get_field, ApiClient};
use super::gt4::solve_gt4;
use super::model::{Session, HOST};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde_json::{json, Value};

pub const GEEVALIDATE_PATH: &str = "/api/v70270/security/geevalidate";
pub const GEECHECK_PATH: &str = "/api/v65/security/checkGeeUse";
pub const LOGIN_PATH: &str = "/api/v70100/login";
pub const LOGOUT_PATH: &str = "/api/v6/user/logout";

/// bizType=0 的 captchaId（captchaIDForBizType 静态提取值；GT4 会话直接使用，
/// 走 v31 geepreprocess 属 Android GT3 旧链，iOS 登录链不引用）。
pub const CAPTCHA_ID_BIZ0: &str = "8c065103d81f5fd3efec8ad3e3a84c30";

/// checkGeeUse 免验证码探测：data=true 时本次登录可跳过 GT4 与 geevalidate。
/// 未登录态调用（uid=-1/token=""）；请求异常一律按 false 走原流程。
fn gee_check(client: &mut ApiClient, username: &str, log: &mut dyn FnMut(&str)) -> bool {
    let body = json!({
        "username": username,
        "uuid": uuid::Uuid::new_v4().to_string(),
        "unid": 0,
        "type": 2,
    })
    .to_string();
    match client.call("POST", GEECHECK_PATH, &body, &[]) {
        Ok(biz) => {
            let skip = biz.get("data").and_then(|d| d.as_bool()).unwrap_or(false);
            log(&format!(
                "[gee] checkGeeUse data={}（{}）",
                skip,
                if skip { "跳过 GT4" } else { "走 GT4 流程" }
            ));
            skip
        }
        Err(e) => {
            log(&format!("[gee] checkGeeUse 失败（{e}），按需验证码处理"));
            false
        }
    }
}

/// 完整登录：checkGeeUse → [GT4 → geevalidate] → login。成功返回 Session。
pub fn login(
    client: &mut ApiClient,
    username: &str,
    password: &str,
    log: &mut dyn FnMut(&str),
) -> Result<Session, String> {
    let uuid_value = uuid::Uuid::new_v4().to_string();
    if gee_check(client, username, log) {
        log("[login] 免验证码路径：直接登录");
    } else {
        // GT4 → geevalidate；10003（验证失败）时重破滑块再验，最多 3 轮
        let captcha_id = CAPTCHA_ID_BIZ0;
        let mut validated = false;
        for attempt in 1..=3 {
            log("[login] 开始 GT4 滑块验证…");
            let creds = solve_gt4(captcha_id, client, log)?;
            log(&format!(
                "[login] GT4 通过 lot_number={}",
                creds.get("lotNumber").and_then(|v| v.as_str()).unwrap_or("")
            ));

            let body = json!({
                "lotNumber": creds["lotNumber"],
                "captchaOutput": creds["captchaOutput"],
                "passToken": creds["passToken"],
                "genTime": creds["genTime"],
                "isOffline": false,
                "osType": 1,
                "businessType": 0,
                "uuid": uuid_value,
                "username": username,
            })
            .to_string();
            let url = format!("{HOST}{GEEVALIDATE_PATH}");
            let out = client.envelope_request("POST", &url, &body, crate::crypto::header::UA_IOS, &[])?;
            let gv_err = out.decrypted.as_ref().and_then(|d| d.business.get("error").cloned());
            match gv_err.as_ref().and_then(|e| e.as_i64()) {
                Some(10000) => {
                    log("[geevalidate] 验证通过");
                    validated = true;
                    break;
                }
                Some(10003) => {
                    log(&format!("[geevalidate] 验证失败（{attempt}/3），重试"));
                    std::thread::sleep(std::time::Duration::from_secs(2));
                }
                other => {
                    log(&format!("[geevalidate] err={other:?}（继续尝试登录）"));
                    validated = true; // 非 10003 的失败不阻塞登录（服务端可能仍放行）
                    break;
                }
            }
        }
        if !validated {
            return Err("geevalidate 连续验证失败（10003），请重试".into());
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }

    // login（Authorization Basic + Android 形态信封 body）
    let identity = client.identity.clone();
    let device_id = if identity.device_id.is_empty() {
        uuid::Uuid::new_v4().to_string().to_uppercase()
    } else {
        identity.device_id.clone()
    };
    let login_body = json!({
        "device_model": identity.device_name,
        "os_version": identity.os_version,
        "mac_address": identity.mac_address,
        "imei": "",
        "loginType": 0,
        "username": username,
        "password": password,
        "uuid": uuid_value,
        "osType": "0",
    })
    .to_string();
    let credential = B64.encode(format!("{username}:{password}"));
    let extra = vec![("Authorization".to_string(), format!("Basic {credential}"))];
    let url = format!("{HOST}{LOGIN_PATH}");
    let out = client.envelope_request("POST", &url, &login_body, crate::crypto::header::UA_IOS, &extra)?;
    let dec = out
        .decrypted
        .ok_or_else(|| format!("HTTP {} 且响应解密失败", out.http_status))?;
    let biz = check_business(&dec.business)?;
    let uid = get_field(&biz, "uid").and_then(|v| v.as_i64()).unwrap_or(0);
    let token = get_field(&biz, "token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if uid < 1 || token.is_empty() {
        return Err(format!("登录响应缺 uid/token: {}", truncate(&biz.to_string(), 200)));
    }
    let sess = Session {
        uid,
        token,
        unid: {
            let v = get_field(&biz, "unid").cloned().unwrap_or(Value::String("0".into()));
            match &v {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                _ => "0".to_string(),
            }
        },
        name: get_field(&biz, "name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        weight: get_field(&biz, "weight").and_then(|v| v.as_f64()).unwrap_or(68.0),
        username: username.to_string(),
        device_id,
        profile: super::client::parse_data_field(&biz),
    };
    if let Err(e) = super::model::save_session(&sess) {
        eprintln!("[login] 会话保存失败: {e}");
    }
    eprintln!(
        "[session] uid={} token={} unid={} name={} weight={} device={}",
        sess.uid,
        &sess.token,
        sess.unid,
        sess.name,
        sess.weight,
        &sess.device_id
    );
    eprintln!(
        "[session] profile 字段数={}",
        sess.profile.as_object().map(|o| o.len()).unwrap_or(0)
    );
    Ok(sess)
}

/// 登出并清理 。
pub fn logout(client: &mut ApiClient, log: &mut dyn FnMut(&str)) {
    if let Some(sess) = client.login.clone() {
        let _ = client.call("POST", LOGOUT_PATH, "{}", &[]);
        log(&format!("[logout] 已请求登出 uid={}", sess.uid));
    }
    super::model::clear_session();
    log("[logout] 本地  已删除");
}

fn truncate(s: &str, n: usize) -> String {
    crate::textlog::truncate(s, n)
}
