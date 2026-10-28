//! `cargo run -p libsms --example p2_seq_list -- <Seq.arc>`: lists the sequences in Pikmin 2's `Seqs/Seq.arc` (a RARC).
fn main() {
    let path = std::env::args().nth(1).expect("Seq.arc");
    let bytes = std::fs::read(path).expect("read");
    let arc = libsms::extract::rarc::Archive::parse(bytes).expect("rarc");
    let paths: Vec<String> = arc.paths().map(str::to_owned).collect();
    for p in &paths {
        println!("{p} {}", arc.file(p).map_or(0, |d| d.len()));
    }
}
