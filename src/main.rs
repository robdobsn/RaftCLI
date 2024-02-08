// main.rs
use clap::Parser;
use std::path::Path;
mod app_new;
use app_new::generate_new_app;
mod app_config;
use app_config::get_user_input;

// Define the command line arguments
#[derive(Parser, Debug)]
#[clap(name = "Raft", version = "0.1.0", author = "Rob Dobson", about = "Raft CLI")]
struct Cli {
    // action is a required argument and can be "new", "monitor"
    action: String,

    // folder_base is an optional argument
    // when creating a new app a folder is created here for the app
    #[clap(short = 'f', long, default_value = ".")]
    folder_base: String,

    // Optional argument to clean contents of target folder
    /// Force clean of target folder contents
    #[clap(short = 'c', long)]
    clean: bool,
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
                match std::fs::remove_dir_all(&target_folder) {
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
fn main() {
    let args = Cli::parse();
    println!("{:?}", args);

    // Call the function to test the templates
    match args.action.as_str() {
        "new" => {

            // Get configuration
            let json_config_str = get_user_input();
            let json_config = serde_json::from_str(&json_config_str.unwrap()).unwrap();

            // Validate target folder
            check_target_folder_valid(&args.folder_base, args.clean);

            // Generate a new app
            let result = generate_new_app(&args.folder_base, json_config).unwrap();
            println!("{:?}", result);

        }
        _ => {}
    }
}
