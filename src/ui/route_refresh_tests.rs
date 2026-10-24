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
            assert!(headers.starts_with(&format!("POST {expected_path} HTTP/1.1\r\n")));
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
    check_string_data_refresh(false);
}

#[test]
fn string_point_list_http_refresh_reaches_a_complete_campus_preview() {
    check_string_data_refresh(true);
}

fn check_string_data_refresh(encoded_list: bool) {
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
    let points = json!({"pointsResModels": if encoded_list { json!(serde_json::to_string(&live).unwrap()) } else { json!(live) }});
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
            crate::api::points::POINTS_PATH,
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
            crate::api::points::POINTS_PATH,
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

fn campus_osm() -> String {
    LOCAL_OSM.replace("</osm>",r#"<node id="101" lat="39.8995" lon="116.3995"/><node id="102" lat="39.8995" lon="116.4015"/><node id="103" lat="39.9015" lon="116.4015"/><node id="104" lat="39.9015" lon="116.3995"/><way id="100"><nd ref="101"/><nd ref="102"/><nd ref="103"/><nd ref="104"/><nd ref="101"/><tag k="amenity" v="university"/><tag k="name" v="测试学校"/></way></osm>"#)
}
fn absent_replies(value:Value, must:Value) -> Vec<(&'static str,Value)> {
    vec![
        (crate::api::fence::FENCE_PATH,json!({"error":10000,"data":{"geoFences":[]}})),
        (crate::api::policy::POLICY_PATH,json!({"error":10000,"data":{"timestamp":123,"policy":1,"mustPoints":must}})),
        (crate::api::points::POINTS_PATH,value),
    ]
}
#[test]
fn no_points_http_refresh_plans_and_generates_with_and_without_map_boundary() {
    for (response,must,confirmed) in [
        (json!({"error":10600,"message":"当前学校尚未设置点位，请联系学校老师"}),json!([]),true),
        (json!({"error":10000,"pointsResModels":[]}),json!([]),false),
        (json!({"error":10600,"message":"当前学校尚未设置点位，请联系学校老师"}),json!([wire_point(39.901,116.401)]),true),
    ] {
        let root=FixtureDir::new(); let ctx=egui::Context::default();
        let (host,server)=mock_school(absent_replies(response,must.clone()));
        let mut app=fixture_app(&root,&ctx,host);
        std::fs::write(&app.config.osm_path,campus_osm()).unwrap();
        if confirmed {
            let c=route_planner::campus::parse(campus_osm().as_bytes()).unwrap().remove(0);
            app.config.map_boundary=Some(crate::api::map_boundary::Selection{school:"77".into(),id:c.id.clone(),revision:crate::api::map_boundary::revision(&c)});
        }
        app.refresh_school_route(false); wait_for_refresh(&mut app); assert_eq!(server.join().unwrap(),3);
        let inputs=app.run_page.preview_inputs.as_ref().expect(&app.osm_page.status);
        assert_eq!(inputs.fences.is_empty(),!confirmed);
        if must.as_array().unwrap().is_empty(){ assert_eq!(inputs.points,vec![(inputs.anchor.latitude,inputs.anchor.longitude)]); }
        else { assert_eq!(inputs.points,crate::api::points::points_bd(must.as_array().unwrap())); }
        let net=app.network.as_ref().unwrap();
        let preview=crate::track::generate_road::plan_road_view(net,&inputs.points,2400.,960,42,&inputs.fences).unwrap();
        assert!((preview.laps.iter().sum::<f64>()-2400.).abs()<0.01);
        let mut aligned=(**net).clone(); crate::track::generate_road::align_network(&mut aligned);
        let clipped=crate::track::generate_road::apply_fences(&aligned,&inputs.fences);
        let track=crate::track::generate_road::build_road(2400.,960,42,123000,&inputs.points,&[],&clipped).unwrap();
        track.validate_consistency().unwrap();
        if confirmed { assert!(track.locations.windows(2).all(|w| route_planner::boundary::segment_inside(&clipped.boundaries,route_planner::Coord::new(w[0].gLng,w[0].gLat),route_planner::Coord::new(w[1].gLng,w[1].gLat)))); }
        let wrapper=crate::track::wire::five_point_wrapper(&[],&track,&Default::default());
        assert!(crate::track::wire::validate_five_point_wrapper_for(&wrapper,true).is_ok());
        assert!(crate::track::wire::validate_five_point_wrapper_for(&wrapper,false).is_err());
        let value:Value=serde_json::from_str(&wrapper).unwrap();
        assert_eq!(value["fivePointJson"],"[]"); assert_eq!(value["geoFencesJson"],"[]"); assert_eq!(value["runAreaId"],-1);
        assert!(!root.0.join("points_cache.json").exists());
        assert!(!app.run_busy);
    }
}
#[test]
fn campus_confirmation_is_reachable_and_clickable_on_short_screens() {
    for (w,h) in [(320.,360.),(360.,400.),(412.,600.),(1100.,900.)] {
        let root=FixtureDir::new(); let ctx=egui::Context::default();
        let (host,server)=mock_school(absent_replies(json!({"error":10000,"pointsResModels":[]}),json!([])));
        let mut app=fixture_app(&root,&ctx,host); app.tab=7;
        std::fs::write(&app.config.osm_path,campus_osm()).unwrap();
        app.network=Some(std::sync::Arc::new(route_planner::load_osm(campus_osm().as_bytes()).unwrap()));
        frame(&mut app,&ctx,w,h,vec![]);
        let pos=scroll_to_label(&mut app,&ctx,w,h,"确认使用此校园边界");
        click(&mut app,&ctx,w,h,pos);
        assert!(app.config.map_boundary.is_some(),"{w}x{h}");
        wait_for_refresh(&mut app); assert_eq!(server.join().unwrap(),3);
        assert!(!app.run_page.preview_inputs.as_ref().expect(&app.osm_page.status).fences.is_empty());
        assert!(!app.run_busy);
    }
}

#[test]
fn failed_boundary_confirmation_save_does_not_enable_unsaved_constraint() {
    let root=FixtureDir::new(); let ctx=egui::Context::default();
    let mut app=fixture_app(&root,&ctx,"http://127.0.0.1:1".into());
    std::fs::create_dir(root.0.join("config.json")).unwrap();
    app.save_map_boundary_choice(Some(crate::api::map_boundary::Selection{school:"77".into(),id:"way/1".into(),revision:"fixture".into()}));
    assert!(app.config.map_boundary.is_none());
    assert!(!app.osm_page.busy); assert!(app.osm_page.status.contains("未保存"));
}

#[test]
fn school_lookup_works_without_roads_or_school_api_on_small_screens() {
    for (w,h) in [(320.,360.),(360.,400.),(412.,600.),(1100.,900.)] {
        let root=FixtureDir::new(); let ctx=egui::Context::default();
        let mut app=fixture_app(&root,&ctx,"http://127.0.0.1:1".into()); app.tab=7;
        app.session.as_mut().unwrap().profile=json!({"campusName":"肇庆医学院 鼎湖校区"});
        frame(&mut app,&ctx,w,h,vec![]);
        assert_eq!(app.osm_page.lookup.name,"肇庆医学院 鼎湖校区");
        let pos=scroll_to_label(&mut app,&ctx,w,h,"按校名查找校园边界");
        click(&mut app,&ctx,w,h,pos);
        let deadline=Instant::now()+Duration::from_secs(3);
        while app.osm_page.lookup.busy {
            assert!(Instant::now()<deadline); std::thread::sleep(Duration::from_millis(5)); app.poll_messages();
        }
        assert!(app.osm_page.lookup.status.contains("查询完成"),"{}",app.osm_page.lookup.status);
        assert!(app.network.is_none()); assert!(!app.osm_page.busy); assert!(!app.run_busy);
        assert_eq!(app.config.map_search_name.as_ref().unwrap().school,"77");
    }
}
#[test]
fn map_lookup_rejects_old_school_and_cancelled_results_and_keeps_errors_visible() {
    use crate::ui::campus_lookup::LookupResult;
    let root=FixtureDir::new(); let ctx=egui::Context::default();
    let mut app=fixture_app(&root,&ctx,"http://127.0.0.1:1".into());
    let items=route_planner::campus::parse(campus_osm().as_bytes()).unwrap();
    app.osm_page.lookup.id=2; app.osm_page.lookup.school="77".into();
    let tx=app.osm_page.lookup.tx.clone();
    tx.send(LookupResult{id:1,school:"77".into(),result:Ok(items.clone())}).unwrap();
    tx.send(LookupResult{id:2,school:"other".into(),result:Ok(items.clone())}).unwrap();
    app.poll_messages(); assert!(app.osm_page.lookup.items.is_empty());
    tx.send(LookupResult{id:2,school:"77".into(),result:Err("timeout".into())}).unwrap();
    app.poll_messages(); assert!(app.osm_page.lookup.status.contains("地图查询失败：timeout"));
    app.osm_page.lookup.cancel.cancel();
    tx.send(LookupResult{id:2,school:"77".into(),result:Ok(items)}).unwrap();
    app.poll_messages(); assert!(app.osm_page.lookup.items.is_empty());
}

#[test]
fn configured_dinghu_anchor_refreshes_to_a_complete_route_preview() {
    let root=FixtureDir::new();let ctx=egui::Context::default();
    let (host,server)=mock_school(absent_replies(json!({"error":10600,"message":"当前学校尚未设置点位，请联系学校老师"}),json!([])));
    let mut app=fixture_app(&root,&ctx,host);
    let (lat,lon)=crate::track::geom::wgs84_to_bd09(23.2057797,112.5811961);
    app.identity.anchor_lat=lat;app.identity.anchor_lon=lon;app.identity.city="肇庆市".into();
    let xml=include_bytes!("../../crates/route-planner/tests/fixtures/dinghu-roads.osm");
    std::fs::create_dir_all(root.0.join("maps")).unwrap();std::fs::write(root.0.join("maps/dinghu.osm"),xml).unwrap();
    app.config.osm_path="maps/dinghu.osm".into();
    let items=route_planner::campus::parse(xml).unwrap();let c=items.iter().find(|c|c.id=="way/689151593").unwrap();
    app.config.map_boundary=Some(crate::api::map_boundary::Selection{school:"77".into(),id:c.id.clone(),revision:crate::api::map_boundary::revision(c)});
    crate::api::model::save_config(&app.config).unwrap();crate::api::model::save_identity(&app.identity).unwrap();
    app.config=crate::api::model::load_config();app.identity=crate::api::model::load_identity();
    assert_eq!(std::path::Path::new(&app.config.osm_path),root.0.join("maps/dinghu.osm"));
    assert_eq!(app.identity.anchor_lat,lat);assert_eq!(app.identity.anchor_lon,lon);
    app.refresh_school_route(false);wait_for_refresh(&mut app);assert_eq!(server.join().unwrap(),3);
    let inputs=app.run_page.preview_inputs.as_ref().expect(&app.osm_page.status);
    assert_eq!(inputs.points,vec![(lat,lon)]);assert_eq!(inputs.boundary_label,"已确认的地图校园边界");
    let deadline=Instant::now()+Duration::from_secs(15);
    while app.run_page.preview.is_none() {
        app.poll_messages();frame(&mut app,&ctx,1100.,800.,vec![]);
        assert!(Instant::now()<deadline,"{}",app.run_page.preview_status);std::thread::sleep(Duration::from_millis(10));
    }
    assert!(app.run_page.check.as_ref().unwrap().issues.is_empty());
    let p=app.run_page.preview.as_ref().unwrap();assert!((p.laps.iter().sum::<f64>()-2400.).abs()<0.01);
    assert!(!app.run_busy);
}

// Exercise the actual disk cache and startup loader; no route inputs or worker
// replies are fabricated. Cache coordinates use the same conversion as roads.
fn saved_offline_fixture(root: &FixtureDir, ctx: &egui::Context) -> (App, String) {
    let mut app = fixture_app(root, ctx, "http://127.0.0.1:1".into());
    app.session = None;
    app.config.osm_path = "school.osm".into();
    app.config.update_check = "off".into();
    crate::api::model::save_identity(&app.identity).unwrap();
    crate::api::model::save_config(&app.config).unwrap();
    let fences = vec![[(39.8998, 116.3998), (39.8998, 116.4012),
        (39.9012, 116.4012), (39.9012, 116.3998)]
        .into_iter().map(|(lat, lon)| crate::track::geom::wgs84_to_bd09(lat, lon))
        .collect::<Vec<_>>()];
    let points = vec![crate::track::geom::wgs84_to_bd09(39.9, 116.4),
        crate::track::geom::wgs84_to_bd09(39.901, 116.401)];
    let key = crate::api::school_roads::scope_key("77", &fences);
    std::fs::create_dir_all(root.0.join("school_roads")).unwrap();
    std::fs::write(root.0.join("school_roads").join(format!("{key}.json")),
        serde_json::to_vec(&json!({"key":key,"school":"77","name":"离线重启测试校区",
            "loaded_ms":1,"xml":LOCAL_OSM,"fences":fences,"points":points})).unwrap()).unwrap();
    app.open_cached_road(&key);
    wait_for_refresh(&mut app);
    assert!(app.osm_page.offline, "{}", app.osm_page.status);
    assert!(app.network.is_some(), "{}", app.osm_page.status);
    assert_eq!(crate::api::model::load_config().offline_preview.as_ref().unwrap().key, key);
    (app, key)
}

fn restarted_offline_app(ctx: &egui::Context) -> App {
    App::from_state(ctx, crate::api::model::load_identity(), Session::default(),
        crate::api::model::load_config())
}

fn assert_offline_preview_completes(app: &mut App, ctx: &egui::Context, distance_m: f64) {
    wait_for_refresh(app);
    assert!(app.osm_page.offline, "{}", app.osm_page.status);
    app.tab = 0;
    let deadline = Instant::now() + Duration::from_secs(15);
    while app.run_page.preview.is_none() {
        app.poll_messages();
        frame(app, ctx, 1100., 900., vec![]);
        assert!(Instant::now() < deadline, "{} / {}", app.osm_page.status, app.run_page.preview_status);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(app.run_page.check.as_ref().unwrap().issues.is_empty());
    assert!((app.run_page.preview.as_ref().unwrap().laps.iter().sum::<f64>() - distance_m).abs() < 0.05);
    assert!(app.session.is_none());
    assert!(!app.run_busy);
}

#[test]
fn offline_preview_reopens_from_disk_without_another_click_or_login() {
    let root = FixtureDir::new();
    let ctx = egui::Context::default();
    let (app, key) = saved_offline_fixture(&root, &ctx);
    let expected = app.run_page.preview_inputs.as_ref().unwrap().clone();
    drop(app);
    let ctx = egui::Context::default();
    let mut restarted = restarted_offline_app(&ctx);
    assert!(restarted.network.is_none());
    assert!(restarted.run_page.preview_inputs.is_none());
    restarted.restore_route_on_startup();
    // Closing before the async loader reply is polled must not erase the saved
    // choice. A subsequent fresh process should still restore without a click.
    assert!(restarted.osm_page.busy);
    assert_eq!(crate::api::model::load_config().offline_preview.as_ref().unwrap().key, key);
    drop(restarted);
    let ctx = egui::Context::default();
    let mut restarted = restarted_offline_app(&ctx);
    restarted.restore_route_on_startup();
    assert_offline_preview_completes(&mut restarted, &ctx, 2400.);
    let actual = restarted.run_page.preview_inputs.as_ref().unwrap();
    assert_eq!(actual.points, expected.points);
    assert_eq!(actual.fences, expected.fences);
    assert_eq!(actual.unid, expected.unid);
    assert_eq!(restarted.config.offline_preview.as_ref().unwrap().key, key);
}

#[test]
fn offline_preview_reopens_after_moving_the_portable_directory() {
    let mut root = FixtureDir::new();
    let ctx = egui::Context::default();
    let (app, key) = saved_offline_fixture(&root, &ctx);
    drop(app);
    let old = root.0.clone();
    let moved = old.with_file_name(format!("route-refresh-moved-{}", uuid::Uuid::new_v4()));
    std::fs::rename(&old, &moved).unwrap();
    root.0 = moved;
    crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = Some(root.0.clone()));
    let ctx = egui::Context::default();
    let mut restarted = restarted_offline_app(&ctx);
    assert!(!old.exists());
    assert_eq!(std::path::Path::new(&restarted.config.osm_path), root.0.join("school.osm"));
    restarted.restore_route_on_startup();
    assert_offline_preview_completes(&mut restarted, &ctx, 2400.);
    assert_eq!(restarted.config.offline_preview.as_ref().unwrap().key, key);
}

#[test]
fn offline_restart_missing_or_broken_cache_does_not_load_a_different_map() {
    for failure in ["missing", "invalid-json", "invalid-osm"] {
        let root = FixtureDir::new();
        let ctx = egui::Context::default();
        let (app, key) = saved_offline_fixture(&root, &ctx);
        drop(app);
        let path = root.0.join("school_roads").join(format!("{key}.json"));
        match failure {
            "missing" => std::fs::remove_file(&path).unwrap(),
            "invalid-json" => std::fs::write(&path, "{broken").unwrap(),
            _ => {
                let mut cache: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                cache["xml"] = json!("<osm version=\"0.6\"></osm>");
                std::fs::write(&path, serde_json::to_vec(&cache).unwrap()).unwrap();
            }
        }
        let mut restarted = restarted_offline_app(&egui::Context::default());
        assert!(std::path::Path::new(&restarted.config.osm_path).is_file());
        restarted.restore_route_on_startup();
        wait_for_refresh(&mut restarted);
        assert!(restarted.network.is_none(), "{failure}: must not silently load school.osm");
        assert!(restarted.run_page.preview_inputs.is_none(), "{failure}");
        assert!(restarted.run_page.preview.is_none(), "{failure}");
        assert!(restarted.config.offline_preview.is_none(), "{failure}");
        assert!(crate::api::model::load_config().offline_preview.is_none(), "{failure}");
        assert!(!restarted.osm_page.status.is_empty());
        assert!(!restarted.osm_page.status.contains("加载成功"), "{failure}");
    }
}

#[test]
fn offline_restart_rejects_changed_school_anchor_or_route_mode() {
    for changed in ["school", "anchor-lat", "anchor-lon", "route-mode"] {
        let root = FixtureDir::new();
        let ctx = egui::Context::default();
        let (app, _) = saved_offline_fixture(&root, &ctx);
        drop(app);
        let mut restarted = restarted_offline_app(&egui::Context::default());
        match changed {
            "school" => restarted.session = Some(Session { uid: 43, unid: "88".into(),
                token: "other-local-fixture".into(), ..Default::default() }),
            "anchor-lat" => restarted.identity.anchor_lat += 0.01,
            "anchor-lon" => restarted.identity.anchor_lon += 0.01,
            _ => restarted.run_page.route_mode = crate::track::generate_road::RouteMode::Legacy,
        }
        restarted.restore_route_on_startup();
        assert!(!restarted.osm_page.busy, "{changed}");
        assert!(restarted.network.is_none(), "{changed}");
        assert!(restarted.run_page.preview_inputs.is_none(), "{changed}");
        assert!(restarted.osm_page.status.contains("未恢复"), "{changed}");
        assert!(crate::api::model::load_config().offline_preview.is_none(), "{changed}");
    }
}

#[test]
fn explicitly_selected_portable_cache_restores_for_the_same_logged_in_school_only() {
    let root = FixtureDir::new();
    let ctx = egui::Context::default();
    let (mut app, key) = saved_offline_fixture(&root, &ctx);
    let session = Session {
        uid: 43,
        unid: "139002".into(),
        token: "portable-selection-local-fixture".into(),
        ..Default::default()
    };
    app.session = Some(session.clone());
    // The portable cache identifies its own offline dataset (school 77), while
    // this explicit user selection belongs to the active account's school.
    app.open_cached_road(&key);
    wait_for_refresh(&mut app);
    assert_eq!(app.run_page.preview_inputs.as_ref().unwrap().unid, "77");
    assert_eq!(crate::api::model::load_config().offline_preview.as_ref().unwrap().school, "139002");
    drop(app);

    let ctx = egui::Context::default();
    let mut restarted = App::from_state(&ctx, crate::api::model::load_identity(),
        session, crate::api::model::load_config());
    restarted.restore_route_on_startup();
    wait_for_refresh(&mut restarted);
    assert!(restarted.osm_page.offline, "{}", restarted.osm_page.status);
    assert!(restarted.network.is_some(), "{}", restarted.osm_page.status);
    assert_eq!(restarted.run_page.preview_inputs.as_ref().unwrap().unid, "77");
    assert_eq!(restarted.config.offline_preview.as_ref().unwrap().school, "139002");
    let deadline = Instant::now() + Duration::from_secs(15);
    while restarted.run_page.preview.is_none() {
        restarted.poll_messages();
        frame(&mut restarted, &ctx, 1100., 900., vec![]);
        assert!(Instant::now() < deadline, "{}", restarted.run_page.preview_status);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(restarted.run_page.check.as_ref().unwrap().issues.is_empty());
    assert!(!restarted.run_busy);
    drop(restarted);

    let other_session = Session { uid: 44, unid: "other-school".into(),
        token: "other-school-local-fixture".into(), ..Default::default() };
    let mut other = App::from_state(&egui::Context::default(), crate::api::model::load_identity(),
        other_session, crate::api::model::load_config());
    other.restore_route_on_startup();
    assert!(!other.osm_page.busy);
    assert!(other.network.is_none());
    assert!(other.run_page.preview_inputs.is_none());
    assert!(other.osm_page.status.contains("未恢复"));
    assert!(crate::api::model::load_config().offline_preview.is_none());
}

#[test]
fn manual_map_switch_clears_offline_restart_and_discards_its_delayed_reply() {
    let root = FixtureDir::new();
    let ctx = egui::Context::default();
    let (mut app, key) = saved_offline_fixture(&root, &ctx);
    app.open_cached_road(&key);
    // Hold a real cache worker result until after a new manual-map request.
    let old_reply = app.net_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(old_reply.result.is_ok());
    let path = root.0.join("replacement.osm");
    std::fs::write(&path, LOCAL_OSM).unwrap();
    app.load_osm_async(path.display().to_string());
    assert_ne!(app.osm_page.request_id, old_reply.request_id);
    app.net_tx.send(old_reply).unwrap();
    wait_for_refresh(&mut app);
    assert!(app.network.is_some());
    assert!(!app.osm_page.offline);
    assert!(app.run_page.preview_inputs.is_none());
    assert!(app.run_page.preview.is_none());
    assert!(app.config.offline_preview.is_none());
    assert!(crate::api::model::load_config().offline_preview.is_none());
    assert_eq!(app.config.osm_path, path.display().to_string());
}

#[test]
fn deleting_active_cache_clears_current_preview_and_restart_selection() {
    let root = FixtureDir::new();
    let ctx = egui::Context::default();
    let (mut app, key) = saved_offline_fixture(&root, &ctx);
    app.tab = 7;
    let pos = scroll_to_label(&mut app, &ctx, 1100., 900., "已保存的学校路网");
    click(&mut app, &ctx, 1100., 900., pos);
    let pos = scroll_to_label(&mut app, &ctx, 1100., 900., "清除此缓存");
    click(&mut app, &ctx, 1100., 900., pos);
    assert!(!root.0.join("school_roads").join(format!("{key}.json")).exists());
    assert!(crate::api::model::load_config().offline_preview.is_none());
    assert!(app.network.is_none());
    assert!(app.run_page.preview_inputs.is_none());
    assert!(!app.osm_page.busy);
}

#[test]
#[ignore="explicit offline acceptance check against the exported portable preset"]
fn exported_dinghu_preset_opens_offline_preview_without_login() {
    let exported=std::path::PathBuf::from(std::env::var("NEKO_DINGHU_EXPORT_DIR").expect("export directory required"));
    let root=FixtureDir::new();
    for file in ["config.json","identity.json","maps/dinghu.osm"] {
        let target=root.0.join(file);std::fs::create_dir_all(target.parent().unwrap()).unwrap();std::fs::copy(exported.join(file),target).unwrap();
    }
    std::fs::create_dir_all(root.0.join("school_roads")).unwrap();
    for f in std::fs::read_dir(exported.join("school_roads")).unwrap() {let f=f.unwrap();std::fs::copy(f.path(),root.0.join("school_roads").join(f.file_name())).unwrap();}
    let identity=crate::api::model::load_identity();let config=crate::api::model::load_config();
    assert!(std::path::Path::new(&config.osm_path).is_file());
    let ctx=egui::Context::default();let mut app=App::from_state(&ctx,identity,Session::default(),config);app.tab=7;
    app.osm_page.cache_entries=crate::api::school_roads::entries();assert_eq!(app.osm_page.cache_entries.len(),1);
    frame(&mut app,&ctx,1100.,900.,vec![]);
    let pos=scroll_to_label(&mut app,&ctx,1100.,900.,"已保存的学校路网");click(&mut app,&ctx,1100.,900.,pos);
    let pos=scroll_to_label(&mut app,&ctx,1100.,900.,"离线预览");click(&mut app,&ctx,1100.,900.,pos);
    wait_for_refresh(&mut app);assert!(app.osm_page.offline);assert!(app.network.is_some(),"{}",app.osm_page.status);
    app.tab=0;
    let deadline=Instant::now()+Duration::from_secs(15);
    while app.run_page.preview.is_none() {
        app.poll_messages();frame(&mut app,&ctx,1100.,900.,vec![]);
        assert!(Instant::now()<deadline,"{}",app.run_page.preview_status);std::thread::sleep(Duration::from_millis(10));
    }
    assert!(app.run_page.check.as_ref().unwrap().issues.is_empty());
    assert!((app.run_page.preview.as_ref().unwrap().laps.iter().sum::<f64>()-1000.).abs()<0.01);
    assert!(app.session.is_none());assert!(!app.run_busy);
    assert!(crate::api::model::load_config().offline_preview.is_some());
    drop(app);
    let ctx = egui::Context::default();
    let mut restarted = restarted_offline_app(&ctx);
    restarted.restore_route_on_startup();
    assert_offline_preview_completes(&mut restarted, &ctx, 1000.);
    println!("Portable preset: no-login offline UI generated a 1000 m route and restored it after restart without another click");
}
