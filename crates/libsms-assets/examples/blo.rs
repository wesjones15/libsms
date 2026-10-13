//! Lists a J2D screen's panes: `cargo run -p libsms-assets --example blo -- assets/game_6/scrn/standard_1.blo`.

use libsms_assets::blo::Screen;

fn main() {
    let path = std::env::args().nth(1).expect("usage: blo <file.blo>");
    let d = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let s = Screen::parse(&d).unwrap_or_else(|e| panic!("{path}: {e}"));
    println!("screen {}x{}, {} panes", s.width, s.height, s.panes.len());
    for (i, p) in s.panes.iter().enumerate() {
        let tag: String = p
            .tag
            .to_be_bytes()
            .iter()
            .map(|&b| if b.is_ascii_graphic() { b as char } else { '.' })
            .collect();
        println!(
            "{i:3} {} tag {tag:4} (0x{:08X}) parent {:?} vis {} rect {:?} at {:?} a {} tex {:?} black {:08X} white {:08X}",
            String::from_utf8_lossy(&p.kind),
            p.tag,
            p.parent,
            p.visible,
            p.rect,
            p.at,
            p.alpha,
            p.texture,
            p.black,
            p.white
        );
    }
}
