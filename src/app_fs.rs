// RaftCLI: Filesystem management commands (`raft fs ...`)
// Talks to a running Raft device's FileManager REST API.
// Rob Dobson 2024

use clap::{Args, Subcommand};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::http_client::{http_get, http_post_multipart_file};

// ---------------------------------------------------------------------------
// CLI definitions
// ---------------------------------------------------------------------------

#[derive(Clone, Args, Debug)]
pub struct FsCmd {
    #[clap(subcommand)]
    pub command: FsSub,
}

// Options common to all fs subcommands
#[derive(Clone, Args, Debug)]
pub struct FsCommon {
    /// Device IP address or hostname
    #[clap(value_name = "IP_ADDRESS_OR_HOSTNAME")]
    pub ip_addr: String,
    /// HTTP port (default 80)
    #[clap(short = 'p', long)]
    pub port: Option<u16>,
    /// Target filesystem: local or sd (default local)
    #[clap(long)]
    pub fs: Option<String>,
}

#[derive(Clone, Subcommand, Debug)]
pub enum FsSub {
    /// List files in a folder on the device
    #[clap(alias = "ls")]
    List {
        #[clap(flatten)]
        common: FsCommon,
        /// Folder to list (default root)
        folder: Option<String>,
        /// Output raw device JSON
        #[clap(long)]
        json: bool,
        /// Recurse into sub-folders
        #[clap(short = 'r', long)]
        recursive: bool,
    },
    /// Show disk size/used/free for a filesystem
    Df {
        #[clap(flatten)]
        common: FsCommon,
    },
    /// Download a file from the device
    Get {
        #[clap(flatten)]
        common: FsCommon,
        /// Remote file path on the device
        remote: String,
        /// Local destination path (default: basename of remote; '-' for stdout)
        local: Option<String>,
    },
    /// Upload a file to the device
    Put {
        #[clap(flatten)]
        common: FsCommon,
        /// Local file to upload
        local: String,
        /// Remote destination filename (default: basename of local)
        remote: Option<String>,
    },
    /// Print a text file's contents
    Cat {
        #[clap(flatten)]
        common: FsCommon,
        /// Remote file path on the device
        remote: String,
    },
    /// Delete a file on the device
    #[clap(alias = "rm")]
    Delete {
        #[clap(flatten)]
        common: FsCommon,
        /// Remote file path on the device
        remote: String,
        /// Skip confirmation
        #[clap(short = 'y', long)]
        yes: bool,
    },
    /// Mirror a local folder onto the device (primary web-UI update workflow)
    Sync {
        #[clap(flatten)]
        common: FsCommon,
        /// Local directory to mirror
        localdir: String,
        /// Remote base folder (default root)
        remotedir: Option<String>,
        /// Delete remote files not present locally (mirror mode)
        #[clap(long)]
        delete: bool,
        /// Show planned actions without changing the device
        #[clap(long)]
        dry_run: bool,
        /// Use content hash (CRC16) to detect changes (else size-only)
        #[clap(long)]
        hash: bool,
        /// Skip confirmation for deletions
        #[clap(short = 'y', long)]
        yes: bool,
    },
    /// Reformat a filesystem (DESTRUCTIVE - reboots the device)
    Format {
        #[clap(flatten)]
        common: FsCommon,
        /// Required confirmation flag
        #[clap(short = 'y', long)]
        yes: bool,
    },
}

// ---------------------------------------------------------------------------
// Resolved connection context
// ---------------------------------------------------------------------------

struct DeviceCtx {
    ip_addr: String,
    port: u16,
    fs: String,
}

fn resolve_ctx(common: &FsCommon) -> Result<DeviceCtx, Box<dyn std::error::Error>> {
    Ok(DeviceCtx {
        ip_addr: common.ip_addr.clone(),
        port: common.port.unwrap_or(80),
        fs: common.fs.clone().unwrap_or_else(|| "local".to_string()),
    })
}

