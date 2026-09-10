//! 设备信息页：iOS/Android 单选 + 6 字段 + 整套随机 + 持久化。
//! ⚠ 设备 ID 固定复用（防 10121 风控）；随机后必须手动「保存」才生效。

use super::{theme, App};
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
    pub fn draw_device(&mut self, ui: &mut egui::Ui) {
        let mut page = std::mem::take(&mut self.device_page);
        self.draw_device_inner(ui, &mut page);
        self.device_page = page;
    }

    fn draw_device_inner(&mut self, ui: &mut egui::Ui, page: &mut DevicePage) {
        ui.add_space(6.0);

        let is_ios = self.device_buf.platform != "android";
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.device_buf.platform, "ios".to_string(), "iOS");
            ui.radio_value(&mut self.device_buf.platform, "android".to_string(), "Android");
            ui.separator();
            let platform = self.device_buf.platform.clone();
            if ui.button("随机生成").clicked() {
                randomize(&mut self.device_buf, &platform);
                self.status = "已生成随机设备（未保存——手动保存后才生效）".into();
            }
        });

        ui.add_space(6.0);
        egui::Grid::new("device_grid")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                let id_label = if is_ios { "DeviceId（UUID 大写）" } else { "DeviceId（Android）" };
                ui.label(id_label);
                ui.add(
                    egui::TextEdit::singleline(&mut self.device_buf.device_id).desired_width(340.0),
                );
                ui.end_row();

                ui.label(if is_ios { "IDFA（可空）" } else { "IMEI（可空）" });
                ui.add(egui::TextEdit::singleline(&mut self.device_buf.idfa).desired_width(340.0));
                ui.end_row();

                ui.label("系统版本：");
                ui.add(
                    egui::TextEdit::singleline(&mut self.device_buf.os_version).desired_width(120.0),
                );
                ui.end_row();

                ui.label(if is_ios { "设备名" } else { "机型" });
                ui.add(
                    egui::TextEdit::singleline(&mut self.device_buf.device_name).desired_width(200.0),
                );
                ui.end_row();

                ui.label("定位锚点纬度：");
                ui.add(
                    egui::DragValue::new(&mut self.device_buf.anchor_lat)
                        .speed(0.00001)
                        .fixed_decimals(6),
                );
                ui.end_row();

                ui.label("定位锚点经度：");
                ui.add(
                    egui::DragValue::new(&mut self.device_buf.anchor_lon)
                        .speed(0.00001)
                        .fixed_decimals(6),
                );
                ui.end_row();
            });

        ui.add_space(8.0);
        ui.horizontal(|ui| {
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
            "当前生效身份：{} / {} / {} / 锚点({:.6},{:.6})",
            if self.identity.platform == "android" { "Android" } else { "iOS" },
            self.identity.device_name,
            self.identity.os_version,
            self.identity.anchor_lat,
            self.identity.anchor_lon,
        ));
        if page.saved_flash > 0.0 {
            page.saved_flash -= ui.ctx().input(|i| i.stable_dt);
        }
    }
}

/// 整套随机：uuid v4 设备 ID（大写）；机型/系统按平台池抽取。
fn randomize(buf: &mut HeaderIdentity, platform: &str) {
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
