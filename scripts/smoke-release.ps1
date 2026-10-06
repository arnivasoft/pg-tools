<#
.SYNOPSIS
  Yayın (release) exe'si için duman testi: başlar, pencere açar ve çökmeden ayakta kalır mı?
.DESCRIPTION
  exe'yi geçici, boş APPDATA/LOCALAPPDATA dizinleriyle başlatır (kullanıcının gerçek yapılandırmasına ve
  loglarına dokunmaz; sunucu listesi boş, hiçbir sunucuya bağlanılmaz, parola kullanılmaz).
  - ana pencere en fazla 10 sn içinde GÖRÜNÜR olmazsa hata (pencere gizli oluşturulur, ön yüz ilk çizimden sonra
    gösterir; MainWindowHandle kullanılmaz, bkz. PgtWindow açıklaması),
  - süreç -Seconds dolmadan çıkarsa ExitCode ile hata (ör. release'de açılıştan ~1 sn sonra
    STATUS_STACK_OVERFLOW = -1073741571 / 0xC00000FD),
  - sonunda uygulama + WebView2 alt süreç ağacının tamamını kapatır (ana süreç önceden ölmüş olsa bile:
    alt süreçler üst süreç kimliği + ad + başlangıç zamanıyla bulunur),
  - geçici %LOCALAPPDATA%\pg-tools\logs altında "panicked" / "stack overflow" geçiyorsa hata,
  - pencereyi ön yüz değil Rust'taki 3 sn'lik yedek zamanlayıcı gösterdiyse (logda "fallback-show") hata.
  Çıkış kodu 0 = başarılı, 1 = başarısız.
.PARAMETER Exe      Sınanacak exe (varsayılan: target\release\pg-tools.exe)
                    Ön yüzü gömülü exe gerekir: `bun run tauri build --no-bundle` ile derleyin. Düz
                    `cargo build --release` exe'si devUrl'ye (localhost:1420) yönelir; betik bunu reddeder.
.PARAMETER Seconds  Pencere oluştuktan sonra sürecin ayakta kalması gereken süre (varsayılan 6)
#>
param(
    [string]$Exe = (Join-Path (Split-Path -Parent $PSScriptRoot) "target\release\pg-tools.exe"),
    [int]$Seconds = 6
)
$ErrorActionPreference = "Stop"

# Ana süreç ölmüş olsa bile alt süreçleri ParentProcessId hâlâ onun kimliğini taşır. Kimlik yeniden
# kullanılmış olabileceğinden yalnızca beklenen adlı ve betik başladıktan sonra oluşmuş süreçler alınır.
$TreeNames = @([IO.Path]::GetFileName($Exe), "pg-tools.exe", "msedgewebview2.exe")
function Get-Tree([int]$rootPid, [datetime]$since) {
    $all = Get-CimInstance Win32_Process
    $ids = New-Object System.Collections.Generic.HashSet[int]
    [void]$ids.Add($rootPid)
    do {
        $added = $false
        foreach ($p in $all) {
            if ($ids.Contains([int]$p.ParentProcessId) -and $TreeNames -contains $p.Name -and
                $p.CreationDate -ge $since -and $ids.Add([int]$p.ProcessId)) { $added = $true }
        }
    } while ($added)
    , $ids
}

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

if (-not (Test-Path $Exe)) { Write-Output "BAŞARISIZ: exe bulunamadı: $Exe"; exit 1 }
# Gömülü ön yüz denetimi: tauri build exe'si varlık anahtarlarını ("assets/index-…") düz metin taşır; düz cargo
# build (custom-protocol yok) ve debug exe'si taşımaz ve devUrl'ye yönelir (arayüz komutları hiç çağrılmaz).
$exeText = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($Exe))
if ($exeText.IndexOf("assets/index-", [StringComparison]::Ordinal) -lt 0) {
    Write-Output "BAŞARISIZ: exe ön yüzü gömmüyor (cargo build?). 'bun run tauri build --no-bundle' ile derleyin: $Exe"; exit 1
}
Remove-Variable exeText

$root = Join-Path $env:TEMP ("pgt-smoke-" + [guid]::NewGuid().ToString("N"))
$appdata = Join-Path $root "Roaming"
$localappdata = Join-Path $root "Local"
$origAppData = $env:APPDATA
$origLocalAppData = $env:LOCALAPPDATA
$failure = $null
$proc = $null
$started = (Get-Date).AddSeconds(-1)   # CreationDate çözünürlüğü için küçük pay
try {
    try {
        New-Item -ItemType Directory $appdata, $localappdata | Out-Null
        $env:APPDATA = $appdata
        $env:LOCALAPPDATA = $localappdata
        $sw = [Diagnostics.Stopwatch]::StartNew()
        $proc = Start-Process $Exe -PassThru
        $null = $proc.Handle   # tutamacı önbelleğe al; yoksa süreç çıktıktan sonra ExitCode boş kalabilir
        # Alt süreç (WebView2) ortamı devraldı; kendi ortamımızı hemen geri yükleyebiliriz.
        $env:APPDATA = $origAppData
        $env:LOCALAPPDATA = $origLocalAppData

        while (-not [PgtWindow]::MainVisible($proc.Id)) {
            if ($proc.HasExited) { $failure = "pencere görünmeden çıktı (kod $($proc.ExitCode))"; break }
            if ($sw.Elapsed.TotalSeconds -gt 10) { $failure = "10 sn içinde ana pencere görünmedi"; break }
            Start-Sleep -Milliseconds 20
        }
        if (-not $failure) {
            $windowMs = $sw.ElapsedMilliseconds
            Write-Output "Pencere $windowMs ms'de göründü; $Seconds sn izleniyor..."
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
        if ($proc) {
            # Ana süreç çökmüş olsa da öksüz kalan WebView2 süreçleri kapatılmalı.
            $ids = Get-Tree $proc.Id $started
            # Kök kimlik yalnızca ana süreç hâlâ yaşıyorsa alınır (ölmüşse kimliği başkasına geçmiş olabilir).
            $live = @(Get-Process -Id ([int[]]@($ids)) -ErrorAction SilentlyContinue |
                Where-Object { $_.Id -ne $proc.Id -or -not $proc.HasExited })
            if ($live.Count -gt 0) {
                $live | Stop-Process -Force -ErrorAction SilentlyContinue
                Start-Sleep -Seconds 2   # WebView2 süreçlerinin dosya kilitlerini bırakması için
            }
        }
    }

    $logDir = Join-Path $localappdata "pg-tools\logs"
    if (Test-Path $logDir) {
        $hits = Get-ChildItem $logDir -File | Select-String -Pattern "panicked", "stack overflow", "fallback-show" -SimpleMatch
        if ($hits) {
            $msg = "logda panik/yığın taşması/yedek gösterim: " + (($hits | Select-Object -First 3 | ForEach-Object { $_.Line.Trim() }) -join " | ")
            $failure = if ($failure) { "$failure; $msg" } else { $msg }
        }
    } elseif (-not $failure) {
        Write-Output "Uyarı: log dizini oluşmadı ($logDir)"
    }
} finally {
    # Erken hata/kesmede de geçici dizin kalmasın.
    Remove-Item $root -Recurse -Force -ErrorAction SilentlyContinue
}

if ($failure) {
    Write-Output "BAŞARISIZ: $failure"
    exit 1
}
Write-Output "BAŞARILI: exe başladı, pencere açıldı, $Seconds sn sonra hâlâ çalışıyordu."
exit 0
