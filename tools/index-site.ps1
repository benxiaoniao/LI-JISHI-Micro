# 从 `砾索引.toml` 生成静态包站（LI028）：外置包管理站直接读同一份索引，不维护第二套目录。
# 只调 `micc search --json`，不自己解析 TOML；产物是 index.html + index.json，无服务器、无外部依赖。
param(
    [string]$Index = "砾索引.toml",
    [string]$Out = "build/包站",
    [string]$Pubkey = "",
    [string]$Identity = "li"
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot "toolchains.ps1")
$micc = Join-Path $root ("compiler/target/release/micc" + $ExeSuffix)

function Fail([string]$message) {
    throw ("包站生成失败：" + $message)
}

if (-not (Test-Path $micc)) {
    Fail ("找不到 " + $micc + "；先运行 tools/build.ps1")
}
if (-not (Test-Path $Index)) {
    Fail ("找不到索引 " + $Index + "（用 -Index 指定）")
}
$idxPath = (Resolve-Path $Index).Path
$idxDir = Split-Path -Parent $idxPath
$outDir = if ([IO.Path]::IsPathRooted($Out)) {
    [IO.Path]::GetFullPath($Out)
}
else {
    [IO.Path]::GetFullPath((Join-Path (Get-Location).Path $Out))
}

$argv = @("search", "--json", "--索引", $idxPath)
$signed = $false
if ($Pubkey) {
    if (-not (Test-Path $Pubkey)) {
        Fail ("找不到公钥允许列表 " + $Pubkey)
    }
    $argv += @("--公钥", (Resolve-Path $Pubkey).Path, "--签名身份", $Identity)
    $signed = $true
}

$raw = (& $micc @argv 2>&1 | Out-String)
$jsonLine = ($raw -split "`r?`n" | Where-Object { $_.TrimStart().StartsWith("{") } | Select-Object -Last 1)
if (-not $jsonLine) {
    Fail ("micc search --json 没有输出 JSON：`n" + $raw)
}
$data = $jsonLine | ConvertFrom-Json

