# Plan: EIM support and per-SysType ESP-IDF version selection

Date: 2026-09-19
Status: implemented on 2026-09-19 (uncommitted, branch `eim-and-systype-idf-version` in RaftCLI and
`esp-idf-version-check` in RaftCore). See "Implementation status" immediately below; the rest of the
document is the plan as agreed and is otherwise unchanged.

## Implementation status (2026-09-19)

Code: `src/esp_idf.rs` (new: versions, required version, Dockerfile handling, EIM manifest, locator,
environment capture, all with unit tests), `src/app_build.rs`, `src/raft_cli_utils.rs` (`raft.info`),
`src/main.rs` and `src/app_config.rs` (`raft new --defaults/--set`), the scaffold templates, the README,
`.github/workflows/tests.yml`; RaftCore `scripts/RaftBootstrapPhase2.cmake` and `scripts/RaftProject.cmake` (fatal
version check) and `unit_tests` moved to the new scheme.

Tested:

- `cargo test`: 64 tests pass on Windows and on Linux (WSL).
- Linux (WSL, legacy 6.0.1/6.0.2 in `~/esp` plus EIM v6.1): full local build of a scaffolded app with the
  version from `Common/features.cmake` (legacy install, `idf.py` run via python, CMake check reports a match);
  SysType override to 6.1 deletes the build folder, selects the EIM install from the manifest, captures its
  environment with `-e` and builds under 6.1 (see "Found" below for how that build ended); `-e` as EIM name,
  EIM path and legacy path; saved `-e` reuse; `--idf-version`; legacy-activated and EIM-activated shells
  (the latter used to panic); wrong-version shell active; version not installed; bad `-e`; `--eim-json` with
  an install marked failed; running `idf.py` directly with the wrong ESP-IDF gives the fatal CMake error.
- Docker (WSL): placeholder Dockerfile built with `--build-arg`, image `raftbuilder:idf-6.0.2`, full project
  build; literal Dockerfile with a different required version (`RaftCore/unit_tests`, 6.0.1 vs 6.0.2) uses a
  generated copy differing only in the tag, project Dockerfile untouched.
- Windows: scaffold with `--defaults`, the generated-Dockerfile Docker path, and the "nothing installed" message.

