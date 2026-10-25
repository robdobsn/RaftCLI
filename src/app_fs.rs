// RaftCLI: Filesystem management commands (`raft fs ...`)
// Talks to a running Raft device's FileManager REST API, over any link DeviceConn supports.
// Rob Dobson 2024

use clap::{Args, Subcommand};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::device_conn::{hand_off_to_windows, DeviceArgs, DeviceConn, LinkKind, Positionals};
use crate::raft_cli_utils::utils_get_sys_type;

// ---------------------------------------------------------------------------
// CLI definitions
// ---------------------------------------------------------------------------

#[derive(Clone, Args, Debug)]
#[clap(args_conflicts_with_subcommands = true)]
pub struct FsCmd {
    #[clap(subcommand)]
    pub command: Option<FsSub>,
    /// Default action (no subcommand): mirror a built FSImage onto the device
    #[clap(flatten)]
    pub image: FsImageArgs,
}

// Options for the default action: `raft fs <device> -s <systype>` uploads the built
// filesystem image (build/<systype>/raft/FSImage) in mirror mode.
#[derive(Clone, Args, Debug)]
pub struct FsImageArgs {
    #[clap(flatten)]
    pub device: DeviceArgs,
    /// System type whose built FSImage to upload (default: last built / first found)
    #[clap(short = 's', long = "sys-type", value_name = "SYSTYPE")]
    pub sys_type: Option<String>,
    /// Path to the application folder (default: current directory)
    #[clap(long = "app-folder", value_name = "APPLICATION_FOLDER")]
    pub app_folder: Option<String>,
    /// Target filesystem: local or sd (default local)
    #[clap(long)]
    pub fs: Option<String>,
    /// Glob/name patterns to exclude (repeatable; default: .built)
    #[clap(long, value_name = "PATTERN")]
    pub exclude: Vec<String>,
    /// Do not delete remote files missing locally (default is mirror/delete)
    #[clap(long)]
    pub no_delete: bool,
    /// Use size-only change detection instead of content hash (CRC16)
    #[clap(long)]
    pub no_hash: bool,
    /// Preview the planned actions without changing the device
    #[clap(long)]
    pub dry_run: bool,
    /// Prompt before deleting stale remote files (default: skip prompt)
    #[clap(long)]
    pub confirm: bool,
}

