// RaftCLI: Connection to a running Raft device, whatever the link
// Device commands (raft fs, raft devtypes) send REST commands - the device's /api/... strings without "/api/" - and
// upload files through DeviceConn, so they work the same on every link: HTTP, and RICREST over serial, WebSocket
// and BLE (devdocs/fs-over-serial-ws-ble-plan.md).
// Rob Dobson 2026

use clap::Args;
use std::collections::VecDeque;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::app_fs::{crc16_ccitt, encode_device_path};
use crate::app_ports::{select_most_likely_port, PortsCmd};
use crate::http_client::{http_get, http_get_timeout, http_post_multipart_file, ProgressFn};
#[cfg(feature = "ble")]
use crate::link::ble::BleLink;
use crate::link::serial::{SerialLink, TextSink};
use crate::link::websocket::WebSocketLink;
use crate::raft_cli_utils::is_wsl;
use crate::ricrest::conn::RicrestConn;

pub type CmdResult<T> = Result<T, Box<dyn std::error::Error>>;

// Environment variable giving the device when there is no -d/--device and no device argument
pub const DEVICE_ENV: &str = "RAFT_DEVICE";

// Environment variable naming the Windows raftcli that WSL hands serial and BLE devices to (default raft.exe)
const WINDOWS_RAFT_ENV: &str = "RAFTCLI_WINDOWS_EXE";

const SERIAL_BAUD_DEFAULT: u32 = 115200;

// The device options of every device command. The device is the first positional argument unless -d/--device or
// RAFT_DEVICE gives it (see resolve_device), so clap sees the positional arguments after it as optional too
#[derive(Clone, Args, Debug)]
pub struct DeviceArgs {
    /// Device: IP address or hostname, http://host[:port], ws://host[:port][/path] (path default /ws),
    /// serial:<port>[@baud] (serial: picks the port), or ble:<name, name prefix or address> (ble: for the only
    /// one in range). Leave it out when -d/--device or RAFT_DEVICE gives it
    #[clap(value_name = "DEVICE")]
    pub device: Option<String>,
    /// Device, instead of naming it first (default: the RAFT_DEVICE environment variable)
    #[clap(short = 'd', long = "device", value_name = "DEVICE")]
    pub device_opt: Option<String>,
    /// HTTP port (default 80)
    #[clap(short = 'p', long)]
    pub port: Option<u16>,
    /// Show the device's log output (serial devices)
    #[clap(long)]
    pub show_log: bool,
    /// Show link statistics at the end (serial, WebSocket and BLE devices: frames dropped for a bad CRC, requests
    /// sent again)
    #[clap(short = 'v', long)]
    pub verbose: bool,
    /// In WSL, use this system's serial ports (by default serial devices are handed to the Windows raft.exe)
    #[clap(short = 'n', long)]
    pub native_serial_port: bool,
    #[clap(flatten)]
    pub ble: BleArgs,
}

// Overrides for BLE devices with other UUIDs (by default the RICREST services Raft devices use are found)
#[derive(Clone, Args, Debug, Default)]
pub struct BleArgs {
    /// BLE service UUID (default: any of the Raft RICREST services)
    #[clap(long, value_name = "UUID", help_heading = "BLE")]
    pub ble_service: Option<String>,
    /// BLE command characteristic UUID (default: the service's writable one)
    #[clap(long, value_name = "UUID", help_heading = "BLE")]
    pub ble_cmd: Option<String>,
    /// BLE response characteristic UUID (default: the service's notifying one)
    #[clap(long, value_name = "UUID", help_heading = "BLE")]
    pub ble_resp: Option<String>,
    /// BLE MTU, if the one reported is wrong (writes are MTU - 3 bytes)
    #[clap(long, value_name = "BYTES", help_heading = "BLE")]
    pub ble_mtu: Option<u16>,
}

#[cfg(feature = "ble")]
impl BleArgs {
    pub fn options(&self) -> crate::link::ble::BleOptions {
        crate::link::ble::BleOptions {
            service: self.ble_service.clone(),
            cmd: self.ble_cmd.clone(),
            resp: self.ble_resp.clone(),
            mtu: self.ble_mtu,
        }
    }
}

impl DeviceArgs {
    // The device, and the command's other positional arguments (`rest`, in order)
    pub fn resolve(&self, rest: Vec<Option<String>>) -> CmdResult<(String, Positionals)> {
        let mut positionals = vec![self.device.clone()];
        positionals.extend(rest);
        resolve_device(&self.device_opt, positionals)
    }

