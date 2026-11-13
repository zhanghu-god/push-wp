//! Tauri 命令层：前端通过 invoke 调用这些函数。

use std::fs;
use std::path::Path;

use serde::Serialize;
use tauri::AppHandle;

use crate::cache::{entry_key, load_site_cache, post_cache_key};
use crate::config::{self, Settings, Site};
use crate::posts::{self, PostDetail, PostSummary, Terms};
use crate::publish::{self, PublishRequest};
use crate::render;

/// 渲染结果
#[derive(Debug, Clone, Serialize)]
pub struct MarkdownDoc {
    pub html: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReadResult {
    pub content: String,
    pub html: String,
}

fn render_doc(content: &str, path: &str) -> MarkdownDoc {
    let p = Path::new(path);
    let base_dir = p.parent().unwrap_or(Path::new("."));
    // 头部元数据不再解析，整篇内容都作为正文渲染
    let html = render::render_body(content, base_dir);
    MarkdownDoc { html }
}

/// 打开并解析 Markdown 文件
#[tauri::command]
pub fn read_markdown(path: String) -> Result<ReadResult, String> {
    let content = fs::read_to_string(&path).map_err(|e| format!("打开文件失败：{e}"))?;
    let doc = render_doc(&content, &path);
    Ok(ReadResult {
        content,
        html: doc.html,
    })
}

/// 编辑后重新解析渲染（预览用）
#[tauri::command]
pub fn render_markdown(content: String, path: String) -> Result<MarkdownDoc, String> {
    Ok(render_doc(&content, &path))
}

#[tauri::command]
pub fn save_file(path: String, content: String) -> Result<(), String> {
    fs::write(&path, content).map_err(|e| format!("保存文件失败：{e}"))
}

#[tauri::command]
pub fn get_settings(app: AppHandle) -> Result<Settings, String> {
    config::load_settings(&app)
}

#[tauri::command]
pub fn save_settings(app: AppHandle, settings: Settings) -> Result<(), String> {
    config::save_settings(&app, &settings)
}

/// 查询某站点下当前文章对应的已缓存 ID（发布对话框展示用）
#[tauri::command]
pub fn get_cached_post(
    app: AppHandle,
    site: Site,
    url: Option<String>,
    file_path: String,
) -> Result<Option<String>, String> {
    let cache = load_site_cache(&app, &site)?;
    let key = entry_key(&post_cache_key(url.as_deref(), &file_path));
    Ok(cache.post.get(&key).map(|p| p.post_id.clone()))
}

/// 执行发布（阻塞式，Tauri 会在独立线程池运行同步命令）
#[tauri::command]
pub fn publish_post(app: AppHandle, request: PublishRequest) -> Result<String, String> {
    publish::publish(&app, &request)
}

/// 查询远程文章列表（分页/搜索/状态筛选）
#[tauri::command]
pub fn list_posts(
    site: Site,
    count: u32,
    offset: u32,
    status: String,
    search: String,
) -> Result<Vec<PostSummary>, String> {
    posts::list_posts(&site, count, offset, &status, &search)
}

/// 获取远程文章详情（含正文 HTML）
#[tauri::command]
pub fn get_post_detail(site: Site, post_id: String) -> Result<PostDetail, String> {
    posts::get_detail(&site, &post_id)
}

/// 删除远程文章并清理本地缓存
#[tauri::command]
pub fn delete_post(app: AppHandle, site: Site, post_id: String) -> Result<(), String> {
    posts::delete_post(&app, &site, &post_id)
}

/// 拉取站点已有分类与标签（发布表单候选项）
#[tauri::command]
pub fn get_terms(site: Site) -> Result<Terms, String> {
    posts::get_terms(&site)
}