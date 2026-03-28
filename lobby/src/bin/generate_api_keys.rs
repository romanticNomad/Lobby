use serde_json::Value;
use std::{
    error::Error,
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};
use uuid::Uuid;

// ============================================================

/// Binary file for generating API Keys for the custody **test accounts**
/// stored in `test_keys.json`
///
/// The generated api_keys are automatically written to the `.env` file
/// in the format
/// ```bash
/// export LOBBY_API_KEY_<N>="<api_token>:<client_id>:<from_address>""
///
/// ```
fn main() -> Result<(), Box<dyn Error>> {
    let input_path = Path::new("test_keys.json");
    let output_path = Path::new(".env");

    let content = fs::read_to_string(input_path)?;
    let parsed: Value = serde_json::from_str(&content)?;

    // extract object Map from the test_keys.json
    let object = parsed
        .as_object()
        .ok_or("test_keys.json must contain a top-level JSON object")?;

    let mut env_lines = Vec::new();
    let accounts: Vec<(&String, &Value)> = object.iter().collect();

    for (account_name, account_data) in accounts {
        let account_num = account_name
            .strip_prefix("account")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0);

        let from_address = account_data
            .get("address")
            .and_then(Value::as_str)
            .ok_or("missing 'address' field in account entry")?;

        let client_id = Uuid::new_v4();
        let token_suffix = Uuid::new_v4().simple().to_string();
        let api_token = format!("lobby_live_{}", &token_suffix[..9]);

        let env_var = format!(
            "export LOBBY_API_KEY_{}=\"{}:{}:{}\"",
            account_num, api_token, client_id, from_address
        );

        env_lines.push(env_var);
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(output_path)?;

    // Add a blank line to separate from existing content
    writeln!(file)?;
    for line in env_lines {
        writeln!(file, "{}", line)?;
    }

    println!("Generated API keys appended to {}", output_path.display());
    Ok(())
}

// ============================================================
