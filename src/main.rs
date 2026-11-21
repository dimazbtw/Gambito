#![windows_subsystem = "windows"]
#[macro_use]
mod i18n;
mod book;
mod chesscom;
mod engine;
mod explain;
mod games;
mod motion;
mod pgn;
mod screens;
mod setup;
mod sound;
mod train;
mod update;
mod vision;

/// Where an app update stands (src/update.rs); shared with the threads that check and download.
#[derive(Clone, Default)]
enum Upd { #[default] Idle, Checking, Latest, Found(update::Release), Getting(f32), Restarting, Failed(String) }

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use engine::{Engine, Ev, Job, Reply};
use explain::{explain, val};
use i18n::tl;
use shakmaty::{fen::Fen, san::SanPlus, uci::UciMove, CastlingMode, Chess, Color, EnPassantMode, Move, Position, Role, Square};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

/// Board colours (Settings): name, light square, dark square. The first is chess.com's green.
pub(crate) const THEMES: [(&str, Color32, Color32); 5] = [
    ("Verde", Color32::from_rgb(238, 238, 210), Color32::from_rgb(118, 150, 86)),
    ("Castanho", Color32::from_rgb(240, 217, 181), Color32::from_rgb(181, 136, 99)),
    ("Azul", Color32::from_rgb(222, 227, 230), Color32::from_rgb(140, 162, 173)),
    ("Dourado", Color32::from_rgb(236, 222, 186), Color32::from_rgb(176, 136, 68)),
    ("Cinzento", Color32::from_rgb(224, 224, 224), Color32::from_rgb(140, 140, 146)),
];
static THEME: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
fn theme() -> usize { THEME.load(std::sync::atomic::Ordering::Relaxed).min(THEMES.len() - 1) }
fn light() -> Color32 { THEMES[theme()].1 }
fn dark() -> Color32 { THEMES[theme()].2 }

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

/// Straight arrow, or an L-shaped one for a knight jump (long side first), ending in a head, in chess.com's
/// proportions. It is built from pieces that never overlap, so a see-through colour stays even (no darker joints).
fn draw_mark_arrow(p: &egui::Painter, a: Pos2, b: Pos2, cell: f32, col: Color32, knight: bool) {
    let corner = knight.then(|| if (b.x - a.x).abs() > (b.y - a.y).abs() { Pos2::new(b.x, a.y) } else { Pos2::new(a.x, b.y) });
    let (w, head_w, head_l) = (cell * 0.2, cell * 0.52, cell * 0.42);
    let dir = (b - corner.unwrap_or(a)).normalized();
    let tip = b - dir * cell * 0.06;
    let base = tip - dir * head_l;
    let shaft = |s: Pos2, e: Pos2| {
        let n = { let d = (e - s).normalized(); Vec2::new(-d.y, d.x) * w / 2.0 };
        p.add(egui::Shape::convex_polygon(vec![s + n, e + n, e - n, s - n], col, Stroke::NONE));
    };
    match corner {
        Some(c) => {
            let d1 = (c - a).normalized();
            shaft(a, c + d1 * w / 2.0); // runs through the corner, so the second leg starts where it ends
            shaft(c + dir * w / 2.0, base);
        }
        None => shaft(a, base),
    }
    let n = Vec2::new(-dir.y, dir.x) * head_w / 2.0;
    p.add(egui::Shape::convex_polygon(vec![tip, base + n, base - n], col, Stroke::NONE));
}

const ANIM_SECS: f64 = 0.18;

/// A piece sliding to `to` (the shown position's idx); `capture` is a piece fading out at `to`.
#[derive(Clone, Copy)]
struct Anim { idx: usize, from: Square, to: Square, t0: f64, capture: Option<(Square, shakmaty::Piece)> }

struct EngineEntry { name: String, known: Option<usize>, path: Option<PathBuf> }

fn my_games_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("Gambito").join("games.pgn"))
}

fn load_my_games() -> Vec<pgn::Game> {
    my_games_path().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| pgn::parse_games(&t).into_iter().filter(|g| !g.moves.is_empty()).collect()).unwrap_or_default()
}

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

/// What the board is being used for. Everything but Free is training (Exercícios / Aprender).
enum Mode { Free, Puzzle(PuzzleRun), Drill(Drill), Example }
struct PuzzleRun {
    idx: usize,              // into App::puzzles
    step: usize,             // next move of the solution to be played
    checked: usize,          // moves already judged (after_move can be called again for the same move)
    failed: bool,            // a wrong move, a hint or the solution was used
    hinted: u8,
    done: bool,
    delta: Option<f64>,      // rating change, once known
    lesson: Option<usize>,   // practising a lesson
    daily: bool,
    rated: bool,             // first time this puzzle is played: it counts for the rating
    showing: bool,           // playing the solution by itself
    next_at: Option<f64>,    // when the next scripted move is played
    undo_at: Option<f64>,    // when a wrong move is taken back
}
struct Drill { lesson: usize, goal: train::Goal, done: Option<bool>, hints: bool }
/// The game that was on the board before a puzzle or lesson, put back when you return to Jogar.
struct Snapshot { pos: Chess, hist: Vec<(Chess, String)>, mvs: Vec<Move>, quality: Vec<Option<(Class, Option<String>)>>, view: Option<usize>, review: Option<Review>, me: Color, flipped: bool, last: Option<(Square, Square)>, loaded: Option<GameInfo>, both: bool, game_idx: Option<(u8, usize)> }

/// Turns shakmaty's English position error into something readable.
fn pos_error(e: &str) -> String {
    let l = e.to_lowercase();
    if l.contains("king") { tl("tem de haver exatamente um rei de cada cor").into() }
    else if l.contains("pawn") { tl("há peões na primeira ou na última fila").into() }
    else if l.contains("check") { tl("um dos reis está em xeque de forma impossível").into() }
    else { e.to_string() }
}

