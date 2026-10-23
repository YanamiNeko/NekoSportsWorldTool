//! Exercise preparation against loopback only; any unexpected point/submit request fails.
use super::*;
use serde_json::json;
use std::io::{Read, Write};
use std::time::{Duration, Instant};

fn mock_api(replies: Vec<(&'static str, Value)>) -> (String, std::thread::JoinHandle<Vec<String>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let host = format!("http://{}", listener.local_addr().unwrap());
    let task = std::thread::spawn(move || {
        let mut paths = Vec::new();
        for (expected, response) in replies {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "missing {expected}");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut bytes = Vec::new();
            let end = loop {
                let mut buffer = [0; 4096];
                let n = stream.read(&mut buffer).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
                assert!(bytes.len() < 128 * 1024);
                if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") { break end + 4; }
            };
            let headers = String::from_utf8_lossy(&bytes[..end]).into_owned();
            let path = headers.lines().next().unwrap().split_whitespace().nth(1).unwrap();
            assert_eq!(path, expected);
            let length: usize = headers.lines().find_map(|line| {
                let (key, value) = line.split_once(':')?;
                key.eq_ignore_ascii_case("content-length").then(|| value.trim().parse().unwrap())
            }).unwrap();
            while bytes.len() < end + length {
                let mut buffer = [0; 4096];
                let n = stream.read(&mut buffer).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
            }
            assert!(serde_json::from_slice::<Value>(&bytes[end..end + length]).unwrap().is_object());
            paths.push(path.to_owned());
            let body = response.to_string();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        }
        paths
    });
    (host, task)
}

fn client(host: String) -> ApiClient {
    let mut client = ApiClient::new(crate::crypto::header::HeaderIdentity {
        city: "测试城市".into(), anchor_lat: 39.9, anchor_lon: 116.4,
        ..Default::default()
    }, Some(Session { uid: 42, token: "loopback-only".into(), unid: "77".into(), ..Default::default() }));
    client.set_test_host(host);
    client
}

fn params(mode: RouteMode) -> RunParams {
    RunParams { dist: 1200.0, dur: 600, start_ms: 1_700_000_000_000,
        face_check: 0, manual_altitude: None, manual_altitude_range: None, seed: 42, route_mode: mode, free_use_roads:false }
}

fn policy(enabled: bool) -> Value {
    json!({"error":10000,"data":{"timestamp":123,"policy":0,"runRuleModel":{
        "minDistance":100,"freedomRunCalculateSingle":enabled,"freedomElectronicFence":0
    }}})
}

#[test]
fn free_run_prepares_without_points_or_submission_requests() {
    let (host, server) = mock_api(vec![
        (crate::api::policy::POLICY_PATH, policy(true)),
        (crate::api::fence::FENCE_PATH, json!({"error":10000,"data":{"geoFences":[]}})),
    ]);
    let control = crate::task_control::Control::default();
    let prepared = prepare_run(&mut client(host), &params(RouteMode::Free), &mut |_| {}, &control).unwrap();
    assert!(control.cancellable(), "preview must not enter the submission state");
    assert!(prepared.free_run);
    prepared.track.validate_consistency().unwrap();
    let wrapper: Value = serde_json::from_str(&prepared.five_point_json).unwrap();
    assert_eq!(wrapper["fivePointJson"], "[]");
    assert_eq!(wrapper["geoFencesJson"], "[]");
    assert_eq!(prepared.track.totalDistance, 1200.0);
    assert!(prepared.track.locations.iter().all(|p| (p.gLat - 39.9).abs() < 0.002 && (p.gLng - 116.4).abs() < 0.002));
    assert_eq!(server.join().unwrap(), [crate::api::policy::POLICY_PATH, crate::api::fence::FENCE_PATH]);
}

#[test]
fn point_modes_still_propagate_the_real_10600_error() {
    for mode in [RouteMode::Legacy, RouteMode::Road] {
        let (host, server) = mock_api(vec![
            (crate::api::policy::POLICY_PATH, policy(true)),
            (crate::api::fence::FENCE_PATH, json!({"error":10000,"data":{"geoFences":[]}})),
            (crate::api::points::POINTS_PATH, json!({"error":10600,"message":"当前学校尚未设置点位，请联系学校老师"})),
        ]);
        let result = prepare_run(&mut client(host), &params(mode), &mut |_| {}, &Default::default());
        let error = match result { Ok(_) => panic!("point mode unexpectedly accepted empty points"), Err(error) => error };
        assert!(error.contains("10600"));
        assert_eq!(server.join().unwrap().len(), 3);
    }
}

