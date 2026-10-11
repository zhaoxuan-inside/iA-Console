//! iac：iA-console 前端（CLI + TUI）。经 IPC 调用 daemon，不持业务状态。
//!
//! 入口为框架签名豁免（见 _00.conventions）；子命令随 T-022 落地。

fn main() {
    println!("iac 0.1.0 — iA-console 前端");
    println!("daemon 未接线：请运行 iacd 后使用（占位输出，子命令随 T-022 落地）");
}