#[derive(Clone, Copy, PartialEq, PartialOrd)]
enum Class { Brilliant, Great, Best, Excellent, Good, Book, Inaccuracy, Mistake, Blunder }
const CLASSES: [Class; 9] = [Class::Brilliant, Class::Great, Class::Best, Class::Excellent, Class::Good, Class::Book, Class::Inaccuracy, Class::Mistake, Class::Blunder];
impl Class {
    fn label(self) -> &'static str {
        tl(["Brilhante", "Ótima", "Melhor jogada", "Excelente", "Boa", "Teoria", "Imprecisão", "Erro", "Gafe"][self as usize])
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
struct Review { rows: Vec<Row>, acc: [f64; 2], counts: [[u32; 9]; 2], series: Vec<f32>, phase: [[Option<f64>; 3]; 2] } // index 0 = white, 1 = black; phases: opening, middlegame, endgame

/// Which part of the game a position is in: 0 opening (the first 10 moves), 2 endgame (little material left), else 1.
fn phase_of(pos: &Chess, ply: usize) -> usize {
    let b = pos.board();
    let mat: u32 = b.occupied().into_iter().filter_map(|s| b.role_at(s)).filter(|r| !matches!(r, Role::Pawn | Role::King)).map(val).sum();
    if mat <= 26 || (b.queens().is_empty() && mat <= 32) { 2 } else if ply < 20 { 0 } else { 1 }
}

/// A rough playing strength for an accuracy in one game (like the "game rating" of review sites).
/// ponytail: a fixed curve; a single game says little, so it is only shown as an estimate.
fn est_elo(acc: f64) -> u32 {
    const PTS: [(f64, f64); 9] = [(0.0, 300.0), (50.0, 700.0), (60.0, 1000.0), (70.0, 1350.0), (78.0, 1650.0), (85.0, 1950.0), (90.0, 2250.0), (95.0, 2600.0), (100.0, 3000.0)];
    let a = acc.clamp(0.0, 100.0);
    let k = PTS.windows(2).find(|w| a <= w[1].0).unwrap_or(&PTS[7..9]);
    (k[0].1 + (k[1].1 - k[0].1) * (a - k[0].0) / (k[1].0 - k[0].0)).round() as u32 / 50 * 50
}

fn win_pct(cp: f64) -> f64 { 50.0 + 50.0 * (2.0 / (1.0 + (-0.00368208 * cp).exp()) - 1.0) }

struct App {
    pos: Chess,
    hist: Vec<(Chess, String)>, // position before the move + its SAN
    mvs: Vec<Move>,             // the moves themselves (parallel to hist)
    quality: Vec<Option<(Class, Option<String>)>>, // rating of each move + engine's better move (parallel to hist)
    view: Option<usize>,        // None = live position; Some(i) = browsing the position after i moves
    review: Option<Review>,
    reviewing: bool,
    review_prog: (usize, usize), // positions reviewed so far, of how many
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
    new_menu: bool,            // the "Novo jogo" choices are open
    panel_tab: u8,             // board screen panel: 0 Lances, 1 Sugestões, 2 Análise
    eval_shown: f32,           // evaluation bar, animated towards the latest evaluation (white %)
    art: HashMap<&'static str, (egui::TextureHandle, [usize; 2])>, // pictures, decoded on first use
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
    hint_at: u32,              // depth the shown hints come from
    game: Option<games::Run>,  // the mini-game being played (tab 6)
    hint_deep: bool,           // an uncapped hint search is running
    hint_paused: bool,         // ... and was stopped because no board is on screen
    thinking: bool,
    msg: String,
    toast: Option<(String, u8, f64)>,            // short notice over any screen: text, kind (0 good, 1 bad, 2 info), when
    result: Option<(String, String, u8, f64)>,   // end-of-game card on the board: title, line, kind (0 won, 1 lost, 2 draw), when
    last_paste: f64,                             // when Ctrl+V was last handled (egui sends it more than one way)
    upd: std::sync::Arc<std::sync::Mutex<Upd>>,
    upd_told: bool,                              // the "new version" notice was shown
    upd_asked: bool,                             // the user asked (Procurar agora / Atualizar): say how it went
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
    tpl: Option<vision::Templates>, // piece shapes for reading screenshots, made on first use
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
    cc_site: u8,               // where the user's games come from: 0 chess.com, 1 Lichess
    tab_t: (u8, f64),          // the screen last drawn and when it appeared (fade-in)
    net_tx: Sender<chesscom::Msg>,
    net_rx: Receiver<chesscom::Msg>,
    marks: Vec<Mark>,          // arrows / squares drawn with the right mouse button
    mark_from: Option<Square>, // where the current right-button drag started
    promo: Option<(Square, Square)>, // a promotion waiting for the piece to be picked
    resigned: bool,            // I gave up the game against the engine
    check_t: (usize, f64),     // the position whose check was last pulsed, and when
    my_games: Vec<pgn::Game>,  // games played here against the engine (games.pgn), oldest first
    brush: Option<Option<shakmaty::Piece>>, // board editor: None = off, Some(None) = eraser, Some(Some(p)) = place p
    mode: Mode,
    snapshot: Option<Snapshot>,
    puzzles: Vec<train::Puzzle>,
    progress: train::Progress,
    rng: train::Rng,
    pz_theme: Option<&'static str>, // theme filter of the Exercícios screen
    lesson: Option<usize>,          // lesson open in Aprender
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
        // only the engines the installer put on this computer (the user may have left some out)
        let mut engines: Vec<EngineEntry> = engine::known().iter().enumerate().filter(|(_, k)| engine::path(k).exists())
            .map(|(i, k)| EngineEntry { name: k.name.to_string(), known: Some(i), path: None }).collect();
        engines.extend(load_engines());
        if engines.is_empty() { engines.push(EngineEntry { name: "Stockfish 19".into(), known: Some(0), path: None }); } // reports "won't start"
        let exe = match (&engines[0].known, &engines[0].path) { (Some(k), _) => engine::path(&engine::known()[*k]), (_, Some(p)) => p.clone(), _ => PathBuf::new() };
        let svgs = piece_svgs();
        let t_pieces = std::time::Instant::now();
        let mut tex = HashMap::new();
        for (c, r, d) in svgs.iter() {
            let px = vision::raster(d, 160);
            let img = egui::ColorImage::from_rgba_premultiplied([160, 160], &px);
            tex.insert((*c, *r), ctx.load_texture(format!("{c:?}{r:?}"), img, egui::TextureOptions::LINEAR));
        }
        if std::env::var_os("GAMBITO_PERF").is_some() { eprintln!("startup pieces (12 SVGs): {:.0} ms", t_pieces.elapsed().as_secs_f64() * 1000.0); }
        let mut app = App {
            pos: Chess::default(), hist: vec![], mvs: vec![], quality: vec![], view: None, review: None, reviewing: false, review_prog: (0, 0),
            reviewer: Engine::heavy(&exe, tx.clone(), ctx.clone()), book: timed("openings", book::Book::new), me: Color::White, flipped: false, sel: None, drag: None, last: None,
            elo: 1500, hints_on: true, hint_elo: 2200, tab: 0, expanded: [true, false, false], new_menu: false, panel_tab: 0, eval_shown: 50.0, art: HashMap::new(), show_picture: false, fen_text: String::new(), fen_editing: false, logo: load_logo(&ctx), playing: false, play_t: 0.0, game_idx: None, shot_requested: false, eval_cp: 0, eval_mate: false, both: false, hints: vec![], waiting_hint: false, hint_at: 0, game: None, hint_deep: false, hint_paused: false, thinking: false, msg: String::new(), toast: None, result: None, last_paste: -10.0, upd: Default::default(), upd_told: false, upd_asked: false,
            player: Engine::new(&exe, tx.clone(), ctx.clone()),
            analyst: Engine::heavy(&exe, tx.clone(), ctx.clone()),
            tx, ctx, engines, opp: 0, ana: 0, time: 0.0, anim: None, badge_t: (usize::MAX, 0.0), hints_t0: 0.0, skip_anim: false,
            rx, tex, tpl: None, import: None, brush: None, loaded: None, pgn_text: String::new(), pgn_games: vec![], cc_user: String::new(), cc_query: String::new(),
            cc_games: vec![], cc_archives: vec![], cc_loaded: 0, cc_busy: false, cc_err: String::new(), cc_site: 0, tab_t: (u8::MAX, 0.0), net_tx, net_rx, marks: vec![], mark_from: None, promo: None, resigned: false, check_t: (usize::MAX, 0.0), my_games: load_my_games(),
            mode: Mode::Free, snapshot: None, puzzles: timed("puzzles", train::load_puzzles), progress: train::Progress::load(), rng: train::Rng::new(), pz_theme: None, lesson: None,
        };
        app.tab = 5; // opens on Início
        // choices kept in settings.json
        let num = |k: &str| i18n::setting(k).and_then(|v| v.as_u64());
        if let Some(e) = num("elo") { app.elo = (e as u32).clamp(1000, 2600); }
        if let Some(e) = num("hint_elo") { app.hint_elo = (e as u32).clamp(1000, 2600); }
        if let Some(b) = i18n::setting("hints_on").and_then(|v| v.as_bool()) { app.hints_on = b; }
        if let Some(b) = i18n::setting("sound").and_then(|v| v.as_bool()) { sound::set_on(b); }
        // animations follow "Show animations in Windows" unless the user chose in Settings
        motion::set_on(i18n::setting("motion").and_then(|v| v.as_bool()).unwrap_or_else(motion::system_allows));
        if let Some(t) = num("theme") { THEME.store(t as usize, std::sync::atomic::Ordering::Relaxed); }
        app.after_move();
        // dev hooks for checking the look without touching the screen:
        // GAMBITO_PGN=<file> GAMBITO_TAB=<0..5> GAMBITO_LESSON=<n> GAMBITO_SHOT=<png>
        if let Some(t) = std::env::var_os("GAMBITO_PGN").and_then(|p| std::fs::read_to_string(p).ok()) { app.try_pgn(&t); }
        if let Ok(t) = std::env::var("GAMBITO_TAB") { app.tab = t.parse().unwrap_or(0); }
        // GAMBITO_PANEL=<0..2> GAMBITO_HINT_ELO=<1000..2600>: the board panel tab and the suggestions' strength
        // GAMBITO_MARKS="e2e4:0 g1f3:1 d5:2": right-button drawings (arrow or square, then the colour)
        if let Ok(m) = std::env::var("GAMBITO_MARKS") {
            for tok in m.split_whitespace() {
                let (sqs, col) = tok.split_once(':').unwrap_or((tok, "0"));
                let col = col.parse::<u8>().unwrap_or(0).min(3);
                let sq = |s: &str| s.parse::<Square>().ok();
                match (sqs.get(..2).and_then(sq), sqs.get(2..4).and_then(sq)) {
                    (Some(a), Some(b)) => app.marks.push(Mark::Arrow(a, b, col)),
                    (Some(a), None) => app.marks.push(Mark::Square(a, col)),
                    _ => {}
                }
            }
        }
        // GAMBITO_CC="lichess:name" or "chesscom:name": searches that user's games on start
        if let Some((site, user)) = std::env::var("GAMBITO_CC").ok().as_deref().and_then(|v| v.split_once(':')) {
            app.cc_site = (site == "lichess") as u8;
            app.cc_user = user.to_string();
            app.cc_search(false);
        }
        // GAMBITO_GAME=<0..10>: opens that mini-game
        if let Some(g) = std::env::var("GAMBITO_GAME").ok().and_then(|g| g.parse::<usize>().ok()).filter(|&g| g < games::GAMES.len()) { app.tab = 6; app.start_game(g); }
        if let Ok(t) = std::env::var("GAMBITO_PANEL") { app.panel_tab = t.parse().unwrap_or(0); }
        if let Some(e) = std::env::var("GAMBITO_HINT_ELO").ok().and_then(|e| e.parse::<u32>().ok()) { app.hint_elo = e.clamp(1000, 2600); app.request_hints(); }
        if app.tab == 3 { app.next_puzzle(None); }
        if update::due() { app.check_update(false); } // in the background, at most once a day
        if let Some(l) = std::env::var("GAMBITO_LESSON").ok().and_then(|l| l.parse().ok()) { app.open_lesson(l); if std::env::var_os("GAMBITO_PRACTICE").is_some() { app.start_practice(l); } }
        // GAMBITO_MOVES="f3 e5 g4 Qh4": plays these moves (both sides) on the board
        if let Ok(moves) = std::env::var("GAMBITO_MOVES") {
            app.both = true;
            for san in moves.split_whitespace() {
                let Some(m) = san.parse::<shakmaty::san::San>().ok().and_then(|s| s.to_move(&app.pos).ok()) else { break };
                if let Some(from) = m.from() { app.try_move(from, dest(&m)); }
            }
        }
        app
    }

