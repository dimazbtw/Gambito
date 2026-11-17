//! Reads a chess position from a screenshot of a board (chess.com green / lichess brown).
//! Method: find the board by its dark-square colour, cut 64 cells, take each cell's piece
//! silhouette (pixels that differ from the cell's dominant background colour) and match it
//! against known piece shapes: the bundled ones plus shapes the app learned from screenshots the
//! user corrected. White/black comes from how bright the piece pixels are. Finally the rules of
//! chess fix obvious mistakes (exactly one king per side, no pawns on the first/last rank).
use image::RgbImage;
use shakmaty::{Color, Role};
use std::collections::HashMap;

pub type Grid = [[Option<(Color, Role)>; 8]; 8]; // [row from top][col from left], as seen in the image
pub type Sils = [[Option<Sil>; 8]; 8];
const N: usize = 24;
const ROLES: [Role; 6] = [Role::Pawn, Role::Knight, Role::Bishop, Role::Rook, Role::Queen, Role::King];

pub fn raster(svg: &[u8], size: u32) -> Vec<u8> {
    let tree = resvg::usvg::Tree::from_data(svg, &resvg::usvg::Options::default()).expect("svg");
    let mut pm = resvg::tiny_skia::Pixmap::new(size, size).unwrap();
    let s = size as f32 / tree.size().width();
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(s, s), &mut pm.as_mut());
    pm.take() // RGBA, premultiplied
}

#[derive(Clone)]
pub struct Sil {
    bits: [bool; N * N],
    aspect: f32,
    pub hf: f32, // bbox height / cell height
    pub wf: f32, // bbox width / cell width
}

fn role_char(r: Role) -> char {
    match r { Role::Pawn => 'P', Role::Knight => 'N', Role::Bishop => 'B', Role::Rook => 'R', Role::Queen => 'Q', Role::King => 'K' }
}

impl Sil {
    /// One text line: `<role> <576 bits> <hf> <wf> <aspect>`
    pub fn to_line(&self, r: Role) -> String {
        let bits: String = self.bits.iter().map(|b| if *b { '1' } else { '0' }).collect();
        format!("{} {} {:.3} {:.3} {:.3}", role_char(r), bits, self.hf, self.wf, self.aspect)
    }

    pub fn from_line(l: &str) -> Option<(Role, Sil)> {
        let mut it = l.split_whitespace();
        let tag = it.next()?.chars().next()?;
        let role = ROLES.into_iter().find(|r| role_char(*r) == tag)?;
        let bits_s = it.next()?;
        if bits_s.len() != N * N { return None; }
        let mut bits = [false; N * N];
        for (i, c) in bits_s.chars().enumerate() { bits[i] = c == '1'; }
        let mut f = || it.next().and_then(|s| s.parse::<f32>().ok());
        Some((role, Sil { bits, hf: f()?, wf: f()?, aspect: f()? }))
    }
}

fn silhouette(mask: &[bool], w: usize, h: usize) -> Option<Sil> {
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if mask[y * w + x] {
                x0 = x0.min(x);
                x1 = x1.max(x);
                y0 = y0.min(y);
                y1 = y1.max(y);
            }
        }
    }
    if x1 < x0 {
        return None;
    }
    let (bw, bh) = (x1 - x0 + 1, y1 - y0 + 1);
    let mut bits = [false; N * N];
    for ty in 0..N {
        for tx in 0..N {
            let (sx0, sx1) = (x0 + tx * bw / N, (x0 + (tx + 1) * bw / N).max(x0 + tx * bw / N + 1));
            let (sy0, sy1) = (y0 + ty * bh / N, (y0 + (ty + 1) * bh / N).max(y0 + ty * bh / N + 1));
            let (mut on, mut tot) = (0, 0);
            for y in sy0..sy1.min(y1 + 1) {
                for x in sx0..sx1.min(x1 + 1) {
                    tot += 1;
                    on += mask[y * w + x] as usize;
                }
            }
            bits[ty * N + tx] = tot > 0 && on * 2 >= tot;
        }
    }
    Some(Sil { bits, aspect: bw as f32 / bh as f32, hf: bh as f32 / h as f32, wf: bw as f32 / w as f32 })
}

