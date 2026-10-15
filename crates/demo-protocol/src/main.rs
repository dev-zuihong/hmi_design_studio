use demo_protocol::{ProtocolEngine, TemperatureDevice};
use hmi_core::{
    framing::{decode_demo_frame, LineDecoder},
    CoreResult,
};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_serial::SerialPortBuilderExt;

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{}: {}", error.code, error.message);
        std::process::exit(1);
    }
}
async fn run() -> CoreResult<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--help") {
        eprintln!("demo-protocol: JSON-RPC plugin on stdin/stdout\n  --device COM8 [--baud 115200]: reference device on a serial port\n  --device-stdio: reference wire device on stdin/stdout");
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("--device") {
        let port = args
            .get(1)
            .ok_or_else(|| hmi_core::CoreError::new("ARGUMENT", "--device requires a COM port"))?;
        let baud = args
            .iter()
            .position(|v| v == "--baud")
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(115200);
        let serial = tokio_serial::new(port, baud)
            .open_native_async()
            .map_err(|e| hmi_core::CoreError::new("SERIAL_OPEN", e.to_string()))?;
        let (reader, writer) = tokio::io::split(serial);
        return device(reader, writer).await;
    }
    if args.first().map(String::as_str) == Some("--device-stdio") {
        return device(tokio::io::stdin(), tokio::io::stdout()).await;
    }
    if !args.is_empty() {
        return Err(hmi_core::CoreError::new("ARGUMENT", "Unknown argument"));
    }
    let mut reader = tokio::io::stdin();
    let mut writer = tokio::io::stdout();
    let mut engine = ProtocolEngine::default();
    let mut decoder = LineDecoder::new(1024 * 1024);
    let mut bytes = [0u8; 4096];
    let mut timer = tokio::time::interval(Duration::from_millis(5));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let output = tokio::select! {
            result=reader.read(&mut bytes)=>{
                let count=result?;if count==0 {break;}
                let mut output=Vec::new();
                for line in decoder.push(&bytes[..count])? {output.extend(engine.receive(serde_json::from_slice(&line)?,Instant::now()));}
                output
            },
            _=timer.tick()=>engine.tick(Instant::now()),
        };
        for message in output {
            let mut bytes = serde_json::to_vec(&message)?;
            bytes.push(b'\n');
            writer.write_all(&bytes).await?;
        }
        writer.flush().await?;
    }
    Ok(())
}
async fn device<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    mut reader: R,
    mut writer: W,
) -> CoreResult<()> {
    let mut device = TemperatureDevice::default();
    let mut decoder = LineDecoder::new(64 * 1024);
    let mut bytes = [0u8; 4096];
    loop {
        let count = reader.read(&mut bytes).await?;
        if count == 0 {
            break;
        }
        for line in decoder.push(&bytes[..count])? {
            match decode_demo_frame(&line).and_then(|request| device.wire_response(&request)) {
                Ok(response) => {
                    writer.write_all(&response).await?;
                    writer.flush().await?;
                }
                Err(error) => eprintln!("Discarded invalid device request: {error}"),
            }
        }
    }
    Ok(())
}
