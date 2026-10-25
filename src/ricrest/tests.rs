// RaftCLI: RICREST tests against the simulated device (src/link/sim.rs)

use crate::app_fs::crc16_ccitt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::conn::RicrestConn;
use crate::device_conn::{CmdResult, DeviceConn, LinkKind};
use crate::link::sim::SimulatedLink;
use crate::link::{Link, WireFormat};

fn sim(kind: LinkKind) -> RicrestConn<SimulatedLink> {
    let format = if kind == LinkKind::Serial { WireFormat::OverAsciiHdlc } else { WireFormat::Hdlc };
    RicrestConn::new(SimulatedLink::new(kind, format), "sim", kind, false)
}

fn data(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 7 + i / 251) as u8).collect()
}

// Upload `bytes` as local/<name> through a temporary local file (unique, as tests run in parallel)
fn put(conn: &RicrestConn<SimulatedLink>, name: &str, bytes: &[u8]) -> CmdResult<()> {
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    let n = COUNT.fetch_add(1, Ordering::Relaxed);
    let local: PathBuf = std::env::temp_dir().join(format!("raftcli_ricrest_{}_{}_{}", std::process::id(), n, name.replace('/', "_")));
    std::fs::write(&local, bytes)?;
    let result = conn.put_file(&local, name, "local", &mut |_, _, _| {});
    let _ = std::fs::remove_file(&local);
    result
}

fn stored(conn: &RicrestConn<SimulatedLink>, key: &str) -> Option<Vec<u8>> {
    conn.session().link().device.store.file(key)
}

fn received(conn: &RicrestConn<SimulatedLink>, prefix: &str) -> Vec<String> {
    conn.session().link().device.received.iter().filter(|r| r.starts_with(prefix)).cloned().collect()
}

#[test]
fn rest_commands_on_every_link() {
    for kind in [LinkKind::Serial, LinkKind::WebSocket, LinkKind::Ble] {
        let conn = sim(kind);
        conn.session_mut().link_mut().device.store.add("local/a.txt", b"hello");
        let list = conn.rest_ok("filelist/local/").unwrap();
        assert_eq!(list["files"][0]["name"], "a.txt", "{:?}", kind);
        assert_eq!(list["req"], "filelist/local/");
        let err = conn.rest_ok("filehash/local/missing.txt").unwrap_err().to_string();
        assert!(err.contains("nofile"), "{}", err);
    }
}

