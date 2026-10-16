#![cfg(windows)]
use hmi_python::{PythonEnvironment, PythonError, PythonExecutor, PythonLimits};
use serde_json::json;
use std::{path::PathBuf, time::Duration};
use tokio_util::sync::CancellationToken;

fn executor() -> PythonExecutor {
    let root = std::env::var_os("HMI_TEST_PYTHON_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.cache/python/3.14.8-win_amd64")
        });
    PythonExecutor::new(
        PythonEnvironment::load(root).expect("Run scripts/prepare-python.ps1 first"),
    )
}

#[tokio::test]
async fn project_modules_support_imports_revisions_and_compile_only_validation() {
    let runner = executor();
    runner.validate_modules(json!([{"id":"library","name":"project_library","source":"raise RuntimeError('validation must not execute')"}])).await.unwrap();
    for value in [7, 9] {
        let modules = json!([
            {"id":"values","name":"project_values","source":format!("VALUE={value}")},
            {"id":"library","name":"project_library","source":"from project_values import VALUE\ncount=0\ndef read():\n global count\n count+=1\n return VALUE,count"}
        ]);
        let result=runner.execute_with_modules("from project_library import read\ndef transform(msg):\n msg['result']=list(read())\n return msg",json!({}),modules,PythonLimits::default(),CancellationToken::new()).await.unwrap();
        assert_eq!(result.message.unwrap()["result"], json!([value, 1]));
    }
    for (name, source) in [
        ("json", "VALUE=1"),
        ("bad-name", "VALUE=1"),
        ("application", "def broken(:"),
    ] {
        assert!(runner
            .validate_modules(json!([{"id":"invalid","name":name,"source":source}]))
            .await
            .is_err());
    }
}

async fn execute(source: &str) -> Result<hmi_python::PythonResult, PythonError> {
    executor()
        .execute(
            source,
            json!({"payload":{"rawTemperature":253}}),
            PythonLimits::default(),
            CancellationToken::new(),
        )
        .await
}

#[tokio::test]
async fn isolated_dictionary_none_and_native_stdout() {
    let result = execute("def transform(msg):\n import os, ctypes\n print('python-log')\n os.write(1,b'fd-log\\n')\n kernel=ctypes.WinDLL('kernel32')\n kernel.GetStdHandle.restype=ctypes.c_void_p\n kernel.WriteFile.argtypes=[ctypes.c_void_p,ctypes.c_void_p,ctypes.c_uint32,ctypes.c_void_p,ctypes.c_void_p]\n kernel.WriteFile(kernel.GetStdHandle(-11),b'native-log\\n',11,None,None)\n msg['payload']['temperature']=msg['payload']['rawTemperature']/10\n return msg").await.unwrap();
    assert_eq!(result.message.unwrap()["payload"]["temperature"], 25.3);
    for expected in ["python-log", "fd-log", "native-log"] {
        assert!(result.logs.contains(expected), "{}", result.logs);
    }
    assert!(execute("def transform(msg):\n return None")
        .await
        .unwrap()
        .message
        .is_none());
}

#[tokio::test]
async fn invalid_results_and_exceptions_are_visible() {
    for source in [
        "def transform(msg):\n raise ValueError('expected-error')",
        "def transform(msg):\n return {'x':float('nan')}",
        "def transform(msg):\n return {'x':float('inf')}",
        "def transform(msg):\n return {'x':object()}",
        "def transform(msg):\n return {1:'bad key'}",
        "def transform(msg):\n return []",
        "def transform(msg):\n yield msg",
        "async def helper():\n return {}\ndef transform(msg):\n return helper()",
    ] {
        assert!(
            matches!(execute(source).await, Err(PythonError::Code { .. })),
            "{source}"
        );
    }
}

#[tokio::test]
async fn validation_never_executes_top_level_code() {
    let runner = executor();
    runner
        .validate_source(
            "raise RuntimeError('must never execute')\ndef transform(msg):\n return msg",
        )
        .await
        .unwrap();
    for source in [
        "async def transform(msg):\n return msg",
        "def transform(value):\n return value",
        "def transform(msg)\n return msg",
        "return 1\ndef transform(msg):\n return msg",
    ] {
        assert!(runner.validate_source(source).await.is_err());
    }
    runner.validate_imports().await.unwrap();
}

#[tokio::test]
async fn every_call_has_fresh_module_state() {
    let runner = executor();
    let source = "import builtins\ndef transform(msg):\n n=getattr(builtins,'hmi_count',0)+1\n builtins.hmi_count=n\n return {'count':n}";
    for _ in 0..3 {
        let result = runner
            .execute(
                source,
                json!({}),
                PythonLimits::default(),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(result.message.unwrap()["count"], 1);
    }
}

#[tokio::test]
async fn timeout_cancel_and_memory_failure_leave_executor_usable() {
    let runner = executor();
    let limits = PythonLimits {
        timeout: Duration::from_millis(250),
        ..Default::default()
    };
    assert!(matches!(
        runner
            .execute(
                "def transform(msg):\n while True: pass",
                json!({}),
                limits,
                CancellationToken::new()
            )
            .await,
        Err(PythonError::Timeout)
    ));
    let token = CancellationToken::new();
    let cancellation = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        cancellation.cancel();
    });
    assert!(matches!(
        runner
            .execute(
                "def transform(msg):\n import time\n time.sleep(60)\n return msg",
                json!({}),
                PythonLimits::default(),
                token
            )
            .await,
        Err(PythonError::Cancelled)
    ));
    let memory = runner
        .execute(
            "def transform(msg):\n return {'size':len(bytearray(400*1024*1024))}",
            json!({}),
            PythonLimits::default(),
            CancellationToken::new(),
        )
        .await;
    assert!(memory.is_err());
    assert!(runner
        .execute(
            "def transform(msg):\n return msg",
            json!({}),
            PythonLimits::default(),
            CancellationToken::new()
        )
        .await
        .is_ok());
}

