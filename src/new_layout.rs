// RaftCLI: what raft new works out from the answers - the partition table, an estimate of the app's size, and a
// password for the WiFi access point
// Rob Dobson 2026

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};

const KB: u64 = 1024;
const MB: u64 = 1024 * 1024;

// Before the app: the bootloader and partition table, then NVS (0x9000-0x1E000) and OTA data (0x1E000-0x20000)
const NVS_OFFSET: u64 = 0x9000;
const NVS_SIZE: u64 = 0x15000;
const OTADATA_OFFSET: u64 = 0x1E000;
const OTADATA_SIZE: u64 = 0x2000;
const APP_OFFSET: u64 = 0x20000;
const APP_ALIGN: u64 = 0x10000;
const MAX_APP_SLOT: u64 = 3 * MB;
const MIN_FS: u64 = 512 * KB;

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub ota: bool,
    pub app_slot: u64,
    pub fs_offset: u64,
    pub fs_size: u64,
    pub csv: String,
}

// The partition table for a flash size: app slots as large as possible up to 3 MB, leaving at least 512 KB of file
// system - two slots with OTA updates, one without
pub fn partition_layout(flash_mb: u32, ota: bool) -> Layout {
    let flash = flash_mb as u64 * MB;
    let slots = if ota { 2 } else { 1 };
    let available = flash - APP_OFFSET - MIN_FS;
    let app_slot = ((available / slots).min(MAX_APP_SLOT) / APP_ALIGN) * APP_ALIGN;
    let fs_offset = APP_OFFSET + app_slot * slots;
    let fs_size = flash - fs_offset;
    let mut csv = String::from("# Name,   Type, SubType, Offset,  Size, Flags\n");
    csv += &format!("nvs,      data, nvs,     0x{:06x},  0x{:06x},\n", NVS_OFFSET, NVS_SIZE);
    if ota {
        csv += &format!("otametadata,  data, ota,     0x{:06x},  0x{:06x},\n", OTADATA_OFFSET, OTADATA_SIZE);
        csv += &format!("app0,     app,  ota_0,   0x{:06x},  0x{:06x},\n", APP_OFFSET, app_slot);
        csv += &format!("app1,     app,  ota_1,   0x{:06x},  0x{:06x},\n", APP_OFFSET + app_slot, app_slot);
    } else {
        csv += &format!("app0,     app,  factory, 0x{:06x},  0x{:06x},\n", APP_OFFSET, app_slot);
    }
    csv += &format!("fs,       data, 0x83,    0x{:06x},  0x{:06x},", fs_offset, fs_size);
    Layout { ota, app_slot, fs_offset, fs_size, csv }
}

// What the app includes, for the size estimate
#[derive(Debug, Clone)]
pub struct AppFeatures {
    pub chip: String,
    pub wifi: bool,
    pub ble: bool,
    pub webserver: bool,
    pub i2c: bool,
    pub device_types: String,       // none, common, extended or all
    pub runtime_decode: String,     // none, attrs or code
}

// Measured on an ESP32-S3 with ESP-IDF 6.1 (RaftCLI devdocs/raft-new-questions-and-defaults-review.md section 2): the
// app with everything (WiFi, BLE, web server, I2C and all of RaftCore's device types) and what each choice saves
const FULL_APP: i64 = 1_672_976;
const SAVED_NO_BLE: i64 = 242_608;
const SAVED_NO_WEBSERVER: i64 = 38_432;
const SAVED_NO_I2C: i64 = 174_480;          // and so no device types
const SAVED_NO_DEVICE_TYPES: i64 = 81_168;
const SAVED_COMMON_SET: i64 = 61_264;       // 10 common records instead of all 59
const SAVED_EXTENDED_SET: i64 = 41_000;     // 25 records (about 1.6 KB each)
const SAVED_NO_RUNTIME_DECODE: i64 = 17_744;
const ADDED_DECODE_CODE: i64 = 26_128;
const SAVED_NO_WIFI: i64 = 728_880;         // RAFT_NO_NETWORKING: the WiFi driver, IP stack, mDNS and TLS
const BLE_WITHOUT_WIFI: i64 = 46_496;       // what BLE shares with WiFi (PHY, coexistence, crypto) when there's no WiFi

