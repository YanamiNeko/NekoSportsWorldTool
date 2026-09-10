//! 请求头明文构造。
//! 固定键序紧凑 JSON；appInstallTime/appUpdateTime 从设备身份读取，
//! 同一 DeviceId 全生命周期稳定（逐请求漂移会影响设备一致性）。

use super::envelope::{md5_hex, now_ms};

/// tokenSign 固定后缀（与 Android 签名盐同源）。
pub const TOKEN_SIGN_SUFFIX: &str = "2slhe02lsfiwowlcixisla_sls-_slaor";

pub const IOS_APP_VERSION: &str = "7.3.40";
pub const ANDROID_APP_VERSION: &str = "7.3.70";

pub const UA_IOS: &str = "SWCampus/7.3.40 (iPhone; iOS 18.1; Scale/3.00)";
pub const UA_ANDROID: &str = "Mozilla/5.0 (Linux; Android 14; 22081212C Build/UKQ1.231003.002) AppleWebKit/537.36 SWCampus/7.3.70";

/// 设备身份。app_install_time <= 0 表示尚未生成。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HeaderIdentity {
    #[serde(default = "default_platform")]
    pub platform: String, // "ios" | "android"
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub idfa: String,
    #[serde(default = "default_os_version")]
    pub os_version: String,
    #[serde(default = "default_device_name")]
    pub device_name: String,
    #[serde(default = "default_anchor_lat")]
    pub anchor_lat: f64,
    #[serde(default = "default_anchor_lon")]
    pub anchor_lon: f64,
    #[serde(default)]
    pub app_install_time: i64,
    /// 设备 MAC（首次生成后持久化，避免写死指纹）
    #[serde(default)]
    pub mac_address: String,
    /// 提交 body 的城市名
    #[serde(default = "default_city")]
    pub city: String,
}

pub fn random_mac() -> String {
    // 本地管理位随机单播 MAC
    let mut rng = rand::thread_rng();
    let octets: [u8; 3] = [0x08, 0xa4, rand::Rng::gen(&mut rng)];
    let tail: [u8; 3] = rand::Rng::gen(&mut rng);
    format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        octets[0], octets[1], octets[2], tail[0], tail[1], tail[2]
    )
}

fn default_platform() -> String { "ios".into() }
fn default_os_version() -> String { "26.5.2".into() }
fn default_device_name() -> String { "iPhone".into() }
fn default_anchor_lat() -> f64 { 38.901678 }
fn default_anchor_lon() -> f64 { 121.540241 }
fn default_city() -> String { "大连市".into() }

impl HeaderIdentity {
    /// 安装时间：持久值优先；缺失时按平台惯例回退（iOS 3 天 / Android 90 天前）。
    pub fn install_time(&self, ts: i64) -> i64 {
        if self.app_install_time > 0 {
            self.app_install_time
        } else if self.platform == "android" {
            ts - 90 * 86_400_000
        } else {
            ts - 3 * 86_400_000
        }
    }

    /// 按平台生成一份新的安装时间（设备随机化时调用）。
    pub fn fresh_install_time(platform: &str) -> i64 {
        let days = if platform == "android" { 90 } else { 3 };
        now_ms() - days * 86_400_000
    }
}

impl Default for HeaderIdentity {
    fn default() -> Self {
        Self {
            platform: default_platform(),
            device_id: String::new(),
            idfa: String::new(),
            os_version: default_os_version(),
            device_name: default_device_name(),
            anchor_lat: default_anchor_lat(),
            anchor_lon: default_anchor_lon(),
            app_install_time: 0,
            mac_address: String::new(),
            city: default_city(),
        }
    }
}

/// SWEncryption.signObject:suffix: 的 tokenSign。
/// SWJSON.sortedQueryString 按 key 排序：`timeStamp=<ts>&token=<token>&uid=<uid>`。
pub fn native_token_sign(uid: i64, token: &str, timestamp_ms: i64) -> String {
    let query = format!("timeStamp={}&token={}&uid={}", timestamp_ms, token, uid);
    md5_hex(format!("{}{}", query, TOKEN_SIGN_SUFFIX).as_bytes())
}

fn insert_uuid(map: &mut serde_json::Map<String, serde_json::Value>) -> String {
    let u = uuid::Uuid::new_v4().to_string().to_uppercase();
    map.insert("nonce".into(), serde_json::Value::String(u.clone()));
    u
}

