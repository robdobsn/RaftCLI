// RaftCLI: the hardware a new app is for - chip, flash, PSRAM and I2C pins - from a connected board (esptool), a dev
// board preset, an Espressif module's ordering code, or the chip with safe defaults (raft new, raft detect)
// Rob Dobson 2026

use dialoguer::Select;
use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;

use crate::app_ports::{select_most_likely_port, PortsCmd};
use crate::raft_cli_utils::{get_flash_tool_cmd, is_wsl};

pub const CHIPS: [&str; 6] = ["esp32", "esp32s3", "esp32c3", "esp32c5", "esp32c6", "esp32p4"];
pub const FLASH_SIZES: [u32; 4] = [4, 8, 16, 32];
pub const PSRAM_SIZES: [u32; 5] = [0, 2, 4, 8, 16];

// Safe defaults when the hardware isn't known: run on almost any board of the chip
pub const DEFAULT_CHIP: &str = "esp32s3";
pub const DEFAULT_FLASH_MB: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PsramMode {
    None,
    Quad,
    Octal,
}

impl PsramMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            PsramMode::None => "none",
            PsramMode::Quad => "quad",
            PsramMode::Octal => "octal",
        }
    }
    pub fn parse(text: &str) -> Option<PsramMode> {
        match text.trim().to_lowercase().as_str() {
            "none" | "" => Some(PsramMode::None),
            "quad" | "qspi" => Some(PsramMode::Quad),
            "octal" | "opi" | "oct" => Some(PsramMode::Octal),
            _ => None,
        }
    }
}

// What raft new needs to know about the hardware
#[derive(Debug, Clone)]
pub struct HardwareFacts {
    pub route: String,              // detect, board, module or chip
    pub description: String,        // e.g. "Unexpected Maker FeatherS3", "ESP32-S3-WROOM-1-N8R2"
    pub board: Option<String>,      // preset id
    pub module: Option<String>,     // module name (family and ordering code)
    pub chip: String,
    pub flash_mb: u32,
    pub psram_mb: u32,
    pub psram_mode: PsramMode,
    pub pins_known: bool,           // the I2C pins come from a board preset
    pub sda: u32,
    pub scl: u32,
    pub reserved_pins: Vec<u32>,    // pins the module uses itself (octal PSRAM, external PSRAM)
    pub assumed: Vec<String>,       // what was assumed rather than known
}

