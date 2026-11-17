#![windows_subsystem = "windows"]
mod book;
mod chesscom;
mod engine;
mod explain;
mod pgn;
mod vision;

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use engine::{Engine, Ev, Job, Reply};
use explain::{explain, val};
use shakmaty::{fen::Fen, san::SanPlus, uci::UciMove, CastlingMode, Chess, Color, EnPassantMode, Move, Position, Role, Square};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

const LIGHT: Color32 = Color32::from_rgb(238, 238, 210);
const DARK: Color32 = Color32::from_rgb(118, 150, 86);

macro_rules! svgs {
    ($($c:ident $r:ident $f:literal),*) => { [$((Color::$c, Role::$r, &include_bytes!(concat!("../assets/pieces/", $f, ".svg"))[..])),*] };
}
fn piece_svgs() -> [(Color, Role, &'static [u8]); 12] {
    svgs!(White Pawn "wP", White Knight "wN", White Bishop "wB", White Rook "wR", White Queen "wQ", White King "wK",
          Black Pawn "bP", Black Knight "bN", Black Bishop "bB", Black Rook "bR", Black Queen "bQ", Black King "bK")
}


/// Drawn by the user with the right mouse button, like chess.com. The u8 is the colour (0 default, 1 Shift, 2 Ctrl, 3 Alt).
#[derive(Clone, Copy, PartialEq)]
enum Mark { Square(Square, u8), Arrow(Square, Square, u8) }

fn arrow_color(i: u8) -> Color32 {
    [Color32::from_rgba_unmultiplied(255, 170, 0, 205), Color32::from_rgba_unmultiplied(21, 120, 27, 205),
     Color32::from_rgba_unmultiplied(136, 32, 32, 205), Color32::from_rgba_unmultiplied(0, 48, 136, 205)][i as usize]
}
fn square_color(i: u8) -> Color32 {
    [Color32::from_rgba_unmultiplied(235, 97, 80, 205), Color32::from_rgba_unmultiplied(172, 206, 89, 205),
     Color32::from_rgba_unmultiplied(255, 170, 0, 205), Color32::from_rgba_unmultiplied(82, 176, 220, 205)][i as usize]
}

/// Straight arrow, or an L-shaped one for a knight jump (long side first), ending in a head.
fn draw_mark_arrow(p: &egui::Painter, a: Pos2, b: Pos2, cell: f32, col: Color32, knight: bool) {
    let corner = knight.then(|| if (b.x - a.x).abs() > (b.y - a.y).abs() { Pos2::new(b.x, a.y) } else { Pos2::new(a.x, b.y) });
    let dir = (b - corner.unwrap_or(a)).normalized();
    let tip = b - dir * cell * 0.08;
    let base = tip - dir * cell * 0.30;
    let n = Vec2::new(-dir.y, dir.x) * cell * 0.19;
    let mut pts = vec![a];
    pts.extend(corner);
    pts.push(base);
    p.add(egui::Shape::line(pts, Stroke::new(cell * 0.15, col)));
    p.add(egui::Shape::convex_polygon(vec![tip, base + n, base - n], col, Stroke::NONE));
}

const ANIM_SECS: f64 = 0.18;

/// A piece sliding to `to` (the shown position's idx); `capture` is a piece fading out at `to`.
#[derive(Clone, Copy)]
struct Anim { idx: usize, from: Square, to: Square, t0: f64, capture: Option<(Square, shakmaty::Piece)> }

struct EngineEntry { name: String, known: Option<usize>, path: Option<PathBuf> }

fn cfg_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("Gambito").join("engines.txt"))
}

/// Engines the user added (name<TAB>path per line); entries whose file vanished are skipped.
fn load_engines() -> Vec<EngineEntry> {
    let Some(text) = cfg_path().and_then(|p| std::fs::read_to_string(p).ok()) else { return vec![] };
    text.lines().filter_map(|l| {
        let (name, path) = l.split_once('\t')?;
        let path = PathBuf::from(path);
        path.exists().then(|| EngineEntry { name: name.to_string(), known: None, path: Some(path) })
    }).collect()
}

fn save_engines(list: &[EngineEntry]) {
    let Some(p) = cfg_path() else { return };
    let text: String = list.iter().filter_map(|e| Some(format!("{}\t{}\n", e.name, e.path.as_ref()?.display()))).collect();
    if let Some(dir) = p.parent() { let _ = std::fs::create_dir_all(dir); }
    let _ = std::fs::write(p, text);
}

struct Hint { san: String, text: String, line: String, ev: String, white_cp: i32, from: Square, to: Square }

/// Who played a loaded game (from PGN headers or chess.com).
struct GameInfo { white: String, black: String, white_elo: Option<u32>, black_elo: Option<u32>, result: String, cc_acc: Option<(f64, f64)> }

struct Import { grid: vision::Grid, sils: vision::Sils, white_top: bool, white_to_move: bool }

/// Turns shakmaty's English position error into something readable.
fn pos_error(e: &str) -> String {
    let l = e.to_lowercase();
    if l.contains("king") { "tem de haver exatamente um rei de cada cor".into() }
    else if l.contains("pawn") { "há peões na primeira ou na última fila".into() }
    else if l.contains("check") { "um dos reis está em xeque de forma impossível".into() }
    else { e.to_string() }
}

#[derive(Clone, Copy, PartialEq, PartialOrd)]
enum Class { Brilliant, Great, Best, Excellent, Good, Book, Inaccuracy, Mistake, Blunder }
const CLASSES: [Class; 9] = [Class::Brilliant, Class::Great, Class::Best, Class::Excellent, Class::Good, Class::Book, Class::Inaccuracy, Class::Mistake, Class::Blunder];
impl Class {
    fn label(self) -> &'static str {
        ["Brilhante", "Ótima", "Melhor jogada", "Excelente", "Boa", "Teoria", "Imprecisão", "Erro", "Gafe"][self as usize]
    }
    fn sym(self) -> &'static str { ["!!", "!", "", "", "", "", "?!", "?", "??"][self as usize] }
    /// text drawn inside the board badge (empty = vector icon, see `draw_icon`)
    fn glyph(self) -> &'static str { ["!!", "!", "", "", "", "", "?!", "?", "??"][self as usize] }
    fn color(self) -> Color32 {
        [Color32::from_rgb(28, 173, 164), Color32::from_rgb(92, 139, 214), Color32::from_rgb(129, 182, 76),
         Color32::from_rgb(150, 188, 75), Color32::from_rgb(166, 175, 139), Color32::from_rgb(168, 126, 86),
         Color32::from_rgb(240, 200, 60), Color32::from_rgb(230, 143, 42), Color32::from_rgb(202, 52, 49)][self as usize]
    }
}

/// Did the mover give up material (a piece, net of recaptures) that the opponent can take?
fn is_sacrifice(prev: &Chess, mv: &Move) -> bool {
    let after = prev.clone().play(mv.clone()).unwrap();
    let gained = mv.capture().map_or(0, val) as i32;
    for m2 in after.legal_moves() {
        let Some(cap) = m2.capture() else { continue };
        if cap == Role::King || val(cap) < 3 { continue; }
        let to = m2.to();
        let after2 = after.clone().play(m2.clone()).unwrap();
        let recapture = after2.legal_moves().iter().any(|m| m.to() == to && m.capture().is_some());
        let net = val(cap) as i32 - if recapture { val(m2.role()) as i32 } else { 0 };
        if net - gained >= 2 { return true; }
    }
    false
}

struct Row { class: Class, san: String, best_san: Option<String>, color: Color }
struct Review { rows: Vec<Row>, acc: [f64; 2], counts: [[u32; 9]; 2] } // index 0 = white, 1 = black

fn win_pct(cp: f64) -> f64 { 50.0 + 50.0 * (2.0 / (1.0 + (-0.00368208 * cp).exp()) - 1.0) }

struct App {
    pos: Chess,
    hist: Vec<(Chess, String)>, // position before the move + its SAN
    mvs: Vec<Move>,             // the moves themselves (parallel to hist)
    quality: Vec<Option<(Class, Option<String>)>>, // rating of each move + engine's better move (parallel to hist)
    view: Option<usize>,        // None = live position; Some(i) = browsing the position after i moves
    review: Option<Review>,
    reviewing: bool,
    reviewer: Engine,
    book: book::Book,
    me: Color,
    flipped: bool,
    sel: Option<Square>,
    drag: Option<Square>,
    last: Option<(Square, Square)>,
    elo: u32,
    hints_on: bool,
    tab: u8,                   // 0 Jogar, 1 Análise, 2 Print
    eval_cp: i32,              // last known evaluation, centipawns from white's side (eval bar)
    eval_mate: bool,
    hint_elo: u32,             // strength of the suggestions (drives analysis depth)
    both: bool,
    hints: Vec<Hint>,
    waiting_hint: bool,
    thinking: bool,
    msg: String,
    player: Engine,
    analyst: Engine,
    tx: Sender<Reply>,
    ctx: egui::Context,
    engines: Vec<EngineEntry>, // built-in ones first, then the ones the user added
    opp: usize,                // opponent engine (index into engines)
    ana: usize,                // engine for the top-3 hints and the game review
    time: f64,                 // seconds, refreshed every frame (drives the animations)
    anim: Option<Anim>,
    badge_t: (usize, f64),     // when the rating icon of position idx first appeared
    hints_t0: f64,             // when the current hints arrived (arrow fade-in)
    skip_anim: bool,           // next move came from a drag: no slide
    rx: Receiver<Reply>,
    tex: HashMap<(Color, Role), egui::TextureHandle>,
    tpl: vision::Templates,
    import: Option<Import>,
    loaded: Option<GameInfo>,  // set while a PGN / chess.com game is open
    pgn_text: String,
    pgn_games: Vec<pgn::Game>, // games of a multi-game PGN waiting to be picked
    cc_user: String,           // what the user typed
    cc_query: String,          // the user name of the list below
    cc_games: Vec<chesscom::CcGame>,
    cc_archives: Vec<String>,
    cc_loaded: usize,          // months fetched so far
    cc_busy: bool,
    cc_err: String,
    net_tx: Sender<chesscom::Msg>,
    net_rx: Receiver<chesscom::Msg>,
    marks: Vec<Mark>,          // arrows / squares drawn with the right mouse button
    mark_from: Option<Square>, // where the current right-button drag started
    brush: Option<Option<shakmaty::Piece>>, // board editor: None = off, Some(None) = eraser, Some(Some(p)) = place p
}

