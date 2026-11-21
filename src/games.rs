//! Mini-games for board vision and memory (side menu "Jogos"). Each run is a small state machine driven by
//! the clock (`App::time`) and by clicks; the best score of each game is kept in progress.json.
//! Positions come from the bundled Lichess puzzles; "Peça pendurada" has the engine play itself.
use crate::screens::{GOLD_LINE, TEXT2};
use crate::train::{Puzzle, Rng};
use crate::*;
use shakmaty::{attacks, Bitboard, Board, File, Piece, Rank};

const RED: Color32 = Color32::from_rgb(235, 107, 92);

pub(crate) struct Info {
    pub key: &'static str,
    pub name: &'static str,
    pub blurb: &'static str,
    pub piece: (Color, Role),
    pub lower_better: bool,
}

pub(crate) const GAMES: [Info; 11] = [
    Info { key: "memory", name: "Memória de peões", blurb: "Memoriza onde estão os peões e toca em todas as casas onde estavam.", piece: (Color::White, Role::Pawn), lower_better: false },
    Info { key: "trail", name: "Rota do cavalo", blurb: "Vê o cavalo saltar e repete o mesmo caminho, pela mesma ordem.", piece: (Color::White, Role::Knight), lower_better: false },
    Info { key: "blind", name: "Às cegas", blurb: "As peças desaparecem: segue os lances de cabeça e responde à pergunta.", piece: (Color::Black, Role::Bishop), lower_better: false },
    Info { key: "check", name: "Quem dá xeque?", blurb: "Toca na peça que dá xeque, o mais depressa possível. Cinco rondas; um erro conta 5 segundos.", piece: (Color::Black, Role::King), lower_better: true },
    Info { key: "mates", name: "Mates contra o relógio", blurb: "Faz o máximo de xeque-mates em dois minutos, do mate em 1 ao mate em 3. Três erros e acaba.", piece: (Color::White, Role::Rook), lower_better: false },
    Info { key: "change", name: "O que mudou?", blurb: "Estuda o tabuleiro, carrega em Pronto e toca em todas as casas que mudaram.", piece: (Color::White, Role::Bishop), lower_better: false },
    Info { key: "legal", name: "Legal ou ilegal?", blurb: "Diz se o lance da seta é legal ou ilegal, antes que o tempo acabe.", piece: (Color::Black, Role::Pawn), lower_better: false },
    Info { key: "crash", name: "Peça pendurada", blurb: "O multiplicador sobe até alguém deixar uma peça pendurada. Recolhe o ouro antes que ela caia, ou voltas aos 100.", piece: (Color::Black, Role::Queen), lower_better: false },
    Info { key: "mines", name: "Casas seguras", blurb: "Toca só nas casas que nenhuma peça preta ataca. Dois minutos, tabuleiro atrás de tabuleiro; três erros e acaba.", piece: (Color::Black, Role::Rook), lower_better: false },
    Info { key: "queens", name: "Damas por cores", blurb: "Uma dama em cada cor, sem nenhuma atacar outra (linha, coluna ou diagonal). Resolve o máximo de grelhas em três minutos.", piece: (Color::White, Role::Queen), lower_better: false },
    Info { key: "pairs", name: "Pares", blurb: "Vira duas cartas de cada vez e encontra todos os pares antes de acabarem os lances.", piece: (Color::Black, Role::Knight), lower_better: false },
];

/// What "score" means for each game, for the panel and the list.
fn score_text(kind: usize, v: f64) -> String {
    match GAMES[kind].key {
        "check" => format!("{:.2} s", v),
        "crash" => format!("{:.0}", v),
        "memory" | "trail" | "blind" | "change" | "pairs" => tr!("nível {}", v as u32),
        _ => format!("{}", v as u32),
    }
}

// ---------- small helpers ----------

fn rn(r: &mut Rng, n: usize) -> usize { (r.next() % n.max(1) as u64) as usize }
fn sq_at(f: u32, r: u32) -> Square { Square::from_coords(File::new(f), Rank::new(r)) }
fn coords(sq: Square) -> (u32, u32) { (u32::from(sq.file()), u32::from(sq.rank())) }

fn parse(fen: &str) -> Option<Chess> { fen.parse::<Fen>().ok()?.into_position(CastlingMode::Standard).ok() }

/// A real position from the puzzle set (after the opponent's first move), filtered.
fn random_pos(pz: &[Puzzle], r: &mut Rng, ok: impl Fn(&Puzzle, &Chess) -> bool) -> Chess {
    for _ in 0..400 {
        let p = &pz[rn(r, pz.len())];
        let Some(pos) = parse(&p.fen) else { continue };
        let Some(m) = UciMove::from_ascii(p.moves[0].as_bytes()).ok().and_then(|u| u.to_move(&pos).ok()) else { continue };
        let Ok(after) = pos.play(m) else { continue };
        if ok(p, &after) { return after; }
    }
    Chess::default()
}

/// "a torre branca", "o cavalo preto": the piece with its article, for the questions of "Às cegas".
fn piece_phrase(color: Color, role: Role) -> &'static str {
    match (color, role) {
        (Color::White, Role::Pawn) => tl("o peão branco"), (Color::White, Role::Knight) => tl("o cavalo branco"),
        (Color::White, Role::Bishop) => tl("o bispo branco"), (Color::White, Role::Rook) => tl("a torre branca"),
        (Color::White, Role::Queen) => tl("a dama branca"), (Color::White, Role::King) => tl("o rei branco"),
        (Color::Black, Role::Pawn) => tl("o peão preto"), (Color::Black, Role::Knight) => tl("o cavalo preto"),
        (Color::Black, Role::Bishop) => tl("o bispo preto"), (Color::Black, Role::Rook) => tl("a torre preta"),
        (Color::Black, Role::Queen) => tl("a dama preta"), (Color::Black, Role::King) => tl("o rei preto"),
    }
}

fn mmss(s: f64) -> String { let s = s.max(0.0).ceil() as u32; format!("{}:{:02}", s / 60, s % 60) }

/// The moves of a random game continuation, avoiding positions that end the game.
fn random_line(pos: &Chess, n: usize, r: &mut Rng) -> Vec<Move> {
    let mut p = pos.clone();
    let mut out = vec![];
    for _ in 0..n {
        let moves: Vec<Move> = p.legal_moves().into_iter().filter(|m| m.promotion().map_or(true, |q| q == Role::Queen)).collect();
        let safe: Vec<&Move> = moves.iter().filter(|m| !p.clone().play((*m).clone()).map_or(true, |a| a.is_game_over())).collect();
        if safe.is_empty() { break; }
        let m = safe[rn(r, safe.len())].clone();
        p = p.play(m.clone()).unwrap();
        out.push(m);
    }
    out
}

// ---------- the games' states ----------

#[derive(Clone)]
enum Quest { Where { from: Square, role: Role, color: Color, answer: Square }, What { sq: Square, answer: Option<Piece> } }

enum St {
    Memory { level: u32, pawns: Vec<Square>, window: Bitboard, show_until: f64, found: Vec<Square>, next: Option<f64> },
    Trail { level: u32, route: Vec<Square>, shown_at: f64, dt: f64, input: Vec<Square>, next: Option<f64> },
    Blind { level: u32, start: Chess, moves: Vec<Move>, end: Chess, phase: u8, t: f64, dt: f64, quest: Quest, right: Option<bool> },
    Check { round: u32, pos: Chess, checker: Square, t: f64, times: Vec<f64>, wrong_at: Option<Square> },
    Mates { pos: Chess, sol: Vec<String>, i: usize, flipped: bool, reply_at: Option<f64>, next: Option<f64>, last: Option<(Square, Square)>, pools: [Vec<usize>; 3] },
    Change { level: u32, before: Board, after: Board, changed: Bitboard, found: Bitboard, phase: u8, t: f64, wrong_at: Option<Square> },
    Legal { pos: Chess, from: Square, to: Square, legal: bool, t: f64, limit: f64, answered: Option<(bool, f64)> },
    Crash { round: u32, gold: f64, pos: Chess, climb: f64, pending: Option<String>, asked: bool, move_at: f64, plies: u32, last: Option<(Square, Square)>, end: Option<(bool, f64, f64)>, log: Vec<(bool, f64)> },
    Mines { board: Board, safe: Bitboard, found: Bitboard, wrong: Bitboard, boards: u32, next: Option<f64> },
    Queens { n: usize, region: Vec<u8>, sol: Vec<usize>, placed: Vec<usize>, solved: u32, next: Option<f64> },
    Pairs { level: u32, faces: Vec<(Piece, Option<Square>)>, up: Vec<usize>, matched: Vec<bool>, moves: u32, hide_at: Option<f64>, flip_t: Vec<f64>, next: Option<f64> },
}

pub(crate) struct Run {
    pub kind: usize,
    rng: Rng,
    t0: f64,
    limit: Option<f64>, // seconds for the whole run (penalties shorten it)
    hearts: u8,
    score: f64,
    over: Option<f64>,
    flash: (f64, bool),
    pop: f64,
    sel: Option<Square>,
    record: bool,
    st: St,
    engine: Option<(Engine, Receiver<Reply>)>,
    frame: u64,  // the frame whose input was used (egui may draw a frame twice)
    input: bool, // this pass may act on clicks and keys
    bot_t: f64,  // dev: next move of the GAMBITO_GAMEBOT player
    inject: Option<usize>, // dev: the grid cell that player picks
}

impl Run {
    fn good(&mut self, now: f64) { self.flash = (now, true); crate::sound::play(crate::sound::Snd::Right); }
    fn bad(&mut self, now: f64) { self.flash = (now, false); crate::sound::play(crate::sound::Snd::Wrong); }
    fn lose_heart(&mut self, now: f64) { self.bad(now); self.hearts = self.hearts.saturating_sub(1); if self.hearts == 0 { self.over = Some(now); } }
    fn add(&mut self, v: f64, now: f64) { self.score += v; self.pop = now; }
    fn left(&self, now: f64) -> f64 { self.limit.map_or(f64::INFINITY, |l| self.t0 + l - now) }
}

// ---------- generators ----------

