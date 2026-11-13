//! Markdown 渲染与 HTML 后处理。
//!
//! 对应原版 markdown-it-init.ts / markdown-text-to-html.ts：
//! GFM 语法、标题锚点、本地图片路径重写、独立图片居中（argon 主题 fancybox）、
//! 正文 tab 转全角空格。MVP 暂不做 MathJax 与代码高亮。

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use html5ever::serialize::{serialize, SerializeOpts};
use html5ever::tendril::TendrilSink;
use html5ever::{local_name, ns, Attribute, LocalName, QualName};
use markup5ever_rcdom::{Handle, Node, NodeData, RcDom, SerializableHandle};

/// 图片路径解析：远程/data URL 原样返回，相对路径基于文章目录转绝对路径
pub fn resolve_image_src(base_dir: &Path, src: &str) -> String {
    let src = src.replace('\\', "/");
    if src.starts_with("http://")
        || src.starts_with("https://")
        || src.starts_with("data:")
        || src.starts_with("file://")
    {
        return src;
    }
    let p = Path::new(&src);
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        base_dir.join(p)
    };
    // HTML 中统一使用正斜杠，Windows 下同样可读
    abs.to_string_lossy().replace('\\', "/")
}

/// 判断 src 是否为本地文件（需要在发布时上传）
pub fn is_local_src(src: &str) -> bool {
    !(src.starts_with("http://")
        || src.starts_with("https://")
        || src.starts_with("data:")
        || src.starts_with("file://"))
}

/// Markdown 正文 → HTML（允许原始 HTML、硬换行，与原版 markdown-it 配置对齐）
pub fn markdown_to_html(body: &str) -> String {
    let mut options = comrak::Options::default();
    options.extension.strikethrough = true;
    options.extension.table = true;
    options.extension.autolink = true;
    options.extension.tasklist = true;
    options.extension.header_ids = Some(String::new());
    options.render.unsafe_ = true;
    options.render.hardbreaks = true;
    comrak::markdown_to_html(body, &options)
}

/// 渲染后的后处理：本地图片转绝对路径、独立图片居中、tab 转全角空格
pub fn process_html(html: &str, base_dir: &Path) -> String {
    let dom = parse_fragment(html);
    rewrite_images(&dom.document, base_dir);
    tabs_to_fullwidth(&dom.document, false, false);
    serialize_dom(&dom)
}

/// 完整管线：正文渲染 + 后处理
pub fn render_body(body: &str, base_dir: &Path) -> String {
    process_html(&markdown_to_html(body), base_dir)
}

/// 发布时把本地图片 src 替换为上传后的远程 URL。
/// resolver 输入本地绝对路径，返回远程 URL（None 表示保留原样）。
/// 独立图片会额外包一层 fancybox 锚点（argon 主题预览放大）。
pub fn replace_images_for_upload(html: &str, resolver: &dyn Fn(&str) -> Option<String>) -> String {
    let dom = parse_fragment(html);
    replace_and_wrap(&dom.document, resolver);
    serialize_dom(&dom)
}

/// 从 HTML 提取纯文本摘要（去掉开头标题，截取前 n 个字符）
pub fn extract_abstract(title: &str, html: &str, n: usize) -> String {
    let dom = parse_fragment(html);
    let mut text = String::new();
    collect_text(&dom.document, &mut text);
    let text = text.trim();
    let start = if text.starts_with(title) { title.len() } else { 0 };
    let rest = text[start..].trim();
    if rest.chars().count() > n {
        let truncated: String = rest.chars().take(n).collect();
        format!("{}...", truncated.trim_end())
    } else {
        rest.to_string()
    }
}

// ---------- DOM 辅助 ----------

fn parse_fragment(html: &str) -> RcDom {
    html5ever::parse_fragment(
        RcDom::default(),
        Default::default(),
        QualName::new(None, ns!(html), local_name!("div")),
        vec![],
        false,
    )
    .one(html)
}

