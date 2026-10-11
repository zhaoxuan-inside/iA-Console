//! iA Console 独立 GUI 客户端（主形态，入口豁免）。
//! 布局：中央真终端（egui_term，真实 shell）+ 右侧 AI 侧栏（IPC → daemon）。
//! 设计语言：_04/03（14 令牌日夜主题、iA Writer 式无框淡化）。

#![warn(clippy::all, rust_2018_idioms)]

mod app;
mod ipc;
mod sidebar;
mod theme;

fn main() -> eframe::Result {
    // 版本查询/无图形会话保护：不进 GUI（安装脚本与 CI 冒烟依赖）
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("ia-console 0.1.0");
        return Ok(());
    }
    if std::env::var("DISPLAY").is_err() && std::env::var("WAYLAND_DISPLAY").is_err() {
        eprintln!("ia-console 需要图形会话（DISPLAY/WAYLAND_DISPLAY 未设置）");
        std::process::exit(1);
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([900.0, 600.0])
            .with_title("iA Console"),
        ..Default::default()
    };
    eframe::run_native("ia-console", options, Box::new(|cc| Ok(Box::new(app::ConsoleApp::new(cc)))))
}
