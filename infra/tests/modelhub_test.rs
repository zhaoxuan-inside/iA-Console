//! T-026/T-027 测试：mock HTTP 清单+文件下载校验、sha 篡改拒绝、本地引擎 ensure 成功/未安装。

use iac_core::framework::{Evolution, Metrics, TestEnv};
use iac_core::ports::{MigrationAck, MigrationDir};
use infra::ai::local::{local_engine_ensure, EngineSpec};
use infra::modelhub::{modelhub_install, modelhub_list, DownloadSource, PluginRef};
use infra::storage::Storage;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;

/// mock 源：GET /<plugin>/manifest.json 与 /<plugin>/<file>。
fn serve_source(manifest: String, files: Vec<(&'static str, Vec<u8>)>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let req = String::from_utf8_lossy(&buf);
            let path = req.split(' ').nth(1).unwrap_or("/");
            let body = if path.ends_with("manifest.json") {
                manifest.as_bytes().to_vec()
            } else {
                files
                    .iter()
                    .find(|(p, _)| path.ends_with(p))
                    .map(|(_, b)| b.clone())
                    .unwrap_or_else(|| b"not found".to_vec())
            };
            let resp = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.write_all(&body);
        }
    });
    format!("http://{addr}")
}

fn sha(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

const MANIFEST: &str = r#"{"name":"测试插件","engine":"llama-server-test","model":"model.gguf","files":[
  {"path":"llama-server","sha256":"__SHA_BIN__","size":11},
  {"path":"model.gguf","sha256":"__SHA_MODEL__","size":9}
]}"#;

fn plugin_ref(base: String, install_dir: PathBuf) -> PluginRef {
    PluginRef { plugin_id: "builtin-test".into(), source: DownloadSource::Hf, base_url: Some(base), install_dir: Some(install_dir) }
}

#[test]
fn install_downloads_verifies_and_registers() {
    let bin = b"engine-bin".to_vec();
    let model = b"gguf-data!".to_vec();
    let manifest = MANIFEST.replace("__SHA_BIN__", &sha(&bin)).replace("__SHA_MODEL__", &sha(&model));
    let base = serve_source(manifest, vec![("llama-server", bin.clone()), ("model.gguf", model.clone())]);
    let dir = std::env::temp_dir().join(format!("iac-mhub-{}", std::process::id()));

    let store = Storage::open_in_memory().expect("db");
    let env = TestEnv::new().with_store(Box::new(store));
    let mut evo = Evolution::default();
    let mut m = Metrics::default();
    let mut mig = MigrationAck::default();
    assert!(env.deps.store.storage_migrate(&MigrationDir {}, &mut mig, &env.context(), &mut m, &mut evo));

    let mut out = infra::modelhub::InstallResult::default();
    assert!(modelhub_install(&plugin_ref(base.clone(), dir.clone()), &mut out, &env.context(), &mut m, &mut evo), "evo={evo:?}");
    assert_eq!(out.plugin_id, "builtin-test");
    assert_eq!(out.total_size, (bin.len() + model.len()) as u64);
    // 文件落盘
    assert!(dir.join("builtin-test/llama-server").exists());
    assert!(dir.join("builtin-test/model.gguf").exists());

    // 重复安装：sha 匹配跳过下载（仍成功）
    let mut out2 = infra::modelhub::InstallResult::default();
    assert!(modelhub_install(&plugin_ref(base, dir.clone()), &mut out2, &env.context(), &mut m, &mut evo));

    // 列表有记录
    let mut rows = iac_core::ports::QueryRows::default();
    assert!(modelhub_list(&(), &mut rows, &env.context(), &mut m, &mut evo));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows.text(0, "id"), Some("builtin-test"));
}

#[test]
fn tampered_sha_rejects_install() {
    let bin = b"engine-bin".to_vec();
    let model = b"gguf-data!".to_vec();
    let manifest = MANIFEST.replace("__SHA_BIN__", &"0".repeat(64)).replace("__SHA_MODEL__", &sha(&model));
    let base = serve_source(manifest, vec![("llama-server", bin.clone()), ("model.gguf", model.clone())]);
    let dir = std::env::temp_dir().join(format!("iac-mhub-bad-{}", std::process::id()));

    let store = Storage::open_in_memory().expect("db");
    let env = TestEnv::new().with_store(Box::new(store));
    let mut evo = Evolution::default();
    let mut m = Metrics::default();
    let mut mig = MigrationAck::default();
    assert!(env.deps.store.storage_migrate(&MigrationDir {}, &mut mig, &env.context(), &mut m, &mut evo));

    let mut out = infra::modelhub::InstallResult::default();
    assert!(!modelhub_install(&plugin_ref(base, dir), &mut out, &env.context(), &mut m, &mut evo));
    assert_eq!(evo.reason_code.as_deref(), Some("IAC-1402"));
}

#[test]
fn local_engine_missing_plugin_guides_install() {
    let env = TestEnv::new();
    let spec = EngineSpec { plugin_id: "builtin-test".into(), ..Default::default() };
    let mut out = infra::ai::local::EngineHandle { base_url: String::new(), pid: 0, model: String::new() };
    let mut evo = Evolution::default();
    assert!(!local_engine_ensure(&spec, &mut out, &env.context(), &mut Metrics::default(), &mut evo));
    assert_eq!(evo.reason_code.as_deref(), Some("IAC-1401"));
    assert!(evo.reason.as_deref().unwrap().contains("install"));
}

#[test]
fn local_engine_starts_and_reports_handle() {
    // 假引擎：python 单文件，按 --port 参数自起健康端点（模拟 llama-server /health）
    let dir = std::env::temp_dir().join(format!("iac-local-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let engine = dir.join("fake-engine.py");
    std::fs::write(
        &engine,
        "#!/usr/bin/env python3\nimport sys, http.server\nport = int(sys.argv[sys.argv.index('--port') + 1])\nclass H(http.server.BaseHTTPRequestHandler):\n    def do_GET(self):\n        self.send_response(200)\n        self.send_header('Content-Length', '2')\n        self.end_headers()\n        self.wfile.write(b'ok')\n    def log_message(self, *a):\n        pass\nhttp.server.HTTPServer(('127.0.0.1', port), H).serve_forever()\n",
    )
    .expect("engine script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&engine, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    std::fs::write(dir.join("model.gguf"), b"fake").expect("model");

    let spec = EngineSpec {
        plugin_id: "builtin-test".into(),
        plugin_dir: Some(dir.clone()),
        engine_binary: Some("fake-engine.py".into()),
        model_file: Some("model.gguf".into()),
        context_len: 2048,
        preferred_port: None,
    };
    let env = TestEnv::new();
    let mut out = infra::ai::local::EngineHandle { base_url: String::new(), pid: 0, model: String::new() };
    let mut evo = Evolution::default();
    assert!(local_engine_ensure(&spec, &mut out, &env.context(), &mut Metrics::default(), &mut evo), "evo={evo:?}");
    assert!(out.base_url.starts_with("http://127.0.0.1:"));
    assert!(out.model.ends_with("model.gguf"));
}
