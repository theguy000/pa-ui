use pa_client::{list_sessions, Client};

fn main() -> std::io::Result<()> {
    let client = Client::connect()?;
    println!(
        "connected: protocol {} / app {} / {} capabilities",
        client.hello.pointer("/protocol/version").map_or("?".into(), ToString::to_string),
        client.hello.get("appVersion").and_then(serde_json::Value::as_str).unwrap_or("?"),
        client
            .hello
            .get("serverCapabilities")
            .and_then(serde_json::Value::as_array)
            .map_or(0, Vec::len)
    );

    let rows = list_sessions(&client)?;
    println!("{:<28} {:<10} {:<14} {:<6} {:>6} {:>9} {:>7}", "session", "activity", "model", "think", "msgs", "tokens", "cost");
    for row in &rows {
        if row.lifecycle != "live" {
            continue;
        }
        println!(
            "{:<28} {:<10} {:<14} {:<6} {:>6} {:>9} {:>7}",
            truncate(&row.name, 28),
            row.activity,
            truncate(&row.model, 14),
            row.thinking,
            row.messages,
            row.input_tokens + row.output_tokens,
            format!("{:.2}", row.cost),
        );
    }
    let live = rows.iter().filter(|row| row.lifecycle == "live").count();
    println!("\n{live} live of {} known", rows.len());
    Ok(())
}

fn truncate(text: &str, width: usize) -> String {
    let cleaned: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.chars().count() <= width {
        return cleaned;
    }
    cleaned.chars().take(width.saturating_sub(1)).chain(['…']).collect()
}
