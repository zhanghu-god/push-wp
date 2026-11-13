//! 最小 XML-RPC 客户端与 MetaWeblog API 封装。
//!
//! 对应原版 metaweblog-api 依赖：newPost / editPost / getPost / newMediaObject。
//! 请求体手工构造，响应用 quick-xml 递归解析。

use base64::Engine;
use quick_xml::events::Event;
use quick_xml::Reader;

/// XML-RPC 值类型
#[derive(Debug, Clone)]
pub enum Value {
    Str(String),
    Int(i64),
    Bool(bool),
    DateTime(String),
    Base64(Vec<u8>),
    Struct(Vec<(String, Value)>),
    Array(Vec<Value>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Struct(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// WordPress 的 id 有时返回 int 有时返回 string，统一转字符串
    pub fn as_string_lossy(&self) -> String {
        match self {
            Value::Str(s) => s.clone(),
            Value::Int(i) => i.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::DateTime(s) => s.clone(),
            _ => String::new(),
        }
    }
}

fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

fn write_value(out: &mut String, v: &Value) {
    out.push_str("<value>");
    match v {
        Value::Str(s) => {
            out.push_str("<string>");
            out.push_str(&escape_xml(s));
            out.push_str("</string>");
        }
        Value::Int(i) => out.push_str(&format!("<int>{i}</int>")),
        Value::Bool(b) => out.push_str(&format!("<boolean>{}</boolean>", i32::from(*b))),
        Value::DateTime(s) => out.push_str(&format!("<dateTime.iso8601>{s}</dateTime.iso8601>")),
        Value::Base64(bytes) => {
            out.push_str("<base64>");
            out.push_str(&base64::engine::general_purpose::STANDARD.encode(bytes));
            out.push_str("</base64>");
        }
        Value::Struct(fields) => {
            out.push_str("<struct>");
            for (k, val) in fields {
                out.push_str("<member><name>");
                out.push_str(&escape_xml(k));
                out.push_str("</name>");
                write_value(out, val);
                out.push_str("</member>");
            }
            out.push_str("</struct>");
        }
        Value::Array(items) => {
            out.push_str("<array><data>");
            for item in items {
                write_value(out, item);
            }
            out.push_str("</data></array>");
        }
    }
    out.push_str("</value>");
}

/// XML-RPC 客户端（阻塞式，命令在 Tauri 的独立线程池中执行）
pub struct Client {
    endpoint: String,
    http: reqwest::blocking::Client,
}

impl Client {
    pub fn new(endpoint: &str) -> Result<Self, String> {
        let http = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(180))
            .build()
            .map_err(|e| format!("创建 HTTP 客户端失败：{e}"))?;
        Ok(Self {
            endpoint: endpoint.to_string(),
            http,
        })
    }

    pub fn call(&self, method: &str, params: &[Value]) -> Result<Value, String> {
        let mut body = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?><methodCall><methodName>");
        body.push_str(&escape_xml(method));
        body.push_str("</methodName><params>");
        for p in params {
            body.push_str("<param>");
            write_value(&mut body, p);
            body.push_str("</param>");
        }
        body.push_str("</params></methodCall>");

        let resp = self
            .http
            .post(&self.endpoint)
            .header("Content-Type", "text/xml; charset=utf-8")
            .body(body)
            .send()
            .map_err(|e| format!("请求 {} 失败：{e}", self.endpoint))?;
        let status = resp.status();
        let text = resp
            .text()
            .map_err(|e| format!("读取响应失败：{e}"))?;
        if !status.is_success() {
            let brief: String = text.chars().take(300).collect();
            return Err(format!("HTTP {status}：{brief}"));
        }
        parse_response(&text)
    }
}

// ---------- 响应解析 ----------

fn parse_response(xml: &str) -> Result<Value, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) if e.name().as_ref() == b"fault" => {
                let v = parse_value(&mut reader)?;
                let code = v.get("faultCode").map(Value::as_string_lossy).unwrap_or_default();
                let msg = v
                    .get("faultString")
                    .map(Value::as_string_lossy)
                    .unwrap_or_else(|| "未知错误".into());
                return Err(format!("XML-RPC 错误 [{code}] {msg}"));
            }
            Ok(Event::Start(e)) if e.name().as_ref() == b"param" => {
                return parse_value(&mut reader);
            }
            Ok(Event::Eof) => return Err("XML-RPC 响应为空".into()),
            Ok(_) => {}
            Err(e) => return Err(format!("解析 XML-RPC 响应失败：{e}")),
        }
    }
}

