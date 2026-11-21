//! Training data: the puzzles (a sample of the Lichess puzzle database, CC0), the player's puzzle
//! rating and lesson progress (kept in %APPDATA%\Gambito\progress.json), and the lessons.
use std::collections::HashMap;
use std::io::Read;
use crate::i18n::tl;

pub struct Puzzle {
    pub id: String,
    pub fen: String,        // position before the opponent's move
    pub moves: Vec<String>, // UCI; moves[0] is the opponent's, then the player and the opponent alternate
    pub rating: u32,
    pub themes: Vec<String>,
}

/// assets/puzzles.tsv.gz: id, FEN, moves, rating, themes (tab-separated; lists space-separated).
pub fn load_puzzles() -> Vec<Puzzle> {
    let mut text = String::new();
    let _ = flate2::read::GzDecoder::new(&include_bytes!("../assets/puzzles.tsv.gz")[..]).read_to_string(&mut text);
    text.lines().filter_map(|l| {
        let f: Vec<&str> = l.split('\t').collect();
        if f.len() < 5 { return None; }
        Some(Puzzle {
            id: f[0].into(), fen: f[1].into(), moves: f[2].split(' ').map(String::from).collect(),
            rating: f[3].parse().ok()?, themes: f[4].split(' ').map(String::from).collect(),
        })
    }).collect()
}

/// Lichess theme names in Portuguese (the ones worth showing).
pub fn theme_name(t: &str) -> Option<&'static str> {
    Some(tl(match t {
        "mate" => "Mate", "mateIn1" => "Mate em 1", "mateIn2" => "Mate em 2", "mateIn3" => "Mate em 3", "mateIn4" => "Mate em 4",
        "fork" => "Garfo", "pin" => "Cravada", "skewer" => "Espeto", "discoveredAttack" => "Ataque descoberto",
        "doubleCheck" => "Xeque duplo", "hangingPiece" => "Peça desprotegida", "deflection" => "Desvio", "attraction" => "Atração",
        "sacrifice" => "Sacrifício", "backRankMate" => "Mate na última fila", "smotheredMate" => "Mate abafado",
        "promotion" => "Promoção", "advancedPawn" => "Peão avançado", "endgame" => "Final", "middlegame" => "Meio-jogo",
        "opening" => "Abertura", "trappedPiece" => "Peça presa", "quietMove" => "Lance calmo", "defensiveMove" => "Defesa",
        "kingsideAttack" => "Ataque ao roque", "queensideAttack" => "Ataque na ala da dama", "exposedKing" => "Rei exposto",
        "capturingDefender" => "Eliminar o defensor", "interference" => "Interferência", "intermezzo" => "Lance intermédio",
        "xRayAttack" => "Raio X", "zugzwang" => "Zugzwang", "clearance" => "Desobstrução", "rookEndgame" => "Final de torres",
        "pawnEndgame" => "Final de peões", "queenEndgame" => "Final de damas", "bishopEndgame" => "Final de bispos",
        "knightEndgame" => "Final de cavalos", "castling" => "Roque", "enPassant" => "En passant", "underPromotion" => "Subpromoção",
        "anastasiaMate" => "Mate de Anastácia", "arabianMate" => "Mate árabe", "bodenMate" => "Mate de Boden",
        "hookMate" => "Mate do gancho", "doubleBishopMate" => "Mate dos dois bispos", "dovetailMate" => "Mate da cauda de andorinha",
        "crushing" => "Vantagem decisiva", "advantage" => "Vantagem", "equality" => "Igualar", "short" => "Curto", "long" => "Longo",
        "veryLong" => "Muito longo", "oneMove" => "Um lance",
        _ => return None,
    }))
}

/// Themes offered in the filter of the Exercícios screen.
pub const FILTERS: [&str; 15] = ["mateIn1", "mateIn2", "fork", "pin", "skewer", "discoveredAttack", "doubleCheck", "hangingPiece",
    "deflection", "attraction", "sacrifice", "backRankMate", "promotion", "endgame", "opening"];

// ---------- progress ----------

pub struct Progress {
    pub rating: f64,
    pub solved: u32,
    pub failed: u32,
    pub streak: u32,
    pub best_streak: u32,
    pub seen: Vec<String>,            // puzzle ids already played (not offered again)
    pub lessons: HashMap<String, u32>, // lesson id -> practice items done
    pub history: Vec<f64>,            // rating after each puzzle (last 60)
    pub daily: Option<(u64, bool)>,   // (day, solved) of the puzzle of the day
    pub games: HashMap<String, f64>,  // best score of each mini-game (games.rs)
}

fn progress_path() -> Option<std::path::PathBuf> {
    std::env::var_os("APPDATA").map(|a| std::path::PathBuf::from(a).join("Gambito").join("progress.json"))
}

impl Progress {
    pub fn load() -> Self {
        let v: serde_json::Value = progress_path().and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let n = |k: &str| v[k].as_u64().unwrap_or(0) as u32;
        Progress {
            rating: v["rating"].as_f64().unwrap_or(1200.0),
            solved: n("solved"), failed: n("failed"), streak: n("streak"), best_streak: n("best_streak"),
            seen: v["seen"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
            lessons: v["lessons"].as_object().map(|o| o.iter().map(|(k, x)| (k.clone(), x.as_u64().unwrap_or(0) as u32)).collect()).unwrap_or_default(),
            history: v["history"].as_array().map(|a| a.iter().filter_map(|x| x.as_f64()).collect()).unwrap_or_default(),
            daily: v["daily"].as_array().and_then(|a| Some((a.first()?.as_u64()?, a.get(1)?.as_bool()?))),
            games: v["games"].as_object().map(|o| o.iter().filter_map(|(k, x)| Some((k.clone(), x.as_f64()?))).collect()).unwrap_or_default(),
        }
    }

    pub fn save(&self) {
        let v = serde_json::json!({
            "rating": self.rating, "solved": self.solved, "failed": self.failed, "streak": self.streak,
            "best_streak": self.best_streak, "seen": self.seen, "lessons": self.lessons, "history": self.history,
            "daily": self.daily.map(|(d, s)| serde_json::json!([d, s])),
            "games": self.games,
        });
        if let Some(p) = progress_path() {
            if let Some(d) = p.parent() { let _ = std::fs::create_dir_all(d); }
            let _ = std::fs::write(p, v.to_string());
        }
    }

    /// Elo update after a puzzle; returns the change. Bigger steps for the first 15 puzzles.
    pub fn record(&mut self, puzzle_rating: u32, won: bool) -> f64 {
        let k = if self.solved + self.failed < 15 { 40.0 } else { 20.0 };
        let expected = 1.0 / (1.0 + 10f64.powf((puzzle_rating as f64 - self.rating) / 400.0));
        let delta = k * (if won { 1.0 } else { 0.0 } - expected);
        self.rating = (self.rating + delta).clamp(400.0, 3000.0);
        if won {
            self.solved += 1;
            self.streak += 1;
            self.best_streak = self.best_streak.max(self.streak);
        } else {
            self.failed += 1;
            self.streak = 0;
        }
        self.history.push(self.rating);
        if self.history.len() > 60 { self.history.remove(0); }
        delta
    }
}

/// Days since 1970 (for the puzzle of the day).
pub fn today() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() / 86_400)
}

