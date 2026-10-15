use hmi_core::RuntimeEvent;
use serde_json::json;
use std::{
    io,
    path::{Path, PathBuf},
};
use tokio::{
    fs::{self, File, OpenOptions},
    io::AsyncWriteExt,
};

const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;
const MAX_EVENT_BYTES: usize = 1024 * 1024;

/// Three files total: runtime.jsonl, runtime.1.jsonl, runtime.2.jsonl.
pub(crate) struct RuntimeLog {
    path: PathBuf,
    file: Option<File>,
    bytes: u64,
    limit: u64,
}
impl RuntimeLog {
    pub async fn new(path: PathBuf) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await?;
        let bytes = file.metadata().await?.len();
        Ok(Self {
            path,
            file: Some(file),
            bytes,
            limit: MAX_FILE_BYTES,
        })
    }
    fn rotated(&self, index: usize) -> PathBuf {
        self.path.with_file_name(format!("runtime.{index}.jsonl"))
    }
    pub async fn write(&mut self, event: &RuntimeEvent) -> io::Result<()> {
        let bytes = encode_event(event)?;
        if self.bytes > 0 && self.bytes + bytes.len() as u64 > self.limit {
            self.rotate().await?;
        }
        if self.file.is_none() {
            self.file = Some(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.path)
                    .await?,
            );
        }
        self.file.as_mut().unwrap().write_all(&bytes).await?;
        self.bytes += bytes.len() as u64;
        Ok(())
    }
    async fn rotate(&mut self) -> io::Result<()> {
        if let Some(mut file) = self.file.take() {
            file.flush().await?;
        }
        remove_if_exists(&self.rotated(2)).await?;
        rename_if_exists(&self.rotated(1), &self.rotated(2)).await?;
        rename_if_exists(&self.path, &self.rotated(1)).await?;
        self.bytes = 0;
        Ok(())
    }
    pub async fn flush(&mut self) -> io::Result<()> {
        if let Some(file) = &mut self.file {
            file.flush().await?;
            file.sync_data().await?;
        }
        Ok(())
    }
}
async fn remove_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path).await {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}
async fn rename_if_exists(from: &Path, to: &Path) -> io::Result<()> {
    match fs::rename(from, to).await {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}
fn encode_event(event: &RuntimeEvent) -> io::Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(event)?;
    if bytes.len() + 1 > MAX_EVENT_BYTES {
        let mut truncated = event.clone();
        truncated.data = json!({"truncated":true,"originalBytes":bytes.len(),
            "preview":String::from_utf8_lossy(&bytes[..bytes.len().min(8192)])});
        bytes = serde_json::to_vec(&truncated)?;
    }
    bytes.push(b'\n');
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn rotates_during_one_session_and_keeps_only_three_complete_records() {
        let directory = tempfile::tempdir().unwrap();
        let mut log = RuntimeLog::new(directory.path().join("runtime.jsonl"))
            .await
            .unwrap();
        log.limit = 140;
        for n in 0..7 {
            log.write(&RuntimeEvent::new(
                "log",
                json!({"sequence":n,"message":"x".repeat(50)}),
            ))
            .await
            .unwrap();
        }
        log.flush().await.unwrap();
        let mut sequences = Vec::new();
        for name in ["runtime.2.jsonl", "runtime.1.jsonl", "runtime.jsonl"] {
            let bytes = fs::read(directory.path().join(name)).await.unwrap();
            assert!(bytes.len() <= 140);
            for line in String::from_utf8(bytes).unwrap().lines() {
                sequences.push(
                    serde_json::from_str::<serde_json::Value>(line).unwrap()["data"]["sequence"]
                        .as_u64()
                        .unwrap(),
                );
            }
        }
        assert_eq!(sequences, vec![4, 5, 6]);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 3);
    }
    #[test]
    fn oversized_event_retains_identity_and_does_not_mutate_live_data() {
        let mut event = RuntimeEvent::new(
            "node.completed",
            json!({"output":"温度".repeat(MAX_EVENT_BYTES)}),
        );
        event.run_id = Some("run".into());
        event.node_id = Some("node".into());
        let original = event.data.clone();
        let bytes = encode_event(&event).unwrap();
        assert!(bytes.len() <= MAX_EVENT_BYTES);
        let saved: RuntimeEvent = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(saved.run_id, event.run_id);
        assert_eq!(saved.node_id, event.node_id);
        assert_eq!(saved.data["truncated"], true);
        assert_eq!(event.data, original);
    }
}