    pub fn open(&self, device: &str) -> CmdResult<Rc<dyn DeviceConn>> {
        open_device(
            device,
            &ConnOptions { http_port: self.port, show_log: self.show_log, verbose: self.verbose, ble: self.ble.clone() },
        )
    }
}

#[derive(Clone, Debug, Default)]
pub struct ConnOptions {
    pub http_port: Option<u16>,
    pub show_log: bool,
    pub verbose: bool,
    #[cfg_attr(not(feature = "ble"), allow(dead_code))]
    pub ble: BleArgs,
}

// ---------------------------------------------------------------------------
// Connection
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkKind {
    Http,
    WebSocket,
    Serial,
    #[cfg_attr(not(feature = "ble"), allow(dead_code))]
    Ble,
}

impl LinkKind {
    // Bytes per filesection request: a serial reply is kept small, as firmware without the SerialConsole fix
    // encodes it in a stack buffer of twice its length
    pub fn filesection_chunk(self) -> u64 {
        match self {
            LinkKind::Serial => 1024,
            LinkKind::Http | LinkKind::WebSocket | LinkKind::Ble => 4096,
        }
    }
}

// A connection to a device. Methods take &self so commands can share one connection (links with state keep it
// behind interior mutability).
pub trait DeviceConn {
    /// REST command without "/api/", e.g. "filelist/local/web"; returns the reply body
    fn rest_raw(&self, cmd: &str) -> CmdResult<Vec<u8>>;
    /// Upload a local file as <remote> on file system <fs>, reporting (sent, total, bytes per second)
    fn put_file(&self, local: &Path, remote: &str, fs: &str, progress: &mut ProgressFn) -> CmdResult<()>;
    /// Wait for the device to answer again after it restarts
    fn wait_for_restart(&self, timeout: Duration) -> bool;
    /// The device as it was given, for messages (and as a key for records kept per device)
    fn describe(&self) -> &str;
    fn link_kind(&self) -> LinkKind;
    /// Link statistics for -v (None if the link has none)
    fn link_stats(&self) -> Option<String> {
        None
    }

    /// REST command whose reply is JSON
    fn rest(&self, cmd: &str) -> CmdResult<serde_json::Value> {
        let body = self.rest_raw(cmd)?;
        serde_json::from_slice(&body).map_err(|e| {
            let text = String::from_utf8_lossy(&body);
            let shown: String = text.chars().take(200).collect();
            format!("invalid reply from the device to {}: {} ({})", cmd, e, shown).into()
        })
    }

    /// REST command whose reply must be "rslt":"ok"
    fn rest_ok(&self, cmd: &str) -> CmdResult<serde_json::Value> {
        let json = self.rest(cmd)?;
        if json["rslt"].as_str() != Some("ok") {
            let error = json["error"].as_str().or(json["rslt"].as_str()).unwrap_or("unknown");
            return Err(format!("Device error: {}", error).into());
        }
        Ok(json)
    }
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

/// Check a file uploaded arrived intact (the device's CRC of it matches), removing it if not
pub(crate) fn check_uploaded(conn: &dyn DeviceConn, data: &[u8], remote: &str, fs: &str) -> CmdResult<()> {
    let remote = remote.trim_start_matches('/');
    let enc = encode_device_path(remote);
    let hash = conn.rest_ok(&format!("filehash/{}/{}", fs, enc))?;
    let device_crc = hash["crc16"].as_str().unwrap_or("").to_lowercase();
    let local_crc = format!("{:04x}", crc16_ccitt(data));
    if device_crc != local_crc {
        let _ = conn.rest(&format!("filedelete/{}/{}", fs, enc));
        return Err(format!(
            "/{}/{} arrived damaged (CRC {} on the device, {} here) and was removed - upload it again",
            fs, remote, device_crc, local_crc
        )
        .into());
    }
    Ok(())
}

pub struct HttpConn {
    host: String,
    port: u16,
    text: String,
}

impl DeviceConn for HttpConn {
    fn rest_raw(&self, cmd: &str) -> CmdResult<Vec<u8>> {
        // The body is returned whatever the HTTP status: the device's JSON says what went wrong
        Ok(http_get(&self.host, self.port, &format!("/api/{}", cmd))?.body)
    }

