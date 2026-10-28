//! `cargo run -p libsms --example p2_extract -- <Pikmin 2 image> <out folder>`: Pikmin 2's player
//! models and motions (Louie, Olimar's alt-skin in RoR2); with a third argument `audio` its sound files too.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 3 {
        eprintln!("usage: p2_extract <Pikmin 2 .iso/.ciso> <out folder>");
        std::process::exit(2);
    }
    if a.get(3).is_some_and(|s| s == "audio") {
        match libsms::extract::p2::extract_audio(std::path::Path::new(&a[1]), std::path::Path::new(&a[2]), &mut |m| println!("{m}")) {
            Ok(n) => println!("{n} files"),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        return;
    }
    match libsms::extract::p2::extract_navi(std::path::Path::new(&a[1]), std::path::Path::new(&a[2]), &mut |m| println!("{m}")) {
        Ok(n) => println!("{n} files"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
