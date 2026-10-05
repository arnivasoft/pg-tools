<#
.SYNOPSIS
  Yayın (release) exe'si için duman testi: başlar, pencere açar ve çökmeden ayakta kalır mı?
.DESCRIPTION
  exe'yi geçici, boş APPDATA/LOCALAPPDATA dizinleriyle başlatır (kullanıcının gerçek yapılandırmasına ve
  loglarına dokunmaz; sunucu listesi boş, hiçbir sunucuya bağlanılmaz, parola kullanılmaz).
  - MainWindowHandle en fazla 10 sn içinde oluşmazsa hata,
  - süreç -Seconds dolmadan çıkarsa ExitCode ile hata (ör. release'de açılıştan ~1 sn sonra
    STATUS_STACK_OVERFLOW = -1073741571 / 0xC00000FD),
  - sonunda uygulama + WebView2 alt süreç ağacının tamamını kapatır,
  - geçici %LOCALAPPDATA%\pg-tools\logs altında "panicked" / "stack overflow" geçiyorsa hata.
  Çıkış kodu 0 = başarılı, 1 = başarısız.
.PARAMETER Exe      Sınanacak exe (varsayılan: target\release\pg-tools.exe)
.PARAMETER Seconds  Pencere oluştuktan sonra sürecin ayakta kalması gereken süre (varsayılan 6)
#>
param(
    [string]$Exe = (Join-Path (Split-Path -Parent $PSScriptRoot) "target\release\pg-tools.exe"),
    [int]$Seconds = 6
)
$ErrorActionPreference = "Stop"

function Get-Tree([int]$rootPid) {
    $all = Get-CimInstance Win32_Process
    $ids = New-Object System.Collections.Generic.HashSet[int]
    [void]$ids.Add($rootPid)
    do {
        $added = $false
        foreach ($p in $all) {
            if ($ids.Contains([int]$p.ParentProcessId) -and $ids.Add([int]$p.ProcessId)) { $added = $true }
        }
    } while ($added)
    , $ids
}

if (-not (Test-Path $Exe)) { Write-Output "BAŞARISIZ: exe bulunamadı: $Exe"; exit 1 }

$root = Join-Path $env:TEMP ("pgt-smoke-" + [guid]::NewGuid().ToString("N"))
$appdata = Join-Path $root "Roaming"
$localappdata = Join-Path $root "Local"
New-Item -ItemType Directory $appdata, $localappdata | Out-Null
$origAppData = $env:APPDATA
$origLocalAppData = $env:LOCALAPPDATA
$failure = $null
$proc = $null
try {
    $env:APPDATA = $appdata
    $env:LOCALAPPDATA = $localappdata
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $proc = Start-Process $Exe -PassThru
    $null = $proc.Handle   # tutamacı önbelleğe al; yoksa süreç çıktıktan sonra ExitCode boş kalabilir
    # Alt süreç (WebView2) ortamı devraldı; kendi ortamımızı hemen geri yükleyebiliriz.
    $env:APPDATA = $origAppData
    $env:LOCALAPPDATA = $origLocalAppData

    while (-not $proc.MainWindowHandle -or $proc.MainWindowHandle -eq 0) {
        if ($proc.HasExited) { $failure = "pencere oluşmadan çıktı (kod $($proc.ExitCode))"; break }
        if ($sw.Elapsed.TotalSeconds -gt 10) { $failure = "10 sn içinde ana pencere oluşmadı"; break }
        Start-Sleep -Milliseconds 50; $proc.Refresh()
    }
    if (-not $failure) {
        $windowMs = $sw.ElapsedMilliseconds
        Write-Output "Pencere $windowMs ms'de oluştu; $Seconds sn izleniyor..."
        $watch = [Diagnostics.Stopwatch]::StartNew()
        while ($watch.Elapsed.TotalSeconds -lt $Seconds) {
            if ($proc.HasExited) {
                $failure = "pencereden $([math]::Round($watch.Elapsed.TotalSeconds, 1)) sn sonra çıktı (kod $($proc.ExitCode))"
                break
            }
            Start-Sleep -Milliseconds 100
        }
    }
} finally {
    $env:APPDATA = $origAppData
    $env:LOCALAPPDATA = $origLocalAppData
    if ($proc -and -not $proc.HasExited) {
        $ids = Get-Tree $proc.Id
        Get-Process -Id ([int[]]@($ids)) -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
        Start-Sleep -Seconds 2   # WebView2 süreçlerinin dosya kilitlerini bırakması için
    }
}

$logDir = Join-Path $localappdata "pg-tools\logs"
if (Test-Path $logDir) {
    $hits = Get-ChildItem $logDir -File | Select-String -Pattern "panicked", "stack overflow" -SimpleMatch
    if ($hits) {
        $msg = "logda panik/yığın taşması: " + (($hits | Select-Object -First 3 | ForEach-Object { $_.Line.Trim() }) -join " | ")
        $failure = if ($failure) { "$failure; $msg" } else { $msg }
    }
} elseif (-not $failure) {
    Write-Output "Uyarı: log dizini oluşmadı ($logDir)"
}

Remove-Item $root -Recurse -Force -ErrorAction SilentlyContinue

if ($failure) {
    Write-Output "BAŞARISIZ: $failure"
    exit 1
}
Write-Output "BAŞARILI: exe başladı, pencere açıldı, $Seconds sn sonra hâlâ çalışıyordu."
exit 0