#[derive(Debug, Clone, Deserialize)]
pub struct BoardPreset {
    pub id: String,
    pub name: String,
    pub chip: String,
    pub flash: u32,
    pub psram: u32,
    #[serde(rename = "psramMode")]
    pub psram_mode: Option<String>,
    pub sda: u32,
    pub scl: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModuleFamily {
    #[allow(dead_code)]
    pub id: String,
    pub name: String,
    pub chip: String,
    pub codes: Vec<String>,
    #[serde(rename = "psramModes")]
    pub psram_modes: Option<HashMap<String, String>>,
    #[serde(rename = "reservedPins")]
    pub reserved_pins: Option<HashMap<String, Vec<u32>>>,
    pub sda: u32,
    pub scl: u32,
}

#[derive(Deserialize)]
struct BoardsFile {
    boards: Vec<BoardPreset>,
}

#[derive(Deserialize)]
struct ModulesFile {
    families: Vec<ModuleFamily>,
}

pub fn board_presets() -> Vec<BoardPreset> {
    serde_json::from_str::<BoardsFile>(include_str!("new_data/boards.json")).expect("new_data/boards.json").boards
}

pub fn module_families() -> Vec<ModuleFamily> {
    serde_json::from_str::<ModulesFile>(include_str!("new_data/modules.json")).expect("new_data/modules.json").families
}

pub fn chip_supports_psram(chip: &str) -> bool {
    matches!(chip, "esp32" | "esp32s3" | "esp32c5" | "esp32p4")
}

// I2C pins to suggest for a chip when no board says (the Arduino core's defaults where it has them)
pub fn chip_default_i2c_pins(chip: &str) -> (u32, u32) {
    match chip {
        "esp32" => (21, 22),
        "esp32s3" | "esp32c3" => (8, 9),
        "esp32c6" => (23, 22),
        "esp32p4" => (7, 8),
        _ => (5, 6),
    }
}

// The usual PSRAM mode for a size: on the ESP32-S3, 8 MB and more is octal (S3R8, S3R16V), less is quad (S3R2)
pub fn default_psram_mode(chip: &str, psram_mb: u32) -> PsramMode {
    if psram_mb == 0 {
        PsramMode::None
    } else if chip == "esp32s3" && psram_mb >= 8 {
        PsramMode::Octal
    } else {
        PsramMode::Quad
    }
}

/////////////////////////////////////////////////////////////////////////////////////////////////////////////////
// Facts for each route
/////////////////////////////////////////////////////////////////////////////////////////////////////////////////

pub fn facts_for_chip(chip: &str, flash_mb: u32, psram_mb: u32, psram_mode: Option<PsramMode>) -> HardwareFacts {
    let (sda, scl) = chip_default_i2c_pins(chip);
    HardwareFacts {
        route: "chip".to_string(),
        description: chip.to_uppercase().replacen("ESP32", "ESP32-", 1).trim_end_matches('-').to_string(),
        board: None,
        module: None,
        chip: chip.to_string(),
        flash_mb,
        psram_mb,
        psram_mode: psram_mode.unwrap_or_else(|| default_psram_mode(chip, psram_mb)),
        pins_known: false,
        sda,
        scl,
        reserved_pins: Vec::new(),
        assumed: Vec::new(),
    }
}

pub fn facts_for_board(board: &BoardPreset) -> HardwareFacts {
    HardwareFacts {
        route: "board".to_string(),
        description: board.name.clone(),
        board: Some(board.id.clone()),
        module: None,
        chip: board.chip.clone(),
        flash_mb: board.flash,
        psram_mb: board.psram,
        psram_mode: board.psram_mode.as_deref().and_then(PsramMode::parse)
            .unwrap_or_else(|| default_psram_mode(&board.chip, board.psram)),
        pins_known: true,
        sda: board.sda,
        scl: board.scl,
        reserved_pins: Vec::new(),
        assumed: Vec::new(),
    }
}

// An ordering code N<flash MB>[R<PSRAM MB>] (e.g. N8R2) -> (flash MB, PSRAM MB)
pub fn parse_ordering_code(code: &str) -> Option<(u32, u32)> {
    let re = Regex::new(r"^[Nn](\d+)(?:[Rr](\d+))?$").unwrap();
    let caps = re.captures(code.trim())?;
    let flash = caps[1].parse().ok()?;
    let psram = caps.get(2).map(|m| m.as_str().parse().unwrap_or(0)).unwrap_or(0);
    Some((flash, psram))
}

pub fn facts_for_module(family: &ModuleFamily, code: &str) -> Result<HardwareFacts, String> {
    let code = code.trim().to_uppercase();
    if !family.codes.iter().any(|c| c.eq_ignore_ascii_case(&code)) {
        return Err(format!("{} has no variant {} (variants: {})", family.name, code, family.codes.join(", ")));
    }
    let (flash_mb, psram_mb) = parse_ordering_code(&code).ok_or_else(|| format!("{} isn't an ordering code (N<flash>R<PSRAM>)", code))?;
    let psram_key = psram_mb.to_string();
    let psram_mode = family.psram_modes.as_ref().and_then(|m| m.get(&psram_key)).and_then(|m| PsramMode::parse(m))
        .unwrap_or_else(|| default_psram_mode(&family.chip, psram_mb));
    let reserved_pins = family.reserved_pins.as_ref().and_then(|m| m.get(&psram_key)).cloned().unwrap_or_default();
    let base_name = family.name.split(" / ").next().unwrap_or(&family.name).to_string();
    Ok(HardwareFacts {
        route: "module".to_string(),
        description: format!("{}-{}", base_name, code),
        board: None,
        module: Some(format!("{}-{}", base_name, code)),
        chip: family.chip.clone(),
        flash_mb,
        psram_mb,
        psram_mode,
        pins_known: false,
        sda: family.sda,
        scl: family.scl,
        reserved_pins,
        assumed: Vec::new(),
    })
}

// A module by its full name, e.g. ESP32-S3-WROOM-1-N8R2 or ESP32-S3-WROOM-1U-N16R8
pub fn find_module(name: &str) -> Result<(ModuleFamily, String), String> {
    let wanted = name.trim().to_uppercase().replace(' ', "");
    for family in module_families() {
        for variant in family.name.split(" / ") {
            // "ESP32-S3-WROOM-1 / -1U": the second variant is a suffix of the first's name
            let base = family.name.split(" / ").next().unwrap_or(&family.name);
            let prefix = if variant.starts_with('-') {
                let stem = base.rsplitn(2, '-').nth(1).unwrap_or(base);
                format!("{}{}", stem, variant)
            } else {
                variant.to_string()
            };
            let prefix = prefix.to_uppercase();
            if let Some(code) = wanted.strip_prefix(&format!("{}-", prefix)) {
                return Ok((family.clone(), code.to_string()));
            }
        }
    }
    Err(format!("\"{}\" isn't a module raft new knows - give the chip, flash and PSRAM instead (--set target_chip=..., \
                 flash_size=..., psram=...)", name))
}

/////////////////////////////////////////////////////////////////////////////////////////////////////////////////
// Detection with esptool
/////////////////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug, Clone, PartialEq, serde::Serialize, Deserialize)]
pub struct DetectedHardware {
    pub port: String,
    pub chip: String,               // raft new's name, e.g. esp32s3
    #[serde(rename = "chipDescription")]
    pub chip_description: String,   // as esptool reports it, e.g. "ESP32-S3 (QFN56) (revision v0.2)"
    #[serde(rename = "flashMb")]
    pub flash_mb: Option<u32>,
    #[serde(rename = "psramMb")]
    pub psram_mb: u32,              // embedded (in-package) PSRAM only - esptool can't see external PSRAM
    pub features: String,
}

// esptool's chip name -> raft new's (ESP32-S3 -> esp32s3; ESP32-D0WD-V3 and other classic ESP32 parts -> esp32)
pub fn chip_from_esptool_name(name: &str) -> String {
    let upper = name.trim().to_uppercase();
    let second = upper.strip_prefix("ESP32-").and_then(|rest| rest.split(|c: char| c == '-' || c == ' ').next()).unwrap_or("");
    match second {
        "S2" | "S3" | "C2" | "C3" | "C5" | "C6" | "C61" | "H2" | "P4" => format!("esp32{}", second.to_lowercase()),
        _ => "esp32".to_string(),
    }
}

// The facts in esptool's flash-id output (esptool v4 "Chip is ..." and v5 "Chip type: ..." forms)
pub fn parse_flash_id_output(port: &str, text: &str) -> Option<DetectedHardware> {
    let chip_re = Regex::new(r"(?m)(?:Chip type:\s+|Chip is\s+)(ESP32[^\r\n]*)").unwrap();
    let detecting_re = Regex::new(r"Detecting chip type\.*\s*(ESP32[-A-Za-z0-9]*)").unwrap();
    let chip_description = chip_re.captures(text).map(|c| c[1].trim().to_string())
        .or_else(|| detecting_re.captures(text).map(|c| c[1].trim().to_string()))?;
    let features = Regex::new(r"(?m)^Features:\s*(.*)$").unwrap().captures(text).map(|c| c[1].trim().to_string()).unwrap_or_default();
    let flash_mb = Regex::new(r"Detected flash size:\s*(\d+)MB").unwrap().captures(text).and_then(|c| c[1].parse().ok());
    let psram_mb = Regex::new(r"Embedded PSRAM (\d+)MB").unwrap().captures(text).and_then(|c| c[1].parse().ok()).unwrap_or(0);
    Some(DetectedHardware {
        port: port.to_string(),
        chip: chip_from_esptool_name(&chip_description),
        chip_description,
        flash_mb,
        psram_mb,
        features,
    })
}

// Run esptool flash-id on a port. Under WSL the USB serial ports are Windows ones: Windows raft.exe detect, or (with
// a raft.exe from before raft detect) the Windows esptool on the COM port
pub fn detect_hardware(port: Option<String>, native_serial_port: bool, flash_tool: Option<String>)
            -> Result<DetectedHardware, String> {
    if is_wsl() && !native_serial_port {
        let mut args = vec!["detect".to_string(), "--json".to_string(), "-n".to_string()];
        if let Some(p) = &port {
            args.push("-p".to_string());
            args.push(p.clone());
        }
        let output = std::process::Command::new("raft.exe").args(&args).output()
            .map_err(|e| format!("couldn't run Windows raft.exe ({}) - under WSL raft uses it for USB serial ports", e))?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        if let Some(json) = stdout.lines().rev().find(|l| l.trim_start().starts_with('{')) {
            let value: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("raft.exe detect: {}", e))?;
            if let Some(err) = value.get("error").and_then(|e| e.as_str()) {
                return Err(err.to_string());
            }
            return serde_json::from_value(value).map_err(|e| format!("raft.exe detect: {}", e));
        }
        // raft.exe without detect: run the Windows esptool here (the flash tool under WSL is the Windows one)
        return detect_with_esptool(port, false, flash_tool);
    }
    detect_with_esptool(port, native_serial_port, flash_tool)
}

