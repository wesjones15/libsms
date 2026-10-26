//! First-run asset preparation for non-Rust hosts. No game data ships with the library.

use std::ffi::{CStr, c_char};
use std::panic::catch_unwind;
use std::path::{Path, PathBuf};

/// Whether the user's derived portal animation is ready for gameplay.
/// # Safety
/// `dir` is null or a nul-terminated UTF-8 path.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_entrance_cache_complete(dir: *const c_char) -> u8 {
    catch_unwind(|| unsafe { path_arg(dir) }.is_some_and(|p| libsms_present::portal_cache::complete(&p)) as u8).unwrap_or(0)
}

/// Prepare the exact portal TEV frames on the asset worker. No graphics/audio device required.
/// # Safety
/// `dir` is null or a nul-terminated path; `msg` null or writable for `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_prepare_entrance(dir: *const c_char, msg: *mut u8, len: u32) -> u8 {
    let result = catch_unwind(|| {
        let dir = unsafe { path_arg(dir) }.ok_or("missing asset directory".to_string())?;
        libsms_present::portal_cache::prepare(&dir).map(|p| format!("cached portal animation: {}",p.display()))
    });
    match result {
        Ok(Ok(s)) => { unsafe { write_str(&s,msg,len) }; 1 },
        Ok(Err(s)) => { unsafe { write_str(&s,msg,len) }; 0 },
        Err(_) => { unsafe { write_str("portal cache preparation failed",msg,len) }; 0 },
    }
}

/// Whether `dir` (UTF-8) holds a complete extraction.
///
/// # Safety
/// `dir` must be null or a nul-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_assets_complete(dir: *const c_char) -> u8 {
    catch_unwind(|| {
        // SAFETY: as documented.
        unsafe { path_arg(dir) }.is_some_and(|p| libsms::extract::is_complete(&p)) as u8
    })
    .unwrap_or(0)
}

/// Find the user's Sunshine ISO/GCM/CISO via `$SMS_DISC`, then `;`-separated directories.
/// Writes a nul-terminated UTF-8 path; returns 0 if absent or the buffer is too small.
///
/// # Safety
/// `dirs` must be null or a nul-terminated string; `out` null or writable for `out_len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_find_disc(dirs: *const c_char, out: *mut u8, out_len: u32) -> u8 {
    // SAFETY: as documented.
    unsafe { write_str("", out, out_len) };
    let result = catch_unwind(|| {
        // SAFETY: as documented.
        let dirs = unsafe { path_arg(dirs) }?;
        let paths: Vec<_> = dirs
            .to_str()?
            .split(';')
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from)
            .collect();
        libsms::extract::find_disc(&paths)
    });
    match result {
        // SAFETY: as documented.
        Ok(Some(p)) => unsafe { write_str(&p.to_string_lossy(), out, out_len) },
        _ => 0,
    }
}

/// Extract the user's disc into `out_dir`. A complete cache succeeds without opening the disc.
/// Returns 1 on success; `msg` receives a short result or error in nul-terminated UTF-8.
///
/// # Safety
/// `disc` and `out_dir` must be null or nul-terminated strings; `msg` null or writable for
/// `msg_len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_extract(
    disc: *const c_char,
    out_dir: *const c_char,
    msg: *mut u8,
    msg_len: u32,
) -> u8 {
    let result = catch_unwind(|| -> Result<String, String> {
        // SAFETY: as documented.
        let out = unsafe { path_arg(out_dir) }.ok_or("missing asset directory")?;
        if libsms::extract::is_complete(&out) {
            return Ok(format!("assets already complete: {}", out.display()));
        }
        // SAFETY: as documented.
        let disc = unsafe { path_arg(disc) }.ok_or("missing disc path")?;
        let report =
            libsms::extract::extract_assets(&disc, &out, &mut |_| {}).map_err(|e| e.to_string())?;
        Ok(format!(
            "{}: {} tuning files, {} clips into {}",
            report.game_id,
            report.prm_files,
            report.clips,
            report.out.display()
        ))
    });
    let (ok, text) = match result {
        Ok(Ok(text)) => (1, text),
        Ok(Err(text)) => (0, text),
        Err(_) => (0, "the extractor panicked".into()),
    };
    // SAFETY: as documented.
    unsafe { write_str(&text, msg, msg_len) };
    ok
}

