# Xadrez

Tabuleiro de xadrez para Windows, para **treinar e analisar**: joga contra engines com Elo ajustável (1000–2600),
vê as 3 melhores jogadas com uma explicação em linguagem simples, e revê as tuas partidas lance a lance.
O estilo do tabuleiro e do painel é inspirado no chess.com. Não tem qualquer ligação ao chess.com nem ao lichess.

Feito em Rust com [egui](https://github.com/emilk/egui). É uma única `.exe`, com as engines lá dentro.

> **Fair play.** Isto é uma ferramenta de estudo. Usar uma engine para escolher lances numa partida em curso
> (online, ranked, torneios) viola as regras do chess.com, do lichess e de praticamente qualquer plataforma.

## Instalar

Descarrega o `Xadrez-Setup.exe` na página de [Releases](../../releases) e executa-o. Instala só para o teu
utilizador (sem pedir administrador), cria atalhos e aparece em "Apps instaladas" para desinstalar.

## O que faz

- **Jogar** contra a engine que escolheres, com o Elo que quiseres (1000–2600), ou jogar os dois lados.
- **3 melhores jogadas** com setas numeradas, avaliação e uma explicação, por exemplo
  *"O bispo dá xeque e ameaça a dama."* O Elo das sugestões também se ajusta (muda a profundidade da análise).
- **Avaliação de cada lance**: ícones como no chess.com (brilhante, ótima, melhor jogada, excelente, boa, teoria,
  imprecisão, erro, gafe), precisão por lado e resumo da partida.
- **Aberturas e defesas**: nome e código ECO da posição (base de dados do lichess) e as continuações de livro.
- **Navegar** na partida com ← →, `Home`, `End`, ou clicando num lance. A partir de qualquer posição podes jogar
  outra coisa (os lances seguintes são descartados). Sugestões e setas acompanham a posição mostrada.
- **Ler uma posição de um print** (`Ctrl+V`, arrastar a imagem ou abrir ficheiro): tabuleiros verdes (chess.com) e
  castanhos (lichess). Há um editor para corrigir peças mal lidas e um botão para a app aprender o estilo de peças.
- **Desenhar** setas e casas com o botão direito, como no chess.com (Shift = verde, Ctrl = vermelho, Alt = azul).
- **Engines**: Stockfish, Berserk, Obsidian, Koivisto, Alexandria, Caissa e Viridithas vêm incluídas.
  O botão "Adicionar engine UCI…" aceita qualquer outra (por exemplo, o Komodo).

Atalhos: `F` vira o tabuleiro · `Ctrl+V` cola um print · `←`/`→` navegam · clique esquerdo limpa os desenhos.

## Compilar a partir do código

Requisitos: Windows 64 bits, [Rust](https://rustup.rs) (com as ferramentas de build do Visual Studio), PowerShell.

```powershell
# 1. descarrega as engines (versões fixas, com verificação SHA-256) para assets/
powershell -ExecutionPolicy Bypass -File scripts/fetch-engines.ps1

# 2. compila (a .exe fica em target/release/xadrez.exe)
cargo build --release

# 3. testes
cargo test --release

# 4. instalador (precisa do NSIS: https://nsis.sourceforge.io)
makensis installer.nsi        # gera dist/Xadrez-Setup.exe
```

Em caminhos muito longos o Windows pode falhar a ligar o executável. Se acontecer, usa uma pasta de build curta:
`$env:CARGO_TARGET_DIR="C:\xt"`.

O workflow `.github/workflows/release.yml` faz tudo isto no GitHub: corre-o à mão (Actions → Release) ou
publica uma tag `v1.0.0` para gerar também a Release com o instalador.

### Notas técnicas

- As engines estão no `.exe` comprimidas em gzip e são extraídas para `%TEMP%` na primeira utilização.
- O Elo do adversário usa o `UCI_Elo` da engine quando existe; senão `Skill Level` ou profundidade de pesquisa
  (estimativas, podem precisar de ajuste por engine).
- As explicações vêm de regras simples sobre o tabuleiro (xeques, capturas, ameaças, ataques descobertos,
  peças salvas); não detetam cravadas nem planos de longo prazo.
- A classificação "brilhante" é uma heurística (lance bom que sacrifica material), não a do chess.com.
- Para pôr outra engine de fábrica: `gzip` do `.exe` para `assets/engines/` e uma linha em `src/engine.rs`.

## Licença

[GPL-3.0](LICENSE). Os créditos de tudo o que vem incluído (engines, peças, aberturas, bibliotecas) estão em
[THIRD_PARTY.md](THIRD_PARTY.md).
