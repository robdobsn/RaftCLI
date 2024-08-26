// RaftCLI: App configuration module
// Rob Dobson 2024

use serde::{Deserialize, Serialize};
use dialoguer::Input;
use serde_json::Result;
use serde_json::json;
use handlebars::Handlebars;
use regex;

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

// Get the populated schema for the user input
fn get_schema() -> serde_json::Value {

    // Populate schema for the user input
    let schema = json!([
        {
            "key": "project_name",
            "prompt": "Project Name",
            "default": "NewRaftProject",
            "datatype": "string",
            "description": "The name of the project to create",
            "pattern": "^[a-zA-Z0-9_]+$",
            "message": "Project name must be alphanumeric with underscores only (no spaces or other punctuation)",
            "error": "Invalid project name"
        },
        {
            "key": "project_semver",
            "prompt": "Project Version (e.g. 1.0.0)",
            "default": "1.0.0",
            "datatype": "string",
            "description": "The version of the project to create",
            "pattern": r"^\d+\.\d+(\.\d+)?(-[\da-zA-Z-]+(\.[\da-zA-Z-]+)*)?$",
            "message": "Project version must be in the form x.y.z",
            "error": "Invalid project version"
        },
        {
            "key": "target_chip",
            "prompt": "Target Chip (e.g. esp32, esp32s3, esp32c3)",
            "default": "esp32",
            "datatype": "string",
            "description": "The target chip for the project",
            "pattern": "^(esp32|esp32s3|esp32c3)$",
            "message": "Target chip must be one of esp32, esp32s3, esp32c3",
            "error": "Invalid target chip"
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
            "key": "esp_idf_version",
            "prompt": "ESP-IDF Version",
            "default": "5.2.1",
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
            "pattern": "^(true|false|y|n)$",
            "message": "Create user SysMod must be true or false",
            "error": "Invalid user SysMod choice"
        },
        {
            "key": "user_sys_mod_class",
            "prompt": "User SysMod Class",
            "default": "MySysMod",
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
            "pattern": "^(true|false|y|n)$",
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
            "key": "use_raft_webserver",
            "prompt": "Use Raft Web Server",
            "default": "true",
            "datatype": "boolean",
            "description": "Use the Raft WebServer library",
            "pattern": "^(true|false|y|n)$",
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

// Get user input
pub fn get_user_input() -> Result<String> {
    
    // Load and deserialize the schema
    let schema = get_schema();
    let questions: Result<Vec<ConfigQuestion>> = serde_json::from_value(schema);

    // Check for errors
    if questions.is_err() {
        println!("Error: {}", questions.err().unwrap());
        return Ok("".to_string());
    }
    let questions = questions.unwrap();

    let mut responses = serde_json::Map::new();
    let mut context = serde_json::json!({});

    // Iterate over the questions, prompting the user for each one
    for question in questions {
        // Check if input is required (prompt the user for input)
        let response: String;
        
        // Check condition
        if question.condition.is_some() {
            // Extract condition
            let handlebars = Handlebars::new();
            let condition = handlebars.render_template(&question.condition.unwrap(), &context);
            if condition.is_err() {
                println!("Error: {}", condition.err().unwrap());
                return Ok("".to_string());
            }
            let condition = condition.unwrap();
            // Extract condition from context
            match context.get(&condition) {
                Some(value) => {
                    if value.is_boolean() {
                        if !value.as_bool().unwrap() {
                            continue;
                        }
                    } else if value.is_string() {
                        if value.as_str().unwrap() == "" {
                            continue;
                        }
                    }
                },
                None => {
                    continue;
                }
            }
        }
        if question.prompt.is_some() {
            // Process the default value using handlebars
            let default_value = 
                if question.default.is_some() {
                    let handlebars = Handlebars::new();
                    let rendered = handlebars.render_template(&question.default.unwrap(), &context);
                    if rendered.is_err() {
                        println!("Error: {}", rendered.err().unwrap());
                        return Ok("".to_string());
                    }
                    rendered.unwrap()
                } else {
                    "".to_string()
                };
            // Get the regex for validation
            let pattern = 
                if question.pattern.is_some() {
                    let handlebars = Handlebars::new();
                    let rendered = handlebars.render_template(&question.pattern.unwrap(), &context);
                    if rendered.is_err() {
                        println!("Error: {}", rendered.err().unwrap());
                        return Ok("".to_string());
                    }
                    rendered.unwrap()
                } else {
                    "".to_string()
                };
            let regex_pattern = regex::Regex::new(&pattern);
            if regex_pattern.is_err() {
                println!("Error: {}", regex_pattern.err().unwrap());
                return Ok("".to_string());
            }
            let re = regex_pattern.unwrap();
            let message = question.message.unwrap_or("The input does not match the required pattern".to_string());
            let user_response = Input::new()
                .with_prompt(question.prompt.unwrap())
                .default(default_value)
                .validate_with(|input: &String| {
                    if re.is_match(input) {
                        Ok(())
                    } else {
                        Err(message.clone())
                    }
                })
                .interact_text();
            // Check for error
            if user_response.is_err() {
                println!("Error: {}", user_response.err().unwrap());
                return Ok("".to_string());
            }
            response = user_response.unwrap();
        } else if question.generator.is_some() {
            // Process the generator using handlebars
            let handlebars = Handlebars::new();
            let generated = handlebars.render_template(&question.generator.unwrap(), &context);
            if generated.is_err() {
                println!("Error: {}", generated.err().unwrap());
                return Ok("".to_string());
            }

            response = generated.unwrap();
        } else {
            response = question.default.unwrap_or("".to_string());
        }

        // Populate the JSON object
        match question.datatype.unwrap_or("string".to_string()).as_str() {
            "boolean" => {
                let response = response.to_lowercase();
                if response == "true" || response == "t" || response == "yes" || response == "y" {
                    responses.insert(question.key.clone(), serde_json::Value::Bool(true));
                } else {
                    responses.insert(question.key.clone(), serde_json::Value::Bool(false));
                }
            },
            "number" => {
                let response = response.parse::<f64>();
                if response.is_err() {
                    println!("Error: {}", response.err().unwrap());
                    return Ok("".to_string());
                }
                responses.insert(question.key.clone(), serde_json::Value::Number(serde_json::Number::from_f64(response.unwrap()).unwrap()));
            },
            _ => {
                responses.insert(question.key.clone(), serde_json::Value::String(response));
            }
        }

        // Regenerate the context
        context = serde_json::json!(responses);
    }

    // // Convert the map to a JSON string
    let config_json = serde_json::to_string_pretty(&responses)?;

    // Debug
    // println!("Generated Config: {}", config_json);

    // Return the JSON string
    Ok(config_json)
}
