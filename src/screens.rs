//! The screens (see the "Gambito — Redesign" canvas): a fixed side menu, the board screen shared by
//! Jogar / Análise da partida / Puzzles / lessons, and the full pages Início, Analisar, Aprender, Definições.
//! Images are decoded the first time they are drawn, so they cost nothing until a page shows them.
use crate::i18n::tl;
use crate::train::{daily, today, LESSONS};
use crate::*;
use shakmaty::FromSetup;

const PANEL_W: (f32, f32) = (380.0, 460.0);
const SIDE: f32 = 232.0;
const LINE2: Color32 = Color32::from_rgb(52, 52, 58);
const ELEV: Color32 = Color32::from_rgb(27, 27, 31);
pub(crate) const TEXT2: Color32 = Color32::from_rgb(216, 209, 191);
pub(crate) const GOLD_LINE: Color32 = Color32::from_rgb(232, 176, 48);
pub(crate) const GOLD_CARD: Color32 = Color32::from_rgb(29, 24, 12);

/// The pictures of the app: flat 2D art (made with the ChatGPT image generator, see THIRD_PARTY.md) on a transparent background, as small WebPs.
pub(crate) fn art_bytes(name: &str) -> &'static [u8] {
    match name {
        "home_banner" => include_bytes!("../assets/art/home_banner.webp"),
        "card_play" => include_bytes!("../assets/art/card_play.webp"),
        "card_puzzles" => include_bytes!("../assets/art/card_puzzles.webp"),
        "card_learn" => include_bytes!("../assets/art/card_learn.webp"),
        "card_analyse" => include_bytes!("../assets/art/card_analyse.webp"),
        "lesson_tactics" => include_bytes!("../assets/art/lesson_tactics.webp"),
        "lesson_mates" => include_bytes!("../assets/art/lesson_mates.webp"),
        "lesson_endgames" => include_bytes!("../assets/art/lesson_endgames.webp"),
        "lesson_openings" => include_bytes!("../assets/art/lesson_openings.webp"),
        "knight" => include_bytes!("../assets/setup/knight.png"),
        "game_memory" => include_bytes!("../assets/art/game_memory.webp"),
        "game_trail" => include_bytes!("../assets/art/game_trail.webp"),
        "game_blind" => include_bytes!("../assets/art/game_blind.webp"),
        "game_check" => include_bytes!("../assets/art/game_check.webp"),
        "game_mates" => include_bytes!("../assets/art/game_mates.webp"),
        "game_change" => include_bytes!("../assets/art/game_change.webp"),
        "game_legal" => include_bytes!("../assets/art/game_legal.webp"),
        "game_crash" => include_bytes!("../assets/art/game_crash.webp"),
        "game_mines" => include_bytes!("../assets/art/game_mines.webp"),
        "game_queens" => include_bytes!("../assets/art/game_queens.webp"),
        "game_pairs" => include_bytes!("../assets/art/game_pairs.webp"),
        _ => include_bytes!("../assets/art/empty_state.webp"),
    }
}

/// White's winning chances (0..100) back to centipawns, for the number on the evaluation bar.
fn cp_of_win(w: f32) -> f32 {
    let w = w.clamp(0.5, 99.5);
    -(100.0 / w - 1.0).ln() / 0.003_682_08
}

