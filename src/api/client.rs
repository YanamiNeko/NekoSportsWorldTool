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

#[derive(Clone)]
pub struct AuthContext {
    pub epoch: u64,
    pub rejected: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub notify: std::sync::mpsc::Sender<AuthFailure>,
}

pub struct AuthFailure {
    pub epoch: u64,
    pub uid: i64,
    pub token: String,
}

pub struct ApiClient {
    auth_context: Option<AuthContext>,
    auth_rejected: bool,
    #[cfg(test)]
    test_host: Option<String>,
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
            auth_context: None,
            auth_rejected: false,
            #[cfg(test)]
            test_host: None,
            agent: make_agent(),
            session: EnvelopeSession::new(),
            identity,
            login,
        }
    }

    pub fn with_auth_context(mut self, context: Option<AuthContext>) -> Self {
        self.auth_context = context;
        self
    }

    #[cfg(test)]
    pub(crate) fn set_test_host(&mut self, host: String) {
        assert!(host.starts_with("http://127.0.0.1:"));
        self.test_host = Some(host);
    }

    pub(crate) fn handle_request_error(&mut self, error: ureq::Error) -> String {
        let authenticated = self.login.as_ref().is_some_and(Session::is_logged_in);
        let error = request_error(error, authenticated);
        if is_session_expired(&error) {
            self.auth_rejected = true;
            if let (Some(context), Some(session)) = (&self.auth_context, &self.login) {
                if !context.rejected.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    let _ = context.notify.send(AuthFailure {
                        epoch: context.epoch, uid: session.uid, token: session.token.clone(),
                    });
                }
            }
        }
        error
    }

    pub(crate) fn ensure_session_valid(&self) -> Result<(), String> {
        if self.login.as_ref().is_some_and(Session::is_logged_in)
            && (self.auth_rejected || self.auth_context.as_ref().is_some_and(|c| c.rejected.load(std::sync::atomic::Ordering::SeqCst))) {
            return Err(SESSION_EXPIRED.into());
        }
        Ok(())
    }

    pub(crate) fn record_url(&self) -> String {
        #[cfg(test)]
        if let Some(host) = &self.test_host {
            return format!("{}{}", host.trim_end_matches('/'), super::submit::RECORD_PATH);
        }
        format!("{}{}", super::model::HOST, super::submit::RECORD_PATH)
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
        self.ensure_session_valid()?;
        let uid = self.uid();
        let token = self.token();
        let (header_plain, _) = build_header_for(&self.identity, uid, &token, None);
        let header_env = build_envelope(&mut self.session, &header_plain, OuterOrder::Observed);
        // body 时间戳 = header ts + 1（原生两次独立读取毫秒）
        let body_env =
            build_envelope_ts(&mut self.session, body_plain, OuterOrder::Insert, header_env.ts_ms + 1);

        #[cfg(test)]
        let test_url = self.test_host.as_ref().map(|host| {
            let authority_and_path = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
            let path = authority_and_path.find('/').map(|at| &authority_and_path[at..]).unwrap_or("/");
            format!("{}{path}", host.trim_end_matches('/'))
        });
        #[cfg(test)]
        let url = test_url.as_deref().unwrap_or(url);
        let mut req = self.agent.request(method, url);
        req = req.set("Content-Type", "application/json; charset=utf-8");
        req = req.set("User-Agent", ua);
        req = req.set("appVersion", "7.3.40");
        req = req.set("headerSign", &header_env.json);
        for (k, v) in extra_headers {
            req = req.set(k, v);
        }
        let resp = req.send_string(&body_env.json)
            .map_err(|e| self.handle_request_error(e))?;
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
    /// 每次调用打印敏感字段打码后的解密响应日志。
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
        let biz_str = serde_json::to_string(&crate::textlog::redact_json(&business)).unwrap_or_default();
        eprintln!("[resp] {} -> {} ({}B) {}", path, out.http_status, out.raw_len, biz_str);
        Ok(business)
    }
}

/// 业务结果检查：error == 10000 → Ok(business)；否则 Err(描述)。
pub fn check_business(v: &serde_json::Value) -> Result<serde_json::Value, String> {
    let err = v.get("error").and_then(|e| e.as_i64()).unwrap_or(0);
    if err == 10000 {
        normalize_business_data(v)
    } else {
        let msg = v
            .get("message")
            .or_else(|| v.get("msg"))
            .and_then(|m| m.as_str())
            .unwrap_or("(无消息)");
        Err(format!("业务错误 {err}: {msg}"))
    }
}

/// Some successful endpoints encode the business data object/array as JSON text.
/// Decode only that known wrapper, retaining ordinary text results unchanged.
pub(crate) fn normalize_business_data(v: &serde_json::Value) -> Result<serde_json::Value, String> {
    let mut normalized = v.clone();
    if let Some(serde_json::Value::String(text)) = v.get("data") {
        let text = text.trim();
        if text.starts_with('{') || text.starts_with('[') {
            if text.len() > 8 * 1024 * 1024 {
                return Err("业务 data JSON 超过 8 MiB".into());
            }
            normalized["data"] = serde_json::from_str(text)
                .map_err(|_| "业务 data JSON 格式无效".to_string())?;
        }
    }
    Ok(normalized)
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
            format!("HTTP {code}: {}", crate::textlog::truncate(&body, 300))
        }
        ureq::Error::Transport(t) => format!("网络错误: {t}"),
    }
}


pub const SESSION_EXPIRED: &str = "登录已失效（HTTP 401）：请重新登录获取有效凭证";

