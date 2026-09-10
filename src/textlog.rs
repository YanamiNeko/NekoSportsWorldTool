//! 日志文本处理：着色前缀剥离与关键词归类，GUI/CLI 共用。

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LogKind {
    Plain,
    Ok,
    Err,
    Warn,
}

/// 归类并剥离前缀：入站 √/×/⚠ 仅用于着色，不进入展示文本。
pub fn classify(t: &str) -> (LogKind, &str) {
    if let Some(rest) = t.strip_prefix('×') {
        return (LogKind::Err, rest.trim_start());
    }
    if let Some(rest) = t.strip_prefix('⚠') {
        return (LogKind::Warn, rest.trim_start());
    }
    if let Some(rest) = t.strip_prefix('√') {
        return (LogKind::Ok, rest.trim_start());
    }
    if t.contains("失败") || t.contains("错误") {
        return (LogKind::Err, t);
    }
    if t.contains("成功") {
        return (LogKind::Ok, t);
    }
    (LogKind::Plain, t)
}

/// 只取展示文本（CLI 输出用）。
pub fn clean(t: &str) -> &str {
    classify(t).1
}

/// 按字符截断（字节切片会劈开多字节字符导致 panic）。
pub fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 多字节字符中间截断不允许 panic。
    #[test]
    fn test_truncate_multibyte() {
        let s = "中文字符串测试";
        for n in 0..=s.len() + 4 {
            let out = truncate(s, n);
            assert!(out.chars().count() <= n.max(0));
        }
        assert_eq!(truncate("中文abc", 4), "中文ab");
    }
}
