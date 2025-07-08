// RaftCLI: App configuration module
// Rob Dobson 2024

use evalexpr::{eval_boolean_with_context, HashMapContext, Value, ContextWithMutableVariables};
use handlebars::Handlebars;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value as JsonValue};
use regex::Regex;
use dialoguer::Input;

use crate::raft_cli_utils::default_esp_idf_version;

// Define the schema for the user input
#[derive(Debug, Serialize, Deserialize, Clone)]
struct ConfigQuestion {
    key: String,
    prompt: Option<String>,
    default: Option<String>,
    datatype: Option<String>,
    description: Option<String>,
    pattern: Option<String>,
    message: Option<String>,
    error: Option<String>,
    condition: Option<String>,
    generator: Option<String>,
}

// Extract project name from folder path and sanitize it
fn extract_project_name_from_folder(base_folder: &str) -> String {
    let path = std::path::Path::new(base_folder);
    
    // If it's current directory, get the actual current directory name
    let folder_name = if base_folder == "." {
        std::env::current_dir()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| "NewRaftProject".to_string())
    } else {
        path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "NewRaftProject".to_string())
    };
    
    // Sanitize the folder name to match the pattern ^[a-zA-Z0-9_]+$
    let sanitized = folder_name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_')
        .collect::<String>();
    
    // Ensure it starts with a letter and isn't empty
    if sanitized.is_empty() || !sanitized.chars().next().unwrap().is_alphabetic() {
        "NewRaftProject".to_string()
    } else {
        sanitized
    }
}

