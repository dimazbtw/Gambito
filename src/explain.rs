//! Plain-language reasons for a move: "O bispo dá xeque e ameaça a dama."
//! Built from simple board facts (checks, captures, new threats, discovered attacks, safety), not from engine lines.
use shakmaty::{san::SanPlus, Board, Chess, Color, Move, Position, Role, Square};

pub fn role_name(r: Role) -> &'static str {
    match r { Role::Pawn => "peão", Role::Knight => "cavalo", Role::Bishop => "bispo", Role::Rook => "torre", Role::Queen => "dama", Role::King => "rei" }
}
pub fn val(r: Role) -> u32 {
    match r { Role::Pawn => 1, Role::Knight | Role::Bishop => 3, Role::Rook => 5, Role::Queen => 9, Role::King => 100 }
}
fn fem(r: Role) -> bool { matches!(r, Role::Rook | Role::Queen) }
/// "o bispo" / "a dama"
fn the(r: Role) -> String { format!("{} {}", if fem(r) { "a" } else { "o" }, role_name(r)) }
/// "do bispo" / "da dama"
fn of(r: Role) -> String { format!("{} {}", if fem(r) { "da" } else { "do" }, role_name(r)) }
fn cap(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}
/// "a, b e c"
fn join(v: &[String]) -> String {
    match v.len() {
        0 => String::new(),
        1 => v[0].clone(),
        n => format!("{} e {}", v[..n - 1].join(", "), v[n - 1]),
    }
}

/// Non-king material of one side, in pawns.
pub fn material(b: &Board, c: Color) -> i32 {
    (0..64).filter_map(|i| b.piece_at(Square::new(i))).filter(|p| p.color == c && p.role != Role::King).map(|p| val(p.role) as i32).sum()
}

/// (attacker square, target square, target role): `c`'s pieces attacking an enemy piece worth winning
/// (more valuable than the attacker, or undefended). Pawns and kings are not listed as targets.
fn threats(b: &Board, c: Color) -> Vec<(Square, Square, Role)> {
    let occ = b.occupied();
    let mut v = vec![];
    for i in 0..64 {
        let a = Square::new(i);
        let Some(pa) = b.piece_at(a).filter(|p| p.color == c) else { continue };
        for t in b.attacks_from(a) {
            let Some(pt) = b.piece_at(t) else { continue };
            if pt.color == c || matches!(pt.role, Role::King | Role::Pawn) { continue; }
            if val(pt.role) > val(pa.role) || b.attacks_to(t, !c, occ).is_empty() {
                v.push((a, t, pt.role));
            }
        }
    }
    v
}

/// `c`'s non-pawn, non-king pieces that the enemy can win (attacked by something cheaper, or undefended).
fn hanging(b: &Board, c: Color) -> Vec<(Square, Role)> {
    let occ = b.occupied();
    let mut v = vec![];
    for i in 0..64 {
        let s = Square::new(i);
        let Some(p) = b.piece_at(s).filter(|p| p.color == c && !matches!(p.role, Role::King | Role::Pawn)) else { continue };
        let attackers = b.attacks_to(s, !c, occ);
        let Some(cheapest) = attackers.into_iter().filter_map(|a| b.piece_at(a)).map(|p| val(p.role)).min() else { continue };
        if val(p.role) > cheapest || b.attacks_to(s, c, occ).is_empty() {
            v.push((s, p.role));
        }
    }
    v
}

