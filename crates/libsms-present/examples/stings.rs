//! Renders the cached stings (`assets/ror2_audio/*.wav`) that are missing: `cargo run -p libsms-present --example stings`.

fn main() {
    let dir = libsms::assets::find(None).expect("no assets/");
    libsms_present::audio::render_stings(&dir).expect("stings");
    for e in std::fs::read_dir(dir.join("ror2_audio")).unwrap().flatten() {
        println!("{:?} {} bytes", e.file_name(), e.metadata().unwrap().len());
    }
}
