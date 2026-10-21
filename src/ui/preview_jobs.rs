use super::{
    run::{PreviewInputs, RunPlan},
    App,
};
use std::sync::Arc;
pub struct PreviewResult {
    pub key: String,
    pub check: route_planner::check::Check,
    pub plan: Result<crate::track::generate_road::RoadPlan, String>,
}
impl App {
    pub(crate) fn poll_preview_results(&mut self) {
        while let Ok(result) = self.preview_rx.try_recv() {
            self.run_page.preview_busy = false;
            if result.key == self.run_page.preview_key
                && self.run_page.preview_cancel.check().is_ok()
            {
                self.run_page.check = Some(result.check);
                self.run_page.map_fitted = false;
                match result.plan {
                    Ok(p) => {
                        self.run_page.preview = Some(p);
                        self.run_page.preview_status =
                            if self.run_page.preview.as_ref().unwrap().fences.is_empty() {
                                "道路预览已完成（未提供学校边界）".into()
                            } else {
                                "路线检查和预览已完成".into()
                            };
                    }
                    Err(e) => self.run_page.preview_status = crate::road_status::explain(&e),
                }
            }
        }
    }

    pub(crate) fn prepare_preview(
        &mut self,
        ui: &mut egui::Ui,
        net: Arc<route_planner::RoadGraph>,
        inputs: &PreviewInputs,
        plan: &RunPlan,
    ) {
        let key = format!(
            "{}:{}:{}:{}:{}",
            self.osm_page.request_id, inputs.loaded_ms, plan.dist, plan.dur, plan.seed
        );
        if key != self.run_page.preview_key {
            self.run_page.preview_cancel.cancel();
            self.run_page.preview_key = key.clone();
            self.run_page.preview_changed_at = Some(std::time::Instant::now());
            self.run_page.preview_stale = true;
            self.run_page.preview = None;
            self.run_page.check = None;
        }
        self.poll_preview_results();
        if self.run_page.preview_busy {
            ui.label(&self.run_page.preview_status);
            if ui
                .add_enabled(
                    self.run_page.preview_cancel.cancellable(),
                    egui::Button::new("取消路线规划"),
                )
                .clicked()
            {
                self.run_page.preview_cancel.cancel();
                self.run_page.preview_stale = false;
                self.run_page.preview_status =
                    "已请求取消；后台规划结束前仍可能占用计算资源，其结果将丢弃".into();
            }
        } else if ui.button("重新检查并规划路线").clicked() {
            self.run_page.preview_stale = true;
            self.run_page.preview_changed_at = None;
        }
        if self.run_page.preview_stale
            && !self.run_page.preview_busy
            && self
                .run_page
                .preview_changed_at
                .map(|t| t.elapsed().as_millis() >= 300)
                .unwrap_or(true)
        {
            self.run_page.preview_cancel = Arc::new(crate::task_control::Control::default());
            let cancel = self.run_page.preview_cancel.clone();
            let inputs = inputs.clone();
            let plan = plan.clone();
            let tx = self.preview_tx.clone();
            self.run_page.preview_busy = true;
            self.run_page.preview_stale = false;
            self.run_page.preview_status = if inputs.fences.is_empty() {
                "规划本地预览：检查点位与道路连通性（学校边界未校验）…".into()
            } else {
                "规划路线：裁剪围栏、检查点位和校内道路…".into()
            };
            std::thread::spawn(move || {
                let mut aligned = (*net).clone();
                crate::track::generate_road::align_network(&mut aligned);
                let clipped = crate::track::generate_road::apply_fences(&aligned, &inputs.fences);
                let check = route_planner::check::inspect(
                    &clipped,
                    &inputs
                        .points
                        .iter()
                        .map(|&(a, b)| route_planner::Coord::new(b, a))
                        .collect::<Vec<_>>(),
                    plan.dist * 1000.,
                );
                let result = cancel
                    .check()
                    .and_then(|_| {
                        crate::track::generate_road::plan_road_view(
                            &net,
                            &inputs.points,
                            plan.dist * 1000.,
                            plan.dur,
                            plan.seed,
                            &inputs.fences,
                        )
                    })
                    .and_then(|p| cancel.check().map(|_| p));
                let _ = tx.send(PreviewResult {
                    key,
                    check,
                    plan: result,
                });
            });
        }
        if !self.run_page.preview_status.is_empty() {
            ui.label(&self.run_page.preview_status);
        }
        if self.run_page.preview_busy || self.run_page.preview_stale {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}
