// RaftCLI: A simulated device on a simulated link, for tests
// Frames go through the real wire encoding in both directions (HDLC, or OverAscii(HDLC) as on a serial port), into a
// device that answers REST commands from FakeConn's file tree and runs the upload protocol as
// FileUploadOKTOProtocol does - batch counting, the first-block ack, timer acks when blocks stop, and cancels -
// with the rules of current RaftCore (a block out of sequence acked at once) or of earlier versions. Every ack is
// {"req":"ufBlock","rslt":"ok","okto":N}, whatever prompted it, as on the device.
// Faults can be switched on: lost blocks and replies, duplicate replies, corrupted bytes, log text between frames, a
// device-side cancel, a damaged upload.
// Time doesn't pass: when the host waits and nothing is due, the device acts as its timer would (a timer ack, or a
// cancel after 5), and otherwise the wait ends at once as a timeout.
// Rob Dobson 2026

use std::collections::VecDeque;
use std::time::Duration;

use super::{encode_wire, Link, WireDecoder, WireFormat};
use crate::device_conn::fake::FakeConn;
use crate::device_conn::{CmdResult, LinkKind};
use crate::ricrest::frame::{text_frame, ElemCode, Frame, MsgType};

const MAX_TOTAL_BYTES_IN_BATCH: u32 = 50000;
const MAX_TIMER_ACKS: u32 = 5;

struct Upload {
    key: String,
    len: usize,
    data: Vec<u8>,
    batch: u32,
    batch_count: u32,
    block_count: u32,
    stream_id: u8,
    timer_acks: u32,
}

#[derive(Default)]
pub(crate) struct Faults {
    // The first block sent at each of these file positions is lost
    pub(crate) lose_blocks_at: Vec<u32>,
    // Every block is lost (and the device is silent while it waits)
    pub(crate) lose_all_blocks: bool,
    // The first reply to a URL starting with this is lost
    pub(crate) lose_reply_to: Option<String>,
    // Every numbered reply is sent twice
    pub(crate) duplicate_replies: bool,
    // One byte of the next reply is changed on the wire
    pub(crate) corrupt_next_reply: bool,
    // Log text before and inside every reply (OverAscii only)
    pub(crate) log_text: bool,
    // The device cancels the upload when this many blocks have arrived
    pub(crate) cancel_after_blocks: Option<u32>,
    // Uploaded files are stored with a byte changed
    pub(crate) damage_uploads: bool,
    // Frames longer than this on the wire are lost on the way in (a serial port's receive buffer)
    pub(crate) max_wire_len: Option<usize>,
}

pub(crate) struct SimDevice {
    pub(crate) store: FakeConn,
    pub(crate) faults: Faults,
    // RaftCore from 2026-10-23: the first block out of sequence is acked at once, blocks out of sequence don't
    // count towards the batch, and a timer ack starts the count again. Earlier versions count every block, which
    // puts the acks out of step with the host's batches after a loss (and, with the second block lost, ack every
    // block after it)
    pub(crate) ack_out_of_sequence: bool,
    out_of_sequence_acked: bool,
    // Timer acks sent (each one is 1s, or 5s before the second block, that a device would have waited)
    pub(crate) timer_acks: u32,
    // The link's inbound block size (1200 on serial and WebSocket, 500 on BLE); uploads clamp blocks to 2/3 of it
    chan_block_max: u32,
    upload: Option<Upload>,
    next_stream_id: u8,
    // Commands received, as text: URLs, command frames' JSON, "block <pos> <len>"
    pub(crate) received: Vec<String>,
    // The message number of each URL request
    pub(crate) url_msg_nums: Vec<u8>,
    // Frames lost for being longer than faults.max_wire_len
    pub(crate) frames_too_long: u32,
}

fn reply(msg_num: u8, json: serde_json::Value) -> Vec<u8> {
    text_frame(msg_num, MsgType::Response, ElemCode::CmdRespJson, &json.to_string())
}

