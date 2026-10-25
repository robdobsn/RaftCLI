// RaftCLI: Links that carry RICREST frames to and from a device
// A link takes and returns whole RICREST frames and does its own wire encoding: HDLC on WebSocket and BLE,
// OverAscii(HDLC) on a serial console port (where frames share the port with log text). The simulator is for
// tests.
// Rob Dobson 2026

use std::time::Duration;

use crate::device_conn::CmdResult;
use crate::ricrest::{hdlc, overascii};

#[cfg(feature = "ble")]
pub mod ble;
pub mod serial;
pub mod websocket;
#[cfg(test)]
pub(crate) mod sim;

pub trait Link {
    /// Send a RICREST frame (the link adds its wire encoding)
    fn send(&mut self, frame: &[u8]) -> CmdResult<()>;
    /// The next RICREST frame received, waiting up to `timeout`; None if none came in that time. A zero timeout
    /// returns a frame already received, without waiting
    fn recv(&mut self, timeout: Duration) -> CmdResult<Option<Vec<u8>>>;
    /// The bytes a frame takes on the wire
    fn wire_len(&self, frame: &[u8]) -> usize {
        hdlc::encode(frame).len()
    }
    /// Frames dropped for a bad CRC so far
    fn bad_frames(&self) -> u32 {
        0
    }
    /// Open the link again after the device restarts; true once it is open
    fn reopen(&mut self, _timeout: Duration) -> bool {
        false
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireFormat {
    // WebSocket (one frame per message) and BLE (frames split across writes and notifications)
    Hdlc,
    // A serial console port
    OverAsciiHdlc,
}

pub fn encode_wire(format: WireFormat, frame: &[u8]) -> Vec<u8> {
    match format {
        WireFormat::Hdlc => hdlc::encode(frame),
        WireFormat::OverAsciiHdlc => overascii::encode(&hdlc::encode(frame)),
    }
}

// Bytes from the wire in, RICREST frames out (and, on a console port, the text between them)
pub struct WireDecoder {
    format: WireFormat,
    hdlc: hdlc::Decoder,
    ascii: overascii::Decoder,
    data: Vec<u8>,
}

impl WireDecoder {
    pub fn new(format: WireFormat) -> WireDecoder {
        WireDecoder { format, hdlc: hdlc::Decoder::default(), ascii: overascii::Decoder::default(), data: Vec::new() }
    }

    pub fn feed(&mut self, bytes: &[u8], frames: &mut Vec<Vec<u8>>, text: &mut Vec<u8>) {
        match self.format {
            WireFormat::Hdlc => self.hdlc.feed(bytes, frames),
            WireFormat::OverAsciiHdlc => {
                self.data.clear();
                self.ascii.feed(bytes, &mut self.data, text);
                self.hdlc.feed(&self.data, frames);
            }
        }
    }

    pub fn bad_frames(&self) -> u32 {
        self.hdlc.bad_frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_round_trip() {
        let frames = vec![vec![1u8, 0x02, 0x00, b'v', 0], (0..=255u8).collect::<Vec<u8>>()];
        for format in [WireFormat::Hdlc, WireFormat::OverAsciiHdlc] {
            let mut wire = Vec::new();
            for f in &frames {
                wire.extend(encode_wire(format, f));
                if format == WireFormat::OverAsciiHdlc {
                    wire.extend(b"W (99) log line\r\n");
                }
            }
            let mut dec = WireDecoder::new(format);
            let (mut out, mut text) = (Vec::new(), Vec::new());
            for chunk in wire.chunks(5) {
                dec.feed(chunk, &mut out, &mut text);
            }
            assert_eq!(out, frames, "{:?}", format);
            assert_eq!(dec.bad_frames(), 0);
            if format == WireFormat::OverAsciiHdlc {
                assert_eq!(text, b"W (99) log line\r\nW (99) log line\r\n".to_vec());
            }
        }
    }
}