    fn put_file(&self, local: &Path, remote: &str, fs: &str, progress: &mut ProgressFn) -> CmdResult<()> {
        // The multipart upload has no field for the file system, so it is named in the file name (/sd/... or
        // /local/...), which the device uses in place of its default file system; '/' separators in the name are
        // kept for sub-folders
        let local = local.to_str().ok_or("local file path isn't valid UTF-8")?;
        let file_name = format!("/{}/{}", fs, remote.trim_start_matches('/'));
        let resp = http_post_multipart_file(&self.host, self.port, "/api/fileupload", "file", &file_name, local, Some(progress))?;
        let body = resp.body_string();
        if !(resp.is_success() && body.contains("\"rslt\":\"ok\"")) {
            return Err(format!("Upload failed (HTTP {}): {}", resp.status, body).into());
        }
        // The reply doesn't show the file arrived intact (older firmware replies ok even if a block was lost)
        let data = std::fs::read(local)?;
        check_uploaded(self, &data, remote, fs)
    }

    fn wait_for_restart(&self, timeout: Duration) -> bool {
        let start = Instant::now();
        // Give it time to go down first
        std::thread::sleep(Duration::from_secs(2));
        while start.elapsed() < timeout {
            if let Ok(resp) = http_get_timeout(&self.host, self.port, "/api/v", Duration::from_secs(2)) {
                if resp.is_success() {
                    return true;
                }
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        false
    }

    fn describe(&self) -> &str {
        &self.text
    }

    fn link_kind(&self) -> LinkKind {
        LinkKind::Http
    }
}

// ---------------------------------------------------------------------------
// Device specs: how a device is named on the command line
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeviceSpec {
    // 192.168.86.52, host, host:8080, http://host[:port]
    Http { host: String, port: Option<u16> },
    // ws://host[:port][/path], path default /ws
    WebSocket { host: String, port: Option<u16>, path: String },
    // serial:<port>[@baud], serial: to pick the port as raft monitor does
    Serial { port: Option<String>, baud: Option<u32> },
    // ble:<name, name prefix or address>, ble: for the only one found
    Ble { target: Option<String> },
}

// The text after `scheme` if `text` starts with it (ignoring case)
fn strip_scheme<'a>(text: &'a str, scheme: &str) -> Option<&'a str> {
    if text.len() >= scheme.len() && text.is_char_boundary(scheme.len()) && text[..scheme.len()].eq_ignore_ascii_case(scheme) {
        Some(&text[scheme.len()..])
    } else {
        None
    }
}

// host[:port] into its parts
fn parse_host_port(text: &str, whole: &str) -> CmdResult<(String, Option<u16>)> {
    let (host, port) = match text.rsplit_once(':') {
        Some((host, port)) => {
            let port: u16 = port.parse().map_err(|_| format!("invalid port '{}' in device '{}'", port, whole))?;
            (host, Some(port))
        }
        None => (text, None),
    };
    if host.is_empty() {
        return Err(format!("no host in device '{}'", whole).into());
    }
    Ok((host.to_string(), port))
}

pub fn parse_device_spec(text: &str) -> CmdResult<DeviceSpec> {
    let text = text.trim();
    if text.is_empty() {
        return Err("the device is empty".into());
    }
    if let Some(rest) = strip_scheme(text, "http://") {
        let authority = rest.strip_suffix('/').unwrap_or(rest);
        if authority.contains('/') {
            return Err(format!("device '{}' can't include a path", text).into());
        }
        let (host, port) = parse_host_port(authority, text)?;
        return Ok(DeviceSpec::Http { host, port });
    }
    if let Some(rest) = strip_scheme(text, "ws://") {
        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, ""),
        };
        let path = if path.is_empty() || path == "/" { "/ws" } else { path };
        let (host, port) = parse_host_port(authority, text)?;
        return Ok(DeviceSpec::WebSocket { host, port, path: path.to_string() });
    }
    if let Some(rest) = strip_scheme(text, "serial:") {
        let (port, baud) = match rest.rsplit_once('@') {
            Some((port, baud)) => {
                let baud: u32 = baud.parse().map_err(|_| format!("invalid baud rate '{}' in device '{}'", baud, text))?;
                (port, Some(baud))
            }
            None => (rest, None),
        };
        let port = if port.is_empty() { None } else { Some(port.to_string()) };
        return Ok(DeviceSpec::Serial { port, baud });
    }
    if let Some(rest) = strip_scheme(text, "ble:") {
        let target = if rest.is_empty() { None } else { Some(rest.to_string()) };
        return Ok(DeviceSpec::Ble { target });
    }
    if let Some((scheme, _)) = text.split_once("://") {
        return Err(format!("'{}://' devices aren't supported (use an IP address or hostname)", scheme).into());
    }
    let (host, port) = parse_host_port(text, text)?;
    Ok(DeviceSpec::Http { host, port })
}