// Get the populated schema for the user input
fn get_schema(base_folder: &str) -> serde_json::Value {
    let default_project_name = extract_project_name_from_folder(base_folder);
    
    // Populate schema for the user input
    let schema = json!([
        {
            "key": "project_name",
            "prompt": "Project Name",
            "default": default_project_name,
            "datatype": "string",
            "description": "The name of the project to create",
            "pattern": "^[a-zA-Z0-9_]+$",
            "message": "Project name must be alphanumeric with underscores only (no spaces or other punctuation)",
            "error": "Invalid project name"
        },
        {
            "key": "sys_type_name",
            "prompt": "System Type Name",
            "default": "SysTypeMain",
            "datatype": "string",
            "description": "The name of the system type to create",
            "pattern": "^[a-zA-Z0-9_]+$",
            "message": "System type name must be alphanumeric with underscores only (no spaces or other punctuation)",
            "error": "Invalid system type name"
        },
        {
            "key": "target_chip",
            "prompt": "Target Chip (e.g. esp32, esp32s3, esp32c3,esp32c6)",
            "default": "esp32s3",
            "datatype": "string",
            "description": "The target chip for the project",
            "pattern": "^(esp32|esp32s3|esp32c3|esp32c6)$",
            "message": "Target chip must be one of esp32, esp32s3, esp32c3, esp32c6",
            "error": "Invalid target chip"
        },
        // {
        //     "key": "use_spiram",
        //     "prompt": "Use SPIRAM (PSRAM)",
        //     "default": "false",
        //     "datatype": "boolean",
        //     "description": "Specify whether SPIRAM (PSRAM) should be used",
        //     "pattern": "^(true|false|t|f|yes|no|y|n)$",
        //     "message": "Input must be true or false",
        //     "error": "Invalid SPIRAM choice"
        // },
        // {
        //     "key": "add_use_spiram_to_sdkconfig",
        //     "condition": "use_spiram",
        //     "generator": "\n# SPIRAM\nCONFIG_SPIRAM=y\n"
        // },
        {
            "key": "flash_size_for_partition_table",
            "prompt": "Flash Size in MB (e.g. 4, 8, 16, 32)",
            "default": "4",
            "datatype": "int",
            "description": "The flash size in MB",
            "pattern": "^(4|8|16|32)$",
            "message": "Flash size must be one of 4, 8, 16, 32",
            "error": "Invalid flash size"
        },
        {
            "key": "flash_size_4MB",
            "condition": "{{flash_size_for_partition_table}}==4",
            "generator": "# Name,   Type, SubType, Offset,  Size, Flags\nnvs,      data, nvs,     0x009000,  0x015000,\notametadata,  data, ota,     0x01e000,  0x002000,\napp0,     app,  ota_0,   0x020000,  0x1b0000,\napp1,     app,  ota_1,   0x1d0000,  0x1b0000,\nfs,       data, 0x83,    0x380000,  0x080000,"
        },
        {
            "key": "flash_size_4MB_sdkconfig",
            "condition": "{{flash_size_for_partition_table}}==4",
            "generator": "# Flash size\nCONFIG_ESPTOOLPY_FLASHSIZE_4MB=y"
        },
        {
            "key": "flash_size_8MB",
            "condition": "{{flash_size_for_partition_table}}==8",
            "generator": "# Name,   Type, SubType, Offset,  Size, Flags\nnvs,      data, nvs,     0x009000,  0x015000,\notametadata,  data, ota,     0x01e000,  0x002000,\napp0,     app,  ota_0,   0x020000,  0x200000,\napp1,     app,  ota_1,   0x220000,  0x200000,\nfs,       data, 0x83,    0x420000,  0x3E0000,"
        },
        {
            "key": "flash_size_8MB_sdkconfig",
            "condition": "{{flash_size_for_partition_table}}==8",
            "generator": "# Flash size\nCONFIG_ESPTOOLPY_FLASHSIZE_8MB=y"
        },
        {
            "key": "flash_size_8MB",
            "condition": "{{flash_size_for_partition_table}}==16",
            "generator": "# Name,   Type, SubType, Offset,  Size, Flags\nnvs,      data, nvs,     0x009000,  0x015000,\notametadata,  data, ota,     0x01e000,  0x002000,\napp0,     app,  ota_0,   0x020000,  0x200000,\napp1,     app,  ota_1,   0x220000,  0x200000,\nfs,       data, 0x83,    0x420000,  0xBE0000,"
        },
        {
            "key": "flash_size_16MB_sdkconfig",
            "condition": "{{flash_size_for_partition_table}}==16",
            "generator": "# Flash size\nCONFIG_ESPTOOLPY_FLASHSIZE_16MB=y"
        },
        {
            "key": "flash_size_32MB",
            "condition": "{{flash_size_for_partition_table}}==32",
            "generator": "# Name,   Type, SubType, Offset,  Size, Flags\nnvs,      data, nvs,     0x009000,  0x015000,\notametadata,  data, ota,     0x01e000,  0x002000,\napp0,     app,  ota_0,   0x020000,  0x200000,\napp1,     app,  ota_1,   0x220000,  0x200000,\nfs,       data, 0x83,    0x420000,  0x1BE0000,"
        },
        {
            "key": "flash_size_32MB_sdkconfig",
            "condition": "{{flash_size_for_partition_table}}==32",
            "generator": "# Flash size\nCONFIG_ESPTOOLPY_FLASHSIZE_32MB=y"
        },
        {
            "key": "esp_idf_version",
            "prompt": "ESP-IDF Version",
            "default": default_esp_idf_version(),
            "datatype": "string",
            "description": "The version of the ESP-IDF to use",
            "pattern": r"^\d+\.\d+(\.\d+)?(-[\da-zA-Z-]+(\.[\da-zA-Z-]+)*)?$",
            "message": "ESP-IDF version must be in the form x.y.z",
            "error": "Invalid ESP-IDF version"
        },
        {
            "key": "create_user_sysmod",
            "prompt": "Create User SysMod",
            "default": "true",
            "datatype": "boolean",
            "description": "Create a user SysMod",
            "pattern": "^(true|false|t|f|yes|no|y|n)$",
            "message": "Create user SysMod must be true or false",
            "error": "Invalid user SysMod choice"
        },
        {
            "key": "user_sys_mod_class",
            "prompt": "User SysMod Class",
            "default": "MainSysMod",
            "datatype": "string",
            "description": "The name of the user SysMod class",
            "pattern": "^[a-zA-Z0-9_]+$",
            "message": "User SysMod class must be alphanumeric with underscores only (no spaces or other punctuation)",
            "error": "Invalid user SysMod class",
            "condition": "create_user_sysmod"
        },
        {
            "key": "user_sys_mod_name",
            "prompt": "User SysMod Name",
            "default": "{{user_sys_mod_class}}",
            "datatype": "string",
            "description": "The name of the user SysMod",
            "pattern": "^[a-zA-Z0-9_]+$",
            "message": "User SysMod name must be alphanumeric with underscores only (no spaces or other punctuation)",
            "error": "Invalid user SysMod name",
            "condition": "create_user_sysmod"
        },
        {
            "key": "depends_user_sysmod",
            "condition": "create_user_sysmod",
            "generator": "\n        {{{user_sys_mod_name}}}"
        },
        {
            "key": "raft_core_git_tag",
            "prompt": "Raft Core Git Tag",
            "default": "main",
            "datatype": "string",
            "description": "The git tag for the Raft Core library",
            "pattern": "^[a-zA-Z0-9_]*$",
            "message": "",
            "error": "Invalid git tag"
        },
        {
            "key": "use_raft_sysmods",
            "prompt": "Use Raft SysMods",
            "default": "true",
            "datatype": "boolean",
            "description": "Use the Raft SysMods library",
            "pattern": "^(true|false|t|f|yes|no|y|n)$",
            "message": "Use Raft SysMods must be true or false",
            "error": "Invalid Raft SysMods choice"
        },
        {
            "key": "raft_sysmods_git_tag",
            "prompt": "Raft SysMods Git Tag",
            "default": "main",
            "datatype": "string",
            "description": "The git tag for the Raft SysMods library",
            "pattern": "^[a-zA-Z0-9_]*$",
            "message": "",
            "error": "Invalid git tag",
            "condition": "use_raft_sysmods"
        },
        {
            "key": "depends_raft_sysmods",
            "condition": "use_raft_sysmods",
            "generator": "\n        RaftSysMods"
        },
        {
            "key": "use_raft_webserver",
            "prompt": "Use Raft Web Server",
            "default": "true",
            "datatype": "boolean",
            "description": "Use the Raft WebServer library",
            "pattern": "^(true|false|t|f|yes|no|y|n)$",
            "message": "Use Raft WebServer must be true or false",
            "error": "Invalid Raft WebServer choice"
        },
        {
            "key": "raft_webserver_git_tag",
            "prompt": "Raft Web Server Git Tag",
            "default": "main",
            "datatype": "string",
            "description": "The git tag for the Raft Web Server library",
            "pattern": "^[a-zA-Z0-9_]*$",
            "message": "",
            "error": "Invalid git tag",
            "condition": "use_raft_webserver"
        },
        {
            "key": "inc_raft_webserver",
            "condition": "use_raft_webserver",
            "generator": "RaftWebServer@{{raft_webserver_git_tag}}",
        },
        {
            "key": "include_raft_webserver",
            "condition": "use_raft_webserver",
            "generator": "#include \"RegisterWebServer.h\"",
        },
        {
            "key": "register_raft_webserver",
            "condition": "use_raft_webserver",
            "generator": "\n    // Register WebServer from RaftWebServer library\n    RegisterSysMods::registerWebServer(raftCoreApp.getSysManager());\n",
        },
        {
            "key": "depends_raft_webserver",
            "condition": "use_raft_webserver",
            "generator": "\n        RaftWebServer"
        },
        {
            "key": "use_raft_ble",
            "prompt": "Add support for Raft BLE",
            "default": "true",
            "datatype": "boolean",
            "description": "Specify whether Raft BLE support should be added",
            "pattern": "^(true|false|t|f|yes|no|y|n)$",
            "message": "Input must be true or false",
            "error": "Invalid BLE support choice"
        },
        {
            "key": "use_raft_ble_peripheral",
            "condition": "use_raft_ble",
            "prompt": "Add support for Raft BLE Peripheral",
            "default": "true",
            "datatype": "boolean",
            "description": "Specify whether Raft BLE Peripheral support should be added",
            "pattern": "^(true|false|t|f|yes|no|y|n)$",
            "message": "Input must be true or false",
            "error": "Invalid BLE peripheral support choice"
        },
        {
            "key": "use_raft_ble_central",
            "condition": "use_raft_ble",
            "prompt": "Add support for Raft BLE Central (for BTHome support)",
            "default": "false",
            "datatype": "boolean",
            "description": "Specify whether Raft BLE Central support should be added",
            "pattern": "^(true|false|t|f|yes|no|y|n)$",
            "message": "Input must be true or false",
            "error": "Invalid BLE central support choice"
        },
        {
            "key": "inc_bleman_in_systypes",
            "condition": "use_raft_ble",
            "generator": "\"BLEMan\": { \"enable\": 1, \"peripheral\": {{{use_raft_ble_peripheral}}}, \"advIntervalMs\": 100, \"connIntvPrefMs\": 15, \"uuidCmdRespService\": \"bb76677e-9cfd-4626-a510-0d305be57c8d\", \"uuidCmdRespCommand\": \"bb76677e-9cfd-4626-a510-0d305be57c8e\", \"uuidCmdRespResponse\": \"bb76677e-9cfd-4626-a510-0d305be57c8f\", \"central\": {{{use_raft_ble_central}}}, \"scanBTHome\": 1, \"busConnName\": \"BusBLE\", \"nimLogLev\": \"E\" },"
        },
        {
            "key": "use_raft_ble_central_yn",
            "condition": "use_raft_ble_central",
            "generator": "CONFIG_BT_NIMBLE_ROLE_CENTRAL=y\n"
        },
        {
            "key": "inc_bleman_in_sdkconfig",
            "condition": "use_raft_ble",
            "generator": "\n# Bluetooth\nCONFIG_BT_ENABLED=y\nCONFIG_BT_NIMBLE_ENABLED=y\n{{{use_raft_ble_central_yn}}}CONFIG_BT_NIMBLE_ROLE_OBSERVER=n\nCONFIG_BT_NIMBLE_CRYPTO_STACK_MBEDTLS=n\nCONFIG_BT_NIMBLE_LOG_LEVEL_WARNING=y\n#CONFIG_BT_NIMBLE_MEM_ALLOC_MODE_EXTERNAL=y\n"
        },
        {
            "key": "use_raft_i2c",
            "prompt": "Add support for I2C",
            "default": "true",
            "datatype": "boolean",
            "description": "Specify whether Raft I2C bus support should be added",
            "pattern": "^(true|false|t|f|yes|no|y|n)$",
            "message": "Input must be true or false",
            "error": "Invalid I2C support choice"
        },
        {
            "key": "raft_i2c_git_tag",
            "prompt": "Raft I2C Git Tag",
            "default": "main",
            "datatype": "string",
            "description": "The git tag for the Raft I2C library",
            "pattern": "^[a-zA-Z0-9_]*$",
            "message": "",
            "error": "Invalid git tag",
            "condition": "use_raft_i2c"
        },
        {
            "key": "raft_i2c_sda_pin",
            "prompt": "I2C SDA Pin number",
            "default": "5",
            "datatype": "int",
            "description": "The pin number for the I2C SDA line",
            "pattern": "^[0-9]*$",
            "message": "",
            "error": "Invalid pin number",
            "condition": "use_raft_i2c"
        },
        {
            "key": "raft_i2c_scl_pin",
            "prompt": "I2C SCL Pin number",
            "default": "6",
            "datatype": "int",
            "description": "The pin number for the I2C SCL line",
            "pattern": "^[0-9]*$",
            "message": "",
            "error": "Invalid pin number",
            "condition": "use_raft_i2c"
        },
        {
            "key": "use_raft_core_dev_types",
            "prompt": "Include Raft Core Device Types",
            "default": "true",
            "datatype": "boolean",
            "description": "Specify whether device types JSON in RaftCore should be included",
            "pattern": "^(true|false|t|f|yes|no|y|n)$",
            "message": "Input must be true or false",
            "error": "Invalid device types include choice"
        },
        {
            "key": "inc_raft_core_dev_types",
            "condition": "use_raft_core_dev_types",
            "generator": "\nset(DEV_TYPE_JSON_FILES \"/devtypes/DeviceTypeRecords.json\")\n"
        },
        {
            "key": "depends_raft_i2c",
            "condition": "use_raft_i2c",
            "generator": "\n        RaftI2C"
        },
        {
            "key": "inc_raft_i2c_sysmod",
            "condition": "use_raft_i2c",
            "generator": "RaftI2C@{{raft_i2c_git_tag}}",
        },        
        {
            "key": "inc_i2c_in_devman",
            "condition": "use_raft_i2c",
            "generator": "{\"name\":\"I2CA\",\"type\":\"I2C\",\"sdaPin\":{{{raft_i2c_sda_pin}}},\"sclPin\":{{{raft_i2c_scl_pin}}},\"i2cFreq\":100000}"
        },
        {
            "key": "include_raft_i2c",
            "condition": "use_raft_i2c",
            "generator": "#include \"BusI2C.h\"",
        },
        {
            "key": "register_raft_i2c",
            "condition": "use_raft_i2c",
            "generator": "\n    // Register BusI2C\n    raftBusSystem.registerBus(\"I2C\", BusI2C::createFn);\n",
        },
        {
            "key": "inc_raft_sysmods",
            "condition": "use_raft_sysmods",
            "generator": "RaftSysMods@{{raft_sysmods_git_tag}}",
        },
        {
            "key": "include_raft_sysmods",
            "condition": "use_raft_sysmods",
            "generator": "#include \"RegisterSysMods.h\"",
        },
        {
            "key": "register_raft_sysmods",
            "condition": "use_raft_sysmods",
            "generator": "\n    // Register SysMods from RaftSysMods library\n    RegisterSysMods::registerSysMods(raftCoreApp.getSysManager());\n",
        },
        {
            "key": "include_user_sysmod",
            "condition": "create_user_sysmod",
            "generator": "#include \"{{user_sys_mod_class}}.h\"",
        },
        {
            "key": "register_user_sysmod",
            "condition": "create_user_sysmod",
            "generator": "\n    // Register sysmod\n    raftCoreApp.registerSysMod(\"{{user_sys_mod_name}}\", {{user_sys_mod_class}}::create, true);\n",
        }
    ]);

    // Return the schema
    schema
}

