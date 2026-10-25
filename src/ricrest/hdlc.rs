// RaftCLI: HDLC framing as Raft devices use it (MiniHDLC / ProtocolRICSerial)
// A frame is E7 | escaped(payload, CRC16 high, CRC16 low) | E7. Only E7 and D7 are escaped, as D7 then the byte
// XOR 0x20. The CRC is CRC16-CCITT (init 0xFFFF, poly 0x1021) of the payload.
// Rob Dobson 2026

use crate::app_fs::crc16_ccitt;

const FRAME_BOUNDARY: u8 = 0xE7;
const CONTROL_ESCAPE: u8 = 0xD7;
const ESCAPE_XOR: u8 = 0x20;

// Frames longer than this are dropped (devices with PSRAM accept up to 200000 bytes)
const MAX_FRAME_LEN: usize = 256 * 1024;

pub fn encode(payload: &[u8]) -> Vec<u8> {
    let crc = crc16_ccitt(payload);
    let mut out = Vec::with_capacity(payload.len() + 8);
    out.push(FRAME_BOUNDARY);
    for &b in payload.iter().chain([(crc >> 8) as u8, crc as u8].iter()) {
        if b == FRAME_BOUNDARY || b == CONTROL_ESCAPE {
            out.push(CONTROL_ESCAPE);
            out.push(b ^ ESCAPE_XOR);
        } else {
            out.push(b);
        }
    }
    out.push(FRAME_BOUNDARY);
    out
}

// Streaming decoder: bytes in, whole frames (payloads, CRC checked) out
#[derive(Default)]
pub struct Decoder {
    buf: Vec<u8>,
    escaped: bool,
    overflow: bool,
    // Frames dropped for a bad CRC or for being too long
    pub bad_frames: u32,
}

impl Decoder {
    pub fn feed(&mut self, bytes: &[u8], frames: &mut Vec<Vec<u8>>) {
        for &b in bytes {
            self.feed_byte(b, frames);
        }
    }

    pub fn feed_byte(&mut self, b: u8, frames: &mut Vec<Vec<u8>>) {
        match b {
            FRAME_BOUNDARY => {
                // Back-to-back boundaries (an empty frame) are normal between frames
                if self.overflow {
                    self.bad_frames += 1;
                } else if self.buf.len() >= 2 {
                    let n = self.buf.len();
                    let rx_crc = ((self.buf[n - 2] as u16) << 8) | self.buf[n - 1] as u16;
                    if crc16_ccitt(&self.buf[..n - 2]) == rx_crc {
                        frames.push(self.buf[..n - 2].to_vec());
                    } else {
                        self.bad_frames += 1;
                    }
                } else if !self.buf.is_empty() {
                    self.bad_frames += 1;
                }
                self.buf.clear();
                self.escaped = false;
                self.overflow = false;
            }
            CONTROL_ESCAPE => self.escaped = true,
            _ => {
                let b = if self.escaped { b ^ ESCAPE_XOR } else { b };
                self.escaped = false;
                if self.buf.len() < MAX_FRAME_LEN {
                    self.buf.push(b);
                } else {
                    self.overflow = true;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn known_frames() {
        // The plan's test vectors: RICREST URL frames [msgNum, 0x02, 0x00, url, NUL]. The first is the frame every
        // BLE spike run sent first, which the device answered
        assert_eq!(hex(&encode(b"\x01\x02\x00v\x00")), "e7 01 02 00 76 00 f4 ca e7");
        assert_eq!(hex(&encode(b"\x01\x02\x00filelist\x00")), "e7 01 02 00 66 69 6c 65 6c 69 73 74 00 d1 dc e7");
        // msgNum 0xE7 is escaped
        assert_eq!(hex(&encode(b"\xe7\x02\x00v\x00")), "e7 d7 c7 02 00 76 00 02 43 e7");
    }

    #[test]
    fn round_trip_in_pieces() {
        let payloads: Vec<Vec<u8>> = vec![
            vec![0xE7, 0xD7, 0x00, 0xFF, 0xE7],
            (0..=255u8).collect(),
            b"{\"rslt\":\"ok\"}\0".to_vec(),
            (0..3000u32).map(|i| (i * 31) as u8).collect(),
        ];
        let mut wire = vec![0xE7, 0xE7];
        for p in &payloads {
            wire.extend(encode(p));
        }
        // Fed one byte at a time and in odd-sized pieces, as links deliver them
        for piece in [1, 7, 182, 4096] {
            let mut dec = Decoder::default();
            let mut frames = Vec::new();
            for chunk in wire.chunks(piece) {
                dec.feed(chunk, &mut frames);
            }
            assert_eq!(frames, payloads, "piece size {}", piece);
            assert_eq!(dec.bad_frames, 0);
        }
    }

    #[test]
    fn corrupt_frames_are_dropped_and_counted() {
        let mut wire = encode(b"first");
        let mut second = encode(b"second");
        second[3] ^= 0x01;
        wire.extend(second);
        wire.extend(encode(b"third"));
        let mut dec = Decoder::default();
        let mut frames = Vec::new();
        dec.feed(&wire, &mut frames);
        assert_eq!(frames, vec![b"first".to_vec(), b"third".to_vec()]);
        assert_eq!(dec.bad_frames, 1);
    }
}
