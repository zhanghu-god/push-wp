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

/// 应用配置目录（不存在时自动创建）
pub fn config_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("无法获取配置目录：{e}"))?;
    fs::create_dir_all(&dir).map_err(|e| format!("无法创建配置目录：{e}"))?;
    Ok(dir)
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