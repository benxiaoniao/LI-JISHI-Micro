# 把 libjishim + jm_blob.c 打成 Arduino（STM32duino）库
#
# 产物：
#   build/arduino/LiJishim/                  可直接拷进 ~/Arduino/libraries/
#   build/arduino/LiJishim-<版本>.zip        同样的目录，打包分发
#
# 用法：
#   .\tools\pack-arduino.ps1                     # 默认内嵌 examples/你好.jsh
#   .\tools\pack-arduino.ps1 -Example 小数       # 换一个示例
#   .\tools\pack-arduino.ps1 -NoZip              # 只生成目录

param(
    [string]$Example = "你好",
    [string]$Out = "build/arduino",
    [switch]$NoZip
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot "toolchains.ps1")

$micc = Join-Path $root ("compiler/target/release/micc" + $ExeSuffix)
if (-not (Test-Path $micc)) { throw ("先构建 micc：" + $micc) }

$template = Join-Path $root "integrations/arduino"
if (-not (Test-Path (Join-Path $template "library.properties"))) {
    throw ("缺少 Arduino 库模板：" + $template)
}

$outRoot = if ([IO.Path]::IsPathRooted($Out)) { $Out } else { Join-Path $root $Out }
$libDir = Join-Path $outRoot "LiJishim"
$srcDir = Join-Path $libDir "src"

if (Test-Path $libDir) { Remove-Item $libDir -Recurse -Force }
New-Item -ItemType Directory -Force -Path $srcDir | Out-Null

Write-Host "== 1/5 编译示例为 .jmb（剥离调试段）"
$appJmb = Join-Path $outRoot "app.jmb"
& $micc (Join-Path $root ("examples/" + $Example + ".jsh")) -o $appJmb --strip-debug | Out-Null
if ($LASTEXITCODE -ne 0) { throw ("编译失败：" + $Example + ".jsh") }

Write-Host "== 2/5 生成内嵌字节数组 jm_blob.c"
& $micc --emit-blob $appJmb -o (Join-Path $srcDir "jm_blob.c")
if ($LASTEXITCODE -ne 0) { throw "生成 jm_blob.c 失败" }

Write-Host "== 3/5 拷入运行时与端口"
Copy-Item (Join-Path $template "library.properties") $libDir -Force
Copy-Item (Join-Path $template "src/*") $srcDir -Force
Copy-Item (Join-Path $template "examples") $libDir -Recurse -Force
Get-ChildItem (Join-Path $root "runtime/include") -Filter "*.h" | Copy-Item -Destination $srcDir -Force
Get-ChildItem (Join-Path $root "runtime/src") -Filter "*.h" | Copy-Item -Destination $srcDir -Force
Get-ChildItem (Join-Path $root "runtime/src") -Filter "*.c" | Copy-Item -Destination $srcDir -Force

Write-Host "== 4/5 自检库目录"
$need = @(
    "library.properties",
    "src/LiJishim.h", "src/LiJishim.cpp", "src/jm_port_arduino.cpp", "src/jm_blob.c",
    "src/jishim.h", "src/jm_port.h", "src/jm_internal.h",
    "src/jm_vm.c", "src/jm_heap.c", "src/jm_builtin.c", "src/jm_loader.c",
    "src/jm_error.c", "src/jm_native.c", "src/jm_debug.c",
    "examples/Hello/Hello.ino"
)
$missing = @($need | Where-Object { -not (Test-Path (Join-Path $libDir $_)) })
if ($missing.Count -gt 0) {
    throw ("库目录缺文件：" + ($missing -join "、"))
}

$version = ((Select-String -Path (Join-Path $libDir "library.properties") -Pattern "^version=(.+)$").Matches[0].Groups[1].Value).Trim()
$srcCount = (Get-ChildItem $srcDir -File | Measure-Object).Count
$srcBytes = (Get-ChildItem $srcDir -File | Measure-Object -Property Length -Sum).Sum

Write-Host "== 5/5 打包"
$zip = $null
if (-not $NoZip) {
    $zip = Join-Path $outRoot ("LiJishim-" + $version + ".zip")
    if (Test-Path $zip) { Remove-Item $zip -Force }
    Compress-Archive -Path $libDir -DestinationPath $zip
}

Write-Host ("库目录：" + $libDir)
Write-Host ("  src 文件 " + $srcCount + " 个，共 " + $srcBytes + " 字节；内嵌程序：" + $Example + ".jsh")
if ($zip) { Write-Host ("  压缩包：" + $zip + "（" + (Get-Item $zip).Length + " 字节）") }
