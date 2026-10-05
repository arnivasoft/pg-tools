<#
.SYNOPSIS
  TLS zincir entegrasyon testini (crates/pgt-core/tests/tls.rs) Docker'daki TLS'li PostgreSQL'e karşı çalıştırır.
.DESCRIPTION
  Bir yardımcı kapsayıcıda openssl ile test CA'sı, bu CA'dan iki sunucu sertifikası (A: yalnızca DNS:pgt-tls.test;
  B: DNS:pgt-tls.test + IP:127.0.0.1) ve bağımsız ikinci bir CA üretir. Yalnızca hostssl kabul eden iki PostgreSQL
  açar (A: 127.0.0.1:55480, B: 127.0.0.1:55481) ve `cargo test -p pgt-core --test tls` çalıştırır. Parola her
  çalıştırmada rastgele üretilir ve yalnızca ortam değişkeninde yaşar. Kapsayıcılar ve geçici dizin (özel anahtarlar
  dahil) her durumda silinir.
.PARAMETER Version
  PostgreSQL ana sürümü (varsayılan: 16).
.PARAMETER KeepContainer
  Testten sonra sunucu kapsayıcılarını silme (hata ayıklama için). Kapsayıcılarda sunucu özel anahtarları ve
  parola (POSTGRES_PASSWORD) kalır; işiniz bitince docker rm -f ile silin. Geçici dizin (CA'lar, anahtarlar) yine silinir.
#>
param(
    [string]$Version = "16",
    [switch]$KeepContainer
)

$ErrorActionPreference = "Stop"
Set-Location (Split-Path -Parent $PSScriptRoot)

docker info *> $null
if ($LASTEXITCODE -ne 0) {
    throw "Docker çalışmıyor. Docker Desktop'ı başlatıp tekrar deneyin."
}

$image = "postgres:$Version"
$gen = "pgt-tls-gen"
$servers = @(
    @{ Name = "pgt-tls-a"; Port = 55480; Cert = "server-a" },
    @{ Name = "pgt-tls-b"; Port = 55481; Cert = "server-b" }
)
$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("pgt-tls-" + [guid]::NewGuid().ToString("N"))
$started = @()

# Sertifika üretimi (kapsayıcı içinde, /out). Tüm sertifikalar 2 gün geçerli; anahtarlar şifresiz.
$genScript = @'
set -e
mkdir -p /out && cd /out
openssl req -x509 -newkey rsa:2048 -nodes -days 2 -subj /CN=pgt-test-ca -keyout ca.key -out ca.pem 2>/dev/null
openssl req -x509 -newkey rsa:2048 -nodes -days 2 -subj /CN=pgt-other-ca -keyout other-ca.key -out other-ca.pem 2>/dev/null
sign() {
  openssl req -newkey rsa:2048 -nodes -subj /CN=pgt-tls.test -keyout "$1.key" -out "$1.csr" 2>/dev/null
  printf 'subjectAltName=%s\nbasicConstraints=CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n' "$2" > "$1.ext"
  openssl x509 -req -in "$1.csr" -CA ca.pem -CAkey ca.key -CAcreateserial -days 2 -extfile "$1.ext" -out "$1.crt" 2>/dev/null
}
sign server-a DNS:pgt-tls.test
sign server-b DNS:pgt-tls.test,IP:127.0.0.1
printf 'local all all trust\nhostssl all all all scram-sha-256\n' > pg_hba.conf
'@

# Sunucu kapsayıcısının girişi: sertifikaları postgres sahipliğine al, anahtarı 600 yap, resmi girişi TLS ile başlat.
$serverCmd = "mkdir -p /etc/pgt-tls && cp /tls-src/* /etc/pgt-tls/ && chown -R postgres:postgres /etc/pgt-tls && chmod 600 /etc/pgt-tls/server.key && exec docker-entrypoint.sh postgres -c ssl=on -c ssl_cert_file=/etc/pgt-tls/server.crt -c ssl_key_file=/etc/pgt-tls/server.key -c hba_file=/etc/pgt-tls/pg_hba.conf -c password_encryption=scram-sha-256"

try {
    New-Item -ItemType Directory -Path $tmp | Out-Null

    docker rm -f $gen *> $null
    docker run -d --name $gen --entrypoint sleep $image infinity | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "$image başlatılamadı" }
    # Betik dosya olarak kopyalanır (PowerShell 5.1 yerel komut argümanlarındaki tırnak/satır sonu sorunlarından kaçınmak için).
    $genFile = Join-Path $tmp "gen.sh"
    [System.IO.File]::WriteAllText($genFile, ($genScript -replace "`r", ""))
    docker cp $genFile "${gen}:/gen.sh" | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "sertifika betiği kopyalanamadı" }
    docker exec $gen sh /gen.sh
    if ($LASTEXITCODE -ne 0) { throw "sertifikalar üretilemedi" }
    docker cp "${gen}:/out/." $tmp | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "sertifikalar kopyalanamadı" }
    docker rm -f $gen *> $null

    # Rastgele parola; yalnızca bu sürecin ortamında (docker -e ADI biçimi değeri komut satırına koymaz).
    $env:POSTGRES_PASSWORD = [guid]::NewGuid().ToString("N")
    $env:POSTGRES_INITDB_ARGS = "--auth-host=scram-sha-256"

    foreach ($s in $servers) {
        $src = Join-Path $tmp $s.Name
        New-Item -ItemType Directory -Path $src | Out-Null
        Copy-Item (Join-Path $tmp "$($s.Cert).crt") (Join-Path $src "server.crt")
        Copy-Item (Join-Path $tmp "$($s.Cert).key") (Join-Path $src "server.key")
        Copy-Item (Join-Path $tmp "pg_hba.conf") (Join-Path $src "pg_hba.conf")

        docker rm -f $s.Name *> $null
        docker create --name $s.Name -e POSTGRES_PASSWORD -e POSTGRES_INITDB_ARGS `
            -p "127.0.0.1:$($s.Port):5432" --entrypoint sh $image -c $serverCmd | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "$($s.Name) oluşturulamadı" }
        $started += $s.Name
        docker cp "$src\." "$($s.Name):/tls-src" | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "$($s.Name) için sertifikalar kopyalanamadı" }
        docker start $s.Name | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "$($s.Name) başlatılamadı" }
    }

    # Kapsayıcının son günlük satırları (PostgreSQL günlüğü; parola içermez). Yerel komutun stderr'i
    # PS 5.1'de ErrorActionPreference=Stop iken hata sayılmasın diye geçici olarak Continue.
    function Get-LogTail([string]$name) {
        $old = $ErrorActionPreference
        $ErrorActionPreference = "Continue"
        try { ((docker logs --tail 20 $name 2>&1) | ForEach-Object { "$_" }) -join "`n" } finally { $ErrorActionPreference = $old }
    }
    function Test-Running([string]$name) {
        $old = $ErrorActionPreference
        $ErrorActionPreference = "Continue"
        try { "$(docker inspect -f '{{.State.Running}}' $name 2>$null)".Trim() -eq "true" } finally { $ErrorActionPreference = $old }
    }

    foreach ($s in $servers) {
        $ready = $false
        for ($i = 0; $i -lt 90; $i++) {
            # Erken çöken kapsayıcı için 90 sn bekleme: çalışmıyorsa hemen günlükle birlikte dur.
            if (-not (Test-Running $s.Name)) {
                throw "$($s.Name) çalışmıyor (erken çıktı). Son günlük satırları:`n$(Get-LogTail $s.Name)"
            }
            # 127.0.0.1 üzerinden: ilk başlatmadaki geçici (yalnızca soket) sunucu değil, kalıcı sunucu beklenir.
            docker exec $s.Name pg_isready -U postgres -h 127.0.0.1 *> $null
            if ($LASTEXITCODE -eq 0) { $ready = $true; break }
            Start-Sleep -Seconds 1
        }
        if (-not $ready) {
            throw "$($s.Name) hazır olmadı. Son günlük satırları:`n$(Get-LogTail $s.Name)"
        }
    }

    $env:PGT_TLS_PASSWORD = $env:POSTGRES_PASSWORD
    $env:PGT_TLS_URL = "host=127.0.0.1 port=$($servers[0].Port) user=postgres"
    $env:PGT_TLS_IP_URL = "host=127.0.0.1 port=$($servers[1].Port) user=postgres"
    $env:PGT_TLS_CA = Join-Path $tmp "ca.pem"
    $env:PGT_TLS_OTHER_CA = Join-Path $tmp "other-ca.pem"

    cargo test -p pgt-core --test tls -- --nocapture
    if ($LASTEXITCODE -ne 0) { throw "TLS testleri başarısız" }
}
finally {
    foreach ($v in "POSTGRES_PASSWORD", "POSTGRES_INITDB_ARGS", "PGT_TLS_PASSWORD", "PGT_TLS_URL", "PGT_TLS_IP_URL", "PGT_TLS_CA", "PGT_TLS_OTHER_CA") {
        Remove-Item "Env:$v" -ErrorAction SilentlyContinue
    }
    docker rm -f $gen *> $null
    if (-not $KeepContainer) {
        foreach ($name in $started) { docker rm -f $name *> $null }
    }
    if (Test-Path $tmp) { Remove-Item -Recurse -Force $tmp }
}
