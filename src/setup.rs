//! Gambito's own installer and uninstaller, drawn like the rest of the app.
//!
//! Gambito.exe is its own installer: named `Gambito-Setup*.exe` (or run with `--install`) it shows the setup
//! screen, copies itself into the chosen folder and downloads the chosen engines (engine::known) from their
//! official releases, each checked against its SHA-256 before it is put in place. `Gambito.exe --uninstall`
//! (what "Installed apps" runs) removes it. Per user: no administrator rights.
//! `--silent` does either without a window (every engine; `--dir <folder>`, `--desktop`, `--remove-data`).
//! The installer starts in English (unless Gambito already has a language saved) and saves the language picked.
//! Dev only: GAMBITO_SETUP_NAME=<name> registers under another name, so tests leave a real install alone;
//! `--auto` starts installing at once, GAMBITO_SETUP_PAGE=0..2 opens on that page and GAMBITO_SHOT=<png>
//! (+ GAMBITO_SHOT_AT=<seconds>) takes a picture.
use crate::engine::{self, Known};
use crate::*;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const NO_WINDOW: u32 = 0x0800_0000;
const ART_W: f32 = 340.0;

#[derive(Clone, Copy, PartialEq)]
pub enum Mode { Install, Uninstall }

/// What this run of the exe is for (None: the app itself).
pub fn mode() -> Option<Mode> {
    if has_arg("--uninstall") { return Some(Mode::Uninstall); }
    let stem = std::env::current_exe().ok().and_then(|e| e.file_stem().map(|s| s.to_string_lossy().to_lowercase()));
    (stem.is_some_and(|s| s.contains("setup")) || has_arg("--install")).then_some(Mode::Install)
}

fn has_arg(flag: &str) -> bool { std::env::args().any(|a| a == flag) }
fn arg_after(flag: &str) -> Option<String> { let mut a = std::env::args().skip_while(|x| x != flag); a.next()?; a.next() }

fn name() -> String { std::env::var("GAMBITO_SETUP_NAME").unwrap_or_else(|_| "Gambito".into()) }
fn uninstall_key() -> String { format!(r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\{}", name()) }

/// Where "Installed apps" says Gambito is. (reg.exe prints in the console code page: a folder with other
/// letters may not read back, and then it simply isn't found.)
fn registered_dir() -> Option<PathBuf> {
    let out = Command::new("reg").args(["query", &uninstall_key(), "/v", "InstallLocation"]).creation_flags(NO_WINDOW).output().ok()?;
    let dir = String::from_utf8_lossy(&out.stdout).lines().find_map(|l| l.split_once("REG_SZ").map(|(_, v)| PathBuf::from(v.trim())))?;
    dir.join("Gambito.exe").exists().then_some(dir)
}

fn install_dir() -> PathBuf {
    arg_after("--dir").map(PathBuf::from).or_else(registered_dir)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("Programs").join("Gambito"))
}

fn engine_file(dir: &Path, k: &Known) -> PathBuf { dir.join("engines").join(format!("{}.exe", k.file)) }

fn app_kb() -> u64 { std::env::current_exe().and_then(std::fs::metadata).map_or(0, |m| m.len() / 1024) }

fn hex(d: &[u8]) -> String { d.iter().map(|b| format!("{b:02x}")).collect() }

fn sha_file(p: &Path) -> Option<String> {
    let mut f = File::open(p).ok()?;
    let (mut h, mut buf) = (Sha256::new(), vec![0u8; 1 << 16]);
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 { break; }
        h.update(&buf[..n]);
    }
    Some(hex(&h.finalize()))
}

// ---------- the work (shared by the window and --silent) ----------

#[derive(Clone, PartialEq)]
enum Row { Off, Wait, Get(u64, u64), Check, Ok, Fail(String) }

/// Downloads one engine into `dir\engines` (nothing to do if the right file is already there).
fn fetch(k: &Known, dir: &Path, row: &Mutex<Row>, stop: &AtomicBool, wake: &dyn Fn()) -> Result<(), String> {
    let set = |r: Row| { *row.lock().unwrap() = r; wake(); };
    let out = engine_file(dir, k);
    set(Row::Check);
    if sha_file(&out).as_deref() == Some(k.sha256) { return Ok(()); }
    let zip = k.url.ends_with(".zip");
    let part = out.with_extension("part");
    let res = (|| {
        let agent: ureq::Agent = ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(20))).timeout_recv_response(Some(Duration::from_secs(30))).build().into();
        let resp = agent.get(k.url).call().map_err(|e| e.to_string())?;
        let total = resp.headers().get("content-length").and_then(|v| v.to_str().ok()?.parse().ok()).unwrap_or(k.kb * 1024);
        let mut body = resp.into_body().into_reader();
        let mut f = File::create(&part).map_err(|e| e.to_string())?;
        let (mut buf, mut done, mut h) = (vec![0u8; 1 << 16], 0u64, Sha256::new());
        loop {
            if stop.load(Ordering::Relaxed) { return Err(tl("Cancelado.").to_string()); }
            let n = body.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 { break; }
            f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            h.update(&buf[..n]);
            done += n as u64;
            set(Row::Get(done, total));
        }
        drop(f);
        set(Row::Check);
        let (file, sha) = if zip {
            let exe = out.with_extension("unzipped");
            unzip_exe(&part, &exe)?;
            (exe.clone(), sha_file(&exe).unwrap_or_default())
        } else { (part.clone(), hex(&h.finalize())) };
        if sha != k.sha256 {
            let _ = std::fs::remove_file(&file);
            return Err(tl("O ficheiro descarregado não é o esperado (SHA-256 diferente).").to_string());
        }
        let _ = std::fs::remove_file(&out);
        std::fs::rename(&file, &out).map_err(|e| e.to_string())
    })();
    let _ = std::fs::remove_file(&part);
    res
}

/// Takes the first .exe out of a zip (Stockfish's release is a zip). Stored or deflated entries, no zip64.
fn unzip_exe(zip: &Path, out: &Path) -> Result<(), String> {
    let bad = || "zip".to_string();
    let u16at = |b: &[u8], i: usize| u16::from_le_bytes([b[i], b[i + 1]]) as u64;
    let u32at = |b: &[u8], i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as u64;
    let mut f = File::open(zip).map_err(|e| e.to_string())?;
    let len = f.metadata().map_err(|e| e.to_string())?.len();
    let tail_len = len.min(66_000);
    let mut tail = vec![0u8; tail_len as usize];
    f.seek(SeekFrom::Start(len - tail_len)).and_then(|_| f.read_exact(&mut tail)).map_err(|e| e.to_string())?;
    let e = (0..tail.len().saturating_sub(21)).rev().find(|&i| tail[i..i + 4] == [0x50, 0x4b, 5, 6]).ok_or_else(bad)?;
    let (count, cd_size, cd_off) = (u16at(&tail, e + 10), u32at(&tail, e + 12), u32at(&tail, e + 16));
    let mut cd = vec![0u8; cd_size as usize];
    f.seek(SeekFrom::Start(cd_off)).and_then(|_| f.read_exact(&mut cd)).map_err(|e| e.to_string())?;
    let mut i = 0usize;
    for _ in 0..count {
        if cd.len() < i + 46 || cd[i..i + 4] != [0x50, 0x4b, 1, 2] { break; }
        let (method, csize, nlen, xlen, clen, loff) = (u16at(&cd, i + 10), u32at(&cd, i + 20), u16at(&cd, i + 28) as usize, u16at(&cd, i + 30) as usize, u16at(&cd, i + 32) as usize, u32at(&cd, i + 42));
        let entry = String::from_utf8_lossy(cd.get(i + 46..i + 46 + nlen).ok_or_else(bad)?).to_lowercase();
        if entry.ends_with(".exe") {
            let mut h = [0u8; 30];
            f.seek(SeekFrom::Start(loff)).and_then(|_| f.read_exact(&mut h)).map_err(|e| e.to_string())?;
            f.seek(SeekFrom::Start(loff + 30 + u16at(&h, 26) + u16at(&h, 28))).map_err(|e| e.to_string())?;
            let mut src = (&mut f).take(csize);
            let mut o = File::create(out).map_err(|e| e.to_string())?;
            let copied = match method {
                0 => std::io::copy(&mut src, &mut o),
                8 => std::io::copy(&mut flate2::read::DeflateDecoder::new(src), &mut o),
                _ => return Err(bad()),
            };
            return copied.map(|_| ()).map_err(|e| e.to_string());
        }
        i += 46 + nlen + xlen + clen;
    }
    Err(bad())
}

