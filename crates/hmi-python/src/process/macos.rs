use crate::{PythonEnvironment, PythonError, PythonLimits, MAX_MESSAGE_BYTES};
use std::{
    io::{Read, Write},
    os::unix::process::CommandExt,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

fn drain(mut stream: impl Read, maximum: usize) -> (Vec<u8>, bool) {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8192];
    let mut truncated = false;
    loop {
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(length) => {
                let available = maximum.saturating_sub(bytes.len()).min(length);
                bytes.extend_from_slice(&buffer[..available]);
                truncated |= available < length;
            }
        }
    }
    (bytes, truncated)
}

fn group_rss_kib(pgid: u32) -> Option<u64> {
    let output = Command::new("/bin/ps")
        .args(["-axo", "pgid=,rss="])
        .output()
        .ok()?;
    let text = std::str::from_utf8(&output.stdout).ok()?;
    Some(
        text.lines()
            .filter_map(|line| {
                let mut fields = line.split_whitespace();
                let group = fields.next()?.parse::<u32>().ok()?;
                let rss = fields.next()?.parse::<u64>().ok()?;
                (group == pgid).then_some(rss)
            })
            .sum(),
    )
}

fn kill_group(pgid: u32) {
    // The child is created as a new process group. This also closes inherited
    // pipes if user code spawned a background child before returning.
    unsafe {
        libc::kill(-(pgid as i32), libc::SIGKILL);
    }
}

pub(crate) fn run(
    environment: &PythonEnvironment,
    request: Vec<u8>,
    limits: PythonLimits,
    cancel: CancellationToken,
) -> Result<(Vec<u8>, String), PythonError> {
    let started = Instant::now();
    if cancel.is_cancelled() {
        return Err(PythonError::Cancelled);
    }
    let mut command = Command::new(environment.executable());
    command
        .args(["-I", "-S", "-B", "-u", "-X", "utf8"])
        .arg(environment.root().join("executor/worker.py"))
        .current_dir(environment.root())
        .env_remove("PYTHONPATH")
        .env_remove("PYTHONHOME")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }
    let mut child = command
        .spawn()
        .map_err(|e| PythonError::Process(format!("Spawn Python: {e}")))?;
    let pgid = child.id();
    let stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let writer = thread::spawn(move || {
        let mut stdin = stdin;
        stdin.write_all(&request)
    });
    let output_reader = thread::spawn(move || drain(stdout, 2 * MAX_MESSAGE_BYTES));
    let log_reader = thread::spawn(move || drain(stderr, MAX_MESSAGE_BYTES));
    let mut memory_exceeded = false;
    let result = loop {
        if cancel.is_cancelled() {
            break Err(PythonError::Cancelled);
        }
        if started.elapsed() >= limits.timeout {
            break Err(PythonError::Timeout);
        }
        match group_rss_kib(pgid) {
            Some(rss) if rss.saturating_mul(1024) > limits.memory_bytes as u64 => {
                memory_exceeded = true;
                break Err(PythonError::MemoryLimit);
            }
            None => {
                break Err(PythonError::Process(
                    "Could not sample Python process-group memory".into(),
                ))
            }
            _ => {}
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                break if status.success() {
                    Ok(())
                } else {
                    Err(PythonError::Process(format!("Worker exited with {status}")))
                }
            }
            Ok(None) => thread::sleep(Duration::from_millis(100)),
            Err(error) => break Err(PythonError::Process(error.to_string())),
        }
    };
    kill_group(pgid);
    let _ = child.kill();
    let _ = child.wait();
    let _ = writer.join();
    let (output, oversized) = output_reader
        .join()
        .map_err(|_| PythonError::Process("stdout reader panicked".into()))?;
    let (logs, truncated) = log_reader
        .join()
        .map_err(|_| PythonError::Process("stderr reader panicked".into()))?;
    let mut logs = String::from_utf8_lossy(&logs).into_owned();
    if truncated {
        logs.push_str("\n[Python log truncated after 1 MiB]");
    }
    if let Err(error) = result {
        return Err(if memory_exceeded {
            PythonError::MemoryLimit
        } else if let PythonError::Process(message) = error {
            PythonError::Process(format!("{message}\n{logs}"))
        } else {
            error
        });
    }
    if oversized {
        return Err(PythonError::Process(
            "Python protocol response exceeds 2 MiB".into(),
        ));
    }
    Ok((output, logs))
}
