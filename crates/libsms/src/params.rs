//! Tuning values. Same story as Pikmin: the `TParamRT` defaults in the decomp constructors
//! (MarioInit.cpp, WaterGun.hpp, NozzleBase.hpp) are overridden by the `.prm` files on the
//! disc (`params.szs:mario/*.prm`), and most differ (gravity 0.8 in code vs 1.0 on disc,
//! running max 45 vs 80...). `Default` here is the code default; [`Params::from_files`]
//! layers the disc values on top.

use crate::prm::Prm;

/// One parameter value type readable from a `.prm`.
trait Field: Sized + Copy {
    fn read(prm: &Prm, key: &str) -> Option<Self>;
}

impl Field for f32 {
    fn read(prm: &Prm, key: &str) -> Option<f32> {
        prm.f32(key)
    }
}
impl Field for i32 {
    fn read(prm: &Prm, key: &str) -> Option<i32> {
        prm.i32(key)
    }
}
impl Field for i16 {
    fn read(prm: &Prm, key: &str) -> Option<i16> {
        prm.i16(key)
    }
}
impl Field for u8 {
    fn read(prm: &Prm, key: &str) -> Option<u8> {
        prm.u8(key)
    }
}

/// Declares a parameter struct: `field: type = code_default, "mKeyInPrm"` (extra keys after
/// the first are accepted aliases, for names the decomp got wrong).
macro_rules! params {
    ($(#[$m:meta])* $name:ident { $($field:ident : $ty:ty = $def:expr, $($key:literal),+ ;)* }) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq)]
        pub struct $name { $(pub $field: $ty,)* }

        impl Default for $name {
            fn default() -> Self { Self { $($field: $def,)* } }
        }

        impl $name {
            /// Overlays every key present in `prm`.
            pub fn apply(&mut self, prm: &Prm) {
                $(
                    for key in [$($key),+] {
                        if let Some(v) = <$ty as Field>::read(prm, key) {
                            self.$field = v;
                            break;
                        }
                    }
                )*
            }
        }
    };
}

params! {
    /// `/Mario/Run.prm` (TRunParams).
    Run {
        max_speed: f32 = 32.0, "mMaxSpeed";
        vel_minus_brake: f32 = 1.1, "mVelMinusBrake";
        add_base: f32 = 1.1, "mAddBase";
        add_vel_div: f32 = 0.023255814, "mAddVelDiv";
        dec_start_nrm_y: f32 = 0.95, "mDecStartNrmY";
        dec_brake: f32 = 1.0, "mDecBrake";
        soft_to_walk: f32 = 8.0, "mSoft2Walk";
        walk_to_soft: f32 = 5.0, "mWalk2Soft";
        swim_depth: f32 = 120.0, "mSwimDepth";
        in_water_brake: f32 = 0.9, "mInWaterBrake";
        do_jump_catch_sp: f32 = 15.0, "mDoJumpCatchSp";
        turn_need_sp: f32 = 10.0, "mTurnNeedSp";
        pumping_slide_sp: f32 = 0.1, "mPumpingSlideSp";
        dash_rot_sp: i16 = 100, "mDashRotSp";
    }
}

params! {
    /// `/Mario/Jump.prm` (TJumpParams).
    Jump {
        gravity: f32 = 0.8, "mGravity";
        spin_jump_gravity: f32 = 0.6, "mSpinJumpGravity";
        jumping_max: f32 = 80.0, "mJumpingMax";
        jump_speed_brake: f32 = 0.999, "mJumpSpeedBrake";
        jump_accel_control: f32 = 0.5, "mJumpAccelControl";
        jump_slide_control: f32 = 0.5, "mJumpSlideControl";
        turn_jump_force: f32 = 62.0, "mTurnJumpForce";
        broad_jump_force: f32 = 80.0, "mBroadJumpForce";
        broad_jump_force_y: f32 = 30.0, "mBroadJumpForceY";
        rotate_jump_force_y: f32 = 62.0, "mRotateJumpForceY";
        back_jump_force: f32 = -16.0, "mBackJumpForce";
        back_jump_force_y: f32 = 62.0, "mBackJumpForceY";
        hip_attack_speed_y: f32 = -50.0, "mHipAttackSpeedY";
        super_hip_attack_speed_y: f32 = -80.0, "mSuperHipAttackSpeedY";
        rot_broad_jump_force: f32 = 60.0, "mRotBroadJumpForce";
        rot_broad_jump_force_y: f32 = 20.0, "mRotBroadJumpForceY";
        sec_jump_enable_sp: f32 = 20.0, "mSecJumpEnableSp";
        sec_jump_force: f32 = 52.0, "mSecJumpForce";
        sec_jump_speed_mult: f32 = 0.0, "mSecJumpSpeedMult";
        sec_jump_xz_mult: f32 = 0.8, "mSecJumpXZMult";
        tri_jump_enable_sp: f32 = 20.0, "mTriJumpEnableSp";
        ultra_jump_force: f32 = 70.0, "mUltraJumpForce";
        ultra_jump_speed_mult: f32 = 0.25, "mUltraJumpSpeedMult";
        ultra_jump_xz_mult: f32 = 0.8, "mUltraJumpXZMult";
        clash_angle: i16 = 0x3555, "mClashAngle";
        jump_jump_catch_sp: f32 = 50.0, "mJumpJumpCatchSp";
        super_hip_attack_ct: i16 = 0x32, "mSuperHipAttackCt";
        trampoline_dec: f32 = 1.0, "mTrampolineDec";
    }
}

params! {
    /// `/Mario/Mario.prm` (TDeParams): Mario's general parameters.
    Mario {
        running_max: f32 = 45.0, "mRunningMax";
        dash_max: f32 = 60.0, "mDashMax";
        dash_acc: f32 = 0.5, "mDashAcc";
        dash_brake: f32 = 0.8, "mDashBrake";
        dash_start_time: i16 = 0x78, "mDashStartTime";
        waiting_rot_sp: i16 = 0x100, "mWaitingRotSp";
        running_rot_sp_min: i16 = 0x200, "mRunningRotSpMin";
        running_rot_sp_max: i16 = 0x400, "mRunningRotSpMax";
        pumping_rot_sp_min: i16 = 0x100, "mPumpingRotSpMin";
        pumping_rot_sp_max: i16 = 0x200, "mPumpingRotSpMax";
        trample_radius: f32 = 100.0, "mTrampleRadius";
        pushup_radius: f32 = 100.0, "mPushupRadius";
        pushup_height: f32 = 200.0, "mPushupHeight";
        hipdrop_radius: f32 = 250.0, "mHipdropRadius";
        attack_height: f32 = 50.0, "mAttackHeight";
        trample_pow_step1: f32 = 30.0, "mTramplePowStep1";
        trample_pow_step2: f32 = 40.0, "mTramplePowStep2";
        trample_pow_step3: f32 = 50.0, "mTramplePowStep3";
        jump_wall_height: f32 = 1000.0, "mJumpWallHeight";
        slip_start: f32 = 0.34202015, "mSlipStart";
        force_slip_angle: f32 = 0.5, "mForceSlipAngle";
        clash_speed: f32 = 50.0, "mClashSpeed";
        damage_fall_height: f32 = 1000.0, "mDamageFallHeight";
        feel_deep: f32 = 500.0, "mFeelDeep";
        hang_wall_movable_angle: f32 = 0.3, "mHangWallMovableAngle";
    }
}

params! {
    /// One of the `/Mario/SlipParam*.prm` sets (TSlipParams).
    Slip {
        slip_friction: f32 = 0.9, "mSlipFriction";
        slope_accele_up: f32 = 0.0, "mSlopeAcceleUp";
        slope_accele_down: f32 = 0.0, "mSlopeAcceleDown";
        slide_accele_up: f32 = 0.0, "mSlideAcceleUp";
        slide_accele_down: f32 = 0.0, "mSlideAcceleDown";
        slide_stop_normal: f32 = 15.0, "mSlideStopNormal";
        slide_stop_catch: f32 = 15.0, "mSlideStopCatch";
        jump_enable: u8 = 1, "mJumpEnable";
        miss_jump: u8 = 1, "mMissJump";
        slide_angle_y_sp: i16 = 0x200, "mSlideAngleYSp";
        stick_slide_mult: f32 = 0.05, "mStickSlideMult";
    }
}

params! {
    /// `/Mario/Controller.prm` (TControllerParams).
    Controller {
        start_to_walk_level: f32 = 15.0, "mStartToWalkLevel";
        stick_rotate_time: i16 = 0x18, "mStickRotateTime";
        length_mult_times: i16 = 10, "mLengthMultTimes";
        length_mult: f32 = 0.935, "mLengthMult";
        squat_rot_mid_analog: f32 = 0.7, "mSquatRotMidAnalog";
        squat_rot_mid_value: f32 = 0.05, "mSquatRotMidValue";
        analog_lr_to_zero_val: u8 = 0x1e, "mAnalogLRToZeroVal";
        analog_lr_to_middle_val: u8 = 0x5a, "mAnalogLRToMiddleVal";
        analog_lr_to_max_val: u8 = 0x96, "mAnalogLRToMaxVal";
        analog_lr_middle_level: f32 = 0.1, "mAnalogLRMiddleLevel";
    }
}

params! {
    /// `/Mario/UpperBody.prm` (TUpperBodyParams): the pumping stance.
    UpperBody {
        pump_wait_time: i16 = 10, "mPumpWaitTime";
        pump_anm_speed: f32 = 0.01, "mPumpAnmSpeed";
        hover_head_angle: i16 = -0x2000, "mHoverHeadAngle";
        feel_deep_head_angle: i16 = 0x2000, "mFeelDeepHeadAngle";
        front_wall_head_angle: i16 = -0x2000, "mFrontWallHeadAngle";
    }
}

params! {
    /// `/Mario/Hanging.prm` (THangingParams): hanging from a ledge.
    Hanging {
        move_sp: f32 = 0.1, "mMoveSp";
        anm_rate: f32 = 0.5, "mAnmRate";
        rapid_time: i16 = 2000, "mRapidTime";
        limit_time: i16 = 2400, "mLimitTime";
        anm_rapid: f32 = 8.0, "mAnmRapid";
        descent_sp: f32 = 10.0, "mDescentSp";
    }
}

params! {
    /// `/Mario/HHover.prm` (THHoverParams): hover-nozzle steering.
    Hover {
        rot_sp: i16 = 0x80, "mRotSp";
        accel_rate: f32 = 0.03, "mAccelRate";
        brake: f32 = 0.95, "mBrake";
    }
}

params! {
    /// `/Mario/Diving.prm` (TDivingParams): the underwater dive.
    Diving {
        rot_sp: i16 = 0x80, "mRotSp";
        gravity: f32 = 0.5, "mGravity";
        accel_control: f32 = 0.02, "mAccelControl";
        sea_brake: f32 = 0.999, "mSeaBrake";
        sea_brake_y: f32 = 0.98, "mSeaBrakeY";
    }
}

params! {
    /// `/Mario/WaterGun.prm` (TWaterGunParams).
    WaterGun {
        rocket_height: f32 = 1500.0, "mRocketHeight";
        hover_height: f32 = 160.0, "mHoverHeight", "mHHoverHeight";
        nozzle_angle_y_speed: f32 = 1.0, "mNozzleAngleYSpeed";
        nozzle_angle_y_brake: f32 = 0.995, "mNozzleAngleYBrake";
        nozzle_angle_y_speed_max: i16 = 0x2000, "mNozzleAngleYSpeedMax";
        hover_rot_max: i16 = 0x2000, "mHoverRotMax";
        hover_smooth: f32 = 0.05, "mHoverSmooth";
        change_speed: f32 = 0.1, "mChangeSpeed";
    }
}

params! {
    /// One `/Mario/WaterGun/Nozzle*.prm` (TNozzleBase::TEmitParams). Only the fields the
    /// sim reads are listed; the rest are visuals and sounds.
    Nozzle {
        rocket_type: u8 = 0, "mRocketType";
        num: f32 = 1.0, "mNum";
        attack: i16 = 1, "mAttack";
        emit_pow: f32 = 40.0, "mEmitPow";
        emit_ctrl: f32 = 1.0, "mEmitCtrl";
        amount_max: i32 = 0x834, "mAmountMax";
        reaction_pow: f32 = 0.0, "mReactionPow";
        reaction_y: f32 = 0.0, "mReactionY";
        dec_rate: i16 = 0, "mDecRate";
        trigger_rate: i16 = 0x100, "mTriggerRate";
        damage_loss: i32 = 0xfa, "mDamageLoss";
        suck_rate: f32 = 0.1, "mSuckRate";
        hit_radius: f32 = 50.0, "mHitRadius";
        hit_height: f32 = 80.0, "mHitHeight";
        num_min: f32 = 1.0, "mNumMin";
        attack_min: i16 = 1, "mAttackMin";
        emit_pow_min: f32 = 40.0, "mEmitPowMin";
        reaction_pow_min: f32 = 0.0, "mReactionPowMin";
        inside_pressure_dec: f32 = 100.0, "mInsidePressureDec";
        inside_pressure_max: f32 = 4500.0, "mInsidePressureMax";
        trigger_time: i16 = 1, "mTriggerTime";
        side_angle_max_side: i16 = 0x4000, "mSideAngleMaxSide";
        side_angle_max_front: i16 = 0x4000, "mSideAngleMaxFront";
        side_angle_max_back: i16 = 0x2000, "mSideAngleMaxBack";
        l_angle_base: i16 = 0x1000, "mLAngleBase";
        l_angle_min: i16 = -0x2000, "mLAngleMin";
        l_angle_max: i16 = 0x2000, "mLAngleMax";
        l_angle_chase: f32 = 0.1, "mLAngleChase";
        dir_tremble: f32 = 0.01, "mDirTremble";
        pow_tremble: f32 = 1.0, "mPowTremble";
        size: f32 = 40.0, "mSize";
        size_tremble: f32 = 16.0, "mSizeTremble";
        size_min_pressure: f32 = 0.0, "mSizeMinPressure";
        size_max_pressure: f32 = 1.0, "mSizeMaxPressure";
        dir_tremble_min: f32 = 0.01, "mDirTrembleMin";
        size_min: f32 = 40.0, "mSizeMin";
        particle_type: i16 = 0, "mType";
        r_button_mult: f32 = 10000.0, "mRButtonMult";
        emit_pow_scale: f32 = 10.0, "mEmitPowScale";
    }
}

params! {
    /// One `/Mario/WTPnn_*.prm` (TWaterParticleType, ModelWaterManager.cpp): how a kind of
    /// FLUDD water particle falls, spreads and is drawn.
    WaterParticle {
        extension: f32 = 0.5, "mExtension";
        magnify: f32 = 0.3, "mMagnify";
        clean_size: f32 = 12.0, "mCleanSize";
        gravity: f32 = -0.4, "mGravity";
        alive: f32 = 255.0, "mAlive";
    }
}

/// FLUDD nozzle slots, in the decomp's `TNozzleType` order (WaterGun.hpp).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum NozzleKind {
    #[default]
    Spray = 0,
    Rocket = 1,
    Underwater = 2,
    Yoshi = 3,
    Hover = 4,
    Turbo = 5,
}

