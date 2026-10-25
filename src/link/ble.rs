// RaftCLI: RICREST over BLE
// The device's RICREST BLE service (RaftSysMods BLEMan) takes HDLC frames written to its command characteristic,
// split into writes of up to the MTU, and sends HDLC frames back as indications on its response characteristic.
// btleplug is async, so a thread runs a single-threaded tokio runtime that owns the connection: writes go to it over
// a channel, and notifications are decoded into frames on another, so the rest of RaftCLI stays blocking.
// Windows keeps the BLE link up after the program disconnects or exits, and the device doesn't advertise while it is
// connected, so the device is always told to release the link (bledisconnect) when the link closes - including on
// Ctrl-C (devdocs/fs-over-serial-ws-ble-plan.md §6).
// Rob Dobson 2026

use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicU16, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use btleplug::api::{
    BDAddr, Central, CentralEvent, CharPropFlags, Characteristic, Manager as _, Peripheral as _, ScanFilter, WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral, PeripheralId};
use futures::StreamExt;
use uuid::Uuid;

use super::{encode_wire, Link, WireDecoder, WireFormat};
use crate::device_conn::CmdResult;
use crate::ricrest::frame::{text_frame, ElemCode, MsgType};

// RICREST BLE services: the RaftSysMods default (and raftjs's first choice), the one apps made by raft new before
// 2026-10-23 advertise, and one raftjs also accepts
const RAFT_SERVICES: [&str; 3] = [
    "aa76677e-9cfd-4626-a510-0d305be57c8d",
    "bb76677e-9cfd-4626-a510-0d305be57c8d",
    "da903f65-d5c2-4f4d-a065-d1aade7af874",
];

// Client Characteristic Configuration descriptor
const CCCD_UUID: Uuid = Uuid::from_u128(0x00002902_0000_1000_8000_00805f9b34fb);

// Write size when the MTU isn't known (raftjs web uses the same), and the largest Windows accepts (MTU 512 - 3)
const WRITE_SIZE_DEFAULT: usize = 182;
const WRITE_SIZE_MAX: usize = 509;
const ATT_HEADER_LEN: usize = 3;
const MTU_DEFAULT: u16 = 23;

// Scanning for a named device stops as soon as it is seen, but Windows can take several seconds to report one
const SCAN_FOR_NAMED_TIME: Duration = Duration::from_secs(10);
// A name prefix matched: how long to keep scanning for an exact match
const PREFIX_GRACE: Duration = Duration::from_millis(1500);
// With no device named, how long to scan before taking the only one found (long enough to see every device in
// range: a device can take over 2 s to be seen)
const SCAN_ANY_TIME: Duration = Duration::from_secs(5);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
// Connecting to a cached address: if the device isn't there, scan instead without waiting long
const CACHED_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
// Connecting and finding the service: attempts, and the wait between them
const SETUP_ATTEMPTS: u32 = 3;
const SETUP_RETRY_DELAY: Duration = Duration::from_millis(1500);
// Scan, connect and service discovery together
const OPEN_TIMEOUT: Duration = Duration::from_secs(60);

// Releasing the link: bledisconnect is sent up to this many times, waiting this long for the reply each time
const RELEASE_ATTEMPTS: u32 = 3;
const RELEASE_REPLY_TIMEOUT: Duration = Duration::from_secs(2);
// The msgNum of bledisconnect (its reply is taken by the link, not passed on)
const RELEASE_MSG_NUM: u8 = 255;
const RELEASE_NOT_WAITING: u16 = 0x100;

// Overrides for devices with other UUIDs or a known MTU (--ble-service, --ble-cmd, --ble-resp, --ble-mtu)
#[derive(Clone, Debug, Default)]
pub struct BleOptions {
    pub service: Option<String>,
    pub cmd: Option<String>,
    pub resp: Option<String>,
    pub mtu: Option<u16>,
}

// A Raft device seen in a scan
#[derive(Clone, Debug)]
pub struct FoundDevice {
    pub name: String,
    pub address: String,
    pub rssi: Option<i16>,
    pub service: Uuid,
}

enum Command {
    Write(Vec<u8>, SyncSender<Result<(), String>>),
    // Send bledisconnect (if `release`), disconnect and end the thread; replies true if the device acknowledged
    Close { release: bool, done: SyncSender<bool> },
}

