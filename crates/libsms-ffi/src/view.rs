//! Drawing and sound for hosts with their own engine: `libsms-present`'s [`View`] (the disc's
//! Mario, posed and skinned every frame) and [`Audio`] (the game's sounds for the sim) behind a
//! C ABI.
//!
//! The model structs match libpikmin's `pik_view_*` field for field, so one host-side model
//! uploader serves both: models are loaded once, and a host builds a mesh per model from
//! [`sms_view_model`], [`sms_view_batches`] (one submesh each), [`sms_view_indices`] and
//! [`sms_view_uvs`]. Each frame [`sms_view_update`] poses the sim; [`sms_view_draws`] lists
//! the instances (Mario's parts, all in his model space at his feet), each with its vertices
//! in [`sms_view_positions`] / [`sms_view_normals`] (sim units) and per batch a colour in
//! [`sms_view_colors`] and a texture in [`sms_view_texture_ids`]. Textures are each material's
//! TEV baked into one RGBA image ([`sms_view_texture`]); new ones (blink frames) can appear in
//! any frame. Sound plays for the ticks a sim runs with this view passed to `sms_sim_step`.
//!
//! FLUDD's water is separate geometry in world space: [`sms_view_water_vertices`] (quads, the
//! drops, splashes, then ground wetness, counted in [`sms_view_water_info`]) over the two textures from
//! [`sms_view_water_texture`].
//!
//! So are the game's particle effects on Mario (`libsms_present::particles`: movement dust,
//! stars, smears, slide spray, stomp bursts and pound flames): [`sms_view_fx_batches`] are
//! runs of quads in [`sms_view_fx_vertices`], each
//! over one of the textures counted in [`sms_view_fx_info`] ([`sms_view_fx_texture`]; a
//! texture whose [`sms_view_fx_texture_version`] changed has new pixels).
//!
//! FLUDD's water gauge (`libsms_present::hud`, the HUD's tank) is one RGBA image
//! ([`sms_view_hud_pixels`]) placed on the game's 600x480 screen as [`sms_view_hud_info`] says;
//! its `version` changes when the picture does.

use std::ffi::{CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;

use glam::{Mat4, Vec3, Vec4};
use libsms_present::particles::{FxBatch, FxVertex};
use libsms_present::water::WaterVertex;
use libsms_present::{Audio, View, WaterLook};

use crate::{SmsSim, SmsVec3};

/// One model's sizes.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SmsModelInfo {
    pub vertex_count: u32,
    pub index_count: u32,
    pub batch_count: u32,
    /// Textures baked so far, every model's ([`sms_view_texture`] ids are below this).
    pub texture_count: u32,
    /// Bind-pose bounds.
    pub lo: SmsVec3,
    pub hi: SmsVec3,
}

/// One batch (a submesh with one material).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SmsBatch {
    pub first_index: u32,
    pub index_count: u32,
    pub first_vertex: u32,
    pub vertex_count: u32,
    /// Its texture with no per-draw changes ([`sms_view_texture`]), or -1. A draw's own is in
    /// [`sms_view_texture_ids`].
    pub texture: i32,
    /// `libsms_present::view::blend`: 0 opaque, 1 cutout, 2 translucent, 3 additive.
    pub blend: u8,
    /// Lit in the game: 1 its colour, 2 its alpha. 0: unlit. Plus 4: the batch fades by its
    /// vertices' alpha ([`sms_view_vertex_alphas`]); its texture carries none of it.
    pub lit: u8,
    /// 0 back, 1 front, 2 none (GX faces are front when clockwise on screen).
    pub cull: u8,
    /// 1: environment mapped: its texture coordinates change every frame
    /// ([`sms_view_frame_uvs`]).
    pub env: u8,
}

/// A texture's size and wrap modes (0 repeat, 1 clamp, 2 mirror).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SmsTexture {
    pub width: u32,
    pub height: u32,
    pub wrap_s: u8,
    pub wrap_t: u8,
    pub _pad: [u8; 2],
}

/// One posed instance: an object of model `model` at `pos` rotated by `rot` (x, y, z, w),
/// with the vertices and colours at the offsets. Mirror it across its own X in a left-handed
/// engine.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SmsDraw {
    /// Stable while the part is shown (the part in the top byte).
    pub key: u64,
    pub model: u32,
    /// `libsms_present::view::models`: 0 body, 1-2 open hands, 3-4 fists, 5 cap, 6 FLUDD,
    /// 7-11 nozzles (Spray, Hover, Rocket, Turbo, Yoshi).
    pub kind: u8,
    /// 1: flashing from a hit.
    pub flags: u8,
    pub _pad: [u8; 2],
    /// Mario's feet (render-interpolated), sim units.
    pub pos: SmsVec3,
    pub rot: [f32; 4],
    pub first_vertex: u32,
    pub first_color: u32,
    /// 1 opaque .. 0 gone.
    pub alpha: f32,
    /// Which Mario (0).
    pub id: u32,
}

/// The frame's camera and listening for [`sms_view_update`].
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SmsViewFrame {
    /// Seconds since the last update (0 while paused).
    pub dt: f32,
    /// Camera position and facing, sim units and axes.
    pub eye: SmsVec3,
    pub forward: SmsVec3,
    pub up: SmsVec3,
    /// Sound volume 0..1.
    pub volume: f32,
    /// How many 120 Hz ticks behind the sim the host draws Mario (its own smoothing between
    /// steps); his pose, facing and water are taken as far back, so they match where he's
    /// drawn and move at the display's rate.
    pub lag: f32,
}

/// A corner of a water quad: world position (sim units), texture coordinates and opacity.
/// Quads are 4 corners in order.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SmsWaterVertex {
    pub pos: SmsVec3,
    pub uv: [f32; 2],
    pub alpha: f32,
}

/// This frame's water: [`Self::drop_quads`] drop quads (texture 0, drawn see-through in
/// [`Self::tint`]), then [`Self::splash_quads`] splash quads (texture 1, as they are).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SmsWaterInfo {
    pub drop_quads: u32,
    pub splash_quads: u32,
    pub tint: [f32; 4],
    /// After splashes: ground wetness over texture 0, drawn black with low alpha.
    pub wet_quads: u32,
}

/// A corner of a particle quad: world position (sim units), texture coordinates and colour
/// (0..1). Quads are 4 corners in order.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SmsFxVertex {
    pub pos: SmsVec3,
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

/// A run of particle quads over one texture. `blend` 0: added (`src * src.a + dst`, the
/// colours carry the effect's alpha); 1: see-through (`src * src.a + dst * (1 - src.a)`).
/// Draw the texture times the vertex colour, unlit, without writing depth.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SmsFxBatch {
    pub texture: u32,
    pub first_vertex: u32,
    pub vertex_count: u32,
    /// 0 additive, 1 alpha, 2 scene refraction. Refraction textures are linear packed normals
    /// (R=1, G=4*Y, B=coverage, A=4*X), for the host's existing scene-distortion shader.
    /// ABI 20: put B in a separate mask's alpha and divide material magnitude by 4.
    pub blend: u32,
}

/// This frame's particle counts.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SmsFxInfo {
    pub batches: u32,
    pub vertices: u32,
    pub textures: u32,
}

/// FLUDD's water gauge this frame.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SmsHudInfo {
    /// Its top-left on the game's screen, `screen_w` by `screen_h` (bottom right in the game).
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub screen_w: u32,
    pub screen_h: u32,
    /// Changes whenever the pixels do.
    pub version: u32,
    /// 1 when the game would show it (Mario has FLUDD).
    pub shown: u8,
    pub _pad: [u8; 3],
}

/// An opaque view handle.
pub struct SmsView {
    pub(crate) view: View,
    pub(crate) water: WaterLook,
    pub(crate) audio: Option<Audio>,
    status: CString,
    poisoned: bool,
    entrance: Option<libsms_present::view::Entrance>,
}

/// Whose body the view draws, by `libsms_present::view::Skin` id: 0 Mario, 1 Luigi (HSD model
/// from the Melee cache), 2 Piantissimo (a J3D model on Mario's rig). A skin that was not loaded
/// with [`sms_view_load_skin`] draws Mario's body; an unknown id is Mario.
/// # Safety
/// `view` must be a live Mario view from sms_view_new.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_skin(view: *mut SmsView, skin: u32) {
    unsafe {
        with_view(view, (), |v| {
            let skin = libsms_present::view::Skin::from_id(skin);
            v.view.set_skin(skin);
            // The skin's voice: Piantissimo's jump cue replaces Mario's jump voices.
            if let Some(audio) = &v.audio {
                audio.set_voice_set(skin.voice_set());
            }
        })
    }
}

/// Overrides the body pose this frame with one model-space 4x4 per Sunshine joint
/// (column-major, `count * 16` floats), or clears it with `count` 0. 1 when the
/// pose was accepted, 0 on a null/wrong-size buffer. Used by the Mario A-attack
/// port to draw a Melee clip on Sunshine's rig.
/// # Safety
/// `view` must be a live Mario view; `mats` must hold `count * 16` floats.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_set_pose(
    view: *mut SmsView,
    mats: *const f32,
    count: u32,
) -> u8 {
    if view.is_null() {
        return 0;
    }
    if count == 0 {
        return unsafe { with_view(view, 0, |v| v.view.set_pose_override(None) as u8) };
    }
    if mats.is_null() || count > 256 {
        return 0;
    }
    let floats = unsafe { std::slice::from_raw_parts(mats, count as usize * 16) };
    let joints: Vec<Mat4> = floats
        .chunks_exact(16)
        .map(|c| Mat4::from_cols_array(c.try_into().unwrap()))
        .collect();
    unsafe { with_view(view, 0, |v| v.view.set_pose_override(Some(joints)) as u8) }
}