impl NozzleKind {
    pub const ALL: [NozzleKind; 6] = [
        NozzleKind::Spray,
        NozzleKind::Rocket,
        NozzleKind::Underwater,
        NozzleKind::Yoshi,
        NozzleKind::Hover,
        NozzleKind::Turbo,
    ];

    /// `getNozzleKind() == 1`: Rocket, Hover and Turbo charge a trigger (TNozzleTrigger);
    /// Spray, Underwater and Yoshi are continuous (TNozzleBase/Deform).
    pub fn is_trigger(self) -> bool {
        matches!(
            self,
            NozzleKind::Rocket | NozzleKind::Hover | NozzleKind::Turbo
        )
    }
}

/// All the sim's tuning in one place.
///
/// `Default` is the decomp's code defaults with one concession: plain ground does not make every
/// jump a "miss jump" (the generic `TSlipParams` default says it does; the shipped
/// `SlipParamNormal.prm` turns it off). Real play needs the disc's values, see [`Params::from_files`].
#[derive(Debug, Clone, PartialEq)]
pub struct Params {
    pub run: Run,
    pub jump: Jump,
    pub mario: Mario,
    pub controller: Controller,
    pub upper_body: UpperBody,
    pub hanging: Hanging,
    pub hover: Hover,
    pub diving: Diving,
    pub water_gun: WaterGun,
    pub slip_normal: Slip,
    pub slip_all: Slip,
    pub slip_all_slider: Slip,
    pub slip_45: Slip,
    pub slip_water_ground: Slip,
    pub slip_water_slope: Slip,
    pub nozzles: [Nozzle; 6],
    /// By `mType` (`prmNames`, ModelWaterManager.cpp).
    pub water_particles: [WaterParticle; 17],
}

