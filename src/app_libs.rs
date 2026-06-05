// RaftCLI: Local Raft library management
// Rob Dobson 2024

use clap::{Parser, Subcommand};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::raft_cli_utils::check_app_folder_valid;

const DEFAULT_DEST_FOLDER: &str = "raftdevlibs";
const DEFAULT_LIBS: [&str; 4] = ["RaftCore", "RaftSysMods", "RaftI2C", "RaftWebServer"];

#[derive(Clone, Parser, Debug)]
pub struct LibsCmd {
    #[clap(subcommand)]
    pub command: Option<LibsSubcommand>,
}

#[derive(Clone, Subcommand, Debug)]
pub enum LibsSubcommand {
    /// Clone or update Raft libraries into raftdevlibs (default action)
    #[clap(alias = "f")]
    Fetch(LibsFetchCmd),
    /// Show the git status of each library in raftdevlibs
    #[clap(alias = "st")]
    Status(LibsStatusCmd),
    /// Fast-forward pull each library in raftdevlibs from its upstream
    #[clap(alias = "p")]
    Pull(LibsPullCmd),
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
    #[clap(long, num_args = 1.., value_name = "LIB", help = "Libraries to fetch")]
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
}

pub fn run_libs(cmd: &LibsCmd) -> Result<(), Box<dyn Error>> {
    match &cmd.command {
        Some(LibsSubcommand::Fetch(fetch)) => fetch_raft_libs(fetch),
        Some(LibsSubcommand::Status(status)) => status_raft_libs(status),
        Some(LibsSubcommand::Pull(pull)) => pull_raft_libs(pull),
        None => fetch_raft_libs(&LibsFetchCmd {
            app_folder: None,
            account: "robdobsn".to_string(),
            libs: Vec::new(),
            branch: "main".to_string(),
            dest: None,
            force: false,
        }),
    }
}

pub fn fetch_raft_libs(cmd: &LibsFetchCmd) -> Result<(), Box<dyn Error>> {
    let app_folder = cmd.app_folder.clone().unwrap_or(".".to_string());
    if !check_app_folder_valid(app_folder.clone()) {
        return Err("app folder is not a valid Raft project root".into());
    }

    let dest_dir = resolve_dest_dir(&app_folder, &cmd.dest);
    prepare_dest_dir(&dest_dir)?;

    let libs = if cmd.libs.is_empty() {
        DEFAULT_LIBS.iter().map(|lib| lib.to_string()).collect()
    } else {
        cmd.libs.clone()
    };

    let mut failures = Vec::new();
    for lib in libs {
        if let Err(err) = fetch_lib(&cmd.account, &lib, &dest_dir, &cmd.branch, cmd.force) {
            eprintln!("  ERROR fetching {}: {}", lib, err);
            failures.push(lib);
        }
    }

    if !failures.is_empty() {
        return Err(format!("Failed to fetch: {}", failures.join(", ")).into());
    }

    println!("\nAll libraries fetched successfully.");
    Ok(())
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
    account: &str,
    lib: &str,
    dest_dir: &Path,
    branch: &str,
    force: bool,
) -> Result<(), Box<dyn Error>> {
    let repo_url = format!("https://github.com/{}/{}.git", account, lib);
    let lib_path = dest_dir.join(lib);

    if lib_path.join(".git").is_dir() {
        println!("Updating {} from {}...", lib, account);
        update_existing_repo(&lib_path, branch, force)?;
    } else {
        println!("Cloning {} from {}...", lib, account);
        clone_repo(&repo_url, &lib_path)?;
        checkout_ref(&lib_path, branch)?;
    }

    println!("  {} OK", lib);
    Ok(())
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
    name: String,
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
    let dest_dir = resolve_dest_dir(&app_folder, &cmd.dest);

    if !dest_dir.is_dir() {
        return Err(format!("library folder not found: {}", dest_dir.display()).into());
    }

    let mut repos = list_git_repos(&dest_dir)?;
    repos.sort();

    if repos.is_empty() {
        println!("No git repositories found in {}", dest_dir.display());
        return Ok(());
    }

    if cmd.fetch {
        println!("Fetching from remotes...");
        for repo in &repos {
            git_fetch_quiet(&dest_dir.join(repo));
        }
    }

    let rows: Vec<RepoStatus> = repos
        .iter()
        .map(|repo| gather_status(repo, &dest_dir.join(repo)))
        .collect();

    print_status_table(&dest_dir, &rows);
    Ok(())
}