pub fn is_session_expired(error: &str) -> bool {
    error.contains(SESSION_EXPIRED)
}

// Only authenticated school API requests may invalidate a school session.
// Public map services and unauthenticated login failures use their own errors.
fn request_error(error: ureq::Error, authenticated: bool) -> String {
    if authenticated && matches!(&error, ureq::Error::Status(401, _)) {
        SESSION_EXPIRED.into()
    } else {
        ureq_err(error)
    }
}

#[cfg(test)]
mod business_data_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn successful_json_text_data_matches_object_and_array_responses() {
        for data in [json!({"geoFences":[],"runAreaId":42}), json!([{"id":1}])] {
            let object = json!({"error":10000,"data":data});
            let encoded = json!({"error":10000,"data":data.to_string()});
            assert_eq!(check_business(&encoded).unwrap(), check_business(&object).unwrap());
        }
        for data in ["操作成功", "https://example.invalid/file", "null", "42", ""] {
            let response = json!({"error":10000,"data":data});
            assert_eq!(check_business(&response).unwrap(), response);
        }
    }

    #[test]
    fn invalid_or_excessive_json_text_is_not_silently_empty() {
        for data in ["{broken-private-payload".to_string(), format!("[\"{}\"]", "x".repeat(8 * 1024 * 1024))] {
            let error = check_business(&json!({"error":10000,"data":data})).unwrap_err();
            assert!(error.contains("data JSON"));
            assert!(!error.contains("private-payload"));
        }
        assert!(check_business(&json!({"error":10603,"data":"{broken"})).unwrap_err().starts_with("业务错误 10603"));
    }
}

#[cfg(test)]
mod auth_tests {
    use super::*;
    fn response_error(code: u16, body: &str) -> ureq::Error {
        ureq::Error::Status(code, ureq::Response::new(code, "fixture", body).unwrap())
    }
    #[test]
    fn authenticated_401_requires_login_without_exposing_raw_response() {
        let error = request_error(response_error(401, r#"{"message":"token有误或缓存中token不存在","token":"secret-fixture"}"#), true);
        assert_eq!(error, SESSION_EXPIRED);
        assert!(is_session_expired(&error));
        assert!(!error.contains("secret-fixture"));
    }
    #[test]
    fn login_and_non_auth_http_failures_do_not_expire_a_session() {
        assert!(!is_session_expired(&request_error(response_error(401, "login rejected"), false)));
        for code in [403, 429, 500] {
            assert!(!is_session_expired(&request_error(response_error(code, "fixture"), true)));
        }
    }
    #[test]
    fn chinese_http_errors_are_truncated_on_character_boundaries() {
        let body = format!("a{}", "错误".repeat(100));
        let error = ureq_err(response_error(500, &body));
        assert!(error.starts_with("HTTP 500: a错误"));
    }
}


#[cfg(test)]
mod shared_auth_tests {
    use super::*;
    #[test]
    fn one_rejection_stops_other_school_reads_and_only_notifies_once() {
        let (tx, rx) = std::sync::mpsc::channel();
        let context = AuthContext { epoch: 7, rejected: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)), notify: tx };
        let session = Session { uid: 42, token: "test-token".into(), ..Default::default() };
        let mut first = ApiClient::new(HeaderIdentity::default(), Some(session.clone())).with_auth_context(Some(context.clone()));
        // Exercise the actual request/error path using an isolated loopback HTTP server.
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut stream = loop {
                if let Ok((stream, _)) = listener.accept() { break stream; }
                assert!(std::time::Instant::now() < deadline, "mock HTTP request did not arrive");
                std::thread::sleep(std::time::Duration::from_millis(5));
            };
            stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
            let mut request = Vec::new();
            let header_end = loop {
                let mut chunk = [0u8; 4096];
                let n = stream.read(&mut chunk).unwrap();
                assert!(n > 0);
                request.extend_from_slice(&chunk[..n]);
                if let Some(at) = request.windows(4).position(|w| w == b"\r\n\r\n") { break at + 4; }
            };
            let headers = String::from_utf8_lossy(&request[..header_end]).to_lowercase();
            assert!(headers.contains("headersign:"));
            let length: usize = headers.lines().find_map(|line| line.strip_prefix("content-length:")).unwrap().trim().parse().unwrap();
            while request.len() < header_end + length {
                let mut chunk = [0u8; 4096];
                let n = stream.read(&mut chunk).unwrap();
                assert!(n > 0); request.extend_from_slice(&chunk[..n]);
            }
            let body = r#"{"status":401,"message":"token有误或缓存中token不存在"}"#;
            write!(stream, "HTTP/1.1 401 Unauthorized\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        });
        assert_eq!(first.call_host(&host, "POST", crate::api::fence::FENCE_PATH, "{}", &[]).unwrap_err(), SESSION_EXPIRED);
        server.join().unwrap();
        let failure = rx.try_recv().unwrap();
        assert_eq!(failure.epoch, 7);
        assert_eq!(failure.uid, 42);
        for path in [crate::api::fence::FENCE_PATH, crate::api::cheat::CHEATLIST_PATH, "/api/v70230/runnings/records", crate::api::user::HOME_PAGE_INFO_PATH] {
            let mut client = ApiClient::new(HeaderIdentity::default(), Some(session.clone())).with_auth_context(Some(context.clone()));
            // Rejection must happen before contacting this unreachable loopback address.
            assert_eq!(client.call_host("http://127.0.0.1:1", "POST", path, "{}", &[]).unwrap_err(), SESSION_EXPIRED);
        }
        assert!(rx.try_recv().is_err());
    }
}
