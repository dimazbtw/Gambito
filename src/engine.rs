use std::io::{BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, Sender};

/// An engine the installer (src/setup.rs) puts in `engines\` next to Gambito.exe, downloaded from its official
/// release. scripts/fetch-engines.ps1 fetches the same files for development builds and tests.
pub struct Known {
    pub name: &'static str,
    pub file: &'static str,
    pub url: &'static str,
    /// SHA-256 of the .exe (for Stockfish, of the .exe inside the zip)
    pub sha256: &'static str,
    /// size of the .exe in KB
    pub kb: u64,
    /// who makes it (its GitHub name) and its licence, for the installer's list
    pub by: &'static str,
    pub license: &'static str,
}

/// The first one is the default, and the one the app can't work without.
pub fn known() -> Vec<Known> {
    let list: [(&str, &str, &str, &str, u64, &str, &str); 7] = [
        ("Stockfish 19", "stockfish", "https://github.com/official-stockfish/Stockfish/releases/download/sf_19/stockfish-windows-x86-64-universal.zip", "45bc8e4969147db9c2eb533810637994619bff0eacc81ccfd9854394901bcbd0", 100631, "official-stockfish", "GPL-3.0"),
        ("Berserk 14", "berserk", "https://github.com/jhonnold/berserk/releases/download/14/berserk-14-avx2.exe", "129df854a536d72143d431d1b764ddbd7e50e2f54c28e6ac9ffb50d7a85d6462", 25944, "jhonnold", "GPL-3.0"),
        ("Obsidian 16", "obsidian", "https://github.com/gab8192/Obsidian/releases/download/v16.0/Obsidian160-avx2.exe", "00f9f5566e815275e29fec85f1e4d1dff0b8301ceb23f6bb79de68dcdce98743", 31483, "gab8192", "GPL-3.0"),
        ("Koivisto 9", "koivisto", "https://github.com/Luecx/Koivisto/releases/download/v9.0/Koivisto_9.0-windows-avx2-pgo.exe", "9fcea126ed7dc882718bd22804d6c1227377009cd24fb983a1694a6d625e443e", 15317, "Luecx", "GPL-3.0"),
        ("Alexandria 9.1", "alexandria", "https://github.com/PGG106/Alexandria/releases/download/v9.1.0/Alexandria-9.1.0-avx2.exe", "21fe201c21d7178f520fce94c7f9a6e378597b13f5d3a7e8aa00b4c1adf533b6", 40097, "PGG106", "GPL-3.0"),
        ("Caissa 2.0", "caissa", "https://github.com/Witek902/Caissa/releases/download/2.0/caissa-2.0-x64-avx2.exe", "043c0925df8c608d0d87b9e6b1c761240ddd1901ee8cba49e346686b28816b97", 77318, "Witek902", "MIT"),
        ("Viridithas 20", "viridithas", "https://github.com/cosmobobak/viridithas/releases/download/v20.0.0/viridithas-20-win-x86-64-v3.exe", "c7fa86b2b46fe1c34b0ea3715ad3b4a61d3d984ec63d50b1d6b4f187332fbc51", 55224, "cosmobobak", "AGPL-3.0"),
    ];
    list.into_iter().map(|(name, file, url, sha256, kb, by, license)| Known { name, file, url, sha256, kb, by, license }).collect()
}

/// Where the engine's .exe is: `engines\` next to the app, or assets/engines/ in a development build.
pub fn path(k: &Known) -> PathBuf {
    let name = format!("{}.exe", k.file);
    let installed = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("engines").join(&name)));
    match installed {
        Some(p) if p.exists() => p,
        _ => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets").join("engines").join(name),
    }
}

/// An engine process uses 100–250 MB: it starts with its first job and quits after this long without work.
const IDLE: std::time::Duration = std::time::Duration::from_secs(120);

pub enum Job {
    Play { fen: String, elo: u32 },
    /// depth 0: no cap (`go infinite`, until the next job arrives; send Stop to end it)
    Hint { fen: String, depth: u32, children: Vec<(String, String, u8)> }, // every legal move: (uci, fen after it, 0 normal / 1 mate / 2 other game end)
    /// ends an uncapped hint search
    Stop,
    Review { fens: Vec<String> },
}

#[derive(Clone)]
pub struct Line {
    pub mate: bool,
    pub score: i32,
    pub pv: Vec<String>,
}