/// Overrides whose voice the Mario cues play: 0 Mario's, 1 Piantissimo's jump voice with Mario's
/// lines for the rest (what [`sms_view_skin`] picks for him), 2 Piantissimo exactly as the game
/// has him (the jump voice and nothing else). Call after [`sms_view_skin`].
///
/// # Safety
/// `view` must be a live Mario view from sms_view_new.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_voice_set(view: *mut SmsView, set: u32) {
    unsafe {
        with_view(view, (), |v| {
            if let Some(audio) = &v.audio {
                audio.set_voice_set(match set {
                    1 => libsms_present::view::VoiceSet::Piantissimo,
                    2 => libsms_present::view::VoiceSet::PiantissimoStrict,
                    _ => libsms_present::view::VoiceSet::Mario,
                });
            }
        })
    }
}

/// Makes a skin drawable: Luigi reads the Melee cache at `dir`; Piantissimo (and any later
/// same-rig skin) loads from the view's Sunshine assets and ignores `dir` (null is fine). 1 when
/// its files were there and its rig fits; 0 otherwise, so a host can offer the skin only then.
/// Load every skin before the host reads the model table (the table is one list).
///
/// # Safety
/// `view` must be a live Mario view from sms_view_new; `dir` null or a nul-terminated UTF-8 path.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_load_skin(
    view: *mut SmsView,
    skin: u32,
    dir: *const c_char,
) -> u8 {
    let dir = (!dir.is_null()).then(|| {
        unsafe { std::ffi::CStr::from_ptr(dir) }
            .to_string_lossy()
            .into_owned()
    });
    unsafe {
        with_view(view, 0, |v| {
            let path = dir.as_deref().map(std::path::Path::new);
            v.view
                .load_skin(libsms_present::view::Skin::from_id(skin), path) as u8
        })
    }
}

/// Loads Luigi's HSD mesh from the Melee cache so [`sms_view_skin`]'s Luigi draws his body. 1
/// when the cache was complete; 0 (and the view keeps drawing Mario) otherwise, so a host can
/// offer the skin only then. The view must have been built from the same Sunshine assets as the
/// clips Luigi's retarget reads.
///
/// # Safety
/// `view` must be a live Mario view from sms_view_new; `melee_dir` a nul-terminated UTF-8 path.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_set_luigi(view: *mut SmsView, melee_dir: *const c_char) -> u8 {
    if melee_dir.is_null() {
        return 0;
    }
    let dir = unsafe { std::ffi::CStr::from_ptr(melee_dir) }
        .to_string_lossy()
        .into_owned();
    unsafe { with_view(view, 0, |v| v.view.set_luigi(std::path::Path::new(&dir)) as u8) }
}

/// Presentation-only Air Pump still life. Uses the existing model/draw readers.
/// # Safety
/// `view` must be a live Mario view from sms_view_new.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_nozzle_portrait(view: *mut SmsView) {
    unsafe { with_view(view, (), |v| v.view.nozzle_portrait()) }
}

/// Loads Mario's models (and with `sound` non-zero, opens the default output device for the
/// game's sounds, without music) from `assets_dir`. Null if the models can't be loaded. Free
/// with [`sms_view_free`].
///
/// # Safety
/// `assets_dir` a nul-terminated UTF-8 path.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_new(assets_dir: *const c_char, sound: u8) -> *mut SmsView {
    // SAFETY: as documented.
    unsafe { sms_view_new_look(assets_dir, sound, 0) }
}

/// [`sms_view_new`] drawing `look`: 0 Mario, 1 Shadow Mario, 2 a static yellow coin.
/// Coin views use the usual model readers; hosts supply instance transforms and never
/// call sms_view_update (Mario's animation update) on them. `look` 3 is the Shine Sprite: the
/// same readers, with the model posed by [`sms_view_shine_update`] (never `sms_view_update`),
/// its glow in the `sms_view_fx_*` readers and the Shine Get camera in [`sms_view_camera_shot`].
/// `look` 5, 6 and 7 are the sambo flower, tower and head: one instance posed by
/// [`sms_view_prop_update`] from a clip found with [`sms_view_prop_clip`].
///
/// # Safety
/// `assets_dir` a nul-terminated UTF-8 path.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_new_look(
    assets_dir: *const c_char,
    sound: u8,
    look: u8,
) -> *mut SmsView {
    if assets_dir.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: as documented.
    let dir = unsafe { std::ffi::CStr::from_ptr(assets_dir) }
        .to_string_lossy()
        .into_owned();
    let made = catch_unwind(|| {
        let dir = PathBuf::from(dir);
        let look = match look {
            1 => libsms_present::Look::Shadow,
            2 => libsms_present::Look::Coin,
            3 => libsms_present::Look::Shine,
            4 => libsms_present::Look::BlueCoin,
            5 => libsms_present::Look::Flower,
            6 => libsms_present::Look::Sambo,
            7 => libsms_present::Look::SamboHead,
            8 => libsms_present::Look::Effects,
            _ => libsms_present::Look::Mario,
        };
        let view = View::load_look(&dir, look).ok()?;
        let (audio, sound_status) = if sound != 0 {
            match Audio::open(&dir, false) {
                Ok(a) => {
                    let s = format!("sound {} Hz, {} channels", a.rate, a.channels);
                    (Some(a), s)
                }
                Err(e) => (None, format!("no sound ({e})")),
            }
        } else {
            (None, "sound off".to_string())
        };
        let status = format!(
            "{} models, {} textures; {sound_status}",
            view.models.len(),
            view.textures.list.len()
        );
        Some(SmsView {
            view,
            water: WaterLook::load(&dir),
            audio,
            status: CString::new(status).unwrap_or_default(),
            poisoned: false,
            entrance: None,
        })
    });
    match made {
        Ok(Some(v)) => Box::into_raw(Box::new(v)),
        _ => std::ptr::null_mut(),
    }
}

/// The approved statue/portal entrance using the ordinary model, texture and FX readers.
/// # Safety
/// `assets_dir` is a nul-terminated UTF-8 path.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_entrance_new(
    assets_dir: *const c_char,
    sound: u8,
) -> *mut SmsView {
    unsafe { make_entrance(assets_dir, sound, false) }
}

/// Short game entrance. The original `sms_view_entrance_new` remains available unchanged.
/// # Safety
/// `assets_dir` is a nul-terminated UTF-8 path with a worker-prepared portal cache.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_entrance_fast_new(assets_dir: *const c_char, sound: u8) -> *mut SmsView {
    unsafe { make_entrance(assets_dir, sound, true) }
}

unsafe fn make_entrance(assets_dir: *const c_char, sound: u8, fast: bool) -> *mut SmsView {
    let ptr = unsafe { sms_view_new(assets_dir, sound) };
    if ptr.is_null() {
        return ptr;
    }
    let loaded = catch_unwind(AssertUnwindSafe(|| {
        let dir = PathBuf::from(
            unsafe { std::ffi::CStr::from_ptr(assets_dir) }
                .to_string_lossy()
                .into_owned(),
        );
        let v = unsafe { &mut *ptr };
        v.entrance = Some(if fast { libsms_present::view::Entrance::load_fast(&dir, &mut v.view).ok()? } else { libsms_present::view::Entrance::load(&dir, &mut v.view).ok()? });
        Some(())
    }));
    if !matches!(loaded, Ok(Some(()))) {
        unsafe { sms_view_free(ptr) };
        return std::ptr::null_mut();
    }
    ptr
}

/// 1 playing, 2 finished, 0 failure. `seconds` is idle time or time since activation.
/// # Safety
/// `view` is a live entrance view; `frame` readable and `position` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_entrance_update(
    view: *mut SmsView,
    seconds: f32,
    activated: u8,
    frame: *const SmsViewFrame,
    position: *mut SmsVec3,
) -> u8 {
    let (Some(f), Some(pos)) = (unsafe { frame.as_ref() }, unsafe { position.as_mut() }) else {
        return 0;
    };
    if !seconds.is_finite() || !(0.0..=86400.0).contains(&seconds) {
        return 0;
    }
    unsafe {
        with_view(view, 0, |v| {
            let Some(entrance) = &mut v.entrance else {
                return 0;
            };
            let forward = Vec3::from(f.forward).normalize_or(Vec3::Z);
            let right = Vec3::from(f.up).cross(forward).normalize_or(Vec3::X);
            let up = forward.cross(right);
            let fast = entrance.is_fast();
            let done = if fast { entrance.update_fast(&mut v.view, seconds, activated, right, up) } else { entrance.update(&mut v.view, seconds, activated != 0, right, up) };
            let feet = entrance.position(seconds);
            *pos = feet.into();
            if let Some(a) = &mut v.audio {
                a.set_volume(f.volume);
                a.set_listener(listener_view(f.eye.into(), forward, up), f.eye.into());
                if fast && activated == 1 {
                    a.portal_exit((seconds / libsms_present::portal_cache::REVEAL * 89.).clamp(0., 89.), feet);
                }
            }
            if done { 2 } else { 1 }
        })
    }
}

