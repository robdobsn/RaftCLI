# Plan: `raft libs` review and linked libraries in raftdevlibs

Date: 2026-10-15
Status: implemented 2026-10-15 (uncommitted, RaftCLI `main` and RaftCore `main`). See "Implementation status"
below; the rest of the document is the plan as agreed (decisions in section 7).

## Implementation status (2026-10-15)

Code: `src/app_libs.rs` (rewritten around the classifier), `src/app_build.rs`, `src/raft_cli_utils.rs`
(`raft.info`), `src/main.rs` (help text, readable build errors), `Cargo.toml` (`junction`, Windows only),
`raft_templates/CMakeLists.txt`, `README.md`; RaftCore `scripts/RaftBootstrap.cmake`.

Done, by plan section:

- 3.1 `scan_devlibs()` / `classify_devlib()`: clone, link, broken link, link to a non-repo, plain folder, other.
  A `.git` file (worktree, submodule) counts as a repo. Hidden entries are skipped.
- 3.2 `status` (now the default for bare `raft l`): TYPE (with link target, `~` for home), PINNED (version from
  `RAFT_COMPONENTS`, `unused` if the project doesn't use it), broken links flagged, "Fetched by the build" list,
  no error when `raftdevlibs` is missing, app folder validated, `--fetch` refreshes each distinct repo once.
- 3.3 `fetch`: linked libraries skipped, broken links and links to non-repos are errors, default list from
  `RAFT_COMPONENTS` (fixed four as fallback), a component given as a URL is cloned from that URL.
- 3.4 `pull`: linked libraries included and labelled, `--no-links`, each repo pulled once, broken links reported.
- 3.5 `link`: as planned. Windows falls back to a junction on error 1314 (via the `junction` crate); `--relative`
  is then not applied and the output says so.
- 3.6 `unlink` and `remove` (`rm`): as planned. `remove` uses the `remove_dir_all` crate (copes with read-only
  git objects on Windows) only after checking the entry is not a link.
- 3.7 Docker: option (a) - `raft build` mounts each link target over `/project/raftdevlibs/<Name>`. Prototyped
  first: works for absolute and relative symlinks on Linux Docker and for a junction on Docker Desktop for
  Windows; the links on the host are not changed. Before any build, a broken link for a library in
  `RAFT_COMPONENTS` is an error (a warning for an unused library). RaftCore `RaftBootstrap.cmake` and the
  template `CMakeLists.txt` stop with `FATAL_ERROR` on a broken link instead of fetching the remote copy.
  `check_docker_mount()` refuses a Docker build when a link would be mounted over a system folder in the
  container (`/opt` - where the ESP-IDF image keeps the toolchain - `/usr`, `/etc`, ...), over the project
  itself, or over a folder containing it. It works out the mount point the way Docker does: a relative link
  from `/project/raftdevlibs`, an absolute Unix link at its own path; Windows paths map under `/mnt/host`
  and are always allowed. The error suggests `--no-docker` or keeping the repo elsewhere.
- 6.2 (switching between local and remote): `raft.info` records the local libraries per SysType
  (`local_libs`); when they change, `raft build` deletes the build folder, as it already does for an ESP-IDF
  version change. Nothing is recorded by older versions, so the first build after upgrading never deletes.
- 3.8 README section rewritten ("Local development libraries"); `main.rs` help text; status hint names
  `raft l pull`.

Tested:

- `cargo test`: 79 pass on Windows (links made as junctions), 80 on Linux/WSL (adds the relative-link test). New
  tests cover the classifier, `RAFT_COMPONENTS` parsing, link/unlink/remove (including that targets survive and
  that local work blocks `remove`), the build check and `raft.info` `local_libs`.
- Linux (WSL): `raft l` on RoboticalAxiom1 (read only); on a scaffolded app with links to scratch clones -
  `link`, re-`link`, `status`, `fetch` (skipped), `pull`; three Docker builds (ESP-IDF 6.0.2): linked RaftCore
  and RaftSysMods mounted and used (CMake "LOCAL (DEBUG) LIBRARY ... /project/raftdevlibs/..."), build complete;
  after `unlink --libs RaftSysMods` the build folder was recreated and the build completed; with the RaftCore
  target moved the build was refused before Docker started (exit 1). CMake `IS_SYMLINK`/`EXISTS` check verified
  with `cmake -P`.
- Windows: `link --relative` (junctions, note shown), `status`, `remove` of a real clone (deleted) and of a link
  (refused), `unlink` (targets intact).
- Full builds with linked RaftCore and RaftSysMods, all completed with both used as local libraries:
  - Linux (WSL), local build without Docker (ESP-IDF 6.0.2 from `~/esp`), through the symlinks.
  - Docker Desktop for Windows, links made as junctions, both mounted. It took about 48 minutes against a few
    minutes for the same Docker build in WSL. That is Docker Desktop sharing a Windows folder (in
    `AppData\Local\Temp`, so also scanned by Defender) with its Linux VM, not the links: the link targets are
    mounted the same way as the project. A WSL local build also ran alongside it for the first part.
- RaftCore `FATAL_ERROR` in a real CMake run: with a copy of RaftCore including the change linked in, the
  RaftSysMods link broken and `idf.py reconfigure` run directly (bypassing the `raft build` check), CMake stopped
  with "raftdevlibs/RaftSysMods is a broken link ...".
- `check_docker_mount()` unit tests: mount points for relative, absolute and Windows targets; `/opt/...`,
  `../../../opt/esp`, `/usr/...`, `../RaftCore` (inside the project), `/project`, `..` and `/` refused.

Not tested: macOS (no Mac available).

## 1. Summary

`raft libs` (alias `raft l`) manages local checkouts of Raft libraries in `<app>/raftdevlibs`. The build
(RaftCore `RaftBootstrap.cmake`) uses `raftdevlibs/<Name>` instead of fetching that library. Developers also
point `raftdevlibs/<Name>` at a shared working repo with a symlink, so several projects build against one
checkout of RaftCore.

Findings:

- `raft l status` already lists linked libraries correctly. This was checked with the current source and with
  the installed 1.24.1 on WSL, using `RoboticalAxiom1/raftdevlibs/RaftCore -> /home/rob/rdev/raft/RaftCore`,
  and on Windows using a junction. What's missing is that the output doesn't say the library is a link or
  where it points. Broken links, plain folders and other entries are silently left out.
- **`raft l` / `raft l fetch` is unsafe for linked libraries.** It runs `git checkout <branch>` (default
  `main`) and a pull in the link *target*, which is the shared working repo. That switches a feature branch
  back to `main` in a repo used by other projects, with no warning, whenever the tree is clean.
- **Docker builds probably drop linked libraries without any error.** The container mounts only the
  project folder (`-v <project>:/project` in `src/app_build.rs`). A link to a folder outside the project
  points at nothing inside the container. `RaftBootstrap.cmake` tests `EXISTS raftdevlibs/<Name>`, gets
  false, and fetches the remote library, so the build succeeds with the wrong code. This follows from reading
  the code and has not been tested in Docker.
- There is no command to create or remove links. On Windows, creating a symlink needs Administrator rights or
  Developer Mode (confirmed on Rob's machine), but a directory junction doesn't, and the build and
  `raft l status` both handle junctions.
- Smaller CLI issues: the README describes the old interface (no subcommands; `raft libs --libs X` no
  longer parses); the bare `raft l` default is a hand-copied `LibsFetchCmd`; `status` and `pull` don't
  validate the app folder; `fetch` treats a git worktree (`.git` *file*) as "not a git repository"; the
  `main.rs` help says "(fetch, status)" and leaves out pull.

Proposed:

1. Classify every entry in `raftdevlibs` (clone, link, broken link, other) in one place and use that in
   every subcommand.
2. `raft l status` shows the entry type and link target, flags broken links, and shows which version of each
   `RAFT_COMPONENTS` library the build will use.
3. `fetch` skips linked libraries. `pull` still updates them (fast-forward only), marks them as linked and
   pulls each target only once.
4. New `raft l link [SOURCE_DIR]` and `raft l unlink`.
5. Docker builds handle linked libraries, or at the very least stop with an error instead of building the
   wrong code.

## 2. Current behaviour (as reviewed)

Code: `src/app_libs.rs` (641 lines), wired up in `src/main.rs` (`Action::Libs`, alias `l`).

| Command | What it does | Folder/link handling |
| -- | -- | -- |
| `raft l` (no subcommand) | `fetch` with hard-coded defaults (copy of `LibsFetchCmd`) | as fetch |
| `raft l fetch` / `f` | clone `RaftCore`, `RaftSysMods`, `RaftI2C`, `RaftWebServer` from `github.com/<account>`, or `git fetch` + checkout + `pull --ff-only` if present | `lib_path.join(".git").is_dir()` follows links, so a linked repo is **updated in place** |
| `raft l status` / `st` | table of branch, commit, ahead/behind, dirty count | `list_git_repos`: `is_dir()` and `.git` `exists()` follow links, so links are listed but not marked. Broken links and folders without `.git` are skipped silently |
| `raft l pull` / `p` | `pull --ff-only` on each clean repo with an upstream | follows links; a target shared by two links would be pulled twice |

Build side (RaftCore, not this repo):

- `RaftBootstrap.cmake:103-105`: `if(EXISTS raftdevlibs/<Name> AND IS_DIRECTORY ...)` uses the local copy,
  otherwise falls through to fetching. CMake's `EXISTS`/`IS_DIRECTORY` follow links, so local builds with
  links work, as Rob reported.
- The template `CMakeLists.txt` uses `raftdevlibs/RaftCore/scripts/RaftBootstrap.cmake` if it exists,
  otherwise it downloads RaftBootstrap. A broken RaftCore link falls back to the download in the same silent
  way.
- `RaftBootstrapPhase2.cmake:287` searches `raftdevlibs/RaftCore` for dev type JSON.

Other: `raft_templates/.gitignore` ignores `raftdevlibs`, so links are local to one machine and never
committed. `scripts/find_git_changes.py` uses `os.walk` without `followlinks`, so it reports a linked repo
at its real location and not inside each project. That's the right behaviour; leave it alone.

## 3. Design

### 3.1 One classifier for raftdevlibs entries

Add to `app_libs.rs`:

```rust
enum DevLibKind {
    Clone,                                  // real directory containing .git (dir or file)
    Link { target: PathBuf, resolved: PathBuf }, // symlink or junction to a git repo
    BrokenLink { target: PathBuf },         // link whose target is missing
    LinkNotRepo { target: PathBuf },        // link to a directory without .git
    NotRepo,                                // real directory without .git
    Other,                                  // file etc. (ignored by the build)
}
struct DevLibEntry { name: String, path: PathBuf, kind: DevLibKind }
fn scan_devlibs(dir: &Path) -> io::Result<Vec<DevLibEntry>>
```

- Use `fs::symlink_metadata` / `FileType::is_symlink()` to detect links without following them. On Windows
  `is_symlink()` is false for junctions, so also check the reparse-point attribute
  (`std::os::windows::fs::MetadataExt::file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT`) or use the
  `junction` crate's `junction::exists`. Read the target with `fs::read_link` (symlinks) or
  `junction::get_target` (junctions).
- Treat a repo as git if `.git` exists as a dir **or a file**, which covers worktrees and submodules. Fix
  `fetch_lib`, which currently requires a dir, to match.
- Skip hidden entries (`.` prefix) so backup or editor folders don't show up.

`status`, `pull`, `fetch`, `link` and `unlink` all use this classifier.

### 3.2 `raft l status`

Example output:

```
Local libraries in ./raftdevlibs (used instead of the version in features.cmake)

NAME           TYPE                              BRANCH  COMMIT   SYNC   STATE
RaftCore       link -> ~/rdev/raft/RaftCore      main    12fb875  +0/-0  clean
RaftSysMods    clone                             main    a1b2c3d  +0/-2  3 changed
RaftWebServer  BROKEN link -> ../RaftWebServer   -       -        -      target missing
notes          not a git repo                    -       -        -      -

Remote libraries (from RAFT_COMPONENTS):
RaftI2C@v1.3.2
```

- A TYPE column, with the link target shown as typed; long paths shortened with `~` for the home folder.
- Broken links, links to non-repos and plain folders are listed with a warning line, not skipped. A broken
  link is the case most worth seeing, because the build uses the remote copy without saying so.
- "Remote libraries" lists the `RAFT_COMPONENTS` entries (from `systypes/Common/features.cmake` and any
  `systypes/<SysType>/features.cmake`) with no local entry, so the output answers "what will this build
  use?". The parse is a simple regex like the one the template `CMakeLists.txt` already uses for
  `RaftCore@<tag>`. Optional: when a local clone's commit isn't the pinned tag, show the pin (for example
  `pinned v1.40.0`) to make mismatches visible.
- A missing `raftdevlibs` folder is not an error: print "No local libraries - the build fetches all
  libraries" and then the remote list.
- Validate the app folder with `check_app_folder_valid`, as `fetch` does.
- `--fetch` refreshes each *distinct resolved* repo once (see 3.4).

### 3.3 `raft l fetch` (and bare `raft l`)

- **Skip linked entries**: `RaftCore is linked to ~/rdev/raft/RaftCore - skipped (manage that repo
  directly)`. Linked repos are working repos; `fetch` must not switch their branch. Broken links and non-repo
  folders are reported as errors for that library, not cloned over.
- Default library list: today it is the fixed four. Proposal: when `--libs` isn't given, use the
  `RAFT_COMPONENTS` names from `features.cmake` (falling back to the fixed four if parsing finds nothing), so
  a project without RaftI2C doesn't get it cloned. Decision 7.3.
- Bare `raft l`: use `#[command(args_conflicts_with_subcommands = true)]` with a flattened
  `Option<LibsFetchCmd>` so `raft l --libs RaftCore` works again as the README describes, and remove the
  duplicated default struct. Whether bare `raft l` should keep defaulting to fetch is decision 7.1.

### 3.4 `raft l pull`

- Include linked libraries. `pull --ff-only` on a clean tree with an upstream can't lose work or switch
  branches, and keeping a shared working repo up to date is useful. Mark them in the output:
  `RaftCore (link)  already up to date`.
- Remove duplicates by canonical path, so a repo that is both cloned and linked, or linked under two names,
  is pulled once.
- Report broken links as skipped, with the target path.
- If a linked repo shouldn't be touched by default, add `--no-links`. Decision 7.4.

### 3.5 New: `raft l link [SOURCE_DIR]`

```
raft l link [SOURCE_DIR] [--libs <LIB>...] [--app <APP_FOLDER>] [--dest <DIR>] [--relative] [--force]
```

- `SOURCE_DIR` is the folder containing the library repos. Default `..`, relative to the **app folder** (not
  the current directory). For Rob's layout (`~/rdev/Robotical/RoboticalAxiom1`, libraries in
  `~/rdev/raft`) this would be `raft l link ../../raft`.
- The positional argument is the source folder, so the app folder is `--app` (default `.`). The other
  subcommands take the app folder as their positional argument. That's a small inconsistency, but it gives
  the shortest form for the common case (`raft l link`, `raft l link ../raft`). Alternative in decision 7.2.
- Which libraries:
  - `--libs` given: link exactly those; it's an error if `<SOURCE_DIR>/<Lib>` isn't a git repo.
  - Otherwise: link each `RAFT_COMPONENTS` library (fixed four as fallback) found as a git repo in
    `SOURCE_DIR`, and list the ones not found. If none are found, fail with the folder searched (this catches
    a wrong `SOURCE_DIR`).
  - Always check that the source is a git repo and has the expected name. Optionally do a cheap sanity check
    such as `CMakeLists.txt` existing.
- Link form: **absolute** by default. Reasons:
  - Windows junctions can only store absolute targets.
  - The Docker fix (3.7) needs the real path anyway.
  - The existing link in RoboticalAxiom1 is absolute.
  - Moving the project doesn't break it.

  `--relative` stores a relative link (Unix and Windows symlinks only). It's useful when the whole workspace
  moves together.
- Platform:
  - Unix: `std::os::unix::fs::symlink`.
  - Windows: try `std::os::windows::fs::symlink_dir`. On `ERROR_PRIVILEGE_NOT_HELD` (1314) fall back to a
    junction with `junction::create`, and say so in the output. Junctions work without Developer Mode and,
    as tested, with the build and `status`. Adds a dependency on the `junction` crate (small, widely used);
    the alternative is running `cmd /c mklink /J`.
- Existing entry at `raftdevlibs/<Lib>`:
  - Link to the same target: report "already linked", success, no change.
  - Link to a different target or a broken link: refuse unless `--force`, which replaces it. Only a link is
    removed, so this is safe.
  - Real directory (clone): **always refuse**, even with `--force`, and say the clone must first be removed
    with `raft l remove` (3.6) or by hand. Automatically deleting a clone that may hold unpushed work isn't
    worth the risk.
- Create `raftdevlibs` if missing (as `fetch` does).
- Print the result as a short table (lib, target, symlink/junction) and remind the user that a build folder
  configured with the remote library must be reconfigured. In practice the next `raft b` re-runs CMake;
  check whether the existing build needs `--clean` when switching between remote and local (see 6.2).

### 3.6 New: `raft l unlink` (and optionally `raft l remove`)

```
raft l unlink [--libs <LIB>...] [--app <APP_FOLDER>] [--dest <DIR>]
```

- Removes links only (all links when `--libs` is omitted, including broken ones). Never follows a link:
  - Unix: `fs::remove_file(link)`.
  - Windows: `fs::remove_dir(link)` removes a directory symlink or junction without touching the target.
  - Never `remove_dir_all` on a link path.
- A named library that is a real directory: refuse with "is a clone, not a link".
- Optional `raft l remove --libs <LIB>...` deletes clones, refusing if there are uncommitted changes, stashes
  or commits not on any remote (`git status --porcelain`, `git stash list`,
  `git log --branches --not --remotes`), unless `--force`. Covers "switch this project from a clone to a
  link". It could be left out; deleting the folder by hand works. Decision 7.5.

### 3.7 Docker builds

Problem (2nd point in section 1): links to folders outside the project don't resolve inside the container, and
the bootstrap fetches the remote library without saying so.

Steps, in order of value:

1. **RaftCLI warning (cheap, do first).** Before a Docker build, `scan_devlibs`. For each link whose resolved
   target is outside the project folder, either mount it (step 2) or print a clear warning, or stop
   with an error, saying that the library will come from the remote. Same for broken links on local builds.
2. **Mount linked libraries.** For each link, add `-v <canonical target>:<container path>` to the `docker run`
   arguments in `src/app_build.rs` (both the unix and non-unix argument lists), using `convert_path_for_docker`
   for the host path. Choosing the container path needs a prototype:
   - (a) Mount at `/project/raftdevlibs/<Name>`, on top of the link. Docker/runc resolves symlinks in mount
     destinations inside the container root, so for an absolute link this probably mounts at the link's
     target path and the link then resolves. It may behave differently for relative links and Windows
     junctions. Needs testing on Linux and Docker Desktop for Windows.
   - (b) Mount at `/raftdevlinks/<Name>` and pass `RAFT_DEVLIB_<NAME>=/raftdevlinks/<Name>` (or one
     `RAFT_DEVLIB_PATHS` list) with `-e`. RaftBootstrap checks the variable before `raftdevlibs/<Name>`. This
     needs a RaftCore change but is unambiguous on every platform. The template `CMakeLists.txt` needs the
     same check for RaftCore's `RaftBootstrap.cmake`.

   Recommendation: prototype (a). If it fails on any platform, implement (b).
3. **RaftCore: stop on broken links.** In `RaftBootstrap.cmake`, if `IS_SYMLINK raftdevlibs/<Name>` and
   `NOT EXISTS`, call `message(FATAL_ERROR ...)` (or at least a loud `WARNING`) instead of falling back to the
   fetch. This also catches broken links outside Docker, for example after renaming the source repo. Junctions
   may not report as `IS_SYMLINK` in CMake on Windows; the RaftCLI check in step 1 covers those.

### 3.8 Housekeeping

- README: rewrite the "Fetching local development libraries" section with the subcommands (`fetch`, `status`,
  `pull`, `link`, `unlink`), the clone-vs-link workflows, the Windows junction note and the Docker
  behaviour.
- `main.rs`: help text "Manage local Raft development libraries (fetch, status, pull, link)".
- Validate the app folder in `status` and `pull` the same way as in `fetch`.
- The final hint in `print_status_table` says "then update"; change it to name `raft l pull`.

## 4. Implementation phases

1. **Classifier and status** (3.1, 3.2, housekeeping). No behaviour change beyond output. Low risk.
2. **Safety fixes** (3.3 skip links in fetch, worktree `.git`, bare `raft l` options; 3.4 pull changes;
   3.7 step 1 Docker warning).
3. **`link` / `unlink`** (3.5, 3.6), including the `junction` crate.
4. **Docker mounting** (3.7 step 2 prototype, then (a) or (b)) and the **RaftCore fatal error on a broken link**
   (3.7 step 3, separate repo and release).
5. Optional `remove` and `RAFT_COMPONENTS`-based defaults if not taken up in phase 2.

## 5. Tests

Unit tests (`cargo test`, in `app_libs.rs`, using a temp folder):

- `scan_devlibs` with a clone (`git init`), a worktree-style `.git` file, a plain folder, a file, a symlink to a
  repo, a symlink to a missing path, a symlink to a non-repo. On Windows, create links as junctions (no
  privilege needed) and, when `symlink_dir` returns 1314, skip the symlink cases with a message, not a
  failure. CI on Windows may not have Developer Mode.
- `link`: links the requested libraries; same target is a no-op; different target refused without `--force`
  and replaced with it; clone never replaced; missing source repo errors; no libraries found errors.
- `unlink`: removes links (including broken ones) and **leaves the target untouched** (assert a file inside
  the target still exists); refuses a clone.
- Duplicate detection for `pull`/`status --fetch` by canonical path.
- `RAFT_COMPONENTS` parsing from sample `features.cmake` text (Common plus a SysType, `@tag` and `#tag`,
  commented-out lines ignored).

Manual:

- WSL/Linux: `RoboticalAxiom1` (existing absolute link): `status`, `pull`, `fetch` skips it; `link`/`unlink` with
  `../../raft`; local `raft b` uses the linked RaftCore ("LOCAL (DEBUG) LIBRARY" banner).
- Docker on WSL and Docker Desktop on Windows: build with a linked RaftCore and check the banner shows the
  mounted path; before the fix, confirm the silent fallback described in section 1.
- Windows without Developer Mode: `link` falls back to a junction; build and `status` work; `unlink` leaves the
  target repo intact.

## 6. Risks and notes

1. **Deleting through a link.** The worst bug would be removing the target repo's contents. Never use
   `remove_dir_all` on any path in `raftdevlibs` that may be a link; classify with `symlink_metadata` first;
   the unlink tests check the target survives.
2. **Switching a library from remote to local (or back) in an existing build folder.** CMake caches
   FetchContent state. Check whether `RaftBootstrap` notices the change on the next configure, or whether
   `link`/`unlink` should tell the user to run `raft b --clean`. Test this as part of phase 3.
3. **Shared working repo.** Linking means every linked project builds whatever branch is checked out in the
   shared repo. That's the intent, but `status` should make it obvious; the TYPE column does that.
4. **macOS.** Same code paths as Linux; not available to test here.

## 7. Decisions (agreed 2026-10-15)

1. Bare `raft l` changes from `fetch` to `status`; `raft l f` still fetches. The README and help text must say so.
2. `raft l link` takes a positional `SOURCE_DIR` with the app folder as `--app`, as proposed in 3.5.
3. `fetch` defaults to the libraries in `RAFT_COMPONENTS`, falling back to the fixed four.
4. `pull` includes linked libraries by default (`--no-links` to exclude them), as in 3.4.
5. Add `raft l remove` for clones, with the safety checks in 3.6.
6. Mounting folders outside the project into the Docker build container is acceptable (3.7 step 2).