pub enum Reply {
    Play { fen: String, uci: String },
    /// `depth`: the depth the lines come from (an uncapped hint sends one of these per depth)
    Hint { fen: String, depth: u32, lines: Vec<Line> },
    /// per position: see `Ev`; None if the position has no moves
    Review { fens: Vec<String>, evals: Vec<Option<Ev>> },
    /// positions of a whole-game review done so far, of how many
    ReviewProgress(usize, usize),
    /// the engine process could not start or died
    Dead(String),
}

/// (centipawns for the side to move clamped to +-1000, best move, centipawns of the 2nd best move)
pub type Ev = (i32, String, Option<i32>);

/// What the engine says it can do (from its `uci` option list).
#[derive(Default)]
struct Caps {
    elo: Option<(u32, u32)>,
    limit: bool,
    skill: bool,
    multipv: bool,
    threads: bool,
    hash: bool,
}

pub struct Engine {
    tx: Sender<Job>,
}

impl Engine {
    /// A light engine: one thread, 16 MB of hash (about 260 MB of memory). Enough to play a move at a set Elo.
    pub fn new(exe: &Path, out: Sender<Reply>, ctx: eframe::egui::Context) -> Self { Self::with(exe, out, ctx, 1, 16) }

    /// An engine for analysis (suggestions, game review): a few threads and more hash, but never all the cores,
    /// so the board stays smooth. Each extra thread costs about 28 MB; ten of them made 600 MB per engine.
    pub fn heavy(exe: &Path, out: Sender<Reply>, ctx: eframe::egui::Context) -> Self {
        let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
        Self::with(exe, out, ctx, (cores / 4).clamp(1, 4), 64)
    }

