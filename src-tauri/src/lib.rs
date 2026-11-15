//! push-wp —— 将本地 Markdown 发布到 WordPress，Tauri 后端入口。

mod cache;
mod commands;
mod config;
mod posts;
mod publish;
mod render;
mod update;
mod xmlrpc;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = portable_kit::logging::init("push-wp");
    portable_kit::logging::install_panic_hook();
    portable_kit::logging::info(&format!(
        "startup version={} exe={:?}",
        env!("CARGO_PKG_VERSION"),
        std::env::current_exe().ok()
    ));
    let mut context = tauri::generate_context!();
    // 把 Tauri 的宿主目录统一重定向到 exe 同级 data\，彻底消除系统目录副作用；
    // WebView2 的 localData 指向 data/cache/webview2
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let data = dir.join("data");
            context.config_mut().app.app_directories_override =
                Some(tauri::utils::config::AppDirectoriesOverride::Directories(
                    tauri::utils::config::AppDirectoryOverrides {
                        config: Some(data.clone()),
                        data: Some(data.clone()),
                        local_data: Some(data.join("cache").join("webview2")),
                        cache: Some(data.join("caches")),
                        log: Some(data.join("logs")),
                    },
                ));
        }
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // 清理上次更新遗留的脚本目录（有 update.json 的暂存更新会保留）
            if let Ok(dir) = config::config_dir(app.handle()) {
                update::cleanup_leftover(&dir);
            }
            Ok(())
        })
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
            commands::get_app_info,
            commands::get_app_version,
            commands::check_update,
            commands::prepare_update,
            commands::pending_update,
            commands::apply_update_and_restart,
        ])
        .run(context)
        .expect("启动 Tauri 应用失败");
}
