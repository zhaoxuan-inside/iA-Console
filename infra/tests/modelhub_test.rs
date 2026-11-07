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
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
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
    PluginRef {
        plugin_id: "builtin-test".into(),
        source: DownloadSource::Hf,
        base_url: Some(base),
        install_dir: Some(install_dir),
    }
}

#[test]
fn install_downloads_verifies_and_registers() {
    let bin = b"engine-bin".to_vec();
    let model = b"gguf-data!".to_vec();
    let manifest = MANIFEST
        .replace("__SHA_BIN__", &sha(&bin))
        .replace("__SHA_MODEL__", &sha(&model));
    let base = serve_source(
        manifest,
        vec![("llama-server", bin.clone()), ("model.gguf", model.clone())],
    );
    let dir = std::env::temp_dir().join(format!("iac-mhub-{}", std::process::id()));

    let store = Storage::open_in_memory().expect("db");
    let env = TestEnv::new().with_store(Box::new(store));
    let mut evo = Evolution::default();
    let mut m = Metrics::default();
    let mut mig = MigrationAck::default();
    assert!(env.deps.store.storage_migrate(
        &MigrationDir {},
        &mut mig,
        &env.context(),
        &mut m,
        &mut evo
    ));

    let mut out = infra::modelhub::InstallResult::default();
    assert!(
        modelhub_install(
            &plugin_ref(base.clone(), dir.clone()),
            &mut out,
            &env.context(),
            &mut m,
            &mut evo
        ),
        "evo={evo:?}"
    );
    assert_eq!(out.plugin_id, "builtin-test");
    assert_eq!(out.total_size, (bin.len() + model.len()) as u64);
    // 文件落盘
    assert!(dir.join("builtin-test/llama-server").exists());
    assert!(dir.join("builtin-test/model.gguf").exists());

    // 重复安装：sha 匹配跳过下载（仍成功）
    let mut out2 = infra::modelhub::InstallResult::default();
    assert!(modelhub_install(
        &plugin_ref(base, dir.clone()),
        &mut out2,
        &env.context(),
        &mut m,
        &mut evo
    ));

    // 列表有记录
    let mut rows = iac_core::ports::QueryRows::default();
    assert!(modelhub_list(
        &(),
        &mut rows,
        &env.context(),
        &mut m,
        &mut evo
    ));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows.text(0, "id"), Some("builtin-test"));
}

#[test]
fn tampered_sha_rejects_install() {
    let bin = b"engine-bin".to_vec();
    let model = b"gguf-data!".to_vec();
    let manifest = MANIFEST
        .replace("__SHA_BIN__", &"0".repeat(64))
        .replace("__SHA_MODEL__", &sha(&model));
    let base = serve_source(
        manifest,
        vec![("llama-server", bin.clone()), ("model.gguf", model.clone())],
    );
    let dir = std::env::temp_dir().join(format!("iac-mhub-bad-{}", std::process::id()));

    let store = Storage::open_in_memory().expect("db");
    let env = TestEnv::new().with_store(Box::new(store));
    let mut evo = Evolution::default();
    let mut m = Metrics::default();
    let mut mig = MigrationAck::default();
    assert!(env.deps.store.storage_migrate(
        &MigrationDir {},
        &mut mig,
        &env.context(),
        &mut m,
        &mut evo
    ));

    let mut out = infra::modelhub::InstallResult::default();
    assert!(!modelhub_install(
        &plugin_ref(base, dir),
        &mut out,
        &env.context(),
        &mut m,
        &mut evo
    ));
    assert_eq!(evo.reason_code.as_deref(), Some("IAC-1402"));
}

