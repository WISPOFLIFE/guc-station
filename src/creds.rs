//! Remembered sign-in, kept in the OS credential store (Windows Credential
//! Manager, macOS Keychain, Secret Service). Nothing leaves the machine.

use keyring::Entry;

const SERVICE: &str = "GUC Station";
const ACCOUNT: &str = "guc-login";

pub fn save(login: &str, password: &str) -> Result<(), String> {
    let value = serde_json::json!({ "login": login, "password": password }).to_string();
    entry()?.set_password(&value).map_err(|e| e.to_string())
}

pub fn load() -> Option<(String, String)> {
    let raw = entry().ok()?.get_password().ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    Some((v["login"].as_str()?.to_string(), v["password"].as_str()?.to_string()))
}

pub fn forget() {
    if let Ok(e) = entry() {
        let _ = e.delete_credential();
    }
}

fn entry() -> Result<Entry, String> {
    Entry::new(SERVICE, ACCOUNT).map_err(|e| e.to_string())
}
