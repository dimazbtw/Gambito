//! Motion rules, in one place. They come from published animation standards (Emil Kowalski's design-engineering
//! notes, Material motion, Apple's HIG, and what chess.com does with 0.1-0.25 s tokens):
//!  - nothing animates forever: every animation is short (mostly under 300 ms) and ends, so an idle window draws nothing;
//!  - entrances ease out (they start fast, so the screen feels responsive); nothing grows from zero size;
//!  - what is done hundreds of times (moving pieces with the keyboard, legal-move dots) is not decorated;
//!  - celebrations are for rare events (a record, a win);
//!  - "Show animations in Windows" off, or Settings -> Animations off, turns it all down to instant changes.
use eframe::egui::{self, Rect, Vec2};
use std::sync::atomic::{AtomicBool, Ordering};

static ON: AtomicBool = AtomicBool::new(true);

pub fn on() -> bool { ON.load(Ordering::Relaxed) }
pub fn set_on(v: bool) { ON.store(v, Ordering::Relaxed); }

/// The Windows setting "Show animations in Windows" (Accessibility > Visual effects).
#[cfg(windows)]
pub fn system_allows() -> bool {
    #[link(name = "user32")]
    unsafe extern "system" { fn SystemParametersInfoW(action: u32, param: u32, value: *mut i32, winini: u32) -> i32; }
    const SPI_GETCLIENTAREAANIMATION: u32 = 0x1042;
    let mut v: i32 = 1;
    // SAFETY: the call writes one BOOL into `v`
    let ok = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, &mut v, 0) };
    ok == 0 || v != 0
}
#[cfg(not(windows))]
pub fn system_allows() -> bool { true }

/// A strong ease-out (like cubic-bezier(0.23, 1, 0.32, 1)): fast start, long settle.
pub fn ease_out(t: f32) -> f32 { 1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(4) }

/// Ease-out with a small overshoot, for things that "land" (a record, a counter that just changed).
pub fn ease_out_back(t: f32) -> f32 {
    let (c1, c3) = (1.70158f32, 2.70158f32);
    let t = t.clamp(0.0, 1.0) - 1.0;
    1.0 + c3 * t.powi(3) + c1 * t.powi(2)
}

/// Progress (0..1) of an entrance that starts `delay` seconds after the screen appeared and lasts `dur`.
/// Instant (1) when animations are off.
pub fn enter(age: f64, delay: f64, dur: f64) -> f32 {
    if !on() { return 1.0; }
    ease_out(((age - delay) / dur) as f32)
}

/// Seconds until an entrance with this stagger index has finished (to keep repainting only while it runs).
pub fn enter_end(index: usize, dur: f64) -> f64 { index as f64 * STAGGER + dur }

/// Delay between the items of a group (30-80 ms feels right; longer feels slow).
pub const STAGGER: f64 = 0.05;
pub const ENTER_SECS: f64 = 0.28;

/// A pressed control shrinks a little (scale 0.97 in 120 ms), like a real button. Returns the factor to draw with.
pub fn press(ctx: &egui::Context, resp: &egui::Response) -> f32 {
    if !on() { return 1.0; }
    let t = ctx.animate_bool_with_time_and_easing(resp.id.with("press"), resp.is_pointer_button_down_on(), 0.12, egui::emath::easing::quadratic_out);
    1.0 - 0.03 * t
}

/// Hover progress (0..1, 150 ms), for the lift of a card and the colour of its border.
pub fn hover(ctx: &egui::Context, id: egui::Id, hovered: bool) -> f32 {
    if !on() { return if hovered { 1.0 } else { 0.0 }; }
    ctx.animate_bool_with_time_and_easing(id.with("hover"), hovered, 0.15, egui::emath::easing::quadratic_out)
}

/// Where a card is drawn: its slot, lifted when hovered and risen into place during its entrance.
/// Returns (rect, opacity).
pub fn card(slot: Rect, age: f64, index: usize, hover: f32) -> (Rect, f32) {
    let k = enter(age, index as f64 * STAGGER, ENTER_SECS);
    (slot.translate(Vec2::new(0.0, (1.0 - k) * 12.0 - 3.0 * hover)), k)
}

/// A number that counts up when its screen appears (ease-out, `delay` after it).
pub fn count(age: f64, delay: f64, target: f32) -> f32 { target * enter(age, delay, 0.5) }

/// Confetti over `area`: `age` seconds since the event, fades out at 2.5 s. Deterministic (no randomness to store).
pub fn confetti(p: &egui::Painter, area: Rect, age: f32, colors: &[egui::Color32]) {
    if !on() || age >= 2.5 { return; }
    let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = || { seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17; (seed % 1000) as f32 / 1000.0 };
    for _ in 0..70 {
        let x0 = area.left() + next() * area.width();
        let v = 120.0 + next() * 260.0;
        let phase = next() * 100.0;
        let col = colors[(next() * colors.len() as f32) as usize % colors.len()];
        let y = area.top() - 20.0 + v * age + 140.0 * age * age;
        let x = x0 + (age * 3.0 + phase).sin() * 18.0;
        if y < area.bottom() {
            p.rect_filled(Rect::from_center_size(egui::Pos2::new(x, y), Vec2::new(7.0, 4.0)), 1.0, col.gamma_multiply((1.0 - age / 2.5).max(0.0)));
        }
    }
}

/// A placeholder row while something loads (a pulse, not a spinner): a badge and two bars.
pub fn skeleton_row(p: &egui::Painter, row: Rect, time: f64) {
    let pulse = if on() { 0.5 + 0.5 * (time * 4.2).sin() as f32 } else { 0.5 };
    let c = egui::Color32::from_rgb(34, 34, 39).lerp_to_gamma(egui::Color32::from_rgb(52, 52, 58), pulse);
    p.rect_filled(Rect::from_min_size(egui::Pos2::new(row.left() + 14.0, row.center().y - 14.0), Vec2::splat(28.0)), 8.0, c);
    p.rect_filled(Rect::from_min_size(egui::Pos2::new(row.left() + 56.0, row.center().y - 12.0), Vec2::new(row.width() * 0.34, 10.0)), 5.0, c);
    p.rect_filled(Rect::from_min_size(egui::Pos2::new(row.left() + 56.0, row.center().y + 5.0), Vec2::new(row.width() * 0.52, 8.0)), 4.0, c);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_start_at_0_and_end_at_1() {
        for f in [ease_out, ease_out_back] { assert!(f(0.0).abs() < 1e-6 && (f(1.0) - 1.0).abs() < 1e-6); }
        assert!(ease_out(0.2) > 0.5, "an ease-out is already well along at 20%");
        assert!(ease_out_back(0.6) > 1.0, "the back curve overshoots");
    }

    #[test]
    fn entrances_stagger_and_finish() {
        set_on(true);
        assert_eq!(enter(0.0, 0.1, 0.28), 0.0);
        assert!(enter(0.2, 0.1, 0.28) > 0.0 && enter(0.2, 0.1, 0.28) < 1.0);
        assert_eq!(enter(1.0, 0.1, 0.28), 1.0);
        assert!(enter_end(3, ENTER_SECS) < 0.5, "a four-card group is done in under half a second");
        assert_eq!(count(5.0, 0.0, 1197.0), 1197.0);
    }
}
