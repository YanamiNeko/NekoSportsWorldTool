//! Thin Android host: application storage, native IME editing, screen policy.

use android_activity::AndroidApp;
use jni::{objects::{JObject, JString, JValue}, sys::jlong, JNIEnv, JavaVM};
use std::{collections::HashMap, path::PathBuf, sync::{atomic::{AtomicBool, Ordering}, Mutex, OnceLock}};
use winit::platform::android::EventLoopBuilderExtAndroid;

static APP: Mutex<Option<AndroidApp>> = Mutex::new(None);
static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();
static CONTEXT: Mutex<Option<egui::Context>> = Mutex::new(None);
static EDITS: OnceLock<Mutex<HashMap<u64, String>>> = OnceLock::new();
static SCREEN_ON: AtomicBool = AtomicBool::new(false);
static SAFE_INSETS: Mutex<[i32; 4]> = Mutex::new([0; 4]);
static DEVICE_INFO: Mutex<Option<DeviceInfoReply>> = Mutex::new(None);

pub(crate) struct DeviceInfoReply {
    pub initial: bool,
    pub result: Result<Option<crate::api::model::DeviceInfo>, String>,
}

#[no_mangle]
fn android_main(app: AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info));
    let directory = app.internal_data_path().expect("Android did not provide an app data directory");
    std::fs::create_dir_all(&directory).expect("Cannot create Android app data directory");
    let _ = DATA_DIR.set(directory);
    *APP.lock().unwrap() = Some(app.clone());
    EDITS.get_or_init(Default::default).lock().unwrap().clear();
    *DEVICE_INFO.lock().unwrap() = None;
    SCREEN_ON.store(false, Ordering::Relaxed);
    let options = eframe::NativeOptions {
        run_and_return: false,
        event_loop_builder: Some(Box::new(move |builder| { builder.with_android_app(app); })),
        viewport: egui::ViewportBuilder::default().with_title("NekoSportsWorldTool"),
        ..Default::default()
    };
    if let Err(error) = crate::run_gui(options) {
        log::error!("GUI startup failed: {error}");
    }
    set_keep_screen_on(false);
    *CONTEXT.lock().unwrap() = None;
    *APP.lock().unwrap() = None;
}

pub(crate) fn data_dir() -> PathBuf {
    DATA_DIR.get().expect("Android storage must be initialized before use").clone()
}

pub(crate) fn set_context(context: &egui::Context) {
    *CONTEXT.lock().unwrap() = Some(context.clone());
}

pub(crate) fn safe_insets() -> [i32; 4] { *SAFE_INSETS.lock().unwrap() }

#[no_mangle]
pub extern "system" fn Java_org_nekosportsworld_tool_MainActivity_nativeSetInsets(
    _env: JNIEnv<'_>, _activity: JObject<'_>, left: i32, top: i32, right: i32, bottom: i32,
) {
    *SAFE_INSETS.lock().unwrap() = [left.max(0), top.max(0), right.max(0), bottom.max(0)];
    if let Some(context) = CONTEXT.lock().unwrap().as_ref() { context.request_repaint(); }
}

fn with_activity(action: impl FnOnce(&mut JNIEnv<'_>, &JObject<'_>) -> jni::errors::Result<()>) -> bool {
    let app = APP.lock().unwrap().clone();
    let Some(app) = app else { return false };
    // android-activity owns these VM/activity references for the lifetime of app.
    let result = (|| {
        let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }?;
        let mut env = vm.attach_current_thread()?;
        let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
        // android_main remains attached to the JVM for its lifetime; bound
        // temporary Java strings to this call rather than to the whole thread.
        let result = env.with_local_frame(16, |env| action(env, &activity));
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
        }
        result
    })();
    if let Err(error) = result {
        log::error!("Android UI bridge: {error}");
        return false;
    }
    true
}

pub(crate) fn edit_text(id: u64, value: &str, kind: crate::platform::InputKind) {
    let kind = match kind {
        crate::platform::InputKind::Text => 0,
        crate::platform::InputKind::Password => 1,
        crate::platform::InputKind::Integer => 2,
        crate::platform::InputKind::Decimal => 3,
    };
    with_activity(|env, activity| {
        let value = env.new_string(value)?;
        env.call_method(activity, "openEditor", "(JLjava/lang/String;I)V", &[
            JValue::Long(id as i64), JValue::Object(value.as_ref()), JValue::Int(kind),
        ])?;
        Ok(())
    });
}

pub(crate) fn take_edited_text(id: u64) -> Option<String> {
    EDITS.get_or_init(Default::default).lock().unwrap().remove(&id)
}

pub(crate) fn request_device_info(initial: bool) {
    with_activity(|env, activity| {
        env.call_method(activity, "requestDeviceInfo", "(Z)V", &[JValue::Bool(initial.into())])?;
        Ok(())
    });
}

pub(crate) fn take_device_info() -> Option<DeviceInfoReply> {
    DEVICE_INFO.lock().unwrap().take()
}

pub(crate) fn complete_device_info() {
    with_activity(|env, activity| {
        env.call_method(activity, "completeDeviceInfo", "()V", &[])?;
        Ok(())
    });
}

#[no_mangle]
pub extern "system" fn Java_org_nekosportsworld_tool_MainActivity_nativeDeviceInfo(
    mut env: JNIEnv<'_>, _activity: JObject<'_>, initial: jni::sys::jboolean,
    info: JString<'_>, error: JString<'_>,
) {
    let Ok(info) = env.get_string(&info) else { return };
    let info: String = info.into();
    let Ok(error) = env.get_string(&error) else { return };
    let error: String = error.into();
    let result = if !error.is_empty() { Err(error) }
        else if info.is_empty() { Ok(None) }
        else { serde_json::from_str(&info).map(Some).map_err(|_| "无法解析本机信息，请手动填写".into()) };
    *DEVICE_INFO.lock().unwrap() = Some(DeviceInfoReply { initial: initial != 0, result });
    if let Some(context) = CONTEXT.lock().unwrap().as_ref() { context.request_repaint(); }
}

pub(crate) fn copy_text(text: &str) {
    with_activity(|env, activity| {
        let text = env.new_string(text)?;
        env.call_method(activity, "copyText", "(Ljava/lang/String;)V", &[JValue::Object(text.as_ref())])?;
        Ok(())
    });
}

pub(crate) fn set_keep_screen_on(enabled: bool) {
    if SCREEN_ON.load(Ordering::Relaxed) == enabled { return; }
    if with_activity(|env, activity| {
        env.call_method(activity, "setTaskActive", "(Z)V", &[JValue::Bool(enabled.into())])?;
        Ok(())
    }) {
        SCREEN_ON.store(enabled, Ordering::Relaxed);
    }
}

#[no_mangle]
pub extern "system" fn Java_org_nekosportsworld_tool_MainActivity_nativeSubmitEdit(
    mut env: JNIEnv<'_>, _activity: JObject<'_>, id: jlong, value: JString<'_>,
) {
    let Ok(value) = env.get_string(&value) else { return };
    let value: String = value.into();
    EDITS.get_or_init(Default::default).lock().unwrap().insert(id as u64, value);
    if let Some(context) = CONTEXT.lock().unwrap().as_ref() { context.request_repaint(); }
}