    fn with(exe: &Path, out: Sender<Reply>, ctx: eframe::egui::Context, threads: usize, hash_mb: u32) -> Self {
        let (tx, rx) = channel::<Job>();
        let exe = exe.to_path_buf();
        std::thread::spawn(move || {
            let label = exe.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let Ok(mut job) = rx.recv() else { return }; // no process until there is something to do
            'run: loop {
            let Ok(mut child) = Command::new(&exe)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                // CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS: the engines never take the CPU from the interface
                .creation_flags(0x0800_0000 | 0x0000_4000)
                .spawn()
            else {
                let _ = out.send(Reply::Dead(label));
                ctx.request_repaint();
                return;
            };
            let mut stdin = child.stdin.take().unwrap();
            let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
            let _ = writeln!(stdin, "uci");
            let mut caps = Caps::default();
            while let Some(Ok(l)) = lines.next() {
                if l.trim() == "uciok" {
                    break;
                }
                if let Some(rest) = l.strip_prefix("option name ") {
                    let name = rest.split(" type ").next().unwrap_or("");
                    let t: Vec<&str> = rest.split_whitespace().collect();
                    let num = |k: &str| t.iter().position(|x| *x == k).and_then(|i| t.get(i + 1)).and_then(|v| v.parse::<u32>().ok());
                    match name {
                        "UCI_Elo" => caps.elo = num("min").zip(num("max")),
                        "UCI_LimitStrength" => caps.limit = true,
                        "Skill Level" => caps.skill = true,
                        "MultiPV" => caps.multipv = true,
                        "Threads" => caps.threads = true,
                        "Hash" => caps.hash = true,
                        _ => {}
                    }
                }
            }
            if caps.threads { let _ = writeln!(stdin, "setoption name Threads value {threads}"); }
            if caps.hash { let _ = writeln!(stdin, "setoption name Hash value {hash_mb}"); }
            loop {
                if !matches!(job, Job::Review { .. }) {
                    while let Ok(newer) = rx.try_recv() {
                        job = newer; // only the latest play/hint request matters (reviews all count)
                    }
                }
                let mut ok = false; // false if the engine's output ended before an answer
                match job {
                    Job::Play { fen, elo } => {
                        // Strength: native UCI_Elo when the engine has it, else Skill Level, else search depth.
                        // ponytail: the depth/skill mappings are rough guesses, tune if an engine feels off.
                        let mut cmd = String::new();
                        let go = match caps.elo.filter(|_| caps.limit) {
                            Some((lo, hi)) if elo >= lo => {
                                cmd += &format!("setoption name UCI_LimitStrength value true\nsetoption name UCI_Elo value {}\n", elo.min(hi));
                                "go movetime 400".to_string()
                            }
                            Some(_) => {
                                cmd += "setoption name UCI_LimitStrength value false\n";
                                if caps.skill { cmd += "setoption name Skill Level value 0\n"; }
                                format!("go depth {}", 1 + (elo - 1000) / 110)
                            }
                            None if caps.skill => {
                                cmd += &format!("setoption name Skill Level value {}\n", ((elo - 1000) / 80).min(20));
                                "go movetime 400".to_string()
                            }
                            None => format!("go depth {}", (1 + (elo - 1000) / 100).min(16)),
                        };
                        let _ = writeln!(stdin, "{cmd}position fen {fen}\n{go}");
                        while let Some(Ok(l)) = lines.next() {
                            if let Some(rest) = l.strip_prefix("bestmove ") {
                                let uci = rest.split_whitespace().next().unwrap_or("").to_string();
                                let _ = out.send(Reply::Play { fen, uci });
                                ok = true;
                                break;
                            }
                        }
                    }
                    Job::Review { fens } => {
                        let _ = writeln!(stdin, "setoption name MultiPV value 2");
                        let mut evals = vec![];
                        ok = true;
                        for fen in &fens {
                            // depth 16, but at most 0.6 s a position (0.35 s for the rating of a single move, which is
                            // asked after every move): a long game is still reviewed in under a minute
                            let ms = if fens.len() <= 2 { 350 } else { 600 };
                            let _ = writeln!(stdin, "position fen {fen}\ngo depth 16 movetime {ms}");
                            let mut cur: [Option<(i32, String)>; 2] = [None, None];
                            let mut got = false;
                            while let Some(Ok(l)) = lines.next() {
                                if l.starts_with("bestmove") { got = true; break; }
                                if !l.starts_with("info") || !l.contains(" pv ") { continue; }
                                let t: Vec<&str> = l.split_whitespace().collect();
                                let get = |k: &str| t.iter().position(|x| *x == k);
                                let (Some(sc), Some(pv)) = (get("score"), get("pv")) else { continue };
                                let idx: usize = get("multipv").and_then(|m| t[m + 1].parse().ok()).unwrap_or(1);
                                if !(1..=2).contains(&idx) { continue; }
                                let v: i32 = t[sc + 2].parse().unwrap_or(0);
                                let cp = if t[sc + 1] == "mate" { if v > 0 { 1000 } else { -1000 } } else { v.clamp(-1000, 1000) };
                                cur[idx - 1] = Some((cp, t[pv + 1].to_string()));
                            }
                            if !got { ok = false; break; }
                            let [first, second] = cur;
                            evals.push(first.map(|(cp, mv)| (cp, mv, second.map(|s| s.0))));
                            if fens.len() > 2 { let _ = out.send(Reply::ReviewProgress(evals.len(), fens.len())); ctx.request_repaint(); }
                        }
                        if ok {
                            let _ = out.send(Reply::Review { fens, evals });
                        }
                    }
                    Job::Hint { fen, depth, children } => {
                        // A capped depth follows the hint Elo: short, understandable lines.
                        // Uncapped (depth 0): search until the next job, sending the lines at each finished depth.
                        let mut best: Vec<Line> = vec![];
                        let mut reached = depth;
                        ok = true;
                        if caps.multipv {
                            let go = if depth == 0 { "go infinite".to_string() } else { format!("go depth {depth}") };
                            let _ = writeln!(stdin, "setoption name MultiPV value 3\nposition fen {fen}\n{go}");
                            let mut cur: [Option<Line>; 3] = [None, None, None];
                            let (mut at, mut sent) = (0u32, 0u32); // depth of the lines in `cur`, depth last sent
                            let mut newer = None;
                            ok = false;
                            while let Some(Ok(l)) = lines.next() {
                                if l.starts_with("bestmove") { ok = true; break; }
                                if depth == 0 && newer.is_none() {
                                    if let Ok(j) = rx.try_recv() {
                                        newer = Some(j); // the user moved on: stop, and skip what is left of this one
                                        let _ = writeln!(stdin, "stop");
                                    }
                                }
                                if !l.starts_with("info") || !l.contains(" pv ") { continue; }
                                let t: Vec<&str> = l.split_whitespace().collect();
                                let get = |k: &str| t.iter().position(|x| *x == k);
                                let (Some(sc), Some(pv)) = (get("score"), get("pv")) else { continue };
                                let idx: usize = get("multipv").and_then(|m| t[m + 1].parse().ok()).unwrap_or(1);
                                if !(1..=3).contains(&idx) { continue; }
                                let d: u32 = get("depth").and_then(|i| t[i + 1].parse().ok()).unwrap_or(at);
                                // The first line of a new depth: the lines so far are a finished depth, send them.
                                if depth == 0 && newer.is_none() && idx == 1 && d > at && at > sent && at >= 8 {
                                    let _ = out.send(Reply::Hint { fen: fen.clone(), depth: at, lines: cur.iter().flatten().cloned().collect() });
                                    ctx.request_repaint();
                                    sent = at;
                                }
                                if idx == 1 { at = d; }
                                cur[idx - 1] = Some(Line {
                                    mate: t[sc + 1] == "mate",
                                    score: t[sc + 2].parse().unwrap_or(0),
                                    pv: t[pv + 1..].iter().take(6).map(|s| s.to_string()).collect(),
                                });
                            }
                            if let (true, Some(j)) = (ok, newer) {
                                job = j;
                                continue;
                            }
                            best = cur.into_iter().flatten().collect();
                            reached = at;
                        } else {
                            // No MultiPV (nor reliable `searchmoves`): score the position after every legal move
                            // with a shallower search, from the mover's side, and keep the best three.
                            let d = if depth == 0 { 10 } else { depth.saturating_sub(1).clamp(1, 10) };
                            reached = d + 1;
                            let mut scored: Vec<(i64, Line)> = vec![];
                            for (m, cfen, kind) in &children {
                                let line = match kind {
                                    1 => Line { mate: true, score: 1, pv: vec![m.clone()] },
                                    2 => Line { mate: false, score: 0, pv: vec![m.clone()] },
                                    _ => {
                                        let _ = writeln!(stdin, "position fen {cfen}\ngo depth {d}");
                                        let (mut cur, mut done) = (None, false);
                                        while let Some(Ok(l)) = lines.next() {
                                            if l.starts_with("bestmove") { done = true; break; }
                                            if !l.starts_with("info") || !l.contains(" pv ") { continue; }
                                            let t: Vec<&str> = l.split_whitespace().collect();
                                            let get = |k: &str| t.iter().position(|x| *x == k);
                                            let (Some(sc), Some(pv)) = (get("score"), get("pv")) else { continue };
                                            cur = Some((t[sc + 1] == "mate", t[sc + 2].parse::<i32>().unwrap_or(0), t[pv + 1..].iter().take(5).map(|s| s.to_string()).collect::<Vec<_>>()));
                                        }
                                        if !done { ok = false; break; }
                                        let Some((mate, v, rest)) = cur else { continue };
                                        let mut pv = vec![m.clone()];
                                        pv.extend(rest);
                                        Line { mate, score: -v, pv } // child score is for the opponent
                                    }
                                };
                                let key = if line.mate { if line.score > 0 { 100_000 - line.score as i64 } else { -100_000 - line.score as i64 } } else { line.score as i64 };
                                scored.push((key, line));
                            }
                            scored.sort_by(|a, b| b.0.cmp(&a.0));
                            best = scored.into_iter().take(3).map(|x| x.1).collect();
                        }
                        if ok {
                            let _ = out.send(Reply::Hint { fen, depth: reached, lines: best });
                        }
                    }
                    Job::Stop => ok = true, // whatever it stopped already ended above

                }
                if !ok {
                    let _ = out.send(Reply::Dead(label.clone()));
                    ctx.request_repaint();
                    let _ = child.kill();
                    return;
                }
                ctx.request_repaint();
                match rx.recv_timeout(IDLE) {
                    Ok(j) => job = j,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        // idle: give the memory back, start again on the next job
                        let _ = writeln!(stdin, "quit");
                        let _ = child.wait();
                        match rx.recv() { Ok(j) => { job = j; continue 'run; } Err(_) => return }
                    }
                    Err(_) => { let _ = child.kill(); return; }
                }
            }
            }
        });
        Engine { tx }
    }