/// Runs a PowerShell script (written to a file with a BOM, so any folder name survives).
fn powershell(script: &str) -> Result<(), String> {
    let path = std::env::temp_dir().join(format!("gambito-setup-{}.ps1", std::process::id()));
    std::fs::write(&path, format!("\u{feff}$ErrorActionPreference = 'Stop'\n{script}")).map_err(|e| e.to_string())?;
    let out = Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"]).arg(&path).creation_flags(NO_WINDOW).output();
    let _ = std::fs::remove_file(&path);
    let out = out.map_err(|e| e.to_string())?;
    if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_string()) }
}

/// A PowerShell string literal.
fn q(s: impl AsRef<std::ffi::OsStr>) -> String { format!("'{}'", s.as_ref().to_string_lossy().replace('\'', "''")) }

/// The registry key without "HKCU", for PowerShell.
fn ps_key() -> String { format!("HKCU:{}", &uninstall_key()[4..]) }

/// Start menu (and desktop) shortcuts and the "Installed apps" entry.
fn register(dir: &Path, desktop: bool, kb: u64) -> Result<(), String> {
    let (n, exe) = (q(name()), dir.join("Gambito.exe"));
    let un = format!("\"{}\" --uninstall", exe.display());
    let script = format!(r#"
$k = {key}
New-Item -Force $k | Out-Null
$v = @{{ DisplayName = {n}; DisplayVersion = {ver}; Publisher = {n}; InstallLocation = {dir}; DisplayIcon = {exe}; UninstallString = {un}; QuietUninstallString = {unq} }}
foreach ($p in $v.Keys) {{ Set-ItemProperty $k $p $v[$p] }}
foreach ($p in 'NoModify', 'NoRepair') {{ Set-ItemProperty $k $p 1 -Type DWord }}
Set-ItemProperty $k EstimatedSize {kb} -Type DWord
$s = New-Object -ComObject WScript.Shell
$progs = $s.SpecialFolders('Programs')
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue (Join-Path $progs {n})   # the folder the first installer made
$desk = Join-Path $s.SpecialFolders('Desktop') ({n} + '.lnk')
$links = @(Join-Path $progs ({n} + '.lnk'))
if ({d}) {{ $links += $desk }} else {{ Remove-Item -Force -ErrorAction SilentlyContinue $desk }}
foreach ($l in $links) {{ $o = $s.CreateShortcut($l); $o.TargetPath = {exe}; $o.WorkingDirectory = {dir}; $o.Save() }}
"#, key = q(ps_key()), ver = q(env!("CARGO_PKG_VERSION")), dir = q(dir), exe = q(&exe), unq = q(format!("{un} --silent")), un = q(&un),
        d = if desktop { "$true" } else { "$false" });
    powershell(&script)
}

/// After an update: the new version and size in "Installed apps" (shortcuts and the rest stay as they were).
fn set_version(kb: u64) -> Result<(), String> {
    powershell(&format!("$k = {key}\nif (Test-Path $k) {{ Set-ItemProperty $k DisplayVersion {ver}; Set-ItemProperty $k EstimatedSize {kb} -Type DWord }}\n",
        key = q(ps_key()), ver = q(env!("CARGO_PKG_VERSION"))))
}

fn unregister() -> Result<(), String> {
    powershell(&format!(r#"
$s = New-Object -ComObject WScript.Shell
foreach ($f in 'Programs', 'Desktop') {{ Remove-Item -Force -ErrorAction SilentlyContinue (Join-Path $s.SpecialFolders($f) ({n} + '.lnk')) }}
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue (Join-Path $s.SpecialFolders('Programs') {n})
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue {key}
"#, n = q(name()), key = q(ps_key())))
}

/// Engines the old versions unpacked into %TEMP%.
fn clean_temp() {
    let Ok(rd) = std::fs::read_dir(std::env::temp_dir()) else { return };
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().to_lowercase();
        if n.starts_with("gambito-") && !n.contains("setup") && (n.ends_with(".exe") || n.ends_with(".part")) { let _ = std::fs::remove_file(e.path()); }
    }
}

/// Copies this exe to `dir\Gambito.exe`.
fn place_app(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir.join("engines")).map_err(|e| e.to_string())?;
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let target = dir.join("Gambito.exe");
    if me.canonicalize().ok() != target.canonicalize().ok() {
        std::fs::copy(&me, &target).map_err(|_| tl("Não consegui copiar o Gambito para esta pasta. Se estiver aberto, fecha-o e tenta de novo.").to_string())?;
    }
    for old in ["Uninstall.exe", "Desinstalar.exe"] { let _ = std::fs::remove_file(dir.join(old)); } // the first installer's
    clean_temp();
    Ok(())
}

/// Removes the engines, the shortcuts and the registry entry; Gambito.exe goes after this process ends.
fn remove_app(dir: &Path, data: bool) -> Result<(), String> {
    for k in engine::known() {
        if let Err(e) = std::fs::remove_file(engine_file(dir, &k)) {
            if e.kind() != std::io::ErrorKind::NotFound { return Err(tl("Fecha o Gambito antes de desinstalar.").into()); }
        }
        for ext in ["part", "unzipped"] { let _ = std::fs::remove_file(engine_file(dir, &k).with_extension(ext)); } // a download that was cut off
    }
    let _ = std::fs::remove_dir(dir.join("engines")); // only if empty
    for old in ["Uninstall.exe", "Desinstalar.exe"] { let _ = std::fs::remove_file(dir.join(old)); }
    clean_temp();
    let _ = unregister();
    if data { if let Some(a) = std::env::var_os("APPDATA") { let _ = std::fs::remove_dir_all(PathBuf::from(a).join("Gambito")); } }
    Ok(())
}

/// A running exe can't delete itself: cmd does it a moment after we quit (rmdir only takes an empty folder).
fn delete_later(dir: &Path) {
    let cmd = format!("/C ping -n 3 127.0.0.1 >nul & del /f /q \"{}\" & rmdir \"{}\"", dir.join("Gambito.exe").display(), dir.display());
    let _ = Command::new("cmd").raw_arg(cmd).creation_flags(NO_WINDOW).spawn();
}

static REMOVED: OnceLock<PathBuf> = OnceLock::new();

fn silent(mode: Mode) -> i32 {
    let dir = install_dir();
    match mode {
        Mode::Install => {
            let update = has_arg("--update"); // from the app (src/update.rs): keep the user's engines and shortcuts
            let known: Vec<Known> = engine::known().into_iter().enumerate()
                .filter(|(i, k)| !update || *i == 0 || engine_file(&dir, k).exists()).map(|(_, k)| k).collect();
            // the app that started the update is closing: its exe can't be replaced until it has
            let mut placed = place_app(&dir);
            for _ in 0..60 {
                if placed.is_ok() || !update { break; }
                std::thread::sleep(Duration::from_millis(500));
                placed = place_app(&dir);
            }
            if let Err(e) = placed { eprintln!("{e}"); return 1; }
            let kb = app_kb() + known.iter().map(|k| k.kb).sum::<u64>();
            let reg = if update { set_version(kb) } else { register(&dir, has_arg("--desktop"), kb) };
            if let Err(e) = reg { eprintln!("{e}"); }
            let stop = AtomicBool::new(false);
            let results: Vec<Result<(), String>> = std::thread::scope(|s| {
                let jobs: Vec<_> = known.iter().map(|k| { let (dir, stop) = (&dir, &stop); s.spawn(move || fetch(k, dir, &Mutex::new(Row::Wait), stop, &|| {})) }).collect();
                jobs.into_iter().map(|j| j.join().unwrap_or_else(|_| Err("panic".into()))).collect()
            });
            for (k, r) in known.iter().zip(&results) { if let Err(e) = r { eprintln!("{}: {e}", k.name); } }
            if update && !has_arg("--no-start") { let _ = Command::new(dir.join("Gambito.exe")).current_dir(&dir).spawn(); } // back to the app
            if results[0].is_ok() { 0 } else { 1 }
        }
        Mode::Uninstall => match remove_app(&dir, has_arg("--remove-data")) {
            Ok(()) => { delete_later(&dir); 0 }
            Err(e) => { eprintln!("{e}"); 1 }
        },
    }
}