fn new_memory(level: u32, r: &mut Rng, now: f64) -> St {
    let s = (3..=8u32).find(|s| s * s >= level * 2).unwrap_or(8);
    let (fx, fy) = (rn(r, (9 - s) as usize) as u32, rn(r, (9 - s) as usize) as u32);
    let mut window = Bitboard::EMPTY;
    for f in fx..fx + s { for k in fy..fy + s { window.add(sq_at(f, k)); } }
    let cells: Vec<Square> = window.into_iter().collect();
    let n = (level as usize).min(cells.len() * 3 / 4).max(1);
    let mut pawns = vec![];
    while pawns.len() < n { let c = cells[rn(r, cells.len())]; if !pawns.contains(&c) { pawns.push(c); } }
    let look = (0.8 + 0.3 * level.min(24) as f64) * 0.92f64.powi(level.saturating_sub(24) as i32);
    St::Memory { level, pawns, window, show_until: now + 0.4 + look, found: vec![], next: None }
}

fn new_trail(level: u32, r: &mut Rng, now: f64, route: Option<Vec<Square>>) -> St {
    let route = route.unwrap_or_else(|| loop {
        let mut v = vec![Square::new(rn(r, 64) as u32)];
        let mut ok = true;
        for _ in 0..level {
            let cur = *v.last().unwrap();
            let opts: Vec<Square> = attacks::knight_attacks(cur).into_iter()
                .filter(|s| if level < 6 { !v.contains(s) } else { v.len() < 2 || *s != v[v.len() - 2] }).collect();
            if opts.is_empty() { ok = false; break; }
            v.push(opts[rn(r, opts.len())]);
        }
        if ok { break v; }
    });
    let dt = (0.8 - 0.03 * level as f64).max(0.34);
    St::Trail { level, route, shown_at: now + 0.5, dt, input: vec![], next: None }
}

fn new_blind(level: u32, pz: &[Puzzle], r: &mut Rng, now: f64) -> St {
    loop {
        let start = random_pos(pz, r, |_, p| p.board().occupied().count() <= 20 && !p.is_check());
        let k = (((level + 1) / 2) as usize).clamp(1, 15);
        let moves = random_line(&start, k, r);
        if moves.len() < k { continue; }
        let mut end = start.clone();
        for m in &moves { end = end.play(m.clone()).unwrap(); }
        // follow each piece that moves from its starting square to where it stands at the end
        // (captured ones drop out; with castling in the line only "what is on" questions are asked)
        let mut cands = vec![];
        let castles = moves.iter().any(|m| m.is_castle());
        for (i, m) in moves.iter().enumerate() {
            let (Some(from), mut at) = (m.from(), dest(m)) else { continue };
            if castles || moves[..i].iter().any(|x| x.from() == Some(from) || dest(x) == from || x.to() == from) { continue; }
            let mut alive = true;
            for later in &moves[i + 1..] {
                if later.to() == at && later.is_capture() { alive = false; break; }
                if later.from() == Some(at) { at = dest(later); }
            }
            if alive { cands.push((from, m.role(), at, i)); }
        }
        let quest = if rn(r, 2) == 0 && !cands.is_empty() {
            let (from, role, answer, i) = cands[rn(r, cands.len())];
            let color = if i % 2 == 0 { start.turn() } else { !start.turn() };
            Quest::Where { from, role, color, answer }
        } else {
            let touched: Vec<Square> = moves.iter().flat_map(|m| [m.from(), Some(dest(m))]).flatten().collect();
            let sq = touched[rn(r, touched.len())];
            Quest::What { sq, answer: end.board().piece_at(sq) }
        };
        let dt = (1.7 - 0.03 * level as f64).max(0.9);
        return St::Blind { level, start, moves, end, phase: 0, t: now, dt, quest, right: None };
    }
}

fn new_check(round: u32, pz: &[Puzzle], r: &mut Rng, now: f64) -> St {
    loop {
        let pos = random_pos(pz, r, |_, p| !p.is_check() && p.board().occupied().count() >= 10);
        let checks: Vec<Chess> = pos.legal_moves().into_iter().filter_map(|m| pos.clone().play(m).ok())
            .filter(|a| a.checkers().count() == 1 && !a.is_game_over()).collect();
        if checks.is_empty() { continue; }
        let a = checks[rn(r, checks.len())].clone();
        let checker = a.checkers().first().unwrap();
        return St::Check { round, pos: a, checker, t: now + 0.9, times: vec![], wrong_at: None };
    }
}

fn mate_pools(pz: &[Puzzle]) -> [Vec<usize>; 3] {
    let f = |t: &str| (0..pz.len()).filter(|&i| pz[i].themes.iter().any(|x| x == t)).collect::<Vec<_>>();
    [f("mateIn1"), f("mateIn2"), f("mateIn3")]
}

fn new_mate(solved: u32, pools: [Vec<usize>; 3], pz: &[Puzzle], r: &mut Rng) -> St {
    let pool = &pools[if solved < 6 { 0 } else if solved < 14 { 1 } else { 2 }];
    for _ in 0..200 {
        let p = &pz[pool[rn(r, pool.len())]];
        let Some(pos) = parse(&p.fen) else { continue };
        let Some(m) = UciMove::from_ascii(p.moves[0].as_bytes()).ok().and_then(|u| u.to_move(&pos).ok()) else { continue };
        let last = m.from().map(|f| (f, dest(&m)));
        let Ok(after) = pos.play(m) else { continue };
        let flipped = after.turn() == Color::Black;
        return St::Mates { pos: after, sol: p.moves.clone(), i: 1, flipped, reply_at: None, next: None, last, pools };
    }
    St::Mates { pos: Chess::default(), sol: vec![], i: 1, flipped: false, reply_at: None, next: None, last: None, pools }
}

fn new_change(level: u32, pz: &[Puzzle], r: &mut Rng, now: f64) -> St {
    let before = random_pos(pz, r, |_, p| p.board().occupied().count() >= 14).board().clone();
    let mut after = before.clone();
    let mut touched = Bitboard::EMPTY;
    let want = (((level + 1) / 2) as usize).clamp(1, 6);
    let roles = [Role::Knight, Role::Bishop, Role::Rook, Role::Queen, Role::Pawn];
    let mut done = 0;
    for _ in 0..200 {
        if done == want { break; }
        let occ: Vec<Square> = after.occupied().into_iter().filter(|s| !touched.contains(*s) && after.role_at(*s) != Some(Role::King)).collect();
        let empty: Vec<Square> = (!after.occupied()).into_iter().filter(|s| !touched.contains(*s)).collect();
        if occ.is_empty() || empty.is_empty() { break; }
        let back = |s: Square| matches!(s.rank(), Rank::First | Rank::Eighth);
        match rn(r, 4) {
            0 => { // a piece moves
                let (a, b) = (occ[rn(r, occ.len())], empty[rn(r, empty.len())]);
                let pc = after.piece_at(a).unwrap();
                if pc.role == Role::Pawn && back(b) { continue; }
                after.discard_piece_at(a); after.set_piece_at(b, pc); touched.add(a); touched.add(b);
            }
            1 => { let a = occ[rn(r, occ.len())]; after.discard_piece_at(a); touched.add(a); }
            2 => {
                let b = empty[rn(r, empty.len())];
                let role = roles[rn(r, roles.len())];
                if role == Role::Pawn && back(b) { continue; }
                after.set_piece_at(b, Piece { color: if rn(r, 2) == 0 { Color::White } else { Color::Black }, role }); touched.add(b);
            }
            _ => {
                let a = occ[rn(r, occ.len())];
                let pc = after.piece_at(a).unwrap();
                let role = roles[rn(r, roles.len())];
                if role == pc.role || (role == Role::Pawn && back(a)) { continue; }
                after.set_piece_at(a, Piece { color: pc.color, role }); touched.add(a);
            }
        }
        done += 1;
    }
    let changed: Bitboard = (0..64).map(Square::new).filter(|s| before.piece_at(*s) != after.piece_at(*s)).collect();
    St::Change { level, before, after, changed, found: Bitboard::EMPTY, phase: 0, t: now, wrong_at: None }
}

/// A move that looks possible for the piece but is not legal here (blocked, pinned, king into check, ...).
fn illegal_move(pos: &Chess, r: &mut Rng) -> Option<(Square, Square)> {
    let b = pos.board();
    let me = pos.turn();
    let legal: Vec<(Square, Square)> = pos.legal_moves().iter().filter_map(|m| Some((m.from()?, dest(m)))).collect();
    let mut close = vec![];
    let mut far = vec![];
    for from in b.by_color(me) {
        let pc = b.piece_at(from).unwrap();
        let targets = if pc.role == Role::Pawn {
            let (f, k) = coords(from);
            let step: i32 = if me == Color::White { 1 } else { -1 };
            let mut t = attacks::pawn_attacks(me, from);
            let one = k as i32 + step;
            if (0..8).contains(&one) { t.add(sq_at(f, one as u32)); }
            let two = k as i32 + 2 * step;
            if (me == Color::White && k == 1) || (me == Color::Black && k == 6) { t.add(sq_at(f, two as u32)); }
            t
        } else {
            attacks::attacks(from, pc, Bitboard::EMPTY)
        };
        for to in targets {
            if b.by_color(me).contains(to) || legal.contains(&(from, to)) { continue; }
            if (attacks::between(from, to) & b.occupied()).count() <= 1 { close.push((from, to)); } else { far.push((from, to)); }
        }
    }
    let pool = if close.is_empty() { far } else { close };
    (!pool.is_empty()).then(|| pool[rn(r, pool.len())])
}

fn new_legal(score: f64, pz: &[Puzzle], r: &mut Rng, now: f64) -> St {
    loop {
        let pos = random_pos(pz, r, |_, p| p.board().occupied().count() >= 12);
        let want_legal = rn(r, 2) == 0;
        let pick = if want_legal {
            let ms: Vec<(Square, Square)> = pos.legal_moves().iter().filter_map(|m| Some((m.from()?, dest(m)))).collect();
            (!ms.is_empty()).then(|| ms[rn(r, ms.len())])
        } else { illegal_move(&pos, r) };
        let Some((from, to)) = pick else { continue };
        let limit = (6.0 - 0.12 * score).max(2.2);
        return St::Legal { pos, from, to, legal: want_legal, t: now, limit, answered: None };
    }
}

fn new_crash(round: u32, gold: f64, log: Vec<(bool, f64)>, pz: &[Puzzle], r: &mut Rng, now: f64) -> St {
    let pos = random_pos(pz, r, |p, a| p.themes.iter().any(|t| t == "middlegame") && p.rating < 1700 && !a.is_check());
    St::Crash { round, gold, pos, climb: now + 1.4, pending: None, asked: false, move_at: now + 1.4, plies: 0, last: None, end: None, log }
}

