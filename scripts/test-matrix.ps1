<#
.SYNOPSIS
  PostgreSQL sürüm matrisinde pg-tools entegrasyon testlerini çalıştırır.
.PARAMETER Versions
  Test edilecek ana sürümler (varsayılan: 10–18 arası 9 sürüm: 10, 11, 12, 13, 14, 15, 16, 17, 18).
.PARAMETER KeepContainers
  Testten sonra kapsayıcıları silme (hata ayıklama için).
.PARAMETER SkipPerf
  5.000 tablolu performans testini (five_thousand_tables_stay_within_budget) atla; matris süresinin
  çoğunu bu test oluşturur. Varsayılan: çalıştırılır.
#>
param(
    [string[]]$Versions = @("10", "11", "12", "13", "14", "15", "16", "17", "18"),
    [switch]$KeepContainers,
    [switch]$SkipPerf
)

$ErrorActionPreference = "Stop"
# `powershell -File ... -Versions 13,15` diziyi tek "13,15" dizesi olarak verir; virgüllerden böl.
$Versions = @($Versions | ForEach-Object { $_ -split "," } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
Set-Location (Split-Path -Parent $PSScriptRoot)

docker info *> $null
if ($LASTEXITCODE -ne 0) {
    throw "Docker çalışmıyor. Docker Desktop'ı başlatıp tekrar deneyin."
}

# Geçici, her çalıştırmada rastgele parola; hiçbir yere yazılmaz.
# `max_connections=300`: test ikilileri çok çekirdekte tüm testleri paralel koşar (her test birkaç bağlantı açar);
# varsayılan 100 sınırı aşılınca alakasız testler 53300 ile düşer.
$password = [guid]::NewGuid().ToString("N")
$urls = @()
$names = @()

try {
    foreach ($v in $Versions) {
        $name = "pgt-matrix-$v"
        $port = 55400 + [int]$v
        docker rm -f $name *> $null
        docker run -d --name $name `
            -e POSTGRES_PASSWORD=$password `
            -e POSTGRES_INITDB_ARGS="--auth-host=scram-sha-256" `
            -e POSTGRES_HOST_AUTH_METHOD=scram-sha-256 `
            -p "127.0.0.1:${port}:5432" "postgres:$v" -c max_connections=300 | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "postgres:$v başlatılamadı" }
        $names += $name
        $urls += "postgres://postgres:$password@127.0.0.1:$port/postgres"
    }

    foreach ($name in $names) {
        $ready = $false
        for ($i = 0; $i -lt 90; $i++) {
            docker exec $name pg_isready -U postgres -h 127.0.0.1 *> $null
            if ($LASTEXITCODE -eq 0) { $ready = $true; break }
            Start-Sleep -Seconds 1
        }
        if (-not $ready) { throw "$name hazır olmadı" }
    }
    # Resmi imaj ilk başlatmada geçici bir sunucu açıp kapatır; kalıcı sunucunun ayağa kalkması için kısa bekleme.
    Start-Sleep -Seconds 3

    $env:PGT_TEST_URLS = ($urls -join ",")
    if ($SkipPerf) {
        cargo test --workspace --all-features -- --skip five_thousand_tables_stay_within_budget
    } else {
        cargo test --workspace --all-features
    }
    if ($LASTEXITCODE -ne 0) { throw "testler başarısız" }
}
finally {
    Remove-Item Env:PGT_TEST_URLS -ErrorAction SilentlyContinue
    if (-not $KeepContainers) {
        foreach ($name in $names) { docker rm -f $name *> $null }
    }
}
