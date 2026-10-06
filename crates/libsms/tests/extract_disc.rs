//! End to end: extract from a real disc image and load the result.
//!
//! Runs only when `SMS_ISO` points at a Super Mario Sunshine image (plain ISO or CISO); otherwise it
//! reports a skip. The game files are written to a temp folder and removed afterwards.

use std::path::{Path, PathBuf};

#[test]
fn extracts_a_real_disc_and_the_result_loads() {
    let Some(iso) = std::env::var_os("SMS_ISO")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
    else {
        eprintln!("skipped: set SMS_ISO to a Super Mario Sunshine disc image");
        return;
    };
    let out = std::env::temp_dir().join(format!("libsms_e2e_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);

    let mut log = Vec::new();
    let report = libsms::extract::extract_assets(&iso, &out, &mut |m| log.push(m.to_owned()))
        .unwrap_or_else(|e| panic!("extraction failed: {e}\n{log:#?}"));
    assert!(report.prm_files > 100, "{report:?}");
    assert!(report.clips > 150, "{report:?}");
    assert!(libsms::extract::is_complete(&out));

    // The extracted files parse and give the shipped tuning.
    let a = libsms::Assets::load(&out);
    assert!(a.prm_files > 10);
    assert_eq!(a.params.jump.gravity, 1.0);
    assert!(a.skeleton.is_some_and(|s| s.len() == 29));
    assert!(a.clips.get("wait").is_some());
    // Models, clip sound keys and the sound data came along.
    for f in [
        "bmd/ma_cap1.bmd",
        "bas/ma_jump.bas",
        "AudioRes/msound.aaf",
        "AudioRes/Banks/w1stLoad_0.aw",
    ] {
        assert!(out.join(f).is_file(), "{f}");
    }

    // A second run is a no-op.
    let mut again = Vec::new();
    libsms::extract::extract_assets(&iso, &out, &mut |m| again.push(m.to_owned())).unwrap();
    assert!(
        again.iter().any(|m| m.contains("already complete")),
        "{again:?}"
    );

    let _ = std::fs::remove_dir_all(&out);
    let _: &Path = &iso;
}