/// Small xorshift generator: enough to pick puzzles.
pub struct Rng(pub u64);
impl Rng {
    pub fn new() -> Self {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
        Rng(t | 1)
    }
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// A puzzle near `target` rating with the theme, not played before. The window widens until one is found.
pub fn pick(puzzles: &[Puzzle], seen: &[String], theme: Option<&str>, target: f64, rng: &mut Rng) -> Option<usize> {
    let fits = |p: &Puzzle| theme.map_or(true, |t| p.themes.iter().any(|x| x == t));
    for (w, allow_seen) in [(100.0, false), (200.0, false), (400.0, false), (9999.0, false), (9999.0, true)] {
        let c: Vec<usize> = (0..puzzles.len())
            .filter(|&i| fits(&puzzles[i]) && (puzzles[i].rating as f64 - target).abs() <= w && (allow_seen || !seen.contains(&puzzles[i].id)))
            .collect();
        if !c.is_empty() { return Some(c[(rng.next() % c.len() as u64) as usize]); }
    }
    None
}

/// The puzzle of the day: same for everyone on a given day, from the 1000–1800 range.
pub fn daily(puzzles: &[Puzzle], day: u64) -> Option<usize> {
    let c: Vec<usize> = (0..puzzles.len()).filter(|&i| (1000..=1800).contains(&puzzles[i].rating)).collect();
    (!c.is_empty()).then(|| c[(day.wrapping_mul(2_654_435_761) % c.len() as u64) as usize])
}

// ---------- lessons ----------

pub enum Practice {
    /// Solve puzzles of this theme (up to this rating).
    Puzzles(&'static str, u32),
    /// Play this position against the engine until the goal.
    Endgame(&'static str, Goal),
}

#[derive(Clone, Copy, PartialEq)]
pub enum Goal { Mate, Promote }

pub struct Lesson {
    pub id: &'static str,
    pub group: &'static str,
    pub title: &'static str,
    pub text: &'static str,
    /// Example on the board (FEN) and arrows (from, to); None = the easiest puzzle of the theme.
    pub example: Option<(&'static str, &'static [(&'static str, &'static str, u8)])>, // arrows: from, to, colour (1 green = the move, 2 red = a threat, 0 = only the target square, red)
    pub practice: Practice,
}

/// How many puzzles complete a lesson.
pub const LESSON_PUZZLES: u32 = 5;

pub const LESSONS: &[Lesson] = &[
    Lesson { id: "fork", group: "Táticas", title: "Garfo",
        text: "Uma peça ataca duas ao mesmo tempo. O adversário só consegue salvar uma, e tu ganhas a outra. O cavalo é o rei dos garfos, porque salta e ataca em todas as direções, mas qualquer peça pode dar um, até um peão.\n\nNo exemplo, Cc7+ dá xeque ao rei e ataca a torre: o rei tem de sair e a torre cai.",
        example: Some(("r3k3/8/8/3N4/8/8/8/4K3 w - - 0 1", &[("d5", "c7", 1), ("c7", "e8", 0), ("c7", "a8", 0)])),
        practice: Practice::Puzzles("fork", 1600) },
    Lesson { id: "pin", group: "Táticas", title: "Cravada",
        text: "Uma peça fica presa porque, se sair, deixa exposta uma peça mais valiosa atrás dela. Se atrás está o rei, a peça cravada nem pode mexer-se (cravada absoluta).\n\nNo exemplo, o bispo crava o cavalo ao rei: o cavalo não sai de c6. A seguir, ataca a peça cravada com outra, porque ela não foge.",
        example: Some(("4k3/8/2n5/8/B7/8/8/4K3 w - - 0 1", &[("a4", "e8", 2)])),
        practice: Practice::Puzzles("pin", 1600) },
    Lesson { id: "skewer", group: "Táticas", title: "Espeto",
        text: "É uma cravada ao contrário: a peça mais valiosa está à frente. Quando ela foge do ataque, a peça de trás fica à mercê.\n\nNo exemplo, Tb1+ dá xeque ao rei. O rei tem de sair da coluna b e a torre come a dama em b8.",
        example: Some(("1q6/8/8/1k6/8/8/8/4K2R w - - 0 1", &[("h1", "b1", 1), ("b1", "b8", 2)])),
        practice: Practice::Puzzles("skewer", 1700) },
    Lesson { id: "discoveredAttack", group: "Táticas", title: "Ataque descoberto",
        text: "Mexes uma peça e abres a linha de outra que estava atrás dela. Ficam duas ameaças num só lance: a da peça que se mexeu e a da que ficou a descoberto.\n\nNo exemplo, o bispo come a dama em a8 e, ao sair da coluna e, a torre dá xeque ao rei.",
        example: Some(("q3k3/8/8/8/4B3/8/8/K3R3 w - - 0 1", &[("e4", "a8", 1), ("e1", "e8", 2)])),
        practice: Practice::Puzzles("discoveredAttack", 1700) },
    Lesson { id: "doubleCheck", group: "Táticas", title: "Xeque duplo",
        text: "Duas peças dão xeque ao mesmo tempo. Não há bloqueio nem captura que resolva os dois xeques: o rei é obrigado a fugir. É o ataque descoberto mais forte que existe.\n\nNo exemplo, Cf6+ dá xeque com o cavalo e, ao mesmo tempo, abre a coluna e para a torre.",
        example: Some(("4k3/8/8/8/4N3/8/8/K3R3 w - - 0 1", &[("e4", "f6", 1), ("e1", "e8", 2)])),
        practice: Practice::Puzzles("doubleCheck", 1800) },
    Lesson { id: "hangingPiece", group: "Táticas", title: "Peças desprotegidas",
        text: "Antes de cada lance, faz duas perguntas: o que é que o adversário deixou sem defesa? E o que é que eu deixei? Uma peça atacada e sem defensores pode simplesmente ser comida.\n\nNo exemplo, o bispo em e6 não tem quem o defenda: Cxe6.",
        example: Some(("4k3/8/4b3/8/3N4/8/8/4K3 w - - 0 1", &[("d4", "e6", 1)])),
        practice: Practice::Puzzles("hangingPiece", 1400) },
    Lesson { id: "deflection", group: "Táticas", title: "Desvio",
        text: "Uma peça do adversário tem uma tarefa: defender uma casa ou outra peça. Obriga-a a sair (com um xeque, uma captura ou uma oferta) e o que ela guardava fica desprotegido.",
        example: None, practice: Practice::Puzzles("deflection", 1800) },
    Lesson { id: "attraction", group: "Táticas", title: "Atração",
        text: "Atrais uma peça do adversário, muitas vezes o rei, para uma casa má, quase sempre com um sacrifício. Lá, ela fica sujeita a um garfo, a uma cravada ou a mate.",
        example: None, practice: Practice::Puzzles("attraction", 1900) },
    Lesson { id: "mateIn1", group: "Mates", title: "Mate em 1",
        text: "Procura primeiro os xeques. Para cada xeque, vê se o rei tem para onde fugir, se alguém pode tapar a linha ou comer a peça que dá xeque. Se nada disso for possível, é mate.\n\nNo exemplo, Dg7# é mate: a dama está protegida pelo rei e tira todas as casas ao rei preto.",
        example: Some(("7k/8/6K1/8/8/8/8/6Q1 w - - 0 1", &[("g1", "g7", 1)])),
        practice: Practice::Puzzles("mateIn1", 1400) },
    Lesson { id: "backRankMate", group: "Mates", title: "Mate na última fila",
        text: "O rei rocado fica atrás dos seus peões. Se nenhuma peça guardar a última fila, uma torre ou a dama dá mate lá atrás. Para te defenderes, abre uma \"janela\" (h3 ou g3) ou mantém uma torre na primeira fila.\n\nNo exemplo, Ta8#.",
        example: Some(("6k1/5ppp/8/8/8/8/8/R5K1 w - - 0 1", &[("a1", "a8", 1)])),
        practice: Practice::Puzzles("backRankMate", 1600) },
    Lesson { id: "smotheredMate", group: "Mates", title: "Mate abafado",
        text: "O rei está rodeado pelas próprias peças e não tem para onde ir. Basta um xeque de cavalo, que não se pode tapar.\n\nNo exemplo, Cf7#.",
        example: Some(("6rk/6pp/8/6N1/8/8/8/6K1 w - - 0 1", &[("g5", "f7", 1)])),
        practice: Practice::Puzzles("smotheredMate", 1900) },
    Lesson { id: "mateIn2", group: "Mates", title: "Mate em 2",
        text: "Dois lances para o mate. Muitas vezes o primeiro é um xeque ou um sacrifício que tira a última casa de fuga ao rei. Pensa em todas as respostas do adversário antes de jogar.",
        example: None, practice: Practice::Puzzles("mateIn2", 1700) },
    Lesson { id: "kq", group: "Finais", title: "Rei e dama contra rei",
        text: "Com a dama, empurra o rei adversário para a borda: põe-na a um salto de cavalo dele e vai seguindo-o. Quando ele estiver na borda, traz o teu rei e dá mate.\n\nCuidado com o afogamento: se o rei adversário não tiver nenhum lance e não estiver em xeque, é empate. Deixa-lhe sempre uma casa até dares o mate.",
        example: Some(("8/8/8/4k3/8/8/8/3QK3 w - - 0 1", &[])),
        practice: Practice::Endgame("8/8/8/4k3/8/8/8/3QK3 w - - 0 1", Goal::Mate) },
    Lesson { id: "kr", group: "Finais", title: "Rei e torre contra rei",
        text: "A torre corta o rei numa fila ou coluna, como uma parede. O teu rei aproxima-se de frente ao outro (oposição), e então um xeque de torre empurra-o uma fila para trás. Repete até à borda e dá mate.\n\nSe o rei adversário atacar a torre, afasta-a para o outro lado do tabuleiro.",
        example: Some(("8/8/8/4k3/8/8/8/R3K3 w - - 0 1", &[])),
        practice: Practice::Endgame("8/8/8/4k3/8/8/8/R3K3 w - - 0 1", Goal::Mate) },
    Lesson { id: "kp", group: "Finais", title: "Rei e peão: a oposição",
        text: "O rei vai à frente do peão e abre-lhe caminho. Oposição: os reis frente a frente, com uma casa entre eles, e o adversário é que tem de jogar. Quem tem a oposição obriga o outro rei a ceder.\n\nCom o teu rei na sexta fila à frente do peão, a vitória é certa. Leva o peão até à promoção.",
        example: Some(("3k4/8/3K4/3P4/8/8/8/8 w - - 0 1", &[("d6", "e6", 1)])),
        practice: Practice::Endgame("3k4/8/3K4/3P4/8/8/8/8 w - - 0 1", Goal::Promote) },
    Lesson { id: "promotion", group: "Finais", title: "Promoção",
        text: "Um peão que chega à última fila vira dama (ou outra peça). Às vezes vale tudo, até sacrificar uma peça, para o fazer passar. Conta os tempos: quem chega primeiro?",
        example: None, practice: Practice::Puzzles("promotion", 1700) },
    Lesson { id: "opening", group: "Aberturas", title: "Princípios da abertura",
        text: "1. Ocupa o centro com peões (e4, d4).\n2. Desenvolve cavalos e bispos antes de mexer a dama.\n3. Faz o roque cedo para pôr o rei em segurança.\n4. Não mexas a mesma peça duas vezes sem razão.\n5. Liga as torres.\n\nDepois pratica com puzzles da abertura: armadilhas e táticas que aparecem logo nos primeiros lances.",
        example: Some(("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1", &[("e2", "e4", 1), ("g1", "f3", 1), ("f1", "c4", 1), ("e1", "g1", 1)])),
        practice: Practice::Puzzles("opening", 1600) },
];

// ---------- screens: Exercícios, Aprender, Início ----------
use crate::{card, dark_btn, dest, draw_icon, eval_pill, explain, gold_btn, med, numbered, semi, win_chart};
use crate::{App, Class, Drill, Ico, Mark, Mode, PuzzleRun, Snapshot, AMBER, CREAM, GOLD, MUTED, POS};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use shakmaty::{fen::Fen, san::SanPlus, uci::UciMove, CastlingMode, Chess, Color, Move, Position};

const RED: Color32 = Color32::from_rgb(235, 107, 92);

fn parse_fen(f: &str) -> Option<Chess> {
    f.parse::<Fen>().ok()?.into_position(CastlingMode::Standard).ok()
}

fn uci_move(u: &str, pos: &Chess) -> Option<Move> {
    UciMove::from_ascii(u.as_bytes()).ok()?.to_move(pos).ok()
}

/// Theme names worth showing as tags (no "short", "advantage", ...).
fn shown_themes(themes: &[String]) -> Vec<&'static str> {
    const META: [&str; 8] = ["short", "long", "veryLong", "oneMove", "crushing", "advantage", "equality", "mate"];
    themes.iter().filter(|t| !META.contains(&t.as_str())).filter_map(|t| theme_name(t)).collect()
}

/// Ratings mapped to 10..90 so the small chart can draw them.
fn rating_series(h: &[f64]) -> Vec<f32> {
    let (lo, hi) = h.iter().fold((f64::MAX, f64::MIN), |(a, b), &x| (a.min(x), b.max(x)));
    let span = (hi - lo).max(40.0);
    h.iter().map(|&x| (10.0 + 80.0 * (x - lo) / span) as f32).collect()
}

impl App {
    /// Training modes decide when the board takes moves.
    pub(crate) fn can_move(&self, live: bool) -> bool {
        match &self.mode {
            Mode::Free => true,
            Mode::Example => false,
            Mode::Puzzle(r) => live && !r.done && !r.showing && r.next_at.is_none() && r.undo_at.is_none(),
            Mode::Drill(d) => live && d.done.is_none(),
        }
    }

