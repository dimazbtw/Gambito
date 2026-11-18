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

#[derive(Clone)]
struct Hint { san: String, text: String, line: String, ev: String, white_cp: i32, role: Role, color: Color, from: Square, to: Square }

/// Who played a loaded game (from PGN headers or chess.com).
struct GameInfo { white: String, black: String, white_elo: Option<u32>, black_elo: Option<u32>, result: String, cc_acc: Option<(f64, f64)> }

struct Import { grid: vision::Grid, sils: vision::Sils, white_top: bool, white_to_move: bool, edit: bool } // edit: opened by the board editor, not from a screenshot

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
struct Review { rows: Vec<Row>, acc: [f64; 2], counts: [[u32; 9]; 2], series: Vec<f32> } // index 0 = white, 1 = black

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
    tab: u8,                   // 0 Principal, 1 Importar, 2 Análise (opened from Principal)
    expanded: [bool; 3],       // which suggestion lines show their explanation
    show_settings: bool,
    show_picture: bool,
    fen_text: String,
    fen_editing: bool,
    logo: egui::TextureHandle, // the logo, black made transparent
    playing: bool,             // autoplay through the game
    play_t: f64,
    game_idx: Option<(u8, usize)>, // which list the open game came from: (0 chess.com / 1 PGN, index)
    shot_requested: bool,      // dev only: GAMBITO_SHOT
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
        setup_style(&ctx);
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
            elo: 1500, hints_on: true, hint_elo: 2200, tab: 0, expanded: [true, false, false], show_settings: false, show_picture: false, fen_text: String::new(), fen_editing: false, logo: load_logo(&ctx), playing: false, play_t: 0.0, game_idx: None, shot_requested: false, eval_cp: 0, eval_mate: false, both: false, hints: vec![], waiting_hint: false, thinking: false, msg: String::new(),
            player: Engine::new(&exe, tx.clone(), ctx.clone()),
            analyst: Engine::new(&exe, tx.clone(), ctx.clone()),
            tx, ctx, engines, opp: 0, ana: 0, time: 0.0, anim: None, badge_t: (usize::MAX, 0.0), hints_t0: 0.0, skip_anim: false,
            rx, tex, tpl: vision::Templates::new(&svgs), import: None, brush: None, loaded: None, pgn_text: String::new(), pgn_games: vec![], cc_user: String::new(), cc_query: String::new(),
            cc_games: vec![], cc_archives: vec![], cc_loaded: 0, cc_busy: false, cc_err: String::new(), net_tx, net_rx, marks: vec![], mark_from: None,
        };
        app.after_move();
        // dev hooks for checking the look without touching the screen: GAMBITO_PGN=<file> GAMBITO_TAB=<0|1|2> GAMBITO_SHOT=<png>
        if let Some(t) = std::env::var_os("GAMBITO_PGN").and_then(|p| std::fs::read_to_string(p).ok()) { app.try_pgn(&t); }
        if let Ok(t) = std::env::var("GAMBITO_TAB") { app.tab = t.parse().unwrap_or(0); }
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
        // white's winning chances after each position, for the charts
        let series: Vec<f32> = (0..=self.mvs.len()).map(|i| {
            let pos = self.pos_at(i);
            match &ev[i] {
                Some((cp, _, _)) => win_pct((if pos.turn() == Color::White { *cp } else { -*cp }) as f64) as f32,
                None => if pos.is_checkmate() { if pos.turn() == Color::White { 0.0 } else { 100.0 } } else { 50.0 },
            }
        }).collect();
        Review { rows, acc: [sum[0] / n[0].max(1) as f64, sum[1] / n[1].max(1) as f64], counts, series }
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
                        Some(Hint { san, text, line: numbered(&shown, &sans), ev, white_cp, role: first.role(), color: stm, from: first.from()?, to: dest(&first) })
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
                        if self.loaded.is_some() { self.msg.clear(); } // the "Avaliação concluída" chip says it
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
                self.import = Some(Import { grid: scan.grid, sils: scan.sils, white_top, white_to_move: !white_top, edit: false });
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
            if INNER_COORDS && at_left { p.text(r0.left_top() + Vec2::new(3.0, 2.0), egui::Align2::LEFT_TOP, sq.rank().char(), font.clone(), txt); }
            if INNER_COORDS && at_bottom { p.text(r0.right_bottom() - Vec2::new(3.0, 1.0), egui::Align2::RIGHT_BOTTOM, sq.file().char(), font, txt); }
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
        self.dev_screenshot(ctx);
        if self.playing {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
            if self.time - self.play_t > 0.9 {
                self.play_t = self.time;
                let cur = self.view.unwrap_or(self.hist.len());
                if cur >= self.hist.len() { self.playing = false; } else { self.goto(cur + 1); if self.view.is_none() { self.playing = false; } }
            }
        }
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
        // columns laid out by hand: menu | board | panel
        let full = ui.max_rect();
        let inside = |r: Rect| egui::UiBuilder::new().max_rect(r);
        ui.painter().rect_filled(full, 0.0, BG);
        let rail_rect = Rect::from_min_max(full.min, Pos2::new(full.left() + 96.0, full.bottom()));
        let content = Rect::from_min_max(Pos2::new(rail_rect.right(), full.top()), full.max);
        ui.painter().rect_filled(rail_rect, 0.0, RAIL);
        ui.painter().line_segment([rail_rect.right_top(), rail_rect.right_bottom()], Stroke::new(1.0, Color32::from_white_alpha(14)));
        ui.scope_builder(inside(rail_rect), |ui| { ui.set_clip_rect(rail_rect); self.rail(ui, rail_rect) });

        let (m, top_h) = (20.0, 52.0);
        let panel_w = (content.width() * 0.42).clamp(430.0, 600.0);
        let panel_rect = Rect::from_min_max(Pos2::new(content.right() - m - panel_w, content.top() + top_h), Pos2::new(content.right() - m, content.bottom() - m));
        let board_area = Rect::from_min_max(Pos2::new(content.left() + 22.0, content.top() + 12.0), Pos2::new(panel_rect.left() - 22.0, content.bottom() - 14.0));
        let bar = Rect::from_min_max(Pos2::new(panel_rect.left(), content.top() + 8.0), Pos2::new(panel_rect.right(), content.top() + top_h - 6.0));
        ui.scope_builder(inside(bar), |ui| self.top_bar(ui));
        ui.scope_builder(inside(board_area), |ui| { ui.set_clip_rect(board_area.expand(3.0)); self.center(ui, board_area) });
        self.panel(ui, panel_rect);
        self.settings_window(ui.ctx());

        // keep repainting only while something is animating
        let t = self.time;
        if self.anim.map_or(false, |a| t - a.t0 < ANIM_SECS + 0.05) || t - self.badge_t.1 < 0.35 || t - self.hints_t0 < 0.35 {
            ui.ctx().request_repaint();
        }
    }
}

// ---------- look: black, gold and cream (see the design canvas) ----------
const BG: Color32 = Color32::from_rgb(10, 10, 11);
const RAIL: Color32 = Color32::from_rgb(13, 13, 15);
const PANEL: Color32 = Color32::from_rgb(17, 17, 19);
const CARD: Color32 = Color32::from_rgb(22, 22, 25);
const CARD2: Color32 = Color32::from_rgb(28, 28, 32);
const ROW_ALT: Color32 = Color32::from_rgb(24, 24, 27);
const LINE: Color32 = Color32::from_rgb(40, 40, 46);
const CREAM: Color32 = Color32::from_rgb(251, 243, 225);
const MUTED: Color32 = Color32::from_rgb(168, 162, 146);
const GOLD: Color32 = Color32::from_rgb(242, 193, 74);
const GOLD_HI: Color32 = Color32::from_rgb(255, 214, 92);
const GOLD_LO: Color32 = Color32::from_rgb(217, 149, 26);
const GOLD_BG: Color32 = Color32::from_rgb(40, 31, 9);
const GREEN: Color32 = Color32::from_rgb(76, 190, 88);
const POS: Color32 = Color32::from_rgb(127, 224, 138);
const TXT2: Color32 = MUTED;
const AMBER: Color32 = Color32::from_rgb(230, 190, 90);

fn f_semi(size: f32) -> egui::FontId { egui::FontId::new(size, egui::FontFamily::Name("Outfit-SemiBold".into())) }
fn f_med(size: f32) -> egui::FontId { egui::FontId::new(size, egui::FontFamily::Name("Outfit-Medium".into())) }
fn semi(t: impl Into<String>, size: f32) -> egui::RichText { egui::RichText::new(t).font(f_semi(size)) }
fn med(t: impl Into<String>, size: f32) -> egui::RichText { egui::RichText::new(t).font(f_med(size)) }

/// Rounded card that fills the width of its parent.
fn card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new().fill(CARD).stroke(Stroke::new(1.0, LINE)).corner_radius(16.0).inner_margin(14.0).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui);
    });
    ui.add_space(10.0);
}

fn piece_img(tex: &egui::TextureHandle, size: f32) -> egui::Image<'static> {
    egui::Image::new(egui::load::SizedTexture::new(tex.id(), Vec2::splat(size)))
}

/// Evaluation pill: cream when white is better, dark when black is.
fn eval_pill(ui: &mut egui::Ui, text: &str, white_better: bool, size: f32) {
    let (bg, fg) = if white_better { (Color32::from_rgb(245, 240, 230), Color32::from_rgb(20, 20, 20)) } else { (Color32::from_rgb(52, 52, 58), CREAM) };
    egui::Frame::new().fill(bg).corner_radius(size * 0.45).inner_margin(egui::Margin::symmetric((size * 0.62) as i8, (size * 0.14) as i8)).show(ui, |ui| {
        ui.label(semi(text, size).color(fg));
    });
}

