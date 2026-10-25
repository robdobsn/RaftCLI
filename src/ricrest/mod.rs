// RaftCLI: RICREST - REST commands and file transfer over any link (serial, WebSocket, BLE)
// The protocol Raft devices speak on every channel, as implemented on the device by RaftCore (ProtocolExchange,
// ProtocolRICSerial, FileUploadOKTOProtocol) and in raftjs (RaftMiniHDLC, RaftMsgHandler, RaftFileHandler).
// See devdocs/fs-over-serial-ws-ble-plan.md section 3.
// Rob Dobson 2026

pub mod conn;
pub mod frame;
pub mod hdlc;
pub mod overascii;
pub mod session;
pub mod upload;

#[cfg(test)]
mod tests;