pub struct BleLink {
    target: Option<String>,
    options: BleOptions,
    // The device connected to, for messages
    name: String,
    commands: CommandSender,
    frames: Receiver<Vec<u8>>,
    bad_frames: Arc<AtomicU32>,
    thread: Option<JoinHandle<()>>,
    closed: bool,
}

// What the BLE thread reports once it has connected
struct Connected {
    name: String,
    address: String,
    write_size: usize,
}

fn uuid_option(text: &Option<String>, what: &str) -> CmdResult<Option<Uuid>> {
    match text {
        Some(t) => Ok(Some(Uuid::from_str(t.trim()).map_err(|_| format!("invalid {} UUID '{}'", what, t))?)),
        None => Ok(None),
    }
}

fn service_uuids(options: &BleOptions) -> CmdResult<Vec<Uuid>> {
    Ok(match uuid_option(&options.service, "--ble-service")? {
        Some(uuid) => vec![uuid],
        None => RAFT_SERVICES.iter().map(|s| Uuid::from_str(s).unwrap()).collect(),
    })
}

// Run `f` on a single-threaded tokio runtime (for scanning, outside a link)
fn block_on<F: std::future::Future>(f: F) -> CmdResult<F::Output> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    Ok(runtime.block_on(f))
}

async fn adapter() -> Result<Adapter, String> {
    let manager = Manager::new().await.map_err(|e| format!("Bluetooth isn't available: {}", e))?;
    manager
        .adapters()
        .await
        .map_err(|e| format!("Bluetooth isn't available: {}", e))?
        .into_iter()
        .next()
        .ok_or_else(|| "no Bluetooth adapter found".to_string())
}

// What a scan has seen so far. The service UUID is in the advertisement and the name in the scan response (a
// separate packet), so a device is a Raft device if any packet from its address had a Raft service, and its name is
// whatever the latest packet from that address gave
#[derive(Default)]
struct ScanState {
    raft: HashMap<PeripheralId, Uuid>,
    names: HashMap<PeripheralId, String>,
    rssi: HashMap<PeripheralId, i16>,
    addresses: HashMap<PeripheralId, String>,
}

impl ScanState {
    async fn update(&mut self, adapter: &Adapter, id: &PeripheralId, services: &[Uuid]) {
        let Ok(p) = adapter.peripheral(id).await else { return };
        let Ok(Some(props)) = p.properties().await else { return };
        self.addresses.insert(id.clone(), props.address.to_string());
        if let Some(name) = props.local_name.filter(|n| !n.is_empty()) {
            self.names.insert(id.clone(), name);
        }
        if let Some(rssi) = props.rssi {
            self.rssi.insert(id.clone(), rssi);
        }
        if let Some(service) = props.services.iter().find(|u| services.contains(u)) {
            self.raft.insert(id.clone(), *service);
        }
    }

    fn devices(&self) -> Vec<(FoundDevice, PeripheralId)> {
        let mut found: Vec<(FoundDevice, PeripheralId)> = self
            .raft
            .iter()
            .map(|(id, service)| {
                let device = FoundDevice {
                    name: self.names.get(id).cloned().unwrap_or_default(),
                    address: self.addresses.get(id).cloned().unwrap_or_default(),
                    rssi: self.rssi.get(id).copied(),
                    service: *service,
                };
                (device, id.clone())
            })
            .collect();
        found.sort_by(|(a, _), (b, _)| a.name.cmp(&b.name).then(a.address.cmp(&b.address)));
        found
    }
}

// How well a device matches what was asked for
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Match {
    None,
    Prefix,
    Exact,
}

fn same_address(a: &str, b: &str) -> bool {
    let strip = |s: &str| s.chars().filter(|c| c.is_ascii_hexdigit()).collect::<String>().to_ascii_lowercase();
    let (a, b) = (strip(a), strip(b));
    a.len() == 12 && a == b
}

fn match_device(device: &FoundDevice, target: &str) -> Match {
    if device.name == target || same_address(&device.address, target) {
        Match::Exact
    } else if !device.name.is_empty() && device.name.starts_with(target) {
        Match::Prefix
    } else {
        Match::None
    }
}

