//! Tuning values. Same story as Pikmin: the `TParamRT` defaults in the decomp constructors
//! (MarioInit.cpp, WaterGun.hpp, NozzleBase.hpp) are overridden by the `.prm` files on the
//! disc (`params.szs:mario/*.prm`), and most differ (gravity 0.8 in code vs 1.0 on disc,
//! running max 45 vs 80...). `Default` here is the code default; [`Params::from_files`]
//! layers the disc values on top.

use crate::damage::DamageKind;
use crate::prm::Prm;
use glam::Vec3;

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

impl Field for Vec3 {
    fn read(prm: &Prm, key: &str) -> Option<Vec3> { prm.vec3(key) }
}

/// Declares a parameter struct: `field: type = code_default, "mKeyInPrm"` (extra keys after
/// the first are accepted aliases, for names the decomp got wrong).
macro_rules! params {
    ($(#[$m:meta])* $name:ident { $($(#[$fm:meta])* $field:ident : $ty:ty = $def:expr, $($key:literal),+ ;)* }) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq)]
        pub struct $name { $($(#[$fm])* pub $field: $ty,)* }

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
        fire_down_force: f32 = 80.0, "mFireDownForce";
        fire_down_control: f32 = 1.0, "mFireDownControl";
        fire_back_velocity: f32 = 1.0, "mFireBackVelocity";
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
        get_off_yoshi_y: f32 = 30.0, "mGetOffYoshiY";
    }
}

params! {
    /// `/Mario/WaterEffect.prm` (TWaterEffectParams): the splashes and ripples of water.
    WaterEffect {
        /// A run through water whose surface is within this of his feet ripples at each step.
        running_ripple_depth: f32 = 30.0, "mRunningRippleDepth";
    }
}

params! {
    /// `/Mario/Yoshi.prm` (TYoshiParams): riding Yoshi.
    YoshiParams {
        run_yoshi_mult: f32 = 1.2, "mRunYoshiMult";
        jump_yoshi_mult: f32 = 1.0, "mJumpYoshiMult";
        rot_yoshi_mult: f32 = 1.5, "mRotYoshiMult";
        head_front: f32 = 80.0, "mHeadFront";
        head_radius: f32 = 50.0, "mHeadRadius";
        hold_out_acc_ctrl_f: f32 = 0.01, "mHoldOutAccCtrlF";
        hold_out_acc_ctrl_b: f32 = 0.023, "mHoldOutAccCtrlB";
        hold_out_sld_ctrl: f32 = 0.3, "mHoldOutSldCtrl";
        dec_brake: f32 = 1.0, "mDecBrake";
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
        was_on_water_slip: f32 = 0.97, "mWasOnWaterSlip";
        in_water_slip: f32 = 0.9, "mInWaterSlip";
        force_slip_angle: f32 = 0.5, "mForceSlipAngle";
        /// Ticks on electric goop before it shocks (`checkGraffitoElec`).
        graffito_no_dmg_time: u8 = 10, "mGraffitoNoDmgTime";
        clash_speed: f32 = 50.0, "mClashSpeed";
        damage_fall_height: f32 = 1000.0, "mDamageFallHeight";
        feel_deep: f32 = 500.0, "mFeelDeep";
        hang_wall_movable_angle: f32 = 0.3, "mHangWallMovableAngle";
        hold_radius: f32 = 100.0, "mHoldRadius";
        throw_power: f32 = 2.0, "mThrowPower";
        hp_max: i16 = 8, "mHPMax";
    }
}

params! {
    /// One `/Mario/Dmg*.prm` (`TMario::TEParams`, MarioInit.cpp): what a kind of hit does to
    /// Mario (`damageExec`).
    Damage {
        damage: u8 = 1, "mDamage";
        /// 0 a stumble (`SAFE_*_DOWN`), 1 a short fall, 2 a full fall, 3 no knock-back.
        down_type: u8 = 1, "mDownType";
        /// Bursts of FLUDD's water lost (`TWaterGun::damage`).
        water_emit: u8 = 0, "mWaterEmit";
        /// Rumble length.
        motor: u8 = 25, "mMotor";
        /// Knock-back speed at least this.
        min_speed: f32 = 16.0, "mMinSpeed";
        dirty: f32 = 0.0, "mDirty";
        /// Ticks Mario can't be hurt again, flashing pink (`addDamageFog`).
        invincible_time: i16 = 300, "mInvincibleTime";
    }
}

params! {
    /// `/Mario/Dirty.prm` (`TMario::TDirtyParams`, MarioInit.cpp): how goop builds up on Mario
    /// (`thinkDirty`) and washes off.
    Dirty {
        inc_running: f32 = 0.1, "mIncRunning";
        inc_catching: f32 = 0.3, "mIncCatching";
        inc_slipping: f32 = 0.2, "mIncSlipping";
        dec_swimming: f32 = 0.5, "mDecSwimming";
        dec_water_hit: f32 = 0.2, "mDecWaterHit";
        dec_rot_jump: f32 = 0.1, "mDecRotJump";
        brake_start_val_slip: f32 = 0.99, "mBrakeStartValSlip";
        brake_start_val_run: f32 = 0.98, "mBrakeStartValRun";
        dirty_time_slip: i16 = 600, "mDirtyTimeSlip";
        dirty_time_run: i16 = 600, "mDirtyTimeRun";
        pol_size_slip: f32 = 200.0, "mPolSizeSlip";
        pol_size_run: f32 = 80.0, "mPolSizeRun";
        pol_size_foot_print: f32 = 200.0, "mPolSizeFootPrint";
        pol_size_jump: f32 = 200.0, "mPolSizeJump";
        slope_angle: f32 = 0.99, "mSlopeAngle";
        /// The most goop Mario carries; also the TEV konst alpha his materials show it through.
        dirty_max: f32 = 200.0, "mDirtyMax";
        slip_anm_speed: f32 = 3.0, "mSlipAnmSpeed";
        slip_run_sp: f32 = 0.01, "mSlipRunSp";
        slip_catch_sp: f32 = 0.01, "mSlipCatchSp";
        slip_rotate: i16 = 0x100, "mSlipRotate";
        slip_catch_rotate: i16 = 0x100, "mSlipCatchRotate";
        brake_slip_no_pollute: f32 = 0.98, "mBrakeSlipNoPollute";
        /// Oil: a floor hit every `fog_time_yellow + fog_time_red` ticks on it.
        fog_time_yellow: i16 = 240, "mFogTimeYellow";
        fog_time_red: i16 = 600, "mFogTimeRed";
    }
}

params! {
    /// `/Mario/Graffito.prm` (`TMario::TGraffitoParams`, MarioInit.cpp): what goop of each kind
    /// does to Mario; the sink fields drive `checkSink`.
    Graffito {
        /// Ticks of standing in brown goop to sink all the way.
        sink_time: i16 = 240, "mSinkTime";
        /// Floor damage every this many ticks while sinking.
        sink_dmg_time: i16 = 240, "mSinkDmgTime";
        /// How far the body is drawn down when fully sunk.
        sink_height: f32 = 150.0, "mSinkHeight";
        sink_move_min: f32 = 0.3, "mSinkMoveMin";
        sink_move_max: f32 = 0.5, "mSinkMoveMax";
        sink_recover: f32 = 0.05, "mSinkRecover";
        sink_jump_rate_min: f32 = 0.1, "mSinkJumpRateMin";
        sink_jump_rate_max: f32 = 0.3, "mSinkJumpRateMax";
        sink_pump_limit: f32 = 0.25, "mSinkPumpLimit";
        /// While Mario has health he stops sinking at this share of `sink_time`.
        sink_dmg_depth: f32 = 0.25, "mSinkDmgDepth";
        /// Burning goop catches him only this close over the floor.
        fire_height: f32 = 1000.0, "mFireHeight";
        fire_invincible_time: i16 = 0x96, "mFireInvincibleTime";
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

params! {
    /// `TWaterEmitInfo` (ModelWaterManager.cpp); loaded from `/Mario/WetWaterEmit.prm`.
    WetWaterEmit {
        count: i32 = 1, "mNum";
        alive: i16 = 255, "mAlive";
        attack: i16 = 1, "mAttack";
        dir: Vec3 = Vec3::Y, "mDir";
        dir_tremble: f32 = 0.0, "mDirTremble";
        power: f32 = 0.0, "mPow";
        pow_tremble: f32 = 0.0, "mPowTremble";
        size: f32 = 17.0, "mSize";
        size_tremble: f32 = 0.0, "mSizeTremble";
        hit_radius: f32 = 50.0, "mHitRadius";
        hit_height: f32 = 80.0, "mHitHeight";
        particle_type: i16 = 0, "mType";
    }
}

/// All the sim's tuning in one place.
///
/// `Default` is the decomp's code defaults with one concession: plain ground does not make every
/// jump a "miss jump" (the generic `TSlipParams` default says it does; the shipped
/// `SlipParamNormal.prm` turns it off). Real play needs the disc's values, see [`Params::from_files`].
#[derive(Debug, Clone, PartialEq)]
pub struct Params {
    pub wet_emit: WetWaterEmit,
    pub run: Run,
    pub jump: Jump,
    pub mario: Mario,
    pub controller: Controller,
    pub water_effect: WaterEffect,
    pub upper_body: UpperBody,
    pub hanging: Hanging,
    pub hover: Hover,
    pub diving: Diving,
    pub water_gun: WaterGun,
    pub yoshi: YoshiParams,
    pub slip_normal: Slip,
    pub slip_all: Slip,
    pub slip_all_slider: Slip,
    pub slip_45: Slip,
    pub slip_water_ground: Slip,
    pub slip_water_slope: Slip,
    pub slip_yoshi: Slip,
    pub nozzles: [Nozzle; 6],
    /// By `mType` (`prmNames`, ModelWaterManager.cpp).
    pub water_particles: [WaterParticle; 17],
    /// By [`crate::DamageKind`].
    pub damage: [Damage; crate::damage::DamageKind::ALL.len()],
    pub dirty: Dirty,
    pub graffito: Graffito,
}

impl Params {
    /// Code-only gameplay baseline. It carries the same simulation inputs the asset path
    /// uses for locomotion, slides, pounds, water and knockdown reactions, so RoR2's
    /// tuning, health, damage and items act on the same base action with or without the
    /// disc. Damage amounts, goop accumulation and nozzle visuals stay at their authored
    /// defaults: the host owns RoR2 damage and effects. Constructor defaults are not a
    /// playable reference (MarioJump.cpp::doJumping multiplies stick magnitude by the
    /// acceleration, and WaterGun.cpp::movement advances the spray pressure gate every
    /// tick), so the values below are the playable ones.
    pub fn builtin() -> Self {
        let mut params = Self::default();
        for nozzle in &mut params.nozzles { nozzle.dec_rate = 1; }
        params.jump.gravity = 1.;
        params.jump.jump_accel_control = 0.008;
        params.jump.jump_slide_control = 0.3;
        params.jump.spin_jump_gravity = 0.35;
        params.jump.turn_jump_force = 70.;
        params.jump.broad_jump_force = 60.;
        params.jump.broad_jump_force_y = 70.;
        params.jump.rotate_jump_force_y = 70.;
        params.jump.back_jump_force = -18.;
        params.jump.back_jump_force_y = 70.;
        params.jump.rot_broad_jump_force = 55.;
        params.jump.rot_broad_jump_force_y = 38.;
        params.jump.sec_jump_enable_sp = 0.;
        params.jump.sec_jump_speed_mult = 0.25;
        params.jump.tri_jump_enable_sp = 25.;
        params.jump.ultra_jump_force = 75.;
        params.jump.ultra_jump_speed_mult = 0.15;
        params.jump.ultra_jump_xz_mult = 1.;
        params.jump.clash_angle = 12000;
        params.jump.jump_jump_catch_sp = 80.;
        // The asset path's pound: same fall speed and super-pound timing in RoR2.
        params.jump.hip_attack_speed_y = -80.;
        params.jump.super_hip_attack_ct = 80;
        params.jump.fire_down_force = 50.;
        params.jump.fire_down_control = 0.;
        params.jump.fire_back_velocity = 8.;
        params.jump.get_off_yoshi_y = 60.;
        params.mario.running_max = 80.;
        params.mario.dash_max = 80.;
        params.mario.dash_brake = 0.1;
        params.mario.dash_start_time = 0;
        // The asset path's turn rates, reach, throw power and impact thresholds: the same
        // steering, grabs, throws and knockdowns in RoR2.
        params.mario.waiting_rot_sp = 220;
        params.mario.running_rot_sp_min = 1500;
        params.mario.running_rot_sp_max = 1500;
        params.mario.pumping_rot_sp_min = 300;
        params.mario.pumping_rot_sp_max = 600;
        params.mario.pushup_radius = 60.;
        params.mario.pushup_height = 180.;
        params.mario.hipdrop_radius = 100.;
        params.mario.trample_pow_step1 = 35.;
        params.mario.trample_pow_step2 = 45.;
        params.mario.trample_pow_step3 = 70.;
        params.mario.jump_wall_height = 500.;
        params.mario.slip_start = 0.4;
        params.mario.was_on_water_slip = 0.985;
        params.mario.force_slip_angle = 0.4;
        params.mario.clash_speed = 40.;
        params.mario.damage_fall_height = 1500.;
        params.mario.feel_deep = 800.;
        params.mario.hold_radius = 80.;
        params.mario.throw_power = 2.8;
        params.controller.start_to_walk_level = 1.;
        params.controller.stick_rotate_time = 40;
        params.controller.length_mult_times = 2;
        params.controller.length_mult = 0.78;
        params.controller.analog_lr_to_zero_val = 10;
        params.controller.analog_lr_to_middle_val = 60;
        params.controller.analog_lr_to_max_val = 150;
        params.controller.analog_lr_middle_level = 0.5;
        params.run.dec_brake = 5.;
        params.run.soft_to_walk = 13.;
        params.run.walk_to_soft = 8.;
        params.run.swim_depth = 110.;
        params.run.in_water_brake = 0.95;
        params.run.pumping_slide_sp = 0.12;
        params.run.dash_rot_sp = 60;
        // The asset path's hanging, pumping, Yoshi and underwater control.
        params.upper_body.pump_wait_time = 5;
        params.upper_body.pump_anm_speed = 0.015;
        params.upper_body.hover_head_angle = 5294;
        params.hanging.move_sp = 0.06;
        params.hanging.anm_rate = 0.8;
        params.diving.rot_sp = 90;
        params.diving.gravity = 2.;
        params.diving.accel_control = 0.01;
        params.diving.sea_brake = 0.995;
        params.diving.sea_brake_y = 0.9;
        params.yoshi.jump_yoshi_mult = 1.2;
        params.yoshi.rot_yoshi_mult = 2.;
        params.yoshi.head_front = 120.;
        params.yoshi.head_radius = 10.;
        params.yoshi.hold_out_acc_ctrl_f = 0.001;
        params.yoshi.hold_out_acc_ctrl_b = 0.02;
        // The asset path's slide: the same friction, acceleration and stop rates make the
        // belly slide cover the same ground in RoR2, on dry ground, wet ground, slopes and
        // Yoshi.
        params.slip_normal.slip_friction = 0.97;
        params.slip_normal.slide_accele_down = 1.5;
        params.slip_normal.slide_stop_normal = 1.;
        params.slip_normal.slide_stop_catch = 1.;
        params.slip_normal.slide_angle_y_sp = 100;
        params.slip_normal.stick_slide_mult = 0.005;
        params.slip_all.slip_friction = 0.95;
        params.slip_all.slope_accele_up = 4.5;
        params.slip_all.slope_accele_down = 2.;
        params.slip_all.slide_accele_up = 4.;
        params.slip_all.slide_accele_down = 5.;
        params.slip_all.slide_stop_normal = 1.;
        params.slip_all.slide_stop_catch = 1.;
        params.slip_all.jump_enable = 0;
        params.slip_all.miss_jump = 1;
        params.slip_all.slide_angle_y_sp = 0;
        params.slip_all.stick_slide_mult = 0.;
        params.slip_all_slider.slip_friction = 0.975;
        params.slip_all_slider.slide_accele_up = 4.5;
        params.slip_all_slider.slide_accele_down = 2.;
        params.slip_all_slider.slide_stop_normal = 3.;
        params.slip_all_slider.slide_stop_catch = 3.;
        params.slip_all_slider.stick_slide_mult = 0.025;
        params.slip_45.slip_friction = 0.93;
        params.slip_45.slope_accele_up = 1.;
        params.slip_45.slope_accele_down = 0.021972656;
        params.slip_45.slide_accele_down = 1.5;
        params.slip_45.slide_stop_normal = 0.;
        params.slip_45.slide_stop_catch = 0.;
        params.slip_45.slide_angle_y_sp = 100;
        params.slip_45.stick_slide_mult = 0.005;
        params.slip_water_ground.slip_friction = 0.985;
        params.slip_water_ground.slide_accele_up = 5.;
        params.slip_water_ground.slide_accele_down = 2.;
        params.slip_water_ground.slide_stop_normal = 5.;
        params.slip_water_ground.slide_stop_catch = 5.;
        params.slip_water_ground.stick_slide_mult = 0.;
        params.slip_water_slope.slip_friction = 0.98;
        params.slip_water_slope.slide_accele_up = 5.;
        params.slip_water_slope.slide_accele_down = 2.;
        params.slip_water_slope.slide_stop_normal = 5.;
        params.slip_water_slope.slide_stop_catch = 5.;
        params.slip_water_slope.stick_slide_mult = 0.1;
        params.slip_yoshi.slip_friction = 0.97;
        params.slip_yoshi.slide_accele_up = 0.1;
        params.slip_yoshi.slide_accele_down = 1.;
        params.slip_yoshi.slide_stop_normal = 10.;
        params.slip_yoshi.slide_stop_catch = 10.;
        params.slip_yoshi.stick_slide_mult = 0.005;
        // The asset path's water: the same droplet arcs, lifetimes and body drips, so FLUDD
        // reaches and wets the same ground in RoR2.
        params.wet_emit.count = 3;
        params.wet_emit.alive = 128;
        params.wet_emit.particle_type = 1;
        params.water_particles[0].extension = 0.7;
        params.water_particles[0].magnify = 0.4;
        params.water_particles[0].clean_size = 18.;
        params.water_particles[1].extension = 1.;
        params.water_particles[1].magnify = 0.5;
        params.water_particles[1].clean_size = 6.;
        params.water_particles[1].gravity = -0.5;
        params.water_particles[2].extension = 0.8;
        params.water_particles[2].clean_size = 24.;
        params.water_particles[2].gravity = -0.2;
        params.water_particles[2].alive = 100.;
        params.water_particles[3].extension = 0.6;
        params.water_particles[3].clean_size = 25.;
        params.water_particles[3].gravity = -10.;
        params.water_particles[4].clean_size = 8.;
        params.water_particles[4].gravity = -5.;
        params.water_particles[5].extension = 0.7;
        params.water_particles[5].magnify = 1.;
        params.water_particles[5].clean_size = 28.;
        params.water_particles[6].extension = 1.;
        params.water_particles[6].clean_size = 5.;
        params.water_particles[7].extension = 1.3;
        params.water_particles[7].magnify = 0.1;
        params.water_particles[7].clean_size = 0.5;
        params.water_particles[7].gravity = 1.;
        params.water_particles[7].alive = 120.;
        params.water_particles[8].extension = 0.8;
        params.water_particles[8].clean_size = 25.;
        params.water_particles[8].gravity = 0.01;
        params.water_particles[10].clean_size = 5.;
        params.water_particles[10].gravity = -0.2;
        params.water_particles[11].extension = 3.;
        params.water_particles[11].clean_size = 16.;
        params.water_particles[13].extension = 1.;
        params.water_particles[13].clean_size = 10.;
        params.water_particles[13].gravity = 0.;
        params.water_particles[13].alive = 3.;
        params.water_particles[15].clean_size = 20.;
        params.water_particles[15].gravity = -1.;
        // The knockdown reactions (how Mario falls, how far he slides, how long he can't be
        // hit again). RoR2 still supplies the health and damage; these are the sim's reaction.
        let enemy = DamageKind::EnemyCommon as usize;
        params.damage[enemy].min_speed = 30.;
        let stu = DamageKind::Hamukuri as usize;
        params.damage[stu].down_type = 0;
        params.damage[stu].min_speed = 30.;
        params.damage[stu].motor = 8;
        let swoopin = DamageKind::Namekuri as usize;
        params.damage[swoopin].down_type = 0;
        params.damage[swoopin].min_speed = 20.;
        params.damage[swoopin].motor = 6;
        params.damage[DamageKind::BossEel as usize].down_type = 3;
        params.damage[DamageKind::Killer as usize].min_speed = 50.;
        params.damage[DamageKind::LampTrapIron as usize].invincible_time = 20;
        params.damage[DamageKind::LampTrapSpike as usize].invincible_time = 20;
        params.damage[DamageKind::EnemyMario as usize].down_type = 2;
        params.damage[DamageKind::EnemyMario as usize].invincible_time = 120;
        let fire_goop = DamageKind::GraffitoFire as usize;
        params.damage[fire_goop].down_type = 0;
        params.damage[fire_goop].min_speed = 0.;
        params.damage[fire_goop].invincible_time = 50;
        params.damage[fire_goop].motor = 10;
        params.damage[DamageKind::GraffitoElec as usize].down_type = 0;
        params.damage[DamageKind::GraffitoElec as usize].motor = 10;
        params.damage[DamageKind::GraffitoPoison as usize].down_type = 0;
        params.damage[DamageKind::GraffitoPoison as usize].motor = 10;
        params.damage[DamageKind::GraffitoLava as usize].down_type = 0;
        params.damage[DamageKind::GraffitoLava as usize].motor = 10;
        params.hover.rot_sp = 200;
        params.hover.accel_rate = 0.02;
        params.hover.brake = 0.985;
        params.water_gun.rocket_height = 100.;
        params.water_gun.hover_height = 230.;
        params.water_gun.nozzle_angle_y_speed = 100.;
        params.water_gun.nozzle_angle_y_brake = 0.99;
        params.water_gun.hover_rot_max = 0;
        params.water_gun.hover_smooth = 0.03;
        let spray = &mut params.nozzles[NozzleKind::Spray as usize];
        spray.amount_max = 10_000;
        spray.trigger_rate = 50;
        spray.num = 0.8;
        let hover = &mut params.nozzles[NozzleKind::Hover as usize];
        hover.rocket_type = 1;
        hover.amount_max = 10_000;
        hover.emit_pow = 37.5;
        hover.reaction_pow = 0.005;
        hover.reaction_y = 0.;
        hover.emit_ctrl = 0.;
        hover.l_angle_base = 0;
        hover.side_angle_max_side = 5000;
        hover.side_angle_max_front = 0;
        hover.side_angle_max_back = 8000;
        hover.l_angle_chase = 0.5;
        hover.trigger_rate = 0;
        hover.inside_pressure_dec = 0.;
        hover.inside_pressure_max = 1.;
        hover.trigger_time = 360;
        let rocket = &mut params.nozzles[NozzleKind::Rocket as usize];
        rocket.rocket_type = 1;
        rocket.emit_pow = 50.;
        rocket.reaction_y = 0.7;
        rocket.dec_rate = 35;
        rocket.num_min = 0.;
        rocket.trigger_rate = 0;
        rocket.trigger_time = 5;
        rocket.inside_pressure_max = 8000.;
        rocket.inside_pressure_dec = 105.;
        let turbo = &mut params.nozzles[NozzleKind::Turbo as usize];
        turbo.reaction_pow = 0.002;
        turbo.reaction_y = 0.002;
        turbo.trigger_time = i16::MAX;
        params
    }
}

impl Default for Params {
    fn default() -> Self {
        let slip = Slip {
            miss_jump: 0,
            ..Slip::default()
        };
        Params {
            wet_emit: WetWaterEmit::default(),
            run: Run::default(),
            jump: Jump::default(),
            mario: Mario::default(),
            controller: Controller::default(),
            water_effect: WaterEffect::default(),
            upper_body: UpperBody::default(),
            hanging: Hanging::default(),
            hover: Hover::default(),
            diving: Diving::default(),
            water_gun: WaterGun::default(),
            yoshi: YoshiParams::default(),
            slip_normal: slip.clone(),
            slip_all: slip.clone(),
            slip_all_slider: slip.clone(),
            slip_45: slip.clone(),
            slip_water_ground: slip.clone(),
            slip_water_slope: slip.clone(),
            slip_yoshi: slip,
            nozzles: Default::default(),
            water_particles: Default::default(),
            damage: Default::default(),
            dirty: Dirty::default(),
            graffito: Graffito::default(),
        }
    }
}

/// `.prm` paths relative to the extracted `params.szs:mario/` folder, lower-case.
mod files {
    pub const RUN: &str = "run.prm";
    pub const JUMP: &str = "jump.prm";
    pub const MARIO: &str = "mario.prm";
    pub const CONTROLLER: &str = "controller.prm";
    pub const WATER_EFFECT: &str = "watereffect.prm";
    pub const UPPER_BODY: &str = "upperbody.prm";
    pub const HANGING: &str = "hanging.prm";
    pub const HOVER: &str = "hhover.prm";
    pub const DIVING: &str = "diving.prm";
    pub const WATER_GUN: &str = "watergun.prm";
    pub const YOSHI: &str = "yoshi.prm";
    pub const SLIP_YOSHI: &str = "slipparamyoshi.prm";
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
    /// Indexed by `DamageKind` (`TMario::TMario`'s `mDmgParams*`).
    pub const DAMAGE: [&str; 17] = [
        "dmgenemycommon.prm",
        "dmghamukuri.prm",
        "dmgnamekuri.prm",
        "dmghinokuri.prm",
        "dmgfire.prm",
        "dmgbgtentacle.prm",
        "dmgbosseel.prm",
        "dmghanachanboss.prm",
        "dmgpoihana.prm",
        "dmgkiller.prm",
        "dmglamptrapiron.prm",
        "dmglamptrapspike.prm",
        "dmgenemymario.prm",
        "dmggraffitofire.prm",
        "dmggraffitoelec.prm",
        "dmggraffitopoison.prm",
        "dmggraffitolava.prm",
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
        if let Some(prm) = get("wetwateremit.prm") {
            p.wet_emit.apply(&prm);
        }
        if let Some(prm) = get(files::WATER_EFFECT) {
            p.water_effect.apply(&prm);
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
        if let Some(prm) = get(files::YOSHI) {
            p.yoshi.apply(&prm);
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
        p.slip_yoshi = p.slip_normal.clone();
        if let Some(prm) = get(files::SLIP_YOSHI) {
            p.slip_yoshi.apply(&prm);
        }
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
        for (i, path) in files::DAMAGE.iter().enumerate() {
            if let Some(prm) = get(path) {
                p.damage[i].apply(&prm);
            }
        }
        if let Some(prm) = get("dirty.prm") {
            p.dirty.apply(&prm);
        }
        if let Some(prm) = get("graffito.prm") {
            p.graffito.apply(&prm);
        }
        (p, loaded)
    }

    /// Convenience: read from a directory laid out like the extracted `params.szs:mario/`.
    pub fn from_dir(dir: &std::path::Path) -> (Params, usize) {
        Params::from_files(|rel| std::fs::read(dir.join(rel)).ok())
    }

    /// Gameplay requirements are independent of models and sound. A partial or
    /// corrupt parameter set selects the complete authored fallback profile.
    pub fn gameplay_files_present(dir: &std::path::Path) -> bool {
        [files::RUN, files::JUMP, files::MARIO, files::CONTROLLER, files::WATER_GUN]
            .into_iter().chain(files::NOZZLES).all(|name|
                std::fs::read(dir.join(name)).ok().and_then(|b| Prm::parse(&b).ok()).is_some())
    }

    pub fn nozzle(&self, kind: NozzleKind) -> &Nozzle {
        &self.nozzles[kind as usize]
    }

    pub fn damage(&self, kind: crate::damage::DamageKind) -> &Damage {
        &self.damage[kind as usize]
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
