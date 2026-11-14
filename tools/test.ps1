$ErrorActionPreference = "Continue"
$root = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot "toolchains.ps1")

Push-Location $root
try {
    $micc = Join-Path $root ("compiler/target/release/micc" + $ExeSuffix)
    $jmrun = Join-Path $root ("build/jmrun" + $ExeSuffix)
    if (-not (Test-Path $jmrun)) { throw "先运行 tools/build.ps1" }

    $fail = 0
    $total = 0
    Get-ChildItem "examples" -Filter "*.jsh" | Sort-Object Name | ForEach-Object {
        $total++
        $name = $_.BaseName
        $jmb = "build/" + $name + ".jmb"
        & $micc ("examples/" + $_.Name) -o $jmb | Out-Null
        if ($LASTEXITCODE -ne 0) {
            Write-Host ("[编译失败] " + $_.Name)
            $fail++
            return
        }
        $actual = ((& $jmrun $jmb 2>&1 | Out-String).TrimEnd() -replace "`r`n", "`n")
        $gold = "tests/expected/" + $name + ".txt"
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
    $blobJmb = "build/你好.jmb"
    if (Test-Path $blobJmb) {
        Remove-Item "build/jm_blob.c" -Force -ErrorAction SilentlyContinue
        & $micc --emit-blob $blobJmb | Out-Null
        $total++
        if (($LASTEXITCODE -eq 0) -and (Test-Path "build/jm_blob.c")) {
            Write-Host "[通过] emit-blob 默认输出同目录 jm_blob.c"
        } else {
            Write-Host "[不通过] emit-blob 默认输出同目录 jm_blob.c"
            $fail++
        }
        $text = Get-Content "build/jm_blob.c" -Raw -Encoding UTF8
        $len = (Get-Item $blobJmb).Length
        $total++
        if (($text -match "const unsigned char jm_app_jmb\[\]") -and
            ($text -match ("jm_app_jmb_len = " + $len + "u"))) {
            Write-Host "[通过] emit-blob 符号名与内嵌长度一致"
        } else {
            Write-Host "[不通过] emit-blob 符号名与内嵌长度一致"
            $fail++
        }
        $out2 = "build/jm_blob-指定.c"
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
    [IO.File]::WriteAllText((Join-Path $root "build/pin-bad.jsh"), "导入 引脚`n引脚写(48, 1)`n", $enc)
    & $micc "build/pin-bad.jsh" -o "build/pin-bad.jmb" | Out-Null
    $o = (& $jmrun "build/pin-bad.jmb" 2>&1 | Out-String)
    $code = $LASTEXITCODE
    $total++
    if (($code -ne 0) -and $o.Contains("E030")) {
        Write-Host "[通过] 引脚写(48) 报 E030"
    } else {
        Write-Host "[不通过] 引脚写(48) 报 E030"
        Write-Host $o
        $fail++
    }

    [IO.File]::WriteAllText((Join-Path $root "build/pin-neg.jsh"), "导入 引脚`n引脚读(-1)`n", $enc)
    & $micc "build/pin-neg.jsh" -o "build/pin-neg.jmb" | Out-Null
    $o = (& $jmrun "build/pin-neg.jmb" 2>&1 | Out-String)
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
    [IO.File]::WriteAllText((Join-Path $root "build/enum-bad.jsh"), "枚举 颜色:`n    红`n    绿`n`n打印(颜色.紫)`n", $enc)
    $o = (& $micc "build/enum-bad.jsh" -o "build/enum-bad.jmb" 2>&1 | Out-String)
    $code = $LASTEXITCODE
    $total++
    if (($code -ne 0) -and $o.Contains("E139")) {
        Write-Host "[通过] 枚举未知成员 报 E139"
    } else {
        Write-Host "[不通过] 枚举未知成员 报 E139"
        Write-Host $o
        $fail++
    }

    [IO.File]::WriteAllText((Join-Path $root "build/match-dup.jsh"), "枚举 颜色:`n    红`n    绿`n`n匹配 0:`n    情形 0:`n        打印(1)`n    情形 颜色.红:`n        打印(2)`n", $enc)
    $o = (& $micc "build/match-dup.jsh" -o "build/match-dup.jmb" 2>&1 | Out-String)
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
    $pkgRoot = Join-Path $root "build/n11pkg"
    foreach ($d in @("甲", "乙", "应用", "坏应用")) {
        New-Item -ItemType Directory -Force -Path (Join-Path $pkgRoot $d) | Out-Null
    }
    [IO.File]::WriteAllText((Join-Path $pkgRoot "甲/砾包.toml"), "[包]`n名字 = ""甲""`n版本 = ""1.0.0""`n入口 = ""主.jsh""`n`n[导出]`n名字 = [""颜色名""]`n", $enc)
    [IO.File]::WriteAllText((Join-Path $pkgRoot "甲/主.jsh"), "枚举 颜色:`n    红`n    绿`n`n函数 颜色名(k):`n    匹配 k:`n        情形 颜色.绿:`n            返回 ""绿""`n        情形 其他:`n            返回 ""其他""`n", $enc)
    [IO.File]::WriteAllText((Join-Path $pkgRoot "乙/砾包.toml"), "[包]`n名字 = ""乙""`n版本 = ""1.0.0""`n入口 = ""主.jsh""`n", $enc)
    [IO.File]::WriteAllText((Join-Path $pkgRoot "乙/主.jsh"), "枚举 颜色:`n    绿`n    蓝`n", $enc)
    [IO.File]::WriteAllText((Join-Path $pkgRoot "应用/砾包.toml"), "[包]`n名字 = ""应用""`n版本 = ""1.0.0""`n入口 = ""主.jsh""`n`n[依赖]`n甲 = { 路径 = ""../甲"", 版本 = ""^1.0.0"" }`n乙 = { 路径 = ""../乙"", 版本 = ""^1.0.0"" }`n", $enc)
    [IO.File]::WriteAllText((Join-Path $pkgRoot "应用/主.jsh"), "枚举 颜色:`n    蓝`n`n打印(颜色.蓝)`n打印(甲.颜色名(1))`n", $enc)
    [IO.File]::WriteAllText((Join-Path $pkgRoot "坏应用/砾包.toml"), "[包]`n名字 = ""坏应用""`n版本 = ""1.0.0""`n入口 = ""主.jsh""`n`n[依赖]`n甲 = { 路径 = ""../甲"", 版本 = ""^1.0.0"" }`n", $enc)
    [IO.File]::WriteAllText((Join-Path $pkgRoot "坏应用/主.jsh"), "打印(颜色.红)`n", $enc)

    & $micc pack --目录 "build/n11pkg/应用" -o "build/n11pkg/应用.jmb" | Out-Null
    $packCode = $LASTEXITCODE
    $pkgOut = ""
    if ($packCode -eq 0) {
        $pkgOut = ((& $jmrun "build/n11pkg/应用.jmb" 2>&1 | Out-String).TrimEnd() -replace "`r`n", "`n")
    }
    $total++
    if (($packCode -eq 0) -and ($pkgOut -eq "0`n绿")) {
        Write-Host "[通过] 跨包同名枚举隔离：依赖包内匹配仍正确"
    } else {
        Write-Host "[不通过] 跨包同名枚举隔离：依赖包内匹配仍正确"
        Write-Host $pkgOut
        $fail++
    }

    $o = (& $micc pack --目录 "build/n11pkg/坏应用" -o "build/n11pkg/坏应用.jmb" 2>&1 | Out-String)
    $code = $LASTEXITCODE
    $total++
    if (($code -ne 0) -and $o.Contains("E128")) {
        Write-Host "[通过] 根包直接引用依赖包枚举 报 E128"
    } else {
        Write-Host "[不通过] 根包直接引用依赖包枚举 报 E128"
        Write-Host $o
        $fail++
    }

    # ---------- LI004：断言与 --no-assert ----------
    & $micc "examples/断言.jsh" -o "build/assert-on.jmb" | Out-Null
    $onCode = $LASTEXITCODE
    $onOut = ""
    $onSize = 0
    if ($onCode -eq 0) {
        $onOut = ((& $jmrun "build/assert-on.jmb" 2>&1 | Out-String).TrimEnd() -replace "`r`n", "`n")
        $onSize = (Get-Item "build/assert-on.jmb").Length
    }
    & $micc "examples/断言.jsh" -o "build/assert-off.jmb" --no-assert | Out-Null
    $offCode = $LASTEXITCODE
    $offOut = ""
    $offSize = 0
    if ($offCode -eq 0) {
        $offOut = ((& $jmrun "build/assert-off.jmb" 2>&1 | Out-String).TrimEnd() -replace "`r`n", "`n")
        $offSize = (Get-Item "build/assert-off.jmb").Length
    }
    $total++
    if (($onCode -eq 0) -and ($onOut -eq "42`n捕获1 n 不能为负`n捕获2 断言失败`n继续") -and
        ($offCode -eq 0) -and ($offOut -eq "42`n-1`n200`n继续") -and ($offSize -lt $onSize)) {
        Write-Host "[通过] --no-assert 整句消除（不触发断言且产物更小）"
    } else {
        Write-Host "[不通过] --no-assert 整句消除（不触发断言且产物更小）"
        Write-Host $onOut
        Write-Host $offOut
        Write-Host ("大小 " + $onSize + " / " + $offSize)
        $fail++
    }

    [IO.File]::WriteAllText((Join-Path $root "build/assert-uncaught.jsh"), "断言 假, ""坏输入""`n", $enc)
    & $micc "build/assert-uncaught.jsh" -o "build/assert-uncaught.jmb" | Out-Null
    $o = (& $jmrun "build/assert-uncaught.jmb" 2>&1 | Out-String)
    $code = $LASTEXITCODE
    $total++
    if (($code -ne 0) -and $o.Contains("E027") -and $o.Contains("坏输入")) {
        Write-Host "[通过] 未捕获的断言报 E027 并带消息"
    } else {
        Write-Host "[不通过] 未捕获的断言报 E027 并带消息"
        Write-Host $o
        $fail++
    }

    # ---------- LI015：--strict-type ----------
    $strictGold = ((Get-Content "tests/expected/类型标注.txt" -Raw -Encoding UTF8).TrimEnd() -replace "`r`n", "`n")
    & $micc "examples/类型标注.jsh" -o "build/类型标注-strict.jmb" --strict-type | Out-Null
    $strictCode = $LASTEXITCODE
    $strictOut = ""
    if ($strictCode -eq 0) {
        $strictOut = ((& $jmrun "build/类型标注-strict.jmb" 2>&1 | Out-String).TrimEnd() -replace "`r`n", "`n")
    }
    $total++
    if (($strictCode -eq 0) -and ($strictOut -eq $strictGold)) {
        Write-Host "[通过] --strict-type 接受合法标注并正常执行"
    } else {
        Write-Host "[不通过] --strict-type 接受合法标注并正常执行"
        Write-Host $strictOut
        $fail++
    }

    $strictBad = @(
        @{ file = "strict-arg.jsh"; src = "函数 取(x: 整数) -> 整数:`n    返回 x`n打印(取(""坏""))`n"; why = "实参类型不符" },
        @{ file = "strict-ret.jsh"; src = "函数 坏() -> 整数:`n    返回 ""文本""`n"; why = "返回值类型不符" },
        @{ file = "strict-unknown.jsh"; src = "函数 未知(x: 数字):`n    返回 x`n"; why = "类型名未知" },
        @{ file = "strict-new.jsh"; src = "类 甲:`n    令 v = 0`n    函数 初始化(a: 整数):`n        本.v = a`n令 c = 新建 甲(""坏"")`n"; why = "构造实参类型不符" }
    )
    foreach ($c in $strictBad) {
        [IO.File]::WriteAllText((Join-Path $root ("build/" + $c.file)), $c.src, $enc)
        $o = (& $micc ("build/" + $c.file) -o "build/strict.jmb" --strict-type 2>&1 | Out-String)
        $total++
        if (($LASTEXITCODE -ne 0) -and $o.Contains("E143")) {
            Write-Host ("[通过] --strict-type " + $c.why + " 报 E143")
        } else {
            Write-Host ("[不通过] --strict-type " + $c.why + " 报 E143")
            Write-Host $o
            $fail++
        }
    }

    # 默认保持宽松：同样的“实参类型不符”不加开关仍能编过
    [IO.File]::WriteAllText((Join-Path $root "build/strict-lax.jsh"), "函数 取(x: 整数) -> 整数:`n    返回 x`n打印(取(""坏""))`n", $enc)
    & $micc "build/strict-lax.jsh" -o "build/strict-lax.jmb" | Out-Null
    $total++
    if ($LASTEXITCODE -eq 0) {
        Write-Host "[通过] 不加 --strict-type 时标注仍被忽略"
    } else {
        Write-Host "[不通过] 不加 --strict-type 时标注仍被忽略"
        $fail++
    }

    # 子类实例可以传给父类标注
    [IO.File]::WriteAllText((Join-Path $root "build/strict-sub.jsh"), "类 父:`n    令 v = 0`n类 子 继承 父:`n    令 w = 0`n函数 收(x: 父) -> 整数:`n    返回 1`n打印(收(新建 子()))`n", $enc)
    $o = (& $micc "build/strict-sub.jsh" -o "build/strict-sub.jmb" --strict-type 2>&1 | Out-String)
    $total++
    if ($LASTEXITCODE -eq 0) {
        Write-Host "[通过] --strict-type 允许子类实例传给父类标注"
    } else {
        Write-Host "[不通过] --strict-type 允许子类实例传给父类标注"
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