fn fen_of(p: &Chess) -> String {
    Fen::from_position(p, EnPassantMode::Legal).to_string()
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        let ctx = cc.egui_ctx.clone();
        let mut v = egui::Visuals::dark();
        v.panel_fill = Color32::from_rgb(38, 36, 33);
        v.window_fill = v.panel_fill;
        v.widgets.inactive.weak_bg_fill = Color32::from_rgb(62, 59, 55);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(80, 76, 72);
        v.widgets.active.weak_bg_fill = Color32::from_rgb(98, 153, 36);
        v.selection.bg_fill = Color32::from_rgb(98, 153, 36);
        ctx.set_visuals(v);
        ctx.global_style_mut(|st| for f in st.text_styles.values_mut() { f.size *= 1.1; });
        let (tx, rx) = channel();
        let (net_tx, net_rx) = channel();
        let mut engines: Vec<EngineEntry> = engine::known().iter().enumerate()
            .map(|(i, k)| EngineEntry { name: k.name.to_string(), known: Some(i), path: None }).collect();
        engines.extend(load_engines());
        let exe = engine::extract(&engine::known()[0]);
        let svgs = piece_svgs();
        let mut tex = HashMap::new();
        for (c, r, d) in svgs.iter() {
            let px = vision::raster(d, 160);
            let img = egui::ColorImage::from_rgba_premultiplied([160, 160], &px);
            tex.insert((*c, *r), ctx.load_texture(format!("{c:?}{r:?}"), img, egui::TextureOptions::LINEAR));
        }
        let mut app = App {
            pos: Chess::default(), hist: vec![], mvs: vec![], quality: vec![], view: None, review: None, reviewing: false,
            reviewer: Engine::new(&exe, tx.clone(), ctx.clone()), book: book::Book::new(), me: Color::White, flipped: false, sel: None, drag: None, last: None,
            elo: 1500, hints_on: true, hint_elo: 2200, tab: 0, eval_cp: 0, eval_mate: false, both: false, hints: vec![], waiting_hint: false, thinking: false, msg: String::new(),
            player: Engine::new(&exe, tx.clone(), ctx.clone()),
            analyst: Engine::new(&exe, tx.clone(), ctx.clone()),
            tx, ctx, engines, opp: 0, ana: 0, time: 0.0, anim: None, badge_t: (usize::MAX, 0.0), hints_t0: 0.0, skip_anim: false,
            rx, tex, tpl: vision::Templates::new(&svgs), import: None, brush: None, loaded: None, pgn_text: String::new(), pgn_games: vec![], cc_user: String::new(), cc_query: String::new(),
            cc_games: vec![], cc_archives: vec![], cc_loaded: 0, cc_busy: false, cc_err: String::new(), net_tx, net_rx, marks: vec![], mark_from: None,
        };
        app.after_move();
        app
    }

    fn new_game(&mut self, me: Color) {
        self.pos = Chess::default();
        if self.loaded.take().is_some() { self.both = false; } // back to playing the engine
        self.pgn_games.clear();
        self.reset_history();
        self.me = me;
        self.flipped = me == Color::Black;
        self.sel = None;
        self.last = None;
        self.import = None;
        self.msg.clear();
        self.after_move();
    }

    fn after_move(&mut self) {
        if self.both { self.me = self.pos.turn(); } // playing both sides: whoever is to move is "me"
        self.hints.clear();
        self.waiting_hint = false;
        self.thinking = false;
        if self.pos.is_game_over() {
            self.start_review(); // evaluate the finished game automatically
            return;
        }
        if self.pos.turn() != self.me {
            self.thinking = true;
            self.player.send(Job::Play { fen: fen_of(&self.pos), elo: self.elo });
        } else {
            self.request_hints();
        }
    }

    /// Search depth for the suggestions: the "hint Elo" slider trades depth for how obvious the moves are.
    /// ponytail: depth is only a rough stand-in for Elo (1000 -> 2, 2200 -> 10, 2600 -> 12).
    fn hint_depth(&self) -> u32 {
        (2 + (self.hint_elo - 1000) / 150).clamp(2, 14)
    }

    /// Suggestions for the position being shown (also when browsing earlier moves).
    fn request_hints(&mut self) {
        self.hints.clear();
        self.waiting_hint = false;
        let shown = self.pos_at(self.view.unwrap_or(self.hist.len())).clone();
        if self.hints_on && !shown.is_game_over() && (self.view.is_some() || shown.turn() == self.me) {
            self.waiting_hint = true;
            let children = children(&shown);
            self.analyst.send(Job::Hint { fen: fen_of(&shown), depth: self.hint_depth(), children });
        }
    }

    fn play(&mut self, mv: Move) {
        self.marks.clear();
        let san = SanPlus::from_move(self.pos.clone(), mv.clone()).to_string();
        self.last = mv.from().map(|f| (f, dest(&mv)));
        let capture = if mv.is_en_passant() { None } else { mv.capture().and_then(|_| self.pos.board().piece_at(mv.to()).map(|p| (mv.to(), p))) };
        self.anim = match (mv.from(), self.skip_anim) {
            (Some(from), false) => Some(Anim { idx: self.hist.len() + 1, from, to: dest(&mv), t0: self.time, capture }),
            _ => None,
        };
        self.skip_anim = false;
        self.badge_t.0 = usize::MAX;
        let next = self.pos.clone().play(mv.clone()).unwrap();
        self.hist.push((std::mem::replace(&mut self.pos, next), san));
        self.mvs.push(mv);
        self.quality.push(None);
        self.view = None;
        self.review = None;
        self.reviewing = false;
        self.sel = None;
        self.rate_last_move();
        self.after_move();
    }

    fn reset_history(&mut self) {
        self.marks.clear();
        self.hist.clear();
        self.mvs.clear();
        self.quality.clear();
        self.view = None;
        self.review = None;
        self.reviewing = false;
    }

    /// Position after `i` moves (i == hist.len() is the live position).
    fn pos_at(&self, i: usize) -> &Chess {
        self.hist.get(i).map_or(&self.pos, |h| &h.0)
    }

    /// Arrow keys: move through the game. `to` is an absolute index; None/len = live.
    fn goto(&mut self, to: usize) {
        self.marks.clear();
        let n = self.hist.len();
        let old = self.view.unwrap_or(n);
        let to = to.min(n);
        // one step forward/backward: slide the piece (backwards = reversed)
        self.anim = if to == old + 1 {
            let m = &self.mvs[old];
            let capture = if m.is_en_passant() { None } else { m.capture().and_then(|_| self.pos_at(old).board().piece_at(m.to()).map(|p| (m.to(), p))) };
            m.from().map(|from| Anim { idx: to, from, to: dest(m), t0: self.time, capture })
        } else if old > 0 && to == old - 1 {
            let m = &self.mvs[to];
            m.from().map(|from| Anim { idx: to, from: dest(m), to: from, t0: self.time, capture: None })
        } else { None };
        self.view = if to >= n { None } else { Some(to) };
        self.sel = None;
        self.drag = None;
        self.request_hints(); // suggestions follow the position being shown
    }

    fn start_review(&mut self) {
        if self.mvs.is_empty() || self.reviewing || self.review.is_some() { return; }
        self.reviewing = true;
        let mut fens: Vec<String> = self.hist.iter().map(|h| fen_of(&h.0)).collect();
        fens.push(fen_of(&self.pos));
        self.reviewer.send(Job::Review { fens });
    }

    /// Rates one move from the engine's eval before (e0) and after (e1) it. Evals are for the side to move.
    /// Returns (class, win% lost, engine's best move).
    fn classify(&self, prev: &Chess, mv: &Move, e0: Option<&Ev>, e1: Option<&Ev>) -> (Class, f64, Option<Move>) {
        let after = prev.clone().play(mv.clone()).unwrap();
        let cp0 = e0.map_or(0.0, |e| e.0 as f64);
        let cp1 = e1.map_or(if after.is_checkmate() { -1000.0 } else { 0.0 }, |e| e.0 as f64);
        let (w0, w1) = (win_pct(cp0), win_pct(-cp1)); // mover's winning chances before / after
        let drop = (w0 - w1).max(0.0);
        let best = e0.and_then(|e| UciMove::from_ascii(e.1.as_bytes()).ok()?.to_move(prev).ok());
        let is_best = best.as_ref() == Some(mv);
        let only_move = e0.and_then(|e| e.2).map_or(false, |s| w0 - win_pct(s as f64) >= 15.0);
        let class = if drop < 15.0 && self.book.name(&book::key(&after)).is_some() { Class::Book }
            else if (is_best || drop < 3.0) && w1 >= 45.0 && w0 < 85.0 && is_sacrifice(prev, mv) { Class::Brilliant }
            else if is_best && only_move && w1 >= 30.0 { Class::Great }
            else if is_best { Class::Best }
            else if drop < 3.0 { Class::Excellent } else if drop < 8.0 { Class::Good }
            else if drop < 15.0 { Class::Inaccuracy } else if drop < 30.0 { Class::Mistake } else { Class::Blunder };
        (class, drop, best)
    }

    fn build_review(&self, ev: &[Option<Ev>]) -> Review {
        let (mut rows, mut counts, mut sum, mut n) = (vec![], [[0u32; 9]; 2], [0.0f64; 2], [0u32; 2]);
        for i in 0..self.mvs.len() {
            let (prev, mv) = (self.pos_at(i), &self.mvs[i]);
            let side = (prev.turn() == Color::Black) as usize;
            let (class, drop, best) = self.classify(prev, mv, ev[i].as_ref(), ev[i + 1].as_ref());
            sum[side] += (103.1668 * (-0.04354 * drop).exp() - 3.1669).clamp(0.0, 100.0);
            n[side] += 1;
            counts[side][class as usize] += 1;
            let best_san = if class >= Class::Inaccuracy { best.map(|b| SanPlus::from_move(prev.clone(), b).to_string()) } else { None };
            rows.push(Row { class, san: self.hist.get(i).map(|h| h.1.clone()).unwrap_or_default(), best_san, color: prev.turn() });
        }
        Review { rows, acc: [sum[0] / n[0].max(1) as f64, sum[1] / n[1].max(1) as f64], counts }
    }

    /// Per-move rating, requested right after each move so the icon shows up on the board.
    fn rate_last_move(&mut self) {
        let n = self.mvs.len();
        if n == 0 { return; }
        self.reviewer.send(Job::Review { fens: vec![fen_of(self.pos_at(n - 1)), fen_of(&self.pos)] });
    }

    fn opening(&self, idx: usize) -> Option<&(String, String)> {
        (0..=idx.min(40)).rev().find_map(|j| self.book.name(&book::key(self.pos_at(j))))
    }

    /// Drop every move after position `i` and make it the live position (to play another move from there).
    fn truncate_to(&mut self, i: usize) {
        self.marks.clear();
        self.pos = self.hist[i].0.clone();
        self.hist.truncate(i);
        self.mvs.truncate(i);
        self.quality.truncate(i);
        self.view = None;
        self.review = None;
        self.reviewing = false;
        self.thinking = false; // any engine reply still in flight is for the old position and gets ignored
        self.anim = None;
        self.badge_t.0 = usize::MAX;
        self.last = i.checked_sub(1).and_then(|j| self.mvs[j].from().map(|f| (f, dest(&self.mvs[j]))));
    }

    /// Plays from the position being shown (live, or an earlier one picked with the arrow keys).
    fn try_move(&mut self, from: Square, to: Square) -> bool {
        let base = self.pos_at(self.view.unwrap_or(self.hist.len())).clone();
        let cands: Vec<Move> = base.legal_moves().into_iter().filter(|m| m.from() == Some(from) && (dest(m) == to || m.to() == to)).collect();
        // ponytail: promotion is always a queen
        let pick = cands.iter().find(|m| m.promotion() == Some(Role::Queen)).or(cands.first()).cloned();
        match pick {
            Some(m) => {
                if let Some(i) = self.view { self.truncate_to(i); }
                self.play(m);
                true
            }
            None => false,
        }
    }

    /// Right-button drawing: same mark again removes it, same place with another colour recolours it.
    fn toggle_mark(&mut self, m: Mark) {
        let same = |a: &Mark| match (a, &m) {
            (Mark::Square(x, _), Mark::Square(y, _)) => x == y,
            (Mark::Arrow(a1, b1, _), Mark::Arrow(a2, b2, _)) => a1 == a2 && b1 == b2,
            _ => false,
        };
        match self.marks.iter().position(same) {
            Some(i) if self.marks[i] == m => { self.marks.remove(i); }
            Some(i) => self.marks[i] = m,
            None => self.marks.push(m),
        }
    }

    fn click(&mut self, sq: Square) {
        if let (Some(brush), true) = (self.brush, self.import.is_some()) { self.edit_square(sq, brush); return; }
        let base = self.pos_at(self.view.unwrap_or(self.hist.len())).clone();
        if (self.thinking && self.view.is_none()) || base.is_game_over() || (!self.both && base.turn() != self.me) { return; }
        if let Some(s) = self.sel { if s != sq && self.try_move(s, sq) { return; } }
        self.sel = match base.board().piece_at(sq) {
            Some(p) if p.color == base.turn() && self.sel != Some(sq) => Some(sq),
            _ => None,
        };
    }

    /// Takes back my last move (and the engine's reply, if it already answered), so I can play something else.
    fn undo(&mut self) {
        self.marks.clear();
        for _ in 0..2 {
            if let Some((p, _)) = self.hist.pop() { self.pos = p; self.mvs.pop(); self.quality.pop(); }
            if self.both || self.pos.turn() == self.me { break; }
        }
        self.view = None;
        self.review = None;
        self.reviewing = false;
        self.anim = None;
        self.badge_t.0 = usize::MAX;
        self.last = self.mvs.last().and_then(|m| m.from().map(|f| (f, dest(m))));
        self.sel = None;
        self.after_move();
    }

    // ---------- engines ----------
    fn engine_path(&self, i: usize) -> PathBuf {
        match (&self.engines[i].known, &self.engines[i].path) {
            (Some(k), _) => engine::extract(&engine::known()[*k]),
            (_, Some(p)) => p.clone(),
            _ => unreachable!(),
        }
    }

    fn spawn(&self, i: usize) -> Engine {
        Engine::new(&self.engine_path(i), self.tx.clone(), self.ctx.clone())
    }

    fn add_engine(&mut self, path: PathBuf) {
        let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Engine".into());
        let i = match self.engines.iter().position(|e| e.path.as_ref() == Some(&path)) {
            Some(i) => i,
            None => {
                self.engines.push(EngineEntry { name, known: None, path: Some(path) });
                save_engines(&self.engines);
                self.engines.len() - 1
            }
        };
        self.opp = i;
        self.player = self.spawn(i);
        self.msg = format!("Engine \"{}\" adicionada e escolhida como adversária.", self.engines[i].name);
        if self.thinking { self.after_move(); }
    }

    fn poll_engines(&mut self) {
        while let Ok(m) = self.net_rx.try_recv() {
            self.cc_busy = false;
            match m {
                chesscom::Msg::Games { user, archives, loaded, games } => {
                    if user == self.cc_query {
                        self.cc_archives = archives;
                        self.cc_loaded = loaded;
                        self.cc_games.extend(games);
                        self.cc_games.sort_by(|a, b| b.end_time.cmp(&a.end_time));
                        if self.cc_games.is_empty() { self.cc_err = "Sem partidas de xadrez normal nesses meses.".into(); }
                    }
                }
                chesscom::Msg::Err(e) => self.cc_err = e,
            }
        }
        while let Ok(r) = self.rx.try_recv() {
            match r {
                Reply::Play { fen, uci } if !self.both && fen == fen_of(&self.pos) => {
                    if let Some(m) = UciMove::from_ascii(uci.as_bytes()).ok().and_then(|u| u.to_move(&self.pos).ok()) {
                        self.play(m);
                    }
                }
                Reply::Hint { fen, lines } if fen == fen_of(self.pos_at(self.view.unwrap_or(self.hist.len()))) => {
                    self.waiting_hint = false;
                    let shown = self.pos_at(self.view.unwrap_or(self.hist.len())).clone();
                    let stm = shown.turn();
                    let cp_of = |l: &engine::Line| if l.mate { if l.score > 0 { 1000 } else { -1000 } } else { l.score };
                    let gap = if lines.len() >= 2 { Some(cp_of(&lines[0]) - cp_of(&lines[1])) } else { None };
                    let base = explain::material(shown.board(), stm) - explain::material(shown.board(), !stm);
                    self.hints = lines.iter().enumerate().filter_map(|(k, l)| {
                        let first = UciMove::from_ascii(l.pv.first()?.as_bytes()).ok()?.to_move(&shown).ok()?;
                        let (san, why, _) = explain(&shown, &first);
                        // principal line in SAN, and where it leaves the material
                        let (mut p, mut sans) = (shown.clone(), vec![]);
                        for u in &l.pv {
                            let m = UciMove::from_ascii(u.as_bytes()).ok()?.to_move(&p).ok()?;
                            sans.push(SanPlus::from_move(p.clone(), m.clone()).to_string());
                            p = p.play(m).ok()?;
                        }
                        let mut text = why;
                        if l.mate && l.score > 0 {
                            text += &format!(" Mate forçado em {} lance{}.", l.score, if l.score == 1 { "" } else { "s" });
                        } else {
                            let gain = explain::material(p.board(), stm) - explain::material(p.board(), !stm) - base;
                            if gain >= 1 { text += &format!(" Na linha principal ganha {gain} ponto{} de material.", if gain == 1 { "" } else { "s" }); }
                        }
                        if let (0, Some(g)) = (k, gap) {
                            if g >= 150 && !l.mate { text += &format!(" É claramente a melhor: a segunda opção perde {:.1} peões de vantagem.", g as f32 / 100.0); }
                        }
                        let cp = cp_of(l);
                        let white_cp = if stm == Color::White { cp } else { -cp };
                        let ev = if l.mate { format!("{}#{}", if white_cp >= 0 { "+" } else { "-" }, l.score.abs()) } else { format!("{:+.1}", white_cp as f32 / 100.0) };
                        Some(Hint { san, text, line: sans.join(" "), ev, white_cp, from: first.from()?, to: dest(&first) })
                    }).collect();
                    if let Some(h) = self.hints.first() { self.eval_cp = h.white_cp; self.eval_mate = h.ev.contains('#'); }
                    self.hints_t0 = self.time;
                }
                Reply::Dead(name) => {
                    if self.opp == 0 && self.ana == 0 {
                        self.msg = format!("O motor \"{name}\" não arranca.");
                    } else {
                        // a user-added engine failed: fall back to the built-in one
                        self.msg = format!("A engine \"{name}\" parou ou não é UCI. Voltei ao {}.", self.engines[0].name);
                        self.opp = 0;
                        self.ana = 0;
                        self.player = self.spawn(0);
                        self.analyst = self.spawn(0);
                        self.reviewer = self.spawn(0);
                        self.reviewing = false;
                        self.after_move();
                    }
                }
                Reply::Review { fens, evals } => {
                    // a) rating of a single move, for the board icon
                    if fens.len() == 2 {
                        let k = (0..self.mvs.len()).rev().find(|&k| fen_of(self.pos_at(k)) == fens[0] && fen_of(self.pos_at(k + 1)) == fens[1]);
                        if let Some(k) = k {
                            let (class, _, best) = self.classify(self.pos_at(k), &self.mvs[k], evals[0].as_ref(), evals[1].as_ref());
                            let best_san = if class >= Class::Inaccuracy { best.map(|b| SanPlus::from_move(self.pos_at(k).clone(), b).to_string()) } else { None };
                            self.quality[k] = Some((class, best_san));
                        }
                    }
                    // b) whole-game review
                    if self.reviewing && evals.len() == self.hist.len() + 1 && fens.last() == Some(&fen_of(&self.pos)) {
                        let r = self.build_review(&evals);
                        for (i, row) in r.rows.iter().enumerate() { self.quality[i] = Some((row.class, row.best_san.clone())); }
                        self.review = Some(r);
                        self.reviewing = false;
                        if self.loaded.is_some() { self.msg = "Avaliação concluída. Vê o separador Análise.".into(); }
                    }
                }
                _ => {}
            }
        }
    }

    // ---------- screenshot import ----------
    fn load_image(&mut self, img: image::RgbImage) {
        match vision::read_board(&img, &self.tpl) {
            Ok(scan) => {
                let white_top = vision::white_on_top(&scan.grid);
                self.import = Some(Import { grid: scan.grid, sils: scan.sils, white_top, white_to_move: !white_top });
                self.brush = None;
                self.apply_import("Posição lida do print. Se houver peças mal lidas, corrige-as com o editor.");
            }
            Err(e) => self.msg = e,
        }
    }

    /// Builds the position from the (possibly edited) scan. If it is illegal with the chosen side to move,
    /// tries the other side before giving up.
    fn apply_import(&mut self, ok_msg: &str) {
        let Some(i) = &self.import else { return };
        let (grid, white_top, wtm) = (i.grid, i.white_top, i.white_to_move);
        let parse = |wtm: bool| -> Result<Chess, String> {
            vision::grid_to_fen(&grid, white_top, wtm).parse::<Fen>().map_err(|e| e.to_string())
                .and_then(|f| f.into_position::<Chess>(CastlingMode::Standard).map_err(|e| pos_error(&e.to_string())))
        };
        let (pos, wtm, adjusted) = match parse(wtm) {
            Ok(p) => (Ok(p), wtm, false),
            Err(e) => match parse(!wtm) { Ok(p) => (Ok(p), !wtm, true), Err(_) => (Err(e), wtm, false) },
        };
        if let Some(i) = self.import.as_mut() { i.white_to_move = wtm; }
        match pos {
            Ok(p) => {
                self.me = p.turn();
                self.flipped = white_top; // show it like the screenshot
                self.pos = p;
                self.loaded = None;
                self.reset_history();
                self.last = None;
                self.sel = None;
                self.msg = if adjusted { format!("{ok_msg} (Ajustei de quem é a vez para a posição ser legal.)") } else { ok_msg.to_string() };
                self.after_move();
            }
            Err(e) => self.msg = format!("A posição lida não é legal: {e}. Corrige as peças com o editor ou muda a vez."),
        }
    }

    /// Put (or erase) a piece on a square of the scanned position.
    fn edit_square(&mut self, sq: Square, piece: Option<shakmaty::Piece>) {
        let Some(i) = self.import.as_mut() else { return };
        let (r, c) = vision::grid_index(u32::from(sq.file()) as usize, u32::from(sq.rank()) as usize, i.white_top);
        i.grid[r][c] = piece.map(|p| (p.color, p.role));
        self.apply_import("Posição editada.");
    }

    /// Remember the piece shapes of the current (corrected) scan, so the next screenshots of this style read right.
    fn learn_import(&mut self) {
        let Some(i) = &self.import else { return };
        let n = self.tpl.learn(&i.grid, &i.sils);
        self.msg = format!("Aprendi {n} peças deste print. Os próximos prints com este estilo devem sair melhores.");
    }

    /// Ctrl+V: a screenshot becomes a position, PGN text becomes a game.
    fn paste_any(&mut self) {
        if let Ok(d) = arboard::Clipboard::new().and_then(|mut c| c.get_image()) {
            if let Some(im) = image::RgbaImage::from_raw(d.width as u32, d.height as u32, d.bytes.into_owned()) {
                self.load_image(image::DynamicImage::ImageRgba8(im).to_rgb8());
                return;
            }
        }
        if let Ok(t) = arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
            if self.try_pgn(&t) { return; }
        }
        self.msg = "Na área de transferência não há um print nem um PGN.".into();
    }

    fn open_file(&mut self, path: &std::path::Path) {
        let is_pgn = path.extension().map_or(false, |e| e.eq_ignore_ascii_case("pgn") || e.eq_ignore_ascii_case("txt"));
        if is_pgn {
            match std::fs::read_to_string(path) {
                Ok(t) => { self.pgn_text = t.clone(); if !self.try_pgn(&t) { self.msg = "Não encontrei nenhuma partida neste ficheiro.".into(); } }
                Err(e) => self.msg = format!("Não consegui abrir o ficheiro: {e}"),
            }
            return;
        }
        match image::open(path) {
            Ok(im) => self.load_image(im.to_rgb8()),
            Err(e) => self.msg = format!("Não consegui abrir a imagem: {e}"),
        }
    }

    fn status(&self) -> String {
        if self.both {
            let side = |c: Color| if c == Color::White { "brancas" } else { "pretas" };
            if self.pos.is_checkmate() { return format!("Xeque-mate. Ganham as {}.", side(!self.pos.turn())); }
            if self.pos.is_game_over() { return "Empate.".into(); }
            return format!("Vez das {}{}", side(self.pos.turn()), if self.pos.is_check() { " (xeque!)" } else { "" });
        }
        if self.pos.is_checkmate() {
            return if self.pos.turn() == self.me { "Xeque-mate. Perdeste.".into() } else { "Xeque-mate. Ganhaste!".into() };
        }
        if self.pos.is_game_over() { return "Empate.".into(); }
        if self.pos.turn() == self.me { format!("A tua vez{}", if self.pos.is_check() { " (xeque!)" } else { "" }) } else { "A engine está a pensar…".into() }
    }

    // ---------- UI ----------
    fn board_ui(&mut self, ui: &mut egui::Ui, side: f32) {
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(side), Sense::click_and_drag());
        let cell = side / 8.0;
        let flipped = self.flipped;
        // when browsing with the arrow keys we draw an earlier position (read-only)
        let live = self.view.is_none();
        let shown: Chess = self.pos_at(self.view.unwrap_or(self.hist.len())).clone();
        let last = match self.view { None => self.last, Some(0) => None, Some(i) => self.mvs.get(i - 1).and_then(|m| m.from().map(|f| (f, dest(m)))) };
        let to_sq = |p: Pos2| -> Option<Square> {
            if !rect.contains(p) { return None; }
            let (c, r) = (((p.x - rect.left()) / cell) as u32, ((p.y - rect.top()) / cell) as u32);
            let (f, rk) = if flipped { (7 - c, r) } else { (c, 7 - r) };
            Some(Square::from_coords(shakmaty::File::new(f), shakmaty::Rank::new(rk)))
        };
        let pos_of = |sq: Square| -> Pos2 {
            let (f, rk) = (u32::from(sq.file()), u32::from(sq.rank()));
            let (c, r) = if flipped { (7 - f, rk) } else { (f, 7 - rk) };
            rect.min + Vec2::new(c as f32, r as f32) * cell
        };

        // moves are allowed on any shown position (playing from an earlier one drops the later moves)
        let editing = self.import.is_some() && self.brush.is_some();
        let movable = !editing && !shown.is_game_over() && (self.both || shown.turn() == self.me) && !(self.thinking && live);
        // right button, chess.com style: drag = arrow, click = square; Shift / Ctrl / Alt pick the colour
        let mods = ui.input(|i| i.modifiers);
        let mark_col: u8 = if mods.shift { 1 } else if mods.ctrl || mods.command { 2 } else if mods.alt { 3 } else { 0 };
        if resp.drag_started_by(egui::PointerButton::Secondary) { self.mark_from = resp.interact_pointer_pos().and_then(to_sq); }
        if resp.drag_stopped_by(egui::PointerButton::Secondary) {
            if let (Some(a), Some(b)) = (self.mark_from.take(), ui.ctx().pointer_interact_pos().and_then(to_sq)) {
                self.toggle_mark(if a == b { Mark::Square(a, mark_col) } else { Mark::Arrow(a, b, mark_col) });
            }
        }
        if resp.secondary_clicked() {
            if let Some(sq) = resp.interact_pointer_pos().and_then(to_sq) { self.toggle_mark(Mark::Square(sq, mark_col)); }
        }
        if resp.clicked() || resp.drag_started() { self.marks.clear(); } // a left click wipes the drawings
        if resp.drag_started() {
            if let Some(sq) = resp.interact_pointer_pos().and_then(to_sq) {
                if movable && shown.board().piece_at(sq).map_or(false, |p| p.color == shown.turn()) {
                    self.sel = Some(sq);
                    self.drag = Some(sq);
                }
            }
        } else if resp.drag_stopped() {
            if let (Some(from), Some(to)) = (self.drag.take(), ui.ctx().pointer_interact_pos().and_then(to_sq)) {
                if from != to {
                    self.sel = None;
                    self.skip_anim = true; // the piece is already under the cursor, no slide
                    if !self.try_move(from, to) { self.skip_anim = false; self.sel = Some(from); }
                }
            }
        } else if resp.clicked() {
            if let Some(sq) = resp.interact_pointer_pos().and_then(to_sq) { self.click(sq); }
        }

        let p = ui.painter_at(rect);
        let legal: Vec<Move> = self.sel.filter(|_| movable).map(|s| shown.legal_moves().into_iter().filter(|m| m.from() == Some(s)).collect()).unwrap_or_default();
        let king_chk = if shown.is_check() { shown.board().king_of(shown.turn()) } else { None };
        for i in 0..64u32 {
            let sq = Square::new(i);
            let r0 = Rect::from_min_size(pos_of(sq), Vec2::splat(cell));
            let light = (u32::from(sq.file()) + u32::from(sq.rank())) % 2 == 1;
            p.rect_filled(r0, 0.0, if light { LIGHT } else { DARK });
            if last.map_or(false, |(a, b)| a == sq || b == sq) || (movable && self.sel == Some(sq)) {
                p.rect_filled(r0, 0.0, Color32::from_rgba_unmultiplied(255, 235, 59, 130));
            }
            for m in &self.marks { if let Mark::Square(ms, c) = m { if *ms == sq { p.rect_filled(r0, 0.0, square_color(*c)); } } }
            if king_chk == Some(sq) { p.circle_filled(r0.center(), cell * 0.5, Color32::from_rgba_unmultiplied(230, 0, 0, 170)); }
            let txt = if light { DARK } else { LIGHT };
            let at_left = if self.flipped { sq.file() == shakmaty::File::H } else { sq.file() == shakmaty::File::A };
            let at_bottom = if self.flipped { sq.rank() == shakmaty::Rank::Eighth } else { sq.rank() == shakmaty::Rank::First };
            let font = egui::FontId::proportional((cell * 0.17).max(9.0));
            if at_left { p.text(r0.left_top() + Vec2::new(3.0, 2.0), egui::Align2::LEFT_TOP, sq.rank().char(), font.clone(), txt); }
            if at_bottom { p.text(r0.right_bottom() - Vec2::new(3.0, 1.0), egui::Align2::RIGHT_BOTTOM, sq.file().char(), font, txt); }
            if let Some(m) = legal.iter().find(|m| dest(m) == sq) {
                let c = Color32::from_rgba_unmultiplied(0, 0, 0, 38);
                if shown.board().piece_at(sq).is_some() || m.is_en_passant() { p.circle_stroke(r0.center(), cell * 0.45, Stroke::new(cell * 0.09, c)); } else { p.circle_filled(r0.center(), cell * 0.16, c); }
            }
        }
        let shown_idx = self.view.unwrap_or(self.hist.len());
        // slide animation of the move just made (or stepped through with the arrow keys)
        let anim = self.anim.filter(|a| a.idx == shown_idx && self.time - a.t0 < ANIM_SECS);
        let ease = anim.map_or(1.0, |a| 1.0 - (1.0 - ((self.time - a.t0) / ANIM_SECS).clamp(0.0, 1.0) as f32).powi(3));
        let draw_piece = |pc: shakmaty::Piece, at: Pos2, tint: Color32| {
            p.image(self.tex[&(pc.color, pc.role)].id(), Rect::from_min_size(at, Vec2::splat(cell)),
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), tint);
        };
        for i in 0..64u32 {
            let sq = Square::new(i);
            if self.drag == Some(sq) || anim.map_or(false, |a| a.to == sq) { continue; }
            if let Some(pc) = shown.board().piece_at(sq) { draw_piece(pc, pos_of(sq), Color32::WHITE); }
        }
        if let Some(a) = anim {
            if let Some((sq, pc)) = a.capture { draw_piece(pc, pos_of(sq), Color32::from_white_alpha((255.0 * (1.0 - ease)) as u8)); } // captured piece fades out
            if let Some(pc) = shown.board().piece_at(a.to) { draw_piece(pc, pos_of(a.from) + (pos_of(a.to) - pos_of(a.from)) * ease, Color32::WHITE); }
        }
        // arrows drawn by the user (above the pieces) + the one being dragged
        let preview = if resp.dragged_by(egui::PointerButton::Secondary) {
            self.mark_from.zip(ui.ctx().pointer_interact_pos().and_then(to_sq)).filter(|(a, b)| a != b).map(|(a, b)| Mark::Arrow(a, b, mark_col))
        } else { None };
        for m in self.marks.iter().copied().chain(preview) {
            if let Mark::Arrow(a, b, c) = m {
                let (ca, cb) = (pos_of(a) + Vec2::splat(cell / 2.0), pos_of(b) + Vec2::splat(cell / 2.0));
                let df = (u32::from(a.file()) as i32 - u32::from(b.file()) as i32).abs();
                let dr = (u32::from(a.rank()) as i32 - u32::from(b.rank()) as i32).abs();
                draw_mark_arrow(&p, ca, cb, cell, arrow_color(c), (df, dr) == (1, 2) || (df, dr) == (2, 1));
            }
        }
        // rating icon of the last move shown, on its destination square (top-right corner, like chess.com); pops in
        let cls = if shown_idx > 0 { self.quality.get(shown_idx - 1).and_then(|q| q.as_ref()).map(|q| q.0) } else { None };
        if let Some(class) = cls {
            if self.badge_t.0 != shown_idx { self.badge_t = (shown_idx, self.time); }
            let t = ((self.time - self.badge_t.1) / 0.28).clamp(0.0, 1.0) as f32;
            let (c1, c3) = (1.70158f32, 2.70158f32);
            let pop = 1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2); // ease-out-back: overshoots then settles
            let c = pos_of(dest(&self.mvs[shown_idx - 1])) + Vec2::new(cell * 0.82, cell * 0.18);
            draw_icon(&p, c, cell * 0.17 * pop.max(0.01), class);
        }
        // top-3 arrows (fade in)
        let fade = ((self.time - self.hints_t0) / 0.25).clamp(0.0, 1.0) as f32;
        for (i, h) in self.hints.iter().enumerate() {
            let col = Color32::from_rgba_unmultiplied(10, 123, 214, ([210.0, 150.0, 100.0][i] * fade) as u8);
            let (a, b) = (pos_of(h.from) + Vec2::splat(cell / 2.0), pos_of(h.to) + Vec2::splat(cell / 2.0));
            let dir = (b - a).normalized();
            let tip = b - dir * cell * 0.12;
            let base = tip - dir * cell * 0.32;
            let n = Vec2::new(-dir.y, dir.x) * cell * 0.17;
            p.line_segment([a, base], Stroke::new(cell * 0.14, col));
            p.add(egui::Shape::convex_polygon(vec![tip, base + n, base - n], col, Stroke::NONE));
            // rank badge on the destination square (top-left corner; top-right is for the move rating icon)
            let badge = b + Vec2::new(-cell * 0.3, -cell * 0.3);
            p.circle_filled(badge, cell * 0.17, Color32::from_rgba_unmultiplied(10, 123, 214, (255.0 * fade) as u8));
            p.circle_stroke(badge, cell * 0.17, Stroke::new(cell * 0.025, Color32::from_white_alpha((255.0 * fade) as u8)));
            p.text(badge, egui::Align2::CENTER_CENTER, i + 1, egui::FontId::proportional(cell * 0.22), Color32::from_white_alpha((255.0 * fade) as u8));
        }
        if let (Some(d), Some(ptr)) = (self.drag, ui.ctx().pointer_interact_pos()) {
            if let Some(pc) = shown.board().piece_at(d) {
                p.image(self.tex[&(pc.color, pc.role)].id(), Rect::from_center_size(ptr, Vec2::splat(cell)),
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            }
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _f: &mut eframe::Frame) {
        self.time = ctx.input(|i| i.time);
        self.poll_engines();
        // shortcuts: F = flip, Ctrl+V = paste screenshot, dropped file = import
        let (flip, paste, dropped) = ctx.input(|i| (
            i.key_pressed(egui::Key::F) && !i.modifiers.command,
            i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))) || (i.modifiers.command && i.key_pressed(egui::Key::V)),
            i.raw.dropped_files.iter().find_map(|f| Some(f.path().to_path_buf())),
        ));
        if flip { self.flipped = !self.flipped; }
        // ←/→ step through the game, Home/End jump to start/live. Consumed so a focused slider doesn't take them.
        let (l, r, h, e) = ctx.input_mut(|i| {
            let m = egui::Modifiers::NONE;
            (i.consume_key(m, egui::Key::ArrowLeft), i.consume_key(m, egui::Key::ArrowRight), i.consume_key(m, egui::Key::Home), i.consume_key(m, egui::Key::End))
        });
        let cur = self.view.unwrap_or(self.hist.len());
        if l { self.goto(cur.saturating_sub(1)); }
        if r { self.goto(cur + 1); }
        if h { self.goto(0); }
        if e { self.goto(usize::MAX); }
        if paste && !ctx.egui_wants_keyboard_input() { self.paste_any(); } // inside a text box, Ctrl+V is just text
        if let Some(p) = dropped { self.open_file(&p); }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _f: &mut eframe::Frame) {
        egui::Panel::right("side").exact_size(390.0).frame(egui::Frame::new().fill(PANEL)).show(ui, |ui| self.side_panel(ui));
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG).inner_margin(14.0)).show(ui, |ui| self.center(ui));
        // keep repainting only while something is animating
        let t = self.time;
        if self.anim.map_or(false, |a| t - a.t0 < ANIM_SECS + 0.05) || t - self.badge_t.1 < 0.35 || t - self.hints_t0 < 0.35 {
            ui.ctx().request_repaint();
        }
    }
}