fn score(a: &Sil, b: &Sil) -> f32 {
    let (mut i, mut u) = (0, 0);
    for k in 0..N * N {
        i += (a.bits[k] && b.bits[k]) as u32;
        u += (a.bits[k] || b.bits[k]) as u32;
    }
    i as f32 / u.max(1) as f32 - 0.6 * (a.aspect - b.aspect).abs()
}

pub struct Templates {
    base: Vec<(Role, Sil)>,    // bundled: the SVG set + a few common chess.com shapes
    learned: Vec<(Role, Sil)>, // from screenshots the user corrected (kept in %APPDATA%)
}

fn learned_path() -> Option<std::path::PathBuf> {
    std::env::var_os("APPDATA").map(|a| std::path::PathBuf::from(a).join("Gambito").join("learned.txt"))
}

impl Templates {
    pub fn new(svgs: &[(Color, Role, &[u8])]) -> Self {
        let mut base = vec![];
        for (_, role, svg) in svgs {
            let px = raster(svg, 96);
            let mask: Vec<bool> = px.chunks(4).map(|p| p[3] > 100).collect();
            base.push((*role, silhouette(&mask, 96, 96).unwrap()));
        }
        base.extend(include_str!("../assets/neo_silhouettes.txt").lines().filter_map(Sil::from_line));
        let learned = learned_path().and_then(|p| std::fs::read_to_string(p).ok())
            .map(|t| t.lines().filter_map(Sil::from_line).collect()).unwrap_or_default();
        Templates { base, learned }
    }

    /// Best match score per role (max over every known shape of that role).
    fn scores(&self, s: &Sil) -> [f32; 6] {
        let mut out = [f32::MIN; 6];
        for (r, t) in self.base.iter().chain(self.learned.iter()) {
            let i = ROLES.iter().position(|x| x == r).unwrap();
            out[i] = out[i].max(score(s, t));
        }
        out
    }

    /// Remember these shapes (used after the user corrected a screenshot). Returns how many were stored.
    pub fn learn(&mut self, grid: &Grid, sils: &Sils) -> usize {
        let mut lines = String::new();
        let mut n = 0;
        for r in 0..8 {
            for c in 0..8 {
                if let (Some((_, role)), Some(s)) = (grid[r][c], &sils[r][c]) {
                    lines += &(s.to_line(role) + "\n");
                    self.learned.push((role, s.clone()));
                    n += 1;
                }
            }
        }
        if let Some(p) = learned_path() {
            if let Some(d) = p.parent() { let _ = std::fs::create_dir_all(d); }
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) { let _ = f.write_all(lines.as_bytes()); }
        }
        n
    }
}

/// Locate the board: bbox of dark-square-coloured pixels. Returns (x, y, w, h).
fn find_board(img: &RgbImage) -> Result<(u32, u32, u32, u32), String> {
    const THEMES: [[i32; 3]; 2] = [[118, 150, 86], [181, 136, 99]]; // chess.com green, lichess brown
    let mut best: Option<(u32, u32, u32, u32)> = None;
    for t in THEMES {
        let (mut x0, mut y0, mut x1, mut y1, mut n) = (u32::MAX, u32::MAX, 0u32, 0u32, 0u64);
        for (x, y, p) in img.enumerate_pixels() {
            if (0..3).all(|c| (p[c] as i32 - t[c]).abs() <= 12) {
                x0 = x0.min(x);
                x1 = x1.max(x);
                y0 = y0.min(y);
                y1 = y1.max(y);
                n += 1;
            }
        }
        if n == 0 {
            continue;
        }
        let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
        let sq = (w as f32 - h as f32).abs() / w.max(h) as f32;
        if w >= 160 && sq < 0.12 && n as f64 > 0.15 * (w as f64 * h as f64) && best.map_or(true, |b| w * h > b.2 * b.3) {
            best = Some((x0, y0, w, h));
        }
    }
    if let Some(b) = best {
        return Ok(b);
    }
    let (w, h) = img.dimensions();
    if (w as f32 - h as f32).abs() / w.max(h) as f32 > 0.08 {
        return Err("Não encontrei o tabuleiro. Usa um print do tabuleiro verde (chess.com) ou castanho (lichess).".into());
    }
    Ok((0, 0, w, h)) // already cropped to the board
}