    fn new_game(&mut self, me: Color) {
        self.leave_training();
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
        match self.mode {
            Mode::Puzzle(_) => return self.puzzle_after_move(),
            Mode::Example => return,
            Mode::Drill(_) => if self.drill_after_move() { return },
            Mode::Free => {}
        }
        if self.pos.is_game_over() {
            self.announce_end();
            self.start_review(); // evaluate the finished game automatically
            return;
        }
        if self.pos.turn() != self.me {
            self.thinking = true;
            let elo = if matches!(self.mode, Mode::Drill(_)) { 2600 } else { self.elo }; // endgame lessons: the best defence
            self.player.send(Job::Play { fen: fen_of(&self.pos), elo });
        } else {
            self.request_hints();
        }
    }

    /// Search depth for the suggestions: the "hint Elo" slider trades depth for how obvious the moves are.
    /// ponytail: depth is only a rough stand-in for Elo (1000 -> 2, 2200 -> 10, 2550 -> 12).
    /// The top of the slider (2600) is 0: no cap, the engine keeps deepening (engine::DEEP_MS).
    fn hint_depth(&self) -> u32 {
        if self.hint_elo >= 2600 { 0 } else { (2 + (self.hint_elo - 1000) / 150).clamp(2, 14) }
    }

    /// Suggestions for the position being shown (also when browsing earlier moves).
    fn request_hints(&mut self) {
        self.hints.clear();
        self.waiting_hint = false;
        self.hint_at = 0;
        let allowed = match &self.mode { Mode::Free => true, Mode::Drill(d) => d.hints, _ => false };
        // the suggestions' engine starts only when a board is on screen (a tab change asks again, see screens())
        if !allowed || !self.board_on_screen() { self.stop_deep_hint(); return; }
        let shown = self.pos_at(self.view.unwrap_or(self.hist.len())).clone();
        if self.hints_on && !shown.is_game_over() && (self.view.is_some() || shown.turn() == self.me) {
            self.waiting_hint = true;
            let children = children(&shown);
            self.hint_deep = self.hint_depth() == 0;
            self.analyst.send(Job::Hint { fen: fen_of(&shown), depth: self.hint_depth(), children });
        } else {
            self.stop_deep_hint();
        }
    }

    /// Is a chessboard being shown? (Not on the home page, the lists, puzzles or settings.)
    pub(crate) fn board_on_screen(&self) -> bool {
        match self.tab {
            0 | 2 => true,
            4 => self.lesson.is_some() || matches!(self.mode, Mode::Drill(_)) || matches!(&self.mode, Mode::Puzzle(r) if r.lesson.is_some()),
            _ => false,
        }
    }

    /// An uncapped hint search runs until told otherwise: end it.
    fn stop_deep_hint(&mut self) {
        if std::mem::take(&mut self.hint_deep) { self.analyst.send(Job::Stop); }
    }

