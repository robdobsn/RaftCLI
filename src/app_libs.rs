// RaftCLI: Local Raft library management
// Rob Dobson 2024

use clap::{Parser, Subcommand};
use std::collections::HashSet;
use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::raft_cli_utils::check_app_folder_valid;

const DEFAULT_DEST_FOLDER: &str = "raftdevlibs";
const DEFAULT_LIBS: [&str; 4] = ["RaftCore", "RaftSysMods", "RaftI2C", "RaftWebServer"];

#[derive(Clone, Parser, Debug)]
#[clap(args_conflicts_with_subcommands = true)]
pub struct LibsCmd {
    #[clap(subcommand)]
    pub command: Option<LibsSubcommand>,
    // Options for the default action (status)
    #[clap(flatten)]
    pub status: LibsStatusCmd,
}

#[derive(Clone, Subcommand, Debug)]
pub enum LibsSubcommand {
    /// Show the libraries in raftdevlibs and their git status (default action)
    #[clap(alias = "st")]
    Status(LibsStatusCmd),
    /// Clone or update Raft libraries into raftdevlibs (linked libraries are not changed)
    #[clap(alias = "f")]
    Fetch(LibsFetchCmd),
    /// Fast-forward pull each library in raftdevlibs from its upstream
    #[clap(alias = "p")]
    Pull(LibsPullCmd),
    /// Link library repos from a folder (default: the folder containing the app) into raftdevlibs
    Link(LibsLinkCmd),
    /// Remove links from raftdevlibs (the linked repos are not changed)
    Unlink(LibsUnlinkCmd),
    /// Delete cloned libraries from raftdevlibs (refused if they hold local work)
    #[clap(alias = "rm")]
    Remove(LibsRemoveCmd),
}

#[derive(Clone, Parser, Debug)]
pub struct LibsFetchCmd {
    #[clap(
        help = "Path to the application folder",
        value_name = "APPLICATION_FOLDER"
    )]
    pub app_folder: Option<String>,
    #[clap(
        long,
        default_value = "robdobsn",
        help = "GitHub account or organisation"
    )]
    pub account: String,
    #[clap(
        long,
        num_args = 1..,
        value_name = "LIB",
        help = "Libraries to fetch (default: RAFT_COMPONENTS in features.cmake)"
    )]
    pub libs: Vec<String>,
    #[clap(
        long,
        default_value = "main",
        help = "Git branch, tag or commit to checkout"
    )]
    pub branch: String,
    #[clap(
        long,
        value_name = "DEST_DIR",
        help = "Destination directory (default: <app-folder>/raftdevlibs)"
    )]
    pub dest: Option<String>,
    #[clap(
        long,
        help = "Update existing repositories even when they have uncommitted changes"
    )]
    pub force: bool,
}

#[derive(Clone, Parser, Debug)]
pub struct LibsStatusCmd {
    #[clap(
        help = "Path to the application folder",
        value_name = "APPLICATION_FOLDER"
    )]
    pub app_folder: Option<String>,
    #[clap(
        long,
        value_name = "DEST_DIR",
        help = "Library directory to inspect (default: <app-folder>/raftdevlibs)"
    )]
    pub dest: Option<String>,
    #[clap(
        long,
        help = "Run 'git fetch' first so ahead/behind counts reflect the remote"
    )]
    pub fetch: bool,
}

#[derive(Clone, Parser, Debug)]
pub struct LibsPullCmd {
    #[clap(
        help = "Path to the application folder",
        value_name = "APPLICATION_FOLDER"
    )]
    pub app_folder: Option<String>,
    #[clap(
        long,
        value_name = "DEST_DIR",
        help = "Library directory to inspect (default: <app-folder>/raftdevlibs)"
    )]
    pub dest: Option<String>,
    #[clap(long, help = "Do not pull linked libraries")]
    pub no_links: bool,
}

#[derive(Clone, Parser, Debug)]
pub struct LibsLinkCmd {
    #[clap(
        help = "Folder containing the library repos (relative to the app folder)",
        value_name = "SOURCE_DIR",
        default_value = ".."
    )]
    pub source_dir: String,
    #[clap(
        long,
        num_args = 1..,
        value_name = "LIB",
        help = "Libraries to link (default: those in RAFT_COMPONENTS found in SOURCE_DIR)"
    )]
    pub libs: Vec<String>,
    #[clap(
        long,
        value_name = "APPLICATION_FOLDER",
        default_value = ".",
        help = "Path to the application folder"
    )]
    pub app: String,
    #[clap(
        long,
        value_name = "DEST_DIR",
        help = "Library directory (default: <app-folder>/raftdevlibs)"
    )]
    pub dest: Option<String>,
    #[clap(
        long,
        help = "Store a relative link instead of an absolute one (not possible with a Windows junction)"
    )]
    pub relative: bool,
    #[clap(long, help = "Replace an existing link which points somewhere else")]
    pub force: bool,
}

#[derive(Clone, Parser, Debug)]
pub struct LibsUnlinkCmd {
    #[clap(
        help = "Path to the application folder",
        value_name = "APPLICATION_FOLDER"
    )]
    pub app_folder: Option<String>,
    #[clap(
        long,
        num_args = 1..,
        value_name = "LIB",
        help = "Links to remove (default: all links)"
    )]
    pub libs: Vec<String>,
    #[clap(
        long,
        value_name = "DEST_DIR",
        help = "Library directory (default: <app-folder>/raftdevlibs)"
    )]
    pub dest: Option<String>,
}

#[derive(Clone, Parser, Debug)]
pub struct LibsRemoveCmd {
    #[clap(
        help = "Path to the application folder",
        value_name = "APPLICATION_FOLDER"
    )]
    pub app_folder: Option<String>,
    #[clap(
        long,
        required = true,
        num_args = 1..,
        value_name = "LIB",
        help = "Libraries to delete"
    )]
    pub libs: Vec<String>,
    #[clap(
        long,
        value_name = "DEST_DIR",
        help = "Library directory (default: <app-folder>/raftdevlibs)"
    )]
    pub dest: Option<String>,
    #[clap(
        long,
        help = "Delete even with uncommitted changes, stashes or unpushed commits"
    )]
    pub force: bool,
}

pub fn run_libs(cmd: &LibsCmd) -> Result<(), Box<dyn Error>> {
    match &cmd.command {
        Some(LibsSubcommand::Status(status)) => status_raft_libs(status),
        Some(LibsSubcommand::Fetch(fetch)) => fetch_raft_libs(fetch),
        Some(LibsSubcommand::Pull(pull)) => pull_raft_libs(pull),
        Some(LibsSubcommand::Link(link)) => link_raft_libs(link),
        Some(LibsSubcommand::Unlink(unlink)) => unlink_raft_libs(unlink),
        Some(LibsSubcommand::Remove(remove)) => remove_raft_libs(remove),
        None => status_raft_libs(&cmd.status),
    }
}

/////////////////////////////////////////////////////////////////////////////////////////////////////////////////
// Entries in raftdevlibs
/////////////////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug, Clone, PartialEq)]
pub enum DevLibKind {
    // Real directory which is a git repo
    Clone,
    // Symlink or junction to a git repo
    Link { target: PathBuf },
    // Link whose target does not exist
    BrokenLink { target: PathBuf },
    // Link to a directory which is not a git repo
    LinkNotRepo { target: PathBuf },
    // Real directory which is not a git repo
    NotRepo,
    // A file or anything else (ignored by the build)
    Other,
}

#[derive(Debug, Clone)]
pub struct DevLibEntry {
    pub name: String,
    pub path: PathBuf,
    pub kind: DevLibKind,
}

impl DevLibEntry {
    pub fn is_git_repo(&self) -> bool {
        matches!(self.kind, DevLibKind::Clone | DevLibKind::Link { .. })
    }
}

// Classify every entry in a raftdevlibs folder (sorted by name, hidden entries skipped)
pub fn scan_devlibs(dir: &Path) -> io::Result<Vec<DevLibEntry>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let kind = classify_devlib(&path);
        entries.push(DevLibEntry { name, path, kind });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

fn classify_devlib(path: &Path) -> DevLibKind {
    if is_link(path) {
        let target = fs::read_link(path).unwrap_or_default();
        if !path.exists() {
            DevLibKind::BrokenLink { target }
        } else if is_git_repo(path) {
            DevLibKind::Link { target }
        } else {
            DevLibKind::LinkNotRepo { target }
        }
    } else if path.is_dir() {
        if is_git_repo(path) {
            DevLibKind::Clone
        } else {
            DevLibKind::NotRepo
        }
    } else {
        DevLibKind::Other
    }
}

