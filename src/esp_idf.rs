// RaftCLI: ESP-IDF selection
// Rob Dobson 2026
//
// - Which ESP-IDF version does a build require (SysType features.cmake, Common features.cmake, Dockerfile, default)
// - Where is an ESP-IDF of that version (explicit path, active environment, legacy install folders,
//   Espressif Installation Manager (EIM) installs)
// - How is its environment obtained and how is idf.py run
// - How is a Docker build made to use that version
//
// The functions which make decisions take their inputs (folders, environment, file contents) as parameters
// so that they can be unit tested on any platform without an ESP-IDF being installed.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use regex::Regex;
use serde::Deserialize;

/// Name of the variable in features.cmake, the Docker build argument and (with RAFT_ prefix) the
/// environment variable passed to CMake
pub const ESP_IDF_VERSION_VAR: &str = "ESP_IDF_VERSION";
pub const RAFT_ESP_IDF_VERSION_ENV: &str = "RAFT_ESP_IDF_VERSION";
pub const RAFT_EIM_IDF_JSON_ENV: &str = "RAFT_EIM_IDF_JSON";

/////////////////////////////////////////////////////////////////////////////////////////////////
// Versions
/////////////////////////////////////////////////////////////////////////////////////////////////

/// Numeric ESP-IDF version - a missing patch number is 0 (so "6.1" equals "6.1.0")
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct IdfVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl IdfVersion {
    /// Parse a version from text such as "6.0.2", "v6.1", "v5.4.2-dirty" or "release-v6.0"
    /// Returns None if there is no major.minor number in the text (e.g. "latest")
    pub fn parse(text: &str) -> Option<IdfVersion> {
        let re = Regex::new(r"(\d+)\.(\d+)(?:\.(\d+))?").unwrap();
        let caps = re.captures(text)?;
        Some(IdfVersion {
            major: caps.get(1)?.as_str().parse().ok()?,
            minor: caps.get(2)?.as_str().parse().ok()?,
            patch: caps.get(3).map_or(Some(0), |m| m.as_str().parse().ok())?,
        })
    }
}

impl fmt::Display for IdfVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Read the version of an ESP-IDF from its tools/cmake/version.cmake file
/// (this file is present however the ESP-IDF was installed)
pub fn read_idf_version_from_folder(idf_path: &Path) -> Option<IdfVersion> {
    let content = fs::read_to_string(idf_path.join("tools").join("cmake").join("version.cmake")).ok()?;
    parse_version_cmake(&content)
}

pub fn parse_version_cmake(content: &str) -> Option<IdfVersion> {
    let get = |name: &str| -> Option<u32> {
        let re = Regex::new(&format!(r"(?m)^\s*set\s*\(\s*IDF_VERSION_{}\s+(\d+)\s*\)", name)).unwrap();
        re.captures(content)?.get(1)?.as_str().parse().ok()
    };
    Some(IdfVersion { major: get("MAJOR")?, minor: get("MINOR")?, patch: get("PATCH").unwrap_or(0) })
}

/////////////////////////////////////////////////////////////////////////////////////////////////
// Required version
/////////////////////////////////////////////////////////////////////////////////////////////////

/// The ESP-IDF version required for a build
#[derive(Debug, Clone, PartialEq)]
pub struct RequiredVersion {
    /// As written by the user but without any leading 'v' (e.g. "6.1") - used for the Docker image tag
    /// since the image is espressif/idf:v6.1 and there is no v6.1.0 image
    pub as_written: String,
    /// Numeric version used to match local installs (None if as_written isn't a version e.g. "latest")
    pub version: Option<IdfVersion>,
    /// Where the requirement came from (for messages)
    pub source: String,
}

impl RequiredVersion {
    pub fn new(as_written: &str, source: &str) -> RequiredVersion {
        let trimmed = as_written.trim();
        let without_v = trimmed.strip_prefix('v').unwrap_or(trimmed);
        RequiredVersion {
            as_written: without_v.to_string(),
            version: IdfVersion::parse(without_v),
            source: source.to_string(),
        }
    }

    /// Tag of the espressif/idf Docker image
    pub fn docker_tag(&self) -> String {
        // Tags which are versions have a leading v (v6.0.2) others (latest, release-v6.0) do not
        if self.as_written.chars().next().map_or(false, |c| c.is_ascii_digit()) {
            format!("v{}", self.as_written)
        } else {
            self.as_written.clone()
        }
    }

    /// Value passed to CMake (RaftBootstrap checks it against the ESP-IDF in use)
    pub fn for_cmake(&self) -> String {
        self.version.map_or(self.as_written.clone(), |v| v.to_string())
    }

    pub fn matches(&self, version: &IdfVersion) -> bool {
        self.version.map_or(false, |v| v == *version)
    }
}

/// Find set(ESP_IDF_VERSION "x.y.z") in the content of a features.cmake file
/// Ok(None) if not set, Err if it is set to something that can't be used (e.g. a variable reference)
pub fn parse_features_cmake_version(content: &str) -> Result<Option<String>, String> {
    let re = Regex::new(&format!(
        r#"(?m)^[ \t]*set[ \t]*\([ \t]*{}[ \t]+([^)\r\n]*)\)"#, ESP_IDF_VERSION_VAR)).unwrap();
    // The last uncommented definition wins (as it would in CMake)
    let mut found: Option<String> = None;
    for caps in re.captures_iter(content) {
        let raw = caps.get(1).map_or("", |m| m.as_str()).trim();
        // Remove CACHE/other trailing keywords and quotes
        let value = raw.split_whitespace().next().unwrap_or("").trim_matches('"').to_string();
        if value.is_empty() {
            continue;
        }
        if value.contains("${") || value.contains("$ENV") {
            return Err(format!(
                "{} must be set to a literal version (e.g. set({} \"6.0.2\")) but is set to {}",
                ESP_IDF_VERSION_VAR, ESP_IDF_VERSION_VAR, raw));
        }
        found = Some(value);
    }
    Ok(found)
}

/// Kinds of Dockerfile as far as the ESP-IDF version is concerned
#[derive(Debug, Clone, PartialEq)]
pub enum DockerfileKind {
    /// ARG ESP_IDF_VERSION (with optional default) and FROM espressif/idf:${ESP_IDF_VERSION}
    Placeholder { default_tag: Option<String> },
    /// FROM espressif/idf:<tag>
    Literal { tag: String },
    /// No espressif/idf base image
    Custom,
    /// No Dockerfile
    Missing,
}

fn dockerfile_from_regex() -> Regex {
    // FROM [--platform=xxx] espressif/idf:<tag> [AS name]
    Regex::new(r"(?mi)^([ \t]*FROM[ \t]+(?:--platform=\S+[ \t]+)?espressif/idf:)([^\s]+)").unwrap()
}

pub fn classify_dockerfile(content: Option<&str>) -> DockerfileKind {
    let content = match content {
        Some(content) => content,
        None => return DockerfileKind::Missing,
    };
    let caps = match dockerfile_from_regex().captures(content) {
        Some(caps) => caps,
        None => return DockerfileKind::Custom,
    };
    let tag = caps.get(2).map_or("", |m| m.as_str()).to_string();
    if tag.contains(&format!("${{{}}}", ESP_IDF_VERSION_VAR)) || tag.contains(&format!("${}", ESP_IDF_VERSION_VAR)) {
        let arg_re = Regex::new(&format!(r"(?mi)^[ \t]*ARG[ \t]+{}(?:=(\S+))?[ \t]*\r?$", ESP_IDF_VERSION_VAR)).unwrap();
        let default_tag = arg_re.captures(content)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim_matches('"').to_string());
        return DockerfileKind::Placeholder { default_tag };
    }
    DockerfileKind::Literal { tag }
}

/// Return a copy of the Dockerfile content with only the espressif/idf image tag replaced
pub fn rewrite_dockerfile_tag(content: &str, new_tag: &str) -> Option<String> {
    let re = dockerfile_from_regex();
    if !re.is_match(content) {
        return None;
    }
    let mut done = false;
    Some(re.replace_all(content, |caps: &regex::Captures| {
        if done {
            return caps[0].to_string();
        }
        done = true;
        format!("{}{}", &caps[1], new_tag)
    }).to_string())
}

/// Inputs to the required version decision (file contents rather than paths so that this is testable)
pub struct RequiredVersionInputs<'a> {
    pub cli_version: Option<&'a str>,
    pub sys_type_name: &'a str,
    pub sys_type_features: Option<&'a str>,
    pub common_features: Option<&'a str>,
    pub dockerfile: Option<&'a str>,
    pub default_version: &'a str,
}

