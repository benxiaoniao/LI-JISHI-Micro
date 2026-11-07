param([switch]$SkipTests)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
. "$PSScriptRoot\toolchains.ps1"

Push-Location $root
try {
    Write-Host "== 1/4 构建 micc（Rust）"
    & cargo build --release --manifest-path "compiler\Cargo.toml"
    if ($LASTEXITCODE -ne 0) { throw "micc 构建失败" }
    $micc = "compiler\target\release\micc.exe"

    Write-Host "== 2/4 生成操作码头文件"
    $header = & $micc --emit-header | Out-String
    [System.IO.File]::WriteAllText(
        "runtime\include\jm_opcodes.h",
        $header,
        (New-Object System.Text.UTF8Encoding($false)))

    Write-Host "== 3/4 构建主机运行时"
    if (-not $HostGcc) { throw "没有主机 gcc，无法构建 jmrun" }
    New-Item -ItemType Directory -Force -Path "build" | Out-Null
    & $HostGcc -std=c99 -Wall -Wextra -O2 `
        -I "runtime\include" -I "runtime\src" `
        "runtime\src\jm_heap.c" "runtime\src\jm_vm.c" "runtime\src\jm_builtin.c" `
        "runtime\src\jm_loader.c" "runtime\src\jm_error.c" "runtime\src\jm_native.c" `
        "runtime\port\host\jm_port_host.c" "runtime\port\host\jm_main.c" `
        -o "build\jmrun.exe"
    if ($LASTEXITCODE -ne 0) { throw "jmrun 构建失败" }

    Write-Host "== 4/4 编译示例"
    Get-ChildItem "examples" -Filter "*.jsh" | ForEach-Object {
        $out = "build\" + $_.BaseName + ".jmb"
        & $micc ("examples\" + $_.Name) -o $out | Out-Null
        if ($LASTEXITCODE -ne 0) { throw ("编译失败：" + $_.Name) }
        Write-Host ("   " + $_.Name + " -> " + $out)
    }

    if (-not $SkipTests) {
        Write-Host "== 回归"
        & "$PSScriptRoot\test.ps1"
        if ($LASTEXITCODE -ne 0) { throw "回归失败" }
        & "$PSScriptRoot\test-errors.ps1"
        if ($LASTEXITCODE -ne 0) { throw "错误用例失败" }
    }
    Write-Host "完成。"
}
finally {
    Pop-Location
}
