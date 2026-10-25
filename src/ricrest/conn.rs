// RaftCLI: DeviceConn over RICREST, on any link
// REST commands go as RICREST URL frames (the same strings as /api/... over HTTP) and the device replies with the
// same JSON. Uploads use the file transfer protocol, then filehash checks the result (the device doesn't).
// Rob Dobson 2026

use std::cell::RefCell;
#[cfg(test)]
use std::cell::{Ref, RefMut};
use std::path::Path;
use std::time::{Duration, Instant};

use super::frame::ElemCode;
use super::session::{RequestPolicy, Session};
use super::upload::{upload, UploadSizes};
use crate::device_conn::{check_uploaded, CmdResult, DeviceConn, LinkKind};
use crate::http_client::ProgressFn;
use crate::link::Link;

// How long to wait for a reply, and how often to send a request
fn request_policy(kind: LinkKind) -> RequestPolicy {
    match kind {
        LinkKind::WebSocket => RequestPolicy { timeout: Duration::from_secs(2), attempts: 3 },
        LinkKind::Serial | LinkKind::Ble | LinkKind::Http => RequestPolicy { timeout: Duration::from_secs(3), attempts: 3 },
    }
}

// Block and batch sizes to ask for (the device clamps the block to 2/3 of the link's inbound block size: 800 on
// serial, 333 on BLE, and on WebSocket 16000 with PSRAM and RaftWebServer from 2026-10, 800 otherwise).
// WebSocket: bigger blocks help where storage is fast (uploads to an SD card: 28 KB/s with 800 byte blocks, 43 with
// 4000, 58 with 16000). 4000 keeps each block's write short, as the device writes it in its main loop.
// Serial: one block at a time, of at most 900 bytes on the wire. SerialConsole's receive buffer is 1024 bytes by
// default (USB Serial/JTAG drops what doesn't fit), and it queues what it reads as messages of a USB packet or so
// each, in a channel queue of 20. Both overflow if more arrives at once, and the blocks are lost. Writing to flash
// takes longer than the transfer anyway, so a block at a time costs little (about 11 KB/s on an ESP32-S3)
const SERIAL_MAX_WIRE_LEN: usize = 900;

fn upload_sizes(kind: LinkKind) -> UploadSizes {
    // For tuning: RAFTCLI_UPLOAD_SIZES=<block>,<batch>[,<most bytes on the wire per block>]
    if let Ok(sizes) = std::env::var("RAFTCLI_UPLOAD_SIZES") {
        let values: Vec<usize> = sizes.split(',').filter_map(|v| v.trim().parse().ok()).collect();
        if values.len() >= 2 {
            return UploadSizes { block: values[0] as u32, batch: values[1] as u32, max_wire_len: values.get(2).copied() };
        }
    }
    match kind {
        LinkKind::Serial => UploadSizes { block: 800, batch: 1, max_wire_len: Some(SERIAL_MAX_WIRE_LEN) },
        LinkKind::WebSocket | LinkKind::Http => UploadSizes { block: 4000, batch: 10, max_wire_len: None },
        LinkKind::Ble => UploadSizes { block: 333, batch: 20, max_wire_len: None },
    }
}

// How long a device may take to go down when told to restart
const RESTART_GOING_DOWN_MAX: Duration = Duration::from_secs(10);

// Formatting a file system takes a while, and the reply may not come at all as the device then restarts
const REFORMAT_POLICY: RequestPolicy = RequestPolicy { timeout: Duration::from_secs(30), attempts: 1 };

pub struct RicrestConn<L: Link> {
    session: RefCell<Session<L>>,
    text: String,
    kind: LinkKind,
    // Print the link statistics when done
    verbose: bool,
}

impl<L: Link> RicrestConn<L> {
    pub fn new(link: L, text: &str, kind: LinkKind, verbose: bool) -> RicrestConn<L> {
        RicrestConn { session: RefCell::new(Session::new(link)), text: text.to_string(), kind, verbose }
    }

    #[cfg(test)]
    pub fn session(&self) -> Ref<'_, Session<L>> {
        self.session.borrow()
    }

    #[cfg(test)]
    pub fn session_mut(&self) -> RefMut<'_, Session<L>> {
        self.session.borrow_mut()
    }
}

impl<L: Link> DeviceConn for RicrestConn<L> {
    fn rest_raw(&self, cmd: &str) -> CmdResult<Vec<u8>> {
        let policy = if cmd.starts_with("reformatfs") { REFORMAT_POLICY } else { request_policy(self.kind) };
        let reply = self.session.borrow_mut().request(ElemCode::Url, cmd, policy)?;
        Ok(reply.text().into_bytes())
    }

    fn put_file(&self, local: &Path, remote: &str, fs: &str, progress: &mut ProgressFn) -> CmdResult<()> {
        let data = std::fs::read(local)?;
        let remote = remote.trim_start_matches('/');
        // The file system is always named, so --fs is honoured (the device prepends its default FS otherwise)
        let file_name = format!("/{}/{}", fs, remote);
        upload(&mut self.session.borrow_mut(), &file_name, &data, upload_sizes(self.kind), request_policy(self.kind), progress)?;

        // Check it arrived intact
        check_uploaded(self, &data, remote, fs)
    }

    fn wait_for_restart(&self, timeout: Duration) -> bool {
        let start = Instant::now();
        let quick = RequestPolicy { timeout: Duration::from_secs(2), attempts: 1 };
        let mut session = self.session.borrow_mut();
        // First see it go down: until then the link may still reach it (it can take a moment to restart), and an
        // answer from before the restart says nothing
        while start.elapsed() < RESTART_GOING_DOWN_MAX {
            if session.request(ElemCode::Url, "v", quick).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        // Then open the link again until it answers (the link can drop again while the device starts up)
        while start.elapsed() < timeout {
            if session.link_mut().reopen(timeout.saturating_sub(start.elapsed())) && session.request(ElemCode::Url, "v", quick).is_ok() {
                return true;
            }
        }
        false
    }

    fn describe(&self) -> &str {
        &self.text
    }

    fn link_kind(&self) -> LinkKind {
        self.kind
    }

    fn link_stats(&self) -> Option<String> {
        let session = self.session.borrow();
        Some(format!(
            "{} frames dropped (bad CRC), {} requests sent again, {} late replies",
            session.link().bad_frames(),
            session.resent,
            session.stale_responses
        ))
    }
}

impl<L: Link> Drop for RicrestConn<L> {
    fn drop(&mut self) {
        if self.verbose {
            if let Some(stats) = self.link_stats() {
                eprintln!("{}: {}", self.text, stats);
            }
        }
    }
}
