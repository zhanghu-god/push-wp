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
use crate::update::{self, PreparedUpdate, UpdateInfo};

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

// ---------- 自动更新 ----------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub repo: String,
    pub data_dir: String,
}

/// 关于面板信息：版本、仓库、数据目录
#[tauri::command]
pub fn get_app_info(app: AppHandle) -> Result<AppInfo, String> {
    let dir = config::config_dir(&app)?;
    Ok(AppInfo {
        version: update::current_version().to_string(),
        repo: "zhanghu-god/push-wp".to_string(),
        data_dir: dir.to_string_lossy().into_owned(),
    })
}

#[tauri::command]
pub fn get_app_version() -> String {
    update::current_version().to_string()
}

/// 检查 GitHub 最新 Release 是否有新版本
#[tauri::command]
pub fn check_update() -> Result<UpdateInfo, String> {
    update::check_update()
}

/// 下载并校验更新包，解压暂存 exe 与 README
#[tauri::command]
pub fn prepare_update(app: AppHandle, info: UpdateInfo) -> Result<PreparedUpdate, String> {
    let dir = config::config_dir(&app)?;
    update::prepare_update(&dir, &info)
}

/// 查询已暂存待安装的更新（用户之前选择了“稍后”）
#[tauri::command]
pub fn pending_update(app: AppHandle) -> Result<Option<PreparedUpdate>, String> {
    let dir = config::config_dir(&app)?;
    Ok(update::pending_update(&dir))
}

/// 立即重启：后台脚本等待本进程退出后替换 exe/README 并重新启动
#[tauri::command]
pub fn apply_update_and_restart(app: AppHandle) -> Result<(), String> {
    let dir = config::config_dir(&app)?;
    update::apply_update_and_restart(&dir)?;
    app.exit(0);
    Ok(())
}