pub fn run(mode: Mode) -> eframe::Result {
    // the installer starts in English unless Gambito already has a language saved
    if mode == Mode::Install && crate::i18n::saved().is_none() { crate::i18n::show(1); }
    if has_arg("--silent") { std::process::exit(silent(mode)); }
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([W, H]).with_resizable(false).with_decorations(false).with_title(tl("Gambito — Instalador")).with_icon(Arc::new(app_icon())),
        centered: true,
        ..Default::default()
    };
    let r = eframe::run_native("Gambito Setup", opts, Box::new(move |cc| Ok(Box::new(Setup::new(cc, mode)))));
    if let Some(dir) = REMOVED.get() { delete_later(dir); }
    r
}

// ---------- the window (see the "Gambito — Instalador" design canvas) ----------

const W: f32 = 600.0;
const H: f32 = 656.0;
const TXT: Color32 = CREAM;
const TXT2: Color32 = Color32::from_rgb(216, 209, 191);
const SUB: Color32 = Color32::from_rgb(196, 189, 172);
const DIM: Color32 = MUTED;
const ACC: Color32 = GOLD;
const ACC_HI: Color32 = GOLD_HI;
const INK: Color32 = Color32::from_rgb(28, 20, 4);
const CARD_BG: Color32 = CARD;
const CARD_LINE: Color32 = LINE;
const DIV: Color32 = LINE;
const ROW_LINE: Color32 = Color32::from_rgb(34, 34, 39);
const OUT_BG: Color32 = Color32::from_rgb(22, 22, 26);
const OUT_HOV: Color32 = Color32::from_rgb(34, 34, 39);
const OUT_LINE: Color32 = Color32::from_rgb(58, 58, 64);
const TRACK: Color32 = Color32::from_rgb(38, 38, 43);
const HILITE: Color32 = GOLD_BG;
const FAIL: Color32 = Color32::from_rgb(240, 110, 90);

fn inter(size: f32) -> egui::FontId { egui::FontId::proportional(size) }

#[derive(Clone, Copy)]
enum Gl { ArrowL, ArrowR, Download, Check, Globe, Folder, Info, Clock, Retry, ChevD }

/// The small line icons of the design.
fn glyph(p: &egui::Painter, g: Gl, c: Pos2, s: f32, col: Color32) {
    let u = s / 24.0;
    let pt = |x: f32, y: f32| c + Vec2::new((x - 12.0) * u, (y - 12.0) * u);
    let sw = Stroke::new((1.9 * u).max(1.3), col);
    let line = |pts: &[(f32, f32)]| { p.add(egui::Shape::line(pts.iter().map(|&(x, y)| pt(x, y)).collect(), sw)); };
    match g {
        Gl::ArrowR => { line(&[(4.0, 12.0), (20.0, 12.0)]); line(&[(14.0, 6.0), (20.0, 12.0), (14.0, 18.0)]); }
        Gl::ArrowL => { line(&[(20.0, 12.0), (4.0, 12.0)]); line(&[(10.0, 6.0), (4.0, 12.0), (10.0, 18.0)]); }
        Gl::Download => { line(&[(12.0, 3.0), (12.0, 15.0)]); line(&[(7.0, 10.5), (12.0, 15.5), (17.0, 10.5)]); line(&[(3.5, 16.0), (3.5, 20.0), (20.5, 20.0), (20.5, 16.0)]); }
        Gl::Check => line(&[(4.5, 12.5), (9.5, 17.5), (19.5, 7.0)]),
        Gl::Globe => {
            p.circle_stroke(pt(12.0, 12.0), 9.5 * u, sw);
            line(&[(2.5, 12.0), (21.5, 12.0)]);
            let e: Vec<Pos2> = (0..=24).map(|i| { let a = i as f32 / 24.0 * std::f32::consts::TAU; pt(12.0 + a.cos() * 4.2, 12.0 + a.sin() * 9.5) }).collect();
            p.add(egui::Shape::line(e, sw));
        }
        Gl::Folder => { p.add(egui::Shape::closed_line([(2.0, 4.0), (9.0, 4.0), (11.5, 7.0), (22.0, 7.0), (22.0, 20.0), (2.0, 20.0)].iter().map(|&(x, y)| pt(x, y)).collect(), sw)); line(&[(2.0, 10.0), (22.0, 10.0)]); }
        Gl::Info => { p.circle_stroke(pt(12.0, 12.0), 9.5 * u, sw); line(&[(12.0, 10.5), (12.0, 17.0)]); p.circle_filled(pt(12.0, 7.3), 1.3 * u, col); }
        Gl::Clock => { p.circle_stroke(pt(12.0, 12.0), 9.5 * u, sw); line(&[(12.0, 6.5), (12.0, 12.0), (15.5, 14.5)]); }
        Gl::Retry => ico(p, Ico::Restart, c, s, col),
        Gl::ChevD => line(&[(6.0, 9.0), (12.0, 15.0), (18.0, 9.0)]),
    }
}

