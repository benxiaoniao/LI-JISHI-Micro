# 下载并按锁定的哈希校验工具链，装进 ASCII 缓存目录
#
# 用法：
#   powershell -ExecutionPolicy Bypass -File tools\fetch-toolchain.ps1
#   powershell -ExecutionPolicy Bypass -File tools\fetch-toolchain.ps1 -Name host-gcc
#   powershell -ExecutionPolicy Bypass -File tools\fetch-toolchain.ps1 -Bundle   # 装进仓库（仅限 ASCII 路径的仓库）
#
# 默认装到 %LOCALAPPDATA%\jishi\toolchains\<名字>（可用 JM_TOOLCHAIN_ROOT 改）。
# 原因：MinGW 版 gcc/ld 用 ANSI 文件 API，工具链放在中文目录下会找不到 crti.o / -lgcc。
# 只支持 zip 形式的发布包；host-gcc 目前没有自动下载，按 docs/工具链.md 手动放置。

param(
    [string]$Name = "arm-none-eabi",
    [switch]$Force,
    [switch]$Bundle
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$toolchainDir = Join-Path $PSScriptRoot "toolchains"
$lockPath = Join-Path $toolchainDir "toolchain.lock.json"
if (-not (Test-Path $lockPath)) { throw ("缺少锁定文件：" + $lockPath) }

$lock = Get-Content -Path $lockPath -Raw -Encoding UTF8 | ConvertFrom-Json
$tc = $lock.toolchains.$Name
if (-not $tc) { throw ("锁定文件里没有工具链：" + $Name) }

$cacheRoot = if ($env:JM_TOOLCHAIN_ROOT) { $env:JM_TOOLCHAIN_ROOT } else { Join-Path $env:LOCALAPPDATA "jishi\toolchains" }
$target = if ($Bundle) { Join-Path $toolchainDir $Name } else { Join-Path $cacheRoot $Name }
if ((Test-Path $target) -and (-not $Force)) {
    throw ("目录已存在：" + $target + "；要重装请加 -Force")
}

if (-not $tc.archive.url) {
    throw ("$Name 没有自动下载地址。说明：" + $tc.archive.note)
}

$zipPath = Join-Path $env:TEMP ([System.IO.Path]::GetFileName($tc.archive.url))
Write-Host ("下载 " + $tc.archive.url)
Invoke-WebRequest -Uri $tc.archive.url -OutFile $zipPath -UseBasicParsing

$got = (Get-FileHash -Path $zipPath -Algorithm SHA256).Hash
if ($tc.archive.sha256) {
    if ($got -ne $tc.archive.sha256.ToUpper()) {
        throw ("压缩包哈希不符：期望 " + $tc.archive.sha256 + "，实际 " + $got)
    }
    Write-Host ("压缩包哈希 OK：" + $got)
} else {
    Write-Host ("压缩包哈希（发布方未提供校验文件，仅记录）：" + $got)
}

$extractRoot = Join-Path $env:TEMP ("jm-toolchain-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $extractRoot | Out-Null
Expand-Archive -Path $zipPath -DestinationPath $extractRoot -Force

# 发布包通常多一层目录，找到含 bin/<exe> 的那层
$exe = $tc.exe
$inner = Get-ChildItem -Path $extractRoot -Directory | Where-Object {
    Test-Path (Join-Path $_.FullName ("bin\" + $exe))
} | Select-Object -First 1
if (-not $inner) {
    if (Test-Path (Join-Path $extractRoot ("bin\" + $exe))) {
        $inner = Get-Item $extractRoot
    } else {
        throw ("解压后找不到 bin/" + $exe)
    }
}

if (Test-Path $target) { Remove-Item -Path $target -Recurse -Force }
New-Item -ItemType Directory -Path (Split-Path -Parent $target) -Force | Out-Null
Move-Item -Path $inner.FullName -Destination $target

# 按锁定清单逐个校验关键二进制
foreach ($f in $tc.files) {
    $p = Join-Path $target $f.path
    if (-not (Test-Path $p)) { throw ("缺少文件：" + $p) }
    $h = (Get-FileHash -Path $p -Algorithm SHA256).Hash
    if ($h -ne $f.sha256.ToUpper()) {
        throw ("哈希不符：" + $f.path + " 期望 " + $f.sha256 + " 实际 " + $h)
    }
    Write-Host ("  [OK] " + $f.path)
}

$ver = (& (Join-Path $target ("bin\" + $exe)) --version | Select-Object -First 1)
Write-Host ("安装完成：" + $target)
Write-Host ("版本：" + $ver)

Remove-Item -Path $extractRoot -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -Path $zipPath -Force -ErrorAction SilentlyContinue