struct Cell {
    mask: Vec<bool>,
    w: usize,
    h: usize,
    white: bool,
}

fn read_cell(img: &RgbImage, cx: u32, cy: u32, cw: u32, ch: u32) -> Option<Cell> {
    let (ix, iy) = (cw * 4 / 100, ch * 4 / 100);
    let (x0, y0, w, h) = (cx + ix, cy + iy, (cw - 2 * ix) as usize, (ch - 2 * iy) as usize);
    let px = |x: usize, y: usize| img.get_pixel(x0 + x as u32, y0 + y as u32).0;
    // background = most common colour in the cell
    let mut hist: HashMap<[u8; 3], u32> = HashMap::new();
    for y in 0..h {
        for x in 0..w {
            let p = px(x, y);
            *hist.entry([p[0] >> 3, p[1] >> 3, p[2] >> 3]).or_default() += 1;
        }
    }
    let bq = hist.into_iter().max_by_key(|(_, n)| *n)?.0;
    let bg = [(bq[0] as i32) * 8 + 4, (bq[1] as i32) * 8 + 4, (bq[2] as i32) * 8 + 4];
    let mut mask = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let p = px(x, y);
            let d: i32 = (0..3).map(|c| (p[c] as i32 - bg[c]).abs()).sum();
            mask[y * w + x] = d > 60;
        }
    }
    // keep the biggest blob (+ comparable ones): drops the coordinate letters in the corners
    let mut label = vec![0u32; w * h];
    let mut sizes = vec![0usize];
    for s in 0..w * h {
        if !mask[s] || label[s] != 0 {
            continue;
        }
        let id = sizes.len() as u32;
        let (mut stack, mut cnt) = (vec![s], 0);
        label[s] = id;
        while let Some(i) = stack.pop() {
            cnt += 1;
            let (x, y) = (i % w, i / w);
            let mut nb = vec![];
            if x > 0 { nb.push(i - 1) }
            if x + 1 < w { nb.push(i + 1) }
            if y > 0 { nb.push(i - w) }
            if y + 1 < h { nb.push(i + w) }
            for j in nb {
                if mask[j] && label[j] == 0 {
                    label[j] = id;
                    stack.push(j);
                }
            }
        }
        sizes.push(cnt);
    }
    let biggest = *sizes.iter().max().unwrap();
    if (biggest as f32) < 0.05 * (w * h) as f32 {
        return None; // empty square
    }
    let keep: Vec<bool> = label.iter().map(|&l| l != 0 && sizes[l as usize] * 100 >= biggest * 15).collect();
    let (mut bright, mut tot) = (0, 0);
    for i in 0..w * h {
        if keep[i] {
            let p = px(i % w, i / w);
            let lum = (p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114) / 1000;
            tot += 1;
            bright += (lum > 175) as u32;
        }
    }
    Some(Cell { mask: keep, w, h, white: bright as f32 / tot as f32 > 0.22 })
}

pub struct Scan {
    pub grid: Grid,
    pub sils: Sils,
}

