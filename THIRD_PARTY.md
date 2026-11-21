# Third-party components

Gambito is distributed under the **GPL-3.0** (see `LICENSE`). The installer downloads the engines below from each
one's official GitHub releases, each under its own licence. Each one's source code is at the page listed (the
installer uses exactly the versions listed and checks the SHA-256 of every file; `scripts/fetch-engines.ps1`
downloads the same ones for development).

## Engines (run as separate processes)

| Engine | Version | Licence | Source code |
|---|---|---|---|
| Stockfish | 19 | GPL-3.0 | https://github.com/official-stockfish/Stockfish |
| Berserk | 14 | GPL-3.0 | https://github.com/jhonnold/berserk |
| Obsidian | 16.0 | GPL-3.0 | https://github.com/gab8192/Obsidian |
| Koivisto | 9.0 | GPL-3.0 | https://github.com/Luecx/Koivisto |
| Alexandria | 9.1.0 | GPL-3.0 | https://github.com/PGG106/Alexandria |
| Caissa | 2.0 | MIT | https://github.com/Witek902/Caissa |
| Viridithas | 20.0.0 | AGPL-3.0 | https://github.com/cosmobobak/viridithas |

The engines are not modified: they are the official binaries of their releases.

## Fonts, logo and images

- **Outfit** (Medium and SemiBold): The Outfit Project Authors, SIL Open Font License 1.1 (`assets/fonts/OFL-Outfit.txt`).
- **Inter**: The Inter Project Authors, SIL Open Font License 1.1 (`assets/fonts/OFL-Inter.txt`).
- **Logo** (`assets/logo.webp`, `assets/icon.ico`): by Gambito's author.
- **Images** (`assets/art/*.webp`): flat 2D art made by Gambito's author with ChatGPT's image generator (OpenAI), in
  the style of the installer images and in the app's colours.
- **Installer images** (`assets/setup/*.png`): by Gambito's author.

## Data

- **Openings** (`assets/openings/*.tsv`): [lichess-org/chess-openings](https://github.com/lichess-org/chess-openings),
  public domain (CC0).
- **Puzzles** (`assets/puzzles.tsv.gz`): a selection of about 10,000 puzzles from the
  [Lichess puzzle database](https://database.lichess.org/#puzzles), public domain (CC0). Each puzzle keeps its id,
  position, solution, rating and themes.
- **Pieces** (`assets/pieces/*.svg`): the "cburnett" set by Colin M. L. Burnett (Wikimedia Commons), licensed under
  GPLv2+ (also BSD and GFDL), taken from the Lichess repository.
- **`assets/neo_silhouettes.txt` and `tests_data/`**: piece shapes (24×24 bits) and a screenshot, derived from a
  chess.com screenshot, used only so the screenshot reader recognises that piece style, and for a test. The original
  art belongs to chess.com. This project has no connection to that site. If the owner asks for removal, this file and
  the `real_chesscom_screenshot` test are deleted (the reader keeps working with "Learn these pieces").

## Rust libraries

[egui / eframe](https://github.com/emilk/egui) (MIT/Apache-2.0), [shakmaty](https://github.com/niklasf/shakmaty) (GPL-3.0),
[image](https://github.com/image-rs/image), [resvg](https://github.com/linebender/resvg), [arboard](https://github.com/1Password/arboard),
[rfd](https://github.com/PolyMeilex/rfd), [flate2](https://github.com/rust-lang/flate2-rs),
[ureq](https://github.com/algesten/ureq) (HTTPS requests), [serde_json](https://github.com/serde-rs/json),
[sys-locale](https://github.com/1Password/sys-locale) (the Windows language, MIT/Apache-2.0) and
[sha2](https://github.com/RustCrypto/hashes) (checking engines and updates, MIT/Apache-2.0).
Only for building the puzzle set (not shipped in the app): [ruzstd](https://github.com/KillingSpark/zstd-rs) (MIT).
The full list and versions are in `Cargo.lock`.

## External services

- **chess.com public API** (`api.chess.com/pub`, "Published-Data API"): used only to read a user's public, finished
  games, when the user presses "Search". Not affiliated with or endorsed by chess.com. Check the API's terms of use
  before distributing.
- **Lichess API** (`lichess.org/api/games/user`): used only to read a user's public games, when the user presses
  "Search" with Lichess selected. Not affiliated with or endorsed by Lichess.
- **GitHub API** (`api.github.com`): once a day, to ask for Gambito's latest release (only when a repository is set
  in `Cargo.toml`; can be turned off in Settings → Updates). Release files are downloaded from GitHub.

## Installer

It is Gambito itself (`src/setup.rs`), with no third-party programs besides the libraries above.