impl SimDevice {
    pub(crate) fn new(kind: LinkKind) -> SimDevice {
        SimDevice {
            store: FakeConn::new(kind),
            faults: Faults::default(),
            ack_out_of_sequence: true,
            out_of_sequence_acked: false,
            timer_acks: 0,
            chan_block_max: if kind == LinkKind::Ble { 500 } else { 1200 },
            upload: None,
            next_stream_id: 1,
            received: Vec::new(),
            url_msg_nums: Vec::new(),
            frames_too_long: 0,
        }
    }

    pub(crate) fn uploading(&self) -> bool {
        self.upload.is_some()
    }

    // An upload of local/<name> left part-way through (the host was interrupted)
    pub(crate) fn abandoned_upload(&mut self, name: &str, len: usize) {
        self.upload = Some(Upload {
            key: format!("local/{}", name),
            len,
            data: vec![0; len / 2],
            batch: 10,
            batch_count: 0,
            block_count: 1,
            stream_id: self.next_stream_id,
            timer_acks: 0,
        });
        self.next_stream_id += 1;
    }

    pub(crate) fn handle(&mut self, frame: Frame) -> Vec<Vec<u8>> {
        if frame.msg_type != MsgType::Command {
            return vec![];
        }
        match frame.elem {
            Some(e) if e == ElemCode::Url as u8 => {
                let url = frame.text();
                self.received.push(url.clone());
                self.url_msg_nums.push(frame.msg_num);
                if let Some(prefix) = &self.faults.lose_reply_to {
                    if url.starts_with(prefix.as_str()) {
                        self.faults.lose_reply_to = None;
                        return vec![];
                    }
                }
                let mut json = self.store.answer(&url);
                json["req"] = serde_json::Value::String(url);
                let out = reply(frame.msg_num, json);
                if self.faults.duplicate_replies {
                    vec![out.clone(), out]
                } else {
                    vec![out]
                }
            }
            Some(e) if e == ElemCode::CommandFrame as u8 => {
                let text = frame.text();
                self.received.push(text.clone());
                let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                self.command_frame(frame.msg_num, &json)
            }
            Some(e) if e == ElemCode::FileBlock as u8 => self.file_block(&frame.body),
            _ => vec![reply(frame.msg_num, serde_json::json!({"rslt": "fail", "error": "unknownElem"}))],
        }
    }

    fn command_frame(&mut self, msg_num: u8, json: &serde_json::Value) -> Vec<Vec<u8>> {
        match json["cmdName"].as_str().unwrap_or("") {
            "ufStart" => {
                if self.upload.as_ref().map(|u| !u.data.is_empty()).unwrap_or(false) {
                    return vec![reply(msg_num, serde_json::json!({"rslt": "fail", "error": "transferInProgress"}))];
                }
                let block = (json["batchMsgSize"].as_u64().unwrap_or(5000) as u32).clamp(1, self.chan_block_max * 2 / 3);
                let mut batch = json["batchAckSize"].as_u64().unwrap_or(40).max(1) as u32;
                if block * batch > MAX_TOTAL_BYTES_IN_BATCH {
                    batch = (MAX_TOTAL_BYTES_IN_BATCH / block).max(1);
                }
                // An absolute path names its file system; anything else goes on the default one
                let name = json["fileName"].as_str().unwrap_or("").trim_start_matches('/');
                let key = if name.starts_with("local/") || name.starts_with("sd/") { name.to_string() } else { format!("local/{}", name) };
                self.out_of_sequence_acked = false;
                let stream_id = self.next_stream_id;
                self.next_stream_id = self.next_stream_id.wrapping_add(1).max(1);
                self.upload = Some(Upload {
                    key,
                    len: json["fileLen"].as_u64().unwrap_or(0) as usize,
                    data: Vec::new(),
                    batch,
                    batch_count: 0,
                    block_count: 0,
                    stream_id,
                    timer_acks: 0,
                });
                vec![reply(msg_num, serde_json::json!({"rslt": "ok", "batchMsgSize": block, "batchAckSize": batch, "streamID": stream_id}))]
            }
            "ufEnd" => {
                // An empty file has no blocks, so it is written here
                if let Some(upload) = self.upload.take() {
                    if upload.len == 0 {
                        self.store.add(&upload.key, &[]);
                    }
                }
                vec![reply(msg_num, serde_json::json!({"rslt": "ok"}))]
            }
            "ufCancel" => {
                // Found by stream ID, or by file name if there isn't one (as ProtocolExchange does)
                let name = json["fileName"].as_str().map(|n| n.trim_start_matches('/').to_string());
                let matches = self.upload.as_ref().is_some_and(|u| match (json["streamID"].as_u64(), &name) {
                    (Some(id), _) => id == u.stream_id as u64,
                    (None, Some(name)) => &u.key == name,
                    (None, None) => true,
                });
                let cancelled = matches && self.upload.take().is_some();
                let mut out = vec![reply(msg_num, serde_json::json!({"rslt": "ok"}))];
                if cancelled {
                    out.push(reply(0, serde_json::json!({"req": "", "rslt": "ok", "cmdName": "ufCancel", "reason": ""})));
                }
                out
            }
            _ => vec![reply(msg_num, serde_json::json!({"rslt": "fail", "error": "unknownCmd"}))],
        }
    }

