// RaftCLI: A RICREST session over a link
// RaftCLI is sequential, so a session has one numbered request outstanding at a time (simpler than raftjs's
// RaftMsgHandler): it numbers requests 1-255, waits for the response with that number, and resends the same frame
// if none comes - the device ignores a repeat of a request it is still answering. Frames nobody is waiting for
// (late or duplicate responses, publish and report messages) are dropped.
// Rob Dobson 2026

use std::time::{Duration, Instant};

use super::frame::{text_frame, ElemCode, Frame, MsgType};
use crate::device_conn::CmdResult;
use crate::link::Link;

#[derive(Clone, Copy, Debug)]
pub struct RequestPolicy {
    pub timeout: Duration,
    pub attempts: u32,
}

pub struct Session<L: Link> {
    link: L,
    last_msg_num: u8,
    // Responses that arrived when nothing was waiting for them (late or duplicate replies)
    pub stale_responses: u32,
    // Requests sent again for want of a reply
    pub resent: u32,
}

impl<L: Link> Session<L> {
    pub fn new(link: L) -> Session<L> {
        Session { link, last_msg_num: 0, stale_responses: 0, resent: 0 }
    }

    pub fn link(&self) -> &L {
        &self.link
    }

    pub fn link_mut(&mut self) -> &mut L {
        &mut self.link
    }

    // 1 to 255, then 1 again (0 is for unnumbered messages)
    fn next_msg_num(&mut self) -> u8 {
        self.last_msg_num = if self.last_msg_num == 255 { 1 } else { self.last_msg_num + 1 };
        self.last_msg_num
    }

    // Send a numbered command with a text body (a URL or a JSON command frame) and return its response
    pub fn request(&mut self, elem: ElemCode, text: &str, policy: RequestPolicy) -> CmdResult<Frame> {
        let msg_num = self.next_msg_num();
        let frame = text_frame(msg_num, MsgType::Command, elem, text);
        for attempt in 0..policy.attempts.max(1) {
            if attempt > 0 {
                self.resent += 1;
            }
            self.link.send(&frame)?;
            let reply = self.wait_for(|f| f.msg_type == MsgType::Response && f.msg_num == msg_num, policy.timeout)?;
            if let Some(reply) = reply {
                return Ok(reply);
            }
        }
        let shown: String = text.chars().take(60).collect();
        Err(format!(
            "no reply from the device to {} ({} tries of {:.1}s)",
            shown,
            policy.attempts.max(1),
            policy.timeout.as_secs_f32()
        )
        .into())
    }

    // Send a frame that has no response of its own (e.g. a file block)
    pub fn send(&mut self, frame: &[u8]) -> CmdResult<()> {
        self.link.send(frame)
    }

    // The first frame for which `wanted` is true, within `timeout`; others are dropped
    pub fn wait_for(&mut self, wanted: impl Fn(&Frame) -> bool, timeout: Duration) -> CmdResult<Option<Frame>> {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let Some(bytes) = self.link.recv(remaining)? else {
                return Ok(None);
            };
            if let Some(frame) = Frame::parse(&bytes) {
                if wanted(&frame) {
                    return Ok(Some(frame));
                }
                if frame.msg_type == MsgType::Response {
                    self.stale_responses += 1;
                }
            }
        }
    }
}