#[derive(Clone, Copy, PartialEq)]
enum Ico { Home, Bars, Book, Puzzle, Cap, Gear, ChevL, ChevR, ChevD, SkipPrev, SkipNext, Play, Pause, Flip, Restart, Sliders, Pencil, Image, Copy, Star, CheckCircle, Target, Search, Download, Kebab }

/// Small vector icons, drawn with strokes so they take any colour and size.
fn ico(p: &egui::Painter, k: Ico, c: Pos2, s: f32, col: Color32) {
    let u = s / 24.0;
    let pt = |x: f32, y: f32| c + Vec2::new((x - 12.0) * u, (y - 12.0) * u);
    let sw = Stroke::new((1.8 * u).max(1.2), col);
    let line = |pts: &[(f32, f32)]| { p.add(egui::Shape::line(pts.iter().map(|&(x, y)| pt(x, y)).collect(), sw)); };
    let closed = |pts: &[(f32, f32)]| { p.add(egui::Shape::closed_line(pts.iter().map(|&(x, y)| pt(x, y)).collect(), sw)); };
    let fill = |pts: &[(f32, f32)]| { p.add(egui::Shape::convex_polygon(pts.iter().map(|&(x, y)| pt(x, y)).collect(), col, Stroke::NONE)); };
    let rect_s = |a: (f32, f32), b: (f32, f32), r: f32| { p.rect_stroke(Rect::from_two_pos(pt(a.0, a.1), pt(b.0, b.1)), r * u, sw, egui::StrokeKind::Middle); };
    match k {
        Ico::Home => { line(&[(4.0, 11.0), (12.0, 4.0), (20.0, 11.0)]); line(&[(6.0, 10.0), (6.0, 19.0), (18.0, 19.0), (18.0, 10.0)]); line(&[(10.0, 19.0), (10.0, 14.0), (14.0, 14.0), (14.0, 19.0)]); }
        Ico::Bars => { for (x, y, h) in [(4.0, 13.0, 7.0), (10.0, 8.0, 12.0), (16.0, 4.0, 16.0)] { p.rect_filled(Rect::from_two_pos(pt(x, y), pt(x + 4.0, y + h)), 1.0 * u, col); } }
        Ico::Book => { line(&[(3.0, 5.5), (12.0, 6.5), (12.0, 19.5), (3.0, 18.5), (3.0, 5.5)]); line(&[(21.0, 5.5), (12.0, 6.5), (12.0, 19.5), (21.0, 18.5), (21.0, 5.5)]); }
        Ico::Puzzle => { rect_s((4.0, 8.0), (17.0, 20.0), 2.0); p.circle_filled(pt(10.5, 6.0), 2.8 * u, col); p.circle_filled(pt(19.0, 14.0), 2.8 * u, col); }
        Ico::Cap => { closed(&[(2.0, 9.0), (12.0, 4.0), (22.0, 9.0), (12.0, 14.0)]); line(&[(6.0, 11.5), (6.0, 16.0), (12.0, 19.0), (18.0, 16.0), (18.0, 11.5)]); }
        Ico::Gear => {
            p.circle_stroke(pt(12.0, 12.0), 3.2 * u, sw);
            for i in 0..8 { let a = i as f32 * std::f32::consts::FRAC_PI_4; line(&[(12.0 + a.cos() * 6.2, 12.0 + a.sin() * 6.2), (12.0 + a.cos() * 9.2, 12.0 + a.sin() * 9.2)]); }
        }
        Ico::ChevL => line(&[(15.0, 5.0), (8.0, 12.0), (15.0, 19.0)]),
        Ico::ChevR => line(&[(9.0, 5.0), (16.0, 12.0), (9.0, 19.0)]),
        Ico::ChevD => line(&[(6.0, 9.0), (12.0, 15.0), (18.0, 9.0)]),
        Ico::SkipPrev => { p.rect_filled(Rect::from_two_pos(pt(4.0, 5.0), pt(6.5, 19.0)), 1.0 * u, col); fill(&[(19.0, 5.0), (19.0, 19.0), (8.5, 12.0)]); }
        Ico::SkipNext => { p.rect_filled(Rect::from_two_pos(pt(17.5, 5.0), pt(20.0, 19.0)), 1.0 * u, col); fill(&[(5.0, 5.0), (5.0, 19.0), (15.5, 12.0)]); }
        Ico::Play => fill(&[(7.0, 4.5), (7.0, 19.5), (19.0, 12.0)]),
        Ico::Pause => { p.rect_filled(Rect::from_two_pos(pt(6.5, 5.0), pt(10.5, 19.0)), 1.0 * u, col); p.rect_filled(Rect::from_two_pos(pt(13.5, 5.0), pt(17.5, 19.0)), 1.0 * u, col); }
        Ico::Flip => { fill(&[(12.0, 3.0), (18.0, 10.0), (6.0, 10.0)]); fill(&[(12.0, 21.0), (6.0, 14.0), (18.0, 14.0)]); }
        Ico::Restart => {
            let pts: Vec<(f32, f32)> = (0..=16).map(|i| { let a = 0.7 + i as f32 * 0.3; (12.0 + a.cos() * 8.0, 12.0 + a.sin() * 8.0) }).collect();
            let end = *pts.last().unwrap();
            line(&pts);
            fill(&[(end.0 - 1.0, end.1 - 6.0), (end.0 - 1.0, end.1 + 4.0), (end.0 + 6.0, end.1 - 1.0)]);
        }
        Ico::Sliders => { line(&[(4.0, 7.0), (14.0, 7.0)]); line(&[(18.0, 7.0), (20.0, 7.0)]); line(&[(4.0, 17.0), (6.0, 17.0)]); line(&[(10.0, 17.0), (20.0, 17.0)]); p.circle_stroke(pt(16.0, 7.0), 2.0 * u, sw); p.circle_stroke(pt(8.0, 17.0), 2.0 * u, sw); }
        Ico::Pencil => { closed(&[(4.0, 20.0), (8.0, 20.0), (19.0, 9.0), (15.0, 5.0), (4.0, 16.0)]); line(&[(13.0, 7.0), (17.0, 11.0)]); }
        Ico::Image => { rect_s((3.5, 4.5), (20.5, 19.5), 2.5); p.circle_stroke(pt(9.0, 10.0), 1.8 * u, sw); line(&[(20.0, 16.0), (15.0, 11.5), (7.0, 19.5)]); }
        Ico::Copy => { rect_s((8.0, 8.0), (20.0, 20.0), 2.0); line(&[(16.0, 8.0), (16.0, 6.0), (14.0, 4.0), (6.0, 4.0), (4.0, 6.0), (4.0, 14.0), (6.0, 16.0), (8.0, 16.0)]); }
        Ico::Star => {
            let ring: Vec<(f32, f32)> = (0..10).map(|i| { let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0; let r = if i % 2 == 0 { 10.0 } else { 4.4 }; (12.0 + a.cos() * r, 12.5 + a.sin() * r) }).collect();
            for i in 0..10 { fill(&[(12.0, 12.5), ring[i], ring[(i + 1) % 10]]); }
        }
        Ico::CheckCircle => { p.circle_stroke(pt(12.0, 12.0), 9.0 * u, sw); line(&[(8.0, 12.5), (11.0, 15.5), (16.0, 9.5)]); }
        Ico::Target => { p.circle_stroke(pt(12.0, 12.0), 7.0 * u, sw); p.circle_stroke(pt(12.0, 12.0), 2.5 * u, sw); for (a, b) in [((12.0, 2.0), (12.0, 5.0)), ((12.0, 19.0), (12.0, 22.0)), ((2.0, 12.0), (5.0, 12.0)), ((19.0, 12.0), (22.0, 12.0))] { line(&[a, b]); } }
        Ico::Search => { p.circle_stroke(pt(11.0, 11.0), 6.5 * u, sw); line(&[(16.0, 16.0), (20.5, 20.5)]); }
        Ico::Download => { line(&[(12.0, 4.0), (12.0, 14.0)]); line(&[(8.0, 10.5), (12.0, 14.5), (16.0, 10.5)]); line(&[(5.0, 19.0), (19.0, 19.0)]); }
        Ico::Kebab => { for y in [5.0, 12.0, 19.0] { p.circle_filled(pt(12.0, y), 1.8 * u, col); } }
    }
}

/// Gold button (primary): a raised face with a light top, like the design.
fn gold_btn(ui: &mut egui::Ui, label: &str, icon: Option<Ico>, size: Vec2) -> bool {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let p = ui.painter();
    let down = resp.is_pointer_button_down_on();
    let base = if resp.hovered() { Color32::from_rgb(247, 200, 82) } else { Color32::from_rgb(240, 178, 42) };
    p.rect_filled(rect.expand(2.0), 16.0, Color32::from_rgba_unmultiplied(232, 176, 48, if resp.hovered() { 46 } else { 26 }));
    p.rect_filled(rect, 14.0, base);
    let top = Rect::from_min_size(rect.min + Vec2::new(1.0, 1.0), Vec2::new(rect.width() - 2.0, rect.height() * 0.5));
    p.rect_filled(top, egui::CornerRadius { nw: 13, ne: 13, sw: 0, se: 0 }, Color32::from_rgba_unmultiplied(255, 232, 150, 70));
    p.rect_stroke(rect, 14.0, Stroke::new(1.0, GOLD_HI), egui::StrokeKind::Inside);
    let ink = Color32::from_rgb(28, 20, 4);
    let tw = p.layout_no_wrap(label.to_string(), f_semi(size.y * 0.36), ink).size().x;
    let total = tw + icon.map_or(0.0, |_| size.y * 0.5 + 10.0);
    let mut x = rect.center().x - total / 2.0;
    let cy = rect.center().y + if down { 1.0 } else { 0.0 };
    if let Some(i) = icon { ico(p, i, Pos2::new(x + size.y * 0.25, cy), size.y * 0.46, ink); x += size.y * 0.5 + 10.0; }
    p.text(Pos2::new(x, cy), egui::Align2::LEFT_CENTER, label, f_semi(size.y * 0.36), ink);
    resp.clicked()
}