# Windows 上相对路径用正斜杠；跨盘符退回绝对路径（与索引里的 来源 同口径）。
function Get-RelLink([string]$fromDir, [string]$target) {
    if ($target -match '^(?i)https?://') { return $target }
    $p = $target
    if ($p -match '^(?i)file:') { $p = $p.Substring(5) }
    if (-not [IO.Path]::IsPathRooted($p)) { $p = Join-Path $idxDir $p }
    $full = [IO.Path]::GetFullPath($p)
    $from = [IO.Path]::GetFullPath($fromDir).TrimEnd('\', '/')
    $fs = $from -split '[\\/]'
    $ts = $full -split '[\\/]'
    if ($fs[0] -ne $ts[0]) { return $full.Replace('\', '/') }
    $i = 0
    while ($i -lt $fs.Count -and $i -lt $ts.Count -and $fs[$i] -eq $ts[$i]) { $i++ }
    $parts = @()
    for ($k = $i; $k -lt $fs.Count; $k++) { $parts += '..' }
    for ($k = $i; $k -lt $ts.Count; $k++) { $parts += $ts[$k] }
    return ($parts -join '/')
}

function Html([string]$s) {
    if ($null -eq $s) { return "" }
    return ($s -replace '&', '&amp;' -replace '<', '&lt;' -replace '>', '&gt;' -replace '"', '&quot;')
}

$entries = @()
foreach ($p in @($data.'包')) {
    if ($null -eq $p) { continue }
    $entries += [ordered]@{
        '名字' = [string]$p.'名字'
        '版本' = [string]$p.'版本'
        '来源' = [string]$p.'来源'
        '校验' = [string]$p.'校验'
        '链接' = (Get-RelLink $outDir ([string]$p.'来源'))
    }
}

New-Item -ItemType Directory -Path $outDir -Force | Out-Null
$jsonPath = Join-Path $outDir "index.json"
$htmlPath = Join-Path $outDir "index.html"
$now = (Get-Date).ToString("yyyy-MM-ddTHH:mm:ssK")

$site = [ordered]@{
    '索引' = $idxPath
    '生成时间' = $now
    '已验签' = $signed
    '条数' = $entries.Count
    '包' = $entries
}
$utf8 = New-Object System.Text.UTF8Encoding($false)
[IO.File]::WriteAllText($jsonPath, (($site | ConvertTo-Json -Depth 6) + "`n"), $utf8)

$rows = New-Object System.Text.StringBuilder
foreach ($e in $entries) {
    $name = Html $e.'名字'
    $ver = Html $e.'版本'
    $link = Html $e.'链接'
    $hash = Html $e.'校验'
    [void]$rows.AppendLine("      <tr data-name=""$($name.ToLower())""><td>$name</td><td>$ver</td>" +
        "<td><a href=""$link"">$link</a></td><td><code>$hash</code></td></tr>")
}
$badge = if ($signed) { "已验签（" + (Html $Identity) + "）" } else { "未验签（只做哈希校验）" }
$count = $entries.Count
$indexLabel = Html $idxPath

$html = @"
<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>砾包站</title>
<style>
  :root { color-scheme: light dark; --bg:#fbfaf7; --fg:#1d1b18; --muted:#6b6560; --line:#e4ded5; --accent:#8a4b19; --card:#fff; }
  @media (prefers-color-scheme: dark) { :root { --bg:#161513; --fg:#eae4da; --muted:#a0988e; --line:#34302b; --accent:#e0a26b; --card:#1e1c19; } }
  * { box-sizing: border-box; }
  body { margin:0; background:var(--bg); color:var(--fg); font:15px/1.6 "Segoe UI","Microsoft YaHei",system-ui,sans-serif; }
  main { max-width: 960px; margin: 0 auto; padding: 40px 20px 64px; }
  h1 { font-size: 26px; margin: 0 0 6px; letter-spacing: .04em; }
  .sub { color: var(--muted); margin: 0 0 20px; }
  .bar { display:flex; flex-wrap:wrap; gap:12px; align-items:center; margin: 18px 0 12px; }
  input[type=search] { flex:1 1 220px; padding:8px 10px; border:1px solid var(--line); border-radius:8px; background:var(--card); color:inherit; font:inherit; }
  .badge { padding:3px 10px; border:1px solid var(--line); border-radius:999px; color:var(--muted); font-size:13px; }
  table { width:100%; border-collapse: collapse; background:var(--card); border:1px solid var(--line); border-radius:10px; overflow:hidden; }
  th, td { text-align:left; padding:10px 12px; border-bottom:1px solid var(--line); vertical-align:top; }
  th { font-size:13px; color:var(--muted); font-weight:600; }
  tr:last-child td { border-bottom:0; }
  td:nth-child(1) { white-space:nowrap; font-weight:600; }
  td:nth-child(2) { white-space:nowrap; }
  td:nth-child(3) { word-break:break-all; }
  code { font-family: ui-monospace,Consolas,monospace; font-size:12.5px; color:var(--muted); word-break:break-all; }
  a { color: var(--accent); }
  footer { margin-top: 18px; color: var(--muted); font-size: 13px; word-break: break-all; }
</style>
</head>
<body>
<main>
  <h1>砾包站</h1>
  <p class="sub">数据直接来自同一份 <code>砾索引.toml</code>：外置站点只读索引，不维护第二套包目录。</p>
  <div class="bar">
    <input id="q" type="search" placeholder="按包名筛选…" autocomplete="off">
    <span class="badge" id="count">共 $count 条</span>
    <span class="badge">$badge</span>
  </div>
  <table>
    <thead><tr><th>包名</th><th>版本</th><th>来源</th><th>校验</th></tr></thead>
    <tbody id="rows">
$($rows.ToString().TrimEnd())
    </tbody>
  </table>
  <footer>索引：$indexLabel<br>生成时间：$now　生成器：tools/index-site.ps1</footer>
</main>
<script>
  const q = document.getElementById('q');
  const rows = Array.from(document.querySelectorAll('#rows tr'));
  const count = document.getElementById('count');
  q.addEventListener('input', () => {
    const v = q.value.trim().toLowerCase();
    let shown = 0;
    for (const row of rows) {
      const hit = !v || row.dataset.name.includes(v);
      row.style.display = hit ? '' : 'none';
      if (hit) shown++;
    }
    count.textContent = '共 ' + shown + ' 条';
  });
</script>
</body>
</html>
"@
[IO.File]::WriteAllText($htmlPath, $html, $utf8)

Write-Host ("包站已生成：" + $htmlPath)
Write-Host ("  索引：" + $idxPath)
Write-Host ("  条目：" + $count + " 条；验签：" + $(if ($signed) { "是" } else { "否" }))
Write-Host ("  JSON：" + $jsonPath)
