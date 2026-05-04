#!/usr/bin/env python3

"""Find Git repositories with local changes under a folder tree."""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Optional


@dataclass(frozen=True)
class RepoStatus:
    path: Path
    has_changes: bool
    error: Optional[str] = None


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="List Git repository folders with local changes under a folder tree."
    )
    parser.add_argument(
        "root",
        nargs="?",
        default=".",
        help="Folder tree to scan. Defaults to the current working directory.",
    )
    parser.add_argument(
        "--absolute",
        action="store_true",
        help="Print absolute paths instead of paths relative to the scan root.",
    )
    parser.add_argument(
        "--fail-on-changes",
        action="store_true",
        help="Exit with status 1 when changed repositories are found.",
    )
    parser.add_argument(
        "--show-errors",
        action="store_true",
        help="Print Git errors for folders that look like repos but cannot be checked.",
    )
    return parser.parse_args()


def is_git_repo(folder: Path) -> bool:
    return (folder / ".git").exists()


def iter_git_repos(root: Path):
    for current_folder, dir_names, _file_names in os.walk(root):
        if ".git" in dir_names:
            dir_names.remove(".git")

        current_path = Path(current_folder)
        if is_git_repo(current_path):
            yield current_path


def get_repo_status(repo_path: Path) -> RepoStatus:
    try:
        result = subprocess.run(
            [
                "git",
                "-C",
                str(repo_path),
                "status",
                "--porcelain=v1",
                "--untracked-files=normal",
            ],
            check=False,
            capture_output=True,
            text=True,
        )
    except OSError as exc:
        return RepoStatus(repo_path, False, str(exc))

    if result.returncode != 0:
        error = result.stderr.strip() or f"git status failed with code {result.returncode}"
        return RepoStatus(repo_path, False, error)

    return RepoStatus(repo_path, bool(result.stdout.strip()))


def display_path(repo_path: Path, root: Path, absolute: bool) -> str:
    if absolute:
        return str(repo_path.resolve())

    try:
        relative_path = repo_path.relative_to(root)
    except ValueError:
        relative_path = repo_path

    return str(relative_path) if str(relative_path) else "."


def main() -> int:
    args = parse_args()
    root = Path(args.root).expanduser().resolve()

    if not root.is_dir():
        print(f"Scan root is not a folder: {root}", file=sys.stderr)
        return 2

    changed_repos = []
    had_errors = False

    for repo_path in iter_git_repos(root):
        status = get_repo_status(repo_path)
        if status.error:
            had_errors = True
            if args.show_errors:
                print(f"{display_path(status.path, root, args.absolute)}: {status.error}", file=sys.stderr)
        elif status.has_changes:
            changed_repos.append(status.path)

    for repo_path in changed_repos:
        print(display_path(repo_path, root, args.absolute))

    if had_errors and args.show_errors:
        return 2
    if changed_repos and args.fail_on_changes:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
