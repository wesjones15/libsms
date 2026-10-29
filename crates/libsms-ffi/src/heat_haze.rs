//! Additive, camera-independent access to TShimmer's disc assets.
use crate::view::SmsTexture;
use libsms_present::heat_haze::HeatHaze;
use std::ffi::{CStr, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};

/// # Safety
/// `dir` is null or a NUL-terminated UTF-8 asset path. Free with sms_heat_free.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_heat_new(dir: *const c_char) -> *mut HeatHaze {
    catch_unwind(|| {
        if dir.is_null() {
            return std::ptr::null_mut();
        }
        let Ok(dir) = (unsafe { CStr::from_ptr(dir) }).to_str() else {
            return std::ptr::null_mut();
        };
        HeatHaze::load(std::path::Path::new(dir))
            .ok()
            .map_or(std::ptr::null_mut(), |h| Box::into_raw(Box::new(h)))
    })
    .unwrap_or(std::ptr::null_mut())
}

/// # Safety
/// `heat` is null or a live handle, freed exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_heat_free(heat: *mut HeatHaze) {
    if !heat.is_null() {
        drop(unsafe { Box::from_raw(heat) })
    }
}

/// Packed AG normal map (asset-derived gain, compensated by host magnitude). No partial writes.
/// # Safety
/// `heat` is live; info is writable; rgba is null or writable for capacity bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_heat_texture(
    heat: *const HeatHaze,
    info: *mut SmsTexture,
    rgba: *mut u8,
    capacity: u32,
) -> u8 {
    catch_unwind(AssertUnwindSafe(|| {
        let (Some(h), Some(info)) = (unsafe { heat.as_ref() }, unsafe { info.as_mut() }) else {
            return 0;
        };
        *info = SmsTexture {
            width: h.texture.image.width,
            height: h.texture.image.height,
            wrap_s: 0,
            wrap_t: 0,
            _pad: [0; 2],
        };
        if !rgba.is_null() {
            if (capacity as usize) < h.packed.len() {
                return 0;
            }
            unsafe { std::ptr::copy_nonoverlapping(h.packed.as_ptr(), rgba, h.packed.len()) };
        }
        1
    }))
    .unwrap_or(0)
}

/// Eight floats: camera depth (sim units), gain, then UV transform rows (3 each).
/// # Safety
/// `heat` is live and exclusively borrowed; values is writable for capacity floats.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_heat_frame(
    heat: *mut HeatHaze,
    seconds: f32,
    values: *mut f32,
    capacity: u32,
) -> u8 {
    catch_unwind(AssertUnwindSafe(|| {
        if values.is_null() || capacity < 8 || !seconds.is_finite() {
            return 0;
        }
        let Some(h) = (unsafe { heat.as_mut() }) else {
            return 0;
        };
        let rows = h.uv_rows(seconds);
        let result = [
            h.depth,
            h.normal_gain,
            rows[0][0],
            rows[0][1],
            rows[0][2],
            rows[1][0],
            rows[1][1],
            rows[1][2],
        ];
        unsafe { std::ptr::copy_nonoverlapping(result.as_ptr(), values, 8) };
        1
    }))
    .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn heat_haze_crosses_the_boundary_without_partial_buffer_writes() {
        unsafe {
            assert!(sms_heat_new(std::ptr::null()).is_null());
            let Some(dir) = libsms::assets::find(None) else {
                eprintln!("SKIP: Sunshine assets absent");
                return;
            };
            if !libsms_present::heat_haze::FILES
                .iter()
                .all(|f| dir.join(f).is_file())
            {
                eprintln!("SKIP: Shimmer absent");
                return;
            }
            let dir = std::ffi::CString::new(dir.to_string_lossy().as_bytes()).unwrap();
            let h = sms_heat_new(dir.as_ptr());
            assert!(!h.is_null());
            let mut info = SmsTexture::default();
            let mut short = [0xAB; 4];
            assert_eq!(sms_heat_texture(h, &mut info, short.as_mut_ptr(), 4), 0);
            assert_eq!(short, [0xAB; 4]);
            let mut pixels = vec![0; 64 * 64 * 4];
            assert_eq!(
                sms_heat_texture(h, &mut info, pixels.as_mut_ptr(), pixels.len() as u32),
                1
            );
            let mut frame = [123.; 8];
            assert_eq!(sms_heat_frame(h, 0., frame.as_mut_ptr(), 7), 0);
            assert_eq!(frame, [123.; 8]);
            assert_eq!(sms_heat_frame(h, 0., frame.as_mut_ptr(), 8), 1);
            assert_eq!(frame[0], 2425.);
            assert!(frame[1].is_finite() && frame[1] > 1.0);
            assert_eq!(sms_heat_frame(h, f32::NAN, frame.as_mut_ptr(), 8), 0);
            sms_heat_free(h);
        }
    }
}
