//! Opening names (ECO) from the lichess chess-openings database (CC0), matched by position,
//! so transpositions are recognised too.
use shakmaty::{fen::Fen, san::San, Chess, EnPassantMode, Position};
use std::collections::HashMap;

/// Position key: first 4 FEN fields (board, turn, castling, en passant).
pub fn key(pos: &Chess) -> String {
    let f = Fen::from_position(pos, EnPassantMode::Legal).to_string();
    f.split(' ').take(4).collect::<Vec<_>>().join(" ")
}

pub struct Book {
    names: HashMap<String, (String, String)>,      // position -> (eco, name)
    next: HashMap<String, Vec<(String, String)>>,  // position -> [(move that completes a named line, name)]
}

impl Book {
    pub fn new() -> Self {
        let (mut names, mut next): (HashMap<_, _>, HashMap<String, Vec<(String, String)>>) = Default::default();
        for file in [
            include_str!("../assets/openings/a.tsv"),
            include_str!("../assets/openings/b.tsv"),
            include_str!("../assets/openings/c.tsv"),
            include_str!("../assets/openings/d.tsv"),
            include_str!("../assets/openings/e.tsv"),
        ] {
            for line in file.lines().skip(1) {
                let mut it = line.split('\t');
                let (Some(eco), Some(name), Some(pgn)) = (it.next(), it.next(), it.next()) else { continue };
                let mut pos = Chess::default();
                let (mut before, mut last, mut ok) = (String::new(), String::new(), true);
                for tok in pgn.split_whitespace() {
                    let t = tok.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.');
                    if t.is_empty() { continue; }
                    let Some(m) = San::from_ascii(t.as_bytes()).ok().and_then(|s| s.to_move(&pos).ok()) else { ok = false; break };
                    before = key(&pos);
                    last = t.to_string();
                    pos = pos.play(m).unwrap();
                }
                if !ok || last.is_empty() { continue; }
                names.entry(key(&pos)).or_insert((eco.to_string(), name.to_string()));
                next.entry(before).or_default().push((last, name.to_string()));
            }
        }
        Book { names, next }
    }

    pub fn name(&self, k: &str) -> Option<&(String, String)> {
        self.names.get(k)
    }

    /// Book moves from this position, each with the shortest (most general) opening name it leads to.
    pub fn continuations(&self, k: &str) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = vec![];
        for (san, name) in self.next.get(k).map(|v| v.as_slice()).unwrap_or(&[]) {
            match out.iter_mut().find(|(s, _)| s == san) {
                Some(e) => if name.len() < e.1.len() { e.1 = name.clone() },
                None => out.push((san.clone(), name.clone())),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn play(moves: &str) -> Chess {
        let mut p = Chess::default();
        for t in moves.split_whitespace() {
            let m = San::from_ascii(t.as_bytes()).unwrap().to_move(&p).unwrap();
            p = p.play(m).unwrap();
        }
        p
    }

    #[test]
    fn names() {
        let b = Book::new();
        assert_eq!(b.name(&key(&play("d4 d5 c4"))).unwrap().1, "Queen's Gambit");
        // transposition: same position reached through another move order
        assert_eq!(b.name(&key(&play("c4 d5 d4"))).unwrap().1, "Queen's Gambit");
        assert!(b.name(&key(&play("e4 e5 Nf3 Nc6 Bb5"))).unwrap().1.contains("Ruy Lopez"));
        assert!(!b.continuations(&key(&play("e4"))).is_empty());
    }
}