/// Fills in the optional pieces of an otherwise complete asset folder (the screen wipes' pictures)
/// from the user's disc: `msg` says what was written. Returns 1 if the folder has them all after.
///
/// # Safety
/// `disc` and `out_dir` must be null or nul-terminated strings; `msg` null or writable for
/// `msg_len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_extract_extras(
    disc: *const c_char,
    out_dir: *const c_char,
    msg: *mut u8,
    msg_len: u32,
) -> u8 {
    let result = catch_unwind(|| -> Result<String, String> {
        // SAFETY: as documented.
        let out = unsafe { path_arg(out_dir) }.ok_or("missing asset directory")?;
        if libsms::extract::has_extras(&out) {
            return Ok("wipe pictures already there".into());
        }
        // SAFETY: as documented.
        let disc = unsafe { path_arg(disc) }.ok_or("missing disc path")?;
        let mut notes = Vec::new();
        libsms::extract::extract_extras(&disc, &out, &mut |m| notes.push(m.to_owned()))
            .map_err(|e| e.to_string())?;
        if libsms::extract::has_extras(&out) {
            Ok(notes.join("; "))
        } else {
            Err(notes.join("; "))
        }
    });
    let (ok, text) = match result {
        Ok(Ok(text)) => (1, text),
        Ok(Err(text)) => (0, text),
        Err(_) => (0, "the extractor panicked".into()),
    };
    // SAFETY: as documented.
    unsafe { write_str(&text, msg, msg_len) };
    ok
}

/// Cache RoR2's arrival/death jingles in `dir/ror2_audio` from the extracted banks. No device
/// is opened. Existing complete files are reused. Returns 1 on success, with a UTF-8 message.
///
/// # Safety
/// `dir` must be null or a nul-terminated string; `msg` null or writable for `msg_len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_render_stings(dir: *const c_char, msg: *mut u8, msg_len: u32) -> u8 {
    let result = catch_unwind(|| -> Result<(), String> {
        // SAFETY: as documented.
        let dir = unsafe { path_arg(dir) }.ok_or("missing asset directory")?;
        libsms_present::audio::render_stings(&dir)
    });
    let (ok, text) = match result {
        Ok(Ok(())) => (1, "arrival/death/game over/blue coin stings cached".into()),
        Ok(Err(text)) => (0, text),
        Err(_) => (0, "sting rendering panicked".into()),
    };
    // SAFETY: as documented.
    unsafe { write_str(&text, msg, msg_len) };
    ok
}

/// # Safety
/// `p` must be null or a nul-terminated string.
unsafe fn path_arg(p: *const c_char) -> Option<PathBuf> {
    if p.is_null() {
        return None;
    }
    // SAFETY: as documented.
    let text = unsafe { CStr::from_ptr(p) }.to_str().ok()?;
    (!text.trim().is_empty()).then(|| Path::new(text).to_path_buf())
}

