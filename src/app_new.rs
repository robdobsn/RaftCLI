// RaftCLI: New raft app generator
// Rob Dobson 2024

use std::collections::HashMap;
use std::fs;
use include_dir::{include_dir, Dir};
use handlebars::Handlebars;

use crate::new_layout::content_hash;

// Define the embedded directory of templates
static RAFT_TEMPLATES_DIR: Dir = include_dir!("./raft_templates");

// A rendered template file: its path in the app and its content
pub struct RenderedFile {
    pub path: String,
    pub content: String,
}

// Render a template directory into the app's files (paths relative to the app folder)
fn render_dir(handlebars: &mut Handlebars, in_dir: &Dir, context: &serde_json::Value, out: &mut Vec<RenderedFile>) ->
                            Result<(), Box<dyn std::error::Error>> {
    // Iterate through the embedded folders
    for folder in in_dir.dirs() {
        render_dir(handlebars, folder, context, out)?;
    }

    // Iterate through the embedded files
    for file in in_dir.files() {
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

            // Read the file content as a string
            let content = std::str::from_utf8(file.contents())?;

            // Decide to render or copy file based on its content or extension
            let final_content = if content.contains("{{") && content.contains("}}") {
                // File likely contains Handlebars syntax; attempt to register it and then render it
                handlebars.register_template_string(path.as_str(), content)?;
                let rendered = handlebars.render_template(&content, context)?;

                // Check if this is a JSON file and pretty-print it
                if path.ends_with(".json") {
                    match serde_json::from_str::<serde_json::Value>(&rendered) {
                        Ok(json_value) => serde_json::to_string_pretty(&json_value).unwrap_or(rendered),
                        Err(_) => rendered, // Fallback to original if JSON parsing fails
                    }
                } else {
                    rendered
                }
            } else {
                // File does not contain Handlebars syntax; copy as is
                content.to_string()
            };
            out.push(RenderedFile { path, content: final_content });
        }
    }

    Ok(())
}

// The files of a new app (not written)
pub fn render_new_app(context: &serde_json::Value) -> Result<Vec<RenderedFile>, Box<dyn std::error::Error>> {
    let mut handlebars = Handlebars::new();
    let mut files = Vec::new();
    render_dir(&mut handlebars, &RAFT_TEMPLATES_DIR, context, &mut files)?;
    Ok(files)
}

pub fn write_rendered_file(target_folder: &str, file: &RenderedFile) -> std::io::Result<()> {
    let dest_path = std::path::Path::new(target_folder).join(&file.path);
    if let Some(dest_dir) = dest_path.parent() {
        fs::create_dir_all(dest_dir)?;
    }
    fs::write(dest_path, &file.content)
}

// The answers raft new was given, and a hash of each file it generated (so raft update-project can tell which files
// have been edited since): <app>/.raftnew.json
pub const ANSWERS_FILE: &str = ".raftnew.json";

pub fn answers_file_content(answers: &serde_json::Map<String, serde_json::Value>, files: &[RenderedFile]) -> String {
    let hashes: serde_json::Map<String, serde_json::Value> = files.iter()
        .map(|f| (f.path.clone(), serde_json::Value::String(content_hash(f.content.as_bytes())))).collect();
    let content = serde_json::json!({
        "_notes": "Written by raft new: its answers, and a hash of each file it generated. raft new --answers <this file> makes \
                   the same app; raft update-project --set <key=value> changes an answer and updates the files that \
                   haven't been edited",
        "raftcli": env!("CARGO_PKG_VERSION"),
        "answers": answers,
        "files": hashes,
    });
    serde_json::to_string_pretty(&content).unwrap_or_default() + "\n"
}

// The answers saved in an app's .raftnew.json, as settings (key -> text)
pub fn read_saved_answers(path: &str) -> Result<(HashMap<String, String>, serde_json::Value), String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {}", path, e))?;
    let content: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{}: {}", path, e))?;
    let answers = content.get("answers").and_then(|a| a.as_object()).ok_or(format!("{}: no answers", path))?;
    let settings = answers.iter().map(|(k, v)| (k.clone(), match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    })).collect();
    Ok((settings, content))
}

// Generate a new app (without saving answers - tests)
#[cfg(test)]
pub fn generate_new_app(target_folder: &str, context: serde_json::Value) -> Result<(), Box<dyn std::error::Error>> {
    generate_new_app_with_answers(target_folder, context, None)
}

