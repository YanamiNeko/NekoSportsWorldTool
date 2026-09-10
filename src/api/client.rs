//! HTTP 客户端封装。
//!
//! 通用请求封装：body = 信封(明文, insert)，headerSign = 信封(header, observed)，
//! 响应两层解密到业务 JSON；同一实例复用同一 keyDataFour。

use crate::api::model::Session;
use crate::crypto::decrypt::{decrypt_response, derive_paes_key, Decrypted};
use crate::crypto::envelope::{
    build_envelope, build_envelope_ts, rsa_public_key, EnvelopeSession, OuterOrder,
};
use crate::crypto::header::{build_header_for, HeaderIdentity, UA_IOS};

pub fn make_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(30))
        .build()
}

pub struct ApiClient {
    pub agent: ureq::Agent,
    pub session: EnvelopeSession,
    pub identity: HeaderIdentity,
    pub login: Option<Session>,
}

pub struct RequestOutcome {
    pub http_status: u16,
    pub decrypted: Option<Decrypted>,
    pub raw_len: usize,
    /// 解密失败时的服务端原始返回摘要（诊断用）。
    pub raw_head: String,
}

impl ApiClient {
    pub fn new(identity: HeaderIdentity, login: Option<Session>) -> Self {
        Self {
            agent: make_agent(),
            session: EnvelopeSession::new(),
            identity,
            login,
        }
    }

    fn uid(&self) -> i64 {
        self.login.as_ref().map(|s| s.uid).unwrap_or(-1)
    }

    fn token(&self) -> String {
        self.login.as_ref().map(|s| s.token.clone()).unwrap_or_default()
    }

    /// 发送信封请求（header/headerSign 用 observed 序，body 用 insert 序）。
    /// method 为 "POST" 或 "GET"（GET 同样携带信封 body）。
    pub fn envelope_request(
        &mut self,
        method: &str,
        url: &str,
        body_plain: &str,
        ua: &str,
        extra_headers: &[(String, String)],
    ) -> Result<RequestOutcome, String> {
        let uid = self.uid();
        let token = self.token();
        let (header_plain, _) = build_header_for(&self.identity, uid, &token, None);
        let header_env = build_envelope(&mut self.session, &header_plain, OuterOrder::Observed);
        // body 时间戳 = header ts + 1（原生两次独立读取毫秒）
        let body_env =
            build_envelope_ts(&mut self.session, body_plain, OuterOrder::Insert, header_env.ts_ms + 1);

        let mut req = self.agent.request(method, url);
        req = req.set("Content-Type", "application/json; charset=utf-8");
        req = req.set("User-Agent", ua);
        req = req.set("appVersion", "7.3.40");
        req = req.set("headerSign", &header_env.json);
        for (k, v) in extra_headers {
            req = req.set(k, v);
        }
        let resp = req.send_string(&body_env.json).map_err(ureq_err)?;
        let status = resp.status();
        let raw = resp.into_string().unwrap_or_default();
        let raw_len = raw.len();
        let key = derive_paes_key(
            &body_env.key_data[0],
            &body_env.key_data[1],
            &body_env.key_data[2],
            &body_env.key_data[3],
        );
        let (decrypted, raw_head) = match decrypt_response(raw.as_bytes(), &key, &rsa_public_key()) {
            Ok(d) => (Some(d), String::new()),
            Err(e) => (None, format!("{e} | 原文: {}", crate::textlog::truncate(&raw, 160))),
        };
        Ok(RequestOutcome { http_status: status, decrypted, raw_len, raw_head })
    }

    /// 标准业务请求（RUN 域名）：返回业务 JSON；error != 10000 时 Err。
    pub fn call(
        &mut self,
        method: &str,
        path: &str,
        body_plain: &str,
        extra_headers: &[(String, String)],
    ) -> Result<serde_json::Value, String> {
        self.call_host(crate::api::model::HOST, method, path, body_plain, extra_headers)
    }

    /// 指定域名业务请求（排行榜/违规名单走 DISCOVERY，信封链相同）。
    /// 每次调用打完整解密响应日志。
    pub fn call_host(
        &mut self,
        host: &str,
        method: &str,
        path: &str,
        body_plain: &str,
        extra_headers: &[(String, String)],
    ) -> Result<serde_json::Value, String> {
        let url = format!("{}{}", host, path);
        let out = self.envelope_request(method, &url, body_plain, UA_IOS, extra_headers)?;
        let dec = out.decrypted.ok_or_else(|| {
            format!(
                "HTTP {} 响应解密失败（len={}）原文: {}",
                out.http_status, out.raw_len, out.raw_head
            )
        })?;
        let business = match check_business(&dec.business) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[resp] {} -> Err: {}", path, e);
                return Err(e);
            }
        };
        let biz_str = serde_json::to_string(&business).unwrap_or_default();
        eprintln!("[resp] {} -> {} ({}B) {}", path, out.http_status, out.raw_len, biz_str);
        Ok(business)
    }
}

/// 业务结果检查：error == 10000 → Ok(business)；否则 Err(描述)。
pub fn check_business(v: &serde_json::Value) -> Result<serde_json::Value, String> {
    let err = v.get("error").and_then(|e| e.as_i64()).unwrap_or(0);
    if err == 10000 {
        Ok(v.clone())
    } else {
        let msg = v
            .get("message")
            .or_else(|| v.get("msg"))
            .and_then(|m| m.as_str())
            .unwrap_or("(无消息)");
        Err(format!("业务错误 {err}: {msg}"))
    }
}

/// 业务字段查找：先顶层，再 data 子对象。
pub fn get_field<'a>(biz: &'a serde_json::Value, name: &str) -> Option<&'a serde_json::Value> {
    if let Some(v) = biz.get(name) {
        if !v.is_null() {
            return Some(v);
        }
    }
    biz.get("data").and_then(|d| d.get(name))
}

/// data 字段如果是 JSON 字符串则解析（Obs 一层包裹语义）。
pub fn parse_data_field(biz: &serde_json::Value) -> serde_json::Value {
    match biz.get("data") {
        Some(serde_json::Value::String(s)) => serde_json::from_str(s).unwrap_or(serde_json::Value::Null),
        Some(v) => v.clone(),
        None => serde_json::Value::Null,
    }
}

pub fn ureq_err(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, resp) => {
            let body = resp.into_string().unwrap_or_default();
            format!("HTTP {code}: {}", &body[..body.len().min(300)])
        }
        ureq::Error::Transport(t) => format!("网络错误: {t}"),
    }
}
