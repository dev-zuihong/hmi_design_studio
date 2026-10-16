// Standalone std-only JSON-RPC fixture, compiled once by session integration tests.
use std::{
    io::{self, BufRead, Write},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
fn send(output: &Mutex<io::Stdout>, text: &str) {
    let mut output = output.lock().unwrap();
    writeln!(output, "{text}").unwrap();
    output.flush().unwrap();
}
fn snapshot(generation: u128, sequence: u64, running: u8) -> String {
    format!(
        r#"{{"apiVersion":1,"generation":{generation},"sequence":{sequence},"stats":{{"running":{running}}},"devices":[],"byDevice":{{}}}}"#
    )
}
fn main() {
    let output = Arc::new(Mutex::new(io::stdout()));
    let generation = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_micros();
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        let id: String = line
            .split("\"id\":")
            .nth(1)
            .unwrap()
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if line.contains("\"method\":\"shutdown\"") {
            send(
                &output,
                &format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{}}}}"#),
            );
            break;
        }
        if line.contains("\"failStartup\":true") {
            send(
                &output,
                &format!(
                    r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":-32000,"message":"fixture startup failure"}}}}"#
                ),
            );
            continue;
        }
        if line.contains("\"operation\":\"busy\"") {
            send(
                &output,
                &format!(
                    r#"{{"jsonrpc":"2.0","method":"event","params":{{"kind":"snapshot","generation":{generation},"sequence":1,"data":{}}}}}"#,
                    snapshot(generation, 1, 1)
                ),
            );
            send(
                &output,
                &format!(
                    r#"{{"jsonrpc":"2.0","id":{id},"result":{{"accepted":true,"jobId":"job"}}}}"#
                ),
            );
            let output = output.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(250));
                send(
                    &output,
                    &format!(
                        r#"{{"jsonrpc":"2.0","method":"event","params":{{"kind":"snapshot","generation":{generation},"sequence":2,"data":{}}}}}"#,
                        snapshot(generation, 2, 0)
                    ),
                );
            });
        } else if line.contains("\"operation\":\"crash\"") {
            std::process::exit(2);
        } else {
            send(
                &output,
                &format!(
                    r#"{{"jsonrpc":"2.0","id":{id},"result":{}}}"#,
                    snapshot(generation, 0, 0)
                ),
            );
        }
    }
}