/// Decide the required ESP-IDF version. Precedence:
/// 1. command line 2. SysType features.cmake 3. Common features.cmake 4. literal version in Dockerfile 5. default
pub fn resolve_required_version(inputs: &RequiredVersionInputs) -> Result<RequiredVersion, String> {
    if let Some(cli_version) = inputs.cli_version {
        return Ok(RequiredVersion::new(cli_version, "--idf-version"));
    }
    if let Some(content) = inputs.sys_type_features {
        if let Some(version) = parse_features_cmake_version(content)
                .map_err(|e| format!("systypes/{}/features.cmake: {}", inputs.sys_type_name, e))? {
            return Ok(RequiredVersion::new(&version, &format!("systypes/{}/features.cmake", inputs.sys_type_name)));
        }
    }
    if let Some(content) = inputs.common_features {
        if let Some(version) = parse_features_cmake_version(content)
                .map_err(|e| format!("systypes/Common/features.cmake: {}", e))? {
            return Ok(RequiredVersion::new(&version, "systypes/Common/features.cmake"));
        }
    }
    match classify_dockerfile(inputs.dockerfile) {
        DockerfileKind::Literal { tag } => return Ok(RequiredVersion::new(&tag, "Dockerfile")),
        DockerfileKind::Placeholder { default_tag: Some(tag) } =>
            return Ok(RequiredVersion::new(&tag, "Dockerfile (ARG default)")),
        _ => {}
    }
    Ok(RequiredVersion::new(inputs.default_version, "RaftCLI default - no version is set in features.cmake or the Dockerfile"))
}

/// Read the files for a project and decide the required version
pub fn get_required_version(app_folder: &str, sys_type: &str, cli_version: Option<&str>, default_version: &str)
            -> Result<RequiredVersion, String> {
    let systypes = Path::new(app_folder).join("systypes");
    let sys_type_features = fs::read_to_string(systypes.join(sys_type).join("features.cmake")).ok();
    let common_features = fs::read_to_string(systypes.join("Common").join("features.cmake")).ok();
    let dockerfile = fs::read_to_string(Path::new(app_folder).join("Dockerfile")).ok();
    resolve_required_version(&RequiredVersionInputs {
        cli_version,
        sys_type_name: sys_type,
        sys_type_features: sys_type_features.as_deref(),
        common_features: common_features.as_deref(),
        dockerfile: dockerfile.as_deref(),
        default_version,
    })
}

/////////////////////////////////////////////////////////////////////////////////////////////////
// Docker build plan
/////////////////////////////////////////////////////////////////////////////////////////////////

/// How the Docker image for a build is to be built
#[derive(Debug, Clone, PartialEq)]
pub struct DockerBuildPlan {
    /// Content of a Dockerfile to generate (None means use the project Dockerfile as it is)
    pub generated_dockerfile: Option<String>,
    /// Extra arguments for docker build (--build-arg ...)
    pub build_args: Vec<String>,
    /// Image tag
    pub image_tag: String,
    /// Note for the user (if any)
    pub note: Option<String>,
}

pub fn plan_docker_build(dockerfile: Option<&str>, required: &RequiredVersion) -> Result<DockerBuildPlan, String> {
    // Tags can't contain all characters that a version might
    let tag_safe: String = required.as_written.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '_' }).collect();
    let image_tag = format!("raftbuilder:idf-{}", tag_safe);
    match classify_dockerfile(dockerfile) {
        DockerfileKind::Missing => Err("Dockerfile not found in the project folder".to_string()),
        DockerfileKind::Custom => {
            if required.source == "Dockerfile" || required.source.starts_with("RaftCLI default") {
                // Nothing asks for a specific version so build the custom image as it is
                Ok(DockerBuildPlan { generated_dockerfile: None, build_args: vec![],
                        image_tag: "raftbuilder".to_string(), note: None })
            } else {
                Err(format!(
                    "ESP-IDF {} is required (from {}) but the Dockerfile is not based on an espressif/idf image so \
                    the version can't be applied. Use \"ARG {}\" and \"FROM espressif/idf:${{{}}}\" in the Dockerfile",
                    required.as_written, required.source, ESP_IDF_VERSION_VAR, ESP_IDF_VERSION_VAR))
            }
        }
        DockerfileKind::Placeholder { .. } => Ok(DockerBuildPlan {
            generated_dockerfile: None,
            build_args: vec!["--build-arg".to_string(), format!("{}={}", ESP_IDF_VERSION_VAR, required.docker_tag())],
            image_tag,
            note: None,
        }),
        DockerfileKind::Literal { tag } => {
            if tag == required.docker_tag() || RequiredVersion::new(&tag, "").version.map_or(false, |v| required.matches(&v)) {
                Ok(DockerBuildPlan { generated_dockerfile: None, build_args: vec![], image_tag, note: None })
            } else {
                Ok(DockerBuildPlan {
                    generated_dockerfile: rewrite_dockerfile_tag(dockerfile.unwrap_or(""), &required.docker_tag()),
                    build_args: vec![],
                    image_tag,
                    note: Some(format!(
                        "The Dockerfile specifies ESP-IDF {} but {} is required (from {}) so a generated copy of the Dockerfile \
                        is used. To avoid confusion replace the FROM line in the Dockerfile with \"ARG {}\" and \
                        \"FROM espressif/idf:${{{}}}\"",
                        tag, required.as_written, required.source, ESP_IDF_VERSION_VAR, ESP_IDF_VERSION_VAR)),
                })
            }
        }
    }
}

/////////////////////////////////////////////////////////////////////////////////////////////////
// EIM (Espressif Installation Manager) manifest
/////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct EimInstallation {
    #[serde(rename = "activationScript", default)]
    pub activation_script: Option<String>,
    #[serde(default)]
    pub id: String,
    #[serde(rename = "idfToolsPath", default)]
    pub idf_tools_path: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub python: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

