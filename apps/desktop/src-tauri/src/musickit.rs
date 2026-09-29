//! Apple Music (MusicKit on the Web). The interface plays Apple Music through Apple's own MusicKit JS;
//! this side provides the developer token it needs.
//!
//! Released builds carry a token made at build time (LT_MUSICKIT_TOKEN). People building the app
//! themselves add their own MusicKit key in Settings: the key goes into the system keychain and the
//! app signs short-lived tokens from it. The key never leaves the keychain.

use crate::AppState;
use serde::Serialize;
use std::sync::Mutex;
use tauri::State;

const EMBEDDED: Option<&str> = option_env!("LT_MUSICKIT_TOKEN");
const KEYCHAIN_SERVICE: &str = "Lute & Tavern";
const KEYCHAIN_USER: &str = "musickit-private-key";
/// Tokens are made for 30 days and remade a day before they run out.
const TOKEN_DAYS: u64 = 30;

#[derive(Default)]
pub struct TokenCache(Mutex<Option<(String, u64)>>);

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn stored_key() -> Option<String> {
    keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_USER).ok()?.get_password().ok()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// A token is available (built in, or made from a saved key).
    ready: bool,
    /// The token came with this build.
    built_in: bool,
    key_id: String,
    team_id: String,
}

#[tauri::command]
pub fn musickit_status(state: State<'_, AppState>) -> Status {
    let key_id = state.lib.setting_str("musickitKeyId").unwrap_or_default();
    let team_id = state.lib.setting_str("musickitTeamId").unwrap_or_default();
    let built_in = EMBEDDED.is_some_and(|t| !t.is_empty());
    Status { ready: built_in || (stored_key().is_some() && !key_id.is_empty() && !team_id.is_empty()), built_in, key_id, team_id }
}

/// Saves a MusicKit key (the .p8 file from the Apple Developer site) in the keychain.
#[tauri::command]
pub async fn musickit_set_key(state: State<'_, AppState>, cache: State<'_, TokenCache>, path: String, key_id: String, team_id: String) -> Result<(), String> {
    let result = save_key(&state, &cache, &path, &key_id, &team_id);
    match &result {
        Ok(()) => crate::log::write("info", "MusicKit key saved in the keychain"),
        Err(e) => crate::log::write("error", &format!("MusicKit key not saved: {e}")),
    }
    result
}

fn save_key(state: &AppState, cache: &TokenCache, path: &str, key_id: &str, team_id: &str) -> Result<(), String> {
    let pem = std::fs::read_to_string(path).map_err(|e| format!("Could not read {path}: {e}"))?;
    if !pem.contains("PRIVATE KEY") {
        return Err("That file is not a MusicKit key. It should be the AuthKey_….p8 file from the Apple Developer site.".into());
    }
    // Apple names the file AuthKey_<KeyID>.p8, so an empty Key ID box is filled from the file name.
    let from_name = std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .and_then(|s| s.strip_prefix("AuthKey_"))
        .unwrap_or("")
        .to_string();
    let key_id = if key_id.trim().is_empty() { from_name } else { key_id.trim().trim_start_matches("AuthKey_").to_string() };
    let team_id = team_id.trim().to_string();
    if key_id.len() != 10 || team_id.len() != 10 {
        return Err("The Key ID and Team ID are both 10 characters, like ABC123DEFG.".into());
    }
    // Prove the key signs before saving anything.
    sign(&pem, &key_id, &team_id).map_err(|e| format!("The key could not sign a token: {e}"))?;
    keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_USER)
        .and_then(|e| e.set_password(&pem))
        .map_err(|e| format!("Could not save the key in the keychain: {e}"))?;
    for (k, v) in [("musickitKeyId", key_id), ("musickitTeamId", team_id)] {
        state.lib.set_setting(k, &serde_json::Value::String(v)).map_err(|e| e.to_string())?;
    }
    *cache.0.lock().unwrap_or_else(|e| e.into_inner()) = None;
    Ok(())
}

#[tauri::command]
pub fn musickit_forget_key(cache: State<'_, TokenCache>) -> Result<(), String> {
    if let Ok(e) = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_USER) {
        let _ = e.delete_credential();
    }
    *cache.0.lock().unwrap_or_else(|e| e.into_inner()) = None;
    Ok(())
}

/// The developer token MusicKit JS is configured with.
#[tauri::command]
pub fn musickit_token(state: State<'_, AppState>, cache: State<'_, TokenCache>) -> Result<String, String> {
    if let Some(t) = EMBEDDED.filter(|t| !t.is_empty()) {
        return Ok(t.to_string());
    }
    let mut c = cache.0.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((t, exp)) = c.as_ref()
        && *exp > now() + 86_400
    {
        return Ok(t.clone());
    }
    let key = stored_key().ok_or("Add your MusicKit key in Settings > Apple Music first.")?;
    let key_id = state.lib.setting_str("musickitKeyId").ok_or("The MusicKit Key ID is missing in Settings.")?;
    let team_id = state.lib.setting_str("musickitTeamId").ok_or("The Apple Team ID is missing in Settings.")?;
    let (token, exp) = sign(&key, &key_id, &team_id)?;
    *c = Some((token.clone(), exp));
    Ok(token)
}

fn sign(pem: &str, key_id: &str, team_id: &str) -> Result<(String, u64), String> {
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    let iat = now();
    let exp = iat + TOKEN_DAYS * 86_400;
    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some(key_id.to_string());
    let claims = serde_json::json!({ "iss": team_id, "iat": iat, "exp": exp });
    let key = EncodingKey::from_ec_pem(pem.as_bytes()).map_err(|e| e.to_string())?;
    Ok((encode(&header, &claims, &key).map_err(|e| e.to_string())?, exp))
}

#[cfg(test)]
mod tests {
    use super::sign;

    // A throwaway P-256 key made for this test only.
    const TEST_KEY: &str = "-----BEGIN PRIVATE KEY-----
(test key removed)
(test key removed)
(test key removed)
-----END PRIVATE KEY-----
";

    #[test]
    fn signs_es256_tokens_with_key_id() {
        let (token, exp) = sign(TEST_KEY, "ABC123DEFG", "TEAM123456").unwrap();
        let header = jsonwebtoken::decode_header(&token).unwrap();
        assert_eq!(header.kid.as_deref(), Some("ABC123DEFG"));
        assert_eq!(header.alg, jsonwebtoken::Algorithm::ES256);
        assert!(exp > 0);
    }
}
