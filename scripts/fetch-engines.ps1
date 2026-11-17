# Downloads the chess engines that Xadrez embeds, from their official GitHub releases (pinned versions),
# checks their SHA-256 and stores them gzip-compressed in assets/ (that is what src/engine.rs includes).
# Run once before `cargo build`:   powershell -ExecutionPolicy Bypass -File scripts/fetch-engines.ps1
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"   # the progress bar makes Invoke-WebRequest extremely slow in Windows PowerShell 5.1
$assets = Join-Path (Split-Path -Parent $PSScriptRoot) "assets"
New-Item -ItemType Directory -Force (Join-Path $assets "engines") | Out-Null
$tmp = Join-Path ([IO.Path]::GetTempPath()) "xadrez-engines"
New-Item -ItemType Directory -Force $tmp | Out-Null

# name, download url, sha256 of the .exe, output (relative to assets/)
$engines = @(
  @{ n="Stockfish 19";   u="https://github.com/official-stockfish/Stockfish/releases/download/sf_19/stockfish-windows-x86-64-universal.zip"; zip=$true; h="45bc8e4969147db9c2eb533810637994619bff0eacc81ccfd9854394901bcbd0"; o="stockfish.gz" },
  @{ n="Berserk 14";     u="https://github.com/jhonnold/berserk/releases/download/14/berserk-14-avx2.exe";                                      h="129df854a536d72143d431d1b764ddbd7e50e2f54c28e6ac9ffb50d7a85d6462"; o="engines/berserk.gz" },
  @{ n="Obsidian 16";    u="https://github.com/gab8192/Obsidian/releases/download/v16.0/Obsidian160-avx2.exe";                                  h="00f9f5566e815275e29fec85f1e4d1dff0b8301ceb23f6bb79de68dcdce98743"; o="engines/obsidian.gz" },
  @{ n="Koivisto 9";     u="https://github.com/Luecx/Koivisto/releases/download/v9.0/Koivisto_9.0-windows-avx2-pgo.exe";                        h="9fcea126ed7dc882718bd22804d6c1227377009cd24fb983a1694a6d625e443e"; o="engines/koivisto.gz" },
  @{ n="Alexandria 9.1"; u="https://github.com/PGG106/Alexandria/releases/download/v9.1.0/Alexandria-9.1.0-avx2.exe";                          h="21fe201c21d7178f520fce94c7f9a6e378597b13f5d3a7e8aa00b4c1adf533b6"; o="engines/alexandria.gz" },
  @{ n="Caissa 2.0";     u="https://github.com/Witek902/Caissa/releases/download/2.0/caissa-2.0-x64-avx2.exe";                                  h="043c0925df8c608d0d87b9e6b1c761240ddd1901ee8cba49e346686b28816b97"; o="engines/caissa.gz" },
  @{ n="Viridithas 20";  u="https://github.com/cosmobobak/viridithas/releases/download/v20.0.0/viridithas-20-win-x86-64-v3.exe";               h="c7fa86b2b46fe1c34b0ea3715ad3b4a61d3d984ec63d50b1d6b4f187332fbc51"; o="engines/viridithas.gz" }
)

foreach ($e in $engines) {
  $out = Join-Path $assets $e.o
  if (Test-Path $out) { Write-Host "$($e.n): already there, skipping"; continue }
  Write-Host "$($e.n): downloading..."
  $dl = Join-Path $tmp ([IO.Path]::GetFileName($e.u))
  Invoke-WebRequest -Uri $e.u -OutFile $dl -UseBasicParsing
  $exe = $dl
  if ($e.zip) {
    $dir = Join-Path $tmp "unzip-stockfish"
    if (Test-Path $dir) { Remove-Item $dir -Recurse -Force }
    Expand-Archive $dl $dir
    $exe = (Get-ChildItem $dir -Recurse -Filter *.exe | Select-Object -First 1).FullName
  }
  $hash = (Get-FileHash $exe -Algorithm SHA256).Hash.ToLower()
  if ($hash -ne $e.h) { throw "$($e.n): unexpected SHA-256 ($hash). Refusing to embed it." }
  $in = [IO.File]::OpenRead($exe); $o = [IO.File]::Create($out)
  $gz = New-Object IO.Compression.GZipStream($o, [IO.Compression.CompressionLevel]::Optimal)
  $in.CopyTo($gz); $gz.Dispose(); $o.Dispose(); $in.Dispose()
  Write-Host ("{0}: ok ({1:N1} MB)" -f $e.n, ((Get-Item $out).Length / 1MB))
}
Write-Host "Done. Now: cargo build --release"