// A .git folder, or a .git file for a worktree or submodule
fn is_git_repo(path: &Path) -> bool {
    path.join(".git").exists()
}

// True if the path itself is a symlink or (on Windows) a junction - the link is not followed
fn is_link(path: &Path) -> bool {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => return false,
    };
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        // Junctions are reparse points but are not reported as symlinks - other reparse points
        // (e.g. cloud storage placeholders) are not links so read_link must also succeed
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 && fs::read_link(path).is_ok() {
            return true;
        }
    }
    false
}

// Remove a link without following it
fn remove_link(link: &Path) -> io::Result<()> {
    if !is_link(link) {
        return Err(io::Error::new(io::ErrorKind::Other, format!("not a link: {}", link.display())));
    }
    #[cfg(windows)]
    {
        // Directory symlinks and junctions are removed with remove_dir (which never recurses),
        // file symlinks with remove_file
        fs::remove_dir(link).or_else(|_| fs::remove_file(link))
    }
    #[cfg(not(windows))]
    {
        fs::remove_file(link)
    }
}

#[derive(Debug, PartialEq)]
enum LinkMethod {
    Symlink,
    // Only created on Windows
    #[cfg_attr(not(windows), allow(dead_code))]
    Junction,
}

// Create a directory link - on Windows a junction is used when symlinks are not permitted
// (they need Administrator rights or Developer Mode). Junctions can only hold absolute targets.
fn create_dir_link(abs_target: &Path, link_target: &Path, link: &Path) -> io::Result<LinkMethod> {
    #[cfg(unix)]
    {
        let _ = abs_target;
        std::os::unix::fs::symlink(link_target, link)?;
        Ok(LinkMethod::Symlink)
    }
    #[cfg(windows)]
    {
        const ERROR_PRIVILEGE_NOT_HELD: i32 = 1314;
        match std::os::windows::fs::symlink_dir(link_target, link) {
            Ok(()) => Ok(LinkMethod::Symlink),
            Err(err) if err.raw_os_error() == Some(ERROR_PRIVILEGE_NOT_HELD) => {
                junction::create(abs_target, link)?;
                Ok(LinkMethod::Junction)
            }
            Err(err) => Err(err),
        }
    }
}

// Canonical path without the Windows extended-length prefix (\\?\) so it can be displayed and
// stored in a link
fn canonical_path(path: &Path) -> io::Result<PathBuf> {
    Ok(strip_verbatim_prefix(fs::canonicalize(path)?))
}

fn strip_verbatim_prefix(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy().to_string();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => path,
    }
}

// Path of `to` relative to the folder `from_dir` (both absolute) - None if they have different roots
fn relative_path(from_dir: &Path, to: &Path) -> Option<PathBuf> {
    let from: Vec<_> = from_dir.components().collect();
    let to: Vec<_> = to.components().collect();
    if from.first() != to.first() {
        return None;
    }
    let common = from.iter().zip(to.iter()).take_while(|(a, b)| a == b).count();
    let mut rel = PathBuf::new();
    for _ in common..from.len() {
        rel.push("..");
    }
    for component in &to[common..] {
        rel.push(component);
    }
    Some(rel)
}

// Path for display with the home folder shown as ~
fn display_path(path: &Path) -> String {
    let text = strip_verbatim_prefix(path.to_path_buf()).display().to_string();
    if let Some(home) = dirs::home_dir() {
        let home = home.display().to_string();
        if !home.is_empty() {
            if let Some(rest) = text.strip_prefix(&home) {
                if rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\') {
                    return format!("~{}", rest);
                }
            }
        }
    }
    text
}

fn kind_description(kind: &DevLibKind) -> String {
    match kind {
        DevLibKind::Clone => "clone".to_string(),
        DevLibKind::Link { target } => format!("link -> {}", display_path(target)),
        DevLibKind::BrokenLink { target } => format!("BROKEN link -> {}", display_path(target)),
        DevLibKind::LinkNotRepo { target } => format!("link -> {}", display_path(target)),
        DevLibKind::NotRepo => "folder".to_string(),
        DevLibKind::Other => "file".to_string(),
    }
}

/////////////////////////////////////////////////////////////////////////////////////////////////////////////////
// RAFT_COMPONENTS from features.cmake
/////////////////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug, Clone, PartialEq)]
pub struct RaftComponent {
    pub name: String,
    pub version: Option<String>,
    // Set when the component is given as a full repository URL
    pub repo_url: Option<String>,
}

// Raft components used by any SysType (Common first, then each SysType) - the first entry for a name wins
pub fn read_raft_components(app_folder: &Path) -> Vec<RaftComponent> {
    let systypes = app_folder.join("systypes");
    let mut files = vec![systypes.join("Common").join("features.cmake")];
    let mut others: Vec<PathBuf> = fs::read_dir(&systypes)
        .map(|dir| {
            dir.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.file_name().map(|n| n != "Common").unwrap_or(false))
                .map(|p| p.join("features.cmake"))
                .collect()
        })
        .unwrap_or_default();
    others.sort();
    files.extend(others);

    let mut components: Vec<RaftComponent> = Vec::new();
    for file in files {
        if let Ok(text) = fs::read_to_string(&file) {
            for component in parse_raft_components(&text) {
                if !components.iter().any(|c| c.name == component.name) {
                    components.push(component);
                }
            }
        }
    }
    components
}

// Parse the RAFT_COMPONENTS entries from set(RAFT_COMPONENTS ...) and list(APPEND RAFT_COMPONENTS ...)
// in the text of a features.cmake file. Entries are Name, Name@tag, Name#tag or a repository URL
// (optionally with @tag or #tag) as accepted by RaftBootstrap.cmake.
fn parse_raft_components(text: &str) -> Vec<RaftComponent> {
    // Remove comments - a # starts a comment at the start of a line or after whitespace (Name#tag is not one)
    let mut code = String::new();
    for line in text.lines() {
        let mut end = line.len();
        let mut prev_ws = true;
        for (i, ch) in line.char_indices() {
            if ch == '#' && prev_ws {
                end = i;
                break;
            }
            prev_ws = ch.is_whitespace();
        }
        code.push_str(&line[..end]);
        code.push('\n');
    }

    let mut components = Vec::new();
    let mut rest = code.as_str();
    while let Some(open) = rest.find('(') {
        let command = rest[..open]
            .trim_end()
            .rsplit(|c: char| !(c.is_alphanumeric() || c == '_'))
            .next()
            .unwrap_or("")
            .to_lowercase();
        let after = &rest[open + 1..];
        let close = match after.find(')') {
            Some(close) => close,
            None => break,
        };
        let args: Vec<&str> = after[..close].split_whitespace().collect();
        let items: &[&str] = match (command.as_str(), args.as_slice()) {
            ("set", [var, items @ ..]) if *var == "RAFT_COMPONENTS" => items,
            ("list", [op, var, items @ ..]) if op.eq_ignore_ascii_case("APPEND") && *var == "RAFT_COMPONENTS" => items,
            _ => &[],
        };
        for item in items {
            let item = item.trim_matches('"');
            if item.is_empty() || item.starts_with("${") || item == "PARENT_SCOPE" || item == "CACHE" {
                continue;
            }
            components.push(parse_component(item));
        }
        rest = &after[close + 1..];
    }
    components
}

fn parse_component(item: &str) -> RaftComponent {
    let (name_part, version) = match item.find(|c| c == '@' || c == '#') {
        Some(pos) => (&item[..pos], Some(item[pos + 1..].to_string()).filter(|v| !v.is_empty())),
        None => (item, None),
    };
    if name_part.starts_with("http://") || name_part.starts_with("https://") {
        let name = name_part
            .trim_end_matches(".git")
            .rsplit('/')
            .next()
            .unwrap_or(name_part)
            .to_string();
        RaftComponent { name, version, repo_url: Some(name_part.to_string()) }
    } else {
        RaftComponent { name: name_part.to_string(), version, repo_url: None }
    }
}