// Open a connection to the device named by `text`
pub fn open_device(text: &str, options: &ConnOptions) -> CmdResult<Rc<dyn DeviceConn>> {
    let spec = parse_device_spec(text)?;
    if options.http_port.is_some() && !matches!(spec, DeviceSpec::Http { .. }) {
        return Err(format!("-p/--port is the HTTP port - it doesn't apply to {}", text.trim()).into());
    }
    match spec {
        DeviceSpec::Serial { port, baud } => {
            let port = match port {
                Some(port) => port,
                // As raft monitor picks one (in WSL this runs in the Windows raft.exe, see hand_off_to_windows)
                None => select_most_likely_port(&PortsCmd::new_with_vid(None), true)
                    .map(|p| p.port_name)
                    .ok_or("no serial port found - name it, e.g. serial:COM4 or serial:/dev/ttyACM0")?,
            };
            let text_sink = if options.show_log { TextSink::Stderr } else { TextSink::Drop };
            let link = SerialLink::open(&port, baud.unwrap_or(SERIAL_BAUD_DEFAULT), text_sink)?;
            Ok(Rc::new(RicrestConn::new(link, &format!("serial:{}", port), LinkKind::Serial, options.verbose)))
        }
        DeviceSpec::WebSocket { host, port, path } => {
            let link = WebSocketLink::open(&host, port.unwrap_or(80), &path)?;
            Ok(Rc::new(RicrestConn::new(link, text.trim(), LinkKind::WebSocket, options.verbose)))
        }
        #[cfg(feature = "ble")]
        DeviceSpec::Ble { target } => {
            let link = BleLink::open(target.as_deref(), &options.ble.options())?;
            Ok(Rc::new(RicrestConn::new(link, text.trim(), LinkKind::Ble, options.verbose)))
        }
        DeviceSpec::Http { host, port } => {
            let port = match (port, options.http_port) {
                (Some(a), Some(b)) if a != b => {
                    return Err(format!("the port is given twice: {} in the device and {} with -p/--port", a, b).into())
                }
                (Some(port), _) | (None, Some(port)) => port,
                (None, None) => 80,
            };
            Ok(Rc::new(HttpConn { host, port, text: text.trim().to_string() }))
        }
        #[cfg(not(feature = "ble"))]
        DeviceSpec::Ble { .. } => Err(format!(
            "BLE devices ({}) need a raftcli built with the ble feature (it is on by default: cargo install raftcli)",
            text.trim()
        )
        .into()),
    }
}

// ---------------------------------------------------------------------------
// The device on the command line
// ---------------------------------------------------------------------------

// The positional arguments of a device command, after the device has been taken from them
pub struct Positionals {
    values: VecDeque<String>,
}

impl Positionals {
    pub fn required(&mut self, name: &str) -> CmdResult<String> {
        self.values.pop_front().ok_or_else(|| format!("missing argument <{}>", name).into())
    }

    pub fn optional(&mut self) -> Option<String> {
        self.values.pop_front()
    }

    // A value without taking it (i is counted from the first remaining)
    pub fn get(&self, i: usize) -> Option<&String> {
        self.values.get(i)
    }

    pub fn finish(self) -> CmdResult<()> {
        match self.values.front() {
            Some(extra) => Err(format!("unexpected argument '{}'", extra).into()),
            None => Ok(()),
        }
    }
}

// The device for a command, and its other positional arguments. The device is -d/--device if given, else
// RAFT_DEVICE if set, else the first positional argument - so with RAFT_DEVICE set, `raft fs get config.json`
// works, and without it existing commands (`raft fs get 192.168.86.52 config.json`) are unchanged.
// `positionals` are clap's values in order (those given come first).
pub fn resolve_device(device_opt: &Option<String>, positionals: Vec<Option<String>>) -> CmdResult<(String, Positionals)> {
    resolve_device_from(device_opt, std::env::var(DEVICE_ENV).ok(), positionals)
}

