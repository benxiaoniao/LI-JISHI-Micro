# 包（H4 最小可用切分）回归：源码包 + 单 .jmb。
# 所有夹具都建在 $env:TEMP 下的临时目录，跑完删除，不污染仓库。
$ErrorActionPreference = "Continue"
$root = Split-Path -Parent $PSScriptRoot
$micc = "compiler\target\release\micc.exe"
$jmrun = "build\jmrun.exe"

Push-Location $root
try {
    if (-not (Test-Path $micc)) { throw "先运行 tools\build.ps1" }
    if (-not (Test-Path $jmrun)) { throw "先运行 tools\build.ps1（缺 build\jmrun.exe）" }

    $sandbox = Join-Path $env:TEMP ("li-pack-" + [guid]::NewGuid().ToString("N").Substring(0, 8))
    New-Item -ItemType Directory -Path $sandbox | Out-Null
    $script:fail = 0
    $script:total = 0
    $script:skip = 0

    # 写一个包：砾包.toml + 主.jsh。故意带 UTF-8 BOM，顺带覆盖 BOM 处理。
    function New-Pkg([string]$dir, [string]$toml, [string]$src) {
        New-Item -ItemType Directory -Path $dir -Force | Out-Null
        $enc = New-Object System.Text.UTF8Encoding($true)
        [IO.File]::WriteAllText((Join-Path $dir "砾包.toml"), $toml, $enc)
        [IO.File]::WriteAllText((Join-Path $dir "主.jsh"), $src, $enc)
    }

    function Invoke-Pack([string]$dir, [switch]$Offline) {
        if ($Offline) {
            $o = (& $micc pack --目录 $dir --离线 2>&1 | Out-String)
        }
        else {
            $o = (& $micc pack --目录 $dir 2>&1 | Out-String)
        }
        return @{ code = $LASTEXITCODE; out = $o }
    }

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

    function Check-Error([string]$name, $r, [string]$code, [string]$needle) {
        $ok = ($r.code -ne 0) -and ($r.out -match [regex]::Escape($code)) -and $r.out.Contains($needle)
        Check $name $ok ("期望 " + $code + " 且包含「" + $needle + "」，实际退出码 " + $r.code + "：`n" + $r.out)
    }

    function Run-Jmb([string]$jmb) {
        if (-not (Test-Path $jmb)) { return "<缺少 jmb>" }
        $o = (& $jmrun $jmb 2>&1 | Out-String).TrimEnd()
        return ($o -replace "`r`n", "`n")
    }

    # ---------- 1. 单包 ----------
    $d = Join-Path $sandbox "单包"
    $toml = @"
[包]
名字 = "单包"
版本 = "1.0.0"
"@
    $src = @"
打印("单包", 1 + 2)
"@
    New-Pkg $d $toml $src
    $r = Invoke-Pack $d
    $jmb = Join-Path $d "单包.jmb"
    $out = Run-Jmb $jmb
    Check "单包：打包并运行" (($r.code -eq 0) -and ($out -eq "单包 3") -and (Test-Path $jmb)) ($r.out + "`n实际输出：" + $out)

    # ---------- 2. 双包依赖 ----------
    $root2 = Join-Path $sandbox "双包"
    $tool = Join-Path $root2 "工具"
    $app = Join-Path $root2 "应用"
    $toml = @"
[包]
名字 = "工具"
版本 = "0.1.0"

[导出]
名字 = ["加一", "问候"]
"@
    $src = @"
函数 加一(数)：
    返回 数 + 1

函数 问候(名)：
    返回 "你好，" + 名

令 私有 = 99
"@
    New-Pkg $tool $toml $src
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
工具 = { 路径 = "../工具", 版本 = "^0.1.0" }
"@
    $src = @"
令 结果 = 工具.加一(41)
打印(工具.问候("砾"))
打印(结果)
"@
    New-Pkg $app $toml $src
    $r = Invoke-Pack $app
    $out = Run-Jmb (Join-Path $app "应用.jmb")
    Check "双包依赖：源码级合并并运行" (($r.code -eq 0) -and ($out -eq "你好，砾`n42") -and $r.out.Contains("共 1 个")) ($r.out + "`n实际输出：" + $out)

    # ---------- 3. 同名字共存（不同包同名，不串） ----------
    $root3 = Join-Path $sandbox "同名"
    $toml = @"
[包]
名字 = "甲"
版本 = "1.0.0"

[导出]
名字 = ["名字"]
"@
    $src = @"
函数 名字()：
    返回 "甲"
"@
    New-Pkg (Join-Path $root3 "甲") $toml $src
    $toml = @"
[包]
名字 = "乙"
版本 = "1.0.0"

[导出]
名字 = ["名字"]
"@
    $src = @"
函数 名字()：
    返回 "乙"
"@
    New-Pkg (Join-Path $root3 "乙") $toml $src
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
甲 = { 路径 = "../甲", 版本 = "*" }
乙 = { 路径 = "../乙", 版本 = "*" }
"@
    $src = @"
打印(甲.名字(), 乙.名字())
"@
    New-Pkg (Join-Path $root3 "应用") $toml $src
    $r = Invoke-Pack (Join-Path $root3 "应用")
    $out = Run-Jmb (Join-Path $root3 "应用\应用.jmb")
    Check "同名函数在不同包内共存" (($r.code -eq 0) -and ($out -eq "甲 乙")) ($r.out + "`n实际输出：" + $out)

    # ---------- 4. 菱形依赖（基础只装一份） ----------
    $root4 = Join-Path $sandbox "菱形"
    $toml = @"
[包]
名字 = "基础"
版本 = "1.2.3"

[导出]
名字 = ["值"]
"@
    $src = @"
函数 值()：
    返回 7
"@
    New-Pkg (Join-Path $root4 "基础") $toml $src
    foreach ($side in @("左", "右")) {
        $toml = @"
[包]
名字 = "$side"
版本 = "1.0.0"

[依赖]
基础 = { 路径 = "../基础", 版本 = "^1.2.0" }

[导出]
名字 = ["取"]
"@
        $src = @"
函数 取()：
    返回 基础.值()
"@
        New-Pkg (Join-Path $root4 $side) $toml $src
    }
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
左 = { 路径 = "../左", 版本 = "^1.0.0" }
右 = { 路径 = "../右", 版本 = "^1.0.0" }
"@
    $src = @"
打印(左.取(), 右.取())
"@
    New-Pkg (Join-Path $root4 "应用") $toml $src
    $r = Invoke-Pack (Join-Path $root4 "应用")
    $out = Run-Jmb (Join-Path $root4 "应用\应用.jmb")
    Check "菱形依赖：基础只解析一份（共 3 个）" (($r.code -eq 0) -and $r.out.Contains("共 3 个") -and ($out -eq "7 7")) ($r.out + "`n实际输出：" + $out)

    # ---------- 5. 名字表接近上限 ----------
    $root5 = Join-Path $sandbox "名字表接近"
    $toml = @"
[包]
名字 = "工具"
版本 = "1.0.0"
"@
    $sb = New-Object System.Text.StringBuilder
    for ($i = 1; $i -le 240; $i++) {
        [void]$sb.AppendLine("令 g" + $i.ToString("000") + " = 1")
    }
    New-Pkg (Join-Path $root5 "工具") $toml $sb.ToString()
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
工具 = { 路径 = "../工具", 版本 = "^1.0.0" }
"@
    $src = @"
打印("接近上限")
"@
    New-Pkg (Join-Path $root5 "应用") $toml $src
    $r = Invoke-Pack (Join-Path $root5 "应用")
    $m = [regex]::Match($r.out, "名字表：(\d+) / 255")
    $n = if ($m.Success) { [int]$m.Groups[1].Value } else { 0 }
    Check ("名字表接近上限：" + $n + " / 255 仍可打包") (($r.code -eq 0) -and ($n -ge 200) -and ($n -le 255)) $r.out

    # ---------- 6. 名字表超过 255（E124 并指出占用最多的包） ----------
    $root6 = Join-Path $sandbox "名字表超限"
    $toml = @"
[包]
名字 = "工具"
版本 = "1.0.0"
"@
    $sb = New-Object System.Text.StringBuilder
    for ($i = 1; $i -le 260; $i++) {
        [void]$sb.AppendLine("令 g" + $i.ToString("000") + " = 1")
    }
    New-Pkg (Join-Path $root6 "工具") $toml $sb.ToString()
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
工具 = { 路径 = "../工具", 版本 = "^1.0.0" }
"@
    New-Pkg (Join-Path $root6 "应用") $toml $src
    $r = Invoke-Pack (Join-Path $root6 "应用")
    Check-Error "名字表超限：E124 + 占用最多的包" $r "E124" "占用最多的是：工具"

    # ---------- 7. native ABI 不一致（拒绝） ----------
    $root7 = Join-Path $sandbox "ABI"
    $tomlBad = @"
[包]
名字 = "工具"
版本 = "1.0.0"
原生ABI = 2
"@
    $src = @"
令 值 = 1
"@
    New-Pkg (Join-Path $root7 "工具") $tomlBad $src
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
工具 = { 路径 = "../工具", 版本 = "^1.0.0" }
"@
    New-Pkg (Join-Path $root7 "应用") $toml $src
    $r = Invoke-Pack (Join-Path $root7 "应用")
    Check-Error "native ABI 不一致：E129 拒绝" $r "E129" "ABI"

    # ---------- 8. native 需求并集（依赖声明的模块对根包生效） ----------
    $root8 = Join-Path $sandbox "原生并集"
    $toml = @"
[包]
名字 = "工具"
版本 = "1.0.0"
原生 = ["引脚"]
"@
    $src = @"
导入 引脚

函数 亮()：
    引脚写(13, 0)
"@
    New-Pkg (Join-Path $root8 "工具") $toml $src
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
工具 = { 路径 = "../工具", 版本 = "^1.0.0" }
"@
    $src = @"
打印("native 并集")
"@
    New-Pkg (Join-Path $root8 "应用") $toml $src
    $r = Invoke-Pack (Join-Path $root8 "应用")
    Check "native 并集：依赖声明的模块被合并" (($r.code -eq 0) -and $r.out.Contains("原生并集：引脚")) $r.out

    # ---------- 9. 依赖包顶层出现可执行语句（E127） ----------
    $root9 = Join-Path $sandbox "顶层语句"
    $toml = @"
[包]
名字 = "工具"
版本 = "1.0.0"
"@
    $src = @"
打印("我是可执行语句")
"@
    New-Pkg (Join-Path $root9 "工具") $toml $src
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
工具 = { 路径 = "../工具", 版本 = "^1.0.0" }
"@
    $src = @"
打印("根包")
"@
    New-Pkg (Join-Path $root9 "应用") $toml $src
    $r = Invoke-Pack (Join-Path $root9 "应用")
    Check-Error "依赖包顶层可执行语句：E127" $r "E127" "依赖包"

    # ---------- 10. 引用未导出名字（E128） ----------
    $root10 = Join-Path $sandbox "未导出"
    $toml = @"
[包]
名字 = "工具"
版本 = "1.0.0"

[导出]
名字 = ["加一"]
"@
    $src = @"
函数 加一(数)：
    返回 数 + 1

令 私有 = 99
"@
    New-Pkg (Join-Path $root10 "工具") $toml $src
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
工具 = { 路径 = "../工具", 版本 = "^1.0.0" }
"@
    $src = @"
打印(工具.私有)
"@
    New-Pkg (Join-Path $root10 "应用") $toml $src
    $r = Invoke-Pack (Join-Path $root10 "应用")
    Check-Error "引用未导出名字：E128" $r "E128" "没有导出"

    # ---------- 11. 堆预算超限（E130） ----------
    $root11 = Join-Path $sandbox "堆预算"
    $zeros = (0..254 | ForEach-Object { "0" }) -join ", "
    $sb = New-Object System.Text.StringBuilder
    foreach ($v in @("a", "b", "c", "d", "e")) {
        [void]$sb.AppendLine("令 $v = [$zeros]")
    }
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"
"@
    New-Pkg $root11 $toml $sb.ToString()
    $r = Invoke-Pack $root11
    Check-Error "堆预算超限：E130" $r "E130" "堆下界"

    # ---------- 12. .jmb 体积超限（E130） ----------
    $root12 = Join-Path $sandbox "字节码上限"
    $sb = New-Object System.Text.StringBuilder
    for ($i = 0; $i -lt 60; $i++) {
        [void]$sb.AppendLine('打印("abcdefghij")')
    }
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"
字节码上限 = 100
"@
    New-Pkg $root12 $toml $sb.ToString()
    $r = Invoke-Pack $root12
    Check-Error "字节码上限超限：E130" $r "E130" "字节码上限"

    # ---------- 13. 离线缺包（没装过，E126） ----------
    $root13 = Join-Path $sandbox "离线缺包"
    $toml = @"
[包]
名字 = "工具"
版本 = "1.0.0"
"@
    $src = @"
令 值 = 1
"@
    New-Pkg (Join-Path $root13 "工具") $toml $src
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
工具 = { 路径 = "../工具", 版本 = "^1.0.0" }
"@
    New-Pkg (Join-Path $root13 "应用") $toml $src
    $r = Invoke-Pack (Join-Path $root13 "应用") -Offline
    Check-Error "离线缺包：E126" $r "E126" "锁文件"

    # ---------- 14. 版本冲突：单版本约束 + 来源链 ----------
    $root14 = Join-Path $sandbox "版本冲突"
    $toml = @"
[包]
名字 = "基础"
版本 = "0.1.0"
"@
    $src = @"
函数 值()：
    返回 1
"@
    New-Pkg (Join-Path $root14 "基础") $toml $src
    $toml = @"
[包]
名字 = "左"
版本 = "1.0.0"

[依赖]
基础 = { 路径 = "../基础", 版本 = "^0.1.0" }
"@
    New-Pkg (Join-Path $root14 "左") $toml $src
    $toml = @"
[包]
名字 = "右"
版本 = "1.0.0"

[依赖]
基础 = { 路径 = "../基础", 版本 = "^0.2.0" }
"@
    New-Pkg (Join-Path $root14 "右") $toml $src
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
左 = { 路径 = "../左", 版本 = "^1.0.0" }
右 = { 路径 = "../右", 版本 = "^1.0.0" }
"@
    New-Pkg (Join-Path $root14 "应用") $toml $src
    $r = Invoke-Pack (Join-Path $root14 "应用")
    Check-Error "版本冲突：E126 + 来源链" $r "E126" "来源链"

    # ---------- 15. install + 离线打包（只读锁与缓存） ----------
    $installDir = Join-Path $root2 "应用"
    $io = (& $micc install --目录 $installDir 2>&1 | Out-String)
    $installed = ($LASTEXITCODE -eq 0) -and (Test-Path (Join-Path $installDir ".砾\锁.toml")) -and (Test-Path (Join-Path $installDir ".砾\包\工具-0.1.0\主.jsh"))
    $r = Invoke-Pack $installDir -Offline
    $out = Run-Jmb (Join-Path $installDir "应用.jmb")
    Check "install + 离线打包可运行" ($installed -and ($r.code -eq 0) -and ($r.out.Contains("离线")) -and ($out -eq "你好，砾`n42")) ($io + $r.out + "`n实际输出：" + $out)

    # ---------- 16. 离线缓存被改坏：校验拒绝 ----------
    $cached = Join-Path $installDir ".砾\包\工具-0.1.0\主.jsh"
    [IO.File]::WriteAllText($cached, "函数 加一(数)：`r`n    返回 数 + 2`r`n", (New-Object System.Text.UTF8Encoding($true)))
    $r = Invoke-Pack $installDir -Offline
    Check-Error "离线缓存被改坏：E126 校验失败" $r "E126" "校验"

    # ---------- 17. 仓库内示例包（examples/packages） ----------
    $exampleDir = Join-Path $root "examples\packages\应用"
    $exampleJmb = Join-Path $sandbox "示例.jmb"
    $o = (& $micc pack --目录 $exampleDir -o $exampleJmb 2>&1 | Out-String)
    $code = $LASTEXITCODE
    $out = Run-Jmb $exampleJmb
    Check "仓库示例包 examples/packages：打包并运行" (($code -eq 0) -and ($out -eq "你好，砾`n42")) ($o + "`n实际输出：" + $out)

    # ---------- 18. 静态索引：按 ^ 约束挑版本（1.2.0），install 后在线/离线都可打包 ----------
    $idxRoot = Join-Path $sandbox "索引"
    $artifact = Join-Path $idxRoot "工件"
    foreach ($v in @("1.0.0", "1.2.0", "2.0.0")) {
        $toml = @"
[包]
名字 = "工具"
版本 = "$v"

[导出]
名字 = ["版本"]
"@
        $src = @"
函数 版本()：
    返回 "$v"
"@
        New-Pkg (Join-Path $artifact "工具-$v") $toml $src
    }
    function Get-PkgHash([string]$dir) {
        $o = (& $micc hash --目录 $dir 2>&1 | Out-String).Trim()
        if ($LASTEXITCODE -ne 0) { throw ("micc hash 失败：" + $o) }
        return $o
    }
    $h100 = Get-PkgHash (Join-Path $artifact "工具-1.0.0")
    $h120 = Get-PkgHash (Join-Path $artifact "工具-1.2.0")
    $h200 = Get-PkgHash (Join-Path $artifact "工具-2.0.0")
    $idxFile = Join-Path $idxRoot "砾索引.toml"
    $idxText = @"
# 静态索引夹具：来源相对本文件
索引版本 = 1

[包]
"工具@1.0.0" = { 来源 = "file:工件/工具-1.0.0", 校验 = "$h100" }
"工具@1.2.0" = { 来源 = "file:工件/工具-1.2.0", 校验 = "$h120" }
"工具@2.0.0" = { 来源 = "file:工件/工具-2.0.0", 校验 = "$h200" }
"@
    [IO.File]::WriteAllText($idxFile, $idxText, (New-Object System.Text.UTF8Encoding($false)))

    $app18 = Join-Path $idxRoot "应用"
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
工具 = { 版本 = "^1.0.0" }
"@
    New-Pkg $app18 $toml '打印(工具.版本())'
    $io = (& $micc install --目录 $app18 --索引 $idxFile 2>&1 | Out-String)
    $installCode = $LASTEXITCODE
    $lockText = [IO.File]::ReadAllText((Join-Path $app18 ".砾\锁.toml"))
    $rOff = Invoke-Pack $app18 -Offline
    $outOff = Run-Jmb (Join-Path $app18 "应用.jmb")
    $rOn = Invoke-Pack $app18
    $outOn = Run-Jmb (Join-Path $app18 "应用.jmb")
    Check "索引：^1.0.0 选中 1.2.0，install 后在线/离线都可运行" (($installCode -eq 0) -and $lockText.Contains('版本 = "1.2.0"') -and ($rOff.code -eq 0) -and ($rOn.code -eq 0) -and ($outOff -eq "1.2.0") -and ($outOn -eq "1.2.0")) ($io + $rOff.out + $rOn.out + "`n离线输出：" + $outOff + "`n在线输出：" + $outOn)

    # ---------- 19. 静态索引：* 取最高版本（2.0.0） ----------
    $app19 = Join-Path $idxRoot "应用最大"
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
工具 = { 版本 = "*" }
"@
    New-Pkg $app19 $toml '打印(工具.版本())'
    $io = (& $micc install --目录 $app19 --索引 $idxFile 2>&1 | Out-String)
    $installCode = $LASTEXITCODE
    $lockText = [IO.File]::ReadAllText((Join-Path $app19 ".砾\锁.toml"))
    $r = Invoke-Pack $app19 -Offline
    $out = Run-Jmb (Join-Path $app19 "应用.jmb")
    Check "索引：* 选中最高版本 2.0.0" (($installCode -eq 0) -and $lockText.Contains('版本 = "2.0.0"') -and ($r.code -eq 0) -and ($out -eq "2.0.0")) ($io + $r.out + "`n实际输出：" + $out)

    # ---------- 20. 静态索引：没有满足约束的版本（E134） ----------
    $app20 = Join-Path $idxRoot "应用缺版本"
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
工具 = { 版本 = "^1.3.0" }
"@
    New-Pkg $app20 $toml '打印(工具.版本())'
    $io = (& $micc install --目录 $app20 --索引 $idxFile 2>&1 | Out-String)
    $installCode = $LASTEXITCODE
    Check-Error "索引：没有满足 ^1.3.0 的版本 → E134" @{ code = $installCode; out = $io } "E134" "没有满足"

    # ---------- 21. 静态索引：索引里没有这个包（E133） ----------
    $app21 = Join-Path $idxRoot "应用缺包"
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
无此包 = { 版本 = "*" }
"@
    New-Pkg $app21 $toml '打印(1)'
    $io = (& $micc install --目录 $app21 --索引 $idxFile 2>&1 | Out-String)
    $installCode = $LASTEXITCODE
    Check-Error "索引：包不存在 → E133" @{ code = $installCode; out = $io } "E133" "索引里没有包"

    # ---------- 22. 静态索引：内容哈希不符（E136） ----------
    $badIdx = Join-Path $idxRoot "砾索引-坏校验.toml"
    $badText = @"
索引版本 = 1

[包]
"工具@1.0.0" = { 来源 = "file:工件/工具-1.0.0", 校验 = "fnv1a64:0000000000000000" }
"@
    [IO.File]::WriteAllText($badIdx, $badText, (New-Object System.Text.UTF8Encoding($false)))
    $app22 = Join-Path $idxRoot "应用坏校验"
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
工具 = { 版本 = "1.0.0" }
"@
    New-Pkg $app22 $toml '打印(工具.版本())'
    $io = (& $micc install --目录 $app22 --索引 $badIdx 2>&1 | Out-String)
    $installCode = $LASTEXITCODE
    Check-Error "索引：哈希不符 → E136" @{ code = $installCode; out = $io } "E136" "不一致"

    # ---------- 23. HTTP 静态索引：真 curl 下载 + tar 解包 + 哈希校验 + 打包运行 ----------
    # 本机起 python -m http.server；起不来（没有 python / 没端口 / 2 秒内没就绪）就显式跳过，
    # 不拿本地目录冒充 HTTP 证据。
    $pyExe = Get-Command python -ErrorAction SilentlyContinue
    if ($null -eq $pyExe) {
        Write-Host "[跳过] HTTP 索引：本机没有 python，无法起静态服务"
        $script:skip++
    }
    else {
        $httpRoot = Join-Path $sandbox "http"
        $pkgSrc = Join-Path $httpRoot "工具-1.0.0"
        $toml = @"
[包]
名字 = "工具"
版本 = "1.0.0"

[导出]
名字 = ["版本"]
"@
        $srcHttp = @"
函数 版本()：
    返回 "1.0.0"
"@
        New-Pkg $pkgSrc $toml $srcHttp
        $h = Get-PkgHash $pkgSrc
        Compress-Archive -Path (Join-Path $pkgSrc "*") -DestinationPath (Join-Path $httpRoot "tool-1.0.0.zip") -Force
        $zipBytes = (Get-Item (Join-Path $httpRoot "tool-1.0.0.zip")).Length

        $port = 0
        try {
            $probe = New-Object System.Net.Sockets.TcpListener -ArgumentList @([Net.IPAddress]::Loopback, 0)
            $probe.Start()
            $port = $probe.LocalEndpoint.Port
            $probe.Stop()
        }
        catch {
            Write-Host "[跳过] HTTP 索引：挑不到空闲端口"
            $script:skip++
        }

        if ($port -gt 0) {
            $idxHttp = @"
索引版本 = 1

[包]
"工具@1.0.0" = { 来源 = "http://127.0.0.1:$port/tool-1.0.0.zip", 校验 = "$h" }
"@
            [IO.File]::WriteAllText((Join-Path $httpRoot "index.toml"), $idxHttp, (New-Object System.Text.UTF8Encoding($false)))

            $srv = $null
            try {
                $srv = Start-Process -FilePath $pyExe.Source -ArgumentList @("-m", "http.server", "$port", "--bind", "127.0.0.1", "--directory", $httpRoot) -WindowStyle Hidden -PassThru
                $ready = $false
                for ($i = 0; $i -lt 20; $i++) {
                    Start-Sleep -Milliseconds 250
                    try {
                        $c = New-Object System.Net.Sockets.TcpClient
                        $c.Connect("127.0.0.1", $port)
                        $c.Close()
                        $ready = $true
                        break
                    }
                    catch { }
                }
                if (-not $ready) {
                    Write-Host "[跳过] HTTP 索引：静态服务 2 秒内没就绪"
                    $script:skip++
                }
                else {
                    $appHttp = Join-Path $httpRoot "应用"
                    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
工具 = { 版本 = "1.0.0" }
"@
                    New-Pkg $appHttp $toml '打印(工具.版本())'
                    $savedNoProxy = $env:NO_PROXY
                    $env:NO_PROXY = "127.0.0.1,localhost"
                    $io = (& $micc install --目录 $appHttp --索引 ("http://127.0.0.1:" + $port + "/index.toml") 2>&1 | Out-String)
                    $installCode = $LASTEXITCODE
                    $env:NO_PROXY = $savedNoProxy

                    $lockFile = Join-Path $appHttp ".砾\锁.toml"
                    $lockText = if (Test-Path $lockFile) { [IO.File]::ReadAllText($lockFile) } else { "<缺少锁文件>" }
                    $cached = Join-Path $appHttp ".砾\包\工具-1.0.0\主.jsh"
                    $r = Invoke-Pack $appHttp -Offline
                    $out = Run-Jmb (Join-Path $appHttp "应用.jmb")
                    Write-Host ("[信息] HTTP 索引 1 个条目；归档 " + $zipBytes + " 字节；锁文件：" + ($lockText -replace "`r`n", " | "))
                    Check "HTTP 索引：curl 下载 + tar 解包 + 哈希校验 + 离线打包运行" (($installCode -eq 0) -and ($lockText.Contains('版本 = "1.0.0"')) -and (Test-Path $cached) -and ($r.code -eq 0) -and ($out -eq "1.0.0")) ($io + $r.out + "`n缓存存在：" + (Test-Path $cached) + "`n实际输出：" + $out + "`n锁文件：" + $lockText)
                }
            }
            finally {
                if ($srv -and -not $srv.HasExited) { Stop-Process -Id $srv.Id -Force }
            }
        }
    }

    Write-Host ("包用例：共 " + $script:total + " 个，失败 " + $script:fail + " 个，跳过 " + $script:skip + " 个")
    if ($script:fail -gt 0) { exit 1 }
    exit 0
}
finally {
    # 用 .NET 直接删临时目录：临时夹具不值得进回收站，也更少受策略拦截。
    if ($sandbox -and (Test-Path $sandbox)) {
        try { [IO.Directory]::Delete($sandbox, $true) } catch { }
    }
    Pop-Location
}
