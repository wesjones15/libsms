//! First-run asset preparation for non-Rust hosts. No game data ships with the library.

use std::ffi::{CStr, c_char};
use std::panic::catch_unwind;
use std::path::{Path, PathBuf};

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
}