// Library names from RAFT_COMPONENTS, or the standard libraries if none are found
fn default_lib_names(components: &[RaftComponent]) -> Vec<String> {
    if components.is_empty() {
        DEFAULT_LIBS.iter().map(|lib| lib.to_string()).collect()
    } else {
        components.iter().map(|c| c.name.clone()).collect()
    }
}

/////////////////////////////////////////////////////////////////////////////////////////////////////////////////
// Checks made by raft build
/////////////////////////////////////////////////////////////////////////////////////////////////////////////////

// A linked library in raftdevlibs
#[derive(Debug, Clone, PartialEq)]
pub struct DevLibLink {
    pub name: String,
    // Canonical path of the linked repo
    pub resolved: PathBuf,
    // Target as stored in the link (may be relative)
    pub link_target: PathBuf,
}

// Local libraries which a build of the app will use
pub struct BuildDevLibs {
    // Linked libraries - Docker builds mount each target over its link
    pub links: Vec<DevLibLink>,
    // Names of all libraries used from raftdevlibs (sorted) - a change means the build folder must be
    // reconfigured as CMake does not notice it
    pub local_names: Vec<String>,
}

// Check raftdevlibs before a build. A broken link for a library the build uses is an error as the
// build would otherwise quietly fetch the remote library instead.
pub fn check_devlibs_for_build(app_folder: &str) -> Result<BuildDevLibs, String> {
    let dir = Path::new(app_folder).join(DEFAULT_DEST_FOLDER);
    let mut result = BuildDevLibs { links: Vec::new(), local_names: Vec::new() };
    let entries = match scan_devlibs(&dir) {
        Ok(entries) => entries,
        Err(_) => return Ok(result),
    };
    let components = read_raft_components(Path::new(app_folder));
    let used = |name: &str| components.is_empty() || components.iter().any(|c| c.name == name);

    for entry in &entries {
        match &entry.kind {
            DevLibKind::BrokenLink { target } => {
                let message = format!(
                    "raftdevlibs/{} is a broken link to {} - fix it or remove it with 'raft l unlink --libs {}'",
                    entry.name, display_path(target), entry.name
                );
                if used(&entry.name) {
                    return Err(message);
                }
                println!("Warning: {}", message);
            }
            DevLibKind::Link { target } | DevLibKind::LinkNotRepo { target } => {
                if let Ok(resolved) = canonical_path(&entry.path) {
                    result.links.push(DevLibLink { name: entry.name.clone(), resolved, link_target: target.clone() });
                }
                result.local_names.push(entry.name.clone());
            }
            DevLibKind::Clone | DevLibKind::NotRepo => result.local_names.push(entry.name.clone()),
            DevLibKind::Other => {}
        }
    }
    Ok(result)
}

// Where Docker mounts a linked library in the container. The project is mounted at /project and Docker
// resolves the link inside the container, so a relative link resolves from /project/raftdevlibs and an
// absolute (Unix) link to its own path. None for a Windows path, which Docker Desktop maps under /mnt/host.
fn docker_mount_point(link_target: &Path) -> Option<String> {
    let target = link_target.to_string_lossy().replace('\\', "/");
    if (target.len() >= 2 && target.as_bytes()[1] == b':') || target.starts_with("//") {
        return None;
    }
    let joined = if target.starts_with('/') { target } else { format!("/project/raftdevlibs/{}", target) };
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    Some(format!("/{}", parts.join("/")))
}

// Check a linked library can be mounted in a Docker build container - a mount over a system folder
// (e.g. /opt, where the ESP-IDF is installed) or over the project itself would break the build
pub fn check_docker_mount(link: &DevLibLink) -> Result<(), String> {
    const SYSTEM_FOLDERS: [&str; 16] = ["/bin", "/boot", "/dev", "/etc", "/lib", "/lib32", "/lib64", "/opt",
        "/proc", "/root", "/run", "/sbin", "/srv", "/sys", "/usr", "/var"];
    let mount = match docker_mount_point(&link.link_target) {
        Some(mount) => mount,
        None => return Ok(()),
    };
    let under = |folder: &str| mount == folder || mount.starts_with(&format!("{}/", folder));
    let in_project = under("/project") && !mount.starts_with("/project/raftdevlibs/");
    let contains_project = "/project/".starts_with(&format!("{}/", mount.trim_end_matches('/')));
    if in_project || contains_project || SYSTEM_FOLDERS.iter().any(|f| under(f)) {
        return Err(format!(
            "raftdevlibs/{} links to {} which would be mounted at {} in the Docker build container - \
             build without Docker (--no-docker) or keep the repo somewhere else",
            link.name, display_path(&link.link_target), mount
        ));
    }
    Ok(())
}

/////////////////////////////////////////////////////////////////////////////////////////////////////////////////
// Fetch libraries into raftdevlibs
/////////////////////////////////////////////////////////////////////////////////////////////////////////////////

pub fn fetch_raft_libs(cmd: &LibsFetchCmd) -> Result<(), Box<dyn Error>> {
    let app_folder = cmd.app_folder.clone().unwrap_or(".".to_string());
    if !check_app_folder_valid(app_folder.clone()) {
        return Err("app folder is not a valid Raft project root".into());
    }

    let dest_dir = resolve_dest_dir(&app_folder, &cmd.dest);
    prepare_dest_dir(&dest_dir)?;

    let components = read_raft_components(Path::new(&app_folder));
    let libs = if cmd.libs.is_empty() {
        default_lib_names(&components)
    } else {
        cmd.libs.clone()
    };

    let mut failures = Vec::new();
    let mut skipped = Vec::new();
    for lib in libs {
        // A component given as a URL is fetched from that URL
        let repo_url = components
            .iter()
            .find(|c| c.name == lib)
            .and_then(|c| c.repo_url.clone())
            .unwrap_or_else(|| format!("https://github.com/{}/{}.git", cmd.account, lib));
        match fetch_lib(&repo_url, &lib, &dest_dir, &cmd.branch, cmd.force) {
            Ok(FetchOutcome::Fetched) => {}
            Ok(FetchOutcome::SkippedLink) => skipped.push(lib),
            Err(err) => {
                eprintln!("  ERROR fetching {}: {}", lib, err);
                failures.push(lib);
            }
        }
    }

    if !failures.is_empty() {
        return Err(format!("Failed to fetch: {}", failures.join(", ")).into());
    }

    if skipped.is_empty() {
        println!("\nAll libraries fetched successfully.");
    } else {
        println!("\nAll libraries fetched successfully (linked libraries skipped: {}).", skipped.join(", "));
    }
    Ok(())
}

#[derive(Debug)]
enum FetchOutcome {
    Fetched,
    SkippedLink,
}

fn resolve_dest_dir(app_folder: &str, dest: &Option<String>) -> PathBuf {
    match dest {
        Some(dest) => {
            let dest_path = PathBuf::from(dest);
            if dest_path.is_absolute() {
                dest_path
            } else {
                PathBuf::from(app_folder).join(dest_path)
            }
        }
        None => PathBuf::from(app_folder).join(DEFAULT_DEST_FOLDER),
    }
}

fn prepare_dest_dir(dest_dir: &Path) -> Result<(), Box<dyn Error>> {
    if dest_dir.exists() && !dest_dir.is_dir() {
        return Err(format!(
            "destination exists but is not a directory: {}",
            dest_dir.display()
        )
        .into());
    }
    fs::create_dir_all(dest_dir)?;
    Ok(())
}

fn fetch_lib(
    repo_url: &str,
    lib: &str,
    dest_dir: &Path,
    branch: &str,
    force: bool,
) -> Result<FetchOutcome, Box<dyn Error>> {
    let lib_path = dest_dir.join(lib);

    // A linked library is a working repo managed elsewhere (and possibly shared by other
    // projects) so it is never checked out or pulled here
    match classify_devlib(&lib_path) {
        DevLibKind::Link { target } => {
            println!("{} is linked to {} - skipped (manage that repo directly)", lib, display_path(&target));
            return Ok(FetchOutcome::SkippedLink);
        }
        DevLibKind::BrokenLink { target } => {
            return Err(format!("{} is a broken link to {}", lib_path.display(), display_path(&target)).into());
        }
        DevLibKind::LinkNotRepo { target } => {
            return Err(format!("{} is a link to {} which is not a git repository", lib_path.display(), display_path(&target)).into());
        }
        DevLibKind::Clone => {
            println!("Updating {} from {}...", lib, repo_url);
            update_existing_repo(&lib_path, branch, force)?;
        }
        DevLibKind::NotRepo | DevLibKind::Other => {
            println!("Cloning {} from {}...", lib, repo_url);
            clone_repo(repo_url, &lib_path)?;
            checkout_ref(&lib_path, branch)?;
        }
    }

    println!("  {} OK", lib);
    Ok(FetchOutcome::Fetched)
}

