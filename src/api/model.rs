//! 数据模型与本地持久化。
//!
//! 持久化文件位于桌面 exe 目录或 Android 应用私有目录：identity.json（设备身份，device_id 固定复用
//! ——严禁每次随机，会触发 10121 风控）、session.json（登录态）、config.json
//! （账号/参数）、points_cache.json（点位缓存）。

pub use crate::crypto::header::HeaderIdentity;

pub const HOST: &str = "https://run.gxapp.iydsj.com";
/// 排行榜 / 违规名单域名（信封链与 RUN 相同）。
pub const DISCOVERY: &str = "https://discovery.gxapp.iydsj.com";

/// 登录态（session.json）。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Session {
    #[serde(default)]
    pub uid: i64,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub unid: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_weight")]
    pub weight: f64,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub device_id: String,
    /// 登录响应完整业务数据（我的页展示）
    #[serde(default)]
    pub profile: serde_json::Value,
}

fn default_weight() -> f64 {
    68.0
}

impl Session {
    pub fn is_logged_in(&self) -> bool {
        self.uid >= 1 && !self.token.is_empty()
    }
}

/// UI 参数（config.json）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Config {
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub remember: bool,
    #[serde(default = "default_f32")]
    pub dist_min: f32,
    #[serde(default = "default_f32")]
    pub dist_max: f32,
    #[serde(default = "default_pace_min")]
    pub pace_min: f32,
    #[serde(default = "default_pace_max")]
    pub pace_max: f32,
    #[serde(default)]
    pub face_check: bool,
    #[serde(default = "default_ai_minutes")]
    pub ai_minutes: i64,
    #[serde(default = "default_ai_reps")]
    pub ai_reps: i64,
}

fn default_f32() -> f32 {
    1.0
}
fn default_pace_min() -> f32 {
    360.0
}
fn default_pace_max() -> f32 {
    480.0
}
fn default_ai_minutes() -> i64 {
    1
}
fn default_ai_reps() -> i64 {
    5
}

impl Default for Config {
    fn default() -> Self {
        Self {
            username: String::new(),
            password: String::new(),
            remember: false,
            dist_min: 1.0,
            dist_max: 1.5,
            pace_min: default_pace_min(),
            pace_max: default_pace_max(),
            face_check: true,
            ai_minutes: default_ai_minutes(),
            ai_reps: default_ai_reps(),
        }
    }
}

// ── 持久化（平台数据目录）──────────────────────────────────────

fn exe_dir() -> std::path::PathBuf {
    crate::platform::data_dir()
}