    fn play(&mut self, mv: Move) {
        sound::play(sound::for_move(&self.pos, &mv));
        self.marks.clear();
        let san = SanPlus::from_move(self.pos.clone(), mv.clone()).to_string();
        self.last = mv.from().map(|f| (f, dest(&mv)));
        let capture = if mv.is_en_passant() { None } else { mv.capture().and_then(|_| self.pos.board().piece_at(mv.to()).map(|p| (mv.to(), p))) };
        self.anim = match (mv.from(), self.skip_anim || !motion::on()) {
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
        self.result = None;
        self.resigned = false;
    }

    /// I give up the game against the engine.
    fn resign(&mut self) {
        self.resigned = true;
        self.thinking = false;
        self.hints.clear();
        self.stop_deep_hint();
        self.save_my_game();
        self.result = Some((tl("Desististe").to_string(), tl("Vê na análise onde a partida mudou.").to_string(), 1, self.time));
        self.notify(1, tl("Desististe"));
        sound::play(sound::Snd::End);
        self.start_review();
    }

    fn upd(&self) -> Upd { self.upd.lock().unwrap().clone() }

    /// Asks GitHub for a newer release, in the background.
    fn check_update(&mut self, asked: bool) {
        self.upd_asked = asked;
        let (st, ctx) = (self.upd.clone(), self.ctx.clone());
        *st.lock().unwrap() = Upd::Checking;
        std::thread::spawn(move || {
            let r = match update::check() { Ok(Some(r)) => Upd::Found(r), Ok(None) => Upd::Latest, Err(e) => Upd::Failed(e) };
            *st.lock().unwrap() = r;
            ctx.request_repaint();
        });
    }

    /// Downloads the new installer, checks it and hands over to it; the app then closes (see poll_update).
    fn start_update(&mut self) {
        let Upd::Found(r) = self.upd() else { return };
        self.upd_asked = true;
        let (st, ctx) = (self.upd.clone(), self.ctx.clone());
        *st.lock().unwrap() = Upd::Getting(0.0);
        std::thread::spawn(move || {
            let res = update::download(&r, &|d, t| { if t > 0 { *st.lock().unwrap() = Upd::Getting(d as f32 / t as f32); ctx.request_repaint(); } })
                .and_then(|p| update::launch(&p));
            *st.lock().unwrap() = match res { Ok(()) => Upd::Restarting, Err(e) => Upd::Failed(e) };
            ctx.request_repaint();
        });
    }

    fn poll_update(&mut self) {
        match self.upd() {
            Upd::Found(r) if !self.upd_told => { self.upd_told = true; self.notify(2, tr!("Há uma versão nova do Gambito: {}. Carrega em \"Atualizar\" no menu.", r.version)); }
            Upd::Restarting => self.ctx.send_viewport_cmd(egui::ViewportCommand::Close), // the installer puts the new version in and opens it
            Upd::Failed(e) if self.upd_asked => { self.upd_asked = false; self.notify(1, tr!("A atualização falhou: {}", e)); }
            Upd::Latest if self.upd_asked => { self.upd_asked = false; self.notify(0, tl("Tens a versão mais recente.")); }
            Upd::Getting(_) => self.ctx.request_repaint_after(std::time::Duration::from_millis(200)),
            _ => {}
        }
    }

    /// A short notice at the top of the window, seen on any screen.
    fn notify(&mut self, kind: u8, text: impl Into<String>) {
        if kind == 3 { sound::play(sound::Snd::Notify); }
        self.toast = Some((text.into(), kind, self.time));
        self.ctx.request_repaint();
    }

    /// The game just ended on the board: the card over the board, a notice, and the taskbar button
    /// flashes if Gambito is in the background.
    fn announce_end(&mut self) {
        self.save_my_game();
        let title = self.status();
        let (line, kind) = if self.pos.is_checkmate() {
            let winner = !self.pos.turn();
            if self.both { (tr!("Partida de {} lances.", self.hist.len().div_ceil(2)), 0) }
            else if winner == self.me { (tr!("Venceste o {} com Elo {}.", self.engines[self.opp].name, self.elo), 0) }
            else { (tl("Vê na análise onde a partida mudou.").to_string(), 1) }
        } else if self.pos.is_stalemate() {
            (tl("Rei afogado: quem joga não tem lances e não está em xeque.").to_string(), 2)
        } else if self.pos.is_insufficient_material() {
            (tl("Já não há peças para dar xeque-mate.").to_string(), 2)
        } else {
            (tl("A partida terminou.").to_string(), 2)
        };
        self.result = Some((title.clone(), line, kind, self.time));
        self.notify(if kind == 1 { 1 } else { 0 }, title);
        self.ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational));
    }

    /// A game played here (not an imported one) is kept in games.pgn when it ends, to review it later.
    fn save_my_game(&mut self) {
        if self.loaded.is_some() || self.hist.len() < 2 || !matches!(self.mode, Mode::Free) { return; }
        let name = |c: Color| if self.both { if c == Color::White { tl("Brancas").to_string() } else { tl("Pretas").to_string() } }
            else if c == self.me { tl("Tu").to_string() } else { self.engines[self.opp].name.clone() };
        let pgn = self.export_pgn();
        let date = chesscom::date(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())).replace('-', ".");
        let mut head = format!("[Event \"Gambito\"]
[Site \"Gambito\"]
[Date \"{date}\"]
[White \"{}\"]
[Black \"{}\"]
", name(Color::White), name(Color::Black));
        if !self.both { head += &format!("[{}Elo \"{}\"]
", if self.me == Color::White { "Black" } else { "White" }, self.elo); }
        // keep the Result / SetUp / FEN lines and the moves of export_pgn, with the new head
        let body: String = pgn.lines().filter(|l| !["[Event ", "[Site ", "[Date ", "[White ", "[Black "].iter().any(|k| l.starts_with(k))).map(|l| format!("{l}
")).collect();
        let text = format!("{head}{body}
");
        if let Some(p) = my_games_path() {
            if let Some(d) = p.parent() { let _ = std::fs::create_dir_all(d); }
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) { let _ = f.write_all(text.as_bytes()); }
        }
        self.my_games.extend(pgn::parse_games(&text));
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
            sound::play(sound::for_move(self.pos_at(old), m));
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
        self.review_prog = (0, self.hist.len() + 1);
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
        let (mut psum, mut pn) = ([[0.0f64; 3]; 2], [[0u32; 3]; 2]);
        for i in 0..self.mvs.len() {
            let (prev, mv) = (self.pos_at(i), &self.mvs[i]);
            let side = (prev.turn() == Color::Black) as usize;
            let (class, drop, best) = self.classify(prev, mv, ev[i].as_ref(), ev[i + 1].as_ref());
            let acc = (103.1668 * (-0.04354 * drop).exp() - 3.1669).clamp(0.0, 100.0);
            sum[side] += acc;
            n[side] += 1;
            let ph = phase_of(prev, i);
            psum[side][ph] += acc;
            pn[side][ph] += 1;
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
        let phase = [0, 1].map(|s| [0, 1, 2].map(|k| (pn[s][k] >= 3).then(|| psum[s][k] / pn[s][k] as f64)));
        Review { rows, acc: [sum[0] / n[0].max(1) as f64, sum[1] / n[1].max(1) as f64], counts, series, phase }
    }

    /// Per-move rating, requested right after each move so the icon shows up on the board.
    fn rate_last_move(&mut self) {
        let n = self.mvs.len();
        if n == 0 || matches!(self.mode, Mode::Puzzle(_)) { return; } // puzzles mark right/wrong themselves
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
        // a pawn reaching the last rank: the player picks the piece (chess.com's column of four)
        if cands.len() > 1 && cands.iter().all(|m| m.promotion().is_some()) {
            self.promo = Some((from, to));
            return true;
        }
        match cands.first().cloned() {
            Some(m) => {
                if let Some(i) = self.view { self.truncate_to(i); }
                self.play(m);
                true
            }
            None => false,
        }
    }

    /// The promotion picked in the column: the move is played; None cancels it.
    fn promote(&mut self, role: Option<Role>) {
        let Some((from, to)) = self.promo.take() else { return };
        let Some(role) = role else { self.sel = None; return };
        let base = self.pos_at(self.view.unwrap_or(self.hist.len())).clone();
        if let Some(m) = base.legal_moves().into_iter().find(|m| m.from() == Some(from) && m.to() == to && m.promotion() == Some(role)) {
            if let Some(i) = self.view { self.truncate_to(i); }
            self.play(m);
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
        if (self.thinking && self.view.is_none()) || base.is_game_over() || (!self.both && base.turn() != self.me) || !self.can_move(self.view.is_none()) { return; }
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
            (Some(k), _) => engine::path(&engine::known()[*k]),
            (_, Some(p)) => p.clone(),
            _ => unreachable!(),
        }
    }

    fn spawn(&self, i: usize) -> Engine {
        Engine::new(&self.engine_path(i), self.tx.clone(), self.ctx.clone())
    }

    /// The engine for suggestions and reviews: more threads and hash than the one that plays.
    fn spawn_heavy(&self, i: usize) -> Engine {
        Engine::heavy(&self.engine_path(i), self.tx.clone(), self.ctx.clone())
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
        self.msg = tr!("Engine \"{}\" adicionada e escolhida como adversária.", self.engines[i].name);
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
                        let n = games.len();
                        self.cc_games.extend(games);
                        self.cc_games.sort_by(|a, b| b.end_time.cmp(&a.end_time));
                        if self.cc_games.is_empty() { self.cc_err = tl("Sem partidas de xadrez normal nesses meses.").into(); }
                        else if n > 0 { self.notify(0, tr!("{} partidas carregadas do {}.", n, if self.cc_site == 1 { "Lichess" } else { "chess.com" })); }
                    }
                }
                chesscom::Msg::Err(e) => self.cc_err = e,
            }
        }
        while let Ok(r) = self.rx.try_recv() {
            match r {
                Reply::Play { fen, uci } if !self.both && !self.resigned && fen == fen_of(&self.pos) => {
                    if let Some(m) = UciMove::from_ascii(uci.as_bytes()).ok().and_then(|u| u.to_move(&self.pos).ok()) {
                        self.play(m);
                    }
                }
                Reply::Hint { fen, depth, lines } if fen == fen_of(self.pos_at(self.view.unwrap_or(self.hist.len()))) => {
                    let first = std::mem::take(&mut self.waiting_hint); // later replies are the same search, deeper
                    self.hint_at = depth;
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
                            text += " ";
                            text += &if l.score == 1 { tl("Mate forçado em 1 lance.").to_string() } else { tr!("Mate forçado em {} lances.", l.score) };
                        } else {
                            let gain = explain::material(p.board(), stm) - explain::material(p.board(), !stm) - base;
                            if gain >= 1 {
                                text += " ";
                                text += &if gain == 1 { tl("Na linha principal ganha 1 ponto de material.").to_string() } else { tr!("Na linha principal ganha {} pontos de material.", gain) };
                            }
                        }
                        if let (0, Some(g)) = (k, gap) {
                            if g >= 150 && !l.mate { text += " "; text += &tr!("É claramente a melhor: a segunda opção perde {} peões de vantagem.", format!("{:.1}", g as f32 / 100.0)); }
                        }
                        let cp = cp_of(l);
                        let white_cp = if stm == Color::White { cp } else { -cp };
                        let ev = if l.mate { format!("{}#{}", if white_cp >= 0 { "+" } else { "-" }, l.score.abs()) } else { format!("{:+.1}", white_cp as f32 / 100.0) };
                        Some(Hint { san, text, line: numbered(&shown, &sans), ev, white_cp, role: first.role(), color: stm, from: first.from()?, to: dest(&first) })
                    }).collect();
                    if let Some(h) = self.hints.first() { self.eval_cp = h.white_cp; self.eval_mate = h.ev.contains('#'); }
                    if first { self.hints_t0 = self.time; }
                }
                Reply::Dead(name) => {
                    if self.opp == 0 && self.ana == 0 {
                        self.msg = tr!("O motor \"{}\" não arranca.", name);
                    } else {
                        // a user-added engine failed: fall back to the built-in one
                        self.msg = tr!("A engine \"{}\" parou ou não é UCI. Voltei ao {}.", name, self.engines[0].name);
                        self.opp = 0;
                        self.ana = 0;
                        self.player = self.spawn(0);
                        self.analyst = self.spawn_heavy(0);
                        self.reviewer = self.spawn_heavy(0);
                        self.reviewing = false;
                        self.after_move();
                    }
                }
                Reply::ReviewProgress(done, total) => if self.reviewing && total == self.review_prog.1 { self.review_prog.0 = done; },
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
                        let acc = r.acc;
                        self.review = Some(r);
                        self.reviewing = false;
                        if self.loaded.is_some() { self.msg.clear(); } // the "Avaliação concluída" chip says it
                        self.notify(0, tr!("Análise pronta: precisão {}% das brancas e {}% das pretas.", format!("{:.1}", acc[0]), format!("{:.1}", acc[1])));
                    }
                }
                _ => {}
            }
        }
    }

    // ---------- screenshot import ----------
    /// Reads a board screenshot; true if it gave a legal position (now on the board, in Jogar).
    fn load_image(&mut self, img: image::RgbImage) -> bool {
        match vision::read_board(&img, self.tpl.get_or_insert_with(|| vision::Templates::new(&piece_svgs()))) {
            Ok(scan) => {
                let white_top = vision::white_on_top(&scan.grid);
                self.import = Some(Import { grid: scan.grid, sils: scan.sils, white_top, white_to_move: !white_top, edit: false });
                self.brush = None;
                self.tab = 0; // show it, wherever the picture was pasted
                self.panel_tab = 0;
                self.apply_import(tl("Posição lida do print. Se houver peças mal lidas, corrige-as com o editor."))
            }
            Err(e) => { self.msg = e; false }
        }
    }

    /// Builds the position from the (possibly edited) scan. If it is illegal with the chosen side to move,
    /// tries the other side before giving up.
    fn apply_import(&mut self, ok_msg: &str) -> bool {
        let Some(i) = &self.import else { return false };
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
                self.leave_training();
                self.me = p.turn();
                self.flipped = white_top; // show it like the screenshot
                self.pos = p;
                self.loaded = None;
                self.reset_history();
                self.last = None;
                self.sel = None;
                self.msg = if adjusted { format!("{ok_msg} {}", tl("(Ajustei de quem é a vez para a posição ser legal.)")) } else { ok_msg.to_string() };
                self.after_move();
                true
            }
            Err(e) => { self.msg = tr!("A posição lida não é legal: {}. Corrige as peças com o editor ou muda a vez.", e); false }
        }
    }

    /// Put (or erase) a piece on a square of the scanned position.
    fn edit_square(&mut self, sq: Square, piece: Option<shakmaty::Piece>) {
        let Some(i) = self.import.as_mut() else { return };
        let (r, c) = vision::grid_index(u32::from(sq.file()) as usize, u32::from(sq.rank()) as usize, i.white_top);
        i.grid[r][c] = piece.map(|p| (p.color, p.role));
        self.apply_import(tl("Posição editada."));
    }

    /// Remember the piece shapes of the current (corrected) scan, so the next screenshots of this style read right.
    fn learn_import(&mut self) {
        let Some(i) = &self.import else { return };
        let n = self.tpl.get_or_insert_with(|| vision::Templates::new(&piece_svgs())).learn(&i.grid, &i.sils);
        self.msg = tr!("Aprendi {} peças deste print. Os próximos prints com este estilo devem sair melhores.", n);
    }

    /// Ctrl+V: a screenshot becomes a position, PGN text becomes a game. Always says what happened.
    fn paste_any(&mut self) {
        self.last_paste = self.time;
        if let Ok(d) = arboard::Clipboard::new().and_then(|mut c| c.get_image()) {
            if let Some(im) = image::RgbaImage::from_raw(d.width as u32, d.height as u32, d.bytes.into_owned()) {
                return self.paste_image(image::DynamicImage::ImageRgba8(im).to_rgb8());
            }
        }
        if let Ok(t) = arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
            if self.try_pgn(&t) { let m = self.msg.clone(); return self.notify(0, if m.is_empty() { tl("Partida importada.").to_string() } else { m }); }
        }
        self.notify(1, tl("Na área de transferência não há um print nem um PGN."));
    }

    /// A pasted picture: read the board from it and say how it went.
    fn paste_image(&mut self, img: image::RgbImage) {
        self.last_paste = self.time;
        let ok = self.load_image(img);
        let m = self.msg.clone();
        self.notify(if ok { 0 } else { 1 }, m);
    }

    fn open_file(&mut self, path: &std::path::Path) {
        let is_pgn = path.extension().map_or(false, |e| e.eq_ignore_ascii_case("pgn") || e.eq_ignore_ascii_case("txt"));
        if is_pgn {
            match std::fs::read_to_string(path) {
                Ok(t) => { self.pgn_text = t.clone(); if !self.try_pgn(&t) { self.msg = tl("Não encontrei nenhuma partida neste ficheiro.").into(); } }
                Err(e) => self.msg = tr!("Não consegui abrir o ficheiro: {}", e),
            }
            return;
        }
        match image::open(path) {
            Ok(im) => { self.load_image(im.to_rgb8()); }
            Err(e) => self.msg = tr!("Não consegui abrir a imagem: {}", e),
        }
    }

    fn status(&self) -> String {
        if self.both {
            let check = if self.pos.is_check() { tl(" (xeque!)") } else { "" };
            if self.pos.is_checkmate() { return if self.pos.turn() == Color::Black { tl("Xeque-mate. Ganham as brancas.") } else { tl("Xeque-mate. Ganham as pretas.") }.into(); }
            if self.pos.is_game_over() { return tl("Empate.").into(); }
            return format!("{}{check}", if self.pos.turn() == Color::White { tl("Vez das brancas") } else { tl("Vez das pretas") });
        }
        if self.pos.is_checkmate() {
            return if self.pos.turn() == self.me { tl("Xeque-mate. Perdeste.").into() } else { tl("Xeque-mate. Ganhaste!").into() };
        }
        if self.pos.is_game_over() { return tl("Empate.").into(); }
        if self.pos.turn() == self.me { format!("{}{}", tl("A tua vez"), if self.pos.is_check() { tl(" (xeque!)") } else { "" }) } else { tl("A engine está a pensar…").into() }
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
        let movable = !editing && !shown.is_game_over() && !self.resigned && (self.both || shown.turn() == self.me) && !(self.thinking && live) && self.can_move(live);
        // the promotion column: Q, N, R, B from the promotion square toward the middle of the board
        let promo_col: Vec<(Square, Role)> = self.promo.map(|(_, to)| {
            let down = if to.rank() == shakmaty::Rank::Eighth { -1 } else { 1 };
            [Role::Queen, Role::Knight, Role::Rook, Role::Bishop].into_iter().enumerate()
                .map(|(k, r)| (Square::from_coords(to.file(), shakmaty::Rank::new((u32::from(to.rank()) as i32 + down * k as i32) as u32)), r)).collect()
        }).unwrap_or_default();
        if !promo_col.is_empty() && (resp.clicked() || resp.secondary_clicked()) {
            let at = resp.interact_pointer_pos().and_then(to_sq);
            let pick = if resp.clicked() { promo_col.iter().find(|(s, _)| Some(*s) == at).map(|x| x.1) } else { None };
            self.promote(pick);
        }
        let promo_open = !promo_col.is_empty();
        // right button, chess.com style: drag = arrow, click = square; Shift / Ctrl / Alt pick the colour
        let mods = ui.input(|i| i.modifiers);
        let mark_col: u8 = if mods.shift { 1 } else if mods.ctrl || mods.command { 2 } else if mods.alt { 3 } else { 0 };
        // the square where the button went down (by the time a drag is recognised the pointer may be past it)
        if resp.drag_started_by(egui::PointerButton::Secondary) { self.mark_from = ui.input(|i| i.pointer.press_origin()).and_then(to_sq); }
        if resp.drag_stopped_by(egui::PointerButton::Secondary) {
            if let (Some(a), Some(b)) = (self.mark_from.take(), ui.ctx().pointer_interact_pos().and_then(to_sq)) {
                self.toggle_mark(if a == b { Mark::Square(a, mark_col) } else { Mark::Arrow(a, b, mark_col) });
            }
        }
        if resp.secondary_clicked() {
            if let Some(sq) = resp.interact_pointer_pos().and_then(to_sq) { self.toggle_mark(Mark::Square(sq, mark_col)); }
        }
        // pieces move with the left button only; the right one is just for drawing
        let left_down = resp.drag_started_by(egui::PointerButton::Primary) && !promo_open;
        if (resp.clicked() || left_down) && !matches!(self.mode, Mode::Example) { self.marks.clear(); } // a left click wipes the drawings (not a lesson's arrows)
        if left_down {
            if let Some(sq) = resp.interact_pointer_pos().and_then(to_sq) {
                if movable && shown.board().piece_at(sq).map_or(false, |p| p.color == shown.turn()) {
                    self.sel = Some(sq);
                    self.drag = Some(sq);
                }
            }
        } else if resp.drag_stopped_by(egui::PointerButton::Primary) {
            if let (Some(from), Some(to)) = (self.drag.take(), ui.ctx().pointer_interact_pos().and_then(to_sq)) {
                if from != to {
                    self.sel = None;
                    self.skip_anim = true; // the piece is already under the cursor, no slide
                    if !self.try_move(from, to) { self.skip_anim = false; self.sel = Some(from); }
                }
            }
        } else if resp.clicked() && !promo_open {
            if let Some(sq) = resp.interact_pointer_pos().and_then(to_sq) { self.click(sq); }
        }

        let p = ui.painter_at(rect);
        let legal: Vec<Move> = self.sel.filter(|_| movable).map(|s| shown.legal_moves().into_iter().filter(|m| m.from() == Some(s)).collect()).unwrap_or_default();
        let king_chk = if shown.is_check() { shown.board().king_of(shown.turn()) } else { None };
        for i in 0..64u32 {
            let sq = Square::new(i);
            let r0 = Rect::from_min_size(pos_of(sq), Vec2::splat(cell));
            let light = (u32::from(sq.file()) + u32::from(sq.rank())) % 2 == 1;
            p.rect_filled(r0, 0.0, if light { crate::light() } else { crate::dark() });
            if last.map_or(false, |(a, b)| a == sq || b == sq) || (movable && self.sel == Some(sq)) {
                p.rect_filled(r0, 0.0, Color32::from_rgba_unmultiplied(255, 235, 59, 130));
            }
            for m in &self.marks { if let Mark::Square(ms, c) = m { if *ms == sq { p.rect_filled(r0, 0.0, square_color(*c)); } } }
            if king_chk == Some(sq) {
                p.circle_filled(r0.center(), cell * 0.5, Color32::from_rgba_unmultiplied(230, 0, 0, 170));
                // once, when the check appears on the live position: a ring spreads and fades (0.45 s)
                let idx = self.view.unwrap_or(self.hist.len());
                if live && motion::on() {
                    if self.check_t.0 != idx { self.check_t = (idx, self.time); }
                    let k = ((self.time - self.check_t.1) / 0.45) as f32;
                    if k < 1.0 {
                        let e = motion::ease_out(k);
                        p.circle_stroke(r0.center(), cell * (0.5 + 0.55 * e), Stroke::new(cell * 0.06 * (1.0 - e), Color32::from_rgba_unmultiplied(255, 90, 80, (200.0 * (1.0 - e)) as u8)));
                        ui.ctx().request_repaint();
                    }
                }
            }
            let txt = if light { crate::dark() } else { crate::light() };
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
        // checkmate: a crown on the winner's king, a mark on the mated one
        if shown.is_checkmate() {
            let loser = shown.turn();
            for (c, win) in [(!loser, true), (loser, false)] {
                if let Some(k) = shown.board().king_of(c) {
                    let b = pos_of(k) + Vec2::new(cell * 0.8, cell * 0.2);
                    p.circle_filled(b, cell * 0.17, if win { GOLD } else { Color32::from_rgb(214, 48, 49) });
                    p.circle_stroke(b, cell * 0.17, Stroke::new(cell * 0.02, Color32::WHITE));
                    if win { ico(&p, Ico::Crown, b, cell * 0.2, Color32::from_rgb(26, 18, 4)); }
                    else { p.text(b, egui::Align2::CENTER_CENTER, "#", f_semi(cell * 0.2), Color32::WHITE); }
                }
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
        for (i, h) in self.hints.iter().enumerate().filter(|_| !promo_open) {
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
        if promo_open {
            p.rect_filled(rect, 0.0, Color32::from_black_alpha(110));
            let color = shown.turn();
            for (sq, role) in &promo_col {
                let r = Rect::from_min_size(pos_of(*sq), Vec2::splat(cell));
                let hot = ui.ctx().pointer_hover_pos().is_some_and(|h| r.contains(h));
                p.rect_filled(r.shrink(2.0), cell * 0.5, if hot { GOLD } else { Color32::from_rgb(240, 236, 226) });
                p.image(self.tex[&(color, *role)].id(), r.shrink(cell * 0.08), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            }
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
        // An uncapped search pauses while no board is on screen (play, analysis, lessons), and starts again on return.
        let on_board = self.board_on_screen() && !self.reviewing;
        if self.hint_deep && !on_board { self.stop_deep_hint(); self.hint_paused = true; }
        if self.hint_paused && on_board { self.hint_paused = false; self.request_hints(); }
        self.poll_update();
        self.tick_training();
        // shortcuts: F = flip, Ctrl+V = paste screenshot, dropped file = import.
        // egui turns Ctrl+V into Paste(text) or PasteImage(picture), or into nothing at all when the clipboard
        // holds neither (then only the V key coming back up arrives): every case ends in paste_any / paste_image.
        let (flip, paste_text, paste_img, v_up, dropped) = ctx.input(|i| (
            i.key_pressed(egui::Key::F) && !i.modifiers.command,
            i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))),
            i.events.iter().find_map(|e| if let egui::Event::PasteImage(im) = e { Some(im.clone()) } else { None }),
            i.events.iter().any(|e| matches!(e, egui::Event::Key { key: egui::Key::V, pressed: false, modifiers, .. } if modifiers.command)),
            i.raw.dropped_files.iter().find_map(|f| Some(f.path().to_path_buf())),
        ));
        let paste = paste_text || v_up && self.time - self.last_paste > 1.0;
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
        if !ctx.egui_wants_keyboard_input() { // inside a text box, Ctrl+V is just text
            if let Some(im) = paste_img {
                let [w, h] = im.size;
                self.paste_image(image::RgbImage::from_fn(w as u32, h as u32, |x, y| { let c = im.pixels[y as usize * w + x as usize]; image::Rgb([c.r(), c.g(), c.b()]) }));
            } else if paste { self.paste_any(); }
        }
        if let Some(p) = dropped { self.open_file(&p); }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _f: &mut eframe::Frame) {
        let t0 = std::time::Instant::now();
        self.screens(ui); // src/screens.rs
        // dev: GAMBITO_PERF=1 prints, every second, the frames drawn, the average time of our own drawing code, and why
        if std::env::var_os("GAMBITO_PERF").is_some() {
            thread_local! { static ST: std::cell::Cell<(f64, u32, f64)> = const { std::cell::Cell::new((0.0, 0, 0.0)) }; }
            // who asked for this frame (the first few causes, by file:line)
            thread_local! { static WHY: std::cell::RefCell<std::collections::BTreeMap<String, u32>> = const { std::cell::RefCell::new(std::collections::BTreeMap::new()) }; }
            WHY.with(|w| for c in ui.ctx().repaint_causes() { *w.borrow_mut().entry(format!("{}:{} {}", c.file.rsplit(['\\', '/']).next().unwrap_or(""), c.line, c.reason)).or_default() += 1; });
            let (start, n, ms) = ST.get();
            let ms = ms + t0.elapsed().as_secs_f64() * 1000.0;
            if self.time - start >= 1.0 {
                eprintln!("perf t={:.0} frames={} avg_ui={:.2}ms tab={} anim={} engines_busy={}", self.time, n + 1, (ms) / (n + 1) as f64, self.tab, self.anim.is_some(), self.thinking || self.waiting_hint);
                if n + 1 > 40 { WHY.with(|w| eprintln!("   causes: {:?}", w.borrow())); }
                WHY.with(|w| w.borrow_mut().clear());
                ST.set((self.time, 0, 0.0));
            } else { ST.set((start, n + 1, ms)); }
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

/// Dev: with GAMBITO_PERF=1, prints how long `f` took (startup work).
fn timed<T>(what: &str, f: impl FnOnce() -> T) -> T {
    let t0 = std::time::Instant::now();
    let v = f();
    if std::env::var_os("GAMBITO_PERF").is_some() { eprintln!("startup {what}: {:.0} ms", t0.elapsed().as_secs_f64() * 1000.0); }
    v
}

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
enum Ico { Home, Bars, Book, Puzzle, Cap, Gear, ChevL, ChevR, ChevD, SkipPrev, SkipNext, Play, Pause, Flip, Restart, Sliders, Pencil, Image, Copy, Star, CheckCircle, Target, Search, Download, Kebab,
    ArrowR, Trophy, Flame, Crown, Fork, Link, Skewer, Bolt, Plus2, Shield, Swap, Magnet, Cross, Layers, Flag, King, Knight, Trend, PlayCircle, Users, Turn }

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
        Ico::ArrowR => { line(&[(5.0, 12.0), (19.0, 12.0)]); line(&[(13.0, 6.0), (19.0, 12.0), (13.0, 18.0)]); }
        Ico::Trophy => {
            closed(&[(8.0, 4.0), (16.0, 4.0), (16.0, 9.0), (14.5, 12.0), (12.0, 13.0), (9.5, 12.0), (8.0, 9.0)]);
            line(&[(8.0, 6.0), (5.0, 6.0), (5.0, 8.0), (8.0, 10.5)]); line(&[(16.0, 6.0), (19.0, 6.0), (19.0, 8.0), (16.0, 10.5)]);
            line(&[(12.0, 13.0), (12.0, 17.0)]); line(&[(8.0, 20.0), (16.0, 20.0)]);
        }
        Ico::Flame => closed(&[(12.0, 3.0), (15.0, 7.0), (17.0, 11.0), (17.0, 14.5), (15.5, 18.0), (12.0, 20.0), (8.5, 18.0), (7.0, 14.5), (8.0, 10.5), (9.5, 8.0), (10.5, 11.0)]),
        Ico::Crown => closed(&[(3.0, 8.0), (7.5, 12.0), (12.0, 5.0), (16.5, 12.0), (21.0, 8.0), (19.0, 19.0), (5.0, 19.0)]),
        Ico::Fork => { line(&[(7.0, 3.0), (7.0, 9.0), (9.0, 12.5), (12.0, 14.0), (15.0, 12.5), (17.0, 9.0), (17.0, 3.0)]); line(&[(12.0, 3.0), (12.0, 21.0)]); }
        Ico::Link => { p.circle_stroke(pt(9.0, 14.0), 4.5 * u, sw); p.circle_stroke(pt(15.0, 10.0), 4.5 * u, sw); }
        Ico::Skewer => { line(&[(4.0, 20.0), (20.0, 4.0)]); line(&[(14.0, 4.0), (20.0, 4.0), (20.0, 10.0)]); }
        Ico::Bolt => closed(&[(13.0, 2.0), (5.0, 14.0), (11.0, 14.0), (10.0, 22.0), (18.0, 10.0), (12.0, 10.0)]),
        Ico::Plus2 => { line(&[(3.0, 12.0), (10.0, 12.0)]); line(&[(6.5, 8.5), (6.5, 15.5)]); line(&[(14.0, 12.0), (21.0, 12.0)]); line(&[(17.5, 8.5), (17.5, 15.5)]); }
        Ico::Shield => closed(&[(12.0, 3.0), (20.0, 6.0), (20.0, 12.0), (18.0, 17.0), (12.0, 21.0), (6.0, 17.0), (4.0, 12.0), (4.0, 6.0)]),
        Ico::Swap => { line(&[(4.0, 8.0), (19.0, 8.0)]); line(&[(16.0, 5.0), (19.0, 8.0), (16.0, 11.0)]); line(&[(20.0, 16.0), (5.0, 16.0)]); line(&[(8.0, 13.0), (5.0, 16.0), (8.0, 19.0)]); }
        Ico::Magnet => {
            line(&[(6.0, 4.0), (6.0, 12.0), (7.0, 15.5), (9.0, 17.5), (12.0, 18.2), (15.0, 17.5), (17.0, 15.5), (18.0, 12.0), (18.0, 4.0), (14.0, 4.0), (14.0, 12.0), (12.0, 14.0), (10.0, 12.0), (10.0, 4.0), (6.0, 4.0)]);
        }
        Ico::Cross => { line(&[(5.0, 5.0), (19.0, 19.0)]); line(&[(19.0, 5.0), (5.0, 19.0)]); }
        Ico::Layers => { closed(&[(12.0, 3.0), (21.0, 8.0), (12.0, 13.0), (3.0, 8.0)]); line(&[(3.0, 13.0), (12.0, 18.0), (21.0, 13.0)]); }
        Ico::Flag => { line(&[(5.0, 21.0), (5.0, 4.0)]); closed(&[(5.0, 4.0), (16.0, 4.0), (14.0, 8.0), (16.0, 12.0), (5.0, 12.0)]); }
        Ico::King => { line(&[(12.0, 3.0), (12.0, 8.0)]); line(&[(10.0, 5.0), (14.0, 5.0)]); closed(&[(6.0, 20.0), (18.0, 20.0), (17.0, 15.0), (15.0, 11.0), (12.0, 10.0), (9.0, 11.0), (7.0, 15.0)]); }
        Ico::Knight => closed(&[(7.0, 20.0), (18.0, 20.0), (17.5, 14.0), (16.0, 9.0), (12.5, 5.5), (10.0, 4.5), (10.5, 7.5), (6.0, 11.5), (7.0, 13.5), (10.5, 12.5), (8.0, 16.5)]),
        Ico::Trend => { line(&[(3.0, 17.0), (8.0, 12.0), (12.0, 15.0), (20.0, 7.0)]); line(&[(15.0, 7.0), (20.0, 7.0), (20.0, 12.0)]); }
        Ico::PlayCircle => { p.circle_stroke(pt(12.0, 12.0), 9.0 * u, sw); fill(&[(10.0, 8.5), (10.0, 15.5), (15.5, 12.0)]); }
        Ico::Users => {
            p.circle_stroke(pt(8.0, 8.0), 3.0 * u, sw); p.circle_stroke(pt(16.0, 8.0), 3.0 * u, sw);
            line(&[(3.0, 20.0), (3.5, 17.0), (5.5, 15.0), (8.0, 14.5), (10.5, 15.0), (12.5, 17.0), (13.0, 20.0)]);
            line(&[(13.5, 15.2), (16.0, 14.5), (18.5, 15.0), (20.5, 17.0), (21.0, 20.0)]);
        }
        Ico::Turn => {
            // half white, half black circle with an arrow: whose turn it is
            p.circle_stroke(pt(11.0, 13.0), 8.0 * u, sw);
            let half: Vec<Pos2> = (0..=16).map(|i| { let a = std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 16.0; pt(11.0 + a.cos() * 8.0, 13.0 + a.sin() * 8.0) }).collect();
            p.add(egui::Shape::convex_polygon(half, col, Stroke::NONE));
            line(&[(17.0, 2.5), (21.0, 2.5), (21.0, 6.5)]);
            line(&[(21.0, 2.5), (16.5, 7.0)]);
        }
    }
}

/// Gold button (primary): a raised face with a light top, like the design.
fn gold_btn(ui: &mut egui::Ui, label: &str, icon: Option<Ico>, size: Vec2) -> bool {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let rect = rect.scale_from_center(motion::press(ui.ctx(), &resp)); // pressed: 3% smaller
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
    let rect = rect.scale_from_center(motion::press(ui.ctx(), &resp));
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

/// An on/off switch (Settings): gold when on; the knob slides.
fn toggle(ui: &mut egui::Ui, on: &mut bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(46.0, 26.0), Sense::click());
    if resp.clicked() { *on = !*on; }
    let t = ui.ctx().animate_bool_with_time(resp.id, *on, 0.15);
    let p = ui.painter();
    p.rect_filled(rect, 13.0, Color32::from_rgb(52, 52, 58).lerp_to_gamma(GOLD, t));
    if resp.hovered() { p.rect_stroke(rect, 13.0, Stroke::new(1.0, GOLD_HI), egui::StrokeKind::Outside); }
    let c = Pos2::new(rect.left() + 13.0 + t * (rect.width() - 26.0), rect.center().y);
    p.circle_filled(c, 10.0, if t > 0.5 { Color32::from_rgb(28, 20, 4) } else { CREAM });
    resp.clicked()
}

/// Square icon button; `active` gives it the gold highlight.
fn icon_btn(ui: &mut egui::Ui, k: Ico, size: f32, active: bool, tip: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let rect = rect.scale_from_center(motion::press(ui.ctx(), &resp));
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
/// The evaluation over a game (white's chances, 0-100). In a review, `marks` puts the icons of the big moments on
/// the line and a click jumps to that position (returned).
fn win_chart(ui: &mut egui::Ui, series: &[f32], size: Vec2, mark: Option<usize>, small: bool, marks: &[(usize, Class)]) -> Option<usize> {
    let (rect, resp) = ui.allocate_exact_size(size, if small { Sense::hover() } else { Sense::click() });
    let p = ui.painter_at(rect.expand(8.0));
    if small { p.rect_filled(rect, 12.0, CARD2); p.rect_stroke(rect, 12.0, Stroke::new(1.0, LINE), egui::StrokeKind::Inside); }
    if series.len() < 2 { return None; }
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
    for &(i, class) in marks { if i < series.len() { draw_icon(&p, pt(i), 7.5, class); } }
    if !small {
        if let Some(h) = resp.hover_pos() {
            let i = (((h.x - rect.left() - pad) / w * (series.len() - 1) as f32).round().max(0.0) as usize).min(series.len() - 1);
            p.line_segment([Pos2::new(pt(i).x, rect.top() + pad), Pos2::new(pt(i).x, rect.bottom() - pad)], Stroke::new(1.0, Color32::from_white_alpha(50)));
            if resp.clicked() { return Some(i); }
        }
    }
    None
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
    let mut img = image::imageops::resize(&img, 640, 640, image::imageops::FilterType::Triangle);
    for px in img.pixels_mut() {
        let m = px[0].max(px[1]).max(px[2]);
        px[3] = if m < 16 { 0 } else if m < 44 { ((m as u32 - 16) * 255 / 28) as u8 } else { 255 };
    }
    let (w, h) = (img.width() as usize, img.height() as usize);
    ctx.load_texture("logo", egui::ColorImage::from_rgba_unmultiplied([w, h], img.as_raw()), egui::TextureOptions::LINEAR)
}

/// Where the logo's parts are in the logo picture (normalised): 0 crown, 1 app icon, 2 knight.
fn logo_uv(k: u8) -> Rect {
    let (x0, y0, x1, y1) = match k { 0 => (500.0, 155.0, 755.0, 292.0), 1 => (340.0, 255.0, 915.0, 825.0), _ => (420.0, 300.0, 780.0, 640.0) };
    Rect::from_min_max(Pos2::new(x0 / 1254.0, y0 / 1254.0), Pos2::new(x1 / 1254.0, y1 / 1254.0))
}

/// Window / taskbar icon: the logo's mark.
fn app_icon() -> egui::IconData {
    let img = image::load_from_memory(include_bytes!("../assets/logo.webp")).expect("logo").to_rgba8();
    let crop = image::imageops::crop_imm(&img, 340, 250, 575, 575).to_image();
    let small = image::imageops::resize(&crop, 256, 256, image::imageops::FilterType::Triangle);
    egui::IconData { rgba: small.into_raw(), width: 256, height: 256 }
}

const INNER_COORDS: bool = true; // coordinates inside the edge squares, like chess.com
impl App {
    fn player_name(&self, c: Color) -> String {
        match &self.mode {
            Mode::Puzzle(_) => return if c == self.me { tl("Tu").into() } else { "Puzzle".into() },
            Mode::Example => return if c == Color::White { tl("Brancas").into() } else { tl("Pretas").into() },
            _ => {}
        }
        if let Some(i) = &self.loaded {
            let (n, e) = if c == Color::White { (&i.white, i.white_elo) } else { (&i.black, i.black_elo) };
            return match e { Some(e) => format!("{n} ({e})"), None => n.clone() };
        }
        if self.both { return if c == Color::White { tl("Brancas").into() } else { tl("Pretas").into() }; }
        if c == self.me { tl("Tu").into() } else { self.engines[self.opp].name.clone() }
    }

    /// Second line under a player's name.
    fn player_sub(&self, c: Color, shown: &Chess, top: bool) -> String {
        match &self.mode {
            Mode::Puzzle(r) => return if c == self.me { format!("Rating {:.0}", self.progress.rating) } else { format!("Rating {}", self.puzzles[r.idx].rating) },
            Mode::Example => return String::new(),
            Mode::Drill(_) if c != self.me => return tl("Defesa máxima").into(),
            _ => {}
        }
        if let Some(i) = &self.loaded { return if top { i.result.clone() } else { String::new() }; }
        let diff = explain::material(shown.board(), c) - explain::material(shown.board(), !c);
        if !self.both && c != self.me { return format!("Elo {}", self.elo); }
        if diff > 0 { tr!("+{} de material", diff) } else { String::new() }
    }

    fn finished(&self) -> bool { self.pos.is_game_over() || self.loaded.is_some() || self.resigned }

    /// Dev only: with GAMBITO_SHOT=<png>, saves a picture of this window after a few seconds and quits.
    fn dev_screenshot(&mut self, ctx: &egui::Context) {
        let Some(path) = std::env::var_os("GAMBITO_SHOT") else { return };
        ctx.request_repaint_after(std::time::Duration::from_millis(400));
        let at: f64 = std::env::var("GAMBITO_SHOT_AT").ok().and_then(|s| s.parse().ok()).unwrap_or(9.0);
        if self.time > at && !self.shot_requested {
            self.shot_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img| {
                let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
                if let Some(im) = image::RgbaImage::from_raw(img.size[0] as u32, img.size[1] as u32, bytes) { let _ = im.save(&path); }
                std::process::exit(0);
            })));
        }
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
        let (w, b) = self.loaded.as_ref().map(|i| (i.white.clone(), i.black.clone())).unwrap_or((tl("Brancas").into(), tl("Pretas").into()));
        let last = &self.pos;
        let res = if self.resigned { if self.me == Color::White { "0-1" } else { "1-0" } }
            else if last.is_checkmate() { if last.turn() == Color::Black { "1-0" } else { "0-1" } }
            else if last.is_game_over() { "1/2-1/2" }
            else {
                // the result text was written in the language of the moment: accept either
                let is = |r: &str, pt: &'static str| r.starts_with(pt) || r.starts_with(tl(pt));
                match self.loaded.as_ref().map(|i| i.result.as_str()) { Some(r) if is(r, "Brancas ganham") => "1-0", Some(r) if is(r, "Pretas ganham") => "0-1", Some(r) if is(r, "Empate") => "1/2-1/2", _ => "*" }
            };
        let sans: Vec<String> = self.hist.iter().map(|h| h.1.clone()).collect();
        let start = self.pos_at(0);
        let mut s = format!("[Event \"Gambito\"]\n[Site \"?\"]\n[Date \"????.??.??\"]\n[White \"{w}\"]\n[Black \"{b}\"]\n[Result \"{res}\"]\n");
        if fen_of(start) != fen_of(&Chess::default()) { s += &format!("[SetUp \"1\"]\n[FEN \"{}\"]\n", fen_of(start)); }
        s += &format!("\n{} {res}\n", numbered(start, &sans));
        s
    }

    /// (title, text) of the card at the top: what the last move did, or a hint.
    fn hero_text(&self, idx: usize) -> (String, String) {
        if idx > 0 {
            let (prev, mv) = (self.pos_at(idx - 1), &self.mvs[idx - 1]);
            let (san, why, _) = explain(prev, mv);
            let tag = match self.quality.get(idx - 1).and_then(|x| x.as_ref()) {
                Some((c, Some(best))) => format!("  ·  {}", tr!("{} (melhor era {})", c.label(), best)),
                Some((c, None)) => format!("  ·  {}", c.label()),
                None => String::new(),
            };
            return (format!("{san}{tag}"), why);
        }
        if self.loaded.is_some() || self.pos.is_game_over() { (tl("Partida pronta.").into(), tl("Usa as setas ← → para percorrer a partida e vê a Análise.").into()) }
        else { (tl("Joga um lance.").into(), tl("Eu digo o que ele faz e qual era o melhor.").into()) }
    }

    fn hints_block(&mut self, ui: &mut egui::Ui) {
        if !self.hints_on {
            card(ui, |ui| {
                ui.label(egui::RichText::new(tl("Sugestões desligadas")).color(MUTED));
                ui.horizontal(|ui| {
                    if toggle(ui, &mut self.hints_on) { i18n::save_setting("hints_on", serde_json::json!(self.hints_on)); self.request_hints(); }
                    ui.label(med(tl("Mostrar as 3 melhores jogadas"), 15.0));
                });
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
                        ui.label(med(tl("Melhor jogada"), 17.0));
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
                        if let Some(s) = &series { win_chart(ui, s, Vec2::new(spark_w, 62.0), Some(idx), true, &[]); }
                    });
                });
        } else {
            card(ui, |ui| { ui.label(egui::RichText::new(if self.waiting_hint { tl("A analisar…") } else { tl("Sem sugestões para esta posição.") }).color(MUTED)); });
        }
        ui.add_space(4.0);

        // strength
        ui.horizontal(|ui| {
            ui.label(med(tl("Elo das sugestões"), 17.0));
            let depth = match (self.hint_depth(), self.hint_at) {
                (0, 0) => tl("(profundidade máx.)").to_string(),
                (0, at) => tr!("(profundidade {})", at),
                (d, _) => tr!("(profundidade {})", d),
            };
            ui.label(egui::RichText::new(depth).small().color(MUTED));
        });
        ui.scope(|ui| {
            ui.spacing_mut().slider_width = (ui.available_width() - 96.0).max(80.0);
            ui.visuals_mut().slider_trailing_fill = true;
            ui.visuals_mut().selection.bg_fill = GOLD;
            ui.visuals_mut().widgets.inactive.bg_fill = Color32::from_rgb(44, 44, 49);
            ui.visuals_mut().widgets.hovered.bg_fill = GOLD_HI;
            ui.visuals_mut().widgets.active.bg_fill = GOLD;
            if ui.add(egui::Slider::new(&mut self.hint_elo, 1000..=2600).step_by(50.0)).changed() { i18n::save_setting("hint_elo", serde_json::json!(self.hint_elo)); self.request_hints(); }
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
        self.msg = tl("Editor aberto: escolhe uma peça e clica nas casas.").into();
    }

    /// Loads a position from a FEN typed in the FEN box.
    fn load_fen(&mut self, fen: &str) {
        match fen.trim().parse::<Fen>().map_err(|e| e.to_string())
            .and_then(|f| f.into_position::<Chess>(CastlingMode::Standard).map_err(|e| pos_error(&e.to_string())))
        {
            Ok(p) => {
                self.leave_training();
                self.pos = p;
                self.loaded = None;
                self.reset_history();
                self.me = self.pos.turn();
                self.last = None;
                self.sel = None;
                self.msg = tl("Posição carregada do FEN.").into();
                self.after_move();
            }
            Err(e) => self.msg = tr!("FEN inválido: {}", e),
        }
    }

    // ---------- PGN / chess.com games ----------
    fn cc_search(&mut self, more: bool) {
        let user = chesscom::clean_user(&self.cc_user);
        if user.is_empty() { self.cc_err = tl("Escreve o nome de utilizador.").into(); return; }
        if !more || user != self.cc_query {
            self.cc_games.clear();
            self.cc_archives.clear();
            self.cc_loaded = 0;
            self.cc_query = user.clone();
        }
        self.cc_err.clear();
        self.cc_busy = true;
        if self.cc_site == 1 {
            let until = self.cc_archives.first().and_then(|u| u.parse().ok()).filter(|_| more);
            chesscom::fetch_lichess(user, until, self.net_tx.clone(), self.ctx.clone());
        } else {
            chesscom::fetch(user, self.cc_archives.clone(), self.cc_loaded, if more { 3 } else { 2 }, self.net_tx.clone(), self.ctx.clone());
        }
    }

    fn load_cc_game(&mut self, i: usize) {
        let g = self.cc_games[i].clone();
        let Some(pgn) = pgn::parse_games(&g.pgn).into_iter().find(|p| !p.moves.is_empty()) else {
            self.msg = tl("Esta partida não tem lances.").into();
            return;
        };
        let me = if g.black.eq_ignore_ascii_case(&self.cc_query) { Color::Black } else { Color::White };
        let result = match (g.white_result.as_str(), g.black_result.as_str()) {
            ("win", _) => tl("Brancas ganham"),
            (_, "win") => tl("Pretas ganham"),
            _ => tl("Empate"),
        };
        let how = chesscom::termination(&g);
        let info = GameInfo {
            white: g.white.clone(), black: g.black.clone(), white_elo: g.white_elo, black_elo: g.black_elo,
            result: format!("{result}{} · {} · {}", if how.is_empty() { String::new() } else { format!(" {}", tr!("por {}", how)) }, g.time_class, chesscom::date(g.end_time)),
            cc_acc: g.acc,
        };
        self.load_game(&pgn, info, me);
        self.game_idx = Some((0, i));
    }

    fn load_pgn_game(&mut self, g: &pgn::Game) {
        let elo = |k: &str| g.header(k).and_then(|v| v.parse::<u32>().ok());
        let result = match g.header("Result") { Some("1-0") => tl("Brancas ganham"), Some("0-1") => tl("Pretas ganham"), Some("1/2-1/2") => tl("Empate"), _ => "" };
        let mut extra: Vec<&str> = vec![result];
        extra.extend(g.header("Termination"));
        extra.extend(g.header("Date").filter(|d| !d.contains('?')));
        let info = GameInfo {
            white: g.header("White").unwrap_or(tl("Brancas")).to_string(), black: g.header("Black").unwrap_or(tl("Pretas")).to_string(),
            white_elo: elo("WhiteElo"), black_elo: elo("BlackElo"),
            result: extra.into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · "), cc_acc: None,
        };
        self.load_game(g, info, Color::White);
    }

    /// Puts a whole game on the board, opens it at the first move (like chess.com) and evaluates it.
    fn load_game(&mut self, g: &pgn::Game, info: GameInfo, me: Color) {
        let (start, moves) = match g.replay() {
            Ok(x) => x,
            Err(e) => { self.msg = tr!("Não consegui ler a partida: {}", e); return; }
        };
        self.leave_training();
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
        self.tab = 2; // opens in Análise da partida
        self.panel_tab = 2;
        self.msg = tr!("Partida carregada: {} lances. A avaliar…", self.mvs.len());
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
                self.msg = tr!("O PGN tem {} partidas: escolhe uma.", n);
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
    if let Some(m) = setup::mode() { i18n::init(); return setup::run(m); } // Gambito-Setup.exe / --uninstall
    migrate_old_data();
    i18n::init();
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