impl App {
    pub(crate) fn art(&mut self, name: &'static str) -> (egui::TextureId, [usize; 2]) {
        if let Some((t, s)) = self.art.get(name) { return (t.id(), *s); }
        let img = image::load_from_memory(art_bytes(name)).map(|i| i.to_rgba8()).unwrap_or_else(|_| image::RgbaImage::new(1, 1));
        let size = [img.width() as usize, img.height() as usize];
        let t = self.ctx.load_texture(name, egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()), egui::TextureOptions::LINEAR);
        let id = t.id();
        self.art.insert(name, (t, size));
        (id, size)
    }

    /// Draws a picture (flat art on a transparent background) as tall as `rect` allows, never cropped,
    /// pushed to one side (`anchor_x` 0 left … 1 right).
    fn art_cover(&mut self, ui: &mut egui::Ui, name: &'static str, rect: Rect, _radius: u8, anchor_x: f32) {
        let (id, size) = self.art(name);
        let (iw, ih) = (size[0] as f32, size[1] as f32);
        let pad = rect.height() * 0.08;
        let (aw, ah) = (rect.width() - 2.0 * pad, rect.height() - 2.0 * pad);
        let s = (ah / ih).min(aw / iw);
        let (w, h) = (iw * s, ih * s);
        let at = Pos2::new(rect.left() + pad + (aw - w) * anchor_x, rect.center().y - h / 2.0);
        ui.painter().image(id, Rect::from_min_size(at, Vec2::new(w, h)), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
    }

    /// The whole window: side menu, then one screen.
    pub(crate) fn screens(&mut self, ui: &mut egui::Ui) {
        let full = ui.max_rect();
        ui.painter().rect_filled(full, 0.0, BG);
        let side = Rect::from_min_max(full.min, Pos2::new(full.left() + SIDE, full.bottom()));
        let content = Rect::from_min_max(Pos2::new(side.right(), full.top()), full.max);
        ui.scope_builder(egui::UiBuilder::new().max_rect(side), |ui| { ui.set_clip_rect(side); self.side_menu(ui, side) });
        let learn_list = self.tab == 4 && self.lesson.is_none() && !matches!(self.mode, Mode::Drill(_)) && !matches!(&self.mode, Mode::Puzzle(r) if r.lesson.is_some());
        let page = |ui: &mut egui::Ui, f: &mut dyn FnMut(&mut egui::Ui)| {
            let r = content.shrink2(Vec2::new(40.0, 30.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    egui::Frame::new().inner_margin(egui::Margin { left: 0, right: 14, top: 0, bottom: 8 }).show(ui, |ui| { ui.set_width(ui.available_width()); f(ui) });
                });
            });
        };
        match self.tab {
            5 => page(ui, &mut |ui| self.home_page(ui)),
            1 => page(ui, &mut |ui| self.analyse_page(ui)),
            9 => page(ui, &mut |ui| self.settings_page(ui)),
            4 if learn_list => page(ui, &mut |ui| self.lessons_page(ui)),
            6 if self.game.is_some() => self.game_screen(ui, content),
            6 => page(ui, &mut |ui| self.games_page(ui)),
            _ => self.board_screen(ui, content),
        }
        if self.tab_t.0 != self.tab {
            self.tab_t = (self.tab, self.time);
            let bit = 1u16 << (self.tab as u32).min(15);
            self.replay = self.seen_tabs & bit == 0;
            self.seen_tabs |= bit;
            self.request_hints();
        }
        self.toast_ui(ui, content);
        let t = self.time;
        if self.anim.map_or(false, |a| t - a.t0 < ANIM_SECS + 0.05) || t - self.badge_t.1 < 0.35 || t - self.hints_t0 < 0.35 {
            ui.ctx().request_repaint();
        }
    }

    // ---------- side menu ----------

    /// Seconds since the current screen appeared (what the entrances are timed by).
    /// A screen's entrance (cards rising, numbers counting, bars filling) plays only the first time it is opened in a
    /// session: the menu is used tens of times a day, so later visits are instant.
    pub(crate) fn page_age(&self) -> f64 { if self.replay { self.time - self.tab_t.1 } else { 10.0 } }

    fn side_menu(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let (knight, _) = self.art("knight");
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 0.0, RAIL);
        p.line_segment([rect.right_top(), rect.right_bottom()], Stroke::new(1.0, Color32::from_rgb(30, 30, 34)));
        // the installer's knight, whole, above Definições
        let k = Rect::from_min_size(Pos2::new(rect.left() + 6.0, rect.bottom() - 318.0), Vec2::splat(rect.width() - 12.0));
        p.image(knight, k, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::from_white_alpha(150));
        let mark = Rect::from_min_size(Pos2::new(rect.left() + 24.0, rect.top() + 24.0), Vec2::splat(42.0));
        p.image(self.logo.id(), mark, logo_uv(1), Color32::WHITE);
        p.text(Pos2::new(mark.right() + 12.0, mark.center().y), egui::Align2::LEFT_CENTER, "Gambito", f_semi(23.0), CREAM);
        p.line_segment([Pos2::new(rect.left() + 24.0, rect.top() + 90.0), Pos2::new(rect.right() - 24.0, rect.top() + 90.0)], Stroke::new(1.0, Color32::from_rgb(34, 34, 39)));
        let daily_open = self.progress.daily.map_or(true, |d| d.0 != today());
        let items: [(Ico, &str, u8, bool); 6] = [
            (Ico::Home, tl("Início"), 5, false), (Ico::Knight, tl("Jogar"), 0, false), (Ico::Puzzle, tl("Exercícios"), 3, daily_open),
            (Ico::Bolt, tl("Jogos"), 6, false), (Ico::Cap, tl("Aprender"), 4, false), (Ico::Bars, tl("Analisar"), 1, false),
        ];
        let mut y = rect.top() + 106.0;
        let mut go = None;
        // the gold highlight glides to the selected item
        let sel_i = items.iter().position(|it| match it.2 { 1 => self.tab == 1 || self.tab == 2, t => self.tab == t });
        if let Some(i) = sel_i {
            let gy = motion::glide(ui.ctx(), egui::Id::new("menu_sel"), y + i as f32 * 50.0, 0.22);
            p.rect_filled(Rect::from_min_size(Pos2::new(rect.left() + 14.0, gy), Vec2::new(rect.width() - 28.0, 46.0)), 12.0, GOLD_BG);
        }
        let mut row = |ui: &mut egui::Ui, y: f32, icon: Ico, label: &str, target: u8, badge: bool, selected: bool| {
            let r = Rect::from_min_size(Pos2::new(rect.left() + 14.0, y), Vec2::new(rect.width() - 28.0, 46.0));
            let resp = ui.interact(r, egui::Id::new(("menu", target)), Sense::click());
            if selected && target == 9 { p.rect_filled(r, 12.0, GOLD_BG); } else if !selected && resp.hovered() { p.rect_filled(r, 12.0, Color32::from_rgb(24, 24, 28)); }
            let col = if selected { GOLD } else { TEXT2 };
            ico(&p, icon, Pos2::new(r.left() + 26.0, r.center().y), 22.0, col);
            p.text(Pos2::new(r.left() + 52.0, r.center().y), egui::Align2::LEFT_CENTER, label, f_med(16.0), col);
            if badge {
                let c = Pos2::new(r.right() - 20.0, r.center().y);
                p.circle_filled(c, 10.0, GOLD);
                p.text(c, egui::Align2::CENTER_CENTER, "1", f_semi(12.0), Color32::from_rgb(26, 18, 4));
            }
            if resp.on_hover_text(label).clicked() { go = Some(target); }
        };
        for (icon, label, target, badge) in items {
            let selected = match target { 1 => self.tab == 1 || self.tab == 2, t => self.tab == t };
            row(ui, y, icon, label, target, badge, selected);
            y += 50.0;
        }
        // a new version: the button that installs it (and its progress)
        let (label, frac) = match self.upd() {
            Upd::Found(r) => (Some(tr!("Atualizar para {}", r.version)), None),
            Upd::Getting(f) => (Some(tr!("A descarregar… {}%", (f * 100.0).round())), Some(f)),
            Upd::Restarting => (Some(tl("A reiniciar…").to_string()), Some(1.0)),
            _ => (None, None),
        };
        if let Some(label) = label {
            let r = Rect::from_min_size(Pos2::new(rect.left() + 14.0, rect.bottom() - 124.0), Vec2::new(rect.width() - 28.0, 42.0));
            let resp = ui.interact(r, egui::Id::new("update_btn"), Sense::click());
            p.rect_filled(r, 12.0, if resp.hovered() && frac.is_none() { GOLD_HI } else { GOLD });
            if let Some(f) = frac { p.rect_filled(Rect::from_min_size(r.min, Vec2::new(r.width() * f, r.height())), 12.0, GOLD_LO); }
            ico(&p, Ico::Download, Pos2::new(r.left() + 24.0, r.center().y), 20.0, Color32::from_rgb(28, 20, 4));
            p.text(Pos2::new(r.left() + 44.0, r.center().y), egui::Align2::LEFT_CENTER, label, f_semi(14.5), Color32::from_rgb(28, 20, 4));
            if resp.on_hover_text(tl("Descarrega a versão nova, fecha o Gambito e volta a abri-lo já atualizado.")).clicked() && frac.is_none() { self.start_update(); }
        }
        row(ui, rect.bottom() - 70.0, Ico::Gear, tl("Definições"), 9, false, self.tab == 9);
        p.text(Pos2::new(rect.center().x, rect.bottom() - 14.0), egui::Align2::CENTER_CENTER, format!("Gambito {}", update::VERSION), egui::FontId::proportional(12.0), Color32::from_rgb(120, 115, 104));
        match go {
            Some(0) => { self.back_to_game(); self.tab = 0; }
            Some(3) => { self.tab = 3; if !matches!(&self.mode, Mode::Puzzle(r) if r.lesson.is_none()) { self.next_puzzle(None); } }
            Some(t) => self.tab = t,
            None => {}
        }
    }

    // ---------- the board screen ----------

    fn board_screen(&mut self, ui: &mut egui::Ui, content: Rect) {
        let m = 24.0;
        let panel_w = (content.width() * 0.36).clamp(PANEL_W.0, PANEL_W.1);
        let panel = Rect::from_min_max(Pos2::new(content.right() - m - panel_w, content.top() + m), Pos2::new(content.right() - m, content.bottom() - m));
        let area = Rect::from_min_max(Pos2::new(content.left() + 24.0, content.top() + 14.0), Pos2::new(panel.left() - 20.0, content.bottom() - 14.0));
        self.board_column(ui, area);
        self.side_panel(ui, panel);
    }

    fn board_column(&mut self, ui: &mut egui::Ui, area: Rect) {
        let game = matches!(self.tab, 0 | 2);
        let (bar_w, tool_w, gap, row_h) = (30.0, 48.0, 12.0, 52.0);
        let avail_h = area.height() - 2.0 * row_h - 16.0;
        let avail_w = area.width() - bar_w - tool_w - 2.0 * gap;
        let side = ((avail_h.min(avail_w) / 8.0).floor() * 8.0).max(240.0);
        let group_w = bar_w + gap + side + gap + tool_w;
        let x0 = area.left() + ((area.width() - group_w) / 2.0).max(0.0);
        let y0 = area.top() + ((area.height() - side - 2.0 * row_h - 16.0) / 2.0).max(0.0);
        let board = Rect::from_min_size(Pos2::new(x0 + bar_w + gap, y0 + row_h + 8.0), Vec2::splat(side));
        let (top, bottom) = if self.flipped { (Color::White, Color::Black) } else { (Color::Black, Color::White) };
        let shown = self.pos_at(self.view.unwrap_or(self.hist.len())).clone();
        let p = ui.painter().clone();

        self.player_row(&p, Rect::from_min_size(Pos2::new(board.left(), y0), Vec2::new(side, row_h)), top, &shown, true);
        ui.scope_builder(egui::UiBuilder::new().max_rect(board), |ui| self.board_ui(ui, side));
        if game { self.eval_bar(ui, Rect::from_min_size(Pos2::new(x0, board.top()), Vec2::new(bar_w, side))); }
        self.board_tools(ui, Rect::from_min_size(Pos2::new(board.right() + gap, board.top()), Vec2::new(tool_w, side)), game);
        let bottom_row = Rect::from_min_size(Pos2::new(board.left(), board.bottom() + 8.0), Vec2::new(side + gap + tool_w, row_h));
        self.player_row(&p, bottom_row, bottom, &shown, false);
        if game || !self.hist.is_empty() { self.nav_buttons(ui, bottom_row); }
        if game { self.result_card(ui, board); }
    }

    /// The notice of `notify`: a pill at the top of the page for a few seconds (a click closes it).
    fn toast_ui(&mut self, ui: &mut egui::Ui, content: Rect) {
        const LIFE: f64 = 4.5;
        let Some((text, kind, t0)) = self.toast.clone() else { return };
        let age = self.time - t0;
        if age > LIFE { self.toast = None; return; }
        ui.ctx().request_repaint();
        let a = (age / 0.12).min(1.0).min((LIFE - age) / 0.18) as f32; // fade in 120 ms, out 180 ms (leaving is faster)
        // slides down 24 px with an ease-out in 250 ms: no bounce, and still when animations are off
        let t = if motion::on() { (age / 0.25).clamp(0.0, 1.0) as f32 } else { 1.0 };
        let back = motion::ease_out(t);
        let p = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("toast")));
        let g = p.layout(text, egui::FontId::proportional(14.5), CREAM.gamma_multiply(a), 520.0);
        let size = g.size() + Vec2::new(66.0, 26.0);
        let r = Rect::from_center_size(Pos2::new(content.center().x, content.top() + 26.0 + size.y / 2.0 - (1.0 - back) * 24.0), size);
        p.rect_filled(r.translate(Vec2::new(0.0, 4.0)), 14.0, Color32::from_black_alpha((90.0 * a) as u8));
        // 3 = an achievement: gold card with a trophy
        p.rect_filled(r, 14.0, if kind == 3 { GOLD_CARD } else { CARD2 }.gamma_multiply(a));
        let (accent, icon) = match kind { 0 => (POS, Ico::CheckCircle), 1 => (Color32::from_rgb(240, 110, 90), Ico::Cross), 3 => (GOLD, Ico::Trophy), _ => (GOLD, Ico::Bolt) };
        p.rect_stroke(r, 14.0, Stroke::new(if kind == 3 { 1.5 } else { 1.0 }, accent.gamma_multiply(0.55 * a)), egui::StrokeKind::Inside);
        let ic = Pos2::new(r.left() + 26.0, r.center().y);
        if kind == 1 { p.circle_stroke(ic, 10.0, Stroke::new(1.6, accent.gamma_multiply(a))); ico(&p, icon, ic, 11.0, accent.gamma_multiply(a)); } else { ico(&p, icon, ic, 22.0, accent.gamma_multiply(a)); }
        p.galley(Pos2::new(r.left() + 48.0, r.center().y - g.size().y / 2.0), g, CREAM.gamma_multiply(a));
        // how long it stays: a thin line that runs out along the bottom
        let left = (1.0 - age / LIFE).clamp(0.0, 1.0) as f32;
        let line = Rect::from_min_size(Pos2::new(r.left() + 14.0, r.bottom() - 4.0), Vec2::new((r.width() - 28.0) * left, 2.0));
        p.rect_filled(line, 1.0, accent.gamma_multiply(0.6 * a));
        if ui.input(|i| i.pointer.any_click() && i.pointer.interact_pos().is_some_and(|q| r.contains(q))) { self.toast = None; }
    }

    /// End of the game: a card over the board with the result and what to do next.
    fn result_card(&mut self, ui: &mut egui::Ui, board: Rect) {
        let Some((title, line, kind, t0)) = self.result.clone() else { return };
        if !(self.pos.is_game_over() || self.resigned) || self.view.is_some() { return; }
        let t = ((self.time - t0) / 0.3).clamp(0.0, 1.0) as f32;
        if t < 1.0 { ui.ctx().request_repaint(); }
        let ease = 1.0 - (1.0 - t).powi(3);
        let p = ui.painter().clone();
        p.rect_filled(board, 0.0, Color32::from_black_alpha((100.0 * ease) as u8));
        let w = (board.width() * 0.62).clamp(270.0, 340.0);
        let card = Rect::from_center_size(board.center() + Vec2::new(0.0, (1.0 - ease) * 16.0), Vec2::new(w, 256.0));
        p.rect_filled(card.translate(Vec2::new(0.0, 8.0)).expand(4.0), 22.0, Color32::from_black_alpha(90));
        p.rect_filled(card, 18.0, CARD);
        p.rect_stroke(card, 18.0, Stroke::new(1.0, if kind == 0 { GOLD_LINE } else { LINE2 }), egui::StrokeKind::Inside);
        if kind == 0 && self.time - t0 < 2.5 { // a win: gold confetti, once
            motion::confetti(&p, board, (self.time - t0) as f32, &[GOLD, CREAM, GREEN]);
            ui.ctx().request_repaint();
        }
        let c = Pos2::new(card.center().x, card.top() + 48.0);
        match kind {
            0 => { p.circle_filled(c, 26.0, GOLD_BG); p.circle_stroke(c, 26.0, Stroke::new(1.5, GOLD_LINE)); ico(&p, Ico::Trophy, c, 28.0, GOLD); }
            1 => { p.circle_filled(c, 26.0, Color32::from_rgb(48, 22, 22)); ico(&p, Ico::Flag, c, 26.0, Color32::from_rgb(240, 110, 90)); }
            _ => { p.circle_filled(c, 26.0, ELEV); p.text(c, egui::Align2::CENTER_CENTER, "½", f_semi(24.0), CREAM); }
        }
        p.text(Pos2::new(card.center().x, card.top() + 96.0), egui::Align2::CENTER_CENTER, title, f_semi(22.0), CREAM);
        let g = p.layout(line, egui::FontId::proportional(14.0), MUTED, w - 48.0);
        let gw = g.size().x;
        p.galley(Pos2::new(card.center().x - gw / 2.0, card.top() + 114.0), g, MUTED);
        let x = Rect::from_center_size(card.right_top() + Vec2::new(-24.0, 24.0), Vec2::splat(28.0));
        let xr = ui.interact(x, egui::Id::new("result_x"), Sense::click());
        ico(&p, Ico::Cross, x.center(), 14.0, if xr.hovered() { CREAM } else { MUTED });
        if xr.on_hover_text(tl("Fechar")).clicked() { self.result = None; }
        let bw = w - 40.0;
        let review = Rect::from_min_size(Pos2::new(card.left() + 20.0, card.bottom() - 108.0), Vec2::new(bw, 44.0));
        let again = Rect::from_min_size(Pos2::new(card.left() + 20.0, card.bottom() - 56.0), Vec2::new(bw, 40.0));
        if ui.scope_builder(egui::UiBuilder::new().max_rect(review), |ui| gold_btn(ui, tl("Ver a análise"), Some(Ico::Bars), Vec2::new(bw, 44.0))).inner {
            self.panel_tab = 2;
            self.result = None;
        }
        if ui.scope_builder(egui::UiBuilder::new().max_rect(again), |ui| dark_btn(ui, tl("Novo jogo"), Some(Ico::Restart), Vec2::new(bw, 40.0), false)).inner {
            self.result = None;
            self.new_menu = true;
        }
    }

    fn player_row(&self, p: &egui::Painter, rect: Rect, c: Color, shown: &Chess, top: bool) {
        let av = Rect::from_min_size(Pos2::new(rect.left(), rect.center().y - 20.0), Vec2::splat(40.0));
        p.rect_filled(av, 10.0, ELEV);
        p.rect_stroke(av, 10.0, Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
        p.image(self.tex[&(c, Role::King)].id(), av.shrink(5.0), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        let (name, sub) = (self.player_name(c), self.player_sub(c, shown, top));
        let x = av.right() + 12.0;
        let turn = shown.turn() == c && !shown.is_game_over();
        if sub.is_empty() {
            p.text(Pos2::new(x, rect.center().y), egui::Align2::LEFT_CENTER, name, f_semi(17.0), CREAM);
        } else {
            p.text(Pos2::new(x, rect.center().y - 9.0), egui::Align2::LEFT_CENTER, name, f_semi(17.0), CREAM);
            p.text(Pos2::new(x, rect.center().y + 11.0), egui::Align2::LEFT_CENTER, sub, egui::FontId::proportional(13.0), MUTED);
        }
        if turn { p.circle_filled(Pos2::new(av.left() - 9.0, av.center().y), 3.5, POS); }
    }

    /// White's share of the bar, animated towards the latest evaluation; the number sits on the side that is better.
    fn eval_bar(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let idx = self.view.unwrap_or(self.hist.len());
        let target = match self.review.as_ref().and_then(|r| r.series.get(idx)) {
            Some(w) => *w,
            None if self.eval_mate => if self.eval_cp > 0 { 100.0 } else { 0.0 },
            None => win_pct(self.eval_cp as f64) as f32,
        };
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        self.eval_shown += (target - self.eval_shown) * (dt * 7.0).min(1.0);
        if (target - self.eval_shown).abs() > 0.15 { ui.ctx().request_repaint(); } else { self.eval_shown = target; }
        let p = ui.painter();
        p.rect_filled(rect, 8.0, Color32::from_rgb(43, 43, 48));
        let white_h = rect.height() * self.eval_shown / 100.0;
        let wr = Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - white_h), rect.max);
        p.rect_filled(wr, egui::CornerRadius { nw: if white_h > rect.height() - 8.0 { 8 } else { 0 }, ne: if white_h > rect.height() - 8.0 { 8 } else { 0 }, sw: 8, se: 8 }, Color32::from_rgb(245, 239, 225));
        let txt = if self.eval_mate && self.review.is_none() { format!("M{}", self.hints.first().map_or(0, |h| h.ev.trim_start_matches(['+', '-', '#']).trim_start_matches('#').parse::<i32>().unwrap_or(0))) }
            else { format!("{:.1}", (cp_of_win(target) / 100.0).abs().min(99.0)) };
        if target >= 50.0 {
            p.text(Pos2::new(rect.center().x, rect.bottom() - 10.0), egui::Align2::CENTER_BOTTOM, txt, f_semi(11.0), Color32::from_rgb(20, 20, 20));
        } else {
            p.text(Pos2::new(rect.center().x, rect.top() + 10.0), egui::Align2::CENTER_TOP, txt, f_semi(11.0), CREAM);
        }
    }

    /// Square button of the column next to the board. kind: 0 normal, 1 primary (gold), 2 switched on.
    fn tool(&self, ui: &mut egui::Ui, rect: Rect, icon: Ico, tip: &str, kind: u8) -> bool {
        let resp = ui.interact(rect, egui::Id::new(("tool", tip)), Sense::click());
        let p = ui.painter();
        let hot = resp.hovered();
        let (fill, stroke, fg) = match kind {
            1 => (if hot { GOLD_HI } else { GOLD }, Stroke::NONE, Color32::from_rgb(26, 18, 4)),
            2 => (GOLD_BG, Stroke::new(1.5, GOLD_LINE), GOLD),
            _ => (if hot { Color32::from_rgb(36, 36, 41) } else { ELEV }, Stroke::new(1.0, LINE2), CREAM),
        };
        p.rect_filled(rect, 12.0, fill);
        p.rect_stroke(rect, 12.0, stroke, egui::StrokeKind::Inside);
        ico(p, icon, rect.center(), 22.0, fg);
        resp.on_hover_text(tip).clicked()
    }

    fn board_tools(&mut self, ui: &mut egui::Ui, col: Rect, game: bool) {
        let b = |i: f32| Rect::from_min_size(Pos2::new(col.left(), col.top() + i * 58.0), Vec2::splat(48.0));
        if !game {
            if self.tool(ui, b(0.0), Ico::Flip, tl("Virar o tabuleiro (F)"), 0) { self.flipped = !self.flipped; }
            return;
        }
        if self.tool(ui, b(0.0), Ico::Restart, tl("Novo jogo"), 1) { self.new_menu = !self.new_menu; }
        if self.tool(ui, b(1.0), Ico::Flip, tl("Virar o tabuleiro (F)"), 0) { self.flipped = !self.flipped; }
        if self.tool(ui, b(2.0), Ico::Users, tl("Jogar os dois lados, sem engine"), if self.both { 2 } else { 0 }) {
            self.both = !self.both;
            self.thinking = false;
            self.after_move();
        }
        if self.tool(ui, b(3.0), Ico::Turn, tl("Trocar de quem é a vez (brancas / pretas)"), 0) { self.swap_turn(); }
        if self.new_menu {
            let anchor = b(0.0).right_top() + Vec2::new(10.0, 0.0);
            let mut pick = None;
            let area = egui::Area::new(egui::Id::new("newgame")).fixed_pos(anchor).order(egui::Order::Foreground).show(ui.ctx(), |ui| {
                egui::Frame::new().fill(CARD2).stroke(Stroke::new(1.0, LINE2)).corner_radius(14.0).inner_margin(8.0).show(ui, |ui| {
                    ui.set_width(220.0);
                    ui.label(med(tl("Novo jogo"), 15.0).color(MUTED));
                    for (k, label) in [tl("Jogar de brancas"), tl("Jogar de pretas"), tl("Os dois lados")].iter().enumerate() {
                        if ui.add(egui::Button::new(med(*label, 16.0)).frame(false).min_size(Vec2::new(204.0, 40.0))).clicked() { pick = Some(k); }
                    }
                });
            });
            let outside = ui.input(|i| i.pointer.any_click()) && !area.response.hovered() && !b(0.0).contains(ui.input(|i| i.pointer.interact_pos().unwrap_or_default()));
            match pick {
                Some(2) => { self.both = true; self.new_game(Color::White); self.new_menu = false; }
                Some(k) => { self.both = false; self.new_game(if k == 0 { Color::White } else { Color::Black }); self.new_menu = false; }
                None if outside => self.new_menu = false,
                None => {}
            }
        }
    }

    /// Whose turn it is, switched on the position on the board (kept if it stays legal).
    fn swap_turn(&mut self) {
        let shown = self.pos_at(self.view.unwrap_or(self.hist.len())).clone();
        let mut setup = shown.to_setup(EnPassantMode::Legal);
        setup.turn = !setup.turn;
        setup.ep_square = None;
        match Chess::from_setup(setup, CastlingMode::Standard) {
            Ok(p) => {
                self.leave_training();
                self.pos = p;
                self.reset_history();
                self.loaded = None;
                self.last = None;
                self.sel = None;
                if !self.both { self.me = self.pos.turn(); }
                self.msg = if self.pos.turn() == Color::White { tl("Agora jogam as brancas.") } else { tl("Agora jogam as pretas.") }.into();
                self.after_move();
            }
            Err(_) => self.msg = tl("Não dá para trocar a vez: o rei de quem acabou de jogar ficaria em xeque.").into(),
        }
    }

    fn nav_buttons(&mut self, ui: &mut egui::Ui, row: Rect) {
        let cur = self.view.unwrap_or(self.hist.len());
        let icons = [(Ico::SkipPrev, tl("Início da partida (Home)")), (Ico::ChevL, tl("Lance anterior (←)")), (if self.playing { Ico::Pause } else { Ico::Play }, tl("Reproduzir")), (Ico::ChevR, tl("Lance seguinte (→)")), (Ico::SkipNext, tl("Fim da partida (End)"))];
        let mut x = row.right() - 5.0 * 44.0 - 4.0 * 8.0;
        for (k, (icon, tip)) in icons.into_iter().enumerate() {
            let r = Rect::from_min_size(Pos2::new(x, row.center().y - 20.0), Vec2::new(44.0, 40.0));
            x += 52.0;
            let resp = ui.interact(r, egui::Id::new(("nav", k)), Sense::click());
            let p = ui.painter();
            p.rect_filled(r, 10.0, if resp.hovered() { Color32::from_rgb(36, 36, 41) } else { ELEV });
            p.rect_stroke(r, 10.0, Stroke::new(1.0, LINE2), egui::StrokeKind::Inside);
            ico(p, icon, r.center(), 18.0, CREAM);
            if resp.on_hover_text(tip).clicked() {
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

    // ---------- the panel next to the board ----------

    fn side_panel(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let p = ui.painter();
        p.rect_filled(rect, 20.0, PANEL);
        p.rect_stroke(rect, 20.0, Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
        let inner = rect.shrink2(Vec2::new(18.0, 18.0));
        if !matches!(self.tab, 0 | 2) {
            // puzzles and lessons keep their own panels
            ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
                ui.set_clip_rect(inner.expand(2.0));
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    egui::Frame::new().inner_margin(egui::Margin { left: 0, right: 12, top: 0, bottom: 6 }).show(ui, |ui| {
                        if self.tab == 3 { self.view_puzzle(ui) } else { self.view_learn(ui) }
                    });
                });
            });
            return;
        }
        // header
        let head = Rect::from_min_size(inner.min, Vec2::new(inner.width(), 50.0));
        let (title, sub) = self.game_title();
        // long names are cut with "…" so they never run under the game counter
        let room = head.width() - if self.game_pos().is_some() { 130.0 } else { 0.0 };
        let one_line = |text: String, font: egui::FontId, color: Color32| {
            let mut job = egui::text::LayoutJob::simple_singleline(text, font, color);
            job.wrap = egui::text::TextWrapping { max_width: room, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
            ui.fonts_mut(|f| f.layout_job(job))
        };
        p.galley(head.left_top(), one_line(title, f_semi(20.0), CREAM), CREAM);
        p.galley(head.left_top() + Vec2::new(0.0, 28.0), one_line(sub, egui::FontId::proportional(13.0), MUTED), MUTED);
        if let Some((kind, i, n)) = self.game_pos() {
            let r = Rect::from_min_size(Pos2::new(head.right() - 120.0, head.top()), Vec2::new(120.0, 34.0));
            let (l, rr) = (Rect::from_min_size(r.min, Vec2::new(34.0, 34.0)), Rect::from_min_size(Pos2::new(r.right() - 34.0, r.top()), Vec2::new(34.0, 34.0)));
            let (lr, rrr) = (ui.interact(l, egui::Id::new("gprev"), Sense::click()), ui.interact(rr, egui::Id::new("gnext"), Sense::click()));
            ico(p, Ico::ChevL, l.center(), 18.0, if lr.hovered() { GOLD } else { CREAM });
            ico(p, Ico::ChevR, rr.center(), 18.0, if rrr.hovered() { GOLD } else { CREAM });
            p.text(r.center(), egui::Align2::CENTER_CENTER, format!("{} / {n}", i + 1), f_med(14.0), CREAM);
            if lr.on_hover_text(tl("Partida anterior")).clicked() && i > 0 { self.open_game(kind, i - 1); }
            if rrr.on_hover_text(tl("Partida seguinte")).clicked() && i + 1 < n { self.open_game(kind, i + 1); }
        }
        // tabs
        let tabs = Rect::from_min_size(Pos2::new(inner.left(), head.bottom() + 6.0), Vec2::new(inner.width(), 42.0));
        p.rect_filled(tabs, 12.0, BG);
        p.rect_stroke(tabs, 12.0, Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
        let names = [tl("Lances"), tl("Sugestões"), tl("Análise")];
        let w = (tabs.width() - 8.0) / 3.0;
        let glide = if motion::on() { motion::glide(ui.ctx(), egui::Id::new("ptab_glide"), self.panel_tab as f32, 0.18) } else { self.panel_tab as f32 };
        p.rect_filled(Rect::from_min_size(Pos2::new(tabs.left() + 4.0 + glide * w, tabs.top() + 4.0), Vec2::new(w, tabs.height() - 8.0)), 9.0, Color32::from_rgb(35, 35, 40));
        for (k, name) in names.iter().enumerate() {
            let r = Rect::from_min_size(Pos2::new(tabs.left() + 4.0 + k as f32 * w, tabs.top() + 4.0), Vec2::new(w, tabs.height() - 8.0));
            let resp = ui.interact(r, egui::Id::new(("ptab", k)), Sense::click());
            let on = self.panel_tab == k as u8;
            p.text(r.center(), egui::Align2::CENTER_CENTER, *name, f_med(15.0), if on { CREAM } else if resp.hovered() { TEXT2 } else { MUTED });
            if resp.clicked() { self.panel_tab = k as u8; }
        }
        // content and the pinned bottom
        let pinned_h = 110.0;
        let body = Rect::from_min_max(Pos2::new(inner.left(), tabs.bottom() + 12.0), Pos2::new(inner.right(), inner.bottom() - pinned_h));
        ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
            ui.set_clip_rect(body.expand(2.0));
            egui::ScrollArea::vertical().id_salt(("panel", self.panel_tab)).auto_shrink([false, false]).show(ui, |ui| {
                egui::Frame::new().inner_margin(egui::Margin { left: 0, right: 12, top: 0, bottom: 6 }).show(ui, |ui| {
                    match self.panel_tab { 0 => self.tab_moves(ui), 1 => self.hints_block(ui), _ => self.tab_analysis(ui) }
                });
            });
        });
        let bottom = Rect::from_min_max(Pos2::new(inner.left(), inner.bottom() - pinned_h + 14.0), inner.max);
        ui.painter().line_segment([Pos2::new(rect.left() + 1.0, bottom.top() - 8.0), Pos2::new(rect.right() - 1.0, bottom.top() - 8.0)], Stroke::new(1.0, LINE));
        ui.scope_builder(egui::UiBuilder::new().max_rect(bottom), |ui| self.panel_bottom(ui));
    }

    fn game_title(&self) -> (String, String) {
        let idx = self.view.unwrap_or(self.hist.len());
        let title = match &self.loaded {
            Some(i) => format!("{} vs {}", i.white, i.black),
            None if self.both => tl("Análise livre").into(),
            None => tr!("Partida contra {}", self.engines[self.opp].name),
        };
        let sub = match (self.opening(idx), &self.loaded) {
            (Some((eco, name)), _) => format!("{eco} · {name}"),
            (None, Some(i)) => i.result.clone(),
            _ => self.status(),
        };
        (title, sub)
    }

    /// Ctrl+V tip, and the two ways to change what is on the board.
    fn panel_bottom(&mut self, ui: &mut egui::Ui) {
        let full = ui.max_rect().width();
        // "Dica: Ctrl V cola um print ou um PGN", centred, with the shortcut as a key cap
        let tip = [tl("Dica:"), "Ctrl V", tl("cola um print ou um PGN")];
        let widths: Vec<f32> = tip.iter().map(|t| ui.painter().layout_no_wrap(t.to_string(), egui::FontId::proportional(13.0), MUTED).size().x).collect();
        let total = widths.iter().sum::<f32>() + 12.0 + 2.0 * 6.0;
        let (row, _) = ui.allocate_exact_size(Vec2::new(full, 22.0), Sense::hover());
        let p = ui.painter();
        let mut x = row.left() + ((full - total) / 2.0).max(0.0);
        p.text(Pos2::new(x, row.center().y), egui::Align2::LEFT_CENTER, tip[0], egui::FontId::proportional(13.0), GOLD);
        x += widths[0] + 6.0;
        let cap = Rect::from_min_size(Pos2::new(x, row.center().y - 10.0), Vec2::new(widths[1] + 12.0, 20.0));
        p.rect_filled(cap, 6.0, Color32::from_rgb(35, 35, 40));
        p.rect_stroke(cap, 6.0, Stroke::new(1.0, LINE2), egui::StrokeKind::Inside);
        p.text(cap.center(), egui::Align2::CENTER_CENTER, tip[1], egui::FontId::proportional(12.0), CREAM);
        p.text(Pos2::new(cap.right() + 6.0, row.center().y), egui::Align2::LEFT_CENTER, tip[2], egui::FontId::proportional(13.0), MUTED);
        ui.add_space(8.0);
        let w = (full - 12.0) / 2.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let editing = self.import.as_ref().map_or(false, |i| i.edit);
            if dark_btn(ui, if editing { tl("Fechar editor") } else { tl("Editor") }, Some(Ico::Pencil), Vec2::new(w, 50.0), true) {
                if editing { self.import = None; self.brush = None; } else { self.start_edit(); self.panel_tab = 0; }
            }
            if gold_btn(ui, tl("Importar"), Some(Ico::Image), Vec2::new(w, 50.0)) {
                if let Some(p) = rfd::FileDialog::new().add_filter(tl("Imagem ou PGN"), &["png", "jpg", "jpeg", "webp", "pgn", "txt"]).pick_file() {
                    self.open_file(&p);
                    self.panel_tab = 0;
                }
            }
        });
    }

    fn tab_moves(&mut self, ui: &mut egui::Ui) {
        let idx = self.view.unwrap_or(self.hist.len());
        if self.import.is_some() { self.import_card(ui); }
        // what the last move did
        let (title, text) = self.hero_text(idx);
        egui::Frame::new().fill(Color32::from_rgb(24, 24, 27)).stroke(Stroke::new(1.0, Color32::from_rgb(46, 46, 52))).corner_radius(16.0).inner_margin(egui::Margin::symmetric(14, 12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.add(piece_img(&self.tex[&(Color::White, Role::Knight)], 40.0));
                ui.vertical(|ui| {
                    ui.add(egui::Label::new(semi(title, 18.0)).truncate());
                    ui.add(egui::Label::new(egui::RichText::new(text).size(14.0).color(TEXT2)).wrap());
                });
            });
        });
        ui.add_space(8.0);
        if self.reviewing { self.review_bar(ui); }
        if !self.msg.is_empty() { ui.add(egui::Label::new(egui::RichText::new(&self.msg).color(AMBER)).wrap()); }
        if self.view.is_some() { ui.label(egui::RichText::new(tr!("A ver o lance {} de {}  (End = voltar ao fim)", idx, self.hist.len())).small().color(Color32::from_rgb(130, 180, 240))); }
        ui.add_space(8.0);
        // the moves, two per row
        let mut jump = None;
        let rows = self.hist.len().div_ceil(2);
        let cw = (ui.available_width() - 36.0) / 2.0;
        for row in 0..rows {
            egui::Frame::new().fill(if row % 2 == 1 { ROW_ALT } else { Color32::TRANSPARENT }).corner_radius(6.0).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    ui.add_sized([32.0, 26.0], egui::Label::new(egui::RichText::new(format!("{}.", row + 1)).color(MUTED)));
                    for k in 0..2 {
                        let i = row * 2 + k;
                        if i >= self.hist.len() { break; }
                        let q = self.quality.get(i).and_then(|x| x.as_ref()).map(|x| x.0);
                        let mut rt = egui::RichText::new(format!("{}{}", self.hist[i].1, q.map_or("", |c| c.sym()))).size(15.0);
                        if let Some(c) = q { rt = rt.color(c.color()); }
                        if ui.add_sized([cw, 26.0], egui::Button::new(rt).frame(false).selected(idx == i + 1)).clicked() { jump = Some(i + 1); }
                    }
                });
            });
        }
        if self.hist.is_empty() { ui.label(egui::RichText::new(tl("Ainda sem lances.")).color(MUTED)); }
        if let Some(j) = jump { self.playing = false; self.goto(j); }
        ui.add_space(10.0);
        let bw = (ui.available_width() - 16.0) / 3.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if dark_btn(ui, tl("Desfazer"), None, Vec2::new(bw, 38.0), false) { self.undo(); }
            if dark_btn(ui, tl("Copiar PGN"), None, Vec2::new(bw, 38.0), false) && !self.mvs.is_empty() {
                ui.ctx().copy_text(self.export_pgn());
                self.msg = tl("PGN copiado para a área de transferência.").into();
            }
            if dark_btn(ui, tl("Copiar FEN"), None, Vec2::new(bw, 38.0), false) {
                ui.ctx().copy_text(fen_of(self.pos_at(idx)));
                self.msg = tl("FEN copiado para a área de transferência.").into();
            }
        });
        // giving up a game against the engine (it ends, is kept and reviewed)
        if matches!(self.mode, Mode::Free) && !self.both && self.loaded.is_none() && !self.finished() && !self.resigned && self.hist.len() >= 2 {
            ui.add_space(8.0);
            if dark_btn(ui, tl("Desistir"), Some(Ico::Flag), Vec2::new(ui.available_width(), 38.0), false) { self.resign(); }
        }
    }

    /// Board editor, or the correction of a screenshot just read.
    fn import_card(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        let mut done = false;
        let editing = self.import.as_ref().map_or(false, |i| i.edit);
        card(ui, |ui| {
            ui.label(med(if editing { tl("Editor") } else { tl("Corrigir posição") }, 17.0));
            if let Some(i) = &mut self.import {
                if !i.edit { changed |= ui.checkbox(&mut i.white_top, tl("Brancas estão em cima no print")).changed(); }
                ui.horizontal(|ui| {
                    ui.label(tl("Vez de:"));
                    changed |= ui.radio_value(&mut i.white_to_move, true, tl("Brancas")).changed();
                    changed |= ui.radio_value(&mut i.white_to_move, false, tl("Pretas")).changed();
                });
            }
            ui.label(egui::RichText::new(tl("Escolhe uma peça (ou a borracha) e clica nas casas do tabuleiro.")).small().color(MUTED));
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
                if ui.add(egui::Button::new(tl("Borracha")).selected(brush == Some(None))).clicked() { brush = if brush == Some(None) { None } else { Some(None) }; }
                if ui.button(tl("Concluir")).clicked() { done = true; }
                if !editing && ui.button(tl("Aprender estas peças")).on_hover_text(tl("Guarda o aspeto das peças deste print (já corrigido) para ler melhor os próximos.")).clicked() { self.learn_import(); }
            });
            self.brush = brush;
            if editing {
                // the FEN of the position: copy it, or type another and press Enter
                ui.add_space(6.0);
                let cur = fen_of(self.pos_at(self.view.unwrap_or(self.hist.len())));
                if !self.fen_editing { self.fen_text = cur; }
                let r = ui.add(egui::TextEdit::singleline(&mut self.fen_text).desired_width(f32::INFINITY).font(egui::FontId::proportional(13.0)).hint_text("FEN"));
                self.fen_editing = r.has_focus();
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) { let f = self.fen_text.clone(); self.load_fen(&f); }
            }
        });
        if changed { self.apply_import(tl("Posição atualizada.")); }
        if done { self.import = None; self.brush = None; }
    }

    /// "A avaliar a partida… 42%" with a bar that fills as the engine goes through the positions.
    fn review_bar(&self, ui: &mut egui::Ui) {
        let (done, total) = self.review_prog;
        let f = if total == 0 { 0.0 } else { done as f32 / total as f32 };
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl("A avaliar a partida…")).size(14.0).color(TEXT2));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(format!("{:.0}%", f * 100.0)).font(f_med(14.0)).color(GOLD));
            });
        });
        let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 8.0), Sense::hover());
        let shown = if motion::on() { ui.ctx().animate_value_with_time(egui::Id::new("review_fill"), f, 0.2) } else { f };
        ui.painter().rect_filled(r, 4.0, Color32::from_rgb(38, 38, 43));
        if shown > 0.0 { ui.painter().rect_filled(Rect::from_min_size(r.min, Vec2::new(r.width() * shown, 8.0)), 4.0, GOLD); }
        ui.label(egui::RichText::new(tr!("Posição {} de {}", done.min(total), total)).size(12.5).color(MUTED));
    }

    fn tab_analysis(&mut self, ui: &mut egui::Ui) {
        if self.review.is_none() {
            card(ui, |ui| {
                if self.reviewing {
                    self.review_bar(ui);
                } else if self.mvs.is_empty() {
                    ui.add(egui::Label::new(egui::RichText::new(tl("A análise aparece aqui no fim da partida, com a precisão e os lances a rever.")).color(MUTED)).wrap());
                } else {
                    ui.add(egui::Label::new(egui::RichText::new(tl("A análise aparece no fim da partida. Também a podes pedir já.")).color(MUTED)).wrap());
                    ui.add_space(6.0);
                    if gold_btn(ui, tl("Avaliar partida"), Some(Ico::Bars), Vec2::new(ui.available_width(), 46.0)) { self.start_review(); }
                }
            });
            return;
        }
        let idx = self.view.unwrap_or(self.hist.len());
        let cc_acc = self.loaded.as_ref().and_then(|i| i.cc_acc);
        let Some(r) = self.review.as_ref() else { return };
        let (acc, series, counts, phase) = (r.acc, r.series.clone(), r.counts, r.phase);
        let marks: Vec<(usize, Class)> = r.rows.iter().enumerate().filter(|(_, w)| matches!(w.class, Class::Brilliant | Class::Great | Class::Mistake | Class::Blunder)).map(|(i, w)| (i + 1, w.class)).collect();
        // where the game turned: the biggest drop of the mover's winning chances
        let turn = (0..r.rows.len()).filter_map(|i| {
            let (a, b) = (*series.get(i)?, *series.get(i + 1)?);
            let lost = if r.rows[i].color == Color::White { a - b } else { b - a };
            (lost > 20.0 && r.rows[i].class >= Class::Mistake).then_some((i, lost))
        }).max_by(|x, y| x.1.total_cmp(&y.1)).map(|(i, _)| (i, r.rows[i].san.clone(), r.rows[i].best_san.clone(), r.rows[i].color));
        let shown_row = idx.checked_sub(1).and_then(|k| r.rows.get(k)).map(|w| (w.class, w.san.clone(), w.best_san.clone(), w.color));
        let rows: Vec<(usize, Color, String, Option<String>, Class)> = r.rows.iter().enumerate().filter(|(_, w)| w.class >= Class::Inaccuracy)
            .map(|(i, w)| (i, w.color, w.san.clone(), w.best_san.clone(), w.class)).collect();

        // result, accuracy
        if let Some(i) = &self.loaded { if !i.result.is_empty() { ui.label(med(&i.result, 15.0).color(POS)); ui.add_space(4.0); } }
        ui.columns(2, |c| {
            egui::Frame::new().fill(Color32::from_rgb(244, 235, 212)).corner_radius(14.0).inner_margin(egui::Margin::symmetric(14, 10)).show(&mut c[0], |ui| {
                ui.set_width(ui.available_width());
                ui.label(semi(format!("{:.1}%", acc[0]), 30.0).color(Color32::from_rgb(20, 17, 10)));
                ui.label(med(tl("Brancas · precisão"), 13.0).color(Color32::from_rgb(58, 49, 32)));
            });
            egui::Frame::new().fill(ROW_ALT).stroke(Stroke::new(1.5, Color32::from_rgb(107, 83, 23))).corner_radius(14.0).inner_margin(egui::Margin::symmetric(14, 10)).show(&mut c[1], |ui| {
                ui.set_width(ui.available_width());
                ui.label(semi(format!("{:.1}%", acc[1]), 30.0).color(GOLD));
                ui.label(med(tl("Pretas · precisão"), 13.0).color(TEXT2));
            });
        });
        if let Some((w, b)) = cc_acc { ui.label(egui::RichText::new(tr!("Precisão no site: brancas {}% · pretas {}%", format!("{w:.1}"), format!("{b:.1}"))).small().color(MUTED)); }
        ui.add_space(8.0);
        // a short word from the coach: who played better and where the game turned
        let mut coach = if (acc[0] - acc[1]).abs() < 3.0 { tl("Partida equilibrada: as duas cores jogaram com precisão parecida.").to_string() }
            else { tr!("{} jogaram com mais precisão ({}% contra {}%).", if acc[0] > acc[1] { tl("As brancas") } else { tl("As pretas") }, format!("{:.0}", acc[0].max(acc[1])), format!("{:.0}", acc[0].min(acc[1]))) };
        if let Some((i, san, best, color)) = &turn {
            coach += " ";
            coach += &tr!("A partida virou no lance {}: {}{}.", i / 2 + 1, if *color == Color::White { "" } else { "..." }, san);
            if let Some(b) = best { coach += " "; coach += &tr!("Melhor era {}.", b); }
        }
        egui::Frame::new().fill(GOLD_CARD).stroke(Stroke::new(1.0, Color32::from_rgb(90, 70, 24))).corner_radius(14.0).inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                let (rc, _) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::hover());
                ico(ui.painter(), Ico::Knight, rc.center(), 24.0, GOLD);
                ui.add(egui::Label::new(egui::RichText::new(coach).size(14.0).color(TEXT2)).wrap());
            });
        });
        ui.add_space(10.0);
        ui.label(med(tl("Avaliação ao longo da partida"), 15.0));
        ui.add_space(4.0);
        let chart_jump = win_chart(ui, &series, Vec2::new(ui.available_width(), 96.0), Some(idx), false, &marks);
        ui.add_space(10.0);
        // estimated strength and each phase of the game
        let grid = |ui: &mut egui::Ui, label: &str, w: String, b: String| {
            let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::hover());
            let p = ui.painter();
            p.text(Pos2::new(row.left(), row.center().y), egui::Align2::LEFT_CENTER, label, f_med(14.0), TEXT2);
            p.text(Pos2::new(row.right() - 64.0, row.center().y), egui::Align2::RIGHT_CENTER, w, f_semi(14.0), CREAM);
            p.text(Pos2::new(row.right() - 6.0, row.center().y), egui::Align2::RIGHT_CENTER, b, f_semi(14.0), GOLD);
        };
        grid(ui, tl("Elo estimado nesta partida"), est_elo(acc[0]).to_string(), est_elo(acc[1]).to_string());
        for (k, name) in [tl("Abertura"), tl("Meio-jogo"), tl("Final")].into_iter().enumerate() {
            let f = |v: Option<f64>| v.map_or("-".to_string(), |v| format!("{v:.0}%"));
            if phase[0][k].is_some() || phase[1][k].is_some() { grid(ui, name, f(phase[0][k]), f(phase[1][k])); }
        }
        ui.add_space(8.0);

        // the move shown
        if let Some((class, san, best, color)) = shown_row {
            let bad = class >= Class::Inaccuracy;
            egui::Frame::new().fill(if bad { Color32::from_rgb(28, 20, 18) } else { ROW_ALT }).stroke(Stroke::new(1.0, if bad { Color32::from_rgb(90, 45, 39) } else { LINE })).corner_radius(14.0).inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (rc, _) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::hover());
                    draw_icon(ui.painter(), rc.center(), 12.0, class);
                    ui.label(semi(format!("{}{} {} · {}", (idx - 1) / 2 + 1, if color == Color::White { "." } else { "..." }, san, class.label()), 17.0));
                });
                let (_, why, _) = explain(self.pos_at(idx - 1), &self.mvs[idx - 1]);
                let text = match best { Some(b) if bad => format!("{why} {}", tr!("Melhor era {}.", b)), _ => why };
                ui.add(egui::Label::new(egui::RichText::new(text).size(14.0).color(TEXT2)).wrap());
            });
            ui.add_space(10.0);
        }

        // how many of each kind
        for c in CLASSES.into_iter().filter(|c| counts[0][*c as usize] + counts[1][*c as usize] > 0) {
            let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::hover());
            let p = ui.painter();
            draw_icon(p, Pos2::new(row.left() + 11.0, row.center().y), 9.5, c);
            p.text(Pos2::new(row.left() + 30.0, row.center().y), egui::Align2::LEFT_CENTER, c.label(), f_med(14.5), c.color());
            p.text(Pos2::new(row.right() - 52.0, row.center().y), egui::Align2::RIGHT_CENTER, counts[0][c as usize].to_string(), f_med(14.5), CREAM);
            p.text(Pos2::new(row.right() - 6.0, row.center().y), egui::Align2::RIGHT_CENTER, counts[1][c as usize].to_string(), f_med(14.5), GOLD);
        }
        ui.label(egui::RichText::new(tl("coluna 1: brancas · coluna 2: pretas")).small().color(MUTED));
        ui.add_space(10.0);

        ui.label(med(tl("Lances a rever"), 15.0));
        ui.add_space(4.0);
        let mut jump = None;
        for (i, color, san, best, class) in &rows {
            let mut t = format!("{}{} {}{}", i / 2 + 1, if *color == Color::White { "." } else { "..." }, san, class.sym());
            if let Some(b) = best { t += &format!("     {}", tr!("melhor: {}", b)); }
            let btn = egui::Button::new(egui::RichText::new(t).size(15.0).color(class.color())).fill(ROW_ALT).stroke(Stroke::new(if idx == i + 1 { 1.5 } else { 1.0 }, if idx == i + 1 { class.color() } else { LINE })).corner_radius(12.0).min_size(Vec2::new(ui.available_width(), 40.0));
            if ui.add(btn).clicked() { jump = Some(i + 1); }
        }
        if rows.is_empty() { ui.label(egui::RichText::new(tl("Sem imprecisões nem erros. Boa partida!")).color(MUTED)); }
        if let Some(j) = jump.or(chart_jump) { self.goto(j); }
    }

    // ---------- Início ----------

    fn home_page(&mut self, ui: &mut egui::Ui) {
        let w = ui.available_width();
        let gap = 20.0;
        let day = today();
        let daily = daily(&self.puzzles, day);
        let daily_done = self.progress.daily.filter(|d| d.0 == day).map(|d| d.1);
        let mut action = None;

        // hero: the puzzle of the day on the banner
        let (hero, _) = ui.allocate_exact_size(Vec2::new(w, 270.0), Sense::hover());
        ui.painter().rect_filled(hero, 24.0, BG);
        self.art_cover(ui, "home_banner", hero, 24, 1.0);
        ui.painter().rect_stroke(hero, 24.0, Stroke::new(1.0, Color32::from_rgb(42, 36, 25)), egui::StrokeKind::Inside);
        let tag = match (daily, daily_done) {
            (Some(i), Some(true)) => tr!("PUZZLE DO DIA · RATING {} · RESOLVIDO", self.puzzles[i].rating),
            (Some(i), _) => tr!("PUZZLE DO DIA · RATING {}", self.puzzles[i].rating),
            (None, _) => tl("PUZZLE DO DIA").to_string(),
        };
        ui.scope_builder(egui::UiBuilder::new().max_rect(Rect::from_min_size(hero.min + Vec2::new(44.0, 52.0), Vec2::new((w * 0.5).min(560.0), 200.0))), |ui| {
            ui.label(egui::RichText::new(tag).font(f_med(13.0)).color(GOLD));
            ui.add_space(6.0);
            ui.label(egui::RichText::new(tl("Encontra o lance que ganha.")).font(egui::FontId::new(40.0, egui::FontFamily::Name("Outfit-SemiBold".into()))).color(CREAM));
            ui.label(egui::RichText::new(tl("Um puzzle novo por dia, escolhido para o teu nível.")).size(16.0).color(TEXT2));
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                if gold_btn(ui, if daily_done.is_some() { tl("Ver outra vez") } else { tl("Resolver agora") }, None, Vec2::new(170.0, 48.0)) { action = Some(1); }
                if dark_btn(ui, tl("Mais puzzles"), None, Vec2::new(150.0, 48.0), false) { action = Some(6); }
            });
        });
        ui.add_space((hero.bottom() + 24.0 - ui.cursor().top()).max(0.0));

        // three ways in
        let cards: [(&'static str, &str, String, u8); 4] = [
            ("card_play", tl("Jogar"), tl("Contra a engine, ao Elo que quiseres, com as melhores jogadas explicadas.").into(), 0),
            ("card_learn", tl("Aprender"), tr!("{} lições curtas de táticas, mates, finais e aberturas.", LESSONS.len()), 2),
            ("game_pairs", tl("Jogos"), tr!("{} jogos curtos de visão, memória e cálculo.", crate::games::GAMES.len()), 7),
            ("card_analyse", tl("Analisar"), tl("Revê as tuas partidas do chess.com, do Lichess ou de um PGN, lance a lance.").into(), 3),
        ];
        let cols = if w > 1040.0 { 4 } else if w > 560.0 { 2 } else { 1 };
        let cw = (w - gap * (cols as f32 - 1.0)) / cols as f32;
        for (ci, chunk) in cards.chunks(cols).enumerate() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (n, (img, title, text, act)) in chunk.iter().enumerate() {
                  ui.scope(|ui| {
                    let (slot, resp) = ui.allocate_exact_size(Vec2::new(cw, 250.0), Sense::click());
                    let hov = motion::hover(ui.ctx(), resp.id, resp.hovered());
                    let (rect, alpha) = motion::card(slot, self.page_age(), ci * cols + n, hov);
                    if alpha < 1.0 || hov > 0.0 && hov < 1.0 { ui.ctx().request_repaint(); }
                    ui.multiply_opacity(alpha);
                    let hot = resp.hovered();
                    ui.painter().rect_filled(rect, 20.0, PANEL);
                    let top = Rect::from_min_size(rect.min, Vec2::new(cw, 150.0));
                    self.art_cover(ui, img, top, 20, 1.0);
                    let p = ui.painter();
                    p.rect_stroke(rect, 20.0, Stroke::new(if hot { 1.5 } else { 1.0 }, if hot { GOLD_LINE } else { LINE }), egui::StrokeKind::Inside);
                    p.text(Pos2::new(rect.left() + 20.0, top.bottom() - 2.0), egui::Align2::LEFT_TOP, *title, f_semi(20.0), CREAM);
                    ico(p, Ico::ArrowR, Pos2::new(rect.right() - 28.0, top.bottom() + 11.0), 20.0, GOLD);
                    let g = p.layout(text.clone(), egui::FontId::proportional(14.0), Color32::from_rgb(184, 178, 161), cw - 40.0);
                    p.galley(Pos2::new(rect.left() + 20.0, top.bottom() + 30.0), g, MUTED);
                    if resp.clicked() { action = Some(*act); }
                  });
                }
            });
            ui.add_space(gap);
        }

        // progress, and the game kept on the board
        let kept = self.kept_game();
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            let pw = if kept.is_some() && w > 760.0 { (w - gap) * 0.6 } else { w };
            egui::Frame::new().fill(PANEL).stroke(Stroke::new(1.0, LINE)).corner_radius(20.0).inner_margin(egui::Margin::symmetric(24, 20)).show(ui, |ui| ui.vertical(|ui| {
                ui.set_width(pw - 50.0);
                ui.label(semi(tl("O teu progresso"), 18.0));
                ui.add_space(10.0);
                let pr = &self.progress;
                let age = self.page_age();
                if motion::on() && age < 0.8 { ui.ctx().request_repaint(); }
                let stats = [(format!("{:.0}", motion::count(age, 0.1, pr.rating as f32)), tl("rating de puzzles"), true), (format!("{:.0}", motion::count(age, 0.15, pr.solved as f32)), tl("puzzles resolvidos"), false), (format!("{:.0}", motion::count(age, 0.2, pr.best_streak as f32)), tl("melhor série"), false), (format!("{}/{}", motion::count(age, 0.25, self.lessons_done() as f32).round(), LESSONS.len()), tl("lições"), false)];
                ui.columns(4, |c| {
                    for (k, (v, l, gold)) in stats.iter().enumerate() {
                        c[k].label(semi(v, 28.0).color(if *gold { GOLD } else { CREAM }));
                        c[k].label(egui::RichText::new(*l).size(13.0).color(MUTED));
                    }
                });
                ui.add_space(10.0);
                let f = self.lessons_done() as f32 / LESSONS.len() as f32;
                let (bar, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 6.0), Sense::hover());
                ui.painter().rect_filled(bar, 3.0, Color32::from_rgb(38, 38, 43));
                let fill = f * motion::enter(self.page_age(), 0.2, 0.5);
                if fill < f { ui.ctx().request_repaint(); }
                if f > 0.0 { ui.painter().rect_filled(Rect::from_min_size(bar.min, Vec2::new(bar.width() * fill, 6.0)), 3.0, GOLD); }
                ui.add_space(6.0);
                let next = (0..LESSONS.len()).find(|&l| !self.lesson_status(l).1);
                let line = match next {
                    Some(l) => tr!("{} de {} lições concluídas · próxima: {}", self.lessons_done(), LESSONS.len(), tl(LESSONS[l].title)),
                    None => tl("Todas as lições concluídas").to_string(),
                };
                ui.label(egui::RichText::new(line).size(13.0).color(MUTED));
            }));
            if let Some((pos, _, flipped, title, sub)) = &kept {
                egui::Frame::new().fill(PANEL).stroke(Stroke::new(1.0, LINE)).corner_radius(20.0).inner_margin(egui::Margin::symmetric(24, 20)).show(ui, |ui| ui.vertical(|ui| {
                    ui.set_width((w - pw - gap - 50.0).max(260.0));
                    ui.label(semi(tl("Continua de onde paraste"), 18.0));
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        let (b, _) = ui.allocate_exact_size(Vec2::splat(72.0), Sense::hover());
                        let p = ui.painter_at(b);
                        let s = 9.0;
                        for i in 0..64u32 {
                            let sq = Square::new(i);
                            let (f, rk) = (u32::from(sq.file()), u32::from(sq.rank()));
                            let (c, r) = if *flipped { (7 - f, rk) } else { (f, 7 - rk) };
                            let cell = Rect::from_min_size(b.min + Vec2::new(c as f32 * s, r as f32 * s), Vec2::splat(s));
                            p.rect_filled(cell, 0.0, if (f + rk) % 2 == 1 { light() } else { dark() });
                            if let Some(pc) = pos.board().piece_at(sq) { p.image(self.tex[&(pc.color, pc.role)].id(), cell, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE); }
                        }
                        ui.vertical(|ui| {
                            ui.set_width((ui.available_width() - 110.0).max(80.0));
                            ui.add(egui::Label::new(med(title.as_str(), 15.5)).wrap());
                            ui.label(egui::RichText::new(sub.as_str()).size(13.0).color(MUTED));
                        });
                        if dark_btn(ui, tl("Continuar"), None, Vec2::new(104.0, 42.0), false) { action = Some(0); }
                    });
                }));
            }
        });
        match action {
            Some(0) => { self.back_to_game(); self.tab = 0; }
            Some(1) => if let Some(i) = daily { self.start_puzzle(i, None, true); self.tab = 3; },
            Some(2) => { self.lesson = None; self.tab = 4; }
            Some(3) => self.tab = 1,
            Some(7) => self.tab = 6,
            Some(6) => { self.tab = 3; if !matches!(&self.mode, Mode::Puzzle(r) if r.lesson.is_none()) { self.next_puzzle(None); } }
            _ => {}
        }
    }

    // ---------- Aprender ----------

    fn lessons_page(&mut self, ui: &mut egui::Ui) {
        let w = ui.available_width();
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(semi(tl("Aprender"), 32.0));
                ui.label(egui::RichText::new(tl("Lê a ideia, vê o exemplo no tabuleiro e pratica com puzzles ou contra a engine.")).size(15.0).color(Color32::from_rgb(184, 178, 161)));
            });
        });
        ui.add_space(8.0);
        let done = self.lessons_done();
        ui.label(egui::RichText::new(tr!("{} de {} lições concluídas", done, LESSONS.len())).size(13.0).color(MUTED));
        let (bar, _) = ui.allocate_exact_size(Vec2::new(w.min(320.0), 6.0), Sense::hover());
        ui.painter().rect_filled(bar, 3.0, Color32::from_rgb(38, 38, 43));
        let fill = motion::enter(self.page_age(), 0.1, 0.5);
        if fill < 1.0 { ui.ctx().request_repaint(); }
        if done > 0 { ui.painter().rect_filled(Rect::from_min_size(bar.min, Vec2::new(bar.width() * done as f32 / LESSONS.len() as f32 * fill, 6.0)), 3.0, GOLD); }
        ui.add_space(20.0);

        let groups: [(&str, &'static str); 4] = [("Táticas", "lesson_tactics"), ("Mates", "lesson_mates"), ("Finais", "lesson_endgames"), ("Aberturas", "lesson_openings")];
        let cols = if w > 1000.0 { 4 } else { 2 };
        let gap = 16.0;
        let cw = (w - gap * (cols as f32 - 1.0)) / cols as f32;
        let mut open = None;
        for (ci, chunk) in groups.chunks(cols).enumerate() {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (n, (group, img)) in chunk.iter().enumerate() {
                  ui.scope(|ui| {
                    let list: Vec<usize> = (0..LESSONS.len()).filter(|&i| LESSONS[i].group == *group).collect();
                    let h = 150.0 + 12.0 + list.len() as f32 * 46.0 + 8.0;
                    let (slot, _) = ui.allocate_exact_size(Vec2::new(cw, h), Sense::hover());
                    let (rect, alpha) = motion::card(slot, self.page_age(), ci * cols + n, 0.0);
                    if alpha < 1.0 { ui.ctx().request_repaint(); }
                    ui.multiply_opacity(alpha);
                    ui.painter().rect_filled(rect, 20.0, PANEL);
                    let top = Rect::from_min_size(rect.min, Vec2::new(cw, 150.0));
                    self.art_cover(ui, img, Rect::from_min_size(top.min + Vec2::new(0.0, 6.0), Vec2::new(cw, 96.0)), 20, 0.5); // above the title
                    let p = ui.painter();
                    p.rect_stroke(rect, 20.0, Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
                    p.text(Pos2::new(rect.left() + 18.0, top.bottom() - 44.0), egui::Align2::LEFT_TOP, tl(group), f_semi(22.0), CREAM);
                    let mut y = top.bottom() + 6.0;
                    for &l in &list {
                        let r = Rect::from_min_size(Pos2::new(rect.left() + 10.0, y), Vec2::new(cw - 20.0, 44.0));
                        y += 46.0;
                        let resp = ui.interact(r, egui::Id::new(("lesson", l)), Sense::click());
                        let p = ui.painter();
                        let hv = motion::hover(ui.ctx(), resp.id, resp.hovered());
                        if hv > 0.0 { p.rect_filled(r, 10.0, Color32::from_rgb(26, 26, 30).gamma_multiply(hv)); if hv < 1.0 { ui.ctx().request_repaint(); } }
                        p.line_segment([Pos2::new(r.left() + 6.0, r.top()), Pos2::new(r.right() - 6.0, r.top())], Stroke::new(1.0, Color32::from_rgb(30, 30, 34)));
                        let (status, finished) = self.lesson_status(l);
                        let started = !finished && status.contains('/');
                        let c = Pos2::new(r.left() + 18.0, r.center().y);
                        if finished { p.circle_filled(c, 8.0, POS); } else { p.circle_stroke(c, 7.0, Stroke::new(2.0, if started { GOLD } else { Color32::from_rgb(74, 74, 82) })); }
                        p.text(Pos2::new(r.left() + 36.0, r.center().y), egui::Align2::LEFT_CENTER, tl(LESSONS[l].title), f_med(15.0), CREAM);
                        // the circle already says "to do": the text only for started (3/5) and finished
                        if started || finished { p.text(Pos2::new(r.right() - 8.0, r.center().y), egui::Align2::RIGHT_CENTER, if finished { "✓".to_string() } else { status }, egui::FontId::proportional(13.0), if finished { POS } else { GOLD }); }
                        if resp.clicked() { open = Some(l); }
                    }
                  });
                }
            });
            ui.add_space(gap);
        }
        if let Some(l) = open { self.open_lesson(l); }
    }

    // ---------- Analisar ----------

    fn analyse_page(&mut self, ui: &mut egui::Ui) {
        let w = ui.available_width();
        ui.label(semi(tl("Analisar partidas"), 32.0));
        ui.label(egui::RichText::new(tl("Abre uma partida terminada: o Gambito avalia cada lance e mostra onde a partida mudou.")).size(15.0).color(Color32::from_rgb(184, 178, 161)));
        ui.add_space(18.0);
        if !self.msg.is_empty() { ui.label(egui::RichText::new(&self.msg).color(AMBER)); ui.add_space(8.0); }
        let gap = 20.0;
        let two = w > 980.0;
        let lw = if two { (w - gap) * 0.62 } else { w };
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.allocate_ui_with_layout(Vec2::new(lw, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(lw);
                self.chesscom_card(ui);
                if !two { ui.add_space(gap); self.pgn_card(ui); ui.add_space(gap); self.my_games_card(ui); }
            });
            if two {
                ui.allocate_ui_with_layout(Vec2::new(w - lw - gap, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(w - lw - gap);
                    self.pgn_card(ui);
                    ui.add_space(gap);
                    self.my_games_card(ui);
                });
            }
        });
    }

    fn chesscom_card(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new().fill(PANEL).stroke(Stroke::new(1.0, LINE)).corner_radius(20.0).inner_margin(egui::Margin::symmetric(20, 18)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(semi(tl("As tuas partidas online"), 18.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // the site: chess.com or Lichess (a segmented switch)
                    let (r, _) = ui.allocate_exact_size(Vec2::new(220.0, 34.0), Sense::hover());
                    let p = ui.painter();
                    p.rect_filled(r, 10.0, BG);
                    p.rect_stroke(r, 10.0, Stroke::new(1.0, LINE2), egui::StrokeKind::Inside);
                    let slide = motion::glide(ui.ctx(), egui::Id::new("cc_site"), self.cc_site as f32, 0.18);
                    let half = (r.width() - 6.0) / 2.0;
                    p.rect_filled(Rect::from_min_size(r.min + Vec2::new(3.0 + slide * half, 3.0), Vec2::new(half, r.height() - 6.0)), 8.0, GOLD_BG);
                    for (k, name) in ["chess.com", "Lichess"].into_iter().enumerate() {
                        let rr = Rect::from_min_size(r.min + Vec2::new(3.0 + k as f32 * half, 3.0), Vec2::new(half, r.height() - 6.0));
                        let resp = ui.interact(rr, egui::Id::new(("cc_site_btn", k)), Sense::click());
                        let on = self.cc_site == k as u8;
                        ui.painter().text(rr.center(), egui::Align2::CENTER_CENTER, name, f_med(14.0), if on { GOLD } else if resp.hovered() { CREAM } else { MUTED });
                        if resp.clicked() && !on && !self.cc_busy {
                            self.cc_site = k as u8;
                            self.cc_games.clear();
                            self.cc_archives.clear();
                            self.cc_loaded = 0;
                            self.cc_query.clear();
                            self.cc_err.clear();
                        }
                    }
                });
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                egui::Frame::new().fill(BG).stroke(Stroke::new(1.0, LINE2)).corner_radius(12.0).inner_margin(egui::Margin::symmetric(12, 6)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
                        ico(ui.painter(), Ico::Search, r.center(), 20.0, MUTED);
                        let te = ui.add(egui::TextEdit::singleline(&mut self.cc_user).frame(egui::Frame::NONE).hint_text(tl("utilizador")).desired_width((ui.available_width() - 140.0).max(80.0)).font(egui::FontId::proportional(15.0)));
                        if te.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !self.cc_busy { self.cc_search(false); }
                    });
                });
                if gold_btn(ui, tl("Procurar"), None, Vec2::new(116.0, 44.0)) && !self.cc_busy { self.cc_search(false); }
            });
            ui.label(egui::RichText::new(tl("Só o nome de utilizador sai do teu computador, e só quando carregas em Procurar.")).size(12.5).color(MUTED));
            if self.cc_busy {
                ui.label(egui::RichText::new(tl("A buscar partidas…")).color(MUTED));
                ui.add_space(6.0);
                for _ in 0..5 {
                    let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 56.0), Sense::hover());
                    motion::skeleton_row(ui.painter(), row, self.time);
                }
                ui.ctx().request_repaint();
            }
            if !self.cc_err.is_empty() { ui.label(egui::RichText::new(&self.cc_err).color(AMBER)); }
            ui.add_space(8.0);
            let user = self.cc_query.clone();
            let mut pick = None;
            for (i, g) in self.cc_games.iter().enumerate().take(300) {
                let white = g.white.eq_ignore_ascii_case(&user);
                let (my_res, opp, opp_elo) = if white { (&g.white_result, &g.black, g.black_elo) } else { (&g.black_result, &g.white, g.white_elo) };
                let won = my_res == "win";
                let drew = g.white_result != "win" && g.black_result != "win";
                let (tag, bg, fg) = if won { (tl("V"), Color32::from_rgb(43, 107, 63), Color32::from_rgb(234, 247, 238)) } else if drew { (tl("E"), Color32::from_rgb(58, 58, 64), CREAM) } else { (tl("D"), Color32::from_rgb(107, 42, 35), Color32::from_rgb(251, 233, 230)) };
                let (row, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 56.0), Sense::click());
                let p = ui.painter();
                if resp.hovered() { p.rect_filled(row, 10.0, Color32::from_rgb(24, 24, 28)); }
                p.line_segment([row.left_top(), row.right_top()], Stroke::new(1.0, Color32::from_rgb(28, 28, 32)));
                // the result at a glance: a coloured bar, then the letter
                p.rect_filled(Rect::from_min_size(Pos2::new(row.left() + 2.0, row.top() + 10.0), Vec2::new(4.0, row.height() - 20.0)), 2.0, bg.lerp_to_gamma(fg, 0.25));
                let badge = Rect::from_min_size(Pos2::new(row.left() + 14.0, row.center().y - 14.0), Vec2::splat(28.0));
                p.rect_filled(badge, 8.0, bg);
                p.text(badge.center(), egui::Align2::CENTER_CENTER, tag, f_semi(14.0), fg);
                let elo = opp_elo.map_or("?".to_string(), |e| e.to_string());
                p.text(Pos2::new(row.left() + 56.0, row.center().y - 10.0), egui::Align2::LEFT_CENTER, format!("{opp} ({elo})"), f_med(15.0), CREAM);
                let side = if white { tl("com as brancas") } else { tl("com as pretas") };
                let mut line2 = format!("{side} · {} · {}", g.time_class, chesscom::date(g.end_time));
                if g.moves > 0 { line2 += &format!(" · {}", tr!("{} lances", g.moves)); }
                if !g.opening.is_empty() { line2 += &format!(" · {}", g.opening); }
                let room = row.width() - 56.0 - 160.0;
                let mut job = egui::text::LayoutJob::simple_singleline(line2, egui::FontId::proportional(12.5), MUTED);
                job.wrap = egui::text::TextWrapping { max_width: room, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
                let gal = ui.fonts_mut(|f| f.layout_job(job));
                let p = ui.painter();
                p.galley(Pos2::new(row.left() + 56.0, row.center().y + 3.0), gal, MUTED);
                // my accuracy, when the site has it (as on chess.com / Chessigma)
                if let Some((wa, ba)) = g.acc {
                    let a = if white { wa } else { ba };
                    let pill = Rect::from_min_size(Pos2::new(row.right() - 152.0, row.center().y - 13.0), Vec2::new(56.0, 26.0));
                    p.rect_filled(pill, 8.0, Color32::from_rgb(30, 30, 35));
                    p.text(pill.center(), egui::Align2::CENTER_CENTER, format!("{a:.1}"), f_semi(13.5), if a >= 85.0 { POS } else if a >= 70.0 { CREAM } else { AMBER });
                }
                let open = Rect::from_min_size(Pos2::new(row.right() - 86.0, row.center().y - 17.0), Vec2::new(80.0, 34.0));
                p.rect_stroke(open, 10.0, Stroke::new(1.5, GOLD_LINE), egui::StrokeKind::Inside);
                if resp.hovered() { p.rect_filled(open, 10.0, GOLD_BG); }
                p.text(open.center(), egui::Align2::CENTER_CENTER, tl("Abrir"), f_med(14.0), GOLD);
                if resp.clicked() { pick = Some(i); }
            }
            if self.cc_games.is_empty() && !self.cc_busy {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(Vec2::new(150.0, 100.0), Sense::hover());
                    self.art_cover(ui, "empty_state", r, 12, 1.0);
                    ui.add(egui::Label::new(egui::RichText::new(if self.cc_site == 1 { tl("Escreve o teu nome de utilizador do Lichess para veres as tuas partidas.") } else { tl("Escreve o teu nome de utilizador do chess.com para veres as tuas partidas.") }).size(14.0).color(MUTED)).wrap());
                });
            }
            if let Some(i) = pick { self.load_cc_game(i); }
            if !self.cc_games.is_empty() && self.cc_loaded < self.cc_archives.len() && !self.cc_busy {
                ui.add_space(8.0);
                if dark_btn(ui, tl("Carregar mais antigas"), None, Vec2::new(200.0, 40.0), false) { self.cc_search(true); }
            }
        });
    }

    /// The games played in Gambito against the engine, newest first.
    fn my_games_card(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new().fill(PANEL).stroke(Stroke::new(1.0, LINE)).corner_radius(20.0).inner_margin(egui::Margin::symmetric(20, 18)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(semi(tl("Partidas jogadas no Gambito"), 18.0));
            if self.my_games.is_empty() {
                ui.label(egui::RichText::new(tl("As partidas que acabares contra a engine ficam aqui, para as reveres.")).size(13.0).color(MUTED));
                return;
            }
            ui.add_space(6.0);
            let mut pick = None;
            for (i, g) in self.my_games.iter().enumerate().rev().take(50) {
                let (row, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 46.0), Sense::click());
                let p = ui.painter();
                if resp.hovered() { p.rect_filled(row, 10.0, Color32::from_rgb(24, 24, 28)); }
                p.line_segment([row.left_top(), row.right_top()], Stroke::new(1.0, Color32::from_rgb(28, 28, 32)));
                let (w, b) = (g.header("White").unwrap_or("?"), g.header("Black").unwrap_or("?"));
                p.text(Pos2::new(row.left() + 6.0, row.center().y - 8.0), egui::Align2::LEFT_CENTER, format!("{w} – {b}"), f_med(14.5), CREAM);
                let res = g.header("Result").unwrap_or("*");
                p.text(Pos2::new(row.left() + 6.0, row.center().y + 10.0), egui::Align2::LEFT_CENTER,
                    format!("{res} · {} · {}", tr!("{} lances", g.moves.len().div_ceil(2)), g.header("Date").unwrap_or("")), egui::FontId::proportional(12.0), MUTED);
                let open = Rect::from_min_size(Pos2::new(row.right() - 76.0, row.center().y - 15.0), Vec2::new(70.0, 30.0));
                p.rect_stroke(open, 10.0, Stroke::new(1.5, GOLD_LINE), egui::StrokeKind::Inside);
                if resp.hovered() { p.rect_filled(open, 10.0, GOLD_BG); }
                p.text(open.center(), egui::Align2::CENTER_CENTER, tl("Abrir"), f_med(13.5), GOLD);
                if resp.clicked() { pick = Some(i); }
            }
            if let Some(i) = pick { let g = self.my_games[i].clone(); self.load_pgn_game(&g); }
        });
    }

    fn pgn_card(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new().fill(PANEL).stroke(Stroke::new(1.0, LINE)).corner_radius(20.0).inner_margin(egui::Margin::symmetric(20, 18)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(semi(tl("PGN de qualquer site"), 18.0));
            ui.label(egui::RichText::new(tl("Cola o texto, arrasta um ficheiro .pgn para a janela, ou abre-o.")).size(13.0).color(MUTED));
            ui.add_space(8.0);
            ui.add(egui::TextEdit::multiline(&mut self.pgn_text).hint_text(tl("Cola aqui o PGN (de qualquer site)…")).desired_rows(8).desired_width(f32::INFINITY));
            ui.add_space(10.0);
            let w = (ui.available_width() - 10.0) / 2.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                if gold_btn(ui, tl("Abrir partida"), None, Vec2::new(w, 46.0)) {
                    let t = self.pgn_text.clone();
                    if !self.try_pgn(&t) { self.msg = tl("Não encontrei nenhuma partida neste texto.").into(); }
                }
                if dark_btn(ui, tl("Abrir ficheiro…"), None, Vec2::new(w, 46.0), false) {
                    if let Some(p) = rfd::FileDialog::new().add_filter("PGN", &["pgn", "txt"]).pick_file() { self.open_file(&p); }
                }
            });
            ui.add_space(6.0);
            ui.label(egui::RichText::new(tl("Também aceita vários jogos por ficheiro, comentários, variantes e posições iniciais (FEN).")).size(12.5).color(MUTED));
            let mut pick = None;
            for (i, g) in self.pgn_games.iter().enumerate().take(100) {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(format!("{} vs {}  {}", g.header("White").unwrap_or("?"), g.header("Black").unwrap_or("?"), g.header("Result").unwrap_or("")));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if dark_btn(ui, tl("Abrir"), None, Vec2::new(70.0, 32.0), true) { pick = Some(i); }
                    });
                });
            }
            if let Some(i) = pick { let g = self.pgn_games[i].clone(); self.load_pgn_game(&g); self.game_idx = Some((1, i)); }
        });
    }

    // ---------- Definições ----------

    fn settings_page(&mut self, ui: &mut egui::Ui) {
        // the column is at most 780 px wide and sits in the middle, not against the left edge of a wide window
        let avail = ui.available_width();
        let w0 = avail.min(780.0);
        let pad = ((avail - w0) / 2.0).max(0.0);
        ui.horizontal_top(|ui| {
            ui.add_space(pad);
            ui.vertical(|ui| { ui.set_max_width(w0); ui.set_min_width(w0); self.settings_body(ui); });
        });
    }

    fn settings_body(&mut self, ui: &mut egui::Ui) {
        ui.label(semi(tl("Definições"), 32.0));
        ui.add_space(16.0);
        let w = ui.available_width().min(780.0);
        let section = |ui: &mut egui::Ui, title: &str, add: &mut dyn FnMut(&mut egui::Ui)| {
            egui::Frame::new().fill(PANEL).stroke(Stroke::new(1.0, LINE)).corner_radius(20.0).inner_margin(egui::Margin::symmetric(22, 16)).show(ui, |ui| {
                ui.set_width(w - 44.0);
                ui.label(semi(title, 18.0));
                ui.add_space(8.0);
                add(ui);
            });
            ui.add_space(14.0);
        };
        let row = |ui: &mut egui::Ui, label: &str, help: &str, add: &mut dyn FnMut(&mut egui::Ui)| {
            ui.horizontal(|ui| {
                ui.set_min_height(52.0);
                ui.vertical(|ui| {
                    ui.label(med(label, 15.5));
                    if !help.is_empty() { ui.label(egui::RichText::new(help).size(13.0).color(MUTED)); }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| add(ui));
            });
        };
        let names: Vec<String> = self.engines.iter().map(|e| e.name.clone()).collect();
        let mut lang = i18n::lang();
        let (mut new_opp, mut new_ana) = (self.opp, self.ana);
        let (mut both, mut hints_on, mut elo) = (self.both, self.hints_on, self.elo);
        let mut add_engine = false;
        let mut check_copied = false;
        section(ui, tl("Geral"), &mut |ui| {
            row(ui, tl("Língua"), tl("Na primeira vez, segue a do Windows."), &mut |ui| {
                egui::ComboBox::from_id_salt("lang").width(220.0).selected_text(i18n::LANGS[lang].1).show_ui(ui, |ui| {
                    for (i, (_, name)) in i18n::LANGS.iter().enumerate() { ui.selectable_value(&mut lang, i, *name); }
                });
            });
        });
        let mut anim = motion::on();
        let mut snd = crate::sound::on();
        let mut th = crate::theme();
        section(ui, tl("Tabuleiro e som"), &mut |ui| {
            row(ui, tl("Cores do tabuleiro"), "", &mut |ui| {
                for k in (0..THEMES.len()).rev() {
                    let (r, resp) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::click());
                    let p = ui.painter();
                    let h = r.width() / 2.0;
                    for (i, j) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let c = if (i + j) % 2 == 0 { THEMES[k].1 } else { THEMES[k].2 };
                        p.rect_filled(Rect::from_min_size(r.min + Vec2::new(i as f32 * h, j as f32 * h), Vec2::splat(h)), 0.0, c);
                    }
                    p.rect_stroke(r, 6.0, Stroke::new(if th == k { 3.0 } else { 1.0 }, if th == k { GOLD } else if resp.hovered() { CREAM } else { LINE }), egui::StrokeKind::Outside);
                    if resp.on_hover_text(tl(THEMES[k].0)).clicked() { th = k; }
                    ui.add_space(6.0);
                }
            });
            row(ui, tl("Animações"), tl("Movimento suave nos ecrãs, botões e peças. Desliga para reduzir o movimento."), &mut |ui| { toggle(ui, &mut anim); });
            row(ui, tl("Sons"), tl("Lances, capturas, xeques, fim da partida e respostas certas ou erradas."), &mut |ui| {
                toggle(ui, &mut snd);
                if ui.add(egui::Button::new(egui::RichText::new(tl("Ouvir")).size(13.0)).corner_radius(8.0)).clicked() && snd { crate::sound::play(crate::sound::Snd::Capture); }
            });
        });
        if anim != motion::on() { motion::set_on(anim); i18n::save_setting("motion", serde_json::json!(anim)); }
        if snd != crate::sound::on() { crate::sound::set_on(snd); i18n::save_setting("sound", serde_json::json!(snd)); if snd { crate::sound::play(crate::sound::Snd::Move); } }
        if th != crate::theme() { crate::THEME.store(th, std::sync::atomic::Ordering::Relaxed); i18n::save_setting("theme", serde_json::json!(th)); }
        section(ui, tl("Adversário"), &mut |ui| {
            row(ui, tl("Engine"), "", &mut |ui| {
                egui::ComboBox::from_id_salt("opp").width(220.0).selected_text(names[new_opp].as_str()).show_ui(ui, |ui| {
                    for (i, n) in names.iter().enumerate() { ui.selectable_value(&mut new_opp, i, n.as_str()); }
                });
            });
            row(ui, tl("Força (Elo)"), tl("De 1000 a 2600."), &mut |ui| {
                ui.label(med(elo.to_string(), 15.0));
                ui.scope(|ui| {
                    ui.spacing_mut().slider_width = 200.0;
                    ui.visuals_mut().selection.bg_fill = GOLD;
                    ui.add_enabled(!both, egui::Slider::new(&mut elo, 1000..=2600).step_by(50.0).show_value(false).trailing_fill(true));
                });
            });
            row(ui, tl("Jogar os dois lados"), tl("Sem engine: tu moves as brancas e as pretas."), &mut |ui| { toggle(ui, &mut both); });
        });
        section(ui, tl("Sugestões"), &mut |ui| {
            row(ui, tl("Mostrar as 3 melhores jogadas"), "", &mut |ui| { toggle(ui, &mut hints_on); });
            row(ui, tl("Engine das sugestões e da análise"), tl("A força ajusta-se no próprio painel de sugestões."), &mut |ui| {
                egui::ComboBox::from_id_salt("ana").width(220.0).selected_text(names[new_ana].as_str()).show_ui(ui, |ui| {
                    for (i, n) in names.iter().enumerate() { ui.selectable_value(&mut new_ana, i, n.as_str()); }
                });
            });
        });
        let missing: Vec<&'static str> = engine::known().iter().filter(|k| !engine::path(k).exists()).map(|k| k.name).collect();
        section(ui, tl("Engines"), &mut |ui| {
            ui.horizontal_wrapped(|ui| {
                for n in &names {
                    egui::Frame::new().fill(ROW_ALT).stroke(Stroke::new(1.0, LINE)).corner_radius(10.0).inner_margin(egui::Margin::symmetric(12, 6)).show(ui, |ui| { ui.label(med(n.as_str(), 14.0)); });
                }
            });
            if !missing.is_empty() {
                ui.add_space(6.0);
                ui.add(egui::Label::new(egui::RichText::new(tr!("Não instaladas: {}. Volta a correr o instalador para as juntar.", missing.join(", "))).size(13.0).color(MUTED)).wrap());
            }
            ui.add_space(10.0);
            if dark_btn(ui, tl("Adicionar engine UCI…"), None, Vec2::new(220.0, 42.0), false) { add_engine = true; }
        });
        section(ui, tl("Sobre o Gambito"), &mut |ui| {
            row(ui, &tr!("Versão {}", update::VERSION), tl("Gratuito, sem anúncios, sem conta e de código aberto (GPL-3.0)."), &mut |ui| {
                if dark_btn(ui, tl("Copiar versão"), None, Vec2::new(150.0, 40.0), false) {
                    ui.ctx().copy_text(format!("Gambito {}", update::VERSION));
                    check_copied = true;
                }
            });
        });
        let mut check_now = false;
        if update::available() {
            let mut auto = update::auto();
            let st = self.upd();
            section(ui, tl("Atualizações"), &mut |ui| {
                row(ui, tl("Procurar atualizações automaticamente"), tl("Uma vez por dia, no GitHub. Só é enviado o pedido da versão mais recente."), &mut |ui| { toggle(ui, &mut auto); });
                row(ui, &tr!("Versão {}", update::VERSION), match &st {
                    Upd::Checking => tl("A procurar…"),
                    Upd::Latest => tl("Tens a versão mais recente."),
                    Upd::Found(_) | Upd::Getting(_) | Upd::Restarting => tl("Há uma versão nova: usa o botão Atualizar no menu."),
                    Upd::Failed(_) => tl("Não consegui ver se há versão nova."),
                    Upd::Idle => "",
                }, &mut |ui| { if dark_btn(ui, tl("Procurar agora"), None, Vec2::new(170.0, 40.0), false) { check_now = true; } });
            });
            if auto != update::auto() { update::set_auto(auto); }
        }
        if check_copied { self.notify(0, tl("Versão copiada.")); }
        if check_now && !matches!(self.upd(), Upd::Checking | Upd::Getting(_) | Upd::Restarting) { self.check_update(true); }
        if lang != i18n::lang() { i18n::set(lang); self.msg.clear(); self.request_hints(); }
        if elo != self.elo { self.elo = elo; i18n::save_setting("elo", serde_json::json!(elo)); }
        if both != self.both { self.both = both; self.thinking = false; self.after_move(); }
        if hints_on != self.hints_on { self.hints_on = hints_on; i18n::save_setting("hints_on", serde_json::json!(hints_on)); self.request_hints(); }
        if new_opp != self.opp {
            self.opp = new_opp;
            self.player = self.spawn(new_opp);
            self.msg.clear();
            if self.thinking { self.after_move(); }
        }
        if new_ana != self.ana {
            self.ana = new_ana;
            self.analyst = self.spawn_heavy(new_ana);
            self.reviewer = self.spawn_heavy(new_ana);
            self.msg.clear();
            self.request_hints();
        }
        if add_engine {
            if let Some(p) = rfd::FileDialog::new().add_filter("engine", &["exe"]).pick_file() { self.add_engine(p); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_number_round_trips() {
        assert!((cp_of_win(win_pct(150.0) as f32) - 150.0).abs() < 1.0);
    }
}