#[test]
fn uploads_use_the_devices_block_size() {
    // Serial blocks are cut to fit 900 bytes on the wire (562 bytes of this data)
    for (kind, max_block) in [(LinkKind::Serial, 562), (LinkKind::WebSocket, 800), (LinkKind::Ble, 333)] {
        let conn = sim(kind);
        let bytes = data(5000);
        put(&conn, "web/app.js", &bytes).unwrap();
        assert_eq!(stored(&conn, "local/web/app.js").unwrap(), bytes, "{:?}", kind);
        let blocks = received(&conn, "block ");
        let sizes: Vec<usize> = blocks.iter().map(|b| b.rsplit(' ').next().unwrap().parse().unwrap()).collect();
        assert_eq!(sizes.iter().max(), Some(&max_block), "{:?}", kind);
        assert_eq!(sizes.iter().sum::<usize>(), 5000, "each block sent once: {:?}", blocks);
        // The file system is named in the upload, with the file's CRC, which the device checks (so no filehash)
        let start = &received(&conn, r#"{"cmdName":"ufStart""#)[0];
        assert!(start.contains(r#""fileName":"/local/web/app.js""#), "{}", start);
        assert!(start.contains(&format!(r#""CRC16":"0x{:04x}""#, crc16_ccitt(&bytes))), "{}", start);
        assert_eq!(received(&conn, "filehash/").len(), 0);
        assert!(!conn.session().link().device.uploading());
    }
}

#[test]
fn uploads_to_older_firmware_are_checked_with_filehash() {
    // Firmware before 2026-10-25 ignores the CRC in ufStart (no crc16 in the ufEnd reply)
    let conn = sim(LinkKind::WebSocket);
    conn.session_mut().link_mut().device.check_crc = false;
    let bytes = data(5000);
    put(&conn, "web/app.js", &bytes).unwrap();
    assert_eq!(stored(&conn, "local/web/app.js").unwrap(), bytes);
    assert_eq!(received(&conn, "filehash/local/web~app.js").len(), 1);
}

// Upload 200 blocks (WebSocket: 800-byte blocks, batches of 10) with blocks at these positions (in 100s of bytes)
// lost; returns (blocks sent, timer acks the device needed to send)
fn upload_with_losses(ack_out_of_sequence: bool, lose: &[u32]) -> (usize, u32) {
    let conn = sim(LinkKind::WebSocket);
    {
        let mut session = conn.session_mut();
        let device = &mut session.link_mut().device;
        device.ack_out_of_sequence = ack_out_of_sequence;
        device.faults.lose_blocks_at = lose.iter().map(|p| p * 8).collect();
    }
    let bytes = data(200 * 800);
    put(&conn, "big.bin", &bytes).unwrap();
    assert_eq!(stored(&conn, "local/big.bin").unwrap(), bytes);
    let timer_acks = conn.session().link().device.timer_acks;
    (received(&conn, "block ").len(), timer_acks)
}

const MID_BATCH: [u32; 6] = [2300, 5400, 8600, 11100, 14700, 17200];
const BATCH_START: [u32; 6] = [2000, 5000, 8000, 11000, 14000, 17000];

#[test]
fn lost_blocks_with_current_firmware() {
    assert_eq!(upload_with_losses(true, &[]), (200, 0));
    // The block after a lost one is acked at once, so there's no wait for the timer, and little is sent twice
    let (sent, timer_acks) = upload_with_losses(true, &MID_BATCH);
    assert!(sent <= 200 + 6 * 10 && timer_acks == 0, "{} sent, {} timer acks", sent, timer_acks);
    // A lost block at the end of a batch has no block after it: the timer acks
    let (sent, timer_acks) = upload_with_losses(true, &BATCH_START);
    assert!(sent <= 200 + 6 * 10 && timer_acks <= 1, "{} sent, {} timer acks", sent, timer_acks);
    // The last block lost: one timer ack, and only that block sent again
    assert_eq!(upload_with_losses(true, &[19900]), (201, 1));
}

#[test]
fn lost_blocks_with_earlier_firmware() {
    // Its acks fall out of step with the batches after a loss, so more is sent twice, but it doesn't stall
    assert_eq!(upload_with_losses(false, &[]), (200, 0));
    for lose in [&MID_BATCH, &BATCH_START] {
        let (sent, timer_acks) = upload_with_losses(false, lose);
        assert!(sent <= 2 * 200 && timer_acks <= 1, "{} sent, {} timer acks", sent, timer_acks);
    }
    assert_eq!(upload_with_losses(false, &[19900]), (201, 1));
}

#[test]
fn a_device_cancel_fails_the_upload() {
    let conn = sim(LinkKind::Ble);
    conn.session_mut().link_mut().device.faults.cancel_after_blocks = Some(3);
    let err = put(&conn, "x.bin", &data(3000)).unwrap_err().to_string();
    assert!(err.contains("cancelled") && err.contains("failRetries"), "{}", err);
    assert!(stored(&conn, "local/x.bin").is_none());
    // The next upload starts cleanly
    conn.session_mut().link_mut().device.faults.cancel_after_blocks = None;
    put(&conn, "x.bin", &data(3000)).unwrap();
}

#[test]
fn a_silent_device_is_given_up_on_and_the_upload_cancelled() {
    let conn = sim(LinkKind::WebSocket);
    conn.session_mut().link_mut().device.faults.lose_all_blocks = true;
    let err = put(&conn, "x.bin", &data(2000)).unwrap_err().to_string();
    assert!(err.contains("stopped at byte 0"), "{}", err);
    assert_eq!(received(&conn, r#"{"cmdName":"ufCancel""#).len(), 1);
    assert!(!conn.session().link().device.uploading(), "the cancel ended the device's transfer");
    // So the next upload doesn't find a transfer in progress
    conn.session_mut().link_mut().device.faults.lose_all_blocks = false;
    put(&conn, "x.bin", &data(2000)).unwrap();
}

#[test]
fn a_damaged_upload_is_removed() {
    // Firmware that checks the CRC cancels the upload (failCRC) and doesn't keep the file
    let conn = sim(LinkKind::Serial);
    conn.session_mut().link_mut().device.faults.damage_uploads = true;
    let err = put(&conn, "x.bin", &data(1000)).unwrap_err().to_string();
    assert!(err.contains("didn't match"), "{}", err);
    assert!(stored(&conn, "local/x.bin").is_none());

    // Older firmware keeps it: filehash finds the damage and the file is deleted
    let conn = sim(LinkKind::Serial);
    conn.session_mut().link_mut().device.check_crc = false;
    conn.session_mut().link_mut().device.faults.damage_uploads = true;
    let err = put(&conn, "x.bin", &data(1000)).unwrap_err().to_string();
    assert!(err.contains("arrived damaged"), "{}", err);
    assert!(stored(&conn, "local/x.bin").is_none());
}

#[test]
fn an_empty_file_uploads() {
    let conn = sim(LinkKind::Serial);
    put(&conn, "empty.txt", b"").unwrap();
    assert_eq!(stored(&conn, "local/empty.txt").unwrap(), Vec::<u8>::new());
}

#[test]
fn a_lost_reply_is_retried() {
    let conn = sim(LinkKind::Ble);
    conn.session_mut().link_mut().device.faults.lose_reply_to = Some("filelist".to_string());
    conn.rest_ok("filelist/local/").unwrap();
    assert_eq!(received(&conn, "filelist").len(), 2);
}

#[test]
fn duplicate_replies_are_ignored() {
    let conn = sim(LinkKind::WebSocket);
    conn.session_mut().link_mut().device.store.add("local/a.txt", b"a");
    conn.session_mut().link_mut().device.store.add("local/b.txt", b"bb");
    conn.session_mut().link_mut().device.faults.duplicate_replies = true;
    assert_eq!(conn.rest_ok("filehash/local/a.txt").unwrap()["size"], 1);
    assert_eq!(conn.rest_ok("filehash/local/b.txt").unwrap()["size"], 2);
    assert!(conn.session().stale_responses >= 1);
}

#[test]
fn a_corrupted_reply_is_retried() {
    for kind in [LinkKind::Serial, LinkKind::Ble] {
        let conn = sim(kind);
        conn.session_mut().link_mut().device.faults.corrupt_next_reply = true;
        conn.rest_ok("filelist/local/").unwrap();
        assert!(conn.session().link().bad_frames() >= 1, "{:?}", kind);
        assert_eq!(received(&conn, "filelist").len(), 2, "{:?}", kind);
    }
}

#[test]
fn log_text_between_and_inside_frames() {
    let conn = sim(LinkKind::Serial);
    conn.session_mut().link_mut().device.faults.log_text = true;
    let bytes = data(4000);
    put(&conn, "x.bin", &bytes).unwrap();
    assert_eq!(stored(&conn, "local/x.bin").unwrap(), bytes);
    assert_eq!(conn.session().link().bad_frames(), 0);
    let text = String::from_utf8_lossy(&conn.session().link().text).to_string();
    assert!(text.contains("SysMan: status") && text.contains("Mid-frame log"), "{}", text);
}

#[test]
fn message_numbers_wrap_past_255_without_using_0() {
    let conn = sim(LinkKind::WebSocket);
    for _ in 0..600 {
        conn.rest_ok("filelist/local/").unwrap();
    }
    let nums = conn.session().link().device.url_msg_nums.clone();
    assert_eq!(nums.len(), 600);
    assert!(nums.iter().all(|&n| n != 0));
    assert_eq!((nums[0], nums[254], nums[255]), (1, 255, 1));
}


// The device's serial receive buffer (1024 bytes by default) drops a frame that doesn't fit: serial blocks are cut to
// fit, whatever the data (bytes that take two on the wire, or four: E7 and D7 are escaped twice)
#[test]
fn serial_blocks_fit_the_devices_receive_buffer() {
    let patterns: [(&str, Vec<u8>); 4] = [
        ("random", data(20000)),
        ("zeros", vec![0u8; 20000]),
        ("0xff", vec![0xffu8; 20000]),
        ("0xe7", vec![0xe7u8; 20000]),
    ];
    for (name, bytes) in patterns {
        let conn = sim(LinkKind::Serial);
        conn.session_mut().link_mut().device.faults.max_wire_len = Some(1024);
        put(&conn, "x.bin", &bytes).unwrap();
        assert_eq!(stored(&conn, "local/x.bin").unwrap(), bytes, "{}", name);
        let session = conn.session();
        let device = &session.link().device;
        assert_eq!((device.frames_too_long, device.timer_acks), (0, 0), "{}: frames lost, timer acks", name);
    }
}

#[test]
fn an_interrupted_upload_is_cancelled_and_started_again() {
    let conn = sim(LinkKind::Serial);
    conn.session_mut().link_mut().device.abandoned_upload("x.bin", 4000);
    let bytes = data(4000);
    put(&conn, "x.bin", &bytes).unwrap();
    assert_eq!(stored(&conn, "local/x.bin").unwrap(), bytes);
    let cancels = received(&conn, r#"{"cmdName":"ufCancel""#);
    assert_eq!(cancels.len(), 1);
    assert!(cancels[0].contains(r#""fileName":"/local/x.bin""#), "{}", cancels[0]);
    assert_eq!(received(&conn, r#"{"cmdName":"ufStart""#).len(), 2);
}

// RicrestConn over a real WebSocket: a server thread on localhost runs the simulated device behind tungstenite
#[test]
fn commands_over_a_websocket() {
    use crate::link::sim::SimDevice;
    use crate::link::websocket::WebSocketLink;
    use crate::ricrest::frame::Frame;
    use crate::ricrest::hdlc;
    use tungstenite::Message;

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut ws = tungstenite::accept(stream).unwrap();
        let mut device = SimDevice::new(LinkKind::WebSocket);
        device.store.add("local/a.txt", b"hello");
        let mut decoder = hdlc::Decoder::default();
        loop {
            let data = match ws.read() {
                Ok(Message::Binary(data)) => data,
                Ok(Message::Close(_)) | Err(_) => break,
                Ok(_) => continue,
            };
            let mut frames = Vec::new();
            decoder.feed(&data, &mut frames);
            for frame in frames {
                for reply in device.handle(Frame::parse(&frame).unwrap()) {
                    ws.send(Message::Binary(hdlc::encode(&reply).into())).unwrap();
                }
            }
        }
        device.store.file("local/up.bin")
    });

    let link = WebSocketLink::open("127.0.0.1", port, "/ws").unwrap();
    let conn = RicrestConn::new(link, "ws://test", LinkKind::WebSocket, false);
    assert_eq!(conn.rest_ok("filelist/local/").unwrap()["files"][0]["name"], "a.txt");
    let bytes = data(20000);
    let local = std::env::temp_dir().join(format!("raftcli_ws_test_{}", std::process::id()));
    std::fs::write(&local, &bytes).unwrap();
    conn.put_file(&local, "up.bin", "local", &mut |_, _, _| {}).unwrap();
    let _ = std::fs::remove_file(&local);
    let section = conn.rest_ok("filesection/local/up.bin/0/16").unwrap();
    assert_eq!(section["total"], 20000);
    drop(conn);
    assert_eq!(server.join().unwrap().unwrap(), bytes);
}