/// Native pedestal bounds for host collision, in portal-relative centimetres.
/// # Safety
/// `view` is a live entrance view; `lo` and `hi` are writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_entrance_bounds(
    view: *mut SmsView,
    lo: *mut SmsVec3,
    hi: *mut SmsVec3,
) -> u8 {
    let (Some(lo), Some(hi)) = (unsafe { lo.as_mut() }, unsafe { hi.as_mut() }) else {
        return 0;
    };
    unsafe {
        with_view(view, 0, |v| {
            let Some(e) = &v.entrance else {
                return 0;
            };
            let (a, b) = e.pedestal_bounds(&v.view);
            if !a.is_finite() || !b.is_finite() {
                return 0;
            }
            *lo = a.into();
            *hi = b.into();
            1
        })
    }
}

/// Play TCoin::taken's sound and TMario::emitGetCoinEffect's two particle effects.
/// Health and pickup ownership belong to the host.
///
/// # Safety
/// `view` is null or a live Mario view; `pos` is readable, in sim world units.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_coin_get(view: *mut SmsView, pos: *const SmsVec3) {
    let Some(pos) = (unsafe { pos.as_ref() }) else {
        return;
    };
    let pos = Vec3::from(*pos);
    if !pos.is_finite() {
        return;
    }
    unsafe {
        with_view(view, (), |v| {
            v.view.coin_get(pos);
            if let Some(a) = &v.audio {
                a.coin_get(pos);
            }
        });
    }
}

/// One call of [`sms_view_shine_update`]: where the Shine is and which of its animations.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SmsShineFrame {
    /// Seconds since the last call, for its glow.
    pub dt: f32,
    /// Where it stands (sim world units) and turned how far (a 16-bit angle).
    pub pos: SmsVec3,
    pub yaw: i16,
    /// 0 waiting (the model's own pose), 1 `shine_float`, 2 `shine_demo_shine_get`.
    pub clip: u8,
    /// 1: the host draws the instance mirrored across X (the models are right-handed), so the glow
    /// is placed on the joint as drawn.
    pub mirror: u8,
    /// Frame of that clip, 60 a second.
    pub frame: f32,
    /// The glow's size: 1, or one half once taken.
    pub scale: f32,
    /// The camera's right and up, for the glow's billboards.
    pub right: SmsVec3,
    pub up: SmsVec3,
}

/// A Shine view's animation lengths ([`sms_view_shine_info`]).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SmsShineInfo {
    /// `shine_float`'s and `shine_demo_shine_get`'s frames.
    pub float_frames: f32,
    pub demo_frames: f32,
    /// The Shine Get camera's path for a Shine in the open and under a roof; 0 without it.
    pub camera_outside_frames: f32,
    pub camera_inside_frames: f32,
}

/// The Shine Get camera at one moment ([`sms_view_camera_shot`]).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SmsCameraShot {
    pub eye: SmsVec3,
    pub at: SmsVec3,
    /// Unit up vector.
    pub up: SmsVec3,
    /// Vertical field of view, degrees.
    pub fov: f32,
}

/// Poses a [`sms_view_new_look`] Shine view (look 3) and runs its glow: the model's posed vertices
/// go to the usual readers, one draw (the host places its own instance), the glow to the
/// `sms_view_fx_*` readers (sim world units).
///
/// # Safety
/// `view` a Shine view; `frame` readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_shine_update(view: *mut SmsView, frame: *const SmsShineFrame) {
    // SAFETY: as documented.
    let Some(f) = (unsafe { frame.as_ref() }).copied() else {
        return;
    };
    if !Vec3::from(f.pos).is_finite() {
        return;
    }
    // SAFETY: as documented.
    unsafe {
        with_view(view, (), |v| {
            v.view.set_camera(Vec3::from(f.right), Vec3::from(f.up));
            v.view.update_shine(
                if f.dt.is_finite() { f.dt.clamp(0.0, 0.25) } else { 0.0 },
                Vec3::from(f.pos),
                f.yaw,
                f.clip,
                if f.frame.is_finite() { f.frame.max(0.0) } else { 0.0 },
                if f.scale.is_finite() { f.scale.clamp(0.0, 8.0) } else { 1.0 },
                f.mirror != 0,
            );
        })
    }
}

/// The Shine's animation and camera lengths. Returns 0 for a view that isn't a Shine's.
///
/// # Safety
/// `view` from [`sms_view_new_look`]; `out` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_shine_info(view: *mut SmsView, out: *mut SmsShineInfo) -> u8 {
    // SAFETY: as documented.
    unsafe {
        with_view(view, 0, |v| {
            let (Some(out), Some(clips)) = (out.as_mut(), v.view.shine_clip_frames()) else {
                return 0;
            };
            *out = SmsShineInfo {
                float_frames: clips[0],
                demo_frames: clips[1],
                camera_outside_frames: v.view.demo_camera_frames(false),
                camera_inside_frames: v.view.demo_camera_frames(true),
            };
            1
        })
    }
}

/// The Shine Get camera (`inside` non-zero for a Shine under a roof) at clip frame `frame`: sim
/// units with Mario's feet at the origin, facing along +Z (the demo turns it by the Shine's yaw).
/// Returns 0 without the camera's files.
///
/// # Safety
/// `view` a Shine view; `out` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_camera_shot(
    view: *mut SmsView,
    inside: u8,
    frame: f32,
    out: *mut SmsCameraShot,
) -> u8 {
    // SAFETY: as documented.
    unsafe {
        with_view(view, 0, |v| {
            let (Some(out), Some(s)) = (
                out.as_mut(),
                v.view.demo_camera(inside != 0, if frame.is_finite() { frame } else { 0.0 }),
            ) else {
                return 0;
            };
            *out = SmsCameraShot {
                eye: s.eye.into(),
                at: s.at.into(),
                up: s.up.into(),
                fov: s.fov,
            };
            1
        })
    }
}

/// Plays a sound effect of the game's (`name`, an `MSD_SE_*`) at `pos` (sim world units) through a
/// view that has sound. Returns 0 without sound or for a name the game doesn't have.
///
/// # Safety
/// `view` from [`sms_view_new`]; `name` a nul-terminated UTF-8 string; `pos` readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_sound(
    view: *mut SmsView,
    name: *const c_char,
    pos: *const SmsVec3,
) -> u8 {
    if name.is_null() {
        return 0;
    }
    // SAFETY: as documented.
    let (Some(pos), Ok(name)) = (
        unsafe { pos.as_ref() },
        unsafe { std::ffi::CStr::from_ptr(name) }.to_str(),
    ) else {
        return 0;
    };
    let pos = Vec3::from(*pos);
    if !pos.is_finite() {
        return 0;
    }
    // SAFETY: as documented.
    unsafe {
        with_view(view, 0, |v| {
            v.audio.as_ref().is_some_and(|a| a.play_at(name, pos)) as u8
        })
    }
}

/// A held sound of a host object (the Shine's `MSD_SE_SHINE_EXIST` hum, which the game asks for
/// every frame): call it every frame the object makes it, with its position (sim world units); it
/// starts the first time and fades when the calls stop. Returns 0 without sound or for an unknown
/// name.
///
/// # Safety
/// `view` from [`sms_view_new`]; `name` a nul-terminated UTF-8 string; `pos` readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_hold_sound(
    view: *mut SmsView,
    name: *const c_char,
    pos: *const SmsVec3,
) -> u8 {
    if name.is_null() {
        return 0;
    }
    // SAFETY: as documented.
    let (Some(pos), Ok(name)) = (
        unsafe { pos.as_ref() },
        unsafe { std::ffi::CStr::from_ptr(name) }.to_str(),
    ) else {
        return 0;
    };
    let pos = Vec3::from(*pos);
    if !pos.is_finite() {
        return 0;
    }
    // SAFETY: as documented.
    unsafe {
        with_view(view, 0, |v| {
            v.audio.as_mut().is_some_and(|a| a.hold_at(name, pos)) as u8
        })
    }
}

/// Holds this view's sounds still while the host's game is paused (nonzero) and lets them go on
/// from the same place when it isn't. Call it every frame; a view without sound ignores it.
///
/// # Safety
/// `view` from [`sms_view_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_set_paused(view: *mut SmsView, paused: u8) {
    // SAFETY: as documented.
    unsafe {
        with_view(view, (), |v| {
            if let Some(a) = &v.audio {
                a.set_paused(paused != 0);
            }
        })
    }
}

/// Also draws each forearm and glove on its own (nonzero) for a VR host to put on the
/// player's controllers: [`libsms_present::View::set_vr_arms`]. Each is a draw of the body
/// model keyed `VR_ARM_KEY` (0x10) + side (0 right, 1 left) in its low byte, posed in its own
/// arm space (glove centre at the origin, +z toward the fingers, +y up), so the host places it
/// at the controller instead of at Mario. Mario and the same-rig skins only.
///
/// # Safety
/// `view` from [`sms_view_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_set_vr_arms(view: *mut SmsView, on: u8) {
    // SAFETY: as documented.
    unsafe { with_view(view, (), |v| v.view.set_vr_arms(on != 0)) }
}

/// # Safety
/// `view` from [`sms_view_new`] and not used afterwards. Null is ignored.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_free(view: *mut SmsView) {
    if !view.is_null() {
        // SAFETY: the caller hands back ownership.
        let v = unsafe { Box::from_raw(view) };
        // Stopping the stream joins its thread; never let a panic out.
        let _ = catch_unwind(AssertUnwindSafe(move || drop(v)));
    }
}

/// What loaded ("12 models, 26 textures; sound 48000 Hz, 2 channels"). Valid until the view
/// is freed.
///
/// # Safety
/// `view` from [`sms_view_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_status(view: *const SmsView) -> *const c_char {
    // SAFETY: as documented.
    match unsafe { view.as_ref() } {
        Some(v) => v.status.as_ptr(),
        None => std::ptr::null(),
    }
}

