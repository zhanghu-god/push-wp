//! 发布管线：解析 → 渲染 → 上传图片 → 发布/更新文章 → 写缓存。
//!
//! 对应原版 mdPublish/index.ts 的 MetaPublisher.publish。
//! MVP 简化：图片顺序上传（原版并发 5）、自动模式不做远程标题检索回退、
//! 不做远程图片有效性检查（等价原版 notCheck=true）。

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::cache::{
    entry_key, load_site_cache, post_cache_key, save_site_cache, ImageCache, PostCache, SiteCache,
};
use crate::config::Site;
use crate::render::{self, is_local_src, resolve_image_src};
use crate::xmlrpc::{image_mime, MetaWeblog, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PublishMode {
    /// 自动：本地缓存有文章 ID 则更新，否则创建新文章
    Auto,
    /// 手动：指定文章 ID 强制更新
    Manual,
    /// 始终创建新文章
    Create,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishRequest {
    pub site: Site,
    pub file_path: String,
    /// 编辑器中的当前内容（可能尚未保存到磁盘）
    pub content: String,
    pub mode: PublishMode,
    #[serde(default)]
    pub input_id: String,
    #[serde(default)]
    pub forced_update: bool,
    /// 发布对话框中填写的文章元数据
    pub meta: PublishMeta,
}

/// 发布表单提交的元数据（不再从 Markdown 头部解析）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishMeta {
    pub title: String,
    #[serde(rename = "abstract", default)]
    pub abstract_: Option<String>,
    /// 固定链接 slug；留空则交给 WordPress 根据标题生成
    #[serde(default)]
    pub url: Option<String>,
    /// datetime-local 表单值（YYYY-MM-DDTHH:MM[:SS]）
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// 特色图片：本地路径或 URL
    #[serde(default)]
    pub thumbnail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub state: String,
    pub message: String,
}

fn emit(app: &AppHandle, state: &str, message: impl Into<String>) {
    let _ = app.emit("publish-progress", Progress { state: state.into(), message: message.into() });
}

pub fn publish(app: &AppHandle, req: &PublishRequest) -> Result<String, String> {
    // 0. 校验表单
    let title = req.meta.title.trim();
    if title.is_empty() {
        return Err("标题不能为空".into());
    }

    // 1. 渲染正文（不解析头部元数据，整篇内容都是正文）
    emit(app, "read", "正在渲染正文…");
    let path = Path::new(&req.file_path);
    let base_dir = path.parent().unwrap_or(Path::new("."));
    let html = render::render_body(&req.content, base_dir);

    // 2. 确定目标文章 ID（填了固定链接用 slug 作缓存键，否则用文件路径）
    let mut site_cache = load_site_cache(app, &req.site)?;
    let slug_key = entry_key(&post_cache_key(req.meta.url.as_deref(), &req.file_path));
    let cached_id = site_cache.post.get(&slug_key).map(|p| p.post_id.clone());
    let post_id: Option<String> = match req.mode {
        PublishMode::Manual => {
            let id = req.input_id.trim().to_string();
            if id.parse::<i64>().map(|n| n > 0).unwrap_or(false) {
                Some(id)
            } else {
                cached_id.clone()
            }
        }
        PublishMode::Auto => cached_id.clone(),
        PublishMode::Create => None,
    };

    let api = MetaWeblog::new(&req.site.url)?;

    // 手动模式先验证文章 ID 有效（对应原版 getPost + 确认弹窗）
    if req.mode == PublishMode::Manual {
        if let Some(id) = &post_id {
            emit(app, "read", format!("正在验证文章 ID {id}…"));
            api.get_post(id, &req.site.username, &req.site.password)
                .map_err(|e| format!("文章 ID {id} 无效：{e}"))?;
        }
    }

    // 3. 上传图片
    emit(app, "upload", "正在处理图片…");
    let mut url_map: HashMap<String, String> = HashMap::new();

    // 特色图片（wp_thumbnail_id 只支持本地上传后的图片 ID，远程 URL 忽略）
    let mut thumbnail_id: Option<String> = None;
    if let Some(thumb_raw) = req.meta.thumbnail.as_deref().map(str::trim).filter(|s| !s.is_empty())
    {
        let thumb = resolve_image_src(base_dir, thumb_raw);
        if is_local_src(&thumb) {
            if let Some(ic) =
                upload_one(app, &api, &req.site, &mut site_cache, &thumb, req.forced_update)?
            {
                thumbnail_id = Some(ic.image_id);
            }
        } else {
            emit(app, "upload", "警告：特色图片必须是本地图片，远程 URL 已忽略");
        }
    }

    // 正文图片
    let local_imgs = render::collect_local_images(&html);
    for (i, src) in local_imgs.iter().enumerate() {
        let name = Path::new(src)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| src.clone());
        emit(app, "upload", format!("上传图片 ({}/{})：{name}", i + 1, local_imgs.len()));
        if let Some(ic) = upload_one(app, &api, &req.site, &mut site_cache, src, req.forced_update)?
        {
            url_map.insert(src.clone(), ic.image_url);
        }
    }

    let upload_html = render::replace_images_for_upload(&html, &|src| url_map.get(src).cloned());

    // 4. 摘要：表单优先，留空则从正文提取前 120 字
    let abstract_ = req
        .meta
        .abstract_
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or_else(|| render::extract_abstract(title, &upload_html, 120));

    // 5. 构造 MetaWeblog 文章结构
    let date = req.meta.date.as_deref().and_then(normalize_date).unwrap_or_else(now_local_iso);
    let mut fields: Vec<(String, Value)> = vec![
        ("title".into(), Value::Str(title.to_string())),
        ("description".into(), Value::Str(upload_html)),
        ("post_type".into(), Value::Str("post".into())),
        ("dateCreated".into(), Value::DateTime(date)),
        (
            "categories".into(),
            Value::Array(req.meta.categories.iter().cloned().map(Value::Str).collect()),
        ),
        ("mt_keywords".into(), Value::Str(req.meta.tags.join(","))),
        ("mt_excerpt".into(), Value::Str(abstract_)),
        ("post_status".into(), Value::Str("publish".into())),
    ];
    // 固定链接：留空时不发送 wp_slug，由 WordPress 根据标题自动生成
    if let Some(slug) = req.meta.url.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        fields.push(("wp_slug".into(), Value::Str(slug.to_string())));
    }
    if let Some(tid) = &thumbnail_id {
        fields.push(("wp_post_thumbnail".into(), Value::Str(tid.clone())));
    }
    let post = Value::Struct(fields);

    // 6. 发布或更新
    let res_id = if let Some(id) = &post_id {
        emit(app, "edit", format!("正在更新文章（ID {id}）…"));
        api.edit_post(id, &req.site.username, &req.site.password, &post)?;
        id.clone()
    } else {
        emit(app, "publish", "正在创建新文章…");
        api.new_post(&req.site.username, &req.site.password, &post)?
    };

    // 7. 写缓存
    site_cache.post.insert(slug_key, PostCache { post_id: res_id.clone(), thumbnail_id });
    save_site_cache(app, &req.site, &site_cache)?;

    emit(app, "complete", format!("发布完成，文章 ID：{res_id}"));
    Ok(res_id)
}

