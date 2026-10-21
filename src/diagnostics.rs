//! Diagnostics use a whitelist; raw logs, responses, coordinates and account fields are excluded.
pub fn category(text: &str) -> &'static str {
    for (needle, label) in [
        ("围栏", "围栏缺失、变化或未确认"),
        ("不连通", "校内道路不连通"),
        ("不可达", "点位不可达"),
        ("超过 40", "点位离路超过判定范围"),
        ("不足", "目标距离不足"),
        ("超时", "网络超时"),
        ("timeout", "网络超时"),
        ("下载", "路网下载或缓存处理"),
        ("登录", "登录状态或学校信息"),
        ("失败", "操作失败"),
        ("错误", "操作错误"),
    ] {
        if text.contains(needle) {
            return label;
        }
    }
    "未分类状态（原文为保护隐私不导出）"
}
pub fn report(
    status: &str,
    road_status: &str,
    manual: bool,
    offline: bool,
    nodes: usize,
    edges: usize,
    cache_count: usize,
) -> serde_json::Value {
    serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"os":std::env::consts::OS,"arch":std::env::consts::ARCH,
 "error_category":category(status),"road_status_category":category(road_status),"road_source":if manual{"manual"}else{"school"},
 "offline_preview":offline,"road_nodes":nodes,"road_edges":edges,"saved_maps":cache_count,
 "privacy":"不含账号、姓名、Token、设备标识、原始日志/响应、文件路径、学校编号或轨迹坐标"})
}
#[cfg(test)]
mod tests {
    #[test]
    fn secrets_in_errors_are_never_exported() {
        let s = super::report(
            "登录失败 uid=123 token=super-secret /Users/Alice",
            "下载失败 password=hunter2",
            false,
            false,
            2,
            3,
            1,
        )
        .to_string();
        for secret in ["123", "super-secret", "Alice", "hunter2"] {
            assert!(!s.contains(secret));
        }
        assert!(s.contains("登录状态"));
    }
}