/// Runs `f` on a live view, poisoning it if `f` panics.
///
/// # Safety
/// `view` null or from [`sms_view_new`], not aliased for the call.
pub(crate) unsafe fn with_view<R>(
    view: *mut SmsView,
    fallback: R,
    f: impl FnOnce(&mut SmsView) -> R,
) -> R {
    // SAFETY: as documented.
    let Some(v) = (unsafe { view.as_mut() }) else {
        return fallback;
    };
    if v.poisoned {
        return fallback;
    }
    match catch_unwind(AssertUnwindSafe(|| f(v))) {
        Ok(r) => r,
        Err(_) => {
            v.poisoned = true;
            fallback
        }
    }
}

/// Number of models (see [`SmsDraw::kind`]).
///
/// # Safety
/// `view` from [`sms_view_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_model_count(view: *mut SmsView) -> u32 {
    // SAFETY: as documented.
    unsafe { with_view(view, 0, |v| v.view.models.len() as u32 + v.view.luigi.is_some() as u32) }
}

/// # Safety
/// `view` from [`sms_view_new`]; `out` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_model(
    view: *mut SmsView,
    model: u32,
    out: *mut SmsModelInfo,
) -> u8 {
    // SAFETY: as documented.
    unsafe {
        with_view(view, 0, |v| {
            if model == v.view.models.len() as u32 && v.view.luigi.is_some() {
                let (Some(l), Some(out)) = (&v.view.luigi, out.as_mut()) else {
                    return 0;
                };
                *out = SmsModelInfo {
                    vertex_count: l.vertex_count as u32,
                    index_count: l.indices.len() as u32,
                    batch_count: l.batches.len() as u32,
                    texture_count: v.view.textures.list.len() as u32,
                    lo: l.lo.into(),
                    hi: l.hi.into(),
                };
                return 1;
            }
            let (Some(m), Some(out)) = (v.view.models.get(model as usize), out.as_mut()) else {
                return 0;
            };
            *out = SmsModelInfo {
                vertex_count: m.vertex_count as u32,
                index_count: m.batches.iter().map(|b| b.indices.len() as u32).sum(),
                batch_count: m.batches.len() as u32,
                texture_count: v.view.textures.list.len() as u32,
                lo: m.lo.into(),
                hi: m.hi.into(),
            };
            1
        })
    }
}

/// Writes up to `max` of the model's batches; returns how many.
///
/// # Safety
/// `view` from [`sms_view_new`]; `out` writable for `max`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_batches(
    view: *mut SmsView,
    model: u32,
    out: *mut SmsBatch,
    max: u32,
) -> u32 {
    // SAFETY: as documented.
    unsafe {
        with_view(view, 0, |v| {
            if model == v.view.models.len() as u32 && v.view.luigi.is_some() {
                let Some(l) = &v.view.luigi else {
                    return 0;
                };
                let mut n = 0;
                for b in &l.batches {
                    if n >= max as usize || out.is_null() {
                        break;
                    }
                    out.add(n).write(SmsBatch {
                        first_index: b.first_index,
                        index_count: b.index_count,
                        first_vertex: b.first_vertex,
                        vertex_count: b.vertex_count,
                        texture: b.texture,
                        blend: b.blend,
                        lit: b.lit,
                        cull: 2,
                        env: 0,
                    });
                    n += 1;
                }
                return n as u32;
            }
            let Some(m) = v.view.models.get(model as usize) else {
                return 0;
            };
            let (mut index, mut vertex) = (0u32, 0u32);
            let mut n = 0;
            for (i, b) in m.batches.iter().enumerate() {
                if n >= max as usize || out.is_null() {
                    break;
                }
                out.add(n).write(SmsBatch {
                    first_index: index,
                    index_count: b.indices.len() as u32,
                    first_vertex: vertex,
                    vertex_count: b.vertices.len() as u32,
                    texture: m.batch_texture[i],
                    blend: m.batch_blend[i],
                    lit: m.batch_lit[i],
                    cull: m.batch_cull[i],
                    env: m.batch_env[i].is_some() as u8,
                });
                index += b.indices.len() as u32;
                vertex += b.vertices.len() as u32;
                n += 1;
            }
            n as u32
        })
    }
}

/// The model's triangle indices, batch after batch, into the model's whole vertex list (batch
/// `b`'s are `first_index .. first_index + index_count`). Returns how many were written.
///
/// # Safety
/// `view` from [`sms_view_new`]; `out` writable for `max`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_indices(
    view: *mut SmsView,
    model: u32,
    out: *mut u32,
    max: u32,
) -> u32 {
    // SAFETY: as documented.
    unsafe {
        with_view(view, 0, |v| {
            if model == v.view.models.len() as u32 && v.view.luigi.is_some() {
                let Some(l) = &v.view.luigi else {
                    return 0;
                };
                if out.is_null() {
                    return 0;
                }
                let n = l.indices.len().min(max as usize);
                std::ptr::copy_nonoverlapping(l.indices.as_ptr(), out, n);
                return n as u32;
            }
            let Some(m) = v.view.models.get(model as usize) else {
                return 0;
            };
            if out.is_null() {
                return 0;
            }
            let (mut n, mut base) = (0usize, 0u32);
            for b in &m.batches {
                for &i in &b.indices {
                    if n >= max as usize {
                        return n as u32;
                    }
                    out.add(n).write(base + i);
                    n += 1;
                }
                base += b.vertices.len() as u32;
            }
            n as u32
        })
    }
}

/// The model's texture coordinates (its first set), one per vertex in vertex order. Returns
/// how many.
///
/// # Safety
/// `view` from [`sms_view_new`]; `out` writable for `max` pairs of floats.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_uvs(
    view: *mut SmsView,
    model: u32,
    out: *mut [f32; 2],
    max: u32,
) -> u32 {
    // SAFETY: as documented.
    unsafe {
        with_view(view, 0, |v| {
            if model == v.view.models.len() as u32 && v.view.luigi.is_some() {
                let Some(l) = &v.view.luigi else {
                    return 0;
                };
                if out.is_null() {
                    return 0;
                }
                let n = l.uvs.len().min(max as usize);
                std::ptr::copy_nonoverlapping(l.uvs.as_ptr(), out, n);
                return n as u32;
            }
            let Some(m) = v.view.models.get(model as usize) else {
                return 0;
            };
            if out.is_null() {
                return 0;
            }
            let mut n = 0usize;
            for vtx in m.batches.iter().flat_map(|b| &b.vertices) {
                if n >= max as usize {
                    break;
                }
                out.add(n).write([vtx.uv[0].x, vtx.uv[0].y]);
                n += 1;
            }
            n as u32
        })
    }
}

/// The model's vertex alphas, one float per vertex in vertex order (1 where a vertex has none;
/// batches with [`SmsBatch::lit`] bit 4 have them): what a see-through batch's vertices fade it
/// by. Returns how many.
///
/// # Safety
/// `view` from [`sms_view_new`]; `out` writable for `max` floats.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_vertex_alphas(
    view: *mut SmsView,
    model: u32,
    out: *mut f32,
    max: u32,
) -> u32 {
    // SAFETY: as documented.
    unsafe {
        with_view(view, 0, |v| {
            let Some(m) = v.view.models.get(model as usize) else {
                return 0;
            };
            if out.is_null() {
                return 0;
            }
            let n = m.vertex_alpha.len().min(max as usize);
            std::ptr::copy_nonoverlapping(m.vertex_alpha.as_ptr(), out, n);
            n as u32
        })
    }
}

/// Baked texture `id`: its size and wrap modes in `info`, and if `rgba` isn't null, its pixels
/// (RGBA8, rows top first, `width * height * 4` bytes; `len` is the room). The colour is all
/// there: a host multiplies it by its lighting and the draw's colour. Returns 0 for an unknown
/// id or too little room.
///
/// # Safety
/// `view` from [`sms_view_new`]; `info` null or writable; `rgba` null or writable for `len`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_texture(
    view: *mut SmsView,
    id: u32,
    info: *mut SmsTexture,
    rgba: *mut u8,
    len: u32,
) -> u8 {
    // SAFETY: as documented.
    unsafe {
        with_view(view, 0, |v| {
            let Some(t) = v.view.textures.list.get(id as usize) else {
                return 0;
            };
            if let Some(info) = info.as_mut() {
                *info = SmsTexture {
                    width: t.width,
                    height: t.height,
                    wrap_s: t.wrap[0],
                    wrap_t: t.wrap[1],
                    _pad: [0; 2],
                };
            }
            if rgba.is_null() {
                return 1;
            }
            if t.rgba.len() > len as usize {
                return 0;
            }
            std::ptr::copy_nonoverlapping(t.rgba.as_ptr(), rgba, t.rgba.len());
            1
        })
    }
}

/// # Safety
/// `view` must be a live view handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_texture_version(view: *mut SmsView, id: u32) -> u32 {
    unsafe { with_view(view, 0, |v| v.view.textures.version(id)) }
}

/// World-to-camera for the sound system: camera looking down -Z with +X to its right. Hosts
/// pass Unity-style axes (left-handed: right = up x forward), which is what the sim's numbers
/// are in a host that copies its coordinates straight across.
fn listener_view(eye: Vec3, forward: Vec3, up: Vec3) -> Mat4 {
    let f = forward.normalize_or(Vec3::Z);
    let r = up.cross(f).normalize_or(Vec3::X);
    let u = f.cross(r);
    let rot = Mat4::from_cols(r.extend(0.0), u.extend(0.0), (-f).extend(0.0), Vec4::W).transpose();
    rot * Mat4::from_translation(-eye)
}

