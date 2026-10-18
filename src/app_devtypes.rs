// RaftCLI: Device type record commands (`raft devtypes ...`)
// Manage a project's DeviceTypeSources.json (the device type records compiled into the firmware and those for a
// device type file) and put device type files on a running device.
// The selection logic is RaftCore's (scripts/DeviceTypesTool.py in the RaftCore the project builds with), so it
// follows the rules of the firmware being built; push, pull and status talk to the device's devman/typefile API.
// Rob Dobson 2026

use clap::{Args, Subcommand};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::app_fs::{confirm, crc16_ccitt, delete_remote, download_file, upload_file, DeviceCtx};
use crate::http_client::{http_get, http_get_timeout};
use crate::raft_cli_utils::{find_executable, utils_get_sys_type};

// ---------------------------------------------------------------------------
// CLI definitions
// ---------------------------------------------------------------------------

#[derive(Clone, Args, Debug)]
pub struct DevTypesCmd {
    #[clap(subcommand)]
    pub command: DevTypesSub,
    /// System type whose DeviceTypeSources.json to use (default: last built / first found)
    #[clap(short = 's', long = "sys-type", value_name = "SYSTYPE", global = true)]
    pub sys_type: Option<String>,
    /// Path to the application folder (default: current directory)
    #[clap(long = "app-folder", value_name = "APPLICATION_FOLDER", global = true)]
    pub app_folder: Option<String>,
    /// Use this DeviceTypeSources.json instead of a project's SysType files (e.g. for a device type file only)
    #[clap(long, value_name = "FILE", global = true)]
    pub config: Option<String>,
    /// RaftCore folder (default: raftdevlibs/RaftCore, then the build's copy)
    #[clap(long, value_name = "DIR", global = true)]
    pub raftcore: Option<String>,
    /// Change systypes/Common/DeviceTypeSources.json rather than the SysType's (init, add, remove)
    #[clap(long, global = true)]
    pub common: bool,
}

#[derive(Clone, Subcommand, Debug)]
pub enum DevTypesSub {
    /// The records selected for the firmware and for the type file, with their sources
    #[clap(alias = "ls")]
    List {
        /// Machine-readable output
        #[clap(long)]
        json: bool,
    },
    /// Create a DeviceTypeSources.json selecting every RaftCore record
    Init,
    /// Add records to the selection, e.g. raftcore:LSM6DS or local:*
    Add {
        #[clap(required = true, value_name = "PATTERN")]
        patterns: Vec<String>,
        /// Add to the type file selection (typeFile) rather than the firmware's (builtin)
        #[clap(long)]
        typefile: bool,
    },
    /// Remove records from the selection (one selected by a wildcard is excluded)
    #[clap(alias = "rm")]
    Remove {
        #[clap(required = true, value_name = "PATTERN")]
        patterns: Vec<String>,
        /// Remove from the type file selection (typeFile) rather than the firmware's (builtin)
        #[clap(long)]
        typefile: bool,
    },
    /// Create a new record file in a path source, ready to fill in
    New {
        /// Device type key (the record's file name)
        key: String,
        /// The path source to put it in (needed if there is more than one)
        #[clap(long)]
        source: Option<String>,
        /// I2C address(es), e.g. 0x44
        #[clap(long, default_value = "0x00")]
        address: String,
    },
    /// Check the selected records (grammar, decode code, generator checks) without a build
    Check {
        /// Machine-readable output
        #[clap(long)]
        json: bool,
    },
    /// Resolve git sources' refs again and update the lock file
    Update,
    /// Show local copies against the records they replace, and type file overrides against compiled records
    Diff {
        /// Only this device type key
        key: Option<String>,
    },
    /// Build the device type file (devlib.json) from the typeFile selection
    Typefile {
        /// Output file (default: build/<SysType>/raft/devlib.json)
        #[clap(long)]
        out: Option<String>,
        /// Catalogue file of the records compiled into the firmware (default: the builtin selection)
        #[clap(long)]
        compiled: Option<String>,
        /// Size limit in bytes (default 65536)
        #[clap(long)]
        max_bytes: Option<u64>,
        /// Machine-readable output
        #[clap(long)]
        json: bool,
    },
    /// Build the device type file, check it on the device and apply it (the device restarts)
    Push {
        #[clap(flatten)]
        device: DeviceArgs,
        /// Catalogue file of the records compiled into the firmware (default: the builtin selection)
        #[clap(long)]
        compiled: Option<String>,
        /// Apply without asking
        #[clap(short = 'y', long)]
        yes: bool,
        /// Upload and check on the device, but don't apply (the uploaded file is removed)
        #[clap(long)]
        dry_run: bool,
        /// Push even if the device already has this file
        #[clap(long)]
        force: bool,
    },
    /// Download the device's type file and compare it with the project's
    Pull {
        #[clap(flatten)]
        device: DeviceArgs,
        /// Where to save it (default: build/<SysType>/raft/devlib.device.json)
        #[clap(long)]
        out: Option<String>,
    },
    /// The device's type file: what was applied at startup, and whether it is the project's
    Status {
        #[clap(flatten)]
        device: DeviceArgs,
        /// Machine-readable output (the device's reply)
        #[clap(long)]
        json: bool,
    },
}

