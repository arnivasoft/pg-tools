# pg-tools

pgAdmin4'e hafif, şirket içi alternatif: Tauri 2 + SolidJS + Rust.

## Geliştirme

    bun install
    bun tauri dev

## Testler

    cargo test --workspace --all-features   # PGT_TEST_URLS tanımlı değilse entegrasyon testleri atlanır
    bun run test
    bun run typecheck

`PGT_TEST_URLS`: virgülle ayrılmış `postgres://kullanici:parola@host:port/postgres` adresleri. Test kullanıcısı CREATEDB ve CREATEROLE yetkisine sahip olmalı.

Belgeler: `docs/superpowers/specs/`, `docs/superpowers/plans/`, `docs/smoke-test.md`.
