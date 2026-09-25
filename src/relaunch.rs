//! 桌面端自重启：自更新替换完成后拉起新版本进程。

/// 启动新版本程序，旧进程随后退出。
///
pub fn relaunch_self() -> Result<(), String> {
    let current = std::env::current_exe().map_err(|e| format!("无法定位程序路径: {e}"))?;
    // On Unix, /proc/self/exe can follow the running inode after update.rs
    // renames it to `*.old`; launch the replacement at the original path.
    let exe = match current.file_name().and_then(|name| name.to_str()) {
        Some(name) if name.ends_with(".old") => current.with_file_name(name.trim_end_matches(".old")),
        _ => current,
    };
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    std::process::Command::new(&exe)
        .args(args)
        .spawn()
        .map_err(|e| format!("启动新版本失败: {e}"))?;
    Ok(())
}