#[derive(Clone, Args, Debug)]
pub struct DeviceArgs {
    /// Device IP address or hostname
    #[clap(value_name = "IP_ADDRESS_OR_HOSTNAME")]
    pub ip_addr: String,
    /// HTTP port (default 80)
    #[clap(short = 'p', long)]
    pub port: Option<u16>,
}

// ---------------------------------------------------------------------------
// Project context: the selection files, RaftCore and where things are kept
// ---------------------------------------------------------------------------

type CmdResult<T> = Result<T, Box<dyn std::error::Error>>;

struct Project {
    raftcore: PathBuf,
    python: String,
    // DeviceTypeSources.json files that exist, Common's first
    select: Vec<PathBuf>,
    // The file init, add and remove change
    edit: PathBuf,
    lock: PathBuf,
    cache_dir: PathBuf,
    override_dir: Option<PathBuf>,
    // Where built type files and the record of pushes are kept
    work_dir: PathBuf,
}

const SOURCES_FILE: &str = "DeviceTypeSources.json";
const TOOL_SCRIPT: &str = "scripts/DeviceTypesTool.py";

fn find_raftcore(explicit: &Option<String>, app_folder: &Path, sys_type: Option<&str>) -> CmdResult<PathBuf> {
    if let Some(dir) = explicit {
        let dir = PathBuf::from(dir);
        if !dir.join(TOOL_SCRIPT).exists() {
            return Err(format!("{} has no {} - it needs a RaftCore with device type tools", dir.display(), TOOL_SCRIPT).into());
        }
        return Ok(dir);
    }
    let mut candidates = vec![app_folder.join("raftdevlibs").join("RaftCore")];
    if let Some(sys_type) = sys_type {
        candidates.push(app_folder.join("build").join(sys_type).join("_deps").join("raftcore-src"));
    }
    for candidate in &candidates {
        if candidate.join(TOOL_SCRIPT).exists() {
            return Ok(candidate.clone());
        }
    }
    let found: Vec<String> = candidates.iter().filter(|c| c.exists()).map(|c| c.display().to_string()).collect();
    if !found.is_empty() {
        return Err(format!(
            "the RaftCore at {} has no device type tools ({}) - this needs a newer RaftCore",
            found.join(", "),
            TOOL_SCRIPT
        )
        .into());
    }
    Err("no RaftCore found - build the project once, fetch it with 'raft libs fetch', or give --raftcore".into())
}

