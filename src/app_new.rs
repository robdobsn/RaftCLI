use std::fs;
use include_dir::{include_dir, Dir};
use handlebars::Handlebars;
use serde_json::json;

static RAFT_TEMPLATES_DIR: Dir = include_dir!("./raft_templates");

fn process_dir(handlebars: &mut Handlebars, in_dir: &Dir, target_folder: &str, context: &serde_json::Value) -> 
                            Result<(), Box<dyn std::error::Error>> {
    // Iterate through the embedded folders
    for folder in in_dir.dirs() {
        println!("Folder: {}", folder.path().display());
        process_dir(handlebars, folder, target_folder, context)?;
    }

    // Iterate through the embedded files
    for file in in_dir.files() {
        println!("File: {}", file.path().display());
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

            // Generate the destination path in the target folder
            let dest_path = format!("{}/{}", target_folder, path);

            // Create any folders required to copy the file
            let dest_dir = std::path::Path::new(&dest_path).parent().unwrap();
            fs::create_dir_all(dest_dir)?;

            // Read the template content as a string
            let content = std::str::from_utf8(file.contents())?;

            // Decide to render or copy file based on its content or extension
            if content.contains("{{") && content.contains("}}") {

                println!("Rendering file from {} to: {}", path, dest_path);

                // File likely contains Handlebars syntax; attempt to register it and then render it
                handlebars.register_template_string(path.as_str(), content)?;
                let rendered = handlebars.render_template(&content, context)?;
                fs::write(&dest_path, rendered)?;

            } else {

                println!("Copying file from {} to: {}", path, dest_path);

                // File does not contain Handlebars syntax; copy as is
                fs::write(dest_path, content)?;
            }
        }
    }

    Ok(())
}

pub fn generate_new_app(target_folder: &str, context: serde_json::Value) -> Result<(), Box<dyn std::error::Error>> {

    // // TODO - change this to all user input ...
    // // Parameters to configure
    // // 1. Name of SysType
    // // 2. WebServer
    // // 3. Fixed WiFi SSID / Password - or use a config file which is not checked into git
    // // 4. Basic SysMods
    // // 5. Target chip
    // // 6. Size of flash
    // // 7. SPIRAM
    // // 8. SPIFFS / LittleFS
    // // 9. Name of main app SysMod
    // // 10. Name of main app SysMod

    // // Define the context for the template
    // let context = json!({
    //     "project_name": "MyRaftProject",
    //     "project_semver": "0.0.0",
    //     "sys_type_name": "MySysType",
    //     "target_chip": "esp32",
    //     "esp_idf_version": "5.1.2",
    //     "raft_core_git_tag": "ReWorkConfigBase",
    //     "use_raft_sysmods": true,
    //     "use_raft_webserver": true,
    //     "create_use_sysmod": true,
    //     "user_sys_mod_class": "MySysMod",
    //     "user_sys_mod_name": "my_sys_mod",

    //     "inc_raft_sysmods": "RaftSysMods@ReWorkConfigBase",
    //     "include_raft_sysmods": "#include \"RegisterSysMods.h\"",
    //     "register_raft_sysmods": "\n    // Register SysMods from RaftSysMods library\n    RegisterSysMods::registerSysMods(raftCoreApp.getSysManager());\n",

    //     "inc_raft_webserver": "RaftWebServer@ReWorkConfigBase",
    //     "include_raft_webserver": "#include \"RegisterWebServer.h\"",
    //     "register_raft_webserver": "\n    // Register WebServer from RaftWebServer library\n    RegisterSysMods::registerWebServer(raftCoreApp.getSysManager());\n",

    //     "include_user_sysmod": "#include \"MySysMod.h\"",
    //     "register_user_sysmod": "\n    // Register sysmod\n    raftCoreApp.registerSysMod(\"my_sys_mod\", MySysMod::create, true);\n",
    // });

    // Create an instance of Handlebars
    let mut handlebars = Handlebars::new();
    process_dir(&mut handlebars, &RAFT_TEMPLATES_DIR, &target_folder, &context)?;

    Ok(())
}