/// Poses `sim` for this frame (and points FLUDD's water at the drawn nozzle), builds its water's
/// quads facing the camera, and sets the listener and volume for its sounds.
///
/// # Safety
/// `view` from [`sms_view_new`]; `sim` from `sms_sim_new`; `frame` readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_update(
    view: *mut SmsView,
    sim: *mut SmsSim,
    frame: *const SmsViewFrame,
) {
    // SAFETY: as documented.
    let (Some(s), Some(f)) = (unsafe { sim.as_mut() }, unsafe { frame.as_ref() }) else {
        return;
    };
    if s.poisoned {
        return;
    }
    let f = *f;
    // SAFETY: as documented.
    unsafe {
        with_view(view, (), |v| {
            let fwd = Vec3::from(f.forward).normalize_or(Vec3::Z);
            let right = Vec3::from(f.up).cross(fwd).normalize_or(Vec3::X);
            let up = fwd.cross(right);
            v.view.set_camera(right, up);
            v.view.lag = if f.lag.is_finite() {
                f.lag.clamp(0.0, 16.0)
            } else {
                0.0
            };
            v.view.update(&mut s.sim, f.dt.clamp(0.0, 0.25));
            let frac = (s.water_ticks as f32 + s.sim.alpha() - f.lag) / 4.0;
            let juice = s.sim.mario.juice_color();
            // FLUDD's muzzle as drawn: Mario is drawn `lag` ticks behind the sim.
            v.water.nozzle = s.sim.mario.muzzle_world(0).map(|m| {
                let ticks = s.sim.alpha() - f.lag.clamp(0.0, 16.0);
                m + Vec3::new(s.sim.mario.vel.x, 0.0, s.sim.mario.vel.z) * (0.25 * ticks)
            });
            v.water
                .build(&s.water, frac, f.eye.into(), right, up, juice);
            if let Some(a) = &v.audio {
                a.set_volume(f.volume);
                a.set_listener(listener_view(f.eye.into(), fwd, up), f.eye.into());
            }
        })
    }
}

/// The body attachment point `id` as the last [`sms_view_update`] posed it: 16 floats,
/// column-major, in model space (the space the draws' vertices are in). 0 when the current
/// skin has no such part. The ids are `libsms_present::view::Anchor`, shared with the
/// Pikmin view so one host class mounts item displays on either.
///
/// # Safety
/// `view` from [`sms_view_new`]; `out` 16 writable floats.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_anchor(view: *const SmsView, id: u32, out: *mut f32) -> u32 {
    if view.is_null() || out.is_null() {
        return 0;
    }
    // SAFETY: as documented.
    let Some(m) = (unsafe { &*view }).view.anchor(id as usize) else {
        return 0;
    };
    unsafe { std::ptr::copy_nonoverlapping(m.to_cols_array().as_ptr(), out, 16) };
    1
}

/// Mario says `name` (an `MSD_SE_MV*` voice, e.g. `MSD_SE_MV23_JUMP_LARGE_01`), cutting off
/// his last voice. Nothing without sound or an unknown name.
///
/// # Safety
/// `view` from [`sms_view_new`]; `name` a nul-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_mario_voice(view: *mut SmsView, name: *const c_char) {
    if name.is_null() {
        return;
    }
    // SAFETY: as documented.
    let Ok(name) = (unsafe { std::ffi::CStr::from_ptr(name) }).to_str() else {
        return;
    };
    unsafe {
        with_view(view, (), |v| {
            if let Some(a) = &v.audio {
                a.mario_voice(name);
            }
        })
    }
}

/// Pitches Mario's voice cues with their length kept (Luigi's is Mario's up 14%: `ratio` 1.14);
/// 1.0 leaves them alone. Additive to the ABI, like [`sms_view_mario_voice`].
///
/// # Safety
/// `view` must be a live Mario view from sms_view_new.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_set_voice_pitch(view: *mut SmsView, ratio: f32) {
    unsafe {
        with_view(view, (), |v| {
            if let Some(a) = &v.audio {
                a.set_voice_pitch(ratio);
            }
        })
    }
}

/// Writes up to `max` of this frame's instances; returns how many (with `out` null, how many
/// there are).
///
/// # Safety
/// `view` from [`sms_view_new`]; `out` null or writable for `max`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_draws(view: *mut SmsView, out: *mut SmsDraw, max: u32) -> u32 {
    // SAFETY: as documented.
    unsafe {
        with_view(view, 0, |v| {
            if out.is_null() {
                return v.view.draws.len() as u32;
            }
            let n = v.view.draws.len().min(max as usize);
            for (i, d) in v.view.draws.iter().take(n).enumerate() {
                out.add(i).write(SmsDraw {
                    key: d.key,
                    model: d.model,
                    kind: d.kind,
                    flags: d.flags,
                    _pad: [0; 2],
                    pos: d.pos.into(),
                    rot: d.rot.to_array(),
                    first_vertex: d.first_vertex,
                    first_color: d.first_color,
                    alpha: d.alpha,
                    id: (d.key & 0xFFFF_FFFF) as u32,
                });
            }
            n as u32
        })
    }
}

/// This frame's skinned positions for every instance (each in its model space, sim units);
/// `count` gets how many. Valid until the next [`sms_view_update`] or [`sms_view_free`].
///
/// # Safety
/// `view` from [`sms_view_new`]; `count` null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_positions(view: *mut SmsView, count: *mut u32) -> *const SmsVec3 {
    // SAFETY: as documented. `SmsVec3` and glam's `Vec3` are both three packed f32s.
    unsafe {
        with_view(view, std::ptr::null(), |v| {
            if let Some(c) = count.as_mut() {
                *c = v.view.positions.len() as u32;
            }
            v.view.positions.as_ptr().cast()
        })
    }
}

/// Like [`sms_view_positions`], the normals.
///
/// # Safety
/// As [`sms_view_positions`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_normals(view: *mut SmsView, count: *mut u32) -> *const SmsVec3 {
    // SAFETY: as above.
    unsafe {
        with_view(view, std::ptr::null(), |v| {
            if let Some(c) = count.as_mut() {
                *c = v.view.normals.len() as u32;
            }
            v.view.normals.as_ptr().cast()
        })
    }
}

/// This frame's batch colours (RGBA 0..1, one per batch of every instance: what its texture is
/// multiplied by, or its whole colour if it has none). Valid as [`sms_view_positions`].
///
/// # Safety
/// As [`sms_view_positions`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_colors(view: *mut SmsView, count: *mut u32) -> *const [f32; 4] {
    // SAFETY: as above.
    unsafe {
        with_view(view, std::ptr::null(), |v| {
            if let Some(c) = count.as_mut() {
                *c = v.view.colors.len() as u32;
            }
            v.view.colors.as_ptr()
        })
    }
}

/// This frame's batch textures (alongside [`sms_view_colors`]): a [`sms_view_texture`] id or
/// -1. Valid as [`sms_view_positions`].
///
/// # Safety
/// As [`sms_view_positions`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_texture_ids(view: *mut SmsView, count: *mut u32) -> *const i32 {
    // SAFETY: as above.
    unsafe {
        with_view(view, std::ptr::null(), |v| {
            if let Some(c) = count.as_mut() {
                *c = v.view.texture_ids.len() as u32;
            }
            v.view.texture_ids.as_ptr()
        })
    }
}

/// This frame's texture coordinates for every instance, alongside [`sms_view_positions`]: the
/// model's own, except environment-mapped batches' ([`SmsBatch::env`]), which follow the
/// camera. Valid as [`sms_view_positions`].
///
/// # Safety
/// As [`sms_view_positions`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_frame_uvs(
    view: *mut SmsView,
    count: *mut u32,
) -> *const [f32; 2] {
    // SAFETY: as above.
    unsafe {
        with_view(view, std::ptr::null(), |v| {
            if let Some(c) = count.as_mut() {
                *c = v.view.uvs.len() as u32;
            }
            v.view.uvs.as_ptr()
        })
    }
}

const _: () = assert!(std::mem::size_of::<SmsVec3>() == std::mem::size_of::<Vec3>());

/// This frame's water quads (see [`SmsWaterInfo`]); `count` gets how many corners. Valid as
/// [`sms_view_positions`].
///
/// # Safety
/// As [`sms_view_positions`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_water_vertices(
    view: *mut SmsView,
    count: *mut u32,
) -> *const SmsWaterVertex {
    // SAFETY: as above; the layouts match (checked below).
    unsafe {
        with_view(view, std::ptr::null(), |v| {
            if let Some(c) = count.as_mut() {
                *c = v.water.vertices.len() as u32;
            }
            v.water.vertices.as_ptr().cast()
        })
    }
}

/// Last N drop quads are the unified type-0 surface: white tint texture, vertex-alpha
/// coverage, sphere-map UVs for refraction. Other drop quads keep the disc mask.
/// # Safety
/// `view` from [`sms_view_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_water_stream_quads(view: *mut SmsView) -> u32 {
    unsafe { with_view(view, 0, |v| v.water.stream_quads) }
}

/// Enable cohesive type-0 coverage. Disable for tightly cropped still-life portraits
/// whose approved framing includes the original mask's faint outer edge.
/// # Safety
/// `view` from [`sms_view_new`]. Takes effect on the next view update.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_set_water_cohesive(view: *mut SmsView, on: u8) {
    unsafe { with_view(view, (), |v| v.water.cohesive = on != 0) }
}