pub fn read_board(img: &RgbImage, tpl: &Templates) -> Result<Scan, String> {
    let (bx, by, bw, bh) = find_board(img)?;
    let (cw, ch) = (bw / 8, bh / 8);
    let mut grid: Grid = Default::default();
    let mut sils: Sils = Default::default();
    let mut sc = [[[0f32; 6]; 8]; 8];
    for r in 0..8u32 {
        for c in 0..8u32 {
            if let Some(cell) = read_cell(img, bx + c * cw, by + r * ch, cw, ch) {
                if let Some(s) = silhouette(&cell.mask, cell.w, cell.h) {
                    let (ru, cu) = (r as usize, c as usize);
                    let color = if cell.white { Color::White } else { Color::Black };
                    sc[ru][cu] = tpl.scores(&s);
                    let best = (0..6).max_by(|a, b| sc[ru][cu][*a].total_cmp(&sc[ru][cu][*b])).unwrap();
                    grid[ru][cu] = Some((color, ROLES[best]));
                    sils[ru][cu] = Some(s);
                }
            }
        }
    }
    enforce_rules(&mut grid, &sc);
    Ok(Scan { grid, sils })
}

/// Chess rules as a sanity net over the raw shape matches: exactly one king per side,
/// and no pawns on the first or last rank.
fn enforce_rules(grid: &mut Grid, sc: &[[[f32; 6]; 8]; 8]) {
    let king = 5;
    // pawns can't stand on the first/last rank: rows 0 and 7 in the image, whichever side is on top
    for r in [0usize, 7] {
        for c in 0..8 {
            if let Some((col, Role::Pawn)) = grid[r][c] {
                let best = (0..6).filter(|i| ROLES[*i] != Role::Pawn).max_by(|a, b| sc[r][c][*a].total_cmp(&sc[r][c][*b])).unwrap();
                grid[r][c] = Some((col, ROLES[best]));
            }
        }
    }
    for color in [Color::White, Color::Black] {
        let cells: Vec<(usize, usize)> = (0..64).map(|i| (i / 8, i % 8)).filter(|&(r, c)| grid[r][c].map_or(false, |p| p.0 == color)).collect();
        if cells.is_empty() { continue; }
        // the cell that looks most like a king keeps (or gets) the king
        let top = *cells.iter().max_by(|a, b| sc[a.0][a.1][king].total_cmp(&sc[b.0][b.1][king])).unwrap();
        for &(r, c) in &cells {
            if (r, c) == top {
                grid[r][c] = Some((color, Role::King));
            } else if grid[r][c] == Some((color, Role::King)) {
                let best = (0..6).filter(|i| *i != king).max_by(|a, b| sc[r][c][*a].total_cmp(&sc[r][c][*b])).unwrap();
                grid[r][c] = Some((color, ROLES[best]));
            }
        }
    }
}

pub fn white_on_top(g: &Grid) -> bool {
    let (mut top, mut bot) = (0, 0);
    for r in 0..8 {
        for c in 0..8 {
            if let Some((Color::White, _)) = g[r][c] {
                if r < 4 { top += 1 } else { bot += 1 }
            }
        }
    }
    top > bot
}

pub fn grid_to_fen(g: &Grid, white_top: bool, white_to_move: bool) -> String {
    let at = |r: usize, c: usize| if white_top { g[7 - r][7 - c] } else { g[r][c] }; // r,c: rank 8 -> 1, file a -> h
    let mut fen = String::new();
    for r in 0..8 {
        let mut empty = 0;
        for c in 0..8 {
            match at(r, c) {
                None => empty += 1,
                Some((col, role)) => {
                    if empty > 0 {
                        fen += &empty.to_string();
                        empty = 0;
                    }
                    let ch = role_char(role).to_ascii_lowercase();
                    fen.push(if col == Color::White { ch.to_ascii_uppercase() } else { ch });
                }
            }
        }
        if empty > 0 {
            fen += &empty.to_string();
        }
        if r < 7 {
            fen.push('/');
        }
    }
    // castling rights guessed from king/rook on their home squares
    let is = |r, c, col, role| at(r, c) == Some((col, role));
    let mut cas = String::new();
    if is(7, 4, Color::White, Role::King) {
        if is(7, 7, Color::White, Role::Rook) { cas.push('K') }
        if is(7, 0, Color::White, Role::Rook) { cas.push('Q') }
    }
    if is(0, 4, Color::Black, Role::King) {
        if is(0, 7, Color::Black, Role::Rook) { cas.push('k') }
        if is(0, 0, Color::Black, Role::Rook) { cas.push('q') }
    }
    if cas.is_empty() {
        cas.push('-');
    }
    format!("{fen} {} {cas} - 0 1", if white_to_move { 'w' } else { 'b' })
}

