//! Lists disc archives holding files whose path contains a word, or fills in a folder's
//! optional pieces: `cargo run -p libsms --example discfind -- <iso> <word>` and
//! `... -- <iso> --extras <assets folder>`.

use libsms::extract::{gcm::Disc, rarc, yaz0};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.get(1).map(String::as_str) == Some("--extras") {
        let n = libsms::extract::extract_extras(
            std::path::Path::new(&a[0]),
            std::path::Path::new(&a[2]),
            &mut |m| println!("{m}"),
        )
        .unwrap();
        println!("{n} written");
        return;
    }
    let mut disc = Disc::open(std::path::Path::new(&a[0])).unwrap();
    let word = a[1].to_lowercase();
    for p in disc.paths_under("data/") {
        if p.to_lowercase().contains(&word) {
            println!("disc: {p}");
        }
        if !p.ends_with(".szs") || p.contains("/scene/") && !p.contains("common") {
            continue;
        }
        let Ok(raw) = disc.read_file(&p) else { continue };
        let Ok(bytes) = (if yaz0::is_yaz0(&raw) { yaz0::decompress(&raw) } else { Ok(raw) }) else {
            continue;
        };
        let Ok(arc) = rarc::Archive::parse(bytes) else { continue };
        for f in arc.paths() {
            if f.to_lowercase().contains(&word) {
                println!("{p}: {f}");
            }
        }
    }
}
