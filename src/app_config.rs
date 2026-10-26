// RaftCLI: App configuration module
// Rob Dobson 2024

use evalexpr::{eval_boolean_with_context, HashMapContext, Value, ContextWithMutableVariables};
use handlebars::Handlebars;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value as JsonValue};
use regex::Regex;
use std::collections::HashMap;
use dialoguer::{Confirm, Input, Select};

use crate::new_hardware::{self, HardwareFacts, PsramMode};
use crate::new_layout::{self, AppFeatures};
use crate::raft_cli_utils::default_esp_idf_version;

// A choice in a "choice" question
#[derive(Debug, Serialize, Deserialize, Clone)]
struct ConfigChoice {
    value: String,
    label: String,
}

// Define the schema for the user input
#[derive(Debug, Serialize, Deserialize, Clone)]
struct ConfigQuestion {
    key: String,
    prompt: Option<String>,
    default: Option<String>,
    datatype: Option<String>,       // boolean, int, string, choice (with choices) or hardware (the hardware routes)
    description: Option<String>,
    pattern: Option<String>,
    message: Option<String>,
    error: Option<String>,
    condition: Option<String>,
    generator: Option<String>,
    choices: Option<Vec<ConfigChoice>>,
    advanced: Option<bool>,         // only asked with --advanced (otherwise the default is used, and shown in the summary)
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

const BOOL_PATTERN: &str = "^(true|false|t|f|yes|no|y|n)$";
const WIFI: &str = "(connectivity == \"wifi_ble\" || connectivity == \"wifi\")";
const BLE: &str = "(connectivity == \"wifi_ble\" || connectivity == \"ble\")";

// Get the populated schema for the user input
fn get_schema(base_folder: &str) -> serde_json::Value {
    let default_project_name = extract_project_name_from_folder(base_folder);
    let schema = json!([
        {
            "key": "project_name",
            "prompt": "Project Name",
            "default": default_project_name,
            "datatype": "string",
            "description": "The name of the project",
            "pattern": "^[a-zA-Z0-9_]+$",
            "message": "Project name must be alphanumeric with underscores only (no spaces or other punctuation)",
            "error": "Invalid project name"
        },
        {
            "key": "sys_type_name",
            "prompt": "System Type Name",
            "default": "{{project_name}}",
            "datatype": "string",
            "description": "The name of the system type",
            "pattern": "^[a-zA-Z0-9_]+$",
            "message": "System type name must be alphanumeric with underscores only (no spaces or other punctuation)",
            "error": "Invalid system type name",
            "advanced": true
        },
        {
            // The chip, flash, PSRAM and I2C pins: detected from a connected board, a dev board, a module's ordering
            // code, or the chip (new_hardware.rs). Sets target_chip, flash_size, psram, psram_mode, hw_pins_known
            // and raft_i2c_sda_pin / raft_i2c_scl_pin
            "key": "hardware",
            "prompt": "Hardware",
            "datatype": "hardware"
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
            "condition": "target_chip == \"esp32\" || target_chip == \"esp32s3\" || target_chip == \"esp32p4\"",
            "advanced": true
        },
        {
            "key": "main_task_core_sdkconfig",
            "condition": "(target_chip == \"esp32\" || target_chip == \"esp32s3\" || target_chip == \"esp32p4\") && main_task_core == 1",
            "generator": "\n\n# Run the main task (which runs loop() for every SysMod) on core 1 - away from the WiFi, BLE and\n# other system tasks which run on core 0. Remove the following line to run the main task on core 0.\nCONFIG_ESP_MAIN_TASK_AFFINITY_CPU1=y"
        },
        {
            // The ESP-IDF default CPU speed is 160 MHz; the esp32 and esp32s3 run at up to 240 MHz
            "key": "cpu_freq_sdkconfig",
            "condition": "target_chip == \"esp32\" || target_chip == \"esp32s3\"",
            "generator": "\n\n# CPU speed (the ESP-IDF default is 160 MHz) - main loop work such as receiving and decoding messages\n# is about a third faster at 240 MHz\nCONFIG_ESP_DEFAULT_CPU_FREQ_MHZ_240=y"
        },
        {
            "key": "esp_idf_version",
            "prompt": "ESP-IDF Version",
            "default": default_esp_idf_version(),
            "datatype": "string",
            "description": "The version of the ESP-IDF to use",
            "pattern": r"^\d+\.\d+(\.\d+)?(-[\da-zA-Z-]+(\.[\da-zA-Z-]+)*)?$",
            "message": "ESP-IDF version must be in the form x.y.z",
            "error": "Invalid ESP-IDF version",
            "advanced": true
        },
        {
            "key": "connectivity",
            "prompt": "Wireless connectivity",
            "default": "wifi_ble",
            "datatype": "choice",
            "choices": [
                { "value": "wifi_ble", "label": "WiFi and BLE" },
                { "value": "wifi", "label": "WiFi only (about 240 KB smaller)" },
                { "value": "ble", "label": "BLE only (about 680 KB smaller)" },
                { "value": "none", "label": "None - USB serial only (about 970 KB smaller)" }
            ]
        },
        {
            "key": "ble_roles",
            "prompt": "BLE role",
            "default": "peripheral",
            "datatype": "choice",
            "choices": [
                { "value": "peripheral", "label": "Peripheral - apps and the raft CLI connect to the device" },
                { "value": "central", "label": "Central - the device connects to BLE sensors (BTHome)" },
                { "value": "both", "label": "Both" }
            ],
            "condition": BLE
        },
        {
            "key": "use_raft_webserver",
            "prompt": "Include the web server",
            "default": "true",
            "datatype": "boolean",
            "description": "Use the Raft WebServer library",
            "pattern": BOOL_PATTERN,
            "message": "Must be true or false",
            "error": "Invalid web server choice",
            "condition": WIFI
        },
        {
            "key": "ota",
            "prompt": "OTA updates (two app slots, so the app can be updated over WiFi or BLE)",
            "default": "true",
            "datatype": "boolean",
            "description": "Two app partitions for over-the-air updates, or one larger one",
            "pattern": BOOL_PATTERN,
            "message": "Must be true or false",
            "error": "Invalid OTA choice"
        },
        {
            "key": "i2c_sensors",
            "prompt": "I2C and sensors",
            "default": "common",
            "datatype": "choice",
            "choices": [
                { "value": "common", "label": "I2C with about 10 common sensors" },
                { "value": "extended", "label": "I2C with about 25 sensors (about 20 KB more)" },
                { "value": "all", "label": "I2C with all of RaftCore's sensors (about 60 KB more)" },
                { "value": "i2c", "label": "I2C, no sensors built in - add them later with a device type file (about 20 KB smaller)" },
                { "value": "none", "label": "No I2C (about 110 KB smaller)" }
            ]
        },
        {
            "key": "raft_i2c_sda_pin",
            "prompt": "I2C SDA pin",
            "default": "{{hw_i2c_sda}}",
            "datatype": "int",
            "description": "The pin number for the I2C SDA line",
            "pattern": "^[0-9]+$",
            "message": "A GPIO number",
            "error": "Invalid pin number",
            "condition": "i2c_sensors != \"none\" && !hw_pins_known"
        },
        {
            "key": "raft_i2c_scl_pin",
            "prompt": "I2C SCL pin",
            "default": "{{hw_i2c_scl}}",
            "datatype": "int",
            "description": "The pin number for the I2C SCL line",
            "pattern": "^[0-9]+$",
            "message": "A GPIO number",
            "error": "Invalid pin number",
            "condition": "i2c_sensors != \"none\" && !hw_pins_known"
        },
        {
            "key": "runtime_decode",
            "prompt": "Decoding, on the device, of sensors added later (from a device type file)",
            "default": "attrs",
            "datatype": "choice",
            "choices": [
                { "value": "attrs", "label": "From their attributes" },
                { "value": "code", "label": "From their attributes and custom decode code (about 26 KB more)" },
                { "value": "none", "label": "None - only apps decode their data (about 17 KB smaller)" }
            ],
            "condition": "i2c_sensors != \"none\""
        },
        {
            "key": "create_user_sysmod",
            "prompt": "Create User SysMod",
            "default": "true",
            "datatype": "boolean",
            "description": "Create a user SysMod",
            "pattern": BOOL_PATTERN,
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
            "condition": "create_user_sysmod",
            "advanced": true
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
            "pattern": BOOL_PATTERN,
            "message": "Use Raft SysMods must be true or false",
            "error": "Invalid Raft SysMods choice",
            "advanced": true
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
            "key": "inc_bleman_in_systypes",
            "condition": "use_raft_ble",
            "generator": "\"BLEMan\": { \"enable\": 1, \"peripheral\": {{{use_raft_ble_peripheral}}}, \"advIntervalMs\": 100, \"connIntvPrefMs\": 15, \"uuidCmdRespService\": \"aa76677e-9cfd-4626-a510-0d305be57c8d\", \"uuidCmdRespCommand\": \"aa76677e-9cfd-4626-a510-0d305be57c8e\", \"uuidCmdRespResponse\": \"aa76677e-9cfd-4626-a510-0d305be57c8f\", \"central\": {{{use_raft_ble_central}}}, \"scanBTHome\": 0, \"busConnName\": \"BusBLE\", \"nimLogLev\": \"E\" },"
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
            "key": "inc_netman_in_systypes",
            "condition": "use_wifi",
            "generator": "\"NetMan\": { \"wifiSTAEn\": 1, \"wifiAPEn\": 1, \"wifiAPSSID\": \"{{{project_name}}}\", \"wifiAPPW\": \"{{{ap_password}}}\" }, \"MQTTMan\": { \"enable\": 0, \"brokerHostname\": \"mqttbroker\", \"brokerPort\": 1883, \"clientID\": \"\", \"topics\": [ { \"name\": \"examplein\", \"inbound\": 1, \"path\": \"example/in\", \"qos\": 1 }, { \"name\": \"exampleout\", \"inbound\": 0, \"path\": \"example/out\", \"qos\": 1 } ] },"
        },
        {
            "key": "inc_webserver_in_systypes",
            "condition": "use_raft_webserver",
            "generator": "\"WebServer\": { \"enable\": 1, \"websockets\": [ { \"pfix\": \"ws\", \"pcol\": \"RICSerial\" } ] },"
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
            // Folder of the DeviceTypeSources.json template - empty (so the file isn't generated) without device types
            "key": "dev_type_sources_folder",
            "condition": "use_raft_core_dev_types",
            "generator": "Common"
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

// Set a value in both the responses and the condition context
fn set_value(key: &str, value: JsonValue, responses: &mut Map<String, JsonValue>, eval_context: &mut HashMapContext) {
    let ctx_value = match &value {
        JsonValue::Bool(b) => Value::from(*b),
        JsonValue::Number(n) => Value::Int(n.as_i64().unwrap_or(0)),
        JsonValue::String(s) => Value::from(s.clone()),
        other => Value::from(other.to_string()),
    };
    eval_context.set_value(key.to_string(), ctx_value).unwrap();
    responses.insert(key.to_string(), value);
}

// The value of an answer (as text) for a question's datatype
fn typed_value(question: &ConfigQuestion, text: &str) -> JsonValue {
    match question.datatype.as_deref() {
        Some("boolean") => {
            let value = text.to_lowercase();
            JsonValue::Bool(value == "true" || value == "t" || value == "yes" || value == "y")
        }
        Some("number") | Some("int") => JsonValue::Number(serde_json::Number::from(text.trim().parse::<i64>().unwrap_or(0))),
        _ => JsonValue::String(text.to_string()),
    }
}

// Add a default value for a variable to both responses and eval_context
fn add_default_value_to_context(
    question: &ConfigQuestion,
    responses: &mut Map<String, JsonValue>,
    eval_context: &mut HashMapContext
) {
    // Use the question's default if available, otherwise use type-appropriate defaults
    let default_value = question.default.as_deref().unwrap_or_else(|| {
        match question.datatype.as_deref() {
            Some("boolean") => "false",
            Some("number") | Some("int") => "0",
            _ => ""
        }
    });
    set_value(&question.key, typed_value(question, default_value), responses, eval_context);
}

// The pattern answers to a question must match (a choice's values)
fn question_pattern(question: &ConfigQuestion) -> String {
    if let Some(choices) = &question.choices {
        return format!("^({})$", choices.iter().map(|c| regex::escape(&c.value)).collect::<Vec<_>>().join("|"));
    }
    question.pattern.clone().unwrap_or(".*".to_string())
}

// Settings from older RaftCLI versions (scripts and saved answers): translated to the questions that replace them
fn translate_legacy_settings(overrides: &HashMap<String, String>) -> Result<HashMap<String, String>, String> {
    let mut out = overrides.clone();
    let is_true = |v: &str| matches!(v.to_lowercase().as_str(), "true" | "t" | "yes" | "y");
    if let Some(v) = out.remove("flash_size_for_partition_table") {
        out.entry("flash_size".to_string()).or_insert(v);
    }
    if let Some(v) = out.remove("use_raft_ble") {
        out.entry("connectivity".to_string()).or_insert(if is_true(&v) { "wifi_ble" } else { "wifi" }.to_string());
    }
    let peripheral = out.remove("use_raft_ble_peripheral").map(|v| is_true(&v));
    let central = out.remove("use_raft_ble_central").map(|v| is_true(&v));
    if peripheral.is_some() || central.is_some() {
        let roles = match (peripheral.unwrap_or(true), central.unwrap_or(false)) {
            (_, false) => "peripheral",
            (false, true) => "central",
            (true, true) => "both",
        };
        out.entry("ble_roles".to_string()).or_insert(roles.to_string());
    }
    let i2c = out.remove("use_raft_i2c").map(|v| is_true(&v));
    let dev_types = out.remove("use_raft_core_dev_types").map(|v| is_true(&v));
    if i2c.is_some() || dev_types.is_some() {
        let sensors = match (i2c.unwrap_or(true), dev_types.unwrap_or(true)) {
            (false, _) => "none",
            (true, false) => "i2c",
            (true, true) => "all",
        };
        out.entry("i2c_sensors".to_string()).or_insert(sensors.to_string());
    }
    Ok(out)
}

// A new app's configuration: the values the templates are rendered with, and the answers (saved in the project as
// .raftnew.json so the project can be made again, or its hardware changed, by raft update-project)
pub struct NewAppConfig {
    pub config: JsonValue,
    pub answers: Map<String, JsonValue>,
}

// Get the configuration for a new app
// use_defaults: don't prompt - use the default answer for every question (for scripts and automated tests)
// overrides: answers given on the command line (key=value) or from saved answers, used instead of prompting
// advanced: ask the advanced questions too (otherwise their defaults are used)
pub fn get_new_app_config(base_folder: &str, use_defaults: bool, overrides: &HashMap<String, String>, advanced: bool)
            -> Result<NewAppConfig, Box<dyn std::error::Error>> {
    // Load and deserialize the schema
    let schema = get_schema(base_folder);
    let questions = serde_json::from_value::<Vec<ConfigQuestion>>(schema)?;
    let overrides = translate_legacy_settings(overrides)?;

    // Check that the overrides are all answers to questions (or hardware settings)
    for key in overrides.keys() {
        let is_question = questions.iter().any(|q| q.prompt.is_some() && q.datatype.as_deref() != Some("hardware") && &q.key == key);
        if !is_question && !new_hardware::HARDWARE_ALL_KEYS.contains(&key.as_str()) && key != "port" && key != "native_serial_port" && key != "ap_password" {
            let mut valid_keys: Vec<&str> = questions.iter()
                .filter(|q| q.prompt.is_some() && q.datatype.as_deref() != Some("hardware")).map(|q| q.key.as_str()).collect();
            valid_keys.extend(new_hardware::HARDWARE_ALL_KEYS.iter());
            return Err(format!("Unknown setting \"{}\" - valid settings are: {}", key, valid_keys.join(", ")).into());
        }
    }

    let mut responses = Map::new();
    let handlebars = Handlebars::new();
    let mut eval_context = HashMapContext::new();
    let mut answers = Map::new();
    let mut hardware: Option<HardwareFacts> = None;

    // PRE-PASS: Initialize all variables with defaults
    // This ensures every variable exists in the context before any condition evaluation.
    // Includes prompt-driven questions AND non-prompt questions that have a default
    // (used for values like git tags that are no longer asked for interactively).
    for question in &questions {
        if question.datatype.as_deref() == Some("hardware") {
            continue;
        }
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

            // The hardware: its own routes (new_hardware.rs)
            if question.datatype.as_deref() == Some("hardware") {
                let facts = new_hardware::choose_hardware(use_defaults, &overrides)?;
                println!("{}: {}", prompt, describe_hardware(&facts));
                set_hardware(&facts, &mut responses, &mut eval_context, &mut answers);
                hardware = Some(facts);
                continue;
            }

            // Process the default value with Handlebars
            let default_value = if let Some(default) = &question.default {
                handlebars.render_template(default, &responses)?
            } else {
                "".to_string()
            };

            // Validate input using regex
            let pattern = question_pattern(question);
            let re = Regex::new(&pattern)?;
            let message = question.message.clone().unwrap_or("Invalid input".to_string());
            let ask = !use_defaults && (advanced || !question.advanced.unwrap_or(false));

            // Use the answer from the command line, the default (if not prompting) or prompt the user for input
            let response = if let Some(override_value) = overrides.get(&question.key) {
                if !re.is_match(override_value) {
                    let message = if question.choices.is_some() { format!("must be one of {}", pattern.trim_matches(|c| c == '^' || c == '$')) } else { message };
                    return Err(format!("Invalid value \"{}\" for {}: {}", override_value, question.key, message).into());
                }
                println!("{}: {}", prompt, override_value);
                override_value.clone()
            } else if !ask {
                if !question.advanced.unwrap_or(false) || use_defaults {
                    println!("{}: {}", prompt, choice_label(question, &default_value));
                }
                default_value
            } else if let Some(choices) = &question.choices {
                let labels: Vec<&str> = choices.iter().map(|c| c.label.as_str()).collect();
                let default_idx = choices.iter().position(|c| c.value == default_value).unwrap_or(0);
                let idx = Select::new().with_prompt(prompt.as_str()).items(&labels).default(default_idx).interact()?;
                choices[idx].value.clone()
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
            let value = typed_value(question, &response);
            answers.insert(question.key.clone(), value.clone());
            set_value(&question.key, value, &mut responses, &mut eval_context);
        }
    }
    let hardware = hardware.ok_or("no hardware question")?;
    if let Some(pw) = overrides.get("ap_password") {
        set_value("ap_password", json!(pw), &mut responses, &mut eval_context);
    }

    // Values worked out from the answers (connectivity, sensors, partition table, size estimate...)
    let features = derive_values(&hardware, &mut responses, &mut eval_context)?;
    if features.wifi {
        answers.insert("ap_password".to_string(), json!(get_str(&responses, "ap_password")));
    }

    // Summary, and (interactively) a last chance not to generate
    print_summary(&hardware, &responses, &features);
    if !use_defaults && !Confirm::new().with_prompt("Generate the app?").default(true).interact()? {
        return Err("Not generated".into());
    }

    // PASS 2: Process all generators (all variables now exist in context)
    process_generators(&questions, &handlebars, &mut responses, &eval_context)?;

    Ok(NewAppConfig { config: JsonValue::Object(responses), answers })
}

// The configuration for a new app, as JSON (tests)
#[cfg(test)]
pub fn get_user_input(base_folder: &str, use_defaults: bool, overrides: &HashMap<String, String>)
            -> Result<String, Box<dyn std::error::Error>> {
    let config = get_new_app_config(base_folder, use_defaults, overrides, false)?;
    Ok(serde_json::to_string_pretty(&config.config)?)
}

fn choice_label(question: &ConfigQuestion, value: &str) -> String {
    question.choices.as_ref().and_then(|cs| cs.iter().find(|c| c.value == value)).map(|c| c.label.clone()).unwrap_or(value.to_string())
}

fn describe_hardware(f: &HardwareFacts) -> String {
    let psram = if f.psram_mb > 0 { format!("{} MB PSRAM ({})", f.psram_mb, f.psram_mode.as_str()) } else { "no PSRAM".to_string() };
    format!("{} - {}, {} MB flash, {}", f.description, f.chip, f.flash_mb, psram)
}

// The hardware's values for the templates and conditions, and as answers (so saved answers make the same app)
fn set_hardware(f: &HardwareFacts, responses: &mut Map<String, JsonValue>, ctx: &mut HashMapContext, answers: &mut Map<String, JsonValue>) {
    set_value("target_chip", json!(f.chip), responses, ctx);
    set_value("flash_size", json!(f.flash_mb), responses, ctx);
    set_value("psram", json!(f.psram_mb), responses, ctx);
    set_value("psram_mode", json!(f.psram_mode.as_str()), responses, ctx);
    set_value("hw_pins_known", json!(f.pins_known), responses, ctx);
    set_value("hw_i2c_sda", json!(f.sda), responses, ctx);
    set_value("hw_i2c_scl", json!(f.scl), responses, ctx);
    set_value("hw_description", json!(f.description), responses, ctx);
    // The pins, unless asked for later (a board preset gives them)
    set_value("raft_i2c_sda_pin", json!(f.sda), responses, ctx);
    set_value("raft_i2c_scl_pin", json!(f.scl), responses, ctx);
    answers.insert("hardware".to_string(), json!(if f.route == "detect" { "chip" } else { f.route.as_str() }));
    if let Some(b) = &f.board {
        answers.insert("board".to_string(), json!(b));
    }
    if let Some(m) = &f.module {
        answers.insert("module".to_string(), json!(m));
    }
    answers.insert("target_chip".to_string(), json!(f.chip));
    answers.insert("flash_size".to_string(), json!(f.flash_mb.to_string()));
    answers.insert("psram".to_string(), json!(f.psram_mb.to_string()));
    answers.insert("psram_mode".to_string(), json!(f.psram_mode.as_str()));
}

fn get_str(responses: &Map<String, JsonValue>, key: &str) -> String {
    responses.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

fn get_bool(responses: &Map<String, JsonValue>, key: &str) -> bool {
    responses.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

// Values the templates use, worked out from the answers
fn derive_values(hw: &HardwareFacts, responses: &mut Map<String, JsonValue>, ctx: &mut HashMapContext)
            -> Result<AppFeatures, Box<dyn std::error::Error>> {
    let connectivity = get_str(responses, "connectivity");
    let wifi = connectivity == "wifi_ble" || connectivity == "wifi";
    let ble = connectivity == "wifi_ble" || connectivity == "ble";
    let roles = get_str(responses, "ble_roles");
    let webserver = wifi && get_bool(responses, "use_raft_webserver");
    let sensors = get_str(responses, "i2c_sensors");
    let i2c = sensors != "none";
    let device_types = if i2c && sensors != "i2c" { sensors.clone() } else { "none".to_string() };
    let runtime_decode = if i2c { get_str(responses, "runtime_decode") } else { "attrs".to_string() };
    let ota = get_bool(responses, "ota");

    set_value("use_wifi", json!(wifi), responses, ctx);
    set_value("use_raft_ble", json!(ble), responses, ctx);
    set_value("use_raft_ble_peripheral", json!(ble && (roles == "peripheral" || roles == "both")), responses, ctx);
    set_value("use_raft_ble_central", json!(ble && (roles == "central" || roles == "both")), responses, ctx);
    set_value("use_raft_webserver", json!(webserver), responses, ctx);
    set_value("use_raft_i2c", json!(i2c), responses, ctx);
    set_value("use_raft_core_dev_types", json!(device_types != "none"), responses, ctx);
    let includes = match device_types.as_str() {
        "common" => json!(["raftcore:@common"]),
        "extended" => json!(["raftcore:@extended"]),
        _ => json!(["raftcore:*"]),
    };
    set_value("dev_type_includes", json!(includes.to_string()), responses, ctx);

    // I2C pins: not ones the module uses itself
    if i2c {
        for key in ["raft_i2c_sda_pin", "raft_i2c_scl_pin"] {
            let pin = responses.get(key).and_then(|v| v.as_i64()).unwrap_or(-1);
            if pin < 0 || hw.reserved_pins.contains(&(pin as u32)) {
                return Err(format!("{} GPIO{} can't be used - the {} uses it", key, pin, hw.description).into());
            }
        }
    }

    // Partition table, flash and PSRAM settings
    let layout = new_layout::partition_layout(hw.flash_mb, ota);
    set_value("partitions_csv", json!(layout.csv), responses, ctx);
    set_value("ota_enable", json!(if ota { 1 } else { 0 }), responses, ctx);
    // QIO flash for a dev board preset or an Espressif module, which all have quad flash: the app runs from flash, so
    // every instruction cache miss is a flash read, and QIO reads four bits a clock rather than two (it halved the main
    // loop's average pass on an ESP32-S3-WROOM-1-N8R2). Otherwise ESP-IDF's default, DIO, which any flash supports -
    // the flash of a detected board or a chip alone isn't known, and could be octal
    let flash_mode = if hw.board.is_some() || hw.module.is_some() {
        "\n# Flash mode: QIO (the board's flash is quad) - remove this line if the board doesn't boot\nCONFIG_ESPTOOLPY_FLASHMODE_QIO=y"
    } else {
        ""
    };
    set_value("flash_size_sdkconfig", json!(format!("# Flash size\nCONFIG_ESPTOOLPY_FLASHSIZE_{}MB=y{}", hw.flash_mb, flash_mode)), responses, ctx);
    let psram_sdkconfig = if hw.psram_mb > 0 {
        let mode = if hw.chip == "esp32s3" {
            if hw.psram_mode == PsramMode::Octal { "CONFIG_SPIRAM_MODE_OCT=y\n" } else { "CONFIG_SPIRAM_MODE_QUAD=y\n" }
        } else { "" };
        format!("\n\n# PSRAM ({} MB {}) - boot continues without it if it isn't found\nCONFIG_SPIRAM=y\n{}CONFIG_SPIRAM_IGNORE_NOTFOUND=y",
                hw.psram_mb, hw.psram_mode.as_str(), mode)
    } else {
        String::new()
    };
    set_value("psram_sdkconfig", json!(psram_sdkconfig), responses, ctx);

    // features.cmake: no networking, runtime decoding
    let networking_cmake = if wifi { String::new() } else {
        "\n# No WiFi or IP networking: the networking SysMods, WiFi driver and IP stack aren't built in (raft new's\n\
         # \"Wireless connectivity\" answer). Remove this line to add them back\nadd_compile_definitions(RAFT_NO_NETWORKING)\n".to_string()
    };
    set_value("networking_cmake", json!(networking_cmake), responses, ctx);
    let runtime_decode_cmake = match runtime_decode.as_str() {
        "none" => "\n# Device types added at runtime (a device type file) aren't decoded on the device - only apps decode their data\nset(RAFT_RUNTIME_DECODE OFF)\n",
        "code" => "\n# Device types added at runtime are decoded on the device from their attributes and custom decode code\nset(RAFT_RUNTIME_DECODE_CODE ON)\n",
        _ => "",
    };
    set_value("runtime_decode_cmake", json!(runtime_decode_cmake), responses, ctx);

    // A password for the WiFi access point, so no two apps share one (kept when the app is made again from its answers)
    if get_str(responses, "ap_password").is_empty() {
        set_value("ap_password", json!(new_layout::random_password(12)), responses, ctx);
    }

    let features = AppFeatures { chip: hw.chip.clone(), wifi, ble, webserver, i2c, device_types, runtime_decode };
    let estimate = new_layout::estimate_app_bytes(&features);
    set_value("estimated_app_bytes", json!(estimate), responses, ctx);
    set_value("app_slot_bytes", json!(layout.app_slot), responses, ctx);
    Ok(features)
}

fn print_summary(hw: &HardwareFacts, responses: &Map<String, JsonValue>, features: &AppFeatures) {
    let estimate = responses.get("estimated_app_bytes").and_then(|v| v.as_u64()).unwrap_or(0);
    let slot = responses.get("app_slot_bytes").and_then(|v| v.as_u64()).unwrap_or(1);
    let free = 100.0 * (slot as f64 - estimate as f64) / slot as f64;
    let ota = get_bool(responses, "ota");
    let layout = new_layout::partition_layout(hw.flash_mb, ota);
    println!("\nSummary");
    println!("  Hardware:      {}", describe_hardware(hw));
    for a in &hw.assumed {
        println!("                 (assumed: {})", a);
    }
    println!("  Connectivity:  {}{}", match (features.wifi, features.ble) {
        (true, true) => "WiFi and BLE", (true, false) => "WiFi", (false, true) => "BLE", _ => "none (USB serial)" },
        if features.webserver { ", web server" } else { "" });
    println!("  Sensors:       {}", if !features.i2c { "no I2C".to_string() } else { format!("I2C on SDA {} / SCL {}, device types: {}",
        responses.get("raft_i2c_sda_pin").unwrap_or(&json!("?")), responses.get("raft_i2c_scl_pin").unwrap_or(&json!("?")), features.device_types) });
    println!("  Partitions:    {} app slot{} of {}, file system {}", if ota { 2 } else { 1 }, if ota { "s" } else { "" },
             new_layout::human_bytes(layout.app_slot), new_layout::human_bytes(layout.fs_size));
    println!("  ESP-IDF:       {}   SysType: {}", get_str(responses, "esp_idf_version"), get_str(responses, "sys_type_name"));
    println!("  App size:      about {} ({:.0}% of the app slot free - an estimate from measured builds)", new_layout::human_bytes(estimate), free);
    if free < 10.0 {
        let mut ideas = Vec::new();
        if ota { ideas.push("no OTA updates (one larger app slot)"); }
        if features.device_types == "all" || features.device_types == "extended" { ideas.push("fewer device types"); }
        if features.ble { ideas.push("no BLE"); }
        println!("  Warning: little room for your own code{}", if ideas.is_empty() { String::new() } else { format!(" - consider {}", ideas.join(", or ")) });
    }
    if hw.route == "chip" && !hw.assumed.is_empty() {
        println!("  The hardware can be changed later: raft update-project --set flash_size=8 (for example), or plug the board in and run raft detect");
    }
    println!();
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
    fn esp_idf_version_in_features_cmake_matches_dockerfile_default() {
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

        // The Dockerfile takes the version as a build-arg placeholder (so raft build can override it) whose
        // default is the same version as features.cmake, with the "v" prefix the espressif/idf image tag needs
        let dockerfile = render(include_str!("../raft_templates/Dockerfile"));
        assert_eq!(classify_dockerfile(Some(&dockerfile)), DockerfileKind::Placeholder { default_tag: Some("v6.1".to_string()) });
        assert!(dockerfile.contains("ARG ESP_IDF_VERSION=v6.1"));
        assert!(dockerfile.contains("FROM espressif/idf:${ESP_IDF_VERSION}"));
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