// ---------- chess.com-style layout ----------
const BG: Color32 = Color32::from_rgb(48, 46, 43);
const PANEL: Color32 = Color32::from_rgb(38, 36, 33);
const CARD: Color32 = Color32::from_rgb(49, 46, 43);
const ROW_ALT: Color32 = Color32::from_rgb(43, 41, 38);
const GREEN: Color32 = Color32::from_rgb(129, 182, 76);
const TXT2: Color32 = Color32::from_rgb(190, 189, 187);
const AMBER: Color32 = Color32::from_rgb(230, 190, 90);

/// Rounded dark card that fills the panel width.
fn card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new().fill(CARD).corner_radius(8.0).inner_margin(12.0).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui);
    });
    ui.add_space(8.0);
}

fn piece_img(tex: &egui::TextureHandle, size: f32) -> egui::Image<'static> {
    egui::Image::new(egui::load::SizedTexture::new(tex.id(), Vec2::splat(size)))
}

/// Evaluation pill like chess.com's: white background when white is better, dark when black is.
fn eval_pill(ui: &mut egui::Ui, text: &str, white_better: bool) {
    let (bg, fg) = if white_better { (Color32::WHITE, Color32::from_rgb(30, 30, 30)) } else { (Color32::from_rgb(70, 67, 64), Color32::WHITE) };
    egui::Frame::new().fill(bg).corner_radius(4.0).inner_margin(egui::Margin::symmetric(6, 2)).show(ui, |ui| {
        ui.label(egui::RichText::new(text).color(fg).strong().size(13.0));
    });
}