    /// Another game or position took the board: back to normal play, the saved game is dropped.
    pub(crate) fn leave_training(&mut self) {
        if matches!(self.mode, Mode::Free) { return; }
        self.mode = Mode::Free;
        self.snapshot = None;
        self.marks.clear();
        if matches!(self.tab, 3 | 4) { self.tab = 0; }
    }

    /// Keeps the game on the board aside before a puzzle or lesson takes it.
    fn enter_training(&mut self) {
        if matches!(self.mode, Mode::Free) && self.snapshot.is_none() {
            self.snapshot = Some(Snapshot {
                pos: self.pos.clone(), hist: std::mem::take(&mut self.hist), mvs: std::mem::take(&mut self.mvs),
                quality: std::mem::take(&mut self.quality), view: self.view.take(), review: self.review.take(), me: self.me,
                flipped: self.flipped, last: self.last, loaded: self.loaded.take(), both: self.both, game_idx: self.game_idx.take(),
            });
        }
        self.import = None;
        self.brush = None;
        self.show_picture = false;
        self.playing = false;
    }

    /// Puts a training position on the board (no history, nobody thinking).
    fn set_board(&mut self, pos: Chess, me: Color, mode: Mode) {
        self.enter_training();
        self.pos = pos;
        self.reset_history();
        self.loaded = None;
        self.both = false;
        self.game_idx = None;
        self.me = me;
        self.flipped = me == Color::Black;
        self.last = None;
        self.sel = None;
        self.drag = None;
        self.anim = None;
        self.hints.clear();
        self.waiting_hint = false;
        self.thinking = false;
        self.msg.clear();
        self.mode = mode;
    }

