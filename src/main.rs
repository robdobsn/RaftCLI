#![recursion_limit = "512"]

// RaftCLI: Main module
// Rob Dobson 2024

use clap::Parser;
mod app_new;
use app_new::generate_new_app;
mod app_config;
use app_config::get_user_input;
mod serial_monitor;
mod app_build;
use app_build::build_raft_app;
mod app_flash;
use app_flash::flash_raft_app;
mod app_ota;
use app_ota::ota_raft_app;
mod app_debug_remote;
mod terminal_io;
mod raft_cli_utils;
mod console_log;
use raft_cli_utils::is_wsl;
use raft_cli_utils::check_target_folder_valid;
use raft_cli_utils::get_flash_tool_cmd;
mod app_ports;
use app_ports::{PortsCmd, manage_ports};
mod cmd_history;

const HISTORY_FILE_NAME: &str = ".raftcli_history"; // Default name, configurable if needed

#[derive(Clone, Parser, Debug)]
enum Action {
    #[clap(name = "new", about = "Create a new raft app", alias = "n")]
    New(NewCmd),
    #[clap(name = "build", about = "Build a raft app", alias = "b")]
    Build(BuildCmd),    
    #[clap(name = "monitor", about = "Monitor a serial port", alias = "m")]
    Monitor(MonitorCmd),
    #[clap(name = "run", about = "Build, flash and monitor a raft app", alias = "r")]
    Run(RunCmd),
    #[clap(name = "flash", about = "Flash firmware to the device", alias = "f")]
    Flash(FlashCmd),
    #[clap(name = "ota", about = "Over-the-air update", alias = "o")]
    Ota(OtaCmd),
    #[clap(name = "ports", about = "Manage serial ports", alias = "p")]
    Ports(PortsCmd),
    #[clap(name = "debug", about = "Start remote debug console", alias = "d")]
    DebugRemote(DebugRemoteCmd),
    #[clap(name = "esptool", about = "Run esptool directly with arguments", alias = "e")]
    Esptool(EsptoolCmd),
}

// Define arguments specific to the `new` subcommand
#[derive(Clone, Parser, Debug)]
struct NewCmd {
    // Option to specify the app folder (second positional argument, optional)
    #[clap(help = "Path to the application folder", value_name = "APPLICATION_FOLDER")]
    base_folder: Option<String>,
    #[clap(short = 'c', long, help = "Clean the target folder")]
    clean: bool,
}

// Define arguments specific to the `build` subcommand
#[derive(Clone, Parser, Debug)]
struct BuildCmd {
    // Option to specify the app folder (second positional argument, optional)
    #[clap(help = "Path to the application folder", value_name = "APPLICATION_FOLDER")]
    app_folder: Option<String>,
    // Add an option to specify the system type
    #[clap(short = 's', long, help = "System type to build")]
    sys_type: Option<String>,
    // Option to clean the target folder
    #[clap(short = 'c', long, help = "Clean the target folder")]
    clean: bool,
    // Option to only clean and not build
    #[clap(short = 'n', long, help = "Clean only")]
    clean_only: bool,
    // Option to enable docker
    #[clap(long, help = "Use docker for build")]
    docker: bool,
    // Option to disable docker
    #[clap(long, help = "Do not use docker for build")]
    no_docker: bool,
    // Option to find matching esp idf and source it ready to build locally
    #[clap(short = 'i', long, help = "Find and use local ESP IDF matching Dockerfile version")]
    idf_local_build: bool,    
    // Option to specify path to ESP IDF folder
    #[clap(short = 'e', long, help = "Full path to ESP IDF folder for local build (when not using docker)")]
    esp_idf_path: Option<String>,
}

