# Gambito

Tabuleiro de xadrez para Windows, para **treinar e analisar**: joga contra engines com Elo ajustável (1000–2600),
vê as 3 melhores jogadas com uma explicação em linguagem simples, e revê as tuas partidas lance a lance,
incluindo as do teu perfil do chess.com.

O estilo do tabuleiro e do painel é inspirado no chess.com. Não tem qualquer ligação ao chess.com nem ao lichess.

Feito em Rust com [egui](https://github.com/emilk/egui). É uma única `.exe`, com as engines lá dentro.

> **Fair play.** Isto é uma ferramenta de estudo. Usar uma engine para escolher lances numa partida em curso
> (online, ranked, torneios) viola as regras do chess.com, do lichess e de praticamente qualquer plataforma.
> A importação de partidas só traz partidas já terminadas.

## Instalar

Descarrega o `Gambito-Setup.exe` na página de [Releases](../../releases) e executa-o. Instala só para o teu
utilizador (sem pedir administrador), cria atalhos e aparece em "Apps instaladas" para desinstalar.

## O que faz

- **Menu à esquerda**: **Analisar** (tabuleiro, sugestões, lances, importar imagem, editar tabuleiro e FEN), **Partidas**
  (chess.com e PGN) e **Definições** (engines, Elo do adversário, jogar os dois lados). Início, Exercícios e Aprender
  ainda não existem. Quando a partida termina (ou abres uma importada) aparece o botão dourado **Análise da partida**,
  com a precisão, o gráfico da avaliação e os lances a rever.
- **Barra de reprodução** por baixo do tabuleiro (início, anterior, reproduzir sozinho, seguinte, fim) e, para listas
  de partidas importadas, "Partida N de M" no topo. O ícone de descarregar copia a partida em PGN.
- **Jogar** contra a engine que escolheres, com o Elo que quiseres (1000–2600), ou jogar os dois lados.
- **3 melhores jogadas** com setas numeradas, avaliação e uma explicação, por exemplo
  *"O bispo dá xeque e ameaça a dama."* O Elo das sugestões também se ajusta (muda a profundidade da análise).
- **Avaliação de cada lance**: ícones como no chess.com (brilhante, ótima, melhor jogada, excelente, boa, teoria,
  imprecisão, erro, gafe), precisão por lado e resumo da partida.
- **Aberturas e defesas**: nome e código ECO da posição (base de dados do lichess) e as continuações de livro.
- **Importar partidas**
  - **chess.com**: escreves o nome de utilizador, escolhes uma partida da lista (só xadrez normal, públicas e
    terminadas) e ela abre no tabuleiro, já avaliada. Se o chess.com tiver a precisão dessa partida, é mostrada
    para comparares.
  - **PGN**: cola o texto (`Ctrl+V`) ou abre um ficheiro `.pgn`. Aceita vários jogos por ficheiro, comentários,
    variantes e posições iniciais (FEN).
  - **Print**: lê uma posição de um screenshot de tabuleiro verde (chess.com) ou castanho (lichess), com editor
    para corrigir peças mal lidas e um botão para a app aprender o estilo de peças.
- **Navegar** na partida com ← →, `Home`, `End`, ou clicando num lance. A partir de qualquer posição podes jogar
  outra coisa (os lances seguintes são descartados). Sugestões e setas acompanham a posição mostrada.
- **Desenhar** setas e casas com o botão direito, como no chess.com (Shift = verde, Ctrl = vermelho, Alt = azul).
- **Engines**: Stockfish, Berserk, Obsidian, Koivisto, Alexandria, Caissa e Viridithas vêm incluídas.
  O botão "Adicionar engine UCI…" aceita qualquer outra (por exemplo, o Komodo).

Atalhos: `F` vira o tabuleiro · `Ctrl+V` cola um print ou PGN · `←`/`→` navegam · clique esquerdo limpa os desenhos.

## Privacidade

O Gambito funciona offline. A **única** ligação à internet é a busca de partidas do chess.com, e só acontece quando
carregas em "Procurar": é enviado o nome de utilizador (que vai no endereço do pedido) à API pública do chess.com
(`api.chess.com/pub`), sem login. Não há telemetria nem contas. As engines correm no teu computador.

## Compilar a partir do código

Requisitos: Windows 64 bits, [Rust](https://rustup.rs) (com as ferramentas de build do Visual Studio), PowerShell.

```powershell
# 1. descarrega as engines (versões fixas, com verificação SHA-256) para assets/
powershell -ExecutionPolicy Bypass -File scripts/fetch-engines.ps1

# 2. compila (a .exe fica em target/release/gambito.exe)
cargo build --release

# 3. testes
cargo test --release

# 4. instalador (precisa do NSIS: https://nsis.sourceforge.io)
makensis installer.nsi        # gera dist/Gambito-Setup.exe
```

Em caminhos muito longos o Windows pode falhar a ligar o executável. Se acontecer, usa uma pasta de build curta:
`$env:CARGO_TARGET_DIR="C:\xt"`.

Há também um teste que fala com a API real do chess.com (parado por defeito):
`cargo test live_api -- --ignored --nocapture`.

O workflow `.github/workflows/release.yml` faz tudo isto no GitHub: corre-o à mão (Actions → Release) ou
publica uma tag `v1.0.0` para gerar também a Release com o instalador.

### Notas técnicas

- As engines estão no `.exe` comprimidas em gzip e são extraídas para `%TEMP%` na primeira utilização.
- O Elo do adversário usa o `UCI_Elo` da engine quando existe; senão `Skill Level` ou profundidade de pesquisa
  (estimativas, podem precisar de ajuste por engine).
- As explicações vêm de regras simples sobre o tabuleiro (xeques, capturas, ameaças, ataques descobertos,
  peças salvas); não detetam cravadas nem planos de longo prazo.
- A classificação "brilhante" é uma heurística (lance bom que sacrifica material), não a do chess.com.
- A busca no chess.com faz pedidos um de cada vez, 2 meses na primeira vez e mais 3 por cada "Mais antigas",
  para respeitar os limites da API. Variantes (Chess960, etc.) são ignoradas.
- Modo de diagnóstico (para verificar a interface sem tocar no ecrã): `GAMBITO_SHOT=ficheiro.png` guarda uma imagem da própria
  janela e fecha; `GAMBITO_PGN=ficheiro.pgn`, `GAMBITO_TAB=0|1|2` e `GAMBITO_SIZE=1280x820` escolhem o que mostrar.
- Para pôr outra engine de fábrica: `gzip` do `.exe` para `assets/engines/` e uma linha em `src/engine.rs`.
- Se vens da versão antiga "Xadrez", as engines adicionadas e as peças aprendidas passam sozinhas para a pasta do Gambito.

## Licença

[GPL-3.0](LICENSE). Os créditos de tudo o que vem incluído (engines, peças, aberturas, bibliotecas) estão em
[THIRD_PARTY.md](THIRD_PARTY.md).
