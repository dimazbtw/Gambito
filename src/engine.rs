use std::io::{BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, Sender};

/// An engine shipped inside the .exe, gzip-compressed (unpacked to %TEMP% the first time it is used).
pub struct Known {
    pub name: &'static str,
    pub file: &'static str,
    pub gz: &'static [u8],
}

macro_rules! engines {
    ($($name:literal, $file:literal, $path:literal);* $(;)?) => {
        vec![$(Known { name: $name, file: $file, gz: include_bytes!($path) }),*]
    };
}

/// To ship another engine: `gzip` its .exe into assets/engines/ and add a line here (the first one is the default).
pub fn known() -> Vec<Known> {
    engines![
        "Stockfish 19", "stockfish", "../assets/stockfish.gz";
        "Berserk 14", "berserk", "../assets/engines/berserk.gz";
        "Obsidian 16", "obsidian", "../assets/engines/obsidian.gz";
        "Koivisto 9", "koivisto", "../assets/engines/koivisto.gz";
        "Alexandria 9.1", "alexandria", "../assets/engines/alexandria.gz";
        "Caissa 2.0", "caissa", "../assets/engines/caissa.gz";
        "Viridithas 20", "viridithas", "../assets/engines/viridithas.gz";
    ]
}

pub fn extract(k: &Known) -> PathBuf {
    let p = std::env::temp_dir().join(format!("xadrez-{}-{}.exe", k.file, k.gz.len()));
    if !p.exists() {
        // unpack to a temp name first so an interrupted run never leaves a half-written .exe behind
        let tmp = p.with_extension("part");
        let ok = std::fs::File::create(&tmp)
            .and_then(|mut f| std::io::copy(&mut flate2::read::GzDecoder::new(k.gz), &mut f))
            .is_ok();
        if ok { let _ = std::fs::rename(&tmp, &p); } else { let _ = std::fs::remove_file(&tmp); }
    }
    p
}

pub enum Job {
    Play { fen: String, elo: u32 },
    Hint { fen: String, depth: u32, children: Vec<(String, String, u8)> }, // every legal move: (uci, fen after it, 0 normal / 1 mate / 2 other game end)
    Review { fens: Vec<String> },
}

pub struct Line {
    pub mate: bool,
    pub score: i32,
    pub pv: Vec<String>,
}

pub enum Reply {
    Play { fen: String, uci: String },
    Hint { fen: String, lines: Vec<Line> },
    /// per position: see `Ev`; None if the position has no moves
    Review { fens: Vec<String>, evals: Vec<Option<Ev>> },
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
}

pub struct Engine {
    tx: Sender<Job>,
}

impl Engine {
    pub fn new(exe: &Path, out: Sender<Reply>, ctx: eframe::egui::Context) -> Self {
        let (tx, rx) = channel::<Job>();
        let exe = exe.to_path_buf();
        std::thread::spawn(move || {
            let label = exe.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let Ok(mut child) = Command::new(&exe)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
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
                        _ => {}
                    }
                }
            }
            while let Ok(mut job) = rx.recv() {
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
                            let _ = writeln!(stdin, "position fen {fen}\ngo depth 12");
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
                        }
                        if ok {
                            let _ = out.send(Reply::Review { fens, evals });
                        }
                    }
                    Job::Hint { fen, depth, children } => {
                        // Shallow on purpose (depth follows the hint Elo): short, understandable lines.
                        let mut best: Vec<Line> = vec![];
                        ok = true;
                        if caps.multipv {
                            let _ = writeln!(stdin, "setoption name MultiPV value 3\nposition fen {fen}\ngo depth {depth}");
                            let mut cur: [Option<Line>; 3] = [None, None, None];
                            ok = false;
                            while let Some(Ok(l)) = lines.next() {
                                if l.starts_with("bestmove") { ok = true; break; }
                                if !l.starts_with("info") || !l.contains(" pv ") { continue; }
                                let t: Vec<&str> = l.split_whitespace().collect();
                                let get = |k: &str| t.iter().position(|x| *x == k);
                                let (Some(sc), Some(pv)) = (get("score"), get("pv")) else { continue };
                                let idx: usize = get("multipv").and_then(|m| t[m + 1].parse().ok()).unwrap_or(1);
                                if !(1..=3).contains(&idx) { continue; }
                                cur[idx - 1] = Some(Line {
                                    mate: t[sc + 1] == "mate",
                                    score: t[sc + 2].parse().unwrap_or(0),
                                    pv: t[pv + 1..].iter().take(3).map(|s| s.to_string()).collect(),
                                });
                            }
                            best = cur.into_iter().flatten().collect();
                        } else {
                            // No MultiPV (nor reliable `searchmoves`): score the position after every legal move
                            // with a shallower search, from the mover's side, and keep the best three.
                            let d = depth.saturating_sub(1).clamp(1, 10);
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
                                            cur = Some((t[sc + 1] == "mate", t[sc + 2].parse::<i32>().unwrap_or(0), t[pv + 1..].iter().take(2).map(|s| s.to_string()).collect::<Vec<_>>()));
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
                            let _ = out.send(Reply::Hint { fen, lines: best });
                        }
                    }
                }
                if !ok {
                    let _ = out.send(Reply::Dead(label.clone()));
                    ctx.request_repaint();
                    break;
                }
                ctx.request_repaint();
            }
            let _ = child.kill();
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
        let e = Engine::new(&extract(&known()[0]), tx, eframe::egui::Context::default());
        let fens = [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "rnbqkbnr/pppppppp/8/8/8/5P2/PPPPP1PP/RNBQKBNR b KQkq - 0 1",
            "rnbqkbnr/pppp1ppp/8/4p3/8/5P2/PPPPP1PP/RNBQKBNR w KQkq - 0 2",
            "rnbqkbnr/pppp1ppp/8/4p3/6P1/5P2/PPPPP2P/RNBQKBNR b KQkq - 0 2",
            "rnb1kbnr/pppp1ppp/8/4p3/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - 1 3",
        ];
        e.send(Job::Review { fens: fens.iter().map(|s| s.to_string()).collect() });
        match rx.recv_timeout(std::time::Duration::from_secs(60)).unwrap() {
            Reply::Review { evals, .. } => {
                assert_eq!(evals.len(), 5);
                let e = evals[3].as_ref().unwrap();
                assert_eq!((e.0, e.1.as_str()), (1000, "d8h4"));
                assert!(evals[4].is_none());
            }
            _ => panic!(),
        }
    }

    // A missing executable must report Dead instead of hanging the game.
    #[test]
    fn missing_engine_reports_dead() {
        let (tx, rx) = channel();
        let _e = Engine::new(Path::new("C:/definitely/not/here.exe"), tx, eframe::egui::Context::default());
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
            let e = Engine::new(&extract(&k), tx, eframe::egui::Context::default());
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