// Define arguments specific to the `monitor` subcommand
#[derive(Clone, Parser, Debug)]
struct MonitorCmd {
    // Option to specify the app folder (second positional argument, optional)
    #[clap(help = "Path to the application folder", value_name = "APPLICATION_FOLDER")]
    app_folder: Option<String>,
    // Add an option to specify the serial port
    #[clap(short = 'p', long, help = "Serial port")]
    port: Option<String>,
    // Option to specify the monitor baud rate
    #[clap(short = 'b', long, help = "Baud rate")]
    monitor_baud: Option<u32>,
    // Option to disable serial port reconnection when monitoring
    #[clap(short = 'r', long, help = "Disable serial port reconnection when monitoring")]
    no_reconnect: bool,
    // Option to force native serial port when in WSL
    #[clap(short = 'n', long, help = "Native serial port when in WSL")]
    native_serial_port: bool,
    // Logging options
    #[arg(short = 'l', long, help = "Log serial data to file")]
    log: bool,
    #[arg(short = 'g', long, default_value = "./logs", help = "Folder for log files")]
    log_folder: Option<String>,
    // Option to specify vendor ID
    #[clap(short = 'v', long, help = "Vendor ID")]
    vid: Option<String>,
}

// Define arguments for the 'run' subcommand
#[derive(Clone, Parser, Debug)]
struct RunCmd {
    // Add an option to specify the app folder
    app_folder: Option<String>,
    // Option to clean the system type
    #[clap(short = 's', long, help = "System type to build")]
    sys_type: Option<String>,
    // Option to clean the target folder
    #[clap(short = 'c', long, help = "Clean the target folder")]
    clean: bool,
    // Option to enable docker
    #[clap(long, help = "Use docker for build")]
    docker: bool,
    // Option to disable docker
    #[clap(long, help = "Do not use docker for build")]
    no_docker: bool,
    // Option to find matching esp idf and source it ready to build locally
    #[clap(short = 'i', long, help = "Find and use local ESP IDF matching Dockerfile version")]
    idf_local_build: bool,    
    // Option to specify path to ESP IDF folder
    #[clap(short = 'e', long, help = "Full path to ESP IDF folder for local build (when not using docker)")]
    esp_idf_path: Option<String>,
    // Add an option to specify the serial port
    #[clap(short = 'p', long, help = "Serial port")]
    port: Option<String>,
    // Add an option to specify an IP address/hostname for OTA
    #[clap(short = 'o', long, help = "IP address or hostname for OTA flashing")]
    ip_addr: Option<String>,    
    // Option to specify the monitor baud rate
    #[clap(short = 'b', long, help = "Monitor baud rate")]
    monitor_baud: Option<u32>,
    // Option to disable serial port reconnection when monitoring
    #[clap(short = 'r', long, help = "Disable serial port reconnection when monitoring")]
    no_reconnect: bool,  
    // Force native serial port when in WSL
    #[clap(short = 'n', long, help = "Native serial port when in WSL")]
    native_serial_port: bool,
    // Option to specify flash baud rate
    #[clap(short = 'f', long, help = "Flash baud rate")]
    flash_baud: Option<u32>,
    // Option to specify flashing tool
    #[clap(short = 't', long, help = "Flash tool (e.g. esptool)")]
    flash_tool: Option<String>,
    // Logging options
    #[arg(short = 'l', long, help = "Log serial data to file")]
    log: bool,
    #[arg(short = 'g', long, default_value = "./logs", help = "Folder for log files")]
    log_folder: Option<String>,
    // Option to specify vendor ID
    #[clap(short = 'v', long, help = "Vendor ID")]
    vid: Option<String>,
}

