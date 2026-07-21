use crate::backend::{BackendKind, available_kinds};
use directories::{BaseDirs, ProjectDirs};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

pub const DEFAULT_PROFILE_NAME: &str = "default";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AppConfig {
    pub profiles: Vec<Profile>,
    pub selected_profile: usize,
    #[serde(default)]
    pub run_in_tray: bool,
    #[serde(default)]
    pub launch_at_login: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Profile {
    pub name: String,
    pub backend: BackendKind,
    #[serde(default, skip_serializing)]
    pub config_path: String,
    pub auto_reload: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        let backend = available_kinds()
            .into_iter()
            .next()
            .unwrap_or(BackendKind::Leaf);
        Self {
            profiles: vec![Profile {
                name: DEFAULT_PROFILE_NAME.to_string(),
                backend,
                config_path: profile_config_path(DEFAULT_PROFILE_NAME, backend),
                auto_reload: true,
            }],
            selected_profile: 0,
            run_in_tray: false,
            launch_at_login: false,
        }
    }
}

pub fn load() -> AppConfig {
    let _ = fs::create_dir_all(proxy_config_dir());
    let Some(path) = config_path() else {
        return AppConfig::default();
    };
    let Ok(contents) = fs::read_to_string(path) else {
        return AppConfig::default();
    };
    let mut config: AppConfig = serde_json::from_str(&contents).unwrap_or_default();
    normalize_profiles(&mut config);
    config
}

pub fn save(config: &AppConfig) -> std::io::Result<()> {
    let Some(path) = config_path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut config = config.clone();
    normalize_profiles(&mut config);
    let contents = serde_json::to_string_pretty(&config)?;
    fs::write(path, contents)
}

fn config_path() -> Option<PathBuf> {
    ProjectDirs::from("moe", "rikaaa0928", "riproxy-gui")
        .map(|dirs| dirs.config_dir().join("config.json"))
}

pub fn default_config_path(kind: BackendKind) -> String {
    let file_name = match kind {
        BackendKind::Leaf => "config.conf",
        BackendKind::Rog => "config.toml",
    };
    proxy_config_dir()
        .join(file_name)
        .to_string_lossy()
        .into_owned()
}

pub fn profile_config_path(profile_name: &str, kind: BackendKind) -> String {
    let normalized_name = normalize_profile_name(profile_name);
    if normalized_name == DEFAULT_PROFILE_NAME {
        return default_config_path(kind);
    }

    let extension = match kind {
        BackendKind::Leaf => "conf",
        BackendKind::Rog => "toml",
    };
    proxy_config_dir()
        .join(format!("{normalized_name}.config.{extension}"))
        .to_string_lossy()
        .into_owned()
}

pub fn normalize_profile_name(name: &str) -> String {
    let mut normalized = String::new();
    let mut last_was_separator = false;

    for ch in name.trim().to_lowercase().chars() {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            normalized.push(ch);
            last_was_separator = false;
        } else if matches!(ch, '-' | '_' | ' ' | '.') && !last_was_separator {
            normalized.push('-');
            last_was_separator = true;
        }
    }

    let normalized = normalized.trim_matches('-').to_string();
    if normalized.is_empty() {
        "profile".to_string()
    } else {
        normalized
    }
}

pub fn proxy_config_dir() -> PathBuf {
    BaseDirs::new()
        .map(|dirs| dirs.home_dir().join(".config/riproxy"))
        .unwrap_or_else(|| PathBuf::from(".config/riproxy"))
}

pub fn normalize_profiles(config: &mut AppConfig) {
    if config.profiles.is_empty() {
        *config = AppConfig::default();
        return;
    }

    let mut used = HashSet::new();
    for profile in &mut config.profiles {
        profile.name = unique_profile_name(&profile.name, &used);
        used.insert(profile.name.clone());
        profile.config_path = profile_config_path(&profile.name, profile.backend);
    }
    if !config.profiles.is_empty() {
        config.selected_profile = config
            .selected_profile
            .min(config.profiles.len().saturating_sub(1));
    }
}

fn unique_profile_name(name: &str, used: &HashSet<String>) -> String {
    let base = normalize_profile_name(name);
    if !used.contains(&base) {
        return base;
    }

    for index in 2.. {
        let candidate = format!("{base}-{index}");
        if !used.contains(&candidate) {
            return candidate;
        }
    }

    unreachable!("usize range should always find a unique profile name")
}