/// Dark button; `outline` turns it into the gold-outlined variant.
fn dark_btn(ui: &mut egui::Ui, label: &str, icon: Option<Ico>, size: Vec2, outline: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let p = ui.painter();
    let fill = if outline { if resp.hovered() { GOLD_BG } else { Color32::TRANSPARENT } } else if resp.hovered() { Color32::from_rgb(36, 36, 41) } else { Color32::from_rgb(26, 26, 30) };
    let (stroke, fg) = if outline { (Stroke::new(1.5, Color32::from_rgb(232, 176, 48)), GOLD) } else { (Stroke::new(1.0, Color32::from_rgb(54, 54, 60)), CREAM) };
    p.rect_filled(rect, 14.0, fill);
    p.rect_stroke(rect, 14.0, stroke, egui::StrokeKind::Inside);
    let fs = (size.y * 0.34).min(19.0);
    let tw = p.layout_no_wrap(label.to_string(), f_med(fs), fg).size().x;
    let total = tw + icon.map_or(0.0, |_| fs * 1.3 + 8.0);
    let mut x = rect.center().x - total / 2.0;
    if let Some(i) = icon { ico(p, i, Pos2::new(x + fs * 0.65, rect.center().y), fs * 1.25, fg); x += fs * 1.3 + 8.0; }
    p.text(Pos2::new(x, rect.center().y), egui::Align2::LEFT_CENTER, label, f_med(fs), fg);
    resp.clicked()
}

/// Square icon button; `active` gives it the gold highlight.
fn icon_btn(ui: &mut egui::Ui, k: Ico, size: f32, active: bool, tip: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let p = ui.painter();
    let (fill, stroke, fg) = if active { (GOLD_BG, Stroke::new(1.5, Color32::from_rgb(232, 176, 48)), GOLD) }
        else { (if resp.hovered() { Color32::from_rgb(34, 34, 39) } else { Color32::from_rgb(24, 24, 28) }, Stroke::new(1.0, Color32::from_rgb(52, 52, 58)), CREAM) };
    p.rect_filled(rect, 14.0, fill);
    p.rect_stroke(rect, 14.0, stroke, egui::StrokeKind::Inside);
    ico(p, k, rect.center(), size * 0.46, fg);
    resp.on_hover_text(tip).clicked()
}

/// Bare icon (top bar).
fn icon_only(ui: &mut egui::Ui, k: Ico, tip: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::click());
    ico(ui.painter(), k, rect.center(), 24.0, if resp.hovered() { GOLD } else { CREAM });
    resp.on_hover_text(tip).clicked()
}

/// Green numbered circle of a suggestion line.
fn num_badge(ui: &mut egui::Ui, n: usize) {
    let (r, _) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
    ui.painter().circle_filled(r.center(), 15.0, Color32::from_rgb(46, 158, 60));
    ui.painter().circle_filled(r.center() - Vec2::new(0.0, 1.5), 13.5, GREEN);
    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, n.to_string(), f_semi(16.0), Color32::WHITE);
}

/// "1. e4 e5 2. Nf3" (or "1... e5 2. Nf3" when black is to move)
fn numbered(shown: &Chess, sans: &[String]) -> String {
    let mut n = shown.fullmoves().get();
    let mut white = shown.turn() == Color::White;
    let mut out: Vec<String> = vec![];
    for (i, s) in sans.iter().enumerate() {
        if white { out.push(format!("{n}. {s}")); } else {
            out.push(if i == 0 { format!("{n}... {s}") } else { s.clone() });
            n += 1;
        }
        white = !white;
    }
    out.join(" ")
}

/// Line chart of white's winning chances (0..100) with a soft gold area; `mark` is the ply to highlight.
fn win_chart(ui: &mut egui::Ui, series: &[f32], size: Vec2, mark: Option<usize>, small: bool) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter_at(rect);
    if small { p.rect_filled(rect, 12.0, CARD2); p.rect_stroke(rect, 12.0, Stroke::new(1.0, LINE), egui::StrokeKind::Inside); }
    if series.len() < 2 { return; }
    let pad = if small { 6.0 } else { 4.0 };
    let (w, h) = (rect.width() - 2.0 * pad, rect.height() - 2.0 * pad);
    let pt = |i: usize| Pos2::new(rect.left() + pad + w * i as f32 / (series.len() - 1) as f32, rect.top() + pad + h * (1.0 - series[i] / 100.0));
    // area
    let mut mesh = egui::Mesh::default();
    let (top_c, bot_c) = (Color32::from_rgba_unmultiplied(242, 193, 74, 70), Color32::from_rgba_unmultiplied(242, 193, 74, 0));
    for i in 0..series.len() - 1 {
        let (a, b) = (pt(i), pt(i + 1));
        let base = mesh.vertices.len() as u32;
        for (pos, c) in [(a, top_c), (b, top_c), (Pos2::new(b.x, rect.bottom() - pad), bot_c), (Pos2::new(a.x, rect.bottom() - pad), bot_c)] {
            mesh.vertices.push(egui::epaint::Vertex { pos, uv: egui::epaint::WHITE_UV, color: c });
        }
        mesh.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    p.add(egui::Shape::mesh(mesh));
    if !small { // the 50% line
        let y = rect.top() + pad + h * 0.5;
        let mut x = rect.left() + pad;
        while x < rect.right() - pad { p.line_segment([Pos2::new(x, y), Pos2::new((x + 4.0).min(rect.right() - pad), y)], Stroke::new(1.0, Color32::from_white_alpha(60))); x += 9.0; }
    }
    let line_col = if small { Color32::from_rgb(120, 220, 110) } else { GOLD };
    p.add(egui::Shape::line((0..series.len()).map(pt).collect(), Stroke::new(if small { 2.0 } else { 2.4 }, line_col)));
    let at = mark.map(|m| m.min(series.len() - 1)).unwrap_or(series.len() - 1);
    let q = pt(at);
    if !small { p.line_segment([Pos2::new(q.x, rect.top() + pad), Pos2::new(q.x, rect.bottom() - pad)], Stroke::new(1.2, Color32::from_white_alpha(120))); }
    p.circle_filled(q, if small { 3.4 } else { 5.0 }, if small { Color32::from_rgb(239, 255, 216) } else { CREAM });
    if !small { p.circle_stroke(q, 5.0, Stroke::new(2.0, GOLD)); }
}


fn setup_style(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let faces: [(&str, &'static [u8]); 3] = [
        ("Inter", include_bytes!("../assets/fonts/Inter.ttf")),
        ("Outfit-Medium", include_bytes!("../assets/fonts/Outfit-Medium.ttf")),
        ("Outfit-SemiBold", include_bytes!("../assets/fonts/Outfit-SemiBold.ttf")),
    ];
    for (name, bytes) in faces { fonts.font_data.insert(name.to_string(), std::sync::Arc::new(egui::FontData::from_static(bytes))); }
    fonts.families.get_mut(&egui::FontFamily::Proportional).unwrap().insert(0, "Inter".into());
    for n in ["Outfit-Medium", "Outfit-SemiBold"] { fonts.families.insert(egui::FontFamily::Name(n.into()), vec![n.to_string(), "Inter".into()]); }
    ctx.set_fonts(fonts);
    ctx.set_theme(egui::ThemePreference::Dark);
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = CARD;
    v.window_stroke = Stroke::new(1.0, LINE);
    v.extreme_bg_color = Color32::from_rgb(14, 14, 16);
    v.faint_bg_color = ROW_ALT;
    v.hyperlink_color = GOLD;
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, CREAM);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, CREAM);
    v.widgets.inactive.weak_bg_fill = CARD2;
    v.widgets.inactive.bg_fill = Color32::from_rgb(44, 44, 49);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, CREAM);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(38, 38, 43);
    v.widgets.active.weak_bg_fill = GOLD_BG;
    v.widgets.active.fg_stroke = Stroke::new(1.0, GOLD);
    v.selection.bg_fill = Color32::from_rgb(88, 68, 20);
    v.selection.stroke = Stroke::new(1.0, GOLD);
    ctx.set_visuals(v);
    ctx.global_style_mut(|st| {
        use egui::TextStyle::*;
        for (ts, sz) in [(Body, 15.5), (Button, 15.5), (Small, 13.0), (Monospace, 14.0), (Heading, 22.0)] { st.text_styles.insert(ts, egui::FontId::proportional(sz)); }
        st.spacing.item_spacing = Vec2::new(8.0, 6.0);
    });
}

/// The logo, with its black background made transparent so it sits on any dark surface.
fn load_logo(ctx: &egui::Context) -> egui::TextureHandle {
    let img = image::load_from_memory(include_bytes!("../assets/logo.webp")).expect("logo").to_rgba8();
    let mut img = image::imageops::resize(&img, 640, 640, image::imageops::FilterType::Lanczos3);
    for px in img.pixels_mut() {
        let m = px[0].max(px[1]).max(px[2]);
        px[3] = if m < 16 { 0 } else if m < 44 { ((m as u32 - 16) * 255 / 28) as u8 } else { 255 };
    }
    let (w, h) = (img.width() as usize, img.height() as usize);
    ctx.load_texture("logo", egui::ColorImage::from_rgba_unmultiplied([w, h], img.as_raw()), egui::TextureOptions::LINEAR)
}

