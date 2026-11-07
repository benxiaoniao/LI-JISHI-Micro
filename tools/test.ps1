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
    Write-Host ("共 " + $total + " 个用例，失败 " + $fail + " 个")
    if ($fail -gt 0) { exit 1 }
    exit 0
}
finally {
    Pop-Location
}
