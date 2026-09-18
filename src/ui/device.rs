//! 设备信息页：iOS/Android 单选 + 6 字段 + 整套随机 + 持久化。
//! ⚠ 设备 ID 固定复用（防 10121 风控）；随机后必须手动「保存」才生效。

use super::{mobile, theme, App};
use crate::crypto::header::HeaderIdentity;
use eframe::egui;

const IOS_OS_POOL: [&str; 5] = ["17.5.1", "18.0.1", "18.1", "18.2", "26.5.2"];
const ANDROID_MODEL_POOL: [&str; 8] = [
    "22081212C", "OPPO PGBM10", "Redmi K60", "HUAWEI Mate 40", "vivo V2309A",
    "Pixel 8", "SM-S9210", "OnePlus ACE3",
];
const ANDROID_OS_POOL: [&str; 4] = ["12", "13", "14", "15"];

#[derive(Default)]
pub struct DevicePage {
    saved_flash: f32,
}

impl App {
    #[cfg(target_os = "android")]
    pub(super) fn poll_device_info(&mut self) {
        let Some(reply) = crate::android::take_device_info() else { return };
        let info = match reply.result {
            Ok(Some(info)) => info,
            Ok(None) => {
                self.status = "未读取本机信息，可在设备信息页手动填写或再次读取".into();
                return;
            }
            Err(error) => { self.status = format!("× {error}"); return; }
        };
        let identity = if reply.initial { &self.identity } else { &self.device_buf };
        match crate::api::model::identity_with_device_info(identity, &info) {
            Ok(updated) => {
                self.device_buf = updated;
                if reply.initial {
                    match crate::api::model::save_identity(&self.device_buf) {
                        Ok(()) => {
                            self.identity = self.device_buf.clone();
                            crate::android::complete_device_info();
                            self.status = format!("已读取 {} {} / Android {}；UUID 保持不变",
                                info.manufacturer, info.model, info.os_version);
                        }
                        Err(error) => {
                            self.tab = 5;
                            self.status = format!("× 保存失败：{error}，请在设备页重新保存");
                        }
                    }
                } else {
                    self.status = format!("已读取 {} {}；点击保存后生效，UUID 保持不变", info.manufacturer, info.model);
                }
            }
            Err(error) => self.status = format!("× {error}"),
        }
    }

