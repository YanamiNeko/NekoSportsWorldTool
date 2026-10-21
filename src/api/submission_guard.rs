//! Write-ahead receipts, scoped to an account and school. No tokens or coordinates.
//! OS file locking covers GUI/CLI processes using the same data directory.
use super::{client::ApiClient, records::RecordRow};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum Event {
    Pending {
        uuid: String,
        start_ms: i64,
        stop_ms: i64,
    },
    Accepted {
        uuid: String,
        rrid: i64,
    },
    Rejected {
        uuid: String,
    },
}

#[derive(Debug)]
struct Attempt {
    uuid: String,
    start_ms: i64,
    stop_ms: i64,
    // None: outcome unknown; Some(None): explicit rejection; Some(Some(id)): accepted.
    outcome: Option<Option<i64>>,
}

pub(crate) struct Guard {
    file: File,
    attempts: Vec<Attempt>,
}

impl Guard {
    pub fn open(uid: i64, unid: i64) -> Result<Self, String> {
        Self::open_in(&crate::platform::data_dir(), uid, unid)
    }

    fn open_in(root: &Path, uid: i64, unid: i64) -> Result<Self, String> {
        if uid <= 0 || unid <= 0 {
            return Err("账号或学校编号无效，无法启用重复提交保护".into());
        }
        let directory = root.join("submission_receipts");
        std::fs::create_dir_all(&directory).map_err(|e| format!("无法建立提交保护目录：{e}"))?;
        let name = format!("{:x}.jsonl", Sha256::digest(format!("{uid}:{unid}")));
        let mut file = File::options()
            .read(true)
            .append(true)
            .create(true)
            .open(directory.join(name))
            .map_err(|e| format!("无法读取提交保护记录：{e}"))?;
        file.try_lock()
            .map_err(|_| "同一账号正在提交或核对记录，请等待完成".to_string())?;
        if file.metadata().map_err(|e| e.to_string())?.len() > 16 * 1024 * 1024 {
            return Err("提交保护记录过大，已停止提交，请检查本地记录".into());
        }
        let mut text = String::new();
        file.read_to_string(&mut text)
            .map_err(|e| format!("提交保护记录读取失败：{e}"))?;
        // A torn last write is not evidence that a request was never sent.
        if !text.is_empty() && !text.ends_with('\n') {
            return Err("提交保护记录不完整，已停止提交；请保留文件并核对官方记录".into());
        }
        let mut guard = Self {
            file,
            attempts: Vec::new(),
        };
        for line in text.lines() {
            let event = serde_json::from_str(line).map_err(|_| {
                "提交保护记录损坏，已停止提交；请保留文件并核对官方记录".to_string()
            })?;
            guard.apply(event)?;
        }
        Ok(guard)
    }

    fn apply(&mut self, event: Event) -> Result<(), String> {
        match event {
            Event::Pending {
                uuid,
                start_ms,
                stop_ms,
            } => {
                if uuid::Uuid::parse_str(&uuid).is_err()
                    || start_ms <= 0
                    || stop_ms <= start_ms
                    || self.attempts.iter().any(|a| a.uuid == uuid)
                {
                    return Err("提交保护记录包含无效请求，已停止提交".into());
                }
                self.attempts.push(Attempt {
                    uuid,
                    start_ms,
                    stop_ms,
                    outcome: None,
                });
            }
            Event::Accepted { uuid, rrid } => {
                if rrid <= 0 {
                    return Err("提交回执编号无效".into());
                }
                let a = self
                    .attempts
                    .iter_mut()
                    .find(|a| a.uuid == uuid && a.outcome.is_none())
                    .ok_or("提交保护回执没有对应请求")?;
                a.outcome = Some(Some(rrid));
            }
            Event::Rejected { uuid } => {
                let a = self
                    .attempts
                    .iter_mut()
                    .find(|a| a.uuid == uuid && a.outcome.is_none())
                    .ok_or("提交保护拒绝结果没有对应请求")?;
                a.outcome = Some(None);
            }
        }
        Ok(())
    }

    fn append(&mut self, event: Event) -> Result<(), String> {
        let mut bytes = serde_json::to_vec(&event).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        self.file
            .write_all(&bytes)
            .and_then(|_| self.file.sync_all())
            .map_err(|e| format!("提交保护记录保存失败，禁止重试提交：{e}"))?;
        self.apply(event)
    }

    pub fn has_pending(&self) -> bool {
        self.attempts.iter().any(|a| a.outcome.is_none())
    }

    pub fn check(&self, start_ms: i64, stop_ms: i64) -> Result<(), String> {
        if start_ms <= 0 || stop_ms <= start_ms {
            return Err("提交时间范围无效".into());
        }
        if self.has_pending() {
            return Err("上次提交结果仍未知，已阻止新的提交。请到运动记录页刷新核对；未查到不代表未入账，不会自动重试".into());
        }
        for a in &self.attempts {
            if let Some(Some(rrid)) = a.outcome {
                if start_ms < a.stop_ms && stop_ms > a.start_ms {
                    return Err(format!("该时间段已有本工具提交的记录 #{rrid}，已阻止重复提交；请到运动记录查看详情"));
                }
            }
        }
        Ok(())
    }