// Evaluate a condition using evalexpr
fn evaluate_condition(condition: &str, context: &HashMapContext) -> bool {
    match eval_boolean_with_context(condition, context) {
        Ok(result) => result,
        Err(err) => {
            println!("Error evaluating condition: {}: {}", condition, err);
            false
        }
    }
}

// Add a default value for a variable to both responses and eval_context
fn add_default_value_to_context(
    question: &ConfigQuestion, 
    responses: &mut Map<String, JsonValue>, 
    eval_context: &mut HashMapContext
) {
    let key = &question.key;
    
    // Use the question's default if available, otherwise use type-appropriate defaults
    let default_value = question.default.as_deref().unwrap_or_else(|| {
        match question.datatype.as_deref() {
            Some("boolean") => "false",
            Some("number") | Some("int") => "0",
            _ => ""
        }
    });
    
    match question.datatype.as_deref() {
        Some("boolean") => {
            let bool_value = default_value.to_lowercase();
            let is_true = bool_value == "true" || bool_value == "t" || bool_value == "yes" || bool_value == "y";
            responses.insert(key.clone(), JsonValue::Bool(is_true));
            eval_context.set_value(key.clone(), Value::from(is_true)).unwrap();
        },
        Some("number") | Some("int") => {
            if let Ok(num) = default_value.parse::<i64>() {
                responses.insert(key.clone(), JsonValue::Number(serde_json::Number::from(num)));
                eval_context.set_value(key.clone(), evalexpr::Value::Int(num)).unwrap();
            } else {
                responses.insert(key.clone(), JsonValue::Number(serde_json::Number::from(0)));
                eval_context.set_value(key.clone(), evalexpr::Value::Int(0)).unwrap();
            }
        },
        _ => {
            responses.insert(key.clone(), JsonValue::String(default_value.to_string()));
            eval_context.set_value(key.clone(), Value::from(default_value)).unwrap();
        }
    }
}