    pub fn send(&self, j: Job) {
        let _ = self.tx.send(j);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fool's mate: 1.f3 e5 2.g4?? Qh4#. Review must see mate-in-1 after 2.g4 and no moves after the mate.
    #[test]
    fn review_fools_mate() {
        let (tx, rx) = channel();
        let e = Engine::new(&path(&known()[0]), tx, eframe::egui::Context::default());
        let fens = [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "rnbqkbnr/pppppppp/8/8/8/5P2/PPPPP1PP/RNBQKBNR b KQkq - 0 1",
            "rnbqkbnr/pppp1ppp/8/4p3/8/5P2/PPPPP1PP/RNBQKBNR w KQkq - 0 2",
            "rnbqkbnr/pppp1ppp/8/4p3/6P1/5P2/PPPPP2P/RNBQKBNR b KQkq - 0 2",
            "rnb1kbnr/pppp1ppp/8/4p3/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - 1 3",
        ];
        e.send(Job::Review { fens: fens.iter().map(|s| s.to_string()).collect() });
        let mut progress = 0;
        loop {
            match rx.recv_timeout(std::time::Duration::from_secs(60)).unwrap() {
                Reply::ReviewProgress(done, total) => { assert_eq!((done, total), (progress + 1, 5)); progress = done; }
                Reply::Review { evals, .. } => {
                    assert_eq!(progress, 5); // one progress step per position, before the result
                    assert_eq!(evals.len(), 5);
                    let e = evals[3].as_ref().unwrap();
                    assert_eq!((e.0, e.1.as_str()), (1000, "d8h4"));
                    assert!(evals[4].is_none());
                    break;
                }
                _ => panic!(),
            }
        }
    }

    // An uncapped hint keeps sending deeper lines until stopped, then the engine takes the next job.
    #[test]
    fn uncapped_hint_deepens_until_stopped() {
        let (tx, rx) = channel();
        let e = Engine::new(&path(&known()[0]), tx, eframe::egui::Context::default());
        let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        let children = crate::children(&shakmaty::Chess::default());
        e.send(Job::Hint { fen: start.into(), depth: 0, children: children.clone() });
        let mut depths = vec![];
        while depths.len() < 3 {
            match rx.recv_timeout(std::time::Duration::from_secs(60)).unwrap() {
                Reply::Hint { depth, lines, .. } => { assert_eq!(lines.len(), 3); depths.push(depth); }
                _ => panic!(),
            }
        }
        assert!(depths.windows(2).all(|w| w[1] > w[0]), "depths should grow: {depths:?}");
        e.send(Job::Stop);
        e.send(Job::Hint { fen: start.into(), depth: 4, children });
        // Replies of the stopped search may still be queued; the capped one must arrive.
        loop {
            match rx.recv_timeout(std::time::Duration::from_secs(30)).unwrap() {
                Reply::Hint { depth: 4, lines, .. } => { assert_eq!(lines.len(), 3); break; }
                Reply::Hint { .. } => {}
                _ => panic!(),
            }
        }
    }

    // A missing executable must report Dead instead of hanging the game.
    #[test]
    fn missing_engine_reports_dead() {
        let (tx, rx) = channel();
        let e = Engine::new(Path::new("C:/definitely/not/here.exe"), tx, eframe::egui::Context::default());
        e.send(Job::Play { fen: "8/8/8/8/8/8/8/K6k w - - 0 1".into(), elo: 1500 }); // the process only starts with a job
        assert!(matches!(rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(), Reply::Dead(_)));
    }
}

#[cfg(test)]
mod bundled {
    use super::*;

    // Every bundled engine must unpack, start, and answer a move at 1500 Elo and a 3-line hint.
    #[test]
    fn every_bundled_engine_plays_and_hints() {
        let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        for k in known() {
            let (tx, rx) = channel();
            let e = Engine::new(&path(&k), tx, eframe::egui::Context::default());
            e.send(Job::Play { fen: start.into(), elo: 1500 });
            match rx.recv_timeout(std::time::Duration::from_secs(60)) {
                Ok(Reply::Play { uci, .. }) => assert!(uci.len() >= 4, "{}: bad move {uci:?}", k.name),
                _ => panic!("{} did not answer a Play job", k.name),
            }
            e.send(Job::Hint { fen: start.into(), depth: 6, children: crate::children(&shakmaty::Chess::default()) });
            match rx.recv_timeout(std::time::Duration::from_secs(60)) {
                Ok(Reply::Hint { lines, .. }) => { println!("{}: {} hint line(s)", k.name, lines.len()); assert_eq!(lines.len(), 3, "{} should give 3 hints", k.name); }
                _ => panic!("{} did not answer a Hint job", k.name),
            }
        }
    }
}
