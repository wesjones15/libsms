//! Lists files inside the disc's archives whose paths contain any of the given words.
//!
//!     cargo run -p libsms-assets --example scan -- <disc image> mon ma_mdl

use libsms::extract::{Disc, rarc, yaz0};

fn main() {
    let mut args = std::env::args().skip(1);
    let iso = args.next().expect("disc image");
    let words: Vec<String> = args.map(|w| w.to_lowercase()).collect();
    let mut disc = Disc::open(std::path::Path::new(&iso)).expect("open");
    let mut archives = disc.paths_under("data/");
    archives.retain(|p| p.ends_with(".szs") || p.ends_with(".arc"));
    for a in archives {
        let raw = disc.read_file(&a).expect("read");
        let bytes = if yaz0::is_yaz0(&raw) {
            yaz0::decompress(&raw).expect("yaz0")
        } else {
            raw
        };
        let Ok(arc) = rarc::Archive::parse(bytes) else {
            continue;
        };
        for p in arc.paths() {
            let l = p.to_lowercase();
            if words.iter().any(|w| l.contains(w.as_str())) {
                println!("{a}:{p} {}", arc.file(p).map_or(0, |f| f.len()));
            }
        }
    }
}