fn serialize_dom(dom: &RcDom) -> String {
    let mut out = Vec::new();
    for child in dom.document.children.borrow().iter() {
        let handle = SerializableHandle::from(child.clone());
        serialize(&mut out, &handle, SerializeOpts::default()).expect("HTML 序列化失败");
    }
    String::from_utf8(out).expect("HTML 序列化结果非 UTF-8")
}

fn attr_value(handle: &Handle, name: &str) -> Option<String> {
    if let NodeData::Element { attrs, .. } = &handle.data {
        return attrs
            .borrow()
            .iter()
            .find(|a| a.name.local == *name)
            .map(|a| a.value.to_string());
    }
    None
}

fn set_attr(handle: &Handle, name: &str, value: &str) {
    if let NodeData::Element { attrs, .. } = &handle.data {
        let mut attrs = attrs.borrow_mut();
        if let Some(a) = attrs.iter_mut().find(|a| a.name.local == *name) {
            a.value = value.into();
        } else {
            attrs.push(Attribute {
                name: QualName::new(None, ns!(), LocalName::from(name)),
                value: value.into(),
            });
        }
    }
}

fn is_element(handle: &Handle, tag: &str) -> bool {
    matches!(&handle.data, NodeData::Element { name, .. } if name.local == *tag)
}

/// 读取父节点（rcdom 的 parent 是 Cell，take 后需还原）
fn parent_of(handle: &Handle) -> Option<Handle> {
    let weak = handle.parent.take();
    let parent = weak.as_ref().and_then(|w| w.upgrade());
    handle.parent.set(weak);
    parent
}

fn element_children(handle: &Handle) -> Vec<Handle> {
    handle
        .children
        .borrow()
        .iter()
        .filter(|c| matches!(c.data, NodeData::Element { .. }))
        .cloned()
        .collect()
}

fn has_nonempty_text(handle: &Handle) -> bool {
    handle.children.borrow().iter().any(|c| match &c.data {
        NodeData::Text { contents } => !contents.borrow().trim().is_empty(),
        _ => false,
    })
}

/// 本地图片 src → 绝对路径；独立成段的图片加上居中与 fancybox 属性
fn rewrite_images(root: &Handle, base_dir: &Path) {
    let mut imgs: Vec<Handle> = Vec::new();
    collect_imgs(root, &mut imgs);

    for img in imgs {
        let Some(src) = attr_value(&img, "src") else {
            continue;
        };
        if !is_local_src(&src) {
            continue;
        }
        let abs = resolve_image_src(base_dir, &src);
        set_attr(&img, "src", &abs);

        let parent = parent_of(&img);
        match parent {
            Some(p) if is_element(&p, "p") => {
                let elems = element_children(&p);
                if elems.len() == 1 && !has_nonempty_text(&p) {
                    decorate_standalone(&p);
                }
            }
            Some(_) => {
                // 原始 HTML 的裸 img：手动套一层居中的 p
                if let Some(p) = wrap_in_p(&img) {
                    decorate_standalone(&p);
                }
            }
            None => {}
        }
    }
}

fn collect_imgs(handle: &Handle, out: &mut Vec<Handle>) {
    if is_element(handle, "img") {
        out.push(handle.clone());
    }
    for child in handle.children.borrow().iter() {
        collect_imgs(child, out);
    }
}

/// 给独立图片段落加上 argon 主题 fancybox 需要的属性
fn decorate_standalone(p: &Handle) {
    set_attr(p, "align", "center");
    set_attr(p, "data-fancybox", "post-images");
    let class = attr_value(p, "class").unwrap_or_default();
    let mut classes: Vec<String> = class.split_whitespace().map(String::from).collect();
    for c in ["fancybox-wrapper", "lazyload-container-unload"] {
        if !classes.iter().any(|x| x == c) {
            classes.push(c.to_string());
        }
    }
    set_attr(p, "class", &classes.join(" "));
}

