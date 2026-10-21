//! School selection, offline maps, diagnostics and local GPX analysis controls.
use super::App;
use eframe::egui;
impl App {
    pub(crate) fn draw_road_tools(&mut self, ui: &mut egui::Ui) {
        if !self.osm_page.campuses.is_empty() && !self.osm_page.offline {
            let old = self.osm_page.selected_key.clone();
            let name = self
                .osm_page
                .campuses
                .iter()
                .find(|c| c.key == old)
                .map(|c| c.name.as_str())
                .unwrap_or("选择服务端校区/围栏");
            egui::ComboBox::from_id_salt("campus_choice")
                .selected_text(name)
                .width(ui.available_width().min(300.))
                .show_ui(ui, |ui| {
                    for c in &self.osm_page.campuses {
                        ui.selectable_value(
                            &mut self.osm_page.selected_key,
                            c.key.clone(),
                            &c.name,
                        );
                    }
                });
            if self.osm_page.selected_key != old {
                if let Some(s) = &self.session {
                    self.config.school_campus = Some(crate::api::campus::Selection {
                        school: s.unid.clone(),
                        key: self.osm_page.selected_key.clone(),
                        confirmed: false,
                    });
                }
                let _ = crate::api::model::save_config(&self.config);
                self.refresh_school_route(false);
            }
            if let Some(s) = self.config.school_campus.as_mut() {
                if ui
                    .checkbox(&mut s.confirmed, "已核对下方围栏属于当前校区")
                    .changed()
                {
                    let _ = crate::api::model::save_config(&self.config);
                }
            }
        }
        if self.osm_page.offline {
            ui.colored_label(
                super::theme::warn(),
                "离线预览：使用保存时的边界和点位；正式运行会在线重新获取最新数据。",
            );
        }
        ui.collapsing("已保存的学校路网", |ui| {
            if ui.button("刷新缓存列表").clicked() {
                self.osm_page.cache_entries = crate::api::school_roads::entries();
            }
            if self.osm_page.cache_entries.is_empty() {
                ui.label("没有已列出的缓存，可点击刷新。");
            }
            for entry in self.osm_page.cache_entries.clone() {
                ui.separator();
                ui.label(if entry.name.is_empty() {
                    format!("学校编号 {}（未提供校区名）", entry.school)
                } else {
                    entry.name.clone()
                });
                ui.label(format!(
                    "下载：{} · {:.1} KB",
                    crate::track::geom::fmt_cst(entry.loaded_ms, "%Y-%m-%d %H:%M"),
                    entry.bytes as f64 / 1024.
                ));
                ui.horizontal_wrapped(|ui| {
                    if ui.button("离线预览").clicked() {
                        self.open_cached_road(&entry.key);
                    }
                    if ui.button("更新此学校").clicked() {
                        if self
                            .session
                            .as_ref()
                            .map(|s| s.unid == entry.school)
                            .unwrap_or(false)
                        {
                            if entry.fences.is_empty() {
                                self.config.osm_path.clear();
                                self.config.road_cache_path.clear();
                                let _ = crate::api::model::save_config(&self.config);
                                self.refresh_school_route(true);
                            } else if let Some(c) = self.osm_page.campuses.iter().find(|c| {
                                crate::api::school_roads::scope_key("", &c.polygons)
                                    == crate::api::school_roads::scope_key("", &entry.fences)
                            }) {
                                self.config.school_campus = Some(crate::api::campus::Selection {
                                    school: entry.school.clone(),
                                    key: c.key.clone(),
                                    confirmed: false,
                                });
                                self.config.osm_path.clear();
                                self.config.road_cache_path.clear();
                                let _ = crate::api::model::save_config(&self.config);
                                self.refresh_school_route(true);
                            } else {
                                self.osm_page.status =
                                    "请先在线获取该学校校区列表，选择并确认此缓存对应围栏后更新"
                                        .into();
                            }
                        } else {
                            self.osm_page.status =
                                "请先登录该学校账号，更新时将重新读取当前校区围栏".into();
                        }
                    }
                    if ui.button("清除此缓存").clicked() {
                        match crate::api::school_roads::clear_cache(&entry.key) {
                            Ok(()) => self.osm_page.cache_entries.retain(|c| c.key != entry.key),
                            Err(e) => self.osm_page.status = e,
                        }
                    }
                });
            }
        });
        ui.collapsing("导入真实 GPX 并分析", |ui| {
            ui.label("仅在本地分析；不用于成绩提交。GPX 坐标按 WGS84 读取。");
            ui.add(
                egui::TextEdit::singleline(&mut self.osm_page.gpx_path)
                    .desired_width(ui.available_width())
                    .hint_text("GPX 文件路径"),
            );
            ui.horizontal_wrapped(|ui| {
                #[cfg(not(target_os = "android"))]
                if ui.button("选择 GPX").clicked() {
                    if let Some(p) = rfd::FileDialog::new()
                        .add_filter("GPX", &["gpx"])
                        .pick_file()
                    {
                        self.osm_page.gpx_path = p.display().to_string();
                    }
                }
                if ui.button("导入并分析").clicked() {
                    let result = (|| {
                        let meta = std::fs::metadata(&self.osm_page.gpx_path)
                            .map_err(|e| e.to_string())?;
                        if meta.len() > 24 * 1024 * 1024 {
                            return Err("GPX 超过 24 MiB".into());
                        }
                        std::fs::read_to_string(&self.osm_page.gpx_path).map_err(|e| e.to_string())
                    })();
                    match result {
                        Ok(text) => {
                            self.osm_page.gpx_text = Some(text);
                            self.analyze_gpx();
                        }
                        Err(e) => {
                            self.osm_page.gpx = None;
                            self.osm_page.gpx_text = None;
                            self.osm_page.status = format!("GPX 读取失败：{e}");
                        }
                    }
                }
                if ui.button("按当前围栏重新分析").clicked() {
                    self.analyze_gpx();
                }
            });
            if let Some(a) = &self.osm_page.gpx {
                ui.label(format!(
                    "实测距离 {:.3} km · {} 段",
                    a.distance_m / 1000.,
                    a.segments.len()
                ));
                ui.label(format!(
                    "段内累计用时 {} · 平均配速 {}",
                    a.duration_s
                        .map(|v| format!("{v:.1} 秒"))
                        .unwrap_or("未知".into()),
                    a.pace_min_km
                        .map(|v| format!("{v:.2} 分/公里"))
                        .unwrap_or("未知".into())
                ));
                ui.label(format!(
                    "点位覆盖 {}/{} · 越界采样点 {} · 越界连线 {}",
                    a.coverage.iter().filter(|v| **v).count(),
                    a.coverage.len(),
                    a.outside.len(),
                    a.crossing.len()
                ));
                ui.label(format!(
                    "未覆盖点位：{}",
                    a.coverage
                        .iter()
                        .enumerate()
                        .filter(|(_, v)| !**v)
                        .map(|(i, _)| (i + 1).to_string())
                        .collect::<Vec<_>>()
                        .join("、")
                ));
                for w in &a.warnings {
                    ui.label(w);
                }
                ui.label("下方地图：紫色为 GPX，红色为越界点/连线；覆盖按采样点及相邻连线估算。");
            }
        });
        if ui.button("一键导出脱敏诊断报告").clicked() {
            let (nodes, edges) = self
                .network
                .as_ref()
                .map(|n| (n.graph.node_count(), n.graph.edge_count()))
                .unwrap_or((0, 0));
            let mut value = crate::diagnostics::report(
                &self.status,
                &self.osm_page.status,
                !self.config.manual_road_path().is_empty(),
                self.osm_page.offline,
                nodes,
                edges,
                self.osm_page.cache_entries.len(),
            );
            if let Some(check) = &self.run_page.check {
                value["route_check"] = serde_json::json!({"components":check.components,"minimum_closed_distance_m":check.minimum_m,"issues":check.issues.iter().map(|i|serde_json::json!({"category":crate::diagnostics::category(&i.message),"point_number":i.point.map(|n|n+1)})).collect::<Vec<_>>()});
            }
            let path = crate::platform::data_dir().join(format!(
                "diagnostic-{}.json",
                crate::crypto::envelope::now_ms()
            ));
            match std::fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()) {
                Ok(()) => self.osm_page.status = format!("脱敏诊断已保存：{}", path.display()),
                Err(e) => self.osm_page.status = format!("诊断导出失败：{e}"),
            }
        }
    }
    fn open_cached_road(&mut self, key: &str) {
        let request_id = self.begin_network_load();
        self.osm_page.offline = true;
        self.osm_page.status = "正在后台读取离线缓存…".into();
        self.run_page.route_mode = crate::track::generate_road::RouteMode::Road;
        let anchor = self
            .identity
            .anchor_coordinate()
            .unwrap_or(crate::location::Coordinate::new(0., 0., 0.).unwrap());
        let tx = self.net_tx.clone();
        let key = key.to_string();
        let control = self.osm_page.cancel.clone();
        std::thread::spawn(move || {
            let mut inputs = None;
            let result = crate::api::school_roads::read_cache(&key).and_then(|(net, entry)| {
                control.check()?;
                inputs = Some(super::run::PreviewInputs {
                    uid: 0,
                    unid: entry.school,
                    anchor,
                    points: entry.points,
                    fences: entry.fences,
                    loaded_ms: entry.loaded_ms,
                });
                Ok(std::sync::Arc::new(net))
            });
            let _ = tx.send(super::osm::NetworkLoad {
                source: None,
                pending: false,
                warning: String::new(),
                offline: true,
                request_id,
                scope: None,
                campuses: Vec::new(),
                selected_key: String::new(),
                inputs,
                result,
            });
        });
    }
    pub(crate) fn analyze_gpx(&mut self) {
        let Some(text) = &self.osm_page.gpx_text else {
            return;
        };
        let (fences, points) = self
            .run_page
            .preview_inputs
            .as_ref()
            .map(|i| (i.fences.as_slice(), i.points.as_slice()))
            .unwrap_or((&[], &[]));
        match crate::track::analysis::analyze(text, fences, points) {
            Ok(a) => {
                self.osm_page.gpx = Some(a);
                self.osm_page.fitted = false;
            }
            Err(e) => {
                self.osm_page.gpx = None;
                self.osm_page.status = format!("GPX 分析失败：{e}");
            }
        }
    }
}
