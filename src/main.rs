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

// Define the command line arguments
#[derive(Parser, Debug)]
#[clap(name = "Raft", version = "0.2.0", author = "Rob Dobson", about = "Raft CLI")]
struct Cli {
    // action is a required argument and can be "new", "monitor"
    action: String,

    // action_param is an optional argument
    // when creating a new app this is the base folder in which the app is created
    // when monitoring this is an alternative way to set the serial port
    action_param: Option<String>,

    // Optional argument to clean contents of target folder
    /// Force clean of target folder contents
    #[clap(short = 'c', long)]
    clean: bool,

    // Optional argument to specify the serial port
    #[clap(short = 'p', long)]
    port: Option<String>,

    // Optional argument to specify the baud rate
    #[clap(short = 'b', long)]
    baud: Option<u32>,

    // Logging of serial data
    #[arg(short = 'l', long)]
    pub log: bool,

    // Log folder - default is ./logs
    #[arg(short = 'g', long, default_value = "./logs")]
    pub log_folder: Option<String>,
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
    let args = Cli::parse();
    // println!("{:?}", args);

    // Call the function to test the templates
    match args.action.as_str() {
        "new" => {

            // Validate target folder - doing this first so that any errors in the folder
            // defintion are handled first
            let default_folder: String = "./".to_string();
            let base_folder = args.action_param.as_ref().unwrap_or(&default_folder);
            check_target_folder_valid(&base_folder, args.clean);

            // Get configuration
            let json_config_str = get_user_input();
            let json_config = serde_json::from_str(&json_config_str.unwrap()).unwrap();

            // Generate a new app
            let result = generate_new_app(&base_folder, json_config).unwrap();
            println!("{:?}", result);

        }
        "monitor" => {
            // Extract port - which can be specified by the action_param or the port flagged parameter
            let port = args.action_param.unwrap_or(args.port.unwrap_or(serial_monitor::get_default_port()));
            // Extract other arguments
            let baud = args.baud.unwrap_or(115200);
            let log = args.log;
            let log_folder = args.log_folder.unwrap_or("./logs".to_string());

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
        _ => {}
    }
}