fn resolve_device_from(
    device_opt: &Option<String>,
    device_env: Option<String>,
    positionals: Vec<Option<String>>,
) -> CmdResult<(String, Positionals)> {
    let mut values: VecDeque<String> = positionals.into_iter().flatten().collect();
    let given = device_opt.clone().or(device_env).filter(|d| !d.trim().is_empty());
    let device = match given {
        Some(device) => device,
        None => values.pop_front().ok_or_else(|| {
            format!(
                "no device given - name it first (e.g. raft fs ls 192.168.86.52), with -d/--device, or in {}",
                DEVICE_ENV
            )
        })?,
    };
    Ok((device, Positionals { values }))
}

// In WSL there are no USB serial ports (or Bluetooth), so a command for a serial (or BLE) device is run by the
// Windows raftcli instead, as raft flash does, unless -n/--native-serial-port. Local absolute paths among
// `local_paths` are translated for Windows; relative ones work as they are, as WSL starts raft.exe in the same
// folder. Returns true if the command was handed off (and has run)
pub fn hand_off_to_windows(device: &str, native_serial_port: bool, local_paths: &[String]) -> CmdResult<bool> {
    // An invalid device is reported the same way everywhere
    let spec = parse_device_spec(device)?;
    if native_serial_port || !is_wsl() {
        return Ok(false);
    }
    if !matches!(spec, DeviceSpec::Serial { .. } | DeviceSpec::Ble { .. }) {
        return Ok(false);
    }
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    for arg in args.iter_mut() {
        if arg.starts_with('/') && local_paths.contains(arg) {
            let out = std::process::Command::new("wslpath").arg("-w").arg(arg.as_str()).output()?;
            if out.status.success() {
                *arg = String::from_utf8_lossy(&out.stdout).trim().to_string();
            }
        }
    }
    // Windows programs don't see WSL's environment, so a device from RAFT_DEVICE is passed on with -d
    if !args.iter().any(|a| a == device) {
        args.push("-d".to_string());
        args.push(device.to_string());
    }
    args.push("-n".to_string());
    run_windows_raft(&args, device)?;
    Ok(true)
}

// Run the Windows raftcli with `args` (from WSL), for `what` (in messages)
pub fn run_windows_raft(args: &[String], what: &str) -> CmdResult<()> {
    let exe = std::env::var(WINDOWS_RAFT_ENV).unwrap_or_else(|_| "raft.exe".to_string());
    eprintln!("WSL: handing {} to the Windows {}", what, exe);
    let status = std::process::Command::new(&exe).args(args).status().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!(
                "can't run {} - in WSL, serial and BLE devices need the Windows raftcli (cargo install raftcli on \
                 Windows, or set {} to its path), or -n to use this system's ports",
                exe, WINDOWS_RAFT_ENV
            )
        } else {
            format!("can't run {}: {}", exe, e)
        }
    })?;
    match status.code() {
        Some(0) => Ok(()),
        // clap's usage error: probably an older raft.exe without these devices
        Some(2) => Err(format!("{} rejected the command - it may be an older raftcli without serial and BLE devices", exe).into()),
        code => Err(format!("{} failed (exit code {:?})", exe, code).into()),
    }
}

#[cfg(test)]
pub(crate) mod fake;

#[cfg(test)]
mod tests {
    use super::*;

    fn http(host: &str, port: Option<u16>) -> DeviceSpec {
        DeviceSpec::Http { host: host.to_string(), port }
    }

    #[test]
    fn device_specs() {
        assert_eq!(parse_device_spec("192.168.86.52").unwrap(), http("192.168.86.52", None));
        assert_eq!(parse_device_spec(" axiom.local ").unwrap(), http("axiom.local", None));
        assert_eq!(parse_device_spec("host:8080").unwrap(), http("host", Some(8080)));
        assert_eq!(parse_device_spec("http://host").unwrap(), http("host", None));
        assert_eq!(parse_device_spec("HTTP://host:81/").unwrap(), http("host", Some(81)));
        assert_eq!(
            parse_device_spec("ws://192.168.86.52").unwrap(),
            DeviceSpec::WebSocket { host: "192.168.86.52".into(), port: None, path: "/ws".into() }
        );
        assert_eq!(
            parse_device_spec("ws://host:8080/sock").unwrap(),
            DeviceSpec::WebSocket { host: "host".into(), port: Some(8080), path: "/sock".into() }
        );
        assert_eq!(parse_device_spec("serial:").unwrap(), DeviceSpec::Serial { port: None, baud: None });
        assert_eq!(
            parse_device_spec("serial:COM4@921600").unwrap(),
            DeviceSpec::Serial { port: Some("COM4".into()), baud: Some(921600) }
        );
        assert_eq!(
            parse_device_spec("serial:/dev/ttyACM0").unwrap(),
            DeviceSpec::Serial { port: Some("/dev/ttyACM0".into()), baud: None }
        );
        assert_eq!(parse_device_spec("ble:").unwrap(), DeviceSpec::Ble { target: None });
        assert_eq!(parse_device_spec("BLE:Axiom018").unwrap(), DeviceSpec::Ble { target: Some("Axiom018".into()) });
    }