// Scan for Raft devices. With a target, stops as soon as it is found (exactly, or by prefix with nothing better
// soon after); without one, scans for `duration`
async fn scan(
    adapter: &Adapter,
    services: &[Uuid],
    target: Option<&str>,
    duration: Duration,
) -> Result<Vec<(FoundDevice, PeripheralId)>, String> {
    // No ScanFilter: on Windows it matches nothing with several UUIDs, and drops the scan response (with the name)
    // with one
    adapter.start_scan(ScanFilter::default()).await.map_err(|e| format!("can't start a BLE scan: {}", e))?;
    let mut events = adapter.events().await.map_err(|e| format!("can't start a BLE scan: {}", e))?;
    let mut state = ScanState::default();
    let start = tokio::time::Instant::now();
    let mut deadline = start + duration;
    let mut prefix_seen = false;
    loop {
        let event = match tokio::time::timeout_at(deadline, events.next()).await {
            Ok(Some(event)) => event,
            _ => break,
        };
        match event {
            CentralEvent::DeviceDiscovered(id) | CentralEvent::DeviceUpdated(id) => state.update(adapter, &id, services).await,
            CentralEvent::ServicesAdvertisement { id, services: advertised } => {
                if let Some(service) = advertised.iter().find(|u| services.contains(u)) {
                    state.raft.insert(id.clone(), *service);
                }
                state.update(adapter, &id, services).await;
            }
            _ => continue,
        }
        if let Some(target) = target {
            let best = state.devices().iter().map(|(d, _)| match_device(d, target)).max().unwrap_or(Match::None);
            if best == Match::Exact {
                break;
            }
            if best == Match::Prefix && !prefix_seen {
                prefix_seen = true;
                deadline = deadline.min(tokio::time::Instant::now() + PREFIX_GRACE);
            }
        }
    }
    let _ = adapter.stop_scan().await;
    Ok(state.devices())
}

fn not_found_hint() -> &'static str {
    "If it is in range, it may still be connected to something else (Windows can keep a BLE link after a program \
     exits, and a connected device doesn't advertise). Release it by typing bledisconnect in raft monitor, with \
     http://<device>/api/bledisconnect over WiFi, or by restarting it"
}

fn list_devices(devices: &[FoundDevice]) -> String {
    devices.iter().map(|d| format!("{} ({})", if d.name.is_empty() { "?" } else { &d.name }, d.address)).collect::<Vec<_>>().join(", ")
}

// Choose the device to connect to from a scan (its index in `devices`)
fn choose(devices: &[FoundDevice], target: Option<&str>, scan_time: Duration) -> Result<usize, String> {
    match target {
        None => match devices.len() {
            1 => Ok(0),
            0 => Err(format!("no Raft BLE device found in {} s. {}", scan_time.as_secs(), not_found_hint())),
            _ => Err(format!("more than one Raft BLE device found - name one with ble:<name>: {}", list_devices(devices))),
        },
        Some(target) => {
            if let Some(i) = devices.iter().position(|d| match_device(d, target) == Match::Exact) {
                return Ok(i);
            }
            let prefixed: Vec<usize> = (0..devices.len()).filter(|&i| match_device(&devices[i], target) == Match::Prefix).collect();
            match prefixed.len() {
                1 => Ok(prefixed[0]),
                0 if devices.is_empty() => Err(format!("BLE device '{}' not found (no Raft devices seen in {} s). {}", target, scan_time.as_secs(), not_found_hint())),
                0 => Err(format!("BLE device '{}' not found - Raft devices seen: {}. {}", target, list_devices(devices), not_found_hint())),
                _ => {
                    let matches: Vec<FoundDevice> = prefixed.iter().map(|&i| devices[i].clone()).collect();
                    Err(format!("more than one BLE device starts with '{}': {}", target, list_devices(&matches)))
                }
            }
        }
    }
}

// Scan for Raft devices (raft ble scan)
pub fn scan_devices(duration: Duration, options: &BleOptions) -> CmdResult<Vec<FoundDevice>> {
    let services = service_uuids(options)?;
    let found = block_on(async {
        let adapter = adapter().await?;
        scan(&adapter, &services, None, duration).await
    })??;
    Ok(found.into_iter().map(|(device, _)| device).collect())
}

