// RaftCLI: Local Raft library management
// Rob Dobson 2024

use clap::Parser;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::raft_cli_utils::check_app_folder_valid;

const DEFAULT_DEST_FOLDER: &str = "raftdevlibs";
const DEFAULT_LIBS: [&str; 4] = ["RaftCore", "RaftSysMods", "RaftI2C", "RaftWebServer"];

#[derive(Clone, Parser, Debug)]
pub struct LibsCmd {
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

pub fn fetch_raft_libs(cmd: &LibsCmd) -> Result<(), Box<dyn Error>> {
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
