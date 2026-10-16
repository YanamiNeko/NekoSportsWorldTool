use super::App;

impl App {
    pub(super) fn login_backup_controls(&mut self, ui: &mut egui::Ui) {
        let idle = !self.backup_busy
            && self.active_jobs.load(std::sync::atomic::Ordering::Acquire) == 0
            && !self.any_busy()
            && !self.update.checking
            && !self.update.downloading;
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    idle && self.session.is_some(),
                    egui::Button::new("导出登录 ZIP"),
                )
                .clicked()
            {
                let result = crate::login_archive::export(
                    self.session.as_ref().unwrap(),
                    &self.identity,
                    &self.config,
                );
                match result {
                    Ok(bytes) => self.export_login_zip(bytes),
                    Err(e) => self.status = e,
                }
            }
            if ui
                .add_enabled(idle, egui::Button::new("导入登录 ZIP"))
                .clicked()
            {
                self.import_login_zip();
            }
        });
        ui.small("ZIP 含登录凭证及已记住的密码，未加密；导入将替换当前账号和设备身份。");
    }

    fn export_login_zip(&mut self, bytes: Vec<u8>) {
        #[cfg(target_os = "android")]
        {
            self.backup_busy = crate::android::login_archive_picker(Some(&bytes));
            self.status = if self.backup_busy {
                "请选择 ZIP 保存位置"
            } else {
                "无法打开文件保存窗口"
            }
            .into();
        }
        #[cfg(not(target_os = "android"))]
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("ZIP", &["zip"])
            .set_file_name("neko-login.zip")
            .save_file()
        {
            self.status = match std::fs::write(path, bytes) {
                Ok(()) => "登录信息已导出".into(),
                Err(e) => format!("导出失败：{e}"),
            };
        }
    }

    fn import_login_zip(&mut self) {
        #[cfg(target_os = "android")]
        {
            self.backup_busy = crate::android::login_archive_picker(None);
            self.status = if self.backup_busy {
                "请选择登录 ZIP"
            } else {
                "无法打开文件选择窗口"
            }
            .into();
        }
        #[cfg(not(target_os = "android"))]
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("ZIP", &["zip"])
            .pick_file()
        {
            use std::io::Read;
            let result = (|| {
                let mut bytes = Vec::new();
                std::fs::File::open(path)?
                    .take(crate::login_archive::LIMIT + 1)
                    .read_to_end(&mut bytes)?;
                Ok::<_, std::io::Error>(bytes)
            })();
            match result {
                Ok(bytes) => {
                    if rfd::MessageDialog::new()
                        .set_title("导入登录信息")
                        .set_description("将替换当前账号、登录凭证和设备身份，其他设置保留。")
                        .set_buttons(rfd::MessageButtons::OkCancel)
                        .show()
                        == rfd::MessageDialogResult::Ok
                    {
                        self.apply_login_zip(&bytes);
                    }
                }
                Err(e) => self.status = format!("读取失败：{e}"),
            }
        }
    }

    pub(super) fn poll_login_backup(&mut self) {
        #[cfg(target_os = "android")]
        if let Some(result) = crate::android::take_login_archive() {
            self.backup_busy = false;
            match result {
                Ok(Some(bytes)) => self.apply_login_zip(&bytes),
                Ok(None) => self.status = "登录信息已导出".into(),
                Err(error) => self.status = error,
            }
        }
    }

    fn apply_login_zip(&mut self, bytes: &[u8]) {
        if self.active_jobs.load(std::sync::atomic::Ordering::Acquire) != 0
            || self.any_busy()
            || self.update.checking
            || self.update.downloading
        {
            self.status = "请等待当前任务结束后重新导入".into();
            return;
        }
        match crate::login_archive::import(bytes, &self.config) {
            Ok((b, config)) => {
                // Discard replies from the old account before refreshing the new one.
                (self.tx, self.rx) = std::sync::mpsc::channel();
                #[cfg(target_os = "android")]
                {
                    crate::android::complete_device_info();
                    let _ = crate::android::take_device_info();
                    let _ = crate::android::take_location();
                }
                self.identity = b.identity;
                self.device_buf = self.identity.clone();
                self.device_page = Default::default();
                self.log = Default::default();
                self.popup = None;
                self.ai_confirm = None;
                self.session = Some(b.session);
                self.config = config;
                self.username = self.config.username.clone();
                self.password = self.config.password.clone();
                self.remember = self.config.remember;
                self.records_page = Default::default();
                self.data_page = Default::default();
                self.user_page = Default::default();
                self.ai_page = super::ai::AiPage {
                    days: 1,
                    per_day: 1,
                    ..Default::default()
                };
                self.run_page.plan = None;
                self.run_page.preview = None;
                self.run_page.preview_stale = true;
                self.records_busy = false;
                self.data_busy = false;
                self.user_busy = false;
                self.refresh_data_page();
                self.refresh_records();
                self.refresh_user_page();
                self.refresh_ai_list();
                self.refresh_fence();
                self.refresh_points();
                self.status = "登录信息已导入；凭证是否有效以服务器校验为准".into();
            }
            Err(e) => self.status = format!("导入失败：{e}"),
        }
    }
}
