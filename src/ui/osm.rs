//! 在线路网：地图选范围 → 下载道路拓扑 → 预览与规划。

use eframe::egui;
use std::sync::Arc;

use route_planner::Coord;

use super::map::{collect_bounds, MapState};
use super::{theme, App};

pub struct OsmPage {
    pub path: String,
    load_rx: Option<std::sync::mpsc::Receiver<Result<LoadedRoads, String>>>,
    pub busy: bool,
    pub status: String,
    pub map: MapState,
    pub fitted: bool,
    pub(super) online_center: Option<(f64, f64)>,
    pub(super) radius: u32,
    pub(super) picker: Option<super::location_picker::LocationPicker>,
}

enum LoadedRoads {
    Local {
        path: String,
        network: route_planner::RoadGraph,
    },
    Online(super::online_osm::Download),
}

impl Default for OsmPage {
    fn default() -> Self {
        OsmPage {
            path: String::new(),
            load_rx: None,
            busy: false,
            status: String::new(),
            map: MapState::default(),
            fitted: false,
            online_center: None,
            radius: 1000,
            picker: None,
        }
    }
}

fn ring_to_pts(ring: &[Coord]) -> Vec<(f64, f64)> {
    ring.iter().map(|c| (c.lat, c.lon)).collect()
}