// Encode a device-relative path for the FileManager REST API:
//  - strip leading '/'
//  - replace '/' with '~'
fn encode_device_path(path: &str) -> String {
    path.trim_start_matches('/').replace('/', "~")
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn fs_app(cmd: FsCmd) -> Result<(), Box<dyn std::error::Error>> {
    match cmd.command {
        FsSub::List { common, folder, json, recursive } => {
            let ctx = resolve_ctx(&common)?;
            fs_list(&ctx, folder.as_deref().unwrap_or(""), json, recursive)
        }
        FsSub::Df { common } => {
            let ctx = resolve_ctx(&common)?;
            fs_df(&ctx)
        }
        FsSub::Get { common, remote, local } => {
            let ctx = resolve_ctx(&common)?;
            fs_get(&ctx, &remote, local.as_deref())
        }
        FsSub::Put { common, local, remote } => {
            let ctx = resolve_ctx(&common)?;
            let remote = remote.unwrap_or_else(|| basename(&local));
            fs_put(&ctx, &local, &remote)
        }
        FsSub::Cat { common, remote } => {
            let ctx = resolve_ctx(&common)?;
            fs_cat(&ctx, &remote)
        }
        FsSub::Delete { common, remote, yes } => {
            let ctx = resolve_ctx(&common)?;
            fs_delete(&ctx, &remote, yes)
        }
        FsSub::Sync { common, localdir, remotedir, delete, dry_run, hash, yes } => {
            let ctx = resolve_ctx(&common)?;
            fs_sync(&ctx, &localdir, remotedir.as_deref().unwrap_or(""), delete, dry_run, hash, yes)
        }
        FsSub::Format { common, yes } => {
            let ctx = resolve_ctx(&common)?;
            fs_format(&ctx, yes)
        }
    }
}

// ---------------------------------------------------------------------------
// Subcommand implementations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct RemoteEntry {
    name: String,
    size: u64,
    is_dir: bool,
}

// Fetch a single folder listing; returns (disk_size, disk_used, entries)
fn fetch_listing(ctx: &DeviceCtx, folder: &str) -> Result<(u64, u64, Vec<RemoteEntry>), Box<dyn std::error::Error>> {
    let enc = encode_device_path(folder);
    let path = if enc.is_empty() {
        format!("/api/filelist/{}/", ctx.fs)
    } else {
        format!("/api/filelist/{}/{}", ctx.fs, enc)
    };
    let resp = http_get(&ctx.ip_addr, ctx.port, &path)?;
    let json: serde_json::Value = serde_json::from_str(&resp.body_string())
        .map_err(|e| format!("Invalid JSON from device: {}", e))?;
    if json["rslt"].as_str() != Some("ok") {
        return Err(format!("Device error: {}", json["rslt"].as_str().unwrap_or("unknown")).into());
    }
    let disk_size = json["diskSize"].as_u64().unwrap_or(0);
    let disk_used = json["diskUsed"].as_u64().unwrap_or(0);
    let mut entries = Vec::new();
    if let Some(files) = json["files"].as_array() {
        for f in files {
            entries.push(RemoteEntry {
                name: f["name"].as_str().unwrap_or("").to_string(),
                size: f["size"].as_u64().unwrap_or(0),
                is_dir: f["isDir"].as_u64().unwrap_or(0) != 0,
            });
        }
    }
    Ok((disk_size, disk_used, entries))
}

fn fs_list(ctx: &DeviceCtx, folder: &str, json: bool, recursive: bool) -> Result<(), Box<dyn std::error::Error>> {
    if json {
        let enc = encode_device_path(folder);
        let path = if enc.is_empty() {
            format!("/api/filelist/{}/", ctx.fs)
        } else {
            format!("/api/filelist/{}/{}", ctx.fs, enc)
        };
        let resp = http_get(&ctx.ip_addr, ctx.port, &path)?;
        println!("{}", resp.body_string());
        return Ok(());
    }

    let (disk_size, disk_used, entries) = fetch_listing(ctx, folder)?;
    println!(
        "Filesystem: {}   Size: {}   Used: {}   Free: {}",
        ctx.fs,
        human_size(disk_size),
        human_size(disk_used),
        human_size(disk_size.saturating_sub(disk_used))
    );
    print_entries(ctx, folder, &entries, recursive)?;
    Ok(())
}

fn print_entries(ctx: &DeviceCtx, folder: &str, entries: &[RemoteEntry], recursive: bool) -> Result<(), Box<dyn std::error::Error>> {
    println!("{:>10}  {}", "SIZE", "NAME");
    for e in entries {
        let display = join_remote(folder, &e.name);
        if e.is_dir {
            println!("{:>10}  {}/", "<dir>", display);
        } else {
            println!("{:>10}  {}", e.size, display);
        }
    }
    if recursive {
        for e in entries {
            if e.is_dir {
                let sub = join_remote(folder, &e.name);
                let (_, _, sub_entries) = fetch_listing(ctx, &sub)?;
                print_entries(ctx, &sub, &sub_entries, true)?;
            }
        }
    }
    Ok(())
}

