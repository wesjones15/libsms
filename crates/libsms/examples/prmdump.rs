//! Prints a `.prm` file's keys and values (as f32, and as the integer its size suggests):
//! `cargo run -p libsms --example prmdump -- assets/params_enemy/namekuri.prm [...]`.

fn main() {
    for path in std::env::args().skip(1) {
        let data = std::fs::read(&path).expect("read");
        let prm = libsms::prm::Prm::parse(&data).expect("parse");
        println!("{path}");
        let mut keys: Vec<_> = prm.entries().collect();
        keys.sort_by_key(|e| e.0);
        for (k, v) in keys {
            let shown = match v.len() {
                1 => format!("{}", v[0]),
                2 => format!("{}", i16::from_be_bytes([v[0], v[1]])),
                4 => {
                    let b = [v[0], v[1], v[2], v[3]];
                    format!("{} (i32 {})", f32::from_be_bytes(b), i32::from_be_bytes(b))
                }
                n => format!("<{n} bytes>"),
            };
            println!("  {k} = {shown}");
        }
    }
}