    pub fn begin(&mut self, uuid: &str, start_ms: i64, stop_ms: i64) -> Result<(), String> {
        self.check(start_ms, stop_ms)?;
        self.append(Event::Pending {
            uuid: uuid.into(),
            start_ms,
            stop_ms,
        })
    }
    pub fn accepted(&mut self, uuid: &str, rrid: i64) -> Result<(), String> {
        self.append(Event::Accepted {
            uuid: uuid.into(),
            rrid,
        })
    }
    pub fn rejected(&mut self, uuid: &str) -> Result<(), String> {
        self.append(Event::Rejected { uuid: uuid.into() })
    }

    pub fn reconcile(&mut self, rows: &[RecordRow]) -> Result<Vec<i64>, String> {
        let mut found = Vec::new();
        for attempt in self.attempts.iter().filter(|a| a.outcome.is_none()) {
            let mut matches = rows
                .iter()
                .filter(|r| r.rrid > 0 && r.uuid.eq_ignore_ascii_case(&attempt.uuid));
            if let Some(row) = matches.next() {
                // An ambiguous server response cannot safely resolve the pending request.
                if matches.all(|other| other.rrid == row.rrid) {
                    found.push((attempt.uuid.clone(), row.rrid));
                }
            }
        }
        let mut ids = Vec::new();
        for (uuid, rrid) in found {
            self.accepted(&uuid, rrid)?;
            ids.push(rrid);
        }
        Ok(ids)
    }
}

pub(crate) fn preflight(
    client: &mut ApiClient,
    start_ms: i64,
    duration: i64,
    log: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let session = client.login.as_ref().ok_or("未登录")?;
    let mut guard = Guard::open(
        session.uid,
        session.unid.parse().map_err(|_| "学校编号无效")?,
    )?;
    if guard.has_pending() {
        match super::records::fetch_records(client) {
            Ok(rows) => {
                for id in guard.reconcile(&rows)? {
                    log(&format!("[record] 已核对先前提交的记录 #{id}；未重复提交"));
                }
            }
            Err(e) => log(&format!("[record] 只读核对失败：{e}")),
        }
    }
    let stop = duration
        .checked_mul(1000)
        .and_then(|n| start_ms.checked_add(n))
        .ok_or("时间范围溢出")?;
    guard.check(start_ms, stop)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("submit-guard-{}", uuid::Uuid::new_v4()))
    }
    fn row(uuid: &str, id: i64) -> RecordRow {
        serde_json::from_value(serde_json::json!({"rrid":id,"uuid":uuid,"total_dis":1000.0,
            "total_time":600,"start_time":1000,"complete":false,"avg_step_freq":120}))
        .unwrap()
    }
    #[test]
    fn pending_survives_restart_and_missing_list_is_not_rejection() {
        let root = root();
        let id = uuid::Uuid::new_v4().to_string();
        {
            let mut g = Guard::open_in(&root, 1, 2).unwrap();
            g.begin(&id, 1000, 601000).unwrap();
        }
        let mut g = Guard::open_in(&root, 1, 2).unwrap();
        assert!(g.reconcile(&[]).unwrap().is_empty());
        assert!(g.check(900000, 1000000).is_err());
        assert!(g.reconcile(&[row("different-uuid", 7)]).unwrap().is_empty());
        assert_eq!(g.reconcile(&[row(&id.to_uppercase(), 8)]).unwrap(), vec![8]);
        assert!(g.check(1001, 601001).unwrap_err().contains("#8"));
        assert!(g.check(601000, 700000).is_ok());
        drop(g);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn exclusive_lock_and_account_isolation() {
        let root = root();
        let one = Guard::open_in(&root, 1, 2).unwrap();
        assert!(Guard::open_in(&root, 1, 2).is_err());
        let other = Guard::open_in(&root, 2, 2).unwrap();
        let school = Guard::open_in(&root, 1, 3).unwrap();
        drop((one, other, school));
        assert!(Guard::open_in(&root, 1, 2).is_ok());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn explicit_rejection_allows_a_later_user_retry() {
        let root = root();
        let id = uuid::Uuid::new_v4().to_string();
        let mut g = Guard::open_in(&root, 1, 2).unwrap();
        g.begin(&id, 1000, 2000).unwrap();
        assert!(g
            .begin(&uuid::Uuid::new_v4().to_string(), 1000, 2000)
            .is_err());
        g.rejected(&id).unwrap();
        drop(g);
        assert!(Guard::open_in(&root, 1, 2)
            .unwrap()
            .check(1000, 2000)
            .is_ok());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn torn_journal_fails_closed_and_is_not_overwritten() {
        let root = root();
        let g = Guard::open_in(&root, 1, 2).unwrap();
        drop(g);
        let path = std::fs::read_dir(root.join("submission_receipts"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        std::fs::write(&path, "{\"state\":\"pending\"").unwrap();
        assert!(Guard::open_in(&root, 1, 2).is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\"state\":\"pending\""
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
