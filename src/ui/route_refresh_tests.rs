//! Exercise the real refresh worker and preview UI against loopback HTTP responses.
//! No NetworkLoad or PreviewInputs is fabricated by these tests.
use super::*;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::time::{Duration, Instant};

struct FixtureDir(std::path::PathBuf);
impl FixtureDir {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("route-refresh-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = Some(root.clone()));
        Self(root)
    }
}
impl Drop for FixtureDir {
    fn drop(&mut self) {
        crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = None);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(super) fn mock_school(responses: Vec<(&'static str, Value)>) -> (String, std::thread::JoinHandle<usize>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let host = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let mut count = 0;
        for (expected_path, response) in responses {
            let deadline = Instant::now() + Duration::from_secs(10);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            Instant::now() < deadline,
                            "request missing: {expected_path}"
                        );
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => panic!("fixture accept: {e}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let header_end = loop {
                let mut buf = [0_u8; 4096];
                let size = stream.read(&mut buf).unwrap();
                assert_ne!(size, 0, "incomplete request headers");
                bytes.extend_from_slice(&buf[..size]);
                assert!(bytes.len() < 128 * 1024, "unexpected oversized request");
                if let Some(index) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    break index + 4;
                }
            };
            let headers = String::from_utf8_lossy(&bytes[..header_end]).into_owned();
            // 以 `/` 结尾表示"前缀匹配"：点位接口末尾是本次目标距离，值不固定
            if expected_path.ends_with('/') {
                assert!(
                    headers.starts_with(&format!("POST {expected_path}")),
                    "expected prefix {expected_path}, got {}",
                    headers.lines().next().unwrap_or("")
                );
            } else {
                assert!(headers.starts_with(&format!("POST {expected_path} HTTP/1.1\r\n")));
            }
            assert!(headers.to_ascii_lowercase().contains("headersign:"));
            let length: usize = headers
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().unwrap())
                })
                .unwrap();
            while bytes.len() < header_end + length {
                let mut buf = [0_u8; 4096];
                let size = stream.read(&mut buf).unwrap();
                assert_ne!(size, 0, "incomplete request body");
                bytes.extend_from_slice(&buf[..size]);
            }
            // The real encrypted request was built, not a direct parser invocation.
            assert!(
                serde_json::from_slice::<Value>(&bytes[header_end..header_end + length])
                    .unwrap()
                    .is_object()
            );
            let body = response.to_string();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            count += 1;
        }
        count
    });
    (host, server)
}

fn fixture_app(root: &FixtureDir, ctx: &egui::Context, host: String) -> App {
    let path = root.0.join("school.osm");
    std::fs::write(&path, LOCAL_OSM).unwrap();
    let (lat, lon) = crate::track::geom::wgs84_to_bd09(39.9, 116.4);
    let identity = HeaderIdentity {
        city: "测试城市".into(),
        anchor_lat: lat,
        anchor_lon: lon,
        ..Default::default()
    };
    let session = Session {
        uid: 42,
        unid: "77".into(),
        token: "local-fixture-only".into(),
        ..Default::default()
    };
    let config = Config {
        osm_path: path.display().to_string(),
        route_mode: "road".into(),
        dist_min: 2.4,
        dist_max: 2.4,
        pace_min: 400.,
        pace_max: 400.,
        ..Default::default()
    };
    let mut app = App::from_state(ctx, identity, session, config);
    app.route_fixture = Some((host, root.0.clone()));
    app.run_page.ensure_plan();
    app.run_page.plan.as_mut().unwrap().seed = 42;
    app
}