    pub fn draw_device(&mut self, ui: &mut egui::Ui) {
        let mut page = std::mem::take(&mut self.device_page);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| self.draw_device_inner(ui, &mut page));
        self.device_page = page;
    }

    fn draw_device_inner(&mut self, ui: &mut egui::Ui, page: &mut DevicePage) {
        ui.add_space(6.0);

        mobile::row(ui, |ui| {
            ui.radio_value(&mut self.device_buf.platform, "ios".to_string(), "iOS");
            ui.radio_value(&mut self.device_buf.platform, "android".to_string(), "Android");
            ui.separator();
            let platform = self.device_buf.platform.clone();
            if ui.button("随机生成").clicked() {
                randomize(&mut self.device_buf, &platform);
                self.status = "已生成随机设备（未保存——手动保存后才生效）".into();
            }
        });

        #[cfg(target_os = "android")]
        if ui.button("读取本机信息").clicked() {
            crate::android::request_device_info(false);
        }

        ui.add_space(6.0);
        let is_ios = self.device_buf.platform != "android";
        let id_label = if is_ios { "DeviceId（UUID 大写）" } else { "DeviceId（Android）" };
        let idfa_label = if is_ios { "IDFA（可空）" } else { "IMEI（可空）" };
        let name_label = if is_ios { "设备名" } else { "机型" };
        if mobile::compact_ui(ui) {
            ui.vertical(|ui| {
                ui.label(id_label);
                mobile::text_edit(ui, "device_id", &mut self.device_buf.device_id, crate::platform::InputKind::Text, ui.available_width());
                ui.label(idfa_label);
                mobile::text_edit(ui, "device_idfa", &mut self.device_buf.idfa, crate::platform::InputKind::Text, ui.available_width());
                ui.label("系统版本：");
                mobile::text_edit(ui, "device_os", &mut self.device_buf.os_version, crate::platform::InputKind::Text, ui.available_width());
                ui.label(name_label);
                mobile::text_edit(ui, "device_name", &mut self.device_buf.device_name, crate::platform::InputKind::Text, ui.available_width());
                if !is_ios {
                    ui.label("品牌（本地展示，可空）：");
                    mobile::text_edit(ui, "device_manufacturer", &mut self.device_buf.manufacturer, crate::platform::InputKind::Text, ui.available_width());
                }
                ui.label("城市：");
                mobile::text_edit(ui, "device_city", &mut self.device_buf.city, crate::platform::InputKind::Text, ui.available_width());
                ui.label("定位锚点纬度：");
                mobile::drag_f64(ui, "device_lat", &mut self.device_buf.anchor_lat, -90.0..=90.0, 0.00001, 6);
                ui.label("定位锚点经度：");
                mobile::drag_f64(ui, "device_lon", &mut self.device_buf.anchor_lon, -180.0..=180.0, 0.00001, 6);
            });
        } else {
            egui::Grid::new("device_grid")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label(id_label);
                    mobile::text_edit(ui, "device_id", &mut self.device_buf.device_id, crate::platform::InputKind::Text, 340.0);
                    ui.end_row();
                    ui.label(idfa_label);
                    mobile::text_edit(ui, "device_idfa", &mut self.device_buf.idfa, crate::platform::InputKind::Text, 340.0);
                    ui.end_row();
                    ui.label("系统版本：");
                    mobile::text_edit(ui, "device_os", &mut self.device_buf.os_version, crate::platform::InputKind::Text, 120.0);
                    ui.end_row();
                    ui.label(name_label);
                    mobile::text_edit(ui, "device_name", &mut self.device_buf.device_name, crate::platform::InputKind::Text, 200.0);
                    ui.end_row();
                    if !is_ios {
                        ui.label("品牌（本地展示，可空）：");
                        mobile::text_edit(ui, "device_manufacturer", &mut self.device_buf.manufacturer, crate::platform::InputKind::Text, 200.0);
                        ui.end_row();
                    }
                    ui.label("城市：");
                    mobile::text_edit(ui, "device_city", &mut self.device_buf.city, crate::platform::InputKind::Text, 120.0);
                    ui.end_row();
                    ui.label("定位锚点纬度：");
                    mobile::drag_f64(ui, "device_lat", &mut self.device_buf.anchor_lat, -90.0..=90.0, 0.00001, 6);
                    ui.end_row();
                    ui.label("定位锚点经度：");
                    mobile::drag_f64(ui, "device_lon", &mut self.device_buf.anchor_lon, -180.0..=180.0, 0.00001, 6);
                    ui.end_row();
                });
        }

        ui.add_space(8.0);
        mobile::row(ui, |ui| {
            if ui.add(theme::primary_btn("保存")).clicked() {
                match crate::api::model::save_identity(&self.device_buf) {
                    Ok(()) => {
                        self.identity = self.device_buf.clone();
                        self.status = "√ 设备身份已保存并生效".into();
                        self.log.push("√  已更新（新设备 ID 从下个请求开始生效）");
                        page.saved_flash = 2.0;
                    }
                    Err(e) => self.status = format!("× 保存失败：{e}"),
                }
            }
            if ui.button("撤销修改").clicked() {
                self.device_buf = self.identity.clone();
            }
            if page.saved_flash > 0.0 {
                ui.colored_label(theme::ok(), "已保存");
            }
        });

        ui.add_space(12.0);
        ui.separator();
        ui.colored_label(
            theme::warn(),
            "风控提示：设备 ID 参与服务端会话校验，固定复用可显著降低 10121（设备风险）概率；",
        );
        ui.colored_label(
            theme::warn(),
            "不要每次启动都换设备；「随机生成」只改界面缓冲区，手动「保存」后才写入  生效。",
        );
        ui.add_space(4.0);
        ui.label(format!(
            "当前生效身份：{} / {} / {} / 城市 {} / 锚点({:.6},{:.6})",
            if self.identity.platform == "android" { "Android" } else { "iOS" },
            self.identity.device_name,
            self.identity.os_version,
            self.identity.city,
            self.identity.anchor_lat,
            self.identity.anchor_lon,
        ));
        if page.saved_flash > 0.0 {
            page.saved_flash -= ui.ctx().input(|i| i.stable_dt);
        }
        if self.identity.platform == "android" && !self.identity.manufacturer.is_empty() {
            ui.label(format!("当前保存品牌：{}", self.identity.manufacturer));
        }
    }
}

/// 整套随机：uuid v4 设备 ID（大写）；机型/系统按平台池抽取。
fn randomize(buf: &mut HeaderIdentity, platform: &str) {
    buf.manufacturer.clear();
    buf.device_id = uuid::Uuid::new_v4().to_string().to_uppercase();
    buf.app_install_time = HeaderIdentity::fresh_install_time(platform);
    if platform == "android" {
        let i = (rand::random::<f64>() * ANDROID_MODEL_POOL.len() as f64) as usize
            % ANDROID_MODEL_POOL.len();
        let j =
            (rand::random::<f64>() * ANDROID_OS_POOL.len() as f64) as usize % ANDROID_OS_POOL.len();
        buf.device_name = ANDROID_MODEL_POOL[i].into();
        buf.os_version = ANDROID_OS_POOL[j].into();
        buf.idfa = String::new();
        buf.platform = "android".into();
    } else {
        let k = (rand::random::<f64>() * IOS_OS_POOL.len() as f64) as usize % IOS_OS_POOL.len();
        buf.device_name = "iPhone".into();
        buf.os_version = IOS_OS_POOL[k].into();
        buf.idfa = uuid::Uuid::new_v4().to_string().to_uppercase();
        buf.platform = "ios".into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_device_does_not_keep_a_previously_imported_brand() {
        for platform in ["android", "ios"] {
            let mut identity = HeaderIdentity {
                manufacturer: "Previously imported brand".into(),
                ..Default::default()
            };
            randomize(&mut identity, platform);
            assert!(identity.manufacturer.is_empty(), "random {platform} identity must not retain a different device's brand");
        }
    }
}
