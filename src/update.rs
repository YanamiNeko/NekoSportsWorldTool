//! 自动更新：GitHub Release 检查 / 下载 / 解包 / 自替换。
//!
//! 桌面产物（win zip / mac tar.gz / linux tar.gz ×2）只查上游仓库；
//! Android 查上游 + fork 两个仓库（上游发 APK 前先落在 fork，发了则
//! 自动取更新的那个），取带 APK 资产且版本最新的 Release。
//! NSWT_UPDATE_REPO 环境变量可强制指定仓库（测试用）。
//! 版本号 / 重启 / 安装的平台差异见 platform.rs。

use crate::api::client::{make_agent, ureq_err};

pub const REPO_UPSTREAM: &str = "YanamiNeko/NekoSportsWorldTool";
pub const REPO_FORK: &str = "hb-degithub/NekoSportsWorldTool";

const UA: &str = concat!("NekoSportsWorldTool/", env!("CARGO_PKG_VERSION"));

/// Android 更新包落盘文件名（私有目录，ApkProvider 只暴露这一个文件）。
pub const APK_NAME: &str = "update.apk";

/// 检查更新要查询的仓库（按序）。
fn repos() -> Vec<String> {
    if let Ok(r) = std::env::var("NSWT_UPDATE_REPO") {
        if !r.trim().is_empty() {
            return vec![r.trim().to_string()];
        }
    }
    #[cfg(target_os = "android")]
    let list = [REPO_UPSTREAM, REPO_FORK];
    #[cfg(not(target_os = "android"))]
    let list = [REPO_UPSTREAM];
    list.iter().map(|s| s.to_string()).collect()
}

/// 当前平台对应的 Release 资产名；无匹配产物的平台返回 None。
pub fn asset_name() -> Option<&'static str> {
    platform_asset(std::env::consts::OS, std::env::consts::ARCH, cfg!(feature = "gui"))
}

fn platform_asset(os: &str, arch: &str, gui: bool) -> Option<&'static str> {
    match (os, arch) {
        ("windows", "x86_64") => Some("NekoSportsWorldTool-win-x64.zip"),
        ("macos", "aarch64") => Some("NekoSportsWorldTool-macos-arm64.tar.gz"),
        ("linux", "x86_64") if gui => Some("NekoSportsWorldTool-linux-x64-gui.tar.gz"),
        ("linux", "x86_64") => Some("NekoSportsWorldTool-linux-x64.tar.gz"),
        ("linux", "aarch64") if !gui => Some("NekoSportsWorldTool-linux-aarch64.tar.gz"),
        ("android", "aarch64") => Some("NekoSportsWorldTool-android-arm64.apk"),
        _ => None,
    }
}

/// 当前程序版本（platform.rs：桌面读 Cargo，Android 读 manifest versionName）。
pub fn current_version() -> String {
    crate::platform::version_name()
}

/// Release 摘要（GUI / CLI 共用，经 channel JSON 传递）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReleaseInfo {
    pub repo: String,
    pub tag: String,
    pub title: String,
    pub notes: String,
    pub asset_name: String,
    pub asset_url: String,
    pub asset_size: u64,
}

/// 检查最新 Release；比当前新返回 Some，已是最新返回 None。
/// 多仓库（Android）时取带资产且版本最新的候选。
pub fn check_latest(log: &mut dyn FnMut(&str)) -> Result<Option<ReleaseInfo>, String> {
    let want = asset_name().ok_or_else(|| "当前平台没有对应的发布产物，无法自动更新".to_string())?;
    let current = current_version();
    let mut best: Option<ReleaseInfo> = None;
    let mut last_err: Option<String> = None;
    for repo in repos() {
        match fetch_release(&repo, want, log) {
            Ok(Some(rel)) => {
                if best.as_ref().map(|b| is_newer(&b.tag, &rel.tag)).unwrap_or(true) {
                    best = Some(rel);
                }
            }
            Ok(None) => {}
            Err(e) => {
                log(&format!("[update] {repo}: {e}"));
                last_err = Some(e);
            }
        }
    }
    match best {
        Some(rel) => {
            if is_newer(&current, &rel.tag) {
                log(&format!(
                    "√ [update] {} 最新 {}（当前 v{}，{:.1} MB）",
                    rel.repo,
                    rel.tag,
                    current,
                    rel.asset_size as f64 / 1024.0 / 1024.0
                ));
                Ok(Some(rel))
            } else {
                log(&format!("√ [update] 已是最新版本（v{current}）"));
                Ok(None)
            }
        }
        None => Err(last_err.unwrap_or_else(|| "更新源没有可用的发布资产".into())),
    }
}