/// 读取一个完整的 <value>...</value>
fn parse_value(reader: &mut Reader<&[u8]>) -> Result<Value, String> {
    loop {
        match reader.read_event() {
            Ok(Event::Empty(e)) if e.name().as_ref() == b"value" => {
                return Ok(Value::Str(String::new()));
            }
            Ok(Event::Start(e)) if e.name().as_ref() == b"value" => {
                return parse_value_inner(reader);
            }
            Ok(Event::Text(_)) => continue,
            Ok(Event::Eof) => return Err("响应中缺少 <value>".into()),
            Ok(other) => {
                return Err(format!("期望 <value>，实际遇到 {other:?}"));
            }
            Err(e) => return Err(format!("解析失败：{e}")),
        }
    }
}

/// <value> 的开始标签已消费，解析其内容直到 </value>
fn parse_value_inner(reader: &mut Reader<&[u8]>) -> Result<Value, String> {
    let v = match reader.read_event() {
        // 无类型标签的裸文本按 string 处理
        Ok(Event::Text(t)) => {
            let s = t.unescape().map_err(|e| e.to_string())?.into_owned();
            expect_end(reader, b"value")?;
            return Ok(Value::Str(s));
        }
        Ok(Event::End(e)) if e.name().as_ref() == b"value" => {
            return Ok(Value::Str(String::new()));
        }
        Ok(Event::Empty(e)) => match e.name().as_ref() {
            b"struct" => Value::Struct(vec![]),
            b"array" => Value::Array(vec![]),
            _ => Value::Str(String::new()),
        },
        Ok(Event::Start(e)) => match e.name().as_ref() {
            b"string" => Value::Str(read_text(reader, b"string")?),
            b"int" | b"i4" | b"i8" => {
                let s = read_text(reader, e.name().as_ref())?;
                Value::Int(s.trim().parse().unwrap_or(0))
            }
            b"boolean" => {
                let s = read_text(reader, b"boolean")?;
                Value::Bool(s.trim() == "1" || s.trim() == "true")
            }
            b"dateTime.iso8601" => Value::DateTime(read_text(reader, b"dateTime.iso8601")?),
            b"base64" => {
                let s = read_text(reader, b"base64")?;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(s.trim())
                    .map_err(|e| format!("base64 解码失败：{e}"))?;
                Value::Base64(bytes)
            }
            b"struct" => parse_struct(reader)?,
            b"array" => parse_array(reader)?,
            other => {
                return Err(format!(
                    "不支持的 XML-RPC 类型：{}",
                    String::from_utf8_lossy(other)
                ));
            }
        },
        Ok(other) => return Err(format!("<value> 内容异常：{other:?}")),
        Err(e) => return Err(format!("解析失败：{e}")),
    };
    expect_end(reader, b"value")?;
    Ok(v)
}

fn parse_struct(reader: &mut Reader<&[u8]>) -> Result<Value, String> {
    let mut fields = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) if e.name().as_ref() == b"member" => {
                let name = read_member_name(reader)?;
                let value = parse_value(reader)?;
                expect_end(reader, b"member")?;
                fields.push((name, value));
            }
            Ok(Event::End(e)) if e.name().as_ref() == b"struct" => {
                return Ok(Value::Struct(fields));
            }
            Ok(Event::Text(_)) => continue,
            Ok(Event::Eof) => return Err("<struct> 未闭合".into()),
            Ok(_) => continue,
            Err(e) => return Err(format!("解析失败：{e}")),
        }
    }
}

