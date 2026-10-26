//! Probe: sequential (single-threaded) read/write on the daemon pipe.
use serde_json::json;
use std::io::{BufRead, BufReader, Write};

fn main() -> std::io::Result<()> {
    let pipe = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(r"\\.\pipe\prime-agent-daemon")?;
    let mut writer = pipe.try_clone()?;
    let mut reader = BufReader::new(pipe);

    let mut hello = String::new();
    reader.read_line(&mut hello)?;
    println!("hello {} bytes", hello.len());

    let command = json!({
        "type": "command",
        "id": "probe-2",
        "protocol": {"name": "prime-agent.daemon", "version": 7},
        "command": {"type": "list", "id": "probe-2", "all": true},
    })
    .to_string() + "\n";
    println!("writing {} bytes", command.len());
    writer.write_all(command.as_bytes())?;
    writer.flush()?;
    println!("written; waiting for response");

    let mut response = String::new();
    reader.read_line(&mut response)?;
    println!("response {} bytes: {}", response.len(), response.chars().take(200).collect::<String>());
    Ok(())
}