#[test]
fn local_engine_missing_plugin_guides_install() {
    let env = TestEnv::new();
    let spec = EngineSpec {
        plugin_id: "builtin-test".into(),
        ..Default::default()
    };
    let mut out = infra::ai::local::EngineHandle {
        base_url: String::new(),
        pid: 0,
        model: String::new(),
    };
    let mut evo = Evolution::default();
    assert!(!local_engine_ensure(
        &spec,
        &mut out,
        &env.context(),
        &mut Metrics::default(),
        &mut evo
    ));
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
        "#!/usr/bin/env python3\nimport sys, http.server\nport = int(sys.argv[sys.argv.index('--port') + 1])\nclass H(http.server.BaseHTTPRequestHandler):\n    def do_GET(self):\n        self.send_response(200)\n        self.send_header('Content-Length', '2')\n        self.end_headers()\n        self.wfile.write(b'ok')\n    def log_message(self, *a):\n        pass\nserver = http.server.HTTPServer(('127.0.0.1', port), H)\nimport threading\nthreading.Timer(3.0, server.shutdown).start()\nserver.serve_forever()\n",
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
        startup_timeout_secs: 0,
    };
    let env = TestEnv::new();
    let mut out = infra::ai::local::EngineHandle {
        base_url: String::new(),
        pid: 0,
        model: String::new(),
    };
    let mut evo = Evolution::default();
    assert!(
        local_engine_ensure(
            &spec,
            &mut out,
            &env.context(),
            &mut Metrics::default(),
            &mut evo
        ),
        "evo={evo:?}"
    );
    assert!(out.base_url.starts_with("http://127.0.0.1:"));
    assert!(out.model.ends_with("model.gguf"));
    assert!(out.pid > 0, "pid 应为真实子进程号");
    // 收尾：停掉本模型引擎，不留孤儿进程
    assert!(infra::ai::local::local_engine_stop(&dir.join("model.gguf")));
    assert!(infra::ai::local::registered_engines()
        .iter()
        .all(|(_, _, m)| !m.ends_with("model.gguf")));
}

