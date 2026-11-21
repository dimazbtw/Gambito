//! Board sounds, like a real set of pieces: made when first needed (no sound files) and played with
//! Windows' own PlaySound, so there is no audio library. A wooden "tock" for a move, a sharper double knock for
//! a capture, a small bell for check, a chime when the game ends, and right/wrong cues for puzzles and games.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

static ON: AtomicBool = AtomicBool::new(true);

pub fn on() -> bool { ON.load(Ordering::Relaxed) }
pub fn set_on(v: bool) { ON.store(v, Ordering::Relaxed); }

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Snd { Move, Capture, Check, Castle, Promote, End, Right, Wrong, Notify }
const ALL: [Snd; 9] = [Snd::Move, Snd::Capture, Snd::Check, Snd::Castle, Snd::Promote, Snd::End, Snd::Right, Snd::Wrong, Snd::Notify];

const RATE: u32 = 22_050;

/// A knock of wood: a low tone and a burst of noise, both dying out fast, softened by a low-pass.
fn tock(out: &mut [f32], at: f32, gain: f32, pitch: f32, seed: &mut u32) {
    let start = (at * RATE as f32) as usize;
    let mut lp = 0.0f32;
    for i in 0..(0.09 * RATE as f32) as usize {
        let Some(s) = out.get_mut(start + i) else { break };
        let t = i as f32 / RATE as f32;
        *seed ^= *seed << 13; *seed ^= *seed >> 17; *seed ^= *seed << 5;
        let noise = (*seed as f32 / u32::MAX as f32) * 2.0 - 1.0;
        let x = 0.7 * (std::f32::consts::TAU * pitch * t).sin() * (-t / 0.016).exp() + 0.55 * noise * (-t / 0.0035).exp();
        lp += 0.45 * (x - lp);
        *s += gain * lp;
    }
}

/// A small bell: a sine with a quieter octave, ringing out over `len` seconds.
fn bell(out: &mut [f32], at: f32, gain: f32, freq: f32, len: f32) {
    let start = (at * RATE as f32) as usize;
    for i in 0..(len * RATE as f32) as usize {
        let Some(s) = out.get_mut(start + i) else { break };
        let t = i as f32 / RATE as f32;
        let attack = (t / 0.004).min(1.0);
        let w = std::f32::consts::TAU * freq * t;
        *s += gain * attack * (w.sin() + 0.3 * (2.0 * w).sin()) * (-t / (len / 3.5)).exp();
    }
}

fn synth(s: Snd) -> Vec<f32> {
    let mut o = vec![0.0f32; (0.9 * RATE as f32) as usize];
    let mut seed = 0x9e37_79b9u32;
    match s {
        Snd::Move => tock(&mut o, 0.0, 0.9, 190.0, &mut seed),
        Snd::Capture => { tock(&mut o, 0.0, 1.0, 230.0, &mut seed); tock(&mut o, 0.028, 0.8, 160.0, &mut seed); }
        Snd::Castle => { tock(&mut o, 0.0, 0.85, 190.0, &mut seed); tock(&mut o, 0.11, 0.85, 170.0, &mut seed); }
        Snd::Check => { tock(&mut o, 0.0, 0.9, 200.0, &mut seed); bell(&mut o, 0.02, 0.22, 1318.5, 0.25); }
        Snd::Promote => { tock(&mut o, 0.0, 0.9, 190.0, &mut seed); bell(&mut o, 0.03, 0.18, 880.0, 0.18); bell(&mut o, 0.11, 0.18, 1318.5, 0.25); }
        Snd::End => { for (k, f) in [523.25, 659.25, 783.99].into_iter().enumerate() { bell(&mut o, k as f32 * 0.11, 0.2, f, 0.6); } }
        Snd::Right => { bell(&mut o, 0.0, 0.22, 659.25, 0.22); bell(&mut o, 0.08, 0.22, 987.77, 0.3); }
        Snd::Wrong => {
            for i in 0..(0.2 * RATE as f32) as usize {
                let t = i as f32 / RATE as f32;
                let f = 210.0 - 60.0 * t / 0.2;
                o[i] += 0.28 * ((std::f32::consts::TAU * f * t).sin().signum() * 0.35 + (std::f32::consts::TAU * f * t).sin() * 0.65) * (-t / 0.09).exp();
            }
        }
        Snd::Notify => bell(&mut o, 0.0, 0.18, 880.0, 0.35),
    }
    // trim the silence at the end
    let last = o.iter().rposition(|x| x.abs() > 1e-3).unwrap_or(0) + 1;
    o.truncate(last);
    o
}

