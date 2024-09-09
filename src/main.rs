// RaftCLI: Main module (main.rs)
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
mod raft_cli_utils;
use raft_cli_utils::is_wsl;
use raft_cli_utils::check_target_folder_valid;
mod app_ports;
use app_ports::{PortsCmd, manage_ports};

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
    #[clap(name = "ports", about = "Manage serial ports", alias = "p")]
    Ports(PortsCmd),
}

// Define arguments specific to the `new` subcommand
#[derive(Clone, Parser, Debug)]
struct NewCmd {
    base_folder: Option<String>,
    #[clap(short = 'c', long, help = "Clean the target folder")]
    clean: bool,
}

// Define arguments specific to the `build` subcommand
#[derive(Clone, Parser, Debug)]
struct BuildCmd {
    // Add an option to specify the app folder
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
    // Option to specify path to idf.py
    #[clap(short = 'i', long, help = "Full path to idf.py (when not using docker)")]
    idf_path: Option<String>,
}

// Define arguments specific to the `monitor` subcommand
#[derive(Clone, Parser, Debug)]
struct MonitorCmd {
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
    // Option to specify path to idf.py
    #[clap(short = 'i', long, help = "Full path to idf.py (when not using docker)")]
    idf_path: Option<String>,    
    // Add an option to specify the serial port
    #[clap(short = 'p', long, help = "Serial port")]
    port: Option<String>,
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
            let json_config_str = get_user_input();
            let json_config = serde_json::from_str(&json_config_str.unwrap()).unwrap();

            // Generate a new app
            let _result = generate_new_app(&base_folder, json_config).unwrap();
            // println!("{:?}", _result);

        }

        Action::Build(cmd) => {
            // Get the app folder (or default to current folder)
            let app_folder = cmd.app_folder.unwrap_or(".".to_string());
            let result = build_raft_app(&cmd.sys_type, cmd.clean, 
                        cmd.clean_only, app_folder, cmd.docker, cmd.no_docker, cmd.idf_path);
            // println!("{:?}", result);

            // Check for build error
            if result.is_err() {
                println!("Build failed {:?}", result);
                std::process::exit(1);
            }
        }
        
        Action::Monitor(cmd) => {

            let monitor_baud = cmd.monitor_baud.unwrap_or(115200);
            let log = cmd.log;
            let log_folder = cmd.log_folder.unwrap_or("./logs".to_string());

            // Start the serial monitor
            if !cmd.native_serial_port && is_wsl() {
                let result = serial_monitor::start_non_native(cmd.port, monitor_baud, cmd.no_reconnect, log, log_folder, cmd.vid);
                match result {
                    Ok(()) => std::process::exit(0),
                    Err(e) => {
                        println!("Serial monitor error: {}", e);
                        std::process::exit(1);
                    }
                }
            }

            let result = serial_monitor::start_native(cmd.port, monitor_baud, cmd.no_reconnect, log, log_folder, cmd.vid);
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
                        app_folder.clone(), cmd.docker, cmd.no_docker, cmd.idf_path);

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
                        cmd.flash_tool,
                        result.unwrap());
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
                let result = serial_monitor::start_non_native(cmd.port.clone(), monitor_baud, cmd.no_reconnect, log, log_folder, cmd.vid.clone());
                match result {
                    Ok(()) => std::process::exit(0),
                    Err(e) => {
                        println!("Serial monitor error: {}", e);
                        std::process::exit(1);
                    }
                }
            }

            let result = serial_monitor::start_native(cmd.port, monitor_baud, cmd.no_reconnect, log, log_folder,cmd.vid);
            match result {
                Ok(()) => std::process::exit(0),
                Err(e) => {
                    println!("Serial monitor error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Action::Ports(cmd) => {
            manage_ports(&cmd);
        }
    }
    std::process::exit(0);
}