/// This frame's water counts and drop tint.
///
/// # Safety
/// `view` from [`sms_view_new`]; `out` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_water_info(view: *mut SmsView, out: *mut SmsWaterInfo) -> u8 {
    // SAFETY: as documented.
    unsafe {
        with_view(view, 0, |v| {
            let Some(out) = out.as_mut() else {
                return 0;
            };
            *out = SmsWaterInfo {
                drop_quads: v.water.drop_quads,
                splash_quads: v.water.splash_quads,
                tint: v.water.tint,
                wet_quads: v.water.wet_quads,
            };
            1
        })
    }
}

/// The water's textures, like [`sms_view_texture`]: 0 the drop (white, its coverage in
/// alpha), 1 the splash. Wrap is clamp. Returns 0 if it's missing (draw untextured) or `len`
/// is short.
///
/// # Safety
/// As [`sms_view_texture`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_water_texture(
    view: *mut SmsView,
    which: u32,
    info: *mut SmsTexture,
    rgba: *mut u8,
    len: u32,
) -> u8 {
    // SAFETY: as documented.
    unsafe {
        with_view(view, 0, |v| {
            let t = match which {
                0 => &v.water.drop,
                1 => &v.water.splash,
                _ => return 0,
            };
            if t.rgba.is_empty() {
                return 0;
            }
            if let Some(info) = info.as_mut() {
                *info = SmsTexture {
                    width: t.width,
                    height: t.height,
                    wrap_s: 1,
                    wrap_t: 1,
                    _pad: [0; 2],
                };
            }
            if rgba.is_null() {
                return 1;
            }
            if t.rgba.len() > len as usize {
                return 0;
            }
            std::ptr::copy_nonoverlapping(t.rgba.as_ptr(), rgba, t.rgba.len());
            1
        })
    }
}

const _: () = assert!(std::mem::size_of::<SmsWaterVertex>() == std::mem::size_of::<WaterVertex>());
const _: () = assert!(std::mem::size_of::<SmsFxVertex>() == std::mem::size_of::<FxVertex>());
const _: () = assert!(std::mem::size_of::<SmsFxBatch>() == std::mem::size_of::<FxBatch>());

/// Independently show movement effects (dust, stars, smears, slides, stomps) and the super
/// pound's flames/landing burst. Defaults to both on. No ABI struct layouts change.
///
/// # Safety
/// `view` from [`sms_view_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_set_fx(view: *mut SmsView, movement: u8, flames: u8) {
    // SAFETY: as documented.
    unsafe {
        with_view(view, (), |v| {
            v.view.movement_effects = movement != 0;
            v.view.pound_flames = flames != 0;
        });
    }
}

/// Whether this particle texture stays in world space, without Mario's render offset.
///
/// # Safety
/// `view` from [`sms_view_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_fx_texture_world_space(view: *mut SmsView, texture: u32) -> u8 {
    unsafe { with_view(view, 0, |v| v.view.fx.texture_world_space(texture) as u8) }
}

/// Host smoke playback rate (0.25..4); defaults to the disc's rate, 1. Layouts are unchanged.
///
/// # Safety
/// `view` from [`sms_view_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_set_smoke_speed(view: *mut SmsView, speed: f32) {
    unsafe {
        with_view(view, (), |v| v.view.set_smoke_speed(speed));
    }
}

/// Host water playback rate for movement-thrown water (0.25..4); defaults to the disc's, 1.
/// The Spray stream and the runoff drips are paced by the sim's water manager.
///
/// # Safety
/// `view` from [`sms_view_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_set_water_speed(view: *mut SmsView, speed: f32) {
    unsafe {
        with_view(view, (), |v| v.view.set_water_speed(speed));
    }
}

/// This frame's particle counts (after [`sms_view_update`]).
///
/// # Safety
/// `view` from [`sms_view_new`]; `out` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_fx_info(view: *mut SmsView, out: *mut SmsFxInfo) -> u8 {
    // SAFETY: as documented.
    unsafe {
        with_view(view, 0, |v| {
            let Some(out) = out.as_mut() else {
                return 0;
            };
            *out = SmsFxInfo {
                batches: v.view.fx.batches.len() as u32,
                vertices: v.view.fx.vertices.len() as u32,
                textures: v.view.fx.textures.len() as u32,
            };
            1
        })
    }
}

/// This frame's particle quads; `count` gets how many corners. Valid as
/// [`sms_view_positions`].
///
/// # Safety
/// As [`sms_view_positions`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_fx_vertices(
    view: *mut SmsView,
    count: *mut u32,
) -> *const SmsFxVertex {
    // SAFETY: as above; the layouts match (checked above).
    unsafe {
        with_view(view, std::ptr::null(), |v| {
            if let Some(c) = count.as_mut() {
                *c = v.view.fx.vertices.len() as u32;
            }
            v.view.fx.vertices.as_ptr().cast()
        })
    }
}

/// This frame's particle batches; `count` gets how many. Valid as [`sms_view_positions`].
///
/// # Safety
/// As [`sms_view_positions`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_fx_batches(
    view: *mut SmsView,
    count: *mut u32,
) -> *const SmsFxBatch {
    // SAFETY: as above; the layouts match (checked above).
    unsafe {
        with_view(view, std::ptr::null(), |v| {
            if let Some(c) = count.as_mut() {
                *c = v.view.fx.batches.len() as u32;
            }
            v.view.fx.batches.as_ptr().cast()
        })
    }
}

/// Bumped whenever particle texture `which`'s pixels change (0 before it has any).
///
/// # Safety
/// `view` from [`sms_view_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_fx_texture_version(view: *mut SmsView, which: u32) -> u32 {
    // SAFETY: as documented.
    unsafe {
        with_view(view, 0, |v| {
            v.view
                .fx
                .textures
                .get(which as usize)
                .map_or(0, |t| t.version)
        })
    }
}

/// Particle texture `which`, with independent S/T wrap modes (0 repeat, 1 clamp,
/// 2 mirror). Returns 0 if there's no such texture or `len` is short.
///
/// # Safety
/// As [`sms_view_texture`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_fx_texture(
    view: *mut SmsView,
    which: u32,
    info: *mut SmsTexture,
    rgba: *mut u8,
    len: u32,
) -> u8 {
    // SAFETY: as documented.
    unsafe {
        with_view(view, 0, |v| {
            let Some(t) = v.view.fx.textures.get(which as usize) else {
                return 0;
            };
            if let Some(info) = info.as_mut() {
                let wrap = t.wrap_modes();
                *info = SmsTexture {
                    width: t.width,
                    height: t.height,
                    wrap_s: wrap[0],
                    wrap_t: wrap[1],
                    _pad: [0; 2],
                };
            }
            if rgba.is_null() {
                return 1;
            }
            if t.rgba.len() > len as usize {
                return 0;
            }
            std::ptr::copy_nonoverlapping(t.rgba.as_ptr(), rgba, t.rgba.len());
            1
        })
    }
}

/// FLUDD's water gauge as of the last [`sms_view_update`]. Returns 0 (and leaves `out`) without
/// the HUD's files in the assets.
///
/// # Safety
/// `view` from [`sms_view_new`]; `out` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_hud_info(view: *mut SmsView, out: *mut SmsHudInfo) -> u8 {
    // SAFETY: as documented.
    unsafe {
        with_view(view, 0, |v| {
            let (Some(out), Some(hud)) = (out.as_mut(), v.view.hud.as_ref()) else {
                return 0;
            };
            *out = SmsHudInfo {
                x: hud.origin[0],
                y: hud.origin[1],
                width: hud.image.width,
                height: hud.image.height,
                screen_w: libsms_present::hud::SCREEN[0] as u32,
                screen_h: libsms_present::hud::SCREEN[1] as u32,
                version: hud.version,
                shown: v.view.hud_shown as u8,
                _pad: [0; 3],
            };
            1
        })
    }
}

/// Current nz01..04 nozzle pane within the combined HUD image, as [`SmsHudInfo`].
/// x/y are local top-down pixels; screen_w/h are the combined canvas dimensions.
/// Hosts map this through their tank's actual screen rectangle to anchor other UI.
///
/// # Safety
/// `view` from [`sms_view_new`]; `out` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_hud_nozzle_info(view: *mut SmsView, out: *mut SmsHudInfo) -> u8 {
    unsafe { with_view(view, 0, |v| {
        let (Some(hud), Some(out)) = (v.view.hud.as_ref(), out.as_mut()) else { return 0; };
        let [x,y,w,h] = hud.nozzle_rect();
        *out = SmsHudInfo { x,y,width:w as u32,height:h as u32,
            screen_w:hud.image.width, screen_h:hud.image.height, shown:1, ..Default::default() };
        1
    }) }
}

/// One HUD nozzle icon (`nz01..04`: 0 Spray, 1 Hover, 2 Rocket, 3 Turbo), as
/// [`SmsHudInfo`] (width/height, shown 1, x/y 0). Pixels: [`sms_view_nozzle_icon_pixels`].
/// # Safety
/// `view` from [`sms_view_new`]; `out` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_nozzle_icon_info(
    view: *mut SmsView,
    index: u32,
    out: *mut SmsHudInfo,
) -> u8 {
    // SAFETY: as documented.
    unsafe {
        with_view(view, 0, |v| {
            let (Some(out), Some(icon)) = (
                out.as_mut(),
                v.view.hud.as_ref().and_then(|h| h.nozzle_icon(index as usize)),
            ) else {
                return 0;
            };
            *out = SmsHudInfo {
                x: 0,
                y: 0,
                width: icon.width,
                height: icon.height,
                screen_w: libsms_present::hud::SCREEN[0] as u32,
                screen_h: libsms_present::hud::SCREEN[1] as u32,
                version: 0,
                shown: 1,
                _pad: [0; 3],
            };
            1
        })
    }
}

