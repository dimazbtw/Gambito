# Gambito — development

Technical notes for building Gambito or working on the code. What the app is and does is in the [README](README.md).

## Building

Requirements: 64-bit Windows, [Rust](https://rustup.rs) (with the Visual Studio build tools), PowerShell.

```powershell
# 1. download the engines (pinned versions, SHA-256 checked) into assets/engines/ (to run the app and the tests)
powershell -ExecutionPolicy Bypass -File scripts/fetch-engines.ps1

# 2. build (the exe ends up in target/release/gambito.exe)
cargo build --release

# 3. tests
cargo test --release

# 4. installer: the same exe under another name
Copy-Item target/release/gambito.exe dist/Gambito-Setup.exe
```

With very long paths Windows can fail to link the executable. If that happens, use a short build folder:
`$env:CARGO_TARGET_DIR="C:\xt"`.

There is also a test that talks to the real chess.com API (off by default):
`cargo test live_api -- --ignored --nocapture`.

The puzzles (`assets/puzzles.tsv.gz`) are already in the repository. To build them again, download
`lichess_db_puzzle.csv.zst` from [database.lichess.org](https://database.lichess.org/#puzzles) (about 310 MB) and run
`$env:PUZZLE_DB="path\lichess_db_puzzle.csv.zst"; cargo test --release build_puzzle_set -- --ignored --nocapture`.
It keeps well-played puzzles (popularity ≥ 80, ≥ 300 plays), up to 260 per 100 rating points, plus at least 30 of
each lesson theme per 200 points.

The `.github/workflows/release.yml` workflow does all of this on GitHub: run it by hand (Actions → Release) or push a
tag such as `v1.0.0` to also publish the Release with the installer.

## Technical notes

- Gambito looks for the engines in `engines\` next to the exe (where the installer puts them) and, in a development
  build, in `assets/engines/`.
- **Installer** (`src/setup.rs`): the exe shows the setup screen when its name contains "setup" or with `--install`,
  and the uninstall screen with `--uninstall` (what Windows "Installed apps" runs). `--silent` does everything
  without a window (all engines; `--dir <folder>`, `--desktop`, and `--remove-data` to also delete the user's data
  when uninstalling). Shortcuts and the "Installed apps" entry are made with PowerShell. To test without touching a
  real install: `GAMBITO_SETUP_NAME=GambitoTest` registers under another name; `--auto` starts installing at once;
  `GAMBITO_SETUP_PAGE=0..2` opens on that step (language, engines, folder); `GAMBITO_SHOT=file.png` with
  `GAMBITO_SHOT_AT=seconds` saves a picture of the window.
- The opponent's Elo uses the engine's `UCI_Elo` when it has one; otherwise `Skill Level` or the search depth
  (estimates, may need tuning per engine).
- Suggestions: below the top of the strength slider the depth follows it (2 to 12) for short lines that are easy to
  follow. At the top (2600) there is no cap: `go infinite`, with the lines sent again at each finished depth, until
  the position changes, the suggestions are turned off or no board is on screen. Game review searches each position
  to depth 16. Engines that offer them get half the CPU cores (`Threads`) and 64 MB of `Hash`.
- The explanations come from simple rules about the board (checks, captures, threats, discovered attacks, pieces
  saved); they don't detect pins or long-term plans.
- The "brilliant" rating is a heuristic (a good move that sacrifices material), not chess.com's.
- **Lichess** (`chesscom::fetch_lichess`): one request to the public `api/games/user/{name}` (NDJSON, 50 games, the
  PGN and Lichess's accuracy when the game was analysed there); "Load older games" asks again with `until`. Its games
  are mapped to the same list as chess.com's.
- **Games** (`src/games.rs`, side menu "Jogos", tab 6): 11 mini-games, each a state machine (`St`) driven by the clock
  and by clicks; positions come from the puzzle set, "Peça pendurada" has Stockfish play itself at Elo 1320 and ends a
  round when a capture wins a hanging piece (`hung_capture`). Best scores are kept in `progress.json` (`games`). egui can
  draw a frame twice, so a game acts on a frame's clicks and keys once (`Run::input`).
- The chess.com search sends one request at a time, 2 months the first time and 3 more for each "Load older games",
  to respect the API limits. Variants (Chess960, etc.) are skipped.
- Diagnostic mode (to check the interface without touching the screen): `GAMBITO_SHOT=file.png` saves a picture of
  the window and quits; `GAMBITO_PGN=file.pgn`, `GAMBITO_TAB=0..5` (0 Play, 1 Analyse, 2 Game review, 3 Puzzles,
  4 Learn, 5 Home), `GAMBITO_LESSON=n` (with `GAMBITO_PRACTICE=1` the practice starts) and `GAMBITO_SIZE=1280x820`
  choose what to show; `GAMBITO_PANEL=0..2` (Moves, Suggestions, Review) and `GAMBITO_HINT_ELO=1000..2600` set the
  board panel and the suggestions' strength; `GAMBITO_GAME=0..10` opens a mini-game, and with `GAMBITO_GAMEBOT=1` a
  test player answers right every 0.35 s and prints the score to stderr; `GAMBITO_CC="lichess:name"` (or `chesscom:name`) searches a user's games on start; `GAMBITO_MARKS="e2e4:0 d5:2"` draws
  right-button arrows and squares (the number is the colour: plain, Shift, Ctrl, Alt); `GAMBITO_MOVES="f3 e5 g4 Qh4"` plays those moves on the board (both sides);
  `GAMBITO_SOLVE=right|wrong|show` plays the puzzle by itself (right, with a mistake first, or showing the solution).
  To keep your own progress untouched, point `APPDATA` at a test folder.
- The puzzle rating is a simple Elo (K = 40 for the first 15 puzzles, then 20), not Lichess's Glicko.
- Promotion: a pawn reaching the last rank opens a column of four pieces over the square (`App::promo`); a click
  elsewhere cancels. Underpromotion puzzles are included. The mini-games still promote to a queen.
- **Engines and performance**: each engine is its own process, started when it has something to do and closed after
  2 minutes idle. A Stockfish costs about 250 MB plus ~28 MB per thread and ~50 MB for a 64 MB hash, so the one that
  plays uses 1 thread and 16 MB (`Engine::new`), and the ones for suggestions and reviews use up to 4 threads and
  64 MB (`Engine::heavy`). All run at BELOW_NORMAL priority, so the interface always comes first. The suggestions'
  engine starts only when a board is on screen (`App::board_on_screen`), and its uncapped search pauses during a
  review. A review searches each position for at most 0.6 s (0.35 s for the rating of a single move).
  Nothing animates forever, so an idle window draws nothing (0% CPU). `GAMBITO_PERF=1` prints, every second, the
  frames, the time of our drawing code and who asked for each repaint (by file:line), plus the startup timings.
- **Motion** (`src/motion.rs`): the rules in one place, from published standards (Emil Kowalski's animation notes,
  Material motion). Short (mostly under 300 ms) and ease-out; things that move across the screen (the menu highlight,
  the panel tabs) glide with an ease-in-out (`glide`); buttons shrink to 97% while pressed; a spreading ring marks a
  new check; confetti only for a win or a record; a skeleton pulses while online games load; the toast slides in
  250 ms and leaves in 180 ms, with no bounce. What is done tens or hundreds of times a day is not decorated: a
  screen's entrance (cards rising 40 ms apart, numbers counting, bars filling) plays only the first time it is opened
  in a session (`App::page_age`), hovering a card changes colour only, and stepping through a game with the keyboard
  has no slide. Settings -> Animations (default: Windows' "Show animations") turns it all to instant.
  Every frame costs a lot of CPU in this app, so nothing animates forever and a game picks its repaint rate by what
  moves (`Run::repaint_gap`: every frame for a flash or a flip, 20-30 a second for a counter or a reveal, 10 a second
  while it waits for a tap).
- **Pieces in the games** slide like on the main board (`Slide`: 200 ms, ease-out, a captured piece fades out underneath):
  Mate Sprint (my move and the reply), Hanging Piece, and Knight Route (the knight glides between hops). Pawn Recall's
  pawns pop in one after another, fade out when the time is up and pop back when found, inside a gold border round
  the lit window that glides to the next level's window. New positions in Who Gives Check? and Legal or Illegal?
  fade in. `GAMBITO_SLOWMO=10` makes the slide ten times longer, to look at it frame by frame.
- **Game art**: each of the 11 games has a picture (`assets/art/game_<key>.webp`, shown on its card and in its panel),
  made by the author like the other art. `card_play.webp` is the Play card of the home screen.
- **Sounds** (`src/sound.rs`): made in code when first played (a wooden knock, bells), as 16-bit WAVs in memory,
  played with Windows' `PlaySound` (winmm), so there is no audio library and no sound file.
- **Settings** kept in `settings.json` (`i18n::setting` / `i18n::save_setting`): language, opponent strength,
  suggestion strength, suggestions on/off, sounds, board colours (`THEMES`).
- Games played against the engine (or both sides) are added to `%APPDATA%\Gambito\games.pgn` when they end or when
  the player resigns, and listed on the Analyse page.
- The review: each position is searched to depth 16 for at most 0.6 s; the report adds an estimated rating per side
  (`est_elo`, a fixed curve on the accuracy), accuracy per phase (`phase_of`: the first 10 moves, then little
  material = endgame) and the move where the winning chances dropped the most.
- To ship another engine: a line in `engine::known()` (`src/engine.rs`) with its address, SHA-256 and size, and the
  same in `scripts/fetch-engines.ps1`.
- Coming from the old "Xadrez" version, added engines and learned pieces move to Gambito's folder by themselves.
- **Translations**: the text in the code is Portuguese and goes through `tl("…")` (or `tr!("… {} …", value)` when it
  has values, in `src/i18n.rs`). Each other language is a file `assets/i18n/<code>.tsv`, one line per text:
  `portuguese<TAB>translation`, with `\n` for a new line and `{}` for each value. A text without a translation shows
  in Portuguese. To add a language: a new `.tsv`, a line in `LANGS` and another in `table()` in `src/i18n.rs`.
  `cargo test --release i18n` fails if a translation is missing; with `I18N_MISSING=file.tsv` it writes the missing
  texts to that file, ready to translate.

## Automatic updates

- The app looks for new versions in the **GitHub Releases** of the repository set in `repository` in `Cargo.toml`
  (`https://github.com/<user>/<repo>`). With that field empty, it never checks.
- Once a day, in the background (`src/update.rs`): it asks the public `releases/latest` API, compares the tag
  (`v1.0.1`) with the version in `Cargo.toml` and, if it is newer, shows the **Update** button in the menu. It can be
  turned off in Settings → Updates.
- **Update** downloads the release's `Gambito-Setup.exe`, checks it against the same release's `SHA256SUMS.txt` and
  runs it with `--silent --update --dir <folder>`: it waits for the app to close, replaces `Gambito.exe`, keeps the
  engines that were there (only downloading again those whose SHA-256 doesn't match) and opens the app again.
- **To publish a version**: change `version` in `Cargo.toml`, commit, and push a tag `vX.Y.Z`. The `release.yml`
  workflow builds, tests and publishes `Gambito-Setup.exe` + `SHA256SUMS.txt` (without `SHA256SUMS.txt` the app won't
  update).
- Local testing: `GAMBITO_UPDATE_API=<url>` makes the app ask that address instead of GitHub (same JSON as the API)
  and skips the once-a-day limit. The state is kept in `%APPDATA%\Gambito\update.json`.
