//! 应用配置：站点列表等，持久化到应用配置目录下的 settings.json。

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

/// 一个 WordPress 站点的 XML-RPC 连接配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Site {
    pub name: String,
    /// xmlrpc.php 的完整地址，例如 https://example.com/xmlrpc.php
    pub url: String,
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub sites: Vec<Site>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            sites: vec![Site {
                name: "示例站点配置".into(),
                url: "https://www.example.com/xmlrpc.php".into(),
                username: "username".into(),
                password: String::new(),
            }],
        }
    }
}

/// 应用配置目录（不存在时自动创建）。
///
/// - 便携模式：exe 同级存在 `.portable` 标记文件时，使用 `exe目录\data\`，
///   配置与数据全部自包含，不污染系统目录；首次运行自动迁移旧配置。
/// - 默认：系统应用配置目录（%APPDATA%\com.pushwp.rust）。
pub fn config_dir(app: &AppHandle) -> Result<PathBuf, String> {
    if let Some(dir) = portable_data_dir() {
        migrate_legacy_config(app, &dir);
        fs::create_dir_all(&dir).map_err(|e| format!("无法创建便携数据目录：{e}"))?;
        return Ok(dir);
    }
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("无法获取配置目录：{e}"))?;
    fs::create_dir_all(&dir).map_err(|e| format!("无法创建配置目录：{e}"))?;
    Ok(dir)
}

/// 便携模式数据目录：exe 同级有 `.portable` 标记时返回 Some(exe目录/data)
fn portable_data_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.to_path_buf();
    if dir.join(".portable").exists() {
        Some(dir.join("data"))
    } else {
        None
    }
}

/// 便携版首次运行：把系统目录里的旧配置与缓存带过来（不覆盖已有文件）
fn migrate_legacy_config(app: &AppHandle, portable_dir: &PathBuf) {
    let settings = portable_dir.join("settings.json");
    if settings.exists() {
        return;
    }
    let Ok(legacy) = app.path().app_config_dir() else {
        return;
    };
    if !legacy.join("settings.json").exists() {
        return;
    }
    let _ = fs::create_dir_all(portable_dir);
    let _ = fs::copy(legacy.join("settings.json"), &settings);
    let legacy_cache = legacy.join("cache");
    if legacy_cache.is_dir() {
        let target = portable_dir.join("cache");
        let _ = fs::create_dir_all(&target);
        if let Ok(entries) = fs::read_dir(&legacy_cache) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_file() {
                    let dest = target.join(entry.file_name());
                    if !dest.exists() {
                        let _ = fs::copy(&p, &dest);
                    }
                }
            }
        }
    }
}

pub fn load_settings(app: &AppHandle) -> Result<Settings, String> {
    let path = config_dir(app)?.join("settings.json");
    if !path.exists() {
        return Ok(Settings::default());
    }
    let text = fs::read_to_string(&path).map_err(|e| format!("读取配置失败：{e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("配置文件格式错误：{e}"))
}

pub fn save_settings(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    let path = config_dir(app)?.join("settings.json");
    let text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    fs::write(&path, text).map_err(|e| format!("保存配置失败：{e}"))
}