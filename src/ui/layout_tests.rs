use super::*;
use egui::{Event, PointerButton, Pos2, Rect, Vec2};

#[path = "route_refresh_tests.rs"]
mod route_refresh_tests;
#[path = "violation_tests.rs"]
mod violation_tests;

fn frame(
    app: &mut App,
    ctx: &egui::Context,
    width: f32,
    height: f32,
    events: Vec<Event>,
) -> egui::FullOutput {
    ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, height))),
            events,
            ..Default::default()
        },
        |ctx| app.draw_shell(ctx),
    )
}

fn label_position(output: &egui::FullOutput, label: &str) -> Option<Pos2> {
    output.shapes.iter().find_map(|clipped| {
        if let egui::Shape::Text(text) = &clipped.shape {
            let rect = Rect::from_min_size(text.pos, text.galley.size());
            if text.galley.text() == label && clipped.clip_rect.contains(rect.center()) {
                return Some(rect.center());
            }
        }
        None
    })
}

fn click(app: &mut App, ctx: &egui::Context, w: f32, h: f32, at: Pos2) {
    for pressed in [true, false] {
        frame(
            app,
            ctx,
            w,
            h,
            vec![
                Event::PointerMoved(at),
                Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
    }
}

fn scroll_to_label(app: &mut App, ctx: &egui::Context, w: f32, h: f32, label: &str) -> Pos2 {
    for _ in 0..60 {
        let output = frame(app, ctx, w, h, vec![]);
        if let Some(pos) = label_position(&output, label) {
            return pos;
        }
        frame(
            app,
            ctx,
            w,
            h,
            vec![
                Event::PointerMoved(Pos2::new(w / 2.0, h - 70.0)),
                Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: Vec2::new(0.0, -60.0),
                    modifiers: Default::default(),
                },
            ],
        );
        // Let the scroll animation finish before locating/clicking a target.
        for _ in 0..12 {
            frame(app, ctx, w, h, vec![]);
        }
    }
    panic!("outer scroll cannot reach {label} at {w}x{h}");
}

#[test]
fn short_phone_data_page_can_scroll_and_operate_controls() {
    for (w, h) in [
        (320.0, 360.0),
        (320.0, 480.0),
        (360.0, 400.0),
        (360.0, 540.0),
        (412.0, 600.0),
        (880.0, 720.0),
    ] {
        let ctx = egui::Context::default();
        let mut app = App::from_state(
            &ctx,
            HeaderIdentity::default(),
            Session::default(),
            Config::default(),
        );
        app.tab = 3;
        app.data_page.cheat = Some(crate::api::cheat::CheatReport {
            self_info: serde_json::Value::Null,
            list: (0..20)
                .map(|i| serde_json::json!({"name":format!("row-{i}"),"reason":"fixture"}))
                .collect(),
            ..Default::default()
        });
        app.data_page.rank_rows = (0..30)
            .map(|i| crate::api::rank::RankRow {
                sort: i,
                name: format!("rank-{i}"),
                length: 1000.0,
                gender: 1,
            })
            .collect();
        // Two settling frames populate egui's previous-frame hit-testing rectangles.
        frame(&mut app, &ctx, w, h, vec![]);
        let output = frame(&mut app, &ctx, w, h, vec![]);
        let refresh = label_position(&output, "刷新")
            .expect("refresh must be visible below the compact top bar");
        click(&mut app, &ctx, w, h, refresh);
        assert_eq!(app.status, "请先登录"); // Reached the real handler without a network request.
        let toggle = scroll_to_label(&mut app, &ctx, w, h, "查看列表");
        click(&mut app, &ctx, w, h, toggle);
        assert!(app.data_page.show_cheat_list);
        let query = scroll_to_label(&mut app, &ctx, w, h, "查询");
        app.status = "before-query".into();
        click(&mut app, &ctx, w, h, query);
        assert_eq!(app.status, "请先登录");
    }
}

#[test]
fn compact_account_form_can_expand_and_scroll_to_login() {
    for w in [320.0, 360.0, 412.0] {
        let h = 400.0;
        let ctx = egui::Context::default();
        let mut app = App::from_state(
            &ctx,
            HeaderIdentity::default(),
            Session::default(),
            Config::default(),
        );
        app.tab = 3;
        frame(&mut app, &ctx, w, h, vec![]);
        let output = frame(&mut app, &ctx, w, h, vec![]);
        assert!(label_position(&output, "账号：").is_none());
        click(
            &mut app,
            &ctx,
            w,
            h,
            label_position(&output, "账号与登录").unwrap(),
        );
        let mut login = None;
        for _ in 0..50 {
            let output = frame(
                &mut app,
                &ctx,
                w,
                h,
                vec![
                    Event::PointerMoved(Pos2::new(w / 2.0, 110.0)),
                    Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: Vec2::new(0.0, -40.0),
                        modifiers: Default::default(),
                    },
                ],
            );
            if let Some(pos) = label_position(&output, "登录") {
                login = Some(pos);
                break;
            }
        }
        assert!(login.is_some(), "login must be reachable at {w}x{h}");
        // Empty credentials exercise validation only, so this click stays offline.
        app.status = "before-login".into();
        let output = frame(&mut app, &ctx, w, h, vec![]);
        click(
            &mut app,
            &ctx,
            w,
            h,
            label_position(&output, "登录").unwrap(),
        );
        assert!(!app.login_busy);
        assert_ne!(app.status, "before-login");
    }
}

