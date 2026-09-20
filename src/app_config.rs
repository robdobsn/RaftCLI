// RaftCLI: App configuration module
// Rob Dobson 2024

use evalexpr::{eval_boolean_with_context, HashMapContext, Value, ContextWithMutableVariables};
use handlebars::Handlebars;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value as JsonValue};
use regex::Regex;
use std::collections::HashMap;
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
            "default": "{{project_name}}",
            "datatype": "string",
            "description": "The name of the system type to create",
            "pattern": "^[a-zA-Z0-9_]+$",
            "message": "System type name must be alphanumeric with underscores only (no spaces or other punctuation)",
            "error": "Invalid system type name"
        },
        {
            "key": "target_chip",
            "prompt": "Target Chip (e.g. esp32, esp32s3, esp32c3, esp32c5, esp32c6, esp32p4)",
            "default": "esp32s3",
            "datatype": "string",
            "description": "The target chip for the project",
            "pattern": "^(esp32|esp32s3|esp32c3|esp32c5|esp32c6|esp32p4)$",
            "message": "Target chip must be one of esp32, esp32s3, esp32c3, esp32c5, esp32c6, esp32p4",
            "error": "Invalid target chip"
        },
        {
            // The esp32 has no USB Serial/JTAG peripheral so the console must be on a UART
            "key": "console_uart_sdkconfig",
            "condition": "target_chip == \"esp32\"",
            "generator": "# Serial port (console on UART0 as the esp32 has no USB Serial/JTAG peripheral)\nCONFIG_ESP_CONSOLE_UART_DEFAULT=y"
        },
        {
            "key": "console_usb_jtag_sdkconfig",
            "condition": "target_chip != \"esp32\"",
            "generator": "# Serial port (console on the built-in USB Serial/JTAG peripheral)\nCONFIG_ESP_CONSOLE_USB_SERIAL_JTAG=y\nCONFIG_ESP_CONSOLE_SECONDARY_NONE=y"
        },
        {
            // Only offered for chips with more than one core (esp32, esp32s3, esp32p4)
            // Note that when the question isn't asked the default value remains in the context
            // so the generator below must check for a multi-core chip too
            "key": "main_task_core",
            "prompt": "CPU core for the main task (0 or 1) - WiFi, BLE and other system tasks run on core 0",
            "default": "1",
            "datatype": "int",
            "description": "The CPU core that the main task (which runs the loop() function of every SysMod) is pinned to",
            "pattern": "^(0|1)$",
            "message": "Main task core must be 0 or 1",
            "error": "Invalid main task core",
            "condition": "target_chip == \"esp32\" || target_chip == \"esp32s3\" || target_chip == \"esp32p4\""
        },
        {
            "key": "main_task_core_sdkconfig",
            "condition": "(target_chip == \"esp32\" || target_chip == \"esp32s3\" || target_chip == \"esp32p4\") && main_task_core == 1",
            "generator": "\n\n# Run the main task (which runs loop() for every SysMod) on core 1 - away from the WiFi, BLE and\n# other system tasks which run on core 0. Remove the following line to run the main task on core 0.\nCONFIG_ESP_MAIN_TASK_AFFINITY_CPU1=y"
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
            // Git tag for the Raft Core library. No prompt: edit
            // systypes/Common/features.cmake to change after generation.
            "key": "raft_core_git_tag",
            "default": "main",
            "datatype": "string"
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
            // Git tag for the Raft SysMods library. No prompt: edit
            // systypes/Common/features.cmake to change after generation.
            "key": "raft_sysmods_git_tag",
            "default": "main",
            "datatype": "string",
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
            // Git tag for the Raft Web Server library. No prompt: edit
            // systypes/Common/features.cmake to change after generation.
            "key": "raft_webserver_git_tag",
            "default": "main",
            "datatype": "string",
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
            "generator": "\"BLEMan\": { \"enable\": 1, \"peripheral\": {{{use_raft_ble_peripheral}}}, \"advIntervalMs\": 100, \"connIntvPrefMs\": 15, \"uuidCmdRespService\": \"bb76677e-9cfd-4626-a510-0d305be57c8d\", \"uuidCmdRespCommand\": \"bb76677e-9cfd-4626-a510-0d305be57c8e\", \"uuidCmdRespResponse\": \"bb76677e-9cfd-4626-a510-0d305be57c8f\", \"central\": {{{use_raft_ble_central}}}, \"scanBTHome\": 0, \"busConnName\": \"BusBLE\", \"nimLogLev\": \"E\" },"
        },
        {
            "key": "use_raft_ble_central_yn",
            "condition": "use_raft_ble_central",
            "generator": "CONFIG_BT_NIMBLE_ROLE_CENTRAL=y\n"
        },
        {
            "key": "inc_bleman_in_sdkconfig",
            "condition": "use_raft_ble",
            "generator": "\n# Bluetooth\nCONFIG_BT_ENABLED=y\nCONFIG_BT_NIMBLE_ENABLED=y\n{{{use_raft_ble_central_yn}}}CONFIG_BT_NIMBLE_ROLE_OBSERVER=n\nCONFIG_BT_NIMBLE_CRYPTO_STACK_MBEDTLS=n\nCONFIG_BT_NIMBLE_LOG_LEVEL_WARNING=y\nCONFIG_BT_NIMBLE_HOST_TASK_STACK_SIZE=6144\nCONFIG_BT_NIMBLE_MEM_ALLOC_MODE_EXTERNAL=y\n"
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
            // Git tag for the Raft I2C library. No prompt: edit
            // systypes/Common/features.cmake to change after generation.
            "key": "raft_i2c_git_tag",
            "default": "main",
            "datatype": "string",
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

// Get the configuration for a new app
// use_defaults: don't prompt - use the default answer for every question (for scripts and automated tests)
// overrides: answers given on the command line (key=value) which are used instead of prompting
pub fn get_user_input(base_folder: &str, use_defaults: bool, overrides: &HashMap<String, String>)
            -> Result<String, Box<dyn std::error::Error>> {
    // Load and deserialize the schema
    let schema = get_schema(base_folder);
    let questions = serde_json::from_value::<Vec<ConfigQuestion>>(schema)?;

    // Check that the overrides are all answers to questions
    for key in overrides.keys() {
        if !questions.iter().any(|q| q.prompt.is_some() && &q.key == key) {
            let valid_keys: Vec<&str> = questions.iter().filter(|q| q.prompt.is_some()).map(|q| q.key.as_str()).collect();
            return Err(format!("Unknown setting \"{}\" - valid settings are: {}", key, valid_keys.join(", ")).into());
        }
    }

    let mut responses = Map::new();
    let handlebars = Handlebars::new();
    let mut eval_context = HashMapContext::new();

    // PRE-PASS: Initialize all variables with defaults
    // This ensures every variable exists in the context before any condition evaluation.
    // Includes prompt-driven questions AND non-prompt questions that have a default
    // (used for values like git tags that are no longer asked for interactively).
    for question in &questions {
        if question.prompt.is_some()
            || (question.default.is_some() && question.generator.is_none())
        {
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

            // Use the answer from the command line, the default (if not prompting) or prompt the user for input
            let response = if let Some(override_value) = overrides.get(&question.key) {
                if !re.is_match(override_value) {
                    return Err(format!("Invalid value \"{}\" for {}: {}", override_value, question.key, message).into());
                }
                println!("{}: {}", prompt, override_value);
                override_value.clone()
            } else if use_defaults {
                println!("{}: {}", prompt, default_value);
                default_value
            } else {
                Input::new()
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
                    .unwrap_or_default()
            };

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
    process_generators(&questions, &handlebars, &mut responses, &eval_context)?;

    // Convert the map to a JSON string
    let config_json = serde_json::to_string_pretty(&responses)?;
    Ok(config_json)
}

// Process all generators - adds the generated values to responses
fn process_generators(
    questions: &Vec<ConfigQuestion>,
    handlebars: &Handlebars,
    responses: &mut Map<String, JsonValue>,
    eval_context: &HashMapContext
) -> Result<(), Box<dyn std::error::Error>> {
    for question in questions {
        if let Some(generator) = &question.generator {
            // Process condition
            if let Some(condition) = &question.condition {
                // Render the condition using Handlebars
                let rendered_condition = handlebars.render_template(condition, &responses)?;
                // Evaluate the rendered condition using evalexpr
                if !evaluate_condition(&rendered_condition, eval_context) {
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Generate the config for a target chip using default values for everything else
    fn generate_config_for_chip(target_chip: &str) -> Map<String, JsonValue> {
        generate_config(target_chip, None)
    }

    // Generate the config for a target chip and (optionally) a main task core answer
    fn generate_config(target_chip: &str, main_task_core: Option<i64>) -> Map<String, JsonValue> {
        let questions = serde_json::from_value::<Vec<ConfigQuestion>>(get_schema(".")).unwrap();
        let mut responses = Map::new();
        let mut eval_context = HashMapContext::new();
        for question in &questions {
            if question.prompt.is_some()
                || (question.default.is_some() && question.generator.is_none())
            {
                add_default_value_to_context(&question, &mut responses, &mut eval_context);
            }
        }
        // Defaults may themselves be templates (e.g. the SysType name defaults to the project name) and
        // are rendered when the user is prompted - so do the same here (in question order)
        for question in &questions {
            let rendered = match responses.get(&question.key) {
                Some(JsonValue::String(value)) if value.contains("{{") =>
                    Handlebars::new().render_template(value, &responses).unwrap(),
                _ => continue,
            };
            responses.insert(question.key.clone(), JsonValue::String(rendered));
        }
        responses.insert("target_chip".to_string(), JsonValue::String(target_chip.to_string()));
        eval_context.set_value("target_chip".to_string(), Value::from(target_chip)).unwrap();
        if let Some(core) = main_task_core {
            responses.insert("main_task_core".to_string(), JsonValue::Number(serde_json::Number::from(core)));
            eval_context.set_value("main_task_core".to_string(), Value::Int(core)).unwrap();
        }
        process_generators(&questions, &Handlebars::new(), &mut responses, &eval_context).unwrap();
        responses
    }

    #[test]
    fn console_is_uart_on_esp32() {
        // The esp32 has no USB Serial/JTAG peripheral
        let config = generate_config_for_chip("esp32");
        assert!(config["console_uart_sdkconfig"].as_str().unwrap().contains("CONFIG_ESP_CONSOLE_UART_DEFAULT=y"));
        assert!(!config.contains_key("console_usb_jtag_sdkconfig"));
    }

    #[test]
    fn console_is_usb_jtag_on_other_chips() {
        for chip in ["esp32s3", "esp32c3", "esp32c5", "esp32c6", "esp32p4"] {
            let config = generate_config_for_chip(chip);
            assert!(config["console_usb_jtag_sdkconfig"].as_str().unwrap().contains("CONFIG_ESP_CONSOLE_USB_SERIAL_JTAG=y"), "{}", chip);
            assert!(!config.contains_key("console_uart_sdkconfig"), "{}", chip);
        }
    }

    const MAIN_TASK_CORE_1: &str = "CONFIG_ESP_MAIN_TASK_AFFINITY_CPU1=y";

    fn render_sdkconfig(config: Map<String, JsonValue>) -> String {
        let template = include_str!("../raft_templates/systypes/{{sys_type_name}}/sdkconfig.defaults");
        Handlebars::new().render_template(template, &JsonValue::Object(config)).unwrap()
    }

    #[test]
    fn main_task_defaults_to_core_1_on_multi_core_chips() {
        for chip in ["esp32", "esp32s3", "esp32p4"] {
            assert!(render_sdkconfig(generate_config_for_chip(chip)).contains(MAIN_TASK_CORE_1), "{}", chip);
            assert!(render_sdkconfig(generate_config(chip, Some(1))).contains(MAIN_TASK_CORE_1), "{}", chip);
        }
    }

    #[test]
    fn main_task_core_0_can_be_chosen() {
        for chip in ["esp32", "esp32s3", "esp32p4"] {
            assert!(!render_sdkconfig(generate_config(chip, Some(0))).contains("MAIN_TASK_AFFINITY"), "{}", chip);
        }
    }

    #[test]
    fn main_task_core_is_never_set_on_single_core_chips() {
        // The question isn't asked for these chips so the default value (1) remains in the context
        // and the affinity setting (which is invalid on a single core chip) must not be generated
        for chip in ["esp32c3", "esp32c5", "esp32c6"] {
            assert!(!render_sdkconfig(generate_config_for_chip(chip)).contains("MAIN_TASK_AFFINITY"), "{}", chip);
        }
    }

    #[test]
    fn main_task_core_question_only_asked_for_multi_core_chips() {
        let questions = serde_json::from_value::<Vec<ConfigQuestion>>(get_schema(".")).unwrap();
        let question = questions.iter().find(|q| q.key == "main_task_core").unwrap();
        let condition = question.condition.as_ref().unwrap();
        for (chip, expected) in [("esp32", true), ("esp32s3", true), ("esp32p4", true),
                    ("esp32c3", false), ("esp32c5", false), ("esp32c6", false)] {
            let mut eval_context = HashMapContext::new();
            eval_context.set_value("target_chip".to_string(), Value::from(chip)).unwrap();
            assert_eq!(evaluate_condition(condition, &eval_context), expected, "{}", chip);
        }
    }

    #[test]
    fn non_interactive_generation() {
        // All defaults
        let config: JsonValue = serde_json::from_str(&get_user_input("MyApp", true, &HashMap::new()).unwrap()).unwrap();
        assert_eq!(config["project_name"], "MyApp");
        assert_eq!(config["sys_type_name"], "MyApp");
        assert_eq!(config["target_chip"], "esp32s3");
        assert!(config["main_task_core_sdkconfig"].as_str().unwrap().contains(MAIN_TASK_CORE_1));

        // Answers from the command line
        let overrides: HashMap<String, String> = [("target_chip", "esp32c6"), ("sys_type_name", "BoardA"), ("use_raft_ble", "false")]
            .iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let config: JsonValue = serde_json::from_str(&get_user_input("MyApp", true, &overrides).unwrap()).unwrap();
        assert_eq!(config["target_chip"], "esp32c6");
        assert_eq!(config["sys_type_name"], "BoardA");
        assert_eq!(config["use_raft_ble"], false);
        assert!(config.get("main_task_core_sdkconfig").is_none());
        assert!(config.get("inc_bleman_in_sdkconfig").is_none());

        // Invalid answers and unknown settings are errors
        let bad_value: HashMap<String, String> = [("target_chip".to_string(), "z80".to_string())].into_iter().collect();
        assert!(get_user_input("MyApp", true, &bad_value).is_err());
        let bad_key: HashMap<String, String> = [("no_such_setting".to_string(), "1".to_string())].into_iter().collect();
        assert!(get_user_input("MyApp", true, &bad_key).is_err());
    }

    #[test]
    fn esp_idf_version_is_in_features_cmake_not_the_dockerfile() {
        use crate::esp_idf::{classify_dockerfile, parse_features_cmake_version, DockerfileKind};
        let overrides: HashMap<String, String> = [("esp_idf_version".to_string(), "6.1".to_string())].into_iter().collect();
        let config: JsonValue = serde_json::from_str(&get_user_input("MyApp", true, &overrides).unwrap()).unwrap();
        let render = |template: &str| Handlebars::new().render_template(template, &config).unwrap();

        // The version is the project default in Common/features.cmake
        let common = render(include_str!("../raft_templates/systypes/Common/features.cmake"));
        assert_eq!(parse_features_cmake_version(&common), Ok(Some("6.1".to_string())));

        // The SysType features.cmake only has a commented-out override
        let sys_type = render(include_str!("../raft_templates/systypes/{{sys_type_name}}/features.cmake"));
        assert_eq!(parse_features_cmake_version(&sys_type), Ok(None));
        assert!(sys_type.contains("# set(ESP_IDF_VERSION \"6.1\")"));

        // The Dockerfile has a placeholder rather than a version
        let dockerfile = render(include_str!("../raft_templates/Dockerfile"));
        assert_eq!(classify_dockerfile(Some(&dockerfile)), DockerfileKind::Placeholder { default_tag: None });
        assert!(!dockerfile.contains("6.1"));
    }

    #[test]
    fn text_templates_render() {
        let config = JsonValue::Object(generate_config_for_chip("esp32s3"));
        for template in [
            include_str!("../raft_templates/README.md"),
            include_str!("../raft_templates/systypes/Common/features.cmake"),
            include_str!("../raft_templates/components/{{user_sys_mod_name}}/{{user_sys_mod_class}}.cpp"),
            include_str!("../raft_templates/components/{{user_sys_mod_name}}/{{user_sys_mod_class}}.h"),
        ] {
            let rendered = Handlebars::new().render_template(template, &config).unwrap();
            let unrendered = rendered.lines().find(|line| line.contains("{{"));
            assert!(unrendered.is_none(), "{:?}", unrendered);
        }
    }

    #[test]
    fn sdkconfig_template_renders_one_console_section() {
        let template = include_str!("../raft_templates/systypes/{{sys_type_name}}/sdkconfig.defaults");
        for (chip, expected, not_expected) in [
            ("esp32", "CONFIG_ESP_CONSOLE_UART_DEFAULT=y", "CONFIG_ESP_CONSOLE_USB_SERIAL_JTAG=y"),
            ("esp32s3", "CONFIG_ESP_CONSOLE_USB_SERIAL_JTAG=y", "CONFIG_ESP_CONSOLE_UART_DEFAULT=y"),
        ] {
            let config = JsonValue::Object(generate_config_for_chip(chip));
            let rendered = Handlebars::new().render_template(template, &config).unwrap();
            assert!(rendered.contains(expected), "{}", chip);
            assert!(!rendered.contains(not_expected), "{}", chip);
            assert!(!rendered.contains("{{"), "{}", chip);
        }
    }
}