/// 16-bit mono WAV in memory.
fn wav(samples: &[f32]) -> Vec<u8> {
    let data: Vec<u8> = samples.iter().flat_map(|x| ((x.clamp(-1.0, 1.0) * 30_000.0) as i16).to_le_bytes()).collect();
    let mut w = Vec::with_capacity(44 + data.len());
    w.extend(b"RIFF"); w.extend((36 + data.len() as u32).to_le_bytes()); w.extend(b"WAVEfmt ");
    w.extend(16u32.to_le_bytes()); w.extend(1u16.to_le_bytes()); w.extend(1u16.to_le_bytes());
    w.extend(RATE.to_le_bytes()); w.extend((RATE * 2).to_le_bytes()); w.extend(2u16.to_le_bytes()); w.extend(16u16.to_le_bytes());
    w.extend(b"data"); w.extend((data.len() as u32).to_le_bytes()); w.extend(data);
    w
}

fn bank() -> &'static Vec<Vec<u8>> {
    static BANK: OnceLock<Vec<Vec<u8>>> = OnceLock::new();
    BANK.get_or_init(|| ALL.iter().map(|&s| wav(&synth(s))).collect())
}

#[cfg(windows)]
#[link(name = "winmm")]
unsafe extern "system" {
    fn PlaySoundW(sound: *const u16, module: *mut std::ffi::c_void, flags: u32) -> i32;
}

/// Plays a sound (a new one cuts the one playing). Silent when sounds are off.
pub fn play(s: Snd) {
    if !on() { return; }
    let bytes = &bank()[ALL.iter().position(|&x| x == s).unwrap()];
    #[cfg(windows)]
    unsafe {
        const SND_ASYNC: u32 = 0x1;
        const SND_NODEFAULT: u32 = 0x2;
        const SND_MEMORY: u32 = 0x4;
        // the buffer lives in a static, so it stays valid while Windows plays it
        PlaySoundW(bytes.as_ptr() as *const u16, std::ptr::null_mut(), SND_MEMORY | SND_ASYNC | SND_NODEFAULT);
    }
    #[cfg(not(windows))]
    let _ = bytes;
}

/// The sound of a move, as chess.com picks it.
pub fn for_move(before: &shakmaty::Chess, m: &shakmaty::Move) -> Snd {
    use shakmaty::Position;
    let after = before.clone().play(m.clone()).ok();
    if after.as_ref().is_some_and(|a| a.is_game_over()) { Snd::End }
    else if after.as_ref().is_some_and(|a| a.is_check()) { Snd::Check }
    else if m.is_promotion() { Snd::Promote }
    else if m.is_castle() { Snd::Castle }
    else if m.is_capture() { Snd::Capture }
    else { Snd::Move }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sound_is_a_short_valid_wav() {
        for &s in &ALL {
            let w = wav(&synth(s));
            assert_eq!(&w[..4], b"RIFF");
            assert_eq!(u32::from_le_bytes(w[4..8].try_into().unwrap()) as usize, w.len() - 8);
            let secs = (w.len() - 44) as f32 / (RATE * 2) as f32;
            assert!(secs > 0.03 && secs < 0.95, "{s:?}: {secs} s");
        }
    }
}
