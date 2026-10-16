use crate::{PythonEnvironment, PythonError, PythonLimits, MAX_MESSAGE_BYTES};
use std::{
    ffi::c_void,
    io::{Read, Write},
    os::windows::{io::AsRawHandle, process::CommandExt},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
        },
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_JOB_MEMORY, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        },
        Threading::{
            OpenThread, ResumeThread, CREATE_NO_WINDOW, CREATE_SUSPENDED, THREAD_SUSPEND_RESUME,
        },
    },
};

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn os_error(context: &str) -> PythonError {
    PythonError::Process(format!("{context}: {}", std::io::Error::last_os_error()))
}

struct Job(Handle);
impl Job {
    fn new(memory: usize) -> Result<Self, PythonError> {
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(os_error("CreateJobObjectW"));
        }
        let job = Self(Handle(handle));
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags =
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_JOB_MEMORY;
        limits.JobMemoryLimit = memory;
        if unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const c_void,
                std::mem::size_of_val(&limits) as u32,
            )
        } == 0
        {
            return Err(os_error("SetInformationJobObject"));
        }
        Ok(job)
    }

    fn terminate(&self) {
        unsafe {
            TerminateJobObject((self.0).0, 1);
        }
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        self.terminate();
    }
}

// CREATE_SUSPENDED guarantees that the initial thread cannot run any user code
// until it has been assigned to the Job. std::process owns the process handle;
// ToolHelp obtains its initial thread for the documented ResumeThread operation.
fn resume_initial_thread(process_id: u32) -> Result<(), PythonError> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(os_error("CreateToolhelp32Snapshot"));
    }
    let snapshot = Handle(snapshot);
    let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of_val(&entry) as u32;
    let mut found = unsafe { Thread32First(snapshot.0, &mut entry) };
    while found != 0 {
        if entry.th32OwnerProcessID == process_id {
            let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
            if thread.is_null() {
                return Err(os_error("OpenThread"));
            }
            let thread = Handle(thread);
            if unsafe { ResumeThread(thread.0) } == u32::MAX {
                return Err(os_error("ResumeThread"));
            }
            return Ok(());
        }
        found = unsafe { Thread32Next(snapshot.0, &mut entry) };
    }
    Err(PythonError::Process(
        "Suspended Python initial thread was not found".into(),
    ))
}

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
    let job = Job::new(limits.memory_bytes)?;
    let mut child = Command::new(environment.root().join("python.exe"))
        .args(["-I", "-S", "-B", "-u", "-X", "utf8"])
        .arg(environment.root().join("executor/worker.py"))
        .current_dir(environment.root())
        .env_remove("PYTHONPATH")
        .env_remove("PYTHONHOME")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| PythonError::Process(format!("Spawn Python: {e}")))?;
    if unsafe { AssignProcessToJobObject((job.0).0, child.as_raw_handle() as HANDLE) } == 0 {
        let error = os_error("AssignProcessToJobObject");
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    if let Err(error) = resume_initial_thread(child.id()) {
        job.terminate();
        let _ = child.wait();
        return Err(error);
    }
    let mut stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let writer = thread::spawn(move || stdin.write_all(&request));
    let output_reader = thread::spawn(move || drain(stdout, 2 * MAX_MESSAGE_BYTES));
    let log_reader = thread::spawn(move || drain(stderr, MAX_MESSAGE_BYTES));
    let result = loop {
        if cancel.is_cancelled() {
            break Err(PythonError::Cancelled);
        }
        if started.elapsed() >= limits.timeout {
            break Err(PythonError::Timeout);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                break if status.success() {
                    Ok(())
                } else {
                    Err(PythonError::Process(format!(
                        "Worker exited with {status} (including possible Job memory exhaustion)"
                    )))
                }
            }
            Ok(None) => thread::sleep(Duration::from_millis(5)),
            Err(error) => break Err(PythonError::Process(error.to_string())),
        }
    };
    // Also kills descendants on successful completion so inherited pipe handles
    // and background tasks cannot outlive a single invocation.
    job.terminate();
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
        return Err(match error {
            PythonError::Process(message) => PythonError::Process(format!("{message}\n{logs}")),
            other => other,
        });
    }
    if oversized {
        return Err(PythonError::Process(
            "Python protocol response exceeds 2 MiB".into(),
        ));
    }
    Ok((output, logs))
}