    /// Back to the game that was on the board before (Jogar).
    pub(crate) fn back_to_game(&mut self) {
        if matches!(self.mode, Mode::Free) { return; }
        self.mode = Mode::Free;
        self.marks.clear();
        self.anim = None;
        self.sel = None;
        match self.snapshot.take() {
            Some(s) => {
                (self.pos, self.hist, self.mvs, self.quality, self.view, self.review) = (s.pos, s.hist, s.mvs, s.quality, s.view, s.review);
                (self.me, self.flipped, self.last, self.loaded, self.both, self.game_idx) = (s.me, s.flipped, s.last, s.loaded, s.both, s.game_idx);
            }
            None => { self.pos = Chess::default(); self.reset_history(); self.me = Color::White; self.flipped = false; self.last = None; }
        }
        self.after_move();
    }

    fn run_mut(&mut self, f: impl FnOnce(&mut PuzzleRun)) {
        if let Mode::Puzzle(r) = &mut self.mode { f(r) }
    }

    // ---------- puzzles ----------

    pub(crate) fn start_puzzle(&mut self, idx: usize, lesson: Option<usize>, daily: bool) {
        let Some(pos) = parse_fen(&self.puzzles[idx].fen) else { return };
        let me = !pos.turn(); // the first move of a puzzle is the opponent's
        let id = self.puzzles[idx].id.clone();
        let rated = !self.progress.seen.contains(&id);
        let next_at = Some(self.time + 0.8);
        self.set_board(pos, me, Mode::Puzzle(PuzzleRun { idx, step: 0, checked: 0, failed: false, hinted: 0, done: false, delta: None, lesson, daily, rated, showing: false, next_at, undo_at: None }));
        self.thinking = true;
        if rated { self.progress.seen.push(id); self.progress.save(); }
    }

    /// A new puzzle: the lesson's theme, or the filter of the Exercícios screen, near the player's rating.
    pub(crate) fn next_puzzle(&mut self, lesson: Option<usize>) {
        let (theme, target) = match lesson.map(|l| &LESSONS[l].practice) {
            Some(Practice::Puzzles(t, max)) => (Some(*t), self.progress.rating.min(*max as f64 - 150.0)),
            _ => (self.pz_theme, self.progress.rating),
        };
        match pick(&self.puzzles, &self.progress.seen, theme, target, &mut self.rng) {
            Some(i) => self.start_puzzle(i, lesson, false),
            None => self.msg = tl("Não encontrei puzzles com esse tema.").into(),
        }
    }

    /// Timed parts of a puzzle: the opponent's replies and taking back a wrong move.
    pub(crate) fn tick_training(&mut self) {
        if let Ok(how) = std::env::var("GAMBITO_SOLVE") { self.dev_solve(&how); }
        let Mode::Puzzle(r) = &self.mode else { return };
        let (undo, next, showing) = (r.undo_at, r.next_at, r.showing);
        if undo.is_none() && next.is_none() { return; }
        self.ctx.request_repaint_after(std::time::Duration::from_millis(60));
        let now = self.time;
        if undo.map_or(false, |t| now >= t) {
            self.run_mut(|r| { r.undo_at = None; if showing { r.next_at = Some(now + 0.4); } });
            let n = self.hist.len();
            if n > 0 { self.truncate_to(n - 1); }
            self.run_mut(|r| r.checked = r.checked.min(n.saturating_sub(1)));
            self.thinking = showing;
        }
        if next.map_or(false, |t| now >= t) {
            let Mode::Puzzle(r) = &mut self.mode else { return };
            r.next_at = None;
            let Some(mv) = self.puzzles[r.idx].moves.get(r.step).and_then(|u| uci_move(u, &self.pos)) else { return };
            if self.pos.turn() != self.me { r.step += 1; } // the player's own moves are counted when checked
            self.view = None;
            self.play(mv);
        }
    }

    /// Dev only: GAMBITO_SOLVE=right|wrong|show plays the puzzle like a user would (wrong: one bad move first).
    fn dev_solve(&mut self, how: &str) {
        let Mode::Puzzle(r) = &self.mode else { return };
        let (idx, step, failed) = (r.idx, r.step, r.failed);
        if !self.can_move(self.view.is_none()) || self.pos.turn() != self.me { return; }
        if how == "show" { return self.puzzle_solution(); }
        let Some(exp) = self.puzzles[idx].moves.get(step).and_then(|u| uci_move(u, &self.pos)) else { return };
        let mv = if how == "wrong" && !failed { self.pos.legal_moves().into_iter().find(|m| *m != exp).unwrap_or(exp) } else { exp };
        if let Some(from) = mv.from() { self.try_move(from, dest(&mv)); }
    }

    /// After every move in a puzzle: was the player's move the solution?
    pub(crate) fn puzzle_after_move(&mut self) {
        let Some(k) = self.mvs.len().checked_sub(1) else { return };
        let (idx, step, showing) = match &mut self.mode {
            Mode::Puzzle(r) if r.checked <= k => { r.checked = k + 1; (r.idx, r.step, r.showing) }
            _ => return, // this move was already judged
        };
        let (now, total, mate) = (self.time, self.puzzles[idx].moves.len(), self.pos.is_checkmate());
        if self.pos_at(k).turn() != self.me {
            // the opponent's move: the player's turn (or the next move of the solution being shown)
            if showing && step < total { self.run_mut(|r| r.next_at = Some(now + 0.9)); self.thinking = true; }
            return;
        }
        let expected = self.puzzles[idx].moves.get(step).and_then(|u| uci_move(u, self.pos_at(k)));
        if expected.as_ref() == Some(&self.mvs[k]) || mate { // any mate is as good as the one in the solution
            self.quality[k] = Some((Class::Best, None));
            let fin = step + 1 >= total || mate;
            self.run_mut(|r| { r.step += 1; if !fin { r.next_at = Some(now + 0.45); } });
            if fin { self.finish_puzzle(); } else { self.thinking = true; }
        } else {
            self.quality[k] = Some((Class::Mistake, None));
            self.fail_puzzle();
            self.run_mut(|r| r.undo_at = Some(now + 0.8));
            self.thinking = true;
        }
    }