    fn file_block(&mut self, body: &[u8]) -> Vec<Vec<u8>> {
        if body.len() < 4 {
            return vec![];
        }
        let pos = ((body[1] as u32) << 16) | ((body[2] as u32) << 8) | body[3] as u32;
        let data = &body[4..];
        self.received.push(format!("block {} {}", pos, data.len()));
        if self.faults.lose_all_blocks {
            return vec![];
        }
        if let Some(i) = self.faults.lose_blocks_at.iter().position(|&p| p == pos) {
            self.faults.lose_blocks_at.remove(i);
            return vec![];
        }
        let Some(upload) = self.upload.as_mut() else {
            return vec![];
        };
        if body[0] != upload.stream_id {
            return vec![];
        }
        // As FileUploadOKTOProtocol::validateRxBlock
        upload.timer_acks = 0;
        let in_sequence = pos as usize == upload.data.len();
        if self.ack_out_of_sequence {
            if !in_sequence {
                let mut out = Vec::new();
                if !self.out_of_sequence_acked {
                    self.out_of_sequence_acked = true;
                    upload.batch_count = 0;
                    out.push(reply(0, serde_json::json!({"req": "ufBlock", "rslt": "ok", "okto": upload.data.len()})));
                }
                return out;
            }
            self.out_of_sequence_acked = false;
        }
        // Earlier versions: a block at the wrong position is dropped but still counts
        upload.batch_count += 1;
        let mut is_final = false;
        if in_sequence {
            upload.data.extend_from_slice(data);
            upload.block_count += 1;
            is_final = upload.data.len() >= upload.len;
        }
        let blocks_in = upload.block_count;
        let ack = upload.batch_count == upload.batch || upload.block_count == 1 || is_final;
        if ack {
            upload.batch_count = 0;
        }
        let okto = upload.data.len();
        let mut out = Vec::new();
        if ack {
            out.push(reply(0, serde_json::json!({"req": "ufBlock", "rslt": "ok", "okto": okto})));
        }
        if is_final {
            let mut upload = self.upload.take().unwrap();
            if self.faults.damage_uploads {
                upload.data[0] ^= 0xFF;
            }
            self.store.add(&upload.key, &upload.data);
        } else if Some(blocks_in) == self.faults.cancel_after_blocks {
            self.upload = None;
            out.push(reply(0, serde_json::json!({"req": "", "rslt": "ok", "cmdName": "ufCancel", "reason": "failRetries"})));
        }
        out
    }

