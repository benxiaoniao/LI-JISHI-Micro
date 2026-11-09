# 把锁定的工具链打成「绿色包」（解压即用，不联网）
#
# 用法：
#   powershell -ExecutionPolicy Bypass -File tools\pack-toolchains.ps1
#   powershell -ExecutionPolicy Bypass -File tools\pack-toolchains.ps1 -CheckOnly
#   powershell -ExecutionPolicy Bypass -File tools\pack-toolchains.ps1 -Name host-gcc
#
# 产物：build\toolchains\jishi-toolchains-<lock 更新日期>.zip
#   zip 内：<名字>/bin/<可执行文件> …（整目录）+ 清单.txt（版本与关键文件 sha256）
# 使用：解压后把根目录设为 JM_TOOLCHAIN_ROOT，tools\toolchains.ps1 会优先读它。
#
# 注意：工具链放在非 ASCII 路径下会让 MinGW 版 gcc/ld 找不到 crti.o / -lgcc，
#       绿色包必须解压到纯 ASCII 路径（例如 D:\jishi-toolchains）。

param(
    [string[]]$Name = @("host-gcc", "arm-none-eabi"),
    [string]$Out = "",
    # Optimal 对 2 GB 级工具链很慢；Fastest 体积略大，但几十秒级完成
    [ValidateSet("Optimal", "Fastest", "NoCompression")]
    [string]$Level = "Fastest",
    [switch]$CheckOnly
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

$root = Split-Path -Parent $PSScriptRoot
$lockPath = Join-Path $PSScriptRoot "toolchains\toolchain.lock.json"
if (-not (Test-Path $lockPath)) { throw ("缺少锁定文件：" + $lockPath) }
$lock = Get-Content -Path $lockPath -Raw -Encoding UTF8 | ConvertFrom-Json
$cache = if ($env:JM_TOOLCHAIN_ROOT) { $env:JM_TOOLCHAIN_ROOT } else { Join-Path $env:LOCALAPPDATA "jishi\toolchains" }

function Find-Installed([string]$n) {
    $a = Join-Path $PSScriptRoot ("toolchains\" + $n)
    if (Test-Path $a) { return $a }
    $b = Join-Path $cache $n
    if (Test-Path $b) { return $b }
    return $null
}

$resolved = @()
foreach ($n in $Name) {
    $tc = $lock.toolchains.$n
    if (-not $tc) { throw ("锁定文件里没有工具链：" + $n) }
    $dir = Find-Installed $n
    if (-not $dir) {
        throw ("找不到已安装的 " + $n + "；先运行 tools\fetch-toolchain.ps1（或按 docs\工具链.md 手动放置）")
    }
    # 逐个校验锁定清单里的关键二进制：确保打的是同一份
    foreach ($f in $tc.files) {
        $p = Join-Path $dir ($f.path -replace "/", "\")
        if (-not (Test-Path $p)) { throw ("缺少文件：" + $p) }
        $h = (Get-FileHash -Path $p -Algorithm SHA256).Hash
        if ($h -ne $f.sha256.ToUpper()) {
            throw ("哈希不符：" + $f.path + " 期望 " + $f.sha256 + " 实际 " + $h)
        }
    }
    $exe = Join-Path $dir ("bin\" + $tc.exe)
    $ver = (& $exe --version 2>$null | Select-Object -First 1)
    $resolved += [pscustomobject]@{ Name = $n; Dir = $dir; Lock = $tc; Version = $ver }
    Write-Host ("[OK] " + $n + " " + $tc.version + "  " + $dir)
    Write-Host ("     " + $ver)
}

$listText = @()
$listText += "# 砾 工具链绿色包清单（由 tools\pack-toolchains.ps1 生成）"
$listText += "# 使用：解压到纯 ASCII 路径，把该根目录设为 JM_TOOLCHAIN_ROOT"
$listText += ("锁文件：" + $lockPath)
$listText += ("锁定日期：" + $lock.updated)
$listText += ""
foreach ($r in $resolved) {
    $listText += ("[" + $r.Name + "]")
    $listText += ("版本 = " + $r.Lock.version)
    if ($r.Lock.binutils_version) { $listText += ("binutils = " + $r.Lock.binutils_version) }
    $listText += ("描述 = " + $r.Lock.display)
    $listText += ("实测版本行 = " + $r.Version)
    foreach ($f in $r.Lock.files) {
        $listText += ("文件 = " + $f.path + "  sha256 = " + $f.sha256.ToUpper())
    }
    $listText += ""
}
$listPath = Join-Path $env:TEMP ("jm-toolchains-清单-" + [guid]::NewGuid().ToString("N") + ".txt")
[IO.File]::WriteAllText($listPath, ($listText -join "`r`n"), (New-Object System.Text.UTF8Encoding($false)))

if ($CheckOnly) {
    Write-Host ""
    Write-Host "只校验，不打包。清单内容："
    Write-Host ($listText -join "`n")
    Remove-Item $listPath -Force -ErrorAction SilentlyContinue
    exit 0
}

if ($Out -eq "") {
    $stamp = $lock.updated
    $Out = Join-Path $root ("build\toolchains\jishi-toolchains-" + $stamp + ".zip")
}
$outDir = Split-Path -Parent $Out
if (-not (Test-Path $outDir)) { New-Item -ItemType Directory -Force -Path $outDir | Out-Null }
if (Test-Path $Out) { Remove-Item $Out -Force }

Write-Host ""
Write-Host ("打包到 " + $Out + " …（Level=" + $Level + "；工具链较大，可能需要几分钟）")
$compression = [System.IO.Compression.CompressionLevel]::$Level
$zip = [System.IO.Compression.ZipFile]::Open($Out, [System.IO.Compression.ZipArchiveMode]::Create)
try {
    foreach ($r in $resolved) {
        $base = $r.Dir.TrimEnd('\')
        Get-ChildItem -Path $base -Recurse -File | ForEach-Object {
            $rel = $_.FullName.Substring($base.Length + 1) -replace "\\", "/"
            $entry = $r.Name + "/" + $rel
            [void][System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
                $zip, $_.FullName, $entry, $compression)
        }
    }
    [void][System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
        $zip, $listPath, "清单.txt", $compression)
}
finally {
    $zip.Dispose()
    Remove-Item $listPath -Force -ErrorAction SilentlyContinue
}

$size = (Get-Item $Out).Length
Write-Host ("完成：" + $Out)
Write-Host ("大小：" + ("{0:N1} MB" -f ($size / 1MB)) + "（未压缩工具链合计 " + ("{0:N1} MB" -f (($resolved | ForEach-Object { (Get-ChildItem $_.Dir -Recurse -File | Measure-Object Length -Sum).Sum } | Measure-Object -Sum).Sum / 1MB)) + "）")
Write-Host "解压后请把根目录设为 JM_TOOLCHAIN_ROOT（必须纯 ASCII 路径），再运行 build.ps1。"
