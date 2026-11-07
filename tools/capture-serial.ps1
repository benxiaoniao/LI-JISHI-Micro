# 串口抓取：一条命令录板子输出
# 顺序很重要：先打开端口开始录制，再提示按复位，避免漏掉启动段。
# 用法：.\tools\capture-serial.ps1                      # COM3 / 115200 / 60 秒，自动落盘
#       .\tools\capture-serial.ps1 -Port COM5 -Seconds 120 -Out .\build\uart\run1.log
# 落盘文件是原始 UTF-8 字节（含固件输出的 \r\n），可直接用编辑器打开。
#
# 注意：本脚本是同步阻塞式读取，必须在「前台」或「工具托管的常驻后台」里运行。
#       不要用 Start-Process 起子进程：子进程会随宿主终端被回收，
#       表现是端口打开了一瞬、只写入了驱动缓冲区里的旧数据，看起来在录其实没听众。
#       录制期间日志以「共享读」方式打开，其它工具可以边录边读。

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

# 共享读方式落盘：录制过程中外部工具也能读这份日志（旧版 File.Create 是独占锁）
$fs = New-Object System.IO.FileStream(
    $Out, [System.IO.FileMode]::Create,
    [System.IO.FileAccess]::Write, [System.IO.FileShare]::ReadWrite)

# 自检：开端口后先观察 2 秒，确认端口真的在收数据
$probeDeadline = (Get-Date).AddSeconds(2)
$probe = 0
while ((Get-Date) -lt $probeDeadline) {
    $n = $sp.BytesToRead
    if ($n -gt 0) {
        $buf = New-Object byte[] $n
        $got = $sp.Read($buf, 0, $n)
        $fs.Write($buf, 0, $got)
        $fs.Flush()
        $probe += $got
    }
    Start-Sleep -Milliseconds 25
}
Write-Host ("[自检] 打开端口后 2 秒收到 " + $probe + " 字节")
if ($probe -eq 0) {
    Write-Host "[告警] 当前 0 字节：可能还没复位/板子无输出、TX-RX 接反、波特率不对，或端口被别的程序占用。"
    Write-Host "       若尚未按复位，这条告警属正常；按复位后应立刻开始出数据。"
}

$deadline = (Get-Date).AddSeconds($Seconds)
$total = $probe
$lastReport = Get-Date
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
        if (((Get-Date) - $lastReport).TotalSeconds -ge 10) {
            $left = [int]($deadline - (Get-Date)).TotalSeconds
            Write-Host ("[进度] 已收 " + $total + " 字节，剩余 " + $left + " 秒")
            $lastReport = Get-Date
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