/// 把 img 包进新建的 p（替换 img 在父节点中的位置）
fn wrap_in_p(img: &Handle) -> Option<Handle> {
    let parent = parent_of(img)?;
    let p = Node::new(NodeData::Element {
        name: QualName::new(None, ns!(html), local_name!("p")),
        attrs: RefCell::new(vec![]),
        template_contents: RefCell::new(None),
        mathml_annotation_xml_integration_point: false,
    });
    {
        let mut siblings = parent.children.borrow_mut();
        let pos = siblings.iter().position(|h| Rc::ptr_eq(h, img))?;
        siblings[pos] = p.clone();
    }
    p.children.borrow_mut().push(img.clone());
    img.parent.replace(Some(Rc::downgrade(&p)));
    p.parent.replace(Some(Rc::downgrade(&parent)));
    Some(p)
}

/// 发布阶段：替换本地图片为远程 URL，并给独立图片包 fancybox 锚点
fn replace_and_wrap(handle: &Handle, resolver: &dyn Fn(&str) -> Option<String>) {
    if is_element(handle, "img") {
        if let Some(src) = attr_value(handle, "src") {
            if let Some(remote) = resolver(&src) {
                set_attr(handle, "src", &remote);
                // 独立图片（父 p 带 data-fancybox）包一层放大预览锚点
                if let Some(p) = parent_of(handle) {
                    if is_element(&p, "p") && attr_value(&p, "data-fancybox").is_some() {
                        wrap_in_anchor(handle, &remote);
                    }
                }
            }
        }
    }
    // 先克隆子节点列表：包裹锚点会修改 children，避免 RefCell 借用冲突
    let children: Vec<Handle> = handle.children.borrow().clone();
    for child in children {
        replace_and_wrap(&child, resolver);
    }
}

fn wrap_in_anchor(img: &Handle, href: &str) {
    let Some(parent) = parent_of(img) else {
        return;
    };
    let a = Node::new(NodeData::Element {
        name: QualName::new(None, ns!(html), local_name!("a")),
        attrs: RefCell::new(vec![
            Attribute {
                name: QualName::new(None, ns!(), local_name!("href")),
                value: href.into(),
            },
            Attribute {
                name: QualName::new(None, ns!(), LocalName::from("data-fancybox")),
                value: "post-images".into(),
            },
        ]),
        template_contents: RefCell::new(None),
        mathml_annotation_xml_integration_point: false,
    });
    {
        let mut siblings = parent.children.borrow_mut();
        if let Some(pos) = siblings.iter().position(|h| Rc::ptr_eq(h, img)) {
            siblings[pos] = a.clone();
        }
    }
    a.children.borrow_mut().push(img.clone());
    img.parent.replace(Some(Rc::downgrade(&a)));
    a.parent.replace(Some(Rc::downgrade(&parent)));
}

/// 正文段落与标题中的 tab → 全角空格（跳过 code/pre，对应原版 tab2Emsp）
fn tabs_to_fullwidth(handle: &Handle, in_code: bool, in_ph: bool) {
    let (next_code, next_ph) = match &handle.data {
        NodeData::Text { contents } => {
            if in_ph && !in_code {
                let mut t = contents.borrow_mut();
                if t.contains('\t') {
                    *t = t.replace('\t', "\u{3000}").into();
                }
            }
            return;
        }
        NodeData::Element { name, .. } => {
            let is_code = name.local == local_name!("code") || name.local == local_name!("pre");
            let is_ph = matches!(
                name.local.as_ref(),
                "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
            );
            (in_code || is_code, in_ph || is_ph)
        }
        _ => (in_code, in_ph),
    };
    for child in handle.children.borrow().iter() {
        tabs_to_fullwidth(child, next_code, next_ph);
    }
}

fn collect_text(handle: &Handle, out: &mut String) {
    match &handle.data {
        NodeData::Text { contents } => out.push_str(&contents.borrow()),
        NodeData::Element { name, .. } => {
            if name.local == local_name!("script") || name.local == local_name!("style") {
                return;
            }
        }
        _ => {}
    }
    for child in handle.children.borrow().iter() {
        collect_text(child, out);
    }
}

