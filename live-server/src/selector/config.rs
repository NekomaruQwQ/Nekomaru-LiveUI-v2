//! Selector config: preset parsing, pattern matching, persistence.
//!
//! Each preset is a flat `Vec<String>` of pattern entries.  Entries are
//! include rules by default; `@exclude` prefix marks an exclusion rule.
//! Include entries may carry a `@mode` prefix (e.g. `@code devenv.exe`)
//! that is pushed as the `$liveMode` computed string on capture switch.
//!
//! Full pattern format: `[@mode] <exePath>[@<windowTitle>]`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ── Constants ────────────────────────────────────────────────────────────────

const DATA_DIR: &str = "data";
const CONFIG_FILENAME: &str = "selector-config.json";

fn config_path() -> PathBuf { Path::new(DATA_DIR).join(CONFIG_FILENAME) }

const DEFAULT_PRESET_NAME: &str = "default";

fn default_presets() -> HashMap<String, Vec<String>> {
    let mut m = HashMap::new();
    m.insert(DEFAULT_PRESET_NAME.to_owned(), vec![
        "@code devenv.exe".into(),
        "@code C:/Program Files/Microsoft Visual Studio Code/Code.exe".into(),
        "@code C:/Program Files/JetBrains/".into(),
        "@game D:/7-Games/".into(),
        "@game D:/7-Games.Steam/steamapps/common/".into(),
        "@game E:/Nekomaru-Games/".into(),
        "@game E:/SteamLibrary/steamapps/common/".into(),
        "@exclude gogh.exe".into(),
        "@exclude vtube studio.exe".into(),
    ]);
    m
}

// ── Preset Config ────────────────────────────────────────────────────────────

/// Full config shape persisted to disk.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PresetConfig {
    pub preset: String,
    pub presets: HashMap<String, Vec<String>>,
}

impl PresetConfig {
    /// Load from disk.  Falls back to defaults if the file is missing.
    /// Panics on corrupt JSON (matches TS strict mode).
    pub fn load() -> Self {
        let path = config_path();
        let Ok(content) = std::fs::read_to_string(&path) else {
            log::info!("no selector config found, using defaults");
            return Self { preset: DEFAULT_PRESET_NAME.into(), presets: default_presets() };
        };

        // Try parsing.  If it's a legacy format, migrate.
        let raw: serde_json::Value = serde_json::from_str(&content)
            .unwrap_or_else(|e| panic!("corrupt selector-config.json: {e}"));

        let preset = raw.get("preset")
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_PRESET_NAME)
            .to_owned();

        let presets_val = raw.get("presets");
        let mut presets = HashMap::new();

        if let Some(obj) = presets_val.and_then(|v| v.as_object()) {
            for (name, value) in obj {
                if let Some(arr) = value.as_array() {
                    // Modern format: flat string array.
                    let entries: Vec<String> = arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_owned()))
                        .collect();
                    presets.insert(name.clone(), entries);
                } else if let Some(legacy) = value.as_object() {
                    // Legacy format: { include: [...], exclude: [...] }
                    let include = legacy.get("include")
                        .and_then(|v| v.as_array())
                        .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_owned())).collect::<Vec<_>>())
                        .unwrap_or_default();
                    let exclude = legacy.get("exclude")
                        .and_then(|v| v.as_array())
                        .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_owned())).collect::<Vec<_>>())
                        .unwrap_or_default();
                    let mut entries = include;
                    for e in exclude {
                        entries.push(format!("@exclude {e}"));
                    }
                    log::info!("migrated legacy preset \"{name}\" to flat format");
                    presets.insert(name.clone(), entries);
                }
            }
        }

        if presets.is_empty() {
            presets = default_presets();
        }

        log::info!("loaded selector config: preset=\"{preset}\", {} preset(s)", presets.len());
        Self { preset, presets }
    }

    /// Persist to disk.
    pub fn save(&self) {
        let path = config_path();
        let json = serde_json::to_string_pretty(self).expect("JSON serialization failed");
        let _ = std::fs::write(path, json);
    }
}

// ── Pattern Parsing ──────────────────────────────────────────────────────────

/// A parsed config pattern with optional mode tag and title filter.
#[derive(Debug, Clone)]
pub struct ParsedPattern {
    /// `@mode` tag (e.g. `"code"`, `"game"`, `"exclude"`).  `None` if no prefix.
    pub mode: Option<String>,
    pub exe_path: String,
    /// Window title filter.  `None` if the pattern has no `@` separator.
    pub title: Option<String>,
}

/// Parse a config string with optional `@mode ` prefix and
/// `<exePath>[@<windowTitle>]` body.
pub fn parse_pattern(pattern: &str) -> ParsedPattern {
    let mut mode: Option<String> = None;
    let mut body = pattern;

    // Extract leading `@mode ` prefix (distinguished from `@title` by the space).
    if body.starts_with('@') {
        if let Some(space_idx) = body.find(' ') {
            if space_idx > 1 {
                mode = Some(body[1..space_idx].to_owned());
                body = &body[space_idx + 1..];
            }
        }
    }

    let (exe_path, title) = match body.find('@') {
        Some(idx) => (body[..idx].to_owned(), Some(body[idx + 1..].to_owned())),
        None => (body.to_owned(), None),
    };

    ParsedPattern { mode, exe_path, title }
}

