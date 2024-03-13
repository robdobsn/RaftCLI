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
mod raft_cli_utils;
use raft_cli_utils::is_wsl;
use raft_cli_utils::check_target_folder_valid;

#[derive(Clone, Parser, Debug)]
enum Action {
    #[clap(name = "new", about = "Create a new raft app")]
    New(NewCmd),
    #[clap(name = "build", about = "Build a raft app")]
    Build(BuildCmd),    
    #[clap(name = "monitor", about = "Monitor a serial port")]
    Monitor(MonitorCmd),
    #[clap(name = "run", about = "Build, flash and monitor a raft app")]
    Run(RunCmd),
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
    // Option to specify that docker is not to be used for the build
    #[clap(short = 'd', long, help = "Don't use docker for build")]
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
    // Option to force native serial port when in WSL
    #[clap(short = 'n', long, help = "Native serial port when in WSL")]
    native_serial_port: bool,
    // Logging options
    #[arg(short = 'l', long, help = "Log serial data to file")]
    log: bool,
    #[arg(short = 'g', long, default_value = "./logs", help = "Folder for log files")]
    log_folder: Option<String>,
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
    // Option to specify whether docker is to be used for the build
    #[clap(short = 'd', long, help = "Don't use docker for build")]
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
}

// Main CLI struct that includes the subcommands
#[derive(Parser, Debug)]
#[clap(version, author, about)]
struct Cli {
    #[clap(subcommand)]
    action: Action,
}

// Main function
#[tokio::main]
async fn main() {
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
            let _result = build_raft_app(&cmd.sys_type, cmd.clean, 
                        app_folder, cmd.no_docker, cmd.idf_path);
            // println!("{:?}", _result);
        }
        
        Action::Monitor(cmd) => {

            // Extract port and buad rate arguments
            let native_serial_port = cmd.native_serial_port;
            let port = cmd.port.unwrap_or(raft_cli_utils::get_default_port(native_serial_port));
            let monitor_baud = cmd.monitor_baud.unwrap_or(115200);
            let log = cmd.log;
            let log_folder = cmd.log_folder.unwrap_or("./logs".to_string());

            // Debug and show the args
            println!("Monitor port: {} baud: {} nativeSerialPort: {} log: {} logFolder: {}", 
                        port, monitor_baud, native_serial_port, log, log_folder);

            // Start the serial monitor
            if !native_serial_port && is_wsl() {
                let result = serial_monitor::start_non_native(port, monitor_baud, log, log_folder);
                match result {
                    Ok(()) => std::process::exit(0),
                    Err(e) => {
                        println!("Error: {}", e);
                        std::process::exit(1);
                    }
                }
            }

            let result = serial_monitor::start_native(port, monitor_baud, log, log_folder).await;
            match result {
                Ok(()) => std::process::exit(0),
                Err(e) => {
                    println!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        }

        Action::Run(cmd) => {

            // Get the app folder (or default to current folder)
            let app_folder = cmd.app_folder.unwrap_or(".".to_string());

            // Build the app
            let result = build_raft_app(&cmd.sys_type, cmd.clean, 
                        app_folder.clone(), cmd.no_docker, cmd.idf_path);

            // Check for build error
            if result.is_err() {
                println!("Build failed {:?}", result);
                std::process::exit(1);
            }

            // Extract the port and force native serial port arguments
            let native_serial_port = cmd.native_serial_port;
            let port = cmd.port.unwrap_or(raft_cli_utils::get_default_port(native_serial_port));

            // Flash the app
            let result = flash_raft_app(app_folder.clone(), 
                        port.clone(),
                        native_serial_port,
                        cmd.flash_baud.unwrap_or(2000000),
                        cmd.flash_tool,
                        result.unwrap());
            if result.is_err() {
                println!("Flash failed {:?}", result);
                std::process::exit(1);
            }

            // Extract logging options
            let log = cmd.log;
            let log_folder = cmd.log_folder.unwrap_or("./logs".to_string());

            // Extract monitor baud rate
            let monitor_baud = cmd.monitor_baud.unwrap_or(115200);

            // Start the serial monitor
            let result = serial_monitor::start_native(port.clone(), 
                            monitor_baud, log, log_folder).await;
            match result {
                Ok(()) => std::process::exit(0),
                Err(e) => {
                    println!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        }
    }
    std::process::exit(0);
}