impl App {
    fn player_name(&self, c: Color) -> String {
        if let Some(i) = &self.loaded {
            let (n, e) = if c == Color::White { (&i.white, i.white_elo) } else { (&i.black, i.black_elo) };
            return match e { Some(e) => format!("{n} ({e})"), None => n.clone() };
        }
        if self.both { return if c == Color::White { "Brancas".into() } else { "Pretas".into() }; }
        if c == self.me { "Tu".into() } else { format!("{}  ({})", self.engines[self.opp].name, self.elo) }
    }

    /// Board with the evaluation bar on its left and a player bar above and below, like chess.com.
    fn center(&mut self, ui: &mut egui::Ui) {
        let (bar_h, eval_w, gap) = (38.0, 26.0, 8.0);
        let avail = ui.available_size();
        let side = (avail.x - eval_w - gap).min(avail.y - 2.0 * bar_h - 16.0).max(200.0);
        let (top, bottom) = if self.flipped { (Color::White, Color::Black) } else { (Color::Black, Color::White) };
        let shown = self.pos_at(self.view.unwrap_or(self.hist.len())).clone();
        ui.spacing_mut().item_spacing = Vec2::new(gap, 6.0);
        ui.vertical(|ui| {
            self.player_bar(ui, top, &shown, side + eval_w + gap, bar_h);
            ui.horizontal(|ui| {
                self.eval_bar(ui, eval_w, side);
                self.board_ui(ui, side);
            });
            self.player_bar(ui, bottom, &shown, side + eval_w + gap, bar_h);
        });
    }

