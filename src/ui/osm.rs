//! 「路网」导入页：选择 OSM 文件 → 后台解析 → 画布预览 + 统计。

use eframe::egui;
use std::sync::Arc;

use route_planner::{Coord, RoadGraph};

use super::map::{collect_bounds, MapState};
use super::{theme, App};

#[derive(Clone)]
pub struct NetworkLoad {
    pub pending: bool,
    pub warning: String,
    pub offline: bool,
    pub request_id: u64,
    pub campuses: Vec<crate::api::campus::Campus>,
    pub selected_key: String,
    pub scope: Option<(i64, String, Option<crate::location::Coordinate>, String)>,
    pub inputs: Option<super::run::PreviewInputs>,
    pub result: Result<Arc<RoadGraph>, String>,
}

pub struct OsmPage {
    pub(crate) lookup: super::campus_lookup::LookupUi,
    pub map_choice: String,
    pub campuses: Vec<crate::api::campus::Campus>,
    pub selected_key: String,
    pub offline: bool,
    pub pending_offline_key: Option<String>,
    pub cancel: Arc<crate::task_control::Control>,
    pub cache_entries: Vec<crate::api::school_roads::Entry>,
    pub gpx_path: String,
    pub gpx_text: Option<String>,
    pub gpx: Option<crate::track::analysis::Analysis>,
    pub request_id: u64,
    pub path: String,
    pub busy: bool,
    pub status: String,
    pub map: MapState,
    pub fitted: bool,
}

impl Default for OsmPage {
    fn default() -> Self {
        OsmPage {
            lookup: Default::default(),
            map_choice: String::new(),
            cancel: Arc::new(crate::task_control::Control::default()),
            campuses: Vec::new(),
            selected_key: String::new(),
            offline: false,
            pending_offline_key: None,
            cache_entries: Vec::new(),
            gpx_path: String::new(),
            gpx_text: None,
            gpx: None,
            request_id: 0,
            path: String::new(),
            busy: false,
            status: String::new(),
            map: MapState::default(),
            fitted: false,
        }
    }
}

fn ring_to_pts(ring: &[Coord]) -> Vec<(f64, f64)> {
    ring.iter().map(|c| (c.lat, c.lon)).collect()
}

impl App {
    pub fn draw_osm_page(&mut self, ui: &mut egui::Ui) {
        if self.auth_expired {
            ui.colored_label(
                theme::warn(),
                "学校登录已失效，请重新登录；本地地图仍可查看。",
            );
            if ui
                .add_enabled(!self.login_busy, theme::primary_btn("重新登录"))
                .clicked()
            {
                self.do_login();
            }
        }
        if !self.osm_page.status.is_empty() {
            ui.colored_label(theme::plain(), &self.osm_page.status);
        }

        egui::ScrollArea::vertical()
            .id_salt("school_roads_scroll")
            .show(ui, |ui| self.draw_osm_contents(ui));
    }