fn detect_with_esptool(port: Option<String>, native_serial_port: bool, flash_tool: Option<String>)
            -> Result<DetectedHardware, String> {
    let port = match port {
        Some(p) => p,
        None => select_most_likely_port(&PortsCmd::new_with_vid(None), native_serial_port)
            .map(|p| p.port_name).ok_or("no serial port found - is the board plugged in?")?,
    };
    let tool = get_flash_tool_cmd(flash_tool, native_serial_port);
    let mut last = String::new();
    // esptool v4 has flash_id; v5 has flash-id (and still takes flash_id)
    for sub in ["flash_id", "flash-id"] {
        let (cmd, mut args) = if let Some(module) = tool.strip_prefix("python -m ") {
            ("python".to_string(), vec!["-m".to_string(), module.to_string()])
        } else {
            (tool.clone(), Vec::new())
        };
        args.extend(["--port".to_string(), port.clone(), sub.to_string()]);
        // Captured, not shown (esptool's progress isn't wanted in raft new or with --json)
        match std::process::Command::new(&cmd).args(&args).output() {
            Ok(output) => {
                let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
                if let Some(found) = parse_flash_id_output(&port, &text) {
                    return Ok(found);
                }
                last = text;
            }
            Err(e) => last = format!("couldn't run {}: {}", cmd, e),
        }
    }
    Err(format!("esptool couldn't read the chip on {} - hold BOOT while pressing RESET and try again\n{}", port,
                last.lines().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n")))
}

pub fn facts_from_detected(found: &DetectedHardware) -> HardwareFacts {
    let flash = found.flash_mb.unwrap_or(DEFAULT_FLASH_MB);
    let mut facts = facts_for_chip(&found.chip, flash, found.psram_mb, None);
    facts.route = "detect".to_string();
    facts.description = format!("{} on {}", found.chip_description, found.port);
    if found.flash_mb.is_none() {
        facts.assumed.push(format!("flash size {} MB (esptool didn't report it)", flash));
    }
    facts
}

/////////////////////////////////////////////////////////////////////////////////////////////////////////////////
// Choosing the hardware
/////////////////////////////////////////////////////////////////////////////////////////////////////////////////

// Settings that choose the hardware without prompting
pub const HARDWARE_ALL_KEYS: [&str; 7] = ["hardware", "board", "module", "target_chip", "flash_size", "psram", "psram_mode"];

fn parse_u32_choice(key: &str, value: &str, allowed: &[u32]) -> Result<u32, String> {
    let v: u32 = value.trim().trim_end_matches(|c: char| c.is_alphabetic()).parse()
        .map_err(|_| format!("{} must be a number of MB, not \"{}\"", key, value))?;
    if !allowed.contains(&v) {
        return Err(format!("{} must be one of {}", key, allowed.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(", ")));
    }
    Ok(v)
}

// The hardware from settings (--set, saved answers) - None if none of them is given
pub fn hardware_from_settings(settings: &HashMap<String, String>) -> Result<Option<HardwareFacts>, String> {
    if !HARDWARE_ALL_KEYS.iter().any(|k| settings.contains_key(*k)) {
        return Ok(None);
    }
    let route = settings.get("hardware").map(|s| s.to_lowercase());
    let mut facts = if let Some(id) = settings.get("board") {
        let board = board_presets().into_iter().find(|b| b.id.eq_ignore_ascii_case(id))
            .ok_or_else(|| format!("no board \"{}\" (boards: {})", id,
                                   board_presets().iter().map(|b| b.id.clone()).collect::<Vec<_>>().join(", ")))?;
        facts_for_board(&board)
    } else if let Some(name) = settings.get("module") {
        let (family, code) = find_module(name)?;
        facts_for_module(&family, &code)?
    } else if route.as_deref() == Some("detect") {
        let found = detect_hardware(settings.get("port").cloned(), false, None)?;
        facts_from_detected(&found)
    } else {
        let chip = settings.get("target_chip").map(|s| s.to_lowercase()).unwrap_or_else(|| DEFAULT_CHIP.to_string());
        if !CHIPS.contains(&chip.as_str()) {
            return Err(format!("target_chip must be one of {}", CHIPS.join(", ")));
        }
        let mut f = facts_for_chip(&chip, DEFAULT_FLASH_MB, 0, None);
        if !settings.contains_key("flash_size") {
            f.assumed.push(format!("flash size {} MB", DEFAULT_FLASH_MB));
        }
        f
    };
    // Explicit settings adjust any route (e.g. a board with a larger flash fitted)
    if let Some(chip) = settings.get("target_chip") {
        if chip.to_lowercase() != facts.chip && facts.route != "chip" {
            return Err(format!("target_chip {} doesn't match the {} ({})", chip, facts.description, facts.chip));
        }
    }
    if let Some(v) = settings.get("flash_size") {
        facts.flash_mb = parse_u32_choice("flash_size", v, &FLASH_SIZES)?;
    }
    if let Some(v) = settings.get("psram") {
        facts.psram_mb = parse_u32_choice("psram", v, &PSRAM_SIZES)?;
        if facts.psram_mb > 0 && !chip_supports_psram(&facts.chip) {
            return Err(format!("{} has no PSRAM support", facts.chip));
        }
        facts.psram_mode = default_psram_mode(&facts.chip, facts.psram_mb);
    }
    if let Some(v) = settings.get("psram_mode") {
        facts.psram_mode = PsramMode::parse(v).ok_or("psram_mode must be none, quad or octal")?;
    }
    if facts.psram_mb == 0 {
        facts.psram_mode = PsramMode::None;
    }
    Ok(Some(facts))
}

fn select(prompt: &str, items: &[String], default: usize) -> Result<usize, String> {
    Select::new().with_prompt(prompt).items(items).default(default).interact().map_err(|e| e.to_string())
}

fn ask_psram(chip: &str, prompt: &str) -> Result<(u32, PsramMode), String> {
    if !chip_supports_psram(chip) {
        return Ok((0, PsramMode::None));
    }
    let items: Vec<String> = PSRAM_SIZES.iter().map(|s| if *s == 0 { "None (or don't know)".to_string() } else { format!("{} MB", s) }).collect();
    let psram = PSRAM_SIZES[select(prompt, &items, 0)?];
    let mut mode = default_psram_mode(chip, psram);
    if chip == "esp32s3" && psram > 0 {
        let modes = vec!["Quad (QSPI) - e.g. ESP32-S3R2, most 2 MB parts".to_string(),
                         "Octal (OPI) - e.g. ESP32-S3R8, WROOM-1 R8 modules".to_string()];
        mode = if select("PSRAM mode", &modes, if mode == PsramMode::Octal { 1 } else { 0 })? == 1 { PsramMode::Octal } else { PsramMode::Quad };
    }
    Ok((psram, mode))
}

fn ask_chip_route() -> Result<HardwareFacts, String> {
    let mut items: Vec<String> = CHIPS.iter().map(|c| c.to_string()).collect();
    items.push(format!("I don't know ({} is assumed)", DEFAULT_CHIP));
    let idx = select("Chip", &items, CHIPS.iter().position(|c| *c == DEFAULT_CHIP).unwrap_or(0))?;
    let dont_know = idx == CHIPS.len();
    let chip = if dont_know { DEFAULT_CHIP } else { CHIPS[idx] };
    let flash_items: Vec<String> = FLASH_SIZES.iter().map(|s| if *s == DEFAULT_FLASH_MB { format!("{} MB (or don't know)", s) } else { format!("{} MB", s) }).collect();
    let flash = FLASH_SIZES[select("Flash size", &flash_items, 0)?];
    let (psram, mode) = ask_psram(chip, "PSRAM")?;
    let mut facts = facts_for_chip(chip, flash, psram, Some(mode));
    if dont_know {
        facts.assumed.push(format!("chip {}", chip));
    }
    Ok(facts)
}

fn ask_board_route() -> Result<Option<HardwareFacts>, String> {
    let boards = board_presets();
    let mut items: Vec<String> = boards.iter().map(|b| b.name.clone()).collect();
    items.push("Not listed".to_string());
    let idx = select("Board", &items, 0)?;
    Ok(boards.get(idx).map(facts_for_board))
}

fn ask_module_route() -> Result<HardwareFacts, String> {
    let families = module_families();
    let items: Vec<String> = families.iter().map(|f| f.name.clone()).collect();
    let family = &families[select("Module family", &items, 0)?];
    let code = &family.codes[select("Variant (N<flash MB>R<PSRAM MB>)", &family.codes, 0)?];
    facts_for_module(family, code)
}

// The hardware, from settings or by asking (use_defaults: the chip route's safe defaults without asking)
pub fn choose_hardware(use_defaults: bool, settings: &HashMap<String, String>) -> Result<HardwareFacts, String> {
    if let Some(facts) = hardware_from_settings(settings)? {
        return Ok(facts);
    }
    if use_defaults {
        let mut facts = facts_for_chip(DEFAULT_CHIP, DEFAULT_FLASH_MB, 0, None);
        facts.assumed.push(format!("chip {}, {} MB flash, no PSRAM (the defaults)", DEFAULT_CHIP, DEFAULT_FLASH_MB));
        return Ok(facts);
    }
    // A board to detect from (under WSL only if Windows raft.exe is there to reach the USB serial ports - finding a port
    // must never stop raft new)
    let mut port = if is_wsl() && which::which("raft.exe").is_err() {
        None
    } else {
        std::panic::catch_unwind(|| select_most_likely_port(&PortsCmd::new_with_vid(None), false)).ok().flatten().map(|p| p.port_name)
    };
    loop {
        let mut routes: Vec<(&str, String)> = Vec::new();
        if let Some(p) = &port {
            routes.push(("detect", format!("Detect it from the board on {}", p)));
        }
        routes.push(("board", "A dev board (choose from a list)".to_string()));
        routes.push(("module", "An Espressif module (e.g. ESP32-S3-WROOM-1-N8R2)".to_string()));
        routes.push(("chip", "A chip, or I don't know".to_string()));
        let items: Vec<String> = routes.iter().map(|r| r.1.clone()).collect();
        let route = routes[select("Hardware", &items, 0)?].0;
        let facts = match route {
            "detect" => match detect_hardware(port.clone(), false, None) {
                Ok(found) => {
                    println!("Found {}{}{}", found.chip_description,
                             found.flash_mb.map(|f| format!(", {} MB flash", f)).unwrap_or_default(),
                             if found.psram_mb > 0 { format!(", {} MB PSRAM", found.psram_mb) } else { String::new() });
                    let mut facts = facts_from_detected(&found);
                    if found.psram_mb == 0 && chip_supports_psram(&found.chip) {
                        // esptool sees only PSRAM in the chip's package - a module or board may add its own
                        let (psram, mode) = ask_psram(&found.chip, "PSRAM (none found in the chip - a module or board may add it)")?;
                        facts.psram_mb = psram;
                        facts.psram_mode = mode;
                    }
                    facts
                }
                Err(e) => {
                    // Not offered again - choose another way
                    println!("Couldn't detect the hardware: {}", e);
                    port = None;
                    continue;
                }
            },
            "board" => match ask_board_route()? {
                Some(f) => f,
                None => continue,
            },
            "module" => {
                let facts = ask_module_route()?;
                if !facts.reserved_pins.is_empty() {
                    println!("Note: the {} uses GPIO {} itself - don't use them for I2C", facts.description,
                             facts.reserved_pins.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", "));
                }
                facts
            }
            _ => ask_chip_route()?,
        };
        return Ok(facts);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordering_codes() {
        assert_eq!(parse_ordering_code("N8R2"), Some((8, 2)));
        assert_eq!(parse_ordering_code("n16"), Some((16, 0)));
        assert_eq!(parse_ordering_code("N4R8"), Some((4, 8)));
        assert_eq!(parse_ordering_code("R8"), None);
        assert_eq!(parse_ordering_code("N8X"), None);
    }

    #[test]
    fn modules_by_name() {
        let (family, code) = find_module("ESP32-S3-WROOM-1-N8R2").unwrap();
        assert_eq!((family.id.as_str(), code.as_str()), ("esp32-s3-wroom-1", "N8R2"));
        let facts = facts_for_module(&family, &code).unwrap();
        assert_eq!((facts.chip.as_str(), facts.flash_mb, facts.psram_mb, facts.psram_mode), ("esp32s3", 8, 2, PsramMode::Quad));
        assert!(facts.reserved_pins.is_empty());
        // R8 is octal, and its PSRAM pins are reserved; the U (external antenna) variant is the same module
        let (family, code) = find_module("esp32-s3-wroom-1u-n16r8").unwrap();
        let facts = facts_for_module(&family, &code).unwrap();
        assert_eq!((facts.flash_mb, facts.psram_mb, facts.psram_mode), (16, 8, PsramMode::Octal));
        assert!(facts.reserved_pins.contains(&35));
        assert_eq!(facts.description, "ESP32-S3-WROOM-1-N16R8");
        // External PSRAM on the WROVER
        let (family, code) = find_module("ESP32-WROVER-E-N8R8").unwrap();
        let facts = facts_for_module(&family, &code).unwrap();
        assert_eq!((facts.chip.as_str(), facts.psram_mb, facts.reserved_pins.clone()), ("esp32", 8, vec![16, 17]));
        // Unknown variants and modules
        assert!(facts_for_module(&find_module("ESP32-S3-WROOM-1-N8").unwrap().0, "N32").is_err());
        assert!(find_module("ESP8266-12F").is_err());
    }

    #[test]
    fn boards_and_modules_data_is_consistent() {
        for b in board_presets() {
            assert!(CHIPS.contains(&b.chip.as_str()), "{}", b.id);
            assert!(FLASH_SIZES.contains(&b.flash), "{}", b.id);
            assert!(b.psram == 0 || chip_supports_psram(&b.chip), "{}", b.id);
        }
        for f in module_families() {
            assert!(CHIPS.contains(&f.chip.as_str()), "{}", f.id);
            for code in &f.codes {
                let (flash, psram) = parse_ordering_code(code).unwrap();
                assert!(FLASH_SIZES.contains(&flash), "{} {}", f.id, code);
                assert!(psram == 0 || chip_supports_psram(&f.chip), "{} {}", f.id, code);
            }
            assert!(find_module(&format!("{}-{}", f.name.split(" / ").next().unwrap(), f.codes[0])).is_ok(), "{}", f.id);
        }
    }

    #[test]
    fn esptool_output_v4_and_v5() {
        let v4 = "esptool.py v4.7.0\nSerial port /dev/ttyACM0\nConnecting...\nDetecting chip type... ESP32-S3\nChip is ESP32-S3 (QFN56) (revision v0.2)\n\
                  Features: WiFi, BLE, Embedded PSRAM 8MB (AP_3v3)\nCrystal is 40MHz\nMAC: f4:12:fa:00:00:00\nManufacturer: 20\nDevice: 4018\n\
                  Detected flash size: 16MB\nHard resetting via RTS pin...\n";
        let found = parse_flash_id_output("/dev/ttyACM0", v4).unwrap();
        assert_eq!((found.chip.as_str(), found.flash_mb, found.psram_mb), ("esp32s3", Some(16), 8));
        assert_eq!(found.chip_description, "ESP32-S3 (QFN56) (revision v0.2)");
        let facts = facts_from_detected(&found);
        assert_eq!(facts.psram_mode, PsramMode::Octal);
        let v5 = "esptool v5.0.2\nConnected to ESP32-C6 on COM5:\nChip type:          ESP32-C6 (QFN40) (revision v0.1)\n\
                  Features:           Wi-Fi 6, BT 5 (LE), IEEE802.15.4, Single Core + LP Core, 160MHz\nCrystal frequency:  40MHz\n\n\
                  Flash Memory Information:\n=========================\nManufacturer: 0x20\nDevice: 0x4016\nDetected flash size: 4MB\n";
        let found = parse_flash_id_output("COM5", v5).unwrap();
        assert_eq!((found.chip.as_str(), found.flash_mb, found.psram_mb), ("esp32c6", Some(4), 0));
        let classic = "Detecting chip type... Unsupported detection protocol, switching and trying again...\nDetecting chip type... ESP32\n\
                       Chip is ESP32-D0WD-V3 (revision v3.1)\nFeatures: WiFi, BT, Dual Core, 240MHz\nDetected flash size: 4MB\n";
        assert_eq!(parse_flash_id_output("p", classic).unwrap().chip, "esp32");
        assert!(parse_flash_id_output("p", "A fatal error occurred: Failed to connect").is_none());
    }

    #[test]
    fn hardware_from_settings_routes() {
        let s = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<HashMap<_, _>>();
        assert!(hardware_from_settings(&s(&[])).unwrap().is_none());
        let f = hardware_from_settings(&s(&[("board", "um-feathers3")])).unwrap().unwrap();
        assert_eq!((f.chip.as_str(), f.flash_mb, f.psram_mb, f.pins_known, f.sda, f.scl), ("esp32s3", 16, 8, true, 8, 9));
        let f = hardware_from_settings(&s(&[("module", "ESP32-S3-WROOM-1-N8R8")])).unwrap().unwrap();
        assert_eq!((f.flash_mb, f.psram_mb, f.psram_mode, f.pins_known), (8, 8, PsramMode::Octal, false));
        let f = hardware_from_settings(&s(&[("target_chip", "esp32c6")])).unwrap().unwrap();
        assert_eq!((f.chip.as_str(), f.flash_mb, f.psram_mb, f.sda, f.scl), ("esp32c6", 4, 0, 23, 22));
        assert!(!f.assumed.is_empty());
        let f = hardware_from_settings(&s(&[("flash_size", "16"), ("psram", "2")])).unwrap().unwrap();
        assert_eq!((f.chip.as_str(), f.flash_mb, f.psram_mb, f.psram_mode), ("esp32s3", 16, 2, PsramMode::Quad));
        assert!(hardware_from_settings(&s(&[("target_chip", "esp32c3"), ("psram", "2")])).is_err());
        assert!(hardware_from_settings(&s(&[("board", "nope")])).is_err());
        assert!(hardware_from_settings(&s(&[("board", "xiao-esp32c3"), ("target_chip", "esp32s3")])).is_err());
        assert!(hardware_from_settings(&s(&[("flash_size", "3")])).is_err());
    }
}
