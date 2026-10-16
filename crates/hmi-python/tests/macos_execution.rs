#![cfg(target_os = "macos")]
use hmi_python::{PythonEnvironment, PythonError, PythonExecutor, PythonLimits};
use serde_json::json;
use std::{path::PathBuf, time::Duration};
use tokio_util::sync::CancellationToken;

fn executor() -> PythonExecutor {
    let root = std::env::var_os("HMI_TEST_PYTHON_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join(format!("../../.cache/python/{}", hmi_python::runtime_id()))
        });
    PythonExecutor::new(
        PythonEnvironment::load(root).expect("Run scripts/prepare-python-macos.py first"),
    )
}

#[tokio::test]
async fn bundled_python_executes_and_logs() {
    let result = executor().execute(
        "def transform(msg):\n import os\n print('mac-log')\n msg['answer']=os.uname().machine\n return msg",
        json!({}), PythonLimits::default(), CancellationToken::new(),
    ).await.unwrap();
    assert!(result.logs.contains("mac-log"));
    let expected = if std::env::consts::ARCH == "aarch64" {
        "arm64"
    } else {
        std::env::consts::ARCH
    };
    assert_eq!(result.message.unwrap()["answer"], expected);
}

#[tokio::test]
async fn timeout_cancellation_and_memory_limit_are_reported() {
    let runner = executor();
    let timeout = runner
        .execute(
            "def transform(msg):\n while True: pass",
            json!({}),
            PythonLimits {
                timeout: Duration::from_millis(400),
                ..Default::default()
            },
            CancellationToken::new(),
        )
        .await;
    assert!(matches!(timeout, Err(PythonError::Timeout)));
    let cancellation = CancellationToken::new();
    let cancel = cancellation.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancel.cancel();
    });
    let stopped = runner
        .execute(
            "def transform(msg):\n import time\n time.sleep(30)\n return msg",
            json!({}),
            PythonLimits::default(),
            cancellation,
        )
        .await;
    assert!(matches!(stopped, Err(PythonError::Cancelled)));
    let memory = runner.execute("def transform(msg):\n import time\n x=bytearray(128*1024*1024)\n time.sleep(1)\n return {'x':len(x)}", json!({}), PythonLimits { memory_bytes: 48 * 1024 * 1024, ..Default::default() }, CancellationToken::new()).await;
    assert!(
        matches!(memory, Err(PythonError::MemoryLimit)),
        "{memory:?}"
    );
}

#[tokio::test]
async fn successful_call_kills_background_descendants() {
    let runner = executor();
    let result = runner.execute("def transform(msg):\n import subprocess,sys\n p=subprocess.Popen([sys.executable,'-I','-c','import time;time.sleep(60)'])\n return {'pid':p.pid}", json!({}), PythonLimits::default(), CancellationToken::new()).await.unwrap();
    let pid = result.message.unwrap()["pid"].as_u64().unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let alive = std::process::Command::new("/bin/kill")
        .args(["-0", &pid.to_string()])
        .output()
        .map(|status| status.status.success())
        .unwrap_or(false);
    assert!(!alive, "Python descendant {pid} still running");
}

#[tokio::test]
#[ignore = "requires macOS dependency runtime prepared with requirements-macos-test.txt"]
async fn pure_python_and_native_dependency_import() {
    let root = std::env::var_os("HMI_TEST_PYTHON_DEPENDENCIES_ROOT")
        .expect("Set HMI_TEST_PYTHON_DEPENDENCIES_ROOT");
    let environment = PythonEnvironment::load(PathBuf::from(root)).unwrap();
    assert!(environment.imports.contains(&"numpy".to_owned()));
    assert!(environment.imports.contains(&"requests".to_owned()));
    let runner = PythonExecutor::new(environment);
    let limits = PythonLimits {
        timeout: Duration::from_secs(30),
        memory_bytes: 512 * 1024 * 1024,
        ..Default::default()
    };
    runner.validate_imports_with_limits(limits).await.unwrap();
    let result = runner.execute(
        "def transform(msg):\n import numpy as np, requests\n return {'mean':float(np.mean([10,20,30])), 'requests':requests.__version__}",
        json!({}), limits, CancellationToken::new(),
    ).await.unwrap();
    assert_eq!(result.message.unwrap()["mean"], 20.0);
}
