// RaftCLI: File upload over RICREST (FileUploadOKTOProtocol on the device, RaftFileHandler.fileSend in raftjs)
//   ufStart -> the device's reply sets the block size, ack batch size and stream ID actually used
//   blocks  -> unnumbered; the first batch is one block, then batches of batchAckSize
//   okto    -> the device acks how far it has accepted (after the first block, each batch and the last block, at
//              once for a block out of sequence (RaftCore from 2026-10-23), and on its timer when blocks stop
//              arriving); each batch is sent from the latest ack (go-back-N)
//   ufEnd   -> done (the device has usually closed the session after the last block)
// On an error after ufStart, ufCancel is sent so the next upload doesn't find the transfer still in progress.
// ufStart gives the file's CRC16. Firmware from 2026-10-25 checks the file against it once written: a mismatch
// cancels the upload (failCRC, the file deleted) and a match is confirmed in the ufEnd reply (crc16). Older firmware
// ignores it, so without that confirmation RicrestConn checks the file with filehash afterwards.
// Rob Dobson 2026

use std::time::{Duration, Instant};

use super::frame::{file_block, ElemCode, Frame, MsgType};
use super::session::{RequestPolicy, Session};
use crate::app_fs::crc16_ccitt;
use crate::device_conn::CmdResult;
use crate::http_client::ProgressFn;
use crate::link::Link;

// How long to wait for an okto: the device sends one on its timer after 5s without blocks (1s once two blocks are in)
const ACK_TIMEOUT: Duration = Duration::from_secs(10);
// Acks (or waits) in a row without progress before giving up (the device itself cancels after 5 timer acks)
const MAX_STALLS: u32 = 6;
// The block position is 24 bits
const MAX_FILE_LEN: usize = 1 << 24;

#[derive(Clone, Copy, Debug)]
pub struct UploadSizes {
    pub block: u32,
    pub batch: u32,
    // The most a block may take on the wire: blocks are cut shorter to fit (data that encodes to two bytes a byte,
    // or more, makes a longer frame)
    pub max_wire_len: Option<usize>,
}

