//! 防封号 / 降风控：提交频率、时段与批量补签的本地自检闸门。
//!
//! 风控是学校服务器在验，工具只能尽量不制造“机器特征”。本模块默认 fail-closed：
//! 深夜拦截提交、限制每日跑步 / AI 提交次数、限制批量补签天数与总条数。
//! 策略持久化在 config.json 的 `safety` 对象（可调），每日计数持久化在
//! safety_state.json（与 session.json 同目录）。关闭策略请在明白后果的前提下进行。

use chrono::{Datelike, Local, Timelike};

/// 每日提交策略（config.json `safety` 对象）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SafetyPolicy {
    /// 总开关；false 时全部放行（你自行承担风控/封号风险）。
    pub enabled: bool,
    /// 每天跑步提交次数上限。
    pub max_run_per_day: i64,
    /// 每天 AI 运动提交次数上限。
    pub max_ai_per_day: i64,
    /// 批量补签最多回填天数（含今天）。
    pub max_backfill_days: i64,
    /// 是否拦截深夜时段提交。
    pub block_night: bool,
    /// 深夜起始小时（含）。
    pub night_start_hour: u32,
    /// 深夜结束小时（不含）。
    pub night_end_hour: u32,
}

impl Default for SafetyPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            max_run_per_day: 1,
            max_ai_per_day: 2,
            max_backfill_days: 3,
            block_night: true,
            night_start_hour: 23,
            night_end_hour: 6,
        }
    }
}

/// 每日提交计数（safety_state.json）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SafetyState {
    /// 计数所属本地日期 YYYY-MM-DD。
    pub date: String,
    pub run_count: i64,
    pub ai_count: i64,
    pub backfill_count: i64,
}

impl Default for SafetyState {
    fn default() -> Self {
        Self {
            date: String::new(),
            run_count: 0,
            ai_count: 0,
            backfill_count: 0,
        }
    }
}

pub const STATE_FILE: &str = "safety_state.json";

fn date_of(t: &chrono::DateTime<chrono::Local>) -> String {
    format!("{:04}-{:02}-{:02}", t.year(), t.month(), t.day())
}

/// 加载今日计数；日期变了自动清零并落盘（跨天无需手动重置）。
pub fn load_state() -> SafetyState {
    let mut state: SafetyState =
        crate::api::model::load_persisted(STATE_FILE).unwrap_or_default();
    let today = date_of(&Local::now());
    if state.date != today {
        state.date = today;
        state.run_count = 0;
        state.ai_count = 0;
        state.backfill_count = 0;
        save_state(&state);
    }
    state
}

pub fn save_state(state: &SafetyState) {
    let _ = crate::api::model::save_persisted(STATE_FILE, state);
}

/// 深夜时段等全局拦截；返回拒绝原因。
fn blocked(policy: &SafetyPolicy) -> Option<String> {
    if !policy.enabled || !policy.block_night {
        return None;
    }
    let now = Local::now();
    let h = now.hour();
    let start = policy.night_start_hour % 24;
    let end = policy.night_end_hour % 24;
    let in_window = if start < end {
        h >= start && h < end
    } else {
        h >= start || h < end
    };
    if in_window {
        return Some(format!(
            "风控自检拦截：深夜时段（{}:00–{}:00）不提交，请改到白天，或按需调整 safety 策略",
            start, end
        ));
    }
    None
}

/// 跑步提交前检查。
pub fn check_run(policy: &SafetyPolicy, state: &SafetyState) -> Result<(), String> {
    if !policy.enabled {
        return Ok(());
    }
    if let Some(reason) = blocked(policy) {
        return Err(reason);
    }
    if state.run_count >= policy.max_run_per_day.max(1) {
        return Err(format!(
            "风控自检拦截：今天已跑步提交 {} 次（上限 {}），明天再试",
            state.run_count, policy.max_run_per_day
        ));
    }
    Ok(())
}

/// AI 提交前检查。
pub fn check_ai(policy: &SafetyPolicy, state: &SafetyState) -> Result<(), String> {
    if !policy.enabled {
        return Ok(());
    }
    if let Some(reason) = blocked(policy) {
        return Err(reason);
    }
    if state.ai_count >= policy.max_ai_per_day.max(1) {
        return Err(format!(
            "风控自检拦截：今天已 AI 提交 {} 次（上限 {}），明天再试",
            state.ai_count, policy.max_ai_per_day
        ));
    }
    Ok(())
}