// Define arguments for the 'flash' subcommand
#[derive(Clone, Parser, Debug)]
struct FlashCmd {
    // Option to specify the app folder (second positional argument, optional)
    #[clap(help = "Path to the application folder", value_name = "APPLICATION_FOLDER")]
    app_folder: Option<String>,
    // Option to specify the system type
    #[clap(short = 's', long, help = "System type to flash")]
    sys_type: Option<String>,
    // Option to specify a serial port
    #[clap(short = 'p', long, help = "Serial port")]
    port: Option<String>,
    // Option to force native serial port when in WSL
    #[clap(short = 'n', long, help = "Native serial port when in WSL")]
    native_serial_port: bool,
    // Option to specify flash baud rate
    #[clap(short = 'f', long, help = "Flash baud rate")]
    flash_baud: Option<u32>,
    // Option to specify flashing tool
    #[clap(short = 't', long, help = "Flash tool (e.g. esptool)")]
    flash_tool: Option<String>,
    // Option to specify vendor ID
    #[clap(short = 'v', long, help = "Vendor ID")]
    vid: Option<String>,
}

// Define arguments for the 'ota' subcommand
#[derive(Clone, Parser, Debug)]
struct OtaCmd {
    // IP address/hostname for OTA (required positional argument)
    #[clap(help = "IP address or hostname for OTA", value_name = "IP_ADDRESS_OR_HOSTNAME")]
    ip_addr: String,
    // Option to specify the app folder (second positional argument, optional)
    #[clap(help = "Path to the application folder", value_name = "APPLICATION_FOLDER")]
    app_folder: Option<String>,
    // Option to specify the IP Port
    #[clap(short = 'p', long, help = "IP Port")]
    ip_port: Option<u16>,
    // Option to specify the system type
    #[clap(short = 's', long, help = "System type to ota update")]
    sys_type: Option<String>,
    // Option to use curl for OTA
    #[clap(short = 'c', long, help = "Use curl for OTA")]
    use_curl: bool,
}

// Define arguments specific to the `debug` subcommand
#[derive(Clone, Parser, Debug)]
struct DebugRemoteCmd {
    // Required positional argument for the device address
    #[clap(help = "Device address for debugging (hostname or IP)", value_name = "IP_ADDRESS_OR_HOSTNAME")]
    device_address: String,
    // Optional positional argument for the app folder
    #[clap(help = "Path to the application folder", value_name = "APPLICATION_FOLDER")]
    app_folder: Option<String>,
    // Optional argument for the port with a default value
    #[clap(short = 'p', long, help = "Port for debugging", default_value = "8080")]
    port: u16,
    #[clap(short = 'l', long, help = "Log debug console data to file")]
    log: bool,
    #[clap(short = 'g', long, default_value = "./logs", help = "Folder for log files")]
    log_folder: Option<String>,
}

// Define arguments for the `esptool` subcommand
#[derive(Clone, Parser, Debug)]
struct EsptoolCmd {
    // All arguments to pass through to esptool
    #[clap(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
    // Option to specify native serial port when in WSL
    #[clap(short = 'n', long, help = "Native serial port when in WSL")]
    native_serial_port: bool,
}

// Main CLI struct that includes the subcommands
#[derive(Parser, Debug)]
#[clap(version, author, about)]
struct Cli {
    #[clap(subcommand)]
    action: Action,
}