// ---------------------------------------------------------------------------
// Address cache: connecting to a known address skips the scan (on Windows, where that is possible)
// ---------------------------------------------------------------------------

fn cache_path() -> Option<PathBuf> {
    dirs::cache_dir().map(|dir| dir.join("raftcli").join("ble_devices.json"))
}

// The cached address of the device named `target`, or of the only cached device whose name starts with it
fn cached_address(target: &str) -> Option<String> {
    let text = std::fs::read_to_string(cache_path()?).ok()?;
    let cache: HashMap<String, String> = serde_json::from_str(&text).ok()?;
    cached_match(&cache, target)
}

fn cached_match(cache: &HashMap<String, String>, target: &str) -> Option<String> {
    if let Some(address) = cache.get(target) {
        return Some(address.clone());
    }
    let mut prefixed = cache.iter().filter(|(name, _)| name.starts_with(target));
    match (prefixed.next(), prefixed.next()) {
        (Some((_, address)), None) => Some(address.clone()),
        _ => None,
    }
}

fn remember_address(name: &str, address: &str) {
    let Some(path) = cache_path() else { return };
    let mut cache: HashMap<String, String> =
        std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    if cache.get(name).map(|a| a.as_str()) == Some(address) {
        return;
    }
    cache.insert(name.to_string(), address.to_string());
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string_pretty(&cache) {
        let _ = std::fs::write(path, text);
    }
}

// The peripheral at a cached address, without scanning (Windows only: elsewhere a device must be seen first)
#[cfg(windows)]
async fn cached_peripheral(adapter: &Adapter, address: &str) -> Option<Peripheral> {
    let addr = BDAddr::from_str_delim(address).ok()?;
    adapter.add_peripheral(&PeripheralId::from(addr)).await.ok()
}

#[cfg(not(windows))]
async fn cached_peripheral(_adapter: &Adapter, _address: &str) -> Option<Peripheral> {
    None
}

// ---------------------------------------------------------------------------
// The BLE thread
// ---------------------------------------------------------------------------

struct Session {
    peripheral: Peripheral,
    cmd: Characteristic,
    write_type: WriteType,
    write_size: usize,
}

// Find the device (from the cache or a scan) and connect
async fn find_and_connect(adapter: &Adapter, target: Option<&str>, services: &[Uuid]) -> Result<(Peripheral, String, String), String> {
    // A device named by name or address that has been connected to before
    if let Some(target) = target {
        let cached = if BDAddr::from_str_delim(target).is_ok() { Some(target.to_string()) } else { cached_address(target) };
        if let Some(address) = cached {
            if let Some(p) = cached_peripheral(adapter, &address).await {
                if matches!(tokio::time::timeout(CACHED_CONNECT_TIMEOUT, p.connect()).await, Ok(Ok(()))) {
                    let name = p.properties().await.ok().flatten().and_then(|pr| pr.local_name).unwrap_or_else(|| target.to_string());
                    return Ok((p, name, address));
                }
                let _ = p.disconnect().await;
            }
        }
    }

    let scan_time = if target.is_some() { SCAN_FOR_NAMED_TIME } else { SCAN_ANY_TIME };
    let found = scan(adapter, services, target, scan_time).await?;
    let devices: Vec<FoundDevice> = found.iter().map(|(d, _)| d.clone()).collect();
    let (device, id) = found[choose(&devices, target, scan_time)?].clone();
    let p = adapter.peripheral(&id).await.map_err(|e| format!("BLE device {} went away: {}", device.address, e))?;
    match tokio::time::timeout(CONNECT_TIMEOUT, p.connect()).await {
        Ok(Ok(())) => Ok((p, device.name, device.address)),
        Ok(Err(e)) => Err(format!("can't connect to BLE device {} ({}): {}. {}", device.name, device.address, e, not_found_hint())),
        Err(_) => Err(format!("connecting to BLE device {} ({}) timed out. {}", device.name, device.address, not_found_hint())),
    }
}

// The characteristics of a service, for errors
fn describe_service(service: &btleplug::api::Service) -> String {
    if service.characteristics.is_empty() {
        return "it has no characteristics - Windows may have refused access to it".into();
    }
    service.characteristics.iter().map(|c| format!("{} {:?}", c.uuid, c.properties)).collect::<Vec<_>>().join(", ")
}