/// Write a possibly truncated UTF-8 string, always nul-terminated. Returns 1 if it all fit.
///
/// # Safety
/// `out` must be null or writable for `len` bytes.
unsafe fn write_str(text: &str, out: *mut u8, len: u32) -> u8 {
    if out.is_null() || len == 0 {
        return 0;
    }
    let mut n = text.len().min(len as usize - 1);
    while !text.is_char_boundary(n) {
        n -= 1;
    }
    // SAFETY: n + 1 <= len, as promised by the caller.
    unsafe {
        std::ptr::copy_nonoverlapping(text.as_ptr(), out, n);
        *out.add(n) = 0;
    }
    (n == text.len()) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn invalid_arguments_and_utf8_buffers() {
        let mut msg = [255; 64];
        unsafe {
            assert_eq!(sms_assets_complete(std::ptr::null()), 0);
            assert_eq!(sms_assets_complete(c"".as_ptr()), 0);
            assert_eq!(sms_find_disc(std::ptr::null(), msg.as_mut_ptr(), 64), 0);
            assert_eq!(msg[0], 0);
            assert_eq!(
                sms_extract(std::ptr::null(), std::ptr::null(), msg.as_mut_ptr(), 64),
                0
            );
            assert_eq!(
                CStr::from_bytes_until_nul(&msg).unwrap().to_str().unwrap(),
                "missing asset directory"
            );
            let mut short = [255; 4];
            assert_eq!(write_str("éé", short.as_mut_ptr(), 4), 0);
            assert_eq!(&short[..3], &[0xc3, 0xa9, 0]);
            assert_eq!(short[3], 255);
            assert_eq!(write_str("abc", std::ptr::null_mut(), 4), 0);
            assert_eq!(write_str("abc", short.as_mut_ptr(), 0), 0);
        }
    }

    #[test]
    fn incomplete_cache_reports_missing_disc() {
        let dir = CString::new("nonexistent-sms-assets-for-ffi-test").unwrap();
        let disc = CString::new("nonexistent-sunshine-for-ffi-test.iso").unwrap();
        let mut msg = [0; 512];
        unsafe {
            assert_eq!(sms_assets_complete(dir.as_ptr()), 0);
            assert_eq!(
                sms_extract(disc.as_ptr(), dir.as_ptr(), msg.as_mut_ptr(), 512),
                0
            );
        }
        assert!(
            CStr::from_bytes_until_nul(&msg)
                .unwrap()
                .to_str()
                .unwrap()
                .contains("I/O error")
        );
    }

    #[test]
    fn complete_cache_does_not_need_disc() {
        let Some(dir) = libsms::assets::find(None) else {
            return;
        };
        if !libsms::extract::is_complete(&dir) {
            return;
        }
        let dir = CString::new(dir.to_str().unwrap()).unwrap();
        let mut msg = [0; 512];
        unsafe {
            assert_eq!(sms_assets_complete(dir.as_ptr()), 1);
            assert_eq!(
                sms_extract(std::ptr::null(), dir.as_ptr(), msg.as_mut_ptr(), 512),
                1
            );
        }
        assert!(
            CStr::from_bytes_until_nul(&msg)
                .unwrap()
                .to_str()
                .unwrap()
                .contains("already complete")
        );
    }

    #[test]
    #[ignore = "requires SMS_DISC and extracts the user's disc into a temporary cache"]
    fn first_run_extracts_repairs_and_reuses_the_disc_cache() {
        // Exercise exactly the C exports used by the plugin, including a fresh folder and
        // repair of an old cache missing the newly required side-wave/coin effects.
        let disc = std::env::var("SMS_DISC").expect("set SMS_DISC to a Sunshine ISO/GCM/CISO");
        let disc_path = Path::new(&disc);
        let dirs = CString::new(disc_path.parent().unwrap().to_str().unwrap()).unwrap();
        let disc = CString::new(disc).unwrap();
        let cache = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(format!("first-run-ffi-{}", std::process::id()));
        assert!(!cache.exists());
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(cache.clone());
        let out = CString::new(cache.to_str().unwrap()).unwrap();
        let mut found = [0; 32768];
        let mut msg = [0; 4096];
        unsafe {
            assert_eq!(
                sms_find_disc(dirs.as_ptr(), found.as_mut_ptr(), found.len() as u32),
                1
            );
            assert_eq!(CStr::from_bytes_until_nul(&found).unwrap(), disc.as_c_str());
            let mut tiny = [255; 1];
            assert_eq!(sms_find_disc(dirs.as_ptr(), tiny.as_mut_ptr(), 1), 0);
            assert_eq!(tiny[0], 0);
            assert_eq!(sms_assets_complete(out.as_ptr()), 0);
            assert_eq!(
                sms_extract(disc.as_ptr(), out.as_ptr(), msg.as_mut_ptr(), 4096),
                1,
                "{}",
                CStr::from_bytes_until_nul(&msg).unwrap().to_string_lossy()
            );
            assert_eq!(sms_assets_complete(out.as_ptr()), 1);
            // No disc argument on subsequent runs.
            assert_eq!(
                sms_extract(std::ptr::null(), out.as_ptr(), msg.as_mut_ptr(), 4096),
                1
            );
            let side = cache.join("particle/ms_m_watslide_c.jpa");
            let spark = cache.join("particle/ms_coinget_b.jpa");
            std::fs::remove_file(&side).unwrap();
            std::fs::remove_file(&spark).unwrap();
            assert_eq!(sms_assets_complete(out.as_ptr()), 0);
            assert_eq!(
                sms_extract(disc.as_ptr(), out.as_ptr(), msg.as_mut_ptr(), 4096),
                1
            );
            assert!(side.is_file() && spark.is_file());
            assert_eq!(sms_assets_complete(out.as_ptr()), 1);
            let pos = crate::SmsVec3::default();
            let sim = crate::sms_sim_new(out.as_ptr(), &pos, 0);
            assert!(
                !sim.is_null(),
                "first-run tuning/animation must create a sim"
            );
            crate::sms_sim_free(sim);
            let coin = crate::view::sms_view_new_look(out.as_ptr(), 0, 2);
            assert!(
                !coin.is_null(),
                "first-run assets must contain the yellow coin"
            );
            crate::view::sms_view_free(coin);
            assert_eq!(
                sms_render_stings(out.as_ptr(), msg.as_mut_ptr(), 4096),
                1,
                "{}",
                CStr::from_bytes_until_nul(&msg).unwrap().to_string_lossy()
            );
            for name in ["spawn.wav", "death.wav", "gameover.wav", "bluecoin.wav", "shineget.wav", "secret.wav"] {
                let wav = std::fs::read(cache.join("ror2_audio").join(name)).unwrap();
                assert_eq!(&wav[..4], b"RIFF");
                assert_eq!(&wav[8..16], b"WAVEfmt ");
                assert_eq!(&wav[20..24], &[1, 0, 2, 0]);
                assert!(wav.len() > 32000 * 4);
            }
            // Cached jingles do not reload the source banks.
            std::fs::remove_dir_all(cache.join("AudioRes")).unwrap();
            assert_eq!(sms_render_stings(out.as_ptr(), msg.as_mut_ptr(), 4096), 1);
        }
    }
}