/// 拉取单仓库最新 Release 并匹配资产；Release 存在但无匹配资产 → Ok(None)。
fn fetch_release(repo: &str, want: &str, log: &mut dyn FnMut(&str)) -> Result<Option<ReleaseInfo>, String> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let resp = make_agent()
        .get(&url)
        .set("User-Agent", UA)
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(ureq_err)?;
    let text = resp.into_string().map_err(|e| format!("读取 Release 失败: {e}"))?;
    let v: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("解析 Release 失败: {e}"))?;
    let tag = v.get("tag_name").and_then(|x| x.as_str()).unwrap_or("").to_string();
    if tag.is_empty() {
        return Err("Release 缺少 tag".into());
    }
    let Some(asset) = pick_asset(&v, want) else {
        return Ok(None);
    };
    log(&format!("[update] {repo} 最新 {tag}，匹配资产 {}", asset.name));
    Ok(Some(ReleaseInfo {
        repo: repo.to_string(),
        tag,
        title: v.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        notes: v.get("body").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        asset_name: asset.name,
        asset_url: asset.url,
        asset_size: asset.size,
    }))
}

struct AssetPick {
    name: String,
    url: String,
    size: u64,
}

/// 资产匹配：桌面产物要求名字完全一致；APK 先精确后模糊（兼容历史命名）。
fn pick_asset(v: &serde_json::Value, want: &str) -> Option<AssetPick> {
    let assets = v.get("assets")?.as_array()?;
    let find = |pred: &dyn Fn(&str) -> bool| -> Option<AssetPick> {
        assets.iter().find_map(|a| {
            let name = a.get("name")?.as_str()?;
            if !pred(name) {
                return None;
            }
            Some(AssetPick {
                name: name.to_string(),
                url: a.get("browser_download_url")?.as_str()?.to_string(),
                size: a.get("size")?.as_u64().unwrap_or(0),
            })
        })
    };
    if want.ends_with(".apk") {
        find(&|n| n == want)
            .or_else(|| find(&|n| n.ends_with(".apk") && n.contains("arm64")))
            .or_else(|| find(&|n| n.ends_with(".apk")))
    } else {
        find(&|n| n == want)
    }
}

/// 版本比较：剥 v 前缀按点分段数字比较；主版本相同时 -android.N 后缀大的更新。
pub fn is_newer(current: &str, latest: &str) -> bool {
    let (c_base, c_rev) = split_rev(current);
    let (l_base, l_rev) = split_rev(latest);
    if c_base != l_base {
        return l_base > c_base;
    }
    l_rev > c_rev
}

/// "v0.2.5-android.3" → ([0,2,5], Some(3))。
fn split_rev(v: &str) -> (Vec<u64>, Option<u64>) {
    let v = v.trim().trim_start_matches(['v', 'V']);
    let (base, rev) = match v.split_once('-') {
        Some((b, r)) => (b, r.rsplit('.').next().and_then(|n| n.parse().ok())),
        None => (v, None),
    };
    let segs = base.split('.').map(|s| s.parse::<u64>().unwrap_or(0)).collect();
    (segs, rev)
}

/// 流式下载（默认 agent 30s 总超时不够下几 MB 的包，专用长超时）。
pub fn download(url: &str, mut progress: impl FnMut(u64, Option<u64>)) -> Result<Vec<u8>, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(300))
        .build();
    let resp = agent.get(url).set("User-Agent", UA).call().map_err(ureq_err)?;
    let total = resp.header("Content-Length").and_then(|h| h.parse::<u64>().ok());
    let mut buf = Vec::new();
    let mut reader = resp.into_reader();
    let mut chunk = vec![0u8; 65536];
    loop {
        let n = std::io::Read::read(&mut reader, &mut chunk)
            .map_err(|e| format!("下载中断: {e}"))?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        progress(buf.len() as u64, total);
    }
    Ok(buf)
}

/// 从发布包解出新程序二进制。桌面：zip 内找 *.exe（7z 打包带 target/
/// 目录前缀，取最大的一条）；tar.gz 内找根级 nekosportsworldtool。
#[cfg(not(target_os = "android"))]
pub fn extract(asset_name: &str, bytes: &[u8]) -> Result<Vec<u8>, String> {
    if asset_name.ends_with(".zip") {
        extract_zip(bytes)
    } else if asset_name.ends_with(".tar.gz") || asset_name.ends_with(".tgz") {
        extract_tar_gz(bytes)
    } else if asset_name.ends_with(".apk") {
        Ok(bytes.to_vec())
    } else {
        Err(format!("不支持的发布包格式：{asset_name}"))
    }
}

/// Android 无需解包（APK 整包交给系统安装器）。
#[cfg(target_os = "android")]
pub fn extract(asset_name: &str, bytes: &[u8]) -> Result<Vec<u8>, String> {
    if asset_name.ends_with(".apk") {
        Ok(bytes.to_vec())
    } else {
        Err(format!("不支持的发布包格式：{asset_name}"))
    }
}