Not tested: **EIM on Windows and macOS** (no EIM install on Rob's Windows machine; no Mac). The Windows
`-e` capture uses `cmd /C Microsoft.<name>_profile.bat -e` and is covered only by unit tests of the parsing.
The GitHub workflow has not been run.

Differences from the plan below:

- Resolution order is `-e`, then a matching active environment, then the saved `-e` path, then the search
  (4.2 lists the saved path second). This keeps today's behaviour where a matching active shell wins.
- `-e` now implies a local build. Previously `-e` without `-i`/`--no-docker` was silently ignored when Docker
  was available.
- When a different ESP-IDF is active in the shell its variables (`IDF_PATH`, `IDF_PYTHON_ENV_PATH`, ...) are
  removed before capturing, and when running, the environment of the one chosen. `IDF_TOOLS_PATH` is only
  removed when it is EIM's tools folder, so a user's own setting still reaches `export.sh`.
- An install the manifest marks as failed/broken is not resurrected by the folder scan (4.2 step 4c).
- The generated Dockerfile is `build/raft_docker/<SysType>/Dockerfile`, not inside `build/<SysType>`, so it
  is not deleted by a clean and does not leave a non-CMake file in the ESP-IDF build folder.
- `raft.info` gains `idf_versions` (SysType -> version) instead of `last_idf_kind`/`last_idf_name`/`last_idf_version`;
  the kind is recovered from the path via the manifest.
- Not implemented: the `eim run` last-resort fallback (4.3); the nightly CI job that installs EIM and builds
  firmware (5.3) - only the `cargo test` matrix workflow was added.
- 2026-09-20: per-SysType component manager lock file done in RaftCore stage 2 (`RaftBootstrapPhase2.cmake` and
  `RaftProject.cmake`, after the ESP-IDF `project.cmake` include, using the `DEPENDENCIES_LOCK` build property).
  A SysType uses `systypes/<SysType>/dependencies.lock` if the project sets `ESP_IDF_VERSION` or if that file
  already exists; otherwise the root `dependencies.lock` is used exactly as before, so existing projects are
  not affected (this was the concern that had held it back). Per SysType rather than per version because the
  lock also records the target chip. `managed_components/` cannot be relocated and stays shared. Tested with
  two SysTypes on 6.0.2 and 6.0.1: each lock file is created once and is byte-identical after switching back
  and forth; a project without `ESP_IDF_VERSION` still writes the root file; moving that file into the SysType
  folder opts it in. `RaftCore/unit_tests` lock file moved to `unit_tests/systypes/unittest/`.
- 2026-09-20: the CMake version check was moved out of `RaftBootstrap.cmake` (stage 1, which is now back to
  exactly the v1.54.1 release asset) into `RaftBootstrapPhase2.cmake`. Stage 1 is downloaded from a release,
  cached and, in older projects, pinned to an old release, so changes there reach projects late or never;
  stage 2 always matches the RaftCore in use. Verified with the oldest stage 1 still in use (v1.37.1, via
  `RaftCore/unit_tests`) and the current one: mismatch is fatal, match passes. The rule "stage 1 is frozen" and a
  `RAFT_BOOTSTRAP_API` compatibility check were added to stage 2 - see RaftCore
  `devdocs/bootstrap-version-alignment-plan.md` section 3c.
- 2026-09-20: `raft build` warns when a project has a bootstrap release URL written into its `CMakeLists.txt`
  and its RaftCore floats (`src/bootstrap_check.rs`, unit tested, and tried on copies of two real projects);
  `RaftCore/unit_tests` now includes the local `../scripts/RaftBootstrap.cmake` instead of downloading v1.37.1
  (full build verified). Details in RaftCore `devdocs/bootstrap-version-alignment-plan.md` section 3d.
  `cargo test`: 67 pass.
- Also fixed: the `cmd_history` unit test, which had been failing on all platforms (its final expectation was
  wrong: moving down past the newest entry gives an empty line).

Found while testing: **the Raft libraries do not compile with ESP-IDF 6.1.** `uart_config_t` gained
`rx_glitch_filt_thresh`, which breaks the designated initialisers (`-Werror=missing-field-initializers`) in
RaftCore `BusSerial.cpp:118`, RaftSysMods `SerialConsole.cpp:219, 279` and `CommandSerialPort.cpp:78`. The
EIM build above got through configuration and about 1600 compile steps before stopping there; there may be
further 6.1 issues behind it. Not fixed (library work, separate from RaftCLI).

Two pieces of work, planned together because they meet in the same code (deciding which ESP-IDF version
a build needs, then finding or containerising it):

- **A. EIM**: build with an ESP-IDF installed by the Espressif Installation Manager (sections 1-4).
- **B. Per-SysType version**: let a SysType say which ESP-IDF version it needs, for local and Docker
  builds, falling back to the Dockerfile as today (section 4.8).

## 1. Summary

RaftCLI cannot use an ESP-IDF installed by EIM today, for three separate reasons, all confirmed on a
real EIM install (EIM 0.19.0, ESP-IDF v6.1, WSL Ubuntu):

1. **It looks in the wrong places.** `-i` searches `~/esp` (Linux/macOS) and `C:\Espressif\frameworks`
   (Windows) for a folder whose name ends with the version. EIM installs to `~/.espressif/<name>/esp-idf`
   and `C:\esp\<name>\esp-idf`, and `<name>` is whatever the user chose (`v6.1`, or anything after
   `eim rename`).
2. **`export.sh` / `export.bat` do not work in an EIM install.** EIM puts the Python environment in
   `<tools>/python/<name>/venv`, which the export scripts do not know about. `export.sh` prints
   "Python virtual environment ... not found" and **still exits 0**, so RaftCLI's
   `source export.sh && env` "succeeds" with a useless environment and the build then fails with a
   misleading "idf.py not found".
3. **`idf.py` is not on the PATH in an EIM environment.** EIM activation defines `idf.py` as a shell
   function (bash/zsh), an alias (PowerShell) or a doskey macro (cmd). None of these exist for a child
   process, so `Command::new("idf.py")` fails even inside a correctly activated EIM shell. In
   `idf_version_ok` this is an `.expect()`, so RaftCLI panics.

The fix is to discover EIM installs from EIM's own manifest, get the environment from EIM's activation
script, and always run `idf.py` as `<python> <IDF_PATH>/tools/idf.py`. All existing mechanisms stay, in
the same priority order.

## 2. How EIM works (facts the design relies on)

From the EIM source (`espressif/idf-im-ui`), its docs, and the real install. EIM is the same on
Windows, Linux and macOS apart from the paths.

| Item | Windows | Linux / macOS |
|---|---|---|
| Default install root | `C:\esp` | `~/.espressif` |
| ESP-IDF folder | `C:\esp\<name>\esp-idf` | `~/.espressif/<name>/esp-idf` |
| Tools folder | `C:\Espressif\tools` | `~/.espressif/tools` |
| Python venv | `<tools>\python\<name>\venv` | `<tools>/python/<name>/venv` |
| Manifest | `C:\Espressif\tools\eim_idf.json` | `~/.espressif/tools/eim_idf.json` |
| Activation script | `<tools>\Microsoft.<name>.PowerShell_profile.ps1` and `<tools>\Microsoft.<name>_profile.bat` | `<tools>/activate_idf_<name>.sh` (also `.fish`) |

All of these locations can be changed at install time (`-p`, `esp_idf_json_path`,
`activation_script_path_override`), so the manifest is the only reliable source; the defaults are just
where to look for the manifest first.

Manifest (`eim_idf.json`, schema version "3.0" seen):

```json
{
  "gitPath": "/usr/bin/git",
  "idfInstalled": [
    {
      "activationScript": "/home/rob/.espressif/tools/activate_idf_v6.1.sh",
      "id": "esp-idf-6e7e734d...",
      "idfToolsPath": "/home/rob/.espressif/tools",
      "name": "v6.1",
      "path": "/home/rob/.espressif/v6.1/esp-idf",
      "python": "/home/rob/.espressif/tools/python/v6.1/venv/bin/python",
      "installationConfig": "<base64>",
      "status": "finished"
    }
  ],
  "idfSelectedId": "esp-idf-6e7e734d...",
  "eimPath": "/usr/bin/eim",
  "version": "3.0"
}
```

- `status` is one of `in_progress`, `failed`, `finished`, `being_repaired`, `broken`; it may be absent in
  older manifests (treat as finished). `activationScript` and `python` are optional.
- On Windows `activationScript` is the PowerShell profile; the `.bat` profile sits beside it with the
  name `Microsoft.<name>_profile.bat`.
- `name` is a label, not a version. The real version is in `<path>/tools/cmake/version.cmake`
  (`IDF_VERSION_MAJOR/MINOR/PATCH`), which exists in every ESP-IDF however it was installed.
- **Every activation script (sh, ps1, bat) accepts `-e`**, which prints the environment as `KEY=VALUE`
  lines and changes nothing: `PATH=<additions only>`, `SYSTEM_PATH=...`, `ESP_IDF_VERSION`,
  `IDF_VERSION`, `IDF_TOOLS_PATH`, `IDF_PATH`, `IDF_PYTHON_ENV_PATH`, `ESP_ROM_ELF_DIR`,
  `OPENOCD_SCRIPTS`, `IDF_COMPONENT_LOCAL_STORAGE_URL`, ... This is a machine-readable interface and
  avoids parsing a whole shell environment.
- `eim run "<command>" [name]` runs a command inside an installation's environment; `eim select`,
  `eim list` exist; `eim discover` is not implemented.

## 3. Current RaftCLI behaviour (what must keep working)

Build method: `--docker` / `--no-docker` / `RAFT_FORCE_DOCKER` / `RAFT_NO_DOCKER`, else the method saved
in `build/<systype>/raft.info`, else local if Docker is unavailable. The choice of build method is not changed by this work. Docker
builds are untouched by the EIM work (A) and change only as described in 4.8 (B), and then only when a
SysType asks for a version different from the Dockerfile's.

For a local build the ESP-IDF is chosen in this order (`app_build.rs`, `raft_cli_utils.rs`):

1. `-e <path>`: an ESP-IDF folder (contains `export.sh`), or a folder containing `*<version>` subfolders.
2. The explicit path saved in `raft.info` from a previous `-e` build.
3. `IDF_PATH` from the environment: if `idf.py --version` matches the required version the active
   environment is used as-is.
4. Otherwise search: the path from 1-3 if any, then `~/esp` or `C:\Espressif\frameworks`, for a folder
   name ending with the required version; then `source export.sh && env` (or `export.bat && set`) and run
   `idf.py` with that environment.

The required version is the tag in the project's `Dockerfile` (`FROM espressif/idf:v6.0.2`), falling back
to `default_esp_idf_version()`. Section 4.8 adds per-SysType sources ahead of these two; wherever this
document says "required version" it means the result of the precedence list in 4.8.

## 4. Design

### 4.1 One resolver, two kinds of installation

Introduce a small module (`src/esp_idf_locator.rs`) that returns:

```rust
struct IdfInstall {
    idf_path: PathBuf,
    version: IdfVersion,            // from tools/cmake/version.cmake
    kind: IdfKind,                  // Legacy (export script) | Eim { name, id, activation_script, python, tools_path }
    origin: &'static str,           // "explicit", "saved", "active-env", "search:~/esp", "eim-manifest", ...
}
```

All directory roots, the manifest path and the environment are passed in (not read from globals) so the
whole resolver can be unit tested on any OS with temporary directories.

### 4.2 Resolution order (existing order preserved, EIM added)

1. `-e <path>` (unchanged meaning). New: the path may also be an EIM *name* (`-e v6.1`) if it is not an
   existing directory. If the resolved folder is listed in the manifest it is treated as `Eim`.
2. Saved explicit path from `raft.info` (unchanged; same manifest lookup).
3. Active environment (`IDF_PATH` set and version matches). Works for both kinds once 4.4 is done.
4. `-i` search, in this order, first exact version match wins:
   a. legacy roots as today (`~/esp`, `C:\Espressif\frameworks`) - unchanged so existing users see no
      difference;
   b. EIM manifest entries with a usable status, matched on the real version; if several match prefer
      the one equal to `idfSelectedId`;
   c. EIM default roots scanned directly (`~/.espressif/*/esp-idf`, `C:\esp\*\esp-idf`) for the case
      where the manifest is missing or unreadable, guessing the activation script name from the folder
      name.
5. Docker, as today.

Manifest location: `--eim-json <file>` option, then `RAFT_EIM_IDF_JSON` env var, then the platform
default. (Needed because EIM lets the user move the manifest.)

Version matching: compare numerically on major.minor.patch with a missing patch equal to 0, so a
Dockerfile tag `v6.1` matches an IDF reporting 6.1.0 (today's string compare would not). Matching a
folder-name suffix stays for legacy roots but is confirmed against `version.cmake` when present.

If nothing matches, the error lists every installation found (kind, name, version, path) and suggests
`eim install -i v<required>` - today it only prints "No matching ESP-IDF found".

### 4.3 Getting the environment

- `Legacy`: `export.sh` / `export.bat` as today, plus **validation**: the captured environment must
  contain `IDF_PATH` and an existing `IDF_PYTHON_ENV_PATH`. If not, and the folder is an EIM install, fall
  through to the EIM method; otherwise fail with the export script's own output. This fixes the
  exit-code-0 trap in section 1.
- `Eim`, primary: run the activation script with `-e`, parse `KEY=VALUE`, and build the child
  environment as `PATH = <printed PATH> + separator + current PATH` plus the other variables.
  - Linux/macOS: `sh <activate_idf_<name>.sh> -e`.
  - Windows: `cmd /C "<Microsoft.<name>_profile.bat>" -e` (no PowerShell execution-policy problems);
    if the `.bat` is missing, `powershell -NoProfile -ExecutionPolicy Bypass -File <ps1> -e`.
- `Eim`, fallback (older EIM without `-e`, or empty output): source the script and dump the environment,
  as is done for `export.sh`.
- `Eim`, last resort: `eim run "<idf.py command>" <name>` using `eimPath` from the manifest. Kept last
  because of quoting and because it depends on the `eim` binary being present.

### 4.4 Running idf.py

Never rely on `idf.py` being on the PATH. Build the command as:

- `python`: manifest `python` if `Eim`; else `$IDF_PYTHON_ENV_PATH/bin/python` (`Scripts\python.exe` on
  Windows); else `python` from the captured PATH;
- args: `<IDF_PATH>/tools/idf.py -B <build_dir> ...`.

This is valid for legacy installs too (it is what the `idf.py` shims do), so there is a single code path.
`idf_version_ok` stops shelling out: it reads `version.cmake` under `IDF_PATH` (no panic, faster).

### 4.5 Persistence and output

`raft.info` gains `last_idf_kind` ("legacy"/"eim") and `last_idf_name`; old files without them still
load. Replace the `// TODO remove` debug prints with one line stating which ESP-IDF was chosen and why
(`Using ESP-IDF 6.0.2 [EIM "v6.0.2"] at ... (required by systypes/unittest/features.cmake)`).

### 4.6 Docs and help

README "Build using ESP IDF": add EIM as the recommended install route with the three ways to use it
(`raft build -i`, an EIM-activated shell, `-e <name or path>`), and the new option/env var. `-e` help
text: "Path to ESP IDF folder, or name of an EIM installation". `-i` help text changes from "matching
Dockerfile version" to "matching the required version (see ESP_IDF_VERSION in features.cmake)".

### 4.7 Supporting change worth doing first: non-interactive `raft new`

`raft new` needs a terminal, which blocks automated end-to-end tests (the September scaffold test had to
drive it through a pseudo-terminal). Add `raft new --defaults [--set key=value ...]`. This makes the CI
matrix in section 5.3 possible and is useful in its own right.

### 4.8 Per-SysType ESP-IDF version (work item B)

#### Today

The required version comes from one place: the `FROM espressif/idf:v<version>` line of the project's
`Dockerfile` (else `default_esp_idf_version()`). It is per project, so every SysType in a project must
use the same ESP-IDF. A Docker build runs `docker build -t raftbuilder .` with that Dockerfile and then
`idf.py` inside the image. The image tag `raftbuilder` is shared by every project and version.

#### Where the setting lives: `features.cmake` (agreed 2026-09-19)

```cmake
# systypes/unittest/features.cmake
set(IDF_TARGET "esp32s3")
set(ESP_IDF_VERSION "6.0.2")
```

Reasons for choosing this over the alternatives:

| Option | Verdict |
|---|---|
| `systypes/<SysType>/features.cmake` (`set(ESP_IDF_VERSION ...)`) | **Chosen.** It is already the per-SysType *build* configuration: it holds `IDF_TARGET` and the library versions (`RaftCore@...`), which are the settings that go hand in hand with the ESP-IDF version. SysType files already include `Common/features.cmake`, so a project-wide default with per-SysType overrides comes for free. CMake can see the value too (see "CMake check" below). No new file. |
| `SysTypes.json` | No. It is runtime configuration that ends up in the firmware, and it is merged by a script at build time. |
| `sdkconfig.defaults` | No. Kconfig has no such key; it would have to be a magic comment. |
| New file (`systypes/<SysType>/idf_version` or `raft.json`) | Trivial to parse but one more file per SysType for one value. Worth reconsidering only if more RaftCLI-only per-SysType settings appear (default serial port, build method ...). |

RaftCLI does not run CMake to read it. It scans the file for an uncommented
`set(ESP_IDF_VERSION "<v>")` (quotes and a leading `v` optional, one line, literal value - documented as
such). Lookup order: `systypes/<SysType>/features.cmake`, then `systypes/Common/features.cmake`. If the
value is not a literal (contains `${`), RaftCLI stops with an error rather than guessing.

#### Precedence for the required version

1. `--idf-version <v>` on the command line (new, optional; useful for trying a version and for tests).
2. `ESP_IDF_VERSION` in the SysType's `features.cmake`.
3. `ESP_IDF_VERSION` in `systypes/Common/features.cmake`.
4. A literal version in the project `Dockerfile` (unchanged; this is what existing projects have). A
   Dockerfile that uses the placeholder form below contributes nothing here.
5. `default_esp_idf_version()` (unchanged), now with a warning that no version is set anywhere.

An existing project that sets nothing in `features.cmake` behaves exactly as it does today. The chosen version and where it came from
are printed once (`Required ESP-IDF 6.0.2 (from systypes/unittest/features.cmake)`).

The version is kept in two forms: **as written** for Docker (the image tag is `v6.1`, there is no
`v6.1.0` image) and **numeric major.minor.patch** for matching local installs (4.2).

#### Local builds

Nothing else changes: the required version feeds the resolver in 4.2, so a SysType version works with
legacy installs, EIM installs and an already-active shell alike. If the active shell's ESP-IDF is the
wrong version for this SysType, RaftCLI searches for the right one, as it does now.

#### Docker builds

A version written in the Dockerfile is misleading once a SysType can ask for something else, and
rewriting it would not help (two SysTypes can disagree). Agreed 2026-09-19: the Dockerfile stops stating a
version. The Dockerfile was never meant to be built by hand, so it is acceptable that it no longer can be.

**Placeholder form (new projects, and the recommended edit for existing ones):**

```dockerfile
# The ESP-IDF version is NOT set here. It comes from ESP_IDF_VERSION in systypes/<SysType>/features.cmake
# (or systypes/Common/features.cmake) and is supplied by "raft build". This file cannot be built directly.
ARG ESP_IDF_VERSION
FROM espressif/idf:${ESP_IDF_VERSION}
```

This is the placeholder idea expressed in Docker's own syntax rather than a custom token such as
`<ESP-IDF-VERSION>`: it is a valid Dockerfile for editors and linters, RaftCLI supplies the value with
`docker build --build-arg ESP_IDF_VERSION=v<version>` so no file has to be generated or edited, and a
direct `docker build .` fails at once (checked on Docker 29.8: "failed to parse stage name ... invalid
reference format") instead of quietly building with the wrong ESP-IDF.

**What RaftCLI does, by Dockerfile form:**

| Dockerfile | Required version | Action |
|---|---|---|
| Placeholder (`ARG ESP_IDF_VERSION`, no default) | from `features.cmake` / `--idf-version` / default | `--build-arg`; project Dockerfile used as-is |
| Literal `FROM espressif/idf:v<x>` | same as `<x>`, or nothing overrides it | exactly as today |
| Literal `FROM espressif/idf:v<x>` | different from `<x>` | **Do not modify the user's Dockerfile.** Generate `build/<SysType>/raft_docker/Dockerfile`, a copy with only the image tag replaced, and build with `docker build -f <that file> -t <tag> .` (the context is still the project folder, so `COPY`/`ADD` keep working; the file stays on disk for inspection). Print a one-line note recommending the placeholder form. |
| No `espressif/idf` base image (custom image) | set by SysType | Error: the SysType asks for a version this Dockerfile cannot provide. Never build with the wrong ESP-IDF. |

- The literal rewrite touches only the first `FROM [--platform=...] espressif/idf:<tag> [AS name]`;
  everything else, byte for byte, is preserved.
- Image tag becomes `raftbuilder:idf-<version>` whenever a version is known, so switching between
  SysTypes does not rebuild the image each time. (Images are several GB each; mention `docker image prune`
  in the README.)
- `compose.yaml` in the template runs `./build.sh`, which the scaffold does not generate, and cannot know
  the SysType's version. Proposal: remove it from the template (RaftCLI does not use it). Existing projects'
  copies are left alone.

#### Changing version for an existing build folder

`build/<SysType>` configured by one ESP-IDF cannot be reused by another (CMake cache, toolchain and
Python paths). `raft.info` will record `last_idf_version`; when the required version differs from it,
RaftCLI deletes the SysType's build folder before building and says so. This also covers a user editing
the Dockerfile version, which today fails with an obscure CMake error.

#### CMake check (RaftCore): mismatch is a fatal error (agreed 2026-09-19)

Because the value is in `features.cmake`, `RaftBootstrapPhase2.cmake` (stage 2 - see the note below) compares `ESP_IDF_VERSION` with the
ESP-IDF actually running (`$ENV{IDF_PATH}/tools/cmake/version.cmake`) and stops with `FATAL_ERROR` on a
mismatch, naming both versions and telling the user to build with `raft build`. CMake is not meant to be
run on a Raft project directly, so failing hard is right; it also catches any RaftCLI bug that picks the
wrong install.

- Comparison is numeric with a missing patch equal to 0 (`"6.1"` matches 6.1.0), the same rule as RaftCLI.
- No `ESP_IDF_VERSION` set (existing projects): no check, as today.
- `--idf-version` must not trip the check, so RaftCLI passes the version it resolved to CMake
  (environment variable `RAFT_ESP_IDF_VERSION`, which takes precedence over `features.cmake` inside the
  check). The same variable is set in the Docker container.
- This is a RaftCore change and ships in a RaftCore release; RaftCLI does not depend on it, and an older
  RaftCore simply does not check.

#### Scaffold

`raft new` still asks for the ESP-IDF version, but writes it to **`systypes/Common/features.cmake`** as
`set(ESP_IDF_VERSION "6.0.2")` (the project default) instead of into the Dockerfile, and generates the
placeholder Dockerfile above. The generated `systypes/<SysType>/features.cmake` gains a commented
override: `# set(ESP_IDF_VERSION "6.0.2")  # Uncomment to build this SysType with a different ESP-IDF version`.
The README's "Customising this app" section is updated to say where the version lives.

Existing projects need no change. To adopt the new scheme by hand: add `set(ESP_IDF_VERSION ...)` to
`features.cmake` and replace the `FROM` line with the two placeholder lines.

#### Known side effect: `dependencies.lock`

The ESP-IDF component manager writes `dependencies.lock` (which records the ESP-IDF version) and
`managed_components/` at the project root, shared by all SysTypes. Two SysTypes on different ESP-IDF
versions will keep rewriting the lock file. ESP-IDF allows the lock file path to be set per build
(the documented `DEPENDENCIES_LOCK` build property, present in ESP-IDF 6.0.2); setting it per SysType in
`RaftBootstrapPhase2.cmake` is the clean fix. It is a RaftCore change; whether `managed_components/` also needs
separating is to be checked during implementation.

## 5. Test plan

### 5.1 Unit tests (run on every platform by `cargo test`)

Using temporary directories and injected roots/env, so none of them need ESP-IDF:

- Manifest parsing: the real v3.0 file above as a fixture; missing optional fields; unknown extra fields;
  each `status`; empty `idfInstalled`; malformed JSON (must not abort the search); Windows paths.
- `version.cmake` parsing; version normalisation (`v6.1`, `6.1.0`, `6.0.2`, `v5.4.2-dirty`,
  `release-v6.0`) and matching.
- Resolution order: fake legacy tree + fake EIM tree + manifest, asserting which install wins for each
  of: `-e path`, `-e name`, saved path, `IDF_PATH` set, `-i` with legacy only / EIM only / both /
  neither / two EIM installs with one selected / manifest missing (root scan).
- `-e` output parsing: PATH merge with `:` and `;`, values containing `=`, CRLF, blank and banner lines.
- Export validation: environment without `IDF_PYTHON_ENV_PATH` is rejected.
- `idf.py` command construction for each kind on each OS (pure function returning program + args).
- `raft.info` backward compatibility (old file loads; new fields round-trip).
- Required-version precedence (4.8): each of the five sources winning in turn; SysType overrides Common;
  commented-out `set(...)` ignored; quoted/unquoted, with/without `v`, extra whitespace, trailing comment;
  non-literal value (`${...}`) is an error; no `Common` folder (as in `RaftCore/unit_tests`).
- Dockerfile handling (4.8): classification into placeholder / literal / custom base image; version read
  from plain `FROM` and `FROM --platform=... AS name`; placeholder yields no version and a `--build-arg`; the rewrite changes only the image tag and leaves every other byte alone
  (including CRLF files); no `espressif/idf` base image is an error; equal versions produce no generated
  file; image tag and `docker build` argument list (pure function) for each case.
- Build-folder invalidation: `last_idf_version` differing from the required version requests deletion;
  absent `last_idf_version` (old `raft.info`) does not.

### 5.2 Manual integration matrix

Each row: scaffold an ESP32-S3 app (`raft new --defaults`), `raft build ...`, expect success and the
"Using ESP-IDF ..." line naming the expected install; then `raft f` + `raft m` once per platform.

| # | Scenario | Command | Linux (WSL) | Windows | macOS |
|---|---|---|---|---|---|
| 1 | Docker (regression) | `raft b --docker` | x | x | x |
| 2 | Legacy install found by search (regression) | `raft b -i` | x | x (if a legacy install exists) | x |
| 3 | Legacy explicit path (regression) | `raft b -e <legacy path>` | x | x | x |
| 4 | Legacy active shell (regression) | `. export.sh` then `raft b` | x | x (ESP-IDF cmd shortcut) | x |
| 5 | Saved method/path reused (regression) | `raft b` after 3 | x | x | x |
| 6 | EIM found by search | `raft b -i`, required version = the EIM install's version | x | x | x |
| 7 | EIM by name and by path | `raft b -e v6.x`, `raft b -e <eim path>` | x | x | x |
| 8 | EIM-activated shell | sourced script / `eim shell` / IDF_PowerShell shortcut, then `raft b` | bash + zsh | PowerShell + cmd profile | zsh |
| 9 | Both kinds installed, different versions | switch the required version between them | x | - | - |
| 10 | EIM renamed install | `eim rename`, then 6 | x | x | - |
| 11 | EIM non-default location | `eim install -p <dir>`; 6 via manifest, and with `--eim-json` after moving the manifest | x | x | - |
| 12 | No match | required version not installed | x | x | - |
| 13 | Broken EIM entry | set `status` to `failed` in a copy of the manifest, use `--eim-json` | x | - | - |
| 14 | Windows shells | 6 from cmd, PowerShell and Git Bash; path with spaces in the project folder | - | x | - |
| 15 | WSL flashing still delegates to `raft.exe` | `raft f -p COMx` | x | - | - |
| 16 | No SysType version set (regression) | any project as it is today, local and `--docker` | x | x | x |
| 17 | SysType version, local build | `RaftCore/unit_tests`: Dockerfile says 6.0.1, set `ESP_IDF_VERSION "6.0.2"` in `systypes/unittest/features.cmake`, `raft b -i` | x (legacy 6.0.2) | x (EIM) | x (EIM) |
| 18 | SysType version selects an EIM install | same with `"6.1"` | x (EIM v6.1) | - | - |
| 19 | SysType version, Docker build | same with `--docker`; check the generated Dockerfile, the image tag `raftbuilder:idf-6.0.2`, `idf.py --version` in the build log, and that the project `Dockerfile` is unmodified (`git status`) | x | x | x |
| 20 | Two SysTypes, two versions, one project | add a second SysType on another version; build each locally and with Docker, alternating; watch `dependencies.lock` | x | - | - |
| 21 | Version changed for an existing build folder | build, change `ESP_IDF_VERSION`, build again: folder deleted with a message, build succeeds | x | x | - |
| 22 | `Common` default with SysType override | version in `Common/features.cmake`, different one in the SysType | x | - | - |
| 23 | `--idf-version` override | beats the SysType value, local and Docker | x | - | - |
| 24a | Placeholder Dockerfile | freshly scaffolded project: `--docker` build passes `--build-arg`, no generated Dockerfile; `docker build .` by hand fails; removing `ESP_IDF_VERSION` from `features.cmake` falls back to the default with a warning | x | x | x |
| 24b | CMake version check (needs the RaftCore change) | run `idf.py` from a shell with the wrong ESP-IDF active: fatal error naming both versions; `raft b --idf-version <other>` is not blocked | x | - | - |
| 24 | Custom Dockerfile base image | SysType version set, Dockerfile not based on `espressif/idf`: clear error, no build | x | - | - |
| 25 | Required version not installed locally | error lists installs found and the `eim install` hint; `--docker` still works | x | x | - |

Rob's WSL already has both a legacy 6.0.2 (`~/esp`) and an EIM v6.1, so rows 1-13 can be run there
immediately. Windows needs EIM installing (`winget install Espressif.EIM-CLI`, `eim install -i v6.0.2`);
there is currently no `C:\Espressif` on that machine, so row 2/3/4 on Windows need either an old legacy
installer run in a VM or to be accepted as covered by unit tests plus Linux. macOS needs a machine or
the CI runner below.

### 5.3 CI (GitHub Actions, extends `app_release.yml`)

- `cargo test` on `ubuntu-latest`, `windows-latest`, `macos-latest` for every push (section 5.1).
- A separate, manually triggered or nightly workflow per OS: install EIM headless, `eim install -i
  <default_esp_idf_version>`, build RaftCLI, `raft new --defaults`, `raft build -i`, assert the ELF exists
  and the log contains `[EIM`. Cache the EIM install directory keyed on the IDF version (the install is
  several minutes and ~2-3 GB). Check whether Espressif's `install-esp-idf-action` is suitable before
  writing the install steps by hand.
- In that workflow also build `RaftCore/unit_tests` with a SysType version that differs from its
  Dockerfile, locally and (Linux runner only, where Docker is available) with `--docker`.
- The same workflow with a legacy install (`git clone -b v<ver>` + `install.sh`) on Linux keeps rows
  2-4 covered automatically.

### 5.4 Order of work

1. `raft new --defaults` (4.7) and the locator module with unit tests (4.1, 4.2, 5.1) - no behaviour change yet.
2. Required-version resolution (4.8 precedence) as a pure function with unit tests, replacing the direct
   call to `get_esp_idf_version_from_dockerfile`; numeric version type shared with the locator. Rows 16, 17, 22, 23.
   This comes before the EIM work because everything after it consumes "the required version".
3. `idf.py` via python and `version.cmake` version check (4.4) - fixes EIM-activated shells, verify rows 1-5 and 8.
4. EIM environment capture and export validation (4.3) - rows 6, 7, 9-13, 18, 25.
5. Docker: placeholder `--build-arg`, generated Dockerfile for literal versions, per-version image tag
   (4.8) - rows 19, 20, 24, 24a.
6. `raft.info` fields and build-folder invalidation (4.5, 4.8) - row 21.
7. Windows pass (row 14 and 1-8, 16-21), then macOS via CI.
8. Scaffold: version into `Common/features.cmake`, placeholder Dockerfile, commented SysType override,
   remove `compose.yaml`; README/help; scaffold unit tests; CI workflows.
9. RaftCore, separately released: fatal CMake version check honouring `RAFT_ESP_IDF_VERSION` (row 24b),
   per-SysType `DEPENDENCIES_LOCK`, and `RaftCore/unit_tests` moved to the new scheme as the first user.

## 6. Open questions

- Dockerfile tags that are not versions (`latest`, `release-v6.0`): today these can never match a local
  install. Proposal: for such tags use the EIM selected install (or the active environment) and print a
  warning.
- Should `-i` prefer EIM over legacy when both have the same version? The plan keeps legacy first purely
  for backward compatibility.
- Should RaftCLI offer to run `eim install -i <version>` when nothing matches? Proposed: no, print the
  command only.
- Decided 2026-09-19: `ESP_IDF_VERSION` lives in `features.cmake`; the Dockerfile carries a placeholder
  rather than a version and is never edited by RaftCLI; a CMake version mismatch is a fatal error.
- 4.8: the placeholder is planned as `ARG ESP_IDF_VERSION` + `FROM espressif/idf:${ESP_IDF_VERSION}` rather
  than a literal `<ESP-IDF-VERSION>` token (reasons in 4.8). Say if the literal token is preferred.
- 4.8: remove `compose.yaml` from the scaffold template?
- The manifest schema version seen is "3.0". Parse leniently and warn (do not fail) on an unknown major
  version.
