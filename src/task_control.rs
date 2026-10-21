use std::sync::atomic::{AtomicU8, Ordering};
/// 0: cancellable preparation, 1: cancelled, 2: irreversible submission started.
#[derive(Default)]
pub struct Control(AtomicU8);
impl Control {
    pub fn cancel(&self) -> bool {
        self.0
            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }
    pub fn check(&self) -> Result<(), String> {
        if self.0.load(Ordering::SeqCst) == 1 {
            Err("操作已取消；正在等待的网络请求可能仍在结束中".into())
        } else {
            Ok(())
        }
    }
    pub fn begin_submit(&self) -> Result<(), String> {
        self.0
            .compare_exchange(0, 2, Ordering::SeqCst, Ordering::SeqCst)
            .map(|_| ())
            .map_err(|_| "操作已取消，未进入记录提交".into())
    }
    pub fn cancellable(&self) -> bool {
        self.0.load(Ordering::SeqCst) == 0
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn cancellation_and_submission_are_mutually_exclusive() {
        let c = super::Control::default();
        assert!(c.cancel());
        assert!(c.check().is_err());
        assert!(c.begin_submit().is_err());
        let c = super::Control::default();
        assert!(c.begin_submit().is_ok());
        assert!(!c.cancel());
    }
}