/// iOS 请求头明文（SWRequestHeaderProvider._fillFullHeaders 字段序）。
/// 返回 (紧凑 JSON, HTTP 附加头字段: nonce/tokenSign/timeStamp)。
pub fn build_native_ios_header(
    identity: &HeaderIdentity,
    uid: i64,
    token: &str,
    timestamp_ms: Option<i64>,
) -> (String, Vec<(String, String)>) {
    let mut m = serde_json::Map::new();
    let device_id = if identity.device_id.is_empty() {
        uuid::Uuid::new_v4().to_string().to_uppercase()
    } else {
        identity.device_id.clone()
    };
    let ts = timestamp_ms.unwrap_or_else(now_ms);
    let install = identity.install_time(ts);

    m.insert("osType".into(), serde_json::Value::String("1".into()));
    m.insert("DeviceId".into(), serde_json::Value::String(device_id.clone()));
    m.insert("deviceName".into(), serde_json::Value::String(identity.device_name.clone()));
    m.insert(
        "CustomDeviceId".into(),
        serde_json::Value::String(format!("{device_id}_iOS_sportsWorld_campus")),
    );
    m.insert("osVersion".into(), serde_json::Value::String(identity.os_version.clone()));
    if !identity.idfa.is_empty() {
        m.insert("IDFA".into(), serde_json::Value::String(identity.idfa.clone()));
    }
    m.insert("logicPixel".into(), serde_json::Value::String("360x640".into()));
    m.insert("physicPixel".into(), serde_json::Value::String("1080x1920".into()));
    m.insert("cpuModel".into(), serde_json::Value::String("x86_64".into()));
    m.insert("appVersion".into(), serde_json::Value::String(IOS_APP_VERSION.into()));
    m.insert("isRoot".into(), serde_json::Value::Bool(false));
    m.insert("appInstallTime".into(), serde_json::Value::from(install));
    let nonce = insert_uuid(&mut m);
    if uid >= 1 {
        m.insert("uid".into(), serde_json::Value::from(uid));
    }
    if !token.is_empty() {
        m.insert("token".into(), serde_json::Value::String(token.to_string()));
    }
    m.insert("timeStamp".into(), serde_json::Value::from(ts));
    m.insert("studentId".into(), serde_json::Value::from(if uid >= 1 { uid } else { 0 }));
    if uid >= 1 && !token.is_empty() {
        m.insert(
            "tokenSign".into(),
            serde_json::Value::String(native_token_sign(uid, token, ts)),
        );
    }
    let extra = vec![
        ("nonce".to_string(), nonce),
        ("timeStamp".to_string(), ts.to_string()),
        (
            "tokenSign".to_string(),
            native_token_sign(uid, token, ts),
        ),
    ];
    (serde_json::to_string(&m).unwrap_or_default(), extra)
}

/// Android 399 请求头明文（NativeNetworkCommonHeaders.build 键集，osType="0"）。
pub fn build_android_header(
    identity: &HeaderIdentity,
    uid: i64,
    token: &str,
    timestamp_ms: Option<i64>,
) -> (String, Vec<(String, String)>) {
    let mut m = serde_json::Map::new();
    let device_id = if identity.device_id.is_empty() {
        uuid::Uuid::new_v4().to_string().to_uppercase()
    } else {
        identity.device_id.clone()
    };
    let ts = timestamp_ms.unwrap_or_else(now_ms);
    let install = identity.install_time(ts);

    m.insert("Accept".into(), serde_json::Value::String("application/json".into()));
    m.insert("Content-Type".into(), serde_json::Value::String("application/json".into()));
    m.insert("appVersion".into(), serde_json::Value::String(ANDROID_APP_VERSION.into()));
    m.insert("osType".into(), serde_json::Value::String("0".into()));
    m.insert("DeviceId".into(), serde_json::Value::String(device_id.clone()));
    m.insert("osVersion".into(), serde_json::Value::String(identity.os_version.clone()));
    m.insert("deviceName".into(), serde_json::Value::String(identity.device_name.clone()));
    m.insert("IMEI".into(), serde_json::Value::String(String::new()));
    m.insert("logicPixel".into(), serde_json::Value::String("1080x2400".into()));
    m.insert("physicPixel".into(), serde_json::Value::String("1080x2400".into()));
    m.insert("androidId".into(), serde_json::Value::String(String::new()));
    m.insert("blMac".into(), serde_json::Value::String(String::new()));
    m.insert("wifiMac".into(), serde_json::Value::String(String::new()));
    m.insert("cpuModel".into(), serde_json::Value::String("arm64-v8a".into()));
    m.insert("isRoot".into(), serde_json::Value::Bool(false));
    m.insert("appUpdateTime".into(), serde_json::Value::from(install));
    m.insert("appInstallTime".into(), serde_json::Value::from(install));
    let nonce = insert_uuid(&mut m);
    m.insert("timeStamp".into(), serde_json::Value::from(ts));
    m.insert(
        "CustomDeviceId".into(),
        serde_json::Value::String(format!("{device_id}_android_sportsWorld_campus")),
    );
    m.insert("uid".into(), serde_json::Value::from(uid));
    m.insert("token".into(), serde_json::Value::String(token.to_string()));
    m.insert("studentId".into(), serde_json::Value::from(uid));
    let sign = native_token_sign(uid, token, ts);
    m.insert("tokenSign".into(), serde_json::Value::String(sign.clone()));

    let extra = vec![
        ("nonce".to_string(), nonce),
        ("timeStamp".to_string(), ts.to_string()),
        ("tokenSign".to_string(), sign),
    ];
    (serde_json::to_string(&m).unwrap_or_default(), extra)
}

