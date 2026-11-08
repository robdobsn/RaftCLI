// RaftCLI: Serial monitor baud rate selection and detection
// Rob Dobson 2026
//
// The monitor's baud rate only matters on a real UART (a USB-UART bridge or a non-USB serial port) - the ESP32's
// native USB (USB Serial/JTAG or USB CDC) ignores it. On a UART:
//
//   - A baud rate given on the command line (-b) is used as it is
//   - Otherwise the monitor starts with the most likely rate (see monitor_baud_hints and BaudChoice::resolve):
//     the console baud rate of the built firmware (its sdkconfig), the rate last seen working on this adapter
//     (remembered per adapter in the user's config folder), the -b given last time (raft.info), then 115200
//   - and the received data is checked: if it stays unreadable (bytes typical of a UART receiving at the wrong
//     rate, and no log lines) other rates are tried until log text is readable. The rate found is remembered for
//     the adapter

use regex::bytes::Regex;
use serialport::{available_ports, SerialPortType};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Baud rate used when nothing else is known
pub const DEFAULT_MONITOR_BAUD: u32 = 115200;

/// Rates tried (after the hints) when the data is unreadable
const CANDIDATE_BAUDS: [u32; 6] = [115200, 921600, 460800, 230400, 1000000, 2000000];

/// Espressif's USB vendor ID - the native USB Serial/JTAG and USB CDC ports, which ignore the baud rate
const ESPRESSIF_VID: u16 = 0x303a;

// Window of received data that is assessed: closed at WINDOW_MAX_BYTES, or at WINDOW_MIN_BYTES once WINDOW_MIN_TIME
// has passed
const WINDOW_MAX_BYTES: usize = 256;
const WINDOW_MIN_BYTES: usize = 24;
const WINDOW_MIN_TIME: Duration = Duration::from_millis(300);

// Unreadable data must last this long (and span this many windows) before another rate is tried - long enough
// to ride out the ESP32 boot ROM's output (always at 115200) after a reset when the app uses another rate
const BAD_TIME_SEARCHING: Duration = Duration::from_millis(600);
const BAD_WINDOWS_SEARCHING: u32 = 2;
// Once a rate has given readable text, much longer (a different firmware, or a reset into another rate)
const BAD_TIME_LOCKED: Duration = Duration::from_millis(3000);
const BAD_WINDOWS_LOCKED: u32 = 3;
// When every rate has been tried without readable text, wait this long before trying them again
const SEARCH_PAUSE: Duration = Duration::from_secs(10);

/////////////////////////////////////////////////////////////////////////////////////////////////////////////////
// The port
/////////////////////////////////////////////////////////////////////////////////////////////////////////////////

/// What kind of serial port the monitor is using
#[derive(Debug, Clone, PartialEq)]
pub enum PortKind {
    /// The ESP32's native USB (USB Serial/JTAG or USB CDC) - the baud rate is ignored
    NativeUsb,
    /// A USB-UART bridge (CP210x, FTDI, CH340, ...) - the baud rate matters
    UsbUart,
    /// Not a USB port, or not found in the list of ports - assumed to be a UART
    Other,
}

/// The kind of a port and a key identifying the adapter (for remembering its baud rate)
#[derive(Debug, Clone)]
pub struct PortIdentity {
    pub kind: PortKind,
    pub adapter_key: String,
}

/// Identify a port from the list of serial ports
pub fn identify_port(port_name: &str) -> PortIdentity {
    let ports = available_ports().unwrap_or_default();
    let found = ports.iter().find(|p| p.port_name.eq_ignore_ascii_case(port_name));
    match found.map(|p| &p.port_type) {
        Some(SerialPortType::UsbPort(info)) => {
            let kind = if info.vid == ESPRESSIF_VID { PortKind::NativeUsb } else { PortKind::UsbUart };
            // An adapter's serial number identifies it wherever it is plugged in; without one the port name is used
            let adapter_key = match info.serial_number.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                Some(serial) => format!("usb:{:04x}:{:04x}:{}", info.vid, info.pid, serial),
                None => format!("usb:{:04x}:{:04x}@{}", info.vid, info.pid, port_name),
            };
            PortIdentity { kind, adapter_key }
        }
        _ => PortIdentity { kind: PortKind::Other, adapter_key: format!("port:{}", port_name) },
    }
}