fn list_git_repos(dest_dir: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    let mut repos = Vec::new();
    for entry in fs::read_dir(dest_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() && path.join(".git").exists() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                repos.push(name.to_string());
            }
        }
    }
    Ok(repos)
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

fn gather_status(name: &str, lib_path: &Path) -> RepoStatus {
    // If we cannot even read the current branch the repo is unreadable
    let branch = match git_capture_result(lib_path, &["rev-parse", "--abbrev-ref", "HEAD"]) {
        Ok(b) => b,
        Err(err) => {
            return RepoStatus {
                name: name.to_string(),
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
        name: name.to_string(),
        branch,
        commit,
        upstream,
        ahead,
        behind,
        changes,
        error: None,
    }
}

fn print_status_table(dest_dir: &Path, rows: &[RepoStatus]) {
    println!("\nGit status of libraries in {}\n", dest_dir.display());

    let sync_strings: Vec<String> = rows
        .iter()
        .map(|r| {
            if r.error.is_some() {
                "-".to_string()
            } else if r.upstream.is_none() {
                "no upstream".to_string()
            } else {
                format!("+{}/-{}", r.ahead, r.behind)
            }
        })
        .collect();

    let state_strings: Vec<String> = rows
        .iter()
        .map(|r| match &r.error {
            Some(err) => err.clone(),
            None => {
                if r.changes == 0 {
                    "clean".to_string()
                } else {
                    format!("{} changed", r.changes)
                }
            }
        })
        .collect();

    let name_w = rows
        .iter()
        .map(|r| r.name.len())
        .chain(std::iter::once("NAME".len()))
        .max()
        .unwrap_or(4);
    let branch_w = rows
        .iter()
        .map(|r| r.branch.len())
        .chain(std::iter::once("BRANCH".len()))
        .max()
        .unwrap_or(6);
    let commit_w = rows
        .iter()
        .map(|r| r.commit.len())
        .chain(std::iter::once("COMMIT".len()))
        .max()
        .unwrap_or(6);
    let sync_w = sync_strings
        .iter()
        .map(|s| s.len())
        .chain(std::iter::once("SYNC".len()))
        .max()
        .unwrap_or(4);

    println!(
        "{:<name_w$}  {:<branch_w$}  {:<commit_w$}  {:<sync_w$}  {}",
        "NAME",
        "BRANCH",
        "COMMIT",
        "SYNC",
        "STATE",
        name_w = name_w,
        branch_w = branch_w,
        commit_w = commit_w,
        sync_w = sync_w,
    );

    let mut dirty = false;
    let mut behind_any = false;
    for (i, r) in rows.iter().enumerate() {
        println!(
            "{:<name_w$}  {:<branch_w$}  {:<commit_w$}  {:<sync_w$}  {}",
            r.name,
            r.branch,
            r.commit,
            sync_strings[i],
            state_strings[i],
            name_w = name_w,
            branch_w = branch_w,
            commit_w = commit_w,
            sync_w = sync_w,
        );
        if r.error.is_none() && r.changes > 0 {
            dirty = true;
        }
        if r.behind > 0 {
            behind_any = true;
        }
    }

    if dirty || behind_any {
        println!();
        if dirty {
            println!("Some libraries have uncommitted changes.");
        }
        if behind_any {
            println!("Some libraries are behind their upstream (run 'raft libs status --fetch' to refresh, then update).");
        }
    }
}

/////////////////////////////////////////////////////////////////////////////////////////////////////////////////
// Pull (fast-forward) libraries in raftdevlibs
/////////////////////////////////////////////////////////////////////////////////////////////////////////////////

pub fn pull_raft_libs(cmd: &LibsPullCmd) -> Result<(), Box<dyn Error>> {
    let app_folder = cmd.app_folder.clone().unwrap_or(".".to_string());
    let dest_dir = resolve_dest_dir(&app_folder, &cmd.dest);

    if !dest_dir.is_dir() {
        return Err(format!("library folder not found: {}", dest_dir.display()).into());
    }

    let mut repos = list_git_repos(&dest_dir)?;
    repos.sort();

    if repos.is_empty() {
        println!("No git repositories found in {}", dest_dir.display());
        return Ok(());
    }

    println!("Pulling libraries in {}\n", dest_dir.display());

    let mut failures = Vec::new();
    for repo in &repos {
        let lib_path = dest_dir.join(repo);
        match pull_lib(&lib_path) {
            Ok(msg) => println!("  {:<16} {}", repo, msg),
            Err(err) => {
                println!("  {:<16} SKIPPED: {}", repo, err);
                failures.push(repo.clone());
            }
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

