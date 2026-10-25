//! Inspect the exact stage files used by the statue entrance, without extracting unrelated assets.
use libsms::extract::{Disc, rarc::Archive, yaz0};
fn main() {
    let disc = std::env::args().nth(1).expect("portal_assets <disc.iso>");
    let mut disc = Disc::open(std::path::Path::new(&disc)).unwrap();
    for file in ["data/scene/dolpic1.szs", "data/particle.szs"] {
        let raw = disc.read_file(file).unwrap();
        let raw = if yaz0::is_yaz0(&raw) {
            yaz0::decompress(&raw).unwrap()
        } else {
            raw
        };
        let arc = Archive::parse(raw).unwrap();
        for (name, _) in arc.under("") {
            let lower = name.to_ascii_lowercase();
            if [
                "05_gate01",
                "gatewind",
                "scene.bin",
                "palmnormal.bmd",
                "palmleaf.bmd",
                "manhole.bmd",
                "monumentshine.bmd",
                "efdokangate.bmd",
                "map/map.bmd",
            ]
            .iter()
            .any(|s| lower.contains(s))
            {
                println!("{file}:{name}");
            }
        }
    }
}
