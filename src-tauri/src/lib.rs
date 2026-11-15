//! push-wp —— 将本地 Markdown 发布到 WordPress，Tauri 后端入口。

mod cache;
mod commands;
mod config;
mod posts;
mod publish;
mod render;
mod xmlrpc;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut context = tauri::generate_context!();
    // 把 Tauri 的宿主目录（含 WebView2 的 EBWebView 数据目录）
    // 统一重定向到 exe 同级 data\，彻底消除系统目录副作用
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            context.config_mut().app.app_directories_override =
                Some(tauri::utils::config::AppDirectoriesOverride::Root(
                    dir.join("data"),
                ));
        }
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::read_markdown,
            commands::render_markdown,
            commands::save_file,
            commands::get_settings,
            commands::save_settings,
            commands::get_cached_post,
            commands::publish_post,
            commands::list_posts,
            commands::get_post_detail,
            commands::delete_post,
            commands::get_terms,
        ])
        .run(context)
        .expect("启动 Tauri 应用失败");
}