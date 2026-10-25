// RaftCLI: An in-memory device for tests
// Answers the FileManager REST commands raft fs uses (filelist, filesection, filehash, filedelete, reformatfs) over a
// file tree held in memory, and takes uploads, so command logic can be tested without a device. It answers at the
// REST level; the RICREST simulator (phase B) will answer at the frame level.
// Rob Dobson 2026

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use super::{CmdResult, DeviceConn, LinkKind};
use crate::app_fs::crc16_ccitt;
use crate::http_client::ProgressFn;

pub(crate) struct FakeConn {
    // "<fs>/<path>" -> contents
    pub(crate) files: RefCell<BTreeMap<String, Vec<u8>>>,
    // REST commands received, in order
    pub(crate) log: RefCell<Vec<String>>,
    link: LinkKind,
}

impl FakeConn {
    pub(crate) fn new(link: LinkKind) -> FakeConn {
        FakeConn { files: RefCell::new(BTreeMap::new()), log: RefCell::new(Vec::new()), link }
    }

    pub(crate) fn add(&self, fs_path: &str, data: &[u8]) {
        self.files.borrow_mut().insert(fs_path.to_string(), data.to_vec());
    }

    pub(crate) fn file(&self, fs_path: &str) -> Option<Vec<u8>> {
        self.files.borrow().get(fs_path).cloned()
    }

    // The commands received that start with `prefix`
    pub(crate) fn commands(&self, prefix: &str) -> Vec<String> {
        self.log.borrow().iter().filter(|c| c.starts_with(prefix)).cloned().collect()
    }

    fn list(&self, fs: &str, folder: &str) -> serde_json::Value {
        let prefix = if folder.is_empty() { format!("{}/", fs) } else { format!("{}/{}/", fs, folder) };
        let files = self.files.borrow();
        if !folder.is_empty() && !files.keys().any(|k| k.starts_with(&prefix)) {
            return serde_json::json!({"rslt": "fail", "error": "nofolder"});
        }
        let mut entries: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        for (key, data) in files.iter() {
            if let Some(rest) = key.strip_prefix(&prefix) {
                match rest.split_once('/') {
                    Some((dir, _)) => {
                        entries.insert(dir.to_string(), serde_json::json!({"name": dir, "size": 0, "isDir": 1}));
                    }
                    None => {
                        entries.insert(rest.to_string(), serde_json::json!({"name": rest, "size": data.len(), "isDir": 0}));
                    }
                }
            }
        }
        let used: usize = files.values().map(|d| d.len()).sum();
        serde_json::json!({
            "rslt": "ok", "fsName": fs, "diskSize": 1_000_000, "diskUsed": used,
            "folder": format!("/{}", folder), "files": entries.into_values().collect::<Vec<_>>()
        })
    }

    pub(crate) fn answer(&self, cmd: &str) -> serde_json::Value {
        let parts: Vec<&str> = cmd.split('/').collect();
        let path = |i: usize| parts.get(i).copied().unwrap_or("").replace('~', "/");
        let fs = parts.get(1).copied().unwrap_or("");
        let key = format!("{}/{}", fs, path(2));
        match parts[0] {
            "filelist" => self.list(fs, path(2).trim_matches('/')),
            "filesection" => match self.file(&key) {
                Some(data) => {
                    let start: usize = parts.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
                    let len: usize = parts.get(4).and_then(|s| s.parse().ok()).unwrap_or(0);
                    let start = start.min(data.len());
                    let end = (start + len).min(data.len());
                    serde_json::json!({"rslt": "ok", "start": start, "len": end - start, "total": data.len(),
                                       "data": base64_encode(&data[start..end])})
                }
                None => serde_json::json!({"rslt": "fail", "error": "nofile"}),
            },
            "filehash" => match self.file(&key) {
                Some(data) => serde_json::json!({"rslt": "ok", "size": data.len(), "crc16": format!("{:04x}", crc16_ccitt(&data))}),
                None => serde_json::json!({"rslt": "fail", "error": "nofile"}),
            },
            "filedelete" => match self.files.borrow_mut().remove(&key) {
                Some(_) => serde_json::json!({"rslt": "ok"}),
                None => serde_json::json!({"rslt": "fail", "error": "nofile"}),
            },
            "reformatfs" => {
                self.files.borrow_mut().retain(|k, _| !k.starts_with(&format!("{}/", fs)));
                serde_json::json!({"rslt": "ok"})
            }
            _ => serde_json::json!({"rslt": "fail", "error": "failUnknownAPI"}),
        }
    }
}

impl DeviceConn for FakeConn {
    fn rest_raw(&self, cmd: &str) -> CmdResult<Vec<u8>> {
        self.log.borrow_mut().push(cmd.to_string());
        Ok(self.answer(cmd).to_string().into_bytes())
    }

    fn put_file(&self, local: &Path, remote: &str, fs: &str, progress: &mut ProgressFn) -> CmdResult<()> {
        let data = std::fs::read(local)?;
        progress(data.len() as u64, data.len() as u64, 0.0);
        self.log.borrow_mut().push(format!("put {}/{}", fs, remote));
        self.add(&format!("{}/{}", fs, remote.trim_start_matches('/')), &data);
        Ok(())
    }

    fn wait_for_restart(&self, _timeout: Duration) -> bool {
        true
    }

    fn describe(&self) -> &str {
        "fake"
    }

    fn link_kind(&self) -> LinkKind {
        self.link
    }
}

fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}