impl App {
    pub fn draw_osm_page(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| self.draw_osm_inner(ui));
    }

    fn draw_osm_inner(&mut self, ui: &mut egui::Ui) {
        let mut import = false;
        super::mobile::row(ui, |ui| {
            ui.label("本地 OSM 文件：");
            ui.add(
                egui::TextEdit::singleline(&mut self.osm_page.path)
                    .desired_width(240.0)
                    .hint_text("填写 .osm / .xml 路径，或拖入文件"),
            );
            #[cfg(not(target_os = "android"))]
            if ui.button("浏览…").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("OSM XML", &["osm", "xml"])
                    .pick_file()
                {
                    self.osm_page.path = path.display().to_string();
                }
            }
            if ui
                .add_enabled(
                    !self.osm_page.busy && !self.osm_page.path.trim().is_empty(),
                    theme::primary_btn("导入本地路网"),
                )
                .clicked()
            {
                import = true;
            }
        });
        if !self.osm_page.busy {
            if let Some(path) = ui.ctx().input(|input| {
                input
                    .raw
                    .dropped_files
                    .iter()
                    .filter_map(|file| file.path.as_ref())
                    .find(|path| {
                        path.extension().is_some_and(|ext| {
                            ext.eq_ignore_ascii_case("osm") || ext.eq_ignore_ascii_case("xml")
                        })
                    })
                    .cloned()
            }) {
                self.osm_page.path = path.display().to_string();
                import = true;
            }
        }
        if import {
            self.load_osm_async(self.osm_page.path.clone());
        }
        ui.separator();
        let mut center = self
            .osm_page
            .online_center
            .unwrap_or((self.identity.anchor_lat, self.identity.anchor_lon));
        super::mobile::row(ui, |ui| {
            ui.label("在线路网中心：");
            if ui.button("使用已保存锚点").clicked() {
                center = (self.identity.anchor_lat, self.identity.anchor_lon);
                self.osm_page.picker = None;
            }
            if ui
                .button(if self.osm_page.picker.is_some() {
                    "关闭选点"
                } else {
                    "地图选择范围"
                })
                .clicked()
            {
                self.osm_page.picker = if self.osm_page.picker.is_some() {
                    None
                } else {
                    Some(super::location_picker::LocationPicker::new(center))
                };
            }
            ui.label("半径（米）：");
            ui.add(
                egui::DragValue::new(&mut self.osm_page.radius)
                    .range(200..=2000)
                    .speed(100),
            );
            if ui
                .add_enabled(!self.osm_page.busy, theme::primary_btn("在线下载附近路网"))
                .clicked()
            {
                self.download_osm_async(center);
            }
        });
        if let Some(picker) = self.osm_page.picker.as_mut() {
            if let Some(point) = picker.draw(ui, center) {
                center = point;
            }
        }
        self.osm_page.online_center = Some(center);
        ui.label(format!(
            "中心 WGS84：{:.7}, {:.7}；下载道路与建筑，缓存 7 天。此处选点不修改设备锚点。",
            center.0, center.1
        ));
        ui.hyperlink_to(
            "© OpenStreetMap contributors · Overpass API",
            "https://www.openstreetmap.org/copyright",
        );

        if !self.osm_page.status.is_empty() {
            ui.colored_label(theme::plain(), &self.osm_page.status);
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
                let path = if self.config.osm_path.is_empty() {
                    &self.config.road_cache_path
                } else {
                    &self.config.osm_path
                };
                if let Ok(meta) = std::fs::metadata(path) {
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
            egui::Vec2::new(rect.width().max(1.0), rect.height().clamp(220.0, 420.0)),
            egui::Sense::drag(),
        );
        let canvas = response.rect;
        let painter = painter.with_clip_rect(canvas);

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
            let road_color = egui::Color32::from_rgb(190, 200, 196);
            for e in net.edges_deg() {
                self.osm_page.map.draw_polyline(
                    &painter,
                    canvas,
                    &ring_to_pts(&e),
                    road_color,
                    1.0,
                );
            }
            let bld_color = egui::Color32::from_rgb(214, 182, 158);
            for b in &net.buildings {
                self.osm_page
                    .map
                    .draw_polygon(&painter, canvas, &ring_to_pts(b), bld_color, 1.0);
            }
        } else {
            painter.text(
                canvas.center(),
                egui::Align2::CENTER_CENTER,
                "导入本地 OSM 或在线下载路网后可预览道路与建筑",
                egui::FontId::proportional(14.0),
                egui::Color32::from_rgb(150, 160, 156),
            );
        }

        self.osm_page.map.interact(ui, canvas);
    }

    /// Failed loads preserve the current network and saved source.
    pub(super) fn poll_online_osm(&mut self) {
        let Some(receiver) = self.osm_page.load_rx.as_ref() else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err("路网加载线程已退出，请重试".into())
            }
        };
        self.osm_page.load_rx = None;
        self.osm_page.busy = false;
        match result {
            Ok(loaded) => {
                let (network, source) = match loaded {
                    LoadedRoads::Local { path, network } => {
                        self.config.osm_path = path;
                        self.config.road_cache_path.clear();
                        (network, "已导入本地")
                    }
                    LoadedRoads::Online(download) => {
                        self.config.osm_path.clear();
                        self.config.road_cache_path = download.path.display().to_string();
                        (
                            download.network,
                            if download.cached {
                                "已读取缓存"
                            } else {
                                "已下载并缓存"
                            },
                        )
                    }
                };
                self.osm_page.fitted = false;
                self.run_page.preview_stale = true;
                self.osm_page.status = format!(
                    "√ {source}路网：{} 节点 / {} 建筑",
                    network.graph.node_count(),
                    network.buildings.len()
                );
                self.network = Some(Arc::new(network));
                if let Err(error) = crate::api::model::save_config(&self.config) {
                    self.osm_page
                        .status
                        .push_str(&format!("；配置保存失败：{error}"));
                }
            }
            Err(error) => self.osm_page.status = error,
        }
    }

    fn download_osm_async(&mut self, center: (f64, f64)) {
        let coordinate = match crate::location::Coordinate::new(center.0, center.1, 0.0) {
            Ok(coordinate) => coordinate,
            Err(error) => {
                self.osm_page.status = error;
                return;
            }
        };
        let radius = self.osm_page.radius;
        let (sender, receiver) = std::sync::mpsc::channel();
        self.osm_page.load_rx = Some(receiver);
        self.osm_page.busy = true;
        self.osm_page.status = format!("正在下载附近 {radius} 米路网…");
        std::thread::spawn(move || {
            let _ = sender
                .send(super::online_osm::download(coordinate, radius).map(LoadedRoads::Online));
        });
    }

    pub fn load_osm_async(&mut self, path: String) {
        if self.osm_page.busy {
            return;
        }
        self.osm_page.path = path.clone();
        self.osm_page.busy = true;
        self.osm_page.status = format!("正在导入本地路网：{path}");
        let (sender, receiver) = std::sync::mpsc::channel();
        self.osm_page.load_rx = Some(receiver);
        std::thread::spawn(move || {
            let result = std::fs::read(&path)
                .map_err(|error| format!("读取本地路网失败：{error}"))
                .and_then(|bytes| route_planner::load_osm(&bytes))
                .map(|network| LoadedRoads::Local { path, network });
            let _ = sender.send(result);
        });
    }

    /// Restore only the application's downloaded road cache on startup.
    pub(super) fn restore_road_cache(&mut self, path: String) {
        self.osm_page.busy = true;
        self.osm_page.status = "正在读取已下载路网…".into();
        let (sender, receiver) = std::sync::mpsc::channel();
        self.osm_page.load_rx = Some(receiver);
        std::thread::spawn(move || {
            let _ = sender.send(
                super::online_osm::load_cache(std::path::PathBuf::from(path))
                    .map(LoadedRoads::Online),
            );
        });
    }
}