fn read_member_name(reader: &mut Reader<&[u8]>) -> Result<String, String> {
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) if e.name().as_ref() == b"name" => {
                return read_text(reader, b"name");
            }
            Ok(Event::Empty(e)) if e.name().as_ref() == b"name" => return Ok(String::new()),
            Ok(Event::Text(_)) => continue,
            Ok(Event::Eof) => return Err("<member> 缺少 <name>".into()),
            Ok(_) => continue,
            Err(e) => return Err(format!("解析失败：{e}")),
        }
    }
}

fn parse_array(reader: &mut Reader<&[u8]>) -> Result<Value, String> {
    let mut items = Vec::new();
    // 进入 <data>
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) if e.name().as_ref() == b"data" => break,
            Ok(Event::Empty(e)) if e.name().as_ref() == b"data" => {
                expect_end(reader, b"array")?;
                return Ok(Value::Array(items));
            }
            Ok(Event::Text(_)) => continue,
            Ok(Event::Eof) => return Err("<array> 缺少 <data>".into()),
            Ok(other) => return Err(format!("<array> 内容异常：{other:?}")),
            Err(e) => return Err(format!("解析失败：{e}")),
        }
    }
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) if e.name().as_ref() == b"value" => {
                items.push(parse_value_inner(reader)?);
            }
            Ok(Event::Empty(e)) if e.name().as_ref() == b"value" => {
                items.push(Value::Str(String::new()));
            }
            Ok(Event::End(e)) if e.name().as_ref() == b"data" => break,
            Ok(Event::Text(_)) => continue,
            Ok(Event::Eof) => return Err("<data> 未闭合".into()),
            Ok(_) => continue,
            Err(e) => return Err(format!("解析失败：{e}")),
        }
    }
    expect_end(reader, b"array")?;
    Ok(Value::Array(items))
}

/// 读取标签内的文本（含实体解码与 CDATA）
fn read_text(reader: &mut Reader<&[u8]>, tag: &[u8]) -> Result<String, String> {
    let mut out = String::new();
    loop {
        match reader.read_event() {
            Ok(Event::Text(t)) => out.push_str(&t.unescape().map_err(|e| e.to_string())?),
            Ok(Event::CData(t)) => out.push_str(&String::from_utf8_lossy(&t)),
            Ok(Event::End(e)) if e.name().as_ref() == tag => return Ok(out),
            Ok(Event::Eof) => {
                return Err(format!("<{}> 未闭合", String::from_utf8_lossy(tag)));
            }
            Ok(_) => continue,
            Err(e) => return Err(format!("解析失败：{e}")),
        }
    }
}

fn expect_end(reader: &mut Reader<&[u8]>, tag: &[u8]) -> Result<(), String> {
    loop {
        match reader.read_event() {
            Ok(Event::End(e)) if e.name().as_ref() == tag => return Ok(()),
            Ok(Event::Text(_)) => continue,
            Ok(Event::Eof) => {
                return Err(format!("缺少 </{}>", String::from_utf8_lossy(tag)));
            }
            Ok(_) => continue,
            Err(e) => return Err(format!("解析失败：{e}")),
        }
    }
}

// ---------- MetaWeblog 封装 ----------

pub struct MediaResult {
    pub id: String,
    pub url: String,
}

pub struct MetaWeblog {
    client: Client,
}

impl MetaWeblog {
    pub fn new(endpoint: &str) -> Result<Self, String> {
        Ok(Self {
            client: Client::new(endpoint)?,
        })
    }

    pub fn new_post(&self, username: &str, password: &str, post: &Value) -> Result<String, String> {
        let params = vec![
            Value::Str(String::new()),
            Value::Str(username.into()),
            Value::Str(password.into()),
            post.clone(),
            Value::Bool(true),
        ];
        let res = self.client.call("metaWeblog.newPost", &params)?;
        Ok(res.as_string_lossy())
    }