#[test]
fn free_run_does_not_ignore_a_disabled_server_policy() {
    let (host, server) = mock_api(vec![
        (crate::api::policy::POLICY_PATH, policy(false)),
        (crate::api::fence::FENCE_PATH, json!({"error":10000,"data":{"geoFences":[]}})),
    ]);
    let error = match prepare_run(&mut client(host), &params(RouteMode::Free), &mut |_| {}, &Default::default()) {
        Ok(_) => panic!("disabled free run was accepted"), Err(error) => error,
    };
    assert!(error.contains("未明确开启自由跑"));
    assert_eq!(server.join().unwrap().len(), 2);
}

#[test]
fn free_run_enforces_required_points_minimum_distance_and_fences() {
    let mut response = policy(true);
    let mut rule = crate::api::policy::parse_response(&response).unwrap();
    assert!(rule.validate_free_run(1200.0, false).is_ok());
    assert!(rule.validate_free_run(50.0, false).is_err());
    rule.must_points.push((39.9, 116.4));
    assert!(rule.validate_free_run(1200.0, false).is_err());
    response["data"]["runRuleModel"]["freedomElectronicFence"] = json!(1);
    let rule = crate::api::policy::parse_response(&response).unwrap();
    assert!(rule.validate_free_run(1200.0, false).is_err());
    assert!(rule.validate_free_run(1200.0, true).is_ok());
    let track = crate::track::generator::build_free(1200.0, 600, 42, (39.9, 116.4), 1_700_000_000_000).unwrap();
    let enclosing = vec![vec![(39.898,116.398),(39.898,116.402),(39.902,116.402),(39.902,116.398)]];
    assert!(validate_free_run_fences(&track, &enclosing).is_ok());
    let too_small = vec![vec![(39.89999,116.39999),(39.89999,116.40001),(39.90001,116.40001),(39.90001,116.39999)]];
    assert!(validate_free_run_fences(&track, &too_small).is_err());
}

#[test]
fn free_run_uses_real_osm_roads_and_buildings_without_requesting_checkpoints() {
    let root=std::env::temp_dir().join(format!("free-map-{}",uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut()=Some(root.clone()));
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut()=None);
            let _=std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup=Cleanup(root.clone());
    let map=root.join("nearby.osm");
    std::fs::write(&map,r#"<osm version="0.6">
      <node id="1" lat="39.9" lon="116.4"/><node id="2" lat="39.9" lon="116.402"/>
      <node id="3" lat="39.902" lon="116.402"/><node id="4" lat="39.902" lon="116.4"/>
      <node id="5" lat="39.9001" lon="116.4001"/><node id="6" lat="39.9001" lon="116.4002"/>
      <node id="7" lat="39.9002" lon="116.4002"/><node id="8" lat="39.9002" lon="116.4001"/>
      <way id="10"><nd ref="1"/><nd ref="2"/><nd ref="3"/><nd ref="4"/><nd ref="1"/><tag k="highway" v="footway"/></way>
      <way id="11"><nd ref="5"/><nd ref="6"/><nd ref="7"/><nd ref="8"/><nd ref="5"/><tag k="building" v="yes"/></way>
    </osm>"#).unwrap();
    super::super::model::save_config(&super::super::model::Config { osm_path:map.to_string_lossy().into_owned(),free_use_roads:true,..Default::default() }).unwrap();
    let (host,server)=mock_api(vec![
        (crate::api::policy::POLICY_PATH,policy(true)),
        (crate::api::fence::FENCE_PATH,json!({"error":10000,"data":{"geoFences":[]}})),
    ]);
    let mut client=client(host);
    let center=crate::track::geom::wgs84_to_bd09(39.9,116.4);
    client.identity.anchor_lat=center.0;
    client.identity.anchor_lon=center.1;
    let mut params=params(RouteMode::Free);
    params.free_use_roads=true;
    let mut messages=Vec::new();
    let prepared=prepare_run(&mut client,&params,&mut |line| messages.push(line.to_owned()),&Default::default()).unwrap();
    assert!(prepared.free_run);
    assert!(messages.iter().any(|line| line.contains("1 个建筑轮廓")));
    assert_eq!(serde_json::from_str::<Value>(&prepared.five_point_json).unwrap()["fivePointJson"],"[]");
    prepared.track.validate_consistency().unwrap();
    let latitudes:Vec<_>=prepared.track.locations.iter().map(|p| p.gLat).collect();
    let longitudes:Vec<_>=prepared.track.locations.iter().map(|p| p.gLng).collect();
    assert!(latitudes.iter().copied().fold(f64::NEG_INFINITY,f64::max)-latitudes.iter().copied().fold(f64::INFINITY,f64::min)>0.001);
    assert!(longitudes.iter().copied().fold(f64::NEG_INFINITY,f64::max)-longitudes.iter().copied().fold(f64::INFINITY,f64::min)>0.001);
    assert_eq!(server.join().unwrap().len(),2);
}