pub fn estimate_app_bytes(f: &AppFeatures) -> u64 {
    let mut size = FULL_APP;
    if !f.ble { size -= SAVED_NO_BLE; }
    if !f.webserver { size -= SAVED_NO_WEBSERVER; }
    if !f.wifi {
        size -= SAVED_NO_WIFI;
        if f.ble { size += BLE_WITHOUT_WIFI; }
    }
    if !f.i2c {
        size -= SAVED_NO_I2C;
    } else {
        size -= match f.device_types.as_str() {
            "none" => SAVED_NO_DEVICE_TYPES,
            "common" => SAVED_COMMON_SET,
            "extended" => SAVED_EXTENDED_SET,
            _ => 0,
        };
        match f.runtime_decode.as_str() {
            "none" => size -= SAVED_NO_RUNTIME_DECODE,
            "code" => size += ADDED_DECODE_CODE,
            _ => {}
        }
    }
    // RISC-V chips (C3, C5, C6, P4) need more code than the ESP32-S3's Xtensa for the same app: an ESP32-C6 build with
    // WiFi and all device types was 1.16 times the ESP32-S3 estimate (one measurement - approximate)
    if matches!(f.chip.as_str(), "esp32c3" | "esp32c5" | "esp32c6" | "esp32p4") {
        size = size * 116 / 100;
    }
    size.max(0) as u64
}

pub fn human_bytes(bytes: u64) -> String {
    if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else {
        format!("{} KB", (bytes + KB / 2) / KB)
    }
}

// A random password for the WiFi access point (from the OS-seeded hasher keys - no extra crates)
pub fn random_password(len: usize) -> String {
    const CHARS: &[u8] = b"abcdefghjkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut out = String::new();
    let mut counter: u64 = 0;
    while out.len() < len {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(counter);
        hasher.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        let mut v = hasher.finish();
        for _ in 0..8 {
            if out.len() >= len {
                break;
            }
            out.push(CHARS[(v % CHARS.len() as u64) as usize] as char);
            v /= CHARS.len() as u64;
        }
        counter += 1;
    }
    out
}

// FNV-1a 64 of a file's content, to tell later whether a generated file has been edited (stable across Rust versions)
pub fn content_hash(content: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in content {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:016x}", h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts() {
        // 4 MB with OTA is the layout raft new has always made
        let l = partition_layout(4, true);
        assert_eq!((l.app_slot, l.fs_offset, l.fs_size), (0x1b0000, 0x380000, 0x80000));
        assert!(l.csv.contains("app1,     app,  ota_1,   0x1d0000,  0x1b0000,"));
        // 4 MB without OTA: one 3 MB app
        let l = partition_layout(4, false);
        assert_eq!((l.app_slot, l.fs_offset, l.fs_size), (0x300000, 0x320000, 0xe0000));
        assert!(l.csv.contains("factory") && !l.csv.contains("ota"));
        // 8 MB and more: two 3 MB slots, the rest file system
        let l = partition_layout(8, true);
        assert_eq!((l.app_slot, l.fs_offset, l.fs_size), (0x300000, 0x620000, 0x1e0000));
        let l = partition_layout(32, true);
        assert_eq!(l.fs_offset + l.fs_size, 32 * MB);
        for flash in [4, 8, 16, 32] {
            for ota in [true, false] {
                let l = partition_layout(flash, ota);
                assert!(l.fs_size >= MIN_FS && l.app_slot % APP_ALIGN == 0, "{} {}", flash, ota);
            }
        }
    }

    #[test]
    fn estimates_match_the_measured_builds() {
        let all = AppFeatures { chip: "esp32s3".into(), wifi: true, ble: true, webserver: true, i2c: true, device_types: "all".into(), runtime_decode: "attrs".into() };
        assert_eq!(estimate_app_bytes(&all), 1_672_976);
        let minimal = AppFeatures { ble: false, webserver: false, device_types: "none".into(), ..all.clone() };
        // measured 1,310,176 - the savings add up to within 1 KB
        assert!((estimate_app_bytes(&minimal) as i64 - 1_310_176).abs() < 1024);
        let nonet = AppFeatures { wifi: false, ..minimal.clone() };
        assert!((estimate_app_bytes(&nonet) as i64 - 581_296).abs() < 1024);
        let ble_only = AppFeatures { ble: true, ..nonet.clone() };
        assert!((estimate_app_bytes(&ble_only) as i64 - 870_400).abs() < 1024);
        // ESP32-C6, WiFi only, all device types: measured 1,659,536
        let c6 = AppFeatures { chip: "esp32c6".into(), ble: false, ..all.clone() };
        assert!((estimate_app_bytes(&c6) as i64 - 1_659_536).abs() < 20 * 1024);
    }

    #[test]
    fn passwords_and_hashes() {
        let a = random_password(12);
        assert_eq!(a.len(), 12);
        assert_ne!(a, random_password(12));
        assert_eq!(content_hash(b"abc"), content_hash(b"abc"));
        assert_ne!(content_hash(b"abc"), content_hash(b"abd"));
    }
}
