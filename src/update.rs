//! Updates from the GitHub Releases of the repository in Cargo.toml (`repository = "https://github.com/<owner>/<name>"`;
//! empty = no update checks). At most once a day, in the background, the app asks the public API for the latest
//! release. If it is newer, the app offers it. "Atualizar" downloads its `Gambito-Setup.exe`, checks it against the
//! release's `SHA256SUMS.txt` and starts it with `--update` (src/setup.rs): it waits for the app to close, puts the new
//! exe in place, keeps the engines and opens the app again.
//! Dev only: GAMBITO_UPDATE_API=<url> asks that URL instead of GitHub (same JSON as the API).
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const DAY: u64 = 24 * 3600;

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    pub setup_url: String,
    pub sums_url: String,
}

/// The API address of the latest release, if the app knows its repository.
fn api_url() -> Option<String> {
    if let Ok(u) = std::env::var("GAMBITO_UPDATE_API") { return Some(u); }
    let repo = env!("CARGO_PKG_REPOSITORY").trim_end_matches('/').strip_prefix("https://github.com/")?.to_string();
    (repo.split('/').count() == 2).then(|| format!("https://api.github.com/repos/{repo}/releases/latest"))
}

pub fn available() -> bool { api_url().is_some() }

/// "v1.2.10" > "1.2.9"; anything after a '-' (pre-releases) is ignored.
pub fn newer(theirs: &str, ours: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> { s.trim().trim_start_matches(['v', 'V']).split('-').next().unwrap_or("").split('.').map(|n| n.parse().unwrap_or(0)).collect() };
    let (a, b) = (parse(theirs), parse(ours));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y { return x > y; }
    }
    false
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(15))).timeout_recv_response(Some(Duration::from_secs(20))).build().into()
}

/// Reads the API answer: the version and the two files the update needs.
pub fn parse_release(json: &str) -> Option<Release> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    if v["draft"].as_bool() == Some(true) || v["prerelease"].as_bool() == Some(true) { return None; }
    let asset = |name: &str| v["assets"].as_array()?.iter().find(|a| a["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(name)))
        .and_then(|a| a["browser_download_url"].as_str().map(String::from));
    Some(Release { version: v["tag_name"].as_str()?.trim_start_matches(['v', 'V']).to_string(), setup_url: asset("Gambito-Setup.exe")?, sums_url: asset("SHA256SUMS.txt")? })
}

/// The latest release, if it is newer than this app.
pub fn check() -> Result<Option<Release>, String> {
    let Some(url) = api_url() else { return Ok(None) };
    let body = agent().get(&url).header("User-Agent", "Gambito-updater").header("Accept", "application/vnd.github+json")
        .call().map_err(|e| e.to_string())?.into_body().read_to_string().map_err(|e| e.to_string())?;
    Ok(parse_release(&body).filter(|r| newer(&r.version, VERSION)))
}

/// The SHA-256 the sums file gives for `file` ("<hex>  <name>" lines, as sha256sum writes them).
pub fn sum_for(sums: &str, file: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        let (h, n) = (it.next()?, it.next()?.trim_start_matches('*'));
        (n.eq_ignore_ascii_case(file) && h.len() == 64).then(|| h.to_lowercase())
    })
}

/// Downloads the new installer to %TEMP% and checks it. `progress(done, total)` is called as it goes.
pub fn download(r: &Release, progress: &dyn Fn(u64, u64)) -> Result<PathBuf, String> {
    let a = agent();
    let sums = a.get(&r.sums_url).header("User-Agent", "Gambito-updater").call().map_err(|e| e.to_string())?.into_body().read_to_string().map_err(|e| e.to_string())?;
    let want = sum_for(&sums, "Gambito-Setup.exe").ok_or("SHA256SUMS.txt")?;
    let resp = a.get(&r.setup_url).header("User-Agent", "Gambito-updater").call().map_err(|e| e.to_string())?;
    let total = resp.headers().get("content-length").and_then(|v| v.to_str().ok()?.parse().ok()).unwrap_or(0);
    let path = std::env::temp_dir().join(format!("Gambito-Setup-{}.exe", r.version)); // "Setup" in the name: it runs as the installer
    let mut f = std::fs::File::create(&path).map_err(|e| e.to_string())?;
    let (mut body, mut buf, mut done, mut h) = (resp.into_body().into_reader(), vec![0u8; 1 << 16], 0u64, Sha256::new());
    loop {
        let n = body.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 { break; }
        f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        h.update(&buf[..n]);
        done += n as u64;
        progress(done, total);
    }
    drop(f);
    let got: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if got != want {
        let _ = std::fs::remove_file(&path);
        return Err(crate::i18n::tl("O ficheiro descarregado não é o esperado (SHA-256 diferente).").into());
    }
    Ok(path)
}