impl EimInstallation {
    /// Installations which are part-installed or broken are not used
    pub fn is_usable(&self) -> bool {
        match self.status.as_deref() {
            None | Some("finished") => !self.path.is_empty(),
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
pub struct EimManifest {
    #[serde(rename = "idfInstalled", default)]
    pub idf_installed: Vec<EimInstallation>,
    #[serde(rename = "idfSelectedId", default)]
    pub idf_selected_id: Option<String>,
    #[serde(rename = "eimPath", default)]
    pub eim_path: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

pub fn parse_eim_manifest(content: &str) -> Result<EimManifest, String> {
    serde_json::from_str::<EimManifest>(content).map_err(|e| e.to_string())
}

/// Default folder containing eim_idf.json (and the activation scripts)
pub fn default_eim_tools_folder(is_windows: bool, home_dir: Option<&Path>) -> Option<PathBuf> {
    if is_windows {
        Some(PathBuf::from(r"C:\Espressif\tools"))
    } else {
        home_dir.map(|h| h.join(".espressif").join("tools"))
    }
}

/// Default folder in which EIM installs ESP-IDF versions (<root>/<name>/esp-idf)
pub fn default_eim_install_root(is_windows: bool, home_dir: Option<&Path>) -> Option<PathBuf> {
    if is_windows {
        Some(PathBuf::from(r"C:\esp"))
    } else {
        home_dir.map(|h| h.join(".espressif"))
    }
}

/////////////////////////////////////////////////////////////////////////////////////////////////
// Locating an ESP-IDF
/////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug, Clone, PartialEq)]
pub enum IdfKind {
    /// The environment of this process already has the ESP-IDF set up (IDF_PATH etc)
    ActiveEnv,
    /// Installed the traditional way - the environment is obtained from export.sh / export.bat
    Legacy,
    /// Installed by EIM - the environment is obtained from the EIM activation script
    Eim { name: String, activation_script: Option<PathBuf>, python: Option<PathBuf> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct IdfInstall {
    pub idf_path: PathBuf,
    pub version: Option<IdfVersion>,
    pub kind: IdfKind,
    /// How it was found (for messages)
    pub origin: String,
}

impl IdfInstall {
    pub fn describe(&self) -> String {
        let kind = match &self.kind {
            IdfKind::ActiveEnv => "active environment".to_string(),
            IdfKind::Legacy => "export script".to_string(),
            IdfKind::Eim { name, .. } => format!("EIM \"{}\"", name),
        };
        format!("ESP-IDF {} [{}] at {} ({})",
            self.version.map_or("unknown version".to_string(), |v| v.to_string()),
            kind, self.idf_path.display(), self.origin)
    }
}

pub struct LocatorInputs<'a> {
    pub required: &'a RequiredVersion,
    /// -e option: path of an ESP-IDF folder, a folder containing ESP-IDF folders, or the name of an EIM install
    pub explicit: Option<&'a str>,
    /// Explicit path saved from a previous build
    pub saved_explicit: Option<&'a str>,
    /// IDF_PATH from the environment of this process
    pub env_idf_path: Option<&'a str>,
    /// Folders which contain traditionally installed ESP-IDFs (e.g. ~/esp)
    pub legacy_roots: Vec<PathBuf>,
    /// EIM manifest (already loaded)
    pub eim_manifest: Option<&'a EimManifest>,
    /// EIM tools folder (used to guess activation script names if there is no manifest)
    pub eim_tools_folder: Option<PathBuf>,
    /// Folders in which EIM installs ESP-IDF versions (e.g. ~/.espressif)
    pub eim_install_roots: Vec<PathBuf>,
    pub is_windows: bool,
}

#[derive(Debug)]
pub struct LocateError {
    pub message: String,
    /// Everything that was found (to help the user)
    pub found: Vec<IdfInstall>,
}

fn is_idf_folder(path: &Path) -> bool {
    path.join("tools").join("idf.py").is_file() || path.join("export.sh").is_file()
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    let canon = |p: &Path| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    canon(a) == canon(b)
}

fn eim_activation_script_name(name: &str, is_windows: bool) -> String {
    if is_windows { format!("Microsoft.{}.PowerShell_profile.ps1", name) } else { format!("activate_idf_{}.sh", name) }
}

fn install_from_eim(entry: &EimInstallation, origin: &str) -> IdfInstall {
    let idf_path = PathBuf::from(&entry.path);
    IdfInstall {
        version: read_idf_version_from_folder(&idf_path),
        idf_path,
        kind: IdfKind::Eim {
            name: entry.name.clone(),
            activation_script: entry.activation_script.as_ref().map(PathBuf::from),
            python: entry.python.as_ref().map(PathBuf::from),
        },
        origin: origin.to_string(),
    }
}

/// An ESP-IDF folder is an EIM install if it is in the manifest, or it is <eim root>/<name>/esp-idf
/// and the activation script for <name> exists
fn classify_idf_folder(idf_path: &Path, inputs: &LocatorInputs, origin: &str) -> IdfInstall {
    if let Some(manifest) = inputs.eim_manifest {
        for entry in &manifest.idf_installed {
            if entry.is_usable() && paths_equal(Path::new(&entry.path), idf_path) {
                return install_from_eim(entry, origin);
            }
        }
    }
    if let Some(guessed) = guess_eim_install(idf_path, inputs, origin) {
        return guessed;
    }
    IdfInstall {
        idf_path: idf_path.to_path_buf(),
        version: read_idf_version_from_folder(idf_path),
        kind: IdfKind::Legacy,
        origin: origin.to_string(),
    }
}

fn guess_eim_install(idf_path: &Path, inputs: &LocatorInputs, origin: &str) -> Option<IdfInstall> {
    if idf_path.file_name()?.to_str()? != "esp-idf" {
        return None;
    }
    let name_folder = idf_path.parent()?;
    let root = name_folder.parent()?;
    if !inputs.eim_install_roots.iter().any(|r| paths_equal(r, root)) {
        return None;
    }
    let name = name_folder.file_name()?.to_str()?.to_string();
    let tools = inputs.eim_tools_folder.as_ref()?;
    let script = tools.join(eim_activation_script_name(&name, inputs.is_windows));
    if !script.is_file() {
        return None;
    }
    let venv = tools.join("python").join(&name).join("venv");
    let python = if inputs.is_windows { venv.join("Scripts").join("python.exe") } else { venv.join("bin").join("python") };
    Some(IdfInstall {
        idf_path: idf_path.to_path_buf(),
        version: read_idf_version_from_folder(idf_path),
        kind: IdfKind::Eim { name, activation_script: Some(script), python: if python.is_file() { Some(python) } else { None } },
        origin: origin.to_string(),
    })
}

/// ESP-IDF folders directly inside a folder
fn idf_folders_in(root: &Path) -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = fs::read_dir(root).into_iter().flatten()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    folders.sort();
    folders
}

/// Does a legacy folder match the required version - by its real version if known, otherwise by the
/// folder name ending with the version (the original RaftCLI rule e.g. esp-idf-v6.0.2)
fn legacy_folder_matches(folder: &Path, required: &RequiredVersion) -> bool {
    match read_idf_version_from_folder(folder) {
        Some(version) => required.matches(&version),
        None => folder.file_name().map_or(false, |n| n.to_string_lossy().ends_with(&required.as_written)),
    }
}

/// Find the ESP-IDF to use for a build
pub fn locate_esp_idf(inputs: &LocatorInputs) -> Result<IdfInstall, LocateError> {
    let required = inputs.required;
    let mut found: Vec<IdfInstall> = Vec::new();

    // 1 & 3. An explicit path/name (-e option) and, after the active environment, the saved explicit path
    let try_explicit = |explicit: &str, origin: &str, found: &mut Vec<IdfInstall>| -> Option<IdfInstall> {
        let path = Path::new(explicit);
        if path.is_dir() {
            if is_idf_folder(path) {
                // An ESP-IDF folder given explicitly is used whatever its version (as RaftCLI always has)
                return Some(classify_idf_folder(path, inputs, origin));
            }
            // A folder containing ESP-IDF folders (or EIM <name>/esp-idf folders)
            for sub in idf_folders_in(path) {
                for candidate in [sub.clone(), sub.join("esp-idf")] {
                    if is_idf_folder(&candidate) {
                        let install = classify_idf_folder(&candidate, inputs, origin);
                        if legacy_folder_matches(&candidate, required) {
                            return Some(install);
                        }
                        found.push(install);
                    }
                }
            }
            return None;
        }
        // Name of an EIM install
        if let Some(manifest) = inputs.eim_manifest {
            for entry in &manifest.idf_installed {
                if entry.is_usable() && entry.name == explicit {
                    return Some(install_from_eim(entry, origin));
                }
            }
        }
        None
    };

    if let Some(explicit) = inputs.explicit {
        return try_explicit(explicit, "-e option", &mut found).ok_or_else(|| LocateError {
            message: format!("\"{}\" is not an ESP-IDF folder, a folder containing ESP-IDF {} or the name of an EIM installation",
                        explicit, required.as_written),
            found: found.clone(),
        });
    }

    // 2. The active environment if it is the required version
    if let Some(env_idf_path) = inputs.env_idf_path {
        let path = Path::new(env_idf_path);
        if is_idf_folder(path) {
            let version = read_idf_version_from_folder(path);
            let install = IdfInstall { idf_path: path.to_path_buf(), version, kind: IdfKind::ActiveEnv,
                        origin: "IDF_PATH of the active environment".to_string() };
            if version.map_or(false, |v| required.matches(&v)) {
                return Ok(install);
            }
            found.push(install);
        }
    }

    // 3. Explicit path saved from a previous build
    if let Some(saved) = inputs.saved_explicit {
        if let Some(install) = try_explicit(saved, "path saved from a previous build", &mut found) {
            return Ok(install);
        }
    }

    // 4a. Traditional install folders
    for root in &inputs.legacy_roots {
        for folder in idf_folders_in(root) {
            if !is_idf_folder(&folder) {
                continue;
            }
            let install = classify_idf_folder(&folder, inputs, &format!("found in {}", root.display()));
            if legacy_folder_matches(&folder, required) {
                return Ok(install);
            }
            found.push(install);
        }
    }

    // 4b. EIM manifest (prefer the selected installation if more than one matches)
    if let Some(manifest) = inputs.eim_manifest {
        let mut matches: Vec<IdfInstall> = Vec::new();
        for entry in manifest.idf_installed.iter().filter(|e| e.is_usable()) {
            let install = install_from_eim(entry, "EIM manifest");
            if install.version.map_or(false, |v| required.matches(&v)) {
                if manifest.idf_selected_id.as_deref() == Some(entry.id.as_str()) {
                    return Ok(install);
                }
                matches.push(install);
            } else {
                found.push(install);
            }
        }
        if let Some(install) = matches.into_iter().next() {
            return Ok(install);
        }
    }

    // 4c. EIM install folders (in case the manifest is missing or unreadable)
    // Installs which the manifest lists as unusable (failed, part-installed, broken) are not used
    let unusable_paths: Vec<PathBuf> = inputs.eim_manifest.map(|manifest| {
        manifest.idf_installed.iter().filter(|e| !e.is_usable()).map(|e| PathBuf::from(&e.path)).collect()
    }).unwrap_or_default();
    for root in &inputs.eim_install_roots {
        for name_folder in idf_folders_in(root) {
            let candidate = name_folder.join("esp-idf");
            if !is_idf_folder(&candidate) || found.iter().any(|f| paths_equal(&f.idf_path, &candidate))
                        || unusable_paths.iter().any(|p| paths_equal(p, &candidate)) {
                continue;
            }
            let install = classify_idf_folder(&candidate, inputs, &format!("found in {}", root.display()));
            if install.version.map_or(false, |v| required.matches(&v)) {
                return Ok(install);
            }
            found.push(install);
        }
    }

    Err(LocateError {
        message: format!("No ESP-IDF {} found (required by {})", required.as_written, required.source),
        found,
    })
}

/////////////////////////////////////////////////////////////////////////////////////////////////
// Environment for running idf.py
/////////////////////////////////////////////////////////////////////////////////////////////////

/// Parse the output of an EIM activation script run with -e (KEY=VALUE lines)
/// PATH in that output is only the folders to be added so it is prepended to the current PATH
pub fn parse_eim_env_output(output: &str, current_path: &str, path_separator: &str) -> HashMap<String, String> {
    let key_re = Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*$").unwrap();
    let mut env_vars = HashMap::new();
    for line in output.lines() {
        let line = line.trim_end_matches('\r');
        if let Some((key, value)) = line.split_once('=') {
            if !key_re.is_match(key) || key == "SYSTEM_PATH" {
                continue;
            }
            if key == "PATH" {
                let combined = if current_path.is_empty() { value.to_string() }
                            else { format!("{}{}{}", value, path_separator, current_path) };
                env_vars.insert(key.to_string(), combined);
            } else {
                env_vars.insert(key.to_string(), value.to_string());
            }
        }
    }
    env_vars
}

/// Parse the output of "env" / "set" (KEY=VALUE lines)
pub fn parse_env_dump(output: &str) -> HashMap<String, String> {
    let mut env_vars = HashMap::new();
    for line in output.lines() {
        if let Some((key, value)) = line.trim_end_matches('\r').split_once('=') {
            if !key.is_empty() && !key.contains(' ') {
                env_vars.insert(key.to_string(), value.to_string());
            }
        }
    }
    env_vars
}

/// An environment is only usable if it has IDF_PATH and a python environment which exists
/// (export.sh exits with 0 even when it has failed to find the python environment)
pub fn validate_idf_env(env_vars: &HashMap<String, String>) -> Result<(), String> {
    if !env_vars.contains_key("IDF_PATH") {
        return Err("IDF_PATH was not set".to_string());
    }
    match env_vars.get("IDF_PYTHON_ENV_PATH") {
        Some(python_env) if Path::new(python_env).is_dir() => Ok(()),
        Some(python_env) => Err(format!("the python environment {} does not exist", python_env)),
        None => Err("IDF_PYTHON_ENV_PATH was not set (the ESP-IDF tools may not be installed)".to_string()),
    }
}

/// Environment variables set by ESP-IDF activation. When the environment of an ESP-IDF is captured these
/// are removed first as this process may be running in a shell in which a different ESP-IDF is active
/// (in which case, for example, export.sh would otherwise keep the IDF_PATH of that other ESP-IDF)
pub const IDF_ENV_VARS: [&str; 10] = ["IDF_PATH", "IDF_PYTHON_ENV_PATH", "ESP_IDF_VERSION", "IDF_VERSION",
            "ESP_ROM_ELF_DIR", "OPENOCD_SCRIPTS", "IDF_COMPONENT_LOCAL_STORAGE_URL", "IDF_DEACTIVATE_FILE_PATH",
            "ESP_CLANG_LIBS_PATH", "IDF_TOOLS_PATH"];

/// Which of the ESP-IDF variables should be removed given the current environment
/// IDF_TOOLS_PATH is only removed if it was set by EIM activation (it is the folder containing the EIM
/// manifest) since a user with tools in a non-standard place sets it deliberately for export.sh to use
pub fn idf_env_vars_to_remove(current_env: &HashMap<String, String>) -> Vec<String> {
    IDF_ENV_VARS.iter().filter(|name| {
        match current_env.get(**name) {
            None => false,
            Some(value) if **name == "IDF_TOOLS_PATH" => Path::new(value).join("eim_idf.json").is_file(),
            Some(_) => true,
        }
    }).map(|name| name.to_string()).collect()
}

fn run_and_capture(program: &str, args: &[&str]) -> Result<String, String> {
    let mut command = Command::new(program);
    for name in idf_env_vars_to_remove(&std::env::vars().collect()) {
        command.env_remove(name);
    }
    let output = command.args(args).stdin(Stdio::null()).output()
        .map_err(|e| format!("failed to run {}: {}", program, e))?;
    let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    if !output.status.success() {
        return Err(format!("{} failed: {}", program, text.lines().rev().take(5).collect::<Vec<_>>().join(" | ")));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn capture_legacy_env(idf_path: &Path) -> Result<HashMap<String, String>, String> {
    let env_vars;
    #[cfg(not(target_os = "windows"))]
    {
        let export_script = idf_path.join("export.sh");
        if !export_script.is_file() {
            return Err(format!("export.sh not found in {}", idf_path.display()));
        }
        let output = run_and_capture("bash", &["-c", &format!("source \"{}\" >/dev/null 2>&1; env", export_script.display())])?;
        env_vars = parse_env_dump(&output);
    }
    #[cfg(target_os = "windows")]
    {
        let export_script = idf_path.join("export.bat");
        if !export_script.is_file() {
            return Err(format!("export.bat not found in {}", idf_path.display()));
        }
        let output = run_and_capture("cmd", &["/C", export_script.to_str().unwrap_or(""), "&&", "set"])?;
        env_vars = parse_env_dump(&output);
    }
    validate_idf_env(&env_vars).map_err(|e| format!("the ESP-IDF export script in {} did not produce a usable environment: {}",
                idf_path.display(), e))?;
    Ok(env_vars)
}

fn capture_eim_env(activation_script: &Path) -> Result<HashMap<String, String>, String> {
    let current_path = std::env::var("PATH").unwrap_or_default();
    let script = activation_script.to_str().unwrap_or("");

    // Preferred method: the activation script prints its environment when given -e
    #[cfg(not(target_os = "windows"))]
    let (printed, separator) = (run_and_capture("bash", &[script, "-e"]), ":");
    #[cfg(target_os = "windows")]
    let (printed, separator) = {
        // The batch profile sits alongside the PowerShell profile (Microsoft.<name>_profile.bat)
        let bat = PathBuf::from(script.replace(".PowerShell_profile.ps1", "_profile.bat"));
        if bat.is_file() {
            (run_and_capture("cmd", &["/C", bat.to_str().unwrap_or(""), "-e"]), ";")
        } else {
            (run_and_capture("powershell", &["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", script, "-e"]), ";")
        }
    };
    if let Ok(printed) = &printed {
        let env_vars = parse_eim_env_output(printed, &current_path, separator);
        if validate_idf_env(&env_vars).is_ok() {
            return Ok(env_vars);
        }
    }

    // Fallback: activate and dump the whole environment
    #[cfg(not(target_os = "windows"))]
    let dumped = run_and_capture("bash", &["-c", &format!(". \"{}\" >/dev/null 2>&1; env", script)]);
    #[cfg(target_os = "windows")]
    let dumped = {
        let bat = script.replace(".PowerShell_profile.ps1", "_profile.bat");
        run_and_capture("cmd", &["/C", &bat, ">NUL", "&&", "set"])
    };
    let env_vars = parse_env_dump(&dumped?);
    validate_idf_env(&env_vars).map_err(|e| format!("the EIM activation script {} did not produce a usable environment: {}",
                activation_script.display(), e))?;
    Ok(env_vars)
}

/// The environment variables to add, and the command used to run idf.py
#[derive(Debug, Clone)]
pub struct IdfRunSetup {
    pub env_vars: HashMap<String, String>,
    /// Variables of a different (active) ESP-IDF which must not be passed on to idf.py
    pub env_vars_to_remove: Vec<String>,
    pub program: String,
    pub leading_args: Vec<String>,
}

/// idf.py is run using the python of the ESP-IDF environment since idf.py is not necessarily on the PATH
/// (EIM defines it only as a shell function/alias which isn't available to a child process)
pub fn idf_py_command(idf_path: &Path, python: Option<&Path>, python_env_path: Option<&str>, is_windows: bool)
            -> (String, Vec<String>) {
    let idf_py = idf_path.join("tools").join("idf.py").to_string_lossy().to_string();
    let python_from_env = python_env_path.map(|env_path| {
        let env_path = Path::new(env_path);
        if is_windows { env_path.join("Scripts").join("python.exe") } else { env_path.join("bin").join("python") }
    });
    let python = python.map(|p| p.to_path_buf()).filter(|p| p.is_file())
        .or(python_from_env.filter(|p| p.is_file()));
    match python {
        Some(python) => (python.to_string_lossy().to_string(), vec![idf_py]),
        // No python found so rely on idf.py being on the PATH (traditional activated shell)
        None => ("idf.py".to_string(), vec![]),
    }
}

/// ESP-IDF variables of this process (from a different active ESP-IDF) which the captured environment
/// doesn't replace and so must be removed when idf.py is run
fn stale_idf_env_vars(captured: &HashMap<String, String>) -> Vec<String> {
    idf_env_vars_to_remove(&std::env::vars().collect()).into_iter()
        .filter(|name| !captured.contains_key(name)).collect()
}

/// Get everything needed to run idf.py for an install
pub fn prepare_idf_run(install: &IdfInstall) -> Result<IdfRunSetup, String> {
    let is_windows = cfg!(target_os = "windows");
    match &install.kind {
        IdfKind::ActiveEnv => {
            let python_env = std::env::var("IDF_PYTHON_ENV_PATH").ok();
            let (program, leading_args) = idf_py_command(&install.idf_path, None, python_env.as_deref(), is_windows);
            Ok(IdfRunSetup { env_vars: HashMap::new(), env_vars_to_remove: vec![], program, leading_args })
        }
        IdfKind::Legacy => {
            let env_vars = capture_legacy_env(&install.idf_path)?;
            let (program, leading_args) = idf_py_command(&install.idf_path, None,
                        env_vars.get("IDF_PYTHON_ENV_PATH").map(|s| s.as_str()), is_windows);
            let env_vars_to_remove = stale_idf_env_vars(&env_vars);
            Ok(IdfRunSetup { env_vars, env_vars_to_remove, program, leading_args })
        }
        IdfKind::Eim { name, activation_script, python } => {
            let script = activation_script.as_ref()
                .ok_or_else(|| format!("the EIM installation \"{}\" has no activation script - try \"eim fix\"", name))?;
            if !script.is_file() {
                return Err(format!("the EIM activation script {} does not exist - try \"eim fix\"", script.display()));
            }
            let env_vars = capture_eim_env(script)?;
            let (program, leading_args) = idf_py_command(&install.idf_path, python.as_deref(),
                        env_vars.get("IDF_PYTHON_ENV_PATH").map(|s| s.as_str()), is_windows);
            let env_vars_to_remove = stale_idf_env_vars(&env_vars);
            Ok(IdfRunSetup { env_vars, env_vars_to_remove, program, leading_args })
        }
    }
}

/// Load the EIM manifest: --eim-json option, RAFT_EIM_IDF_JSON environment variable then the default location
/// Returns the manifest (if any) and a warning (if the file exists but can't be used)
pub fn load_eim_manifest(cli_path: Option<&str>) -> (Option<EimManifest>, Option<String>) {
    let is_windows = cfg!(target_os = "windows");
    let explicit = cli_path.map(|s| s.to_string()).or_else(|| std::env::var(RAFT_EIM_IDF_JSON_ENV).ok());
    let path = match &explicit {
        Some(path) => PathBuf::from(path),
        None => match default_eim_tools_folder(is_windows, dirs::home_dir().as_deref()) {
            Some(folder) => folder.join("eim_idf.json"),
            None => return (None, None),
        },
    };
    match fs::read_to_string(&path) {
        Ok(content) => match parse_eim_manifest(&content) {
            Ok(manifest) => (Some(manifest), None),
            Err(e) => (None, Some(format!("Warning: the EIM manifest {} could not be read: {}", path.display(), e))),
        },
        Err(_) if explicit.is_some() => (None, Some(format!("Warning: the EIM manifest {} does not exist", path.display()))),
        Err(_) => (None, None),
    }
}

/// Folders in which ESP-IDF has traditionally been installed
pub fn default_legacy_roots() -> Vec<PathBuf> {
    if cfg!(target_os = "windows") {
        vec![PathBuf::from(r"C:\Espressif\frameworks")]
    } else {
        dirs::home_dir().map(|h| vec![h.join("esp")]).unwrap_or_default()
    }
}

/////////////////////////////////////////////////////////////////////////////////////////////////
// Tests
/////////////////////////////////////////////////////////////////////////////////////////////////

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    // Temporary folder which is removed when dropped
    struct TempFolder(PathBuf);
    static TEMP_COUNTER: AtomicU32 = AtomicU32::new(0);
    impl TempFolder {
        fn new() -> TempFolder {
            let path = std::env::temp_dir().join(format!("raftcli_test_{}_{}",
                        std::process::id(), TEMP_COUNTER.fetch_add(1, Ordering::SeqCst)));
            fs::create_dir_all(&path).unwrap();
            TempFolder(path)
        }
        fn path(&self) -> &Path { &self.0 }
    }
    impl Drop for TempFolder {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
    }

    // Create a folder which looks like an ESP-IDF of the given version
    fn make_idf(folder: &Path, major: u32, minor: u32, patch: u32) {
        fs::create_dir_all(folder.join("tools").join("cmake")).unwrap();
        fs::write(folder.join("export.sh"), "").unwrap();
        fs::write(folder.join("tools").join("idf.py"), "").unwrap();
        fs::write(folder.join("tools").join("cmake").join("version.cmake"),
            format!("set(IDF_VERSION_MAJOR {})\nset(IDF_VERSION_MINOR {})\nset(IDF_VERSION_PATCH {})\nset(ENV{{IDF_VERSION}} \"x\")\n",
                    major, minor, patch)).unwrap();
    }

    // Test machine with a legacy root and an EIM install root + tools folder
    struct Machine {
        temp: TempFolder,
    }
    impl Machine {
        fn new() -> Machine {
            let machine = Machine { temp: TempFolder::new() };
            fs::create_dir_all(machine.legacy_root()).unwrap();
            fs::create_dir_all(machine.eim_tools()).unwrap();
            machine
        }
        fn legacy_root(&self) -> PathBuf { self.temp.path().join("esp") }
        fn eim_root(&self) -> PathBuf { self.temp.path().join("dot_espressif") }
        fn eim_tools(&self) -> PathBuf { self.eim_root().join("tools") }
        fn add_legacy(&self, folder_name: &str, version: (u32, u32, u32)) -> PathBuf {
            let folder = self.legacy_root().join(folder_name);
            make_idf(&folder, version.0, version.1, version.2);
            folder
        }
        // Adds the ESP-IDF folder and activation script and returns the manifest entry
        fn add_eim(&self, name: &str, version: (u32, u32, u32), status: Option<&str>) -> EimInstallation {
            let folder = self.eim_root().join(name).join("esp-idf");
            make_idf(&folder, version.0, version.1, version.2);
            let script = self.eim_tools().join(format!("activate_idf_{}.sh", name));
            fs::write(&script, "").unwrap();
            EimInstallation {
                activation_script: Some(script.to_string_lossy().to_string()),
                id: format!("id-{}", name),
                idf_tools_path: self.eim_tools().to_string_lossy().to_string(),
                name: name.to_string(),
                path: folder.to_string_lossy().to_string(),
                python: None,
                status: status.map(|s| s.to_string()),
            }
        }
        fn locate(&self, required: &str, explicit: Option<&str>, saved: Option<&str>, env_idf_path: Option<&str>,
                    manifest: Option<&EimManifest>) -> Result<IdfInstall, LocateError> {
            let required = RequiredVersion::new(required, "test");
            locate_esp_idf(&LocatorInputs {
                required: &required,
                explicit,
                saved_explicit: saved,
                env_idf_path,
                legacy_roots: vec![self.legacy_root()],
                eim_manifest: manifest,
                eim_tools_folder: Some(self.eim_tools()),
                eim_install_roots: vec![self.eim_root()],
                is_windows: false,
            })
        }
    }

    fn manifest_of(entries: Vec<EimInstallation>, selected: Option<&str>) -> EimManifest {
        EimManifest { idf_installed: entries, idf_selected_id: selected.map(|s| s.to_string()), eim_path: None, version: None }
    }

    fn is_eim_named(install: &IdfInstall, expected: &str) -> bool {
        matches!(&install.kind, IdfKind::Eim { name, .. } if name == expected)
    }

    // ---- Versions

    #[test]
    fn version_parsing() {
        let v = |major, minor, patch| Some(IdfVersion { major, minor, patch });
        assert_eq!(IdfVersion::parse("6.0.2"), v(6, 0, 2));
        assert_eq!(IdfVersion::parse("v6.1"), v(6, 1, 0));
        assert_eq!(IdfVersion::parse("v5.4.2-dirty"), v(5, 4, 2));
        assert_eq!(IdfVersion::parse("release-v6.0"), v(6, 0, 0));
        assert_eq!(IdfVersion::parse("esp-idf-v6.0.2"), v(6, 0, 2));
        assert_eq!(IdfVersion::parse("latest"), None);
        assert_eq!(IdfVersion::parse(""), None);
    }

    #[test]
    fn required_version_forms() {
        let required = RequiredVersion::new(" v6.1 ", "test");
        assert_eq!(required.as_written, "6.1");
        assert_eq!(required.docker_tag(), "v6.1");      // There is no v6.1.0 docker image
        assert_eq!(required.for_cmake(), "6.1.0");
        assert!(required.matches(&IdfVersion { major: 6, minor: 1, patch: 0 }));
        assert!(!required.matches(&IdfVersion { major: 6, minor: 1, patch: 1 }));
        let latest = RequiredVersion::new("latest", "test");
        assert_eq!(latest.docker_tag(), "latest");
        assert_eq!(latest.version, None);
    }

    #[test]
    fn version_cmake_parsing() {
        let content = "set(IDF_VERSION_MAJOR 6)\r\nset(IDF_VERSION_MINOR 1)\r\nset(IDF_VERSION_PATCH 0)\r\n";
        assert_eq!(parse_version_cmake(content), Some(IdfVersion { major: 6, minor: 1, patch: 0 }));
        assert_eq!(parse_version_cmake("nothing here"), None);
    }

    // ---- features.cmake

    #[test]
    fn features_cmake_version() {
        let parse = |content: &str| parse_features_cmake_version(content);
        assert_eq!(parse("set(IDF_TARGET \"esp32s3\")\nset(ESP_IDF_VERSION \"6.0.2\")\n"), Ok(Some("6.0.2".to_string())));
        assert_eq!(parse("  set( ESP_IDF_VERSION   v6.1 )  # comment\r\n"), Ok(Some("v6.1".to_string())));
        assert_eq!(parse("# set(ESP_IDF_VERSION \"6.0.2\")\n"), Ok(None));
        assert_eq!(parse("set(IDF_TARGET \"esp32s3\")\n"), Ok(None));
        // Must not be confused by a variable with a longer name
        assert_eq!(parse("set(ESP_IDF_VERSION_EXTRA \"1\")\n"), Ok(None));
        // The last definition wins
        assert_eq!(parse("set(ESP_IDF_VERSION \"5.4.2\")\nset(ESP_IDF_VERSION \"6.0.2\")\n"), Ok(Some("6.0.2".to_string())));
        // A variable reference can't be evaluated
        assert!(parse("set(ESP_IDF_VERSION \"${MY_VERSION}\")\n").is_err());
    }

    // ---- Required version precedence

    fn inputs<'a>(cli: Option<&'a str>, sys_type: Option<&'a str>, common: Option<&'a str>, dockerfile: Option<&'a str>)
                -> RequiredVersionInputs<'a> {
        RequiredVersionInputs { cli_version: cli, sys_type_name: "MySysType", sys_type_features: sys_type,
                    common_features: common, dockerfile, default_version: "6.0.2" }
    }

    #[test]
    fn required_version_precedence() {
        let sys_type = "set(ESP_IDF_VERSION \"6.1\")\n";
        let common = "set(ESP_IDF_VERSION \"5.5.1\")\n";
        let dockerfile = "FROM espressif/idf:v5.4.2\nWORKDIR /project\n";
        let check = |inputs: RequiredVersionInputs, version: &str, source_contains: &str| {
            let required = resolve_required_version(&inputs).unwrap();
            assert_eq!(required.as_written, version);
            assert!(required.source.contains(source_contains), "{}", required.source);
        };
        check(inputs(Some("v6.0"), Some(sys_type), Some(common), Some(dockerfile)), "6.0", "--idf-version");
        check(inputs(None, Some(sys_type), Some(common), Some(dockerfile)), "6.1", "systypes/MySysType/features.cmake");
        check(inputs(None, Some("set(IDF_TARGET \"esp32\")\n"), Some(common), Some(dockerfile)), "5.5.1", "Common");
        check(inputs(None, None, None, Some(dockerfile)), "5.4.2", "Dockerfile");
        check(inputs(None, None, None, None), "6.0.2", "default");
        // A placeholder Dockerfile doesn't provide a version (unless the ARG has a default)
        check(inputs(None, None, None, Some("ARG ESP_IDF_VERSION\nFROM espressif/idf:${ESP_IDF_VERSION}\n")), "6.0.2", "default");
        check(inputs(None, None, None, Some("ARG ESP_IDF_VERSION=v5.3\nFROM espressif/idf:${ESP_IDF_VERSION}\n")), "5.3", "ARG default");
        // An unusable value is an error rather than being ignored
        assert!(resolve_required_version(&inputs(None, Some("set(ESP_IDF_VERSION ${X})\n"), None, None)).is_err());
    }

    // ---- Dockerfile

    #[test]
    fn dockerfile_classification() {
        assert_eq!(classify_dockerfile(None), DockerfileKind::Missing);
        assert_eq!(classify_dockerfile(Some("FROM ubuntu:24.04\n")), DockerfileKind::Custom);
        assert_eq!(classify_dockerfile(Some("FROM espressif/idf:v6.0.2\nWORKDIR /project\n")),
                    DockerfileKind::Literal { tag: "v6.0.2".to_string() });
        assert_eq!(classify_dockerfile(Some("# comment\nFROM --platform=linux/amd64 espressif/idf:v6.1 AS builder\n")),
                    DockerfileKind::Literal { tag: "v6.1".to_string() });
        assert_eq!(classify_dockerfile(Some("ARG ESP_IDF_VERSION\r\nFROM espressif/idf:${ESP_IDF_VERSION}\r\n")),
                    DockerfileKind::Placeholder { default_tag: None });
    }

    #[test]
    fn dockerfile_rewrite_changes_only_the_tag() {
        let original = "# My build\r\nFROM --platform=linux/amd64 espressif/idf:v6.0.1 AS builder\r\nWORKDIR /project\r\nRUN echo espressif/idf:v6.0.1\r\n";
        let rewritten = rewrite_dockerfile_tag(original, "v6.0.2").unwrap();
        assert_eq!(rewritten, original.replacen("espressif/idf:v6.0.1 AS", "espressif/idf:v6.0.2 AS", 1));
        assert_eq!(rewrite_dockerfile_tag("FROM ubuntu:24.04\n", "v6.0.2"), None);
    }

    #[test]
    fn docker_build_plans() {
        let from_sys_type = RequiredVersion::new("6.0.2", "systypes/X/features.cmake");

        // Placeholder - the version is passed as a build argument and nothing is generated
        let plan = plan_docker_build(Some("ARG ESP_IDF_VERSION\nFROM espressif/idf:${ESP_IDF_VERSION}\n"), &from_sys_type).unwrap();
        assert_eq!(plan.generated_dockerfile, None);
        assert_eq!(plan.build_args, vec!["--build-arg".to_string(), "ESP_IDF_VERSION=v6.0.2".to_string()]);
        assert_eq!(plan.image_tag, "raftbuilder:idf-6.0.2");

        // Literal with the same version - exactly as before
        let plan = plan_docker_build(Some("FROM espressif/idf:v6.0.2\n"), &from_sys_type).unwrap();
        assert_eq!((plan.generated_dockerfile, plan.build_args.len(), plan.note), (None, 0, None));

        // Literal with a different version - a copy is generated
        let plan = plan_docker_build(Some("FROM espressif/idf:v6.0.1\nWORKDIR /project\n"), &from_sys_type).unwrap();
        assert_eq!(plan.generated_dockerfile, Some("FROM espressif/idf:v6.0.2\nWORKDIR /project\n".to_string()));
        assert!(plan.note.is_some());

        // v6.1 and v6.1.0 are the same version
        let plan = plan_docker_build(Some("FROM espressif/idf:v6.1\n"), &RequiredVersion::new("6.1.0", "systypes/X/features.cmake")).unwrap();
        assert_eq!(plan.generated_dockerfile, None);

        // Custom base image - an error if a version is asked for but otherwise used as it is
        assert!(plan_docker_build(Some("FROM ubuntu:24.04\n"), &from_sys_type).is_err());
        let plan = plan_docker_build(Some("FROM ubuntu:24.04\n"), &RequiredVersion::new("6.0.2", "RaftCLI default - nothing set")).unwrap();
        assert_eq!(plan.image_tag, "raftbuilder");
        assert!(plan_docker_build(None, &from_sys_type).is_err());
    }

    // ---- EIM manifest

    const REAL_MANIFEST: &str = r#"{
        "gitPath": "/usr/bin/git",
        "idfInstalled": [
            {
            "activationScript": "/home/rob/.espressif/tools/activate_idf_v6.1.sh",
            "id": "esp-idf-6e7e734d2bc241c7863bf882c6173f1f",
            "idfToolsPath": "/home/rob/.espressif/tools",
            "name": "v6.1",
            "path": "/home/rob/.espressif/v6.1/esp-idf",
            "python": "/home/rob/.espressif/tools/python/v6.1/venv/bin/python",
            "installationConfig": "AAAA",
            "status": "finished"
            }
        ],
        "idfSelectedId": "esp-idf-6e7e734d2bc241c7863bf882c6173f1f",
        "eimPath": "/usr/bin/eim",
        "version": "3.0"
    }"#;

    #[test]
    fn eim_manifest_parsing() {
        let manifest = parse_eim_manifest(REAL_MANIFEST).unwrap();
        assert_eq!(manifest.idf_installed.len(), 1);
        assert_eq!(manifest.idf_installed[0].name, "v6.1");
        assert_eq!(manifest.idf_installed[0].python.as_deref(), Some("/home/rob/.espressif/tools/python/v6.1/venv/bin/python"));
        assert!(manifest.idf_installed[0].is_usable());
        assert_eq!(manifest.idf_selected_id.as_deref(), Some("esp-idf-6e7e734d2bc241c7863bf882c6173f1f"));

        // Older/minimal manifests, Windows paths and unknown fields
        let minimal = parse_eim_manifest(r#"{"idfInstalled":[{"name":"v5.4.2","path":"C:\\esp\\v5.4.2\\esp-idf","idfToolsPath":"C:\\Espressif\\tools","id":"x","newField":1}]}"#).unwrap();
        assert!(minimal.idf_installed[0].is_usable());
        assert_eq!(minimal.idf_installed[0].activation_script, None);
        for status in ["in_progress", "failed", "being_repaired", "broken"] {
            let entry = EimInstallation { status: Some(status.to_string()), path: "/x".to_string(), ..minimal.idf_installed[0].clone() };
            assert!(!entry.is_usable(), "{}", status);
        }
        assert_eq!(parse_eim_manifest(r#"{"idfInstalled":[]}"#).unwrap().idf_installed.len(), 0);
        assert!(parse_eim_manifest("not json").is_err());
    }

    // ---- Locating

    #[test]
    fn locate_legacy_only() {
        let machine = Machine::new();
        machine.add_legacy("esp-idf-v5.4.2", (5, 4, 2));
        let wanted = machine.add_legacy("esp-idf-v6.0.2", (6, 0, 2));
        let install = machine.locate("6.0.2", None, None, None, None).unwrap();
        assert_eq!((install.idf_path, install.kind), (wanted, IdfKind::Legacy));
    }

    #[test]
    fn locate_legacy_by_folder_name_when_version_file_is_missing() {
        let machine = Machine::new();
        let folder = machine.add_legacy("esp-idf-v6.0.2", (6, 0, 2));
        fs::remove_file(folder.join("tools").join("cmake").join("version.cmake")).unwrap();
        assert_eq!(machine.locate("6.0.2", None, None, None, None).unwrap().idf_path, folder);
    }

    #[test]
    fn locate_eim_only() {
        let machine = Machine::new();
        let manifest = manifest_of(vec![machine.add_eim("v6.1", (6, 1, 0), Some("finished"))], None);
        let install = machine.locate("6.1", None, None, None, Some(&manifest)).unwrap();
        assert!(is_eim_named(&install, "v6.1"));
        assert_eq!(install.version, Some(IdfVersion { major: 6, minor: 1, patch: 0 }));
    }

    #[test]
    fn locate_eim_by_real_version_not_by_name() {
        let machine = Machine::new();
        let manifest = manifest_of(vec![machine.add_eim("my-favourite", (6, 0, 2), None)], None);
        assert!(is_eim_named(&machine.locate("6.0.2", None, None, None, Some(&manifest)).unwrap(), "my-favourite"));
    }

    #[test]
    fn locate_both_kinds_by_version() {
        let machine = Machine::new();
        let legacy = machine.add_legacy("esp-idf-v6.0.2", (6, 0, 2));
        let manifest = manifest_of(vec![machine.add_eim("v6.1", (6, 1, 0), None)], None);
        assert_eq!(machine.locate("6.0.2", None, None, None, Some(&manifest)).unwrap().idf_path, legacy);
        assert!(is_eim_named(&machine.locate("6.1.0", None, None, None, Some(&manifest)).unwrap(), "v6.1"));
    }

    #[test]
    fn locate_prefers_legacy_when_both_have_the_version() {
        let machine = Machine::new();
        let legacy = machine.add_legacy("esp-idf-v6.0.2", (6, 0, 2));
        let manifest = manifest_of(vec![machine.add_eim("v6.0.2", (6, 0, 2), None)], None);
        assert_eq!(machine.locate("6.0.2", None, None, None, Some(&manifest)).unwrap().idf_path, legacy);
    }

    #[test]
    fn locate_prefers_selected_eim_install() {
        let machine = Machine::new();
        let first = machine.add_eim("first", (6, 0, 2), None);
        let second = machine.add_eim("second", (6, 0, 2), None);
        let manifest = manifest_of(vec![first.clone(), second.clone()], Some("id-second"));
        assert!(is_eim_named(&machine.locate("6.0.2", None, None, None, Some(&manifest)).unwrap(), "second"));
        let manifest = manifest_of(vec![first, second], None);
        assert!(is_eim_named(&machine.locate("6.0.2", None, None, None, Some(&manifest)).unwrap(), "first"));
    }

    #[test]
    fn locate_ignores_unusable_eim_installs() {
        let machine = Machine::new();
        let manifest = manifest_of(vec![machine.add_eim("v6.0.2", (6, 0, 2), Some("failed"))], None);
        // Not taken from the manifest and not found by scanning the install folders either
        assert!(machine.locate("6.0.2", None, None, None, Some(&manifest)).is_err());
        // ... but it is found by scanning if there is no manifest at all
        assert!(is_eim_named(&machine.locate("6.0.2", None, None, None, None).unwrap(), "v6.0.2"));
    }

    #[test]
    fn idf_env_vars_of_another_active_idf_are_removed() {
        let temp = TempFolder::new();
        let env_of = |pairs: &[(&str, &str)]| -> HashMap<String, String> {
            pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
        };
        // Nothing active
        assert!(idf_env_vars_to_remove(&env_of(&[("PATH", "/usr/bin")])).is_empty());

        // A user's own IDF_TOOLS_PATH is kept (it isn't an EIM tools folder)
        let tools = temp.path().to_string_lossy().to_string();
        let mut removed = idf_env_vars_to_remove(&env_of(&[("IDF_PATH", "/x"), ("IDF_PYTHON_ENV_PATH", "/y"), ("IDF_TOOLS_PATH", &tools)]));
        removed.sort();
        assert_eq!(removed, vec!["IDF_PATH".to_string(), "IDF_PYTHON_ENV_PATH".to_string()]);

        // IDF_TOOLS_PATH set by EIM activation (the folder has the EIM manifest in it) is removed
        fs::write(temp.path().join("eim_idf.json"), "{}").unwrap();
        let removed = idf_env_vars_to_remove(&env_of(&[("IDF_TOOLS_PATH", &tools)]));
        assert_eq!(removed, vec!["IDF_TOOLS_PATH".to_string()]);
    }

    #[test]
    fn locate_eim_without_manifest() {
        let machine = Machine::new();
        machine.add_eim("v6.1", (6, 1, 0), None);
        let install = machine.locate("6.1", None, None, None, None).unwrap();
        assert!(is_eim_named(&install, "v6.1"));
        assert!(matches!(&install.kind, IdfKind::Eim { activation_script: Some(script), .. } if script.ends_with("activate_idf_v6.1.sh")));
    }

    #[test]
    fn locate_nothing_matching_lists_what_was_found() {
        let machine = Machine::new();
        machine.add_legacy("esp-idf-v5.4.2", (5, 4, 2));
        let manifest = manifest_of(vec![machine.add_eim("v6.1", (6, 1, 0), None)], None);
        let error = machine.locate("6.0.2", None, None, None, Some(&manifest)).unwrap_err();
        assert!(error.message.contains("6.0.2"));
        assert_eq!(error.found.len(), 2);
    }

    #[test]
    fn locate_explicit_path_name_and_parent_folder() {
        let machine = Machine::new();
        let legacy = machine.add_legacy("esp-idf-v5.4.2", (5, 4, 2));
        let entry = machine.add_eim("v6.1", (6, 1, 0), None);
        let manifest = manifest_of(vec![entry.clone()], None);

        // An explicit ESP-IDF folder is used whatever version is required
        let install = machine.locate("6.0.2", Some(legacy.to_str().unwrap()), None, None, Some(&manifest)).unwrap();
        assert_eq!((install.idf_path, install.kind), (legacy.clone(), IdfKind::Legacy));

        // An explicit path to an EIM install is recognised as EIM
        assert!(is_eim_named(&machine.locate("6.1", Some(&entry.path), None, None, Some(&manifest)).unwrap(), "v6.1"));

        // The name of an EIM install
        assert!(is_eim_named(&machine.locate("6.0.2", Some("v6.1"), None, None, Some(&manifest)).unwrap(), "v6.1"));

        // A folder containing ESP-IDF folders - the required version is picked
        let parent = machine.legacy_root();
        machine.add_legacy("esp-idf-v6.0.2", (6, 0, 2));
        let install = machine.locate("6.0.2", Some(parent.to_str().unwrap()), None, None, None).unwrap();
        assert!(install.idf_path.ends_with("esp-idf-v6.0.2"));

        // Something that isn't any of these is an error (and nothing else is searched)
        assert!(machine.locate("6.0.2", Some("no-such-thing"), None, None, Some(&manifest)).is_err());
    }

    #[test]
    fn locate_active_environment_and_saved_path() {
        let machine = Machine::new();
        let active = machine.add_legacy("esp-idf-v6.0.2", (6, 0, 2));
        let other = machine.add_legacy("esp-idf-v5.4.2", (5, 4, 2));

        // The active environment is used if it is the required version
        let install = machine.locate("6.0.2", None, None, Some(active.to_str().unwrap()), None).unwrap();
        assert_eq!(install.kind, IdfKind::ActiveEnv);

        // ... and not if it is the wrong version (the right one is searched for)
        let install = machine.locate("6.0.2", None, None, Some(other.to_str().unwrap()), None).unwrap();
        assert_eq!((install.idf_path, install.kind), (active.clone(), IdfKind::Legacy));

        // A matching active environment is preferred to a saved path, a saved path is preferred to searching
        let install = machine.locate("6.0.2", None, Some(other.to_str().unwrap()), Some(active.to_str().unwrap()), None).unwrap();
        assert_eq!(install.kind, IdfKind::ActiveEnv);
        let install = machine.locate("6.0.2", None, Some(other.to_str().unwrap()), None, None).unwrap();
        assert_eq!(install.idf_path, other);
        assert!(install.origin.contains("saved"));

        // A saved path which no longer exists is ignored
        let install = machine.locate("6.0.2", None, Some("/no/longer/here"), None, None).unwrap();
        assert_eq!(install.idf_path, active);

        // The -e option beats everything
        let install = machine.locate("6.0.2", Some(other.to_str().unwrap()), None, Some(active.to_str().unwrap()), None).unwrap();
        assert_eq!(install.idf_path, other);
    }

    // ---- Environment

    #[test]
    fn eim_env_output_parsing() {
        let output = "Some banner line\r\nPATH=/tools/a:/tools/b\r\nSYSTEM_PATH=/usr/bin\r\nESP_IDF_VERSION=6.1\r\n\
                    IDF_PATH=/home/me/.espressif/v6.1/esp-idf\r\nODD=a=b=c\r\nnot a variable=1\r\n\r\n";
        let env_vars = parse_eim_env_output(output, "/usr/bin:/bin", ":");
        assert_eq!(env_vars.get("PATH").unwrap(), "/tools/a:/tools/b:/usr/bin:/bin");
        assert_eq!(env_vars.get("IDF_PATH").unwrap(), "/home/me/.espressif/v6.1/esp-idf");
        assert_eq!(env_vars.get("ODD").unwrap(), "a=b=c");
        assert!(!env_vars.contains_key("SYSTEM_PATH"));
        assert_eq!(env_vars.len(), 4);
        let windows = parse_eim_env_output("PATH=C:\\tools\\a;C:\\tools\\b\r\n", "C:\\Windows", ";");
        assert_eq!(windows.get("PATH").unwrap(), "C:\\tools\\a;C:\\tools\\b;C:\\Windows");
    }

    #[test]
    fn idf_env_validation() {
        let temp = TempFolder::new();
        let mut env_vars = HashMap::new();
        assert!(validate_idf_env(&env_vars).is_err());
        env_vars.insert("IDF_PATH".to_string(), "/some/idf".to_string());
        // This is what export.sh produces for an EIM install - IDF_PATH but no python environment
        assert!(validate_idf_env(&env_vars).is_err());
        env_vars.insert("IDF_PYTHON_ENV_PATH".to_string(), temp.path().join("missing").to_string_lossy().to_string());
        assert!(validate_idf_env(&env_vars).is_err());
        env_vars.insert("IDF_PYTHON_ENV_PATH".to_string(), temp.path().to_string_lossy().to_string());
        assert!(validate_idf_env(&env_vars).is_ok());
    }

    #[test]
    fn idf_py_is_run_with_python() {
        let temp = TempFolder::new();
        let idf_path = temp.path().join("esp-idf");
        let idf_py = idf_path.join("tools").join("idf.py").to_string_lossy().to_string();

        // Python from the EIM manifest
        let python = temp.path().join("python");
        fs::write(&python, "").unwrap();
        assert_eq!(idf_py_command(&idf_path, Some(&python), None, false), (python.to_string_lossy().to_string(), vec![idf_py.clone()]));

        // Python from IDF_PYTHON_ENV_PATH (Linux/macOS and Windows layouts)
        for (is_windows, sub_path) in [(false, vec!["bin", "python"]), (true, vec!["Scripts", "python.exe"])] {
            let venv = temp.path().join(format!("venv_{}", is_windows));
            let venv_python = sub_path.iter().fold(venv.clone(), |path, part| path.join(part));
            fs::create_dir_all(venv_python.parent().unwrap()).unwrap();
            fs::write(&venv_python, "").unwrap();
            assert_eq!(idf_py_command(&idf_path, None, venv.to_str(), is_windows),
                        (venv_python.to_string_lossy().to_string(), vec![idf_py.clone()]));
        }

        // Nothing known - rely on idf.py being on the PATH as before
        assert_eq!(idf_py_command(&idf_path, None, None, false), ("idf.py".to_string(), vec![]));
    }
}