// Find the RICREST service and characteristics, and subscribe to the responses
async fn setup(p: &Peripheral, services: &[Uuid], options: &BleOptions) -> Result<(Session, Characteristic), String> {
    p.discover_services().await.map_err(|e| format!("BLE service discovery failed: {}", e))?;
    let service = p
        .services()
        .into_iter()
        .find(|s| services.contains(&s.uuid))
        .ok_or("the device has no RICREST BLE service (or Windows refused access to it)")?;
    let cmd_uuid = uuid_option(&options.cmd, "--ble-cmd").map_err(|e| e.to_string())?;
    let resp_uuid = uuid_option(&options.resp, "--ble-resp").map_err(|e| e.to_string())?;
    let cmd = service
        .characteristics
        .iter()
        .find(|c| match cmd_uuid {
            Some(uuid) => c.uuid == uuid,
            None => c.properties.intersects(CharPropFlags::WRITE | CharPropFlags::WRITE_WITHOUT_RESPONSE)
                && !c.properties.intersects(CharPropFlags::NOTIFY | CharPropFlags::INDICATE),
        })
        .cloned()
        .ok_or_else(|| format!("the RICREST BLE service has no command characteristic ({})", describe_service(&service)))?;
    let resp = service
        .characteristics
        .iter()
        .find(|c| match resp_uuid {
            Some(uuid) => c.uuid == uuid,
            None => c.properties.intersects(CharPropFlags::NOTIFY | CharPropFlags::INDICATE),
        })
        .cloned()
        .ok_or_else(|| format!("the RICREST BLE service has no response characteristic ({})", describe_service(&service)))?;

    p.subscribe(&resp).await.map_err(|e| format!("can't subscribe to the device's responses: {}", e))?;
    // btleplug on Windows writes indicate-only to the CCCD, which firmware before 2026-10-23 doesn't count as a
    // subscription, so it never replies. Notify works with old and new firmware (the device sends indications
    // anyway). BlueZ manages the CCCD itself and may refuse this, which is fine
    if let Some(cccd) = resp.descriptors.iter().find(|d| d.uuid == CCCD_UUID) {
        let _ = p.write_descriptor(cccd, &[0x01, 0x00]).await;
    }

    // Write size from the MTU (read after subscribing: earlier it can still be the default)
    let write_size = match options.mtu {
        Some(mtu) => (mtu as usize).saturating_sub(ATT_HEADER_LEN),
        None if p.mtu() > MTU_DEFAULT => (p.mtu() as usize - ATT_HEADER_LEN).min(WRITE_SIZE_MAX),
        None => WRITE_SIZE_DEFAULT,
    }
    .max(20);
    let write_type = if cmd.properties.contains(CharPropFlags::WRITE_WITHOUT_RESPONSE) {
        WriteType::WithoutResponse
    } else {
        WriteType::WithResponse
    };
    Ok((Session { peripheral: p.clone(), cmd, write_type, write_size }, resp))
}

async fn write(session: &Session, wire: &[u8]) -> Result<(), String> {
    for chunk in wire.chunks(session.write_size) {
        tokio::time::timeout(WRITE_TIMEOUT, session.peripheral.write(&session.cmd, chunk, session.write_type))
            .await
            .map_err(|_| "BLE write timed out".to_string())?
            .map_err(|e| format!("BLE write failed: {}", e))?;
    }
    Ok(())
}

// Ask the device to release the link; true once it has replied
async fn release(session: &Session, waiting: &AtomicU16, replied: &tokio::sync::Notify) -> bool {
    let frame = text_frame(RELEASE_MSG_NUM, MsgType::Command, ElemCode::Url, "bledisconnect");
    let wire = encode_wire(WireFormat::Hdlc, &frame);
    waiting.store(RELEASE_MSG_NUM as u16, Ordering::SeqCst);
    let mut acknowledged = false;
    for _ in 0..RELEASE_ATTEMPTS {
        if write(session, &wire).await.is_err() {
            break;
        }
        if tokio::time::timeout(RELEASE_REPLY_TIMEOUT, replied.notified()).await.is_ok() {
            acknowledged = true;
            break;
        }
    }
    waiting.store(RELEASE_NOT_WAITING, Ordering::SeqCst);
    acknowledged
}

