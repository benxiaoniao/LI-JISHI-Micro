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

    Write-Host ("共 " + $total + " 个用例，失败 " + $fail + " 个")
    if ($fail -gt 0) { exit 1 }
    exit 0
}
finally {
    Pop-Location
}
