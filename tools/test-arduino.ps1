# Arduino 库（LI025）的主机验收
#
# 覆盖：
#   1. tools/pack-arduino.ps1 能生成完整的库目录与 zip
#   2. library.properties 的关键字段、src 的同源文件与 jm_blob.c 长度
#   3. 用 Arduino 兼容桩把库当普通 C/C++ 编一遍（不是只看文件在不在）
#   4. 运行内嵌 .jmb，输出与 tests/expected 逐字节一致
#   5. li::run(jmb, len) 对坏字节码返回 -1 且 lastError() 有文案（由 main.cpp 检查）
#
# 本机验收不替代真机：不验证 STM32duino core 的时钟/串口初始化，也不验证烧录。

$ErrorActionPreference = "Continue"
$root = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot "toolchains.ps1")

$script:total = 0
$script:fail = 0

function Check([string]$name, [bool]$ok, [string]$detail) {
    $script:total++
    if ($ok) {
        Write-Host ("[通过] " + $name)
    }
    else {
        Write-Host ("[不通过] " + $name)
        if ($detail) { Write-Host $detail }
        $script:fail++
    }
}

if (-not $HostGcc) { throw "没有主机 gcc，无法验收 Arduino 库" }
$gxx = Join-Path (Split-Path $HostGcc) ("g++" + $ExeSuffix)
if (-not (Test-Path $gxx)) {
    $cmd = Get-Command "g++" -ErrorAction SilentlyContinue
    if (-not $cmd) { throw "没有主机 g++，无法编译 Arduino 库的 C++ 部分" }
    $gxx = $cmd.Source
}

$outRoot = Join-Path $root "build/arduino"
$pkg = Join-Path $PSScriptRoot "pack-arduino.ps1"

Write-Host "== 生成库"
$packOut = (& $pkg -Example "你好" 2>&1 | Out-String)
if ($LASTEXITCODE -ne 0) {
    Write-Host $packOut
    throw "pack-arduino.ps1 失败"
}

$lib = Join-Path $outRoot "LiJishim"
$src = Join-Path $lib "src"
$props = Join-Path $lib "library.properties"

Write-Host "== 1. 库目录与清单"
$propsText = if (Test-Path $props) { Get-Content $props -Raw -Encoding UTF8 } else { "" }
Check "library.properties 存在" (Test-Path $props) $props
Check "name=LiJishim" ($propsText -match "(?m)^name=LiJishim\r?$") $propsText
Check "architectures 含 stm32" ($propsText -match "(?m)^architectures=.*stm32") $propsText
Check "includes=LiJishim.h" ($propsText -match "(?m)^includes=LiJishim\.h\r?$") $propsText

$needC = @("jm_vm.c", "jm_heap.c", "jm_builtin.c", "jm_loader.c", "jm_error.c", "jm_native.c", "jm_debug.c")
$needH = @("jishim.h", "jm_port.h", "jm_internal.h", "LiJishim.h")
$needCpp = @("LiJishim.cpp", "jm_port_arduino.cpp")
$missing = @()
foreach ($f in ($needC + $needH + $needCpp + @("jm_blob.c"))) {
    if (-not (Test-Path (Join-Path $src $f))) { $missing += $f }
}
Check ("src 齐全（7 个 .c + 4 个头 + 2 个 .cpp + jm_blob.c）") ($missing.Count -eq 0) ($missing -join "、")
Check "示例草图存在" (Test-Path (Join-Path $lib "examples/Hello/Hello.ino")) "examples/Hello/Hello.ino"

# 运行时同源：拷进来的 .c 与 runtime/src 逐字节一致
$same = $true
foreach ($f in $needC) {
    $a = (Get-FileHash (Join-Path $root ("runtime/src/" + $f)) -Algorithm SHA256).Hash
    $b = (Get-FileHash (Join-Path $src $f) -Algorithm SHA256).Hash
    if ($a -ne $b) { $same = $false }
}
Check "运行时 .c 与 runtime/src 逐字节一致" $same "有文件不一致"

$blobText = Get-Content (Join-Path $src "jm_blob.c") -Raw -Encoding UTF8
$m = [regex]::Match($blobText, "jm_app_jmb_len = (\d+)u")
$jmbLen = (Get-Item (Join-Path $outRoot "app.jmb")).Length
Check "jm_blob.c 的内嵌长度等于 app.jmb" ($m.Success -and ([int]$m.Groups[1].Value -eq $jmbLen)) ("blob=" + $m.Groups[1].Value + " jmb=" + $jmbLen)

$zip = Get-ChildItem $outRoot -Filter "LiJishim-*.zip" -ErrorAction SilentlyContinue | Select-Object -First 1
Check "产出 zip" ($null -ne $zip) (Get-ChildItem $outRoot -Name | Out-String)