async fn run(
    target: Option<String>,
    options: BleOptions,
    mut commands: tokio::sync::mpsc::UnboundedReceiver<Command>,
    frames: mpsc::Sender<Vec<u8>>,
    bad_frames: Arc<AtomicU32>,
    connected: SyncSender<Result<Connected, String>>,
) {
    let services = match service_uuids(&options) {
        Ok(s) => s,
        Err(e) => {
            let _ = connected.send(Err(e.to_string()));
            return;
        }
    };
    let opened = async {
        let adapter = adapter().await?;
        let mut attempt = 1;
        loop {
            let (p, name, address) = find_and_connect(&adapter, target.as_deref(), &services).await?;
            match setup(&p, &services, &options).await {
                Ok((session, resp)) => break Ok((session, resp, name, address)),
                Err(e) => {
                    let _ = p.disconnect().await;
                    // Straight after a previous connection is released, Windows can still have the old link up
                    // and the service has no characteristics: wait for it to go, then connect again
                    if attempt >= SETUP_ATTEMPTS {
                        break Err(e);
                    }
                    attempt += 1;
                    tokio::time::sleep(SETUP_RETRY_DELAY).await;
                }
            }
        }
    }
    .await;
    let (session, resp, name, address) = match opened {
        Ok(o) => o,
        Err(e) => {
            let _ = connected.send(Err(e));
            return;
        }
    };

    // Decode notifications into frames, taking the reply to bledisconnect here
    let waiting = Arc::new(AtomicU16::new(RELEASE_NOT_WAITING));
    let replied = Arc::new(tokio::sync::Notify::new());
    let mut notifications = match session.peripheral.notifications().await {
        Ok(n) => n,
        Err(e) => {
            let _ = session.peripheral.disconnect().await;
            let _ = connected.send(Err(format!("can't receive the device's responses: {}", e)));
            return;
        }
    };
    let (task_waiting, task_replied) = (waiting.clone(), replied.clone());
    let resp_uuid = resp.uuid;
    let reader = tokio::spawn(async move {
        let mut decoder = WireDecoder::new(WireFormat::Hdlc);
        let (mut decoded, mut text) = (Vec::new(), Vec::new());
        while let Some(n) = notifications.next().await {
            if n.uuid != resp_uuid {
                continue;
            }
            decoder.feed(&n.value, &mut decoded, &mut text);
            bad_frames.store(decoder.bad_frames(), Ordering::Relaxed);
            for frame in decoded.drain(..) {
                if frame.first().map(|&b| b as u16) == Some(task_waiting.load(Ordering::SeqCst)) {
                    task_replied.notify_one();
                } else if frames.send(frame).is_err() {
                    return;
                }
            }
        }
        // The stream ends when the device disconnects: dropping `frames` tells the link
    });

    let _ = connected.send(Ok(Connected { name, address, write_size: session.write_size }));

    while let Some(command) = commands.recv().await {
        match command {
            Command::Write(wire, done) => {
                let _ = done.send(write(&session, &wire).await);
            }
            Command::Close { release: send_release, done } => {
                let acknowledged = send_release && release(&session, &waiting, &replied).await;
                let _ = session.peripheral.disconnect().await;
                reader.abort();
                let _ = done.send(acknowledged);
                return;
            }
        }
    }
    // The link was dropped without closing (shouldn't happen): still release the device
    release(&session, &waiting, &replied).await;
    let _ = session.peripheral.disconnect().await;
}

// ---------------------------------------------------------------------------
// Ctrl-C: release every open link before exiting
// ---------------------------------------------------------------------------

type CommandSender = tokio::sync::mpsc::UnboundedSender<Command>;

fn open_links() -> &'static Mutex<Vec<(u64, CommandSender)>> {
    static LINKS: OnceLock<Mutex<Vec<(u64, CommandSender)>>> = OnceLock::new();
    LINKS.get_or_init(|| Mutex::new(Vec::new()))
}