    fn draw_osm_contents(&mut self, ui: &mut egui::Ui) {
        let mut load_now = false;

        ui.label("按学校围栏或当前点位获取路网；多出的里程沿同一道路闭环重复。");
        let school = self
            .session
            .as_ref()
            .and_then(|s| s.profile.get("campusName"))
            .and_then(|v| v.as_str())
            .unwrap_or("请先登录并配置定位锚点");
        ui.label(format!("当前学校：{school}"));
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(!self.osm_page.busy, theme::primary_btn("自动获取学校路网"))
                .clicked()
            {
                self.use_school_roads(false);
            }
            if ui
                .add_enabled(
                    !self.osm_page.busy && self.config.osm_path.is_empty(),
                    egui::Button::new("重新下载路网"),
                )
                .clicked()
            {
                self.use_school_roads(true);
            }
            ui.label(if self.config.osm_path.is_empty() {
                "当前：按学校自动获取"
            } else {
                "当前：手动文件"
            });
        });
        ui.hyperlink_to(
            "路网数据 © OpenStreetMap contributors",
            "https://www.openstreetmap.org/copyright",
        );
        ui.label("手动导入支持 OSM XML（.osm/.xml），无需登录；PBF/JSON 请先转换为 OSM XML。路线预览另需登录刷新策略和点位。");
        ui.label("无学校围栏时也可沿道路闭环重复里程，但无法保证路线在校内；服务端仍按实际规则判定成绩。");
        ui.horizontal_wrapped(|ui| {
            ui.label("手动 OSM 文件：");
            ui.add_sized(
                [ui.available_width().min(340.0).max(100.0), 26.0],
                egui::TextEdit::singleline(&mut self.osm_page.path)
                    .hint_text("拖入 .osm 文件或填写路径"),
            );
            #[cfg(not(target_os = "android"))]
            if ui.button("浏览…").clicked() {
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter("OSM XML", &["osm", "xml"])
                    .pick_file()
                {
                    self.osm_page.path = p.display().to_string();
                }
            }
            let can_load = !self.osm_page.path.is_empty() && !self.osm_page.busy;
            if ui
                .add_enabled(can_load, theme::primary_btn("加载路网"))
                .clicked()
            {
                load_now = true;
            }
        });

        ui.horizontal_wrapped(|ui| {
            if self.osm_page.busy && ui.button("取消获取路网").clicked() {
                self.osm_page.cancel.cancel();
                self.osm_page.request_id = self.osm_page.request_id.wrapping_add(1);
                self.osm_page.busy = false;
                self.osm_page.status =
                    "已取消接收结果；已发出的网络请求可能仍会运行到结束或超时".into();
            }
            if !self.osm_page.busy && ui.button("重试获取路网").clicked() {
                if self.config.osm_path.is_empty() {
                    self.refresh_school_route(false);
                } else {
                    self.load_osm_async(self.osm_page.path.clone());
                }
            }
        });
        self.draw_road_tools(ui);

        // 拖拽导入
        let dropped: Vec<String> = ui.ctx().input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.as_ref().map(|p| p.display().to_string()))
                .collect()
        });
        if let Some(p) = dropped.into_iter().find(|p| !p.is_empty()) {
            self.osm_page.path = p;
            if !self.osm_page.busy {
                load_now = true;
            }
        }

        // 统计
        if let Some(net) = &self.network {
            ui.horizontal(|ui| {
                ui.label(format!(
                    "节点 {} · 有向边 {} · 建筑 {}",
                    net.graph.node_count(),
                    net.graph.edge_count(),
                    net.buildings.len()
                ));
                if let Ok(meta) = std::fs::metadata(&self.osm_page.path) {
                    ui.label(format!("· 文件 {:.1} KB", meta.len() as f64 / 1024.0));
                }
            });
        }

        ui.add_space(4.0);
        if self.network.is_some() {
            super::map::legend(
                ui,
                &[
                    ("道路", egui::Color32::from_rgb(190, 200, 196)),
                    ("建筑", egui::Color32::from_rgb(214, 182, 158)),
                ],
            );
        }
        let rect = ui.available_rect_before_wrap();
        let (response, painter) = ui.allocate_painter(
            egui::Vec2::new(
                rect.width().max(200.0),
                if ui.available_width() < 600.0 {
                    240.0
                } else {
                    400.0
                },
            ),
            egui::Sense::drag(),
        );
        let canvas = response.rect;
        let fences = self
            .run_page
            .preview_inputs
            .as_ref()
            .map(|i| i.fences.clone())
            .or_else(|| {
                self.osm_page
                    .campuses
                    .iter()
                    .find(|c| c.key == self.osm_page.selected_key)
                    .map(|c| c.polygons.clone())
            })
            .unwrap_or_default();
        let display_fences: Vec<Vec<(f64, f64)>> = fences
            .iter()
            .map(|f| {
                f.iter()
                    .map(|&(a, b)| crate::api::school_roads::to_wgs(a, b))
                    .collect()
            })
            .collect();
        if !self.osm_page.fitted && (!display_fences.is_empty() || self.osm_page.gpx.is_some()) {
            let mut bounds: Vec<_> = display_fences.iter().flatten().copied().collect();
            if let Some(gpx) = &self.osm_page.gpx {
                bounds.extend(gpx.segments.iter().flatten().copied());
            }
            self.osm_page.map.fit(canvas, &bounds);
            self.osm_page.fitted = true;
        }

        if let Some(net) = &self.network {
            if !self.osm_page.fitted {
                let mut bounds: Vec<(f64, f64)> = collect_bounds(
                    &net.edges_deg()
                        .iter()
                        .map(|e| ring_to_pts(e))
                        .collect::<Vec<_>>(),
                );
                for b in &net.buildings {
                    bounds.extend(ring_to_pts(b));
                }
                self.osm_page.map.fit(canvas, &bounds);
                self.osm_page.fitted = true;
            }
        }

        painter.rect_filled(
            canvas,
            egui::Rounding::ZERO,
            egui::Color32::from_rgb(250, 252, 251),
        );
        if let Some(net) = &self.network {
            self.osm_page.map.draw_network(&painter, canvas, net);
        } else {
            painter.text(
                canvas.center(),
                egui::Align2::CENTER_CENTER,
                "加载 OSM 路网后可预览道路与建筑",
                egui::FontId::proportional(14.0),
                egui::Color32::from_rgb(150, 160, 156),
            );
        }

        self.osm_page.map.draw_boundary(&painter,canvas,&display_fences,egui::Color32::from_rgb(180,118,0),2.5);
        if let Some(gpx) = &self.osm_page.gpx {
            for line in &gpx.segments {
                self.osm_page.map.draw_polyline(
                    &painter,
                    canvas,
                    line,
                    egui::Color32::from_rgb(160, 40, 190),
                    2.,
                );
            }
            for line in &gpx.crossing {
                self.osm_page
                    .map
                    .draw_polyline(&painter, canvas, line, egui::Color32::RED, 3.);
            }
            for &(lat, lon) in &gpx.outside {
                self.osm_page
                    .map
                    .draw_point(&painter, canvas, lat, lon, egui::Color32::RED, 4.);
            }
        }
        self.osm_page.map.interact(ui, canvas);

        if load_now {
            let path = self.osm_page.path.clone();
            self.load_osm_async(path);
        }
    }

    pub(crate) fn begin_network_load(&mut self) -> u64 {
        self.forget_offline_preview();
        self.reset_network_load()
    }

    pub(crate) fn reset_network_load(&mut self) -> u64 {
        self.osm_page.pending_offline_key = None;
        self.osm_page.cancel.cancel();
        self.osm_page.cancel = Arc::new(crate::task_control::Control::default());
        self.run_page.preview_cancel.cancel();
        self.osm_page.campuses.clear();
        self.osm_page.selected_key.clear();
        self.osm_page.request_id = self.osm_page.request_id.wrapping_add(1);
        self.network = None;
        self.run_page.preview = None;
        self.run_page.preview_inputs = None;
        self.run_page.preview_stale = true;
        self.osm_page.busy = true;
        self.osm_page.offline = false;
        self.run_page.check = None;
        self.osm_page.request_id
    }

    fn use_school_roads(&mut self, force: bool) {
        if self.session.is_none() {
            self.osm_page.status = "请先登录，才能识别学校与围栏".into();
            return;
        }
        self.config.osm_path.clear();
        self.osm_page.path.clear();
        self.config.route_mode = "road".into();
        self.run_page.route_mode = crate::track::generate_road::RouteMode::Road;
        let _ = crate::api::model::save_config(&self.config);
        self.refresh_school_route(force);
    }

    /// 后台线程加载 OSM 路网，成功后经 `net_rx` 回传。
    pub fn load_osm_async(&mut self, path: String) {
        // Reading a local map must never depend on login, location or school APIs.
        let inputs = None;
        let campuses = self.osm_page.campuses.clone();
        let selected_key = self.osm_page.selected_key.clone();
        let request_id = self.begin_network_load();
        self.osm_page.path = path.clone();
        self.osm_page.status = format!("加载中：{path} …");
        self.config.osm_path = path.clone();
        let _ = crate::api::model::save_config(&self.config);
        let tx = self.net_tx.clone();
        let control = self.osm_page.cancel.clone();
        std::thread::spawn(move || {
            let result = control
                .check()
                .and_then(|_| crate::track::generate_road::load_network_path(&path).map(Arc::new))
                .and_then(|net| {
                    control.check()?;
                    Ok(net)
                });
            let _ = tx.send(NetworkLoad {
                pending: false,
                warning: if inputs.is_none() {
                    "本地路网已读取；路线预览需登录后刷新策略与点位".into()
                } else {
                    String::new()
                },
                offline: false,
                request_id,
                scope: None,
                campuses,
                selected_key,
                inputs,
                result,
            });
        });
    }
}