#[cfg(not(target_os = "android"))]
fn extract_zip(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("zip 打开失败: {e}"))?;
    let mut best: Option<Vec<u8>> = None;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(|e| format!("zip 读取失败: {e}"))?;
        if f.is_dir() {
            continue;
        }
        let is_exe = std::path::Path::new(f.name())
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("exe"))
            .unwrap_or(false);
        if !is_exe {
            continue;
        }
        let mut out = Vec::with_capacity(f.size() as usize);
        std::io::Read::read_to_end(&mut f, &mut out).map_err(|e| format!("zip 解压失败: {e}"))?;
        if best.as_ref().map(|b| out.len() > b.len()).unwrap_or(true) {
            best = Some(out);
        }
    }
    best.ok_or_else(|| "zip 内未找到程序文件".into())
}

#[cfg(not(target_os = "android"))]
fn extract_tar_gz(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let gz = flate2::read::GzDecoder::new(bytes);
    let mut tar = tar::Archive::new(gz);
    let mut best: Option<Vec<u8>> = None;
    let mut entries = tar.entries().map_err(|e| format!("tar 打开失败: {e}"))?;
    while let Some(entry) = entries.next() {
        let mut e = entry.map_err(|e| format!("tar 读取失败: {e}"))?;
        if !e.header().entry_type().is_file() {
            continue;
        }
        let path = e.path().map_err(|e| format!("tar 路径异常: {e}"))?;
        if path.file_name().and_then(|n| n.to_str()) != Some("nekosportsworldtool") {
            continue;
        }
        let mut out = Vec::new();
        std::io::Read::read_to_end(&mut e, &mut out).map_err(|e| format!("tar 解压失败: {e}"))?;
        if best.as_ref().map(|b| out.len() > b.len()).unwrap_or(true) {
            best = Some(out);
        }
    }
    best.ok_or_else(|| "tar.gz 内未找到程序文件".into())
}

/// 自替换：新二进制落 .new → 当前程序改名 .old（Windows 允许重命名运行中的
/// exe）→ .new 转正；失败回滚。调用方随后退出，残留 .old 由下次启动清理。
#[cfg(not(target_os = "android"))]
pub fn apply(binary: &[u8]) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("无法定位自身路径: {e}"))?;
    let dir = exe.parent().ok_or("无法定位 exe 目录")?;
    let file = exe.file_name().and_then(|n| n.to_str()).ok_or("exe 文件名异常")?;
    let new = dir.join(format!("{file}.new"));
    let old = dir.join(format!("{file}.old"));
    #[cfg(target_os = "macos")]
    let old = exe.ancestors()
        .find(|path| path.extension().is_some_and(|ext| ext == "app"))
        .and_then(|bundle| bundle.parent())
        .map(|parent| parent.join(format!(".neko-update-{}.old", std::process::id())))
        .unwrap_or(old);
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&new)
            .map_err(|e| format!("写入更新文件失败（目录可能只读）: {e}"))?;
        f.write_all(binary).map_err(|e| format!("写入更新内容失败: {e}"))?;
        f.sync_all().map_err(|e| format!("落盘失败: {e}"))?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("设置执行权限失败: {e}"))?;
    }
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&exe, &old).map_err(|e| {
        let _ = std::fs::remove_file(&new);
        format!("无法移动当前程序（可能被占用）: {e}")
    })?;
    if let Err(e) = std::fs::rename(&new, &exe) {
        let _ = std::fs::rename(&old, &exe);
        return Err(format!("替换程序失败: {e}"));
    }
    #[cfg(target_os = "macos")]
    if let Some(bundle) = exe.ancestors().find(|path| path.extension().is_some_and(|ext| ext == "app")) {
        // The release is an ad-hoc signed app bundle. Replacing its executable
        // invalidates the bundle seal; restore it before relaunching.
        let signed = std::process::Command::new("/usr/bin/codesign")
            .args(["--force", "--deep", "--sign", "-"])
            .arg(bundle)
            .status();
        if !signed.is_ok_and(|status| status.success()) {
            let _ = std::fs::remove_file(&exe);
            std::fs::rename(&old, &exe).map_err(|error| format!("应用签名失败且回滚失败：{error}"))?;
            let _ = std::process::Command::new("/usr/bin/codesign")
                .args(["--force", "--deep", "--sign", "-"])
                .arg(bundle)
                .status();
            return Err("macOS 应用签名失败，已恢复旧程序；请下载新的应用包".into());
        }
        // Keep rollback files outside the signed bundle, then remove them once
        // signing succeeds; deleting sealed resources would invalidate it again.
        let _ = std::fs::remove_file(&old);
    }
    Ok(())
}