impl Default for Params {
    fn default() -> Self {
        let slip = Slip {
            miss_jump: 0,
            ..Slip::default()
        };
        Params {
            run: Run::default(),
            jump: Jump::default(),
            mario: Mario::default(),
            controller: Controller::default(),
            upper_body: UpperBody::default(),
            hanging: Hanging::default(),
            hover: Hover::default(),
            diving: Diving::default(),
            water_gun: WaterGun::default(),
            slip_normal: slip.clone(),
            slip_all: slip.clone(),
            slip_all_slider: slip.clone(),
            slip_45: slip.clone(),
            slip_water_ground: slip.clone(),
            slip_water_slope: slip,
            nozzles: Default::default(),
            water_particles: Default::default(),
        }
    }
}

/// `.prm` paths relative to the extracted `params.szs:mario/` folder, lower-case.
mod files {
    pub const RUN: &str = "run.prm";
    pub const JUMP: &str = "jump.prm";
    pub const MARIO: &str = "mario.prm";
    pub const CONTROLLER: &str = "controller.prm";
    pub const UPPER_BODY: &str = "upperbody.prm";
    pub const HANGING: &str = "hanging.prm";
    pub const HOVER: &str = "hhover.prm";
    pub const DIVING: &str = "diving.prm";
    pub const WATER_GUN: &str = "watergun.prm";
    pub const SLIP_NORMAL: &str = "slipparamnormal.prm";
    pub const SLIP_ALL: &str = "slipparamall.prm";
    pub const SLIP_ALL_SLIDER: &str = "slipparamall_slider.prm";
    pub const SLIP_45: &str = "slipparam45.prm";
    pub const SLIP_WATER_GROUND: &str = "slipparamwaterground.prm";
    pub const SLIP_WATER_SLOPE: &str = "slipparamwaterslope.prm";
    /// Indexed by `NozzleKind`. Spray uses NozzleDeform (WaterGun.cpp constructor).
    pub const NOZZLES: [&str; 6] = [
        "watergun/nozzledeform.prm",
        "watergun/nozzletrgrocket.prm",
        "watergun/nozzlediving.prm",
        "watergun/nozzleyoshimouth.prm",
        "watergun/nozzletrghover.prm",
        "watergun/nozzletrgturbo.prm",
    ];
    pub const WATER_PARTICLES: [&str; 17] = [
        "wtp00_normal.prm",
        "wtp01_wide.prm",
        "wtp02_bomb.prm",
        "wtp03_trgrocket.prm",
        "wtp04_hhover.prm",
        "wtp05_sinper.prm",
        "wtp06_yoshi.prm",
        "wtp07_diving.prm",
        "wtp08_bubble.prm",
        "wtp09_parasol.prm",
        "wtp10_back.prm",
        "wtp11_sprinkler.prm",
        "wtp12_yoshimouth.prm",
        "wtp13_turbo.prm",
        "wtp14_rocket.prm",
        "wtp15_hover.prm",
        "wtp16_sprocket.prm",
    ];
}