/// Copies nozzle icon `index`'s pixels (RGBA, straight alpha, top row first,
/// `width * height * 4` bytes) to `rgba`. Returns 0 if `len` is short or there is no icon.
///
/// # Safety
/// `view` from [`sms_view_new`]; `rgba` writable for `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_nozzle_icon_pixels(
    view: *mut SmsView,
    index: u32,
    rgba: *mut u8,
    len: u32,
) -> u8 {
    // SAFETY: as documented.
    unsafe {
        with_view(view, 0, |v| {
            let Some(icon) = v.view.hud.as_ref().and_then(|h| h.nozzle_icon(index as usize)) else {
                return 0;
            };
            if rgba.is_null() || icon.rgba.len() > len as usize {
                return 0;
            }
            std::ptr::copy_nonoverlapping(icon.rgba.as_ptr(), rgba, icon.rgba.len());
            1
        })
    }
}

/// Sets what Mario's life meter `which` shows (0 the HUD's, with the splash and "LIFE"; 1 the
/// sun alone with ten rays, for a lock-on marker) and redraws it if that changed the rays: a
/// share 0..1 of the meter lit in the game's colours for `health`, the rays after them in
/// cyan for `shield` (the two together are clamped to 1). Then read it with
/// [`sms_view_life_info`] and [`sms_view_life_pixels`]. Returns 0 without the HUD's files.
///
/// # Safety
/// `view` from [`sms_view_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_life_set(
    view: *mut SmsView,
    which: u8,
    health: f32,
    shield: f32,
) -> u8 {
    // SAFETY: as documented.
    unsafe {
        with_view(view, 0, |v| {
            let Some(m) = v.view.life.get_mut(which as usize).and_then(|m| m.as_mut()) else {
                return 0;
            };
            let share = |x: f32| {
                if x.is_finite() {
                    x.clamp(0.0, 1.0)
                } else {
                    0.0
                }
            };
            m.update(libsms_present::life::Life {
                health: share(health),
                shield: share(shield),
            });
            1
        })
    }
}

/// Life meter `which`'s place and size (the same struct as the water gauge's: its top-left on
/// the game's 600x480 screen, `shown` always 1 here). Returns 0 without the HUD's files.
///
/// # Safety
/// `view` from [`sms_view_new`]; `out` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_life_info(
    view: *mut SmsView,
    which: u8,
    out: *mut SmsHudInfo,
) -> u8 {
    // SAFETY: as documented.
    unsafe {
        with_view(view, 0, |v| {
            let (Some(out), Some(m)) = (
                out.as_mut(),
                v.view.life.get(which as usize).and_then(|m| m.as_ref()),
            ) else {
                return 0;
            };
            *out = SmsHudInfo {
                x: m.origin[0],
                y: m.origin[1],
                width: m.image.width,
                height: m.image.height,
                screen_w: libsms_present::hud::SCREEN[0] as u32,
                screen_h: libsms_present::hud::SCREEN[1] as u32,
                version: m.version,
                shown: 1,
                _pad: [0; 3],
            };
            1
        })
    }
}

/// Copies life meter `which`'s pixels (RGBA, straight alpha, top row first) to `rgba`. Returns 0
/// if `len` is short or there is no such meter.
///
/// # Safety
/// `view` from [`sms_view_new`]; `rgba` writable for `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_life_pixels(
    view: *mut SmsView,
    which: u8,
    rgba: *mut u8,
    len: u32,
) -> u8 {
    // SAFETY: as documented.
    unsafe {
        with_view(view, 0, |v| {
            let Some(m) = v.view.life.get(which as usize).and_then(|m| m.as_ref()) else {
                return 0;
            };
            let px = &m.image.rgba;
            if rgba.is_null() || px.len() > len as usize {
                return 0;
            }
            std::ptr::copy_nonoverlapping(px.as_ptr(), rgba, px.len());
            1
        })
    }
}

/// The losing screens' state ([`sms_view_screenfx_info`]).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SmsScreenFx {
    /// 0 nothing running, 1 "TOO BAD!", 2 GAME OVER, 3 the Shine Get's letters.
    pub kind: u32,
    /// 1 once the running one has played out.
    pub finished: u32,
    /// Changes whenever anything below does.
    pub version: u32,
    /// Black over the whole screen, alpha 0..1.
    pub fill: f32,
    /// Black past the overlay's edges (a wider screen than 4:3), alpha 0..1.
    pub edge: f32,
    pub sprites: u32,
    /// The overlay's pixels, over the middle 4:3 of the screen: size, 0 without one.
    pub overlay_w: u32,
    pub overlay_h: u32,
    /// The screen its sprites are placed on.
    pub screen_w: f32,
    pub screen_h: f32,
    /// A flat box behind the overlay (GAME OVER's red one): left, top, width, height on that screen,
    /// and its colour, alpha 0 when there is none.
    pub card: [f32; 4],
    pub card_color: [f32; 4],
}

/// One picture of a losing screen: which, where its middle is on `screen_w` by `screen_h`, its size
/// there, turned `rotation` radians clockwise about it, drawn in `color` (RGBA, times its pixels).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SmsScreenSprite {
    pub picture: u32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub rotation: f32,
    pub color: [f32; 4],
}

/// Starts screen `which` (0 "TOO BAD!", 1 GAME OVER, 2 the Shine Get's "SHINE!" letters) from its
/// first frame; it plays as the host steps it ([`sms_view_screenfx_step`]). Returns 0 without its
/// pictures (the disc's wipe pictures are an optional part of the assets).
///
/// # Safety
/// `view` from [`sms_view_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_screenfx_start(view: *mut SmsView, which: u8) -> u8 {
    // SAFETY: as documented.
    unsafe {
        with_view(view, 0, |v| {
            let kind = match which {
                0 => libsms_present::screenfx::Kind::TooBad,
                2 => libsms_present::screenfx::Kind::ShineGet,
                _ => libsms_present::screenfx::Kind::GameOver,
            };
            v.view.screenfx.as_mut().is_some_and(|f| f.start(kind)) as u8
        })
    }
}

/// Runs the losing screen `seconds` on (in whole game frames, 30 a second).
///
/// # Safety
/// `view` from [`sms_view_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_screenfx_step(view: *mut SmsView, seconds: f32) {
    // SAFETY: as documented.
    unsafe {
        with_view(view, (), |v| {
            if let Some(f) = v.view.screenfx.as_mut() {
                f.step(if seconds.is_finite() { seconds } else { 0.0 });
            }
        })
    }
}

/// Stops the losing screen.
///
/// # Safety
/// `view` from [`sms_view_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_screenfx_stop(view: *mut SmsView) {
    // SAFETY: as documented.
    unsafe {
        with_view(view, (), |v| {
            if let Some(f) = v.view.screenfx.as_mut() {
                f.stop();
            }
        })
    }
}

/// What the losing screen shows now. Returns 0 without the pictures.
///
/// # Safety
/// `view` from [`sms_view_new`]; `out` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_screenfx_info(view: *mut SmsView, out: *mut SmsScreenFx) -> u8 {
    // SAFETY: as documented.
    unsafe {
        with_view(view, 0, |v| {
            let (Some(out), Some(f)) = (out.as_mut(), v.view.screenfx.as_ref()) else {
                return 0;
            };
            let frame = f.frame();
            *out = SmsScreenFx {
                kind: match f.kind() {
                    None => 0,
                    Some(libsms_present::screenfx::Kind::TooBad) => 1,
                    Some(libsms_present::screenfx::Kind::GameOver) => 2,
                    Some(libsms_present::screenfx::Kind::ShineGet) => 3,
                },
                finished: f.finished() as u32,
                version: f.version,
                fill: frame.fill,
                edge: frame.edge,
                sprites: frame.sprites.len() as u32,
                overlay_w: frame.overlay.as_ref().map_or(0, |o| o.width),
                overlay_h: frame.overlay.as_ref().map_or(0, |o| o.height),
                screen_w: libsms_present::screenfx::SCREEN[0],
                screen_h: libsms_present::screenfx::SCREEN[1],
                card: frame.card.rect,
                card_color: frame.card.color,
            };
            1
        })
    }
}

/// Writes up to `max` of the losing screen's sprites, back to front; returns how many there are.
///
/// # Safety
/// `view` from [`sms_view_new`]; `out` null or writable for `max`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_screenfx_sprites(
    view: *mut SmsView,
    out: *mut SmsScreenSprite,
    max: u32,
) -> u32 {
    // SAFETY: as documented.
    unsafe {
        with_view(view, 0, |v| {
            let Some(f) = v.view.screenfx.as_ref() else {
                return 0;
            };
            let sprites = &f.frame().sprites;
            if !out.is_null() {
                for (i, s) in sprites.iter().take(max as usize).enumerate() {
                    out.add(i).write(SmsScreenSprite {
                        picture: s.picture,
                        x: s.center[0],
                        y: s.center[1],
                        w: s.size[0],
                        h: s.size[1],
                        rotation: s.rotation,
                        color: s.color,
                    });
                }
            }
            sprites.len() as u32
        })
    }
}