    /// First wrong move, hint or look at the solution: the puzzle counts as failed (once).
    fn fail_puzzle(&mut self) {
        let Mode::Puzzle(r) = &mut self.mode else { return };
        if r.failed { return; }
        r.failed = true;
        crate::sound::play(crate::sound::Snd::Wrong);
        if r.rated {
            r.delta = Some(self.progress.record(self.puzzles[r.idx].rating, false));
            if r.daily { self.progress.daily = Some((today(), false)); }
            self.progress.save();
        }
    }

    fn finish_puzzle(&mut self) {
        self.thinking = false;
        let Mode::Puzzle(r) = &mut self.mode else { return };
        r.done = true; // `showing` stays set: it tells the result card the solution was shown
        r.next_at = None;
        let mut news: Option<String> = None;
        if !r.failed {
            crate::sound::play(crate::sound::Snd::Right);
            if r.rated {
                let best = self.progress.best_streak;
                r.delta = Some(self.progress.record(self.puzzles[r.idx].rating, true));
                let s = self.progress.streak;
                if s > best && s >= 3 { news = Some(tr!("Nova melhor sequência: {} puzzles certos seguidos!", s)); }
                else if [5, 10, 20, 50, 100].contains(&s) { news = Some(tr!("{} puzzles certos seguidos!", s)); }
            }
            if r.daily { self.progress.daily = Some((today(), true)); news = Some(tl("Puzzle do dia resolvido. Volta amanhã para o próximo!").into()); }
            if let Some(l) = r.lesson {
                let n = self.progress.lessons.entry(LESSONS[l].id.to_string()).or_insert(0);
                *n += 1;
                if *n == LESSON_PUZZLES { news = Some(tr!("Lição concluída: {}", tl(LESSONS[l].title))); }
            }
        }
        self.progress.save();
        if let Some(n) = news { self.notify(3, n); }
    }

    /// First press: the piece to move. Second: the whole move.
    fn puzzle_hint(&mut self) {
        if !self.can_move(self.view.is_none()) { return; }
        let (idx, step, hinted) = match &self.mode { Mode::Puzzle(r) => (r.idx, r.step, r.hinted), _ => return };
        let Some(mv) = self.puzzles[idx].moves.get(step).and_then(|u| uci_move(u, &self.pos)) else { return };
        let Some(from) = mv.from() else { return };
        self.fail_puzzle();
        self.run_mut(|r| r.hinted += 1);
        self.marks = vec![if hinted == 0 { Mark::Square(from, 1) } else { Mark::Arrow(from, dest(&mv), 1) }];
    }

    fn puzzle_solution(&mut self) {
        if !matches!(&self.mode, Mode::Puzzle(r) if !r.done && !r.showing) { return; }
        self.fail_puzzle();
        self.marks.clear();
        let now = self.time;
        self.run_mut(|r| { r.showing = true; if r.next_at.is_none() && r.undo_at.is_none() { r.next_at = Some(now + 0.3); } });
        self.thinking = true;
    }

    // ---------- lessons ----------

    /// Opens a lesson: its text in the panel and its example on the board.
    pub(crate) fn open_lesson(&mut self, l: usize) {
        let Some(lesson) = LESSONS.get(l) else { return };
        self.lesson = Some(l);
        self.tab = 4;
        let (pos, marks) = match lesson.example {
            // colour 0: not an arrow, the target square in red (what a knight attacks reads better that way)
            Some((fen, arrows)) => (parse_fen(fen), arrows.iter().filter_map(|(a, b, c)| {
                let (a, b) = (a.parse().ok()?, b.parse().ok()?);
                Some(if *c == 0 { Mark::Square(b, 0) } else { Mark::Arrow(a, b, *c) })
            }).collect()),
            None => {
                // the easiest puzzle of the theme, after the opponent's move, with the answer drawn
                let t = match lesson.practice { Practice::Puzzles(t, _) => t, _ => "" };
                let found = self.puzzles.iter().filter(|p| p.themes.iter().any(|x| x == t)).min_by_key(|p| p.rating).and_then(|p| {
                    let start = parse_fen(&p.fen)?;
                    let pos = start.clone().play(uci_move(&p.moves[0], &start)?).ok()?;
                    let key = uci_move(p.moves.get(1)?, &pos)?;
                    Some((pos, key))
                });
                match found {
                    Some((pos, key)) => (Some(pos), key.from().map(|f| Mark::Arrow(f, dest(&key), 1)).into_iter().collect()),
                    None => (None, vec![]),
                }
            }
        };
        let Some(pos) = pos else { return };
        let me = pos.turn();
        self.set_board(pos, me, Mode::Example);
        self.marks = marks;
    }

    pub(crate) fn start_practice(&mut self, l: usize) {
        match LESSONS[l].practice {
            Practice::Puzzles(..) => self.next_puzzle(Some(l)),
            Practice::Endgame(fen, goal) => {
                let Some(pos) = parse_fen(fen) else { return };
                let me = pos.turn();
                self.set_board(pos, me, Mode::Drill(Drill { lesson: l, goal, done: None, hints: false }));
                self.after_move();
            }
        }
    }

    /// Endgame lessons: reached the goal (or a draw)? True stops the engine from replying.
    pub(crate) fn drill_after_move(&mut self) -> bool {
        let me = self.me;
        let Mode::Drill(d) = &mut self.mode else { return false };
        if d.done.is_some() { return true; }
        let board = self.pos.board();
        let won = match d.goal {
            Goal::Mate => self.pos.is_checkmate() && self.pos.turn() != me,
            Goal::Promote => !(board.by_color(me) & board.queens()).is_empty(),
        };
        if won {
            d.done = Some(true);
            let l = d.lesson;
            let first = !self.progress.lessons.contains_key(LESSONS[l].id);
            self.progress.lessons.insert(LESSONS[l].id.to_string(), 1);
            self.progress.save();
            if first { self.notify(3, tr!("Lição concluída: {}", tl(LESSONS[l].title))); }
            return true;
        }
        if self.pos.is_game_over() { d.done = Some(false); return true; }
        false
    }

    /// "3/5", "Concluída" or "Por fazer"; and whether it is done.
    pub(crate) fn lesson_status(&self, l: usize) -> (String, bool) {
        let n = self.progress.lessons.get(LESSONS[l].id).copied().unwrap_or(0);
        let goal = match LESSONS[l].practice { Practice::Puzzles(..) => LESSON_PUZZLES, Practice::Endgame(..) => 1 };
        if n >= goal { (tl("Concluída").into(), true) } else if n > 0 { (format!("{n}/{goal}"), false) } else { (tl("Por fazer").into(), false) }
    }

    pub(crate) fn lessons_done(&self) -> usize {
        (0..LESSONS.len()).filter(|&l| self.lesson_status(l).1).count()
    }

    // ---------- panels ----------

