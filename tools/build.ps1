param([switch]$SkipTests)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot "toolchains.ps1")

Push-Location $root
try {
    Write-Host "== 1/7 构建 micc（Rust）"
    # cargo 把进度（含 Finished）写到 stderr；Stop 会把任意 stderr 当成致命错误。
    # 这里只看退出码：成功时不打印噪声，失败时才把 cargo 输出带出来。
    $savedEap = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    $cargoLog = & cargo build --release --manifest-path "compiler/Cargo.toml" 2>&1
    $cargoExit = $LASTEXITCODE
    $ErrorActionPreference = $savedEap
    if ($cargoExit -ne 0) {
        $cargoLog | Out-Host
        throw "micc 构建失败"
    }
    $micc = "compiler/target/release/micc" + $ExeSuffix

    Write-Host "== 2/7 校验原生表头（与 compiler/src/native_spec.rs 同源）"
    & $micc --check-native
    if ($LASTEXITCODE -ne 0) { throw "原生表头与规格不一致" }

    Write-Host "== 3/7 生成操作码头文件"
    $header = & $micc --emit-header | Out-String
    [System.IO.File]::WriteAllText(
        "runtime/include/jm_opcodes.h",
        $header,
        (New-Object System.Text.UTF8Encoding($false)))

    Write-Host "== 4/7 构建主机运行时"
    if (-not $HostGcc) { throw "没有主机 gcc，无法构建 jmrun" }
    New-Item -ItemType Directory -Force -Path "build" | Out-Null
    # Linux/macOS 用 gnu99：-std=c99 会隐藏 POSIX 接口（popen / clock_gettime / nanosleep）。
    # Windows 保持 c99，行为与既有产物一致。
    $stdFlag = if ($ExeSuffix -eq "") { "-std=gnu99" } else { "-std=c99" }
    & $HostGcc $stdFlag -Wall -Wextra -O2 -DJM_HOST_DEBUG -DJM_DEBUG_HOOKS `
        -I "runtime/include" -I "runtime/src" `
        "runtime/src/jm_heap.c" "runtime/src/jm_vm.c" "runtime/src/jm_builtin.c" `
        "runtime/src/jm_loader.c" "runtime/src/jm_error.c" "runtime/src/jm_native.c" `
        "runtime/src/jm_debug.c" `
        "runtime/port/host/jm_port_host.c" "runtime/port/host/jm_host_native.c" `
        "runtime/port/host/jm_main.c" `
        -o ("build/jmrun" + $ExeSuffix)
    if ($LASTEXITCODE -ne 0) { throw "jmrun 构建失败" }

    Write-Host "== 5/7 构建原生 ABI v2 主机回归"
    & $HostGcc $stdFlag -Wall -Wextra -O2 `
        -I "runtime/include" -I "runtime/src" `
        "runtime/src/jm_heap.c" "runtime/src/jm_vm.c" "runtime/src/jm_builtin.c" `
        "runtime/src/jm_loader.c" "runtime/src/jm_error.c" "runtime/src/jm_native.c" `
        "runtime/src/jm_debug.c" "runtime/port/host/jm_port_host.c" `
        "tests/native-abi-v2.c" `
        -o ("build/native-abi-v2" + $ExeSuffix)
    if ($LASTEXITCODE -ne 0) { throw "原生 ABI v2 回归构建失败" }
    & ("build/native-abi-v2" + $ExeSuffix)
    if ($LASTEXITCODE -ne 0) { throw "原生 ABI v2 回归失败" }

    Write-Host "== 6/7 构建 native 表运行期主机回归"
    & $HostGcc $stdFlag -Wall -Wextra -O2 `
        -I "runtime/include" -I "runtime/src" `
        "runtime/src/jm_heap.c" "runtime/src/jm_vm.c" "runtime/src/jm_builtin.c" `
        "runtime/src/jm_loader.c" "runtime/src/jm_error.c" "runtime/src/jm_native.c" `
        "runtime/src/jm_debug.c" "runtime/port/host/jm_port_host.c" `
        "runtime/port/host/jm_host_native.c" "tests/native-host-runtime.c" `
        -o ("build/native-host-runtime" + $ExeSuffix)
    if ($LASTEXITCODE -ne 0) { throw "native 表运行期回归构建失败" }
    & ("build/native-host-runtime" + $ExeSuffix)
    if ($LASTEXITCODE -ne 0) { throw "native 表运行期回归失败" }

    Write-Host "== 7/7 编译示例"
    Get-ChildItem "examples" -Filter "*.jsh" | ForEach-Object {
        $out = "build/" + $_.BaseName + ".jmb"
        & $micc ("examples/" + $_.Name) -o $out | Out-Null
        if ($LASTEXITCODE -ne 0) { throw ("编译失败：" + $_.Name) }
        Write-Host ("   " + $_.Name + " -> " + $out)
    }

    if (-not $SkipTests) {
        Write-Host "== 回归"
        & (Join-Path $PSScriptRoot "test.ps1")
        if ($LASTEXITCODE -ne 0) { throw "回归失败" }
        & (Join-Path $PSScriptRoot "test-errors.ps1")
        if ($LASTEXITCODE -ne 0) { throw "错误用例失败" }
        & (Join-Path $PSScriptRoot "test-native-import.ps1")
        if ($LASTEXITCODE -ne 0) { throw "原生导入回归失败" }
        & (Join-Path $PSScriptRoot "test-packages.ps1")
        if ($LASTEXITCODE -ne 0) { throw "包用例失败" }
        & (Join-Path $PSScriptRoot "test-debug.ps1")
        if ($LASTEXITCODE -ne 0) { throw "调试表用例失败" }
    }
    Write-Host "完成。"
}
finally {
    Pop-Location
}