pub fn get_user_input(base_folder: &str) -> Result<String, Box<dyn std::error::Error>> {
    // Load and deserialize the schema
    let schema = get_schema(base_folder);
    let questions = serde_json::from_value::<Vec<ConfigQuestion>>(schema)?;

    let mut responses = Map::new();
    let handlebars = Handlebars::new();
    let mut eval_context = HashMapContext::new();

    // PRE-PASS: Initialize all variables with defaults
    // This ensures every variable exists in the context before any condition evaluation
    for question in &questions {
        if question.prompt.is_some() {
            add_default_value_to_context(&question, &mut responses, &mut eval_context);
        }
    }

    // PASS 1: Process all user prompts (overwriting defaults when conditions match)
    for question in &questions {
        if let Some(prompt) = &question.prompt {
            // Process condition
            if let Some(condition) = &question.condition {
                // Render the condition using Handlebars
                let rendered_condition = handlebars.render_template(condition, &responses)?;
                // Evaluate the rendered condition using evalexpr
                if !evaluate_condition(&rendered_condition, &eval_context) {
                    continue; // Skip this question if the condition is false (keep default value)
                }
            }

            // Process the default value with Handlebars
            let default_value = if let Some(default) = &question.default {
                handlebars.render_template(default, &responses)?
            } else {
                "".to_string()
            };

            // Validate input using regex
            let pattern = question.pattern.clone().unwrap_or(".*".to_string());
            let re = Regex::new(&pattern)?;
            let message = question.message.clone().unwrap_or("Invalid input".to_string());

            // Prompt user for input
            let response = Input::new()
                .with_prompt(prompt)
                .default(default_value)
                .validate_with({
                    let re = re; // Move `re` into the closure
                    let message = message.clone(); // Clone `message` for use in the closure
                    move |input: &String| {
                        if re.is_match(input) {
                            Ok(())
                        } else {
                            Err(message.clone())
                        }
                    }
                })
                .interact_text()
                .unwrap_or_default();

            // Save response (overwriting the default)
            let key = question.key.clone();
            match question.datatype.as_deref() {
                Some("boolean") => {
                    let value = response.to_lowercase();
                    let is_true = value == "true" || value == "t" || value == "yes" || value == "y";
                    responses.insert(key.clone(), JsonValue::Bool(is_true));
                    eval_context.set_value(key.clone(), Value::from(is_true)).unwrap();
                }
                Some("number") | Some("int") => {
                    if let Ok(num) = response.parse::<i64>() {
                        responses.insert(key.clone(), JsonValue::Number(serde_json::Number::from(num)));
                        eval_context.set_value(key.clone(), evalexpr::Value::Int(num)).unwrap();
                    }
                }
                _ => {
                    responses.insert(key.clone(), JsonValue::String(response.clone()));
                    eval_context.set_value(key.clone(), Value::from(response)).unwrap();
                }
            }
        }
    }

    // PASS 2: Process all generators (all variables now exist in context)
    for question in &questions {
        if let Some(generator) = &question.generator {
            // Process condition
            if let Some(condition) = &question.condition {
                // Render the condition using Handlebars
                let rendered_condition = handlebars.render_template(condition, &responses)?;
                // Evaluate the rendered condition using evalexpr
                if !evaluate_condition(&rendered_condition, &eval_context) {
                    continue; // Skip this generator if the condition is false
                }
            }

            // Generate the value
            let generated_value = handlebars.render_template(generator, &responses)?;
            
            // Save generated value
            let key = question.key.clone();
            responses.insert(key, JsonValue::String(generated_value));
        }
    }

    // Convert the map to a JSON string
    let config_json = serde_json::to_string_pretty(&responses)?;
    Ok(config_json)
}