/// 按 identity 平台选择 header 构造器。
pub fn build_header_for(
    identity: &HeaderIdentity,
    uid: i64,
    token: &str,
    timestamp_ms: Option<i64>,
) -> (String, Vec<(String, String)>) {
    if identity.platform == "android" {
        build_android_header(identity, uid, token, timestamp_ms)
    } else {
        build_native_ios_header(identity, uid, token, timestamp_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 实测向量。
    #[test]
    fn test_token_sign_vector() {
        assert_eq!(
            native_token_sign(13056447, "TOKENABC", 1788958186123),
            "4a8d163186d91ac7b539a7bb07d457ba"
        );
    }

    /// iOS 头键序与条件字段（缺省字段跳过）。
    #[test]
    fn test_ios_header_shape() {
        let identity = HeaderIdentity {
            device_id: "0FA3EA16-46F0-4E44-9A97-1B63F0D9B11A".into(),
            idfa: String::new(),
            ..Default::default()
        };
        let (json, _) = build_native_ios_header(&identity, -1, "", Some(1788958186123));
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let keys: Vec<&str> = v.as_object().unwrap().keys().map(|s| s.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "osType", "DeviceId", "deviceName", "CustomDeviceId", "osVersion",
                "logicPixel", "physicPixel", "cpuModel", "appVersion", "isRoot",
                "appInstallTime", "nonce", "timeStamp", "studentId"
            ]
        );
        assert_eq!(v["osType"], "1");
        assert_eq!(v["studentId"], 0);
        // 带 uid/token 时出现 tokenSign
        let (json2, _) = build_native_ios_header(&identity, 13056447, "TOKEN", Some(1788958186123));
        let v2: serde_json::Value = serde_json::from_str(&json2).unwrap();
        assert!(v2.get("tokenSign").is_some());
        assert_eq!(v2["uid"], 13056447);
        assert_eq!(v2["tokenSign"], native_token_sign(13056447, "TOKEN", 1788958186123));
    }

    /// Android 头键集 + osType="0"。
    #[test]
    fn test_android_header_shape() {
        let identity = HeaderIdentity {
            platform: "android".into(),
            device_id: "D0FA3EA16-46F0-4E44-9A97-1B63F0D9B11A".into(),
            os_version: "14".into(),
            device_name: "22081212C".into(),
            ..Default::default()
        };
        let (json, _) = build_android_header(&identity, 42, "TK", Some(1788958186123));
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["osType"], "0");
        assert_eq!(v["appVersion"], "7.3.70");
        assert_eq!(v["studentId"], 42);
        assert!(v.get("tokenSign").is_some());
        assert!(json.starts_with("{\"Accept\":\"application/json\""));
    }

    /// 同一身份的 appInstallTime 跨请求零漂移（设备一致性防护）。
    #[test]
    fn test_install_time_stable_across_requests() {
        let identity = HeaderIdentity { app_install_time: 1_700_000_000_000_i64, ..Default::default() };
        let (j1, _) = build_native_ios_header(&identity, 1, "T", Some(1788958186123));
        let (j2, _) = build_native_ios_header(&identity, 1, "T", Some(1788958187000));
        let v1: serde_json::Value = serde_json::from_str(&j1).unwrap();
        let v2: serde_json::Value = serde_json::from_str(&j2).unwrap();
        assert_eq!(v1["appInstallTime"], v2["appInstallTime"]);
        assert_eq!(v1["appInstallTime"], 1_700_000_000_000_i64);
        // Android appUpdateTime == appInstallTime，同样稳定
        let (a1, _) = build_android_header(&identity, 1, "T", Some(1788958186123));
        let (a2, _) = build_android_header(&identity, 1, "T", Some(1788958190000));
        let a1: serde_json::Value = serde_json::from_str(&a1).unwrap();
        let a2: serde_json::Value = serde_json::from_str(&a2).unwrap();
        assert_eq!(a1["appInstallTime"], a2["appInstallTime"]);
        assert_eq!(a1["appUpdateTime"], a1["appInstallTime"]);
        // 身份未持久化安装时间时，回退值在同一请求内至少自洽
        let fallback = HeaderIdentity::default();
        assert_eq!(fallback.install_time(1000), 1000 - 3 * 86_400_000);
    }
}
