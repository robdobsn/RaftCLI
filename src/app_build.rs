
use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::fs;
use std::io;
use std::path::Path;
#[cfg(unix)]
use nix::unistd::{getuid, getgid};
use crate::raft_cli_utils::{default_esp_idf_version, is_docker_available, utils_get_sys_type, write_build_info, read_build_info, BuildInfo};
use crate::raft_cli_utils::check_app_folder_valid;
use crate::raft_cli_utils::{execute_and_capture_output, execute_and_capture_output_env};
use crate::raft_cli_utils::convert_path_for_docker;
use crate::raft_cli_utils::CommandError;
use crate::esp_idf::{self, IdfInstall, IdfKind, LocatorInputs, RequiredVersion};

/// Options which control which ESP-IDF is used for a build
#[derive(Debug, Clone, Default)]
pub struct IdfBuildOptions {
    /// -i option: find and use a local ESP-IDF of the required version
    pub use_local_idf: bool,
    /// -e option: path to an ESP-IDF folder (or the name of an EIM installation)
    pub idf_path_or_name: Option<String>,
    /// --idf-version option: overrides the version from features.cmake / Dockerfile
    pub idf_version: Option<String>,
    /// --eim-json option: location of the EIM manifest (eim_idf.json)
    pub eim_json: Option<String>,
}

pub fn build_raft_app(build_sys_type: &Option<String>, clean: bool, clean_only: bool, app_folder: String,
            force_docker_arg: bool, no_docker_arg: bool,
            idf_options: IdfBuildOptions)
                            -> Result<String, Box<dyn std::error::Error>> {

    // println!("Building the app in folder: {} clean {} clean_only {} no_docker_arg {}", app_folder, clean, clean_only, no_docker_arg);

    // Check the app folder is valid
    if !check_app_folder_valid(app_folder.clone()) {
        return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "Invalid app folder")));
    }

    // Determine the Systype to build
    let sys_type = utils_get_sys_type(build_sys_type, app_folder.clone());
    if sys_type.is_err() {
        return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "Error determining SysType")));
    }
    let sys_type = sys_type.unwrap();

    // Determine the ESP-IDF version required - this can be set for the SysType (or for all SysTypes) in
    // features.cmake and otherwise comes from the Dockerfile
    let required_version = esp_idf::get_required_version(&app_folder, &sys_type,
                idf_options.idf_version.as_deref(), &default_esp_idf_version())
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    println!("Required ESP-IDF {} (from {})", required_version.as_written, required_version.source);

    // Warn if the project downloads a RaftBootstrap.cmake which doesn't match the RaftCore version it uses
    if let Some(warning) = crate::bootstrap_check::check_bootstrap_version(&app_folder, &sys_type) {
        println!("{}", warning);
    }

    // Read previous build information
    let build_info = read_build_info(&app_folder);

    // Flags indicating the build folder should be deleted
    let mut delete_build_folder = clean || clean_only;

    // A build folder configured with one version of the ESP-IDF can't be used with another
    if let Some(last_version) = build_info.idf_versions.get(&sys_type) {
        if *last_version != required_version.for_cmake() && !delete_build_folder {
            println!("The build folder for SysType {} was built with ESP-IDF {} and {} is now required so it will be deleted",
                        sys_type, last_version, required_version.for_cmake());
            delete_build_folder = true;
        }
    }

    // Determine if docker is to be used for build
    let mut no_docker = std::env::var("RAFT_NO_DOCKER").unwrap_or("false".to_string()) == "true";
    if no_docker_arg {
        no_docker = true;
    }

    // Determine if docker is to be forced for build
    let mut force_docker = std::env::var("RAFT_FORCE_DOCKER").unwrap_or("false".to_string()) == "true";
    if force_docker_arg {
        force_docker = true;
    }

    // Apply saved build method preference if no explicit flags set
    if !no_docker_arg && !force_docker_arg && !idf_options.use_local_idf && idf_options.idf_path_or_name.is_none() {
        if let Some(ref last_method) = build_info.last_build_method {
            if last_method == "docker" {
                if is_docker_available() {
                    force_docker = true;
                } else {
                    println!("Warning: Previous build used Docker but Docker is not available, falling back to local IDF");
                }
            } else if last_method == "local_idf" {
                no_docker = true;
            }
        }
    }

    // Handle building with or without docker
    let (build_result, actual_build_method, actual_idf_path, idf_path_was_explicit) =
        if idf_options.use_local_idf || (idf_options.idf_path_or_name.is_some() && !force_docker)
                    || no_docker || !is_docker_available() && !force_docker {
        // (specifying an ESP-IDF with the -e option implies a local build)

        // Explicit path saved from a previous build (only used if there is no -e option)
        let saved_explicit = if build_info.last_idf_path_explicit { build_info.last_idf_path.clone() } else { None };

        // Build without docker
        let result = build_without_docker(app_folder.clone(), sys_type.clone(), clean, clean_only,
                    delete_build_folder, &required_version, &idf_options, saved_explicit.clone());
        match result {
            Ok((output, install)) => {
                // The -e option (or a saved -e option which was used) is remembered for the next build
                let (path_to_save, explicit) = if let Some(ref explicit) = idf_options.idf_path_or_name {
                    (Some(explicit.clone()), true)
                } else if install.origin.contains("saved") {
                    (saved_explicit, true)
                } else {
                    (Some(install.idf_path.to_string_lossy().to_string()), false)
                };
                (Ok(output), "local_idf", path_to_save, explicit)
            }
            Err(e) => (Err(e), "local_idf", None, false)
        }
    } else if is_docker_available() {
        // Build with docker
        let result = build_with_docker(app_folder.clone(), sys_type.clone(), clean, clean_only,
                    delete_build_folder, &required_version);
        (result, "docker", None, false)
    } else
    {
        // Either ESP IDF or docker must be available to build
        let result = Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Either ESP IDF or Docker must be available to build",
        ));
        (result, "unknown", None, false)
    };

    // If the build failed, return the error
    if build_result.is_err() {
        return Err(Box::new(build_result.unwrap_err()));
    }

    // Save complete build info to raft.info file after successful build
    let mut idf_versions = HashMap::new();
    if !clean_only {
        idf_versions.insert(sys_type.clone(), required_version.for_cmake());
    }
    let build_updates = BuildInfo {
        last_built_systype: Some(sys_type.clone()),
        last_build_method: Some(actual_build_method.to_string()),
        last_idf_path_explicit: idf_path_was_explicit,
        last_idf_path: actual_idf_path,
        idf_versions,
        ..BuildInfo::default()
    };
    if let Err(e) = write_build_info(
        &app_folder,
        &build_updates,
    ) {
        println!("Warning: Failed to write raft.info file: {}", e);
    }

    Ok(build_result.unwrap().to_string())
}

