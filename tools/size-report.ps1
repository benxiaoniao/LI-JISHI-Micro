# 固件体积报告：Flash / RAM 实测、模块贡献、与基线对比
#
# 用法：
#   powershell -ExecutionPolicy Bypass -File tools\size-report.ps1
#   powershell -ExecutionPolicy Bypass -File tools\size-report.ps1 -WriteBaseline
#
# 数据来源：
#   - Flash / RAM 段：arm-none-eabi-size -A（.isr_vector/.text/.ARM.exidx/.data/.bss）
#   - Flash 整包：jishim.bin 的字节数（链接器实际要烧进去的镜像）
#   - RAM 实际占用：链接符号 _ebss - 0x20000000（含 .bss 末尾对齐）
#   - 模块贡献：arm-none-eabi-objdump --dwarf=info，按编译单元汇总函数地址区间
#   - RAM 大户：arm-none-eabi-nm --print-size
#   - 基线：tools\size-baseline.json（入库，供 CI 与本地对比）

param(
    [string]$Elf = "build\stm32\jishim.elf",
    [string]$Bin = "build\stm32\jishim.bin",
    [string]$Map = "build\stm32\jishim.map",
    [string]$Ld = "firmware\stm32f103\stm32f103c8.ld",
    [string]$Baseline = "tools\size-baseline.json",
    [string]$OutJson = "build\stm32\size-report.json",
    [string]$Label = "",
    # 保留调试段的 .jmb（build-firmware 生成），用来算调试段体积；缺失时该行显示「未覆盖」
    [string]$DebugJmb = "build\stm32\app.debug.jmb",
    [switch]$WriteBaseline
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot

function Get-ModuleGroup([string]$name) {
    if ($name -match '/jm_(vm|heap|builtin|loader|error|native)\.c$') { return "运行时" }
    if ($name -match '/jm_blob\.c$') { return "内嵌字节码" }
    if ($name -match '/jmk(_port_cm3)?\.(c|s)$') { return "内核 JMK" }
    if ($name -match '/startup_') { return "启动文件" }
    if ($name -match '^firmware/') { return "固件与板级" }
    return "其他/编译器辅助"
}

function Format-Delta([int64]$now, [int64]$before) {
    $d = $now - $before
    if ($d -gt 0) { return ("+" + $d) }
    if ($d -lt 0) { return ("-" + [Math]::Abs($d)) }
    return "+0"
}

Push-Location $root
try {
    . "$PSScriptRoot\toolchains.ps1"
    if (-not $ArmGcc) { throw "找不到 arm-none-eabi-gcc（先跑 tools\fetch-toolchain.ps1）" }
    $binDir = Split-Path $ArmGcc
    $sizeExe = Join-Path $binDir "arm-none-eabi-size.exe"
    $nmExe = Join-Path $binDir "arm-none-eabi-nm.exe"
    $objdumpExe = Join-Path $binDir "arm-none-eabi-objdump.exe"
    foreach ($p in @($sizeExe, $nmExe, $objdumpExe)) {
        if (-not (Test-Path $p)) { throw ("找不到工具：" + $p) }
    }
    if (-not (Test-Path $Elf)) { throw ("找不到 " + $Elf + "；先运行 tools\build-firmware.ps1") }

    # ---------- 芯片容量（从链接脚本读，避免手写漂移） ----------
    $flashLimit = 64 * 1024
    $ramLimit = 20 * 1024
    if (Test-Path $Ld) {
        $ldText = Get-Content $Ld -Raw -Encoding UTF8
        if ($ldText -match 'FLASH\s*\([^)]*\)\s*:\s*ORIGIN\s*=\s*0x[0-9A-Fa-f]+,\s*LENGTH\s*=\s*(\d+)K') {
            $flashLimit = [int]$Matches[1] * 1024
        }
        if ($ldText -match 'RAM\s*\([^)]*\)\s*:\s*ORIGIN\s*=\s*0x[0-9A-Fa-f]+,\s*LENGTH\s*=\s*(\d+)K') {
            $ramLimit = [int]$Matches[1] * 1024
        }
    }

    # ---------- 段体积 ----------
    $sections = @{}
    foreach ($line in (& $sizeExe -A $Elf)) {
        if ($line -match '^\s*(\.\S+)\s+(\d+)\s+(\d+)\s*$') {
            $sections[$Matches[1]] = @{ size = [int64]$Matches[2]; addr = [int64]$Matches[3] }
        }
    }
    $flashFromSections = 0
    foreach ($s in @(".isr_vector", ".text", ".ARM.exidx", ".data")) {
        if ($sections.ContainsKey($s)) { $flashFromSections += $sections[$s].size }
    }
    $flashUsed = if (Test-Path $Bin) { (Get-Item $Bin).Length } else { $flashFromSections }
    $ramFromSections = 0
    foreach ($s in @(".data", ".bss")) {
        if ($sections.ContainsKey($s)) { $ramFromSections += $sections[$s].size }
    }

    # ---------- 符号：RAM 占用与 _ebss ----------
    $symbols = @{}
    $ramSymbols = @{}
    foreach ($line in (& $nmExe --print-size --size-sort --radix=d $Elf)) {
        if ($line -match '^\s*(\d+)\s+(\d+)\s+([A-Za-z])\s+(.+?)\s*$') {
            $addr = [int64]$Matches[1]
            $size = [int64]$Matches[2]
            $type = $Matches[3]
            $name = $Matches[4]
            $symbols[$name] = @{ addr = $addr; size = $size; type = $type }
            if (($type -match '[bBdD]') -and ($size -gt 0)) { $ramSymbols[$name] = $size }
        }
    }
    $ramUsed = $ramFromSections
    $ramBase = [int64]0x20000000
    # nm --print-size 不列无尺寸的链接符号，边界符号单独取一次。
    $bounds = @{}
    foreach ($line in (& $nmExe $Elf)) {
        if ($line -match '^\s*([0-9a-fA-F]+)\s+([A-Za-z])\s+(\S+)\s*$') {
            $bounds[$Matches[3]] = [int64]::Parse($Matches[1], [Globalization.NumberStyles]::HexNumber)
        }
    }
    if ($bounds.ContainsKey("_ebss")) { $ramUsed = $bounds["_ebss"] - $ramBase }
    $stackHeadroom = 0
    if ($bounds.ContainsKey("_estack") -and $bounds.ContainsKey("_ebss")) {
        $stackHeadroom = $bounds["_estack"] - $bounds["_ebss"]
    }
    $appJmbBytes = 0
    if ($symbols.ContainsKey("jm_app_jmb")) { $appJmbBytes = $symbols["jm_app_jmb"].size }

    # ---------- .jmb 调试段：剥离前 / 剥离后 ----------
    # 剥离后以内嵌符号为准；剥离前取 build-firmware 另存的 app.debug.jmb（不进固件）。
    $jmbStripped = $appJmbBytes
    $jmbRaw = 0
    if (Test-Path $DebugJmb) { $jmbRaw = (Get-Item $DebugJmb).Length }
    $jmbDbgBytes = if ($jmbRaw -gt 0) { $jmbRaw - $jmbStripped } else { -1 }

    # ---------- 模块贡献：按 DWARF 编译单元汇总函数区间 ----------
    $modules = @{}
    $cuName = $null
    $cname = $null
    $tag = $null
    $low = $null
    $high = $null
    $lastWasCu = $false
    foreach ($line in (& $objdumpExe --dwarf=info $Elf 2>$null)) {
        if ($line -match '^\s*<\d+><[0-9a-f]+>:\s+Abbrev Number:') {
            # 上一个 DIE 结束：收尾
            if ($tag -eq "DW_TAG_compile_unit" -and $cname) {
                $cuName = $cname
                $lastWasCu = $true
            }
            elseif ($tag -eq "DW_TAG_subprogram" -and $cuName -and $null -ne $low -and $null -ne $high) {
                $size = if (($high -ge 0x08000000) -and ($high -gt $low)) { $high - $low } else { $high }
                if ($size -gt 0) {
                    if (-not $modules.ContainsKey($cuName)) { $modules[$cuName] = [int64]0 }
                    $modules[$cuName] += [int64]$size
                }
            }
            $cname = $null
            $low = $null
            $high = $null
            if ($line -match '\((DW_TAG_\w+)\)') { $tag = $Matches[1] } else { $tag = $null }
            continue
        }
        if ($line -match 'DW_AT_low_pc\s*:\s*(0x[0-9a-fA-F]+)') {
            $low = [int64]$Matches[1]
        }
        elseif ($line -match 'DW_AT_high_pc\s*:\s*(0x[0-9a-fA-F]+|\d+)') {
            $high = if ($Matches[1].StartsWith("0x")) { [int64]::Parse($Matches[1].Substring(2), [Globalization.NumberStyles]::HexNumber) } else { [int64]$Matches[1] }
        }
        elseif ($line -match 'DW_AT_name\s*:\s*(?:\(indirect string[^)]*\):\s*)?(.+?)\s*$') {
            if (-not $cname) {
                $cname = ($Matches[1] -replace '\\', '/')
                if ($cname.StartsWith("./")) { $cname = $cname.Substring(2) }
                if ($cname -match '/build-assets/') { $cname = "libgcc/" + (Split-Path -Leaf $cname) }
            }
        }
    }
    # 最后一个 DIE
    if ($tag -eq "DW_TAG_subprogram" -and $cuName -and $null -ne $low -and $null -ne $high) {
        $size = if (($high -ge 0x08000000) -and ($high -gt $low)) { $high - $low } else { $high }
        if ($size -gt 0) {
            if (-not $modules.ContainsKey($cuName)) { $modules[$cuName] = [int64]0 }
            $modules[$cuName] += [int64]$size
        }
    }

    $moduleTotal = 0
    foreach ($v in $modules.Values) { $moduleTotal += $v }
    $textSize = if ($sections.ContainsKey(".text")) { $sections[".text"].size } else { 0 }
    $textAddr = if ($sections.ContainsKey(".text")) { $sections[".text"].addr } else { [int64]0 }
    $textEnd = $textAddr + $textSize

    # ---------- 只读数据：链接脚本把 *(.rodata*) 并进 .text ----------
    $rodataBytes = 0
    if (Test-Path $Map) {
        $mapLines = Get-Content $Map
        $ranges = New-Object System.Collections.ArrayList
        for ($i = 0; $i -lt $mapLines.Count; $i++) {
            $l = $mapLines[$i]
            if ($l -match '^\s+(\.rodata\S*)\s+0x([0-9a-fA-F]+)\s+0x([0-9a-fA-F]+)\s+\S+') {
                $a = [int64]::Parse($Matches[2], [Globalization.NumberStyles]::HexNumber)
                $n = [int64]::Parse($Matches[3], [Globalization.NumberStyles]::HexNumber)
                [void]$ranges.Add([pscustomobject]@{ a = $a; e = $a + $n })
            }
            elseif (($l -match '^\s+(\.rodata\S*)\s*$') -and ($i + 1 -lt $mapLines.Count)) {
                if ($mapLines[$i + 1] -match '^\s+0x([0-9a-fA-F]+)\s+0x([0-9a-fA-F]+)') {
                    $a = [int64]::Parse($Matches[1], [Globalization.NumberStyles]::HexNumber)
                    $n = [int64]::Parse($Matches[2], [Globalization.NumberStyles]::HexNumber)
                    [void]$ranges.Add([pscustomobject]@{ a = $a; e = $a + $n })
                }
            }
        }
        # 合并重叠区间（字符串常量会被链接器折叠到同一地址），只统计落在 .text 里的部分。
        $cursor = [int64]0
        foreach ($r in ($ranges | Sort-Object a)) {
            $a = [Math]::Max($r.a, $textAddr)
            $e = [Math]::Min($r.e, $textEnd)
            if ($e -le $a) { continue }
            if ($e -le $cursor) { continue }
            if ($a -gt $cursor) { $rodataBytes += $e - $a }
            else { $rodataBytes += $e - $cursor }
            $cursor = $e
        }
    }
    $unattributed = $textSize - $moduleTotal - $rodataBytes

    # ---------- 输出：人读 ----------
    $labelText = if ($Label) { $Label } else { "（未指定）" }
    $flashPct = 100.0 * $flashUsed / $flashLimit
    $ramPct = 100.0 * $ramUsed / $ramLimit
    Write-Host "砾 固件体积报告"
    Write-Host ("  ELF      " + $Elf)
    Write-Host ("  标签     " + $labelText)
    Write-Host ("  Flash    {0} B / {1} B（{2:N2}%），剩余 {3} B" -f $flashUsed, $flashLimit, $flashPct, ($flashLimit - $flashUsed))
    Write-Host ("  RAM      {0} B / {1} B（{2:N2}%），剩余 {3} B" -f $ramUsed, $ramLimit, $ramPct, ($ramLimit - $ramUsed))
    Write-Host ("  段       .isr_vector {0}  .text {1}  .ARM.exidx {2}  .data {3}（加载镜像）" -f `
        $(if ($sections.ContainsKey(".isr_vector")) { $sections[".isr_vector"].size } else { 0 }), `
        $(if ($sections.ContainsKey(".text")) { $sections[".text"].size } else { 0 }), `
        $(if ($sections.ContainsKey(".ARM.exidx")) { $sections[".ARM.exidx"].size } else { 0 }), `
        $(if ($sections.ContainsKey(".data")) { $sections[".data"].size } else { 0 }))
    Write-Host ("  RAM 段   .data {0}  .bss {1}" -f `
        $(if ($sections.ContainsKey(".data")) { $sections[".data"].size } else { 0 }), `
        $(if ($sections.ContainsKey(".bss")) { $sections[".bss"].size } else { 0 }))
    if ($stackHeadroom -gt 0) {
        Write-Host ("  主栈余量 " + $stackHeadroom + " B（_estack - _ebss，链接脚本要求 ≥ 2048 B）")
    }
    Write-Host ""
    Write-Host "Flash 构成（.text 内，按来源）"
    Write-Host ("  代码（DWARF 函数）      {0} B" -f $moduleTotal)
    Write-Host ("  只读数据（.rodata*）    {0} B（其中内嵌 .jmb {1} B）" -f $rodataBytes, $appJmbBytes)
    Write-Host ("  汇编/链接器生成         {0} B" -f $unattributed)
    Write-Host ""
    Write-Host ".jmb 调试段（剥离前 / 剥离后，均不占固件 Flash）"
    if ($jmbRaw -gt 0) {
        Write-Host ("  剥离前（保留 DBG1）     {0} B" -f $jmbRaw)
        Write-Host ("  剥离后（内嵌固件）       {0} B" -f $jmbStripped)
        $dbgPct = 100.0 * $jmbDbgBytes / $jmbRaw
        Write-Host ("  调试段                  {0} B（占剥离前 {1:N1}%）" -f $jmbDbgBytes, $dbgPct)
    }
    else {
        Write-Host "  剥离前 / 调试段         未覆盖（缺 app.debug.jmb；先跑 tools\build-firmware.ps1）"
        Write-Host ("  剥离后（内嵌固件）       {0} B" -f $jmbStripped)
    }
    Write-Host ""
    Write-Host "模块贡献（代码，按 DWARF 编译单元）"
    Write-Host ("  {0,-42} {1,-12} {2,8}  {3,7}" -f "编译单元", "分组", "代码 B", "占比")
    foreach ($kv in ($modules.GetEnumerator() | Sort-Object Value -Descending)) {
        $pct = if ($textSize -gt 0) { 100.0 * $kv.Value / $textSize } else { 0 }
        Write-Host ("  {0,-42} {1,-12} {2,8}  {3,6:N1}%" -f $kv.Key, (Get-ModuleGroup $kv.Key), $kv.Value, $pct)
    }
    Write-Host ("  {0,-42} {1,-12} {2,8}" -f "合计（有调试信息的函数）", "", $moduleTotal)
    Write-Host ("  {0,-42} {1,-12} {2,8}" -f "汇编/链接器生成（无 DWARF）", "", $unattributed)
    Write-Host ""
    Write-Host "RAM 主要符号（nm）"
    $topRam = $ramSymbols.GetEnumerator() | Sort-Object Value -Descending | Select-Object -First 8
    foreach ($kv in $topRam) {
        Write-Host ("  {0,-32} {1,8} B" -f $kv.Key, $kv.Value)
    }
    $restCount = [Math]::Max(0, $ramSymbols.Count - $topRam.Count)
    $restBytes = 0
    foreach ($kv in ($ramSymbols.GetEnumerator() | Sort-Object Value -Descending | Select-Object -Skip 8)) {
        $restBytes += $kv.Value
    }
    Write-Host ("  其余 {0} 个符号 {1} B" -f $restCount, $restBytes)
    Write-Host ""

    # ---------- 快照 ----------
    $sectionSizes = [ordered]@{}
    foreach ($k in $sections.Keys) { $sectionSizes[$k] = $sections[$k].size }
    $snapshot = [ordered]@{
        schema = "jishi.size/1"
        generated = (Get-Date -Format "yyyy-MM-dd")
        label = $labelText
        flash = [ordered]@{ used = $flashUsed; limit = $flashLimit }
        ram = [ordered]@{ used = $ramUsed; limit = $ramLimit; stack_headroom = $stackHeadroom }
        text = [ordered]@{ code = $moduleTotal; rodata = $rodataBytes; other = $unattributed; app_jmb = $appJmbBytes }
        jmb_debug = [ordered]@{
            stripped = $jmbStripped
            raw = $jmbRaw
            section = $(if ($jmbDbgBytes -ge 0) { $jmbDbgBytes } else { $null })
        }
        sections = $sectionSizes
        modules = $modules
        ram_symbols = $ramSymbols
    }
    function Write-JsonFile([string]$path, $obj) {
        $json = ($obj | ConvertTo-Json -Depth 8) -replace "`r`n", "`n"
        $dir = Split-Path -Parent $path
        if ($dir -and -not (Test-Path $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
        [System.IO.File]::WriteAllText((Join-Path $root $path), $json + "`n", (New-Object System.Text.UTF8Encoding($false)))
    }

    if ($WriteBaseline) {
        Write-JsonFile $Baseline $snapshot
        Write-Host ("已写基线：" + $Baseline)
    }
    else {
        Write-JsonFile $OutJson $snapshot
        Write-Host ("快照：" + $OutJson)
        if (Test-Path $Baseline) {
            $before = Get-Content $Baseline -Raw -Encoding UTF8 | ConvertFrom-Json
            Write-Host ""
            Write-Host ("基线对比（" + $Baseline + "）")
            Write-Host ("  {0,-34} {1,9} {2,9} {3,9}" -f "项目", "本次", "基线", "增减")
            Write-Host ("  {0,-34} {1,9} {2,9} {3,9}" -f "Flash 整包", $flashUsed, $before.flash.used, (Format-Delta $flashUsed $before.flash.used))
            Write-Host ("  {0,-34} {1,9} {2,9} {3,9}" -f "RAM 实际占用", $ramUsed, $before.ram.used, (Format-Delta $ramUsed $before.ram.used))
            if ($before.text) {
                Write-Host ("  {0,-34} {1,9} {2,9} {3,9}" -f "代码（DWARF 函数）", $moduleTotal, $before.text.code, (Format-Delta $moduleTotal $before.text.code))
                Write-Host ("  {0,-34} {1,9} {2,9} {3,9}" -f "只读数据（.rodata*）", $rodataBytes, $before.text.rodata, (Format-Delta $rodataBytes $before.text.rodata))
                Write-Host ("  {0,-34} {1,9} {2,9} {3,9}" -f "内嵌 .jmb", $appJmbBytes, $before.text.app_jmb, (Format-Delta $appJmbBytes $before.text.app_jmb))
            }
            if ($before.jmb_debug -and $jmbRaw -gt 0) {
                $oldRaw = if ($null -ne $before.jmb_debug.raw) { [int64]$before.jmb_debug.raw } else { [int64]0 }
                Write-Host ("  {0,-34} {1,9} {2,9} {3,9}" -f ".jmb 剥离前", $jmbRaw, $oldRaw, (Format-Delta $jmbRaw $oldRaw))
                Write-Host ("  {0,-34} {1,9} {2,9} {3,9}" -f ".jmb 调试段", $jmbDbgBytes, ([int64]$oldRaw - [int64]$before.jmb_debug.stripped), (Format-Delta $jmbDbgBytes ([int64]$oldRaw - [int64]$before.jmb_debug.stripped)))
            }
            $beforeModules = @{}
            if ($before.modules) {
                foreach ($p in $before.modules.PSObject.Properties) { $beforeModules[$p.Name] = [int64]$p.Value }
            }
            $allModules = @{}
            foreach ($k in $modules.Keys) { $allModules[$k] = $modules[$k] }
            foreach ($k in $beforeModules.Keys) {
                if (-not $allModules.ContainsKey($k)) { $allModules[$k] = [int64]0 }
            }
            foreach ($kv in ($allModules.GetEnumerator() | Sort-Object Value -Descending)) {
                $old = if ($beforeModules.ContainsKey($kv.Key)) { $beforeModules[$kv.Key] } else { [int64]0 }
                Write-Host ("  {0,-34} {1,9} {2,9} {3,9}" -f ("代码 " + $kv.Key), $kv.Value, $old, (Format-Delta $kv.Value $old))
            }
            $beforeRam = @{}
            if ($before.ram_symbols) {
                foreach ($p in $before.ram_symbols.PSObject.Properties) { $beforeRam[$p.Name] = [int64]$p.Value }
            }
            foreach ($kv in $topRam) {
                $old = if ($beforeRam.ContainsKey($kv.Key)) { $beforeRam[$kv.Key] } else { [int64]0 }
                Write-Host ("  {0,-34} {1,9} {2,9} {3,9}" -f ("RAM " + $kv.Key), $kv.Value, $old, (Format-Delta $kv.Value $old))
            }
        }
        else {
            Write-Host ("（还没有基线；运行 tools\size-report.ps1 -WriteBaseline 生成 " + $Baseline + "）")
        }
    }
}
finally {
    Pop-Location
}
