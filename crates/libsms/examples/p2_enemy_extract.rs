//! `cargo run -p libsms --example p2_enemy_extract -- <Pikmin 2 image> <cache folder>`: Pikmin 2's
//! enemies (models, clips, textures, parameter text) into `<cache folder>/enemy/`.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 3 {
        eprintln!("usage: p2_enemy_extract <Pikmin 2 .iso/.ciso> <cache folder>");
        std::process::exit(2);
    }
    match libsms::extract::p2_enemy::extract(std::path::Path::new(&a[1]), std::path::Path::new(&a[2]), &mut |m| println!("{m}")) {
        Ok(n) => println!("{n} files"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
