use serde_json::{Map, Value, json};
use std::{error::Error, fs, path::Path};
use uuid::Uuid;

// check Lobby_API_Doc.md, Section 6 for information about this binary.

fn main() -> Result<(), Box<dyn Error>> {
    let input_path = Path::new("test_keys.json");
    let output_dir = Path::new("api_keys");
    let output_path = output_dir.join("api_keys.json");

    let content = fs::read_to_string(input_path)?;
    let parsed: Value = serde_json::from_str(&content)?;
    let object = parsed
        .as_object()
        .ok_or("test_keys.json must contain a top-level JSON object")?;

    let mut generated = Vec::new();

    for (idx, (_account_name, account_data)) in object.iter().enumerate() {
        let from_address = account_data
            .get("address")
            .and_then(Value::as_str)
            .ok_or("missing 'address' field in account entry")?;

        let client_id = Uuid::new_v4();
        let token_suffix = Uuid::new_v4().simple().to_string();
        let api_token = format!("lobby_live_{}", &token_suffix[..9]);
        let env_var = format!("LOBBY_API_KEY_{}", idx + 1);
        let api_key_value = format!("{api_token}:{client_id}:{from_address}");

        generated.push(json!({
            "env_var": env_var,
            "api_token": api_token,
            "client_id": client_id.to_string(),
            "from_address": from_address,
            "api_key_value": api_key_value
        }));
    }

    fs::create_dir_all(output_dir)?;

    let mut root = Map::new();
    root.insert("source_file".to_string(), json!(input_path.display().to_string()));
    root.insert("count".to_string(), json!(generated.len()));
    root.insert("api_keys".to_string(), Value::Array(generated));

    let formatted = serde_json::to_string_pretty(&Value::Object(root))?;
    fs::write(&output_path, formatted)?;

    println!("Generated API keys written to {}", output_path.display());
    Ok(())
}
