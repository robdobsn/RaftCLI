// RaftCLI: RICREST frames
// [msgNum][(msgType << 6) | protocol][elemCode][body]. msgNum 0 is unnumbered (the device uses it for messages it
// sends on its own); a response carries its request's msgNum. Text bodies (URLs, JSON) are NUL-terminated.
// Rob Dobson 2026

pub const PROTOCOL_RICREST: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MsgType {
    Command = 0,
    Response = 1,
    Publish = 2,
    Report = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElemCode {
    Url = 0,
    #[cfg_attr(not(test), allow(dead_code))] // sent by the device (and the simulator)
    CmdRespJson = 1,
    CommandFrame = 3,
    FileBlock = 4,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub msg_num: u8,
    pub msg_type: MsgType,
    pub protocol: u8,
    // RICREST frames only (None for other protocols, or a frame too short to have one)
    pub elem: Option<u8>,
    pub body: Vec<u8>,
}

impl Frame {
    pub fn parse(bytes: &[u8]) -> Option<Frame> {
        if bytes.len() < 2 {
            return None;
        }
        let msg_type = match bytes[1] >> 6 {
            0 => MsgType::Command,
            1 => MsgType::Response,
            2 => MsgType::Publish,
            _ => MsgType::Report,
        };
        let protocol = bytes[1] & 0x3F;
        let (elem, body) = if protocol == PROTOCOL_RICREST && bytes.len() >= 3 {
            (Some(bytes[2]), bytes[3..].to_vec())
        } else {
            (None, bytes[2..].to_vec())
        };
        Some(Frame { msg_num: bytes[0], msg_type, protocol, elem, body })
    }

    // The body as text, without its NUL terminator
    pub fn text(&self) -> String {
        let end = self.body.iter().position(|&b| b == 0).unwrap_or(self.body.len());
        String::from_utf8_lossy(&self.body[..end]).into_owned()
    }

    // A RICREST response's JSON body (None if it isn't one, or isn't JSON)
    pub fn json(&self) -> Option<serde_json::Value> {
        if self.protocol != PROTOCOL_RICREST || self.msg_type != MsgType::Response {
            return None;
        }
        serde_json::from_str(&self.text()).ok()
    }
}

// A RICREST frame with a NUL-terminated text body (a URL or JSON), as raftjs sends them
pub fn text_frame(msg_num: u8, msg_type: MsgType, elem: ElemCode, text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + 4);
    out.extend([msg_num, ((msg_type as u8) << 6) | PROTOCOL_RICREST, elem as u8]);
    out.extend_from_slice(text.as_bytes());
    out.push(0);
    out
}

// An (unnumbered) upload block: stream ID and a 24-bit file position, then the data
pub fn file_block(stream_id: u8, pos: u32, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 7);
    out.extend([0, ((MsgType::Command as u8) << 6) | PROTOCOL_RICREST, ElemCode::FileBlock as u8, stream_id]);
    out.extend([(pos >> 16) as u8, (pos >> 8) as u8, pos as u8]);
    out.extend_from_slice(data);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_build_and_parse() {
        let url = text_frame(7, MsgType::Command, ElemCode::Url, "filelist/local/");
        assert_eq!(&url[..3], &[7, 0x02, 0x00]);
        assert_eq!(*url.last().unwrap(), 0);
        let frame = Frame::parse(&url).unwrap();
        assert_eq!((frame.msg_num, frame.msg_type, frame.elem), (7, MsgType::Command, Some(0)));
        assert_eq!(frame.text(), "filelist/local/");

        let resp = Frame::parse(&text_frame(7, MsgType::Response, ElemCode::CmdRespJson, r#"{"rslt":"ok"}"#)).unwrap();
        assert_eq!(resp.msg_type, MsgType::Response);
        assert_eq!(resp.json().unwrap()["rslt"], "ok");

        let block = file_block(3, 0x012345, b"abc");
        assert_eq!(block, vec![0, 0x02, 0x04, 3, 0x01, 0x23, 0x45, b'a', b'b', b'c']);

        // Publish frames of another protocol have no element code
        let publish = Frame::parse(&[0, 0x80 | 0x05, 1, 2]).unwrap();
        assert_eq!((publish.msg_type, publish.protocol, publish.elem), (MsgType::Publish, 5, None));
        assert!(Frame::parse(&[1]).is_none());
    }
}
