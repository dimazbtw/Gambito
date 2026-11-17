//! Games of a chess.com user, through chess.com's public "Published-Data" API (read-only, no login).
//! Requests are sequential and spaced out, in a background thread; nothing is sent besides the user name in the URL.
use serde_json::Value;
use std::sync::mpsc::Sender;
use std::time::Duration;

#[derive(Clone)]
pub struct CcGame {
    pub pgn: String,
    pub white: String,
    pub black: String,
    pub white_elo: Option<u32>,
    pub black_elo: Option<u32>,
    pub white_result: String, // chess.com's words: win, resigned, checkmated, timeout, agreed, ...
    pub black_result: String,
    pub time_class: String, // bullet / blitz / rapid / daily
    pub end_time: u64,      // unix seconds
    pub acc: Option<(f64, f64)>, // chess.com's own accuracy, when the game was reviewed there
}

pub enum Msg {
    /// `loaded` = how many months (counting back from the newest) have been fetched so far
    Games { user: String, archives: Vec<String>, loaded: usize, games: Vec<CcGame> },
    Err(String),
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).build().into()
}

fn get(agent: &ureq::Agent, url: &str) -> Result<Value, String> {
    let mut resp = agent.get(url).header("User-Agent", "Gambito/1.0 (chess trainer)").call().map_err(|e| match e {
        ureq::Error::StatusCode(404) => "Utilizador não encontrado no chess.com.".to_string(),
        ureq::Error::StatusCode(429) => "O chess.com pediu para esperar. Tenta daqui a um minuto.".to_string(),
        ureq::Error::StatusCode(c) => format!("O chess.com respondeu com o erro {c}."),
        other => format!("Sem ligação ao chess.com ({other})."),
    })?;
    let text = resp.body_mut().read_to_string().map_err(|e| format!("Resposta inválida: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("Resposta inválida: {e}"))
}

fn parse_game(g: &Value) -> Option<CcGame> {
    if g["rules"].as_str().unwrap_or("chess") != "chess" { return None; } // no Chess960, bughouse, ...
    let side = |c: &str, k: &str| g[c][k].as_str().unwrap_or("").to_string();
    Some(CcGame {
        pgn: g["pgn"].as_str()?.to_string(),
        white: side("white", "username"),
        black: side("black", "username"),
        white_elo: g["white"]["rating"].as_u64().map(|v| v as u32),
        black_elo: g["black"]["rating"].as_u64().map(|v| v as u32),
        white_result: side("white", "result"),
        black_result: side("black", "result"),
        time_class: g["time_class"].as_str().unwrap_or("").to_string(),
        end_time: g["end_time"].as_u64().unwrap_or(0),
        acc: g["accuracies"]["white"].as_f64().zip(g["accuracies"]["black"].as_f64()),
    })
}

/// Only letters, digits, `_` and `-` are valid in chess.com user names.
pub fn clean_user(s: &str) -> String {
    s.trim().to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').collect()
}

/// Fetches `months` more months of games (newest first), starting after the `already` ones.
/// `archives` is empty on the first call; it is then downloaded.
pub fn fetch(user: String, mut archives: Vec<String>, already: usize, months: usize, tx: Sender<Msg>, ctx: eframe::egui::Context) {
    std::thread::spawn(move || {
        let mut run = || -> Result<Msg, String> {
            let agent = agent();
            if archives.is_empty() {
                let v = get(&agent, &format!("https://api.chess.com/pub/player/{user}/games/archives"))?;
                archives = v["archives"].as_array().map(|a| a.iter().filter_map(|u| u.as_str().map(str::to_string)).collect()).unwrap_or_default();
                if archives.is_empty() { return Err("Este utilizador não tem partidas públicas.".into()); }
            }
            let mut games = vec![];
            let mut loaded = already;
            for i in already..(already + months).min(archives.len()) {
                if i > already { std::thread::sleep(Duration::from_millis(200)); } // be polite: one request at a time
                let v = get(&agent, &archives[archives.len() - 1 - i])?;
                games.extend(v["games"].as_array().map(|a| a.iter().filter_map(parse_game).collect::<Vec<_>>()).unwrap_or_default());
                loaded = i + 1;
            }
            games.sort_by(|a, b| b.end_time.cmp(&a.end_time));
            Ok(Msg::Games { user: user.clone(), archives: archives.clone(), loaded, games })
        };
        let _ = tx.send(run().unwrap_or_else(Msg::Err));
        ctx.request_repaint();
    });
}

/// "2026-11-17" from unix seconds (UTC), without a date library.
pub fn date(unix: u64) -> String {
    let z = (unix / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    format!("{y:04}-{m:02}-{d:02}")
}

/// How a game ended, from the loser's / drawer's result word.
pub fn termination(g: &CcGame) -> &'static str {
    let w = if g.white_result == "win" { &g.black_result } else { &g.white_result };
    match w.as_str() {
        "checkmated" => "xeque-mate",
        "resigned" => "desistência",
        "timeout" => "tempo",
        "abandoned" => "abandono",
        "agreed" => "empate por acordo",
        "repetition" => "repetição",
        "stalemate" => "afogamento",
        "insufficient" | "timevsinsufficient" => "material insuficiente",
        "50move" => "regra dos 50 lances",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(1_700_000_000), "2023-11-14");
        assert_eq!(date(1_709_164_800), "2024-02-29");
    }

    #[test]
    fn user_names_are_sanitised() {
        assert_eq!(clean_user("  Hikaru/../x "), "hikarux");
        assert_eq!(clean_user("Magnus_C-1"), "magnus_c-1");
    }

    #[test]
    fn parses_an_archive_entry() {
        let v: Value = serde_json::from_str(r#"{"rules":"chess","pgn":"1. e4 e5 1-0","time_class":"blitz","end_time":1700000000,
            "white":{"username":"A","rating":1500,"result":"win"},"black":{"username":"B","rating":1480,"result":"resigned"},
            "accuracies":{"white":91.5,"black":80.2}}"#).unwrap();
        let g = parse_game(&v).unwrap();
        assert_eq!((g.white.as_str(), g.white_elo, g.acc), ("A", Some(1500), Some((91.5, 80.2))));
        assert_eq!(termination(&g), "desistência");
        let bughouse: Value = serde_json::from_str(r#"{"rules":"bughouse","pgn":"x"}"#).unwrap();
        assert!(parse_game(&bughouse).is_none());
    }
}

#[cfg(test)]
mod live {
    use super::*;

    // Hits the real API once (a famous public account). Run with:  cargo test live_api -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_api() {
        let (tx, rx) = std::sync::mpsc::channel();
        fetch("hikaru".into(), vec![], 0, 1, tx, eframe::egui::Context::default());
        match rx.recv_timeout(Duration::from_secs(90)).unwrap() {
            Msg::Games { archives, loaded, games, .. } => {
                assert!(!archives.is_empty() && loaded == 1 && !games.is_empty());
                let mut ok = 0;
                for g in games.iter().take(25) {
                    let parsed = crate::pgn::parse_games(&g.pgn);
                    if parsed.first().map_or(false, |p| p.replay().is_ok()) { ok += 1; }
                }
                println!("{} months in archive, {} games fetched, {ok}/25 replayed; first: {} vs {} on {} ({})",
                    archives.len(), games.len(), games[0].white, games[0].black, date(games[0].end_time), games[0].time_class);
                assert_eq!(ok, games.len().min(25), "some chess.com PGNs failed to replay");
            }
            Msg::Err(e) => panic!("{e}"),
        }
    }
}