fn resolve_project(cmd: &DevTypesCmd) -> CmdResult<Project> {
    let python = find_executable(&["python3", "python"]).ok_or("Python 3 is needed (python3 or python not found)")?;
    let app_folder = PathBuf::from(cmd.app_folder.clone().unwrap_or_else(|| ".".to_string()));

    // A DeviceTypeSources.json on its own (no project)
    if let Some(config) = &cmd.config {
        let config = PathBuf::from(config);
        let base = config.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
        let base = if base.as_os_str().is_empty() { PathBuf::from(".") } else { base };
        let work_dir = base.join(".raftdevtypes");
        let raftcore = find_raftcore(&cmd.raftcore, &app_folder, None)?;
        return Ok(Project {
            raftcore,
            python,
            select: if config.exists() { vec![config.clone()] } else { vec![] },
            edit: config.clone(),
            lock: config.with_extension("lock"),
            cache_dir: work_dir.join("devtypes"),
            override_dir: Some(base.join("raftdevtypes")),
            work_dir,
        });
    }

    let sys_type = utils_get_sys_type(&cmd.sys_type, app_folder.to_string_lossy().to_string())?;
    if sys_type.is_empty() {
        return Err("no SysType found in systypes/ - give one with -s or use --config".into());
    }
    let systypes = app_folder.join("systypes");
    if !systypes.join(&sys_type).is_dir() {
        return Err(format!("SysType folder {} not found", systypes.join(&sys_type).display()).into());
    }
    let common_file = systypes.join("Common").join(SOURCES_FILE);
    let systype_file = systypes.join(&sys_type).join(SOURCES_FILE);
    let select: Vec<PathBuf> = [&common_file, &systype_file].iter().filter(|p| p.exists()).map(|p| p.to_path_buf()).collect();
    let edit = if cmd.common {
        common_file.clone()
    } else if systype_file.exists() || !common_file.exists() {
        systype_file.clone()
    } else {
        common_file.clone()
    };
    let work_dir = app_folder.join("build").join(&sys_type).join("raft");
    let raftcore = find_raftcore(&cmd.raftcore, &app_folder, Some(&sys_type))?;
    Ok(Project {
        raftcore,
        python,
        select,
        edit,
        lock: systypes.join(&sys_type).join("DeviceTypeSources.lock"),
        cache_dir: work_dir.join("devtypes"),
        override_dir: Some(app_folder.join("raftdevtypes")),
        work_dir,
    })
}

impl Project {
    fn tool_command(&self, json: bool, args: &[String]) -> Command {
        let mut command = Command::new(&self.python);
        command.arg(self.raftcore.join(TOOL_SCRIPT));
        command.arg("--raftcore").arg(&self.raftcore);
        let select: Vec<String> = self.select.iter().map(|p| p.to_string_lossy().to_string()).collect();
        command.arg("--select").arg(select.join(","));
        command.arg("--edit").arg(&self.edit);
        command.arg("--lock").arg(&self.lock);
        command.arg("--cache-dir").arg(&self.cache_dir);
        if let Some(dir) = &self.override_dir {
            command.arg("--override-dir").arg(dir);
        }
        if json {
            command.arg("--json");
        }
        command.args(args);
        command
    }

    // Run the tool with its output shown; its exit code becomes the result
    fn run_tool(&self, json: bool, args: &[String]) -> CmdResult<()> {
        let status = self.tool_command(json, args).status()?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("devtypes {} failed", args.first().map(|s| s.as_str()).unwrap_or("")).into())
        }
    }

    // Run the tool for its JSON output
    fn run_tool_json(&self, args: &[String]) -> CmdResult<serde_json::Value> {
        let output = self.tool_command(true, args).output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            return Err(format!("{}{}", stdout, stderr).trim().to_string().into());
        }
        Ok(serde_json::from_slice(&output.stdout)?)
    }

    fn has_selection(&self) -> bool {
        !self.select.is_empty()
    }

    // Build the type file; returns the tool's report (file, bytes, crc, entries)
    fn build_type_file(&self, out: &Path, compiled: &Option<String>, max_bytes: Option<u64>) -> CmdResult<serde_json::Value> {
        let mut args = vec!["typefile".to_string(), "--out".to_string(), out.to_string_lossy().to_string()];
        if let Some(compiled) = compiled {
            args.push("--compiled".to_string());
            args.push(compiled.clone());
        }
        if let Some(max_bytes) = max_bytes {
            args.push("--max-bytes".to_string());
            args.push(max_bytes.to_string());
        }
        self.run_tool_json(&args)
    }
}

// ---------------------------------------------------------------------------
// The device's devman/typefile API
// ---------------------------------------------------------------------------