fn update_existing_repo(lib_path: &Path, branch: &str, force: bool) -> Result<(), Box<dyn Error>> {
    if !force && repo_has_uncommitted_changes(lib_path)? {
        return Err(format!(
            "working tree has uncommitted changes: {} (commit/stash changes or use --force)",
            lib_path.display()
        )
        .into());
    }

    run_git(lib_path, &["fetch", "--all", "--tags"])?;
    checkout_ref(lib_path, branch)?;
    Ok(())
}

fn clone_repo(repo_url: &str, lib_path: &Path) -> Result<(), Box<dyn Error>> {
    if lib_path.exists() {
        return Err(format!(
            "destination exists but is not a git repository: {}",
            lib_path.display()
        )
        .into());
    }

    let status = Command::new("git")
        .arg("clone")
        .arg(repo_url)
        .arg(lib_path)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;

    if !status.success() {
        return Err(format!("git clone failed for {}", repo_url).into());
    }

    Ok(())
}

fn checkout_ref(lib_path: &Path, branch: &str) -> Result<(), Box<dyn Error>> {
    if remote_branch_exists(lib_path, branch)? {
        if local_branch_exists(lib_path, branch)? {
            run_git(lib_path, &["checkout", branch])?;
            run_git(lib_path, &["pull", "--ff-only", "origin", branch])?;
        } else {
            let remote_branch = format!("origin/{}", branch);
            run_git(lib_path, &["checkout", "--track", &remote_branch])?;
        }
    } else {
        run_git(lib_path, &["checkout", branch])?;
    }
    Ok(())
}

fn repo_has_uncommitted_changes(lib_path: &Path) -> Result<bool, Box<dyn Error>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(lib_path)
        .args(["status", "--porcelain"])
        .output()?;

    if !output.status.success() {
        return Err(format!("git status failed in {}", lib_path.display()).into());
    }

    Ok(!output.stdout.is_empty())
}

fn remote_branch_exists(lib_path: &Path, branch: &str) -> Result<bool, Box<dyn Error>> {
    git_ref_exists(lib_path, &format!("refs/remotes/origin/{}", branch))
}

fn local_branch_exists(lib_path: &Path, branch: &str) -> Result<bool, Box<dyn Error>> {
    git_ref_exists(lib_path, &format!("refs/heads/{}", branch))
}

fn git_ref_exists(lib_path: &Path, git_ref: &str) -> Result<bool, Box<dyn Error>> {
    let status = Command::new("git")
        .arg("-C")
        .arg(lib_path)
        .args(["rev-parse", "--verify", git_ref])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;

    Ok(status.success())
}

fn run_git(lib_path: &Path, args: &[&str]) -> Result<(), Box<dyn Error>> {
    let status = Command::new("git")
        .arg("-C")
        .arg(lib_path)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;

    if !status.success() {
        return Err(format!("git {} failed in {}", args.join(" "), lib_path.display()).into());
    }

    Ok(())
}

/////////////////////////////////////////////////////////////////////////////////////////////////////////////////
// Status of libraries in raftdevlibs
/////////////////////////////////////////////////////////////////////////////////////////////////////////////////

struct RepoStatus {
    branch: String,
    commit: String,
    upstream: Option<String>,
    ahead: u32,
    behind: u32,
    changes: usize,
    error: Option<String>,
}

pub fn status_raft_libs(cmd: &LibsStatusCmd) -> Result<(), Box<dyn Error>> {
    let app_folder = cmd.app_folder.clone().unwrap_or(".".to_string());
    if !check_app_folder_valid(app_folder.clone()) {
        return Err("app folder is not a valid Raft project root".into());
    }
    let dest_dir = resolve_dest_dir(&app_folder, &cmd.dest);
    let components = read_raft_components(Path::new(&app_folder));

    let entries = if dest_dir.is_dir() { scan_devlibs(&dest_dir)? } else { Vec::new() };

    if entries.is_empty() {
        println!("\nNo local libraries in {} - the build fetches every library", dest_dir.display());
    } else {
        if cmd.fetch {
            println!("Fetching from remotes...");
            for path in distinct_repo_paths(&entries, true) {
                git_fetch_quiet(&path);
            }
        }
        print_status_table(&dest_dir, &entries, &components);
    }

    // Libraries the build fetches as there is nothing for them in raftdevlibs (a broken link stops the build)
    let remote: Vec<String> = components
        .iter()
        .filter(|c| !entries.iter().any(|e| e.name == c.name && e.kind != DevLibKind::Other))
        .map(|c| match &c.version {
            Some(version) => format!("{}@{}", c.name, version),
            None => c.name.clone(),
        })
        .collect();
    if !remote.is_empty() {
        println!("\nFetched by the build: {}", remote.join(", "));
    }
    Ok(())
}

// Paths of the git repos among the entries, each repo once (two links, or a link and a clone,
// can be the same repo)
fn distinct_repo_paths(entries: &[DevLibEntry], include_links: bool) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut paths = Vec::new();
    for entry in entries {
        let include = match entry.kind {
            DevLibKind::Clone => true,
            DevLibKind::Link { .. } => include_links,
            _ => false,
        };
        if include {
            let key = canonical_path(&entry.path).unwrap_or_else(|_| entry.path.clone());
            if seen.insert(key) {
                paths.push(entry.path.clone());
            }
        }
    }
    paths
}