    pub fn edit_post(
        &self,
        post_id: &str,
        username: &str,
        password: &str,
        post: &Value,
    ) -> Result<(), String> {
        let params = vec![
            Value::Str(post_id.into()),
            Value::Str(username.into()),
            Value::Str(password.into()),
            post.clone(),
            Value::Bool(true),
        ];
        self.client.call("metaWeblog.editPost", &params)?;
        Ok(())
    }

    pub fn get_post(&self, post_id: &str, username: &str, password: &str) -> Result<Value, String> {
        let params = vec![
            Value::Str(post_id.into()),
            Value::Str(username.into()),
            Value::Str(password.into()),
        ];
        self.client.call("metaWeblog.getPost", &params)
    }

    /// WordPress 扩展接口：按条件查询文章列表（支持分页/搜索/状态筛选）
    pub fn get_posts(&self, username: &str, password: &str, filter: &Value) -> Result<Value, String> {
        let params = vec![
            Value::Str(String::new()),
            Value::Str(username.into()),
            Value::Str(password.into()),
            filter.clone(),
        ];
        self.client.call("wp.getPosts", &params)
    }

    /// WordPress 扩展接口：按分类法查询词条（category / post_tag），返回字段统一为 name
    pub fn get_terms_of(
        &self,
        username: &str,
        password: &str,
        taxonomy: &str,
        number: i64,
    ) -> Result<Value, String> {
        let filter = Value::Struct(vec![
            ("number".into(), Value::Int(number)),
            ("offset".into(), Value::Int(0)),
        ]);
        let params = vec![
            Value::Str(String::new()),
            Value::Str(username.into()),
            Value::Str(password.into()),
            Value::Str(taxonomy.into()),
            filter,
        ];
        self.client.call("wp.getTerms", &params)
    }

    /// WordPress 扩展接口：获取全部分类
    pub fn get_categories(&self, username: &str, password: &str) -> Result<Value, String> {
        let params = vec![
            Value::Str(String::new()),
            Value::Str(username.into()),
            Value::Str(password.into()),
        ];
        self.client.call("wp.getCategories", &params)
    }

    /// WordPress 扩展接口：获取全部标签
    pub fn get_tags(&self, username: &str, password: &str) -> Result<Value, String> {
        let params = vec![
            Value::Str(String::new()),
            Value::Str(username.into()),
            Value::Str(password.into()),
        ];
        self.client.call("wp.getTags", &params)
    }

    /// WordPress 扩展接口：删除文章（默认移入回收站）
    pub fn delete_post(&self, username: &str, password: &str, post_id: &str) -> Result<(), String> {
        let params = vec![
            Value::Str(String::new()),
            Value::Str(username.into()),
            Value::Str(password.into()),
            Value::Str(post_id.into()),
        ];
        self.client.call("wp.deletePost", &params)?;
        Ok(())
    }

    pub fn new_media_object(
        &self,
        username: &str,
        password: &str,
        name: &str,
        mime: &str,
        bits: Vec<u8>,
        overwrite: bool,
    ) -> Result<MediaResult, String> {
        let obj = Value::Struct(vec![
            ("name".into(), Value::Str(name.into())),
            ("type".into(), Value::Str(mime.into())),
            ("bits".into(), Value::Base64(bits)),
            ("overwrite".into(), Value::Bool(overwrite)),
        ]);
        let params = vec![
            Value::Str(String::new()),
            Value::Str(username.into()),
            Value::Str(password.into()),
            obj,
        ];
        let res = self.client.call("metaWeblog.newMediaObject", &params)?;
        let id = res
            .get("id")
            .map(Value::as_string_lossy)
            .filter(|s| !s.is_empty())
            .ok_or("上传响应缺少 id")?;
        let url = res
            .get("url")
            .map(Value::as_string_lossy)
            .filter(|s| !s.is_empty())
            .ok_or("上传响应缺少 url")?;
        Ok(MediaResult { id, url })
    }
}

