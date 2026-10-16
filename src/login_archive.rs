//! Portable login backup; never extract archive-controlled paths.
use crate::api::model::{Config, HeaderIdentity, Session};
use std::io::{Cursor, Read, Write};

pub const LIMIT: u64 = 4 * 1024 * 1024;
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Account {
    pub username: String,
    pub password: String,
    pub remember: bool,
}
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Backup {
    version: u32,
    pub session: Session,
    pub identity: HeaderIdentity,
    pub account: Account,
}
pub fn export(
    session: &Session,
    identity: &HeaderIdentity,
    config: &Config,
) -> Result<Vec<u8>, String> {
    let backup = Backup {
        version: 1,
        session: session.clone(),
        identity: identity.clone(),
        account: Account {
            username: config.username.clone(),
            password: if config.remember {
                config.password.clone()
            } else {
                String::new()
            },
            remember: config.remember,
        },
    };
    validate(&backup)?;
    let json = serde_json::to_vec_pretty(&backup).map_err(|e| e.to_string())?;
    if json.len() as u64 > LIMIT {
        return Err("登录信息过大".into());
    }
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file(
        "login.json",
        zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated),
    )
    .map_err(|e| e.to_string())?;
    zip.write_all(&json).map_err(|e| e.to_string())?;
    Ok(zip.finish().map_err(|e| e.to_string())?.into_inner())
}
fn validate(b: &Backup) -> Result<(), String> {
    if b.version != 1 {
        return Err("不支持的登录备份版本".into());
    }
    if !b.session.is_logged_in()
        || b.identity.device_id.trim().is_empty()
        || b.session.device_id != b.identity.device_id
    {
        return Err("登录凭证缺失或设备身份不匹配".into());
    }
    Ok(())
}
pub fn decode(bytes: &[u8]) -> Result<Backup, String> {
    if bytes.len() as u64 > LIMIT {
        return Err("ZIP 超过 4 MB 限制".into());
    }
    let mut zip =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| "不是有效的 ZIP 登录备份")?;
    if zip.len() != 1 {
        return Err("登录备份应仅包含 login.json".into());
    }
    let entry = zip.by_index(0).map_err(|e| e.to_string())?;
    if entry.name() != "login.json" || entry.size() > LIMIT {
        return Err("登录备份内容不合法".into());
    }
    let mut json = Vec::new();
    entry
        .take(LIMIT + 1)
        .read_to_end(&mut json)
        .map_err(|_| "读取登录备份失败")?;
    if json.len() as u64 > LIMIT {
        return Err("登录备份内容过大".into());
    }
    let backup = serde_json::from_slice(&json).map_err(|_| "登录备份 JSON 格式错误")?;
    validate(&backup)?;
    Ok(backup)
}
pub fn import(bytes: &[u8], config: &Config) -> Result<(Backup, Config), String> {
    let b = decode(bytes)?;
    let mut config = config.clone();
    config.username = b.account.username.clone();
    config.remember = b.account.remember;
    config.password = if config.remember {
        b.account.password.clone()
    } else {
        String::new()
    };
    let dir = crate::platform::data_dir();
    let files = [
        ("identity.json", serde_json::to_vec_pretty(&b.identity)),
        ("session.json", serde_json::to_vec_pretty(&b.session)),
        ("config.json", serde_json::to_vec_pretty(&config)),
        // Account-derived caches must not survive an account switch.
        ("points_cache.json", Ok(b"{}".to_vec())),
        ("fence_cache.json", Ok(b"{}".to_vec())),
        ("ai_sports.json", Ok(b"[]".to_vec())),
    ];
    // Snapshot all destinations before writing; restore on an ordinary I/O failure.
    let mut old = Vec::new();
    for (name, _) in &files {
        let path = dir.join(name);
        let data = match std::fs::read(&path) {
            Ok(data) => Some(data),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(format!("无法备份原登录信息：{e}")),
        };
        old.push((path, data));
    }
    for (i, (_, bytes)) in files.into_iter().enumerate() {
        let bytes = bytes.map_err(|e| e.to_string())?;
        if let Err(e) = std::fs::write(&old[i].0, bytes) {
            let mut rollback_failed = false;
            for (path, data) in &old[..=i] {
                let restored = match data {
                    Some(data) => std::fs::write(path, data),
                    None => {
                        if path.exists() {
                            std::fs::remove_file(path)
                        } else {
                            Ok(())
                        }
                    }
                };
                rollback_failed |= restored.is_err();
            }
            return Err(format!(
                "保存登录信息失败：{e}{}",
                if rollback_failed {
                    "；恢复原文件失败，请检查存储空间"
                } else {
                    "；已恢复原文件"
                }
            ));
        }
    }
    Ok((b, config))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn zip_json(name: &str, json: &[u8]) -> Vec<u8> {
        let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
        z.start_file(
            name,
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated),
        )
        .unwrap();
        z.write_all(json).unwrap();
        z.finish().unwrap().into_inner()
    }
    #[test]
    fn hostile_archives_are_rejected() {
        assert!(decode(&zip_json("../session.json", b"{}")).is_err());
        assert!(decode(&zip_json("login.json", b"not json")).is_err());
        assert!(decode(&zip_json("login.json", &vec![b' '; LIMIT as usize + 1])).is_err());
        let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
        z.start_file("login.json", zip::write::SimpleFileOptions::default())
            .unwrap();
        z.write_all(b"{}").unwrap();
        z.start_file("extra.json", zip::write::SimpleFileOptions::default())
            .unwrap();
        assert!(decode(&z.finish().unwrap().into_inner()).is_err());
    }
    #[test]
    fn import_preserves_settings_and_invalid_input_preserves_files() {
        use crate::api::model;
        let directory =
            std::env::temp_dir().join(format!("neko-login-test-{}", uuid::Uuid::new_v4()));
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
        let identity = model::load_identity();
        let session = Session {
            uid: 42,
            token: "test-token".into(),
            device_id: identity.device_id.clone(),
            ..Default::default()
        };
        let source = Config {
            username: "new-user".into(),
            password: "not-remembered".into(),
            remember: false,
            ..Default::default()
        };
        let bytes = export(&session, &identity, &source).unwrap();
        assert!(decode(&bytes).unwrap().account.password.is_empty());
        let current = Config {
            dist_min: 7.0,
            osm_path: "keep.osm".into(),
            ..Default::default()
        };
        let (_, restored) = import(&bytes, &current).unwrap();
        assert_eq!(restored.dist_min, 7.0);
        assert_eq!(restored.osm_path, "keep.osm");
        assert_eq!(model::load_session().token, session.token);
        assert_eq!(model::load_config().username, "new-user");
        let saved = std::fs::read(directory.join("session.json")).unwrap();
        assert!(import(b"bad zip", &current).is_err());
        assert_eq!(
            std::fs::read(directory.join("session.json")).unwrap(),
            saved
        );
        // A non-file destination is detected before any existing file is modified.
        std::fs::remove_file(directory.join("config.json")).unwrap();
        std::fs::create_dir(directory.join("config.json")).unwrap();
        assert!(import(&bytes, &current).is_err());
        assert_eq!(
            std::fs::read(directory.join("session.json")).unwrap(),
            saved
        );
        let mut value = serde_json::to_value(decode(&bytes).unwrap()).unwrap();
        value["version"] = 999.into();
        assert!(decode(&zip_json(
            "login.json",
            &serde_json::to_vec(&value).unwrap()
        ))
        .is_err());
    }
    #[test]
    fn roundtrip_and_rejection() {
        let mut identity = HeaderIdentity::default();
        identity.device_id = "backup-test".into();
        let session = Session {
            uid: 42,
            token: "test-token".into(),
            device_id: identity.device_id.clone(),
            ..Default::default()
        };
        let config = Config {
            username: "test".into(),
            password: "secret".into(),
            remember: true,
            ..Default::default()
        };
        let bytes = export(&session, &identity, &config).unwrap();
        let b = decode(&bytes).unwrap();
        assert_eq!(b.account.password, "secret");
        assert_eq!(b.session.token, session.token);
        assert!(decode(b"not zip").is_err());
        assert!(decode(&bytes[..bytes.len() / 2]).is_err());
        identity.device_id = "other".into();
        assert!(export(&session, &identity, &config).is_err());
    }
}
