//! 发布缓存：文章 slug → 文章 ID、图片文件名 → 远程地址。
//!
//! 对应原版 PublishCache.ts + electron-store。每个站点一个 JSON 文件，
//! 存放在应用配置目录的 cache/ 下。键统一小写、'.' 替换为 '!'（沿用原版规则）。

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::config::{config_dir, Site};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostCache {
    pub post_id: String,
    #[serde(default)]
    pub thumbnail_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageCache {
    pub image_id: String,
    pub image_url: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SiteCache {
    /// slug → 文章缓存
    #[serde(default)]
    pub post: HashMap<String, PostCache>,
    /// 图片文件名 → 媒体缓存
    #[serde(default)]
    pub media: HashMap<String, ImageCache>,
}

/// 站点唯一键：url&&username 中的特殊字符全部替换为 !
pub fn site_key(site: &Site) -> String {
    format!("{}&&{}", site.url, site.username)
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '!' })
        .collect()
}

/// 缓存条目键：小写 + '.' → '!'
pub fn entry_key(object: &str) -> String {
    object.to_lowercase().replace('.', "!")
}

/// 文章缓存键：填了固定链接用 slug，否则用文件绝对路径
pub fn post_cache_key(url: Option<&str>, file_path: &str) -> String {
    match url.map(str::trim).filter(|u| !u.is_empty()) {
        Some(slug) => slug.to_string(),
        None => format!("file:{file_path}"),
    }
}

fn cache_path(app: &AppHandle, site: &Site) -> Result<PathBuf, String> {
    let dir = config_dir(app)?.join("cache");
    fs::create_dir_all(&dir).map_err(|e| format!("无法创建缓存目录：{e}"))?;
    Ok(dir.join(format!("{}.json", site_key(site))))
}

pub fn load_site_cache(app: &AppHandle, site: &Site) -> Result<SiteCache, String> {
    let path = cache_path(app, site)?;
    if !path.exists() {
        return Ok(SiteCache::default());
    }
    let text = fs::read_to_string(&path).map_err(|e| format!("读取缓存失败：{e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("缓存文件格式错误：{e}"))
}

pub fn save_site_cache(app: &AppHandle, site: &Site, cache: &SiteCache) -> Result<(), String> {
    let path = cache_path(app, site)?;
    let text = serde_json::to_string_pretty(cache).map_err(|e| e.to_string())?;
    fs::write(&path, text).map_err(|e| format!("保存缓存失败：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_site_key() {
        let site = Site {
            name: "test".into(),
            url: "https://x.com/xmlrpc.php".into(),
            username: "admin".into(),
            password: String::new(),
        };
        let key = site_key(&site);
        assert!(!key.contains('/'));
        assert!(!key.contains('.'));
        assert!(key.contains("admin"));
    }

    #[test]
    fn test_entry_key() {
        assert_eq!(entry_key("My-Post.Slug"), "my-post!slug");
        assert_eq!(entry_key("图片.PNG"), "图片!png");
    }

    #[test]
    fn test_post_cache_key() {
        assert_eq!(post_cache_key(Some(" my-post "), "C:/a.md"), "my-post");
        assert_eq!(post_cache_key(None, "C:/a.md"), "file:C:/a.md");
        assert_eq!(post_cache_key(Some("   "), "C:/a.md"), "file:C:/a.md");
    }
}