/// 按扩展名推断图片 MIME（与原版 blog-api.ts 的映射一致）
pub fn image_mime(path: &str) -> Result<&'static str, String> {
    let ext = std::path::Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" | "jpe" => Ok("image/jpeg"),
        "gif" => Ok("image/gif"),
        "png" => Ok("image/png"),
        "bmp" => Ok("image/bmp"),
        "tif" | "tiff" => Ok("image/tiff"),
        "ico" => Ok("image/x-icon"),
        "webp" => Ok("image/webp"),
        _ => Err(format!("不支持的图片格式：.{ext}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_string_response() {
        let xml = r#"<?xml version="1.0"?>
<methodResponse><params><param><value><string>123</string></value></param></params></methodResponse>"#;
        let v = parse_response(xml).unwrap();
        assert_eq!(v.as_string_lossy(), "123");
    }

    #[test]
    fn test_parse_fault() {
        let xml = r#"<methodResponse><fault><value><struct>
<member><name>faultCode</name><value><int>403</int></value></member>
<member><name>faultString</name><value><string>Invalid username</string></value></member>
</struct></value></fault></methodResponse>"#;
        let err = parse_response(xml).unwrap_err();
        assert!(err.contains("403"), "err: {err}");
        assert!(err.contains("Invalid username"), "err: {err}");
    }

    #[test]
    fn test_parse_struct_with_array() {
        let xml = r#"<methodResponse><params><param><value><struct>
<member><name>id</name><value><int>42</int></value></member>
<member><name>url</name><value><string>https://x.com/a.png</string></value></member>
<member><name>cats</name><value><array><data>
<value><string>a</string></value><value><string>b</string></value>
</data></array></value></member>
</struct></value></param></params></methodResponse>"#;
        let v = parse_response(xml).unwrap();
        assert_eq!(v.get("id").unwrap().as_string_lossy(), "42");
        assert_eq!(v.get("url").unwrap().as_string_lossy(), "https://x.com/a.png");
        match v.get("cats").unwrap() {
            Value::Array(items) => assert_eq!(items.len(), 2),
            _ => panic!("cats 应为数组"),
        }
    }

    #[test]
    fn test_parse_bare_text_value() {
        let xml = "<methodResponse><params><param><value>hello &amp; world</value></param></params></methodResponse>";
        let v = parse_response(xml).unwrap();
        assert_eq!(v.as_string_lossy(), "hello & world");
    }

    #[test]
    fn test_parse_empty_value() {
        let xml = "<methodResponse><params><param><value/></param></params></methodResponse>";
        let v = parse_response(xml).unwrap();
        assert_eq!(v.as_string_lossy(), "");
    }

    #[test]
    fn test_write_value_roundtrip() {
        let post = Value::Struct(vec![
            ("title".into(), Value::Str("标题 <test> & \"引号\"".into())),
            ("categories".into(), Value::Array(vec![Value::Str("分享".into())])),
            ("dateCreated".into(), Value::DateTime("2024-01-02T03:04:05".into())),
            ("publish".into(), Value::Bool(true)),
        ]);
        let mut body = String::new();
        write_value(&mut body, &post);
        assert!(body.contains("&lt;test&gt;"));
        assert!(body.contains("&amp;"));
        assert!(body.contains("<dateTime.iso8601>2024-01-02T03:04:05</dateTime.iso8601>"));
        // 包进 methodResponse 再解析回来
        let xml = format!("<methodResponse><params><param>{body}</param></params></methodResponse>");
        let v = parse_response(&xml).unwrap();
        assert_eq!(v.get("title").unwrap().as_string_lossy(), "标题 <test> & \"引号\"");
    }

    #[test]
    fn test_image_mime() {
        assert_eq!(image_mime("a.PNG").unwrap(), "image/png");
        assert_eq!(image_mime("a.webp").unwrap(), "image/webp");
        assert!(image_mime("a.svg").is_err());
    }
}