if ($zip) {
    $unzipDir = Join-Path ([IO.Path]::GetTempPath()) ("li-arduino-" + [guid]::NewGuid().ToString("N").Substring(0, 8))
    try {
        Expand-Archive -Path $zip.FullName -DestinationPath $unzipDir -Force
        $inZip = @(
            "LiJishim/library.properties",
            "LiJishim/src/jm_blob.c",
            "LiJishim/src/LiJishim.h",
            "LiJishim/examples/Hello/Hello.ino"
        )
        $absent = @($inZip | Where-Object { -not (Test-Path (Join-Path $unzipDir $_)) })
        Check "zip 内路径以 LiJishim/ 为根且关键文件齐全" ($absent.Count -eq 0) ($absent -join "、")
    }
    finally {
        Remove-Item $unzipDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}

Write-Host "== 2. 主机编译（Arduino 兼容桩）"
# MinGW 版 gcc 用 ANSI 文件 API：仓库路径含中文时，-o / -I / 源文件名都不能给绝对路径。
# 统一在仓库根目录下用相对路径，和 build.ps1 的做法一致。
$objDir = Join-Path $outRoot "obj"
if (Test-Path $objDir) { Remove-Item $objDir -Recurse -Force }
New-Item -ItemType Directory -Force -Path $objDir | Out-Null

$relSrc = "build/arduino/LiJishim/src"
$relObj = "build/arduino/obj"
$relStub = "tests/arduino/stub"

Push-Location $root
try {
    $objs = @()
    $compileOk = $true
    foreach ($c in Get-ChildItem $src -Filter "*.c") {
        $o = $relObj + "/" + $c.BaseName + ".o"
        & $HostGcc -std=c99 -Wall -Wextra -O2 -I $relSrc -c ($relSrc + "/" + $c.Name) -o $o 2>&1 | Out-String | ForEach-Object { if ($_) { Write-Host $_ } }
        if ($LASTEXITCODE -ne 0) { $compileOk = $false; continue }
        $objs += $o
    }
    Check "8 个 .c 编译通过（7 个运行时 + jm_blob.c）" ($compileOk -and $objs.Count -eq 8) ("成功 " + $objs.Count + " 个")

    $cppSources = @()
    $cppSources += (Get-ChildItem $src -Filter "*.cpp" | ForEach-Object { $relSrc + "/" + $_.Name })
    $cppSources += "tests/arduino/main.cpp"
    $cppSources += "tests/arduino/stub/Arduino.cpp"
    $cppOk = $true
    $i = 0
    foreach ($f in $cppSources) {
        $o = $relObj + "/cpp" + $i + ".o"
        $i++
        & $gxx -std=c++11 -Wall -Wextra -O2 -I $relSrc -I $relStub -c $f -o $o 2>&1 | Out-String | ForEach-Object { if ($_) { Write-Host $_ } }
        if ($LASTEXITCODE -ne 0) { $cppOk = $false; continue }
        $objs += $o
    }
    Check ("C++ 侧编译通过（" + $cppSources.Count + " 个文件：包装 + 端口 + 桩）") $cppOk ($cppSources -join "、")

    $relExe = "build/arduino/arduino-host" + $ExeSuffix
    $linkOk = $false
    # Windows 的 MinGW g++ 默认动态链接 libstdc++，直接跑会报 0xC0000135（找不到 DLL）；
    # 这里只在本机验收用，静态链接省掉 PATH 依赖。macOS 不支持 -static，不加。
    $linkFlags = @()
    if ($ExeSuffix -ne "") { $linkFlags += "-static" }
    if ($objs.Count -eq (8 + $cppSources.Count)) {
        & $gxx @objs @linkFlags -o $relExe 2>&1 | Out-String | ForEach-Object { if ($_) { Write-Host $_ } }
        $linkOk = ($LASTEXITCODE -eq 0) -and (Test-Path (Join-Path $root $relExe))
    }
    Check "链接成主机可执行文件" $linkOk $relExe
}
finally {
    Pop-Location
}

Write-Host "== 3. 运行并对基准"
if ($linkOk) {
    $actual = ((& (Join-Path $root $relExe) 2>&1 | Out-String).TrimEnd() -replace "`r`n", "`n")
    $code = $LASTEXITCODE
    $gold = ((Get-Content (Join-Path $root "tests/expected/你好.txt") -Raw -Encoding UTF8).TrimEnd() -replace "`r`n", "`n")
    Check "内嵌 .jmb 的运行输出与基准逐字节一致" (($code -eq 0) -and ($actual -eq $gold)) ("实际：" + $actual + "`n期望：" + $gold)
}
else {
    Check "内嵌 .jmb 的运行输出与基准逐字节一致" $false "没有可执行文件"
}

Write-Host ("Arduino 库验收：" + $script:total + " 项，失败 " + $script:fail + " 项")
if ($script:fail -gt 0) { exit 1 }
exit 0