/// datetime-local 表单值 → XML-RPC iso8601；无法解析时返回 None（发布时回退当前时间）
fn normalize_date(input: &str) -> Option<String> {
    let s = input.trim();
    for fmt in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M"] {
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
            return Some(dt.format("%Y-%m-%dT%H:%M:%S").to_string());
        }
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Some(format!("{d}T00:00:00"));
    }
    None
}

fn now_local_iso() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

/// 上传单张本地图片：优先复用媒体缓存，forced_update 时强制覆盖上传。
/// 本地文件不存在时告警并跳过（返回 None）。
fn upload_one(
    app: &AppHandle,
    api: &MetaWeblog,
    site: &Site,
    cache: &mut SiteCache,
    local_path: &str,
    forced_update: bool,
) -> Result<Option<ImageCache>, String> {
    let path = Path::new(local_path);
    if !path.exists() {
        emit(app, "upload", format!("警告：本地图片不存在，跳过：{local_path}"));
        return Ok(None);
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "image".into());
    let key = entry_key(&name);

    if !forced_update {
        if let Some(ic) = cache.media.get(&key) {
            emit(app, "upload", format!("{name} 命中本地缓存，复用远程图片"));
            return Ok(Some(ic.clone()));
        }
    }

    let mime = image_mime(local_path)?;
    let bits = fs::read(path).map_err(|e| format!("读取图片 {name} 失败：{e}"))?;
    let res = api
        .new_media_object(&site.username, &site.password, &name, mime, bits, true)
        .map_err(|e| format!("图片 {name} 上传失败：{e}"))?;
    let ic = ImageCache { image_id: res.id, image_url: res.url };
    cache.media.insert(key, ic.clone());
    // 每张图片上传后立即落盘，中途失败也不丢缓存
    save_site_cache(app, site, cache)?;
    emit(app, "upload", format!("{name} 上传成功：{}", ic.image_url));
    Ok(Some(ic))
}
