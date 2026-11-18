# Componentes de terceiros

O Gambito é distribuído sob a **GPL-3.0** (ver `LICENSE`). O instalador inclui os programas abaixo, cada um com a
sua própria licença. O código-fonte de cada um está na página indicada (o instalador usa exatamente as versões
listadas; `scripts/fetch-engines.ps1` descarrega-as com verificação SHA-256).

## Engines incluídas (executadas como processos separados)

| Engine | Versão | Licença | Código-fonte |
|---|---|---|---|
| Stockfish | 19 | GPL-3.0 | https://github.com/official-stockfish/Stockfish |
| Berserk | 14 | GPL-3.0 | https://github.com/jhonnold/berserk |
| Obsidian | 16.0 | GPL-3.0 | https://github.com/gab8192/Obsidian |
| Koivisto | 9.0 | GPL-3.0 | https://github.com/Luecx/Koivisto |
| Alexandria | 9.1.0 | GPL-3.0 | https://github.com/PGG106/Alexandria |
| Caissa | 2.0 | MIT | https://github.com/Witek902/Caissa |
| Viridithas | 20.0.0 | AGPL-3.0 | https://github.com/cosmobobak/viridithas |

As engines não são alteradas: são os binários oficiais das respetivas releases.

## Tipos de letra e logótipo

- **Outfit** (Medium e SemiBold): The Outfit Project Authors, SIL Open Font License 1.1 (`assets/fonts/OFL-Outfit.txt`).
- **Inter**: The Inter Project Authors, SIL Open Font License 1.1 (`assets/fonts/OFL-Inter.txt`).
- **Logótipo** (`assets/logo.webp`, `assets/icon.ico`): do autor do Gambito.

## Dados

- **Aberturas** (`assets/openings/*.tsv`): [lichess-org/chess-openings](https://github.com/lichess-org/chess-openings),
  domínio público (CC0).
- **Peças** (`assets/pieces/*.svg`): conjunto "cburnett" de Colin M. L. Burnett (Wikimedia Commons), licenciado
  em GPLv2+ (também BSD e GFDL), obtido do repositório do lichess.
- **`assets/neo_silhouettes.txt` e `tests_data/`**: formas de peças (24×24 bits) e uma captura de ecrã, derivadas de
  um print do chess.com, usadas só para o leitor de prints reconhecer esse estilo de peças e para um teste.
  A arte original pertence ao chess.com. O projeto não tem qualquer ligação a esse site. Se o detentor pedir a remoção,
  apaga-se este ficheiro e o teste `real_chesscom_screenshot` (o leitor continua a funcionar com "Aprender estas peças").

## Bibliotecas Rust

[egui / eframe](https://github.com/emilk/egui) (MIT/Apache-2.0), [shakmaty](https://github.com/niklasf/shakmaty) (GPL-3.0),
[image](https://github.com/image-rs/image), [resvg](https://github.com/linebender/resvg), [arboard](https://github.com/1Password/arboard),
[rfd](https://github.com/PolyMeilex/rfd), [flate2](https://github.com/rust-lang/flate2-rs),
[ureq](https://github.com/algesten/ureq) (pedidos HTTPS) e [serde_json](https://github.com/serde-rs/json).
A lista completa e as versões estão em `Cargo.lock`.

## Serviços externos

- **API pública do chess.com** (`api.chess.com/pub`, "Published-Data API"): usada só para ler partidas públicas e terminadas
  de um utilizador, quando o utilizador carrega em "Procurar". Não é afiliada nem apoiada pelo chess.com. Confirma os
  termos de utilização da API antes de distribuir.

## Instalador

Gerado com [NSIS](https://nsis.sourceforge.io) (licença zlib/libpng).