// Build with docker and return output as a string
fn build_with_docker(project_dir: String, systype_name: String, clean: bool, clean_only: bool,
            delete_build_folder: bool, required_version: &RequiredVersion) -> Result<String, std::io::Error> {

    // Build with docker
    println!("Raft build SysType {} in {}{}",  systype_name, project_dir.clone(),
                    if clean { " (clean first)" } else { "" });

    // Decide how the Docker image is to be built so that it has the required ESP-IDF version
    // The project's Dockerfile is never modified - if it specifies a different version then a copy is generated
    let dockerfile_content = fs::read_to_string(Path::new(&project_dir).join("Dockerfile")).ok();
    let docker_plan = esp_idf::plan_docker_build(dockerfile_content.as_deref(), required_version)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    if let Some(ref note) = docker_plan.note {
        println!("Note: {}", note);
    }
    let mut docker_image_build_args: Vec<String> = vec!["build".to_string(), "-t".to_string(), docker_plan.image_tag.clone()];
    docker_image_build_args.extend(docker_plan.build_args.iter().cloned());
    if let Some(ref generated_dockerfile) = docker_plan.generated_dockerfile {
        // The generated Dockerfile is not placed in the SysType's build folder as that may be deleted by the build
        let generated_rel_path = format!("build/raft_docker/{}/Dockerfile", systype_name);
        let generated_path = Path::new(&project_dir).join(&generated_rel_path);
        if let Some(parent) = generated_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&generated_path, generated_dockerfile)?;
        println!("Using generated Dockerfile {}", generated_rel_path);
        docker_image_build_args.push("-f".to_string());
        docker_image_build_args.push(generated_rel_path);
    }
    docker_image_build_args.push(".".to_string());

    // Build the Docker image
    let fail_docker_image_msg = format!("Docker build command failed");
    let docker_image_build_status = Command::new("docker")
        .current_dir(project_dir.clone())
        .args(docker_image_build_args)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .expect(&fail_docker_image_msg);

    if !docker_image_build_status.success() {
        eprintln!("Docker image build command failed");
        return Err(std::io::Error::new(std::io::ErrorKind::Other, "Docker image build command failed"));
    }

    // Execute the Docker command to build the app
    let build_dir = format!("./build/{}", systype_name);
    let absolute_project_dir = fs::canonicalize(project_dir.clone())?;
    let docker_compatible_project_dir = convert_path_for_docker(absolute_project_dir);
    let project_dir_full = format!("{}:/project", docker_compatible_project_dir?);

    // Command sequence
    let mut command_sequence = String::new();

    if delete_build_folder {
        command_sequence += format!("rm -rf ./{}; ", build_dir).as_str();
    }

    command_sequence += "idf.py -B ";
    command_sequence += &build_dir;
    if clean {
        command_sequence += " fullclean";
    }
    if !clean_only {
        command_sequence += " build";
    }

    // The required version is passed to CMake (which checks it against the ESP-IDF in use)
    let version_env = format!("{}={}", esp_idf::RAFT_ESP_IDF_VERSION_ENV, required_version.for_cmake());

    // Get current user and group IDs to run Docker with same permissions as host (Unix only)
    #[cfg(unix)]
    let user_group = format!("{}:{}", getuid(), getgid());

    #[cfg(unix)]
    let docker_run_args = vec![
        "run", "--rm",
        "--user", &user_group,
        "-e", &version_env,
        "-v", &project_dir_full,
        "-w", "/project",
        &docker_plan.image_tag,
        "/bin/bash", "-c", &command_sequence,
    ];

    // On Windows, Docker handles permissions differently, so we don't need --user flag
    #[cfg(not(unix))]
    let docker_run_args = vec![
        "run", "--rm",
        "-e", &version_env,
        "-v", &project_dir_full,
        "-w", "/project",
        &docker_plan.image_tag,
        "/bin/bash", "-c", &command_sequence,
    ];

    // Convert to string vector
    let docker_run_args: Vec<String> = docker_run_args.iter().map(|s| s.to_string()).collect();

    // Print args
    // println!("Docker run args: {:?}", docker_run_args);

    // Execute the Docker command and capture its output
    let docker_command = "docker".to_string();
    match execute_and_capture_output(docker_command.clone(), &docker_run_args, project_dir.clone(), HashMap::new()) {
        Ok((output, success_flag)) => {
            if success_flag {
                // Success - return the output as a String
                Ok(output)
            } else {
                // If the command executed but was not successful, log the output and return an error
                eprintln!("Docker run failed but executed: {}", output);
                Err(io::Error::new(io::ErrorKind::Other, "Docker run executed with errors"))
            }
        },
        Err(e) => {
            // More granular error handling based on the CommandError enum
            let error_message = match e {
                CommandError::CommandNotFound(msg) => format!("Docker command not found: {}", msg),
                CommandError::ExecutionFailed(msg) => format!("Docker execution failed: {}", msg),
                CommandError::Other(io_err) => format!("An IO error occurred during Docker execution: {}", io_err),
            };
            eprintln!("Docker run failed: {}", error_message);
            Err(io::Error::new(io::ErrorKind::Other, error_message))
        }
    }
}

