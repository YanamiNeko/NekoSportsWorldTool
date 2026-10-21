//! Actual encrypted POSTs are sent only to a loopback server.
use super::*;
use serde_json::json;
use std::io::{Read, Write};
use std::time::{Duration, Instant};

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("submit-http-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&p).unwrap();
        crate::platform::TEST_DATA_DIR.with(|v| *v.borrow_mut() = Some(p.clone()));
        Self(p)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        crate::platform::TEST_DATA_DIR.with(|v| *v.borrow_mut() = None);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

enum Reply {
    Lost,
    Record(Value),
    Unauthorized,
    List(bool),
}
fn server(replies: Vec<Reply>) -> (String, std::thread::JoinHandle<Vec<String>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let host = format!("http://{}", listener.local_addr().unwrap());
    let task = std::thread::spawn(move || {
        let mut paths = Vec::new();
        let mut request_uuid = String::new();
        for reply in replies {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut stream = loop {
                match listener.accept() {
                    Ok((s, _)) => break s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "expected request did not arrive");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => panic!("{e}"),
                }
            };
            // Windows accepted sockets can inherit the listener's nonblocking mode.
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let end = loop {
                let mut b = [0; 4096];
                let n = stream.read(&mut b).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&b[..n]);
                assert!(bytes.len() < 2 * 1024 * 1024);
                if let Some(i) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    break i + 4;
                }
            };
            let headers = String::from_utf8_lossy(&bytes[..end]).into_owned();
            let path = headers
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap()
                .to_string();
            let length: usize = headers
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().unwrap())
                })
                .unwrap();
            while bytes.len() < end + length {
                let mut b = [0; 4096];
                let n = stream.read(&mut b).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&b[..n]);
            }
            assert!(serde_json::from_slice::<Value>(&bytes[end..end + length])
                .unwrap()
                .is_object());
            let (status, body) = match reply {
                Reply::List(found) => {
                    assert_eq!(path, crate::api::records::RECORDS_PATH);
                    (
                        200,
                        json!({"error":10000,"data":{"list":if found {
                        vec![json!({"rrid":"77","uuid":request_uuid})]
                    } else { vec![] }}}),
                    )
                }
                other => {
                    assert_eq!(path, RECORD_PATH);
                    request_uuid = headers
                        .lines()
                        .find_map(|line| {
                            let (key, value) = line.split_once(':')?;
                            key.eq_ignore_ascii_case("runef")
                                .then(|| value.trim()[..36].to_string())
                        })
                        .unwrap();
                    paths.push(path);
                    if matches!(other, Reply::Lost) {
                        continue;
                    }
                    let answer = match other {
                        Reply::Record(v) => (200, v),
                        Reply::Unauthorized => (401, json!({"message":"fixture token rejected"})),
                        _ => unreachable!(),
                    };
                    let text = answer.1.to_string();
                    write!(stream, "HTTP/1.1 {} fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", answer.0, text.len(), text).unwrap();
                    continue;
                }
            };
            paths.push(path);
            let text = body.to_string();
            write!(stream, "HTTP/1.1 {status} fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", text.len(), text).unwrap();
        }
        paths
    });
    (host, task)
}
fn client(host: &str) -> ApiClient {
    let mut c = ApiClient::new(
        crate::crypto::header::HeaderIdentity::default(),
        Some(crate::api::model::Session {
            uid: 42,
            unid: "77".into(),
            token: "offline-only".into(),
            ..Default::default()
        }),
    );
    c.set_test_host(host.into());
    c.agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(2))
        .build();
    c
}
fn params() -> SubmitParams {
    let track = crate::track::generator::build(
        1000.,
        400,
        42,
        (39.9, 116.4),
        1_700_000_000_000,
        &[(39.9, 116.4), (39.901, 116.401), (39.902, 116.4)],
    );
    track.validate_consistency().unwrap();
    SubmitParams {
        five_point_json: json!({"fivePointJson":json!([{"lat":39.9,"lon":116.4,"glat":39.9,"glon":116.4}]).to_string()}).to_string(),
        track, uid:42, selected_unid:77, policy:1, policy_ts:123, min_distance:1000,
        weight:68., face_check:0, address:"offline fixture".into(),
    }
}
fn error(result: Result<SubmitResult, String>) -> String {
    match result {
        Ok(_) => panic!("unexpected success"),
        Err(e) => e,
    }
}
#[test]
fn successful_post_then_same_plan_only_sends_once() {
    let _fixture = Fixture::new();
    let (host, task) = server(vec![Reply::Record(
        json!({"error":10000,"data":{"rrid":"42"}}),
    )]);
    let p = params();
    let result = submit_record(&mut client(&host), &p, &mut |_| {}).unwrap();
    assert_eq!(result.rrid, 42);
    assert!(error(submit_record(&mut client(&host), &p, &mut |_| {})).contains("重复提交"));
    assert_eq!(task.join().unwrap(), vec![RECORD_PATH]);
}
#[test]
fn lost_response_reconciles_uuid_without_reposting() {
    let _fixture = Fixture::new();
    let (host, task) = server(vec![Reply::Lost, Reply::List(true)]);
    let p = params();
    assert!(
        error(submit_record(&mut client(&host), &p, &mut |_| {})).contains("已找到本次记录 #77")
    );
    assert!(error(submit_record(&mut client(&host), &p, &mut |_| {})).contains("重复提交"));
    assert_eq!(
        task.join().unwrap(),
        vec![RECORD_PATH, crate::api::records::RECORDS_PATH]
    );
}
#[test]
fn missing_list_after_timeout_and_restart_never_reposts() {
    let _fixture = Fixture::new();
    let (host, task) = server(vec![Reply::Lost, Reply::List(false), Reply::List(false)]);
    let p = params();
    assert!(error(submit_record(&mut client(&host), &p, &mut |_| {})).contains("提交结果未知"));
    assert!(
        error(submit_record(&mut client(&host), &p, &mut |_| {})).contains("上次提交结果仍未知")
    );
    assert_eq!(
        task.join()
            .unwrap()
            .iter()
            .filter(|p| *p == RECORD_PATH)
            .count(),
        1
    );
}
#[test]
fn explicit_rejection_can_be_retried_by_user_but_not_automatically() {
    let _fixture = Fixture::new();
    let (host, task) = server(vec![
        Reply::Record(json!({"error":10603,"message":"fixture rejection"})),
        Reply::Record(json!({"error":10000,"data":{"rrid":43}})),
    ]);
    let p = params();
    assert!(error(submit_record(&mut client(&host), &p, &mut |_| {})).contains("明确拒绝"));
    assert_eq!(
        submit_record(&mut client(&host), &p, &mut |_| {})
            .unwrap()
            .rrid,
        43
    );
    assert_eq!(task.join().unwrap().len(), 2);
}
#[test]
fn malformed_success_stays_pending() {
    for response in [
        json!({}),
        json!({"error":10000,"data":{}}),
        json!({"error":10000,"data":"{broken"}),
    ] {
        let _fixture = Fixture::new();
        let (host, task) = server(vec![Reply::Record(response), Reply::List(false)]);
        assert!(
            error(submit_record(&mut client(&host), &params(), &mut |_| {}))
                .contains("提交结果未知")
        );
        assert_eq!(task.join().unwrap().len(), 2);
    }
}
#[test]
fn unauthorized_post_invalidates_session_and_does_not_send_again() {
    let _fixture = Fixture::new();
    let (host, task) = server(vec![Reply::Unauthorized]);
    let mut c = client(&host);
    let p = params();
    assert!(error(submit_record(&mut c, &p, &mut |_| {})).contains("明确拒绝"));
    assert!(crate::api::client::is_session_expired(&error(
        submit_record(&mut c, &p, &mut |_| {})
    )));
    assert_eq!(task.join().unwrap(), vec![RECORD_PATH]);
}
