//! School selection, offline maps, diagnostics and local GPX analysis controls.
use super::App;
use eframe::egui;
impl App {
    pub(crate) fn draw_road_tools(&mut self, ui: &mut egui::Ui) {
        self.draw_map_boundary_choice(ui);
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
                            Ok(()) => {
                                if self.config.offline_preview.as_ref().is_some_and(|s| s.key == entry.key)
                                    || self.osm_page.pending_offline_key.as_deref() == Some(entry.key.as_str())
                                {
                                    self.begin_network_load();
                                    self.osm_page.busy = false;
                                    self.osm_page.status = "已清除当前离线缓存，请重新选择路网".into();
                                }
                                self.osm_page.cache_entries.retain(|c| c.key != entry.key);
                            },
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
                !self.config.osm_path.is_empty(),
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
    pub(crate) fn forget_offline_preview(&mut self) {
        if self.config.offline_preview.take().is_some() {
            if let Err(e) = crate::api::model::save_config(&self.config) {
                self.log.push(&format!("[osm] 清除离线恢复选择失败：{e}"));
            }
        }
    }

    /// Shared by real startup and restart regression tests. Failed offline
    /// restoration must not silently switch to another map or campus.
    pub(crate) fn restore_route_on_startup(&mut self) {
        if let Some(saved) = self.config.offline_preview.clone() {
            if self.run_page.route_mode != crate::track::generate_road::RouteMode::Road
                || self.session.as_ref().is_some_and(|s| s.unid != saved.school)
                || self.identity.anchor_lat != saved.anchor_lat
                || self.identity.anchor_lon != saved.anchor_lon
            {
                self.forget_offline_preview();
                self.osm_page.status = "学校、定位或路线模式已变化，未恢复上次离线预览；请重新选择路网".into();
                return;
            }
            self.open_cached_road(&saved.key);
        } else if !self.config.osm_path.is_empty() {
            self.load_osm_async(self.config.osm_path.clone());
        } else if self.session.is_some() {
            self.refresh_points();
        }
    }

    pub(crate) fn open_cached_road(&mut self, key: &str) {
        // Keep the last successful selection on disk while reading. Closing
        // during startup must not erase it; failure explicitly clears it.
        let request_id = self.reset_network_load();
        self.osm_page.pending_offline_key = Some(key.to_string());
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
        #[cfg(test)]
        let data_dir = crate::platform::TEST_DATA_DIR.with(|p| p.borrow().clone());
        std::thread::spawn(move || {
            #[cfg(test)]
            crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = data_dir);
            let mut inputs = None;
            let result = crate::api::school_roads::read_cache(&key).and_then(|(net, entry)| {
                control.check()?;
                inputs = Some(super::run::PreviewInputs {
                    boundary_label: "离线缓存范围".into(),
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

impl App {
    fn draw_map_boundary_choice(&mut self, ui: &mut egui::Ui) {
        let Some(session) = self.session.clone() else { return; };
        self.draw_school_lookup(ui);
        let mut items = self.network.as_ref().map(|n| n.campuses.clone()).unwrap_or_default();
        if self.osm_page.lookup.school == session.unid {
            for c in &self.osm_page.lookup.items {
                items.retain(|old| old.id != c.id);
                items.push(c.clone());
            }
        }
        let official = self.run_page.preview_inputs.as_ref().is_some_and(|i| i.boundary_label == "官方围栏");
        if official { ui.label("当前使用官方围栏，地图校园边界不覆盖官方范围。"); return; }
        if let Some(choice) = self.config.map_boundary.as_ref().filter(|s|s.school==session.unid) {
            let valid = crate::api::map_boundary::selected(&items,Some(choice),&session.unid).is_some();
            ui.label(if valid { "已启用确认过的地图校园边界（本地规划使用）" } else { "已确认的地图边界不可用或变化，需重新确认后正式运行" });
            if ui.add_enabled(!self.osm_page.busy,egui::Button::new("取消地图边界，使用无边界路线")).clicked() {
                self.save_map_boundary_choice(None);
            }
        }
        if items.is_empty() { ui.label("尚未取得可选校园区域，可在上方按校名查询；校园范围未校验。"); return; }
        ui.label("地图校园边界：请选择并核对轮廓，确认后才限制路线范围。");
        if !items.iter().any(|c|c.id==self.osm_page.map_choice) {
            self.osm_page.map_choice = crate::api::map_boundary::selected(&items,self.config.map_boundary.as_ref(),&session.unid)
                .unwrap_or(&items[0]).id.clone();
        }
        let selected = items.iter().find(|c|c.id==self.osm_page.map_choice).unwrap();
        egui::ComboBox::from_id_salt("map_campus_choice")
            .selected_text(format!("{} ({})",selected.name,selected.id))
            .width(ui.available_width().min(300.))
            .show_ui(ui,|ui| { for c in &items { ui.selectable_value(&mut self.osm_page.map_choice,c.id.clone(),format!("{} ({})",c.name,c.id)); } });
        let c=items.iter().find(|c|c.id==self.osm_page.map_choice).unwrap();
        let (response,painter)=ui.allocate_painter(egui::vec2(ui.available_width(),180.),egui::Sense::hover());
        let mut map=super::map::MapState::default();
        let bounds:Vec<_>=c.outer.iter().flatten().map(|p|(p.lat,p.lon)).collect();
        map.fit(response.rect,&bounds);
        painter.rect_filled(response.rect,0.,egui::Color32::from_rgb(245,248,246));
        if let Some(net)=&self.network { map.draw_network(&painter,response.rect,net); }
        for ring in &c.outer { map.draw_polygon(&painter,response.rect,&ring.iter().map(|p|(p.lat,p.lon)).collect::<Vec<_>>(),egui::Color32::from_rgb(0,135,110),2.); }
        for ring in &c.inner { map.draw_polygon(&painter,response.rect,&ring.iter().map(|p|(p.lat,p.lon)).collect::<Vec<_>>(),egui::Color32::RED,2.); }
        ui.label("绿色为校园外轮廓，红色为排除区域；地图边界不代表服务端达标规则。");
        if ui.add_enabled(!self.osm_page.busy,egui::Button::new("确认使用此校园边界")).clicked() {
            self.save_map_boundary_choice(Some(crate::api::map_boundary::Selection{school:session.unid,id:c.id.clone(),revision:crate::api::map_boundary::revision(c)}));
        }
    }
}

impl App {
    pub(crate) fn save_map_boundary_choice(&mut self, selection:Option<crate::api::map_boundary::Selection>) {
        let mut config=self.config.clone();
        config.map_boundary=selection;
        if let Err(e)=crate::api::model::save_config(&config) {
            self.osm_page.status=format!("校园边界选择未保存，未应用：{e}");
            return;
        }
        self.config=config;
        self.refresh_school_route(false);
    }
}