impl Params {
    /// Builds the tuning from `.prm` files. `read` returns the bytes of a path relative to the
    /// extracted `params.szs:mario/` folder, or `None` if absent (code defaults stay).
    /// Returns the params and the number of files that were found and parsed.
    pub fn from_files(read: impl Fn(&str) -> Option<Vec<u8>>) -> (Params, usize) {
        let mut p = Params::default();
        let mut loaded = 0;
        let mut get = |path: &str| -> Option<Prm> {
            let prm = read(path).and_then(|b| Prm::parse(&b).ok())?;
            loaded += 1;
            Some(prm)
        };
        if let Some(prm) = get(files::RUN) {
            p.run.apply(&prm);
        }
        if let Some(prm) = get(files::JUMP) {
            p.jump.apply(&prm);
        }
        if let Some(prm) = get(files::MARIO) {
            p.mario.apply(&prm);
        }
        if let Some(prm) = get(files::CONTROLLER) {
            p.controller.apply(&prm);
        }
        if let Some(prm) = get(files::UPPER_BODY) {
            p.upper_body.apply(&prm);
        }
        if let Some(prm) = get(files::HANGING) {
            p.hanging.apply(&prm);
        }
        if let Some(prm) = get(files::HOVER) {
            p.hover.apply(&prm);
        }
        if let Some(prm) = get(files::DIVING) {
            p.diving.apply(&prm);
        }
        if let Some(prm) = get(files::WATER_GUN) {
            p.water_gun.apply(&prm);
        }
        if let Some(prm) = get(files::SLIP_NORMAL) {
            p.slip_normal.apply(&prm);
        }
        // Other surface kinds fall back to the normal set if their file is missing, which is
        // closer to the real game than the (unrelated) code defaults.
        p.slip_all = p.slip_normal.clone();
        p.slip_all_slider = p.slip_normal.clone();
        p.slip_45 = p.slip_normal.clone();
        p.slip_water_ground = p.slip_normal.clone();
        p.slip_water_slope = p.slip_normal.clone();
        if let Some(prm) = get(files::SLIP_ALL) {
            p.slip_all.apply(&prm);
        }
        if let Some(prm) = get(files::SLIP_ALL_SLIDER) {
            p.slip_all_slider.apply(&prm);
        }
        if let Some(prm) = get(files::SLIP_45) {
            p.slip_45.apply(&prm);
        }
        if let Some(prm) = get(files::SLIP_WATER_GROUND) {
            p.slip_water_ground.apply(&prm);
        }
        if let Some(prm) = get(files::SLIP_WATER_SLOPE) {
            p.slip_water_slope.apply(&prm);
        }
        for (i, path) in files::NOZZLES.iter().enumerate() {
            if let Some(prm) = get(path) {
                p.nozzles[i].apply(&prm);
            }
        }
        for (i, path) in files::WATER_PARTICLES.iter().enumerate() {
            if let Some(prm) = get(path) {
                p.water_particles[i].apply(&prm);
            }
        }
        (p, loaded)
    }

