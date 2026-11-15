// 防止 Windows release 版弹出控制台窗口
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    push_wp_lib::run()
}
