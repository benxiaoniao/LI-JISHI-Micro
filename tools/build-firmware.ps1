# 构建 STM32F103C8T6 固件骨架（砾 / JMB v1）
# 产物：build\stm32\jishim.elf、jishim.bin、jishim.hex、jishim.map
# 用法：.\tools\build-firmware.ps1 [-Example 你好]

param(
    [string]$Example = "你好",
    # 额外宏，用于体积/观测两档对照，例如 -Define "-DJM_STACK_DIAG=0"
    [string[]]$Define = @()
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
. "$PSScriptRoot\toolchains.ps1"

if (-not $ArmGcc) { throw "找不到 arm-none-eabi-gcc（设置 JM_ARM_GCC）" }

# specs 要用绝对路径：本机工具链按相对名查找会失败
$tcRoot = Split-Path (Split-Path $ArmGcc)
$NanoSpecs = Join-Path $tcRoot "arm-none-eabi\lib\nano.specs"
$NosysSpecs = Join-Path $tcRoot "arm-none-eabi\lib\nosys.specs"
foreach ($p in @($NanoSpecs, $NosysSpecs)) {
    if (-not (Test-Path $p)) { throw ("找不到 specs 文件：" + $p) }
}

Push-Location $root
try {
    $out = "build\stm32"
    New-Item -ItemType Directory -Force -Path $out | Out-Null

    Write-Host "== 1/6 构建 micc"
    # cargo 把进度（含 Finished）写到 stderr；Stop 会把任意 stderr 当成致命错误。
    # 这里只看退出码：成功时不打印 cargo 的进度噪声，失败时才把它的输出带出来。
    $savedEap = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    $cargoLog = & cargo build --release --manifest-path "compiler\Cargo.toml" 2>&1
    $cargoExit = $LASTEXITCODE
    $ErrorActionPreference = $savedEap
    if ($cargoExit -ne 0) {
        $cargoLog | Out-Host
        throw "micc 构建失败"
    }
    $micc = "compiler\target\release\micc.exe"

    Write-Host "== 2/6 校验原生表头（与 compiler/src/native_spec.rs 同源）"
    & $micc --check-native
    if ($LASTEXITCODE -ne 0) { throw "原生表头与规格不一致" }

    Write-Host "== 3/6 编译示例为 .jmb（保留调试段另存，内嵌用剥离版）"
    # app.debug.jmb 只给体积报告算「剥离前/后」两个数，不进固件。
    & $micc ("examples\" + $Example + ".jsh") -o "$out\app.debug.jmb" | Out-Null
    if ($LASTEXITCODE -ne 0) { throw ("编译失败：" + $Example + ".jsh") }
    & $micc ("examples\" + $Example + ".jsh") -o "$out\app.jmb" --strip-debug | Out-Null
    if ($LASTEXITCODE -ne 0) { throw ("编译失败：" + $Example + ".jsh") }

    Write-Host "== 4/6 生成内嵌字节数组"
    # 格式的唯一来源是 `micc --emit-blob`；这里不再内联拼 C 数组，避免两份实现漂移。
    & $micc --emit-blob "$out\app.jmb" -o "$out\jm_blob.c"
    if ($LASTEXITCODE -ne 0) { throw "生成 jm_blob.c 失败" }

    Write-Host "== 5/6 交叉编译并链接"
    $src = @(
        "runtime\src\jm_heap.c", "runtime\src\jm_vm.c", "runtime\src\jm_builtin.c",
        "runtime\src\jm_loader.c", "runtime\src\jm_error.c",
        "runtime\src\jm_native.c",
        "runtime\src\jm_debug.c",
        "firmware\stm32f103\stm32f103_port.c",
        "firmware\stm32f103\f103_native.c",
        "firmware\stm32f103\jmk.c",
        "firmware\stm32f103\main.c",
        "$out\jm_blob.c"
    )
    & $ArmGcc `
        -mcpu=cortex-m3 -mthumb -std=c99 -Os -g `
        -Wall -Wextra -ffreestanding -fno-common `
        -fdata-sections -ffunction-sections `
        -DJM_HEAP_SIZE=6144 `
        @Define `
        -I "runtime\include" -I "runtime\src" -I "firmware\stm32f103" `
        -nostartfiles `
        -T "firmware\stm32f103\stm32f103c8.ld" `
        "-Wl,--gc-sections" "-Wl,--print-memory-usage" `
        "-Wl,-Map=$out\jishim.map" `
        "-specs=$NanoSpecs" "-specs=$NosysSpecs" `
        "firmware\stm32f103\startup_stm32f103c8.s" `
        "firmware\stm32f103\jmk_port_cm3.s" `
        $src `
        -o "$out\jishim.elf"
    if ($LASTEXITCODE -ne 0) { throw "链接失败" }

    Write-Host "== 6/6 生成 bin / hex 并统计体积"
    $objcopy = Join-Path (Split-Path $ArmGcc) "arm-none-eabi-objcopy.exe"
    & $objcopy -O binary "$out\jishim.elf" "$out\jishim.bin"
    & $objcopy -O ihex "$out\jishim.elf" "$out\jishim.hex"
    if ($ArmSize) {
        & $ArmSize "$out\jishim.elf"
    }
    Write-Host ("固件产物：" + (Join-Path $root "$out") + "\jishim.{elf,bin,hex}")
}
finally {
    Pop-Location
}
