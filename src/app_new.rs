// RaftCLI: New raft app generator
// Rob Dobson 2024

use std::fs;
use include_dir::{include_dir, Dir};
use handlebars::Handlebars;

// Define the embedded directory of templates
static RAFT_TEMPLATES_DIR: Dir = include_dir!("./raft_templates");

// Process a template directory and use its contents to generate a new app
fn process_dir(handlebars: &mut Handlebars, in_dir: &Dir, target_folder: &str, context: &serde_json::Value) -> 
                            Result<(), Box<dyn std::error::Error>> {
    // Iterate through the embedded folders
    for folder in in_dir.dirs() {
        // println!("Folder: {}", folder.path().display());
        process_dir(handlebars, folder, target_folder, context)?;
    }

    // Iterate through the embedded files
    for file in in_dir.files() {
        // println!("File: {}", file.path().display());
        let path: std::string::String;
        if let Some(found_path) = file.path().to_str() {

            // Check if the path contains handlebars
            if found_path.contains("{{") && found_path.contains("}}") {
                // Use handlebars to modify the path according to template rules
                handlebars.register_template_string("path", found_path)?;
                path = handlebars.render_template(&found_path, context)?;
            } else {
                path = found_path.to_string();
            }

            // Check if the path contains // (path separator repeated) which indicates as blank path
            if path.contains("//") {
                continue;
            }

            // Generate the destination path in the target folder
            let dest_path = format!("{}/{}", target_folder, path);

            // Create any folders required to copy the file
            let dest_dir = std::path::Path::new(&dest_path).parent().unwrap();
            fs::create_dir_all(dest_dir)?;

            // Read the file content as a string
            let content = std::str::from_utf8(file.contents())?;

            // Decide to render or copy file based on its content or extension
            if content.contains("{{") && content.contains("}}") {

                // println!("Rendering file from {} to: {}", path, dest_path);

                // File likely contains Handlebars syntax; attempt to register it and then render it
                handlebars.register_template_string(path.as_str(), content)?;
                let rendered = handlebars.render_template(&content, context)?;
                
                // Check if this is a JSON file and pretty-print it
                let final_content = if dest_path.ends_with(".json") {
                    match serde_json::from_str::<serde_json::Value>(&rendered) {
                        Ok(json_value) => {
                            match serde_json::to_string_pretty(&json_value) {
                                Ok(pretty_json) => pretty_json,
                                Err(_) => rendered, // Fallback to original if pretty-printing fails
                            }
                        }
                        Err(_) => rendered, // Fallback to original if JSON parsing fails
                    }
                } else {
                    rendered
                };
                
                fs::write(&dest_path, final_content)?;

            } else {

                // println!("Copying file from {} to: {}", path, dest_path);

                // File does not contain Handlebars syntax; copy as is
                fs::write(dest_path, content)?;
            }
        }
    }

    Ok(())
}

// Generate a new app
pub fn generate_new_app(target_folder: &str, context: serde_json::Value) -> Result<(), Box<dyn std::error::Error>> {

    // Create an instance of Handlebars
    let mut handlebars = Handlebars::new();
    process_dir(&mut handlebars, &RAFT_TEMPLATES_DIR, &target_folder, &context)?;

    // Success
    println!("Successfully generated a new raft app in: {}", target_folder);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_config::get_user_input;
    use std::collections::HashMap;

    // Generate an app with default answers (and any overrides) into a new temporary folder
    fn generate(name: &str, overrides: &[(&str, &str)]) -> std::path::PathBuf {
        let folder = std::env::temp_dir().join(format!("raftcli_test_{}_{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        let overrides: HashMap<String, String> = overrides.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let config = get_user_input("MyApp", true, &overrides).unwrap();
        generate_new_app(folder.to_str().unwrap(), serde_json::from_str(&config).unwrap()).unwrap();
        folder
    }

    #[test]
    fn device_type_sources_selects_all_raftcore_records() {
        let folder = generate("devtypes", &[]);
        let sources: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(folder.join("systypes/Common/DeviceTypeSources.json")).unwrap()).unwrap();
        assert_eq!(sources["sources"]["raftcore"]["bundled"], true);
        assert_eq!(sources["builtin"]["include"], serde_json::json!(["raftcore:*"]));
        // RaftCore's records are selected only by DeviceTypeSources.json (the build stops if DEV_TYPE_JSON_FILES
        // also names them)
        let features = fs::read_to_string(folder.join("systypes/MyApp/features.cmake")).unwrap();
        assert!(!features.contains("DEV_TYPE_JSON_FILES"));
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn no_device_type_sources_without_raftcore_device_types() {
        let folder = generate("nodevtypes", &[("use_raft_core_dev_types", "false")]);
        assert!(!folder.join("systypes/Common/DeviceTypeSources.json").exists());
        assert!(!folder.join("systypes/DeviceTypeSources.json").exists());
        assert!(folder.join("systypes/MyApp/features.cmake").exists());
        fs::remove_dir_all(&folder).unwrap();
    }
}