// Main function
fn main() {
    // Parse the command line arguments
    let args = Cli::parse();
    // println!("{:?}", args);

    // Call the function to test the templates
    match args.action {
        Action::New(cmd) => {

            // Validate target folder (before user input to avoid unnecessary input)
            let base_folder = cmd.base_folder.unwrap_or(".".to_string());
            let folder_valid = check_target_folder_valid(&base_folder, cmd.clean);
            if !folder_valid {
                println!("Error: target folder is not valid");
                std::process::exit(1);
            }
            
            // Get configuration
            let json_config_str = get_user_input(&base_folder);
            let json_config = serde_json::from_str(&json_config_str.unwrap()).unwrap();

            // Generate a new app
            let _result = generate_new_app(&base_folder, json_config).unwrap();
            // println!("{:?}", _result);

        }

        Action::Build(cmd) => {
            // Get the app folder (or default to current folder)
            let app_folder = cmd.app_folder.unwrap_or(".".to_string());
            let result = build_raft_app(&cmd.sys_type, cmd.clean, 
                        cmd.clean_only, app_folder, cmd.docker, cmd.no_docker, 
                        cmd.idf_local_build, cmd.esp_idf_path);
            // println!("{:?}", result);

            // Check for build error
            if result.is_err() {
                println!("Build failed {:?}", result);
                std::process::exit(1);
            }
        }
        
        Action::Monitor(cmd) => {

            let app_folder = cmd.app_folder.unwrap_or(".".to_string());
            let monitor_baud = cmd.monitor_baud.unwrap_or(115200);
            let log = cmd.log;
            let mut log_folder = cmd.log_folder.unwrap_or("./logs".to_string());
            // If the log_folder is relative then apply the app_folder as a prefix to it using path::join
            if !log_folder.starts_with("/") {
                let mut log_folder_path = std::path::PathBuf::from(&app_folder);
                log_folder_path.push(log_folder);
                log_folder = log_folder_path.to_str().unwrap().to_string();
            }

            // Start the serial monitor
            if !cmd.native_serial_port && is_wsl() {
                let result = serial_monitor::start_non_native(app_folder, 
                            cmd.port, monitor_baud, cmd.no_reconnect, log, log_folder, cmd.vid);
                match result {
                    Ok(()) => std::process::exit(0),
                    Err(e) => {
                        println!("Serial monitor error: {}", e);
                        std::process::exit(1);
                    }
                }
            }

            let result = serial_monitor::start_native(app_folder, 
                        cmd.port, monitor_baud, cmd.no_reconnect, log, log_folder, cmd.vid,
                        HISTORY_FILE_NAME.to_string());
            match result {
                Ok(()) => std::process::exit(0),
                Err(e) => {
                    println!("Serial monitor error: {}", e);
                    std::process::exit(1);
                }
            }
        }

        Action::Run(cmd) => {

            // Get the app folder (or default to current folder)
            let app_folder = cmd.app_folder.unwrap_or(".".to_string());

            // Build the app
            let result = build_raft_app(&cmd.sys_type, cmd.clean, false,
                        app_folder.clone(), cmd.docker, cmd.no_docker,
                        cmd.idf_local_build, 
                        cmd.esp_idf_path);

            // Check for build error
            if result.is_err() {
                println!("Build failed {:?}", result);
                std::process::exit(1);
            }
            
            // Flash the app
            let result = flash_raft_app(&cmd.sys_type,
                        app_folder.clone(), 
                        cmd.port.clone(),
                        cmd.native_serial_port,
                        cmd.vid.clone(),
                        cmd.flash_baud.unwrap_or(1000000),
                        cmd.flash_tool);
            if result.is_err() {
                println!("Flash operation failed {:?}", result);
                std::process::exit(1);
            }

            // Extract logging options
            let log = cmd.log;
            let log_folder = cmd.log_folder.unwrap_or("./logs".to_string());

            // Extract monitor baud rate
            let monitor_baud = cmd.monitor_baud.unwrap_or(115200);

            // Start the serial monitor
            if !cmd.native_serial_port && is_wsl() {
                let result = serial_monitor::start_non_native(app_folder, 
                            cmd.port.clone(), monitor_baud, cmd.no_reconnect, log, log_folder, cmd.vid.clone());
                match result {
                    Ok(()) => std::process::exit(0),
                    Err(e) => {
                        println!("Serial monitor error: {}", e);
                        std::process::exit(1);
                    }
                }
            }

            let result = serial_monitor::start_native(app_folder, 
                            cmd.port, monitor_baud, cmd.no_reconnect, log, log_folder,cmd.vid,
                            HISTORY_FILE_NAME.to_string());
            match result {
                Ok(()) => std::process::exit(0),
                Err(e) => {
                    println!("Serial monitor error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Action::Flash(cmd) => {

            // Get the app folder (or default to current folder)
            let app_folder = cmd.app_folder.unwrap_or(".".to_string());

            // Flash the app
            let result = flash_raft_app(&cmd.sys_type,
                app_folder.clone(), 
                cmd.port.clone(),
                cmd.native_serial_port,
                cmd.vid.clone(),
                cmd.flash_baud.unwrap_or(1000000),
                cmd.flash_tool);
            if result.is_err() {
                println!("Flash operation failed {:?}", result);
                std::process::exit(1);
            }
        }
        Action::Ota(cmd) => {

            // Get the app folder (or default to current folder)
            let app_folder = cmd.app_folder.unwrap_or(".".to_string());

            // OTA the app
            let result = ota_raft_app(&cmd.sys_type,
                app_folder.clone(), 
                cmd.ip_addr.clone(),
                cmd.ip_port.clone(),
                cmd.use_curl);
            if result.is_err() {
                println!("OTA operation failed {:?}", result);
                std::process::exit(1);
            }
        }
        Action::Ports(cmd) => {
            manage_ports(&cmd);
        }

        Action::DebugRemote(cmd) => {
            let app_folder = cmd.app_folder.unwrap_or(".".to_string());
            let log = cmd.log;
            let mut log_folder = cmd.log_folder.unwrap_or("./logs".to_string());
            // If the log_folder is relative then apply the app_folder as a prefix to it using path::join
            if !log_folder.starts_with("/") {
                let mut log_folder_path = std::path::PathBuf::from(&app_folder);
                log_folder_path.push(log_folder);
                log_folder = log_folder_path.to_str().unwrap().to_string();
            }
            // Construct server address with the specified port
            let server_address = format!("{}:{}", cmd.device_address, cmd.port);

            // Start the debug console
            if let Err(e) = app_debug_remote::start_debug_console(
                app_folder,
                server_address,
                log,
                log_folder,
                HISTORY_FILE_NAME.to_string(),
            ) {
                eprintln!("Error starting debug console: {}", e);
            }
        }
        
        Action::Esptool(cmd) => {
            // Get the esptool command
            let esptool_cmd = get_flash_tool_cmd(None, cmd.native_serial_port);
            
            // In WSL without native serial port, delegate to Windows raft.exe
            if is_wsl() && !cmd.native_serial_port {
                println!("WSL detected: Delegating esptool to Windows raft.exe");
                let mut args = vec!["esptool".to_string()];
                args.extend(cmd.args);
                args.push("-n".to_string());
                
                let output = std::process::Command::new("raft.exe")
                    .args(&args)
                    .status();
                
                match output {
                    Ok(status) => {
                        if !status.success() {
                            std::process::exit(status.code().unwrap_or(1));
                        }
                    }
                    Err(e) => {
                        eprintln!("Error executing raft.exe: {}", e);
                        std::process::exit(1);
                    }
                }
            } else {
                // Execute esptool directly
                println!("Executing: {} {:?}", esptool_cmd, cmd.args);
                
                // Handle "python -m esptool" specially
                let output = if esptool_cmd.starts_with("python -m ") {
                    let module = esptool_cmd.strip_prefix("python -m ").unwrap();
                    let mut args = vec!["-m".to_string(), module.to_string()];
                    args.extend(cmd.args.clone());
                    std::process::Command::new("python")
                        .args(&args)
                        .status()
                } else {
                    std::process::Command::new(&esptool_cmd)
                        .args(&cmd.args)
                        .status()
                };
                
                match output {
                    Ok(status) => {
                        if !status.success() {
                            std::process::exit(status.code().unwrap_or(1));
                        }
                    }
                    Err(e) => {
                        eprintln!("Error executing {}: {}", esptool_cmd, e);
                        eprintln!("\nMake sure esptool is installed. You can install it with:");
                        eprintln!("  pip install esptool");
                        std::process::exit(1);
                    }
                }
            }
        }
    }
    std::process::exit(0);
}
