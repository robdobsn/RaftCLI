// RaftCLI: check that a project's RaftBootstrap.cmake matches the version of RaftCore it uses
// Rob Dobson 2026
//
// A Raft project's CMakeLists.txt downloads RaftBootstrap.cmake (stage 1 of the build bootstrap) from a RaftCore
// release and that script then fetches RaftCore and runs the rest of the bootstrap from it. Projects created by
// current versions of "raft new" derive the release to download from RaftCore@<tag> in features.cmake so the
// two always match. Older projects have the URL of a specific release written into CMakeLists.txt, which
// over time ends up far older than the RaftCore it is used with (and the old scripts use CMake features
// which are deprecated). This is only a warning and only for a RaftCore which floats (main, a branch, no tag):
// a project which pins RaftCore to a release is locked down deliberately and isn't warned about, and neither
// is one which uses the derived URL or includes a local bootstrap.

use regex::Regex;

/// Remove comment lines so that commented-out code and explanations aren't matched
fn without_comment_lines(content: &str) -> String {
    content.lines().filter(|line| !line.trim_start().starts_with('#')).collect::<Vec<_>>().join("\n")
}

/// The release tag in a bootstrap URL which is written literally in CMakeLists.txt (None if the URL is
/// derived e.g. .../releases/download/${_raft_core_tag}/RaftBootstrap.cmake or is releases/latest)
pub fn hardcoded_bootstrap_tag(cmakelists: &str) -> Option<String> {
    let re = Regex::new(r"releases/download/([A-Za-z0-9._\-]+)/RaftBootstrap\.cmake").unwrap();
    re.captures(&without_comment_lines(cmakelists)).map(|caps| caps[1].to_string())
}

