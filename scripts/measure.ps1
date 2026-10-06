<#
.SYNOPSIS
  pg-tools exe'sinin açılış süresini ve boşta belleğini ölçer.
.DESCRIPTION
  Her çalıştırma ayrı bir geçici APPDATA ile başlar (kullanıcının gerçek yapılandırmasına dokunmaz),
  ana pencere GÖRÜNÜR olana kadar geçen süreyi ölçer (pencere gizli oluşturulur, ön yüz tema ve ilk çizimden
  sonra gösterir; yani süre ön yüz paketinin çalışmasını kapsar), 8 sn boşta bekler, sonra uygulama + WebView2
  alt süreçlerinin özel çalışma kümesi (Görev Yöneticisi "Bellek" sütunu) ve çalışma kümesi
  toplamlarını okur (MiB = 1.048.576 bayt; PowerShell'in 1MB sabiti). Başlattığı tüm süreçleri kapatır.
  UTF-8 BOM ile kaydedilmiştir: Windows PowerShell 5.1 BOM'suz dosyayı ANSI sayar ve Türkçe çıktı bozulur.
.PARAMETER Exe   Ölçülecek exe (varsayılan: target\release\pg-tools.exe)
                 Ön yüzü gömülü exe gerekir: `bun run tauri build --no-bundle` ile derleyin (düz cargo build reddedilir).
.PARAMETER Runs  Çalıştırma sayısı
#>
param(
    [string]$Exe = (Join-Path (Split-Path -Parent $PSScriptRoot) "target\release\pg-tools.exe"),
    [int]$Runs = 3
)
$ErrorActionPreference = "Stop"
if (-not (Test-Path $Exe)) { throw "exe bulunamadı: $Exe" }
# Gömülü ön yüz denetimi (bkz. smoke-release.ps1): düz cargo build exe'si devUrl'ye yönelir, ölçüm gerçek arayüzü yansıtmaz.
$exeText = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($Exe))
if ($exeText.IndexOf("assets/index-", [StringComparison]::Ordinal) -lt 0) {
    throw "exe ön yüzü gömmüyor (cargo build?). 'bun run tauri build --no-bundle' ile derleyin: $Exe"
}
Remove-Variable exeText

# Görünür ana pencere denetimi. Process.MainWindowHandle KULLANILMAZ: tao'nun "Tao Thread Event Target" yardımcı
# penceresi (16x16, WS_VISIBLE) süreç başlar başlamaz onu doldurur, yani ana pencere oluşmadan/görünmeden sıfırdan
# farklıdır. Ana pencere ("Tauri Window" sınıfı) gizli oluşturulur ve ön yüz ilk çizimden sonra gösterir (window.rs);
# burada o pencere görünür olana kadar beklenir. Add-Type derlemesi zamanlayıcı başlamadan önce yapılır.
if (-not ("PgtWindow" -as [type])) { Add-Type -TypeDefinition @"
using System; using System.Runtime.InteropServices; using System.Text;
public static class PgtWindow {
    delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    public static bool MainVisible(int pid) {
        bool found = false;
        EnumWindows((h, l) => {
            uint p; GetWindowThreadProcessId(h, out p);
            if (p != (uint)pid || !IsWindowVisible(h)) return true;
            var c = new StringBuilder(64); GetClassName(h, c, 64);
            if (c.ToString() == "Tauri Window") { found = true; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
"@ }

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
        while (-not [PgtWindow]::MainVisible($proc.Id)) {
            if ($proc.HasExited) { throw "exe erken çıktı (kod $($proc.ExitCode))" }
            if ($sw.Elapsed.TotalSeconds -gt 10) { throw "10 sn içinde ana pencere görünmedi" }
            Start-Sleep -Milliseconds 10
        }
        $windowMs = $sw.ElapsedMilliseconds
        Start-Sleep -Seconds 8   # içerik çizimi ve WebView2 süreçlerinin yerleşmesi
        $ids = Get-Tree $proc.Id
        $perf = Get-CimInstance Win32_PerfFormattedData_PerfProc_Process | Where-Object { $ids.Contains([int]$_.IDProcess) }
        $privateWs = ($perf | Measure-Object WorkingSetPrivate -Sum).Sum / 1MB
        $ws = ($perf | Measure-Object WorkingSet -Sum).Sum / 1MB
        $rust = ($perf | Where-Object { [int]$_.IDProcess -eq $proc.Id }).WorkingSetPrivate / 1MB
        $results += [pscustomobject]@{
            Run = $i; WindowMs = $windowMs; TotalPrivateWsMiB = [math]::Round($privateWs, 1)
            TotalWsMiB = [math]::Round($ws, 1); RustPrivateWsMiB = [math]::Round($rust, 1); Processes = $ids.Count
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
"Medyan: pencere $(& $med 'WindowMs') ms, toplam özel WS $(& $med 'TotalPrivateWsMiB') MiB, toplam WS $(& $med 'TotalWsMiB') MiB, Rust özel WS $(& $med 'RustPrivateWsMiB') MiB"