    // The host is waiting and nothing is due: what the device's timer does
    fn idle(&mut self) -> Vec<Vec<u8>> {
        if self.faults.lose_all_blocks {
            return vec![];
        }
        let Some(upload) = self.upload.as_mut() else {
            return vec![];
        };
        upload.timer_acks += 1;
        if upload.timer_acks < MAX_TIMER_ACKS {
            self.timer_acks += 1;
            if self.ack_out_of_sequence {
                upload.batch_count = 0;
            }
            vec![reply(0, serde_json::json!({"req": "ufBlock", "rslt": "ok", "okto": upload.data.len()}))]
        } else {
            self.upload = None;
            vec![reply(0, serde_json::json!({"req": "", "rslt": "ok", "cmdName": "ufCancel", "reason": "failRetries"}))]
        }
    }
}

pub(crate) struct SimulatedLink {
    pub(crate) device: SimDevice,
    format: WireFormat,
    to_device: WireDecoder,
    to_host: WireDecoder,
    inbox: VecDeque<Vec<u8>>,
    // Text received between frames (log lines on a console port)
    pub(crate) text: Vec<u8>,
    pub(crate) frames_sent: u32,
}

impl SimulatedLink {
    pub(crate) fn new(kind: LinkKind, format: WireFormat) -> SimulatedLink {
        SimulatedLink {
            device: SimDevice::new(kind),
            format,
            to_device: WireDecoder::new(format),
            to_host: WireDecoder::new(format),
            inbox: VecDeque::new(),
            text: Vec::new(),
            frames_sent: 0,
        }
    }

    fn deliver(&mut self, replies: Vec<Vec<u8>>) {
        for frame in replies {
            let mut wire = encode_wire(self.format, &frame);
            if self.device.faults.corrupt_next_reply {
                self.device.faults.corrupt_next_reply = false;
                let i = wire.len() / 2;
                // Keep the top bit on a console port, so the byte is still taken as frame data
                wire[i] ^= 0x01;
            }
            if self.device.faults.log_text && self.format == WireFormat::OverAsciiHdlc {
                let mid = wire.len() / 2;
                let mut with_text = b"I (1234) SysMan: status\r\n".to_vec();
                with_text.extend_from_slice(&wire[..mid]);
                with_text.extend_from_slice(b"W (1235) Mid-frame log\r\n");
                with_text.extend_from_slice(&wire[mid..]);
                wire = with_text;
            }
            let mut frames = Vec::new();
            self.to_host.feed(&wire, &mut frames, &mut self.text);
            self.inbox.extend(frames);
        }
    }
}

impl Link for SimulatedLink {
    fn send(&mut self, frame: &[u8]) -> CmdResult<()> {
        self.frames_sent += 1;
        let wire = encode_wire(self.format, frame);
        if self.device.faults.max_wire_len.is_some_and(|max| wire.len() > max) {
            self.device.frames_too_long += 1;
            return Ok(());
        }
        let (mut frames, mut text) = (Vec::new(), Vec::new());
        self.to_device.feed(&wire, &mut frames, &mut text);
        for bytes in frames {
            if let Some(frame) = Frame::parse(&bytes) {
                let replies = self.device.handle(frame);
                self.deliver(replies);
            }
        }
        Ok(())
    }

    fn recv(&mut self, timeout: Duration) -> CmdResult<Option<Vec<u8>>> {
        // The device's timer only fires when the host actually waits
        if self.inbox.is_empty() && !timeout.is_zero() {
            let replies = self.device.idle();
            self.deliver(replies);
        }
        Ok(self.inbox.pop_front())
    }

    fn wire_len(&self, frame: &[u8]) -> usize {
        encode_wire(self.format, frame).len()
    }

    fn bad_frames(&self) -> u32 {
        self.to_host.bad_frames()
    }

    fn reopen(&mut self, _timeout: Duration) -> bool {
        true
    }
}
