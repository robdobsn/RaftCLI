// RaftCLI: bring the CMakeLists.txt of a project created by an older "raft new" up to date
// Rob Dobson 2026
//
// Two changes can be made safely:
// - Projects which include a local bootstrap from raftdevlibs/RaftCore but don't stop on a broken
//   raftdevlibs/RaftCore link (created before RaftCLI 1.25.1) get the check inserted. Without it a link
//   which can't be followed (it was moved, or the build runs in a container which doesn't have it) means
//   the build quietly downloads the published RaftCore instead. Only lines are added so this is safe
//   even if the file has been changed.
// - Projects whose CMakeLists.txt is still exactly the old template which downloads RaftBootstrap.cmake
//   from a fixed RaftCore release are given the current template, but only when RaftCore floats (main, a
//   branch or no tag). A project which pins RaftCore to a release is left alone - older releases don't
//   all have the RaftBootstrap.cmake asset which the current template downloads.

use std::io::IsTerminal;
use std::collections::HashMap;
use std::path::Path;
use crate::bootstrap_check::{hardcoded_bootstrap_tag, raft_core_tag};

const CURRENT_TEMPLATE: &str = include_str!("../raft_templates/CMakeLists.txt");

// The check for a broken raftdevlibs/RaftCore link (present in the current template)
const GUARD_MARKER: &str = "IS_SYMLINK \"${CMAKE_SOURCE_DIR}/raftdevlibs/RaftCore\"";
const GUARD_LINES: [&str; 3] = [
    "if(IS_SYMLINK \"${CMAKE_SOURCE_DIR}/raftdevlibs/RaftCore\" AND NOT EXISTS \"${CMAKE_SOURCE_DIR}/raftdevlibs/RaftCore\")",
    "    message(FATAL_ERROR \"raftdevlibs/RaftCore is a broken link - fix it or remove it with 'raft l unlink --libs RaftCore'\")",
    "endif()",
];
// The line the check goes in front of
const LOCAL_BOOTSTRAP_LINE: &str = "if(EXISTS \"${CMAKE_SOURCE_DIR}/raftdevlibs/RaftCore/scripts/RaftBootstrap.cmake\")";

// The template used by RaftCLI before 1.20.1 (with the release tag as {TAG})
const OLD_HARDCODED_TEMPLATE: &str = "# Raft Project
cmake_minimum_required(VERSION 3.16)

