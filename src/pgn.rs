//! Small PGN reader: headers + main-line moves. Comments `{...}` and `;...`, variations `(...)`, NAGs `$n`,
//! move numbers and `!?` marks are dropped, so chess.com / lichess exports (with clock comments) read fine.
use crate::i18n::tl;
use shakmaty::{fen::Fen, san::San, CastlingMode, Chess, Move, Position};

#[derive(Clone, Default)]
pub struct Game {
    pub headers: Vec<(String, String)>,
    pub moves: Vec<String>, // SAN tokens, in order
}

impl Game {
    pub fn header(&self, key: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str())
    }

    /// Plays the moves from the start position (the `FEN` header, if any). Returns the start position and the moves.
    pub fn replay(&self) -> Result<(Chess, Vec<Move>), String> {
        let start: Chess = match self.header("FEN") {
            Some(f) => f.parse::<Fen>().map_err(|e| tr!("FEN inválido: {}", e))?
                .into_position(CastlingMode::Standard).map_err(|e| tr!("posição inicial inválida: {}", e))?,
            None => Chess::default(),
        };
        let (mut pos, mut out) = (start.clone(), vec![]);
        for (i, t) in self.moves.iter().enumerate() {
            let m = San::from_ascii(t.as_bytes()).ok().and_then(|s| s.to_move(&pos).ok())
                .ok_or_else(|| tr!("lance ilegal ou ilegível no lance {} ({})", i / 2 + 1, t))?;
            pos = pos.play(m.clone()).map_err(|_| tl("lance ilegal").to_string())?;
            out.push(m);
        }
        Ok((start, out))
    }
}

fn header_line(l: &str) -> Option<(String, String)> {
    let inner = l.strip_prefix('[')?.strip_suffix(']')?;
    let sp = inner.find(' ')?;
    let (k, rest) = inner.split_at(sp);
    let (a, b) = (rest.find('"')?, rest.rfind('"')?);
    (b > a).then(|| (k.to_string(), rest[a + 1..b].to_string()))
}

/// SAN tokens from cleaned move text.
fn tokens(text: &str) -> Vec<String> {
    let mut out = vec![];
    for raw in text.split_whitespace() {
        if raw.starts_with('$') || matches!(raw, "1-0" | "0-1" | "1/2-1/2" | "*" | "½-½") { continue; }
        // "12." / "12..." / "12.e4": drop the move number
        let mut t = raw;
        if let Some(dot) = raw.find('.') {
            if dot > 0 && raw[..dot].chars().all(|c| c.is_ascii_digit()) { t = raw[dot..].trim_start_matches('.'); }
        }
        let t = t.trim_end_matches(['!', '?']);
        if t.is_empty() { continue; }
        out.push(match t { "0-0" => "O-O".to_string(), "0-0-0" => "O-O-O".to_string(), _ => t.to_string() });
    }
    out
}

/// Every game in a PGN text (one or many).
pub fn parse_games(text: &str) -> Vec<Game> {
    let (mut games, mut cur, mut movetext) = (vec![], Game::default(), String::new());
    let (mut in_moves, mut in_brace, mut depth) = (false, false, 0usize);
    let mut finish = |games: &mut Vec<Game>, cur: &mut Game, movetext: &mut String| {
        cur.moves = tokens(movetext);
        movetext.clear();
        let g = std::mem::take(cur);
        if !g.headers.is_empty() || !g.moves.is_empty() { games.push(g); }
    };
    for line in text.lines() {
        let l = line.trim_start_matches('\u{feff}').trim();
        if !in_brace && depth == 0 && l.starts_with('[') && l.ends_with(']') {
            if in_moves { finish(&mut games, &mut cur, &mut movetext); in_moves = false; }
            cur.headers.extend(header_line(l));
            continue;
        }
        if l.is_empty() { continue; }
        for ch in l.chars() {
            if in_brace { in_brace = ch != '}'; continue; }
            match ch {
                '{' => in_brace = true,
                ';' => break,
                '(' => depth += 1,
                ')' => depth = depth.saturating_sub(1),
                c if depth == 0 => movetext.push(c),
                _ => {}
            }
        }
        movetext.push(' ');
        in_moves = true;
    }
    finish(&mut games, &mut cur, &mut movetext);
    games
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHESSCOM: &str = "[Event \"Live Chess\"]\n[White \"alice\"]\n[Black \"bob\"]\n[Result \"1-0\"]\n[WhiteElo \"1200\"]\n\n1. e4 {[%clk 0:09:58.3]} 1... e5 {[%clk 0:09:57.1]} 2. Bc4 {[%clk 0:09:50]} 2... Nc6 3. Qh5 Nf6?? 4. Qxf7# {Black is checkmated} 1-0\n";

    #[test]
    fn chesscom_style_game() {
        let g = parse_games(CHESSCOM);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].header("white"), Some("alice"));
        assert_eq!(g[0].moves, ["e4", "e5", "Bc4", "Nc6", "Qh5", "Nf6", "Qxf7#"]);
        let (_, mv) = g[0].replay().unwrap();
        assert_eq!(mv.len(), 7);
    }

    #[test]
    fn variations_comments_nags_and_castling() {
        let t = "[Event \"x\"]\n\n1. e4 e5 (1... c5 2. Nf3 {sicilian}) 2. Nf3 $1 Nc6 3. Bb5 a6 ; rest of line\n4. Ba4 Nf6 5. 0-0 Be7 *";
        let g = parse_games(t);
        assert_eq!(g[0].moves, ["e4", "e5", "Nf3", "Nc6", "Bb5", "a6", "Ba4", "Nf6", "O-O", "Be7"]);
        assert!(g[0].replay().is_ok());
    }

    #[test]
    fn several_games_and_fen_start() {
        let t = format!("{CHESSCOM}\n\n[Event \"two\"]\n[SetUp \"1\"]\n[FEN \"6k1/5ppp/8/8/8/8/8/R3K3 w - - 0 1\"]\n\n1. Ra8# 1-0\n");
        let g = parse_games(&t);
        assert_eq!(g.len(), 2);
        let (start, mv) = g[1].replay().unwrap();
        assert_eq!(mv.len(), 1);
        assert!(start.clone().play(mv[0].clone()).unwrap().is_checkmate());
    }

    #[test]
    fn illegal_move_is_reported() {
        let g = parse_games("1. e4 e5 2. Ke5");
        assert!(g[0].replay().unwrap_err().contains("ilegal"));
    }
}