#[test]
fn failed_network_load_clears_previous_preview() {
    let ctx = egui::Context::default();
    let mut app = App::from_state(
        &ctx,
        HeaderIdentity::default(),
        Session::default(),
        Config::default(),
    );
    app.network = Some(std::sync::Arc::new(route_planner::RoadGraph::new()));
    app.run_page.preview_stale = false;
    app.net_tx
        .send(super::osm::NetworkLoad {
            source: None,
            pending: false,
            warning: String::new(),
            offline: false,
            request_id: 0,
            scope: None,
            campuses: Vec::new(),
            selected_key: String::new(),
            inputs: None,
            result: Err("offline fixture".into()),
        })
        .unwrap();
    app.poll_messages();
    assert!(app.network.is_none());
    assert!(app.run_page.preview.is_none());
    assert!(app.run_page.preview_stale);
    assert!(app.osm_page.status.contains("offline fixture"));
}

#[test]
fn school_download_controls_are_reachable_on_phone_and_desktop() {
    for (w, h) in [(320., 360.), (360., 480.), (412., 600.), (1100., 800.)] {
        let ctx = egui::Context::default();
        let mut app = App::from_state(
            &ctx,
            HeaderIdentity::default(),
            Session::default(),
            Config::default(),
        );
        app.tab = 7;
        let at = scroll_to_label(&mut app, &ctx, w, h, "自动获取学校路网");
        click(&mut app, &ctx, w, h, at);
        assert!(app.osm_page.status.contains("请先登录"));
        assert!(!app.osm_page.busy);
        scroll_to_label(&mut app, &ctx, w, h, "重新下载路网");
    }
}