fn git_fetch_quiet(lib_path: &Path) {
    let _ = Command::new("git")
        .arg("-C")
        .arg(lib_path)
        .args(["fetch", "--all", "--tags", "--quiet"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn git_capture(lib_path: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(lib_path)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

// Run a git command capturing stdout on success or the stderr text on failure.
fn git_capture_result(lib_path: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(lib_path)
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if err.is_empty() {
            Err("git command failed".to_string())
        } else {
            // Collapse to a single line for table display
            Err(err.lines().next().unwrap_or("git command failed").to_string())
        }
    }
}

fn gather_status(lib_path: &Path) -> RepoStatus {
    // If we cannot even read the current branch the repo is unreadable
    let branch = match git_capture_result(lib_path, &["rev-parse", "--abbrev-ref", "HEAD"]) {
        Ok(b) => b,
        Err(err) => {
            return RepoStatus {
                branch: "-".to_string(),
                commit: "-".to_string(),
                upstream: None,
                ahead: 0,
                behind: 0,
                changes: 0,
                error: Some(err),
            };
        }
    };

    let commit = git_capture(lib_path, &["rev-parse", "--short", "HEAD"])
        .unwrap_or_else(|| "-".to_string());

    let upstream = git_capture(
        lib_path,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    );

    let (ahead, behind) = if upstream.is_some() {
        match git_capture(
            lib_path,
            &["rev-list", "--left-right", "--count", "HEAD...@{u}"],
        ) {
            Some(counts) => {
                let mut parts = counts.split_whitespace();
                let ahead = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                let behind = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                (ahead, behind)
            }
            None => (0, 0),
        }
    } else {
        (0, 0)
    };

    let changes = git_capture(lib_path, &["status", "--porcelain"])
        .map(|s| if s.is_empty() { 0 } else { s.lines().count() })
        .unwrap_or(0);

    RepoStatus {
        branch,
        commit,
        upstream,
        ahead,
        behind,
        changes,
        error: None,
    }
}

fn print_status_table(dest_dir: &Path, entries: &[DevLibEntry], components: &[RaftComponent]) {
    println!("\nLibraries in {} (used by the build in place of the remote version)\n", dest_dir.display());

    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut dirty = false;
    let mut behind_any = false;
    let mut problems = Vec::new();
    for entry in entries {
        // The version pinned in features.cmake - "unused" if the project does not use the library
        let pinned = if components.is_empty() {
            "-".to_string()
        } else {
            match components.iter().find(|c| c.name == entry.name) {
                Some(c) => c.version.clone().unwrap_or_else(|| "-".to_string()),
                None => "unused".to_string(),
            }
        };
        let kind = kind_description(&entry.kind);

        if !entry.is_git_repo() {
            let state = match &entry.kind {
                DevLibKind::BrokenLink { .. } => {
                    problems.push(format!("{} is a broken link - fix it or remove it with 'raft l unlink --libs {}'", entry.name, entry.name));
                    "target missing"
                }
                DevLibKind::LinkNotRepo { .. } | DevLibKind::NotRepo => "not a git repo",
                _ => "ignored by the build",
            };
            rows.push(vec![entry.name.clone(), kind, pinned, "-".into(), "-".into(), "-".into(), state.to_string()]);
            continue;
        }

        let status = gather_status(&entry.path);
        let sync = if status.error.is_some() {
            "-".to_string()
        } else if status.upstream.is_none() {
            "no upstream".to_string()
        } else {
            format!("+{}/-{}", status.ahead, status.behind)
        };
        let state = match &status.error {
            Some(err) => err.clone(),
            None if status.changes == 0 => "clean".to_string(),
            None => format!("{} changed", status.changes),
        };
        if status.error.is_none() && status.changes > 0 {
            dirty = true;
        }
        if status.behind > 0 {
            behind_any = true;
        }
        rows.push(vec![entry.name.clone(), kind, pinned, status.branch, status.commit, sync, state]);
    }

    print_table(&["NAME", "TYPE", "PINNED", "BRANCH", "COMMIT", "SYNC", "STATE"], &rows);

    if dirty || behind_any || !problems.is_empty() {
        println!();
        for problem in &problems {
            println!("{}", problem);
        }
        if dirty {
            println!("Some libraries have uncommitted changes.");
        }
        if behind_any {
            println!("Some libraries are behind their upstream (run 'raft l pull' to update them).");
        }
    }
}

fn print_table(headers: &[&str], rows: &[Vec<String>]) {
    let widths: Vec<usize> = (0..headers.len())
        .map(|col| {
            rows.iter()
                .map(|row| row[col].len())
                .chain(std::iter::once(headers[col].len()))
                .max()
                .unwrap_or(0)
        })
        .collect();
    let format_row = |cells: Vec<&str>| {
        let last = cells.len() - 1;
        cells
            .iter()
            .enumerate()
            .map(|(i, cell)| if i == last { cell.to_string() } else { format!("{:<w$}", cell, w = widths[i]) })
            .collect::<Vec<_>>()
            .join("  ")
    };
    println!("{}", format_row(headers.to_vec()));
    for row in rows {
        println!("{}", format_row(row.iter().map(|s| s.as_str()).collect()));
    }
}

/////////////////////////////////////////////////////////////////////////////////////////////////////////////////
// Pull (fast-forward) libraries in raftdevlibs
/////////////////////////////////////////////////////////////////////////////////////////////////////////////////

pub fn pull_raft_libs(cmd: &LibsPullCmd) -> Result<(), Box<dyn Error>> {
    let app_folder = cmd.app_folder.clone().unwrap_or(".".to_string());
    if !check_app_folder_valid(app_folder.clone()) {
        return Err("app folder is not a valid Raft project root".into());
    }
    let dest_dir = resolve_dest_dir(&app_folder, &cmd.dest);

    if !dest_dir.is_dir() {
        println!("No local libraries in {}", dest_dir.display());
        return Ok(());
    }

    let entries = scan_devlibs(&dest_dir)?;
    if !entries.iter().any(|e| e.is_git_repo()) && !entries.iter().any(|e| matches!(e.kind, DevLibKind::BrokenLink { .. })) {
        println!("No git repositories found in {}", dest_dir.display());
        return Ok(());
    }

    println!("Pulling libraries in {}\n", dest_dir.display());

    let mut seen = HashSet::new();
    let mut failures = Vec::new();
    for entry in &entries {
        let label = match entry.kind {
            DevLibKind::Link { .. } => format!("{} (link)", entry.name),
            _ => entry.name.clone(),
        };
        match &entry.kind {
            DevLibKind::Clone | DevLibKind::Link { .. } => {
                if cmd.no_links && matches!(entry.kind, DevLibKind::Link { .. }) {
                    println!("  {:<24} skipped (--no-links)", label);
                    continue;
                }
                let key = canonical_path(&entry.path).unwrap_or_else(|_| entry.path.clone());
                if !seen.insert(key) {
                    println!("  {:<24} same repo as a library above - already pulled", label);
                    continue;
                }
                match pull_lib(&entry.path) {
                    Ok(msg) => println!("  {:<24} {}", label, msg),
                    Err(err) => {
                        println!("  {:<24} SKIPPED: {}", label, err);
                        failures.push(entry.name.clone());
                    }
                }
            }
            DevLibKind::BrokenLink { target } => {
                println!("  {:<24} SKIPPED: broken link to {}", label, display_path(target));
                failures.push(entry.name.clone());
            }
            DevLibKind::LinkNotRepo { .. } | DevLibKind::NotRepo => {
                println!("  {:<24} not a git repo - ignored", label);
            }
            DevLibKind::Other => {}
        }
    }

    if !failures.is_empty() {
        return Err(format!("Could not pull: {}", failures.join(", ")).into());
    }

    println!("\nAll libraries up to date.");
    Ok(())
}

// Fast-forward a single repo. Refuses to touch repos with uncommitted changes
// or without an upstream so local work is never clobbered.
fn pull_lib(lib_path: &Path) -> Result<String, String> {
    // Refuse if the working tree is dirty
    let changes = git_capture(lib_path, &["status", "--porcelain"])
        .ok_or_else(|| "not a readable git repo".to_string())?;
    if !changes.is_empty() {
        return Err("uncommitted changes (commit or stash first)".to_string());
    }

    // Must have an upstream to pull from
    let upstream = git_capture(
        lib_path,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    );
    if upstream.is_none() {
        return Err("no upstream configured".to_string());
    }

    let before = git_capture(lib_path, &["rev-parse", "--short", "HEAD"]);
    git_capture_result(lib_path, &["pull", "--ff-only"])?;
    let after = git_capture(lib_path, &["rev-parse", "--short", "HEAD"]);

    match (before, after) {
        (Some(b), Some(a)) if b == a => Ok("already up to date".to_string()),
        (Some(b), Some(a)) => Ok(format!("updated {} -> {}", b, a)),
        _ => Ok("updated".to_string()),
    }
}

/////////////////////////////////////////////////////////////////////////////////////////////////////////////////
// Link library repos into raftdevlibs
/////////////////////////////////////////////////////////////////////////////////////////////////////////////////

const RECONFIGURE_NOTE: &str = "The next 'raft build' notices the change and rebuilds the SysType from scratch.";

pub fn link_raft_libs(cmd: &LibsLinkCmd) -> Result<(), Box<dyn Error>> {
    if !check_app_folder_valid(cmd.app.clone()) {
        return Err("app folder is not a valid Raft project root".into());
    }
    let app_folder = Path::new(&cmd.app);

    // The source folder is relative to the app folder
    let source = PathBuf::from(&cmd.source_dir);
    let source = if source.is_absolute() { source } else { app_folder.join(source) };
    let source = canonical_path(&source)
        .map_err(|e| format!("source folder {} not found: {}", source.display(), e))?;

    let dest_dir = resolve_dest_dir(&cmd.app, &cmd.dest);
    prepare_dest_dir(&dest_dir)?;
    let dest_dir = canonical_path(&dest_dir)?;

    // Libraries to link - those requested must exist, otherwise all known libraries which are present
    let libs: Vec<String> = if cmd.libs.is_empty() {
        let candidates = default_lib_names(&read_raft_components(app_folder));
        let (found, missing): (Vec<String>, Vec<String>) =
            candidates.into_iter().partition(|lib| is_git_repo(&source.join(lib)));
        if found.is_empty() {
            return Err(format!("none of {} found as git repos in {}", missing.join(", "), display_path(&source)).into());
        }
        if !missing.is_empty() {
            println!("Not found in {} (not linked): {}", display_path(&source), missing.join(", "));
        }
        found
    } else {
        for lib in &cmd.libs {
            if !is_git_repo(&source.join(lib)) {
                return Err(format!("{} is not a git repo", display_path(&source.join(lib))).into());
            }
        }
        cmd.libs.clone()
    };

    let mut rows = Vec::new();
    let mut failures = Vec::new();
    let mut changed = false;
    for lib in &libs {
        let target = canonical_path(&source.join(lib))?;
        let link = dest_dir.join(lib);
        match link_lib(&target, &link, &dest_dir, cmd.relative, cmd.force) {
            Ok((outcome, method)) => {
                if outcome != "already linked" {
                    changed = true;
                }
                let method = match method {
                    Some(LinkMethod::Junction) => "junction",
                    Some(LinkMethod::Symlink) => "symlink",
                    None => "-",
                };
                rows.push(vec![lib.clone(), display_path(&target), method.to_string(), outcome]);
            }
            Err(err) => {
                eprintln!("  ERROR linking {}: {}", lib, err);
                failures.push(lib.clone());
            }
        }
    }

    if !rows.is_empty() {
        println!();
        print_table(&["NAME", "TARGET", "LINK", "RESULT"], &rows);
    }
    if rows.iter().any(|r| r[2] == "junction" && cmd.relative) {
        println!("\nWindows junctions hold absolute paths so --relative was not applied (symlinks need Developer Mode).");
    }
    if changed {
        println!("\n{}", RECONFIGURE_NOTE);
    }
    if !failures.is_empty() {
        return Err(format!("Could not link: {}", failures.join(", ")).into());
    }
    Ok(())
}

// Link one library. Returns the outcome text and the method used (None when nothing was created).
fn link_lib(target: &Path, link: &Path, dest_dir: &Path, relative: bool, force: bool)
        -> Result<(String, Option<LinkMethod>), Box<dyn Error>> {
    let mut outcome = "linked";
    if fs::symlink_metadata(link).is_ok() {
        match classify_devlib(link) {
            DevLibKind::Link { .. } | DevLibKind::LinkNotRepo { .. } | DevLibKind::BrokenLink { .. } => {
                let current = canonical_path(link).ok();
                if current.as_deref() == Some(target) {
                    return Ok(("already linked".to_string(), None));
                }
                if !force {
                    let existing = fs::read_link(link).map(|t| display_path(&t)).unwrap_or_default();
                    return Err(format!("already linked to {} (use --force to replace the link)", existing).into());
                }
                remove_link(link)?;
                outcome = "link replaced";
            }
            DevLibKind::Clone | DevLibKind::NotRepo => {
                return Err(format!(
                    "{} is a local copy, not a link - remove it first with 'raft l remove --libs {}'",
                    link.display(),
                    link.file_name().unwrap_or_default().to_string_lossy()
                )
                .into());
            }
            DevLibKind::Other => {
                return Err(format!("{} exists and is not a folder", link.display()).into());
            }
        }
    }

    let link_target = if relative {
        relative_path(dest_dir, target).ok_or("cannot make a relative link between different drives")?
    } else {
        target.to_path_buf()
    };
    let method = create_dir_link(target, &link_target, link)?;
    Ok((outcome.to_string(), Some(method)))
}

/////////////////////////////////////////////////////////////////////////////////////////////////////////////////
// Unlink libraries
/////////////////////////////////////////////////////////////////////////////////////////////////////////////////

pub fn unlink_raft_libs(cmd: &LibsUnlinkCmd) -> Result<(), Box<dyn Error>> {
    let app_folder = cmd.app_folder.clone().unwrap_or(".".to_string());
    if !check_app_folder_valid(app_folder.clone()) {
        return Err("app folder is not a valid Raft project root".into());
    }
    let dest_dir = resolve_dest_dir(&app_folder, &cmd.dest);
    let entries = if dest_dir.is_dir() { scan_devlibs(&dest_dir)? } else { Vec::new() };
    let is_link_kind = |kind: &DevLibKind| {
        matches!(kind, DevLibKind::Link { .. } | DevLibKind::BrokenLink { .. } | DevLibKind::LinkNotRepo { .. })
    };

    let mut to_remove = Vec::new();
    let mut failures = Vec::new();
    if cmd.libs.is_empty() {
        to_remove.extend(entries.iter().filter(|e| is_link_kind(&e.kind)));
        if to_remove.is_empty() {
            println!("No links in {}", dest_dir.display());
            return Ok(());
        }
    } else {
        for lib in &cmd.libs {
            match entries.iter().find(|e| &e.name == lib) {
                Some(entry) if is_link_kind(&entry.kind) => to_remove.push(entry),
                Some(_) => {
                    eprintln!("  {} is a local copy, not a link - use 'raft l remove --libs {}' to delete it", lib, lib);
                    failures.push(lib.clone());
                }
                None => {
                    eprintln!("  {} is not in {}", lib, dest_dir.display());
                    failures.push(lib.clone());
                }
            }
        }
    }

    for entry in to_remove {
        let target = fs::read_link(&entry.path).map(|t| display_path(&t)).unwrap_or_default();
        match remove_link(&entry.path) {
            Ok(()) => println!("  {:<16} unlinked (was {})", entry.name, target),
            Err(err) => {
                eprintln!("  ERROR unlinking {}: {}", entry.name, err);
                failures.push(entry.name.clone());
            }
        }
    }

    if !failures.is_empty() {
        return Err(format!("Could not unlink: {}", failures.join(", ")).into());
    }
    println!("\n{}", RECONFIGURE_NOTE);
    Ok(())
}

/////////////////////////////////////////////////////////////////////////////////////////////////////////////////
// Remove cloned libraries
/////////////////////////////////////////////////////////////////////////////////////////////////////////////////

pub fn remove_raft_libs(cmd: &LibsRemoveCmd) -> Result<(), Box<dyn Error>> {
    let app_folder = cmd.app_folder.clone().unwrap_or(".".to_string());
    if !check_app_folder_valid(app_folder.clone()) {
        return Err("app folder is not a valid Raft project root".into());
    }
    let dest_dir = resolve_dest_dir(&app_folder, &cmd.dest);

    let mut failures = Vec::new();
    let mut removed = false;
    for lib in &cmd.libs {
        let path = dest_dir.join(lib);
        match remove_lib(&path, cmd.force) {
            Ok(()) => {
                println!("  {:<16} deleted", lib);
                removed = true;
            }
            Err(err) => {
                eprintln!("  {:<16} not deleted: {}", lib, err);
                failures.push(lib.clone());
            }
        }
    }

    if removed {
        println!("\n{}", RECONFIGURE_NOTE);
    }
    if !failures.is_empty() {
        return Err(format!("Could not delete: {}", failures.join(", ")).into());
    }
    Ok(())
}

fn remove_lib(path: &Path, force: bool) -> Result<(), String> {
    if fs::symlink_metadata(path).is_err() {
        return Err("not found".to_string());
    }
    match classify_devlib(path) {
        DevLibKind::Link { .. } | DevLibKind::BrokenLink { .. } | DevLibKind::LinkNotRepo { .. } => {
            return Err("is a link - use 'raft l unlink' (which leaves the linked repo alone)".to_string());
        }
        DevLibKind::Other => return Err("is not a folder".to_string()),
        DevLibKind::NotRepo => {
            if !force {
                return Err("is not a git repo so its contents can't be checked (use --force to delete it)".to_string());
            }
        }
        DevLibKind::Clone => {
            if !force {
                let reasons = local_work_in_repo(path);
                if !reasons.is_empty() {
                    return Err(format!("{} (use --force to delete anyway)", reasons.join(", ")));
                }
            }
        }
    }
    // The remove_dir_all crate copes with read-only files (git objects on Windows) and does not follow links
    remove_dir_all::remove_dir_all(path).map_err(|e| e.to_string())
}

// Reasons why deleting a repo would lose work - empty if it is safe to delete
fn local_work_in_repo(path: &Path) -> Vec<String> {
    let mut reasons = Vec::new();
    match git_capture(path, &["status", "--porcelain"]) {
        Some(changes) if !changes.is_empty() => reasons.push(format!("{} uncommitted changes", changes.lines().count())),
        Some(_) => {}
        None => reasons.push("git status failed".to_string()),
    }
    match git_capture(path, &["stash", "list"]) {
        Some(stashes) if !stashes.is_empty() => reasons.push(format!("{} stashes", stashes.lines().count())),
        Some(_) => {}
        None => reasons.push("git stash list failed".to_string()),
    }
    match git_capture(path, &["log", "--branches", "--not", "--remotes", "--oneline"]) {
        Some(commits) if !commits.is_empty() => reasons.push(format!("{} commits not on any remote", commits.lines().count())),
        Some(_) => {}
        None => reasons.push("git log failed".to_string()),
    }
    reasons
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    // Temporary folder which is removed when dropped
    struct TempFolder(PathBuf);
    static TEMP_COUNTER: AtomicU32 = AtomicU32::new(0);
    impl TempFolder {
        fn new() -> TempFolder {
            let path = std::env::temp_dir().join(format!("raftcli_libs_test_{}_{}",
                        std::process::id(), TEMP_COUNTER.fetch_add(1, Ordering::SeqCst)));
            fs::create_dir_all(&path).unwrap();
            TempFolder(canonical_path(&path).unwrap())
        }
        fn path(&self) -> &Path { &self.0 }
    }
    impl Drop for TempFolder {
        fn drop(&mut self) {
            // Remove links first so nothing can be deleted through them
            for dir in [self.0.join("app").join("raftdevlibs"), self.0.join("raftdevlibs")] {
                if let Ok(entries) = fs::read_dir(&dir) {
                    for entry in entries.flatten() {
                        let _ = remove_link(&entry.path());
                    }
                }
            }
            let _ = remove_dir_all::remove_dir_all(&self.0);
        }
    }

    // Create a directory link (symlink, or a junction where symlinks are not permitted)
    fn make_dir_link(target: &Path, link: &Path) {
        create_dir_link(target, target, link).unwrap();
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.email=test@test", "-c", "user.name=test"])
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {:?} failed in {}", args, dir.display());
    }

    // A git repo with one commit on branch main
    fn make_repo(path: &Path) {
        fs::create_dir_all(path).unwrap();
        git(path, &["init", "-q", "-b", "main"]);
        fs::write(path.join("file.txt"), "content").unwrap();
        git(path, &["add", "."]);
        git(path, &["commit", "-q", "-m", "init"]);
    }

    // A minimal Raft app folder with RAFT_COMPONENTS in Common/features.cmake
    fn make_app(path: &Path, components: &str) {
        fs::create_dir_all(path.join("systypes").join("Common")).unwrap();
        fs::write(path.join("CMakeLists.txt"), "").unwrap();
        fs::write(
            path.join("systypes").join("Common").join("features.cmake"),
            format!("set(RAFT_COMPONENTS\n    {}\n)\n", components),
        )
        .unwrap();
    }

    fn link_cmd(app: &Path, source: &str, libs: &[&str], force: bool) -> LibsLinkCmd {
        LibsLinkCmd {
            source_dir: source.to_string(),
            libs: libs.iter().map(|s| s.to_string()).collect(),
            app: app.display().to_string(),
            dest: None,
            relative: false,
            force,
        }
    }

    #[test]
    fn is_link_detects_links_only() {
        let temp = TempFolder::new();
        let target = temp.path().join("RaftCore");
        fs::create_dir_all(&target).unwrap();
        let link = temp.path().join("linked");
        make_dir_link(&target, &link);

        assert!(is_link(&link));
        assert!(!is_link(&target));
        assert!(!is_link(&temp.path().join("missing")));
        remove_link(&link).unwrap();
    }

    #[test]
    fn scan_classifies_entries() {
        let temp = TempFolder::new();
        let work = temp.path().join("work");
        make_repo(&work.join("Repo"));
        fs::create_dir_all(work.join("Plain")).unwrap();
        fs::create_dir_all(work.join("Gone")).unwrap();

        let dir = temp.path().join("raftdevlibs");
        make_repo(&dir.join("Cloned"));
        fs::create_dir_all(dir.join("Worktree")).unwrap();
        fs::write(dir.join("Worktree").join(".git"), "gitdir: elsewhere").unwrap();
        fs::create_dir_all(dir.join("Folder")).unwrap();
        fs::create_dir_all(dir.join(".hidden")).unwrap();
        fs::write(dir.join("notes.txt"), "").unwrap();
        make_dir_link(&work.join("Repo"), &dir.join("Linked"));
        make_dir_link(&work.join("Plain"), &dir.join("LinkedPlain"));
        make_dir_link(&work.join("Gone"), &dir.join("Broken"));
        fs::remove_dir(work.join("Gone")).unwrap();

        let entries = scan_devlibs(&dir).unwrap();
        let kind = |name: &str| entries.iter().find(|e| e.name == name).map(|e| e.kind.clone());
        assert_eq!(kind("Cloned"), Some(DevLibKind::Clone));
        assert_eq!(kind("Worktree"), Some(DevLibKind::Clone));
        assert_eq!(kind("Folder"), Some(DevLibKind::NotRepo));
        assert_eq!(kind("notes.txt"), Some(DevLibKind::Other));
        assert!(matches!(kind("Linked"), Some(DevLibKind::Link { .. })));
        assert!(matches!(kind("LinkedPlain"), Some(DevLibKind::LinkNotRepo { .. })));
        assert!(matches!(kind("Broken"), Some(DevLibKind::BrokenLink { .. })));
        assert_eq!(kind(".hidden"), None);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }

    #[test]
    fn parses_raft_components() {
        let text = r#"
# set(RAFT_COMPONENTS Commented@v1)
set(RAFT_COMPONENTS
    RaftCore@v1.40.0
    RaftSysMods#main   # trailing comment
    "RaftWebServer"
    https://github.com/someone/RaftExtra.git@v2
    ${EXTRA_COMPONENTS}
)
if(FEATURE_ROS2)
    list(APPEND RAFT_COMPONENTS RaftROS@main)
endif()
set(OTHER_LIST NotAComponent)
"#;
        let components = parse_raft_components(text);
        let names: Vec<&str> = components.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["RaftCore", "RaftSysMods", "RaftWebServer", "RaftExtra", "RaftROS"]);
        assert_eq!(components[0].version.as_deref(), Some("v1.40.0"));
        assert_eq!(components[1].version.as_deref(), Some("main"));
        assert_eq!(components[2].version, None);
        assert_eq!(components[3].repo_url.as_deref(), Some("https://github.com/someone/RaftExtra.git"));
        assert_eq!(components[3].version.as_deref(), Some("v2"));
    }

    #[test]
    fn reads_components_from_common_then_systypes() {
        let temp = TempFolder::new();
        make_app(temp.path(), "RaftCore@v1");
        let board = temp.path().join("systypes").join("BoardA");
        fs::create_dir_all(&board).unwrap();
        fs::write(board.join("features.cmake"), "list(APPEND RAFT_COMPONENTS RaftCore@v2 RaftI2C)").unwrap();
        let components = read_raft_components(temp.path());
        assert_eq!(components.len(), 2);
        assert_eq!(components[0].version.as_deref(), Some("v1"));
        assert_eq!(components[1].name, "RaftI2C");
    }

    #[test]
    fn relative_path_between_folders() {
        let from = Path::new("/a/b/app/raftdevlibs");
        let to = Path::new("/a/raft/RaftCore");
        assert_eq!(relative_path(from, to), Some(PathBuf::from("../../../raft/RaftCore")));
    }

    #[test]
    fn fetch_skips_linked_lib_without_touching_it() {
        let temp = TempFolder::new();
        // A fake repo - any git command run in it would fail, so success shows none was run
        let target = temp.path().join("work").join("RaftCore");
        fs::create_dir_all(target.join(".git")).unwrap();
        fs::write(target.join("marker.txt"), "keep").unwrap();
        let dest = temp.path().join("raftdevlibs");
        fs::create_dir_all(&dest).unwrap();
        make_dir_link(&target, &dest.join("RaftCore"));

        let outcome = fetch_lib("https://example.invalid/RaftCore.git", "RaftCore", &dest, "main", true).unwrap();
        assert!(matches!(outcome, FetchOutcome::SkippedLink));
        assert!(is_link(&dest.join("RaftCore")));
        assert_eq!(fs::read_to_string(target.join("marker.txt")).unwrap(), "keep");
    }

    #[test]
    fn fetch_reports_broken_link() {
        let temp = TempFolder::new();
        let target = temp.path().join("RaftCore");
        fs::create_dir_all(&target).unwrap();
        let dest = temp.path().join("raftdevlibs");
        fs::create_dir_all(&dest).unwrap();
        make_dir_link(&target, &dest.join("RaftCore"));
        fs::remove_dir(&target).unwrap();

        let err = fetch_lib("https://example.invalid/RaftCore.git", "RaftCore", &dest, "main", false).unwrap_err();
        assert!(err.to_string().contains("broken link"), "{}", err);
    }

    #[test]
    fn link_creates_replaces_and_refuses() {
        let temp = TempFolder::new();
        let app = temp.path().join("app");
        make_app(&app, "RaftCore@main\n    RaftSysMods@main\n    RaftI2C@main");
        make_repo(&temp.path().join("RaftCore"));
        make_repo(&temp.path().join("RaftSysMods"));
        let devlibs = app.join("raftdevlibs");

        // Default: libraries in RAFT_COMPONENTS found in the source folder (RaftI2C is missing)
        link_raft_libs(&link_cmd(&app, "..", &[], false)).unwrap();
        let entries = scan_devlibs(&devlibs).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|e| matches!(e.kind, DevLibKind::Link { .. })));
        assert_eq!(canonical_path(&devlibs.join("RaftCore")).unwrap(), temp.path().join("RaftCore"));

        // Linking again is a no-op
        link_raft_libs(&link_cmd(&app, "..", &["RaftCore"], false)).unwrap();

        // A link to somewhere else is only replaced with --force
        let other = temp.path().join("other");
        make_repo(&other.join("RaftCore"));
        assert!(link_raft_libs(&link_cmd(&app, other.to_str().unwrap(), &["RaftCore"], false)).is_err());
        link_raft_libs(&link_cmd(&app, other.to_str().unwrap(), &["RaftCore"], true)).unwrap();
        assert_eq!(canonical_path(&devlibs.join("RaftCore")).unwrap(), other.join("RaftCore"));

        // A clone is never replaced
        remove_link(&devlibs.join("RaftSysMods")).unwrap();
        make_repo(&devlibs.join("RaftSysMods"));
        assert!(link_raft_libs(&link_cmd(&app, "..", &["RaftSysMods"], true)).is_err());
        assert_eq!(classify_devlib(&devlibs.join("RaftSysMods")), DevLibKind::Clone);

        // A requested library missing from the source is an error, as is a source with none
        assert!(link_raft_libs(&link_cmd(&app, "..", &["RaftI2C"], false)).is_err());
        assert!(link_raft_libs(&link_cmd(&app, "../app", &[], false)).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn link_relative() {
        let temp = TempFolder::new();
        let app = temp.path().join("app");
        make_app(&app, "RaftCore@main");
        make_repo(&temp.path().join("RaftCore"));
        let mut cmd = link_cmd(&app, "..", &[], false);
        cmd.relative = true;
        link_raft_libs(&cmd).unwrap();
        let link = app.join("raftdevlibs").join("RaftCore");
        assert_eq!(fs::read_link(&link).unwrap(), PathBuf::from("../../RaftCore"));
        assert!(is_git_repo(&link));
    }

    #[test]
    fn unlink_leaves_target_and_clones_alone() {
        let temp = TempFolder::new();
        let app = temp.path().join("app");
        make_app(&app, "RaftCore@main");
        make_repo(&temp.path().join("RaftCore"));
        let devlibs = app.join("raftdevlibs");
        link_raft_libs(&link_cmd(&app, "..", &[], false)).unwrap();
        make_repo(&devlibs.join("RaftI2C"));
        fs::create_dir_all(temp.path().join("Gone")).unwrap();
        make_dir_link(&temp.path().join("Gone"), &devlibs.join("Broken"));
        fs::remove_dir(temp.path().join("Gone")).unwrap();

        let unlink = |libs: &[&str]| unlink_raft_libs(&LibsUnlinkCmd {
            app_folder: Some(app.display().to_string()),
            libs: libs.iter().map(|s| s.to_string()).collect(),
            dest: None,
        });

        // A clone is refused
        assert!(unlink(&["RaftI2C"]).is_err());
        // All links (including broken ones) are removed and the targets are untouched
        unlink(&[]).unwrap();
        let names: Vec<String> = scan_devlibs(&devlibs).unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["RaftI2C"]);
        assert_eq!(fs::read_to_string(temp.path().join("RaftCore").join("file.txt")).unwrap(), "content");
    }

    #[test]
    fn remove_checks_for_local_work() {
        let temp = TempFolder::new();
        let app = temp.path().join("app");
        make_app(&app, "RaftCore@main");
        let devlibs = app.join("raftdevlibs");
        let remote = temp.path().join("remote");
        make_repo(&remote);
        let clone = devlibs.join("RaftCore");
        fs::create_dir_all(&devlibs).unwrap();
        git(&devlibs, &["clone", "-q", remote.to_str().unwrap(), "RaftCore"]);

        // A clean clone with everything pushed is deleted
        assert!(local_work_in_repo(&clone).is_empty());
        remove_lib(&clone, false).unwrap();
        assert!(!clone.exists());

        // Uncommitted changes, stashes and unpushed commits are each reported
        git(&devlibs, &["clone", "-q", remote.to_str().unwrap(), "RaftCore"]);
        fs::write(clone.join("file.txt"), "changed").unwrap();
        assert_eq!(local_work_in_repo(&clone).len(), 1);
        git(&clone, &["stash", "-q"]);
        fs::write(clone.join("new.txt"), "new").unwrap();
        git(&clone, &["add", "."]);
        git(&clone, &["commit", "-q", "-m", "local"]);
        let reasons = local_work_in_repo(&clone);
        assert_eq!(reasons.len(), 2, "{:?}", reasons);
        assert!(remove_lib(&clone, false).is_err());
        assert!(clone.exists());
        remove_lib(&clone, true).unwrap();
        assert!(!clone.exists());

        // A link is never deleted through
        make_dir_link(&remote, &clone);
        assert!(remove_lib(&clone, true).is_err());
        assert!(remote.join("file.txt").exists());
    }

    #[test]
    fn docker_mount_points_are_checked() {
        let mount = |target: &str| docker_mount_point(Path::new(target));
        assert_eq!(mount("../../raft/RaftCore").as_deref(), Some("/raft/RaftCore"));
        assert_eq!(mount("../RaftCore").as_deref(), Some("/project/RaftCore"));
        assert_eq!(mount("/home/me/raft/RaftCore").as_deref(), Some("/home/me/raft/RaftCore"));
        assert_eq!(mount(r"C:\Users\me\raft\RaftCore"), None);

        let check = |target: &str| check_docker_mount(&DevLibLink {
            name: "RaftCore".to_string(),
            resolved: PathBuf::from(target),
            link_target: PathBuf::from(target),
        });
        for ok in ["/home/me/raft/RaftCore", "../../raft/RaftCore", "/tmp/RaftCore", "/workspace/RaftCore",
                   r"C:\dev\RaftCore", "/optional/RaftCore", "./Other"] {
            assert!(check(ok).is_ok(), "{}", ok);
        }
        for bad in ["/opt/raft/RaftCore", "../../../opt/esp", "/usr/src/RaftCore", "../RaftCore", "/project",
                    "..", "../../..", "/", "/project/raftdevlibs"] {
            assert!(check(bad).is_err(), "{}", bad);
        }
    }

    #[test]
    fn build_check_reports_broken_links_and_local_libs() {
        let temp = TempFolder::new();
        let app = temp.path().join("app");
        make_app(&app, "RaftCore@main\n    RaftSysMods@main");
        make_repo(&temp.path().join("RaftCore"));
        let devlibs = app.join("raftdevlibs");
        link_raft_libs(&link_cmd(&app, "..", &["RaftCore"], false)).unwrap();
        make_repo(&devlibs.join("RaftSysMods"));

        let check = check_devlibs_for_build(app.to_str().unwrap()).unwrap();
        assert_eq!(check.local_names, ["RaftCore", "RaftSysMods"]);
        assert_eq!(check.links.len(), 1);
        assert_eq!(check.links[0].name, "RaftCore");
        assert_eq!(check.links[0].resolved, temp.path().join("RaftCore"));

        // A broken link to an unused library is only a warning, to a used library it is an error
        fs::create_dir_all(temp.path().join("Gone")).unwrap();
        make_dir_link(&temp.path().join("Gone"), &devlibs.join("Unused"));
        fs::remove_dir(temp.path().join("Gone")).unwrap();
        assert!(check_devlibs_for_build(app.to_str().unwrap()).is_ok());
        remove_link(&devlibs.join("RaftCore")).unwrap();
        fs::create_dir_all(temp.path().join("Gone")).unwrap();
        make_dir_link(&temp.path().join("Gone"), &devlibs.join("RaftCore"));
        fs::remove_dir(temp.path().join("Gone")).unwrap();
        assert!(check_devlibs_for_build(app.to_str().unwrap()).is_err());
    }
}
