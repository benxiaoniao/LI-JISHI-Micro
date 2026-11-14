# 调试表回归：行记录、双向查询、剥离、损坏容忍、旧文件、跨包 file_id。
# 夹具全部建在 $env:TEMP，跑完删除，不污染仓库。
# 注意：同一时刻只跑一个长命令。与其它回归/串口录制/后台任务并行时，命令在沙箱后端
#       就起不来（零输出，报 sandbox backend failed），与用例无关（见 docs/JM10 方案 §7.3）。
$ErrorActionPreference = "Continue"
$root = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot "toolchains.ps1")

Push-Location $root
try {
    $micc = Join-Path $root ("compiler/target/release/micc" + $ExeSuffix)
    $jmrun = Join-Path $root ("build/jmrun" + $ExeSuffix)
    if (-not (Test-Path $micc)) { throw "先运行 tools/build.ps1" }
    if (-not (Test-Path $jmrun)) { throw "先运行 tools/build.ps1（缺 build/jmrun）" }

    $sandbox = Join-Path ([IO.Path]::GetTempPath()) ("li-debug-" + [guid]::NewGuid().ToString("N").Substring(0, 8))
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

    # ---------- 9. M2：行表 / 多断点 / 行级单步 / 步过 ----------
    function DbgRun([string]$inText, [string[]]$argv2) {
        $lines = @($inText -split "`n")
        $o = ($lines | & $jmrun @argv2 2>&1 | Out-String) -replace "`r`n", "`n"
        return @{ code = $LASTEXITCODE; text = $o }
    }
    function PauseLines([string]$text) {
        # PowerShell 把 stderr 转成错误记录时会重复一遍文本，连续相同的行去重。
        $ms = [regex]::Matches($text, "暂停：任务 (\d+) 「[^」]*」 .*?:(\d+)")
        $vals = @($ms | ForEach-Object { [int]$_.Groups[2].Value })
        $out = @()
        foreach ($v in $vals) {
            if ($out.Count -eq 0 -or $out[$out.Count - 1] -ne $v) { $out += $v }
        }
        return $out
    }
    # 夹具用 ASCII 文件名：REPL 的断点规格从 stdin 读入，避免与 argv 编码混用。
    $m2f = Join-Path $sandbox "m2step.jsh"
    Write-Utf8 $m2f @"
函数 加(数)：
    令 结果 = 数 + 1
    返回 结果
令 甲 = 1
令 乙 = 加(甲)
打印(乙)
打印("结束")
"@
    $m2jmb = Join-Path $sandbox "m2step.jmb"
    & $micc $m2f -o $m2jmb | Out-Null
    $m2s = $m2f.Replace('\', '/')

    $lt = (& $micc --line-table $m2jmb 2>&1 | Out-String)
    Check "行表包含 代码/偏移/文件/行 四列" `
        (($LASTEXITCODE -eq 0) -and ($lt -match "(?m)^\d+\t\d+\t\d+\t\d+\r?$")) $lt

    $r = DbgRun "c`nc`nc`nq" @("--dbg", "--break", ($m2s + ":4"), "--break", ($m2s + ":5"), "--break", ($m2s + ":6"), $m2jmb)
    Check "三个断点按执行顺序 4 → 5 → 6 命中" `
        (($r.code -eq 0) -and (((PauseLines $r.text) -join ",") -eq "4,5,6")) (($r.text))

    $plain = ((& $jmrun $m2jmb 2>$null | Out-String) -replace "`r`n", "`n").TrimEnd()
    $hitStdout = ((@("c", "c", "c", "q") | & $jmrun --dbg --break ($m2s + ":4") --break ($m2s + ":5") --break ($m2s + ":6") $m2jmb 2>$null | Out-String) -replace "`r`n", "`n").TrimEnd()
    Check "多次暂停恢复后 stdout 与普通运行一致" ($hitStdout -eq $plain) ($plain + "`n---`n" + $hitStdout)

    $r = DbgRun "n`nq" @("--dbg", "--break", ($m2s + ":5"), $m2jmb)
    Check "n 行级单步进入被调函数（5 → 2）" `
        (($r.code -eq 0) -and (((PauseLines $r.text) -join ",") -eq "5,2") -and ($r.text -match "「加」")) $r.text

    $r = DbgRun "o`nq" @("--dbg", "--break", ($m2s + ":5"), $m2jmb)
    Check "o 步过不进入被调函数（5 → 6）" `
        (($r.code -eq 0) -and (((PauseLines $r.text) -join ",") -eq "5,6") -and ($r.text -notmatch "「加」")) $r.text

    $r = DbgRun "bl`nb m2step.jsh:6`nbl`nc`nq" @("--dbg", $m2jmb)
    Check "REPL b/bl：先列入口断点，再加一点并命中" `
        (($r.code -eq 0) -and ($r.text -match "#0 ") -and ($r.text -match "m2step\.jsh:6") -and
         ((PauseLines $r.text) -contains 6)) $r.text

    $tooMany = @("--dbg")
    for ($i = 0; $i -lt 9; $i++) { $tooMany += @("--break", ($m2s + ":4")) }
    $tooMany += $m2jmb
    $r = DbgRun "q`n" $tooMany
    Check "断点超过 8 个明确拒绝" (($r.code -eq 2) -and ($r.text -match "断点最多 8 个")) $r.text

    # ---------- 10. M2：--task 会话过滤 ----------
    $taskJmb = "build/任务.jmb"
    if (Test-Path $taskJmb) {
        $r = DbgRun "c`nq" @("--dbg", "--break", "任务.jsh:5", "--task", "1", $taskJmb)
        Check "--task 1 停在子任务（子甲）" `
            (($r.code -eq 0) -and ($r.text -match "暂停：任务 1 「子甲」") -and
             ((PauseLines $r.text) -contains 5)) $r.text

        $r = DbgRun "q`n" @("--dbg", "--break", "任务.jsh:5", "--task", "7", $taskJmb)
        Check "--task 7 不存在：提示一次且断点不再命中" `
            (($r.code -eq 0) -and ($r.text -match "任务 7 不存在（当前共 4 个）") -and
             ($r.text -notmatch "暂停：")) $r.text

        $r = DbgRun "q`n" @("--dbg", "--break", "任务.jsh:26", "--task", "1", $taskJmb)
        Check "--task 1 已结束：提示一次且不再停" `
            (($r.code -eq 0) -and ($r.text -match "任务 1 已结束") -and ($r.text -notmatch "暂停：")) $r.text
    } else {
        Check "--task 相关用例（缺 build/任务.jmb）" $false "先运行 tools\build.ps1"
    }

    # ---------- 11. M3：条件断点 / 命中计数 / 断点脚本文件 ----------
    $m3f = Join-Path $sandbox "m3cond.jsh"
    Write-Utf8 $m3f @"
函数 跑():
    令 k = 0
    当 k < 5:
        k += 1
        打印("k =", k)
    令 名 = "甲"
    令 好 = 真
    打印(名, 好)
跑()
"@
    $m3jmb = Join-Path $sandbox "m3cond.jmb"
    & $micc $m3f -o $m3jmb | Out-Null
    $m3s = $m3f.Replace('\', '/')
    $m3plain = ((& $jmrun $m3jmb 2>$null | Out-String) -replace "`r`n", "`n").TrimEnd()

    $r = DbgRun "bl`nc`nq" @("--dbg", "--break", ($m3s + ":5 if k == 3"), $m3jmb)
    Check "条件断点：k == 3 时停（第 3 次命中）" `
        (($r.code -eq 0) -and (((PauseLines $r.text) -join ",") -eq "5") -and ($r.text -match "已命中 3")) $r.text

    $r = DbgRun "c`nq" @("--dbg", "--break", ($m3s + ":5 if k == 99"), $m3jmb)
    $m3noHit = ((@("c", "q") | & $jmrun --dbg --break ($m3s + ":5 if k == 99") $m3jmb 2>$null | Out-String) -replace "`r`n", "`n").TrimEnd()
    Check "条件不成立：不停、输出与普通运行一致" `
        (($r.code -eq 0) -and ($r.text -notmatch "暂停：") -and ($m3noHit -eq $m3plain)) $r.text

    $r = DbgRun "c`nq" @("--dbg", "--break", ($m3s + ":8 if 名 == 甲"), $m3jmb)
    Check "文本条件成立（裸文本字面量）" `
        (($r.code -eq 0) -and ((PauseLines $r.text) -contains 8)) $r.text

    $r = DbgRun "c`nq" @("--dbg", "--break", ($m3s + ":8 if 好 == 真"), $m3jmb)
    Check "布尔条件成立（真）" `
        (($r.code -eq 0) -and ((PauseLines $r.text) -contains 8)) $r.text

    $r = DbgRun "c`nq" @("--dbg", "--break", ($m3s + ":5 if k == 甲"), $m3jmb)
    Check "类型不匹配：警告一次并按不成立继续" `
        (($r.code -eq 0) -and ($r.text -match "警告：断点 #0 条件求值失败") -and
         ($r.text -notmatch "暂停：") -and (([regex]::Matches($r.text, "警告：")).Count -le 2)) $r.text

    $r = DbgRun "c`nq" @("--dbg", "--break", ($m3s + ":5 if 不存在 == 1"), $m3jmb)
    Check "未知名条件：警告一次并按不成立继续" `
        (($r.code -eq 0) -and ($r.text -match "警告：断点 #0 条件求值失败") -and
         ($r.text -notmatch "暂停：")) $r.text

    $r = DbgRun "bl`nc`nq" @("--dbg", "--break", ($m3s + ":5#2"), $m3jmb)
    Check "命中计数 #2：只第 2 次命中停" `
        (($r.code -eq 0) -and ($r.text -match "已命中 2") -and (((PauseLines $r.text) -join ",") -eq "5")) $r.text

    $r = DbgRun "bl`nc`nbl`nc`nq" @("--dbg", "--break", ($m3s + ":5%2"), $m3jmb)
    Check "命中计数 %2：第 2、4 次命中停" `
        (($r.code -eq 0) -and ($r.text -match "已命中 2") -and ($r.text -match "已命中 4")) $r.text

    $bpOk = Join-Path $sandbox "断点表.txt"
    Write-Utf8Bom $bpOk ("// 注释`r`n`r`n" + "m3cond.jsh:5 if k == 2`r`n" + 'm3cond.jsh:8 if 名 == "甲"' + "`r`n")
    $r = DbgRun "c`nc`nq" @("--dbg", "--break-file", $bpOk, $m3jmb)
    Check "断点脚本：跳过注释/空行，两条按 5 → 8 命中" `
        (($r.code -eq 0) -and (((PauseLines $r.text) -join ",") -eq "5,8")) $r.text

    $bpBad = Join-Path $sandbox "断点表-坏行.txt"
    Write-Utf8 $bpBad ("m3cond.jsh:5`r`n" + "m3cond.jsh:5 if k === 1`r`n")
    $r = DbgRun "q`n" @("--dbg", "--break-file", $bpBad, $m3jmb)
    Check "断点脚本坏行：报第 2 行并退出 2（不执行任何断点）" `
        (($r.code -eq 2) -and ($r.text -match "第 2 行") -and ($r.text -notmatch "暂停：")) $r.text

    $r = DbgRun "q`n" @("--dbg", "--break-file", (Join-Path $sandbox "不存在.txt"), $m3jmb)
    Check "断点脚本不存在：明确报错并退出 2" `
        (($r.code -eq 2) -and ($r.text -match "打不开断点脚本")) $r.text

    $r = DbgRun "q`n" @("--break-file", $bpOk, $m3jmb)
    Check "断点脚本只能与 --dbg 一起用" `
        (($r.code -eq 2) -and ($r.text -match "只能与 --dbg 一起用")) $r.text

    $r = DbgRun "c`nq" @("--break", ($m3s + ":5 if k == 3"), $m3jmb)
    Check "普通运行（不带 --dbg）拒绝条件断点" `
        (($r.code -eq 2) -and ($r.text -match "只能与 --dbg 一起用")) $r.text

    $r = DbgRun "b m3cond.jsh:5 if k == 4`nbl`nc`nq" @("--dbg", $m3jmb)
    Check "REPL b 支持条件断点" `
        (($r.code -eq 0) -and ($r.text -match "if k == 4") -and ((PauseLines $r.text) -contains 5)) $r.text

    # 组合：条件断点 × --task N（两个子任务同一地址，各自的条件与计数互不干扰）
    $m3tf = Join-Path $sandbox "m3task.jsh"
    Write-Utf8 $m3tf @"
导入 任务

函数 甲():
    令 k = 0
    当 k < 3:
        k += 1
        打印("甲", k)
    让出()

函数 乙():
    令 k = 0
    当 k < 3:
        k += 1
        打印("乙", k)
    让出()

创建(甲)
创建(乙)
让出()
延时(200)
打印("根结束")
"@
    $m3tjmb = Join-Path $sandbox "m3task.jmb"
    & $micc $m3tf -o $m3tjmb | Out-Null
    $m3ts = $m3tf.Replace('\', '/')

    $r = DbgRun "bl`nc`nq" @("--dbg", "--break", ($m3ts + ":7 if k == 2"), "--task", "1", $m3tjmb)
    Check "条件断点 × --task 1：只停子任务甲的第 2 次命中" `
        (($r.code -eq 0) -and ($r.text -match "暂停：任务 1 「甲」") -and ($r.text -match "已命中 2") -and
         ((PauseLines $r.text) -contains 7)) $r.text

    # 乙 的打印在第 14 行：同一份源码里两个任务各在自己的代码对象，行号不共用
    $r = DbgRun "bl`nc`nq" @("--dbg", "--break", ($m3ts + ":14 if k == 2"), "--task", "2", $m3tjmb)
    Check "条件断点 × --task 2：只停子任务乙第 2 次命中" `
        (($r.code -eq 0) -and ($r.text -match "暂停：任务 2 「乙」") -and ($r.text -match "已命中 2") -and
         ($r.text -notmatch "「甲」")) $r.text

    # 含空格文本字面量：走断点脚本（命令行里 shell 会吃掉内嵌双引号）
    $m3xf = Join-Path $sandbox "m3text.jsh"
    Write-Utf8 $m3xf "令 标题 = `"甲 乙`"`n打印(标题)`n"
    $m3xjmb = Join-Path $sandbox "m3text.jmb"
    & $micc $m3xf -o $m3xjmb | Out-Null
    $m3xplain = ((& $jmrun $m3xjmb 2>$null | Out-String) -replace "`r`n", "`n").TrimEnd()
    $bpSpace = Join-Path $sandbox "断点表-空格.txt"
    Write-Utf8 $bpSpace ('m3text.jsh:2 if 标题 == "甲 乙"' + "`r`n")
    $r = DbgRun "c`nq" @("--dbg", "--break-file", $bpSpace, $m3xjmb)
    Check '含空格文本条件成立（断点脚本里的 "甲 乙"）' `
        (($r.code -eq 0) -and ((PauseLines $r.text) -contains 2) -and
         ($r.text -notmatch "警告：")) $r.text

    $bpSpace2 = Join-Path $sandbox "断点表-空格不等.txt"
    Write-Utf8 $bpSpace2 ('m3text.jsh:2 if 标题 == "甲"' + "`r`n")
    $r = DbgRun "c`nq" @("--dbg", "--break-file", $bpSpace2, $m3xjmb)
    $m3xout = ((@("c", "q") | & $jmrun --dbg --break-file $bpSpace2 $m3xjmb 2>$null | Out-String) -replace "`r`n", "`n").TrimEnd()
    Check "含空格文本不等：不停、输出一致" `
        (($r.code -eq 0) -and ($r.text -notmatch "暂停：") -and ($m3xout -eq $m3xplain)) $r.text

    Write-Host ("共 " + $total + " 个用例，失败 " + $fail + " 个")
    if ($fail -gt 0) { exit 1 }
    exit 0
}
finally {
    Pop-Location
}