    pub(crate) fn view_puzzle(&mut self, ui: &mut egui::Ui) {
        let Mode::Puzzle(r) = &self.mode else {
            ui.label(semi(tl("Exercícios"), 28.0));
            ui.add_space(6.0);
            card(ui, |ui| { ui.add(egui::Label::new(egui::RichText::new(tl("Puzzles do Lichess ao teu nível. Cada um tem um lance certo: encontra-o.")).size(15.0)).wrap()); });
            if gold_btn(ui, tl("Começar"), Some(Ico::Puzzle), Vec2::new(ui.available_width(), 52.0)) { self.next_puzzle(None); }
            return;
        };
        let (idx, step, failed, done, delta, lesson, daily, rated, hinted) = (r.idx, r.step, r.failed, r.done, r.delta, r.lesson, r.daily, r.rated, r.hinted);
        let (busy, wrong, r_showing) = (r.next_at.is_some() || r.showing, r.undo_at.is_some(), r.showing);
        let (prating, pid, themes, moves, fen) = {
            let p = &self.puzzles[idx];
            (p.rating, p.id.clone(), p.themes.clone(), p.moves.clone(), p.fen.clone())
        };
        let white = self.me == Color::White;

        ui.horizontal(|ui| {
            let title = match lesson { Some(l) => tl(LESSONS[l].title), None if daily => tl("Puzzle do dia"), None => tl("Exercícios") };
            ui.label(semi(title, 28.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let shown = if crate::motion::on() { ui.ctx().animate_value_with_time(egui::Id::new("puzzle_rating"), self.progress.rating as f32, 0.6) } else { self.progress.rating as f32 };
                eval_pill(ui, &format!("{shown:.0}"), true, 18.0);
                ui.label(egui::RichText::new(tl("o teu rating")).size(14.0).color(MUTED));
            });
        });
        if let Some(l) = lesson { ui.label(egui::RichText::new(tr!("Prática da lição · {}", self.lesson_status(l).0)).size(14.0).color(MUTED)); }
        ui.add_space(8.0);

        // what is going on
        let delta_txt = delta.map(|d| format!("  {:+.0}", d)).unwrap_or_default();
        let (title, text, col, icon) = if done && !failed {
            (format!("{}{delta_txt}", tl("Resolvido!")), if rated { tl("Boa. O rating sobe.").to_string() } else { tl("Já tinhas feito este puzzle: não conta para o rating.").into() }, POS, Some(Class::Best))
        } else if done && r_showing {
            (format!("{}{delta_txt}", tl("Fica para a próxima")), tl("Revê a solução em baixo e tenta outro.").into(), AMBER, Some(Class::Mistake))
        } else if done {
            (format!("{}{delta_txt}", if hinted > 0 { tl("Resolvido, com ajuda") } else { tl("Resolvido, com erros") }), tl("Chegaste lá, mas não conta como resolvido à primeira.").into(), AMBER, Some(Class::Good))
        } else if wrong {
            (tl("Não é isso.").into(), tl("Tenta outra vez.").into(), RED, Some(Class::Mistake))
        } else if busy && step == 0 {
            (tl("O adversário joga…").into(), String::new(), MUTED, None)
        } else if step > 1 {
            (tl("Certo! Continua.").into(), if white { tl("Encontra o próximo lance das brancas.") } else { tl("Encontra o próximo lance das pretas.") }.into(), POS, Some(Class::Best))
        } else {
            (if white { tl("Jogas com as brancas") } else { tl("Jogas com as pretas") }.into(), tl("Encontra o melhor lance.").into(), CREAM, None)
        };
        egui::Frame::new().fill(Color32::from_rgb(24, 24, 27)).stroke(Stroke::new(1.5, if done && !failed { POS } else { Color32::from_rgb(46, 46, 52) })).corner_radius(16.0)
            .inner_margin(egui::Margin::symmetric(14, 12)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (rc, _) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::hover());
                    match icon {
                        Some(c) => draw_icon(ui.painter(), rc.center(), 19.0, c),
                        None => { ui.painter().image(self.tex[&(self.me, shakmaty::Role::King)].id(), rc, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE); }
                    }
                    ui.vertical(|ui| {
                        ui.label(semi(&title, 22.0).color(col));
                        if !text.is_empty() { ui.add(egui::Label::new(egui::RichText::new(&text).size(14.5).color(Color32::from_rgb(216, 209, 191))).wrap()); }
                    });
                });
            });
        ui.add_space(10.0);

        let w2 = (ui.available_width() - 12.0) / 2.0;
        if !done {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                if dark_btn(ui, if hinted == 0 { tl("Dica") } else { tl("Mais uma dica") }, Some(Ico::Target), Vec2::new(w2, 46.0), false) { self.puzzle_hint(); }
                if dark_btn(ui, tl("Ver solução"), Some(Ico::Play), Vec2::new(w2, 46.0), true) { self.puzzle_solution(); }
            });
            ui.label(egui::RichText::new(tl("Com dica ou solução, o puzzle não conta como resolvido.")).small().color(MUTED));
        } else {
            if gold_btn(ui, tl("Próximo puzzle"), Some(Ico::ChevR), Vec2::new(ui.available_width(), 52.0)) { self.next_puzzle(lesson); }
            if let Some(l) = lesson {
                ui.add_space(8.0);
                if dark_btn(ui, tl("Voltar à lição"), Some(Ico::ChevL), Vec2::new(ui.available_width(), 44.0), false) { self.open_lesson(l); }
            }
            // the solution and why the key move works
            ui.add_space(10.0);
            if let Some(start) = parse_fen(&fen) {
                let after_first = moves.first().and_then(|u| uci_move(u, &start)).and_then(|m| start.clone().play(m).ok());
                let mut sans = vec![];
                if let Some(a) = &after_first {
                    let mut p = a.clone();
                    for u in moves.iter().skip(1) {
                        let Some(m) = uci_move(u, &p) else { break };
                        sans.push(SanPlus::from_move(p.clone(), m.clone()).to_string());
                        let Ok(next) = p.play(m) else { break };
                        p = next;
                    }
                }
                card(ui, |ui| {
                    ui.label(med(tl("Solução"), 17.0));
                    if let Some(a) = &after_first { ui.label(egui::RichText::new(numbered(a, &sans)).size(16.0).color(GOLD)); }
                    if self.mvs.len() > 1 {
                        let (san, why, _) = explain(self.pos_at(1), &self.mvs[1]);
                        ui.add_space(4.0);
                        ui.add(egui::Label::new(egui::RichText::new(format!("{san}: {why}")).size(14.5).color(Color32::from_rgb(222, 232, 220))).wrap());
                    }
                    let tags = shown_themes(&themes);
                    if !tags.is_empty() {
                        ui.add_space(6.0);
                        ui.horizontal_wrapped(|ui| { for t in tags { ui.label(egui::RichText::new(format!(" {t} ")).size(13.5).color(Color32::from_rgb(220, 200, 140)).background_color(Color32::from_rgb(36, 31, 18))); } });
                    }
                    ui.label(egui::RichText::new(tr!("Puzzle {} do Lichess · rating {}", pid, prating)).small().color(MUTED));
                });
            }
        }
        if !self.msg.is_empty() { ui.label(egui::RichText::new(&self.msg).color(AMBER)); }
        ui.add_space(10.0);

        // theme filter (not inside a lesson)
        if lesson.is_none() {
            // the themes as chips: one click picks one (the gold one is the current filter)
            let mut pick_theme = self.pz_theme;
            ui.label(med(tl("Tema"), 16.0));
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
                for t in std::iter::once(None).chain(FILTERS.iter().map(|t| Some(*t))) {
                    let on = pick_theme == t;
                    let label = t.and_then(theme_name).unwrap_or(tl("Todos"));
                    let chip = egui::Button::new(egui::RichText::new(label).size(13.5).color(if on { GOLD } else { CREAM }))
                        .fill(if on { Color32::from_rgb(40, 31, 9) } else { Color32::from_rgb(24, 24, 27) })
                        .stroke(Stroke::new(1.0, if on { Color32::from_rgb(107, 83, 23) } else { Color32::from_rgb(52, 52, 58) }))
                        .corner_radius(999.0).min_size(Vec2::new(0.0, 32.0));
                    if ui.add(chip).clicked() { pick_theme = t; }
                }
            });
            if pick_theme != self.pz_theme { self.pz_theme = pick_theme; self.next_puzzle(None); }
            ui.add_space(10.0);
        }
        self.stats_card(ui);
    }

    fn stats_card(&self, ui: &mut egui::Ui) {
        let pr = &self.progress;
        card(ui, |ui| {
            ui.label(med(tl("O teu progresso"), 17.0));
            ui.add_space(4.0);
            let cols = [(format!("{:.0}", pr.rating), tl("rating")), (pr.solved.to_string(), tl("resolvidos")), (pr.streak.to_string(), tl("seguidos")), (pr.best_streak.to_string(), tl("melhor série"))];
            ui.columns(4, |c| {
                for (i, (v, l)) in cols.iter().enumerate() {
                    c[i].label(semi(v, 24.0).color(if i == 0 { GOLD } else { CREAM }));
                    c[i].label(egui::RichText::new(*l).size(13.0).color(MUTED));
                }
            });
            if pr.history.len() >= 2 {
                ui.add_space(6.0);
                win_chart(ui, &rating_series(&pr.history), Vec2::new(ui.available_width(), 56.0), None, true, &[]);
            }
        });
    }

    pub(crate) fn view_learn(&mut self, ui: &mut egui::Ui) {
        if matches!(&self.mode, Mode::Puzzle(r) if r.lesson.is_some()) { return self.view_puzzle(ui); }
        if matches!(self.mode, Mode::Drill(_)) { return self.view_drill(ui); }
        match self.lesson {
            Some(l) => self.view_lesson(ui, l),
            None => {} // the list of lessons is a page of its own (screens.rs)
        }
    }

    fn view_lesson(&mut self, ui: &mut egui::Ui, l: usize) {
        let lesson = &LESSONS[l];
        if dark_btn(ui, tl("Lições"), Some(Ico::ChevL), Vec2::new(110.0, 40.0), false) { self.lesson = None; return; }
        ui.add_space(6.0);
        ui.label(semi(tl(lesson.title), 30.0));
        let (status, _) = self.lesson_status(l);
        ui.label(egui::RichText::new(format!("{} · {status}", tl(lesson.group))).size(14.0).color(MUTED));
        ui.add_space(8.0);
        card(ui, |ui| {
            ui.add(egui::Label::new(egui::RichText::new(tl(lesson.text)).size(15.5).color(Color32::from_rgb(232, 226, 210))).wrap());
            if lesson.example.is_none() {
                ui.add_space(6.0);
                ui.label(egui::RichText::new(tl("No tabuleiro: um puzzle deste tema, com o lance certo a verde.")).small().color(MUTED));
            }
        });
        if !matches!(self.mode, Mode::Example) {
            if dark_btn(ui, tl("Ver o exemplo"), Some(Ico::Image), Vec2::new(ui.available_width(), 42.0), false) { self.open_lesson(l); }
            ui.add_space(8.0);
        }
        let what = match lesson.practice {
            Practice::Puzzles(..) => tr!("Resolve {} puzzles deste tema, sem erros nem dicas.", LESSON_PUZZLES),
            Practice::Endgame(_, Goal::Mate) => tl("Joga contra a engine, que defende o melhor que sabe, até dares mate.").into(),
            Practice::Endgame(_, Goal::Promote) => tl("Joga contra a engine até promoveres o peão.").into(),
        };
        ui.label(egui::RichText::new(what).size(14.5).color(MUTED));
        ui.add_space(6.0);
        if gold_btn(ui, tl("Praticar"), Some(Ico::Play), Vec2::new(ui.available_width(), 52.0)) { self.start_practice(l); }
    }

    fn view_drill(&mut self, ui: &mut egui::Ui) {
        let Mode::Drill(d) = &self.mode else { return };
        let (l, goal, done, hints) = (d.lesson, d.goal, d.done, d.hints);
        ui.label(semi(tl(LESSONS[l].title), 28.0));
        ui.label(egui::RichText::new(if goal == Goal::Mate { tl("Objetivo: dar xeque-mate.") } else { tl("Objetivo: levar o peão até à promoção.") }).size(15.0).color(GOLD));
        ui.add_space(8.0);
        let (title, text, col) = match done {
            Some(true) => (tl("Conseguiste!").to_string(), tl("Lição concluída.").to_string(), POS),
            Some(false) => (tl("Empate.").to_string(), if self.pos.is_stalemate() { tl("Afogamento: o rei não tinha lances e não estava em xeque. Deixa-lhe sempre uma casa até ao mate.").into() } else { tl("Já não há material para ganhar.").into() }, AMBER),
            None => (self.status(), tl("A engine defende o melhor que sabe.").into(), CREAM),
        };
        card(ui, |ui| {
            ui.label(semi(title, 21.0).color(col));
            ui.add(egui::Label::new(egui::RichText::new(text).size(14.5).color(Color32::from_rgb(216, 209, 191))).wrap());
            if let (true, Some(h)) = (hints, self.hints.first()) { ui.label(egui::RichText::new(tr!("Sugestão: {}  ({})", h.san, h.ev)).size(15.0).color(GOLD)); }
        });
        let w3 = (ui.available_width() - 20.0) / 3.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            if dark_btn(ui, tl("Desfazer"), None, Vec2::new(w3, 42.0), false) {
                if let Mode::Drill(d) = &mut self.mode { d.done = None; }
                self.undo();
            }
            if dark_btn(ui, tl("Recomeçar"), Some(Ico::Restart), Vec2::new(w3, 42.0), false) { self.start_practice(l); }
            if dark_btn(ui, if hints { tl("Sem ajuda") } else { tl("Ajuda") }, Some(Ico::Target), Vec2::new(w3, 42.0), !hints) {
                if let Mode::Drill(d) = &mut self.mode { d.hints = !d.hints; }
                self.request_hints();
            }
        });
        ui.add_space(8.0);
        if dark_btn(ui, tl("Voltar à lição"), Some(Ico::ChevL), Vec2::new(ui.available_width(), 42.0), false) { self.open_lesson(l); }
    }

    // ---------- Início ----------

    /// The game kept on the board (live, or put aside while training): its position, move count and a title.
    pub(crate) fn kept_game(&self) -> Option<(Chess, usize, bool, String, String)> {
        let opp = || tr!("Partida contra {}", self.engines[self.opp].name);
        let describe = |loaded: &Option<crate::GameInfo>, both: bool, n: usize| -> (String, String) {
            match loaded {
                Some(i) if !i.result.is_empty() => (format!("{} vs {}", i.white, i.black), tr!("{} lances · {}", n, i.result)),
                Some(i) => (format!("{} vs {}", i.white, i.black), tr!("{} lances", n)),
                None if both => (tl("Análise livre").into(), tr!("{} lances", n)),
                None => (opp(), tr!("Lance {} · Elo {}", n / 2 + 1, self.elo)),
            }
        };
        match (&self.mode, &self.snapshot) {
            (Mode::Free, _) if !self.hist.is_empty() => {
                let (t, s) = describe(&self.loaded, self.both, self.hist.len());
                Some((self.pos.clone(), self.hist.len(), self.flipped, t, s))
            }
            (Mode::Free, _) => None,
            (_, Some(s)) if !s.hist.is_empty() => {
                let (t, sub) = describe(&s.loaded, s.both, s.hist.len());
                Some((s.pos.clone(), s.hist.len(), s.flipped, t, sub))
            }
            _ => None,
        }
    }

}

