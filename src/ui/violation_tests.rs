use super::*;
use crate::api::cheat::{CheatReport, Status};
use serde_json::json;

#[test]
fn query_failure_missing_fields_and_stale_result_are_visible() {
    for width in [320., 360., 412., 880.] {
        let ctx = egui::Context::default();
        let mut app = App::from_state(
            &ctx,
            HeaderIdentity::default(),
            Session::default(),
            Config::default(),
        );
        app.tab = 3;
        let now = crate::crypto::envelope::now_ms();
        for (data, at, expected) in [
            (json!({"self":null}), now, Status::NoRecord),
            (json!({}), now, Status::Unknown),
            (json!({"self":false}), now, Status::Unknown),
            (
                json!({"self":null}),
                now - crate::api::cheat::FRESH_MS,
                Status::Stale,
            ),
        ] {
            let report = CheatReport::from_data(&data, at).unwrap();
            app.tx
                .send(format!(
                    "{CHEAT}{}",
                    serde_json::to_string(&report).unwrap()
                ))
                .unwrap();
            app.poll_messages();
            assert_eq!(app.data_page.cheat.as_ref().unwrap().status(), expected);
            let label = app.data_page.cheat.as_ref().unwrap().label();
            scroll_to_label(&mut app, &ctx, width, 480., label);
        }
        app.data_page.cheat_checking = true;
        app.tx
            .send(format!("{CHEAT}{}", json!({"fetchError":"模拟查询失败"})))
            .unwrap();
        app.poll_messages();
        assert!(!app.data_page.cheat_checking);
        assert!(app.data_page.cheat.is_none());
        scroll_to_label(&mut app, &ctx, width, 480., "检查失败：模拟查询失败");
    }
}

#[test]
fn related_record_button_scrolls_and_opens_only_the_explicit_id() {
    for (width, height) in [(320., 360.), (360., 400.), (412., 480.), (880., 720.)] {
        let (host, server) = super::route_refresh_tests::mock_school(vec![(
            crate::api::records::GET_ONE_PATH,
            json!({"error":10000,"data":{"rrid":"42","statusInfo":"fixture detail","speed":"6833.5","reasonList":[{"reason":"raw reason","extraCode":201}]}}),
        )]);
        let ctx = egui::Context::default();
        let mut app = App::from_state(
            &ctx,
            HeaderIdentity::default(),
            Session {
                uid: 42,
                unid: "77".into(),
                token: "offline-only".into(),
                ..Default::default()
            },
            Config::default(),
        );
        app.route_fixture = Some((host, std::env::temp_dir()));
        app.tab = 3;
        app.data_page.cheat = Some(CheatReport::from_data(&json!({"self":{
            "reason":"服务端原始原因".repeat(10), "rrid":"42", "id":999, "time":"2026-10-20 07:30:00"
        }}), crate::crypto::envelope::now_ms()).unwrap());
        let at = scroll_to_label(&mut app, &ctx, width, height, "查看关联跑步记录 #42");
        click(&mut app, &ctx, width, height, at);
        assert_eq!(app.tab, 2);
        assert_eq!(app.records_page.detail_rrid, Some(42));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.records_page.detail_loading {
            app.poll_messages();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(server.join().unwrap(), 1);
        assert_eq!(app.records_page.detail_raw.as_ref().unwrap()["rrid"], 42);
        assert_eq!(app.records_page.detail_raw.as_ref().unwrap()["speed"], "6833.5");
        assert_eq!(app.records_page.detail_raw.as_ref().unwrap()["reasonList"][0]["extraCode"], 201);
    }
}

#[test]
fn late_record_detail_cannot_replace_current_violation_record() {
    let ctx = egui::Context::default();
    let mut app = App::from_state(
        &ctx,
        HeaderIdentity::default(),
        Session::default(),
        Config::default(),
    );
    app.records_page.detail_rrid = Some(42);
    app.records_page.detail_loading = true;
    app.tx
        .send(format!(
            "{RUN_DETAIL}{}",
            json!({"rrid":9,"statusInfo":"old"})
        ))
        .unwrap();
    app.poll_messages();
    assert!(app.records_page.detail_raw.is_none());
    assert!(app.records_page.detail_loading);
    app.tx
        .send(format!(
            "{RUN_DETAIL}{}",
            json!({"rrid":42,"statusInfo":"current"})
        ))
        .unwrap();
    app.tx
        .send(format!(
            "{RUN_DETAIL}{}",
            json!({"rrid":9,"statusInfo":"late old reply"})
        ))
        .unwrap();
    app.poll_messages();
    assert_eq!(
        app.records_page.detail_raw.unwrap()["statusInfo"],
        "current"
    );
}

#[test]
fn mismatched_detail_finishes_with_an_error_for_the_requested_record() {
    let (host, server) = super::route_refresh_tests::mock_school(vec![(
        crate::api::records::GET_ONE_PATH,
        json!({"error":10000,"data":{"rrid":99}}),
    )]);
    let ctx = egui::Context::default();
    let mut app = App::from_state(
        &ctx,
        HeaderIdentity::default(),
        Session {
            uid: 42,
            unid: "77".into(),
            token: "offline-only".into(),
            ..Default::default()
        },
        Config::default(),
    );
    app.route_fixture = Some((host, std::env::temp_dir()));
    app.records_page.detail_rrid = Some(42);
    app.fetch_run_detail(42);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while app.records_page.detail_loading {
        app.poll_messages();
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(server.join().unwrap(), 1);
    let raw = app.records_page.detail_raw.unwrap();
    assert_eq!(raw["rrid"], 42);
    assert!(raw["statusInfo"]
        .as_str()
        .unwrap()
        .contains("编号与请求不一致"));
}

#[test]
fn active_submission_button_cannot_start_a_second_task() {
    let ctx = egui::Context::default();
    let mut app = App::from_state(
        &ctx,
        HeaderIdentity::default(),
        Session::default(),
        Config::default(),
    );
    app.tab = 0;
    app.run_busy = true;
    app.status = "existing submission".into();
    // Tall viewport isolates button gating from the separately tested short-screen scrolling.
    for _ in 0..2 {
        for _ in 0..3 {
            frame(&mut app, &ctx, 880., 900., vec![]);
        }
        let output = frame(&mut app, &ctx, 880., 900., vec![]);
        let at = label_position(&output, "提交中…").unwrap();
        click(&mut app, &ctx, 880., 900., at);
    }
    assert!(app.run_busy);
    assert_eq!(app.status, "existing submission");
}
