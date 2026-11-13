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
        .run(tauri::generate_context!())
        .expect("启动 Tauri 应用失败");
}