/// Starts the downloaded installer in update mode for the folder this app runs from. The app must close right after.
pub fn launch(setup: &std::path::Path) -> Result<(), String> {
    let dir = std::env::current_exe().map_err(|e| e.to_string())?.parent().ok_or("dir")?.to_path_buf();
    std::process::Command::new(setup).args(["--silent", "--update", "--dir"]).arg(dir).spawn().map(|_| ()).map_err(|e| e.to_string())
}

// ---------- when to check, and the user's switch (%APPDATA%\Gambito\update.json) ----------

fn state_path() -> Option<PathBuf> { std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("Gambito").join("update.json")) }

fn state() -> serde_json::Value {
    state_path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(serde_json::json!({}))
}

fn save(v: &serde_json::Value) {
    if let Some(p) = state_path() {
        if let Some(d) = p.parent() { let _ = std::fs::create_dir_all(d); }
        let _ = std::fs::write(p, v.to_string());
    }
}

fn now() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) }

/// On unless the user turned it off.
pub fn auto() -> bool { state()["auto"].as_bool().unwrap_or(true) }

pub fn set_auto(on: bool) { let mut v = state(); v["auto"] = on.into(); save(&v); }

/// True if an automatic check is due now (and notes it, so the next one is a day later).
pub fn due() -> bool {
    if !available() || !auto() { return false; }
    let mut v = state();
    if now().saturating_sub(v["last"].as_u64().unwrap_or(0)) < DAY && std::env::var_os("GAMBITO_UPDATE_API").is_none() { return false; }
    v["last"] = now().into();
    save(&v);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert!(newer("v1.0.1", "1.0.0"));
        assert!(newer("1.10.0", "1.9.9"));
        assert!(!newer("v1.0.0", "1.0.0"));
        assert!(!newer("0.9.9", "1.0.0"));
        assert!(newer("2", "1.9"));
        assert!(!newer("1.0.0-beta", "1.0.0"));
    }

    #[test]
    fn reads_the_release_and_its_sums() {
        let json = r#"{"tag_name":"v1.2.0","draft":false,"prerelease":false,"assets":[
            {"name":"SHA256SUMS.txt","browser_download_url":"https://x/SHA256SUMS.txt"},
            {"name":"Gambito-Setup.exe","browser_download_url":"https://x/Gambito-Setup.exe"}]}"#;
        assert_eq!(parse_release(json), Some(Release { version: "1.2.0".into(), setup_url: "https://x/Gambito-Setup.exe".into(), sums_url: "https://x/SHA256SUMS.txt".into() }));
        assert_eq!(parse_release(&json.replace("\"prerelease\":false", "\"prerelease\":true")), None);
        assert_eq!(parse_release(&json.replace("SHA256SUMS.txt", "other.txt")), None); // no sums, no update
        let h = "11fe6239bfc9fcb4184af543db18345e91485038022250b53babfcceaa5326f9";
        assert_eq!(sum_for(&format!("{h}  Gambito-Setup.exe\n"), "Gambito-Setup.exe").as_deref(), Some(h));
        assert_eq!(sum_for(&format!("{h} *Gambito-Setup.exe"), "gambito-setup.exe").as_deref(), Some(h));
        assert_eq!(sum_for("abc  Gambito-Setup.exe", "Gambito-Setup.exe"), None);
    }
}
