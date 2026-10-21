pub fn explain(error: &str) -> String {
    let (kind, help) = if crate::api::client::is_session_expired(error) {
        (
            "登录已失效",
            "在账号栏重新登录；成功后会重新获取学校围栏和路网，本地地图仍可查看",
        )
    } else if error.contains("取消") {
        (
            "已取消",
            "可在当前操作结束后重试；已发出的网络请求不能立刻撤回",
        )
    } else if error.contains("timeout") || error.contains("timed out") || error.contains("超时") {
        ("网络超时", "检查网络后重试，或从缓存离线预览")
    } else if error.contains("没有可用") || error.contains("没有可跑") || error.contains("空路网")
    {
        (
            "学校缺少道路数据",
            "更新路网或手动导入完整 OSM；不会向校外补足距离",
        )
    } else if error.contains("围栏") {
        (
            "围栏或校区问题",
            "在线刷新校区列表、核对设备定位，并确认学校边界",
        )
    } else if error.contains("不可达") || error.contains("不连通") || error.contains("超过 40")
    {
        ("点位不可达", "查看地图红色问题点，确认校区并补充缺失道路")
    } else if error.contains("不足") {
        ("目标距离不足", "按路线检查给出的最短距离增大目标里程")
    } else if error.contains("闭环") || error.contains("有效路线") {
        (
            "校内道路无法闭合",
            "更新或补充校内道路，检查单向限制和断开的道路",
        )
    } else if error.contains("下载")
        || error.contains("network")
        || error.contains("http")
        || error.contains("HTTP")
    {
        ("路网网络请求失败", "检查网络后重试；已有缓存可离线查看")
    } else {
        ("处理失败", "查看具体原因，修正输入后重试")
    };
    format!("[{kind}] {error}\n建议：{help}")
}
#[cfg(test)]
mod tests {
    #[test]
    fn failures_have_distinct_actions() {
        for (e, label) in [
            (crate::api::client::SESSION_EXPIRED, "登录已失效"),
            ("timeout", "网络超时"),
            ("没有可用的 OSM 道路", "学校缺少道路数据"),
            ("没有有效围栏", "围栏或校区问题"),
            ("点位不可达", "点位不可达"),
            ("目标距离不足", "目标距离不足"),
        ] {
            assert!(super::explain(e).starts_with(&format!("[{label}]")));
        }
    }
}