/////////////////////////////////////////////////////////////////////////////////////////////////////////////////
// Choosing the starting rate
/////////////////////////////////////////////////////////////////////////////////////////////////////////////////

/// A possible baud rate and where it came from
#[derive(Debug, Clone, PartialEq)]
pub struct BaudHint {
    pub baud: u32,
    pub source: String,
}

/// Baud rate options for the monitor
#[derive(Debug, Clone, Default)]
pub struct MonitorBaud {
    /// Given on the command line - used as it is, without detection
    pub explicit: Option<u32>,
    /// The firmware's console rate (from its sdkconfig), if the console is on a UART
    pub firmware: Option<BaudHint>,
    /// The -b given last time (raft.info)
    pub saved: Option<u32>,
}

/// The rate to start with and how it is used
#[derive(Debug, Clone)]
pub struct BaudChoice {
    pub baud: u32,
    pub source: String,
    /// Rates to try in order (the first is the starting rate) - empty when the rate is fixed
    pub candidates: Vec<u32>,
}

impl BaudChoice {
    /// Choose the starting rate for a port
    pub fn resolve(options: &MonitorBaud, identity: &PortIdentity, remembered: Option<u32>) -> BaudChoice {
        if let Some(baud) = options.explicit {
            return BaudChoice { baud, source: "-b".to_string(), candidates: vec![] };
        }
        if identity.kind == PortKind::NativeUsb {
            // The rate doesn't matter: keep what would have been used before
            let baud = options.saved.unwrap_or(DEFAULT_MONITOR_BAUD);
            return BaudChoice { baud, source: "native USB, rate not used".to_string(), candidates: vec![] };
        }
        let mut hints: Vec<BaudHint> = Vec::new();
        if let Some(h) = &options.firmware {
            hints.push(h.clone());
        }
        if let Some(baud) = remembered {
            hints.push(BaudHint { baud, source: "last working rate for this adapter".to_string() });
        }
        if let Some(baud) = options.saved {
            hints.push(BaudHint { baud, source: "-b given last time".to_string() });
        }
        hints.push(BaudHint { baud: DEFAULT_MONITOR_BAUD, source: "default".to_string() });
        let first = hints[0].clone();
        let mut candidates: Vec<u32> = Vec::new();
        for baud in hints.iter().map(|h| h.baud).chain(CANDIDATE_BAUDS.iter().copied()) {
            if !candidates.contains(&baud) {
                candidates.push(baud);
            }
        }
        BaudChoice { baud: first.baud, source: first.source, candidates }
    }

    pub fn detecting(&self) -> bool {
        !self.candidates.is_empty()
    }
}

/// The console baud rate of the built firmware, if its console is on a UART, from
/// build/<SysType>/raft/sdkconfig (or build/<SysType>/sdkconfig in older projects)
pub fn firmware_console_baud(app_folder: &str, sys_type: &str) -> Option<BaudHint> {
    let build_dir = Path::new(app_folder).join("build").join(sys_type);
    for path in [build_dir.join("raft").join("sdkconfig"), build_dir.join("sdkconfig")] {
        if let Ok(text) = fs::read_to_string(&path) {
            return console_baud_from_sdkconfig(&text).map(|baud| BaudHint {
                baud,
                source: format!("{}'s console setting", sys_type),
            });
        }
    }
    None
}