fn register_link(commands: &CommandSender) -> u64 {
    static NEXT_ID: AtomicU32 = AtomicU32::new(1);
    static HANDLER: OnceLock<()> = OnceLock::new();
    HANDLER.get_or_init(|| {
        let _ = ctrlc::set_handler(|| {
            let links: Vec<_> = open_links().lock().map(|l| l.iter().map(|(_, c)| c.clone()).collect()).unwrap_or_default();
            if !links.is_empty() {
                eprintln!("\nReleasing the BLE device...");
            }
            for commands in links {
                let (done, finished) = mpsc::sync_channel(1);
                if commands.send(Command::Close { release: true, done }).is_ok() {
                    let _ = finished.recv_timeout(RELEASE_REPLY_TIMEOUT * (RELEASE_ATTEMPTS + 1));
                }
            }
            std::process::exit(130);
        });
    });
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed) as u64;
    if let Ok(mut links) = open_links().lock() {
        links.push((id, commands.clone()));
    }
    id
}

fn unregister_link(commands: &CommandSender) {
    if let Ok(mut links) = open_links().lock() {
        links.retain(|(_, c)| !c.same_channel(commands));
    }
}

// ---------------------------------------------------------------------------
// The link
// ---------------------------------------------------------------------------

impl BleLink {
    // Connect to the device named by `target` (a name, name prefix or address; None for the only one found)
    pub fn open(target: Option<&str>, options: &BleOptions) -> CmdResult<BleLink> {
        let (commands, command_rx) = tokio::sync::mpsc::unbounded_channel();
        let (frame_tx, frames) = mpsc::channel();
        let (connected_tx, connected) = mpsc::sync_channel(1);
        let bad_frames = Arc::new(AtomicU32::new(0));
        let thread_bad_frames = bad_frames.clone();
        let (thread_target, thread_options) = (target.map(|t| t.to_string()), options.clone());
        let thread = std::thread::Builder::new().name("ble".into()).spawn(move || {
            match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(runtime) => runtime.block_on(run(thread_target, thread_options, command_rx, frame_tx, thread_bad_frames, connected_tx)),
                Err(e) => {
                    let _ = connected_tx.send(Err(format!("can't start BLE: {}", e)));
                }
            }
        })?;
        let info = match connected.recv_timeout(OPEN_TIMEOUT) {
            Ok(Ok(info)) => info,
            Ok(Err(e)) => {
                let _ = thread.join();
                return Err(e.into());
            }
            Err(RecvTimeoutError::Timeout) => return Err("connecting over BLE timed out".into()),
            Err(RecvTimeoutError::Disconnected) => {
                let _ = thread.join();
                return Err("the BLE connection failed".into());
            }
        };
        if !info.name.is_empty() {
            remember_address(&info.name, &info.address);
        }
        register_link(&commands);
        if std::env::var("RAFTCLI_BLE_DEBUG").is_ok() {
            eprintln!("BLE: connected to {} ({}), write size {}", info.name, info.address, info.write_size);
        }
        Ok(BleLink {
            target: target.map(|t| t.to_string()),
            options: options.clone(),
            name: if info.name.is_empty() { info.address } else { info.name },
            commands,
            frames,
            bad_frames,
            thread: Some(thread),
            closed: false,
        })
    }

    fn dropped(&self) -> String {
        format!("the BLE link to {} dropped (the device restarted or went out of range)", self.name)
    }

    // Close the link, releasing the device if `release`
    fn close(&mut self, release: bool) {
        if self.closed {
            return;
        }
        self.closed = true;
        unregister_link(&self.commands);
        let (done, finished) = mpsc::sync_channel(1);
        if self.commands.send(Command::Close { release, done }).is_ok() {
            let acknowledged = finished.recv_timeout(RELEASE_REPLY_TIMEOUT * (RELEASE_ATTEMPTS + 1)).unwrap_or(false);
            if release && !acknowledged {
                eprintln!(
                    "Warning: {} didn't confirm it released the BLE link - if it can't be found next time, send it \
                     bledisconnect over serial or WiFi, or restart it",
                    self.name
                );
            }
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Link for BleLink {
    fn send(&mut self, frame: &[u8]) -> CmdResult<()> {
        let (done, result) = mpsc::sync_channel(1);
        self.commands.send(Command::Write(encode_wire(WireFormat::Hdlc, frame), done)).map_err(|_| self.dropped())?;
        match result.recv_timeout(WRITE_TIMEOUT * 2) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(format!("{} ({})", e, self.name).into()),
            Err(_) => Err(self.dropped().into()),
        }
    }

    fn recv(&mut self, timeout: Duration) -> CmdResult<Option<Vec<u8>>> {
        let received = if timeout.is_zero() {
            self.frames.try_recv().map_err(|e| match e {
                TryRecvError::Empty => RecvTimeoutError::Timeout,
                TryRecvError::Disconnected => RecvTimeoutError::Disconnected,
            })
        } else {
            self.frames.recv_timeout(timeout)
        };
        match received {
            Ok(frame) => Ok(Some(frame)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(self.dropped().into()),
        }
    }

    fn bad_frames(&self) -> u32 {
        self.bad_frames.load(Ordering::Relaxed)
    }

    // After a restart the device advertises again: connect again (by the name it had, so a prefix that now
    // matches another device isn't used)
    fn reopen(&mut self, timeout: Duration) -> bool {
        self.close(false);
        let target = Some(self.name.clone()).or_else(|| self.target.clone());
        let start = Instant::now();
        std::thread::sleep(Duration::from_millis(1500));
        while start.elapsed() < timeout {
            if let Ok(link) = BleLink::open(target.as_deref(), &self.options) {
                let mut old = std::mem::replace(self, link);
                old.closed = true;
                return true;
            }
            std::thread::sleep(Duration::from_millis(1000));
        }
        false
    }
}

impl Drop for BleLink {
    fn drop(&mut self) {
        self.close(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(name: &str, address: &str) -> FoundDevice {
        FoundDevice { name: name.into(), address: address.into(), rssi: None, service: Uuid::nil() }
    }

    #[test]
    fn matching_names_and_addresses() {
        let axiom = device("Axiom018_2cc23e", "24:58:7C:2C:C2:3E");
        assert!(match_device(&axiom, "Axiom018_2cc23e") == Match::Exact);
        assert!(match_device(&axiom, "24:58:7c:2c:c2:3e") == Match::Exact);
        assert!(match_device(&axiom, "24587C2CC23E") == Match::Exact);
        assert!(match_device(&axiom, "Axiom018") == Match::Prefix);
        assert!(match_device(&axiom, "Feather") == Match::None);
    }

    #[test]
    fn cached_names() {
        let cache: HashMap<String, String> = [("Axiom018_2cc23e", "DC:B4:D9:2C:C2:3E"), ("Axiom018_aaaaaa", "24:58:7C:AA:AA:AA"), ("FeatherS3_1", "11:22:33:44:55:66")]
            .into_iter()
            .map(|(n, a)| (n.to_string(), a.to_string()))
            .collect();
        assert_eq!(cached_match(&cache, "Axiom018_2cc23e").as_deref(), Some("DC:B4:D9:2C:C2:3E"));
        assert_eq!(cached_match(&cache, "Feather").as_deref(), Some("11:22:33:44:55:66"));
        assert_eq!(cached_match(&cache, "Axiom018"), None);
        assert_eq!(cached_match(&cache, "Marty"), None);
    }

    #[test]
    fn choosing_a_device() {
        let found = vec![
            device("Axiom018_2cc23e", "24:58:7C:2C:C2:3E"),
            device("Axiom018_aaaaaa", "24:58:7C:AA:AA:AA"),
            device("FeatherS3_1", "11:22:33:44:55:66"),
        ];
        let t = Duration::from_secs(5);
        assert_eq!(choose(&found, Some("Axiom018_aaaaaa"), t), Ok(1));
        assert_eq!(choose(&found, Some("11:22:33:44:55:66"), t), Ok(2));
        assert_eq!(choose(&found, Some("Feather"), t), Ok(2));
        assert!(choose(&found, Some("Axiom018"), t).unwrap_err().contains("more than one"));
        assert!(choose(&found, Some("Marty"), t).unwrap_err().contains("not found"));
        assert!(choose(&found, None, t).unwrap_err().contains("more than one"));
        assert_eq!(choose(&found[2..], None, t), Ok(0));
        assert!(choose(&[], None, t).unwrap_err().contains("bledisconnect"));
    }
}
