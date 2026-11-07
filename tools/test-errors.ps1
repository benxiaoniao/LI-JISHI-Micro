$ErrorActionPreference = "Continue"
$root = Split-Path -Parent $PSScriptRoot

Push-Location $root
try {
    $micc = "compiler\target\release\micc.exe"
    $jmrun = "build\jmrun.exe"
    $cases = @(
        @{ file = "除零.jsh";       code = "E007"; stage = "运行" },
        @{ file = "小数除零.jsh";   code = "E007"; stage = "运行" },
        @{ file = "真除法.jsh";     code = "E023"; stage = "运行" },
        @{ file = "未定义名字.jsh"; code = "E004"; stage = "运行" },
        @{ file = "小数溢出.jsh";   code = "E009"; stage = "运行" },
        @{ file = "小数下标.jsh";   code = "E013"; stage = "运行" },
        @{ file = "闭包.jsh";       code = "E101"; stage = "编译" },
        @{ file = "指数.jsh";       code = "E102"; stage = "编译" },
        @{ file = "类.jsh";         code = "E103"; stage = "编译" }
    )
    $fail = 0
    foreach ($c in $cases) {
        $src = "tests\errors\" + $c.file
        $jmb = "build\err_" + [IO.Path]::GetFileNameWithoutExtension($c.file) + ".jmb"
        $out = (& $micc $src -o $jmb 2>&1 | Out-String)
        if ($LASTEXITCODE -eq 0) {
            $out += (& $jmrun $jmb 2>&1 | Out-String)
        }
        if ($out -match $c.code) {
            Write-Host ("[通过] " + $c.file + " -> " + $c.code + "（" + $c.stage + "）")
        } else {
            Write-Host ("[不通过] " + $c.file + " —— 没看到 " + $c.code)
            Write-Host $out
            $fail++
        }
    }
    Write-Host ("错误用例：失败 " + $fail + " 个")
    if ($fail -gt 0) { exit 1 }
    exit 0
}
finally {
    Pop-Location
}