/// The console UART baud rate in sdkconfig text, if the console is on a UART
pub fn console_baud_from_sdkconfig(text: &str) -> Option<u32> {
    let mut on_uart = false;
    let mut baud = None;
    for line in text.lines().map(str::trim) {
        if line == "CONFIG_ESP_CONSOLE_UART=y" {
            on_uart = true;
        } else if let Some(v) = line.strip_prefix("CONFIG_ESP_CONSOLE_UART_BAUDRATE=") {
            baud = v.trim().parse::<u32>().ok();
        }
    }
    if on_uart { baud.or(Some(DEFAULT_MONITOR_BAUD)) } else { None }
}

/////////////////////////////////////////////////////////////////////////////////////////////////////////////////
// Rates remembered per adapter (in the user's config folder, as an adapter belongs to the bench not the project)
/////////////////////////////////////////////////////////////////////////////////////////////////////////////////

fn adapter_store_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("raft").join("monitor_bauds.json"))
}

/// The rate last seen working on an adapter
pub fn remembered_baud(adapter_key: &str) -> Option<u32> {
    let text = fs::read_to_string(adapter_store_path()?).ok()?;
    remembered_baud_from(&text, adapter_key)
}

fn remembered_baud_from(text: &str, adapter_key: &str) -> Option<u32> {
    let json: serde_json::Value = serde_json::from_str(text).ok()?;
    json["adapters"][adapter_key]["baud"].as_u64().map(|b| b as u32)
}

/// Remember the rate seen working on an adapter (errors are ignored - this is only a hint for next time)
pub fn remember_baud(adapter_key: &str, baud: u32) {
    let Some(path) = adapter_store_path() else { return };
    let existing = fs::read_to_string(&path).unwrap_or_default();
    let updated = with_remembered_baud(&existing, adapter_key, baud);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&path, updated);
}

fn with_remembered_baud(existing: &str, adapter_key: &str, baud: u32) -> String {
    let mut json: serde_json::Value = serde_json::from_str(existing).unwrap_or_else(|_| serde_json::json!({}));
    if !json.is_object() {
        json = serde_json::json!({});
    }
    if !json["adapters"].is_object() {
        json["adapters"] = serde_json::json!({});
    }
    json["adapters"][adapter_key] = serde_json::json!({
        "baud": baud,
        "updated": chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
    });
    serde_json::to_string_pretty(&json).unwrap_or_default()
}

/////////////////////////////////////////////////////////////////////////////////////////////////////////////////
// Assessing received data
/////////////////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Verdict {
    /// Readable log text
    Good,
    /// Looks like a UART receiving at the wrong rate
    Bad,
    /// Can't tell (binary data, the boot ROM's output, too little text)
    Unclear,
}

/// Measures of a window of received data
#[derive(Debug, Clone, Copy)]
pub struct WindowStats {
    pub text_ratio: f32,
    pub distinct: u32,
    pub log_lines: u32,
    pub newlines: u32,
    pub boot_rom: bool,
}

// A byte that appears in text: printable ASCII, CR, LF, tab, and ESC (ANSI colour sequences in log lines)
fn is_text_byte(b: u8) -> bool {
    (0x20..=0x7e).contains(&b) || b == b'\r' || b == b'\n' || b == b'\t' || b == 0x1b
}

// The start of an ESP-IDF or Raft log line, e.g. "I (12345) Tag: ..." (optionally after an ANSI colour sequence)
fn log_line_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(?:\x1b\[[0-9;]*m)?[EWIDV] \(\d+\) ").unwrap())
}