/// Picture `which` of the losing screens (a letter, the GAME OVER text): its size in `out`
/// (`width`, `height`) and, when `rgba` isn't null and `len` is enough, its pixels (RGBA,
/// straight alpha, top row first). Returns 0 for a picture there isn't.
///
/// # Safety
/// `view` from [`sms_view_new`]; `out` writable; `rgba` null or writable for `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_screenfx_picture(
    view: *mut SmsView,
    which: u32,
    out: *mut SmsHudInfo,
    rgba: *mut u8,
    len: u32,
) -> u8 {
    // SAFETY: as documented.
    unsafe {
        with_view(view, 0, |v| {
            let Some(img) = v.view.screenfx.as_ref().and_then(|f| f.picture(which)) else {
                return 0;
            };
            if let Some(out) = out.as_mut() {
                *out = SmsHudInfo {
                    width: img.width,
                    height: img.height,
                    version: 1,
                    ..SmsHudInfo::default()
                };
            }
            if !rgba.is_null() {
                if img.rgba.len() > len as usize {
                    return 0;
                }
                std::ptr::copy_nonoverlapping(img.rgba.as_ptr(), rgba, img.rgba.len());
            }
            1
        })
    }
}

/// Copies the losing screen's overlay (RGBA, black with alpha, top row first,
/// `overlay_w * overlay_h * 4` bytes) to `rgba`. Returns 0 if `len` is short or there is none.
///
/// # Safety
/// `view` from [`sms_view_new`]; `rgba` writable for `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_screenfx_overlay(
    view: *mut SmsView,
    rgba: *mut u8,
    len: u32,
) -> u8 {
    // SAFETY: as documented.
    unsafe {
        with_view(view, 0, |v| {
            let Some(o) = v.view.screenfx.as_ref().and_then(|f| f.frame().overlay.as_ref()) else {
                return 0;
            };
            if rgba.is_null() || o.rgba.len() > len as usize {
                return 0;
            }
            std::ptr::copy_nonoverlapping(o.rgba.as_ptr(), rgba, o.rgba.len());
            1
        })
    }
}

/// Sets the number counter `which` (0 the yellow coin's, 1 the blue coin's, 2 Mario's lives, two
/// digits with "MARIO" over them) shows, as the HUD draws it, and redraws it if that changed the
/// picture. A coin counter shows up to seven digits (the game shows three); more is clamped. Then read it with [`sms_view_coins_info`] and
/// [`sms_view_coins_pixels`]. Returns 0 without the HUD's files.
///
/// # Safety
/// `view` from [`sms_view_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_coins_set(view: *mut SmsView, which: u8, value: u32) -> u8 {
    // SAFETY: as documented.
    unsafe {
        with_view(view, 0, |v| {
            let Some(m) = v.view.coins.get_mut(which as usize).and_then(|m| m.as_mut()) else {
                return 0;
            };
            m.update(value);
            1
        })
    }
}

/// Coin counter `which`'s place and size (the same struct as the water gauge's: its top-left on
/// the game's 600x480 screen, `shown` always 1 here). Returns 0 without the HUD's files.
///
/// # Safety
/// `view` from [`sms_view_new`]; `out` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_coins_info(
    view: *mut SmsView,
    which: u8,
    out: *mut SmsHudInfo,
) -> u8 {
    // SAFETY: as documented.
    unsafe {
        with_view(view, 0, |v| {
            let (Some(out), Some(m)) = (
                out.as_mut(),
                v.view.coins.get(which as usize).and_then(|m| m.as_ref()),
            ) else {
                return 0;
            };
            *out = SmsHudInfo {
                x: m.origin[0],
                y: m.origin[1],
                width: m.image.width,
                height: m.image.height,
                screen_w: libsms_present::hud::SCREEN[0] as u32,
                screen_h: libsms_present::hud::SCREEN[1] as u32,
                version: m.version,
                shown: 1,
                _pad: [0; 3],
            };
            1
        })
    }
}

/// Copies coin counter `which`'s pixels (RGBA, straight alpha, top row first) to `rgba`.
/// Returns 0 if `len` is short or there is no such counter.
///
/// # Safety
/// `view` from [`sms_view_new`]; `rgba` writable for `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_coins_pixels(
    view: *mut SmsView,
    which: u8,
    rgba: *mut u8,
    len: u32,
) -> u8 {
    // SAFETY: as documented.
    unsafe {
        with_view(view, 0, |v| {
            let Some(m) = v.view.coins.get(which as usize).and_then(|m| m.as_ref()) else {
                return 0;
            };
            let px = &m.image.rgba;
            if rgba.is_null() || px.len() > len as usize {
                return 0;
            }
            std::ptr::copy_nonoverlapping(px.as_ptr(), rgba, px.len());
            1
        })
    }
}

/// Copies the gauge's pixels (RGBA, straight alpha, top row first, `width * height * 4`
/// bytes) to `rgba`. Returns 0 if `len` is short or there is no gauge.
///
/// # Safety
/// `view` from [`sms_view_new`]; `rgba` writable for `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_hud_pixels(view: *mut SmsView, rgba: *mut u8, len: u32) -> u8 {
    // SAFETY: as documented.
    unsafe {
        with_view(view, 0, |v| {
            let Some(hud) = v.view.hud.as_ref() else {
                return 0;
            };
            let px = &hud.image.rgba;
            if rgba.is_null() || px.len() > len as usize {
                return 0;
            }
            std::ptr::copy_nonoverlapping(px.as_ptr(), rgba, px.len());
            1
        })
    }
}

/// A prop view's clip (`sms_view_new_look` looks 5-7) by name, as the file's stem
/// (`flower_hit`, `sambo_fset`...): its index, or -1; `frames` gets its length (60 a second).
///
/// # Safety
/// `view` from [`sms_view_new_look`]; `name` nul-terminated; `frames` writable or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_prop_clip(
    view: *mut SmsView,
    name: *const c_char,
    frames: *mut f32,
) -> i32 {
    if name.is_null() {
        return -1;
    }
    // SAFETY: as documented.
    let name = unsafe { std::ffi::CStr::from_ptr(name) }.to_string_lossy().into_owned();
    // SAFETY: as documented.
    unsafe {
        with_view(view, -1, |v| match v.view.prop_clip(&name) {
            Some((i, f)) => {
                if let Some(out) = frames.as_mut() {
                    *out = f;
                }
                i as i32
            }
            None => -1,
        })
    }
}

/// Poses a prop view: clip `clip` (an index from [`sms_view_prop_clip`], negative for the bind
/// pose) at `frame`, turned `yaw` (a 16-bit angle); the host places its own instance. `mirror`
/// non-zero when the host draws it mirrored across X.
///
/// # Safety
/// `view` a prop view.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_prop_update(
    view: *mut SmsView,
    yaw: i16,
    clip: i32,
    frame: f32,
    mirror: u8,
) {
    // SAFETY: as documented.
    unsafe {
        with_view(view, (), |v| {
            v.view.update_prop(yaw, clip, if frame.is_finite() { frame.max(0.0) } else { 0.0 }, mirror != 0)
        })
    }
}

/// Starts one of the flower effects (`ms_smb_kafun_ow`, `ms_smb_ap_rock`...) at `pos` (sim units) on
/// an effects view (`sms_view_new_look` look 8), `scale` big. Unknown names do nothing.
///
/// # Safety
/// `view` an effects view; `name` nul-terminated; `pos` readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_effect(
    view: *mut SmsView,
    name: *const c_char,
    pos: *const SmsVec3,
    scale: f32,
) {
    // SAFETY: as documented.
    let (Some(p), false) = (unsafe { pos.as_ref() }, name.is_null()) else { return };
    let at = Vec3::from(*p);
    if !at.is_finite() {
        return;
    }
    // SAFETY: as documented.
    let name = unsafe { std::ffi::CStr::from_ptr(name) }.to_string_lossy().into_owned();
    // SAFETY: as documented.
    unsafe { with_view(view, (), |v| v.view.emit_effect(&name, at, if scale.is_finite() { scale.clamp(0.1, 8.0) } else { 1.0 })) }
}

/// Replays an accepted enemy-water contact on a remote Mario's view. Presentation only:
/// no collision query or damage message, and no second advance of the water/audio clock.
/// # Safety
/// `view` is live; `pos` is readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_water_hit(view: *mut SmsView, pos: *const SmsVec3) {
    let Some(pos) = (unsafe { pos.as_ref() }) else { return };
    let pos = Vec3::from(*pos);
    if !pos.is_finite() { return; }
    unsafe { with_view(view, (), |v| {
        v.view.emit_effect(libsms_present::water_hit::EFFECT, pos, 1.);
        if let Some(audio) = &mut v.audio { audio.water_hit(pos); }
    }) }
}

/// Runs an effects view `dt` seconds and builds its quads for the `sms_view_fx_*` readers, with the
/// camera's right and up for the billboards.
///
/// # Safety
/// `view` an effects view; `right` and `up` readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sms_view_effects_step(
    view: *mut SmsView,
    dt: f32,
    right: *const SmsVec3,
    up: *const SmsVec3,
) {
    // SAFETY: as documented.
    let (Some(r), Some(u)) = (unsafe { right.as_ref() }, unsafe { up.as_ref() }) else { return };
    // SAFETY: as documented.
    unsafe {
        with_view(view, (), |v| {
            v.view.set_camera(Vec3::from(*r), Vec3::from(*u));
            v.view.step_effects(if dt.is_finite() { dt.clamp(0.0, 0.25) } else { 0.0 });
        })
    }
}