/// 清理上次更新残留的 .old/.new（旧进程未退出时会删不掉，静默跳过）。
pub fn cleanup_residue() {
    let Ok(exe) = std::env::current_exe() else { return };
    let Some(dir) = exe.parent() else { return };
    let Some(file) = exe.file_name().and_then(|n| n.to_str()) else { return };
    for suffix in [".old", ".new"] {
        let _ = std::fs::remove_file(dir.join(format!("{file}{suffix}")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_compare() {
        assert!(is_newer("0.2.5", "v0.2.6"));
        assert!(is_newer("0.2.9", "v0.3.0"));
        assert!(is_newer("0.2.9", "v0.10.0"));
        assert!(!is_newer("0.2.5", "v0.2.5"));
        assert!(!is_newer("0.2.6", "v0.2.5"));
        // android 后缀序号
        assert!(is_newer("0.2.5-android.3", "v0.2.5-android.4"));
        assert!(!is_newer("0.2.5-android.4", "v0.2.5-android.4"));
        assert!(is_newer("0.2.5", "v0.2.6-android.1"));
        assert!(is_newer("0.2.5-android.99", "v0.2.6-android.1"));
        // 位数不同 / 前后空格
        assert!(is_newer("0.2", "v0.2.1"));
        assert!(!is_newer(" 0.2.5 ", "v0.2.5"));
    }

    #[test]
    fn asset_pick_exact_and_fuzzy() {
        let mk = |names: &[&str]| {
            serde_json::json!({
                "assets": names.iter().map(|n| serde_json::json!({
                    "name": n,
                    "size": 100,
                    "browser_download_url": format!("https://x/{n}"),
                })).collect::<Vec<_>>(),
            })
        };
        let exact = pick_asset(&mk(&["NekoSportsWorldTool-win-x64.zip", "other.txt"]), "NekoSportsWorldTool-win-x64.zip");
        assert_eq!(exact.unwrap().name, "NekoSportsWorldTool-win-x64.zip");
        // 桌面产物必须精确匹配
        assert!(pick_asset(&mk(&["wrong.zip"]), "NekoSportsWorldTool-win-x64.zip").is_none());
        let linux_gui = platform_asset("linux", "x86_64", true).unwrap();
        let linux_cli = platform_asset("linux", "x86_64", false).unwrap();
        assert!(pick_asset(&mk(&[linux_cli]), linux_gui).is_none());
        assert_eq!(pick_asset(&mk(&[linux_cli, linux_gui]), linux_gui).unwrap().name, linux_gui);
        // APK 模糊匹配：先精确、再 arm64、再任意 apk
        let apk = pick_asset(&mk(&["NekoSportsWorldTool-0.2.5-android.2-arm64.apk"]), "NekoSportsWorldTool-android-arm64.apk");
        assert_eq!(apk.unwrap().name, "NekoSportsWorldTool-0.2.5-android.2-arm64.apk");
        let any = pick_asset(&mk(&["app-arm64.apk", "plain.apk"]), "NekoSportsWorldTool-android-arm64.apk");
        assert_eq!(any.unwrap().name, "app-arm64.apk");
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn extract_zip_nested_exe() {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            let opt = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            w.start_file("target/x86_64-pc-windows-msvc/release/nekosportsworldtool.exe", opt).unwrap();
            std::io::Write::write_all(&mut w, b"BIN_CONTENT").unwrap();
            w.start_file("readme.txt", opt).unwrap();
            std::io::Write::write_all(&mut w, b"ignore").unwrap();
            w.finish().unwrap();
        }
        let bytes = buf.into_inner();
        assert_eq!(extract_zip(&bytes).unwrap(), b"BIN_CONTENT");
        assert_eq!(extract("NekoSportsWorldTool-win-x64.zip", &bytes).unwrap(), b"BIN_CONTENT");
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn extract_tar_gz_root_binary() {
        let mut bytes = Vec::new();
        {
            let gz = flate2::write::GzEncoder::new(&mut bytes, flate2::Compression::default());
            let mut tar = tar::Builder::new(gz);
            let mut hdr = tar::Header::new_gnu();
            let content: &[u8] = b"BIN_CONTENT";
            hdr.set_size(content.len() as u64);
            hdr.set_mode(0o755);
            hdr.set_cksum();
            tar.append_data(&mut hdr, "nekosportsworldtool", content).unwrap();
            let gz = tar.into_inner().unwrap();
            gz.finish().unwrap();
        }
        assert_eq!(extract_tar_gz(&bytes).unwrap(), b"BIN_CONTENT");
        assert_eq!(extract("NekoSportsWorldTool-linux-x64.tar.gz", &bytes).unwrap(), b"BIN_CONTENT");
    }
}
