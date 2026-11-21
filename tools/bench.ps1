# LI042 主机基准：编译 tests/bench/*.jsh，用 `jmrun --统计` 采指令数与耗时。
# 输出可直接对比优化前后；不做绝对性能承诺，只保证同一台机器上的相对变化。
param([int]$Runs = 3)

$ErrorActionPreference = "Continue"
$root = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot "toolchains.ps1")
$micc = Join-Path $root ("compiler/target/release/micc" + $ExeSuffix)
$jmrun = Join-Path $root ("build/jmrun" + $ExeSuffix)

if (-not (Test-Path $micc)) { throw "先运行 tools/build.ps1（缺 micc）" }
if (-not (Test-Path $jmrun)) { throw "先运行 tools/build.ps1（缺 jmrun）" }

$benchDir = Join-Path $root "tests/bench"
$outDir = Join-Path $root "build/bench"
New-Item -ItemType Directory -Path $outDir -Force | Out-Null

Push-Location $root
try {
    Write-Host ("基准：每个用例跑 " + $Runs + " 次，取最快一次（含进程启动）。")
    foreach ($file in (Get-ChildItem $benchDir -Filter "*.jsh" | Sort-Object Name)) {
        $name = $file.BaseName
        $jmb = Join-Path $outDir ($name + ".jmb")
        & $micc $file.FullName -o $jmb | Out-Null
        if ($LASTEXITCODE -ne 0) { throw ("编译失败：" + $file.Name) }
        $bestMs = $null
        $bestInstr = 0
        $bestMips = 0
        $errFile = Join-Path $outDir ($name + ".stat.txt")
        for ($i = 0; $i -lt $Runs; $i++) {
            & $jmrun --统计 $jmb 2>$errFile | Out-Null
            $code = $LASTEXITCODE
            $text = ([IO.File]::ReadAllText($errFile, [Text.Encoding]::UTF8) -replace "`r`n", "`n")
            if ($code -ne 0) { throw ("运行失败：" + $text) }
            $m = [regex]::Match($text, "\[统计\] 指令 (\d+) 耗时 ([\d.]+) ms 吞吐 ([\d.]+)")
            if (-not $m.Success) { throw ("统计输出不符合预期：" + $text) }
            $instr = [int64]$m.Groups[1].Value
            $ms = [double]$m.Groups[2].Value
            $mips = [double]$m.Groups[3].Value
            if (($null -eq $bestMs) -or ($ms -lt $bestMs)) {
                $bestMs = $ms
                $bestInstr = $instr
                $bestMips = $mips
            }
        }
        Write-Host ("[基准] {0} 指令 {1} 耗时 {2:N3} ms 吞吐 {3:N2} 百万指令/秒" -f `
            $name, $bestInstr, $bestMs, $bestMips)
    }
}
finally {
    Pop-Location
}
