//! Shared desktop and Android application.

mod api;
mod login_archive;
pub mod cli;
mod crypto;
pub mod location;
pub mod platform;
pub mod template;
mod relaunch;
mod textlog;
mod track;
mod update;

#[cfg(feature = "gui")]
mod ui;

#[cfg(all(target_os = "android", feature = "android"))]
mod android;

#[cfg(all(target_os = "windows", feature = "gui"))]
#[path = "windows_location.rs"]
mod desktop_location;

#[cfg(all(target_os = "linux", feature = "gui"))]
#[path = "linux_location.rs"]
mod desktop_location;

#[cfg(all(target_os = "macos", feature = "gui"))]
#[path = "macos_location.rs"]
mod desktop_location;

#[cfg(feature = "gui")]
pub fn run_gui(options: eframe::NativeOptions) -> eframe::Result<()> {
    eframe::run_native(
        "NekoSportsWorldTool",
        options,
        Box::new(|cc| {
            #[cfg(target_os = "android")]
            android::set_context(&cc.egui_ctx);
            Ok(Box::new(ui::App::new(cc)))
        }),
    )
}