/// Test whether a window matches a parsed pattern.
///
/// - Exe-path matching: substring match with normalized separators (`/` and `\`
///   interchangeable).  `case_insensitive` controls exe-path comparison.
/// - Title matching: always case-insensitive substring match.
/// - Both parts must match (AND) when both are present.
pub fn matches_parsed(
    parsed: &ParsedPattern,
    executable_path: &str,
    window_title: &str,
    case_insensitive: bool,
) -> bool {
    // Check exe-path part.
    if !parsed.exe_path.is_empty() {
        let haystack = executable_path.replace('\\', "/");
        let needle = parsed.exe_path.replace('\\', "/");
        let matches = if case_insensitive {
            haystack.to_lowercase().contains(&needle.to_lowercase())
        } else {
            haystack.contains(&needle)
        };
        if !matches { return false; }
    }

    // Check title part.
    if let Some(ref title_pattern) = parsed.title {
        if !title_pattern.is_empty()
            && !window_title.to_lowercase().contains(&title_pattern.to_lowercase())
        {
            return false;
        }
    }

    true
}

/// Determine whether a window should be captured based on the active preset.
///
/// Returns `None` if the window should not be captured.
/// Returns `Some(mode)` with the mode tag from the first matching include
/// entry (which may itself be `None` if the pattern has no `@mode` prefix).
pub fn should_capture(
    patterns: &[String],
    executable_path: &str,
    title: &str,
) -> Option<Option<String>> {
    let mut matched_mode: Option<String> = None;
    let mut included = false;

    for raw in patterns {
        let parsed = parse_pattern(raw);
        if parsed.mode.as_deref() == Some("exclude") {
            // Exclude entries use case-insensitive exe-path matching.
            if matches_parsed(&parsed, executable_path, title, true) {
                return None;
            }
        } else if !included && matches_parsed(&parsed, executable_path, title, false) {
            included = true;
            matched_mode = parsed.mode;
        }
    }

    if included { Some(matched_mode) } else { None }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_exe() {
        let p = parse_pattern("devenv.exe");
        assert!(p.mode.is_none());
        assert_eq!(p.exe_path, "devenv.exe");
        assert!(p.title.is_none());
    }

    #[test]
    fn parse_mode_prefix() {
        let p = parse_pattern("@code devenv.exe");
        assert_eq!(p.mode.as_deref(), Some("code"));
        assert_eq!(p.exe_path, "devenv.exe");
        assert!(p.title.is_none());
    }

    #[test]
    fn parse_exe_with_title() {
        let p = parse_pattern("@code Code.exe@LiveUI");
        assert_eq!(p.mode.as_deref(), Some("code"));
        assert_eq!(p.exe_path, "Code.exe");
        assert_eq!(p.title.as_deref(), Some("LiveUI"));
    }

    #[test]
    fn parse_exclude() {
        let p = parse_pattern("@exclude gogh.exe");
        assert_eq!(p.mode.as_deref(), Some("exclude"));
        assert_eq!(p.exe_path, "gogh.exe");
    }

    #[test]
    fn matches_exe_path_substring() {
        let p = parse_pattern("devenv.exe");
        assert!(matches_parsed(&p, "C:\\Program Files\\devenv.exe", "Window", false));
        assert!(!matches_parsed(&p, "C:\\Program Files\\code.exe", "Window", false));
    }

    #[test]
    fn matches_path_separator_normalization() {
        let p = parse_pattern("C:/Program Files/JetBrains/");
        assert!(matches_parsed(&p, "C:\\Program Files\\JetBrains\\idea64.exe", "", false));
    }

    #[test]
    fn matches_title_case_insensitive() {
        let p = parse_pattern("Code.exe@liveui");
        assert!(matches_parsed(&p, "C:\\Code.exe", "Nekomaru LiveUI v2", false));
        assert!(!matches_parsed(&p, "C:\\Code.exe", "Some Other Window", false));
    }

    #[test]
    fn should_capture_include_and_exclude() {
        let patterns = vec![
            "@code devenv.exe".into(),
            "@exclude gogh.exe".into(),
        ];

        // devenv matches include.
        let result = should_capture(&patterns, "C:\\devenv.exe", "Test");
        assert_eq!(result, Some(Some("code".into())));

        // gogh matches exclude — vetoed.
        let result = should_capture(&patterns, "C:\\gogh.exe", "Test");
        assert!(result.is_none());

        // unknown exe — no match.
        let result = should_capture(&patterns, "C:\\notepad.exe", "Test");
        assert!(result.is_none());
    }

    #[test]
    fn exclude_takes_priority() {
        let patterns = vec![
            "@game D:/7-Games/".into(),
            "@exclude vtube studio.exe".into(),
        ];

        // VTube Studio is under 7-Games but excluded.
        let result = should_capture(&patterns, "D:/7-Games/vtube studio.exe", "VTube");
        assert!(result.is_none());
    }
}