/// A flag for the language menu: 0 Portugal, 1 United Kingdom.
fn flag(p: &egui::Painter, lang: usize, r: Rect) {
    let p = p.with_clip_rect(r);
    if lang == 0 {
        p.rect_filled(Rect::from_min_max(r.min, Pos2::new(r.left() + r.width() * 0.4, r.bottom())), 0.0, Color32::from_rgb(4, 106, 56));
        p.rect_filled(Rect::from_min_max(Pos2::new(r.left() + r.width() * 0.4, r.top()), r.max), 0.0, Color32::from_rgb(218, 41, 28));
        let c = Pos2::new(r.left() + r.width() * 0.4, r.center().y);
        p.circle_filled(c, r.height() * 0.23, Color32::from_rgb(255, 233, 0));
        p.circle_filled(c, r.height() * 0.13, Color32::from_rgb(218, 41, 28));
    } else {
        let (white, red) = (Color32::WHITE, Color32::from_rgb(200, 16, 46));
        p.rect_filled(r, 0.0, Color32::from_rgb(1, 33, 105));
        for (a, b) in [(r.left_top(), r.right_bottom()), (r.right_top(), r.left_bottom())] {
            p.line_segment([a, b], Stroke::new(r.height() * 0.19, white));
            p.line_segment([a, b], Stroke::new(r.height() * 0.07, red));
        }
        let (h, v) = ([Pos2::new(r.left(), r.center().y), Pos2::new(r.right(), r.center().y)], [Pos2::new(r.center().x, r.top()), Pos2::new(r.center().x, r.bottom())]);
        for (w, c) in [(0.3, white), (0.17, red)] { p.line_segment(h, Stroke::new(r.height() * w, c)); p.line_segment(v, Stroke::new(r.height() * w, c)); }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Look { Accent, Outline, OutlineAcc }

/// A button of the design, at a fixed place.
fn button(ui: &mut egui::Ui, r: Rect, id: &str, label: &str, left: Option<Gl>, right: Option<Gl>, look: Look) -> bool {
    let resp = ui.interact(r, egui::Id::new(("setup_btn", id)), Sense::click());
    let p = ui.painter();
    let hot = resp.hovered();
    match look {
        Look::Accent => { p.rect_filled(r, 7.0, if hot { ACC_HI } else { ACC }); }
        Look::Outline => { p.rect_filled(r, 7.0, if hot { OUT_HOV } else { OUT_BG }); p.rect_stroke(r, 7.0, Stroke::new(1.0, OUT_LINE), egui::StrokeKind::Inside); }
        Look::OutlineAcc => { if hot { p.rect_filled(r, 7.0, GOLD_BG); } p.rect_stroke(r, 7.0, Stroke::new(1.5, ACC), egui::StrokeKind::Inside); }
    }
    let fg = if look == Look::Accent { INK } else { TXT };
    let font = if look == Look::OutlineAcc { inter(14.5) } else { f_med((r.height() * 0.37).min(18.0)) };
    let g = p.layout_no_wrap(label.to_string(), font, fg);
    let (gw, gh) = (g.size().x, g.size().y);
    let (isz, gap) = (19.0, 12.0);
    let total = gw + if left.is_some() || right.is_some() { isz + gap } else { 0.0 };
    let mut x = r.center().x - total / 2.0;
    if let Some(i) = left { glyph(p, i, Pos2::new(x + isz / 2.0, r.center().y), isz, fg); x += isz + gap; }
    p.galley(Pos2::new(x, r.center().y - gh / 2.0), g, fg);
    if let Some(i) = right { glyph(p, i, Pos2::new(x + gw + gap + isz / 2.0, r.center().y), isz, fg); }
    resp.clicked()
}

/// The cyan check box (26 px).
fn checkbox(p: &egui::Painter, at: Pos2, on: bool, enabled: bool) {
    let r = Rect::from_min_size(at, Vec2::splat(26.0));
    if on {
        p.rect_filled(r, 5.0, if enabled { ACC } else { GOLD_LO });
        let c = r.center();
        p.add(egui::Shape::line(vec![c + Vec2::new(-6.5, 0.5), c + Vec2::new(-2.0, 5.0), c + Vec2::new(6.5, -5.0)], Stroke::new(2.6, INK)));
    } else {
        p.rect_stroke(r, 5.0, Stroke::new(1.8, SUB), egui::StrokeKind::Inside);
    }
}

struct Setup {
    mode: Mode,
    page: u8, // install: 0 language, 1 engines, 2 folder, 3 installing, 4 done; uninstall: 0 confirm, 4 done
    dir: String,
    inst: PathBuf,
    known: Vec<Known>,
    pick: Vec<bool>,
    desktop: bool,
    launch: bool,
    data: bool,
    lang_open: bool,
    rows: Arc<Vec<Mutex<Row>>>,
    stop: Arc<AtomicBool>,
    reg: Arc<Mutex<Option<Result<(), String>>>>,
    err: String,
    logo: egui::TextureHandle,
    knight: Option<egui::TextureHandle>,
    board: Option<egui::TextureHandle>,
    t0: Instant,
    shot: bool,
}

fn png_texture(ctx: &egui::Context, name: &str, bytes: &[u8]) -> egui::TextureHandle {
    let img = image::load_from_memory(bytes).map(|i| i.to_rgba8()).unwrap_or_else(|_| image::RgbaImage::new(1, 1));
    let size = [img.width() as usize, img.height() as usize];
    ctx.load_texture(name, egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()), egui::TextureOptions::LINEAR)
}

impl Setup {
    fn new(cc: &eframe::CreationContext, mode: Mode) -> Self {
        setup_style(&cc.egui_ctx);
        let known = engine::known();
        let dir = if mode == Mode::Install { install_dir() } else { arg_after("--dir").map(PathBuf::from).or_else(registered_dir).unwrap_or_else(|| std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)).unwrap_or_default()) };
        let page = std::env::var("GAMBITO_SETUP_PAGE").ok().and_then(|p| p.parse().ok()).filter(|p| *p <= 2).unwrap_or(0); // dev only
        let mut s = Setup {
            mode, page, dir: dir.display().to_string(), inst: dir, pick: vec![true; known.len()], known, desktop: true, launch: true, data: false, lang_open: false,
            rows: Arc::new(vec![]), stop: Arc::new(AtomicBool::new(false)), reg: Arc::new(Mutex::new(None)), err: String::new(),
            logo: load_logo(&cc.egui_ctx), knight: None, board: None, t0: Instant::now(), shot: false,
        };
        if mode == Mode::Install && has_arg("--auto") { s.start(&cc.egui_ctx); }
        s
    }

    fn start(&mut self, ctx: &egui::Context) {
        let dir = PathBuf::from(self.dir.trim());
        if !dir.is_absolute() { self.err = tl("Escolhe uma pasta válida.").into(); return; }
        if let Err(e) = place_app(&dir) { self.err = e; return; }
        crate::i18n::set(crate::i18n::lang()); // the app opens in the language picked here
        let kb = app_kb() + self.known.iter().zip(&self.pick).filter(|(_, on)| **on).map(|(k, _)| k.kb).sum::<u64>();
        let (reg, d, desktop, c) = (self.reg.clone(), dir.clone(), self.desktop, ctx.clone());
        *reg.lock().unwrap() = None;
        std::thread::spawn(move || { let r = register(&d, desktop, kb); *reg.lock().unwrap() = Some(r); c.request_repaint(); });
        self.inst = dir;
        self.stop.store(false, Ordering::Relaxed);
        self.rows = Arc::new(self.pick.iter().map(|&on| Mutex::new(if on { Row::Wait } else { Row::Off })).collect());
        for i in 0..self.known.len() { if self.pick[i] { self.spawn(i, ctx); } }
        self.err.clear();
        self.page = 3;
    }

    fn spawn(&self, i: usize, ctx: &egui::Context) {
        let (rows, stop, dir, ctx) = (self.rows.clone(), self.stop.clone(), self.inst.clone(), ctx.clone());
        *rows[i].lock().unwrap() = Row::Wait;
        std::thread::spawn(move || {
            let k = &engine::known()[i];
            let r = fetch(k, &dir, &rows[i], &stop, &|| ctx.request_repaint());
            *rows[i].lock().unwrap() = match r { Ok(()) => Row::Ok, Err(e) => Row::Fail(e) };
            ctx.request_repaint();
        });
    }

    fn row(&self, i: usize) -> Row { self.rows.get(i).map_or(Row::Off, |r| r.lock().unwrap().clone()) }
    fn busy(&self) -> bool { (0..self.rows.len()).any(|i| matches!(self.row(i), Row::Wait | Row::Get(..) | Row::Check)) }

    /// Window bar: small logo, name, minimise and close; drag anywhere else on it to move the window.
    fn title_bar(&mut self, ui: &mut egui::Ui, o: Pos2) {
        let bar = Rect::from_min_size(o, Vec2::new(W, 36.0));
        let close = Rect::from_min_size(Pos2::new(bar.right() - 46.0, bar.top()), Vec2::new(46.0, 35.0));
        let min = close.translate(Vec2::new(-46.0, 0.0));
        if ui.interact(Rect::from_min_max(bar.min, Pos2::new(min.left(), bar.bottom())), egui::Id::new("drag_bar"), Sense::drag()).drag_started() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        let p = ui.painter();
        p.line_segment([bar.left_bottom(), bar.right_bottom()], Stroke::new(1.0, Color32::from_rgb(34, 34, 39)));
        p.image(self.logo.id(), Rect::from_min_size(o + Vec2::new(22.0, 6.0), Vec2::new(21.0, 24.0)), logo_full_uv(), Color32::WHITE);
        p.text(o + Vec2::new(54.0, 18.0), egui::Align2::LEFT_CENTER, tl("Gambito — Instalador"), inter(14.0), TXT2);
        let m = ui.interact(min, egui::Id::new("min"), Sense::click());
        if m.hovered() { p.rect_filled(min, 0.0, Color32::from_rgb(30, 30, 34)); }
        p.line_segment([min.center() - Vec2::new(6.0, 0.0), min.center() + Vec2::new(6.0, 0.0)], Stroke::new(1.4, TXT));
        if m.clicked() { ui.ctx().send_viewport_cmd(egui::ViewportCommand::Minimized(true)); }
        let c = ui.interact(close, egui::Id::new("close"), Sense::click());
        if c.hovered() { p.rect_filled(close, 0.0, Color32::from_rgb(196, 43, 28)); }
        ico(p, Ico::Cross, close.center(), 15.0, TXT);
        if c.clicked() { ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close); }
    }

    /// Logo and name.
    fn brand(&self, p: &egui::Painter, at: Pos2, h: f32, font: f32) {
        let w = h * 0.863;
        p.image(self.logo.id(), Rect::from_min_size(at, Vec2::new(w, h)), logo_full_uv(), Color32::WHITE);
        p.text(Pos2::new(at.x + w + 16.0, at.y + h / 2.0), egui::Align2::LEFT_CENTER, "Gambito", f_semi(font), TXT);
    }

    fn knight_art(&mut self, ui: &egui::Ui, at: Pos2, w: f32) {
        let t = self.knight.get_or_insert_with(|| png_texture(ui.ctx(), "setup_knight", include_bytes!("../assets/setup/knight.png")));
        ui.painter().image(t.id(), Rect::from_min_size(at, Vec2::splat(w)), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
    }

    fn footer_line(p: &egui::Painter, o: Pos2, y: f32, inset: f32) {
        p.line_segment([o + Vec2::new(inset, y), o + Vec2::new(W - inset, y)], Stroke::new(1.0, DIV));
    }

    // ---------- 1 · language ----------
    fn language_page(&mut self, ui: &mut egui::Ui, o: Pos2) {
        self.knight_art(ui, o + Vec2::new(362.0, 148.0), 251.0);
        let p = ui.painter().clone();
        self.brand(&p, o + Vec2::new(40.0, 92.0), 72.0, 46.0);
        p.text(o + Vec2::new(40.0, 194.0), egui::Align2::LEFT_TOP, tl("Seleciona o idioma"), f_semi(31.0), TXT);
        p.text(o + Vec2::new(40.0, 240.0), egui::Align2::LEFT_TOP, tl("Escolhe o idioma do instalador e do Gambito."), inter(15.5), DIM);
        glyph(&p, Gl::Globe, o + Vec2::new(51.0, 311.0), 22.0, ACC);
        p.text(o + Vec2::new(74.0, 311.0), egui::Align2::LEFT_CENTER, tl("Idioma"), f_med(16.0), TXT);
        let field = Rect::from_min_size(o + Vec2::new(40.0, 335.0), Vec2::new(348.0, 42.0));
        let resp = ui.interact(field, egui::Id::new("lang_field"), Sense::click());
        p.rect_filled(field, 8.0, CARD_BG);
        p.rect_stroke(field, 8.0, Stroke::new(1.0, if resp.hovered() || self.lang_open { Color32::from_rgb(84, 84, 92) } else { Color32::from_rgb(52, 52, 58) }), egui::StrokeKind::Inside);
        let name = |i: usize| if i == 0 { "Português (Portugal)" } else { "English" };
        let cur = crate::i18n::lang();
        flag(&p, cur, Rect::from_min_size(field.min + Vec2::new(16.0, 11.5), Vec2::new(28.0, 19.0)));
        p.text(field.min + Vec2::new(56.0, 21.0), egui::Align2::LEFT_CENTER, name(cur), inter(15.5), TXT);
        glyph(&p, Gl::ChevD, Pos2::new(field.right() - 22.0, field.center().y), 16.0, TXT);
        if resp.clicked() { self.lang_open = !self.lang_open; }
        // footer
        Self::footer_line(&p, o, 539.0, 32.0);
        if button(ui, Rect::from_min_size(o + Vec2::new(32.0, 568.0), Vec2::new(130.0, 40.0)), "cancel", tl("Cancelar"), None, None, Look::Outline) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if button(ui, Rect::from_min_size(o + Vec2::new(384.0, 567.0), Vec2::new(184.0, 43.0)), "continue0", tl("Continuar"), None, Some(Gl::ArrowR), Look::Accent) {
            self.page = 1;
            self.lang_open = false;
        }
        // the menu, on top of the rest
        if self.lang_open {
            let menu = Rect::from_min_size(field.left_bottom() + Vec2::new(0.0, 4.0), Vec2::new(field.width(), 84.0));
            p.rect_filled(menu, 8.0, Color32::from_rgb(28, 28, 32));
            p.rect_stroke(menu, 8.0, Stroke::new(1.0, Color32::from_rgb(52, 52, 58)), egui::StrokeKind::Inside);
            for (k, lang) in [1usize, 0].into_iter().enumerate() {
                let r = Rect::from_min_size(menu.min + Vec2::new(4.0, 4.0 + k as f32 * 38.0), Vec2::new(menu.width() - 8.0, 38.0));
                let it = ui.interact(r, egui::Id::new(("lang_item", lang)), Sense::click());
                if it.hovered() || lang == cur { p.rect_filled(r, 6.0, HILITE); }
                flag(&p, lang, Rect::from_min_size(r.min + Vec2::new(12.0, 9.5), Vec2::new(28.0, 19.0)));
                p.text(r.min + Vec2::new(52.0, 19.0), egui::Align2::LEFT_CENTER, name(lang), inter(15.0), TXT);
                if it.clicked() { crate::i18n::show(lang); self.lang_open = false; }
            }
            if ui.input(|i| i.pointer.any_click()) && !ui.rect_contains_pointer(menu) && !resp.clicked() { self.lang_open = false; }
        }
    }

    // ---------- 2 · engines ----------
    fn engines_page(&mut self, ui: &mut egui::Ui, o: Pos2) {
        self.knight_art(ui, o + Vec2::new(381.0, 32.0), 226.0);
        let p = ui.painter().clone();
        self.brand(&p, o + Vec2::new(44.0, 66.0), 60.0, 40.0);
        p.text(o + Vec2::new(44.0, 148.0), egui::Align2::LEFT_TOP, tl("Escolher engines de xadrez"), f_semi(27.0), TXT);
        p.text(o + Vec2::new(44.0, 188.0), egui::Align2::LEFT_TOP, tl("Seleciona as engines que queres descarregar e instalar."), inter(14.5), DIM);
        let card = Rect::from_min_size(o + Vec2::new(24.0, 222.0), Vec2::new(552.0, 274.0));
        p.rect_filled(card, 10.0, CARD_BG);
        p.rect_stroke(card, 10.0, Stroke::new(1.0, CARD_LINE), egui::StrokeKind::Inside);
        ui.scope_builder(egui::UiBuilder::new().max_rect(card.shrink(1.0)), |ui| {
            ui.set_clip_rect(card.shrink(1.0));
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                for i in 0..self.known.len() {
                    let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 64.0), Sense::click());
                    let p = ui.painter();
                    let locked = i == 0;
                    if resp.hovered() && !locked { p.rect_filled(r, 0.0, Color32::from_rgb(26, 26, 30)); }
                    if i > 0 { p.line_segment([r.left_top(), r.right_top()], Stroke::new(1.0, ROW_LINE)); }
                    checkbox(p, Pos2::new(r.left() + 22.0, r.center().y - 13.0), self.pick[i], !locked);
                    let k = &self.known[i];
                    p.text(Pos2::new(r.left() + 70.0, r.center().y - 9.0), egui::Align2::LEFT_CENTER, k.name, f_med(16.5), TXT);
                    let desc = if locked { tl("Obrigatória: joga, sugere lances e avalia.").to_string() } else { tr!("Adversário extra, de {} · {}", k.by, k.license) };
                    p.text(Pos2::new(r.left() + 70.0, r.center().y + 11.0), egui::Align2::LEFT_CENTER, desc, inter(12.5), SUB);
                    let dx = r.right() - 108.0;
                    p.line_segment([Pos2::new(dx, r.center().y - 15.0), Pos2::new(dx, r.center().y + 15.0)], Stroke::new(1.0, Color32::from_rgb(52, 52, 58)));
                    p.text(Pos2::new(r.right() - 22.0, r.center().y), egui::Align2::RIGHT_CENTER, format!("≈ {} MB", (k.kb as f32 / 1024.0).round()), inter(13.5), TXT2);
                    if resp.clicked() && !locked { self.pick[i] = !self.pick[i]; }
                }
            });
        });
        let chosen: Vec<&Known> = self.known.iter().zip(&self.pick).filter(|(_, on)| **on).map(|(k, _)| k).collect();
        let mb = (chosen.iter().map(|k| k.kb).sum::<u64>() as f32 / 1024.0).round();
        glyph(&p, Gl::Info, o + Vec2::new(41.0, 521.0), 18.0, TXT2);
        p.text(o + Vec2::new(60.0, 521.0), egui::Align2::LEFT_CENTER, tr!("{} de {} escolhidas · {} MB. Podes juntar outras mais tarde.", chosen.len(), self.known.len(), mb), inter(13.5), TXT2);
        Self::footer_line(&p, o, 557.0, 24.0);
        if button(ui, Rect::from_min_size(o + Vec2::new(24.0, 583.0), Vec2::new(132.0, 42.0)), "back1", tl("Voltar"), Some(Gl::ArrowL), None, Look::Outline) { self.page = 0; }
        if button(ui, Rect::from_min_size(o + Vec2::new(404.0, 583.0), Vec2::new(172.0, 43.0)), "continue1", tl("Continuar"), None, Some(Gl::ArrowR), Look::Accent) { self.page = 2; }
    }

    // ---------- 3 · folder ----------
    fn folder_page(&mut self, ui: &mut egui::Ui, o: Pos2) {
        self.knight_art(ui, o + Vec2::new(392.0, 47.0), 226.0);
        let p = ui.painter().clone();
        self.brand(&p, o + Vec2::new(48.0, 94.0), 72.0, 46.0);
        p.text(o + Vec2::new(48.0, 202.0), egui::Align2::LEFT_TOP, tl("Pasta de instalação"), f_semi(31.0), TXT);
        p.text(o + Vec2::new(48.0, 248.0), egui::Align2::LEFT_TOP, tl("Escolhe onde instalar o Gambito e as opções de atalho."), inter(15.0), SUB);
        let card = Rect::from_min_size(o + Vec2::new(24.0, 302.0), Vec2::new(552.0, 172.0));
        p.rect_filled(card, 10.0, CARD_BG);
        p.rect_stroke(card, 10.0, Stroke::new(1.0, CARD_LINE), egui::StrokeKind::Inside);
        glyph(&p, Gl::Folder, o + Vec2::new(59.0, 335.0), 24.0, TXT);
        p.text(o + Vec2::new(84.0, 335.0), egui::Align2::LEFT_CENTER, tl("Pasta de instalação"), f_med(15.5), TXT);
        let field = Rect::from_min_max(o + Vec2::new(46.0, 362.0), o + Vec2::new(436.0, 402.0));
        p.rect_filled(field, 7.0, Color32::from_rgb(14, 14, 16));
        p.rect_stroke(field, 7.0, Stroke::new(1.0, Color32::from_rgb(58, 58, 64)), egui::StrokeKind::Inside);
        ui.put(Rect::from_min_max(Pos2::new(field.left() + 14.0, field.center().y - 10.0), Pos2::new(field.right() - 10.0, field.center().y + 10.0)),
            egui::TextEdit::singleline(&mut self.dir).frame(egui::Frame::NONE).font(inter(14.0)).text_color(TXT).desired_width(field.width() - 24.0));
        if button(ui, Rect::from_min_max(o + Vec2::new(450.0, 362.0), o + Vec2::new(554.0, 402.0)), "browse", tl("Procurar…"), None, None, Look::OutlineAcc) {
            if let Some(d) = rfd::FileDialog::new().set_directory(&self.dir).pick_folder() {
                // a folder of its own, unless the one picked is already Gambito's
                self.dir = if d.file_name().is_some_and(|n| n.eq_ignore_ascii_case("Gambito")) { d } else { d.join("Gambito") }.display().to_string();
            }
        }
        let row = Rect::from_min_size(o + Vec2::new(46.0, 422.0), Vec2::new(400.0, 30.0));
        let resp = ui.interact(row, egui::Id::new("desktop"), Sense::click());
        checkbox(&p, Pos2::new(row.left(), row.top() + 2.0), self.desktop, true);
        p.text(Pos2::new(row.left() + 40.0, row.center().y), egui::Align2::LEFT_CENTER, tl("Criar atalho no ambiente de trabalho"), inter(15.5), TXT);
        if resp.clicked() { self.desktop = !self.desktop; }
        if !self.err.is_empty() {
            let g = p.layout(self.err.clone(), inter(13.5), FAIL, 536.0);
            p.galley(o + Vec2::new(32.0, 490.0), g, FAIL);
        }
        Self::footer_line(&p, o, 542.0, 26.0);
        if button(ui, Rect::from_min_size(o + Vec2::new(26.0, 573.0), Vec2::new(186.0, 46.0)), "back2", tl("Voltar"), Some(Gl::ArrowL), None, Look::Outline) { self.page = 1; self.err.clear(); }
        if button(ui, Rect::from_min_size(o + Vec2::new(360.0, 572.0), Vec2::new(214.0, 48.0)), "install", tl("Instalar"), Some(Gl::Download), None, Look::Accent) { self.start(ui.ctx()); }
    }

    // ---------- 4 · installing ----------
    fn working_page(&mut self, ui: &mut egui::Ui, o: Pos2) {
        let stopping = self.stop.load(Ordering::Relaxed);
        let busy = self.busy();
        if stopping && !busy { ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close); }
        let reg_done = self.reg.lock().unwrap().is_some();
        if !busy && !stopping && reg_done && self.row(0) == Row::Ok { self.page = 4; }
        if busy || !reg_done { ui.ctx().request_repaint_after(Duration::from_millis(50)); }
        let t = self.board.get_or_insert_with(|| png_texture(ui.ctx(), "setup_board", include_bytes!("../assets/setup/board.png")));
        ui.painter().image(t.id(), Rect::from_min_size(o + Vec2::new(278.0, 24.0), Vec2::new(338.0, 225.3)), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        let p = ui.painter().clone();
        self.brand(&p, o + Vec2::new(42.0, 70.0), 72.0, 46.0);
        p.text(o + Vec2::new(42.0, 158.0), egui::Align2::LEFT_TOP, if stopping { tl("A cancelar…") } else { tl("A instalar o Gambito e as engines…") }, f_med(18.0), TXT);
        let card = Rect::from_min_max(o + Vec2::new(20.0, 236.0), o + Vec2::new(580.0, 616.0));
        p.rect_filled(card, 12.0, CARD_BG);
        p.rect_stroke(card, 12.0, Stroke::new(1.0, CARD_LINE), egui::StrokeKind::Inside);
        let x0 = card.left() + 22.0;
        let x1 = card.right() - 22.0;
        // the engine in the spotlight: the first one downloading, else checking, else waiting
        let rows: Vec<Row> = (0..self.known.len()).map(|i| self.row(i)).collect();
        let pick_first = |f: &dyn Fn(&Row) -> bool| rows.iter().position(|r| f(r));
        let now = pick_first(&|r| matches!(r, Row::Get(..))).or_else(|| pick_first(&|r| *r == Row::Check)).or_else(|| pick_first(&|r| *r == Row::Wait));
        let sf_failed = matches!(rows[0], Row::Fail(_)) && !busy && !stopping;
        glyph(&p, Gl::Download, Pos2::new(x0 + 14.0, card.top() + 34.0), 28.0, if sf_failed { FAIL } else { ACC });
        let tx = x0 + 44.0;
        if sf_failed {
            p.text(Pos2::new(tx, card.top() + 26.0), egui::Align2::LEFT_CENTER, tl("Falhou"), f_med(14.5), FAIL);
            p.text(Pos2::new(tx, card.top() + 46.0), egui::Align2::LEFT_CENTER, self.known[0].name, f_med(19.0), TXT);
            p.text(Pos2::new(tx, card.top() + 68.0), egui::Align2::LEFT_CENTER, tl("A descarga falhou. Vê a ligação à internet."), inter(13.5), SUB);
            if button(ui, Rect::from_min_size(Pos2::new(x1 - 150.0, card.top() + 28.0), Vec2::new(150.0, 40.0)), "retry", tl("Tentar de novo"), Some(Gl::Retry), None, Look::Accent) {
                for i in 0..rows.len() { if matches!(rows[i], Row::Fail(_)) { self.spawn(i, ui.ctx()); } }
            }
        } else {
            p.text(Pos2::new(tx, card.top() + 26.0), egui::Align2::LEFT_CENTER, tl("A instalar agora"), f_med(14.5), ACC);
            let (name, status, frac) = match now {
                Some(i) => match &rows[i] {
                    Row::Get(d, t) => (self.known[i].name, tr!("A transferir… ({} MB de {} MB)", (*d as f32 / 1048576.0).round(), (*t as f32 / 1048576.0).round()), *d as f32 / (*t).max(1) as f32),
                    Row::Check => (self.known[i].name, tl("A verificar…").to_string(), 1.0),
                    _ => (self.known[i].name, tl("Em espera").to_string(), 0.0),
                },
                None => ("Gambito", tl("A terminar…").to_string(), 1.0),
            };
            p.text(Pos2::new(tx, card.top() + 48.0), egui::Align2::LEFT_CENTER, name, f_med(19.0), TXT);
            p.text(Pos2::new(tx, card.top() + 70.0), egui::Align2::LEFT_CENTER, status, inter(13.5), SUB);
            p.text(Pos2::new(x1, card.top() + 48.0), egui::Align2::RIGHT_CENTER, format!("{:.0}%", frac * 100.0), f_med(22.0), TXT);
            let bar = Rect::from_min_max(Pos2::new(x0, card.top() + 90.0), Pos2::new(x1, card.top() + 102.0));
            p.rect_filled(bar, 6.0, TRACK);
            if frac > 0.0 { p.rect_filled(Rect::from_min_size(bar.min, Vec2::new(bar.width() * frac.min(1.0), bar.height())), 6.0, ACC); }
        }
        p.line_segment([Pos2::new(x0, card.top() + 116.0), Pos2::new(x1, card.top() + 116.0)], Stroke::new(1.0, CARD_LINE));
        // the app, then one row per engine
        let mut y = card.top() + 124.0;
        let mut line = |icon: u8, name: &str, status: String, col: Color32, hi: bool| {
            let r = Rect::from_min_max(Pos2::new(x0 - 10.0, y), Pos2::new(x1 + 10.0, y + 24.0));
            if hi { p.rect_filled(r, 6.0, HILITE); }
            let c = Pos2::new(x0 + 10.0, r.center().y);
            match icon {
                0 => { let b = Rect::from_center_size(c, Vec2::splat(19.0)); p.rect_filled(b, 4.0, ACC); glyph(&p, Gl::Check, c, 15.0, INK); }
                1 => glyph(&p, Gl::Check, c, 20.0, ACC),
                2 => glyph(&p, Gl::Download, c, 19.0, ACC),
                3 => glyph(&p, Gl::Clock, c, 19.0, DIM),
                _ => ico(&p, Ico::Cross, c, 13.0, FAIL),
            }
            p.text(Pos2::new(x0 + 36.0, r.center().y), egui::Align2::LEFT_CENTER, name, f_med(15.0), TXT);
            p.text(Pos2::new(x1, r.center().y), egui::Align2::RIGHT_CENTER, status, inter(13.5), col);
            y += 24.0;
        };
        line(0, tl("Gambito (Aplicação)"), if reg_done { tl("Instalado").to_string() } else { tl("A instalar…").to_string() }, TXT2, false);
        for (i, row) in rows.iter().enumerate() {
            let name = self.known[i].name;
            match row {
                Row::Off => continue,
                Row::Ok => line(1, name, tl("Instalado").into(), TXT2, false),
                Row::Get(d, t) => line(2, name, tr!("A transferir…  {}%", (100 * d / (*t).max(1))), TXT2, now == Some(i)),
                Row::Check => line(2, name, tl("A verificar…").into(), TXT2, now == Some(i)),
                Row::Wait => line(3, name, tl("Em espera").into(), DIM, false),
                Row::Fail(_) => line(4, name, tl("Falhou").into(), FAIL, false),
            }
        }
        // overall progress, weighted by size
        let (mut done, mut total) = (0.0f32, 0.0f32);
        for (i, k) in self.known.iter().enumerate() {
            let f = match &rows[i] { Row::Off => continue, Row::Get(d, t) => *d as f32 / (*t).max(1) as f32, Row::Check | Row::Ok => 1.0, _ => 0.0 };
            total += k.kb as f32;
            done += k.kb as f32 * f;
        }
        let frac = done / total.max(1.0);
        let yb = card.bottom() - 62.0;
        p.line_segment([Pos2::new(x0, yb), Pos2::new(x1, yb)], Stroke::new(1.0, CARD_LINE));
        p.text(Pos2::new(x0, yb + 20.0), egui::Align2::LEFT_CENTER, tl("Progresso geral"), f_med(15.5), TXT);
        p.text(Pos2::new(x1, yb + 20.0), egui::Align2::RIGHT_CENTER, format!("{:.0}%", frac * 100.0), f_med(15.5), TXT);
        let bar = Rect::from_min_max(Pos2::new(x0, yb + 34.0), Pos2::new(x1, yb + 44.0));
        p.rect_filled(bar, 5.0, TRACK);
        if frac > 0.0 { p.rect_filled(Rect::from_min_size(bar.min, Vec2::new(bar.width() * frac.min(1.0), bar.height())), 5.0, ACC); }
    }

    // ---------- 5 · done (install and uninstall) ----------
    fn done_page(&mut self, ui: &mut egui::Ui, o: Pos2) {
        self.knight_art(ui, o + Vec2::new(344.0, 43.0), 318.0);
        let p = ui.painter().clone();
        self.brand(&p, o + Vec2::new(50.0, 106.0), 72.0, 46.0);
        let c = o + Vec2::new(W / 2.0, 248.0);
        p.circle_filled(c, 34.0, BG);
        p.circle_stroke(c, 33.0, Stroke::new(4.0, ACC));
        glyph(&p, Gl::Check, c, 40.0, TXT);
        let install = self.mode == Mode::Install;
        let (title, sub) = if install { (tl("Instalação concluída"), tl("O Gambito foi instalado com sucesso no teu computador.")) } else { (tl("O Gambito foi removido"), tl("Podes fechar esta janela.")) };
        p.text(o + Vec2::new(W / 2.0, 302.0), egui::Align2::CENTER_TOP, title, f_semi(34.0), TXT);
        p.text(o + Vec2::new(W / 2.0, 352.0), egui::Align2::CENTER_TOP, sub, inter(15.5), SUB);
        if install {
            let failed: Vec<&str> = (0..self.known.len()).filter(|&i| matches!(self.row(i), Row::Fail(_))).map(|i| self.known[i].name).collect();
            let mut note = String::new();
            if !failed.is_empty() { note = tr!("Não consegui descarregar: {}. Volta a correr o instalador para as juntar.", failed.join(", ")); }
            if let Some(Err(e)) = &*self.reg.lock().unwrap() { note = format!("{note} {}", tr!("Os atalhos não foram criados: {}", e)); }
            if !note.is_empty() {
                let g = p.layout(note.trim().to_string(), inter(13.0), FAIL, 480.0);
                let h = g.size().y;
                p.galley(o + Vec2::new(60.0, 388.0), g, FAIL);
                let _ = h;
            }
            let label = tl("Iniciar o Gambito");
            let tw = p.layout_no_wrap(label.to_string(), inter(17.0), TXT).size().x;
            let row = Rect::from_center_size(o + Vec2::new(W / 2.0, 440.0), Vec2::new(26.0 + 16.0 + tw, 30.0));
            let resp = ui.interact(row, egui::Id::new("launch"), Sense::click());
            checkbox(&p, Pos2::new(row.left(), row.top() + 2.0), self.launch, true);
            p.text(Pos2::new(row.left() + 42.0, row.center().y), egui::Align2::LEFT_CENTER, label, inter(17.0), TXT);
            if resp.clicked() { self.launch = !self.launch; }
            Self::footer_line(&p, o, 500.0, 37.0);
            if button(ui, Rect::from_min_size(o + Vec2::new(49.0, 535.0), Vec2::new(234.0, 50.0)), "back4", tl("Voltar"), Some(Gl::ArrowL), None, Look::Outline) { self.page = 1; }
            if button(ui, Rect::from_min_size(o + Vec2::new(317.0, 534.0), Vec2::new(234.0, 52.0)), "finish", tl("Concluir"), Some(Gl::Check), None, Look::Accent) {
                if self.launch { let _ = Command::new(self.inst.join("Gambito.exe")).current_dir(&self.inst).spawn(); }
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        } else {
            Self::footer_line(&p, o, 500.0, 37.0);
            if button(ui, Rect::from_min_size(o + Vec2::new(183.0, 534.0), Vec2::new(234.0, 52.0)), "close_done", tl("Fechar"), None, None, Look::Accent) {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    // ---------- uninstall ----------
    fn confirm_page(&mut self, ui: &mut egui::Ui, o: Pos2) {
        self.knight_art(ui, o + Vec2::new(392.0, 47.0), 226.0);
        let p = ui.painter().clone();
        self.brand(&p, o + Vec2::new(48.0, 94.0), 72.0, 46.0);
        p.text(o + Vec2::new(48.0, 202.0), egui::Align2::LEFT_TOP, tl("Desinstalar o Gambito"), f_semi(31.0), TXT);
        let g = p.layout(tr!("Remove a aplicação, as engines e os atalhos de {}.", self.inst.display()), inter(15.0), SUB, 500.0);
        let h = g.size().y;
        p.galley(o + Vec2::new(48.0, 248.0), g, SUB);
        let card = Rect::from_min_size(o + Vec2::new(24.0, 268.0 + h), Vec2::new(552.0, 84.0));
        p.rect_filled(card, 10.0, CARD_BG);
        p.rect_stroke(card, 10.0, Stroke::new(1.0, CARD_LINE), egui::StrokeKind::Inside);
        let resp = ui.interact(card, egui::Id::new("data"), Sense::click());
        checkbox(&p, Pos2::new(card.left() + 22.0, card.center().y - 13.0), self.data, true);
        p.text(Pos2::new(card.left() + 62.0, card.center().y - 11.0), egui::Align2::LEFT_CENTER, tl("Apagar também os meus dados"), f_med(15.5), TXT);
        p.text(Pos2::new(card.left() + 62.0, card.center().y + 12.0), egui::Align2::LEFT_CENTER, tl("Progresso, puzzles, lições, peças aprendidas e definições."), inter(13.0), DIM);
        if resp.clicked() { self.data = !self.data; }
        if !self.err.is_empty() { p.text(Pos2::new(card.left() + 8.0, card.bottom() + 20.0), egui::Align2::LEFT_CENTER, &self.err, inter(13.5), FAIL); }
        Self::footer_line(&p, o, 542.0, 26.0);
        if button(ui, Rect::from_min_size(o + Vec2::new(26.0, 573.0), Vec2::new(186.0, 46.0)), "cancel_un", tl("Cancelar"), None, None, Look::Outline) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if button(ui, Rect::from_min_size(o + Vec2::new(360.0, 572.0), Vec2::new(214.0, 48.0)), "uninstall", tl("Desinstalar"), None, None, Look::Accent) {
            match remove_app(&self.inst, self.data) {
                Ok(()) => { let _ = REMOVED.set(self.inst.clone()); self.page = 4; }
                Err(e) => self.err = e,
            }
        }
    }

    /// Dev only: GAMBITO_SHOT=<png> saves a picture of the window after GAMBITO_SHOT_AT seconds (default 2) and quits.
    fn dev_shot(&mut self, ctx: &egui::Context) {
        let Some(path) = std::env::var_os("GAMBITO_SHOT") else { return };
        let at: f32 = std::env::var("GAMBITO_SHOT_AT").ok().and_then(|s| s.parse().ok()).unwrap_or(2.0);
        ctx.request_repaint_after(Duration::from_millis(200));
        if self.t0.elapsed().as_secs_f32() > at && !self.shot {
            self.shot = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img| {
                let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
                if let Some(im) = image::RgbaImage::from_raw(img.size[0] as u32, img.size[1] as u32, bytes) { let _ = im.save(&path); }
                std::process::exit(0);
            })));
        }
    }
}

