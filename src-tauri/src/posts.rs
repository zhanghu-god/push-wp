//! 远程文章管理：列表查询、详情查看、删除。
//!
//! 增/改由发布管线（publish.rs）完成；本模块补齐 CRUD 的查与删。
//! 列表用 WordPress 扩展接口 wp.getPosts（支持分页/搜索/状态筛选），
//! 详情用 metaWeblog.getPost（返回渲染后的 HTML 正文）。

use chrono::NaiveDateTime;
use serde::Serialize;
use tauri::AppHandle;

use crate::cache::{load_site_cache, save_site_cache};
use crate::config::Site;
use crate::xmlrpc::{MetaWeblog, Value};

/// 列表项摘要
#[derive(Debug, Clone, Serialize)]
pub struct PostSummary {
    pub post_id: String,
    pub title: String,
    pub status: String,
    pub slug: String,
    pub date: String,
    pub categories: Vec<String>,
    pub tags: Vec<String>,
}

/// 文章详情（含正文 HTML）
#[derive(Debug, Clone, Serialize)]
pub struct PostDetail {
    pub post_id: String,
    pub title: String,
    pub status: String,
    pub slug: String,
    pub date: String,
    pub categories: Vec<String>,
    pub tags: Vec<String>,
    pub excerpt: String,
    pub html: String,
}

/// 查询文章列表。status 传 any/publish/draft 等；search 为空则不过滤。
pub fn list_posts(
    site: &Site,
    count: u32,
    offset: u32,
    status: &str,
    search: &str,
) -> Result<Vec<PostSummary>, String> {
    let api = MetaWeblog::new(&site.url)?;
    let mut filter: Vec<(String, Value)> = vec![
        ("post_type".into(), Value::Str("post".into())),
        ("post_status".into(), Value::Str(status.into())),
        ("number".into(), Value::Int(count as i64)),
        ("offset".into(), Value::Int(offset as i64)),
    ];
    if !search.trim().is_empty() {
        filter.push(("s".into(), Value::Str(search.trim().into())));
    }
    let res = api.get_posts(&site.username, &site.password, &Value::Struct(filter))?;
    match res {
        Value::Array(items) => Ok(items.iter().map(summary_from_value).collect()),
        _ => Err("文章列表响应格式异常".into()),
    }
}

/// 获取文章详情
pub fn get_detail(site: &Site, post_id: &str) -> Result<PostDetail, String> {
    let api = MetaWeblog::new(&site.url)?;
    let v = api.get_post(post_id, &site.username, &site.password)?;
    let categories = match v.get("categories") {
        Some(Value::Array(a)) => a.iter().map(Value::as_string_lossy).collect(),
        _ => Vec::new(),
    };
    let tags = str_field(&v, "mt_keywords")
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    Ok(PostDetail {
        post_id: post_id.to_string(),
        title: str_field(&v, "title"),
        status: str_field(&v, "post_status"),
        slug: str_field(&v, "wp_slug"),
        date: display_date(&str_field(&v, "dateCreated")),
        categories,
        tags,
        excerpt: str_field(&v, "mt_excerpt"),
        html: str_field(&v, "description"),
    })
}

/// 站点已有的分类与标签（发布表单候选）
#[derive(Debug, Clone, Serialize)]
pub struct Terms {
    pub categories: Vec<String>,
    pub tags: Vec<String>,
}

/// 拉取站点全部已有分类与标签名（去重排序）。
/// 优先用 wp.getTerms（字段统一为 name），失败时回退到 wp.getCategories / wp.getTags。
pub fn get_terms(site: &Site) -> Result<Terms, String> {
    let api = MetaWeblog::new(&site.url)?;
    let categories = api
        .get_terms_of(&site.username, &site.password, "category", 500)
        .map(|v| names_of(&v))
        .or_else(|_| {
            api.get_categories(&site.username, &site.password)
                .map(|v| names_of(&v))
        })?;
    let tags = api
        .get_terms_of(&site.username, &site.password, "post_tag", 500)
        .map(|v| names_of(&v))
        .or_else(|_| {
            api.get_tags(&site.username, &site.password)
                .map(|v| names_of(&v))
        })?;
    Ok(Terms { categories, tags })
}

/// 兼容两种词条结构：wp.getTerms 用 name，旧版 getCategories 用 categoryName
fn names_of(v: &Value) -> Vec<String> {
    match v {
        Value::Array(items) => {
            let mut names: Vec<String> = items
                .iter()
                .map(|t| {
                    let n = str_field(t, "name");
                    if n.is_empty() {
                        str_field(t, "categoryName")
                    } else {
                        n
                    }
                })
                .filter(|n| !n.is_empty())
                .collect();
            names.sort();
            names.dedup();
            names
        }
        _ => Vec::new(),
    }
}