fn wait_for_refresh(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.osm_page.busy {
        app.poll_messages();
        assert!(
            Instant::now() < deadline,
            "refresh timed out: {}",
            app.osm_page.status
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn wire_point(lat: f64, lon: f64) -> Value {
    let (lat, lon) = crate::track::geom::wgs84_to_bd09(lat, lon);
    json!({"lat":format!("{lat:.10}"), "lon":format!("{lon:.10}")})
}

#[test]
fn string_data_http_refresh_reaches_a_complete_campus_preview() {
    let root = FixtureDir::new();
    let first = wire_point(39.9, 116.4);
    let live = vec![first.clone(), wire_point(39.901, 116.401)];
    let fence = json!({"runAreaId":"12", "geoFences":[{
        "campusId":"east", "campusName":"测试东校区", "runAreaId":"73",
        "points":[wire_point(39.8998,116.3998),wire_point(39.8998,116.4012),
                  wire_point(39.9012,116.4012),wire_point(39.9012,116.3998)]
    }]});
    let policy = json!({"timestamp":123, "policy":1, "runRuleModel":{"minDistance":1000},
        "mustPoints":[first.clone(), first]});
    let points = json!({"pointsResModels":live});
    let (host, server) = mock_school(vec![
        (
            crate::api::fence::FENCE_PATH,
            json!({"error":10000,"data":fence.to_string()}),
        ),
        (
            crate::api::policy::POLICY_PATH,
            json!({"error":10000,"data":policy.to_string()}),
        ),
        (
            "/api/v560/get/1/distance/",
            json!({"error":10000,"data":points.to_string()}),
        ),
    ]);
    let ctx = egui::Context::default();
    let mut app = fixture_app(&root, &ctx, host);
    app.network = Some(std::sync::Arc::new(
        crate::track::generate_road::load_network_path(&app.config.osm_path).unwrap(),
    ));
    frame(&mut app, &ctx, 1100., 1000., vec![]);
    let wgs_center = app.run_page.map.center;
    assert!(app.run_page.base_map_request.is_some());
    let anchor = app.identity.anchor_coordinate().unwrap();
    // A previous run-area cache must not suppress the current campus request.
    crate::api::model::save_points_cache(
        anchor,
        &crate::api::model::PointsCacheScope {
            uid: 42,
            unid: "77".into(),
            run_area_id: Some("12".into()),
            distance_m: 2400,
        },
        &[json!({"lat":1.,"lon":1.})],
    )
    .unwrap();
    app.refresh_school_route(true);
    wait_for_refresh(&mut app);
    assert_eq!(server.join().unwrap(), 3);
    assert!(
        app.osm_page.status.contains("加载成功"),
        "{}",
        app.osm_page.status
    );
    assert!(
        !app.osm_page.status.contains("暂不可用"),
        "{}",
        app.osm_page.status
    );
    assert_eq!(app.network.as_ref().unwrap().graph.node_count(), 4);
    let inputs = app
        .run_page
        .preview_inputs
        .as_ref()
        .expect("real worker must provide PreviewInputs");
    assert_eq!(
        inputs.points.len(),
        2,
        "required and live duplicate points must merge"
    );
    assert_eq!(
        inputs.points[0],
        crate::api::fence::point_xy(&live[0]).unwrap()
    );
    assert_eq!(inputs.fences.len(), 1);
    assert_eq!(app.osm_page.campuses[0].name, "测试东校区");
    let cache: Value =
        serde_json::from_slice(&std::fs::read(root.0.join("points_cache.json")).unwrap()).unwrap();
    assert_eq!(
        cache["scope"]["run_area_id"], "73",
        "selected campus runAreaId must reach points worker"
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.run_page.preview.is_none() {
        app.poll_messages();
        frame(&mut app, &ctx, 1100., 800., vec![]);
        assert!(
            Instant::now() < deadline,
            "preview failed: {}",
            app.run_page.preview_status
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let check = app.run_page.check.as_ref().unwrap();
    assert!(check.issues.is_empty(), "{:?}", check.issues);
    assert_eq!(check.components, 1);
    assert!(check.minimum_m.unwrap() < 450.);
    let plan = app.run_page.plan.as_ref().unwrap();
    let preview = app.run_page.preview.as_ref().unwrap();
    assert!((plan.dist * 1000. - 2400.).abs() < 0.01);
    assert!((preview.laps.iter().sum::<f64>() - plan.dist * 1000.).abs() < 0.01);
    assert!(
        preview.laps.len() >= 5,
        "long distance must repeat within the campus"
    );
    let polygons: Vec<Vec<route_planner::Coord>> = preview
        .fences
        .iter()
        .map(|p| {
            p.iter()
                .map(|&(lat, lon)| route_planner::Coord::new(lon, lat))
                .collect()
        })
        .collect();
    let route: Vec<_> = preview
        .route
        .iter()
        .map(|&(lat, lon)| route_planner::Coord::new(lon, lat))
        .collect();
    assert!(route
        .iter()
        .all(|&p| route_planner::boundary::contains(&polygons, p)));
    assert!(route
        .windows(2)
        .all(|w| route_planner::boundary::segment_inside(&polygons, w[0], w[1])));
    assert!(route_planner::boundary::segment_inside(
        &polygons,
        *route.last().unwrap(),
        route[0]
    ));
    assert_eq!(app.run_page.preview_status, "路线检查和预览已完成");
    assert!(app.run_page.base_map_request.is_none());
    assert!(
        (app.run_page.map.center.1 - wgs_center.1).abs() > 0.005,
        "switching to BD09 route coordinates must refit the WGS84 viewport"
    );
    app.run_page.preview_inputs = None;
    frame(&mut app, &ctx, 1100., 1000., vec![]);
    assert!((app.run_page.map.center.0 - wgs_center.0).abs() < 1e-10);
    assert!((app.run_page.map.center.1 - wgs_center.1).abs() < 1e-10);
    assert!(app.run_page.preview.is_none());
}

#[test]
fn malformed_fence_http_refresh_keeps_local_map_without_fabricating_preview() {
    let root = FixtureDir::new();
    let (host, server) = mock_school(vec![(
        crate::api::fence::FENCE_PATH,
        json!({"error":10000,"data":"{not-valid-json"}),
    )]);
    let ctx = egui::Context::default();
    let mut app = fixture_app(&root, &ctx, host);
    app.refresh_school_route(true);
    wait_for_refresh(&mut app);
    assert_eq!(server.join().unwrap(), 1);
    assert_eq!(app.network.as_ref().unwrap().graph.node_count(), 4);
    assert!(app.run_page.preview_inputs.is_none());
    assert!(app.run_page.preview.is_none());
    assert!(
        app.osm_page.status.contains("data"),
        "{}",
        app.osm_page.status
    );
    assert!(
        !app.osm_page.status.contains("没有有效校区围栏"),
        "{}",
        app.osm_page.status
    );
    assert!(!root.0.join("points_cache.json").exists());
}

#[test]
fn explicit_null_boundary_refreshes_points_and_repeats_one_road_loop() {
    let root = FixtureDir::new();
    let live = vec![
        wire_point(39.9, 116.4),
        wire_point(39.9, 116.4005),
        wire_point(39.9, 116.401),
        wire_point(39.901, 116.401),
        wire_point(39.901, 116.4),
    ];
    let (host, server) = mock_school(vec![
        (
            crate::api::fence::FENCE_PATH,
            json!({"error":10000,"data":{"geoFences":null}}),
        ),
        (
            crate::api::policy::POLICY_PATH,
            json!({"error":10000,"data":{"timestamp":123,"policy":1,"mustPoints":[]}}),
        ),
        (
            "/api/v560/get/1/distance/",
            json!({"error":10000,"data":{"pointsResModels":live}}),
        ),
    ]);
    let ctx = egui::Context::default();
    let mut app = fixture_app(&root, &ctx, host);
    app.refresh_school_route(false);
    wait_for_refresh(&mut app);
    assert_eq!(server.join().unwrap(), 3);
    let inputs = app
        .run_page
        .preview_inputs
        .as_ref()
        .expect("empty boundary must not block live points");
    assert_eq!(inputs.points.len(), 5);
    assert!(inputs.fences.is_empty());
    assert!(app.osm_page.campuses.is_empty());
    let cache: Value =
        serde_json::from_slice(&std::fs::read(root.0.join("points_cache.json")).unwrap()).unwrap();
    assert!(
        cache["scope"]["run_area_id"].is_null(),
        "never invent an area ID"
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.run_page.preview.is_none() {
        frame(&mut app, &ctx, 1100., 1200., vec![]);
        assert!(Instant::now() < deadline, "{}", app.run_page.preview_status);
        std::thread::sleep(Duration::from_millis(5));
    }
    let preview = app.run_page.preview.as_ref().unwrap();
    assert!(preview.length_m > 100. && preview.length_m < 500.);
    assert!((preview.laps.iter().sum::<f64>() - 2400.).abs() < 0.01);
    assert!(preview.laps.len() >= 5);
    assert_eq!(preview.checkpoints.len(), 5);
    assert!(preview.fences.is_empty());
    assert!(app.run_page.preview_status.contains("未提供学校边界"));
    assert!(!app.run_busy, "preview itself must never start submission");
    assert!(app.rx.try_recv().is_err());
}

#[test]
fn empty_boundary_fetches_policy_before_any_point_scoped_download() {
    let root = FixtureDir::new();
    let (host, server) = mock_school(vec![
        (
            crate::api::fence::FENCE_PATH,
            json!({"error":10000,"data":{"geoFences":[]}}),
        ),
        (
            crate::api::policy::POLICY_PATH,
            json!({"error":11016,"message":"非有效跑步时段"}),
        ),
    ]);
    let ctx = egui::Context::default();
    let mut app = fixture_app(&root, &ctx, host);
    app.config.osm_path.clear();
    app.refresh_school_route(false);
    wait_for_refresh(&mut app);
    assert_eq!(server.join().unwrap(), 2);
    assert!(
        app.osm_page.status.contains("11016"),
        "{}",
        app.osm_page.status
    );
    assert!(app.network.is_none());
    assert!(app.run_page.preview_inputs.is_none());
}

fn base_map_app(root: &FixtureDir, ctx: &egui::Context) -> App {
    let mut app = fixture_app(root, ctx, "http://127.0.0.1:1".into());
    let xml = LOCAL_OSM.replace(
        "</osm>",
        r#"
        <node id="11" lat="39.9003" lon="116.4003"/>
        <node id="12" lat="39.9003" lon="116.4007"/>
        <node id="13" lat="39.9007" lon="116.4007"/>
        <node id="14" lat="39.9007" lon="116.4003"/>
        <way id="20"><nd ref="11"/><nd ref="12"/><nd ref="13"/><nd ref="14"/><nd ref="11"/>
        <tag k="building" v="yes"/></way></osm>"#,
    );
    app.network = Some(std::sync::Arc::new(
        route_planner::load_osm(xml.as_bytes()).unwrap(),
    ));
    assert_eq!(app.network.as_ref().unwrap().buildings.len(), 1);
    app.osm_page.status = "路网加载成功：4 节点 / 1 建筑".into();
    app
}

fn visible_path(output: &egui::FullOutput, color: egui::Color32) -> bool {
    output.shapes.iter().any(|shape| match &shape.shape {
        egui::Shape::Path(path) => {
            path.stroke.color == egui::epaint::ColorMode::Solid(color)
                && path.points.iter().any(|p| shape.clip_rect.contains(*p))
        }
        _ => false,
    })
}

#[test]
fn run_base_map_draws_real_roads_and_buildings_without_route_metadata() {
    for (w, h) in [(320., 420.), (360., 480.), (412., 600.), (1100., 900.)] {
        for state in ["no-login", "policy-failure", "offline"] {
            let root = FixtureDir::new();
            let ctx = egui::Context::default();
            let mut app = base_map_app(&root, &ctx);
            match state {
                "no-login" => app.session = None,
                "policy-failure" => app.osm_page.status = "业务错误 11016：非允许跑步时段".into(),
                _ => {
                    app.session = None;
                    app.osm_page.offline = true;
                }
            }
            let request_id = app.osm_page.request_id;
            let at = scroll_to_label(&mut app, &ctx, w, h, "查看路网与校区");
            click(&mut app, &ctx, w, h, at);
            assert_eq!(
                app.tab, 7,
                "navigation must remain usable: {state}, {w}x{h}"
            );
            app.tab = 0;
            let mut roads = false;
            let mut buildings = false;
            for _ in 0..60 {
                let output = frame(&mut app, &ctx, w, h, vec![]);
                roads |= visible_path(&output, egui::Color32::from_rgb(190, 200, 196));
                buildings |= visible_path(&output, egui::Color32::from_rgb(214, 182, 158));
                if roads && buildings {
                    break;
                }
                frame(
                    &mut app,
                    &ctx,
                    w,
                    h,
                    vec![
                        Event::PointerMoved(Pos2::new(w / 2., h - 55.)),
                        Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: Vec2::new(0., -60.),
                            modifiers: Default::default(),
                        },
                    ],
                );
                for _ in 0..12 {
                    frame(&mut app, &ctx, w, h, vec![]);
                }
            }
            assert!(roads && buildings, "base geometry hidden: {state}, {w}x{h}");
            assert!(app.run_page.preview_inputs.is_none());
            assert!(app.run_page.preview.is_none());
            assert!(!app.run_page.preview_busy);
            assert!(!app.osm_page.busy);
            assert_eq!(
                app.osm_page.request_id, request_id,
                "drawing must not start requests"
            );
            assert!(app.rx.try_recv().is_err());
        }
    }
}

#[test]
fn run_base_map_can_pan_zoom_and_refit_a_reloaded_map() {
    let root = FixtureDir::new();
    let ctx = egui::Context::default();
    let mut app = base_map_app(&root, &ctx);
    app.session = None;
    let output = frame(&mut app, &ctx, 1100., 1200., vec![]);
    let canvas = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Rect(rect)
                if rect.fill == egui::Color32::from_rgb(250, 252, 251)
                    && (rect.rect.height() - 240.).abs() < 0.1
                    && shape.clip_rect.contains(rect.rect.center()) =>
            {
                Some(rect.rect)
            }
            _ => None,
        })
        .expect("visible base-map canvas");
    let center = canvas.center();
    frame(
        &mut app,
        &ctx,
        1100.,
        1200.,
        vec![
            Event::PointerMoved(center),
            Event::PointerButton {
                pos: center,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
        ],
    );
    let dragged = center + Vec2::new(50., 25.);
    frame(
        &mut app,
        &ctx,
        1100.,
        1200.,
        vec![Event::PointerMoved(dragged)],
    );
    frame(
        &mut app,
        &ctx,
        1100.,
        1200.,
        vec![Event::PointerButton {
            pos: dragged,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }],
    );
    assert!(
        app.run_page.map.offset.length() > 20.,
        "drag must move base map"
    );
    let old_scale = app.run_page.map.scale;
    frame(
        &mut app,
        &ctx,
        1100.,
        1200.,
        vec![
            Event::PointerMoved(center),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: Vec2::new(0., 60.),
                modifiers: Default::default(),
            },
        ],
    );
    assert!(
        app.run_page.map.scale > old_scale,
        "wheel must zoom base map"
    );
    app.osm_page.request_id += 1;
    frame(&mut app, &ctx, 1100., 1200., vec![]);
    assert_eq!(
        app.run_page.map.offset,
        Vec2::ZERO,
        "new map must refit instead of retaining old pan"
    );
    assert!(app.run_page.preview.is_none());
    assert!(app.run_page.preview_inputs.is_none());
}