// Options common to all fs subcommands. The device is the first positional argument unless -d/--device or
// RAFT_DEVICE gives it; then the positional arguments are all the subcommand's (see device_conn::resolve_device),
// which is why clap sees them as optional
#[derive(Clone, Args, Debug)]
pub struct FsCommon {
    #[clap(flatten)]
    pub device: DeviceArgs,
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
        /// Remote file path on the device (required)
        #[clap(value_name = "REMOTE")]
        remote: Option<String>,
        /// Local destination path (default: basename of remote; '-' for stdout)
        local: Option<String>,
    },
    /// Upload a file to the device
    Put {
        #[clap(flatten)]
        common: FsCommon,
        /// Local file to upload (required)
        #[clap(value_name = "LOCAL")]
        local: Option<String>,
        /// Remote destination filename (default: basename of local)
        remote: Option<String>,
    },
    /// Print a file's contents
    Cat {
        #[clap(flatten)]
        common: FsCommon,
        /// Remote file path on the device (required)
        #[clap(value_name = "REMOTE")]
        remote: Option<String>,
    },
    /// Delete a file on the device
    #[clap(alias = "rm")]
    Delete {
        #[clap(flatten)]
        common: FsCommon,
        /// Remote file path on the device (required)
        #[clap(value_name = "REMOTE")]
        remote: Option<String>,
        /// Skip confirmation
        #[clap(short = 'y', long)]
        yes: bool,
    },
    /// Mirror a local folder onto the device (primary web-UI update workflow)
    Sync {
        #[clap(flatten)]
        common: FsCommon,
        /// Local directory to mirror (required)
        #[clap(value_name = "LOCALDIR")]
        localdir: Option<String>,
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
        /// Glob/name patterns to exclude (repeatable)
        #[clap(long, value_name = "PATTERN")]
        exclude: Vec<String>,
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

pub(crate) struct DeviceCtx {
    pub(crate) conn: Rc<dyn DeviceConn>,
    pub(crate) fs: String,
}

impl DeviceCtx {
    // The same device, another file system
    pub(crate) fn with_fs(&self, fs: &str) -> DeviceCtx {
        DeviceCtx { conn: self.conn.clone(), fs: fs.to_string() }
    }
}

// Open the device and return the subcommand's own positional arguments (`rest`, in order)
fn resolve_ctx(common: &FsCommon, rest: Vec<Option<String>>) -> Result<(DeviceCtx, Positionals), Box<dyn std::error::Error>> {
    let (device, args) = common.device.resolve(rest)?;
    let ctx = DeviceCtx {
        conn: common.device.open(&device)?,
        fs: common.fs.clone().unwrap_or_else(|| "local".to_string()),
    };
    Ok((ctx, args))
}

// In WSL, a command for a serial device runs in the Windows raft.exe: true if it was handed over (and has run)
fn hand_off(cmd: &FsCmd) -> Result<bool, Box<dyn std::error::Error>> {
    // The device options, the positional arguments after the device, and which of those are local paths
    let (device, rest, local_indexes): (&DeviceArgs, Vec<Option<String>>, Vec<usize>) = match &cmd.command {
        None => (&cmd.image.device, vec![], vec![]),
        Some(FsSub::List { common, folder, .. }) => (&common.device, vec![folder.clone()], vec![]),
        Some(FsSub::Df { common }) | Some(FsSub::Format { common, .. }) => (&common.device, vec![], vec![]),
        Some(FsSub::Get { common, remote, local }) => (&common.device, vec![remote.clone(), local.clone()], vec![1]),
        Some(FsSub::Put { common, local, remote }) => (&common.device, vec![local.clone(), remote.clone()], vec![0]),
        Some(FsSub::Cat { common, remote }) | Some(FsSub::Delete { common, remote, .. }) => {
            (&common.device, vec![remote.clone()], vec![])
        }
        Some(FsSub::Sync { common, localdir, remotedir, .. }) => {
            (&common.device, vec![localdir.clone(), remotedir.clone()], vec![0])
        }
    };
    let Ok((name, args)) = device.resolve(rest) else {
        // Reported when the command runs here
        return Ok(false);
    };
    let mut local_paths: Vec<String> = local_indexes.iter().filter_map(|&i| args.get(i).cloned()).collect();
    if cmd.command.is_none() {
        local_paths.extend(cmd.image.app_folder.clone());
    }
    hand_off_to_windows(&name, device.native_serial_port, &local_paths)
}

// Encode a device-relative path for the FileManager REST API:
//  - strip leading '/'
//  - replace '/' with '~'
pub(crate) fn encode_device_path(path: &str) -> String {
    path.trim_start_matches('/').replace('/', "~")
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn fs_app(cmd: FsCmd) -> Result<(), Box<dyn std::error::Error>> {
    if hand_off(&cmd)? {
        return Ok(());
    }
    let command = match cmd.command {
        Some(c) => c,
        None => return fs_image_default(&cmd.image),
    };
    match command {
        FsSub::List { common, folder, json, recursive } => {
            let (ctx, mut args) = resolve_ctx(&common, vec![folder])?;
            let folder = args.optional().unwrap_or_default();
            args.finish()?;
            fs_list(&ctx, &folder, json, recursive)
        }
        FsSub::Df { common } => {
            let (ctx, args) = resolve_ctx(&common, vec![])?;
            args.finish()?;
            fs_df(&ctx)
        }
        FsSub::Get { common, remote, local } => {
            let (ctx, mut args) = resolve_ctx(&common, vec![remote, local])?;
            let remote = args.required("REMOTE")?;
            let local = args.optional();
            args.finish()?;
            fs_get(&ctx, &remote, local.as_deref())
        }
        FsSub::Put { common, local, remote } => {
            let (ctx, mut args) = resolve_ctx(&common, vec![local, remote])?;
            let local = args.required("LOCAL")?;
            let remote = args.optional().unwrap_or_else(|| basename(&local));
            args.finish()?;
            fs_put(&ctx, &local, &remote)
        }
        FsSub::Cat { common, remote } => {
            let (ctx, mut args) = resolve_ctx(&common, vec![remote])?;
            let remote = args.required("REMOTE")?;
            args.finish()?;
            fs_cat(&ctx, &remote)
        }
        FsSub::Delete { common, remote, yes } => {
            let (ctx, mut args) = resolve_ctx(&common, vec![remote])?;
            let remote = args.required("REMOTE")?;
            args.finish()?;
            fs_delete(&ctx, &remote, yes)
        }
        FsSub::Sync { common, localdir, remotedir, delete, dry_run, hash, exclude, yes } => {
            let (ctx, mut args) = resolve_ctx(&common, vec![localdir, remotedir])?;
            let localdir = args.required("LOCALDIR")?;
            let remotedir = args.optional().unwrap_or_default();
            args.finish()?;
            fs_sync(&ctx, &localdir, &remotedir, delete, dry_run, hash, &exclude, yes)
        }
        FsSub::Format { common, yes } => {
            let (ctx, args) = resolve_ctx(&common, vec![])?;
            args.finish()?;
            fs_format(&ctx, yes)
        }
    }
}

// Default action: `raft fs <device> -s <systype>` mirrors the built filesystem image
// (build/<systype>/raft/FSImage) onto the device's filesystem.
fn fs_image_default(args: &FsImageArgs) -> Result<(), Box<dyn std::error::Error>> {
    let (device, positionals) = args.device.resolve(vec![]).map_err(|_| {
        "no device specified. Usage: raft fs <device> -s <systype>  (or -d/--device or RAFT_DEVICE; for subcommands see 'raft fs --help')"
    })?;
    positionals.finish()?;

    let app_folder = args.app_folder.clone().unwrap_or_else(|| ".".to_string());
    let sys_type = utils_get_sys_type(&args.sys_type, app_folder.clone())?;
    if sys_type.is_empty() {
        return Err("could not determine system type. Specify one with -s <systype>".into());
    }

    let localdir = format!("{}/build/{}/raft/FSImage", app_folder, sys_type);
    if !Path::new(&localdir).is_dir() {
        return Err(format!(
            "built filesystem image not found: {} (build the '{}' system type first)",
            localdir, sys_type
        )
        .into());
    }

    // Default excludes the build marker file; user-supplied patterns override.
    let exclude: Vec<String> = if args.exclude.is_empty() {
        vec![".built".to_string()]
    } else {
        args.exclude.clone()
    };

    let ctx = DeviceCtx {
        conn: args.device.open(&device)?,
        fs: args.fs.clone().unwrap_or_else(|| "local".to_string()),
    };

    println!(
        "Mirroring FSImage for system type '{}' -> device {} (fs {})",
        sys_type, ctx.conn.describe(), ctx.fs
    );

    let delete = !args.no_delete;
    let hash = !args.no_hash;
    let yes = !args.confirm;
    fs_sync(&ctx, &localdir, "", delete, args.dry_run, hash, &exclude, yes)
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
    let json = ctx.conn.rest_ok(&filelist_cmd(ctx, folder))?;
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

fn filelist_cmd(ctx: &DeviceCtx, folder: &str) -> String {
    let enc = encode_device_path(folder);
    if enc.is_empty() {
        format!("filelist/{}/", ctx.fs)
    } else {
        format!("filelist/{}/{}", ctx.fs, enc)
    }
}

fn fs_list(ctx: &DeviceCtx, folder: &str, json: bool, recursive: bool) -> Result<(), Box<dyn std::error::Error>> {
    if json {
        let body = ctx.conn.rest_raw(&filelist_cmd(ctx, folder))?;
        println!("{}", String::from_utf8_lossy(&body));
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

// Download a file using the binary-safe filesection endpoint (base64 in JSON), in chunks that suit the link
pub(crate) fn download_file(ctx: &DeviceCtx, remote: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let chunk_size = ctx.conn.link_kind().filesection_chunk();
    let enc = encode_device_path(remote);
    let mut data: Vec<u8> = Vec::new();
    let mut start: u64 = 0;
    loop {
        let json = ctx.conn.rest_ok(&format!("filesection/{}/{}/{}/{}", ctx.fs, enc, start, chunk_size))?;
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

// The same download as get (binary-safe, and the same on every link - fileread over RICREST never answers for a
// missing file on older firmware)
fn fs_cat(ctx: &DeviceCtx, remote: &str) -> Result<(), Box<dyn std::error::Error>> {
    let data = download_file(ctx, remote)?;
    std::io::stdout().write_all(&data)?;
    Ok(())
}

fn fs_put(ctx: &DeviceCtx, local: &str, remote: &str) -> Result<(), Box<dyn std::error::Error>> {
    if !Path::new(local).exists() {
        return Err(format!("Local file not found: {}", local).into());
    }
    upload_file(ctx, local, remote, true)
}

// Upload a single file, with a progress line if verbose
pub(crate) fn upload_file(ctx: &DeviceCtx, local: &str, remote: &str, verbose: bool) -> Result<(), Box<dyn std::error::Error>> {
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
    let result = ctx.conn.put_file(Path::new(local), remote, &ctx.fs, &mut progress);
    if verbose {
        println!();
    }
    result?;
    if verbose {
        println!("Uploaded {} -> {}", local, remote);
    }
    Ok(())
}

fn fs_delete(ctx: &DeviceCtx, remote: &str, yes: bool) -> Result<(), Box<dyn std::error::Error>> {
    if !yes && !confirm(&format!("Delete '{}' on device {}?", remote, ctx.conn.describe()))? {
        println!("Aborted.");
        return Ok(());
    }
    delete_remote(ctx, remote)?;
    println!("Deleted {}", remote);
    Ok(())
}

pub(crate) fn delete_remote(ctx: &DeviceCtx, remote: &str) -> Result<(), Box<dyn std::error::Error>> {
    let enc = encode_device_path(remote);
    ctx.conn
        .rest_ok(&format!("filedelete/{}/{}", ctx.fs, enc))
        .map_err(|e| format!("Delete failed: {}", e))?;
    Ok(())
}

fn fs_format(ctx: &DeviceCtx, yes: bool) -> Result<(), Box<dyn std::error::Error>> {
    println!("WARNING: this will ERASE filesystem '{}' on {} and reboot the device.", ctx.fs, ctx.conn.describe());
    if !yes && !confirm("Are you sure?")? {
        println!("Aborted.");
        return Ok(());
    }
    match ctx.conn.rest_raw(&format!("reformatfs/{}/force", ctx.fs)) {
        Ok(body) => println!("{}", String::from_utf8_lossy(&body)),
        // Over serial the device may restart before replying (a native USB port then goes away)
        Err(e) if ctx.conn.link_kind() != LinkKind::Http => println!(
            "No reply ({}) - the device restarts after formatting; check it with 'raft fs ls' when it is back",
            e
        ),
        Err(e) => return Err(e),
    }
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
    exclude: &[String],
    yes: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let base = Path::new(localdir);
    if !base.is_dir() {
        return Err(format!("Local directory not found: {}", localdir).into());
    }

    // Enumerate local files (recursively)
    let mut local_files: Vec<LocalEntry> = Vec::new();
    enumerate_local(base, base, &mut local_files)?;

    // Drop excluded files
    if !exclude.is_empty() {
        local_files.retain(|e| !is_excluded(&e.rel, exclude));
    }

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
    let json = ctx.conn.rest(&format!("filehash/{}/{}", ctx.fs, enc)).ok()?;
    json["crc16"].as_str().map(|s| s.to_string())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn basename(path: &str) -> String {
    let p = path.replace('\\', "/");
    p.rsplit('/').next().unwrap_or(&p).to_string()
}

// True if a relative path matches any exclude pattern (against the full path or
// its basename). Patterns support '*' as a wildcard.
fn is_excluded(rel: &str, patterns: &[String]) -> bool {
    let base = rel.rsplit('/').next().unwrap_or(rel);
    patterns
        .iter()
        .any(|pat| glob_match(pat, rel) || glob_match(pat, base))
}

// Minimal glob matcher supporting '*' (matches any sequence, including '/').
fn glob_match(pattern: &str, text: &str) -> bool {
    fn helper(p: &[u8], t: &[u8]) -> bool {
        if p.is_empty() {
            return t.is_empty();
        }
        if p[0] == b'*' {
            helper(&p[1..], t) || (!t.is_empty() && helper(p, &t[1..]))
        } else {
            !t.is_empty() && p[0] == t[0] && helper(&p[1..], &t[1..])
        }
    }
    helper(pattern.as_bytes(), text.as_bytes())
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

pub(crate) fn confirm(prompt: &str) -> Result<bool, Box<dyn std::error::Error>> {
    print!("{} [y/N] ", prompt);
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(matches!(input.trim().to_lowercase().as_str(), "y" | "yes"))
}

// CRC16-CCITT matching MiniHDLC::crcUpdateCCITT on the device (init 0xFFFF, poly 0x1021)
pub(crate) fn crc16_ccitt(data: &[u8]) -> u16 {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device_conn::fake::FakeConn;
    use crate::device_conn::LinkKind;

    fn ctx_for(fake: &Rc<FakeConn>) -> DeviceCtx {
        DeviceCtx { conn: fake.clone(), fs: "local".to_string() }
    }

    // A local folder with the given files, removed when dropped
    struct LocalDir(PathBuf);

    impl LocalDir {
        fn new(name: &str, files: &[(&str, &[u8])]) -> LocalDir {
            let dir = std::env::temp_dir().join(format!("raftcli_fs_test_{}_{}", name, std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            for (rel, data) in files {
                let path = dir.join(rel);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, data).unwrap();
            }
            LocalDir(dir)
        }
        fn path(&self) -> &str {
            self.0.to_str().unwrap()
        }
    }

    impl Drop for LocalDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn sync_uploads_changes_and_deletes_stale() {
        let fake = Rc::new(FakeConn::new(LinkKind::Http));
        fake.add("local/index.html", b"old page");
        fake.add("local/same.txt", b"unchanged");
        fake.add("local/stale.txt", b"gone locally");
        fake.add("local/web/stale.js", b"gone too");
        let local = LocalDir::new("sync", &[("index.html", b"new page"), ("same.txt", b"unchanged"), ("web/app.js", b"js")]);

        fs_sync(&ctx_for(&fake), local.path(), "", true, false, true, &[], true).unwrap();

        assert_eq!(fake.file("local/index.html").unwrap(), b"new page");
        assert_eq!(fake.file("local/web/app.js").unwrap(), b"js");
        assert_eq!(fake.file("local/same.txt").unwrap(), b"unchanged");
        assert!(fake.file("local/stale.txt").is_none());
        assert!(fake.file("local/web/stale.js").is_none());
        // The unchanged file was compared by hash and not uploaded again
        let mut puts = fake.commands("put");
        puts.sort();
        assert_eq!(puts, vec!["put local/index.html", "put local/web/app.js"]);
    }

    #[test]
    fn sync_dry_run_and_excludes_change_nothing() {
        let fake = Rc::new(FakeConn::new(LinkKind::Http));
        fake.add("local/stale.txt", b"x");
        let local = LocalDir::new("dryrun", &[("a.txt", b"a"), (".built", b"")]);

        fs_sync(&ctx_for(&fake), local.path(), "", true, true, true, &[".built".to_string()], true).unwrap();
        assert!(fake.commands("put").is_empty());
        assert!(fake.commands("filedelete").is_empty());

        fs_sync(&ctx_for(&fake), local.path(), "", false, false, false, &[".built".to_string()], true).unwrap();
        assert_eq!(fake.commands("put"), vec!["put local/a.txt"]);
        assert!(fake.file("local/stale.txt").is_some(), "no --delete, so stale files stay");
    }

    #[test]
    fn download_uses_the_links_chunk_size() {
        let data: Vec<u8> = (0..2500u32).map(|i| (i * 7) as u8).collect();
        for (link, requests) in [(LinkKind::Http, 1), (LinkKind::Serial, 3)] {
            let fake = Rc::new(FakeConn::new(link));
            fake.add("local/dir/data.bin", &data);
            assert_eq!(download_file(&ctx_for(&fake), "/dir/data.bin").unwrap(), data);
            let sections = fake.commands("filesection");
            assert_eq!(sections.len(), requests, "{:?}: {:?}", link, sections);
            assert!(sections[0].starts_with("filesection/local/dir~data.bin/0/"));
        }
    }

    #[test]
    fn device_errors_are_reported() {
        let fake = Rc::new(FakeConn::new(LinkKind::Http));
        let ctx = ctx_for(&fake);
        let err = download_file(&ctx, "missing.txt").unwrap_err().to_string();
        assert!(err.contains("nofile"), "{}", err);
        let err = delete_remote(&ctx, "missing.txt").unwrap_err().to_string();
        assert!(err.starts_with("Delete failed") && err.contains("nofile"), "{}", err);
        assert!(remote_hash(&ctx, "missing.txt").is_none());
    }

    // Every command over RICREST: the simulated device on a serial link, with log text between frames
    #[test]
    fn commands_over_ricrest() {
        use crate::link::sim::SimulatedLink;
        use crate::link::{Link, WireFormat};
        use crate::ricrest::conn::RicrestConn;

        let mut link = SimulatedLink::new(LinkKind::Serial, WireFormat::OverAsciiHdlc);
        link.device.faults.log_text = true;
        link.device.store.add("local/stale.txt", b"old");
        let conn = Rc::new(RicrestConn::new(link, "serial:sim", LinkKind::Serial, false));
        let ctx = DeviceCtx { conn: conn.clone(), fs: "local".to_string() };
        let big: Vec<u8> = (0..5000u32).map(|i| (i % 253) as u8).collect();
        let local = LocalDir::new("ricrest", &[("index.html", b"<html/>"), ("web/big.bin", &big)]);

        // sync with hash and delete, then a second sync has nothing to do
        fs_sync(&ctx, local.path(), "", true, false, true, &[], true).unwrap();
        let file = |key: &str| conn.session().link().device.store.file(key);
        assert_eq!(file("local/web/big.bin").unwrap(), big);
        assert!(file("local/stale.txt").is_none());
        let uploads = |c: &RicrestConn<SimulatedLink>| c.session().link().device.received.iter().filter(|r| r.contains("ufStart")).count();
        assert_eq!(uploads(&conn), 2);
        fs_sync(&ctx, local.path(), "", true, false, true, &[], true).unwrap();
        assert_eq!(uploads(&conn), 2, "unchanged files aren't uploaded again");

        // get (1 KB chunks on serial), cat, ls, df, put to another folder, rm
        assert_eq!(download_file(&ctx, "web/big.bin").unwrap(), big);
        fs_cat(&ctx, "index.html").unwrap();
        let (_, _, entries) = fetch_listing(&ctx, "web").unwrap();
        assert_eq!(entries[0].name, "big.bin");
        fs_df(&ctx).unwrap();
        upload_file(&ctx, local.0.join("index.html").to_str().unwrap(), "copy/index.html", false).unwrap();
        assert_eq!(file("local/copy/index.html").unwrap(), b"<html/>");
        delete_remote(&ctx, "copy/index.html").unwrap();
        assert!(file("local/copy/index.html").is_none());
        // --fs is honoured over RICREST (HTTP upload can't name a file system)
        upload_file(&ctx.with_fs("sd"), local.0.join("index.html").to_str().unwrap(), "index.html", false).unwrap();
        assert!(file("sd/index.html").is_some());
        assert_eq!(conn.session().link().bad_frames(), 0);
    }

    #[test]
    fn cat_and_listing_use_filesection_and_filelist() {
        let fake = Rc::new(FakeConn::new(LinkKind::Http));
        fake.add("local/notes.txt", b"hello");
        fake.add("local/sub/x.bin", &[0, 1, 2]);
        let ctx = ctx_for(&fake);
        fs_cat(&ctx, "notes.txt").unwrap();
        assert!(fake.commands("fileread").is_empty());
        assert_eq!(fake.commands("filesection").len(), 1);

        let (size, used, entries) = fetch_listing(&ctx, "").unwrap();
        assert_eq!((size, used), (1_000_000, 8));
        let names: Vec<(String, bool)> = entries.iter().map(|e| (e.name.clone(), e.is_dir)).collect();
        assert_eq!(names, vec![("notes.txt".to_string(), false), ("sub".to_string(), true)]);
        assert_eq!(fake.commands("filelist"), vec!["filelist/local/"]);
    }
}