/// The RaftCore tag in a features.cmake file - Some("") if RaftCore is listed without a tag, None if it isn't listed
pub fn raft_core_tag(features: &str) -> Option<String> {
    let re = Regex::new(r#"(?m)(?:^|[\s"(])RaftCore(?:[@#]([^\s")]+))?(?:$|[\s")])"#).unwrap();
    re.captures(&without_comment_lines(features)).map(|caps| caps.get(1).map_or("", |m| m.as_str()).to_string())
}

/// A warning if the bootstrap is pinned to a release which isn't the RaftCore version being used
pub fn bootstrap_skew_warning(cmakelists: &str, sys_type_features: Option<&str>, common_features: Option<&str>)
            -> Option<String> {
    let bootstrap_tag = hardcoded_bootstrap_tag(cmakelists)?;
    // The SysType's features.cmake normally includes the Common one so look in both
    let core_tag = sys_type_features.and_then(raft_core_tag)
        .or_else(|| common_features.and_then(raft_core_tag))?;
    if core_tag == bootstrap_tag {
        return None;
    }
    // A project which pins RaftCore to a release is locked down deliberately and is reproducible even if its
    // bootstrap is from a different release (that is how it was built and tested) so it isn't warned about.
    // The problem case is a RaftCore which floats (main, a branch or no tag) as the pinned bootstrap then
    // gets further and further behind the RaftCore it is used with.
    let core_is_pinned_release = Regex::new(r"^v?\d+\.\d+").unwrap().is_match(&core_tag);
    if core_is_pinned_release {
        return None;
    }
    let core_description = if core_tag.is_empty() { "RaftCore (latest)".to_string() } else { format!("RaftCore@{}", core_tag) };
    Some(format!(
        "Warning: CMakeLists.txt downloads RaftBootstrap.cmake from RaftCore release {} but features.cmake uses {}\n\
        which is not a fixed release, so the bootstrap script gets further and further behind the RaftCore it is\n\
        used with. To fix this replace the bootstrap section of CMakeLists.txt with the one from a project created\n\
        by a current \"raft new\" (which works out the release to download from RaftCore@<tag> in features.cmake)\n\
        and then do a clean build (raft build -c).",
        bootstrap_tag, core_description))
}

/// Check a project folder
pub fn check_bootstrap_version(app_folder: &str, sys_type: &str) -> Option<String> {
    let app_path = std::path::Path::new(app_folder);
    let cmakelists = std::fs::read_to_string(app_path.join("CMakeLists.txt")).ok()?;
    let systypes = app_path.join("systypes");
    let sys_type_features = std::fs::read_to_string(systypes.join(sys_type).join("features.cmake")).ok();
    let common_features = std::fs::read_to_string(systypes.join("Common").join("features.cmake")).ok();
    bootstrap_skew_warning(&cmakelists, sys_type_features.as_deref(), common_features.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD_STYLE: &str = "# Raft Project\ncmake_minimum_required(VERSION 3.16)\n\
        set(BOOTSTRAP_URL \"https://github.com/robdobsn/RaftCore/releases/download/v1.37.1/RaftBootstrap.cmake\")\n\
        file(DOWNLOAD ${BOOTSTRAP_URL} \"${CMAKE_BINARY_DIR}/RaftBootstrap.cmake\")\n";

    fn features(component: &str) -> String {
        format!("set(IDF_TARGET \"esp32s3\")\nset(RAFT_COMPONENTS\n    {}\n    RaftSysMods@main\n)\n", component)
    }

    #[test]
    fn tags_are_found() {
        assert_eq!(hardcoded_bootstrap_tag(OLD_STYLE), Some("v1.37.1".to_string()));
        assert_eq!(hardcoded_bootstrap_tag("# .../releases/download/v1.37.1/RaftBootstrap.cmake\n"), None);
        assert_eq!(raft_core_tag(&features("RaftCore@main")), Some("main".to_string()));
        assert_eq!(raft_core_tag(&features("RaftCore@v1.52.1")), Some("v1.52.1".to_string()));
        assert_eq!(raft_core_tag(&features("RaftCore#abc123")), Some("abc123".to_string()));
        assert_eq!(raft_core_tag(&features("RaftCore")), Some("".to_string()));
        assert_eq!(raft_core_tag("set(RAFT_COMPONENTS RaftCore@v1.2.3)"), Some("v1.2.3".to_string()));
        assert_eq!(raft_core_tag(&features("# RaftCore@v1.0.0")), None);
        assert_eq!(raft_core_tag("set(RAFT_COMPONENTS\n    RaftCoreExtras@v9\n)\n"), None);
    }

    #[test]
    fn warns_only_when_a_pinned_bootstrap_differs_from_raft_core() {
        // Old bootstrap with floating or different RaftCore
        let warning = bootstrap_skew_warning(OLD_STYLE, Some(&features("RaftCore@main")), None).unwrap();
        assert!(warning.contains("v1.37.1") && warning.contains("RaftCore@main"));
        assert!(bootstrap_skew_warning(OLD_STYLE, Some(&features("RaftCore@my-feature-branch")), None).is_some());
        assert!(bootstrap_skew_warning(OLD_STYLE, Some(&features("RaftCore")), None).is_some());

        // RaftCore pinned to a release (even a different one) is a deliberately locked-down project - no warning
        assert_eq!(bootstrap_skew_warning(OLD_STYLE, Some(&features("RaftCore@v1.54.1")), None), None);
        assert_eq!(bootstrap_skew_warning(OLD_STYLE, Some(&features("RaftCore@1.20.2")), None), None);

        // RaftCore tag in the Common features.cmake
        let sys_type = "include(\"${BUILD_CONFIG_DIR}/../Common/features.cmake\")\n";
        assert!(bootstrap_skew_warning(OLD_STYLE, Some(sys_type), Some(&features("RaftCore@main"))).is_some());

        // Deliberately pinned to the same release - no warning
        assert_eq!(bootstrap_skew_warning(OLD_STYLE, Some(&features("RaftCore@v1.37.1")), None), None);

        // Can't tell which RaftCore is used - no warning
        assert_eq!(bootstrap_skew_warning(OLD_STYLE, None, None), None);
    }

    #[test]
    fn current_template_never_warns() {
        let template = include_str!("../raft_templates/CMakeLists.txt");
        assert_eq!(hardcoded_bootstrap_tag(template), None);
        assert_eq!(bootstrap_skew_warning(template, Some(&features("RaftCore@main")), None), None);
        assert_eq!(bootstrap_skew_warning(template, Some(&features("RaftCore@v1.52.1")), None), None);

        // A project which includes a local bootstrap (e.g. RaftCore/unit_tests) doesn't warn either
        let local = "include(\"${CMAKE_SOURCE_DIR}/../scripts/RaftBootstrap.cmake\")\n";
        assert_eq!(bootstrap_skew_warning(local, Some(&features("RaftCore@main")), None), None);
    }
}