/// The capture wins a piece that was left hanging: undefended, or worth more than what takes it.
fn hung_capture(before: &Chess, m: &Move) -> bool {
    let Some(victim) = m.capture() else { return false };
    if val(victim) < 3 { return false; }
    let (Some(from), to) = (m.from(), m.to()) else { return false };
    let b = before.board();
    let occ = b.occupied().without(from);
    let defenders = b.attacks_to(to, !before.turn(), occ);
    defenders.is_empty() || val(m.role()) < val(victim)
}

fn new_mines(boards: u32, r: &mut Rng) -> St {
    loop {
        let mut board = Board::empty();
        let k = (4 + boards).min(10) as usize;
        let mut queens = 0;
        while board.occupied().count() < k {
            let sq = Square::new(rn(r, 64) as u32);
            if board.occupied().contains(sq) { continue; }
            let role = [Role::Knight, Role::Bishop, Role::Rook, Role::Queen, Role::Pawn, Role::King][rn(r, 6)];
            if role == Role::Queen { if queens >= 1 { continue; } queens += 1; }
            if role == Role::King && board.kings().any() { continue; }
            if role == Role::Pawn && matches!(sq.rank(), Rank::First | Rank::Eighth) { continue; }
            board.set_piece_at(sq, Piece { color: Color::Black, role });
        }
        let occ = board.occupied();
        let mut hit = Bitboard::EMPTY;
        for sq in occ { hit |= attacks::attacks(sq, board.piece_at(sq).unwrap(), occ); }
        let safe = !(occ | hit);
        if (4..=22).contains(&safe.count()) {
            return St::Mines { board, safe, found: Bitboard::EMPTY, wrong: Bitboard::EMPTY, boards, next: None };
        }
    }
}

/// Every way to put n non-attacking queens on an n x n grid (column of the queen in each row).
fn nqueens(n: usize) -> Vec<Vec<usize>> {
    fn go(n: usize, row: usize, cols: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        if row == n { out.push(cols.clone()); return; }
        for c in 0..n {
            if cols.iter().enumerate().all(|(r, &q)| q != c && (row - r) != c.abs_diff(q)) {
                cols.push(c); go(n, row + 1, cols, out); cols.pop();
            }
        }
    }
    let mut out = vec![];
    go(n, 0, &mut vec![], &mut out);
    out
}

/// Coloured regions grown around one solution until it is the only one.
pub(crate) fn queens_puzzle(n: usize, r: &mut Rng) -> (Vec<u8>, Vec<usize>) {
    let sols = nqueens(n);
    let mut best = None;
    for _ in 0..4000 {
        let sol = sols[rn(r, sols.len())].clone();
        let mut reg = vec![u8::MAX; n * n];
        for (row, &c) in sol.iter().enumerate() { reg[row * n + c] = row as u8; }
        let mut left = n * n - n;
        while left > 0 {
            let i = rn(r, n * n);
            if reg[i] != u8::MAX { continue; }
            let (y, x) = (i / n, i % n);
            let mut nb = vec![];
            if y > 0 { nb.push(i - n); } if y + 1 < n { nb.push(i + n); } if x > 0 { nb.push(i - 1); } if x + 1 < n { nb.push(i + 1); }
            let owned: Vec<u8> = nb.into_iter().map(|j| reg[j]).filter(|&g| g != u8::MAX).collect();
            if owned.is_empty() { continue; }
            reg[i] = owned[rn(r, owned.len())];
            left -= 1;
        }
        let count = sols.iter().filter(|s| {
            let mut seen = vec![false; n];
            s.iter().enumerate().all(|(row, &c)| !std::mem::replace(&mut seen[reg[row * n + c] as usize], true))
        }).count();
        if count == 1 { return (reg, sol); }
        best = Some((reg, sol));
    }
    best.unwrap()
}

fn queens_size(solved: u32) -> usize { [4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10].get(solved as usize).copied().unwrap_or(10) }

fn new_pairs(level: u32, r: &mut Rng) -> St {
    let p = ((level + 2) as usize).min(32);
    let mut kinds: Vec<(Piece, Option<Square>)> = vec![];
    let roles = [Role::King, Role::Queen, Role::Rook, Role::Bishop, Role::Knight, Role::Pawn];
    let colors: &[Color] = if level >= 4 { &[Color::White, Color::Black] } else { &[Color::White] };
    for &c in colors { for &role in &roles { kinds.push((Piece { color: c, role }, None)); } }
    let with_squares = level >= 8 || p > kinds.len();
    let mut faces: Vec<(Piece, Option<Square>)> = vec![];
    while faces.len() < p {
        let (pc, _) = kinds[rn(r, kinds.len())];
        let face = if with_squares { (pc, Some(Square::new(rn(r, 64) as u32))) } else { (pc, None) };
        if !faces.contains(&face) { faces.push(face); }
    }
    let mut deck: Vec<(Piece, Option<Square>)> = faces.iter().chain(faces.iter()).copied().collect();
    for i in (1..deck.len()).rev() { let j = rn(r, i + 1); deck.swap(i, j); }
    let n = deck.len();
    let moves = if level <= 6 { p * 2 + 2 } else if level < 30 { p * 7 / 4 + 2 } else { (p * 7 / 4 + 2).saturating_sub((level - 29) as usize).max(p + 4) } as u32;
    St::Pairs { level, faces: deck, up: vec![], matched: vec![false; n], moves, hide_at: None, flip_t: vec![-10.0; n], next: None }
}

// ---------- the screens ----------

impl App {
    pub(crate) fn start_game(&mut self, kind: usize) {
        let now = self.time;
        let mut r = Rng::new();
        let pz = &self.puzzles;
        let (st, limit) = match GAMES[kind].key {
            "memory" => (new_memory(1, &mut r, now), None),
            "trail" => (new_trail(1, &mut r, now, None), None),
            "blind" => (new_blind(1, pz, &mut r, now), None),
            "check" => (new_check(1, pz, &mut r, now), None),
            "mates" => (new_mate(0, mate_pools(pz), pz, &mut r), Some(120.0)),
            "change" => (new_change(1, pz, &mut r, now), None),
            "legal" => (new_legal(0.0, pz, &mut r, now), None),
            "crash" => (new_crash(1, 100.0, vec![], pz, &mut r, now), None),
            "mines" => (new_mines(0, &mut r), Some(120.0)),
            "queens" => { let (region, sol) = queens_puzzle(4, &mut r); (St::Queens { n: 4, region, sol, placed: vec![], solved: 0, next: None }, Some(180.0)) }
            _ => (new_pairs(1, &mut r), None),
        };
        let engine = (GAMES[kind].key == "crash").then(|| {
            let (tx, rx) = channel();
            (Engine::new(&self.engine_path(0), tx, self.ctx.clone()), rx)
        });
        let score = if GAMES[kind].key == "crash" { 100.0 } else { 0.0 };
        self.game = Some(Run { kind, rng: r, t0: now, limit, hearts: 3, score, over: None, flash: (-10.0, true), pop: -10.0, sel: None, record: false, st, engine, frame: u64::MAX, input: false, bot_t: now + 0.8, inject: None });
    }

    /// The run ended: keep the best score and say so when it is a new one.
    fn end_game(&mut self, run: &mut Run) {
        let info = &GAMES[run.kind];
        let score = match &run.st { St::Check { times, .. } if !times.is_empty() => times.iter().sum::<f64>() / times.len() as f64, St::Check { .. } => return, _ => run.score };
        run.score = score;
        let old = self.progress.games.get(info.key).copied();
        let better = match old { None => score > 0.0 || info.lower_better, Some(b) => if info.lower_better { score < b } else { score > b } };
        if better {
            self.progress.games.insert(info.key.to_string(), score);
            self.progress.save();
            if old.is_some() { run.record = true; self.notify(3, tr!("Novo recorde em {}: {}", tl(info.name), score_text(run.kind, score))); }
        }
    }