/// 收集 HTML 中所有本地图片的绝对路径（发布时上传用，去重）
pub fn collect_local_images(html: &str) -> Vec<String> {
    let dom = parse_fragment(html);
    let mut imgs: Vec<Handle> = Vec::new();
    collect_imgs(&dom.document, &mut imgs);
    let mut seen = HashMap::new();
    let mut result = Vec::new();
    for img in imgs {
        if let Some(src) = attr_value(&img, "src") {
            if is_local_src(&src) && seen.insert(src.clone(), ()).is_none() {
                result.push(src);
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_render() {
        let html = markdown_to_html("# 标题\n\n正文 **加粗**\n");
        assert!(html.contains("<h1"));
        assert!(html.contains("<strong>加粗</strong>"));
    }

    #[test]
    fn test_gfm_tasklist_table() {
        let html = markdown_to_html("- [x] 完成\n- [ ] 待办\n\n| a | b |\n|---|---|\n| 1 | 2 |\n");
        assert!(html.contains("type=\"checkbox\""));
        assert!(html.contains("<table>"));
    }

    #[test]
    fn test_image_rewrite_and_center() {
        let html = markdown_to_html("![图](img/a.png)\n");
        let out = process_html(&html, Path::new("C:/blog"));
        assert!(out.contains("C:/blog/img/a.png"), "out: {out}");
        assert!(out.contains("align=\"center\""), "out: {out}");
        assert!(out.contains("data-fancybox=\"post-images\""), "out: {out}");
    }

    #[test]
    fn test_remote_image_untouched() {
        let html = markdown_to_html("![图](https://x.com/a.png)\n");
        let out = process_html(&html, Path::new("C:/blog"));
        assert!(out.contains("https://x.com/a.png"));
        assert!(!out.contains("align=\"center\""));
    }

    #[test]
    fn test_inline_image_not_centered() {
        let html = markdown_to_html("文字 ![图](img/a.png) 更多文字\n");
        let out = process_html(&html, Path::new("C:/blog"));
        assert!(!out.contains("align=\"center\""), "out: {out}");
    }

    #[test]
    fn test_tabs_conversion() {
        let html = markdown_to_html("第一段\t缩进\n\n```\n代码\t保留\n```\n");
        let out = process_html(&html, Path::new("C:/blog"));
        assert!(out.contains("第一段\u{3000}缩进"), "out: {out}");
        assert!(out.contains("代码\t保留"), "out: {out}");
    }

    #[test]
    fn test_replace_for_upload() {
        let html = markdown_to_html("![图](img/a.png)\n");
        let processed = process_html(&html, Path::new("C:/blog"));
        let out = replace_images_for_upload(&processed, &|src| {
            if src.ends_with("a.png") {
                Some("https://cdn.x.com/a.png".to_string())
            } else {
                None
            }
        });
        assert!(out.contains("https://cdn.x.com/a.png"), "out: {out}");
        assert!(out.contains("<a href=\"https://cdn.x.com/a.png\""), "out: {out}");
        assert!(!out.contains("C:/blog"), "out: {out}");
    }

    #[test]
    fn test_collect_local_images() {
        let html = markdown_to_html("![一](img/a.png)\n![二](img/b.png)\n![三](img/a.png)\n![外](https://x.com/c.png)\n");
        let processed = process_html(&html, Path::new("C:/blog"));
        let imgs = collect_local_images(&processed);
        assert_eq!(imgs.len(), 2, "imgs: {imgs:?}");
    }

    #[test]
    fn test_resolve_image_src() {
        let base = Path::new("C:/blog");
        let resolved = resolve_image_src(base, "img/a.png");
        assert!(resolved.ends_with("img/a.png"));
        assert!(resolved.starts_with("C:/blog"));
        assert!(!resolved.contains('\\'));
        assert_eq!(resolve_image_src(base, "https://x.com/a.png"), "https://x.com/a.png");
        assert!(is_local_src("C:/blog/img/a.png"));
        assert!(!is_local_src("https://x.com/a.png"));
    }

    #[test]
    fn test_extract_abstract() {
        let html = "<h1>标题</h1><p>这是一段正文内容，用来提取摘要。</p>";
        let ab = extract_abstract("标题", html, 5);
        assert_eq!(ab, "这是一段正...");
    }
}