fn theme_icon(t: &str) -> Ico {
    match t {
        "mateIn1" | "mateIn2" => Ico::Crown, "fork" => Ico::Fork, "pin" => Ico::Link, "skewer" => Ico::Skewer,
        "discoveredAttack" => Ico::Bolt, "doubleCheck" => Ico::Plus2, "hangingPiece" => Ico::Shield, "deflection" => Ico::Swap,
        "attraction" => Ico::Magnet, "sacrifice" => Ico::Cross, "backRankMate" => Ico::Layers, "promotion" => Ico::Flag,
        "endgame" => Ico::King, _ => Ico::Book,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shakmaty::{fen::Fen, CastlingMode, Chess, Position, uci::UciMove};

    #[test]
    fn lesson_examples_are_legal() {
        for l in LESSONS {
            for fen in l.example.iter().map(|e| e.0).chain(match l.practice { Practice::Endgame(f, _) => Some(f), _ => None }) {
                let pos: Chess = fen.parse::<Fen>().unwrap().into_position(CastlingMode::Standard).unwrap_or_else(|e| panic!("{}: {e}", l.id));
                assert!(!pos.is_game_over(), "{}", l.id);
            }
        }
    }

    #[test]
    fn puzzles_load_and_play() {
        let ps = load_puzzles();
        assert!(ps.len() > 5000, "{}", ps.len());
        for p in ps.iter().step_by(97) {
            let mut pos: Chess = p.fen.parse::<Fen>().unwrap().into_position(CastlingMode::Standard).unwrap();
            for m in &p.moves {
                let mv = UciMove::from_ascii(m.as_bytes()).unwrap().to_move(&pos).unwrap();
                pos = pos.play(mv).unwrap();
            }
        }
        for l in LESSONS {
            if let Practice::Puzzles(t, max) = l.practice {
                let n = ps.iter().filter(|p| p.rating <= max && p.themes.iter().any(|x| x == t)).count();
                assert!(n >= 30, "{t}: {n}");
            }
        }
    }

    #[test]
    fn rating_moves_both_ways() {
        let mut p = Progress { rating: 1200.0, solved: 0, failed: 0, streak: 0, best_streak: 0, seen: vec![], lessons: HashMap::new(), history: vec![], daily: None, games: HashMap::new() };
        assert!(p.record(1200, true) > 0.0);
        assert!(p.record(1200, false) < 0.0);
        assert_eq!((p.solved, p.failed, p.streak, p.best_streak), (1, 1, 0, 1));
    }

    /// Builds assets/puzzles.tsv.gz from the Lichess puzzle database (lichess_db_puzzle.csv.zst):
    ///   PUZZLE_DB=<path to the .zst> cargo test --release build_puzzle_set -- --ignored --nocapture
    /// Keeps well-played puzzles only, a few hundred per 100 rating points, plus enough of every lesson theme.
    #[test]
    #[ignore]
    fn build_puzzle_set() {
        let path = std::env::var("PUZZLE_DB").expect("PUZZLE_DB");
        let mut src = std::io::BufReader::new(std::fs::File::open(path).unwrap());
        // the file is many zstd frames, each after a skippable frame (pzstd): decode them one by one
        let mut text: Vec<u8> = vec![];
        loop {
            let mut hdr = [0u8; 4];
            if src.read_exact(&mut hdr).is_err() { break; }
            if u32::from_le_bytes(hdr) & 0xFFFF_FFF0 == 0x184D_2A50 {
                let mut len = [0u8; 4];
                src.read_exact(&mut len).unwrap();
                std::io::copy(&mut (&mut src).take(u32::from_le_bytes(len) as u64), &mut std::io::sink()).unwrap();
                continue;
            }
            let mut dec = ruzstd::decoding::StreamingDecoder::new((&hdr[..]).chain(&mut src)).unwrap();
            dec.read_to_end(&mut text).unwrap();
        }
        let dec = &text[..];
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let lesson_themes: Vec<&str> = LESSONS.iter().filter_map(|l| match l.practice { Practice::Puzzles(t, _) => Some(t), _ => None }).collect();
        // reservoir per (theme or "", rating band)
        let mut res: HashMap<(String, u32), (u64, Vec<String>)> = HashMap::new();
        let mut take = |key: (String, u32), cap: usize, line: &String, rng: &mut Rng| {
            let e = res.entry(key).or_default();
            e.0 += 1;
            if e.1.len() < cap { e.1.push(line.clone()); } else {
                let j = (rng.next() % e.0) as usize;
                if j < cap { e.1[j] = line.clone(); }
            }
        };
        let (mut total, mut kept) = (0u64, 0u64);
        for line in std::io::BufRead::lines(std::io::BufReader::new(dec)).skip(1) {
            let line = line.unwrap();
            total += 1;
            // PuzzleId,FEN,Moves,Rating,RatingDeviation,Popularity,NbPlays,Themes,GameUrl,OpeningTags
            let f: Vec<&str> = line.split(',').collect();
            if f.len() < 8 { continue; }
            let (rating, dev, pop, plays): (u32, u32, i32, u32) = (f[3].parse().unwrap_or(0), f[4].parse().unwrap_or(999), f[5].parse().unwrap_or(0), f[6].parse().unwrap_or(0));
            if !(500..=2600).contains(&rating) || dev > 100 || pop < 80 || plays < 300 { continue; }
            kept += 1;
            let out = format!("{}\t{}\t{}\t{}\t{}", f[0], f[1], f[2], rating, f[7]);
            take((String::new(), rating / 100), 260, &out, &mut rng);
            let themes: Vec<&str> = f[7].split(' ').collect();
            for t in &lesson_themes {
                if themes.contains(t) { take((t.to_string(), rating / 200), 30, &out, &mut rng); }
            }
        }
        let mut all: Vec<String> = res.into_values().flat_map(|v| v.1).collect();
        all.sort_by_key(|l| (l.split('\t').nth(3).unwrap().parse::<u32>().unwrap(), l.clone()));
        all.dedup();
        let mut gz = flate2::write::GzEncoder::new(std::fs::File::create("assets/puzzles.tsv.gz").unwrap(), flate2::Compression::best());
        std::io::Write::write_all(&mut gz, (all.join("\n") + "\n").as_bytes()).unwrap();
        gz.finish().unwrap();
        println!("read {total} puzzles, {kept} passed the filters, wrote {}", all.len());
    }
}