/// The whole logo (crown and frame) in the logo picture.
fn logo_full_uv() -> Rect { Rect::from_min_max(Pos2::new(330.0 / 1254.0, 145.0 / 1254.0), Pos2::new(925.0 / 1254.0, 835.0 / 1254.0)) }

impl eframe::App for Setup {
    fn ui(&mut self, ui: &mut egui::Ui, _f: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // closing while downloading cancels first, so no half files are left behind
        if ctx.input(|i| i.viewport().close_requested()) && self.page == 3 && self.busy() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.stop.store(true, Ordering::Relaxed);
        }
        let full = ui.max_rect();
        let o = full.min;
        // background: the app's black, a little warmer towards the top right
        let mut mesh = egui::Mesh::default();
        let corners = [(full.left_top(), (11, 11, 12)), (full.right_top(), (19, 17, 13)), (full.right_bottom(), (10, 10, 11)), (full.left_bottom(), (8, 8, 9))];
        for (pos, (r, g, b)) in corners { mesh.vertices.push(egui::epaint::Vertex { pos, uv: egui::epaint::WHITE_UV, color: Color32::from_rgb(r, g, b) }); }
        mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
        ui.painter().add(egui::Shape::mesh(mesh));
        match (self.mode, self.page) {
            (Mode::Install, 0) => self.language_page(ui, o),
            (Mode::Install, 1) => self.engines_page(ui, o),
            (Mode::Install, 2) => self.folder_page(ui, o),
            (Mode::Install, 3) => self.working_page(ui, o),
            (Mode::Uninstall, 0) => self.confirm_page(ui, o),
            _ => self.done_page(ui, o),
        }
        self.title_bar(ui, o);
        ui.painter().rect_stroke(full, 0.0, Stroke::new(1.0, Color32::from_rgb(34, 34, 39)), egui::StrokeKind::Inside);
        self.dev_shot(&ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-file zip, the way release zips are made.
    fn zip_with(name: &str, data: &[u8], deflate: bool) -> Vec<u8> {
        let body = if deflate {
            let mut e = flate2::write::DeflateEncoder::new(vec![], flate2::Compression::default());
            e.write_all(data).unwrap();
            e.finish().unwrap()
        } else { data.to_vec() };
        let method: u16 = if deflate { 8 } else { 0 };
        let sizes = [(body.len() as u32).to_le_bytes(), (data.len() as u32).to_le_bytes()].concat();
        let mut z = vec![];
        z.extend(0x0403_4b50u32.to_le_bytes()); z.extend([20, 0, 0, 0]); z.extend(method.to_le_bytes()); z.extend([0u8; 8]);
        z.extend(&sizes); z.extend((name.len() as u16).to_le_bytes()); z.extend([0, 0]); z.extend(name.as_bytes()); z.extend(&body);
        let cd = z.len() as u32;
        z.extend(0x0201_4b50u32.to_le_bytes()); z.extend([20, 0, 20, 0, 0, 0]); z.extend(method.to_le_bytes()); z.extend([0u8; 8]);
        z.extend(&sizes); z.extend((name.len() as u16).to_le_bytes()); z.extend([0u8; 12]); z.extend(0u32.to_le_bytes()); z.extend(name.as_bytes());
        let cd_size = z.len() as u32 - cd;
        z.extend(0x0605_4b50u32.to_le_bytes()); z.extend([0u8; 4]); z.extend(1u16.to_le_bytes()); z.extend(1u16.to_le_bytes());
        z.extend(cd_size.to_le_bytes()); z.extend(cd.to_le_bytes()); z.extend([0, 0]);
        z
    }

    #[test]
    fn takes_the_exe_out_of_a_zip() {
        let dir = std::env::temp_dir().join(format!("gambito-zip-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let data: Vec<u8> = (0..50_000u32).map(|i| (i % 251) as u8).collect();
        for deflate in [false, true] {
            let (zip, out) = (dir.join("a.zip"), dir.join("a.exe"));
            std::fs::write(&zip, zip_with("stockfish/stockfish-windows-x86-64.exe", &data, deflate)).unwrap();
            unzip_exe(&zip, &out).unwrap();
            assert_eq!(std::fs::read(&out).unwrap(), data);
        }
        std::fs::write(dir.join("b.zip"), zip_with("README.md", b"hi", false)).unwrap();
        assert!(unzip_exe(&dir.join("b.zip"), &dir.join("b.exe")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sha256_of_a_file() {
        let p = std::env::temp_dir().join(format!("gambito-sha-test-{}", std::process::id()));
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(sha_file(&p).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let _ = std::fs::remove_file(&p);
    }
}
