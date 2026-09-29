//! The installers and the release workflow must advertise the same platforms.
//!
//! `scripts/install.sh` detects the host and downloads
//! `alfred-v<VERSION>-<PLATFORM>.tar.gz`; `scripts/install.ps1` downloads the
//! `windows-x64` zip. `.github/workflows/release.yml` is what actually builds
//! and publishes those archives. When the two lists drift apart the failure is
//! invisible until an install on the missing platform 404s.
//!
//! This test parses both sides out of the files on disk and fails when they do
//! not agree, so the disagreement surfaces at `cargo test` / release time
//! instead of at install time. It is deliberately dependency-free: the matrix
//! and the platform mappings are simple enough to read with string scans, and
//! pulling in a YAML or Bash parser for this would not be worth it.

mod release_matrix {
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::PathBuf;

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    fn read(relative: &str) -> String {
        let path = repo_root().join(relative);
        fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
    }

    fn is_platform(value: &str) -> bool {
        !value.is_empty()
            && value
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    }

    /// Every `platform: <value>` entry in the workflow's build matrix.
    fn workflow_platforms(yaml: &str) -> BTreeSet<String> {
        let mut platforms = BTreeSet::new();
        for line in yaml.lines() {
            let trimmed = line.trim_start();
            // Matrix entries may be written `- platform: x` or `platform: x`.
            let trimmed = trimmed.strip_prefix("- ").unwrap_or(trimmed);
            let Some(rest) = trimmed.strip_prefix("platform:") else {
                continue;
            };
            let value = rest.trim().trim_matches('"').trim_matches('\'');
            if value.is_empty() {
                continue;
            }
            assert!(
                is_platform(value),
                "unexpected platform value in release.yml: {value:?}"
            );
            platforms.insert(value.to_string());
        }
        platforms
    }

    /// The `PLATFORM="<value>"` values the Unix installer can emit.
    ///
    /// The scan skips `PI_PLATFORM="..."` (preceded by `_`), which is Pi's
    /// platform vocabulary, not Alfred's.
    fn shell_installer_platforms(script: &str) -> BTreeSet<String> {
        const NEEDLE: &str = "PLATFORM=\"";
        let mut platforms = BTreeSet::new();
        for line in script.lines() {
            let mut cursor = 0;
            while let Some(offset) = line[cursor..].find(NEEDLE) {
                let start_of_key = cursor + offset;
                let preceded_by_ident = line[..start_of_key]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
                let start_of_value = start_of_key + NEEDLE.len();
                let Some(end_of_value) = line[start_of_value..].find('"') else {
                    break;
                };
                let value = &line[start_of_value..start_of_value + end_of_value];
                if !preceded_by_ident && is_platform(value) {
                    platforms.insert(value.to_string());
                }
                cursor = start_of_value + end_of_value + 1;
            }
        }
        platforms
    }

    /// The platform `scripts/install.ps1` downloads (`$AlfredPlatform = "..."`).
    fn windows_installer_platforms(script: &str) -> BTreeSet<String> {
        let mut platforms = BTreeSet::new();
        for line in script.lines() {
            let Some(rest) = line.trim().strip_prefix("$AlfredPlatform") else {
                continue;
            };
            let Some(start) = rest.find('"') else { continue };
            let Some(end) = rest[start + 1..].find('"') else {
                continue;
            };
            let value = &rest[start + 1..start + 1 + end];
            assert!(
                is_platform(value),
                "unexpected platform value in install.ps1: {value:?}"
            );
            platforms.insert(value.to_string());
        }
        platforms
    }

    fn assert_agree(advertised: &BTreeSet<String>, built: &BTreeSet<String>) {
        assert!(
            !advertised.is_empty(),
            "no advertised platforms were parsed from the installers"
        );
        assert!(
            !built.is_empty(),
            "no platforms were parsed from the release matrix"
        );

        let never_built: Vec<&String> = advertised.difference(built).collect();
        let never_installed: Vec<&String> = built.difference(advertised).collect();
        assert!(
            never_built.is_empty() && never_installed.is_empty(),
            "installer platforms and the release matrix disagree\n  \
             advertised by an installer but not built: {never_built:?}\n  \
             built but advertised by no installer:     {never_installed:?}"
        );
    }

    #[test]
    fn installer_platforms_match_release_matrix() {
        let built = workflow_platforms(&read(".github/workflows/release.yml"));

        // Union the Unix and Windows installers: `detect_platform` is Unix-only
        // and `install.ps1` is Windows-only, so together they are the full set
        // of platforms a user can install.
        let mut advertised = shell_installer_platforms(&read("scripts/install.sh"));
        advertised.extend(windows_installer_platforms(&read("scripts/install.ps1")));

        assert_agree(&advertised, &built);
    }

    #[test]
    fn parsers_read_the_real_files() {
        // Guard the guard: if a refactor renames a file or changes the syntax,
        // the parsing functions must not silently return nothing and let the
        // agreement test pass on two empty sets.
        let built = workflow_platforms(&read(".github/workflows/release.yml"));
        assert!(
            built.contains("linux-x64"),
            "release matrix parsing lost linux-x64: {built:?}"
        );

        let unix = shell_installer_platforms(&read("scripts/install.sh"));
        assert!(
            unix.contains("linux-x64") && unix.contains("linux-arm64"),
            "unix installer platform parsing is broken: {unix:?}"
        );

        let windows = windows_installer_platforms(&read("scripts/install.ps1"));
        assert!(
            windows.contains("windows-x64"),
            "windows installer platform parsing is broken: {windows:?}"
        );
    }
}