// Find the ESP-IDF to use for a local build
fn find_esp_idf(required_version: &RequiredVersion, idf_options: &IdfBuildOptions, saved_explicit: Option<String>)
            -> Result<IdfInstall, std::io::Error> {
    let is_windows = cfg!(target_os = "windows");
    let home_dir = dirs::home_dir();

    // ESP-IDFs installed using the Espressif Installation Manager (EIM) are listed in its manifest
    let (eim_manifest, eim_warning) = esp_idf::load_eim_manifest(idf_options.eim_json.as_deref());
    if let Some(warning) = eim_warning {
        println!("{}", warning);
    }

    let env_idf_path = std::env::var("IDF_PATH").ok();
    let locate_result = esp_idf::locate_esp_idf(&LocatorInputs {
        required: required_version,
        explicit: idf_options.idf_path_or_name.as_deref(),
        saved_explicit: saved_explicit.as_deref(),
        env_idf_path: env_idf_path.as_deref(),
        legacy_roots: esp_idf::default_legacy_roots(),
        eim_manifest: eim_manifest.as_ref(),
        eim_tools_folder: esp_idf::default_eim_tools_folder(is_windows, home_dir.as_deref()),
        eim_install_roots: esp_idf::default_eim_install_root(is_windows, home_dir.as_deref()).into_iter().collect(),
        is_windows,
    });

    locate_result.map_err(|e| {
        let mut message = e.message.clone();
        if idf_options.idf_path_or_name.is_some() && e.found.is_empty() {
            // The -e option didn't identify an ESP-IDF (nothing else is searched when it is used)
            eprintln!("{}", message);
            return io::Error::new(io::ErrorKind::Other, e.message);
        }
        if e.found.is_empty() {
            message += "\nNo ESP-IDF installations were found";
        } else {
            message += "\nESP-IDF installations found:";
            for install in &e.found {
                message += &format!("\n  {}", install.describe());
            }
        }
        message += &format!("\nTo install it with the Espressif Installation Manager use: eim install -i v{}", required_version.as_written);
        message += "\nAlternatively build using Docker (--docker) or specify an ESP-IDF using the -e option";
        eprintln!("{}", message);
        io::Error::new(io::ErrorKind::Other, e.message)
    })
}

