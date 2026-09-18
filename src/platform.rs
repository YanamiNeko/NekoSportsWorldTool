//! Platform services shared by the existing UI and the Android host.

#[derive(Clone, Copy, Debug)]
pub enum InputKind {
    Text,
    Password,
    Integer,
    Decimal,
}

pub fn data_dir() -> std::path::PathBuf {
    #[cfg(test)]
    if let Some(path) = TEST_DATA_DIR.with(|p| p.borrow().clone()) {
        return path;
    }
    #[cfg(target_os = "android")]
    {
        crate::android::data_dir()
    }
    #[cfg(not(target_os = "android"))]
    {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| std::path::PathBuf::from("."))
    }
}

pub fn edit_text(id: u64, value: &str, kind: InputKind) {
    #[cfg(target_os = "android")]
    crate::android::edit_text(id, value, kind);
    #[cfg(not(target_os = "android"))]
    let _ = (id, value, kind);
}

pub fn take_edited_text(id: u64) -> Option<String> {
    #[cfg(target_os = "android")]
    { crate::android::take_edited_text(id) }
    #[cfg(not(target_os = "android"))]
    { let _ = id; None }
}

pub fn set_keep_screen_on(enabled: bool) {
    #[cfg(target_os = "android")]
    crate::android::set_keep_screen_on(enabled);
    #[cfg(not(target_os = "android"))]
    let _ = enabled;
}

#[cfg(feature = "gui")]
pub fn sync_clipboard(context: &egui::Context) {
    #[cfg(target_os = "android")]
    {
        let text = context.output(|output| output.copied_text.clone());
        if !text.is_empty() { crate::android::copy_text(&text); }
    }
    #[cfg(not(target_os = "android"))]
    let _ = context;
}

#[cfg(feature = "gui")]
pub fn apply_safe_area(context: &egui::Context, input: &mut egui::RawInput) {
    #[cfg(target_os = "android")]
    if let Some(rect) = input.screen_rect.as_mut() {
        let [left, top, right, bottom] = crate::android::safe_insets();
        let scale = context.pixels_per_point();
        rect.min.x += left as f32 / scale;
        rect.min.y += top as f32 / scale;
        rect.max.x -= right as f32 / scale;
        rect.max.y -= bottom as f32 / scale;
    }
    #[cfg(not(target_os = "android"))]
    let _ = (context, input);
}

#[cfg(test)]
thread_local! {
    pub(crate) static TEST_DATA_DIR: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) };
}
