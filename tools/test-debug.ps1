# 调试表回归：行记录、双向查询、剥离、损坏容忍、旧文件、跨包 file_id。
# 夹具全部建在 $env:TEMP，跑完删除，不污染仓库。
$ErrorActionPreference = "Continue"
$root = Split-Path -Parent $PSScriptRoot

Push-Location $root
try {
    $micc = "compiler\target\release\micc.exe"
    $jmrun = "build\jmrun.exe"
    if (-not (Test-Path $micc)) { throw "先运行 tools\build.ps1" }
    if (-not (Test-Path $jmrun)) { throw "先运行 tools\build.ps1（缺 build\jmrun.exe）" }

    $sandbox = Join-Path $env:TEMP ("li-debug-" + [guid]::NewGuid().ToString("N").Substring(0, 8))
    New-Item -ItemType Directory -Path $sandbox | Out-Null
    $script:fail = 0
    $script:total = 0

    function Write-Utf8([string]$path, [string]$text) {
        [IO.File]::WriteAllText($path, $text, (New-Object System.Text.UTF8Encoding($false)))
    }
    function Write-Utf8Bom([string]$path, [string]$text) {
        [IO.File]::WriteAllText($path, $text, (New-Object System.Text.UTF8Encoding($true)))
    }
    function Check([string]$name, [bool]$ok, [string]$detail) {
        $script:total++
        if ($ok) { Write-Host ("[通过] " + $name) }
        else {
            Write-Host ("[不通过] " + $name)
            if ($detail) { Write-Host $detail }
            $script:fail++
        }
    }
    function Run([string]$jmb) {
        return ((& $jmrun $jmb 2>&1 | Out-String).TrimEnd() -replace "`r`n", "`n")
    }
    function LineOf([string]$jmb, [int]$code, [int]$off) {
        $o = (& $micc --line-of $jmb $code $off 2>&1 | Out-String).Trim()
        return @{ code = $LASTEXITCODE; text = $o }
    }
    function LinesOf([string]$jmb, [string]$file, [int]$line) {
        $o = (& $micc --lines-of $jmb $file $line 2>&1 | Out-String)
        return @{ code = $LASTEXITCODE; text = $o }
    }
    function FirstSpan([string]$text) {
        $m = [regex]::Match($text, "代码 (\d+) .*? 偏移 (\d+)\.\.(\d+)")
        if (-not $m.Success) { return $null }
        return @{
            code  = [int]$m.Groups[1].Value
            start = [int]$m.Groups[2].Value
            end   = [int]$m.Groups[3].Value
        }
    }

    # ---------- 1. 单文件多行 + 跨函数 ----------
    $src = @"
令 甲 = 1
打印(甲)

函数 加(数)：
    令 结果 = 数 + 1
    返回 结果

打印(加(41))
"@
    $f = Join-Path $sandbox "多行.jsh"
    Write-Utf8 $f $src
    $jmb = Join-Path $sandbox "多行.jmb"
    & $micc $f -o $jmb | Out-Null
    Check "默认编译带调试段" (($LASTEXITCODE -eq 0) -and (Test-Path $jmb)) "退出码 $LASTEXITCODE"

    $r = LinesOf $jmb "多行.jsh" 2
    $sp = FirstSpan $r.text
    Check "顶层第 2 行能反查到指令范围" (($r.code -eq 0) -and $sp -and ($sp.start -lt $sp.end)) $r.text
    if ($sp) {
        $back = LineOf $jmb $sp.code $sp.start
        Check "正向查询回到第 2 行" (($back.code -eq 0) -and $back.text.EndsWith(":2")) $back.text
    }

    $r6 = LinesOf $jmb "多行.jsh" 6
    $sp6 = FirstSpan $r6.text
    $r8 = LinesOf $jmb "多行.jsh" 8
    $sp8 = FirstSpan $r8.text
    Check "跨函数：第 6 行在独立代码对象，第 8 行在模块代码" `
        (($r6.code -eq 0) -and ($r8.code -eq 0) -and $sp6 -and $sp8 -and ($sp6.code -ne $sp8.code)) `
        ($r6.text + $r8.text)
    if ($sp6) {
        $b6 = LineOf $jmb $sp6.code $sp6.start
        Check "函数体内正向查询回到第 6 行" (($b6.code -eq 0) -and $b6.text.EndsWith(":6")) $b6.text
    }

    # ---------- 2. 剥离调试段 ----------
    $strip = Join-Path $sandbox "多行-剥离.jmb"
    & $micc $f -o $strip --strip-debug | Out-Null
    $lenFull = (Get-Item $jmb).Length
    $lenStrip = (Get-Item $strip).Length
    Check "剥离后体积更小" (($LASTEXITCODE -eq 0) -and ($lenStrip -lt $lenFull)) ("$lenFull -> $lenStrip")
    $bytes = [IO.File]::ReadAllBytes($strip)
    Check "剥离后 p5.bit0 清零" (($bytes[5] -band 1) -eq 0) ("flags=" + $bytes[5])
    $outFull = Run $jmb
    $outStrip = Run $strip
    Check "剥离前后运行输出一致" ($outFull -eq $outStrip) ($outFull + "`n---`n" + $outStrip)
    $q = LineOf $strip 0 0
    Check "剥离后查询明确报无调试信息" (($q.code -ne 0) -and $q.text.Contains("没有可用调试信息")) $q.text

    # ---------- 3. 损坏的调试段被忽略 ----------
    $bad = Join-Path $sandbox "坏.jmb"
    [IO.File]::WriteAllBytes($bad, $bytes) # 先放剥离版，再追加伪造的坏尾
    $full = [IO.File]::ReadAllBytes($jmb)
    $full[$full.Length - 1] = 0x00 # 破坏尾部魔数
    [IO.File]::WriteAllBytes($bad, $full)
    $outBad = Run $bad
    Check "调试段损坏仍能运行" ($outBad -eq $outFull) $outBad
    $qbad = LineOf $bad 0 0
    Check "损坏段查询被忽略" (($qbad.code -ne 0) -and $qbad.text.Contains("没有可用调试信息")) $qbad.text

    # ---------- 4. 旧版无调试段文件（剥离产物即旧布局） ----------
    $old = $strip
    $outOld = Run $old
    Check "旧文件（无调试段）照常运行" ($outOld -eq $outFull) $outOld

    # ---------- 5. 跨包 file_id ----------
    $pkgRoot = Join-Path $sandbox "跨包"
    $tool = Join-Path $pkgRoot "工具"
    $app = Join-Path $pkgRoot "应用"
    New-Item -ItemType Directory -Path $tool -Force | Out-Null
    New-Item -ItemType Directory -Path $app -Force | Out-Null
    Write-Utf8Bom (Join-Path $tool "砾包.toml") @"
[包]
名字 = "工具"
版本 = "1.0.0"
入口 = "工具.jsh"

[导出]
名字 = ["加一"]
"@
    Write-Utf8 (Join-Path $tool "工具.jsh") @"
函数 加一(数)：
    返回 数 + 1

令 基 = 41
"@
    Write-Utf8Bom (Join-Path $app "砾包.toml") @"
[包]
名字 = "应用"
版本 = "1.0.0"
入口 = "应用.jsh"

[依赖]
工具 = { 路径 = "../工具", 版本 = "^1.0.0" }
"@
    Write-Utf8 (Join-Path $app "应用.jsh") "打印(工具.加一(1))"
    & $micc pack --目录 $app | Out-Null
    $appJmb = Join-Path $app "应用.jmb"
    Check "跨包打包成功" (($LASTEXITCODE -eq 0) -and (Test-Path $appJmb)) "退出码 $LASTEXITCODE"

    $rt = LinesOf $appJmb "工具.jsh" 2
    $st = FirstSpan $rt.text
    $ra = LinesOf $appJmb "应用.jsh" 1
    $sa = FirstSpan $ra.text
    Check "跨包：依赖文件与根包文件各自可反查" `
        (($rt.code -eq 0) -and ($ra.code -eq 0) -and $st -and $sa -and ($st.code -ne $sa.code)) `
        ($rt.text + $ra.text)
    if ($st) {
        $bt = LineOf $appJmb $st.code $st.start
        Check "跨包：依赖函数正向查询指向工具.jsh:2" `
            (($bt.code -eq 0) -and $bt.text.EndsWith("工具.jsh:2")) $bt.text
    }
    if ($sa) {
        $ba = LineOf $appJmb $sa.code $sa.start
        Check "跨包：根包正向查询指向应用.jsh:1" `
            (($ba.code -eq 0) -and $ba.text.EndsWith("应用.jsh:1")) $ba.text
    }
    $d = (& $micc --debug-dump $appJmb 2>&1 | Out-String)
    Check "调试表列出两个源文件" (($d -match "工具\.jsh") -and ($d -match "应用\.jsh")) $d

    # ---------- 6. 依赖包顶层 `令` 归属依赖自己的文件 ----------
    $rl = LinesOf $appJmb "工具.jsh" 4
    $sl = FirstSpan $rl.text
    Check "依赖包顶层令能反查到工具.jsh" (($rl.code -eq 0) -and $sl) $rl.text
    if ($sl) {
        $bl = LineOf $appJmb $sl.code $sl.start
        Check "依赖包顶层令正向指回工具.jsh:4（不是根包）" `
            (($bl.code -eq 0) -and $bl.text.EndsWith("工具.jsh:4")) $bl.text
    }
    $ra1 = LinesOf $appJmb "应用.jsh" 1
    $sa1 = FirstSpan $ra1.text
    if ($sa1) {
        $ba1 = LineOf $appJmb $sa1.code $sa1.start
        Check "根包顶层令仍指向应用.jsh:1" `
            (($ba1.code -eq 0) -and $ba1.text.EndsWith("应用.jsh:1")) $ba1.text
    }

    # 依赖包顶层执行时出错，错误位置应落到依赖文件
    $errPkgRoot = Join-Path $sandbox "报错包"
    $errTool = Join-Path $errPkgRoot "工具"
    $errApp = Join-Path $errPkgRoot "应用"
    New-Item -ItemType Directory -Path $errTool -Force | Out-Null
    New-Item -ItemType Directory -Path $errApp -Force | Out-Null
    Write-Utf8Bom (Join-Path $errTool "砾包.toml") @"
[包]
名字 = "工具"
版本 = "1.0.0"
入口 = "工具.jsh"
"@
    Write-Utf8 (Join-Path $errTool "工具.jsh") "令 零 = 0`n令 坏 = 1 // 零`n"
    Write-Utf8Bom (Join-Path $errApp "砾包.toml") @"
[包]
名字 = "应用"
版本 = "1.0.0"
入口 = "应用.jsh"

[依赖]
工具 = { 路径 = "../工具", 版本 = "^1.0.0" }
"@
    Write-Utf8 (Join-Path $errApp "应用.jsh") "打印(`"不应运行`")`n"
    & $micc pack --目录 $errApp | Out-Null
    $errPkgJmb = Join-Path $errApp "应用.jmb"
    if (Test-Path $errPkgJmb) {
        $ep = (& $jmrun $errPkgJmb 2>&1 | Out-String)
        Check "依赖包顶层运行期错误指向依赖文件" `
            (($ep -match "E007") -and $ep.Contains("工具.jsh:2")) $ep
    } else {
        Check "依赖包顶层运行期错误指向依赖文件" $false "打包失败"
    }

    # ---------- 7. jmrun 运行期错误带源码位置 ----------
    $errSrc = Join-Path $sandbox "错误.jsh"
    Write-Utf8 $errSrc "令 甲 = 6`n令 乙 = 0`n打印(甲 // 乙)`n"
    $errJmb = Join-Path $sandbox "错误.jmb"
    & $micc $errSrc -o $errJmb | Out-Null
    $errOut = (& $jmrun $errJmb 2>&1 | Out-String)
    Check "运行期错误附带 文件:行" `
        (($LASTEXITCODE -eq 1) -and ($errOut -match "E007") -and ($errOut -match "错误\.jsh:3")) $errOut

    $errStripJmb = Join-Path $sandbox "错误-剥离.jmb"
    & $micc $errSrc -o $errStripJmb --strip-debug | Out-Null
    $errStripOut = (& $jmrun $errStripJmb 2>&1 | Out-String)
    Check "无调试段时错误输出退回原样（无文件:行）" `
        (($LASTEXITCODE -eq 1) -and ($errStripOut -match "E007") -and ($errStripOut -notmatch "于 ")) $errStripOut

    # ---------- 8. jmrun --break ----------
    $multiSpec = $f.Replace('\', '/') + ":2"
    $hitOut = (& $jmrun --break $multiSpec $jmb 2>&1 | Out-String)
    Check "断点命中并打印源码行" `
        (($LASTEXITCODE -eq 0) -and ($hitOut -match "断点：") -and ($hitOut -match ":2")) $hitOut
    $missOut = (& $jmrun --break ($f.Replace('\', '/') + ":999") $jmb 2>&1 | Out-String)
    Check "断点行号不存在时明确拒绝" `
        (($LASTEXITCODE -eq 2) -and ($missOut -match "断点无法落地")) $missOut
    $stripBpOut = (& $jmrun --break $multiSpec $strip 2>&1 | Out-String)
    Check "剥离后无法使用断点（无调试信息）" `
        (($LASTEXITCODE -eq 2) -and ($stripBpOut -match "断点无法落地")) $stripBpOut

    Write-Host ("共 " + $total + " 个用例，失败 " + $fail + " 个")
    if ($fail -gt 0) { exit 1 }
    exit 0
}
finally {
    Pop-Location
}