fn read_json<T: serde::de::DeserializeOwned>(name: &str) -> Option<T> {
    let path = exe_dir().join(name);
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_json<T: serde::Serialize>(name: &str, value: &T) -> Result<(), String> {
    let path = exe_dir().join(name);
    let json =
        serde_json::to_string_pretty(value).map_err(|e| format!("序列化 {name} 失败: {e}"))?;
    std::fs::write(&path, json).map_err(|e| format!("写入 {} 失败: {e}", path.display()))
}

/// 加载设备身份；device_id / app_install_time 缺失时生成一次并立即落盘，
/// 此后同一设备全生命周期复用（逐请求漂移会影响设备一致性）。
pub fn load_identity() -> HeaderIdentity {
    load_identity_for_platform(if cfg!(target_os = "android") { "android" } else { "ios" })
}

fn load_identity_for_platform(platform: &str) -> HeaderIdentity {
    let mut id: HeaderIdentity = read_json("identity.json").unwrap_or_else(|| {
        let mut identity = HeaderIdentity::default();
        if platform == "android" {
            identity.platform = "android".into();
            // Generic fallback until the user opts in or fills the fields manually.
            identity.device_name = "Android".into();
            identity.os_version = "16".into();
        }
        identity
    });
    let mut dirty = false;
    if id.device_id.is_empty() {
        id.device_id = uuid::Uuid::new_v4().to_string().to_uppercase();
        dirty = true;
    }
    if id.app_install_time <= 0 {
        id.app_install_time =
            crate::crypto::header::HeaderIdentity::fresh_install_time(&id.platform);
        dirty = true;
    }
    if id.mac_address.is_empty() {
        id.mac_address = crate::crypto::header::random_mac();
        dirty = true;
    }
    if dirty {
        let _ = save_identity(&id);
    }
    id
}

#[cfg(any(target_os = "android", test))]
#[derive(Debug, serde::Deserialize)]
pub struct DeviceInfo {
    pub manufacturer: String,
    pub model: String,
    pub os_version: String,
}

#[cfg(any(target_os = "android", test))]
pub fn identity_with_device_info(identity: &HeaderIdentity, info: &DeviceInfo) -> Result<HeaderIdentity, String> {
    if info.model.trim().is_empty() || info.os_version.trim().is_empty() {
        return Err("未能读取完整的机型和系统版本，请手动填写".into());
    }
    let mut updated = identity.clone();
    updated.platform = "android".into();
    updated.device_name = info.model.trim().into();
    updated.manufacturer = info.manufacturer.trim().into();
    updated.os_version = info.os_version.trim().into();
    Ok(updated)
}

pub fn save_identity(id: &HeaderIdentity) -> Result<(), String> {
    write_json("identity.json", id)
}

/// AI 项目列表永久缓存：拉取成功即落盘，网络异常时兜底展示。
pub fn load_ai_sports() -> Option<Vec<crate::api::ai::AiSport>> {
    read_json("ai_sports.json")
}

pub fn save_ai_sports(list: &[crate::api::ai::AiSport]) -> Result<(), String> {
    write_json("ai_sports.json", &list.to_vec())
}

pub fn load_session() -> Session {
    read_json("session.json").unwrap_or_default()
}

pub fn save_session(s: &Session) -> Result<(), String> {
    write_json("session.json", s)
}

pub fn clear_session() {
    let _ = std::fs::remove_file(exe_dir().join("session.json"));
}

pub fn load_config() -> Config {
    read_json("config.json").unwrap_or_default()
}

pub fn save_config(c: &Config) -> Result<(), String> {
    write_json("config.json", c)
}

#[cfg(test)]
mod storage_tests {
    use super::*;

    #[test]
    fn android_identity_import_persists_brand_and_reuses_uuid() {
        let directory = std::env::temp_dir().join(format!("neko-identity-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = Some(directory.clone()));
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = None);
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory);
        let first = load_identity_for_platform("android");
        assert_eq!(first.platform, "android");
        assert_ne!(first.device_name, "iPhone");
        assert!(uuid::Uuid::parse_str(&first.device_id).is_ok());
        let again = load_identity_for_platform("android");
        assert_eq!(serde_json::to_value(&first).unwrap(), serde_json::to_value(&again).unwrap());

        let mut existing = first.clone();
        existing.platform = "ios".into();
        existing.device_name = "Existing manual name".into();
        existing.idfa = "manual-value".into();
        let mut legacy = serde_json::to_value(&existing).unwrap();
        legacy.as_object_mut().unwrap().remove("manufacturer");
        write_json("identity.json", &legacy).unwrap();
        let loaded = load_identity_for_platform("android");
        assert_eq!(serde_json::to_value(&loaded).unwrap(), serde_json::to_value(&existing).unwrap(),
            "opening an existing install must not overwrite a manual identity");

        let info: DeviceInfo = serde_json::from_str(
            r#"{"manufacturer":"Example","model":"Phone 16","os_version":"16"}"#,
        ).unwrap();
        assert_eq!(info.manufacturer, "Example");
        let imported = identity_with_device_info(&loaded, &info).unwrap();
        assert_eq!(imported.platform, "android");
        assert_eq!(imported.device_name, "Phone 16");
        assert_eq!(imported.os_version, "16");
        let mut expected = serde_json::to_value(&existing).unwrap();
        expected["platform"] = "android".into();
        expected["device_name"] = "Phone 16".into();
        expected["manufacturer"] = "Example".into();
        expected["os_version"] = "16".into();
        assert_eq!(serde_json::to_value(&imported).unwrap(), expected,
            "import must preserve UUID, MAC, manual IMEI/IDFA, install time and location");
        assert_eq!(load_identity().device_name, existing.device_name, "preview must not save implicitly");
        save_identity(&imported).unwrap();
        let stored: serde_json::Value = read_json("identity.json").unwrap();
        assert_eq!(stored.get("manufacturer").and_then(|value| value.as_str()), Some("Example"),
            "the consented brand must be written to identity.json");
        let reloaded = load_identity();
        assert_eq!(serde_json::to_value(&reloaded).unwrap(), serde_json::to_value(&imported).unwrap(),
            "brand and identity must survive a fresh load from disk");
        assert_eq!(reloaded.device_id, first.device_id);
        let (header, _) = crate::crypto::header::build_header_for(&reloaded, 0, "", Some(1_700_000_000_000));
        let header: serde_json::Value = serde_json::from_str(&header).unwrap();
        assert_eq!(header["DeviceId"], first.device_id);
        assert_eq!(header["deviceName"], "Phone 16", "brand must not be prepended to the protocol model field");
        assert_eq!(header["osVersion"], "16");
        assert!(header.get("manufacturer").is_none() && header.get("brand").is_none(),
            "persisting local brand metadata must not invent new protocol fields");
    }

    #[test]
    fn incomplete_native_device_info_cannot_replace_identity() {
        let identity = HeaderIdentity::default();
        for (model, os_version) in [("", "16"), ("Phone", " ")] {
            let info = DeviceInfo { manufacturer: String::new(), model: model.into(), os_version: os_version.into() };
            assert!(identity_with_device_info(&identity, &info).is_err());
        }
    }

    #[test]
    fn session_roundtrip_and_logout_use_private_data_directory() {
        let directory = std::env::temp_dir().join(format!("neko-storage-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = Some(directory.clone()));
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = None);
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory.clone());
        assert_eq!(exe_dir(), directory, "storage must use the injected app directory");
        let session = Session { uid: 123, token: "test-only-token".into(), ..Default::default() };
        save_session(&session).unwrap();
        assert!(directory.join("session.json").is_file());
        assert_eq!(load_session().uid, 123);
        assert_eq!(load_session().token, session.token);
        clear_session();
        assert!(!directory.join("session.json").exists());
        assert_eq!(load_session().uid, 0);
    }
}

/// 点位缓存：{ts_ms, points}，TTL 300s（服务端限流 10603：5 分钟 3 次）。
pub const POINTS_TTL_MS: i64 = 300_000;

pub fn load_points_cache() -> Option<(i64, Vec<serde_json::Value>)> {
    let v: serde_json::Value = read_json("points_cache.json")?;
    let ts = v.get("ts")?.as_i64()?;
    let pts = v.get("points")?.as_array()?.clone();
    Some((ts, pts))
}

pub fn save_points_cache(points: &[serde_json::Value]) -> Result<(), String> {
    let doc =
        serde_json::json!({ "ts": crate::crypto::envelope::now_ms(), "points": points });
    write_json("points_cache.json", &doc)
}
