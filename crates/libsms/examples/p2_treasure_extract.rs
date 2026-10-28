//! `cargo run -p libsms --example p2_treasure_extract -- <Pikmin 2 image> <cache folder>`: Pikmin 2's
//! treasures (models, configs, names, particle containers) into `<cache folder>/treasure/`.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 3 {
        eprintln!("usage: p2_treasure_extract <Pikmin 2 .iso/.ciso> <cache folder>");
        std::process::exit(2);
    }
    match libsms::extract::p2_treasure::extract(std::path::Path::new(&a[1]), std::path::Path::new(&a[2]), &mut |m| println!("{m}")) {
        Ok(n) => println!("{n} files"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