/// 批量补签前检查：回填天数与总条数都不得越过策略上限。
pub fn check_backfill(
    policy: &SafetyPolicy,
    _state: &SafetyState,
    days: i64,
    per_day: i64,
    projects: usize,
) -> Result<(), String> {
    if !policy.enabled {
        return Ok(());
    }
    if let Some(reason) = blocked(policy) {
        return Err(reason);
    }
    if days > policy.max_backfill_days.max(1) {
        return Err(format!(
            "风控自检拦截：批量补签 {} 天超过上限 {} 天",
            days, policy.max_backfill_days
        ));
    }
    let items = days * per_day.max(1) * projects.max(1) as i64;
    if items > policy.max_ai_per_day.max(1) {
        return Err(format!(
            "风控自检拦截：批量补签 {} 条超过每日 AI 上限 {} 条",
            items, policy.max_ai_per_day
        ));
    }
    Ok(())
}

/// 记录一次成功提交并落盘。
pub fn record_run(state: &mut SafetyState) {
    state.run_count += 1;
    save_state(state);
}

pub fn record_ai(state: &mut SafetyState) {
    state.ai_count += 1;
    save_state(state);
}

/// 记录一次批量补签（N 条）并落盘。
pub fn record_backfill(state: &mut SafetyState, n: i64) {
    state.backfill_count += n;
    state.ai_count += n;
    save_state(state);
}

/// `safety` 状态 / 自检报告（CLI 与 GUI 展示）。
pub fn report(policy: &SafetyPolicy, state: &SafetyState) -> Vec<String> {
    let mut out = Vec::new();
    out.push(format!("风控自检（enabled={}）", policy.enabled));
    out.push(format!(
        "  今日 {}：跑步 {} 次 / AI {} 次 / 补签 {} 条",
        state.date, state.run_count, state.ai_count, state.backfill_count
    ));
    out.push(format!(
        "  每日上限：跑步 {} · AI {} · 批量补签最多 {} 天",
        policy.max_run_per_day, policy.max_ai_per_day, policy.max_backfill_days
    ));
    out.push(format!(
        "  深夜时段 {}:00–{}:00 {}",
        policy.night_start_hour,
        policy.night_end_hour,
        if policy.block_night { "拦截提交" } else { "不拦截" }
    ));
    if policy.enabled {
        out.push("  建议：每天 1 次跑步 + 1–2 次 AI；别批量补签刷分；设备身份与网络保持稳定；被标记后立即停用。".into());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> SafetyPolicy {
        SafetyPolicy::default()
    }

    fn state() -> SafetyState {
        SafetyState {
            date: date_of(&Local::now()),
            run_count: 0,
            ai_count: 0,
            backfill_count: 0,
        }
    }

    #[test]
    fn disabled_policy_never_blocks() {
        let mut p = policy();
        p.enabled = false;
        p.max_ai_per_day = 0;
        let s = state();
        assert!(check_ai(&p, &s).is_ok());
        assert!(check_backfill(&p, &s, 60, 10, 10).is_ok());
    }

    #[test]
    fn ai_cap_blocks_after_limit() {
        let p = policy();
        let mut s = state();
        s.ai_count = 2;
        assert!(check_ai(&p, &s).is_err());
        s.ai_count = 1;
        assert!(check_ai(&p, &s).is_ok());
    }

    #[test]
    fn backfill_day_and_item_caps() {
        let p = policy();
        let s = state();
        assert!(check_backfill(&p, &s, 2, 1, 1).is_ok()); // 2 条 = AI 上限
        assert!(check_backfill(&p, &s, 3, 1, 1).is_err()); // 3 条 > AI 上限 2
        assert!(check_backfill(&p, &s, 4, 1, 1).is_err()); // 4 天 > 3 天上限
        assert!(check_backfill(&p, &s, 2, 2, 1).is_err()); // 4 条 > AI 上限 2
    }

    #[test]
    fn run_cap_blocks() {
        let p = policy();
        let mut s = state();
        s.run_count = 1;
        assert!(check_run(&p, &s).is_err());
    }

    #[test]
    fn state_roundtrip_via_persist() {
        let dir = std::env::temp_dir().join(format!("neko-safety-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = Some(dir.clone()));
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = None);
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(dir);
        let mut s = state();
        s.ai_count = 3;
        save_state(&s);
        let loaded: SafetyState =
            crate::api::model::load_persisted(STATE_FILE).unwrap_or_default();
        assert_eq!(loaded.ai_count, 3);
        assert_eq!(loaded.date, s.date);
    }
}