# Bootstrap using RaftBootstrap script
set(BOOTSTRAP_URL \"https://github.com/robdobsn/RaftCore/releases/download/{TAG}/RaftBootstrap.cmake\")
file(DOWNLOAD ${BOOTSTRAP_URL} \"${CMAKE_BINARY_DIR}/RaftBootstrap.cmake\")
include(\"${CMAKE_BINARY_DIR}/RaftBootstrap.cmake\")

# Define the project dependencies
project(${_systype_name} DEPENDS ${ADDED_PROJECT_DEPENDENCIES})
";

#[derive(Debug, Clone, PartialEq)]
pub enum UpdateKind {
    // The broken raftdevlibs/RaftCore link check is added
    AddBrokenLinkCheck,
    // The old template (bootstrap from a fixed release) is replaced by the current one
    ReplaceOldTemplate { bootstrap_tag: String },
}

#[derive(Debug, Clone)]
pub struct CmakeUpdate {
    pub kind: UpdateKind,
    pub new_content: String,
}

impl CmakeUpdate {
    pub fn describe(&self) -> String {
        match &self.kind {
            UpdateKind::AddBrokenLinkCheck =>
                "add a check which stops the build if raftdevlibs/RaftCore is a broken link".to_string(),
            UpdateKind::ReplaceOldTemplate { bootstrap_tag } => format!(
                "replace the old template, which downloads RaftBootstrap.cmake from RaftCore release {}, with the \
                 current one, which works it out from RaftCore@<tag> in features.cmake", bootstrap_tag),
        }
    }
}

fn normalise_line_endings(content: &str) -> String {
    content.replace("\r\n", "\n")
}

/// The update for a CMakeLists.txt (None if it is up to date or can't be updated safely). core_tag is the
/// RaftCore tag from features.cmake (Some("") if RaftCore has no tag, None if it isn't known).
pub fn cmake_update(cmakelists: &str, core_tag: Option<&str>) -> Option<CmakeUpdate> {
    let crlf = cmakelists.contains("\r\n");
    let content = normalise_line_endings(cmakelists);
    let with_line_endings = |s: String| if crlf { s.replace('\n', "\r\n") } else { s };

    if content.contains(GUARD_MARKER) {
        return None;
    }

    // Insert the check in front of the local bootstrap include (keeping its indentation)
    if let Some(pos) = content.lines().position(|line| line.trim() == LOCAL_BOOTSTRAP_LINE) {
        let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
        let indent: String = lines[pos].chars().take_while(|c| c.is_whitespace()).collect();
        for (i, guard) in GUARD_LINES.iter().enumerate() {
            lines.insert(pos + i, format!("{}{}", indent, guard));
        }
        let mut new_content = lines.join("\n");
        if content.ends_with('\n') {
            new_content.push('\n');
        }
        return Some(CmakeUpdate { kind: UpdateKind::AddBrokenLinkCheck, new_content: with_line_endings(new_content) });
    }

    // Replace the old template if it hasn't been changed and RaftCore isn't pinned to a release
    let bootstrap_tag = hardcoded_bootstrap_tag(&content)?;
    let core_is_pinned_release = regex::Regex::new(r"^v?\d+\.\d+").unwrap().is_match(core_tag?);
    if core_is_pinned_release {
        return None;
    }
    let old_template = OLD_HARDCODED_TEMPLATE.replace("{TAG}", &bootstrap_tag);
    if content.trim_end() != old_template.trim_end() {
        return None;
    }
    Some(CmakeUpdate {
        kind: UpdateKind::ReplaceOldTemplate { bootstrap_tag },
        new_content: with_line_endings(normalise_line_endings(CURRENT_TEMPLATE)),
    })
}

// The RaftCore tag from the features.cmake files (the SysType's own if given, then Common, then any SysType)
fn read_core_tag(app_folder: &str, sys_type: Option<&str>) -> Option<String> {
    let systypes = Path::new(app_folder).join("systypes");
    let mut files = Vec::new();
    if let Some(sys_type) = sys_type {
        files.push(systypes.join(sys_type).join("features.cmake"));
    }
    files.push(systypes.join("Common").join("features.cmake"));
    if let Ok(entries) = std::fs::read_dir(&systypes) {
        let mut others: Vec<_> = entries.flatten().map(|e| e.path().join("features.cmake")).collect();
        others.sort();
        files.extend(others);
    }
    files.iter()
        .filter_map(|f| std::fs::read_to_string(f).ok())
        .find_map(|content| raft_core_tag(&content))
}

/// The update for a project's CMakeLists.txt (None if it is up to date, missing or can't be updated safely)
pub fn project_cmake_update(app_folder: &str, sys_type: Option<&str>) -> Option<CmakeUpdate> {
    let cmakelists = std::fs::read_to_string(Path::new(app_folder).join("CMakeLists.txt")).ok()?;
    cmake_update(&cmakelists, read_core_tag(app_folder, sys_type).as_deref())
}

fn apply_update(app_folder: &str, update: &CmakeUpdate) -> std::io::Result<()> {
    std::fs::write(Path::new(app_folder).join("CMakeLists.txt"), &update.new_content)
}

/// Check before a build. When raftdevlibs/RaftCore is a link and the project doesn't check for a broken one
/// this explains why it matters and (in a terminal) offers to update CMakeLists.txt. Other updates get a note.
pub fn check_for_build(app_folder: &str, sys_type: &str, raft_core_linked: bool) {
    let update = match project_cmake_update(app_folder, Some(sys_type)) {
        Some(update) => update,
        None => return,
    };
    // The bootstrap version warning (bootstrap_check) already covers replacing the old template
    if !raft_core_linked || update.kind != UpdateKind::AddBrokenLinkCheck {
        if update.kind == UpdateKind::AddBrokenLinkCheck {
            println!("Note: CMakeLists.txt is from an older \"raft new\" - 'raft update-project' brings it up to date");
        }
        return;
    }
    println!("Warning: raftdevlibs/RaftCore is a link but CMakeLists.txt doesn't stop the build if the link can't be");
    println!("followed (e.g. if it was moved, or a container doesn't have it), so the build would quietly use the");
    println!("published RaftCore instead. 'raft update-project' adds the check to CMakeLists.txt.");
    if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
        return;
    }
    let confirmed = dialoguer::Confirm::new()
        .with_prompt("Update CMakeLists.txt now?")
        .default(true)
        .interact()
        .unwrap_or(false);
    if confirmed {
        match apply_update(app_folder, &update) {
            Ok(()) => println!("CMakeLists.txt updated: {}", update.describe()),
            Err(e) => println!("Warning: failed to update CMakeLists.txt: {}", e),
        }
    }
}

/// raft update-project
pub fn update_project(app_folder: &str) -> Result<(), String> {
    if !crate::raft_cli_utils::check_app_folder_valid(app_folder.to_string()) {
        return Err("app folder is not a valid Raft project root".to_string());
    }
    match project_cmake_update(app_folder, None) {
        Some(update) => {
            apply_update(app_folder, &update).map_err(|e| format!("failed to write CMakeLists.txt: {}", e))?;
            println!("CMakeLists.txt updated: {}", update.describe());
            if matches!(update.kind, UpdateKind::ReplaceOldTemplate { .. }) {
                println!("Do a clean build (raft build -c) so the new bootstrap is used");
            }
        }
        None => {
            let cmakelists = std::fs::read_to_string(Path::new(app_folder).join("CMakeLists.txt")).unwrap_or_default();
            if hardcoded_bootstrap_tag(&cmakelists).is_some() && !cmakelists.contains(GUARD_MARKER) {
                println!("CMakeLists.txt downloads RaftBootstrap.cmake from a fixed RaftCore release and either RaftCore is");
                println!("pinned to a release in features.cmake or the file has been changed, so it hasn't been updated.");
                println!("To update it by hand compare it with the CMakeLists.txt of a project created by \"raft new\".");
            } else {
                println!("CMakeLists.txt is up to date");
            }
        }
    }
    Ok(())
}

// Make a project's files again from the answers raft new saved in it (.raftnew.json), with some answers changed
// (e.g. flash_size=8 after finding the board has 8 MB): files that haven't been edited since they were generated are
// updated, edited ones are left as they are with the new version beside them (<file>.raftnew)
pub fn rerender_project(app_folder: &str, settings: &HashMap<String, String>) -> Result<(), String> {
    use crate::app_new::{answers_file_content, read_saved_answers, render_new_app, write_rendered_file, ANSWERS_FILE};
    use crate::new_layout::content_hash;
    let answers_path = Path::new(app_folder).join(ANSWERS_FILE);
    if !answers_path.exists() {
        return Err(format!("{} has no {} (it was made by an earlier raft new, which didn't save its answers) - \
                            change the files by hand", app_folder, ANSWERS_FILE));
    }
    let (mut answers, saved) = read_saved_answers(answers_path.to_str().unwrap_or(ANSWERS_FILE))?;
    // A different hardware route replaces the old one's settings (e.g. a board after a chip)
    if settings.keys().any(|k| k == "board" || k == "module") {
        for key in ["hardware", "board", "module", "target_chip", "flash_size", "psram", "psram_mode"] {
            answers.remove(key);
        }
    }
    for (key, value) in settings {
        answers.insert(key.clone(), value.clone());
    }
    let config = crate::app_config::get_new_app_config(app_folder, true, &answers, false).map_err(|e| e.to_string())?;
    let files = render_new_app(&config.config).map_err(|e| e.to_string())?;
    let old_hashes = saved.get("files").and_then(|f| f.as_object()).cloned().unwrap_or_default();

    let mut changed = Vec::new();
    let mut kept = Vec::new();
    let mut rebuild = false;
    for file in &files {
        let path = Path::new(app_folder).join(&file.path);
        let current = std::fs::read_to_string(&path).ok();
        if current.as_deref() == Some(file.content.as_str()) {
            continue;
        }
        let unedited = match (&current, old_hashes.get(&file.path).and_then(|h| h.as_str())) {
            (None, _) => true,
            (Some(text), Some(hash)) => content_hash(text.as_bytes()) == hash,
            (Some(_), None) => false,
        };
        if unedited {
            write_rendered_file(app_folder, file).map_err(|e| format!("{}: {}", file.path, e))?;
            changed.push(file.path.clone());
            if file.path.ends_with("partitions.csv") || file.path.ends_with("sdkconfig.defaults") || file.path.ends_with("features.cmake") {
                rebuild = true;
            }
        } else {
            let side = crate::app_new::RenderedFile { path: format!("{}.raftnew", file.path), content: file.content.clone() };
            write_rendered_file(app_folder, &side).map_err(|e| format!("{}: {}", side.path, e))?;
            kept.push(file.path.clone());
        }
    }
    let no_longer: Vec<String> = old_hashes.keys().filter(|p| !files.iter().any(|f| &f.path == *p))
        .filter(|p| Path::new(app_folder).join(p).exists()).cloned().collect();
    std::fs::write(&answers_path, answers_file_content(&config.answers, &files)).map_err(|e| e.to_string())?;

    if changed.is_empty() && kept.is_empty() {
        println!("No files change");
    }
    for p in &changed {
        println!("Updated: {}", p);
    }
    for p in &kept {
        println!("Edited since it was generated, so left as it is: {} (the new version is {}.raftnew - merge it by hand)", p, p);
    }
    for p in &no_longer {
        println!("No longer generated (left in place - delete it if it isn't needed): {}", p);
    }
    if rebuild {
        println!("Do a clean build (raft build -c): the partition table, sdkconfig or features changed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // The template from RaftCLI 1.20.1 to 1.24.1 is the current one without the check
    fn template_without_guard() -> String {
        let mut lines: Vec<&str> = CURRENT_TEMPLATE.lines().collect();
        let pos = lines.iter().position(|l| l.contains(GUARD_MARKER)).unwrap();
        lines.drain(pos..pos + GUARD_LINES.len());
        lines.join("\n") + "\n"
    }

    #[test]
    fn current_template_is_up_to_date() {
        assert!(cmake_update(CURRENT_TEMPLATE, Some("main")).is_none());
        assert!(cmake_update(CURRENT_TEMPLATE, None).is_none());
        // The guard lines match the template
        for line in GUARD_LINES {
            assert!(CURRENT_TEMPLATE.contains(line));
        }
    }

    #[test]
    fn check_is_added_to_older_template() {
        let old = template_without_guard();
        let update = cmake_update(&old, Some("v1.58.1")).unwrap();
        assert_eq!(update.kind, UpdateKind::AddBrokenLinkCheck);
        assert_eq!(update.new_content, CURRENT_TEMPLATE);

        // Line endings are kept
        let update = cmake_update(&old.replace('\n', "\r\n"), None).unwrap();
        assert_eq!(update.new_content, CURRENT_TEMPLATE.replace('\n', "\r\n"));

        // A changed file still gets the check, indented like the line it goes in front of
        let changed = format!("# My changes\n{}", old.replace(LOCAL_BOOTSTRAP_LINE, &format!("  {}", LOCAL_BOOTSTRAP_LINE)));
        let update = cmake_update(&changed, None).unwrap();
        assert!(update.new_content.starts_with("# My changes\n"));
        assert!(update.new_content.contains(&format!("  {}\n  {}\n  {}\n  {}", GUARD_LINES[0], GUARD_LINES[1], GUARD_LINES[2], LOCAL_BOOTSTRAP_LINE)));
        assert!(cmake_update(&update.new_content, None).is_none());
    }

    #[test]
    fn old_hardcoded_template_is_replaced_only_when_unchanged_and_raft_core_floats() {
        let old = OLD_HARDCODED_TEMPLATE.replace("{TAG}", "v1.37.1");
        let update = cmake_update(&old, Some("main")).unwrap();
        assert_eq!(update.kind, UpdateKind::ReplaceOldTemplate { bootstrap_tag: "v1.37.1".to_string() });
        assert_eq!(update.new_content, CURRENT_TEMPLATE);
        assert!(cmake_update(&old, Some("")).is_some());
        assert!(cmake_update(&old.replace('\n', "\r\n"), Some("my-branch")).is_some());

        // RaftCore pinned to a release, or not known
        assert!(cmake_update(&old, Some("v1.37.1")).is_none());
        assert!(cmake_update(&old, None).is_none());

        // Changed file
        let changed = old.replace("# Define the project dependencies", "# Define the project dependencies\nset(X 1)");
        assert!(cmake_update(&changed, Some("main")).is_none());
    }

    #[test]
    fn local_bootstrap_project_is_left_alone() {
        let local = "include(\"${CMAKE_SOURCE_DIR}/../scripts/RaftBootstrap.cmake\")\n";
        assert!(cmake_update(local, Some("main")).is_none());
    }

    #[test]
    fn rerender_changes_unedited_files_and_keeps_edited_ones() {
        use crate::app_new::{generate_new_app_with_answers, ANSWERS_FILE};
        let folder = std::env::temp_dir().join(format!("raftcli_test_rerender_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let app = folder.to_str().unwrap();
        let config = crate::app_config::get_new_app_config("MyApp", true, &HashMap::new(), false).unwrap();
        generate_new_app_with_answers(app, config.config, Some(&config.answers)).unwrap();
        let sdkconfig = folder.join("systypes/MyApp/sdkconfig.defaults");
        let edited = std::fs::read_to_string(&sdkconfig).unwrap() + "\n# my own setting\nCONFIG_FOO=y\n";
        std::fs::write(&sdkconfig, &edited).unwrap();
        let password_before = std::fs::read_to_string(folder.join("systypes/MyApp/SysTypes.json")).unwrap();

        let settings: HashMap<String, String> = [("flash_size".to_string(), "8".to_string())].into_iter().collect();
        rerender_project(app, &settings).unwrap();
        // Not edited: updated (8 MB layout)
        assert!(std::fs::read_to_string(folder.join("systypes/MyApp/partitions.csv")).unwrap().contains("0x300000"));
        // Edited: left as it is, the new version beside it
        assert_eq!(std::fs::read_to_string(&sdkconfig).unwrap(), edited);
        assert!(std::fs::read_to_string(folder.join("systypes/MyApp/sdkconfig.defaults.raftnew")).unwrap()
                    .contains("CONFIG_ESPTOOLPY_FLASHSIZE_8MB=y"));
        // Unchanged: the same AP password, and the new answer saved
        assert_eq!(std::fs::read_to_string(folder.join("systypes/MyApp/SysTypes.json")).unwrap(), password_before);
        assert!(std::fs::read_to_string(folder.join(ANSWERS_FILE)).unwrap().contains("\"flash_size\": \"8\""));
        // Unknown settings are refused
        let bad: HashMap<String, String> = [("no_such".to_string(), "1".to_string())].into_iter().collect();
        assert!(rerender_project(app, &bad).is_err());
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
