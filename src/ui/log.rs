//! 运行日志：彩色等宽滚动，自动滚底，上限 2000 行。
//! 入站行的 √/×/⚠ 前缀仅用于着色，落盘展示时剥离。

use super::theme;
use crate::textlog::{classify, LogKind};
use egui::{Color32, RichText, ScrollArea};
use std::collections::VecDeque;

const MAX_LINES: usize = 2000;

#[derive(Debug, Clone)]
pub struct LogLine {
    pub text: String,
    pub kind: LogKind,
}

impl LogLine {
    fn color(&self) -> Color32 {
        match self.kind {
            LogKind::Plain => theme::plain(),
            LogKind::Ok => theme::ok(),
            LogKind::Err => theme::err(),
            LogKind::Warn => theme::warn(),
        }
    }
}

#[derive(Default)]
pub struct LogStore {
    lines: VecDeque<LogLine>,
}

impl LogStore {
    pub fn push(&mut self, msg: &str) {
        for line in msg.lines() {
            let t = line.trim();
            if t.is_empty() {
                continue;
            }
            if self.lines.len() >= MAX_LINES {
                self.lines.pop_front();
            }
            let (kind, text) = classify(t);
            self.lines.push_back(LogLine { text: text.to_string(), kind });
        }
    }

    #[allow(dead_code)]
    pub fn clear(&mut self) {
        self.lines.clear();
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn render(&mut self, ui: &mut egui::Ui) {
        ScrollArea::vertical()
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for line in &self.lines {
                    ui.label(RichText::new(&line.text).monospace().size(12.5).color(line.color()));
                }
            });
    }
}
