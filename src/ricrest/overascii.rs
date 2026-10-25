// RaftCLI: OverAscii encoding (ProtocolOverAscii), used on a serial console port
// Every byte of an encoded frame has its top bit set, so frames share the port with plain-ASCII log text:
//   00-0F -> 85, (b ^ 0x20) | 0x80     10-7F -> b | 0x80
//   80-8F -> 8E, b ^ 0x20              90-FF -> 8F, b
// Received bytes below 0x80 are text; the rest are decoded and passed on.
// Rob Dobson 2026

const ESCAPE_1: u8 = 0x85;
const ESCAPE_2: u8 = 0x8E;
const ESCAPE_3: u8 = 0x8F;
const MOD_CODE: u8 = 0x20;

pub fn encode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() * 11 / 10 + 4);
    for &b in data {
        match b {
            0x00..=0x0F => out.extend([ESCAPE_1, (b ^ MOD_CODE) | 0x80]),
            0x10..=0x7F => out.push(b | 0x80),
            0x80..=0x8F => out.extend([ESCAPE_2, b ^ MOD_CODE]),
            0x90..=0xFF => out.extend([ESCAPE_3, b]),
        }
    }
    out
}

// Streaming decoder. Text bytes go to `text`, decoded bytes to `data`
#[derive(Default)]
pub struct Decoder {
    escape: u8,
}

impl Decoder {
    pub fn feed(&mut self, bytes: &[u8], data: &mut Vec<u8>, text: &mut Vec<u8>) {
        for &b in bytes {
            if b < 0x80 {
                text.push(b);
                continue;
            }
            // As ProtocolOverAscii::decodeByte
            let escape = std::mem::take(&mut self.escape);
            match escape {
                1 => data.push((b ^ MOD_CODE) & 0x7F),
                2 => data.push(b ^ MOD_CODE),
                3 => data.push(b),
                _ => match b {
                    ESCAPE_1 => self.escape = 1,
                    ESCAPE_2 => self.escape = 2,
                    ESCAPE_3 => self.escape = 3,
                    _ => data.push(b & 0x7F),
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_byte_round_trips_with_the_top_bit_set() {
        let all: Vec<u8> = (0..=255u8).collect();
        let wire = encode(&all);
        assert!(wire.iter().all(|&b| b >= 0x80));
        let (mut data, mut text) = (Vec::new(), Vec::new());
        let mut dec = Decoder::default();
        for b in &wire {
            dec.feed(&[*b], &mut data, &mut text);
        }
        assert_eq!(data, all);
        assert!(text.is_empty());
    }

    #[test]
    fn known_bytes() {
        assert_eq!(encode(&[0x00, 0x41, 0x85, 0xE7]), vec![0x85, 0xA0, 0xC1, 0x8E, 0xA5, 0x8F, 0xE7]);
    }

    #[test]
    fn text_interleaves() {
        let frame = encode(&[0x01, 0xE7, 0x42]);
        let mut wire = b"I (12) boot\r\n".to_vec();
        wire.extend(&frame[..2]);
        wire.extend(b"log");
        wire.extend(&frame[2..]);
        let (mut data, mut text) = (Vec::new(), Vec::new());
        Decoder::default().feed(&wire, &mut data, &mut text);
        assert_eq!(data, vec![0x01, 0xE7, 0x42]);
        assert_eq!(text, b"I (12) boot\r\nlog".to_vec());
    }
}
