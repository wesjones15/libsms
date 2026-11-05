//! Lists files in every disc archive, stage archives included, whose path contains any of the
//! given words, once per distinct path (with the first archive holding it and its size):
//! `cargo run --release -p libsms --example scenefind -- <iso> <word> [word...]`.
//! `--all` prints every archive holding each path.

use std::collections::BTreeMap;

use libsms::extract::{gcm::Disc, rarc, yaz0};

fn main() {
    let mut a: Vec<String> = std::env::args().skip(1).collect();
    let all = a.iter().any(|w| w == "--all");
    a.retain(|w| w != "--all");
    let disc = Disc::open(std::path::Path::new(&a[0])).unwrap();
    let words: Vec<String> = a[1..].iter().map(|w| w.to_lowercase()).collect();
    let hit = |s: &str| {
        let s = s.to_lowercase();
        words.iter().any(|w| s.contains(w.as_str()))
    };
    let mut seen: BTreeMap<String, (String, usize, Vec<String>)> = BTreeMap::new();
    for p in disc.paths_under("data/") {
        if hit(&p) {
            println!("disc: {p}");
        }
        if !p.ends_with(".szs") && !p.ends_with(".arc") {
            continue;
        }
        let Ok(raw) = disc.read_file(&p) else { continue };
        let Ok(bytes) = (if yaz0::is_yaz0(&raw) { yaz0::decompress(&raw) } else { Ok(raw) }) else {
            continue;
        };
        let Ok(arc) = rarc::Archive::parse(bytes) else { continue };
        for f in arc.paths() {
            if hit(f) {
                let size = arc.file(f).map(|d| d.len()).unwrap_or(0);
                seen.entry(f.to_owned())
                    .and_modify(|e| e.2.push(p.clone()))
                    .or_insert((p.clone(), size, Vec::new()));
            }
        }
    }
    for (f, (arc, size, more)) in seen {
        if all && !more.is_empty() {
            println!("{f}\t{size}\t{arc} +{}", more.join(" "));
        } else {
            println!("{f}\t{size}\t{arc}{}", if more.is_empty() { String::new() } else { format!(" (+{})", more.len()) });
        }
    }
}
