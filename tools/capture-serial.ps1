# 串口抓取：一条命令录板子输出
# 顺序很重要：先打开端口开始录制，再提示按复位，避免漏掉启动段。
# 用法：.\tools\capture-serial.ps1                      # COM3 / 115200 / 60 秒，自动落盘
#       .\tools\capture-serial.ps1 -Port COM5 -Seconds 120 -Out .\build\uart\run1.log
# 落盘文件是原始 UTF-8 字节（含固件输出的 \r\n），可直接用编辑器打开。

param(
    [string]$Port = "COM3",
    [int]$Baud = 115200,
    [int]$Seconds = 60,
    [string]$Out = ""
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot

if ($Out -eq "") {
    $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
    $dir = Join-Path $root "build\uart"
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $Out = Join-Path $dir ("uart-" + $stamp + ".log")
} else {
    $dir = Split-Path -Parent $Out
    if ($dir -and -not (Test-Path $dir)) {
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
    }
}

$sp = New-Object System.IO.Ports.SerialPort $Port, $Baud, "None", 8, "One"
# DTR / RTS 保持关闭：部分 USB 转串口会把它们接到板子的复位脚，误触复位
$sp.DtrEnable = $false
$sp.RtsEnable = $false
$sp.ReadTimeout = 1000

try {
    $sp.Open()
}
catch {
    throw ("打不开 " + $Port + "：" + $_.Exception.Message +
           "（用设备管理器确认端口号，或加 -Port COMx）")
}

Write-Host ("[已打开] " + $Port + " " + $Baud + " 8N1，录制 " + $Seconds + " 秒")
Write-Host ("[落盘] " + $Out)
Write-Host ""
Write-Host "== 现在可以按板上的复位键（RST）了 =="
Write-Host ""

$fs = [System.IO.File]::Create($Out)
$deadline = (Get-Date).AddSeconds($Seconds)
$total = 0
try {
    while ((Get-Date) -lt $deadline) {
        $n = $sp.BytesToRead
        if ($n -gt 0) {
            $buf = New-Object byte[] $n
            $got = $sp.Read($buf, 0, $n)
            $fs.Write($buf, 0, $got)
            $fs.Flush()
            $total += $got
        }
        Start-Sleep -Milliseconds 25
    }
}
finally {
    $fs.Close()
    $sp.Close()
}

Write-Host ""
Write-Host ("[完成] 收到 " + $total + " 字节 → " + $Out)
if ($total -eq 0) {
    Write-Host "[提示] 一个字节都没收到：检查 TX/RX 是否接反、波特率是否 115200、以及是否按了复位。"
    exit 2
}
exit 0