/// 长驻假引擎（无自动退出），供复用/停止测试。
fn write_long_engine(dir: &std::path::Path) -> std::path::PathBuf {
    std::fs::create_dir_all(dir).expect("dir");
    let engine = dir.join("fake-engine.py");
    std::fs::write(
        &engine,
        "#!/usr/bin/env python3\nimport sys, http.server\nport = int(sys.argv[sys.argv.index('--port') + 1])\nclass H(http.server.BaseHTTPRequestHandler):\n    def do_GET(self):\n        self.send_response(200)\n        self.send_header('Content-Length', '2')\n        self.end_headers()\n        self.wfile.write(b'ok')\n    def log_message(self, *a):\n        pass\nserver = http.server.HTTPServer(('127.0.0.1', port), H)\nserver.serve_forever()\n",
    )
    .expect("engine script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&engine, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    engine
}

fn ensure_spec(dir: &std::path::Path) -> EngineSpec {
    EngineSpec {
        plugin_id: "builtin-test".into(),
        plugin_dir: Some(dir.to_path_buf()),
        engine_binary: Some("fake-engine.py".into()),
        model_file: Some("model.gguf".into()),
        context_len: 2048,
        preferred_port: None,
        startup_timeout_secs: 20,
    }
}

fn ensure_once(spec: &EngineSpec) -> (bool, infra::ai::local::EngineHandle, Evolution) {
    let env = TestEnv::new();
    let mut out = infra::ai::local::EngineHandle::default();
    let mut evo = Evolution::default();
    let ok = local_engine_ensure(
        spec,
        &mut out,
        &env.context(),
        &mut Metrics::default(),
        &mut evo,
    );
    (ok, out, evo)
}

#[test]
fn local_engine_reuses_healthy_instance_and_stop_works() {
    if std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("python3 不可用，跳过");
        return;
    }
    let dir = std::env::temp_dir().join(format!("iac-local-reuse-{}", std::process::id()));
    write_long_engine(&dir);
    std::fs::write(dir.join("model.gguf"), b"fake").expect("model");
    let spec = ensure_spec(&dir);

    let result = std::panic::catch_unwind(|| {
        let (ok1, h1, evo) = ensure_once(&spec);
        assert!(ok1, "首次拉起失败: {evo:?}");
        // 第二次 ensure：同模型健康实例应复用（同端点、同 pid，不新起进程）
        let (ok2, h2, evo2) = ensure_once(&spec);
        assert!(ok2, "复用失败: {evo2:?}");
        assert_eq!(h1.base_url, h2.base_url, "应复用同一实例");
        assert_eq!(h1.pid, h2.pid);
        let mine = infra::ai::local::registered_engines()
            .into_iter()
            .filter(|(_, _, m)| *m == h1.model)
            .collect::<Vec<_>>();
        assert_eq!(mine.len(), 1, "同模型全局唯一实例");
    });
    // 无论断言成败都停引擎，不留孤儿
    assert!(infra::ai::local::local_engine_stop(&dir.join("model.gguf")));
    let after = infra::ai::local::registered_engines()
        .into_iter()
        .filter(|(_, _, m)| m.ends_with("model.gguf") && m.contains("reuse"))
        .count();
    assert_eq!(after, 0, "stop 后注册表应清空");
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn local_engine_failure_leaks_nothing() {
    // 引擎秒退：每次端口尝试快速失败并清理，不空等预算、不残留子进程
    let dir = std::env::temp_dir().join(format!("iac-local-leak-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let engine = dir.join("fake-engine.sh");
    std::fs::write(&engine, "#!/bin/sh\nexit 1\n").expect("engine script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&engine, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    std::fs::write(dir.join("model.gguf"), b"fake").expect("model");
    let spec = EngineSpec {
        engine_binary: Some("fake-engine.sh".into()),
        startup_timeout_secs: 3,
        ..ensure_spec(&dir)
    };
    let (ok, _, evo) = ensure_once(&spec);
    assert!(!ok);
    assert_eq!(evo.reason_code.as_deref(), Some("IAC-1403"));
    assert!(
        infra::ai::local::registered_engines()
            .iter()
            .all(|(_, _, m)| !m.contains("leak")),
        "失败路径不得注册引擎"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// 支持 Range 的 mock 源：manifest 200；文件请求带 `Range: bytes=N-` 回 206 余量，否则 200 全量。
fn serve_range_source(manifest: String, files: Vec<(&'static str, Vec<u8>)>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut buf = vec![0u8; 8192];
            let n = stream.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).into_owned();
            let path = req.split(' ').nth(1).unwrap_or("/").to_string();
            let range = req
                .lines()
                .find(|l| l.to_ascii_lowercase().starts_with("range:"))
                .and_then(|l| l.split_once("bytes="))
                .and_then(|(_, v)| v.trim().trim_end_matches('-').trim().parse::<usize>().ok());
            let (status, body): (&str, Vec<u8>) = if path.ends_with("manifest.json") {
                ("200 OK", manifest.as_bytes().to_vec())
            } else {
                match files.iter().find(|(p, _)| path.ends_with(p)) {
                    Some((_, content)) => match range {
                        Some(start) if start < content.len() => {
                            ("206 Partial Content", content[start..].to_vec())
                        }
                        // 越界 Range：真实服务器按 RFC 7233 回 416（覆盖 .part 已完整的场景）
                        Some(_) => ("416 Range Not Satisfiable", Vec::new()),
                        None => ("200 OK", content.clone()),
                    },
                    None => ("404 Not Found", b"not found".to_vec()),
                }
            };
            let resp = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.write_all(&body);
        }
    });
    format!("http://{addr}")
}

fn migrated_env() -> TestEnv {
    let store = Storage::open_in_memory().expect("db");
    let env = TestEnv::new().with_store(Box::new(store));
    let mut evo = Evolution::default();
    let mut m = Metrics::default();
    let mut mig = MigrationAck::default();
    assert!(env.deps.store.storage_migrate(
        &MigrationDir {},
        &mut mig,
        &env.context(),
        &mut m,
        &mut evo
    ));
    env
}

#[test]
fn download_resumes_from_partial_file() {
    let content: Vec<u8> = (0..1000u32).map(|i| (i * 7 % 251) as u8).collect();
    let bin = b"engine-bin".to_vec();
    let manifest = MANIFEST
        .replace("__SHA_BIN__", &sha(&bin))
        .replace("__SHA_MODEL__", &sha(&content));
    let base = serve_range_source(
        manifest,
        vec![("llama-server", bin), ("model.gguf", content.clone())],
    );
    let dir = std::env::temp_dir().join(format!("iac-mhub-resume-{}", std::process::id()));
    let env = migrated_env();
    // 预置 400 字节部分文件：本次只应传输余量 600，落位后完整且 .part 消失
    let part = dir.join("builtin-test/model.gguf.part");
    std::fs::create_dir_all(part.parent().unwrap()).unwrap();
    std::fs::write(&part, &content[..400]).unwrap();

    let mut out = infra::modelhub::InstallResult::default();
    let mut evo = Evolution::default();
    let mut m = Metrics::default();
    assert!(
        modelhub_install(
            &plugin_ref(base, dir.clone()),
            &mut out,
            &env.context(),
            &mut m,
            &mut evo
        ),
        "evo={evo:?}"
    );
    assert_eq!(
        std::fs::read(dir.join("builtin-test/model.gguf")).unwrap(),
        content
    );
    assert!(!part.exists(), "落位后 .part 应消失");
    assert_eq!(
        out.total_size, 1010,
        "total_size 记落盘大小（bin 10B + model 1000B）而非本次传输量"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn corrupted_partial_file_is_cleaned_and_rejected() {
    let content: Vec<u8> = (0..500u32).map(|i| (i * 13 % 251) as u8).collect();
    let bin = b"engine-bin".to_vec();
    let manifest = MANIFEST
        .replace("__SHA_BIN__", &sha(&bin))
        .replace("__SHA_MODEL__", &sha(&content));
    let base = serve_range_source(
        manifest,
        vec![("llama-server", bin), ("model.gguf", content)],
    );
    let dir = std::env::temp_dir().join(format!("iac-mhub-corrupt-{}", std::process::id()));
    let env = migrated_env();
    // 损坏的部分文件：续传后整档哈希必不符 → 安装失败且 .part 被清理
    let part = dir.join("builtin-test/model.gguf.part");
    std::fs::create_dir_all(part.parent().unwrap()).unwrap();
    std::fs::write(&part, vec![0xFF; 400]).unwrap();

    let mut out = infra::modelhub::InstallResult::default();
    let mut evo = Evolution::default();
    let mut m = Metrics::default();
    assert!(!modelhub_install(
        &plugin_ref(base, dir.clone()),
        &mut out,
        &env.context(),
        &mut m,
        &mut evo
    ));
    assert_eq!(evo.reason_code.as_deref(), Some("IAC-1402"));
    assert!(!part.exists(), "不可信部分文件应被清理");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn malformed_manifest_rejected_and_traversal_blocked() {
    let model = b"gguf-data!".to_vec();
    // 1) 缺 sha256 条目：强校验报错（此前 filter_map 静默少装）
    let missing =
        r#"{"name":"x","engine":"e","model":"model.gguf","files":[{"path":"model.gguf"}]}"#;
    let base = serve_source(missing.into(), vec![]);
    let env = migrated_env();
    let mut out = infra::modelhub::InstallResult::default();
    let mut evo = Evolution::default();
    let mut m = Metrics::default();
    let dir = std::env::temp_dir().join(format!("iac-mhub-badmf-{}", std::process::id()));
    assert!(!modelhub_install(
        &plugin_ref(base, dir.clone()),
        &mut out,
        &env.context(),
        &mut m,
        &mut evo
    ));
    assert!(evo.reason.as_deref().unwrap().contains("sha256 缺失"));
    // 2) 路径穿越：拒绝安装（dest 会解析到 dir/escape.gguf，不得写出）
    let escaped = format!(
        r#"{{"name":"x","engine":"e","model":"model.gguf","files":[{{"path":"../escape.gguf","sha256":"{}","size":9}}]}}"#,
        sha(&model)
    );
    let base2 = serve_source(escaped, vec![]);
    let mut evo2 = Evolution::default();
    let mut out2 = infra::modelhub::InstallResult::default();
    assert!(!modelhub_install(
        &plugin_ref(base2, dir.clone()),
        &mut out2,
        &env.context(),
        &mut m,
        &mut evo2
    ));
    assert!(!dir.join("escape.gguf").exists(), "穿越文件不得写出");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_records_real_source() {
    let bin = b"engine-bin".to_vec();
    let model = b"gguf-data!".to_vec();
    let manifest = MANIFEST
        .replace("__SHA_BIN__", &sha(&bin))
        .replace("__SHA_MODEL__", &sha(&model));
    let base = serve_source(
        manifest,
        vec![("llama-server", bin.clone()), ("model.gguf", model.clone())],
    );
    let dir = std::env::temp_dir().join(format!("iac-mhub-src-{}", std::process::id()));
    let env = migrated_env();
    let mut out = infra::modelhub::InstallResult::default();
    let mut evo = Evolution::default();
    let mut m = Metrics::default();
    let mut pref = plugin_ref(base, dir.clone());
    pref.source = DownloadSource::Modelscope;
    assert!(modelhub_install(
        &pref,
        &mut out,
        &env.context(),
        &mut m,
        &mut evo
    ));
    let mut rows = iac_core::ports::QueryRows::default();
    assert!(infra::modelhub::modelhub_list(
        &(),
        &mut rows,
        &env.context(),
        &mut m,
        &mut evo
    ));
    assert_eq!(
        rows.text(0, "source"),
        Some("modelscope"),
        "source 应落真值"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn complete_part_finalizes_on_416_and_oversized_part_recovers() {
    let content: Vec<u8> = (0..1000u32).map(|i| (i * 7 % 251) as u8).collect();
    let bin = b"engine-bin".to_vec();
    let manifest = MANIFEST
        .replace("__SHA_BIN__", &sha(&bin))
        .replace("__SHA_MODEL__", &sha(&content));
    let base = serve_range_source(
        manifest,
        vec![("llama-server", bin), ("model.gguf", content.clone())],
    );
    let env = migrated_env();
    let mut evo = Evolution::default();
    let mut m = Metrics::default();

    // 场景 1：.part 已完整（中断在落位前）→ 服务器 416 → 整档校验通过即落位成功
    let dir1 = std::env::temp_dir().join(format!("iac-mhub-416a-{}", std::process::id()));
    let part1 = dir1.join("builtin-test/model.gguf.part");
    std::fs::create_dir_all(part1.parent().unwrap()).unwrap();
    std::fs::write(&part1, &content).unwrap();
    let mut out1 = infra::modelhub::InstallResult::default();
    assert!(
        modelhub_install(
            &plugin_ref(base.clone(), dir1.clone()),
            &mut out1,
            &env.context(),
            &mut m,
            &mut evo
        ),
        "evo={evo:?}"
    );
    assert_eq!(
        std::fs::read(dir1.join("builtin-test/model.gguf")).unwrap(),
        content
    );
    assert!(!part1.exists(), "落位后 .part 消失");
    let _ = std::fs::remove_dir_all(&dir1);

    // 场景 2：.part 超长且内容错误 → 416 → 校验不符 → 删除重下 → 成功
    let dir2 = std::env::temp_dir().join(format!("iac-mhub-416b-{}", std::process::id()));
    let part2 = dir2.join("builtin-test/model.gguf.part");
    std::fs::create_dir_all(part2.parent().unwrap()).unwrap();
    std::fs::write(&part2, vec![0xEE; 1500]).unwrap();
    let mut out2 = infra::modelhub::InstallResult::default();
    assert!(
        modelhub_install(
            &plugin_ref(base, dir2.clone()),
            &mut out2,
            &env.context(),
            &mut m,
            &mut evo
        ),
        "evo={evo:?}"
    );
    assert_eq!(
        std::fs::read(dir2.join("builtin-test/model.gguf")).unwrap(),
        content
    );
    assert!(!part2.exists());
    let _ = std::fs::remove_dir_all(&dir2);
}