#[tokio::test]
async fn successful_invocation_reaps_child_processes() {
    let result = execute("def transform(msg):\n import subprocess,sys\n p=subprocess.Popen([sys.executable,'-I','-c','import time;time.sleep(60)'])\n return {'pid':p.pid}").await.unwrap();
    let pid = result.message.unwrap()["pid"].as_u64().unwrap() as u32;
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
    };
    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if !handle.is_null() {
            assert_eq!(
                WaitForSingleObject(handle, 1000),
                0,
                "child process still running"
            );
            CloseHandle(handle);
        }
    }
}

#[tokio::test]
async fn serial_queue_is_bounded_and_queued_cancellation_is_prompt() {
    let runner = executor();
    let token = CancellationToken::new();
    let mut tasks = Vec::new();
    for _ in 0..65 {
        let executor = runner.clone();
        let child_token = token.child_token();
        tasks.push(tokio::spawn(async move {
            executor
                .execute(
                    "def transform(msg):\n import time\n time.sleep(30)\n return msg",
                    json!({}),
                    PythonLimits {
                        timeout: Duration::from_secs(40),
                        ..Default::default()
                    },
                    child_token,
                )
                .await
        }));
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert!(matches!(
        runner
            .execute(
                "def transform(msg):\n return msg",
                json!({}),
                PythonLimits::default(),
                CancellationToken::new()
            )
            .await,
        Err(PythonError::QueueFull)
    ));
    token.cancel();
    for task in tasks {
        assert!(matches!(task.await.unwrap(), Err(PythonError::Cancelled)));
    }
    assert!(runner
        .execute(
            "def transform(msg):\n return msg",
            json!({}),
            PythonLimits::default(),
            CancellationToken::new()
        )
        .await
        .is_ok());
}

#[tokio::test]
#[ignore = "requires prepare-python-deps.py with numpy and requests; set HMI_TEST_PYTHON_DEPENDENCIES_ROOT"]
async fn native_and_transitive_dependencies_import_in_limited_embeddable() {
    let root = std::env::var_os("HMI_TEST_PYTHON_DEPENDENCIES_ROOT")
        .expect("Set HMI_TEST_PYTHON_DEPENDENCIES_ROOT");
    let environment = PythonEnvironment::load(PathBuf::from(root)).unwrap();
    assert!(environment.imports.contains(&"numpy".to_owned()));
    assert!(environment.imports.contains(&"requests".to_owned()));
    let runner = PythonExecutor::new(environment);
    let limits = PythonLimits {
        memory_bytes: 512 * 1024 * 1024,
        ..Default::default()
    };
    runner.validate_imports_with_limits(limits).await.unwrap();
    let result = runner.execute("def transform(msg):\n import numpy as np, requests\n return {'mean':float(np.mean([10,20,30])), 'requests':requests.__version__}", json!({}), limits, CancellationToken::new()).await.unwrap();
    assert_eq!(result.message.unwrap()["mean"], 20.0);
}

#[tokio::test]
async fn dropping_the_call_future_terminates_the_job_and_releases_queue() {
    let runner = executor();
    let pid_file = std::env::temp_dir().join(format!(
        "hmi-python-pid-{}-{}.txt",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let path_literal = serde_json::to_string(&pid_file.to_string_lossy()).unwrap();
    let source = format!("def transform(msg):\n import os,pathlib,time\n pathlib.Path({path_literal}).write_text(str(os.getpid()),encoding='ascii')\n time.sleep(60)\n return msg");
    let active = runner.clone();
    let task = tokio::spawn(async move {
        active
            .execute(
                &source,
                json!({}),
                PythonLimits {
                    timeout: Duration::from_secs(30),
                    ..Default::default()
                },
                CancellationToken::new(),
            )
            .await
    });
    let pid = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(text) = std::fs::read_to_string(&pid_file) {
                if let Ok(pid) = text.parse::<u32>() {
                    break pid;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("worker did not signal its PID");
    task.abort();
    let _ = task.await;
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
    };
    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if !handle.is_null() {
            assert_eq!(
                WaitForSingleObject(handle, 2000),
                0,
                "aborted call left worker alive"
            );
            CloseHandle(handle);
        }
    }
    std::fs::remove_file(pid_file).unwrap();
    assert!(runner
        .execute(
            "def transform(msg):\n return msg",
            json!({}),
            PythonLimits::default(),
            CancellationToken::new()
        )
        .await
        .is_ok());
}