/// Assess a window of received data. `line` holds the incomplete line from earlier windows and is updated
pub fn assess_window(window: &[u8], line: &mut Vec<u8>) -> WindowStats {
    let len = window.len().max(1) as f32;
    let text = window.iter().filter(|b| is_text_byte(**b)).count() as f32;
    let mut seen = [false; 256];
    window.iter().for_each(|b| seen[*b as usize] = true);
    let distinct = seen.iter().filter(|s| **s).count() as u32;
    let mut log_lines = 0;
    let mut newlines = 0;
    for &b in window {
        if b == b'\n' {
            newlines += 1;
            if log_line_regex().is_match(line.as_slice()) {
                log_lines += 1;
            }
            line.clear();
        } else if b != b'\r' && line.len() < 512 {
            line.push(b);
        }
    }
    let boot_rom = contains(window, b"ESP-ROM:") || contains(window, b"rst:0x");
    WindowStats { text_ratio: text / len, distinct, log_lines, newlines, boot_rom }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// The verdict on a window's measures
pub fn verdict(stats: &WindowStats) -> Verdict {
    // The boot ROM always prints at 115200 (or 74880), whatever rate the app then uses, so its output says nothing
    // about the app's rate
    if stats.boot_rom {
        return Verdict::Unclear;
    }
    if stats.log_lines >= 1 && stats.text_ratio >= 0.85 {
        return Verdict::Good;
    }
    if stats.newlines >= 2 && stats.text_ratio >= 0.97 {
        return Verdict::Good;
    }
    // Receiving faster than the sender: each bit sent spans several received bits, so only a handful of byte values
    // arrive (mostly 0x00, 0x80, 0xF8, 0x78 ...). Text made of a few characters (e.g. a line of '=') is still text
    if stats.distinct <= 6 && stats.text_ratio < 0.9 {
        return Verdict::Bad;
    }
    // Receiving slower than the sender: each byte received is sampled across several bytes sent, so the values look
    // random: under half are text characters (against over 95% for log text). Binary data (e.g. the Raft serial
    // protocol) looks the same, which is why a rate is only changed after this has lasted a while (and much longer
    // once text has been read)
    if stats.text_ratio < 0.7 {
        return Verdict::Bad;
    }
    Verdict::Unclear
}

/////////////////////////////////////////////////////////////////////////////////////////////////////////////////
// The detector
/////////////////////////////////////////////////////////////////////////////////////////////////////////////////

/// What the detector asks the monitor to do
#[derive(Debug, Clone, PartialEq)]
pub enum DetectAction {
    /// Change to this rate (with a reason to show)
    Switch(u32, String),
    /// The current rate gives readable text (remember it for the adapter)
    Readable(u32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum DetectState {
    /// Looking for a rate which gives readable text
    Searching,
    /// The current rate has given readable text
    Locked,
}

pub struct BaudDetector {
    candidates: Vec<u32>,
    current: u32,
    state: DetectState,
    window: Vec<u8>,
    window_start: Option<Instant>,
    line: Vec<u8>,
    bad_since: Option<Instant>,
    bad_windows: u32,
    // Rates tried in this search, and the best text ratio seen at each
    tried: Vec<(u32, f32)>,
    // A rate has just been switched to: one unreadable window is enough to move on
    trial: bool,
    paused_until: Option<Instant>,
}

impl BaudDetector {
    /// A detector starting at the first of the candidates
    pub fn new(candidates: Vec<u32>) -> BaudDetector {
        let current = candidates.first().copied().unwrap_or(DEFAULT_MONITOR_BAUD);
        BaudDetector {
            candidates,
            current,
            state: DetectState::Searching,
            window: Vec::with_capacity(WINDOW_MAX_BYTES),
            window_start: None,
            line: Vec::new(),
            bad_since: None,
            bad_windows: 0,
            tried: vec![(current, 0.0)],
            trial: false,
            paused_until: None,
        }
    }

    #[cfg(test)]
    pub fn current(&self) -> u32 {
        self.current
    }

    /// Data received at time `now`
    pub fn on_data(&mut self, data: &[u8], now: Instant) -> Option<DetectAction> {
        let mut action = None;
        for chunk in data.chunks(WINDOW_MAX_BYTES) {
            if self.window_start.is_none() {
                self.window_start = Some(now);
            }
            let room = WINDOW_MAX_BYTES - self.window.len();
            let (now_part, rest) = chunk.split_at(chunk.len().min(room));
            self.window.extend_from_slice(now_part);
            if self.window.len() >= WINDOW_MAX_BYTES {
                if let Some(a) = self.close_window(now) {
                    action = Some(a);
                    if matches!(action, Some(DetectAction::Switch(..))) {
                        // Data after a switch was received at the old rate
                        return action;
                    }
                }
                if !rest.is_empty() {
                    self.window_start = Some(now);
                    self.window.extend_from_slice(rest);
                }
            }
        }
        if action.is_none() {
            action = self.on_tick(now);
        }
        action
    }

    /// Time has passed (called regularly, with or without data)
    pub fn on_tick(&mut self, now: Instant) -> Option<DetectAction> {
        if let Some(start) = self.window_start {
            if self.window.len() >= WINDOW_MIN_BYTES && now.duration_since(start) >= WINDOW_MIN_TIME {
                return self.close_window(now);
            }
        }
        None
    }

    fn close_window(&mut self, now: Instant) -> Option<DetectAction> {
        let start = self.window_start.take().unwrap_or(now);
        let stats = assess_window(&self.window, &mut self.line);
        self.window.clear();
        if let Some(entry) = self.tried.iter_mut().find(|(b, _)| *b == self.current) {
            entry.1 = entry.1.max(stats.text_ratio);
        }
        match verdict(&stats) {
            Verdict::Good => {
                self.bad_since = None;
                self.bad_windows = 0;
                self.trial = false;
                self.paused_until = None;
                if self.state == DetectState::Searching {
                    self.state = DetectState::Locked;
                    return Some(DetectAction::Readable(self.current));
                }
                None
            }
            Verdict::Unclear => None,
            Verdict::Bad => {
                let since = *self.bad_since.get_or_insert(start);
                self.bad_windows += 1;
                let (time_needed, windows_needed) = match (self.state, self.trial) {
                    (DetectState::Locked, _) => (BAD_TIME_LOCKED, BAD_WINDOWS_LOCKED),
                    (DetectState::Searching, true) => (Duration::ZERO, 1),
                    (DetectState::Searching, false) => (BAD_TIME_SEARCHING, BAD_WINDOWS_SEARCHING),
                };
                if self.bad_windows < windows_needed || now.duration_since(since) < time_needed {
                    return None;
                }
                if let Some(until) = self.paused_until {
                    if now < until {
                        return None;
                    }
                    // Start a new search from the current rate
                    self.paused_until = None;
                    self.tried = vec![(self.current, 0.0)];
                }
                let reason = if self.state == DetectState::Locked {
                    self.state = DetectState::Searching;
                    self.tried = vec![(self.current, 0.0)];
                    format!("output no longer readable at {}", self.current)
                } else {
                    format!("output unreadable at {}", self.current)
                };
                self.next_candidate(now, reason)
            }
        }
    }

    fn next_candidate(&mut self, now: Instant, reason: String) -> Option<DetectAction> {
        self.bad_since = None;
        self.bad_windows = 0;
        self.line.clear();
        let untried = self.candidates.iter().copied().find(|c| !self.tried.iter().any(|(b, _)| b == c));
        match untried {
            Some(baud) => {
                self.current = baud;
                self.tried.push((baud, 0.0));
                self.trial = true;
                Some(DetectAction::Switch(baud, reason))
            }
            None => {
                // Every rate has been tried: go back to the one with the most text and wait before trying again
                let best = self.tried.iter().copied()
                    .fold(None, |best: Option<(u32, f32)>, t| match best {
                        Some(b) if b.1 >= t.1 => Some(b),
                        _ => Some(t),
                    })
                    .map(|(b, _)| b)
                    .unwrap_or(self.current);
                self.trial = false;
                self.paused_until = Some(now + SEARCH_PAUSE);
                if best != self.current {
                    self.current = best;
                    Some(DetectAction::Switch(best, "no rate gave readable text - using the most readable".to_string()))
                } else {
                    None
                }
            }
        }
    }
}

/////////////////////////////////////////////////////////////////////////////////////////////////////////////////
// Tests
/////////////////////////////////////////////////////////////////////////////////////////////////////////////////

#[cfg(test)]
mod tests {
    use super::*;

    // Simulate a UART (8N1) receiving at rx_baud what was sent back to back at tx_baud: the receiver waits for a
    // falling edge, samples each bit at its middle and delivers the byte whatever the stop bit
    pub fn simulate_uart(data: &[u8], tx_baud: u32, rx_baud: u32) -> Vec<u8> {
        let mut bits: Vec<bool> = vec![true; 20];
        for &b in data {
            bits.push(false);
            for i in 0..8 {
                bits.push((b >> i) & 1 == 1);
            }
            bits.push(true);
        }
        bits.extend(std::iter::repeat(true).take(40));
        let tx_bit = 1.0 / tx_baud as f64;
        let rx_bit = 1.0 / rx_baud as f64;
        let end = bits.len() as f64 * tx_bit;
        let level = |t: f64| -> bool {
            let i = (t / tx_bit) as usize;
            *bits.get(i).unwrap_or(&true)
        };
        let mut out = Vec::new();
        let mut t = 0.0;
        loop {
            // Next falling edge (edges are at tx bit boundaries)
            let mut k = (t / tx_bit).ceil() as usize;
            let edge = loop {
                if k >= bits.len() {
                    break None;
                }
                if k > 0 && bits[k - 1] && !bits[k] {
                    break Some(k as f64 * tx_bit);
                }
                k += 1;
            };
            let Some(edge) = edge else { break };
            if edge + 10.0 * rx_bit > end {
                break;
            }
            let mut byte = 0u8;
            for i in 0..8 {
                if level(edge + (1.5 + i as f64) * rx_bit + 1e-12) {
                    byte |= 1 << i;
                }
            }
            out.push(byte);
            t = edge + 9.5 * rx_bit;
        }
        out
    }

    const LOG: &str = "\x1b[0;32mI (1234) DevicePower: battery 3.92V soc 81% charging no\x1b[0m\r\n\
        I (1250) SerialConsole: setup UART console uartNum 0 rxBufLen 1024 txBufLen 4096\r\n\
        W (1302) NetMan: WiFi not connected - retrying\r\n\
        I (1400) DataLogger: session started rate 10Hz devices 3\r\n\
        I (1512) LEDPatternStatus: charge pips 4 amber\r\n\
        E (1600) BusI2C: device 0x48 not responding\r\n\
        I (1712) RaftCoreApp: heap 182344 free psram 7843210 free\r\n";

    fn stats_for(data: &[u8]) -> WindowStats {
        let mut line = Vec::new();
        assess_window(data, &mut line)
    }

    #[test]
    fn simulator_round_trips_at_the_same_rate() {
        assert_eq!(simulate_uart(LOG.as_bytes(), 115200, 115200), LOG.as_bytes());
        assert_eq!(simulate_uart(LOG.as_bytes(), 921600, 921600), LOG.as_bytes());
    }

    #[test]
    fn readable_log_is_good() {
        let s = stats_for(LOG.as_bytes());
        assert_eq!(verdict(&s), Verdict::Good, "{:?}", s);
    }

    #[test]
    fn wrong_rates_are_bad() {
        let log = LOG.repeat(4);
        for (tx, rx) in [(921600, 115200), (115200, 921600), (460800, 115200), (115200, 460800),
                         (921600, 230400), (2000000, 115200), (115200, 2000000), (230400, 921600),
                         (921600, 460800), (460800, 921600), (1000000, 921600), (921600, 1000000)] {
            let received = simulate_uart(log.as_bytes(), tx, rx);
            for window in received.chunks(WINDOW_MAX_BYTES).filter(|w| w.len() >= WINDOW_MIN_BYTES) {
                let s = stats_for(window);
                assert_eq!(verdict(&s), Verdict::Bad, "tx {} rx {} {:?}", tx, rx, s);
            }
        }
    }

    #[test]
    fn boot_rom_and_few_character_text_are_not_bad() {
        let divider = b"==================================================\r\n";
        assert_ne!(verdict(&stats_for(divider)), Verdict::Bad);
        let rom = b"ESP-ROM:esp32s3-20210327\r\nBuild:Mar 27 2021\r\nrst:0x1 (POWERON),boot:0x8 (SPI_FAST_FLASH_BOOT)\r\n";
        assert_eq!(verdict(&stats_for(rom)), Verdict::Unclear);
    }

    #[test]
    fn log_lines_are_found_across_windows() {
        let mut line = Vec::new();
        let (a, b) = LOG.as_bytes().split_at(20);
        let s1 = assess_window(a, &mut line);
        let s2 = assess_window(b, &mut line);
        assert_eq!(s1.log_lines + s2.log_lines, 7);
    }

    // Feed the detector what a device sending at device_baud gives at each rate it switches to
    fn run_detector(det: &mut BaudDetector, device_baud: u32, seconds: u64, start: Instant) -> Vec<DetectAction> {
        let mut actions = Vec::new();
        let mut now = start;
        // Log text arrives in 50ms chunks
        let chunk = LOG.as_bytes();
        for _ in 0..(seconds * 20) {
            now += Duration::from_millis(50);
            let received = simulate_uart(chunk, device_baud, det.current());
            if let Some(a) = det.on_data(&received, now) {
                actions.push(a);
            }
        }
        actions
    }

    #[test]
    fn detector_locks_on_the_right_rate_at_once() {
        let mut det = BaudDetector::new(vec![921600, 115200]);
        let actions = run_detector(&mut det, 921600, 2, Instant::now());
        assert_eq!(actions, vec![DetectAction::Readable(921600)]);
    }

    #[test]
    fn detector_finds_the_rate() {
        for device_baud in [115200, 921600, 460800, 2000000] {
            let choice = BaudChoice::resolve(&MonitorBaud::default(),
                        &PortIdentity { kind: PortKind::UsbUart, adapter_key: "x".into() }, None);
            let mut det = BaudDetector::new(choice.candidates.clone());
            let actions = run_detector(&mut det, device_baud, 10, Instant::now());
            assert_eq!(det.current(), device_baud, "{:?}", actions);
            assert_eq!(actions.last(), Some(&DetectAction::Readable(device_baud)), "{:?}", actions);
        }
    }

    #[test]
    fn detector_rides_out_the_boot_rom() {
        // App at 921600; after a reset the ROM prints at 115200 (garbage at 921600) for ~50ms
        let mut det = BaudDetector::new(vec![921600, 115200]);
        let start = Instant::now();
        let mut actions = run_detector(&mut det, 921600, 1, start);
        let rom = b"ESP-ROM:esp32s3-20210327\r\nBuild:Mar 27 2021\r\nrst:0x1 (POWERON),boot:0x8 (SPI_FAST_FLASH_BOOT)\r\n\
                    SPIWP:0xee\r\nmode:DIO, clock div:1\r\nload:0x3fce2810,len:0x15a0\r\nentry 0x403c8930\r\n";
        let now = start + Duration::from_millis(1100);
        if let Some(a) = det.on_data(&simulate_uart(rom, 115200, 921600), now) {
            actions.push(a);
        }
        actions.extend(run_detector(&mut det, 921600, 2, now));
        assert_eq!(det.current(), 921600);
        assert_eq!(actions, vec![DetectAction::Readable(921600)]);
    }

    #[test]
    fn locked_rate_survives_a_binary_burst() {
        // Readable at 921600, then 2s of binary (e.g. a Raft protocol transfer), then text again
        let mut det = BaudDetector::new(vec![921600, 115200]);
        let start = Instant::now();
        let mut actions = run_detector(&mut det, 921600, 1, start);
        let mut now = start + Duration::from_millis(1000);
        let mut x: u32 = 7;
        for _ in 0..40 {
            now += Duration::from_millis(50);
            let binary: Vec<u8> = (0..200).map(|_| { x = x.wrapping_mul(1103515245).wrapping_add(12345); (x >> 16) as u8 }).collect();
            if let Some(a) = det.on_data(&binary, now) {
                actions.push(a);
            }
        }
        actions.extend(run_detector(&mut det, 921600, 1, now));
        assert_eq!(actions, vec![DetectAction::Readable(921600)]);
    }

    #[test]
    fn detector_holds_when_nothing_is_readable() {
        let mut det = BaudDetector::new(vec![115200, 921600]);
        let mut now = Instant::now();
        let mut switches = 0;
        for _ in 0..200 {
            now += Duration::from_millis(50);
            // All zeros (a line held low) - unreadable at any rate
            let data = vec![0u8; 64];
            if let Some(DetectAction::Switch(..)) = det.on_data(&data, now) {
                switches += 1;
            }
        }
        // Both tried, then a pause of 10s (and one more round in the 10s run)
        assert!(switches <= 4, "{}", switches);
    }

    #[test]
    fn explicit_and_native_usb_are_fixed() {
        let uart = PortIdentity { kind: PortKind::UsbUart, adapter_key: "a".into() };
        let usb = PortIdentity { kind: PortKind::NativeUsb, adapter_key: "b".into() };
        let opts = MonitorBaud { explicit: Some(460800), ..Default::default() };
        let c = BaudChoice::resolve(&opts, &uart, Some(921600));
        assert_eq!((c.baud, c.detecting()), (460800, false));
        let opts = MonitorBaud { saved: Some(921600), ..Default::default() };
        let c = BaudChoice::resolve(&opts, &usb, Some(115200));
        assert_eq!((c.baud, c.detecting()), (921600, false));
    }

    #[test]
    fn hints_are_in_order() {
        let uart = PortIdentity { kind: PortKind::Other, adapter_key: "a".into() };
        let opts = MonitorBaud {
            explicit: None,
            firmware: Some(BaudHint { baud: 921600, source: "fw".into() }),
            saved: Some(230400),
        };
        let c = BaudChoice::resolve(&opts, &uart, Some(460800));
        assert_eq!(c.baud, 921600);
        assert_eq!(&c.candidates[..4], &[921600, 460800, 230400, 115200]);
        let c = BaudChoice::resolve(&MonitorBaud::default(), &uart, Some(460800));
        assert_eq!((c.baud, c.source.as_str()), (460800, "last working rate for this adapter"));
    }

    #[test]
    fn console_baud_from_sdkconfig_text() {
        assert_eq!(console_baud_from_sdkconfig("CONFIG_ESP_CONSOLE_UART=y\nCONFIG_ESP_CONSOLE_UART_BAUDRATE=921600\n"), Some(921600));
        assert_eq!(console_baud_from_sdkconfig("CONFIG_ESP_CONSOLE_UART=y\n"), Some(115200));
        assert_eq!(console_baud_from_sdkconfig("CONFIG_ESP_CONSOLE_USB_SERIAL_JTAG=y\n# CONFIG_ESP_CONSOLE_UART is not set\n"), None);
    }

    #[test]
    fn adapter_store_round_trip() {
        let text = with_remembered_baud("", "usb:10c4:ea60:0001", 921600);
        let text = with_remembered_baud(&text, "port:/dev/ttyS0", 115200);
        assert_eq!(remembered_baud_from(&text, "usb:10c4:ea60:0001"), Some(921600));
        assert_eq!(remembered_baud_from(&text, "port:/dev/ttyS0"), Some(115200));
        assert_eq!(remembered_baud_from(&text, "other"), None);
        assert_eq!(remembered_baud_from("not json", "other"), None);
    }
}