// Build without docker
fn build_without_docker(project_dir: String, systype_name: String, clean: bool, clean_only: bool,
    delete_build_folder: bool, required_version: &RequiredVersion, idf_options: &IdfBuildOptions,
    saved_explicit: Option<String>) -> Result<(String, IdfInstall), std::io::Error> {

    // Debug
    println!(
        "Raft build SysType {} in {}{} (no Docker)",
        systype_name,
        project_dir,
        if clean { " (clean first)" } else { "" }
    );

    // Folders
    let build_dir = format!("build/{}", systype_name);

    // Delete build folder if required
    if delete_build_folder {
        let build_dir_full = format!("{}/{}", project_dir.clone(), build_dir);
        if Path::new(&build_dir_full).exists() {
            fs::remove_dir_all(&build_dir_full)?;
        }
    }

    // Find the ESP-IDF to use
    let install = find_esp_idf(required_version, idf_options, saved_explicit)?;
    println!("Using {}", install.describe());

    // The version passed to CMake (which checks it against the ESP-IDF in use) is the required version unless
    // an ESP-IDF of a different version has been specified explicitly
    let mut version_for_cmake = required_version.for_cmake();
    if let Some(install_version) = install.version {
        if !required_version.matches(&install_version) {
            println!("Warning: ESP-IDF {} is required (from {}) but the ESP-IDF specified is version {}",
                        required_version.as_written, required_version.source, install_version);
            version_for_cmake = install_version.to_string();
        }
    }

    // Get the environment and the command needed to run idf.py
    let run_setup = esp_idf::prepare_idf_run(&install).map_err(|e| {
        let hint = match install.kind {
            IdfKind::Eim { .. } => "",
            _ => " - if this ESP-IDF was downloaded manually then run its install script (install.sh / install.bat)",
        };
        io::Error::new(io::ErrorKind::Other, format!("{}{}", e, hint))
    })?;
    let mut idf_env_vars_to_add = run_setup.env_vars;
    idf_env_vars_to_add.insert(esp_idf::RAFT_ESP_IDF_VERSION_ENV.to_string(), version_for_cmake);

    // IDF args in a vector of Strings
    let mut idf_run_args = run_setup.leading_args;
    idf_run_args.push("-B".to_string());
    idf_run_args.push(build_dir);
    if clean {
        idf_run_args.push("fullclean".to_string());
    }
    if !clean_only {
        idf_run_args.push("build".to_string());
    }

    // Execute the command and handle the output
    match execute_and_capture_output_env(run_setup.program.clone(), &idf_run_args, project_dir.clone(), idf_env_vars_to_add,
                &run_setup.env_vars_to_remove) {
        Ok((output, success_flag)) => {
            if success_flag {
                Ok((output, install)) // Return the output directly
            } else {
                // If the command executed but failed, provide detailed feedback
                eprintln!("idf.py build executed but failed: {}", output);
                Err(io::Error::new(io::ErrorKind::Other, "idf.py build executed with errors"))
            }
        },
        Err(e) => {
            // Detailed error handling based on the failure
            let error_message = match e {
                CommandError::CommandNotFound(msg) => {
                    // Check if the error is due to the idf.py command not being found
                    if msg.contains("idf.py") {
                        "idf.py command was not found - see https://docs.espressif.com/projects/esp-idf/en/stable/esp32/get-started/index.html".to_string()
                    } else {
                        format!("Command not found: {}", msg)
                    }
                },
                CommandError::ExecutionFailed(msg) => format!("Execution failed: {}", msg),
                CommandError::Other(io_err) => format!("An IO error occurred: {}", io_err),
            };
            eprintln!("idf.py build failed: {}", error_message);
            Err(io::Error::new(io::ErrorKind::Other, error_message))
        }
    }
}