    fn player_bar(&self, ui: &mut egui::Ui, c: Color, shown: &Chess, width: f32, h: f32) {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, h), Sense::hover());
        let p = ui.painter_at(rect);
        let av = Rect::from_min_size(rect.min + Vec2::new(0.0, 2.0), Vec2::splat(h - 4.0));
        p.rect_filled(av, 4.0, Color32::from_rgb(64, 61, 57));
        p.image(self.tex[&(c, Role::King)].id(), av.shrink(2.0), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        let name = self.player_name(c);
        p.text(av.right_center() + Vec2::new(10.0, -7.0), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(15.0), Color32::WHITE);
        let diff = explain::material(shown.board(), c) - explain::material(shown.board(), !c);
        if diff > 0 {
            p.text(av.right_center() + Vec2::new(10.0, 9.0), egui::Align2::LEFT_CENTER, format!("+{diff}"), egui::FontId::proportional(12.5), TXT2);
        }
    }

    /// Vertical evaluation bar (white share at white's side of the board). Follows the best suggestion.
    fn eval_bar(&mut self, ui: &mut egui::Ui, w: f32, h: f32) {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
        let target = (win_pct(self.eval_cp as f64) / 100.0) as f32;
        let share = ui.ctx().animate_value_with_time(egui::Id::new("evalbar"), target, 0.35);
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 3.0, Color32::from_rgb(64, 61, 57));
        let white_h = rect.height() * share;
        let white = if self.flipped { Rect::from_min_size(rect.min, Vec2::new(w, white_h)) } else { Rect::from_min_size(Pos2::new(rect.left(), rect.bottom() - white_h), Vec2::new(w, white_h)) };
        p.rect_filled(white, 3.0, Color32::from_rgb(245, 245, 245));
        let label = if self.eval_mate { "#".to_string() } else { format!("{:.1}", (self.eval_cp as f32 / 100.0).abs()) };
        // the number sits at the end of the bar belonging to the side that is better
        let white_better = self.eval_cp >= 0;
        let at_bottom = white_better != self.flipped; // white's end is the bottom unless the board is flipped
        let at = Pos2::new(rect.center().x, if at_bottom { rect.bottom() - 9.0 } else { rect.top() + 9.0 });
        let col = if white_better { Color32::from_rgb(40, 40, 40) } else { Color32::WHITE };
        p.text(at, egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(10.5), col);
    }

    fn side_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.add(piece_img(&self.tex[&(Color::White, Role::Knight)], 30.0));
            ui.label(egui::RichText::new("Gambito").size(22.0).strong());
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.spacing_mut().item_spacing.x = 4.0;
            for (i, t) in ["Jogar", "Análise", "Importar"].iter().enumerate() {
                let sel = self.tab == i as u8;
                let b = egui::Button::new(egui::RichText::new(*t).strong().size(15.0).color(if sel { Color32::WHITE } else { TXT2 }))
                    .fill(if sel { CARD } else { Color32::TRANSPARENT }).min_size(Vec2::new(112.0, 34.0));
                if ui.add(b).clicked() { self.tab = i as u8; }
            }
        });
        ui.add_space(6.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 0)).show(ui, |ui| {
                match self.tab { 0 => self.tab_play(ui), 1 => self.tab_analysis(ui), _ => self.tab_import(ui) }
            });
        });
    }

    fn tab_play(&mut self, ui: &mut egui::Ui) {
        let idx = self.view.unwrap_or(self.hist.len());
        // status + opening
        card(ui, |ui| {
            ui.label(egui::RichText::new(self.status()).size(18.0).strong());
            if !self.msg.is_empty() { ui.label(egui::RichText::new(&self.msg).color(AMBER)); }
            if let Some(i) = &self.loaded {
                ui.label(egui::RichText::new(format!("{} vs {}", i.white, i.black)).strong());
                if !i.result.is_empty() { ui.label(egui::RichText::new(&i.result).small().color(TXT2)); }
            }
            if self.view.is_some() {
                ui.label(egui::RichText::new(format!("A ver o lance {idx} de {}  (End = voltar ao jogo)", self.hist.len())).color(Color32::from_rgb(130, 180, 240)));
            }
            if let Some((eco, name)) = self.opening(idx).cloned() {
                ui.add_space(2.0);
                ui.label(egui::RichText::new(format!("{eco}  {name}")).color(Color32::from_rgb(220, 200, 140)));
            }
            if let Some(Some((c, best))) = self.quality.get(idx.wrapping_sub(1)) {
                let mut t = format!("{}: {}", self.hist[idx - 1].1, c.label());
                if let Some(b) = best { t += &format!("  (melhor era {b})"); }
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                    draw_icon(ui.painter(), r.center(), 9.5, *c);
                    ui.label(egui::RichText::new(t).strong().color(c.color()));
                });
            }
            let conts = self.book.continuations(&book::key(self.pos_at(idx)));
            if !conts.is_empty() {
                egui::CollapsingHeader::new("Teoria: continuações").default_open(false).show(ui, |ui| {
                    for (san, name) in conts.iter().take(8) { ui.label(egui::RichText::new(format!("{san}  →  {name}")).small()); }
                });
            }
        });

        // best moves, with the reason
        if self.hints_on {
            card(ui, |ui| {
                ui.label(egui::RichText::new("Melhores jogadas").strong().size(15.0));
                if self.waiting_hint && self.hints.is_empty() { ui.label(egui::RichText::new("A analisar…").color(TXT2)); }
                for (i, h) in self.hints.iter().enumerate() {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        eval_pill(ui, &h.ev, h.white_cp >= 0);
                        ui.label(egui::RichText::new(format!("{}. {}", i + 1, h.san)).strong().size(16.0));
                    });
                    ui.label(egui::RichText::new(&h.text).size(13.5));
                    if !h.line.is_empty() { ui.label(egui::RichText::new(format!("Linha: {}", h.line)).small().color(TXT2)); }
                }
            });
        }

        // moves, two columns like chess.com + navigation
        card(ui, |ui| {
            ui.label(egui::RichText::new("Lances").strong().size(15.0));
            ui.add_space(4.0);
            let mut jump = None;
            let cur = idx;
            let rows = self.hist.len().div_ceil(2);
            egui::ScrollArea::vertical().max_height(190.0).auto_shrink([false, true]).stick_to_bottom(true).show(ui, |ui| {
                for row in 0..rows {
                    egui::Frame::new().fill(if row % 2 == 1 { ROW_ALT } else { Color32::TRANSPARENT }).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            ui.add_sized([30.0, 22.0], egui::Label::new(egui::RichText::new(format!("{}.", row + 1)).color(TXT2)));
                            for k in 0..2 {
                                let i = row * 2 + k;
                                if i >= self.hist.len() { break; }
                                let q = self.quality.get(i).and_then(|x| x.as_ref()).map(|x| x.0);
                                let mut rt = egui::RichText::new(format!("{}{}", self.hist[i].1, q.map_or("", |c| c.sym())));
                                if let Some(c) = q { rt = rt.color(c.color()); }
                                if ui.add_sized([120.0, 22.0], egui::Button::new(rt).frame(false).selected(cur == i + 1)).clicked() { jump = Some(i + 1); }
                            }
                        });
                    });
                }
                if self.hist.is_empty() { ui.label(egui::RichText::new("Ainda sem lances.").color(TXT2)); }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let n = self.hist.len();
                ui.spacing_mut().item_spacing.x = 6.0;
                for (t, target) in [("|<", Some(0)), ("<", Some(cur.saturating_sub(1))), (">", Some(cur + 1)), (">|", Some(usize::MAX))] {
                    if ui.add_sized([64.0, 30.0], egui::Button::new(egui::RichText::new(t).strong())).clicked() { jump = target; }
                }
                let _ = n;
            });
            if let Some(j) = jump { self.goto(j); }
        });

        // game controls
        card(ui, |ui| {
            ui.horizontal(|ui| {
                let b = |t: &str| egui::Button::new(egui::RichText::new(t).strong().color(Color32::WHITE)).fill(Color32::from_rgb(98, 153, 36)).min_size(Vec2::new(0.0, 34.0));
                if ui.add(b("Novo jogo (brancas)")).clicked() { self.new_game(Color::White); }
                if ui.add(b("Novo jogo (pretas)")).clicked() { self.new_game(Color::Black); }
            });
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new("Voltar atrás").min_size(Vec2::new(0.0, 32.0))).clicked() { self.undo(); }
                if ui.add(egui::Button::new("Virar tabuleiro (F)").min_size(Vec2::new(0.0, 32.0))).clicked() { self.flipped = !self.flipped; }
            });
        });

        // opponent
        card(ui, |ui| {
            ui.label(egui::RichText::new("Adversário").strong().size(15.0));
            let names: Vec<String> = self.engines.iter().map(|e| e.name.clone()).collect();
            let mut new_opp = self.opp;
            egui::ComboBox::from_id_salt("opp").width(220.0).selected_text(names[self.opp].as_str()).show_ui(ui, |ui| {
                for (i, n) in names.iter().enumerate() { ui.selectable_value(&mut new_opp, i, n.as_str()); }
            });
            ui.add_enabled_ui(!self.both, |ui| {
                ui.label(format!("Elo: {}", self.elo));
                ui.add(egui::Slider::new(&mut self.elo, 1000..=2600).step_by(50.0).show_value(false));
            });
            if ui.checkbox(&mut self.both, "Jogar os dois lados (sem engine)").changed() { self.thinking = false; self.after_move(); }
            if ui.button("Adicionar engine UCI…").clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("engine", &["exe"]).pick_file() { self.add_engine(p); }
            }
            if new_opp != self.opp {
                self.opp = new_opp;
                self.player = self.spawn(new_opp);
                self.msg.clear();
                if self.thinking { self.after_move(); }
            }
        });
    }

    fn tab_analysis(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.label(egui::RichText::new("Sugestões").strong().size(15.0));
            if ui.checkbox(&mut self.hints_on, "Mostrar as 3 melhores jogadas").changed() { self.request_hints(); }
            let names: Vec<String> = self.engines.iter().map(|e| e.name.clone()).collect();
            let mut new_ana = self.ana;
            egui::ComboBox::from_id_salt("ana").width(220.0).selected_text(names[self.ana].as_str()).show_ui(ui, |ui| {
                for (i, n) in names.iter().enumerate() { ui.selectable_value(&mut new_ana, i, n.as_str()); }
            });
            ui.label(format!("Elo das sugestões: {}   (profundidade {})", self.hint_elo, self.hint_depth()));
            if ui.add(egui::Slider::new(&mut self.hint_elo, 1000..=2600).step_by(50.0).show_value(false)).changed() { self.request_hints(); }
            if new_ana != self.ana {
                self.ana = new_ana;
                self.analyst = self.spawn(new_ana);
                self.reviewer = self.spawn(new_ana);
                self.msg.clear();
                self.request_hints();
            }
        });

        card(ui, |ui| {
            ui.label(egui::RichText::new("Avaliação da partida").strong().size(15.0));
            let busy = self.reviewing;
            ui.horizontal(|ui| {
                if ui.add_enabled(!busy && !self.mvs.is_empty(), egui::Button::new(if busy { "A avaliar…" } else { "Avaliar partida" }).min_size(Vec2::new(0.0, 32.0))).clicked() {
                    self.review = None;
                    self.start_review();
                }
            });
            if let Some(r) = &self.review {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    for (name, acc, bg, fg) in [("Brancas", r.acc[0], Color32::WHITE, Color32::from_rgb(30, 30, 30)), ("Pretas", r.acc[1], Color32::from_rgb(70, 67, 64), Color32::WHITE)] {
                        egui::Frame::new().fill(bg).corner_radius(8.0).inner_margin(10.0).show(ui, |ui| {
                            ui.set_width(150.0);
                            ui.vertical_centered(|ui| {
                                ui.label(egui::RichText::new(format!("{acc:.0}%")).size(26.0).strong().color(fg));
                                ui.label(egui::RichText::new(format!("Precisão · {name}")).small().color(fg));
                            });
                        });
                    }
                });
                if let Some((w, b)) = self.loaded.as_ref().and_then(|i| i.cc_acc) {
                    ui.label(egui::RichText::new(format!("Precisão no chess.com: brancas {w:.1}% · pretas {b:.1}%")).small().color(TXT2));
                }
                ui.add_space(8.0);
                for c in CLASSES.into_iter().filter(|c| r.counts[0][*c as usize] + r.counts[1][*c as usize] > 0) {
                    ui.horizontal(|ui| {
                        let (rc, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                        draw_icon(ui.painter(), rc.center(), 9.0, c);
                        ui.add_sized([130.0, 20.0], egui::Label::new(egui::RichText::new(c.label()).color(c.color())));
                        ui.add_sized([60.0, 20.0], egui::Label::new(r.counts[0][c as usize].to_string()));
                        ui.add_sized([60.0, 20.0], egui::Label::new(r.counts[1][c as usize].to_string()));
                    });
                }
                ui.add_space(6.0);
                for (i, row) in r.rows.iter().enumerate().filter(|(_, w)| w.class >= Class::Inaccuracy) {
                    let mut t = format!("{}{} {}{}", i / 2 + 1, if row.color == Color::White { "." } else { "..." }, row.san, row.class.sym());
                    if let Some(b) = &row.best_san { t += &format!("   melhor: {b}"); }
                    ui.label(egui::RichText::new(t).small().color(row.class.color()));
                }
            } else if !busy {
                ui.label(egui::RichText::new("Joga alguns lances e carrega em \"Avaliar partida\". Quando a partida acaba, a avaliação é feita sozinha.").small().color(TXT2));
            }
        });
    }

    fn tab_import(&mut self, ui: &mut egui::Ui) {
        if !self.msg.is_empty() {
            card(ui, |ui| { ui.label(egui::RichText::new(&self.msg).color(AMBER)); });
        }

        // chess.com games of a user
        card(ui, |ui| {
            ui.label(egui::RichText::new("Partidas do chess.com").strong().size(15.0));
            ui.label(egui::RichText::new("Escreve o nome de utilizador. Só aparecem partidas públicas e terminadas.").small().color(TXT2));
            ui.horizontal(|ui| {
                let te = ui.add(egui::TextEdit::singleline(&mut self.cc_user).hint_text("utilizador").desired_width(200.0));
                let enter = te.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let clicked = ui.add_enabled(!self.cc_busy, egui::Button::new("Procurar")).clicked();
                if (clicked || enter) && !self.cc_busy { self.cc_search(false); }
            });
            if self.cc_busy { ui.label(egui::RichText::new("A buscar partidas…").color(TXT2)); }
            if !self.cc_err.is_empty() { ui.label(egui::RichText::new(&self.cc_err).color(AMBER)); }
            let user = self.cc_query.clone();
            let mut pick = None;
            egui::ScrollArea::vertical().id_salt("ccgames").max_height(300.0).auto_shrink([false, true]).show(ui, |ui| {
                for (i, g) in self.cc_games.iter().enumerate().take(200) {
                    let white = g.white.eq_ignore_ascii_case(&user);
                    let (my_res, opp, opp_elo) = if white { (&g.white_result, &g.black, g.black_elo) } else { (&g.black_result, &g.white, g.white_elo) };
                    let won = my_res == "win";
                    let drew = g.white_result != "win" && g.black_result != "win";
                    let (tag, col) = if won { ("V", GREEN) } else if drew { ("E", TXT2) } else { ("D", Color32::from_rgb(202, 52, 49)) };
                    ui.horizontal(|ui| {
                        ui.add_sized([16.0, 20.0], egui::Label::new(egui::RichText::new(tag).strong().color(col)));
                        let elo = opp_elo.map_or("?".to_string(), |e| e.to_string());
                        ui.label(format!("{opp} ({elo}) · {} · {}", g.time_class, chesscom::date(g.end_time)));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Analisar").clicked() { pick = Some(i); }
                        });
                    });
                }
            });
            if let Some(i) = pick { self.load_cc_game(i); }
            if !self.cc_games.is_empty() && self.cc_loaded < self.cc_archives.len() && !self.cc_busy && ui.button("Mais antigas").clicked() {
                self.cc_search(true);
            }
        });

        // PGN text or file
        card(ui, |ui| {
            ui.label(egui::RichText::new("PGN").strong().size(15.0));
            ui.add(egui::TextEdit::multiline(&mut self.pgn_text).hint_text("Cola aqui o PGN (de qualquer site)…").desired_rows(5).desired_width(f32::INFINITY));
            ui.horizontal(|ui| {
                if ui.button("Carregar").clicked() {
                    let t = self.pgn_text.clone();
                    if !self.try_pgn(&t) { self.msg = "Não encontrei nenhuma partida neste texto.".into(); }
                }
                if ui.button("Abrir ficheiro…").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("PGN", &["pgn", "txt"]).pick_file() { self.open_file(&p); }
                }
            });
            let mut pick = None;
            for (i, g) in self.pgn_games.iter().enumerate().take(100) {
                ui.horizontal(|ui| {
                    ui.label(format!("{} vs {}  {}", g.header("White").unwrap_or("?"), g.header("Black").unwrap_or("?"), g.header("Result").unwrap_or("")));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Abrir").clicked() { pick = Some(i); }
                    });
                });
            }
            if let Some(i) = pick { let g = self.pgn_games[i].clone(); self.load_pgn_game(&g); }
        });

        // position from a screenshot
        card(ui, |ui| {
            ui.label(egui::RichText::new("Ler posição de um print").strong().size(15.0));
            ui.label(egui::RichText::new("Ctrl+V, arrastar a imagem para a janela, ou:").small().color(TXT2));
            ui.horizontal(|ui| {
                if ui.button("Colar print ou PGN").clicked() { self.paste_any(); }
                if ui.button("Abrir imagem…").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("imagem", &["png", "jpg", "jpeg", "webp"]).pick_file() { self.open_file(&p); }
                }
            });
            let mut changed = false;
            if let Some(i) = &mut self.import {
                ui.add_space(4.0);
                changed |= ui.checkbox(&mut i.white_top, "Brancas estão em cima no print").changed();
                ui.horizontal(|ui| {
                    ui.label("Vez de:");
                    changed |= ui.radio_value(&mut i.white_to_move, true, "Brancas").changed();
                    changed |= ui.radio_value(&mut i.white_to_move, false, "Pretas").changed();
                });
            }
            if changed { self.apply_import("Posição atualizada."); }
        });

        if self.import.is_some() {
            card(ui, |ui| {
                ui.label(egui::RichText::new("Corrigir peças").strong().size(15.0));
                ui.label(egui::RichText::new("Escolhe uma peça (ou a borracha) e clica nas casas do tabuleiro.").small().color(TXT2));
                let mut brush = self.brush;
                for color in [Color::White, Color::Black] {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 3.0;
                        for role in [Role::King, Role::Queen, Role::Rook, Role::Bishop, Role::Knight, Role::Pawn] {
                            let pc = shakmaty::Piece { color, role };
                            let b = egui::Button::image(piece_img(&self.tex[&(color, role)], 30.0)).selected(brush == Some(Some(pc)));
                            if ui.add(b).clicked() { brush = if brush == Some(Some(pc)) { None } else { Some(Some(pc)) }; }
                        }
                    });
                }
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new("Borracha").selected(brush == Some(None))).clicked() { brush = if brush == Some(None) { None } else { Some(None) }; }
                    if brush.is_some() && ui.button("Terminar edição").clicked() { brush = None; }
                });
                self.brush = brush;
                ui.add_space(6.0);
                if ui.button("Aprender estas peças").on_hover_text("Guarda o aspeto das peças deste print (já corrigido) para ler melhor os próximos.").clicked() { self.learn_import(); }
            });
        }
    }

    // ---------- PGN / chess.com games ----------
    fn cc_search(&mut self, more: bool) {
        let user = chesscom::clean_user(&self.cc_user);
        if user.is_empty() { self.cc_err = "Escreve o nome de utilizador.".into(); return; }
        if !more || user != self.cc_query {
            self.cc_games.clear();
            self.cc_archives.clear();
            self.cc_loaded = 0;
            self.cc_query = user.clone();
        }
        self.cc_err.clear();
        self.cc_busy = true;
        chesscom::fetch(user, self.cc_archives.clone(), self.cc_loaded, if more { 3 } else { 2 }, self.net_tx.clone(), self.ctx.clone());
    }

    fn load_cc_game(&mut self, i: usize) {
        let g = self.cc_games[i].clone();
        let Some(pgn) = pgn::parse_games(&g.pgn).into_iter().find(|p| !p.moves.is_empty()) else {
            self.msg = "Esta partida não tem lances.".into();
            return;
        };
        let me = if g.black.eq_ignore_ascii_case(&self.cc_query) { Color::Black } else { Color::White };
        let result = match (g.white_result.as_str(), g.black_result.as_str()) {
            ("win", _) => "Brancas ganham",
            (_, "win") => "Pretas ganham",
            _ => "Empate",
        };
        let how = chesscom::termination(&g);
        let info = GameInfo {
            white: g.white.clone(), black: g.black.clone(), white_elo: g.white_elo, black_elo: g.black_elo,
            result: format!("{result}{} · {} · {}", if how.is_empty() { String::new() } else { format!(" por {how}") }, g.time_class, chesscom::date(g.end_time)),
            cc_acc: g.acc,
        };
        self.load_game(&pgn, info, me);
    }

    fn load_pgn_game(&mut self, g: &pgn::Game) {
        let elo = |k: &str| g.header(k).and_then(|v| v.parse::<u32>().ok());
        let result = match g.header("Result") { Some("1-0") => "Brancas ganham", Some("0-1") => "Pretas ganham", Some("1/2-1/2") => "Empate", _ => "" };
        let mut extra: Vec<&str> = vec![result];
        extra.extend(g.header("Termination"));
        extra.extend(g.header("Date").filter(|d| !d.contains('?')));
        let info = GameInfo {
            white: g.header("White").unwrap_or("Brancas").to_string(), black: g.header("Black").unwrap_or("Pretas").to_string(),
            white_elo: elo("WhiteElo"), black_elo: elo("BlackElo"),
            result: extra.into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · "), cc_acc: None,
        };
        self.load_game(g, info, Color::White);
    }

    /// Puts a whole game on the board, opens it at the first move (like chess.com) and evaluates it.
    fn load_game(&mut self, g: &pgn::Game, info: GameInfo, me: Color) {
        let (start, moves) = match g.replay() {
            Ok(x) => x,
            Err(e) => { self.msg = format!("Não consegui ler a partida: {e}"); return; }
        };
        self.import = None;
        self.brush = None;
        self.reset_history();
        let mut pos = start;
        for m in moves {
            let san = SanPlus::from_move(pos.clone(), m.clone()).to_string();
            let next = pos.clone().play(m.clone()).unwrap();
            self.hist.push((std::mem::replace(&mut pos, next), san));
            self.mvs.push(m);
            self.quality.push(None);
        }
        self.pos = pos;
        self.both = true; // analysing: you move both sides, no engine replies
        self.me = self.pos.turn();
        self.flipped = me == Color::Black;
        self.sel = None;
        self.anim = None;
        self.last = self.mvs.last().and_then(|m| m.from().map(|f| (f, dest(m))));
        self.loaded = Some(info);
        self.view = if self.hist.is_empty() { None } else { Some(0) };
        self.tab = 0;
        self.pgn_games.clear();
        self.msg = format!("Partida carregada: {} lances. A avaliar…", self.mvs.len());
        self.after_move();
        self.request_hints();
        self.start_review();
    }

    /// Loads a PGN text: one game opens at once, several are listed to pick from. False if there is none.
    fn try_pgn(&mut self, text: &str) -> bool {
        let games: Vec<pgn::Game> = pgn::parse_games(text).into_iter().filter(|g| !g.moves.is_empty()).collect();
        match games.len() {
            0 => false,
            1 => { self.load_pgn_game(&games[0]); true }
            n => {
                self.pgn_games = games;
                self.tab = 2;
                self.msg = format!("O PGN tem {n} partidas: escolhe uma.");
                true
            }
        }
    }
}