/// Returns (SAN, explanation, position after the move).
pub fn explain(pos: &Chess, mv: &Move) -> (String, String, Chess) {
    let mover = pos.turn();
    let san = SanPlus::from_move(pos.clone(), mv.clone()).to_string();
    let after = pos.clone().play(mv.clone()).unwrap();
    let (to, piece) = (crate::dest(mv), mv.role());
    let (before_b, after_b) = (pos.board(), after.board());

    let mut acts: Vec<String> = vec![]; // what the moving piece does
    let mut notes: Vec<String> = vec![]; // extra sentences
    let mut positional: Vec<String> = vec![];

    if after.is_checkmate() {
        acts.push("dá xeque-mate".into());
    } else {
        if after.is_check() { acts.push("dá xeque".into()); }
        if mv.is_castle() { acts.push("põe o rei em segurança e ativa a torre".into()); }
        if let Some(c) = mv.capture() {
            let safe = after_b.attacks_to(to, !mover, after_b.occupied()).is_empty();
            acts.push(format!("captura {}{}", the(c), if safe && val(c) > 1 { " sem risco" } else { "" }));
        }
        if mv.promotion().is_some() { acts.push("promove-se a dama".into()); }

        // new threats by the moved piece
        let mine: Vec<Role> = threats(after_b, mover).into_iter().filter(|t| t.0 == to).map(|t| t.2).collect();
        let mut uniq: Vec<Role> = vec![];
        for r in mine { if !uniq.contains(&r) { uniq.push(r); } }
        if uniq.len() >= 2 {
            acts.push(format!("ataca {} ao mesmo tempo (um garfo)", join(&uniq.iter().map(|r| the(*r)).collect::<Vec<_>>())));
        } else if let Some(r) = uniq.first() {
            // not worth a mention when the piece just captured it
            if mv.capture() != Some(*r) { acts.push(format!("ameaça {}", the(*r))); }
        }

        // discovered attacks: new threats by other pieces
        let before_t = threats(before_b, mover);
        for (a, t, tr) in threats(after_b, mover).into_iter().filter(|x| x.0 != to) {
            if !before_t.iter().any(|b| b.0 == a && b.1 == t) {
                if let Some(ap) = after_b.piece_at(a) {
                    notes.push(format!("Descobre um ataque {} sobre {}.", of(ap.role), the(tr)));
                    break;
                }
            }
        }

        // safety
        let (hb, ha) = (hanging(before_b, mover), hanging(after_b, mover));
        if let Some(from) = mv.from() {
            if hb.iter().any(|h| h.0 == from) && !ha.iter().any(|h| h.0 == to) { acts.push("tira-se de um ataque".into()); }
        }
        for (s, r) in &hb {
            if Some(*s) != mv.from() && !ha.iter().any(|h| h.0 == *s) {
                notes.push(format!("Protege {} que estava ameaçad{}.", the(*r), if fem(*r) { "a" } else { "o" }));
                break;
            }
        }
        if crate::is_sacrifice(pos, mv) {
            notes.push("É um sacrifício de material que compensa pela posição que fica.".into());
        } else if mv.capture().is_none() && ha.iter().any(|h| h.0 == to) {
            notes.push(format!("Atenção: {} fica exposto a um ataque.", the(piece)));
        }

        if matches!(piece, Role::Knight | Role::Bishop) {
            let home = if mover == Color::White { shakmaty::Rank::First } else { shakmaty::Rank::Eighth };
            if mv.from().map_or(false, |f| f.rank() == home) { positional.push(format!("desenvolve {}", the(piece))); }
        }
        if [Square::D4, Square::E4, Square::D5, Square::E5].contains(&to) { positional.push("ocupa o centro".into()); }
    }

    let subject = cap(&the(piece));
    let mut sentences: Vec<String> = vec![];
    if !acts.is_empty() {
        sentences.push(format!("{} {}.", subject, join(&acts)));
        if !positional.is_empty() { sentences.push(format!("Também {}.", join(&positional))); }
    } else if !positional.is_empty() {
        sentences.push(format!("{}.", cap(&join(&positional))));
    } else if notes.is_empty() {
        sentences.push(format!("{} melhora a posição das peças.", subject));
    }
    sentences.extend(notes);
    (san, sentences.join(" "), after)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shakmaty::{fen::Fen, san::San, CastlingMode};

    fn text(fen: &str, san: &str) -> String {
        let pos: Chess = fen.parse::<Fen>().unwrap().into_position(CastlingMode::Standard).unwrap();
        let mv = San::from_ascii(san.as_bytes()).unwrap().to_move(&pos).unwrap();
        explain(&pos, &mv).1
    }

    #[test]
    fn mate_and_fork() {
        assert!(text("6k1/5ppp/8/8/8/8/8/R3K3 w - - 0 1", "Ra8#").contains("xeque-mate"));
        // Nc7+ forks the king and the rook on a8
        let t = text("r3k3/8/8/3N4/8/8/8/4K3 w - - 0 1", "Nc7+");
        assert!(t.contains("xeque") && t.contains("a torre"), "{t}");
    }

    #[test]
    fn bishop_checks_and_hits_queen() {
        // Bb5+ checks the king on e8 while attacking the undefended queen on d7... use a clean case:
        // white bishop to b5 gives check (king e8) and attacks the queen on a6 (undefended).
        let t = text("4k3/8/q7/8/8/8/8/4KB2 w - - 0 1", "Bb5+");
        assert!(t.contains("O bispo") && t.contains("dá xeque") && t.contains("a dama"), "{t}");
    }

    #[test]
    fn opening_move_is_plain() {
        let t = text("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1", "Nf3");
        assert!(t.contains("esenvolve o cavalo"), "{t}");
    }
}