fn fs_df(ctx: &DeviceCtx) -> Result<(), Box<dyn std::error::Error>> {
    let (disk_size, disk_used, _) = fetch_listing(ctx, "")?;
    println!("Filesystem: {}", ctx.fs);
    println!("  Size: {}", human_size(disk_size));
    println!("  Used: {}", human_size(disk_used));
    println!("  Free: {}", human_size(disk_size.saturating_sub(disk_used)));
    Ok(())
}

// Download a file using the binary-safe filesection endpoint (base64 in JSON)
fn download_file(ctx: &DeviceCtx, remote: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    const CHUNK: u64 = 4096;
    let enc = encode_device_path(remote);
    let mut data: Vec<u8> = Vec::new();
    let mut start: u64 = 0;
    loop {
        let path = format!("/api/filesection/{}/{}/{}/{}", ctx.fs, enc, start, CHUNK);
        let resp = http_get(&ctx.ip_addr, ctx.port, &path)?;
        let json: serde_json::Value = serde_json::from_str(&resp.body_string())
            .map_err(|e| format!("Invalid JSON from device: {}", e))?;
        if json["rslt"].as_str() != Some("ok") {
            return Err(format!("Device error: {}", json["rslt"].as_str().unwrap_or("unknown")).into());
        }
        let total = json["total"].as_u64().unwrap_or(0);
        let chunk_b64 = json["data"].as_str().unwrap_or("");
        let chunk = base64_decode(chunk_b64)?;
        let chunk_len = chunk.len() as u64;
        data.extend_from_slice(&chunk);
        start += chunk_len;
        if chunk_len == 0 || start >= total {
            break;
        }
    }
    Ok(data)
}

fn fs_get(ctx: &DeviceCtx, remote: &str, local: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let data = download_file(ctx, remote)?;
    match local {
        Some("-") => {
            std::io::stdout().write_all(&data)?;
        }
        other => {
            let dest = other.map(|s| s.to_string()).unwrap_or_else(|| basename(remote));
            std::fs::write(&dest, &data)?;
            println!("Downloaded {} ({} bytes) -> {}", remote, data.len(), dest);
        }
    }
    Ok(())
}

fn fs_cat(ctx: &DeviceCtx, remote: &str) -> Result<(), Box<dyn std::error::Error>> {
    // Use fileread (text) for cat; warn if it looks binary
    let enc = encode_device_path(remote);
    let path = format!("/api/fileread/{}/{}", ctx.fs, enc);
    let resp = http_get(&ctx.ip_addr, ctx.port, &path)?;
    if !resp.is_success() {
        return Err(format!("Failed to read {} (HTTP {})", remote, resp.status).into());
    }
    if resp.body.contains(&0u8) {
        eprintln!("warning: file appears to be binary; output may be truncated. Use 'raft fs get' instead.");
    }
    std::io::stdout().write_all(&resp.body)?;
    Ok(())
}

fn fs_put(ctx: &DeviceCtx, local: &str, remote: &str) -> Result<(), Box<dyn std::error::Error>> {
    if !Path::new(local).exists() {
        return Err(format!("Local file not found: {}", local).into());
    }
    upload_file(ctx, local, remote, true)
}

// Upload a single file via the streaming multipart endpoint
fn upload_file(ctx: &DeviceCtx, local: &str, remote: &str, verbose: bool) -> Result<(), Box<dyn std::error::Error>> {
    // The destination filename is carried in the multipart filename field.
    // The device's default filesystem is used; '/' separators are preserved for sub-folders.
    let path = "/api/fileupload".to_string();
    let mut last_pct: i64 = -1;
    let mut progress = |sent: u64, total: u64, rate: f64| {
        if !verbose || total == 0 {
            return;
        }
        let pct = (sent * 100 / total) as i64;
        if pct != last_pct {
            last_pct = pct;
            print!("\rUploading {} : {}% ({}/{} bytes) {:.1} KB/s   ", remote, pct, sent, total, rate / 1024.0);
            let _ = std::io::stdout().flush();
        }
    };
    let resp = http_post_multipart_file(
        &ctx.ip_addr,
        ctx.port,
        &path,
        "file",
        remote,
        local,
        Some(&mut progress),
    )?;
    if verbose {
        println!();
    }
    let body = resp.body_string();
    if resp.is_success() && body.contains("\"rslt\":\"ok\"") {
        if verbose {
            println!("Uploaded {} -> {}", local, remote);
        }
        Ok(())
    } else {
        Err(format!("Upload failed (HTTP {}): {}", resp.status, body).into())
    }
}