pub fn generate_new_app_with_answers(target_folder: &str, context: serde_json::Value,
            answers: Option<&serde_json::Map<String, serde_json::Value>>) -> Result<(), Box<dyn std::error::Error>> {
    let files = render_new_app(&context)?;
    for file in &files {
        write_rendered_file(target_folder, file)?;
    }
    if let Some(answers) = answers {
        fs::write(std::path::Path::new(target_folder).join(ANSWERS_FILE), answers_file_content(answers, &files))?;
    }

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

    fn read(folder: &std::path::Path, path: &str) -> String {
        fs::read_to_string(folder.join(path)).unwrap_or_default()
    }

    #[test]
    fn device_type_sources_select_a_set() {
        for (sensors, include) in [("common", serde_json::json!(["raftcore:@common"])), ("extended", serde_json::json!(["raftcore:@extended"])),
                                   ("all", serde_json::json!(["raftcore:*"]))] {
            let folder = generate(&format!("devtypes_{}", sensors), &[("i2c_sensors", sensors)]);
            let sources: serde_json::Value = serde_json::from_str(&read(&folder, "systypes/Common/DeviceTypeSources.json")).unwrap();
            assert_eq!(sources["sources"]["raftcore"]["bundled"], true);
            assert_eq!(sources["builtin"]["include"], include, "{}", sensors);
            // RaftCore's records are selected only by DeviceTypeSources.json (the build stops if DEV_TYPE_JSON_FILES
            // also names them)
            assert!(!read(&folder, "systypes/MyApp/features.cmake").contains("DEV_TYPE_JSON_FILES"));
            fs::remove_dir_all(&folder).unwrap();
        }
        // The default is the common set
        let folder = generate("devtypes_default", &[]);
        assert!(read(&folder, "systypes/Common/DeviceTypeSources.json").contains("raftcore:@common"));
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn connectivity_choices() {
        for (connectivity, wifi, ble) in [("wifi_ble", true, true), ("wifi", true, false), ("ble", false, true), ("none", false, false)] {
            let folder = generate(&format!("conn_{}", connectivity), &[("connectivity", connectivity)]);
            let systypes: serde_json::Value = serde_json::from_str(&read(&folder, "systypes/MyApp/SysTypes.json"))
                .unwrap_or_else(|e| panic!("{} SysTypes.json isn't valid JSON: {}", connectivity, e));
            assert_eq!(systypes.get("NetMan").is_some(), wifi, "{}", connectivity);
            assert_eq!(systypes.get("WebServer").is_some(), wifi, "{}", connectivity);
            assert_eq!(systypes.get("BLEMan").is_some(), ble, "{}", connectivity);
            let common = read(&folder, "systypes/Common/features.cmake");
            assert_eq!(common.contains("add_compile_definitions(RAFT_NO_NETWORKING)"), !wifi, "{}", connectivity);
            assert_eq!(common.contains("RaftWebServer@"), wifi, "{}", connectivity);
            assert_eq!(read(&folder, "systypes/MyApp/sdkconfig.defaults").contains("CONFIG_BT_ENABLED=y"), ble, "{}", connectivity);
            assert_eq!(read(&folder, "main/main.cpp").contains("registerWebServer"), wifi, "{}", connectivity);
            if wifi {
                // A random AP password, not the published one
                let pw = systypes["NetMan"]["wifiAPPW"].as_str().unwrap();
                assert_eq!(pw.len(), 12);
                assert_ne!(pw, "raftpassword");
            }
            fs::remove_dir_all(&folder).unwrap();
        }
    }

    #[test]
    fn partitions_flash_and_psram() {
        let folder = generate("hw_feathers3", &[("board", "um-feathers3")]);
        let partitions = read(&folder, "systypes/MyApp/partitions.csv");
        assert!(partitions.contains("app1,     app,  ota_1,   0x320000,  0x300000,"), "{}", partitions);
        let sdkconfig = read(&folder, "systypes/MyApp/sdkconfig.defaults");
        assert!(sdkconfig.contains("CONFIG_ESPTOOLPY_FLASHSIZE_16MB=y"));
        assert!(sdkconfig.contains("CONFIG_ESPTOOLPY_FLASHMODE_QIO=y"), "a board preset's flash is quad");
        assert!(sdkconfig.contains("CONFIG_SPIRAM=y") && sdkconfig.contains("CONFIG_SPIRAM_MODE_QUAD=y"));
        assert!(read(&folder, "systypes/MyApp/SysTypes.json").contains("\"sdaPin\": 8"));
        fs::remove_dir_all(&folder).unwrap();
        // A module with octal PSRAM, no OTA
        let folder = generate("hw_module", &[("module", "ESP32-S3-WROOM-1-N8R8"), ("ota", "false")]);
        let sdkconfig = read(&folder, "systypes/MyApp/sdkconfig.defaults");
        assert!(sdkconfig.contains("CONFIG_SPIRAM_MODE_OCT=y"));
        assert!(sdkconfig.contains("CONFIG_ESPTOOLPY_FLASHMODE_QIO=y"), "a module's flash is quad");
        let partitions = read(&folder, "systypes/MyApp/partitions.csv");
        assert!(partitions.contains("factory") && !partitions.contains("ota_0"));
        assert!(read(&folder, "systypes/MyApp/SysTypes.json").contains("\"enable\": 0"));
        fs::remove_dir_all(&folder).unwrap();
        // No PSRAM, the default
        let folder = generate("hw_default", &[]);
        let sdkconfig = read(&folder, "systypes/MyApp/sdkconfig.defaults");
        assert!(!sdkconfig.contains("SPIRAM"));
        assert!(!sdkconfig.contains("FLASHMODE"), "only the chip is known, so the flash mode is left as the default (DIO)");
        assert!(read(&folder, "systypes/MyApp/partitions.csv").contains("0x1b0000"));
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn reserved_pins_are_refused() {
        let overrides: HashMap<String, String> = [("module", "ESP32-S3-WROOM-1-N8R8"), ("raft_i2c_sda_pin", "35")]
            .iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let err = get_user_input("MyApp", true, &overrides).unwrap_err().to_string();
        assert!(err.contains("GPIO35"), "{}", err);
    }

    #[test]
    fn runtime_decoding_choice() {
        for (choice, line) in [("none", Some("set(RAFT_RUNTIME_DECODE OFF)")), ("code", Some("set(RAFT_RUNTIME_DECODE_CODE ON)")), ("attrs", None)] {
            let folder = generate(&format!("decode_{}", choice), &[("runtime_decode", choice)]);
            let common = read(&folder, "systypes/Common/features.cmake");
            assert_eq!(common.contains("RAFT_RUNTIME_DECODE"), line.is_some(), "{}", choice);
            if let Some(line) = line {
                assert!(common.contains(line), "{}", choice);
            }
            fs::remove_dir_all(&folder).unwrap();
        }
    }

    #[test]
    fn saved_answers_make_the_same_app() {
        let folder = std::env::temp_dir().join(format!("raftcli_test_answers_{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        let overrides: HashMap<String, String> = [("board", "xiao-esp32s3"), ("connectivity", "ble"), ("i2c_sensors", "extended")]
            .iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let config = crate::app_config::get_new_app_config("MyApp", true, &overrides, false).unwrap();
        generate_new_app_with_answers(folder.to_str().unwrap(), config.config, Some(&config.answers)).unwrap();
        let (saved, content) = read_saved_answers(folder.join(ANSWERS_FILE).to_str().unwrap()).unwrap();
        assert_eq!(saved["board"], "xiao-esp32s3");
        assert_eq!(saved["connectivity"], "ble");
        assert!(content["files"]["systypes/MyApp/partitions.csv"].is_string());
        // The same answers give the same files
        let again = crate::app_config::get_new_app_config("MyApp", true, &saved, false).unwrap();
        for file in render_new_app(&again.config).unwrap() {
            assert_eq!(file.content, read(&folder, &file.path), "{}", file.path);
        }
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn older_settings_still_work() {
        // Scripts written for earlier RaftCLI versions
        let folder = generate("legacy", &[("use_raft_ble", "false"), ("flash_size_for_partition_table", "8"),
                                          ("use_raft_core_dev_types", "false"), ("target_chip", "esp32c6")]);
        let systypes = read(&folder, "systypes/MyApp/SysTypes.json");
        assert!(!systypes.contains("BLEMan") && systypes.contains("NetMan"));
        assert!(read(&folder, "systypes/MyApp/sdkconfig.defaults").contains("CONFIG_ESPTOOLPY_FLASHSIZE_8MB=y"));
        assert!(!folder.join("systypes/Common/DeviceTypeSources.json").exists());
        assert!(systypes.contains("\"sdaPin\": 23"), "esp32c6 default pins");
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn no_device_type_sources_without_raftcore_device_types() {
        let folder = generate("nodevtypes", &[("i2c_sensors", "i2c")]);
        assert!(!folder.join("systypes/Common/DeviceTypeSources.json").exists());
        assert!(!folder.join("systypes/DeviceTypeSources.json").exists());
        assert!(folder.join("systypes/MyApp/features.cmake").exists());
        fs::remove_dir_all(&folder).unwrap();
    }
}
