//! Public map lookup stays independent of school policy/points and road loading.
use super::App;
use crate::api::map_boundary;
use route_planner::campus::Campus;
use std::sync::{mpsc, Arc};

pub(crate) struct LookupResult {
    pub id: u64,
    pub school: String,
    pub result: Result<Vec<Campus>, String>,
}

pub(crate) struct LookupUi {
    pub name: String,
    pub school: String,
    pub status: String,
    pub busy: bool,
    pub id: u64,
    pub items: Vec<Campus>,
    pub cancel: Arc<crate::task_control::Control>,
    pub tx: mpsc::Sender<LookupResult>,
    pub rx: mpsc::Receiver<LookupResult>,
}
impl Default for LookupUi {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            name: String::new(),
            school: String::new(),
            status: String::new(),
            busy: false,
            id: 0,
            items: vec![],
            cancel: Arc::default(),
            tx,
            rx,
        }
    }
}

impl App {
    pub(crate) fn poll_map_lookup(&mut self) {
        while let Ok(reply) = self.osm_page.lookup.rx.try_recv() {
            let lookup = &mut self.osm_page.lookup;
            if reply.id != lookup.id {
                continue;
            }
            lookup.busy = false;
            if !self
                .session
                .as_ref()
                .is_some_and(|s| s.unid == reply.school)
                || lookup.cancel.check().is_err()
            {
                lookup.items.clear();
                continue;
            }
            match reply.result {
                Ok(items) => {
                    lookup.status = if items.is_empty() {
                        "查询完成，未找到完整校园区域。可尝试学校全称、旧称或不带校区后缀的名称。"
                            .into()
                    } else {
                        format!(
                            "找到 {} 个校园区域，请核对下方轮廓后确认；定位锚点不会自动改变。",
                            items.len()
                        )
                    };
                    lookup.items = items;
                }
                Err(e) => {
                    lookup.items.clear();
                    lookup.status =
                        format!("地图查询失败：{e}。请重试，不能据此判断学校没有边界。");
                }
            }
        }
    }

    pub(crate) fn search_map_school(&mut self) {
        let Some(session) = &self.session else {
            return;
        };
        let name = self.osm_page.lookup.name.trim().to_owned();
        if let Err(e) = map_boundary::name_query(&name) {
            self.osm_page.lookup.status = e;
            return;
        }
        let mut config = self.config.clone();
        config.map_search_name = Some(map_boundary::SearchName {
            school: session.unid.clone(),
            name: name.clone(),
        });
        if let Err(e) = crate::api::model::save_config(&config) {
            self.osm_page.lookup.status = format!("学校名称未保存：{e}");
            return;
        }
        self.config = config;
        let endpoint = self.config.map_search_endpoint.clone();
        let lookup = &mut self.osm_page.lookup;
        lookup.cancel.cancel();
        lookup.cancel = Arc::default();
        lookup.id = lookup.id.wrapping_add(1);
        lookup.school = session.unid.clone();
        lookup.items.clear();
        lookup.busy = true;
        lookup.status = "正在按学校名称查询地图（不受定位 5 公里范围限制）…".into();
        let (tx, id, school, control) = (
            lookup.tx.clone(),
            lookup.id,
            lookup.school.clone(),
            lookup.cancel.clone(),
        );
        #[cfg(test)]
        let data_dir = crate::platform::TEST_DATA_DIR.with(|p| p.borrow().clone());
        std::thread::spawn(move || {
            #[cfg(test)]
            crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = data_dir);
            let result = map_boundary::lookup_user_requested(&school, &name, &endpoint, &control);
            let _ = tx.send(LookupResult { id, school, result });
        });
    }

    pub(crate) fn draw_school_lookup(&mut self, ui: &mut egui::Ui) {
        let Some(session) = &self.session else {
            return;
        };
        let lookup = &mut self.osm_page.lookup;
        if lookup.school != session.unid {
            lookup.cancel.cancel();
            lookup.id = lookup.id.wrapping_add(1);
            lookup.busy = false;
            lookup.items.clear();
            lookup.status.clear();
            lookup.school = session.unid.clone();
            lookup.name = map_boundary::school_name(session, &self.config);
        }
        ui.label("地图学校名称（可改为全称或旧称）：");
        ui.label("仅点击时查询；主服务失败时尝试备用搜索。校园轮廓仍需核对确认。");
        ui.add(
            egui::TextEdit::singleline(&mut lookup.name)
                .desired_width(ui.available_width())
                .hint_text("例如：肇庆医学院"),
        );
        let busy = lookup.busy;
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !busy && !self.osm_page.busy,
                    egui::Button::new("按校名查找校园边界"),
                )
                .clicked()
            {
                self.search_map_school();
            }
            if busy && ui.button("取消校园查询").clicked() {
                let lookup = &mut self.osm_page.lookup;
                lookup.cancel.cancel();
                lookup.id = lookup.id.wrapping_add(1);
                lookup.busy = false;
                lookup.status = "已取消接收校园查询结果".into();
            }
        });
        if !self.osm_page.lookup.status.is_empty() {
            ui.label(&self.osm_page.lookup.status);
        }
    }
}
