// RaftCLI: Main module
// Rob Dobson 2024

use clap::Parser;
use std::path::Path;
use remove_dir_all::remove_dir_contents;
mod app_new;
use app_new::generate_new_app;
mod app_config;
use app_config::get_user_input;
mod serial_monitor;

#[derive(Clone, Parser, Debug)]
enum Action {
    #[clap(name = "new", about = "Create a new raft app")]
    New(NewCmd),
    #[clap(name = "monitor", about = "Monitor a serial port")]
    Monitor(MonitorCmd),
}

// Define arguments specific to the `new` subcommand
#[derive(Clone, Parser, Debug)]
struct NewCmd {
    base_folder: Option<String>,
    #[clap(short = 'c', long, help = "Clean the target folder")]
    clean: bool,
}

// Define arguments specific to the `monitor` subcommand
#[derive(Clone, Parser, Debug)]
struct MonitorCmd {
    port: Option<String>,
    #[clap(short = 'b', long, help = "Baud rate")]
    baud: Option<u32>,
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

// Check the target folder is valid
fn check_target_folder_valid(target_folder: &str, clean: bool) {
    // Check the target folder exists
    if !Path::new(&target_folder).exists() {
        // Create the folder if possible
        match std::fs::create_dir(&target_folder) {
            Ok(_) => println!("Created folder: {}", target_folder),
            Err(e) => {
                println!("Error creating folder: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        // Check the folder is empty
        if std::fs::read_dir(&target_folder).unwrap().next().is_some() {
            if clean {
                // Delete the contents of the folder
                match remove_dir_contents(&target_folder) {
                    Ok(_) => println!("Deleted folder contents: {}", target_folder),
                    Err(e) => {
                        println!("Error deleting folder contents: {}", e);
                        std::process::exit(1);
                    }
                }
            } else {
                println!("Error: target folder must be empty: {}", target_folder);
                std::process::exit(1);
            }
        }
    }
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
            check_target_folder_valid(&base_folder, cmd.clean);
            
            // Get configuration
            let json_config_str = get_user_input();
            let json_config = serde_json::from_str(&json_config_str.unwrap()).unwrap();

            // Generate a new app
            let _result = generate_new_app(&base_folder, json_config).unwrap();
            // println!("{:?}", result);

        }
        Action::Monitor(cmd) => {
            // Extract port and buad rate arguments
            let port = cmd.port.unwrap_or(serial_monitor::get_default_port());
            let baud = cmd.baud.unwrap_or(115200);
            let log = cmd.log;
            let log_folder = cmd.log_folder.unwrap_or("./logs".to_string());

            // Start the serial monitor
            let result = serial_monitor::start(port, baud, log, log_folder).await;
            match result {
                Ok(()) => std::process::exit(0),
                Err(e) => {
                    println!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        }
    }
}