fn device_api(ctx: &DeviceCtx, path: &str) -> CmdResult<serde_json::Value> {
    let resp = http_get(&ctx.ip_addr, ctx.port, &format!("/api/{}", path))?;
    let body = resp.body_string();
    serde_json::from_str(&body).map_err(|e| format!("invalid reply from the device to {}: {} ({})", path, e, body).into())
}

fn device_status(ctx: &DeviceCtx, entries: bool) -> CmdResult<serde_json::Value> {
    let status = device_api(ctx, if entries { "devman/typefile?entries=1" } else { "devman/typefile" })?;
    if status["rslt"].as_str() != Some("ok") {
        let error = status["error"].as_str().unwrap_or(status["rslt"].as_str().unwrap_or("?"));
        if error == "failUnknownCmd" || error == "failUnknownAPI" || status["rslt"].is_null() {
            return Err("the device doesn't support device type files (its firmware has no devman/typefile)".into());
        }
        return Err(format!("devman/typefile: {}", error).into());
    }
    Ok(status)
}

// CRC of the device's type file as it is now (None if there isn't one)
fn device_file_crc(ctx: &DeviceCtx) -> Option<String> {
    let check = device_api(ctx, "devman/typefile/check").ok()?;
    if check["rslt"].as_str() != Some("ok") {
        return None;
    }
    check["crc"].as_str().map(|s| s.to_string())
}

// The device's type file path split into its file system and name, e.g. /local/devlib.json -> (local, devlib.json)
fn split_device_path(path: &str) -> CmdResult<(String, String)> {
    let trimmed = path.trim_start_matches('/');
    match trimmed.split_once('/') {
        Some((fs, name)) if !fs.is_empty() && !name.is_empty() => Ok((fs.to_string(), name.to_string())),
        _ => Err(format!("unexpected type file path {} from the device", path).into()),
    }
}

fn staged_name(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) => format!("{}.new.{}", stem, ext),
        None => format!("{}.new", name),
    }
}

fn str_of<'a>(value: &'a serde_json::Value, name: &str) -> &'a str {
    value[name].as_str().unwrap_or("")
}

fn num_of(value: &serde_json::Value, name: &str) -> i64 {
    value[name].as_i64().unwrap_or(0)
}

fn print_counts(result: &serde_json::Value) {
    println!(
        "  added {}, replaced {}, unchanged {}, disabled {}, rejected {}, warnings {}",
        num_of(result, "added"),
        num_of(result, "replaced"),
        num_of(result, "unchanged"),
        num_of(result, "disabled"),
        num_of(result, "rejected"),
        num_of(result, "warnings")
    );
    for (label, name) in [("first error", "firstError"), ("first warning", "firstWarning")] {
        if !str_of(result, name).is_empty() {
            println!("  {}: {}", label, str_of(result, name));
        }
    }
}

fn print_entries(result: &serde_json::Value) {
    if let Some(entries) = result["entries"].as_array() {
        for entry in entries {
            let mut line = format!("  {:28} {}", str_of(entry, "key"), str_of(entry, "outcome"));
            if entry["noDecoder"].as_bool().unwrap_or(false) {
                line.push_str(" - no on-device decoder");
            }
            if !str_of(entry, "error").is_empty() {
                line.push_str(&format!(" - {}", str_of(entry, "error")));
            }
            if !str_of(entry, "warnings").is_empty() {
                line.push_str(&format!(" (warning: {})", str_of(entry, "warnings")));
            }
            println!("{}", line);
        }
    }
}

// The CRC of the last type file pushed to each device from this project (to notice a file changed by others)
fn pushed_record_path(project: &Project) -> PathBuf {
    project.work_dir.join("devtypes-pushed.json")
}