#[test]
fn stale_network_results_cannot_replace_newer_school_or_manual_load() {
    let ctx = egui::Context::default();
    let mut app = App::from_state(
        &ctx,
        HeaderIdentity::default(),
        Session::default(),
        Config::default(),
    );
    app.osm_page.request_id = 2;
    app.osm_page.busy = true;
    app.net_tx
        .send(super::osm::NetworkLoad {
            source: None,
            pending: false,
            warning: String::new(),
            offline: false,
            request_id: 1,
            scope: None,
            campuses: Vec::new(),
            selected_key: String::new(),
            inputs: None,
            result: Ok(std::sync::Arc::new(route_planner::RoadGraph::new())),
        })
        .unwrap();
    app.poll_messages();
    assert!(app.network.is_none());
    assert!(app.osm_page.busy);
    app.net_tx
        .send(super::osm::NetworkLoad {
            source: None,
            pending: false,
            warning: String::new(),
            offline: false,
            request_id: 2,
            campuses: Vec::new(),
            selected_key: String::new(),
            scope: Some((
                42,
                "old-school".into(),
                Some(crate::location::Coordinate::new(39.9, 116.4, 5.0).unwrap()),
                "old-token".into(),
            )),
            inputs: None,
            result: Ok(std::sync::Arc::new(route_planner::RoadGraph::new())),
        })
        .unwrap();
    app.poll_messages();
    assert!(app.network.is_none());
    assert!(!app.osm_page.busy);
}

#[test]
fn cancelling_download_discards_late_result_and_retry_is_reachable() {
    let ctx = egui::Context::default();
    let mut app = App::from_state(
        &ctx,
        HeaderIdentity::default(),
        Session::default(),
        Config::default(),
    );
    app.tab = 7;
    let id = app.begin_network_load();
    let at = scroll_to_label(&mut app, &ctx, 320., 420., "取消获取路网");
    click(&mut app, &ctx, 320., 420., at);
    assert!(!app.osm_page.busy);
    assert!(app.osm_page.cancel.check().is_err());
    app.net_tx
        .send(super::osm::NetworkLoad {
            source: None,
            pending: false,
            warning: String::new(),
            offline: false,
            request_id: id,
            scope: None,
            campuses: vec![],
            selected_key: String::new(),
            inputs: None,
            result: Ok(std::sync::Arc::new(route_planner::RoadGraph::new())),
        })
        .unwrap();
    app.poll_messages();
    assert!(app.network.is_none());
    assert!(app.osm_page.status.contains("取消"));
    scroll_to_label(&mut app, &ctx, 320., 420., "重试获取路网");
}

