$ErrorActionPreference = "Continue"
$root = Split-Path -Parent $PSScriptRoot

Push-Location $root
try {
    $micc = "compiler\target\release\micc.exe"
    $jmrun = "build\jmrun.exe"
    if (-not (Test-Path $jmrun)) { throw "先运行 tools\build.ps1" }

    $fail = 0
    $total = 0
    Get-ChildItem "examples" -Filter "*.jsh" | Sort-Object Name | ForEach-Object {
        $total++
        $name = $_.BaseName
        $jmb = "build\" + $name + ".jmb"
        & $micc ("examples\" + $_.Name) -o $jmb | Out-Null
        if ($LASTEXITCODE -ne 0) {
            Write-Host ("[编译失败] " + $_.Name)
            $fail++
            return
        }
        $actual = ((& $jmrun $jmb 2>&1 | Out-String).TrimEnd() -replace "`r`n", "`n")
        $gold = "tests\expected\" + $name + ".txt"
        if (-not (Test-Path $gold)) {
            Write-Host ("[缺少基准] " + $name + " —— 把实际输出存进 " + $gold)
            $fail++
            return
        }
        $want = ((Get-Content $gold -Raw -Encoding UTF8).TrimEnd() -replace "`r`n", "`n")
        if ($actual -eq $want) {
            Write-Host ("[通过] " + $name)
        } else {
            Write-Host ("[不通过] " + $name)
            Write-Host "--- 期望 ---"
            Write-Host $want
            Write-Host "--- 实际 ---"
            Write-Host $actual
            $fail++
        }
    }
    # ---------- N6：--emit-blob（固件内嵌字节数组） ----------
    $blobJmb = "build\你好.jmb"
    if (Test-Path $blobJmb) {
        Remove-Item "build\jm_blob.c" -Force -ErrorAction SilentlyContinue
        & $micc --emit-blob $blobJmb | Out-Null
        $total++
        if (($LASTEXITCODE -eq 0) -and (Test-Path "build\jm_blob.c")) {
            Write-Host "[通过] emit-blob 默认输出同目录 jm_blob.c"
        } else {
            Write-Host "[不通过] emit-blob 默认输出同目录 jm_blob.c"
            $fail++
        }
        $text = Get-Content "build\jm_blob.c" -Raw -Encoding UTF8
        $len = (Get-Item $blobJmb).Length
        $total++
        if (($text -match "const unsigned char jm_app_jmb\[\]") -and
            ($text -match ("jm_app_jmb_len = " + $len + "u"))) {
            Write-Host "[通过] emit-blob 符号名与内嵌长度一致"
        } else {
            Write-Host "[不通过] emit-blob 符号名与内嵌长度一致"
            $fail++
        }
        $out2 = "build\jm_blob-指定.c"
        & $micc --emit-blob $blobJmb -o $out2 | Out-Null
        $total++
        if (($LASTEXITCODE -eq 0) -and (Test-Path $out2)) {
            Write-Host "[通过] emit-blob -o 指定输出路径"
        } else {
            Write-Host "[不通过] emit-blob -o 指定输出路径"
            $fail++
        }
    }
    $total++
    & $micc --emit-blob 2>$null | Out-Null
    if ($LASTEXITCODE -eq 2) {
        Write-Host "[通过] emit-blob 缺参数退出码 2"
    } else {
        Write-Host "[不通过] emit-blob 缺参数退出码 2"
        $fail++
    }

    # ---------- N22 前置：引脚 0–47 的越界错误（运行期 E030） ----------
    $enc = New-Object System.Text.UTF8Encoding($false)
    [IO.File]::WriteAllText((Join-Path $root "build\pin-bad.jsh"), "导入 引脚`n引脚写(48, 1)`n", $enc)
    & $micc "build\pin-bad.jsh" -o "build\pin-bad.jmb" | Out-Null
    $o = (& $jmrun "build\pin-bad.jmb" 2>&1 | Out-String)
    $code = $LASTEXITCODE
    $total++
    if (($code -ne 0) -and $o.Contains("E030")) {
        Write-Host "[通过] 引脚写(48) 报 E030"
    } else {
        Write-Host "[不通过] 引脚写(48) 报 E030"
        Write-Host $o
        $fail++
    }

    [IO.File]::WriteAllText((Join-Path $root "build\pin-neg.jsh"), "导入 引脚`n引脚读(-1)`n", $enc)
    & $micc "build\pin-neg.jsh" -o "build\pin-neg.jmb" | Out-Null
    $o = (& $jmrun "build\pin-neg.jmb" 2>&1 | Out-String)
    $code = $LASTEXITCODE
    $total++
    if (($code -ne 0) -and $o.Contains("E030")) {
        Write-Host "[通过] 引脚读(-1) 报 E030"
    } else {
        Write-Host "[不通过] 引脚读(-1) 报 E030"
        Write-Host $o
        $fail++
    }

    # ---------- N11：枚举/匹配 的编译期错误（E139） ----------
    [IO.File]::WriteAllText((Join-Path $root "build\enum-bad.jsh"), "枚举 颜色:`n    红`n    绿`n`n打印(颜色.紫)`n", $enc)
    $o = (& $micc "build\enum-bad.jsh" -o "build\enum-bad.jmb" 2>&1 | Out-String)
    $code = $LASTEXITCODE
    $total++
    if (($code -ne 0) -and $o.Contains("E139")) {
        Write-Host "[通过] 枚举未知成员 报 E139"
    } else {
        Write-Host "[不通过] 枚举未知成员 报 E139"
        Write-Host $o
        $fail++
    }

    [IO.File]::WriteAllText((Join-Path $root "build\match-dup.jsh"), "枚举 颜色:`n    红`n    绿`n`n匹配 0:`n    情形 0:`n        打印(1)`n    情形 颜色.红:`n        打印(2)`n", $enc)
    $o = (& $micc "build\match-dup.jsh" -o "build\match-dup.jmb" 2>&1 | Out-String)
    $code = $LASTEXITCODE
    $total++
    if (($code -ne 0) -and $o.Contains("E139")) {
        Write-Host "[通过] 匹配重复分支值 报 E139"
    } else {
        Write-Host "[不通过] 匹配重复分支值 报 E139"
        Write-Host $o
        $fail++
    }

    # ---------- N11：枚举跨包隔离（依赖包枚举改名，根包不可直接引用） ----------
    $pkgRoot = Join-Path $root "build\n11pkg"
    foreach ($d in @("甲", "乙", "应用", "坏应用")) {
        New-Item -ItemType Directory -Force -Path (Join-Path $pkgRoot $d) | Out-Null
    }
    [IO.File]::WriteAllText((Join-Path $pkgRoot "甲\砾包.toml"), "[包]`n名字 = ""甲""`n版本 = ""1.0.0""`n入口 = ""主.jsh""`n`n[导出]`n名字 = [""颜色名""]`n", $enc)
    [IO.File]::WriteAllText((Join-Path $pkgRoot "甲\主.jsh"), "枚举 颜色:`n    红`n    绿`n`n函数 颜色名(k):`n    匹配 k:`n        情形 颜色.绿:`n            返回 ""绿""`n        情形 其他:`n            返回 ""其他""`n", $enc)
    [IO.File]::WriteAllText((Join-Path $pkgRoot "乙\砾包.toml"), "[包]`n名字 = ""乙""`n版本 = ""1.0.0""`n入口 = ""主.jsh""`n", $enc)
    [IO.File]::WriteAllText((Join-Path $pkgRoot "乙\主.jsh"), "枚举 颜色:`n    绿`n    蓝`n", $enc)
    [IO.File]::WriteAllText((Join-Path $pkgRoot "应用\砾包.toml"), "[包]`n名字 = ""应用""`n版本 = ""1.0.0""`n入口 = ""主.jsh""`n`n[依赖]`n甲 = { 路径 = ""../甲"", 版本 = ""^1.0.0"" }`n乙 = { 路径 = ""../乙"", 版本 = ""^1.0.0"" }`n", $enc)
    [IO.File]::WriteAllText((Join-Path $pkgRoot "应用\主.jsh"), "枚举 颜色:`n    蓝`n`n打印(颜色.蓝)`n打印(甲.颜色名(1))`n", $enc)
    [IO.File]::WriteAllText((Join-Path $pkgRoot "坏应用\砾包.toml"), "[包]`n名字 = ""坏应用""`n版本 = ""1.0.0""`n入口 = ""主.jsh""`n`n[依赖]`n甲 = { 路径 = ""../甲"", 版本 = ""^1.0.0"" }`n", $enc)
    [IO.File]::WriteAllText((Join-Path $pkgRoot "坏应用\主.jsh"), "打印(颜色.红)`n", $enc)

    & $micc pack --目录 "build\n11pkg\应用" -o "build\n11pkg\应用.jmb" | Out-Null
    $packCode = $LASTEXITCODE
    $pkgOut = ""
    if ($packCode -eq 0) {
        $pkgOut = ((& $jmrun "build\n11pkg\应用.jmb" 2>&1 | Out-String).TrimEnd() -replace "`r`n", "`n")
    }
    $total++
    if (($packCode -eq 0) -and ($pkgOut -eq "0`n绿")) {
        Write-Host "[通过] 跨包同名枚举隔离：依赖包内匹配仍正确"
    } else {
        Write-Host "[不通过] 跨包同名枚举隔离：依赖包内匹配仍正确"
        Write-Host $pkgOut
        $fail++
    }

    $o = (& $micc pack --目录 "build\n11pkg\坏应用" -o "build\n11pkg\坏应用.jmb" 2>&1 | Out-String)
    $code = $LASTEXITCODE
    $total++
    if (($code -ne 0) -and $o.Contains("E128")) {
        Write-Host "[通过] 根包直接引用依赖包枚举 报 E128"
    } else {
        Write-Host "[不通过] 根包直接引用依赖包枚举 报 E128"
        Write-Host $o
        $fail++
    }

    Write-Host ("共 " + $total + " 个用例，失败 " + $fail + " 个")
    if ($fail -gt 0) { exit 1 }
    exit 0
}
finally {
    Pop-Location
}