fn read_pushed(project: &Project) -> BTreeMap<String, String> {
    std::fs::read_to_string(pushed_record_path(project))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_pushed(project: &Project, device: &str, crc: &str) -> CmdResult<()> {
    let mut pushed = read_pushed(project);
    pushed.insert(device.to_string(), crc.to_string());
    std::fs::create_dir_all(&project.work_dir)?;
    std::fs::write(pushed_record_path(project), serde_json::to_string_pretty(&pushed)? + "\n")?;
    Ok(())
}

fn device_ctx(device: &DeviceArgs, fs: &str) -> DeviceCtx {
    DeviceCtx { ip_addr: device.ip_addr.clone(), port: device.port.unwrap_or(80), fs: fs.to_string() }
}

// Wait for the device to answer again after a restart
fn wait_for_device(ctx: &DeviceCtx, timeout: Duration) -> bool {
    let start = Instant::now();
    // Give it time to go down first
    std::thread::sleep(Duration::from_secs(2));
    while start.elapsed() < timeout {
        if let Ok(resp) = http_get_timeout(&ctx.ip_addr, ctx.port, "/api/v", Duration::from_secs(2)) {
            if resp.is_success() {
                return true;
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    false
}

// ---------------------------------------------------------------------------
// Device commands
// ---------------------------------------------------------------------------

fn push(project: &Project, device: &DeviceArgs, compiled: &Option<String>, yes: bool, dry_run: bool, force: bool) -> CmdResult<()> {
    if !project.has_selection() {
        return Err(format!("no {} - create one with 'raft devtypes init'", SOURCES_FILE).into());
    }
    let ctx = device_ctx(device, "local");
    let status = device_status(&ctx, false)?;
    let device_file = str_of(&status, "file").to_string();
    if device_file.is_empty() {
        return Err("the device has no type file set (DevMan typeFile setting)".into());
    }
    let (fs, name) = split_device_path(&device_file)?;
    let ctx = device_ctx(device, &fs);
    let max_bytes = status["maxFileBytes"].as_u64().filter(|&n| n > 0);

    // Build it within the device's limit
    let out = project.work_dir.join("devlib.json");
    let report = project.build_type_file(&out, compiled, max_bytes)?;
    let crc = str_of(&report, "crc").to_string();
    println!("Type file {} ({} bytes, CRC {})", out.display(), num_of(&report, "bytes"), crc);

    // The status CRC is of the file applied at startup; the file there now (changed since, if a restart is due) is
    // what a check of the device's own file reports
    let applied_crc = str_of(&status, "crc").to_string();
    let current_crc = device_file_crc(&ctx);
    if current_crc.as_deref() == Some(crc.as_str()) && applied_crc == crc && str_of(&status, "state") == "applied" && !force {
        println!("The device already has this type file (CRC {}) - nothing to do (--force to push anyway)", crc);
        return Ok(());
    }
    if let (Some(pushed_crc), Some(current_crc)) = (read_pushed(project).get(&device.ip_addr), &current_crc) {
        if pushed_crc != current_crc {
            println!(
                "Warning: the device's type file (CRC {}) isn't the one last pushed from here (CRC {}) - it has been changed since",
                current_crc, pushed_crc
            );
            if !yes && !confirm("Replace it?")? {
                println!("Aborted.");
                return Ok(());
            }
        }
    }

    // Upload under another name and check it on the device: the device's own rules decide
    let staged = staged_name(&name);
    let staged_path = format!("/{}/{}", fs, staged);
    upload_file(&ctx, &out.to_string_lossy(), &staged, true)?;
    let check = device_api(&ctx, &format!("devman/typefile/check?file={}", staged_path))?;
    if check["rslt"].as_str() != Some("ok") {
        let _ = delete_remote(&ctx, &staged);
        return Err(format!("check on the device failed: {}", str_of(&check, "error")).into());
    }
    println!("Device check of {}:", staged_path);
    print_counts(&check);
    print_entries(&check);
    if str_of(&check, "crc") != crc {
        let _ = delete_remote(&ctx, &staged);
        return Err(format!("the uploaded file's CRC on the device is {}, not {} - upload again", str_of(&check, "crc"), crc).into());
    }
    if !check["valid"].as_bool().unwrap_or(false) {
        let _ = delete_remote(&ctx, &staged);
        return Err("the device rejects this type file - nothing was changed".into());
    }
    if dry_run {
        delete_remote(&ctx, &staged)?;
        println!("Dry run: not applied ({} removed)", staged_path);
        return Ok(());
    }
    if !yes && !confirm(&format!("Apply it as {} and restart {}?", device_file, device.ip_addr))? {
        let _ = delete_remote(&ctx, &staged);
        println!("Aborted ({} removed).", staged_path);
        return Ok(());
    }

    // Apply: the device checks it again, moves it into place and restarts
    let apply = device_api(&ctx, &format!("devman/typefile/apply?file={}", staged_path))?;
    if apply["rslt"].as_str() != Some("ok") {
        return Err(format!("apply failed: {}", str_of(&apply, "error")).into());
    }
    println!("Applied - waiting for {} to restart...", device.ip_addr);
    if !wait_for_device(&ctx, Duration::from_secs(60)) {
        return Err(format!("{} didn't answer within 60s of restarting - check it with 'raft devtypes status'", device.ip_addr).into());
    }

    // Check what the device loaded
    let after = device_status(&ctx, true)?;
    let after_crc = str_of(&after, "crc");
    let state = str_of(&after, "state");
    println!("{}: type file {} {} (CRC {})", device.ip_addr, str_of(&after, "file"), state, after_crc);
    print_counts(&after);
    print_entries(&after);
    if after_crc != crc {
        return Err(format!("the device loaded a type file with CRC {}, not {}", after_crc, crc).into());
    }
    write_pushed(project, &device.ip_addr, &crc)?;
    if state != "applied" || num_of(&after, "rejected") > 0 {
        return Err(format!("the device's type file state is {}, with {} rejected", state, num_of(&after, "rejected")).into());
    }
    println!("Done");
    Ok(())
}

fn dev_types_of(text: &str) -> CmdResult<serde_json::Map<String, serde_json::Value>> {
    let value: serde_json::Value = serde_json::from_str(text)?;
    Ok(value["devTypes"].as_object().cloned().unwrap_or_default())
}

fn pull(project: &Project, device: &DeviceArgs, out: &Option<String>) -> CmdResult<()> {
    let ctx = device_ctx(device, "local");
    let status = device_status(&ctx, false)?;
    let device_file = str_of(&status, "file").to_string();
    if device_file.is_empty() {
        return Err("the device has no type file set (DevMan typeFile setting)".into());
    }
    if !status["present"].as_bool().unwrap_or(false) {
        println!("The device has no type file ({} isn't there)", device_file);
        return Ok(());
    }
    let (fs, name) = split_device_path(&device_file)?;
    let data = download_file(&device_ctx(device, &fs), &name)?;
    let dest = out.as_ref().map(PathBuf::from).unwrap_or_else(|| project.work_dir.join("devlib.device.json"));
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&dest, &data)?;
    let crc = format!("{:04x}", crc16_ccitt(&data));
    println!("Downloaded {} ({} bytes, CRC {}) -> {}", device_file, data.len(), crc, dest.display());

    if !project.has_selection() {
        return Ok(());
    }
    let local_path = project.work_dir.join("devlib.json");
    let report = project.build_type_file(&local_path, &None, None)?;
    if str_of(&report, "crc") == crc {
        println!("It is the project's type file");
        return Ok(());
    }
    println!("It differs from the project's type file (CRC {}):", str_of(&report, "crc"));
    let device_types = dev_types_of(&String::from_utf8_lossy(&data))?;
    let local_types = dev_types_of(&std::fs::read_to_string(&local_path)?)?;
    let mut keys: Vec<&String> = device_types.keys().chain(local_types.keys()).collect();
    keys.sort();
    keys.dedup();
    for key in keys {
        match (device_types.get(key), local_types.get(key)) {
            (Some(_), None) => println!("  {:28} only on the device", key),
            (None, Some(_)) => println!("  {:28} only in the project", key),
            (Some(a), Some(b)) if a != b => println!("  {:28} differs", key),
            _ => {}
        }
    }
    Ok(())
}

fn status(project: &Project, device: &DeviceArgs, json: bool) -> CmdResult<()> {
    let ctx = device_ctx(device, "local");
    let status = device_status(&ctx, true)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&status)?);
        return Ok(());
    }
    println!(
        "{}: type file {} - {}{}",
        device.ip_addr,
        str_of(&status, "file"),
        str_of(&status, "state"),
        if status["present"].as_bool().unwrap_or(false) { format!(" (CRC {})", str_of(&status, "crc")) } else { String::new() }
    );
    print_counts(&status);
    print_entries(&status);
    println!(
        "  records {}/{}, {}/{} bytes, file limit {} bytes{}",
        num_of(&status, "records"),
        num_of(&status, "maxRecords"),
        num_of(&status, "bytes"),
        num_of(&status, "maxBytes"),
        num_of(&status, "maxFileBytes"),
        if status["restartRequired"].as_bool().unwrap_or(false) { " - the file has changed: a restart applies it" } else { "" }
    );
    if project.has_selection() && status["present"].as_bool().unwrap_or(false) {
        let report = project.build_type_file(&project.work_dir.join("devlib.json"), &None, None)?;
        if str_of(&report, "crc") == str_of(&status, "crc") {
            println!("It is the project's type file");
        } else {
            println!("It isn't the project's type file (CRC {}) - 'raft devtypes pull' compares them", str_of(&report, "crc"));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn dev_types_app(cmd: DevTypesCmd) -> CmdResult<()> {
    let project = resolve_project(&cmd)?;
    let s = |v: &str| v.to_string();
    match &cmd.command {
        DevTypesSub::List { json } => project.run_tool(*json, &[s("list")]),
        DevTypesSub::Init => project.run_tool(false, &[s("init")]),
        DevTypesSub::Add { patterns, typefile } | DevTypesSub::Remove { patterns, typefile } => {
            let name = if matches!(cmd.command, DevTypesSub::Add { .. }) { "add" } else { "remove" };
            let mut args = vec![s(name)];
            args.extend(patterns.iter().cloned());
            if *typefile {
                args.push(s("--typefile"));
            }
            project.run_tool(false, &args)
        }
        DevTypesSub::New { key, source, address } => {
            let mut args = vec![s("new"), key.clone(), s("--address"), address.clone()];
            if let Some(source) = source {
                args.push(s("--source"));
                args.push(source.clone());
            }
            project.run_tool(false, &args)
        }
        DevTypesSub::Check { json } => project.run_tool(*json, &[s("check")]),
        DevTypesSub::Update => project.run_tool(false, &[s("update")]),
        DevTypesSub::Diff { key } => {
            let mut args = vec![s("diff")];
            args.extend(key.iter().cloned());
            project.run_tool(false, &args)
        }
        DevTypesSub::Typefile { out, compiled, max_bytes, json } => {
            let out = out.as_ref().map(PathBuf::from).unwrap_or_else(|| project.work_dir.join("devlib.json"));
            let mut args = vec![s("typefile"), s("--out"), out.to_string_lossy().to_string()];
            if let Some(compiled) = compiled {
                args.push(s("--compiled"));
                args.push(compiled.clone());
            }
            if let Some(max_bytes) = max_bytes {
                args.push(s("--max-bytes"));
                args.push(max_bytes.to_string());
            }
            project.run_tool(*json, &args)
        }
        DevTypesSub::Push { device, compiled, yes, dry_run, force } => push(&project, device, compiled, *yes, *dry_run, *force),
        DevTypesSub::Pull { device, out } => pull(&project, device, out),
        DevTypesSub::Status { device, json } => status(&project, device, *json),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_paths() {
        assert_eq!(split_device_path("/local/devlib.json").unwrap(), ("local".to_string(), "devlib.json".to_string()));
        assert_eq!(split_device_path("/sd/types/devlib.json").unwrap(), ("sd".to_string(), "types/devlib.json".to_string()));
        assert!(split_device_path("/devlib.json").is_err());
        assert_eq!(staged_name("devlib.json"), "devlib.new.json");
        assert_eq!(staged_name("types/devlib.json"), "types/devlib.new.json");
        assert_eq!(staged_name("devlib"), "devlib.new");
    }

    #[test]
    fn crc_matches_device() {
        // CRC16-CCITT (init 0xffff, poly 0x1021) as the device and DeviceTypesTool.py report it
        assert_eq!(crc16_ccitt(b"123456789"), 0x29b1);
    }
}