/// Window / taskbar icon: the logo's mark.
fn app_icon() -> egui::IconData {
    let img = image::load_from_memory(include_bytes!("../assets/logo.webp")).expect("logo").to_rgba8();
    let crop = image::imageops::crop_imm(&img, 340, 250, 575, 575).to_image();
    let small = image::imageops::resize(&crop, 256, 256, image::imageops::FilterType::Lanczos3);
    egui::IconData { rgba: small.into_raw(), width: 256, height: 256 }
}

const INNER_COORDS: bool = false; // coordinates are drawn outside the board now
impl App {
    fn player_name(&self, c: Color) -> String {
        if let Some(i) = &self.loaded {
            let (n, e) = if c == Color::White { (&i.white, i.white_elo) } else { (&i.black, i.black_elo) };
            return match e { Some(e) => format!("{n} ({e})"), None => n.clone() };
        }
        if self.both { return if c == Color::White { "Brancas".into() } else { "Pretas".into() }; }
        if c == self.me { "Tu".into() } else { self.engines[self.opp].name.clone() }
    }

    /// Second line under a player's name.
    fn player_sub(&self, c: Color, shown: &Chess, top: bool) -> String {
        if let Some(i) = &self.loaded { return if top { i.result.clone() } else { String::new() }; }
        let diff = explain::material(shown.board(), c) - explain::material(shown.board(), !c);
        if !self.both && c != self.me { return format!("Elo {}", self.elo); }
        if diff > 0 { format!("+{diff} de material") } else { String::new() }
    }

    fn finished(&self) -> bool { self.pos.is_game_over() || self.loaded.is_some() }