/// Index into the image-oriented grid for a board square (file 0..8 = a..h, rank 0..8 = 1..8).
pub fn grid_index(file: usize, rank: usize, white_top: bool) -> (usize, usize) {
    let (r, c) = (7 - rank, file);
    if white_top { (7 - r, 7 - c) } else { (r, c) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shakmaty::Role::*;

    const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

    fn bundled_templates() -> Templates {
        let files: [(Color, Role, &str); 12] = [
            (Color::White, Pawn, "wP"), (Color::White, Knight, "wN"), (Color::White, Bishop, "wB"),
            (Color::White, Rook, "wR"), (Color::White, Queen, "wQ"), (Color::White, King, "wK"),
            (Color::Black, Pawn, "bP"), (Color::Black, Knight, "bN"), (Color::Black, Bishop, "bB"),
            (Color::Black, Rook, "bR"), (Color::Black, Queen, "bQ"), (Color::Black, King, "bK"),
        ];
        let data: Vec<(Color, Role, Vec<u8>)> = files.iter().map(|(c, r, f)| (*c, *r, std::fs::read(format!("assets/pieces/{f}.svg")).unwrap())).collect();
        Templates::new(&data.iter().map(|(c, r, d)| (*c, *r, d.as_slice())).collect::<Vec<_>>())
    }

    // Writes assets/neo_silhouettes.txt from the real screenshot (labels typed by hand below). Run once:
    //   cargo test gen_neo -- --ignored
    #[test]
    #[ignore]
    fn gen_neo_silhouettes() {
        let img = image::open("tests_data/chesscom_neo_flipped.png").unwrap().to_rgb8();
        let (bx, by, bw, bh) = find_board(&img).unwrap();
        let (cw, ch) = (bw / 8, bh / 8);
        let truth: Vec<Vec<char>> = "4r1k1/1Q3p1p/4q3/p7/5b2/2N4P/PP3PP1/3R2K1".split("/").map(|r| r.chars().flat_map(|c| if c.is_ascii_digit() { vec!['.'; c.to_digit(10).unwrap() as usize] } else { vec![c] }).collect()).collect();
        let mut lines: Vec<String> = vec![];
        for r in 0..8u32 { for c in 0..8u32 {
            let Some(cell) = read_cell(&img, bx + c * cw, by + r * ch, cw, ch) else { continue };
            let s = silhouette(&cell.mask, cell.w, cell.h).unwrap();
            let t = truth[7 - r as usize][7 - c as usize].to_ascii_uppercase();
            let role = ROLES.into_iter().find(|x| role_char(*x) == t).unwrap();
            let l = s.to_line(role);
            if !lines.iter().any(|x| x.split(' ').nth(1) == l.split(' ').nth(1) && x.starts_with(t)) { lines.push(l); }
        } }
        std::fs::write("assets/neo_silhouettes.txt", lines.join("
") + "
").unwrap();
        println!("wrote {} shapes", lines.len());
    }

    // Real chess.com screenshot (Neo pieces, board flipped, white pieces at the top).
    #[test]
    fn real_chesscom_screenshot() {
        let img = image::open("tests_data/chesscom_neo_flipped.png").unwrap().to_rgb8();
        let g = read_board(&img, &bundled_templates()).unwrap().grid;
        let wt = white_on_top(&g);
        for row in g.iter() {
            println!("{}", row.iter().map(|c| match c { None => ".".to_string(), Some((col, r)) => { let ch = format!("{:?}", r).chars().next().unwrap(); let ch = if *r == Knight { "n".chars().next().unwrap() } else { ch }; if *col == Color::White { ch.to_ascii_uppercase().to_string() } else { ch.to_ascii_lowercase().to_string() } } }).collect::<Vec<_>>().join(" "));
        }
        let fen = grid_to_fen(&g, wt, false);
        println!("white_on_top={wt}
fen={fen}");
        assert!(wt);
        assert!(fen.starts_with("4r1k1/1Q3p1p/4q3/p7/5b2/2N4P/PP3PP1/3R2K1 "), "{fen}");
    }

    // Draws a synthetic chess.com-style board from the bundled SVGs and reads it back.
    #[test]
    fn roundtrip() {
        let files: [(Color, Role, &str); 12] = [
            (Color::White, Pawn, "wP"), (Color::White, Knight, "wN"), (Color::White, Bishop, "wB"),
            (Color::White, Rook, "wR"), (Color::White, Queen, "wQ"), (Color::White, King, "wK"),
            (Color::Black, Pawn, "bP"), (Color::Black, Knight, "bN"), (Color::Black, Bishop, "bB"),
            (Color::Black, Rook, "bR"), (Color::Black, Queen, "bQ"), (Color::Black, King, "bK"),
        ];
        let data: Vec<(Color, Role, Vec<u8>)> = files
            .iter()
            .map(|(c, r, f)| (*c, *r, std::fs::read(format!("assets/pieces/{f}.svg")).unwrap()))
            .collect();
        let tpl = Templates::new(&data.iter().map(|(c, r, d)| (*c, *r, d.as_slice())).collect::<Vec<_>>());
        for flip in [false, true] {
            let mut img = RgbImage::new(880, 880);
            for p in img.pixels_mut() { *p = image::Rgb([48, 46, 43]); }
            let fen: shakmaty::fen::Fen = START.parse().unwrap();
            let setup = fen.as_setup().clone();
            for r in 0..8u32 {
                for c in 0..8u32 {
                    let light = (r + c) % 2 == 0;
                    let col = if light { [238, 238, 210] } else { [118, 150, 86] };
                    for y in 0..100 { for x in 0..100 { img.put_pixel(40 + c * 100 + x, 40 + r * 100 + y, image::Rgb(col)); } }
                    let (fr, ff) = if flip { (r, 7 - c) } else { (7 - r, c) };
                    let sq = shakmaty::Square::from_coords(shakmaty::File::new(ff), shakmaty::Rank::new(fr));
                    if let Some(pc) = setup.board.piece_at(sq) {
                        let d = &data.iter().find(|(cc, rr, _)| *cc == pc.color && *rr == pc.role).unwrap().2;
                        let px = raster(d, 92);
                        for y in 0..92u32 { for x in 0..92u32 {
                            let i = ((y * 92 + x) * 4) as usize;
                            let a = px[i + 3] as f32 / 255.0;
                            let (ox, oy) = (40 + c * 100 + 4 + x, 40 + r * 100 + 4 + y);
                            let bg = img.get_pixel(ox, oy).0;
                            let mix = |k: usize| (px[i + k] as f32 + bg[k] as f32 * (1.0 - a)) as u8;
                            img.put_pixel(ox, oy, image::Rgb([mix(0), mix(1), mix(2)]));
                        } }
                    }
                }
            }
            let g = read_board(&img, &tpl).unwrap().grid;
            let wt = white_on_top(&g);
            assert_eq!(wt, flip);
            assert_eq!(grid_to_fen(&g, wt, true), START, "flip={flip}");
        }
    }
}