#[test]
fn diagnostics_export_button_is_reachable_and_writes_no_account_fields() {
    let root = std::env::temp_dir().join(format!("diagnostic-ui-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = Some(root.clone()));
    let ctx = egui::Context::default();
    let mut app = App::from_state(
        &ctx,
        HeaderIdentity::default(),
        Session::default(),
        Config::default(),
    );
    app.tab = 7;
    app.status = "登录失败 token=very-secret-account-token".into();
    let at = scroll_to_label(&mut app, &ctx, 360., 480., "一键导出脱敏诊断报告");
    click(&mut app, &ctx, 360., 480., at);
    let files: Vec<_> = std::fs::read_dir(&root).unwrap().flatten().collect();
    assert_eq!(files.len(), 1);
    let text = std::fs::read_to_string(files[0].path()).unwrap();
    assert!(!text.contains("very-secret"));
    assert!(text.contains("error_category"));
    crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = None);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn obsolete_and_cancelled_preview_results_are_ignored_even_off_the_run_page() {
    let ctx = egui::Context::default();
    let mut app = App::from_state(
        &ctx,
        HeaderIdentity::default(),
        Session::default(),
        Config::default(),
    );
    app.tab = 7;
    app.run_page.preview_key = "new-school".into();
    app.run_page.preview_status = "current".into();
    app.run_page.preview_busy = true;
    app.preview_tx
        .send(super::preview_jobs::PreviewResult {
            key: "old-school".into(),
            check: Default::default(),
            plan: Err("old failure".into()),
        })
        .unwrap();
    app.poll_messages();
    assert!(!app.run_page.preview_busy);
    assert_eq!(app.run_page.preview_status, "current");
    assert!(app.run_page.check.is_none());
    app.run_page.preview_cancel.cancel();
    app.run_page.preview_busy = true;
    app.preview_tx
        .send(super::preview_jobs::PreviewResult {
            key: "new-school".into(),
            check: Default::default(),
            plan: Err("cancelled failure".into()),
        })
        .unwrap();
    app.poll_messages();
    assert!(!app.run_page.preview_busy);
    assert_eq!(app.run_page.preview_status, "current");
}

const LOCAL_OSM: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<osm version="0.6">
<node id="1" lat="39.9" lon="116.4"/>
<node id="2" lat="39.9" lon="116.401"/>
<node id="3" lat="39.901" lon="116.401"/>
<node id="4" lat="39.901" lon="116.4"/>
<way id="10"><nd ref="1"/><nd ref="2"/><nd ref="3"/><nd ref="4"/><nd ref="1"/>
<tag k="highway" v="footway"/></way></osm>"#;

#[test]
fn manual_osm_button_loads_without_login_and_draws_roads_with_visible_status() {
    for (w, h) in [(320., 420.), (360., 480.), (412., 600.), (1100., 800.)] {
        let root = std::env::temp_dir().join(format!("osm-ui-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = Some(root.clone()));
        let path = root.join("学校 路网.osm");
        std::fs::write(&path, LOCAL_OSM).unwrap();
        let ctx = egui::Context::default();
        let mut app = App::from_state(
            &ctx,
            HeaderIdentity::default(),
            Session::default(),
            Config::default(),
        );
        app.run_page.route_mode = crate::track::generate_road::RouteMode::Road;
        app.tab = 7;
        app.osm_page.path = path.display().to_string();
        let at = scroll_to_label(&mut app, &ctx, w, h, "加载路网");
        click(&mut app, &ctx, w, h, at);
        assert!(app.osm_page.status.contains("加载中"));
        // No session is available: the old implementation never spawned a local file reader.
        let loaded = app
            .net_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        assert!(loaded.result.is_ok());
        app.net_tx.send(loaded).unwrap();
        app.poll_messages();
        assert!(!app.osm_page.busy);
        assert_eq!(app.network.as_ref().unwrap().graph.node_count(), 4);
        let status = app.osm_page.status.clone();
        let output = frame(&mut app, &ctx, w, h, vec![]);
        assert!(
            label_position(&output, &status).is_some(),
            "status is hidden at {w}x{h}"
        );
        let mut drawn = false;
        for _ in 0..40 {
            let out = frame(
                &mut app,
                &ctx,
                w,
                h,
                vec![
                    Event::PointerMoved(Pos2::new(w / 2., h - 35.)),
                    Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: Vec2::new(0., -60.),
                        modifiers: Default::default(),
                    },
                ],
            );
            drawn |= out.shapes.iter().any(|shape| match &shape.shape {
                egui::Shape::Path(p) => {
                    p.stroke.color
                        == egui::epaint::ColorMode::Solid(egui::Color32::from_rgb(190, 200, 196))
                        && p.points
                            .iter()
                            .any(|point| shape.clip_rect.contains(*point))
                }
                _ => false,
            });
        }
        assert!(drawn, "loaded roads are not visible at {w}x{h}");
        crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = None);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn invalid_manual_osm_reports_error_and_retry_reads_corrected_file_offline() {
    let root = std::env::temp_dir().join(format!("osm-retry-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = Some(root.clone()));
    let path = root.join("map.osm");
    std::fs::write(&path, "<html>download failed</html>").unwrap();
    let ctx = egui::Context::default();
    let mut app = App::from_state(
        &ctx,
        HeaderIdentity::default(),
        Session::default(),
        Config::default(),
    );
    app.tab = 7;
    app.run_page.route_mode = crate::track::generate_road::RouteMode::Road;
    app.load_osm_async(path.display().to_string());
    let msg = app
        .net_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    app.net_tx.send(msg).unwrap();
    app.poll_messages();
    assert!(app.network.is_none());
    assert!(app.osm_page.status.contains("OSM XML"));
    std::fs::write(&path, LOCAL_OSM).unwrap();
    let at = scroll_to_label(&mut app, &ctx, 360., 480., "重试获取路网");
    click(&mut app, &ctx, 360., 480., at);
    let msg = app
        .net_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    app.net_tx.send(msg).unwrap();
    app.poll_messages();
    assert!(app.network.is_some());
    assert!(app.osm_page.status.contains("加载成功"));
    crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = None);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn downloaded_roads_are_visible_while_points_load_and_survive_metadata_failure() {
    let ctx = egui::Context::default();
    let mut app = App::from_state(
        &ctx,
        HeaderIdentity::default(),
        Session::default(),
        Config::default(),
    );
    let request_id = app.begin_network_load();
    let net = std::sync::Arc::new(route_planner::load_osm(LOCAL_OSM.as_bytes()).unwrap());
    for (pending, warning) in [
        (true, "正在刷新路线点位"),
        (false, "路线预览暂不可用：点位请求失败"),
    ] {
        app.net_tx
            .send(super::osm::NetworkLoad {
                source: None,
                pending,
                warning: warning.into(),
                offline: false,
                request_id,
                campuses: vec![],
                selected_key: String::new(),
                scope: None,
                inputs: None,
                result: Ok(net.clone()),
            })
            .unwrap();
        app.poll_messages();
        assert_eq!(app.osm_page.busy, pending);
        assert!(std::sync::Arc::ptr_eq(app.network.as_ref().unwrap(), &net));
        assert!(app.osm_page.status.contains("路网加载成功"));
        assert!(app.osm_page.status.contains(warning));
        assert!(app.run_page.preview_inputs.is_none());
    }
    let completed_status = app.osm_page.status.clone();
    app.tx
        .send(format!("__ROAD_STAGE__{request_id}|正在下载（迟到消息）"))
        .unwrap();
    app.poll_messages();
    assert_eq!(app.osm_page.status, completed_status);
}

#[test]
fn school_401_clears_saved_session_but_preserves_manual_map_and_device_settings() {
    let root = std::env::temp_dir().join(format!("expired-login-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = Some(root.clone()));
    let session = Session {
        uid: 42,
        token: "expired-fixture".into(),
        unid: "school".into(),
        ..Default::default()
    };
    model::save_session(&session).unwrap();
    model::save_identity(&HeaderIdentity::default()).unwrap();
    model::save_config(&Config::default()).unwrap();
    let ctx = egui::Context::default();
    let mut app = App::from_state(
        &ctx,
        HeaderIdentity::default(),
        session.clone(),
        Config::default(),
    );
    let net = std::sync::Arc::new(route_planner::load_osm(LOCAL_OSM.as_bytes()).unwrap());
    let id = app.begin_network_load();
    app.net_tx
        .send(super::osm::NetworkLoad {
            source: None,
            pending: false,
            warning: format!("路线预览暂不可用：{}", crate::api::client::SESSION_EXPIRED),
            offline: false,
            request_id: id,
            campuses: vec![],
            selected_key: String::new(),
            scope: Some((
                session.uid,
                session.unid,
                app.identity.anchor_coordinate().ok(),
                session.token,
            )),
            inputs: None,
            result: Ok(net.clone()),
        })
        .unwrap();
    app.poll_messages();
    assert!(app.auth_expired);
    assert!(app.session.is_none());
    assert!(!model::load_session().is_logged_in());
    assert!(root.join("identity.json").is_file());
    assert!(root.join("config.json").is_file());
    assert!(std::sync::Arc::ptr_eq(app.network.as_ref().unwrap(), &net));
    assert!(app.run_page.preview_inputs.is_none());
    assert!(app.osm_page.status.contains("重新登录"));
    assert!(!app.osm_page.busy);
    assert_ne!(app.osm_page.request_id, id);
    crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = None);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn delayed_401_from_old_token_cannot_remove_a_new_login() {
    let ctx = egui::Context::default();
    let session = Session {
        uid: 42,
        token: "fresh-fixture".into(),
        unid: "school".into(),
        ..Default::default()
    };
    let mut app = App::from_state(
        &ctx,
        HeaderIdentity::default(),
        session.clone(),
        Config::default(),
    );
    app.osm_page.busy = true;
    app.osm_page.status = "fresh request".into();
    app.net_tx
        .send(super::osm::NetworkLoad {
            source: None,
            pending: false,
            warning: String::new(),
            offline: false,
            request_id: app.osm_page.request_id,
            campuses: vec![],
            selected_key: String::new(),
            scope: Some((
                session.uid,
                session.unid,
                app.identity.anchor_coordinate().ok(),
                "expired-fixture".into(),
            )),
            inputs: None,
            result: Err(crate::api::client::SESSION_EXPIRED.into()),
        })
        .unwrap();
    app.poll_messages();
    assert_eq!(app.session.as_ref().unwrap().token, "fresh-fixture");
    assert!(!app.auth_expired);
    assert!(app.osm_page.busy);
    assert_eq!(app.osm_page.status, "fresh request");
}

#[test]
fn expired_login_recovery_button_is_reachable_without_network_or_saved_password() {
    for (w, h) in [(320., 420.), (360., 480.), (412., 600.), (1100., 800.)] {
        let ctx = egui::Context::default();
        let mut app = App::from_state(
            &ctx,
            HeaderIdentity::default(),
            Session::default(),
            Config::default(),
        );
        app.auth_expired = true;
        app.tab = 7;
        let at = scroll_to_label(&mut app, &ctx, w, h, "重新登录");
        click(&mut app, &ctx, w, h, at);
        assert_eq!(app.status, "请填写账号和密码");
        assert!(!app.login_busy);
        assert!(app.auth_expired);
    }
}

#[test]
fn failed_queries_are_not_empty_success_or_a_clean_cheat_result() {
    let ctx = egui::Context::default();
    let mut app = App::from_state(
        &ctx,
        HeaderIdentity::default(),
        Session::default(),
        Config::default(),
    );
    app.data_page.cheat = Some(crate::api::cheat::CheatReport {
        self_info: serde_json::Value::Null,
        list: vec![],
        ..Default::default()
    });
    for prefix in [RECORDS, AI_LIST, AI_RECORDS, CHEAT, SEMESTER, RANK] {
        app.tx
            .send(format!(
                "{prefix}{}",
                serde_json::json!({"fetchError":"offline fixture failure"})
            ))
            .unwrap();
        app.poll_messages();
        assert!(app.status.contains("刷新失败"));
        assert!(!app.status.contains("已刷新"));
    }
    assert!(app.data_page.cheat.is_none());
}

#[test]
fn session_wide_401_clears_login_and_late_job_results_cannot_restore_success() {
    let root = std::env::temp_dir().join(format!("global-auth-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = Some(root.clone()));
    let session = Session {
        uid: 42,
        token: "fixture-token".into(),
        ..Default::default()
    };
    let ctx = egui::Context::default();
    let mut app = App::from_state(
        &ctx,
        HeaderIdentity::default(),
        session.clone(),
        Config::default(),
    );
    app.records_busy = true;
    app.user_busy = true;
    app.data_busy = true;
    app.run_busy = true;
    app.ai_submit_active = true;
    app.ai_busy = true;
    let epoch = app.session_epoch;
    app.auth_tx
        .send(crate::api::client::AuthFailure {
            epoch,
            uid: session.uid,
            token: session.token,
        })
        .unwrap();
    app.poll_messages();
    assert!(app.session.is_none());
    assert!(app.auth_expired);
    assert!(!app.records_busy && !app.user_busy && !app.data_busy);
    assert!(
        app.run_busy && app.ai_busy,
        "auth failure cannot pretend an in-flight submission completed"
    );
    app.tx
        .send(format!("__SESSION_JOB__{epoch}|{RECORDS}[]"))
        .unwrap();
    app.tx
        .send(format!(
            "__SESSION_JOB__{epoch}|{CHEAT}{{\"self\":null,\"list\":[]}}"
        ))
        .unwrap();
    app.tx
        .send(format!("__SESSION_JOB__{epoch}|√ [user] 我的页数据已更新"))
        .unwrap();
    app.poll_messages();
    assert!(app.data_page.cheat.is_none());
    assert_eq!(app.status, crate::api::client::SESSION_EXPIRED);
    crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = None);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn old_session_notifications_and_results_cannot_override_a_fresh_login() {
    let ctx = egui::Context::default();
    let session = Session {
        uid: 42,
        token: "new-token".into(),
        ..Default::default()
    };
    let mut app = App::from_state(&ctx, HeaderIdentity::default(), session, Config::default());
    app.renew_session_jobs();
    app.status = "new session".into();
    app.auth_tx
        .send(crate::api::client::AuthFailure {
            epoch: 0,
            uid: 42,
            token: "old-token".into(),
        })
        .unwrap();
    app.tx
        .send(format!("__SESSION_JOB__0|{RECORDS}[]"))
        .unwrap();
    app.poll_messages();
    assert!(app.session.is_some());
    assert!(!app.auth_expired);
    assert_eq!(app.status, "new session");
}

#[test]
fn re_login_errors_remain_visible_instead_of_being_overwritten_by_expiry_banner() {
    let ctx = egui::Context::default();
    let mut app = App::from_state(
        &ctx,
        HeaderIdentity::default(),
        Session::default(),
        Config::default(),
    );
    app.auth_expired = true;
    app.tx
        .send(format!(
            "{LOGIN_DONE}{}",
            serde_json::json!({"ok":false,"message":"密码错误"})
        ))
        .unwrap();
    app.poll_messages();
    assert_eq!(app.status, "登录失败：密码错误");
    app.poll_messages();
    assert_eq!(app.status, "登录失败：密码错误");
}

#[test]
fn run_page_exposes_metadata_failure_and_navigation_after_map_load() {
    for (w, h) in [(320., 420.), (360., 480.), (412., 600.), (1100., 800.)] {
        let ctx = egui::Context::default();
        let mut app = App::from_state(
            &ctx,
            HeaderIdentity::default(),
            Session::default(),
            Config::default(),
        );
        app.run_page.route_mode = crate::track::generate_road::RouteMode::Road;
        app.network = Some(std::sync::Arc::new(route_planner::RoadGraph::new()));
        app.osm_page.status = "点位接口请求失败（模拟）".into();
        app.tab = 0;
        scroll_to_label(&mut app, &ctx, w, h, "点位接口请求失败（模拟）");
        let at = scroll_to_label(&mut app, &ctx, w, h, "查看路网与校区");
        click(&mut app, &ctx, w, h, at);
        assert_eq!(app.tab, 7);
        app.tab = 0;
        let at = scroll_to_label(&mut app, &ctx, w, h, "设置定位锚点");
        click(&mut app, &ctx, w, h, at);
        assert_eq!(app.tab, 5);
    }
}

#[test]
fn manual_map_completion_refreshes_metadata_once_without_discarding_map() {
    let root = std::env::temp_dir().join(format!("manual-metadata-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = Some(root.clone()));
    let ctx = egui::Context::default();
    let session = Session {
        uid: 42,
        token: "fixture".into(),
        unid: "school".into(),
        ..Default::default()
    };
    let mut app = App::from_state(&ctx, HeaderIdentity::default(), session, Config::default());
    app.run_page.route_mode = crate::track::generate_road::RouteMode::Road;
    // A rejected shared session guarantees the worker cannot contact a real service.
    app.auth_guard
        .store(true, std::sync::atomic::Ordering::SeqCst);
    app.config.osm_path = "already-parsed-fixture.osm".into();
    let id = app.begin_network_load();
    let net = std::sync::Arc::new(route_planner::RoadGraph::new());
    app.net_tx
        .send(super::osm::NetworkLoad {
            source: None,
            pending: false,
            warning: "地图已读取".into(),
            offline: false,
            request_id: id,
            campuses: vec![],
            selected_key: String::new(),
            scope: None,
            inputs: None,
            result: Ok(net.clone()),
        })
        .unwrap();
    app.poll_messages();
    assert!(
        app.osm_page.request_id > id,
        "manual import must start metadata refresh"
    );
    assert!(std::sync::Arc::ptr_eq(app.network.as_ref().unwrap(), &net));
    for _ in 0..200 {
        app.poll_messages();
        if !app.osm_page.busy {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(!app.osm_page.busy);
    assert!(
        app.auth_expired,
        "metadata failure must be reported, not re-read a missing OSM file"
    );
    assert_eq!(
        app.osm_page.request_id,
        id + 2,
        "refresh once, then retire the expired session"
    );
    assert!(std::sync::Arc::ptr_eq(app.network.as_ref().unwrap(), &net));
    crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = None);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn run_page_distinguishes_active_refresh_from_idle_missing_data() {
    let ctx = egui::Context::default();
    let mut app = App::from_state(
        &ctx,
        HeaderIdentity::default(),
        Session::default(),
        Config::default(),
    );
    app.tab = 0;
    app.run_page.route_mode = crate::track::generate_road::RouteMode::Road;
    app.network = Some(std::sync::Arc::new(route_planner::RoadGraph::new()));
    app.osm_page.busy = true;
    scroll_to_label(
        &mut app,
        &ctx,
        1100.,
        900.,
        "正在刷新学校围栏、策略与点位，请查看上方进度。",
    );
    app.osm_page.busy = false;
    scroll_to_label(
        &mut app,
        &ctx,
        1100.,
        900.,
        "本地地图已加载；请先登录，再刷新路线点位与围栏。",
    );
    app.session = Some(Session {
        uid: 42,
        token: "fixture".into(),
        ..Default::default()
    });
    scroll_to_label(
        &mut app,
        &ctx,
        1100.,
        900.,
        "本地地图已加载；请设置定位锚点，再刷新路线点位与围栏。",
    );
}

#[test]
fn manual_online_results_preserve_source_and_reject_stale_replies() {
    let root = std::env::temp_dir().join(format!("manual-online-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = Some(root.clone()));
    let ctx = egui::Context::default();
    let mut app = App::from_state(&ctx, HeaderIdentity::default(), Session::default(), Config::default());
    let network = Arc::new(route_planner::load_osm(LOCAL_OSM.as_bytes()).unwrap());
    let cached = root.join("online_roads/fixture.osm").display().to_string();
    let result = |request_id, source, result| super::osm::NetworkLoad {
        source: Some(source), pending: false, warning: String::new(), offline: false,
        request_id, campuses: Vec::new(), selected_key: String::new(), scope: None, inputs: None, result,
    };
    app.net_tx.send(result(0, super::osm::RoadSource::Online(cached.clone()), Ok(network.clone()))).unwrap();
    app.poll_messages();
    assert_eq!(app.config.manual_road_path(), cached);
    assert!(app.config.osm_path.is_empty());
    assert_eq!(model::load_config().road_cache_path, cached);
    app.net_tx.send(result(0, super::osm::RoadSource::Local("broken.osm".into()), Err("invalid map".into()))).unwrap();
    app.poll_messages();
    assert!(Arc::ptr_eq(app.network.as_ref().unwrap(), &network));
    assert_eq!(app.config.manual_road_path(), cached);
    app.begin_network_load();
    app.net_tx.send(result(0, super::osm::RoadSource::Local("stale.osm".into()), Ok(network))).unwrap();
    app.poll_messages();
    assert!(app.network.is_none());
    assert_eq!(app.config.manual_road_path(), cached);
    crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = None);
    std::fs::remove_dir_all(root).unwrap();
}
