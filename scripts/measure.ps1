<#
.SYNOPSIS
  pg-tools exe'sinin açılış süresini ve boşta belleğini ölçer.
.DESCRIPTION
  Her çalıştırma ayrı bir geçici APPDATA ile başlar (kullanıcının gerçek yapılandırmasına dokunmaz),
  pencere tutamacı oluşana kadar geçen süreyi ölçer, 8 sn boşta bekler, sonra uygulama + WebView2
  alt süreçlerinin özel çalışma kümesi (Görev Yöneticisi "Bellek" sütunu) ve çalışma kümesi
  toplamlarını okur. Başlattığı tüm süreçleri kapatır.
.PARAMETER Exe   Ölçülecek exe (varsayılan: target\release\pg-tools.exe)
.PARAMETER Runs  Çalıştırma sayısı
#>
param(
    [string]$Exe = (Join-Path (Split-Path -Parent $PSScriptRoot) "target\release\pg-tools.exe"),
    [int]$Runs = 3
)
$ErrorActionPreference = "Stop"
if (-not (Test-Path $Exe)) { throw "exe bulunamadı: $Exe" }

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

$origAppData = $env:APPDATA
$results = @()
try {
    for ($i = 1; $i -le $Runs; $i++) {
        $appdata = Join-Path $env:TEMP ("pgt-measure-" + [guid]::NewGuid().ToString("N"))
        New-Item -ItemType Directory $appdata | Out-Null
        $env:APPDATA = $appdata
        $sw = [Diagnostics.Stopwatch]::StartNew()
        $proc = Start-Process $Exe -PassThru
        while (-not $proc.MainWindowHandle -or $proc.MainWindowHandle -eq 0) {
            if ($proc.HasExited) { throw "exe erken çıktı (kod $($proc.ExitCode))" }
            Start-Sleep -Milliseconds 20; $proc.Refresh()
        }
        $windowMs = $sw.ElapsedMilliseconds
        Start-Sleep -Seconds 8   # içerik çizimi ve WebView2 süreçlerinin yerleşmesi
        $ids = Get-Tree $proc.Id
        $perf = Get-CimInstance Win32_PerfFormattedData_PerfProc_Process | Where-Object { $ids.Contains([int]$_.IDProcess) }
        $privateWs = ($perf | Measure-Object WorkingSetPrivate -Sum).Sum / 1MB
        $ws = ($perf | Measure-Object WorkingSet -Sum).Sum / 1MB
        $rust = ($perf | Where-Object { [int]$_.IDProcess -eq $proc.Id }).WorkingSetPrivate / 1MB
        $results += [pscustomobject]@{
            Run = $i; WindowMs = $windowMs; TotalPrivateWsMB = [math]::Round($privateWs, 1)
            TotalWsMB = [math]::Round($ws, 1); RustPrivateWsMB = [math]::Round($rust, 1); Processes = $ids.Count
        }
        $ids = Get-Tree $proc.Id
        Get-Process -Id ([int[]]@($ids)) -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
        Start-Sleep -Seconds 2
        Remove-Item $appdata -Recurse -Force -ErrorAction SilentlyContinue
    }
} finally {
    $env:APPDATA = $origAppData
}
$results | Format-Table -AutoSize | Out-String | Write-Output
$med = { param($p) ($results | Sort-Object $p)[[int][math]::Floor($Runs / 2)].$p }
"Medyan: pencere $(& $med 'WindowMs') ms, toplam özel WS $(& $med 'TotalPrivateWsMB') MB, toplam WS $(& $med 'TotalWsMB') MB, Rust özel WS $(& $med 'RustPrivateWsMB') MB"