    #[test]
    fn bad_device_specs() {
        for bad in ["", "host:", "host:http", ":80", "http://host/api", "https://host", "wss://host", "serial:COM4@fast"] {
            assert!(parse_device_spec(bad).is_err(), "'{}' should be rejected", bad);
        }
    }

    #[test]
    fn open_http_devices() {
        assert_eq!(open_device("host", &ConnOptions::default()).unwrap().describe(), "host");
        assert_eq!(open_device("host", &ConnOptions { http_port: Some(8080), ..Default::default() }).unwrap().link_kind(), LinkKind::Http);
        assert!(open_device("host:81", &ConnOptions { http_port: Some(80), ..Default::default() }).is_err());
        assert!(open_device("host:81", &ConnOptions { http_port: Some(81), ..Default::default() }).is_ok());
        // Without the ble feature BLE devices are refused (with it, opening one would use Bluetooth)
        #[cfg(not(feature = "ble"))]
        {
            let err = open_device("ble:Axiom018", &ConnOptions::default()).err().unwrap().to_string();
            assert!(err.contains("the ble feature"), "{}", err);
        }
        // -p is for HTTP only
        let err = open_device("serial:COM4", &ConnOptions { http_port: Some(80), ..Default::default() }).err().unwrap();
        assert!(err.to_string().contains("HTTP port"), "{}", err);
    }

    #[test]
    fn only_serial_and_ble_devices_are_handed_to_windows() {
        // (A serial device without -n would run raft.exe when the tests run in WSL, so isn't tried here)
        assert!(!hand_off_to_windows("192.168.86.52", false, &[]).unwrap());
        assert!(!hand_off_to_windows("ws://host", false, &[]).unwrap());
        assert!(!hand_off_to_windows("serial:COM4", true, &[]).unwrap());
        assert!(hand_off_to_windows("bad:", false, &[]).is_err());
    }

    #[test]
    fn device_from_command_line() {
        let some = |v: &str| Some(v.to_string());
        // Device first, as before
        let (device, mut rest) = resolve_device_from(&None, None, vec![some("1.2.3.4"), some("a.txt"), None]).unwrap();
        assert_eq!(device, "1.2.3.4");
        assert_eq!(rest.required("REMOTE").unwrap(), "a.txt");
        assert_eq!(rest.optional(), None);
        assert!(rest.finish().is_ok());
        // From the environment: every positional is the command's
        let (device, mut rest) = resolve_device_from(&None, some("serial:COM4"), vec![some("a.txt"), some("b.txt"), None]).unwrap();
        assert_eq!(device, "serial:COM4");
        assert_eq!(rest.required("REMOTE").unwrap(), "a.txt");
        assert_eq!(rest.optional().as_deref(), Some("b.txt"));
        // -d wins over the environment
        let (device, _) = resolve_device_from(&some("host"), some("serial:COM4"), vec![]).unwrap();
        assert_eq!(device, "host");
        // An empty environment variable is ignored
        let (device, _) = resolve_device_from(&None, some(""), vec![some("host")]).unwrap();
        assert_eq!(device, "host");
        // Nothing at all
        assert!(resolve_device_from(&None, None, vec![None, None]).is_err());
        // Too many
        let (_, mut rest) = resolve_device_from(&some("host"), None, vec![some("a"), some("b")]).unwrap();
        rest.required("REMOTE").unwrap();
        assert!(rest.finish().is_err());
        // A required argument missing
        let (_, mut rest) = resolve_device_from(&None, None, vec![some("host"), None]).unwrap();
        assert!(rest.required("REMOTE").is_err());
    }
}
