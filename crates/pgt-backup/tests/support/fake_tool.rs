//! Yürütücü testleri için sahte istemci aracı (gerçek PostgreSQL gerekmez). Yalnız `pgt-backup` testlerinde
//! kullanılır; Tauri uygulaması bağımlılıkların ikililerini derlemediği için dağıtıma girmez.
//!
//! Argümanlar (sırasıyla işlenir):
//! - `--print-env`: adı `PG`/`LC_` ile başlayan ya da `LANGUAGE` olan ortam değişkenlerini stderr'e
//!   `ENV ad=değer` olarak yazar.
//! - `--format=directory` / `--file <yol>`: pg_dump gibi çıktıyı oluşturur (dosya ya da klasör + `toc.dat`).
//!   Çıktı bağlantıdan önce oluşur (gerçek pg_dump plain gibi).
//! - `--line <metin>`: stderr'e bir satır yazar.
//! - `--tables <n>`: `pg_dump: dumping contents of table "public.tK"` satırları yazar.
//! - `--long <n>`: `n` adet `ğ` içeren tek satır yazar.
//! - `--crlf <metin>`: satırı `\r\n` ile bitirir.
//! - `--sleep-ms <n>`: bekler.
//! - `--mkdir <yol>`: içinde bir dosya olan klasör oluşturur (hedefe taşımayı bozmak için).
//! - `--exit <kod>`: o kodla çıkar (varsayılan 0).

use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut dir = false;
    let mut code = 0;
    let stderr = std::io::stderr();
    let mut err = stderr.lock();
    let mut i = 0;
    let next = |i: &mut usize| {
        *i += 1;
        args[*i].clone()
    };
    while i < args.len() {
        match args[i].as_str() {
            "--print-env" => {
                for (k, v) in std::env::vars_os() {
                    let (k, v) = (k.to_string_lossy().into_owned(), v.to_string_lossy().into_owned());
                    let up = k.to_uppercase();
                    if up.starts_with("PG") || up.starts_with("LC_") || up == "LANGUAGE" {
                        writeln!(err, "ENV {k}={v}").unwrap();
                    }
                }
            }
            "--format=directory" => dir = true,
            "--file" => {
                let p = next(&mut i);
                if dir {
                    std::fs::create_dir(&p).expect("klasör oluşturulamadı");
                    std::fs::write(std::path::Path::new(&p).join("toc.dat"), vec![b'T'; 300]).unwrap();
                    std::fs::write(std::path::Path::new(&p).join("1.dat"), vec![b'D'; 200]).unwrap();
                } else {
                    std::fs::write(&p, vec![b'F'; 1000]).expect("dosya yazılamadı");
                }
            }
            "--line" => writeln!(err, "{}", next(&mut i)).unwrap(),
            "--crlf" => write!(err, "{}\r\n", next(&mut i)).unwrap(),
            "--tables" => {
                let n: u32 = next(&mut i).parse().unwrap();
                for k in 0..n {
                    writeln!(err, "pg_dump: dumping contents of table \"public.t{k}\"").unwrap();
                }
            }
            "--long" => {
                let n: usize = next(&mut i).parse().unwrap();
                writeln!(err, "{}", "ğ".repeat(n)).unwrap();
            }
            "--sleep-ms" => {
                err.flush().unwrap();
                let ms: u64 = next(&mut i).parse().unwrap();
                std::thread::sleep(std::time::Duration::from_millis(ms));
            }
            "--mkdir" => {
                let p = next(&mut i);
                std::fs::create_dir_all(&p).unwrap();
                std::fs::write(std::path::Path::new(&p).join("x"), b"x").unwrap();
            }
            "--exit" => code = next(&mut i).parse().unwrap(),
            other => panic!("bilinmeyen argüman: {other}"),
        }
        i += 1;
    }
    err.flush().unwrap();
    std::process::exit(code);
}