    /// Convenience: read from a directory laid out like the extracted `params.szs:mario/`.
    pub fn from_dir(dir: &std::path::Path) -> (Params, usize) {
        Params::from_files(|rel| std::fs::read(dir.join(rel)).ok())
    }

    pub fn nozzle(&self, kind: NozzleKind) -> &Nozzle {
        &self.nozzles[kind as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prm(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut d = (entries.len() as u32).to_be_bytes().to_vec();
        for (name, v) in entries {
            d.extend_from_slice(&0u16.to_be_bytes());
            d.extend_from_slice(&(name.len() as u16).to_be_bytes());
            d.extend_from_slice(name.as_bytes());
            d.extend_from_slice(&(v.len() as u32).to_be_bytes());
            d.extend_from_slice(v);
        }
        d
    }

    #[test]
    fn disc_values_override_defaults() {
        let jump = prm(&[("mGravity", 1.0f32.to_be_bytes().to_vec())]);
        let (p, n) = Params::from_files(|path| (path == "jump.prm").then(|| jump.clone()));
        assert_eq!(n, 1);
        assert_eq!(p.jump.gravity, 1.0);
        // untouched fields keep their code default
        assert_eq!(p.jump.jumping_max, 80.0);
    }

    #[test]
    fn alias_key_is_accepted() {
        let wg = prm(&[("mHHoverHeight", 230.0f32.to_be_bytes().to_vec())]);
        let (p, _) = Params::from_files(|path| (path == "watergun.prm").then(|| wg.clone()));
        assert_eq!(p.water_gun.hover_height, 230.0);
    }

    #[test]
    fn missing_slip_sets_inherit_normal() {
        let normal = prm(&[("mSlipFriction", 0.97f32.to_be_bytes().to_vec())]);
        let (p, _) =
            Params::from_files(|path| (path == "slipparamnormal.prm").then(|| normal.clone()));
        assert_eq!(p.slip_all.slip_friction, 0.97);
    }
}