    /// Dev only: with GAMBITO_SHOT=<png>, saves a picture of this window after a few seconds and quits.
    fn dev_screenshot(&mut self, ctx: &egui::Context) {
        let Some(path) = std::env::var_os("GAMBITO_SHOT") else { return };
        ctx.request_repaint_after(std::time::Duration::from_millis(400));
        if self.time > 9.0 && !self.shot_requested {
            self.shot_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img| {
                let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
                if let Some(im) = image::RgbaImage::from_raw(img.size[0] as u32, img.size[1] as u32, bytes) { let _ = im.save(&path); }
                std::process::exit(0);
            })));
        }
    }

    /// Where the logo's parts are in the logo picture (normalised): 0 crown, 1 app icon, 2 knight.
    fn logo_uv(&self, k: u8) -> Rect {
        let (x0, y0, x1, y1) = match k { 0 => (500.0, 155.0, 755.0, 292.0), 1 => (340.0, 255.0, 915.0, 825.0), _ => (420.0, 300.0, 780.0, 640.0) };
        Rect::from_min_max(Pos2::new(x0 / 1254.0, y0 / 1254.0), Pos2::new(x1 / 1254.0, y1 / 1254.0))
    }

    /// (kind 0 chess.com / 1 PGN, index, count) of the game list the open game came from.
    fn game_pos(&self) -> Option<(u8, usize, usize)> {
        match self.game_idx {
            Some((0, i)) if !self.cc_games.is_empty() => Some((0, i, self.cc_games.len())),
            Some((1, i)) if !self.pgn_games.is_empty() => Some((1, i, self.pgn_games.len())),
            _ => None,
        }
    }

    fn open_game(&mut self, kind: u8, i: usize) {
        if kind == 0 { self.load_cc_game(i); } else { let g = self.pgn_games[i].clone(); self.load_pgn_game(&g); self.game_idx = Some((1, i)); }
    }

    /// The game as PGN text (what the download icon copies).
    fn export_pgn(&self) -> String {
        let (w, b) = self.loaded.as_ref().map(|i| (i.white.clone(), i.black.clone())).unwrap_or(("Brancas".into(), "Pretas".into()));
        let last = &self.pos;
        let res = if last.is_checkmate() { if last.turn() == Color::Black { "1-0" } else { "0-1" } }
            else if last.is_game_over() { "1/2-1/2" }
            else { match self.loaded.as_ref().map(|i| i.result.as_str()) { Some(r) if r.starts_with("Brancas ganham") => "1-0", Some(r) if r.starts_with("Pretas ganham") => "0-1", Some(r) if r.starts_with("Empate") => "1/2-1/2", _ => "*" } };
        let sans: Vec<String> = self.hist.iter().map(|h| h.1.clone()).collect();
        let start = self.pos_at(0);
        let mut s = format!("[Event \"Gambito\"]\n[Site \"?\"]\n[Date \"????.??.??\"]\n[White \"{w}\"]\n[Black \"{b}\"]\n[Result \"{res}\"]\n");
        if fen_of(start) != fen_of(&Chess::default()) { s += &format!("[SetUp \"1\"]\n[FEN \"{}\"]\n", fen_of(start)); }
        s += &format!("\n{} {res}\n", numbered(start, &sans));
        s
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let _ = icon_only(ui, Ico::Kebab, "Mais opções");
            if icon_only(ui, Ico::Download, "Copiar a partida (PGN)") && !self.mvs.is_empty() {
                ui.ctx().copy_text(self.export_pgn());
                self.msg = "PGN copiado para a área de transferência.".into();
            }
            if let Some((kind, i, n)) = self.game_pos() {
                ui.add_space(8.0);
                let (rect, _) = ui.allocate_exact_size(Vec2::new(250.0, 40.0), Sense::hover());
                let p = ui.painter();
                p.rect_filled(rect, 12.0, Color32::from_rgb(17, 17, 20));
                p.rect_stroke(rect, 12.0, Stroke::new(1.0, Color32::from_rgb(46, 46, 52)), egui::StrokeKind::Inside);
                let (l, r) = (Rect::from_min_size(rect.min, Vec2::new(48.0, 40.0)), Rect::from_min_max(Pos2::new(rect.right() - 48.0, rect.top()), rect.max));
                let (lr, rr) = (ui.interact(l, ui.id().with("gprev"), Sense::click()), ui.interact(r, ui.id().with("gnext"), Sense::click()));
                let p = ui.painter();
                ico(p, Ico::ChevL, l.center(), 20.0, if lr.hovered() { GOLD } else { CREAM });
                ico(p, Ico::ChevR, r.center(), 20.0, if rr.hovered() { GOLD } else { CREAM });
                p.text(rect.center(), egui::Align2::CENTER_CENTER, format!("Partida {} de {n}", i + 1), f_med(15.5), CREAM);
                if lr.clicked() && i > 0 { self.open_game(kind, i - 1); }
                if rr.clicked() && i + 1 < n { self.open_game(kind, i + 1); }
            }
        });
    }

    /// Board in its frame (coordinates outside), the players, the tool buttons and the playback bar.
    fn center(&mut self, ui: &mut egui::Ui, area: Rect) {
        let inside = |r: Rect| egui::UiBuilder::new().max_rect(r);
        let (pad_l, pad, pad_b, tools_w, gap) = (34.0, 12.0, 30.0, 52.0, 12.0);
        let (head_h, bot_h, play_h) = (54.0, 44.0, 62.0);
        let avail_h = area.height() - head_h - 8.0 - pad - pad_b - 8.0 - bot_h - 8.0 - play_h;
        let avail_w = area.width() - pad_l - pad - gap - tools_w;
        let side = ((avail_h.min(avail_w) / 8.0).floor() * 8.0).max(240.0);
        let frame_w = pad_l + side + pad;
        let group_w = frame_w + gap + tools_w;
        let x0 = area.left() + ((area.width() - group_w) / 2.0).max(0.0);
        let frame = Rect::from_min_size(Pos2::new(x0, area.top() + head_h + 8.0), Vec2::new(frame_w, pad + side + pad_b));
        let board = Rect::from_min_size(frame.min + Vec2::new(pad_l, pad), Vec2::splat(side));
        let (top, bottom) = if self.flipped { (Color::White, Color::Black) } else { (Color::Black, Color::White) };
        let shown = self.pos_at(self.view.unwrap_or(self.hist.len())).clone();
        let p = ui.painter().clone();

        self.player_row(&p, Rect::from_min_size(Pos2::new(x0, area.top()), Vec2::new(frame_w, head_h)), top, &shown, true);
        p.rect_filled(frame, 22.0, PANEL);
        p.rect_stroke(frame, 22.0, Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
        let sq = side / 8.0;
        for i in 0..8 {
            let (rank, file) = if self.flipped { (i + 1, 7 - i) } else { (8 - i, i) };
            p.text(Pos2::new(frame.left() + pad_l / 2.0 + 1.0, board.top() + (i as f32 + 0.5) * sq), egui::Align2::CENTER_CENTER, rank.to_string(), f_med(15.0), MUTED);
            p.text(Pos2::new(board.left() + (i as f32 + 0.5) * sq, board.bottom() + pad_b / 2.0 + 1.0), egui::Align2::CENTER_CENTER, ((b'a' + file as u8) as char).to_string(), f_med(15.0), MUTED);
        }
        ui.scope_builder(inside(board), |ui| self.board_ui(ui, side));

        // tools to the right of the frame
        let tools = Rect::from_min_size(Pos2::new(frame.right() + gap, frame.top()), Vec2::new(tools_w, 3.0 * tools_w + 20.0));
        ui.scope_builder(inside(tools), |ui| {
            ui.spacing_mut().item_spacing.y = 10.0;
            if icon_btn(ui, Ico::Flip, tools_w, true, "Virar o tabuleiro (F)") { self.flipped = !self.flipped; }
            if icon_btn(ui, Ico::Restart, tools_w, false, "Novo jogo") { let me = if self.both { Color::White } else { self.me }; self.new_game(me); }
            if icon_btn(ui, Ico::Sliders, tools_w, false, "Definições (engines, Elo)") { self.show_settings = !self.show_settings; }
        });

        let below = frame.bottom() + 8.0;
        self.player_row(&p, Rect::from_min_size(Pos2::new(x0, below), Vec2::new(frame_w, bot_h)), bottom, &shown, false);

        // playback bar
        let bar = Rect::from_min_size(Pos2::new(x0, below + bot_h + 8.0), Vec2::new(group_w, play_h));
        p.rect_filled(bar, 20.0, PANEL);
        p.rect_stroke(bar, 20.0, Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
        let inner = bar.shrink(8.0);
        let (n, gapb) = (5.3f32, 8.0f32);
        let unit = (inner.width() - 4.0 * gapb) / n;
        let widths = [unit, unit, unit * 1.3, unit, unit];
        let cur = self.view.unwrap_or(self.hist.len());
        let mut x = inner.left();
        for (k, w) in widths.iter().enumerate() {
            let r = Rect::from_min_size(Pos2::new(x, inner.top()), Vec2::new(*w, inner.height()));
            x += w + gapb;
            let resp = ui.interact(r, ui.id().with(("pb", k)), Sense::click());
            let gold = k == 2;
            if gold {
                let base = if resp.hovered() { Color32::from_rgb(247, 200, 82) } else { Color32::from_rgb(240, 178, 42) };
                p.rect_filled(r, 13.0, base);
                p.rect_filled(Rect::from_min_size(r.min + Vec2::new(1.0, 1.0), Vec2::new(r.width() - 2.0, r.height() * 0.5)), egui::CornerRadius { nw: 12, ne: 12, sw: 0, se: 0 }, Color32::from_rgba_unmultiplied(255, 232, 150, 70));
            } else {
                p.rect_filled(r, 13.0, if resp.hovered() { Color32::from_rgb(36, 36, 41) } else { Color32::from_rgb(26, 26, 30) });
                p.rect_stroke(r, 13.0, Stroke::new(1.0, Color32::from_rgb(48, 48, 54)), egui::StrokeKind::Inside);
            }
            let (icon, col) = match k { 0 => (Ico::SkipPrev, CREAM), 1 => (Ico::ChevL, CREAM), 2 => (if self.playing { Ico::Pause } else { Ico::Play }, Color32::from_rgb(28, 20, 4)), 3 => (Ico::ChevR, CREAM), _ => (Ico::SkipNext, CREAM) };
            ico(&p, icon, r.center(), 24.0, col);
            if resp.clicked() {
                match k {
                    0 => { self.playing = false; self.goto(0) }
                    1 => { self.playing = false; self.goto(cur.saturating_sub(1)) }
                    2 => { self.playing = !self.playing; self.play_t = self.time; if self.playing && cur >= self.hist.len() { self.goto(0) } }
                    3 => { self.playing = false; self.goto(cur + 1) }
                    _ => { self.playing = false; self.goto(usize::MAX) }
                }
            }
        }
    }

    fn player_row(&self, p: &egui::Painter, rect: Rect, c: Color, shown: &Chess, top: bool) {
        let a = if top { 46.0 } else { 40.0 };
        let av = Rect::from_min_size(rect.min + Vec2::new(10.0, (rect.height() - a) / 2.0), Vec2::splat(a));
        p.rect_filled(av, 12.0, CARD2);
        p.rect_stroke(av, 12.0, Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
        p.image(self.tex[&(c, Role::King)].id(), av.shrink(a * 0.1), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        let name = self.player_name(c);
        let sub = self.player_sub(c, shown, top);
        let x = av.right() + 14.0;
        if sub.is_empty() {
            p.text(Pos2::new(x, rect.center().y), egui::Align2::LEFT_CENTER, name, f_semi(if top { 23.0 } else { 21.0 }), CREAM);
        } else {
            p.text(Pos2::new(x, rect.center().y - 10.0), egui::Align2::LEFT_CENTER, name, f_semi(if top { 23.0 } else { 21.0 }), CREAM);
            p.text(Pos2::new(x, rect.center().y + 13.0), egui::Align2::LEFT_CENTER, sub, egui::FontId::proportional(14.0), MUTED);
        }
    }

    /// Left menu: the crown, the sections and (for now unavailable ones) greyed entries.
    fn rail(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let p = ui.painter_at(rect);
        p.image(self.logo.id(), Rect::from_min_size(Pos2::new(rect.left() - 70.0, rect.bottom() - 190.0), Vec2::new(220.0, 208.0)), self.logo_uv(2), Color32::from_white_alpha(30));
        p.image(self.logo.id(), Rect::from_center_size(Pos2::new(rect.center().x, rect.top() + 36.0), Vec2::new(52.0, 28.0)), self.logo_uv(0), Color32::WHITE);
        let items: [(Ico, &str, Option<u8>); 6] = [
            (Ico::Home, "Início", None), (Ico::Bars, "Analisar", Some(0)), (Ico::Book, "Partidas", Some(1)),
            (Ico::Puzzle, "Exercícios", None), (Ico::Cap, "Aprender", None), (Ico::Gear, "Definições", Some(9)),
        ];
        let mut y = rect.top() + 78.0;
        for (k, label, target) in items {
            let r = Rect::from_min_size(Pos2::new(rect.left() + 9.0, y), Vec2::new(rect.width() - 18.0, 76.0));
            y += 82.0;
            let resp = ui.interact(r, ui.id().with(label), Sense::click());
            let selected = match target { Some(0) => self.tab == 0 || self.tab == 2, Some(1) => self.tab == 1, Some(9) => self.show_settings, _ => false };
            let enabled = target.is_some();
            if selected {
                p.rect_filled(r, 16.0, GOLD_BG);
                p.rect_stroke(r, 16.0, Stroke::new(1.5, Color32::from_rgb(232, 176, 48)), egui::StrokeKind::Inside);
            } else if resp.hovered() && enabled {
                p.rect_filled(r, 16.0, Color32::from_rgb(22, 22, 25));
            }
            let col = if selected { GOLD } else if enabled { Color32::from_rgb(190, 184, 168) } else { Color32::from_rgb(98, 94, 84) };
            ico(&p, k, Pos2::new(r.center().x, r.top() + 28.0), 28.0, col);
            p.text(Pos2::new(r.center().x, r.bottom() - 16.0), egui::Align2::CENTER_CENTER, label, f_med(13.5), col);
            if enabled && resp.clicked() {
                match target { Some(0) => self.tab = 0, Some(1) => self.tab = 1, _ => self.show_settings = !self.show_settings }
            } else if !enabled {
                resp.on_hover_text("Em breve");
            }
        }
    }

    fn panel(&mut self, ui: &mut egui::Ui, rect: Rect) {
        ui.painter().rect_filled(rect, 24.0, PANEL);
        ui.painter().rect_stroke(rect, 24.0, Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
        let inside = |r: Rect| egui::UiBuilder::new().max_rect(r);
        let inner = rect.shrink2(Vec2::new(16.0, 14.0));
        let pinned_h = if self.tab == 0 { 120.0 } else { 0.0 };
        let scroll = Rect::from_min_max(inner.min, Pos2::new(inner.right(), inner.bottom() - pinned_h));
        ui.scope_builder(inside(scroll), |ui| {
            ui.set_clip_rect(scroll);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                // room on the right so the scroll bar never covers the cards
                egui::Frame::new().inner_margin(egui::Margin { left: 0, right: 12, top: 0, bottom: 6 }).show(ui, |ui| {
                    match self.tab { 0 => self.view_main(ui), 1 => self.view_import(ui), _ => self.view_analysis(ui) }
                });
            });
        });
        if self.tab == 0 {
            let pinned = Rect::from_min_max(Pos2::new(inner.left(), inner.bottom() - pinned_h), inner.max);
            ui.scope_builder(inside(pinned), |ui| self.pinned_bottom(ui));
        }
    }

    /// (title, text) of the card at the top: what the last move did, or a hint.
    fn hero_text(&self, idx: usize) -> (String, String) {
        if idx > 0 {
            let (prev, mv) = (self.pos_at(idx - 1), &self.mvs[idx - 1]);
            let (san, why, _) = explain(prev, mv);
            let tag = match self.quality.get(idx - 1).and_then(|x| x.as_ref()) {
                Some((c, Some(best))) => format!("  ·  {} (melhor era {best})", c.label()),
                Some((c, None)) => format!("  ·  {}", c.label()),
                None => String::new(),
            };
            return (format!("{san}{tag}"), why);
        }
        if self.loaded.is_some() || self.pos.is_game_over() { ("Partida pronta.".into(), "Usa as setas ← → para percorrer a partida e vê a Análise.".into()) }
        else { ("Joga um lance.".into(), "Eu digo o que ele faz e qual era o melhor.".into()) }
    }

    fn view_main(&mut self, ui: &mut egui::Ui) {
        let idx = self.view.unwrap_or(self.hist.len());
        let w = ui.available_width();

        // hero: what just happened
        let (title, text) = self.hero_text(idx);
        let (hero, _) = ui.allocate_exact_size(Vec2::new(w, 92.0), Sense::hover());
        {
            let p = ui.painter_at(hero);
            p.rect_filled(hero, 16.0, Color32::from_rgb(24, 24, 27));
            let cell = 26.0;
            for cx in 0..9 { for cy in 0..5 {
                if (cx + cy) % 2 == 0 {
                    let a = ((cx as f32 / 9.0).powf(1.6) * 130.0) as u8;
                    let r = Rect::from_min_size(Pos2::new(hero.right() - 9.0 * cell + cx as f32 * cell + (cy as f32 * 5.0), hero.top() + cy as f32 * cell - 6.0), Vec2::splat(cell));
                    p.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(214, 160, 30, a));
                }
            } }
            p.rect_stroke(hero, 16.0, Stroke::new(1.0, Color32::from_rgb(46, 46, 52)), egui::StrokeKind::Inside);
        }
        ui.scope_builder(egui::UiBuilder::new().max_rect(hero.shrink2(Vec2::new(16.0, 10.0))), |ui| {
            ui.horizontal(|ui| {
                ui.add(piece_img(&self.tex[&(Color::White, Role::Knight)], 56.0));
                ui.vertical(|ui| {
                    ui.set_max_width(hero.width() * 0.66);
                    ui.add(egui::Label::new(semi(title, 21.0)).truncate());
                    ui.add(egui::Label::new(egui::RichText::new(text).size(14.0).color(Color32::from_rgb(216, 209, 191))));
                });
            });
        });
        ui.add_space(8.0);

        // result / status
        ui.label(semi(self.status(), 20.0));
        if let Some(i) = &self.loaded { ui.label(egui::RichText::new(format!("{} vs {}", i.white, i.black)).size(15.0).color(MUTED)); }
        let mut chips: Vec<(String, Color32)> = vec![];
        if let Some(i) = &self.loaded { if !i.result.is_empty() { chips.push((i.result.clone(), CREAM)); } }
        if self.reviewing { chips.push(("A avaliar…".into(), MUTED)); } else if self.review.is_some() { chips.push(("✓ Avaliação concluída.".into(), POS)); }
        if let Some((eco, name)) = self.opening(idx) { chips.push((format!("{eco} · {name}"), Color32::from_rgb(220, 200, 140))); }
        if !chips.is_empty() {
            ui.horizontal_wrapped(|ui| { for (t, c) in chips { ui.label(egui::RichText::new(t).size(14.0).color(c)); ui.add_space(6.0); } });
        }
        if !self.msg.is_empty() { ui.label(egui::RichText::new(&self.msg).color(AMBER)); }
        if self.view.is_some() { ui.label(egui::RichText::new(format!("A ver o lance {idx} de {}  (End = voltar ao fim)", self.hist.len())).small().color(Color32::from_rgb(130, 180, 240))); }
        ui.add_space(6.0);

        if self.finished() {
            if gold_btn(ui, "Análise da partida", Some(Ico::Bars), Vec2::new(ui.available_width(), 52.0)) { self.tab = 2; }
            ui.add_space(10.0);
        }

        if self.show_picture || self.import.is_some() { self.picture_cards(ui); }
        self.hints_block(ui);

        // moves
        card(ui, |ui| {
            ui.label(med("Lances", 17.0));
            ui.add_space(4.0);
            let mut jump = None;
            let rows = self.hist.len().div_ceil(2);
            egui::ScrollArea::vertical().id_salt("moves").max_height(170.0).auto_shrink([false, true]).stick_to_bottom(true).show(ui, |ui| {
                for row in 0..rows {
                    egui::Frame::new().fill(if row % 2 == 1 { ROW_ALT } else { Color32::TRANSPARENT }).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            ui.add_sized([30.0, 24.0], egui::Label::new(egui::RichText::new(format!("{}.", row + 1)).color(MUTED)));
                            for k in 0..2 {
                                let i = row * 2 + k;
                                if i >= self.hist.len() { break; }
                                let q = self.quality.get(i).and_then(|x| x.as_ref()).map(|x| x.0);
                                let mut rt = egui::RichText::new(format!("{}{}", self.hist[i].1, q.map_or("", |c| c.sym())));
                                if let Some(c) = q { rt = rt.color(c.color()); }
                                if ui.add_sized([110.0, 24.0], egui::Button::new(rt).frame(false).selected(idx == i + 1)).clicked() { jump = Some(i + 1); }
                            }
                        });
                    });
                }
                if self.hist.is_empty() { ui.label(egui::RichText::new("Ainda sem lances.").color(MUTED)); }
            });
            if let Some(j) = jump { self.playing = false; self.goto(j); }
            ui.add_space(8.0);
            let bw = (ui.available_width() - 20.0) / 3.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                if dark_btn(ui, "Desfazer", None, Vec2::new(bw, 40.0), false) { self.undo(); }
                if dark_btn(ui, "Novo (B)", None, Vec2::new(bw, 40.0), false) { self.new_game(Color::White); }
                if dark_btn(ui, "Novo (P)", None, Vec2::new(bw, 40.0), false) { self.new_game(Color::Black); }
            });
            ui.label(egui::RichText::new("Novo (B) joga de brancas · Novo (P) joga de pretas").small().color(MUTED));
        });
    }

    fn hints_block(&mut self, ui: &mut egui::Ui) {
        if !self.hints_on {
            card(ui, |ui| {
                ui.label(egui::RichText::new("Sugestões desligadas").color(MUTED));
                if ui.checkbox(&mut self.hints_on, "Mostrar as 3 melhores jogadas").changed() { self.request_hints(); }
            });
            return;
        }
        let idx = self.view.unwrap_or(self.hist.len());
        // best move
        if let Some(h) = self.hints.first().cloned() {
            let series = self.review.as_ref().map(|r| r.series.clone());
            egui::Frame::new().fill(Color32::from_rgb(18, 18, 20)).stroke(Stroke::new(1.5, Color32::from_rgb(232, 176, 48))).corner_radius(18.0)
                .inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
                        ico(ui.painter(), Ico::Star, r.center(), 24.0, GOLD);
                        ui.label(med("Melhor jogada", 17.0));
                    });
                    ui.add_space(6.0);
                    let total = ui.available_width();
                    let spark_w = if series.is_some() { (total * 0.40).clamp(140.0, 230.0) } else { 0.0 };
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        let left_w = total - spark_w - if spark_w > 0.0 { 10.0 } else { 0.0 };
                        egui::Frame::new().fill(CARD2).stroke(Stroke::new(1.0, LINE)).corner_radius(13.0).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
                            ui.set_width(left_w - 26.0);
                            ui.horizontal(|ui| {
                                ui.add(piece_img(&self.tex[&(h.color, h.role)], 34.0));
                                ui.label(semi(&h.san, 32.0));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| eval_pill(ui, &h.ev, h.white_cp >= 0, 20.0));
                            });
                        });
                        if let Some(s) = &series { win_chart(ui, s, Vec2::new(spark_w, 62.0), Some(idx), true); }
                    });
                });
        } else {
            card(ui, |ui| { ui.label(egui::RichText::new(if self.waiting_hint { "A analisar…" } else { "Sem sugestões para esta posição." }).color(MUTED)); });
        }
        ui.add_space(4.0);

        // strength
        ui.horizontal(|ui| {
            ui.label(med("Elo das sugestões", 17.0));
            ui.label(egui::RichText::new(format!("(profundidade {})", self.hint_depth())).small().color(MUTED));
        });
        ui.scope(|ui| {
            ui.spacing_mut().slider_width = (ui.available_width() - 96.0).max(80.0);
            ui.visuals_mut().slider_trailing_fill = true;
            ui.visuals_mut().selection.bg_fill = GOLD;
            ui.visuals_mut().widgets.inactive.bg_fill = Color32::from_rgb(44, 44, 49);
            ui.visuals_mut().widgets.hovered.bg_fill = GOLD_HI;
            ui.visuals_mut().widgets.active.bg_fill = GOLD;
            if ui.add(egui::Slider::new(&mut self.hint_elo, 1000..=2600).step_by(50.0)).changed() { self.request_hints(); }
        });
        ui.add_space(8.0);

        // the three lines
        let mut toggle = None;
        let hints = self.hints.clone();
        for (i, h) in hints.iter().enumerate() {
            egui::Frame::new().fill(CARD).stroke(Stroke::new(1.0, LINE)).corner_radius(14.0).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    num_badge(ui, i + 1);
                    eval_pill(ui, &h.ev, h.white_cp >= 0, 14.0);
                    let open = self.expanded[i];
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (r, resp) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
                        ico(ui.painter(), if open { Ico::ChevD } else { Ico::ChevR }, r.center(), 22.0, if resp.hovered() { GOLD } else { CREAM });
                        if resp.clicked() { toggle = Some(i); }
                        ui.add(egui::Label::new(egui::RichText::new(&h.line).size(15.0)).truncate());
                    });
                });
                if self.expanded[i] {
                    ui.add_space(6.0);
                    egui::Frame::new().fill(Color32::from_rgba_unmultiplied(76, 194, 89, 30)).stroke(Stroke::new(1.0, Color32::from_rgba_unmultiplied(76, 194, 89, 120))).corner_radius(10.0).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal_top(|ui| {
                            let (r, _) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
                            ico(ui.painter(), Ico::Target, r.center(), 22.0, Color32::from_rgb(139, 229, 154));
                            ui.add(egui::Label::new(egui::RichText::new(&h.text).size(14.5).color(Color32::from_rgb(222, 232, 220))).wrap());
                        });
                    });
                }
            });
            ui.add_space(6.0);
        }
        if let Some(i) = toggle { self.expanded[i] = !self.expanded[i]; }
        ui.add_space(6.0);
    }

    /// Bottom of the play screen: board editor / picture import, and the FEN.
    fn pinned_bottom(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        let w = (ui.available_width() - 12.0) / 2.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let editing = self.import.as_ref().map_or(false, |i| i.edit);
            if dark_btn(ui, if editing { "Fechar editor" } else { "Editar tabuleiro" }, Some(Ico::Pencil), Vec2::new(w, 50.0), false) {
                if editing { self.import = None; self.brush = None; } else { self.start_edit(); }
            }
            if gold_btn(ui, "Imagem", Some(Ico::Image), Vec2::new(w, 50.0)) { self.show_picture = !self.show_picture; }
        });
        ui.add_space(10.0);
        // FEN of the position being shown; edit it and press Enter to load another one
        let cur = fen_of(self.pos_at(self.view.unwrap_or(self.hist.len())));
        if !self.fen_editing { self.fen_text = cur; }
        let mut load = false;
        egui::Frame::new().fill(Color32::from_rgb(14, 14, 16)).stroke(Stroke::new(1.0, Color32::from_rgb(44, 44, 50))).corner_radius(14.0).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
                ico(ui.painter(), Ico::Copy, r.center(), 22.0, Color32::from_rgb(216, 209, 191));
                let r = ui.add(egui::TextEdit::singleline(&mut self.fen_text).frame(egui::Frame::NONE).desired_width(ui.available_width() - 88.0).font(egui::FontId::proportional(14.0)));
                self.fen_editing = r.has_focus();
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) { load = true; }
                if dark_btn(ui, "Copiar", None, Vec2::new(76.0, 34.0), false) { ui.ctx().copy_text(self.fen_text.clone()); }
            });
        });
        if load { let f = self.fen_text.clone(); self.load_fen(&f); }
    }

    /// Picture import (paste / open) and the board editor palette.
    fn picture_cards(&mut self, ui: &mut egui::Ui) {
        if self.show_picture || self.import.as_ref().map_or(false, |i| !i.edit) {
            card(ui, |ui| {
                ui.label(med("Importar imagem", 17.0));
                ui.label(egui::RichText::new("Ctrl+V, arrastar a imagem para a janela, ou:").small().color(MUTED));
                ui.horizontal(|ui| {
                    if dark_btn(ui, "Colar print ou PGN", None, Vec2::new(170.0, 38.0), false) { self.paste_any(); }
                    if dark_btn(ui, "Abrir imagem…", None, Vec2::new(140.0, 38.0), false) {
                        if let Some(p) = rfd::FileDialog::new().add_filter("imagem", &["png", "jpg", "jpeg", "webp"]).pick_file() { self.open_file(&p); }
                    }
                });
            });
        }
        if self.import.is_some() {
            let mut changed = false;
            let mut done = false;
            card(ui, |ui| {
                ui.label(med("Corrigir posição", 17.0));
                if let Some(i) = &mut self.import {
                    if !i.edit { changed |= ui.checkbox(&mut i.white_top, "Brancas estão em cima no print").changed(); }
                    ui.horizontal(|ui| {
                        ui.label("Vez de:");
                        changed |= ui.radio_value(&mut i.white_to_move, true, "Brancas").changed();
                        changed |= ui.radio_value(&mut i.white_to_move, false, "Pretas").changed();
                    });
                }
                ui.label(egui::RichText::new("Escolhe uma peça (ou a borracha) e clica nas casas do tabuleiro.").small().color(MUTED));
                let mut brush = self.brush;
                for color in [Color::White, Color::Black] {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 3.0;
                        for role in [Role::King, Role::Queen, Role::Rook, Role::Bishop, Role::Knight, Role::Pawn] {
                            let pc = shakmaty::Piece { color, role };
                            let b = egui::Button::image(piece_img(&self.tex[&(color, role)], 32.0)).selected(brush == Some(Some(pc)));
                            if ui.add(b).clicked() { brush = if brush == Some(Some(pc)) { None } else { Some(Some(pc)) }; }
                        }
                    });
                }
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new("Borracha").selected(brush == Some(None))).clicked() { brush = if brush == Some(None) { None } else { Some(None) }; }
                    if ui.button("Concluir").clicked() { done = true; }
                    if self.import.as_ref().map_or(false, |i| !i.edit) && ui.button("Aprender estas peças").on_hover_text("Guarda o aspeto das peças deste print (já corrigido) para ler melhor os próximos.").clicked() { self.learn_import(); }
                });
                self.brush = brush;
            });
            if changed { self.apply_import("Posição atualizada."); }
            if done { self.import = None; self.brush = None; self.show_picture = false; }
        }
    }

    /// Opens the board editor on the position being shown.
    fn start_edit(&mut self) {
        let shown = self.pos_at(self.view.unwrap_or(self.hist.len())).clone();
        let white_top = self.flipped;
        let mut grid: vision::Grid = Default::default();
        for i in 0..64u32 {
            let sq = Square::new(i);
            if let Some(p) = shown.board().piece_at(sq) {
                let (r, c) = vision::grid_index(u32::from(sq.file()) as usize, u32::from(sq.rank()) as usize, white_top);
                grid[r][c] = Some((p.color, p.role));
            }
        }
        self.import = Some(Import { grid, sils: Default::default(), white_top, white_to_move: shown.turn() == Color::White, edit: true });
        self.brush = None;
        self.msg = "Editor aberto: escolhe uma peça e clica nas casas.".into();
    }

    /// Loads a position from a FEN typed in the FEN box.
    fn load_fen(&mut self, fen: &str) {
        match fen.trim().parse::<Fen>().map_err(|e| e.to_string())
            .and_then(|f| f.into_position::<Chess>(CastlingMode::Standard).map_err(|e| pos_error(&e.to_string())))
        {
            Ok(p) => {
                self.pos = p;
                self.loaded = None;
                self.reset_history();
                self.me = self.pos.turn();
                self.last = None;
                self.sel = None;
                self.msg = "Posição carregada do FEN.".into();
                self.after_move();
            }
            Err(e) => self.msg = format!("FEN inválido: {e}"),
        }
    }

    fn view_analysis(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if dark_btn(ui, "Voltar", Some(Ico::ChevL), Vec2::new(110.0, 42.0), false) { self.tab = 0; }
            ui.vertical(|ui| {
                ui.label(semi("Análise da partida", 26.0));
                if let Some(i) = &self.loaded { ui.label(egui::RichText::new(format!("{} vs {}", i.white, i.black)).size(14.0).color(MUTED)); }
            });
        });
        ui.add_space(10.0);
        if self.review.is_none() {
            card(ui, |ui| {
                if self.reviewing {
                    ui.label(egui::RichText::new("A avaliar a partida…").color(MUTED));
                } else {
                    ui.label(egui::RichText::new("Ainda não há avaliação desta partida.").color(MUTED));
                    ui.add_space(6.0);
                    if !self.mvs.is_empty() && gold_btn(ui, "Avaliar partida", Some(Ico::Bars), Vec2::new(ui.available_width(), 48.0)) { self.start_review(); }
                }
            });
            return;
        }
        let idx = self.view.unwrap_or(self.hist.len());
        let cc_acc = self.loaded.as_ref().and_then(|i| i.cc_acc);
        let Some(r) = self.review.as_ref() else { return };
        let (acc, series, counts) = (r.acc, r.series.clone(), r.counts);
        let rows: Vec<(usize, Color, String, Option<String>, Class)> = r.rows.iter().enumerate().filter(|(_, w)| w.class >= Class::Inaccuracy)
            .map(|(i, w)| (i, w.color, w.san.clone(), w.best_san.clone(), w.class)).collect();

        // accuracy
        let w2 = (ui.available_width() - 12.0) / 2.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            egui::Frame::new().fill(Color32::from_rgb(244, 235, 212)).corner_radius(18.0).inner_margin(egui::Margin::symmetric(18, 12)).show(ui, |ui| {
                ui.set_width(w2 - 36.0);
                ui.vertical(|ui| {
                    ui.label(semi(format!("{:.0}%", acc[0]), 46.0).color(Color32::from_rgb(20, 17, 10)));
                    ui.label(med("Brancas · precisão", 15.0).color(Color32::from_rgb(40, 34, 20)));
                });
            });
            egui::Frame::new().fill(Color32::from_rgb(24, 24, 27)).stroke(Stroke::new(1.5, Color32::from_rgb(232, 176, 48))).corner_radius(18.0).inner_margin(egui::Margin::symmetric(18, 12)).show(ui, |ui| {
                ui.set_width(w2 - 36.0);
                ui.vertical(|ui| {
                    ui.label(semi(format!("{:.0}%", acc[1]), 46.0).color(GOLD));
                    ui.label(med("Pretas · precisão", 15.0).color(Color32::from_rgb(216, 209, 191)));
                });
            });
        });
        if let Some((w, b)) = cc_acc { ui.label(egui::RichText::new(format!("Precisão no chess.com: brancas {w:.1}% · pretas {b:.1}%")).small().color(MUTED)); }
        ui.add_space(10.0);

        card(ui, |ui| {
            ui.label(med("Avaliação ao longo da partida", 17.0));
            ui.add_space(4.0);
            win_chart(ui, &series, Vec2::new(ui.available_width(), 104.0), Some(idx), false);
        });

        card(ui, |ui| {
            for c in CLASSES.into_iter().filter(|c| counts[0][*c as usize] + counts[1][*c as usize] > 0) {
                ui.horizontal(|ui| {
                    let (rc, _) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
                    draw_icon(ui.painter(), rc.center(), 10.5, c);
                    ui.add_sized([150.0, 24.0], egui::Label::new(med(c.label(), 16.0).color(c.color())));
                    ui.add_sized([46.0, 24.0], egui::Label::new(med(counts[0][c as usize].to_string(), 16.0)));
                    ui.add_sized([46.0, 24.0], egui::Label::new(med(counts[1][c as usize].to_string(), 16.0).color(GOLD)));
                });
            }
            ui.label(egui::RichText::new("coluna 1: brancas · coluna 2: pretas").small().color(MUTED));
        });

        card(ui, |ui| {
            ui.label(med("Lances a rever", 17.0));
            ui.add_space(4.0);
            let mut jump = None;
            for (i, color, san, best, class) in &rows {
                let mut t = format!("{}{} {}{}", i / 2 + 1, if *color == Color::White { "." } else { "..." }, san, class.sym());
                if let Some(b) = best { t += &format!("     melhor: {b}"); }
                let btn = egui::Button::new(egui::RichText::new(t).size(16.0).color(class.color())).fill(Color32::from_rgb(26, 26, 30)).stroke(Stroke::new(1.0, Color32::from_rgb(48, 48, 54))).corner_radius(12.0).min_size(Vec2::new(ui.available_width(), 40.0));
                if ui.add(btn).clicked() { jump = Some(i + 1); }
            }
            if rows.is_empty() { ui.label(egui::RichText::new("Sem imprecisões nem erros. Boa partida!").color(MUTED)); }
            if let Some(j) = jump { self.goto(j); self.tab = 0; }
        });
    }

    /// Engine and suggestion settings (the sliders button next to the board, or the menu).
    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings { return; }
        let mut open = true;
        egui::Window::new("Definições").open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
            ui.set_width(300.0);
            let names: Vec<String> = self.engines.iter().map(|e| e.name.clone()).collect();
            ui.label(med("Adversário", 17.0));
            let mut new_opp = self.opp;
            egui::ComboBox::from_id_salt("opp").width(240.0).selected_text(names[self.opp].as_str()).show_ui(ui, |ui| {
                for (i, n) in names.iter().enumerate() { ui.selectable_value(&mut new_opp, i, n.as_str()); }
            });
            ui.add_enabled_ui(!self.both, |ui| {
                ui.label(format!("Elo: {}", self.elo));
                ui.add(egui::Slider::new(&mut self.elo, 1000..=2600).step_by(50.0).show_value(false));
            });
            if ui.checkbox(&mut self.both, "Jogar os dois lados (sem engine)").changed() { self.thinking = false; self.after_move(); }
            ui.separator();
            ui.label(med("Sugestões", 17.0));
            if ui.checkbox(&mut self.hints_on, "Mostrar as 3 melhores jogadas").changed() { self.request_hints(); }
            let mut new_ana = self.ana;
            egui::ComboBox::from_id_salt("ana").width(240.0).selected_text(names[self.ana].as_str()).show_ui(ui, |ui| {
                for (i, n) in names.iter().enumerate() { ui.selectable_value(&mut new_ana, i, n.as_str()); }
            });
            ui.separator();
            if ui.button("Adicionar engine UCI…").clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("engine", &["exe"]).pick_file() { self.add_engine(p); }
            }
            if new_opp != self.opp {
                self.opp = new_opp;
                self.player = self.spawn(new_opp);
                self.msg.clear();
                if self.thinking { self.after_move(); }
            }
            if new_ana != self.ana {
                self.ana = new_ana;
                self.analyst = self.spawn(new_ana);
                self.reviewer = self.spawn(new_ana);
                self.msg.clear();
                self.request_hints();
            }
        });
        self.show_settings = open;
    }

    fn view_import(&mut self, ui: &mut egui::Ui) {
        ui.label(semi("Partidas", 28.0));
        ui.add_space(8.0);
        if !self.msg.is_empty() {
            card(ui, |ui| { ui.label(egui::RichText::new(&self.msg).color(AMBER)); });
        }

        // chess.com games of a user
        card(ui, |ui| {
            ui.label(med("Perfil e jogos do chess.com", 18.0));
            ui.label(egui::RichText::new("Só aparecem partidas públicas e terminadas.").size(13.5).color(MUTED));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                egui::Frame::new().fill(Color32::from_rgb(14, 14, 16)).stroke(Stroke::new(1.0, Color32::from_rgb(52, 52, 58))).corner_radius(14.0).inner_margin(egui::Margin::symmetric(10, 4)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
                        ico(ui.painter(), Ico::Search, r.center(), 20.0, MUTED);
                        let te = ui.add(egui::TextEdit::singleline(&mut self.cc_user).frame(egui::Frame::NONE).hint_text("utilizador").desired_width((ui.available_width() - 150.0).max(80.0)));
                        let enter = te.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if enter && !self.cc_busy { self.cc_search(false); }
                    });
                });
                if gold_btn(ui, "Procurar", None, Vec2::new(110.0, 44.0)) && !self.cc_busy { self.cc_search(false); }
            });
            ui.label(egui::RichText::new("Só o nome de utilizador sai do teu computador, e só quando carregas em Procurar.").size(12.5).color(MUTED));
            if self.cc_busy { ui.label(egui::RichText::new("A buscar partidas…").color(MUTED)); }
            if !self.cc_err.is_empty() { ui.label(egui::RichText::new(&self.cc_err).color(AMBER)); }
            let user = self.cc_query.clone();
            let mut pick = None;
            egui::ScrollArea::vertical().id_salt("ccgames").max_height(380.0).auto_shrink([false, true]).show(ui, |ui| {
                for (i, g) in self.cc_games.iter().enumerate().take(200) {
                    let white = g.white.eq_ignore_ascii_case(&user);
                    let (my_res, opp, opp_elo) = if white { (&g.white_result, &g.black, g.black_elo) } else { (&g.black_result, &g.white, g.white_elo) };
                    let won = my_res == "win";
                    let drew = g.white_result != "win" && g.black_result != "win";
                    let (tag, col) = if won { ("V", Color32::from_rgb(139, 212, 110)) } else if drew { ("E", MUTED) } else { ("D", Color32::from_rgb(235, 107, 92)) };
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::hover());
                        ui.painter().rect_filled(r, 8.0, col);
                        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, tag, f_semi(15.0), Color32::from_rgb(14, 14, 16));
                        ui.vertical(|ui| {
                            let elo = opp_elo.map_or("?".to_string(), |e| e.to_string());
                            ui.label(med(format!("{opp} ({elo})"), 16.0));
                            ui.label(egui::RichText::new(format!("{} · {}", g.time_class, chesscom::date(g.end_time))).size(13.0).color(MUTED));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if dark_btn(ui, "Analisar", None, Vec2::new(90.0, 34.0), true) { pick = Some(i); }
                        });
                    });
                }
            });
            if let Some(i) = pick { self.load_cc_game(i); }
            if !self.cc_games.is_empty() && self.cc_loaded < self.cc_archives.len() && !self.cc_busy && dark_btn(ui, "Mais antigas", None, Vec2::new(140.0, 36.0), true) {
                self.cc_search(true);
            }
        });

        // PGN text or file
        card(ui, |ui| {
            ui.label(med("PGN", 18.0));
            ui.add(egui::TextEdit::multiline(&mut self.pgn_text).hint_text("Cola aqui o PGN (de qualquer site)…").desired_rows(4).desired_width(f32::INFINITY));
            ui.add_space(6.0);
            let w = (ui.available_width() - 12.0) / 2.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                if gold_btn(ui, "Carregar", None, Vec2::new(w, 46.0)) {
                    let t = self.pgn_text.clone();
                    if !self.try_pgn(&t) { self.msg = "Não encontrei nenhuma partida neste texto.".into(); }
                }
                if dark_btn(ui, "Abrir ficheiro…", None, Vec2::new(w, 46.0), false) {
                    if let Some(p) = rfd::FileDialog::new().add_filter("PGN", &["pgn", "txt"]).pick_file() { self.open_file(&p); }
                }
            });
            let mut pick = None;
            for (i, g) in self.pgn_games.iter().enumerate().take(100) {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(format!("{} vs {}  {}", g.header("White").unwrap_or("?"), g.header("Black").unwrap_or("?"), g.header("Result").unwrap_or("")));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if dark_btn(ui, "Abrir", None, Vec2::new(70.0, 32.0), true) { pick = Some(i); }
                    });
                });
            }
            if let Some(i) = pick { let g = self.pgn_games[i].clone(); self.load_pgn_game(&g); self.game_idx = Some((1, i)); }
        });
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
        self.game_idx = Some((0, i));
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
            1 => { self.pgn_games.clear(); self.game_idx = None; self.load_pgn_game(&games[0]); true }
            n => {
                self.pgn_games = games;
                self.tab = 1;
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

/// Dev only: GAMBITO_SIZE=1280x800 opens the window at that size.
fn dev_size() -> Option<[f32; 2]> {
    let v = std::env::var("GAMBITO_SIZE").ok()?;
    let (w, h) = v.split_once('x')?;
    Some([w.parse().ok()?, h.parse().ok()?])
}

fn main() -> eframe::Result {
    migrate_old_data();
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size(dev_size().unwrap_or([1366.0, 860.0])).with_min_inner_size([1000.0, 680.0]).with_title("Gambito").with_icon(std::sync::Arc::new(app_icon())),
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

#[cfg(test)]
mod icon {
    // Writes assets/icon.ico (the logo's mark at several sizes, PNG-compressed entries). Run once after changing the logo:
    //   cargo test gen_icon -- --ignored
    #[test]
    #[ignore]
    fn gen_icon() {
        let img = image::load_from_memory(include_bytes!("../assets/logo.webp")).unwrap().to_rgba8();
        let crop = image::imageops::crop_imm(&img, 340, 250, 575, 575).to_image();
        let sizes = [16u32, 24, 32, 48, 64, 128, 256];
        let pngs: Vec<Vec<u8>> = sizes.iter().map(|&s| {
            let small = image::imageops::resize(&crop, s, s, image::imageops::FilterType::Lanczos3);
            let mut out = std::io::Cursor::new(Vec::new());
            small.write_to(&mut out, image::ImageFormat::Png).unwrap();
            out.into_inner()
        }).collect();
        let mut ico: Vec<u8> = vec![0, 0, 1, 0];
        ico.extend((sizes.len() as u16).to_le_bytes());
        let mut offset = 6 + 16 * sizes.len() as u32;
        for (s, png) in sizes.iter().zip(&pngs) {
            let b = if *s >= 256 { 0 } else { *s as u8 };
            ico.extend([b, b, 0, 0]);
            ico.extend(1u16.to_le_bytes());
            ico.extend(32u16.to_le_bytes());
            ico.extend((png.len() as u32).to_le_bytes());
            ico.extend(offset.to_le_bytes());
            offset += png.len() as u32;
        }
        for png in &pngs { ico.extend(png); }
        std::fs::write("assets/icon.ico", ico).unwrap();
    }
}