    /// The list of games (tab 6, no game open).
    pub(crate) fn games_page(&mut self, ui: &mut egui::Ui) {
        ui.label(semi(tl("Jogos"), 32.0));
        ui.label(egui::RichText::new(tl("Jogos curtos para treinar a visão do tabuleiro, a memória e o cálculo.")).size(15.0).color(Color32::from_rgb(184, 178, 161)));
        ui.add_space(18.0);
        let w = ui.available_width();
        let cols = if w > 1100.0 { 3 } else if w > 700.0 { 2 } else { 1 };
        let gap = 16.0;
        let cw = (w - gap * (cols as f32 - 1.0)) / cols as f32;
        let mut go = None;
        for row in GAMES.chunks(cols).enumerate() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (k, g) in row.1.iter().enumerate() {
                  ui.scope(|ui| {
                    let i = row.0 * cols + k;
                    let (slot, resp) = ui.allocate_exact_size(Vec2::new(cw, 150.0), Sense::click());
                    let hover = motion::hover(ui.ctx(), resp.id, resp.hovered());
                    let (r, alpha) = motion::card(slot, self.page_age(), i, hover); // rises in one after another, lifts on hover
                    if alpha < 1.0 || (hover > 0.0 && hover < 1.0) { ui.ctx().request_repaint(); }
                    ui.multiply_opacity(alpha);
                    let p = ui.painter();
                    p.rect_filled(r, 18.0, PANEL);
                    p.rect_stroke(r, 18.0, Stroke::new(1.0 + hover, Color32::from_rgb(40, 40, 46).lerp_to_gamma(GOLD_LINE, hover)), egui::StrokeKind::Inside);
                    let tile = Rect::from_min_size(r.min + Vec2::new(16.0, 16.0), Vec2::splat(56.0));
                    p.rect_filled(tile, 14.0, if g.piece.0 == Color::White { Color32::from_rgb(56, 52, 40) } else { light() });
                    p.image(self.tex[&g.piece].id(), tile.shrink(6.0), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                    p.text(Pos2::new(tile.right() + 14.0, tile.top() + 6.0), egui::Align2::LEFT_TOP, tl(g.name), f_semi(19.0), CREAM);
                    let best = self.progress.games.get(g.key).map(|&v| tr!("Recorde: {}", score_text(i, v))).unwrap_or_else(|| tl("Ainda sem recorde").to_string());
                    p.text(Pos2::new(tile.right() + 14.0, tile.top() + 34.0), egui::Align2::LEFT_TOP, best, egui::FontId::proportional(13.0), if self.progress.games.contains_key(g.key) { GOLD } else { MUTED });
                    let job = egui::text::LayoutJob::simple(tl(g.blurb).to_string(), egui::FontId::proportional(13.5), TEXT2, r.width() - 32.0);
                    let gal = ui.fonts_mut(|f| f.layout_job(job));
                    ui.painter().galley(Pos2::new(r.left() + 16.0, tile.bottom() + 12.0), gal, TEXT2);
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() { go = Some(i); }
                  });
                }
            });
            ui.add_space(gap);
        }
        if let Some(i) = go { self.start_game(i); }
    }

    /// One game: the board (or grid) on the left, its panel on the right.
    pub(crate) fn game_screen(&mut self, ui: &mut egui::Ui, content: Rect) {
        let Some(mut run) = self.game.take() else { return };
        let now = self.time;
        let fr = ui.ctx().cumulative_frame_nr();
        run.input = run.frame != fr;
        run.frame = fr;
        let was_over = run.over.is_some();
        self.game_tick(&mut run, now);
        if run.input && run.over.is_none() && now >= run.bot_t && std::env::var_os("GAMBITO_GAMEBOT").is_some() { run.bot_t = now + 0.35; self.game_bot(&mut run, now); }
        let m = 24.0;
        let panel_w = (content.width() * 0.34).clamp(360.0, 440.0);
        let panel = Rect::from_min_max(Pos2::new(content.right() - m - panel_w, content.top() + m), Pos2::new(content.right() - m, content.bottom() - m));
        let area = Rect::from_min_max(Pos2::new(content.left() + 24.0, content.top() + 70.0), Pos2::new(panel.left() - 24.0, content.bottom() - 24.0));
        let side = ((area.width().min(area.height()) / 8.0).floor() * 8.0).max(240.0);
        let board = Rect::from_center_size(area.center(), Vec2::splat(side));
        let mut leave = false;
        self.game_board(ui, board, &mut run, now);
        ui.scope_builder(egui::UiBuilder::new().max_rect(panel), |ui| { leave = self.game_panel(ui, panel, &mut run, now); });
        if run.over.is_some() && !was_over { self.end_game(&mut run); }
        if run.over.is_some() && self.game_over_card(ui, board, &mut run, now) { leave = true; }
        ui.ctx().request_repaint();
        if !leave { if self.game.is_none() { self.game = Some(run); } }
    }

    /// Time-driven steps: show phases ending, replies arriving, clocks running out.
    fn game_tick(&mut self, run: &mut Run, now: f64) {
        if run.over.is_some() { return; }
        if run.left(now) <= 0.0 { run.over = Some(now); return; }
        let pz = &self.puzzles;
        let mut r = Rng(run.rng.0);
        let mut heart = false;
        let mut good = false;
        match &mut run.st {
            St::Memory { level, next, .. } => if let Some(t) = *next { if now >= t { run.st = new_memory(*level, &mut r, now); } },
            St::Trail { level, next, .. } => if let Some(t) = *next { if now >= t { run.st = new_trail(*level, &mut r, now, None); } },
            St::Blind { phase, t, dt, moves, right, level, .. } => {
                if *phase == 1 && now >= *t + *dt * moves.len() as f64 + 0.3 { *phase = 2; }
                if *phase == 3 && now >= *t {
                    let lv = if *right == Some(true) { *level + 1 } else { *level };
                    run.st = new_blind(lv, pz, &mut r, now);
                }
            }
            St::Check { round, t, times, .. } => {
                if times.len() as u32 >= *round && now >= *t {
                    if *round >= 5 { run.over = Some(now); } else { let (n, ts) = (*round + 1, times.clone()); run.st = new_check(n, pz, &mut r, now); if let St::Check { times, .. } = &mut run.st { *times = ts; } }
                }
            }
            St::Mates { pos, sol, i, reply_at, next, last, pools, .. } => {
                if let Some(t) = *reply_at { if now >= t {
                    *reply_at = None;
                    if let Some(m) = sol.get(*i).and_then(|u| UciMove::from_ascii(u.as_bytes()).ok()).and_then(|u| u.to_move(pos).ok()) {
                        *last = m.from().map(|f| (f, dest(&m)));
                        *pos = pos.clone().play(m).unwrap();
                        *i += 1;
                    }
                } }
                if let Some(t) = *next { if now >= t { let p = pools.clone(); run.st = new_mate(run.score as u32, p, pz, &mut r); } }
            }
            St::Change { level, phase, t, .. } => {
                if *phase == 1 && now >= *t + 0.45 { *phase = 2; }
                if *phase == 3 && now >= *t { run.st = new_change(*level, pz, &mut r, now); }
            }
            St::Legal { t, limit, answered, .. } => {
                if answered.is_none() && now >= *t + *limit { *answered = Some((false, now)); heart = true; }
                else if let Some((_, at)) = *answered { if now >= at + 0.7 { run.st = new_legal(run.score, pz, &mut r, now); } }
            }
            St::Crash { round, gold, pos, climb, pending, asked, move_at, plies, last, end, log } => {
                if let Some((tx_rx, _)) = run.engine.as_ref().map(|e| (&e.1, ())) {
                    while let Ok(rep) = tx_rx.try_recv() {
                        match rep {
                            Reply::Play { fen, uci } if fen == fen_of(pos) => *pending = Some(uci),
                            Reply::Dead(_) => { run.over = Some(now); }
                            _ => {}
                        }
                    }
                }
                let mult = if now < *climb { 1.0 } else { (0.03 * (now - *climb)).exp() };
                match *end {
                    None => {
                        if !*asked && now >= *move_at - 0.6 {
                            if let Some((e, _)) = &run.engine { e.send(Job::Play { fen: fen_of(pos), elo: 1320 }); }
                            *asked = true;
                        }
                        if let (Some(u), true) = (pending.clone(), now >= *move_at) {
                            *pending = None;
                            *asked = false;
                            if let Some(m) = UciMove::from_ascii(u.as_bytes()).ok().and_then(|x| x.to_move(pos).ok()) {
                                let crash = hung_capture(pos, &m);
                                *last = m.from().map(|f| (f, dest(&m)));
                                *pos = pos.clone().play(m).unwrap();
                                *plies += 1;
                                *move_at = now + 0.85;
                                if crash { *end = Some((true, mult, now)); *gold = 100.0; log.push((true, mult)); heart = false; run.flash = (now, false); }
                                else if pos.is_game_over() || *plies >= 70 { *end = Some((false, mult, now)); *gold = (*gold * mult).round(); log.push((false, mult)); good = true; }
                            }
                        }
                    }
                    Some((_, _, at)) => if now >= at + 2.0 {
                        if *round >= 10 { run.score = *gold; run.over = Some(now); }
                        else { let (n, g, l) = (*round + 1, *gold, log.clone()); run.score = g; run.st = new_crash(n, g, l, pz, &mut r, now); }
                    },
                }
            }
            St::Mines { boards, next, .. } => if let Some(t) = *next { if now >= t { run.st = new_mines(*boards + 1, &mut r); } },
            St::Queens { solved, next, .. } => if let Some(t) = *next { if now >= t {
                let n = queens_size(*solved);
                let (region, sol) = queens_puzzle(n, &mut r);
                let s = *solved;
                run.st = St::Queens { n, region, sol, placed: vec![], solved: s, next: None };
            } },
            St::Pairs { level, up, hide_at, next, .. } => {
                if let Some(t) = *hide_at { if now >= t { up.clear(); *hide_at = None; } }
                if let Some(t) = *next { if now >= t { run.st = new_pairs(*level, &mut r); } }
            }
        }
        run.rng = r;
        if heart { run.lose_heart(now); }
        if good { run.good(now); }
    }

    /// A click on the board (or grid) cell `sq` / index `i`.
    fn game_click(&mut self, run: &mut Run, sq: Square, now: f64) {
        if run.over.is_some() { return; }
        let mut r = Rng(run.rng.0);
        let mut ev: Option<bool> = None; // Some(true) right, Some(false) wrong (lose a heart)
        let mut add = 0.0;
        match &mut run.st {
            St::Memory { level, pawns, show_until, found, next, .. } => if now >= *show_until && next.is_none() {
                if pawns.contains(&sq) {
                    if !found.contains(&sq) { found.push(sq); }
                    if found.len() == pawns.len() { add = 1.0; *level += 1; *next = Some(now + 0.6); ev = Some(true); }
                } else { ev = Some(false); *next = Some(now + 0.9); }
            },
            St::Trail { level, route, shown_at, dt, input, next } => if now >= *shown_at + *dt * route.len() as f64 + 0.2 && next.is_none() {
                if route.get(input.len() + 1) == Some(&sq) {
                    input.push(sq);
                    if input.len() + 1 == route.len() { add = 1.0; *level += 1; *next = Some(now + 0.6); ev = Some(true); }
                } else {
                    ev = Some(false);
                    let (lv, rt) = (*level, route.clone());
                    run.st = new_trail(lv, &mut r, now + 0.6, Some(rt));
                }
            },
            St::Blind { phase, quest, right, t, .. } => if *phase == 2 {
                if let Quest::Where { answer, .. } = quest {
                    let ok = sq == *answer;
                    *right = Some(ok); *phase = 3; *t = now + 1.6; ev = Some(ok); if ok { add = 1.0; }
                }
            },
            St::Check { round, checker, t, times, wrong_at, .. } => if now >= *t && (times.len() as u32) < *round {
                let ok = sq == *checker;
                times.push(if ok { now - *t } else { 5.0 });
                if !ok { *wrong_at = Some(sq); }
                *t = now + if ok { 0.5 } else { 1.1 };
                run.flash = (now, ok);
            },
            St::Mates { pos, sol, i, reply_at, next, last, .. } => if reply_at.is_none() && next.is_none() {
                let base = pos.clone();
                match run.sel {
                    Some(s) if s != sq => {
                        let cands: Vec<Move> = base.legal_moves().into_iter().filter(|m| m.from() == Some(s) && (dest(m) == sq || m.to() == sq)).collect();
                        if let Some(m) = cands.iter().find(|m| m.promotion() == Some(Role::Queen)).or(cands.first()).cloned() {
                            run.sel = None;
                            let after = base.clone().play(m.clone()).unwrap();
                            let uci = m.to_uci(CastlingMode::Standard).to_string();
                            *last = Some((s, dest(&m)));
                            *pos = after.clone();
                            if after.is_checkmate() { add = 1.0; ev = Some(true); *next = Some(now + 0.7); }
                            else if sol.get(*i) == Some(&uci) { *i += 1; *reply_at = Some(now + 0.35); }
                            else { ev = Some(false); *next = Some(now + 0.9); }
                        } else {
                            run.sel = base.board().piece_at(sq).filter(|p| p.color == base.turn()).map(|_| sq);
                        }
                    }
                    _ => run.sel = if run.sel == Some(sq) { None } else { base.board().piece_at(sq).filter(|p| p.color == base.turn()).map(|_| sq) },
                }
            },
            St::Change { level, changed, found, phase, t, wrong_at, .. } => if *phase == 2 {
                if changed.contains(sq) {
                    found.add(sq);
                    if *found == *changed { add = 1.0; *level += 1; *phase = 3; *t = now + 0.8; ev = Some(true); }
                } else if !found.contains(sq) { *wrong_at = Some(sq); *phase = 3; *t = now + 1.6; ev = Some(false); }
            },
            St::Mines { safe, found, wrong, board, next, .. } => if next.is_none() && !board.occupied().contains(sq) && !found.contains(sq) && !wrong.contains(sq) {
                if safe.contains(sq) {
                    found.add(sq); add = 1.0; run.flash = (now, true);
                    if *found == *safe { *next = Some(now + 0.6); }
                } else { wrong.add(sq); ev = Some(false); }
            },
            _ => {}
        }
        run.rng = r;
        if add > 0.0 { run.add(add, now); }
        match ev { Some(true) => run.good(now), Some(false) => run.lose_heart(now), None => {} }
    }

    /// Dev only (GAMBITO_GAMEBOT=1): plays the right move every 0.35 s, through the same paths as the mouse,
    /// and prints the score to stderr; to check that every game advances one step per right answer.
    fn game_bot(&mut self, run: &mut Run, now: f64) {
        enum Act { Click(Square), Legal(bool), Blind(Option<Piece>), Cash, Ready, Cell(usize), Wait }
        let act = match &run.st {
            St::Memory { pawns, show_until, found, next, .. } if now >= *show_until && next.is_none() => pawns.iter().find(|s| !found.contains(s)).map_or(Act::Wait, |s| Act::Click(*s)),
            St::Trail { route, shown_at, dt, input, next, .. } if next.is_none() && now >= *shown_at + *dt * route.len() as f64 + 0.2 => route.get(input.len() + 1).map_or(Act::Wait, |s| Act::Click(*s)),
            St::Blind { phase: 0, .. } | St::Change { phase: 0, .. } => Act::Ready,
            St::Blind { phase: 2, quest: Quest::Where { answer, .. }, .. } => Act::Click(*answer),
            St::Blind { phase: 2, quest: Quest::What { answer, .. }, .. } => Act::Blind(*answer),
            St::Check { round, checker, t, times, .. } if now >= *t && (times.len() as u32) < *round => Act::Click(*checker),
            St::Mates { pos, sol, i, reply_at: None, next: None, .. } => match sol.get(*i).and_then(|u| UciMove::from_ascii(u.as_bytes()).ok()).and_then(|u| u.to_move(pos).ok()) {
                Some(m) if run.sel == m.from() => Act::Click(dest(&m)),
                Some(m) => m.from().map_or(Act::Wait, Act::Click),
                None => Act::Wait,
            },
            St::Change { phase: 2, changed, found, .. } => (*changed & !*found).first().map_or(Act::Wait, Act::Click),
            St::Legal { legal, answered: None, .. } => Act::Legal(*legal),
            St::Crash { climb, end: None, .. } if now >= *climb && (0.03 * (now - *climb)).exp() > 1.25 => Act::Cash,
            St::Mines { safe, found, next: None, .. } => (*safe & !*found).first().map_or(Act::Wait, Act::Click),
            St::Queens { n, sol, placed, next: None, .. } => (0..*n).map(|y| y * *n + sol[y]).find(|q| !placed.contains(q)).map_or(Act::Wait, Act::Cell),
            St::Pairs { faces, up, matched, hide_at: None, next: None, .. } => {
                let open = |i: usize| !matched[i] && !up.contains(&i);
                match up.first() {
                    Some(&a) => (0..faces.len()).find(|&j| j != a && faces[j] == faces[a] && open(j)).map_or(Act::Wait, Act::Cell),
                    None => (0..faces.len()).find(|&i| open(i)).map_or(Act::Wait, Act::Cell),
                }
            }
            _ => Act::Wait,
        };
        match act {
            Act::Click(sq) => self.game_click(run, sq, now),
            Act::Legal(v) => self.legal_answer(run, v, now),
            Act::Blind(v) => self.blind_answer(run, v, now),
            Act::Cash => self.crash_cash(run, now),
            Act::Ready => if let St::Blind { phase, t, .. } | St::Change { phase, t, .. } = &mut run.st { *phase = 1; *t = now; },
            Act::Cell(i) => run.inject = Some(i),
            Act::Wait => return,
        }
        eprintln!("bot {} t={:.1} score={} hearts={}", GAMES[run.kind].key, now - run.t0, run.score, run.hearts);
    }

    fn legal_answer(&mut self, run: &mut Run, said_legal: bool, now: f64) {
        if let St::Legal { legal, answered, .. } = &mut run.st {
            if answered.is_some() { return; }
            let ok = said_legal == *legal;
            *answered = Some((ok, now));
            if ok { run.add(1.0, now); run.good(now); } else { run.lose_heart(now); }
        }
    }

    fn blind_answer(&mut self, run: &mut Run, pc: Option<Piece>, now: f64) {
        let mut ok = None;
        if let St::Blind { phase, quest: Quest::What { answer, .. }, right, t, .. } = &mut run.st {
            if *phase != 2 { return; }
            let good = pc == *answer;
            *right = Some(good); *phase = 3; *t = now + 1.6; ok = Some(good);
        }
        match ok { Some(true) => { run.add(1.0, now); run.good(now); } Some(false) => run.lose_heart(now), None => {} }
    }

    fn crash_cash(&mut self, run: &mut Run, now: f64) {
        if let St::Crash { gold, climb, end, log, .. } = &mut run.st {
            if end.is_some() || now < *climb { return; }
            let mult = (0.03 * (now - *climb)).exp();
            *gold = (*gold * mult).round();
            *end = Some((false, mult, now));
            log.push((false, mult));
            run.score = *gold;
            run.good(now);
        }
    }

    // ---------- drawing ----------

    fn draw_pc(&self, p: &egui::Painter, r: Rect, pc: Piece, alpha: f32) {
        p.image(self.tex[&(pc.color, pc.role)].id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::from_white_alpha((255.0 * alpha) as u8));
    }

    fn game_board(&mut self, ui: &mut egui::Ui, board: Rect, run: &mut Run, now: f64) {
        let resp = ui.interact(board, egui::Id::new("game_board"), Sense::click());
        let key = GAMES[run.kind].key;
        if key == "queens" || key == "pairs" { return self.grid_game(ui, board, run, now, &resp); }
        let flipped = matches!(run.st, St::Mates { flipped: true, .. });
        let cell = board.width() / 8.0;
        let rect_of = |sq: Square| {
            let (f, k) = coords(sq);
            let (c, rr) = if flipped { (7 - f, k) } else { (f, 7 - k) };
            Rect::from_min_size(board.min + Vec2::new(c as f32, rr as f32) * cell, Vec2::splat(cell))
        };
        let sq_of = |pt: Pos2| {
            let (c, rr) = (((pt.x - board.left()) / cell) as u32, ((pt.y - board.top()) / cell) as u32);
            (c < 8 && rr < 8).then(|| if flipped { sq_at(7 - c, rr) } else { sq_at(c, 7 - rr) })
        };
        if run.input && resp.clicked() { if let Some(sq) = resp.interact_pointer_pos().and_then(sq_of) { self.game_click(run, sq, now); } }
        let p = ui.painter_at(board.expand(8.0));
        // the flash: a glow around the board after a right or wrong answer
        let fa = (1.0 - (now - run.flash.0) / 0.6).clamp(0.0, 1.0) as f32;
        if fa > 0.0 { p.rect_stroke(board.expand(3.0), 6.0, Stroke::new(6.0, (if run.flash.1 { GREEN } else { RED }).gamma_multiply(fa)), egui::StrokeKind::Outside); }
        for i in 0..64u32 {
            let sq = Square::new(i);
            let light = (u32::from(sq.file()) + u32::from(sq.rank())) % 2 == 1;
            p.rect_filled(rect_of(sq), 0.0, if light { crate::light() } else { crate::dark() });
        }
        let tint = |sq: Square, c: Color32| p.rect_filled(rect_of(sq), 0.0, c);
        let hl = Color32::from_rgba_unmultiplied(255, 235, 59, 120);
        let ok_c = Color32::from_rgba_unmultiplied(76, 190, 88, 170);
        let bad_c = Color32::from_rgba_unmultiplied(220, 60, 50, 170);
        let mut pieces: Vec<(Square, Piece, f32)> = vec![];
        let mut banner: Option<(String, Color32)> = None;
        let mut arrows: Vec<(Square, Square, Color32)> = vec![];
        let board_pieces = |b: &Board| b.occupied().into_iter().map(|s| (s, b.piece_at(s).unwrap(), 1.0)).collect::<Vec<_>>();
        match &run.st {
            St::Memory { pawns, window, show_until, found, next, .. } => {
                for sq in Bitboard::FULL { if !window.contains(sq) { tint(sq, Color32::from_black_alpha(120)); } }
                for &sq in found { tint(sq, ok_c); }
                if now < *show_until { for &sq in pawns { pieces.push((sq, Piece { color: Color::White, role: Role::Pawn }, 1.0)); } banner = Some((tl("Memoriza!").into(), GOLD)); }
                else if next.is_some() && run.flash.1 { banner = Some((tl("Certo!").into(), GREEN)); }
                else if next.is_some() { for &sq in pawns { pieces.push((sq, Piece { color: Color::White, role: Role::Pawn }, 0.55)); } }
                else { banner = Some((if pawns.len() == 1 { tl("Encontra o peão").to_string() } else { tr!("Encontra os {} peões", pawns.len()) }, CREAM)); }
            }
            St::Trail { route, shown_at, dt, input, .. } => {
                let showing = now < *shown_at + *dt * route.len() as f64 + 0.2;
                let at = if now < *shown_at { 0 } else { (((now - *shown_at) / *dt) as usize).min(route.len() - 1) };
                let path: Vec<Square> = if showing { route[..=at].to_vec() } else { std::iter::once(route[0]).chain(input.iter().copied()).collect() };
                for w in path.windows(2) {
                    p.line_segment([rect_of(w[0]).center(), rect_of(w[1]).center()], Stroke::new(cell * 0.07, if showing { Color32::from_rgba_unmultiplied(242, 193, 74, 200) } else { Color32::from_rgba_unmultiplied(76, 190, 88, 200) }));
                }
                for &s in &path { p.circle_filled(rect_of(s).center(), cell * 0.09, if showing { GOLD } else { GREEN }); }
                let here = *path.last().unwrap();
                tint(route[0], hl);
                pieces.push((here, Piece { color: Color::White, role: Role::Knight }, 1.0));
                banner = Some(if showing { (tl("Observa o caminho").into(), GOLD) } else { (tr!("Salto {} de {}", input.len() + 1, route.len() - 1), CREAM) });
            }
            St::Blind { start, moves, end, phase, t, dt, quest, right, level } => {
                match phase {
                    0 => { pieces = board_pieces(start.board()); banner = Some((tl("Estuda a posição").into(), GOLD)); }
                    1 => {
                        let k = (((now - *t) / *dt).max(0.0) as usize).min(moves.len().saturating_sub(1));
                        let m = &moves[k];
                        let frac = (((now - *t) / *dt).fract() as f32).min(1.0);
                        if let Some(f) = m.from() {
                            if *level <= 12 { arrows.push((f, dest(m), Color32::from_rgba_unmultiplied(255, 170, 0, (220.0 * (1.0 - frac * 0.5)) as u8))); }
                            else { tint(f, hl); tint(dest(m), hl); }
                            if *level <= 6 {
                                let e = 1.0 - (1.0 - (frac * 2.0).min(1.0)).powi(3);
                                let r = rect_of(f).translate((rect_of(dest(m)).min - rect_of(f).min) * e);
                                let pc = Piece { color: if k % 2 == 0 { start.turn() } else { !start.turn() }, role: m.role() };
                                self.draw_pc(&p, r, pc, 0.85);
                            }
                        }
                        let mut pp = start.clone();
                        for x in &moves[..k] { pp = pp.play(x.clone()).unwrap(); }
                        banner = Some((format!("{}  ·  {}/{}", SanPlus::from_move(pp, m.clone()), k + 1, moves.len()), CREAM));
                    }
                    2 => {
                        if let Quest::What { sq, .. } = quest { tint(*sq, hl); }
                        if let Quest::Where { from, .. } = quest { p.rect_stroke(rect_of(*from).shrink(2.0), 0.0, Stroke::new(3.0, GOLD), egui::StrokeKind::Inside); }
                        banner = Some((tl("Responde no painel").into(), GOLD));
                    }
                    _ => {
                        pieces = board_pieces(end.board());
                        let ans = match quest { Quest::Where { answer, .. } => *answer, Quest::What { sq, .. } => *sq };
                        tint(ans, if *right == Some(true) { ok_c } else { bad_c });
                        banner = Some(if *right == Some(true) { (tl("Certo!").into(), GREEN) } else { (tl("Não era isso").into(), RED) });
                    }
                }
            }
            St::Check { round, pos, checker, t, times, wrong_at } => {
                if now >= *t || (times.len() as u32) >= *round {
                    pieces = board_pieces(pos.board());
                    if let Some(k) = pos.board().king_of(pos.turn()) { p.circle_filled(rect_of(k).center(), cell * 0.5, Color32::from_rgba_unmultiplied(230, 0, 0, 120)); }
                    if (times.len() as u32) >= *round {
                        tint(*checker, ok_c);
                        if let Some(w) = wrong_at { if times.last() == Some(&5.0) { tint(*w, bad_c); } }
                        banner = Some((format!("{:.2} s", times.last().unwrap()), if times.last() == Some(&5.0) { RED } else { GREEN }));
                    }
                } else { banner = Some((tr!("Ronda {} de 5", round), GOLD)); }
            }
            St::Mates { pos, last, next, .. } => {
                if let Some((a, b)) = last { tint(*a, hl); tint(*b, hl); }
                if let Some(s) = run.sel {
                    tint(s, hl);
                    for m in pos.legal_moves().into_iter().filter(|m| m.from() == Some(s)) { p.circle_filled(rect_of(dest(&m)).center(), cell * 0.15, Color32::from_black_alpha(45)); }
                }
                pieces = board_pieces(pos.board());
                if let Some(k) = pos.board().king_of(pos.turn()).filter(|_| pos.is_check()) { p.circle_filled(rect_of(k).center(), cell * 0.5, Color32::from_rgba_unmultiplied(230, 0, 0, 140)); }
                if next.is_some() { banner = Some(if pos.is_checkmate() { (tl("Xeque-mate!").into(), GREEN) } else { (tl("Não é mate").into(), RED) }); }
                else { banner = Some((if pos.turn() == Color::White { tl("Brancas jogam e dão mate") } else { tl("Pretas jogam e dão mate") }.into(), CREAM)); }
            }
            St::Change { before, after, changed, found, phase, wrong_at, .. } => {
                match phase {
                    0 => { pieces = board_pieces(before); banner = Some((tl("Estuda o tabuleiro").into(), GOLD)); }
                    1 => {}
                    2 => { pieces = board_pieces(after); for s in *found { tint(s, ok_c); } banner = Some((if changed.count() == 1 { tl("1 casa mudou").to_string() } else { tr!("{} casas mudaram", changed.count()) }, CREAM)); }
                    _ => {
                        pieces = board_pieces(after);
                        for s in *changed { tint(s, if found.contains(s) { ok_c } else { Color32::from_rgba_unmultiplied(242, 193, 74, 150) }); }
                        if let Some(w) = wrong_at { tint(*w, bad_c); }
                    }
                }
            }
            St::Legal { pos, from, to, legal, t, limit, answered } => {
                pieces = board_pieces(pos.board());
                let col = match answered { None => Color32::from_rgba_unmultiplied(255, 170, 0, 215), Some(_) => if *legal { Color32::from_rgba_unmultiplied(60, 170, 70, 225) } else { Color32::from_rgba_unmultiplied(210, 60, 50, 225) } };
                arrows.push((*from, *to, col));
                let frac = if answered.is_some() { 0.0 } else { (1.0 - (now - *t) / *limit).clamp(0.0, 1.0) as f32 };
                // the time left runs round the board's edge
                let bar = Rect::from_min_size(Pos2::new(board.left(), board.bottom() + 2.0), Vec2::new(board.width() * frac, 5.0));
                p.rect_filled(bar, 2.0, if frac < 0.3 { RED } else { GOLD });
                banner = Some((if pos.turn() == Color::White { tl("Jogam as brancas") } else { tl("Jogam as pretas") }.into(), CREAM));
                if let Some((ok, _)) = answered { banner = Some(if *ok { (tl("Certo!").into(), GREEN) } else { ((if *legal { tl("Era legal") } else { tl("Era ilegal") }).into(), RED) }); }
            }
            St::Crash { pos, last, end, climb, .. } => {
                if let Some((a, b)) = last { tint(*a, hl); tint(*b, hl); }
                pieces = board_pieces(pos.board());
                match end {
                    Some((true, _, _)) => { if let Some((_, b)) = last { tint(*b, bad_c); } banner = Some((tl("Caiu! A peça pendurada foi capturada.").into(), RED)); }
                    Some((false, m, _)) => banner = Some((tr!("Recolhido a {}x", format!("{:.2}", m)), GREEN)),
                    None if now < *climb => banner = Some((tl("Prepara-te…").into(), GOLD)),
                    None => {}
                }
            }
            St::Mines { board: b, found, wrong, .. } => {
                pieces = board_pieces(b);
                for s in *found { tint(s, ok_c); }
                for s in *wrong { tint(s, bad_c); }
            }
            _ => {}
        }
        if let St::Mines { wrong, .. } = &run.st {
            for s in *wrong { let c = rect_of(s).center(); let d = cell * 0.18; for (a, b) in [(Vec2::new(-d, -d), Vec2::new(d, d)), (Vec2::new(-d, d), Vec2::new(d, -d))] { p.line_segment([c + a, c + b], Stroke::new(3.0, Color32::WHITE)); } }
        }
        for (sq, pc, a) in pieces { self.draw_pc(&p, rect_of(sq), pc, a); }
        for (a, b, c) in arrows {
            let df = (u32::from(a.file()) as i32 - u32::from(b.file()) as i32).abs();
            let dr = (u32::from(a.rank()) as i32 - u32::from(b.rank()) as i32).abs();
            draw_mark_arrow(&p, rect_of(a).center(), rect_of(b).center(), cell, c, (df, dr) == (1, 2) || (df, dr) == (2, 1));
        }
        if let Some((text, col)) = banner {
            let p = ui.painter().clone(); // above the board, outside its clip
            let gal = p.layout_no_wrap(text, f_semi(17.0), col);
            let r = Rect::from_center_size(Pos2::new(board.center().x, board.top() - 22.0), gal.size() + Vec2::new(28.0, 12.0));
            p.rect_filled(r, 12.0, Color32::from_rgb(24, 24, 28));
            p.rect_stroke(r, 12.0, Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
            p.galley(r.center() - gal.size() / 2.0, gal, col);
        }
    }

    /// Queens and Pairs: not a chessboard but a grid of cells.
    fn grid_game(&mut self, ui: &mut egui::Ui, board: Rect, run: &mut Run, now: f64, resp: &egui::Response) {
        let p = ui.painter_at(board.expand(8.0));
        let fa = (1.0 - (now - run.flash.0) / 0.6).clamp(0.0, 1.0) as f32;
        if fa > 0.0 { p.rect_stroke(board.expand(3.0), 10.0, Stroke::new(6.0, (if run.flash.1 { GREEN } else { RED }).gamma_multiply(fa)), egui::StrokeKind::Outside); }
        let click = if run.input { resp.clicked().then(|| resp.interact_pointer_pos()).flatten() } else { None };
        let over = run.over.is_some();
        let inject = run.inject.take(); // a cell picked by the dev test player
        let mut solved_now = false;
        let mut out_of_moves = false;
        match &mut run.st {
            St::Queens { n, region, sol, placed, next, .. } => {
                let n = *n;
                let cell = board.width() / n as f32;
                const PAL: [(u8, u8, u8); 10] = [(156, 112, 74), (86, 128, 160), (118, 152, 92), (170, 94, 104), (140, 112, 168), (186, 160, 84), (80, 150, 140), (176, 128, 80), (112, 112, 124), (154, 88, 150)];
                let attacks_any = |a: usize, b: usize| { let (ya, xa, yb, xb) = (a / n, a % n, b / n, b % n); ya == yb || xa == xb || ya.abs_diff(yb) == xa.abs_diff(xb) || region[a] == region[b] };
                for i in 0..n * n {
                    let r = Rect::from_min_size(board.min + Vec2::new((i % n) as f32, (i / n) as f32) * cell, Vec2::splat(cell));
                    let (cr, cg, cb) = PAL[region[i] as usize % 10];
                    p.rect_filled(r, 0.0, Color32::from_rgb(cr, cg, cb));
                    p.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::from_black_alpha(70)), egui::StrokeKind::Inside);
                }
                // thick lines between colours
                for i in 0..n * n {
                    let (y, x) = (i / n, i % n);
                    let r = Rect::from_min_size(board.min + Vec2::new(x as f32, y as f32) * cell, Vec2::splat(cell));
                    if x + 1 < n && region[i] != region[i + 1] { p.line_segment([r.right_top(), r.right_bottom()], Stroke::new(3.0, Color32::from_rgb(20, 20, 22))); }
                    if y + 1 < n && region[i] != region[i + n] { p.line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(3.0, Color32::from_rgb(20, 20, 22))); }
                }
                p.rect_stroke(board, 0.0, Stroke::new(3.0, Color32::from_rgb(20, 20, 22)), egui::StrokeKind::Outside);
                let at = click.map(|pt| ((pt.y - board.top()) / cell) as usize * n + ((pt.x - board.left()) / cell) as usize).or(inject);
                if let (Some(i), true, false) = (at, next.is_none(), over) {
                    if i < n * n { if let Some(k) = placed.iter().position(|&q| q == i) { placed.remove(k); } else { placed.push(i); } }
                }
                for &q in placed.iter() {
                    let r = Rect::from_min_size(board.min + Vec2::new((q % n) as f32, (q / n) as f32) * cell, Vec2::splat(cell));
                    let bad = placed.iter().any(|&o| o != q && attacks_any(o, q));
                    if bad { p.rect_filled(r.shrink(2.0), 4.0, Color32::from_rgba_unmultiplied(220, 50, 40, 150)); }
                    p.image(self.tex[&(Color::Black, Role::Queen)].id(), r.shrink(cell * 0.12), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                }
                let done = placed.len() == n && placed.iter().all(|&a| placed.iter().all(|&b| a == b || !attacks_any(a, b)));
                if done && next.is_none() { *next = Some(now + 0.8); solved_now = true; let _ = sol; }
            }
            St::Pairs { faces, up, matched, moves, hide_at, flip_t, next, .. } => {
                let total = faces.len();
                let cols = (total as f32).sqrt().ceil() as usize;
                let rows = total.div_ceil(cols);
                let cell = (board.width() / cols as f32).min(board.height() / rows as f32);
                let origin = board.center() - Vec2::new(cols as f32, rows as f32) * cell / 2.0;
                let rect_i = |i: usize| Rect::from_min_size(origin + Vec2::new((i % cols) as f32, (i / cols) as f32) * cell, Vec2::splat(cell)).shrink(cell * 0.06);
                if let (Some(i), false) = (click.and_then(|pt| (0..total).find(|&i| rect_i(i).contains(pt))).or(inject), over) {
                    {
                        if next.is_none() && !matched[i] && !up.contains(&i) && *moves > 0 {
                            if hide_at.is_some() { up.clear(); *hide_at = None; }
                            up.push(i); flip_t[i] = now;
                            if up.len() == 2 {
                                *moves -= 1;
                                let (a, b) = (up[0], up[1]);
                                if faces[a] == faces[b] { matched[a] = true; matched[b] = true; up.clear(); run.flash = (now, true); }
                                else { *hide_at = Some(now + 0.9); }
                            }
                        }
                    }
                }
                for i in 0..total {
                    let r = rect_i(i);
                    let shown = matched[i] || up.contains(&i);
                    let t = ((now - flip_t[i]) / 0.18).clamp(0.0, 1.0) as f32;
                    let sx = if t < 1.0 { (t * 2.0 - 1.0).abs().max(0.05) } else { 1.0 };
                    let face_now = if t < 0.5 { !shown } else { shown };
                    let rr = Rect::from_center_size(r.center(), Vec2::new(r.width() * sx, r.height()));
                    if face_now {
                        p.rect_filled(rr, 10.0, if matched[i] { Color32::from_rgb(214, 232, 196) } else { light() });
                        p.image(self.tex[&(faces[i].0.color, faces[i].0.role)].id(), Rect::from_center_size(rr.center(), Vec2::splat(r.height() * 0.62)), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                        if let Some(s) = faces[i].1 { p.text(rr.left_top() + Vec2::new(6.0, 4.0), egui::Align2::LEFT_TOP, s.to_string(), f_semi((r.height() * 0.16).max(10.0)), Color32::from_rgb(40, 40, 40)); }
                    } else {
                        p.rect_filled(rr, 10.0, Color32::from_rgb(34, 30, 20));
                        p.rect_stroke(rr, 10.0, Stroke::new(2.0, GOLD_LINE), egui::StrokeKind::Inside);
                        if sx > 0.6 { ico(&p, Ico::Knight, rr.center(), r.height() * 0.38, Color32::from_rgba_unmultiplied(242, 193, 74, 120)); }
                    }
                }
                if next.is_none() && matched.iter().all(|&m| m) { *next = Some(now + 0.8); solved_now = true; }
                else if next.is_none() && *moves == 0 && up.is_empty() && hide_at.is_none() { *next = Some(now + 1.0); out_of_moves = true; }
            }
            _ => {}
        }
        // results of the grid click
        match &mut run.st {
            St::Queens { solved, .. } if solved_now => { *solved += 1; run.add(1.0, now); run.good(now); }
            St::Pairs { level, .. } if solved_now => { *level += 1; run.add(1.0, now); run.good(now); }
            _ => {}
        }
        if out_of_moves { run.lose_heart(now); }
    }

    fn hearts(&self, p: &egui::Painter, at: Pos2, n: u8, now: f64, flash: (f64, bool)) {
        let shake = if !flash.1 && now - flash.0 < 0.4 { ((now - flash.0) * 60.0).sin() as f32 * 3.0 } else { 0.0 };
        for i in 0..3u8 {
            let c = at + Vec2::new(i as f32 * 26.0 + shake, 0.0);
            let col = if i < n { Color32::from_rgb(232, 76, 72) } else { Color32::from_rgb(60, 60, 66) };
            let s = 7.0;
            p.circle_filled(c + Vec2::new(-s * 0.55, -s * 0.2), s * 0.62, col);
            p.circle_filled(c + Vec2::new(s * 0.55, -s * 0.2), s * 0.62, col);
            p.add(egui::Shape::convex_polygon(vec![c + Vec2::new(-s * 1.12, 0.0), c + Vec2::new(s * 1.12, 0.0), c + Vec2::new(0.0, s * 1.15)], col, Stroke::NONE));
        }
    }

    /// The panel: title, rules, the run's numbers and the game's own buttons. Returns true to leave.
    fn game_panel(&mut self, ui: &mut egui::Ui, rect: Rect, run: &mut Run, now: f64) -> bool {
        let p = ui.painter().clone();
        p.rect_filled(rect, 20.0, PANEL);
        p.rect_stroke(rect, 20.0, Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
        let inner = rect.shrink(20.0);
        let info = &GAMES[run.kind];
        let mut leave = false;
        let ok = run.input; // egui may draw a frame twice: act on its input once
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
            ui.horizontal(|ui| {
                if icon_btn(ui, Ico::ChevL, 38.0, false, tl("Voltar aos jogos")) { leave = ok; }
                ui.add_space(4.0);
                ui.label(semi(tl(info.name), 24.0));
            });
            ui.add_space(6.0);
            ui.add(egui::Label::new(egui::RichText::new(tl(info.blurb)).size(14.0).color(TEXT2)).wrap());
            ui.add_space(14.0);
            // the numbers
            let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 92.0), Sense::hover());
            p.rect_filled(r, 16.0, CARD);
            let pop = (1.0 - (now - run.pop) / 0.35).clamp(0.0, 1.0) as f32;
            let score_label = match info.key {
                "check" => tl("ronda"), "crash" => tl("ouro"), "mates" => tl("mates"), "legal" => tl("certos"), "mines" => tl("casas seguras"), "queens" => tl("grelhas"), _ => tl("nível"),
            };
            let big = match &run.st {
                St::Check { round, .. } => format!("{}/5", round),
                St::Crash { gold, .. } => format!("{:.0}", gold),
                St::Memory { level, .. } | St::Trail { level, .. } | St::Blind { level, .. } | St::Change { level, .. } | St::Pairs { level, .. } => level.to_string(),
                _ => format!("{}", run.score as u32),
            };
            p.text(Pos2::new(r.left() + 18.0, r.center().y - 4.0), egui::Align2::LEFT_CENTER, big, f_semi(34.0 + 8.0 * pop), GOLD);
            p.text(Pos2::new(r.left() + 18.0, r.bottom() - 14.0), egui::Align2::LEFT_CENTER, score_label, egui::FontId::proportional(13.0), MUTED);
            let right = Pos2::new(r.right() - 18.0, r.center().y);
            if let Some(_) = run.limit {
                let left = run.left(now);
                p.text(right, egui::Align2::RIGHT_CENTER, mmss(left), f_semi(28.0), if left < 15.0 { RED } else { CREAM });
            }
            let uses_hearts = !matches!(info.key, "check" | "crash" | "queens");
            if uses_hearts { self.hearts(&p, Pos2::new(r.right() - 70.0 - if run.limit.is_some() { 96.0 } else { 0.0 }, r.center().y), run.hearts, now, run.flash); }
            ui.add_space(12.0);
            let w = ui.available_width();
            // the game's own controls
            let mut legal_ans = None;
            let mut blind_ans = None;
            let mut cash = false;
            match &mut run.st {
                St::Blind { phase, quest, t, .. } => {
                    if ok && *phase == 0 && (gold_btn(ui, tl("Pronto"), None, Vec2::new(w, 48.0)) || ui.input(|i| i.key_pressed(egui::Key::Space))) { *phase = 1; *t = now; }
                    if *phase == 2 {
                        let q = match quest {
                            Quest::Where { from, role, color, .. } => tr!("Onde está agora {} que começou em {}? Toca na casa.", piece_phrase(*color, *role), from),
                            Quest::What { sq, .. } => tr!("O que está agora em {}?", sq),
                        };
                        card(ui, |ui| { ui.add(egui::Label::new(semi(q, 17.0)).wrap()); });
                        if let Quest::What { .. } = quest {
                            ui.add_space(8.0);
                            for color in [Color::White, Color::Black] {
                                ui.horizontal(|ui| {
                                    for role in [Role::King, Role::Queen, Role::Rook, Role::Bishop, Role::Knight, Role::Pawn] {
                                        let (b, rs) = ui.allocate_exact_size(Vec2::splat(46.0), Sense::click());
                                        ui.painter().rect_filled(b, 10.0, if rs.hovered() { light() } else { Color32::from_rgb(210, 210, 190) });
                                        ui.painter().image(self.tex[&(color, role)].id(), b.shrink(5.0), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                                        if rs.clicked() { blind_ans = Some(Some(Piece { color, role })); }
                                    }
                                });
                            }
                            if dark_btn(ui, tl("Casa vazia"), None, Vec2::new(w, 42.0), false) { blind_ans = Some(None); }
                        }
                    }
                }
                St::Change { phase, t, .. } => {
                    if ok && *phase == 0 && (gold_btn(ui, tl("Pronto"), None, Vec2::new(w, 48.0)) || ui.input(|i| i.key_pressed(egui::Key::Space))) { *phase = 1; *t = now; }
                }
                St::Legal { answered, .. } => {
                    ui.horizontal(|ui| {
                        let bw = (w - 10.0) / 2.0;
                        if dark_btn(ui, tl("Ilegal  (F)"), Some(Ico::Cross), Vec2::new(bw, 56.0), false) { legal_ans = Some(false); }
                        if gold_btn(ui, tl("Legal  (J)"), Some(Ico::CheckCircle), Vec2::new(bw, 56.0)) { legal_ans = Some(true); }
                    });
                    if answered.is_none() {
                        ui.input(|i| { if i.key_pressed(egui::Key::F) { legal_ans = Some(false); } if i.key_pressed(egui::Key::J) { legal_ans = Some(true); } });
                    }
                }
                St::Crash { round, climb, end, log, .. } => {
                    let mult = match end { Some((_, m, _)) => *m, None if now >= *climb => (0.03 * (now - *climb)).exp(), _ => 1.0 };
                    let (r, _) = ui.allocate_exact_size(Vec2::new(w, 96.0), Sense::hover());
                    p.rect_filled(r, 16.0, Color32::from_rgb(20, 20, 23));
                    let col = match end { Some((true, _, _)) => RED, Some(_) => GREEN, None => CREAM };
                    p.text(r.center(), egui::Align2::CENTER_CENTER, format!("{:.2}x", mult), f_semi(52.0), col);
                    p.text(Pos2::new(r.left() + 12.0, r.top() + 12.0), egui::Align2::LEFT_TOP, tr!("Ronda {} de 10", round), egui::FontId::proportional(12.5), MUTED);
                    ui.add_space(8.0);
                    if ok && end.is_none() && now >= *climb && (gold_btn(ui, tl("Recolher  (Espaço)"), None, Vec2::new(w, 56.0)) || ui.input(|i| i.key_pressed(egui::Key::Space))) { cash = true; }
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        for (crashed, m) in log.iter() {
                            ui.label(egui::RichText::new(if *crashed { tr!("caiu {}x", format!("{:.2}", m)) } else { format!("{:.2}x", m) }).size(13.0).color(if *crashed { RED } else { POS }));
                        }
                    });
                }
                St::Queens { placed, sol, n, region, .. } => {
                    ui.horizontal(|ui| {
                        let bw = (w - 10.0) / 2.0;
                        if dark_btn(ui, tl("Desfazer"), Some(Ico::Restart), Vec2::new(bw, 46.0), false) { if ok { placed.pop(); } }
                        if dark_btn(ui, tl("Pista (−10 s)"), Some(Ico::Target), Vec2::new(bw, 46.0), true) && ok {
                            if let Some(row) = (0..*n).find(|&y| !placed.contains(&(y * *n + sol[y]))) {
                                let q = row * *n + sol[row];
                                let nn = *n;
                                placed.retain(|&o| { let (ya, xa) = (o / nn, o % nn); !(ya == row || xa == sol[row] || ya.abs_diff(row) == xa.abs_diff(sol[row]) || region[o] == region[q]) });
                                placed.push(q);
                                run.limit = run.limit.map(|l| l - 10.0);
                            }
                        }
                    });
                }
                St::Pairs { moves, .. } => { ui.label(egui::RichText::new(tr!("Lances restantes: {}", moves)).size(16.0).color(CREAM)); }
                St::Check { times, .. } => {
                    for (i, t) in times.iter().enumerate() { ui.label(egui::RichText::new(tr!("Ronda {}: {} s", i + 1, format!("{:.2}", t))).size(14.0).color(if *t >= 5.0 { RED } else { POS })); }
                }
                St::Mines { safe, found, .. } => { ui.label(egui::RichText::new(tr!("Faltam {} casas seguras neste tabuleiro.", safe.count() - found.count())).size(14.5).color(TEXT2)); }
                _ => {}
            }
            ui.add_space(14.0);
            if let Some(b) = self.progress.games.get(info.key) { ui.label(egui::RichText::new(tr!("Recorde: {}", score_text(run.kind, *b))).size(14.0).color(GOLD)); }
            ui.add_space(6.0);
            if dark_btn(ui, tl("Recomeçar"), Some(Ico::Restart), Vec2::new(w, 44.0), false) && ok { let k = run.kind; self.start_game(k); }
            if let (Some(v), true) = (legal_ans, ok) { self.legal_answer(run, v, now); }
            if let (Some(v), true) = (blind_ans, ok) { self.blind_answer(run, v, now); }
            if cash && ok { self.crash_cash(run, now); }
        });
        leave
    }

    /// Over the board when the run ends: the score, the record, play again.
    fn game_over_card(&mut self, ui: &mut egui::Ui, board: Rect, run: &mut Run, now: f64) -> bool {
        let age = (now - run.over.unwrap_or(now)) as f32;
        let a = (age / 0.3).clamp(0.0, 1.0);
        let p = ui.painter();
        p.rect_filled(board, 0.0, Color32::from_black_alpha((150.0 * a) as u8));
        let card_r = Rect::from_center_size(board.center() + Vec2::new(0.0, 20.0 * (1.0 - a)), Vec2::new(340.0, 250.0));
        p.rect_filled(card_r, 22.0, PANEL.gamma_multiply(a));
        p.rect_stroke(card_r, 22.0, Stroke::new(1.5, GOLD_LINE.gamma_multiply(a)), egui::StrokeKind::Inside);
        p.text(card_r.center_top() + Vec2::new(0.0, 36.0), egui::Align2::CENTER_CENTER, if run.record { tl("Novo recorde!") } else { tl("Fim do jogo") }, f_semi(24.0), if run.record { GOLD } else { CREAM }.gamma_multiply(a));
        p.text(card_r.center_top() + Vec2::new(0.0, 92.0), egui::Align2::CENTER_CENTER, score_text(run.kind, run.score), f_semi(44.0), GOLD.gamma_multiply(a));
        if let Some(b) = self.progress.games.get(GAMES[run.kind].key) { p.text(card_r.center_top() + Vec2::new(0.0, 136.0), egui::Align2::CENTER_CENTER, tr!("Recorde: {}", score_text(run.kind, *b)), egui::FontId::proportional(14.0), MUTED.gamma_multiply(a)); }
        // confetti for a new record
        if run.record && age < 2.5 {
            motion::confetti(&p, board, age, &[GOLD, CREAM, GREEN, Color32::from_rgb(232, 76, 72)]);
            ui.ctx().request_repaint();
        }
        let mut leave = false;
        let btns = Rect::from_min_size(Pos2::new(card_r.left() + 24.0, card_r.bottom() - 70.0), Vec2::new(card_r.width() - 48.0, 48.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(btns), |ui| {
            ui.horizontal(|ui| {
                let bw = (btns.width() - 10.0) / 2.0;
                if dark_btn(ui, tl("Sair"), None, Vec2::new(bw, 46.0), false) { leave = run.input; }
                if gold_btn(ui, tl("Outra vez"), Some(Ico::Restart), Vec2::new(bw, 46.0)) && run.input { let k = run.kind; self.start_game(k); }
            });
        });
        leave
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queens_grids_have_one_solution() {
        let mut r = Rng(7);
        for n in [4, 6, 8] {
            let (reg, sol) = queens_puzzle(n, &mut r);
            let count = nqueens(n).iter().filter(|s| {
                let mut seen = vec![false; n];
                s.iter().enumerate().all(|(row, &c)| !std::mem::replace(&mut seen[reg[row * n + c] as usize], true))
            }).count();
            assert_eq!(count, 1, "{n}x{n}");
            assert_eq!(sol.len(), n);
        }
        assert_eq!(nqueens(8).len(), 92);
    }

    #[test]
    fn illegal_moves_are_illegal() {
        let pz = crate::train::load_puzzles();
        let mut r = Rng(11);
        for _ in 0..200 {
            let pos = random_pos(&pz, &mut r, |_, _| true);
            if let Some((a, b)) = illegal_move(&pos, &mut r) {
                assert!(!pos.legal_moves().iter().any(|m| m.from() == Some(a) && dest(m) == b));
                assert_eq!(pos.board().piece_at(a).map(|p| p.color), Some(pos.turn()));
            }
        }
    }

    #[test]
    fn hanging_capture_is_seen() {
        // 1.e4 d5 2.exd5 Qxd5 3.Nc3: the queen on d5 is attacked; Nxd5 wins it (worth more than the knight)
        let pos: Chess = parse("rnb1kbnr/ppp1pppp/8/3q4/8/2N5/PPPP1PPP/R1BQKBNR w KQkq - 0 1").unwrap();
        let m = pos.legal_moves().into_iter().find(|m| m.from() == Some(Square::C3) && m.to() == Square::D5).unwrap();
        assert!(hung_capture(&pos, &m));
        let quiet = pos.legal_moves().into_iter().find(|m| m.from() == Some(Square::G1)).unwrap();
        assert!(!hung_capture(&pos, &quiet));
    }

    #[test]
    fn generators_make_playable_rounds() {
        let pz = crate::train::load_puzzles();
        let mut r = Rng(3);
        for _ in 0..20 {
            if let St::Check { pos, checker, .. } = new_check(1, &pz, &mut r, 0.0) { assert!(pos.checkers().contains(checker)); }
            if let St::Change { changed, .. } = new_change(5, &pz, &mut r, 0.0) { assert!(changed.any()); }
            if let St::Mines { safe, board, .. } = new_mines(3, &mut r) { assert!(safe.count() >= 4 && (safe & board.occupied()).is_empty()); }
            if let St::Blind { moves, .. } = new_blind(9, &pz, &mut r, 0.0) { assert_eq!(moves.len(), 5); }
            if let St::Pairs { faces, .. } = new_pairs(9, &mut r) { assert_eq!(faces.len(), 22); }
        }
        assert!(mate_pools(&pz).iter().all(|p| !p.is_empty()));
    }
}