fn fs_delete(ctx: &DeviceCtx, remote: &str, yes: bool) -> Result<(), Box<dyn std::error::Error>> {
    if !yes && !confirm(&format!("Delete '{}' on device {}?", remote, ctx.ip_addr))? {
        println!("Aborted.");
        return Ok(());
    }
    delete_remote(ctx, remote)?;
    println!("Deleted {}", remote);
    Ok(())
}

fn delete_remote(ctx: &DeviceCtx, remote: &str) -> Result<(), Box<dyn std::error::Error>> {
    let enc = encode_device_path(remote);
    let path = format!("/api/filedelete/{}/{}", ctx.fs, enc);
    let resp = http_get(&ctx.ip_addr, ctx.port, &path)?;
    let body = resp.body_string();
    if resp.is_success() && body.contains("\"rslt\":\"ok\"") {
        Ok(())
    } else {
        Err(format!("Delete failed (HTTP {}): {}", resp.status, body).into())
    }
}

fn fs_format(ctx: &DeviceCtx, yes: bool) -> Result<(), Box<dyn std::error::Error>> {
    println!("WARNING: this will ERASE filesystem '{}' on {} and reboot the device.", ctx.fs, ctx.ip_addr);
    if !yes && !confirm("Are you sure?")? {
        println!("Aborted.");
        return Ok(());
    }
    let path = format!("/api/reformatfs/{}/force", ctx.fs);
    let resp = http_get(&ctx.ip_addr, ctx.port, &path)?;
    println!("{}", resp.body_string());
    Ok(())
}

// ---------------------------------------------------------------------------
// Sync
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct LocalEntry {
    rel: String,
    abs: PathBuf,
    size: u64,
}

fn fs_sync(
    ctx: &DeviceCtx,
    localdir: &str,
    remotedir: &str,
    delete: bool,
    dry_run: bool,
    hash: bool,
    yes: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let base = Path::new(localdir);
    if !base.is_dir() {
        return Err(format!("Local directory not found: {}", localdir).into());
    }

    // Enumerate local files (recursively)
    let mut local_files: Vec<LocalEntry> = Vec::new();
    enumerate_local(base, base, &mut local_files)?;

    // Enumerate remote files (recursively)
    let mut remote_files: Vec<(String, u64)> = Vec::new();
    enumerate_remote(ctx, remotedir, &mut remote_files)?;

    // Index remote by relative path (relative to remotedir)
    let remote_prefix = normalize_rel(remotedir);
    let remote_map: std::collections::HashMap<String, u64> = remote_files
        .iter()
        .map(|(p, sz)| (strip_prefix_rel(p, &remote_prefix), *sz))
        .collect();
    let local_set: std::collections::HashSet<String> =
        local_files.iter().map(|e| e.rel.clone()).collect();

    // Compute to-upload and to-delete
    let mut to_upload: Vec<&LocalEntry> = Vec::new();
    for e in &local_files {
        match remote_map.get(&e.rel) {
            None => to_upload.push(e),
            Some(&rsize) => {
                let changed = if hash {
                    let rhash = remote_hash(ctx, &join_remote(remotedir, &e.rel)).unwrap_or_default();
                    let lhash = local_crc16(&e.abs).unwrap_or_default();
                    rhash.is_empty() || rhash != lhash
                } else {
                    rsize != e.size
                };
                if changed {
                    to_upload.push(e);
                }
            }
        }
    }
    let to_delete: Vec<String> = remote_map
        .keys()
        .filter(|r| !local_set.contains(*r))
        .cloned()
        .collect();

    // Report plan
    println!("Sync plan for {} -> {} (fs {})", localdir, if remotedir.is_empty() { "/" } else { remotedir }, ctx.fs);
    println!("  upload: {}   delete: {}   unchanged: {}",
        to_upload.len(),
        if delete { to_delete.len() } else { 0 },
        local_files.len() - to_upload.len());

    if dry_run {
        for e in &to_upload { println!("  + {}", e.rel); }
        if delete { for r in &to_delete { println!("  - {}", r); } }
        println!("(dry run - no changes made)");
        return Ok(());
    }

    if delete && !to_delete.is_empty() && !yes
        && !confirm(&format!("Delete {} stale remote file(s)?", to_delete.len()))?
    {
        println!("Aborted.");
        return Ok(());
    }

    // Delete first (free space before uploading)
    if delete {
        for r in &to_delete {
            let remote_path = join_remote(remotedir, r);
            match delete_remote(ctx, &remote_path) {
                Ok(_) => println!("  deleted {}", remote_path),
                Err(e) => eprintln!("  failed to delete {}: {}", remote_path, e),
            }
        }
    }

    // Upload changed/new files
    for e in &to_upload {
        let remote_path = join_remote(remotedir, &e.rel);
        upload_file(ctx, e.abs.to_str().unwrap_or(""), &remote_path, true)?;
    }

    // Final disk usage
    if let Ok((disk_size, disk_used, _)) = fetch_listing(ctx, remotedir) {
        println!("Done. Disk used {} of {}", human_size(disk_used), human_size(disk_size));
    } else {
        println!("Done.");
    }
    Ok(())
}