/// 删除远程文章，并清理本地指向该 ID 的文章缓存（图片缓存保留）。
pub fn delete_post(app: &AppHandle, site: &Site, post_id: &str) -> Result<(), String> {
    let api = MetaWeblog::new(&site.url)?;
    api.delete_post(&site.username, &site.password, post_id)?;

    let mut cache = load_site_cache(app, site)?;
    let before = cache.post.len();
    cache.post.retain(|_, p| p.post_id != post_id);
    if cache.post.len() != before {
        save_site_cache(app, site, &cache)?;
    }
    Ok(())
}

// ---------- 字段提取辅助 ----------

fn str_field(v: &Value, key: &str) -> String {
    v.get(key).map(Value::as_string_lossy).unwrap_or_default()
}

fn summary_from_value(v: &Value) -> PostSummary {
    let (categories, tags) = terms_of(v);
    PostSummary {
        post_id: str_field(v, "post_id"),
        title: str_field(v, "post_title"),
        status: str_field(v, "post_status"),
        slug: str_field(v, "post_name"),
        date: display_date(&str_field(v, "post_date")),
        categories,
        tags,
    }
}

/// wp.getPosts 的 terms 数组里混合了分类与标签，按 taxonomy 拆分
fn terms_of(v: &Value) -> (Vec<String>, Vec<String>) {
    let mut cats = Vec::new();
    let mut tags = Vec::new();
    if let Some(Value::Array(terms)) = v.get("terms") {
        for t in terms {
            let name = str_field(t, "name");
            if name.is_empty() {
                continue;
            }
            match str_field(t, "taxonomy").as_str() {
                "category" => cats.push(name),
                "post_tag" => tags.push(name),
                _ => {}
            }
        }
    }
    (cats, tags)
}

/// WordPress XML-RPC 日期可能是紧凑格式（20240102T03:04:05），统一转成可读格式
fn display_date(s: &str) -> String {
    for fmt in ["%Y%m%dT%H:%M:%S", "%Y-%m-%dT%H:%M:%S"] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
            return dt.format("%Y-%m-%d %H:%M").to_string();
        }
    }
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_post() -> Value {
        Value::Struct(vec![
            ("post_id".into(), Value::Str("42".into())),
            ("post_title".into(), Value::Str("测试文章".into())),
            ("post_status".into(), Value::Str("publish".into())),
            ("post_name".into(), Value::Str("ce-shi-wen-zhang".into())),
            ("post_date".into(), Value::DateTime("20240102T030405".into())),
            (
                "terms".into(),
                Value::Array(vec![
                    Value::Struct(vec![
                        ("taxonomy".into(), Value::Str("category".into())),
                        ("name".into(), Value::Str("分享".into())),
                    ]),
                    Value::Struct(vec![
                        ("taxonomy".into(), Value::Str("post_tag".into())),
                        ("name".into(), Value::Str("rust".into())),
                    ]),
                    Value::Struct(vec![
                        ("taxonomy".into(), Value::Str("nav_menu".into())),
                        ("name".into(), Value::Str("忽略".into())),
                    ]),
                ]),
            ),
        ])
    }

    #[test]
    fn test_summary_extraction() {
        let s = summary_from_value(&sample_post());
        assert_eq!(s.post_id, "42");
        assert_eq!(s.title, "测试文章");
        assert_eq!(s.status, "publish");
        assert_eq!(s.slug, "ce-shi-wen-zhang");
        assert_eq!(s.categories, vec!["分享"]);
        assert_eq!(s.tags, vec!["rust"]);
    }

    #[test]
    fn test_display_date() {
        assert_eq!(display_date("20240102T03:04:05"), "2024-01-02 03:04");
        assert_eq!(display_date("2024-01-02T03:04:05"), "2024-01-02 03:04");
        assert_eq!(display_date("未知格式"), "未知格式");
        assert_eq!(display_date(""), "");
    }

    #[test]
    fn test_missing_fields_default() {
        let s = summary_from_value(&Value::Struct(vec![]));
        assert_eq!(s.post_id, "");
        assert!(s.categories.is_empty());
    }

    #[test]
    fn test_terms_names() {
        let arr = Value::Array(vec![
            Value::Struct(vec![
                ("name".into(), Value::Str("技术".into())),
                ("slug".into(), Value::Str("tech".into())),
            ]),
            Value::Struct(vec![("name".into(), Value::Str("分享".into()))]),
            Value::Struct(vec![("name".into(), Value::Str("技术".into()))]),
            Value::Struct(vec![("name".into(), Value::Str(String::new()))]),
        ]);
        assert_eq!(names_of(&arr), vec!["分享", "技术"]);
        assert!(names_of(&Value::Str("x".into())).is_empty());
    }

    #[test]
    fn test_terms_names_legacy_category_name() {
        let arr = Value::Array(vec![Value::Struct(vec![
            ("categoryName".into(), Value::Str("agent".into())),
        ])]);
        assert_eq!(names_of(&arr), vec!["agent"]);
    }
}