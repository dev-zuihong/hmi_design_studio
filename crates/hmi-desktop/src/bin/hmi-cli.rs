#[tokio::main]
async fn main() {
    let args = std::env::args().skip(1).collect();
    match hmi_desktop::tooling::execute(args).await {
        Ok(result) => {
            println!("{}", serde_json::to_string_pretty(&result).unwrap());
            if result.get("valid") == Some(&serde_json::Value::Bool(false)) {
                std::process::exit(1);
            }
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