/// The app used to be called Xadrez: carry its saved engines and learned piece shapes over.
fn migrate_old_data() {
    let Some(a) = std::env::var_os("APPDATA").map(PathBuf::from) else { return };
    let (old, new) = (a.join("Xadrez"), a.join("Gambito"));
    if old.is_dir() && !new.exists() {
        let _ = std::fs::create_dir_all(&new);
        if let Ok(rd) = std::fs::read_dir(&old) { for e in rd.flatten() { let _ = std::fs::copy(e.path(), new.join(e.file_name())); } }
    }
}

fn main() -> eframe::Result {
    migrate_old_data();
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1060.0, 740.0]).with_title("Gambito"),
        ..Default::default()
    };
    eframe::run_native("Gambito", opts, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

/// Square the moved piece ends on (shakmaty reports the rook's square for castling).
fn dest(m: &Move) -> Square {
    match m.from() {
        Some(from) if m.is_castle() => {
            let f = if m.to().file() > from.file() { shakmaty::File::G } else { shakmaty::File::C };
            Square::from_coords(f, from.rank())
        }
        _ => m.to(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mv(pos: &Chess, san: &str) -> Move {
        shakmaty::san::San::from_ascii(san.as_bytes()).unwrap().to_move(pos).unwrap()
    }

    #[test]
    fn sacrifice_detection() {
        let pos: Chess = "4k3/2p5/3p4/8/8/8/8/3QK3 w - - 0 1".parse::<Fen>().unwrap().into_position(CastlingMode::Standard).unwrap();
        assert!(is_sacrifice(&pos, &mv(&pos, "Qxd6")));  // queen takes a pawn and is lost to cxd6
        assert!(!is_sacrifice(&pos, &mv(&pos, "Qd2")));  // quiet safe move
    }

    #[test]
    fn castling_destination() {
        let pos: Chess = "r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1".parse::<Fen>().unwrap().into_position(CastlingMode::Standard).unwrap();
        assert_eq!(dest(&mv(&pos, "O-O")), Square::G1);
        assert_eq!(dest(&mv(&pos, "O-O-O")), Square::C1);
    }
}

/// Round rating badge. Symbols the default font lacks (star, check, thumb, book) are drawn as shapes.
fn draw_icon(p: &egui::Painter, c: Pos2, r: f32, class: Class) {
    let white = Color32::WHITE;
    p.circle_filled(c, r, class.color());
    p.circle_stroke(c, r, Stroke::new(r * 0.12, white));
    let u = r * 0.58; // unit length inside the badge
    let pt = |x: f32, y: f32| c + Vec2::new(x * u, y * u);
    let poly = |pts: Vec<Pos2>| p.add(egui::Shape::convex_polygon(pts, white, Stroke::NONE));
    match class {
        Class::Best => {
            let ring: Vec<Pos2> = (0..10).map(|k| {
                let (a, rad) = (-std::f32::consts::FRAC_PI_2 + k as f32 * std::f32::consts::PI / 5.0, if k % 2 == 0 { 1.15 } else { 0.5 });
                pt(a.cos() * rad, a.sin() * rad)
            }).collect();
            for k in 0..10 { poly(vec![c, ring[k], ring[(k + 1) % 10]]); }
        }
        Class::Excellent => { // thumbs up
            p.rect_filled(Rect::from_two_pos(pt(-1.0, -0.1), pt(-0.55, 0.9)), 1.0, white);
            p.rect_filled(Rect::from_two_pos(pt(-0.4, 0.0), pt(1.0, 0.9)), u * 0.2, white);
            poly(vec![pt(-0.35, 0.05), pt(0.1, -1.0), pt(0.45, -0.95), pt(0.5, 0.05)]);
        }
        Class::Good => { // check mark
            let s = Stroke::new(u * 0.38, white);
            p.line_segment([pt(-0.75, 0.05), pt(-0.2, 0.65)], s);
            p.line_segment([pt(-0.2, 0.65), pt(0.8, -0.6)], s);
        }
        Class::Book => { // open book
            poly(vec![pt(-1.05, -0.55), pt(-0.05, -0.3), pt(-0.05, 0.75), pt(-1.05, 0.5)]);
            poly(vec![pt(1.05, -0.55), pt(0.05, -0.3), pt(0.05, 0.75), pt(1.05, 0.5)]);
        }
        _ => {
            let g = class.glyph();
            p.text(c, egui::Align2::CENTER_CENTER, g, egui::FontId::proportional(r * if g.len() > 1 { 1.05 } else { 1.3 }), white);
        }
    }
}

/// Every legal move with the position after it (kind: 0 normal, 1 checkmate, 2 other game end).
/// Lets engines without MultiPV still give three suggestions.
fn children(pos: &Chess) -> Vec<(String, String, u8)> {
    pos.legal_moves().iter().map(|m| {
        let next = pos.clone().play(m.clone()).unwrap();
        let kind = if next.is_checkmate() { 1 } else if next.is_game_over() { 2 } else { 0 };
        (m.to_uci(CastlingMode::Standard).to_string(), fen_of(&next), kind)
    }).collect()
}
