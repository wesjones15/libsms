//! Builds the `assets/` folder from your own Super Mario Sunshine disc image.
//!
//!     cargo run -p libsms --bin sms-extract -- "path/to/Super Mario Sunshine.iso" [out-dir]
//!
//! Reads plain ISO/GCM and CISO images with no external tools. For RVZ/WIA/WBFS images use
//! `tools/extract_assets.py` (needs decomp-toolkit).

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(iso) = args.next() else {
        eprintln!("usage: sms-extract <disc image> [out-dir (default: assets)]");
        return ExitCode::from(2);
    };
    let out = args.next().unwrap_or_else(|| "assets".to_owned());
    match libsms::extract::extract_assets(Path::new(&iso), Path::new(&out), &mut |m| {
        println!("{m}")
    }) {
        Ok(r) => {
            println!(
                "wrote {} ({} .prm files, {} clips). This folder is git-ignored; do not share it.",
                r.out.display(),
                r.prm_files,
                r.clips
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
