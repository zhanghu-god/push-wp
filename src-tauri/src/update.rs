//! 自动更新薄封装：复用 portable-kit 的方案二更新逻辑（Web 重定向 + SHA256SUMS）。

use std::path::Path;

use portable_kit::update::{self as kit, UpdateConfig};
pub use portable_kit::update::{PreparedUpdate, UpdateInfo};

pub const CFG: UpdateConfig = UpdateConfig {
    repo: "zhanghu-god/push-wp",
    exe_name: "push-wp.exe",
    asset_base: "push-wp",
    user_agent: "push-wp-updater",
};

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub async fn check_update() -> Result<UpdateInfo, String> {
    let result = kit::check_update(&CFG, current_version()).await;
    match &result {
        Ok(info) => portable_kit::logging::info(&format!(
            "update check current={} latest={} has_update={}",
            info.current_version, info.latest_version, info.has_update
        )),
        Err(e) => portable_kit::logging::warn(&format!("update check failed: {e}")),
    }
    result
}

pub async fn prepare_update(data_dir: &Path, info: &UpdateInfo) -> Result<PreparedUpdate, String> {
    kit::prepare_update(&CFG, data_dir, info).await
}

pub fn apply_update_and_restart(data_dir: &Path) -> Result<(), String> {
    kit::apply_update_and_restart(&CFG, data_dir)
}

pub fn pending_update(data_dir: &Path) -> Option<PreparedUpdate> {
    kit::pending_update(data_dir)
}

pub fn cleanup_leftover(data_dir: &Path) {
    kit::cleanup_leftover(data_dir);
}
