//! Key-value string store with computed strings and dual-layer persistence.
//!
//! Two persistence layers, loaded in order (higher layer wins on conflict):
//!   1. `data/strings.json`       — single JSON file for short, single-line values
//!   2. `data/strings/<key>.md`   — individual Markdown files for multiline content
//!
//! Computed strings (`$`-prefixed) are server-derived, readonly, in-memory only.
//! They are merged into GET responses but cannot be written or deleted via the API.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::constant::DATA_DIR;

/// Path to the JSON string store file.
fn strings_json_path() -> PathBuf { Path::new(DATA_DIR).join("strings.json") }

/// Path to the directory containing per-key Markdown files.
fn strings_dir_path() -> PathBuf { Path::new(DATA_DIR).join("strings") }

// ── Store ────────────────────────────────────────────────────────────────────

pub struct StringStore {
    /// User-managed key-value pairs, persisted to disk.
    user: BTreeMap<String, String>,
    /// Server-derived readonly strings (`$`-prefixed), in-memory only.
    computed: BTreeMap<String, String>,
}

impl StringStore {
    pub fn new() -> Self {
        // Ensure data directories exist.
        let _ = std::fs::create_dir_all(strings_dir_path());

        let mut store = Self {
            user: BTreeMap::new(),
            computed: BTreeMap::new(),
        };
        store.load_from_disk();
        store
    }

    /// All entries merged: user store + computed (computed wins on conflict).
    pub fn get_all(&self) -> BTreeMap<String, String> {
        let mut result = self.user.clone();
        for (k, v) in &self.computed {
            result.insert(k.clone(), v.clone());
        }
        result
    }

    /// Set a user string.  Returns `Err` if the key is `$`-prefixed or invalid.
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), StringStoreError> {
        if key.starts_with('$') {
            return Err(StringStoreError::ComputedReadonly);
        }
        if !is_valid_key(key) {
            return Err(StringStoreError::InvalidKey);
        }

        self.user.insert(key.to_owned(), value.to_owned());

        // Persist: multiline → .md file (remove from JSON), single-line → JSON (remove .md).
        if is_multiline(value) {
            let _ = std::fs::write(strings_dir_path().join(format!("{key}.md")), value);
            remove_from_json(key);
        } else {
            let _ = std::fs::remove_file(strings_dir_path().join(format!("{key}.md")));
            save_to_json(key, value);
        }

        Ok(())
    }

    /// Delete a user string.  Returns `Err` if the key is `$`-prefixed or invalid.
    pub fn delete(&mut self, key: &str) -> Result<(), StringStoreError> {
        if key.starts_with('$') {
            return Err(StringStoreError::ComputedReadonly);
        }
        if !is_valid_key(key) {
            return Err(StringStoreError::InvalidKey);
        }

        self.user.remove(key);
        remove_from_json(key);
        let _ = std::fs::remove_file(strings_dir_path().join(format!("{key}.md")));

        Ok(())
    }

    /// Push a computed string.  Key must start with `$`.
    pub fn set_computed(&mut self, key: &str, value: String) {
        debug_assert!(key.starts_with('$'), "computed key must start with $");
        self.computed.insert(key.to_owned(), value);
    }

    /// Remove a computed string.
    pub fn clear_computed(&mut self, key: &str) {
        self.computed.remove(key);
    }

    /// Reload all user strings from disk (called by `POST /refresh`).
    pub fn reload(&mut self) {
        self.user.clear();
        self.load_from_disk();
        log::info!("reloaded {} user string entries", self.user.len());
    }

    /// Load user strings from both disk layers.
    fn load_from_disk(&mut self) {
        // Layer 1: strings.json (lower priority).
        for (k, v) in load_json_map(&strings_json_path()) {
            self.user.insert(k, v);
        }

        // Layer 2: data/strings/*.md (higher priority, overwrites JSON).
        if let Ok(entries) = std::fs::read_dir(strings_dir_path()) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if let Some(key) = name.strip_suffix(".md")
                    && is_valid_key(key)
                        && let Ok(content) = std::fs::read_to_string(entry.path()) {
                            self.user.insert(key.to_owned(), content);
                        }
            }
        }

        log::info!("loaded {} user string entries", self.user.len());
    }
}

// ── Error type ───────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum StringStoreError {
    ComputedReadonly,
    InvalidKey,
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Key must be alphanumeric, hyphens, underscores.
fn is_valid_key(key: &str) -> bool {
    !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// A value is multiline if it contains a newline after trimming trailing whitespace.
fn is_multiline(value: &str) -> bool { value.trim_end().contains('\n') }

/// Load strings.json as a `BTreeMap`.  Returns empty map on missing file.
///
/// Panics on corrupt JSON (strict mode, matches TS behavior).
fn load_json_map(path: &Path) -> BTreeMap<String, String> {
    let Ok(content) = std::fs::read_to_string(path) else { return BTreeMap::new() };
    serde_json::from_str(&content)
        .unwrap_or_else(|e| panic!("corrupt strings.json: {e}"))
}

/// Update a single key in strings.json (load → set → save).
fn save_to_json(key: &str, value: &str) {
    let path = strings_json_path();
    let mut map: BTreeMap<String, String> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    map.insert(key.to_owned(), value.to_owned());
    let json = serde_json::to_string_pretty(&map).expect("JSON serialization failed");
    let _ = std::fs::write(path, json);
}

/// Remove a key from strings.json (load → delete → save).
fn remove_from_json(key: &str) {
    let path = strings_json_path();
    let Ok(content) = std::fs::read_to_string(&path) else { return };
    let Ok(mut map): Result<BTreeMap<String, String>, _> = serde_json::from_str(&content) else { return };
    if map.remove(key).is_some() {
        let json = serde_json::to_string_pretty(&map).expect("JSON serialization failed");
        let _ = std::fs::write(path, json);
    }
}