fn enumerate_local(base: &Path, dir: &Path, out: &mut Vec<LocalEntry>) -> Result<(), Box<dyn std::error::Error>> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            enumerate_local(base, &path, out)?;
        } else if path.is_file() {
            let rel = path
                .strip_prefix(base)?
                .to_string_lossy()
                .replace('\\', "/");
            let size = entry.metadata()?.len();
            out.push(LocalEntry { rel, abs: path, size });
        }
    }
    Ok(())
}

fn enumerate_remote(ctx: &DeviceCtx, folder: &str, out: &mut Vec<(String, u64)>) -> Result<(), Box<dyn std::error::Error>> {
    // A missing remote folder is not an error for sync - treat it as empty.
    let entries = match fetch_listing(ctx, folder) {
        Ok((_, _, entries)) => entries,
        Err(_) => return Ok(()),
    };
    for e in entries {
        let full = join_remote(folder, &e.name);
        if e.is_dir {
            enumerate_remote(ctx, &full, out)?;
        } else {
            out.push((full, e.size));
        }
    }
    Ok(())
}

fn remote_hash(ctx: &DeviceCtx, remote: &str) -> Option<String> {
    let enc = encode_device_path(remote);
    let path = format!("/api/filehash/{}/{}", ctx.fs, enc);
    let resp = http_get(&ctx.ip_addr, ctx.port, &path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&resp.body_string()).ok()?;
    json["crc16"].as_str().map(|s| s.to_string())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn basename(path: &str) -> String {
    let p = path.replace('\\', "/");
    p.rsplit('/').next().unwrap_or(&p).to_string()
}

fn join_remote(folder: &str, name: &str) -> String {
    let f = folder.trim_matches('/');
    if f.is_empty() {
        name.trim_start_matches('/').to_string()
    } else {
        format!("{}/{}", f, name.trim_start_matches('/'))
    }
}

fn normalize_rel(p: &str) -> String {
    p.trim_matches('/').to_string()
}

fn strip_prefix_rel(full: &str, prefix: &str) -> String {
    let full = full.trim_matches('/');
    if prefix.is_empty() {
        full.to_string()
    } else if let Some(rest) = full.strip_prefix(prefix) {
        rest.trim_start_matches('/').to_string()
    } else {
        full.to_string()
    }
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{} {}", bytes, UNITS[0])
    } else {
        format!("{:.1} {}", v, UNITS[i])
    }
}

fn confirm(prompt: &str) -> Result<bool, Box<dyn std::error::Error>> {
    print!("{} [y/N] ", prompt);
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(matches!(input.trim().to_lowercase().as_str(), "y" | "yes"))
}

// CRC16-CCITT matching MiniHDLC::crcUpdateCCITT on the device (init 0xFFFF, poly 0x1021)
fn crc16_ccitt(data: &[u8]) -> u16 {
    let mut fcs: u16 = 0xFFFF;
    for &b in data {
        fcs = (fcs << 8) ^ crc16_table((((fcs >> 8) as u8) ^ b) as usize);
    }
    fcs
}

fn crc16_table(index: usize) -> u16 {
    // CCITT table value computed on the fly (poly 0x1021)
    let mut v: u16 = (index as u16) << 8;
    for _ in 0..8 {
        if v & 0x8000 != 0 {
            v = (v << 1) ^ 0x1021;
        } else {
            v <<= 1;
        }
    }
    v
}

fn local_crc16(path: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let data = std::fs::read(path)?;
    Ok(format!("{:04x}", crc16_ccitt(&data)))
}

// Minimal base64 decoder (standard alphabet, ignores whitespace/padding)
fn base64_decode(s: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::new();
    let mut buf: u32 = 0;
    let mut bits = 0;
    for &c in s.as_bytes() {
        if c == b'=' || c == b'\r' || c == b'\n' || c == b' ' {
            continue;
        }
        let v = val(c).ok_or("Invalid base64 character")? as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Ok(out)
}