// Upload a file; returns true if the device checked it against its CRC (so it needn't be checked again)
pub fn upload<L: Link>(
    session: &mut Session<L>,
    file_name: &str,
    data: &[u8],
    sizes: UploadSizes,
    policy: RequestPolicy,
    progress: &mut ProgressFn,
) -> CmdResult<bool> {
    if data.len() >= MAX_FILE_LEN {
        return Err(format!("{} is too big to upload this way (16 MB or more)", file_name).into());
    }
    let name_json = serde_json::to_string(file_name)?;
    let crc = crc16_ccitt(data);
    let start = format!(
        r#"{{"cmdName":"ufStart","reqStr":"fileupload","fileType":"fs","fileName":{},"fileLen":{},"batchMsgSize":{},"batchAckSize":{},"CRC16":"0x{:04x}"}}"#,
        name_json,
        data.len(),
        sizes.block,
        sizes.batch,
        crc
    );
    let mut reply = command(session, &start, policy)?;
    // An earlier upload of this file was interrupted (Ctrl-C, a lost link) and the device is still waiting for it
    // (until 10s without blocks): cancel it - found by its name - and start again
    if reply["error"].as_str() == Some("transferInProgress") {
        let cancel = format!(r#"{{"cmdName":"ufCancel","fileName":{}}}"#, name_json);
        let _ = command(session, &cancel, RequestPolicy { timeout: Duration::from_secs(2), attempts: 1 });
        reply = command(session, &start, policy)?;
    }
    if reply["rslt"].as_str() != Some("ok") {
        let error = reply["error"].as_str().unwrap_or(reply["rslt"].as_str().unwrap_or("?"));
        return Err(format!("the device didn't start the upload of {}: {}", file_name, error).into());
    }
    // Always the values the device chose
    let block = reply["batchMsgSize"].as_u64().filter(|&b| b > 0).unwrap_or(sizes.block as u64) as usize;
    let batch = reply["batchAckSize"].as_u64().filter(|&b| b > 0).unwrap_or(sizes.batch as u64) as usize;
    let stream_id = reply["streamID"].as_u64().filter(|&id| id <= 0xFF).unwrap_or(0) as u8;

    if let Err(e) = send_blocks(session, data, block, batch, sizes.max_wire_len, stream_id, progress) {
        let cancel = format!(r#"{{"cmdName":"ufCancel","streamID":{}}}"#, stream_id);
        let _ = command(session, &cancel, RequestPolicy { timeout: Duration::from_secs(2), attempts: 1 });
        return Err(e);
    }

    let end = format!(
        r#"{{"cmdName":"ufEnd","reqStr":"fileupload","fileType":"fs","fileName":{},"fileLen":{},"streamID":{}}}"#,
        name_json,
        data.len(),
        stream_id
    );
    let reply = command(session, &end, policy)?;
    if reply["rslt"].as_str() != Some("ok") {
        return Err(format!("the device didn't finish the upload of {}: {}", file_name, reply).into());
    }
    // The device checked the file against the CRC given (firmware from 2026-10-25)
    let checked = reply["crc16"].as_str().and_then(|c| u16::from_str_radix(c, 16).ok()) == Some(crc);
    Ok(checked)
}

// The error for a reason the device gave for cancelling or failing an upload
fn upload_failed(reason: &str, cancelled: bool) -> String {
    match reason {
        "failCRC" | "CRCMismatch" => {
            "the file the device wrote didn't match what was sent (CRC check), so it deleted it - try again".to_string()
        }
        _ if cancelled => format!("the device cancelled the upload ({})", reason),
        _ => format!("the device failed to write the file ({})", reason),
    }
}

// A JSON command frame and its JSON reply
fn command<L: Link>(session: &mut Session<L>, json: &str, policy: RequestPolicy) -> CmdResult<serde_json::Value> {
    let reply = session.request(ElemCode::CommandFrame, json, policy)?;
    reply.json().ok_or_else(|| format!("invalid reply from the device: {}", reply.text()).into())
}

// An unnumbered upload status from the device: an okto, a cancel, or a failure to write a block
fn upload_status(frame: &Frame) -> Option<serde_json::Value> {
    if frame.msg_type != MsgType::Response || frame.msg_num != 0 {
        return None;
    }
    let json = frame.json()?;
    let is_status = json.get("okto").is_some()
        || json["cmdName"].as_str() == Some("ufCancel")
        || json["cmdName"].as_str() == Some("ufStatus");
    is_status.then_some(json)
}

fn send_blocks<L: Link>(
    session: &mut Session<L>,
    data: &[u8],
    block: usize,
    batch: usize,
    max_wire_len: Option<usize>,
    stream_id: u8,
    progress: &mut ProgressFn,
) -> CmdResult<()> {
    let start = Instant::now();
    // What the device has acked
    let mut acked = 0usize;
    let mut stalls = 0u32;
    progress(0, data.len() as u64, 0.0);
    while acked < data.len() {
        // Each batch starts at the latest ack. The first is a single block (the device may take a while over the
        // first one). Carrying on past an ack instead resends fewer blocks with older firmware, but with current
        // firmware, which acks a block out of sequence at once, it sends blocks the device then drops without an
        // ack, and waits for its timer ack (1s) after every lost block
        let count = if acked == 0 { 1 } else { batch };
        let mut pos = acked;
        for _ in 0..count {
            if pos >= data.len() {
                break;
            }
            let mut end = (pos + block).min(data.len());
            let mut frame = file_block(stream_id, pos as u32, &data[pos..end]);
            if let Some(max) = max_wire_len {
                // Shorten in proportion until it fits (a few tries at most)
                loop {
                    let len = session.link().wire_len(&frame);
                    if len <= max || end - pos <= 16 {
                        break;
                    }
                    end = pos + ((end - pos) * max / len).clamp(16, end - pos - 1);
                    frame = file_block(stream_id, pos as u32, &data[pos..end]);
                }
            }
            session.send(&frame)?;
            pos = end;
        }

        // Wait for an okto, then take any more already received: the furthest position is the device's latest
        let mut okto: Option<usize> = None;
        let mut timeout = ACK_TIMEOUT;
        while let Some(frame) = session.wait_for(|f| upload_status(f).is_some(), timeout)? {
            let json = upload_status(&frame).unwrap_or_default();
            if json["cmdName"].as_str() == Some("ufCancel") {
                return Err(upload_failed(json["reason"].as_str().unwrap_or("no reason"), true).into());
            }
            if json["rslt"].as_str() == Some("fail") {
                return Err(upload_failed(json["reason"].as_str().unwrap_or("no reason"), false).into());
            }
            if let Some(pos) = json["okto"].as_u64() {
                let pos = (pos as usize).min(data.len());
                okto = Some(okto.map_or(pos, |o| o.max(pos)));
            }
            // An ack behind the acked position is a late one: keep waiting for the reply to these blocks
            if okto.is_some_and(|o| o >= acked) {
                timeout = Duration::ZERO;
            }
        }
        match okto {
            Some(okto) if okto > acked => {
                acked = okto;
                stalls = 0;
                let secs = start.elapsed().as_secs_f64();
                progress(acked as u64, data.len() as u64, if secs > 0.0 { acked as f64 / secs } else { 0.0 });
            }
            // No ack, or one without progress: send again from the acked position
            _ => {
                stalls += 1;
                if stalls >= MAX_STALLS {
                    return Err(format!("the upload stopped at byte {} of {} (no progress from the device)", acked, data.len()).into());
                }
            }
        }
    }
    Ok(())
}
