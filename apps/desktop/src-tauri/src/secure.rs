//! Secrets stay in the operating system's key store (macOS Keychain, Windows Credential Manager,
//! the Secret Service on Linux), never in the app's files.
//!
//! The interface's web storage is routed here too: web SDKs such as MusicKit keep sign-in tokens in
//! localStorage, which would otherwise sit on disk in the app's data folder.

use tauri::Manager;

const SERVICE: &str = "Lute & Tavern";

/// Added to key store errors: on Linux the key store is a separate service that may not be running.
pub const KEYRING_HINT: &str = if cfg!(target_os = "linux") {
    " (On Linux this needs a key store service such as GNOME Keyring or KWallet to be running.)"
} else {
    ""
};
/// The only names the interface may read or write.
const ALLOWED: &[&str] = &["web-storage"];

fn entry(name: &str) -> Result<keyring::Entry, String> {
    if !ALLOWED.contains(&name) {
        return Err(format!("{name} is not a secure storage slot"));
    }
    keyring::Entry::new(SERVICE, name).map_err(|e| format!("The system key store is not available: {e}{}", crate::secure::KEYRING_HINT))
}

#[tauri::command]
pub fn secure_get(name: String) -> Result<Option<String>, String> {
    match entry(&name)?.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("Could not read from the key store: {e}")),
    }
}

#[tauri::command]
pub fn secure_set(name: String, value: String) -> Result<(), String> {
    entry(&name)?.set_password(&value).map_err(|e| format!("Could not save to the key store: {e}"))
}

#[tauri::command]
pub fn secure_delete(name: String) -> Result<(), String> {
    match entry(&name)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("Could not remove from the key store: {e}")),
    }
}

/// Wipes cookies and site data left in the app's web views (for example Apple ID sign-in cookies).
/// The app keeps nothing of its own there: settings live in the database, secrets in the key store.
#[tauri::command]
pub fn clear_web_data(app: tauri::AppHandle) -> Result<(), String> {
    for (_, w) in app.webview_windows() {
        w.clear_all_browsing_data().map_err(|e| e.to_string())?;
    }
    Ok(())
}
