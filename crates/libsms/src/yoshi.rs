//! Yoshi (`TYoshi`, Player/Yoshi.cpp) and riding him (the `onYoshi()` branches through
//! Player/Mario*.cpp).
//!
//! Yoshi belongs to Mario and starts inside an egg (`egg.rs`). Hatched, he waits where he is
//! (`STATE_UNMOUNTED`), falling under Mario's gravity and now and then tonguing fruit in reach
//! (`doSearch`). Mario mounts by landing on him from above; then Yoshi's position is Mario's,
//! his clip follows Mario's status (`thinkAnimation`) and Mario's clip follows his
//! (`setAnimation`'s table, `mario_clip`). Riding, Mario runs faster and turns quicker
//! (`TYoshiParams`), can't dive, pump or grab, gets a flutter at the top of a jump
//! (`thinkHoldOut`), shoots the tongue with B and spits juice with R (the Yoshi nozzle). X gets
//! off. Juice runs out after `JUICE_MAX` ticks (refilled by eating fruit): Yoshi blinks for the
//! last 5 s, throws Mario and vanishes, and his egg comes back.

use glam::Vec3;

use crate::anim::{AnimTable, ClipName, FrameCtrl};
use crate::event::{Event, SoundCue};
use crate::host::{Fruit, Host, SurfaceFlags};
use crate::mario::{Mario, Status, UpperState};
use crate::math::{Angle, cos, matan, sin};
use crate::params::NozzleKind;
use crate::tongue::{Tongue, TongueState};

/// Yoshi's clips by `MActorAnmData` index: the order of `yoshi.szs` (alphabetical).
pub mod clip {
    pub const BORN: u8 = 0;
    pub const DAMAGE: u8 = 1;
    pub const DEMO_SHINE_GET: u8 = 2;
    pub const EAT: u8 = 3;
    pub const EAT_END: u8 = 4;
    pub const GOFF: u8 = 5;
    pub const HIP_END: u8 = 6;
    pub const HIP_POSE: u8 = 7;
    pub const HIP_START: u8 = 8;
    pub const HOLD_JUMP: u8 = 9;
    pub const JUMP: u8 = 10;
    pub const JUMP_END: u8 = 11;
    pub const JUMP_FALL: u8 = 12;
    pub const PIVOT: u8 = 13;
    pub const RIDE: u8 = 14;
    pub const RUN: u8 = 15;
    pub const SIDEWALK_L: u8 = 16;
    pub const SIDEWALK_R: u8 = 17;
    pub const SLIDE_END: u8 = 18;
    pub const SLIDE_POSE: u8 = 19;
    pub const SLIDE_START: u8 = 20;
    pub const TONGUE_EAT: u8 = 21;
    pub const WAIT: u8 = 22;
    pub const WAIT_ALONE: u8 = 23;
    pub const WALK: u8 = 24;
    pub const WATER_DIE: u8 = 25;
}

/// Clip file stems, by index.
pub const CLIPS: [&str; 26] = [
    "yoshi_born",
    "yoshi_damage",
    "yoshi_demo_shine_get",
    "yoshi_eat",
    "yoshi_eat_end",
    "yoshi_goff",
    "yoshi_hip_end",
    "yoshi_hip_pose",
    "yoshi_hip_start",
    "yoshi_hold_jump",
    "yoshi_jump",
    "yoshi_jump_end",
    "yoshi_jump_fall",
    "yoshi_pivot",
    "yoshi_ride",
    "yoshi_run",
    "yoshi_sidewalk_l",
    "yoshi_sidewalk_r",
    "yoshi_slide_end",
    "yoshi_slide_pose",
    "yoshi_slide_start",
    "yoshi_tongue_eat",
    "yoshi_wait",
    "yoshi_wait_alone",
    "yoshi_walk",
    "yoshi_water_die",
];

/// Eye texture animations, by `thinkBtp` index.
pub const BTPS: [&str; 5] = [
    "yoshi_born_tx",
    "yoshi_damage_tx",
    "yoshi_hold_jump_tx",
    "yoshi_water_die_tx",
    "yoshi_wink_tx",
];

/// `bodyColor`: green, orange, purple, pink.
pub const BODY_COLORS: [[u8; 3]; 4] = [
    [0x40, 0xA1, 0x24],
    [0xFF, 0x8C, 0x1C],
    [0xAA, 0x4C, 0xFF],
    [0xFF, 0xA0, 0xBE],
];

/// `gModelWaterManagerWaterColor`: water, then each juice (ModelWaterManager.cpp).
pub const JUICE_COLORS: [[u8; 4]; 4] = [
    [0x3C, 0x46, 0x78, 0x14],
    [0xFE, 0xA8, 0x02, 0x6E],
    [0x9B, 0x01, 0xFD, 0x6E],
    [0xFD, 0x62, 0xA7, 0x6E],
];

/// `unk8`: ticks of juice a fruit gives.
pub const JUICE_MAX: i32 = 21300;
/// `mMaxFlutterTimer`, `mMaxVSpdStartFlutter`, `mFlutterAcceleration`.
pub const FLUTTER_TIME: u16 = 120;
pub const FLUTTER_START_VY: f32 = -5.0;
pub const FLUTTER_ACCEL: f32 = 1.2;
/// `unk98`/`unk9C`: speeds over which the run clip blends in from the walk.
const RUN_BLEND: (f32, f32) = (20.0, 30.0);
/// `unkA0`/`unkA4`: the run clip's rate per forward speed.
const RUN_RATE: (f32, f32) = (0.1, 0.045);
/// `unk80`: how fast the body colour changes.
const COLOR_RATE: f32 = 0.01;
/// `unk90`: how far `emitTongue` throws the tip at once.
const TONGUE_THROW: f32 = 160.0;
/// `unkE4`, `unkE8`/`unkEA`: turning toward fruit, and ticks between looks.
const SEARCH_TURN: f32 = 0.1;
const SEARCH_WAIT: (f32, f32) = (120.0, 240.0);
/// `unk114`: shadow radius.
pub const SHADOW_RADIUS: f32 = 80.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum YoshiState {
    /// Not hatched (`STATE_EGG`).
    #[default]
    Egg,
    /// Hatching (`STATE_UNK2`): the `born` clip.
    Hatching,
    /// Sinking in deep water (`STATE_DROWNING`).
    Drowning,
    /// Out of juice (`STATE_DYING`).
    Dying,
    /// Gone; the egg returns next tick (`STATE_UNK5`).
    Gone,
    /// Waiting for Mario (`STATE_UNMOUNTED`).
    Unmounted,
    /// Mario is riding (`STATE_MOUNTED`).
    Mounted,
}

#[derive(Debug, Clone)]
pub struct Yoshi {
    pub state: YoshiState,
    /// `mTranslation`.
    pub pos: Vec3,
    /// Facing (`mEggRotSpeed`).
    pub yaw: Angle,
    /// Falling speed when alone (`unk2C`).
    vel_y: f32,
    /// Ticks of juice left (`unkC`).
    pub juice: i32,
    /// Juice kind, which colours him and his spit (`mType`, `Color`): 0 none, 1 orange,
    /// 2 purple, 3 pink.
    pub color: u8,
    /// The drawn body colour, easing toward `BODY_COLORS[color]` (`unk84`).
    pub body_color: Vec3,
    /// The body clip (index into [`CLIPS`]) and its frame controller.
    pub clip: u8,
    pub ctrl: FrameCtrl,
    /// Weight of the walk clip under the run clip (`setMotionBlendRatioForBck`).
    pub blend: f32,
    /// The eye animation (index into [`BTPS`]) and its frame.
    pub btp: u8,
    pub btp_ctrl: FrameCtrl,
    /// The eating head over `jnt_18` and below (`thinkUpper`, `unk5C`): EAT or EAT_END.
    pub eat: Option<(u8, FrameCtrl)>,
    /// `mFlutterState`: 0 can start, 1 fluttering, 2 spent.
    pub(crate) flutter: u8,
    flutter_timer: u16,
    /// `doSearch`'s step (`unkDC`), countdown (`unkDE`) and target angle (`unkE0`).
    search: u8,
    search_timer: i16,
    search_yaw: Angle,
    pub tongue: Tongue,
    /// Which egg he came from.
    pub egg: Option<usize>,
    /// The host's posed `null_tongue` joint in Yoshi's model space: position and X axis.
    mouth: Option<(Vec3, Vec3)>,
}

impl Default for Yoshi {
    fn default() -> Yoshi {
        let c = BODY_COLORS[0];
        Yoshi {
            state: YoshiState::Egg,
            pos: Vec3::ZERO,
            yaw: 0,
            vel_y: 0.0,
            juice: JUICE_MAX,
            color: 0,
            body_color: Vec3::new(c[0] as f32, c[1] as f32, c[2] as f32),
            clip: clip::WAIT_ALONE,
            ctrl: FrameCtrl::new(AnimTable::FALLBACK),
            blend: 0.0,
            btp: 4,
            btp_ctrl: FrameCtrl::new(AnimTable::FALLBACK),
            eat: None,
            flutter: 2,
            flutter_timer: 0,
            search: 0,
            search_timer: 0,
            search_yaw: 0,
            tongue: Tongue::default(),
            egg: None,
            mouth: None,
        }
    }
}

impl Yoshi {
    /// `isHatched`.
    pub fn is_hatched(&self) -> bool {
        self.state != YoshiState::Egg
    }

    /// `setStatusToJumping`: a fresh flutter for each jump.
    pub(crate) fn flutter_timer_reset(&mut self) {
        self.flutter_timer = FLUTTER_TIME;
    }

    /// `curAnmEndsNext`.
    fn ends_next(&self) -> bool {
        self.ctrl.frame + self.ctrl.rate + 0.1 >= self.ctrl.end
    }

    /// Yoshi's model space to the world.
    fn to_world(&self, v: Vec3) -> Vec3 {
        let (s, c) = (sin(self.yaw), cos(self.yaw));
        self.pos + Vec3::new(v.x * c + v.z * s, v.y, -v.x * s + v.z * c)
    }

    /// The `null_tongue` joint in the world: position and X axis (`getEmitPosDir`). Without a
    /// host pose, a stand-in at his mouth.
    pub(crate) fn mouth_world(&self) -> (Vec3, Vec3) {
        let (p, d) = self.mouth.unwrap_or((Vec3::new(0.0, 110.0, 95.0), Vec3::Z));
        let dir = self.to_world(d) - self.pos;
        (self.to_world(p), dir.try_normalize().unwrap_or(Vec3::Z))
    }
}

/// Mario's clip over each Yoshi clip (`TMario::setAnimation`'s jump table, MarioDraw.cpp; the
/// `yo_*` rows of `gMarioAnimeData` name the same pairs). Anything else is `yo_wait`.
pub fn mario_clip(yoshi: u8) -> ClipName {
    match yoshi {
        clip::DEMO_SHINE_GET => "demo_shine_get_yo",
        clip::EAT => "yo_eat",
        clip::EAT_END => "yo_eat_end",
        clip::HIP_END => "yo_hip_end",
        clip::HIP_POSE => "yo_hip_pose",
        clip::HIP_START => "yo_hip_start",
        clip::HOLD_JUMP => "yo_hold_jump",
        clip::JUMP => "yo_jump",
        clip::JUMP_END => "yo_jump_end",
        clip::JUMP_FALL => "yo_jump_fall",
        clip::RIDE => "yo_ride",
        clip::RUN | clip::SIDEWALK_L | clip::SIDEWALK_R => "yo_run",
        clip::SLIDE_END => "yo_slide_end",
        clip::SLIDE_POSE => "yo_slide_pose",
        clip::WALK => "yo_walk",
        _ => "yo_wait",
    }
}

/// What to draw for Yoshi.
#[derive(Debug, Clone, PartialEq)]
pub struct YoshiLook {
    /// The model's root: Mario's while riding (`getTakenMtx`).
    pub pos: Vec3,
    pub yaw: Angle,
    pub clip: &'static str,
    pub frame: f32,
    /// The walk clip blended under the run clip at the same frame, and its weight.
    pub blend: Option<(&'static str, f32)>,
    pub btp: &'static str,
    pub btp_frame: f32,
    /// The eating head over `jnt_18`'s subtree.
    pub eat: Option<(&'static str, f32)>,
    /// TEV colour register 2 on every material (`entry`), 0..255.
    pub body_color: [f32; 3],
    /// Show the separate hand models (`yoshi_hand2_l/r` on joints 37/32) and hide shapes 0
    /// and 1, as in the jump, fall and run clips.
    pub hands: bool,
    /// Head pitch from the nozzle (`YoshiHeadCtrl` on joint 21), angle units.
    pub head_angle: f32,
    /// The tongue (`mHeadPos`, `mHeadDir`, `mTipPos`) while it is out.
    pub tongue: Option<(Vec3, Vec3, Vec3)>,
    pub mounted: bool,
}

/// `bodyColor` for a fruit (`doEat`).
fn color_for(fruit: Option<Fruit>) -> Option<u8> {
    match fruit? {
        Fruit::Papaya | Fruit::Pine => Some(1),
        Fruit::Durian => Some(2),
        Fruit::Banana | Fruit::Coconut => Some(3),
    }
}

impl Mario {
    /// `TMario::onYoshi`.
    pub fn on_yoshi(&self) -> bool {
        self.yoshi.state == YoshiState::Mounted
    }

    /// The juice colour FLUDD's water is drawn with (`TModelWaterManager::unk5D5F`):
    /// Yoshi's while riding him, else plain water.
    pub fn juice_color(&self) -> u8 {
        if self.on_yoshi() { self.yoshi.color } else { 0 }
    }

    /// The host's posed `null_tongue` joint, in Yoshi's model space (position, X axis).
    pub fn set_yoshi_mouth(&mut self, pos: Vec3, x_axis: Vec3) {
        self.yoshi.mouth = Some((pos, x_axis));
    }

    /// What to draw for Yoshi, if he is out of his egg.
    pub fn yoshi_look(&self) -> Option<YoshiLook> {
        let y = &self.yoshi;
        if !y.is_hatched() || y.state == YoshiState::Gone || y.state == YoshiState::Dying {
            return None;
        }
        // `entry`: blinking for the last seconds of juice.
        if matches!(y.state, YoshiState::Unmounted | YoshiState::Mounted)
            && ((360..600).contains(&y.juice) && y.juice & 0x10 == 0
                || y.juice < 360 && y.juice & 0x8 == 0)
        {
            return None;
        }
        let (pos, yaw) = if self.on_yoshi() {
            (self.pos, self.model_yaw)
        } else {
            (y.pos, y.yaw)
        };
        let gun = self
            .fludd
            .as_ref()
            .filter(|f| f.nozzle == NozzleKind::Yoshi)
            .map_or(0.0, |f| f.gun_angle());
        Some(YoshiLook {
            pos,
            yaw,
            clip: CLIPS[y.clip as usize],
            frame: y.ctrl.frame,
            blend: (y.clip == clip::RUN && y.blend > 0.0)
                .then_some((CLIPS[clip::WALK as usize], y.blend)),
            btp: BTPS[y.btp as usize],
            btp_frame: y.btp_ctrl.frame,
            eat: y.eat.map(|(c, f)| (CLIPS[c as usize], f.frame)),
            body_color: y.body_color.to_array(),
            hands: matches!(y.clip, clip::JUMP | clip::JUMP_FALL | clip::RUN),
            head_angle: gun,
            tongue: (y.tongue.state != TongueState::Idle).then_some((
                y.tongue.head,
                y.tongue.head_dir,
                y.tongue.tip,
            )),
            mounted: self.on_yoshi(),
        })
    }

    fn yoshi_sound(&mut self, name: &'static str, pos: Vec3) {
        self.events.push(Event::Sound(SoundCue {
            name,
            pos,
            info: None,
        }));
    }

    /// `setBckFromIndex`: a new clip from its first frame.
    fn yoshi_set_clip(&mut self, id: u8) {
        let y = &mut self.yoshi;
        y.clip = id;
        let rate = y.ctrl.rate;
        y.ctrl = FrameCtrl::new(self.anims.get(CLIPS[id as usize]));
        y.ctrl.rate = rate;
    }

    /// `changeAnimation`.
    pub(crate) fn yoshi_change_anim(&mut self, id: u8) {
        if id != self.yoshi.clip {
            self.yoshi_set_clip(id);
            self.yoshi_think_btp(id);
        }
    }

    /// `thinkBtp`: the eyes for the clip.
    fn yoshi_think_btp(&mut self, id: u8) {
        let btp = match id {
            clip::BORN => 0,
            clip::DAMAGE => 1,
            clip::HOLD_JUMP => 2,
            clip::WATER_DIE => 3,
            _ => 4,
        };
        if self.yoshi.btp != btp {
            self.yoshi.btp = btp;
            let mut c = FrameCtrl::new(self.anims.get(BTPS[btp as usize]));
            c.rate = 0.5;
            self.yoshi.btp_ctrl = c;
        }
    }

    /// `appearFromEgg`: Yoshi hatches at `pos` facing `yaw`, eating the fruit that hatched him.
    pub(crate) fn appear_from_egg(
        &mut self,
        pos: Vec3,
        yaw: Angle,
        egg: usize,
        fruit: Option<(u64, Fruit)>,
    ) {
        self.yoshi.pos = pos + Vec3::Y;
        self.yoshi.yaw = yaw;
        self.yoshi.state = YoshiState::Hatching;
        self.yoshi.ctrl.rate = 0.5;
        self.yoshi_set_clip(clip::BORN);
        self.yoshi_think_btp(clip::BORN);
        if let Some((id, kind)) = fruit {
            if self.held == Some(id) {
                self.held = None;
                self.held_fruit = None;
                if self.upper == UpperState::HoldingObject {
                    self.upper = UpperState::Idle;
                }
            }
            self.events.push(Event::Eaten { id });
            self.yoshi_do_eat(Some(kind));
        }
        self.yoshi.juice = JUICE_MAX;
        self.yoshi.egg = Some(egg);
    }

    /// Puts a hatched Yoshi at `pos` (for hosts and tests that skip the egg), full of the
    /// juice `fruit` gives (none: green, no spit until he eats).
    pub fn spawn_yoshi(&mut self, pos: Vec3, yaw: Angle, fruit: Option<Fruit>) {
        self.yoshi.pos = pos;
        self.yoshi.yaw = yaw;
        self.yoshi.state = YoshiState::Unmounted;
        self.yoshi.ctrl.rate = 0.5;
        self.yoshi_set_clip(clip::WAIT_ALONE);
        self.yoshi_think_btp(clip::WAIT_ALONE);
        self.yoshi.color = color_for(fruit).unwrap_or(0);
        let c = BODY_COLORS[self.yoshi.color as usize];
        self.yoshi.body_color = Vec3::new(c[0] as f32, c[1] as f32, c[2] as f32);
        self.yoshi.juice = JUICE_MAX;
    }

    /// `disappear`: out of juice, into water, or Mario fell out of the world.
    pub(crate) fn yoshi_disappear(&mut self) -> bool {
        if !self.yoshi.is_hatched() {
            return false;
        }
        if self.on_yoshi() {
            self.get_off_yoshi(true);
        }
        if self.in_water {
            self.yoshi.state = YoshiState::Drowning;
            self.yoshi_change_anim(clip::WATER_DIE);
        } else {
            self.yoshi.state = YoshiState::Dying;
        }
        self.yoshi.color = 0;
        true
    }

    /// `ride`.
    fn yoshi_ride(&mut self) {
        self.yoshi.state = YoshiState::Mounted;
        self.yoshi_change_anim(clip::WAIT);
        let p = self.yoshi.pos;
        self.yoshi_sound("MSD_SE_YV_DELICIOUS", p);
    }

    /// `TYoshi::getOff`.
    fn yoshi_get_off(&mut self, knocked: bool) {
        if !self.on_yoshi() {
            return;
        }
        self.yoshi.vel_y = 0.0;
        self.yoshi.state = YoshiState::Unmounted;
        let p = self.yoshi.pos;
        if knocked {
            self.yoshi_change_anim(clip::DAMAGE);
            self.yoshi_sound("MSD_SE_YV_DAMAGE", p);
        } else {
            self.yoshi_change_anim(clip::WAIT_ALONE);
            self.yoshi_sound("MSD_SE_YV_PURU_PURU", p);
        }
    }

    /// `TMario::getOffYoshi`: X hops off backwards; `fly` is being thrown (damage, juice out).
    pub fn get_off_yoshi(&mut self, fly: bool) {
        if !self.on_yoshi() {
            return;
        }
        if fly {
            self.change_status(Status::YoshiThrown, 0, false);
            self.yoshi_get_off(true);
        } else {
            self.change_status(Status::BackJump, 0, false);
            self.vel.y = self.params.jump.get_off_yoshi_y;
            self.yoshi_get_off(false);
        }
        self.set_anim(crate::anim::clip::JUMP, 1.0);
        self.pos.y += 100.0;
        self.forward_vel = -8.0;
        let p = self.params.clone();
        if let Some(f) = self.fludd.as_mut() {
            f.change_nozzle(NozzleKind::Hover, true, &p);
            // `normalizeNozzle`
            f.change_nozzle(NozzleKind::Spray, true, &p);
            f.refill(&p);
        }
    }

    /// `checkCollision`'s Yoshi part: landing on him from above mounts him; otherwise Mario
    /// is kept out of his body (`keepDistance`, radius 80).
    pub(crate) fn check_yoshi_collision(&mut self, host: &mut dyn Host) {
        if !matches!(
            self.yoshi.state,
            YoshiState::Unmounted | YoshiState::Hatching
        ) {
            return;
        }
        let yt = self.yoshi.pos;
        if !(yt.y <= self.pos.y && self.pos.y < yt.y + 100.0) {
            return;
        }
        let d = Vec3::new(yt.x - self.pos.x, 0.0, yt.z - self.pos.z).length();
        if self.status.is_jumping()
            && self.held.is_none()
            && self.vel.y < 0.0
            && yt.y < self.pos.y
            && !matches!(self.status, Status::YoshiThrown | Status::BackJump)
            && d < 180.0
        {
            self.mount_yoshi();
            return;
        }
        self.keep_distance(host, yt, 80.0);
    }

    /// Climbs onto a hatched Yoshi where he stands (what landing on him does).
    pub fn mount_yoshi(&mut self) {
        if !matches!(
            self.yoshi.state,
            YoshiState::Unmounted | YoshiState::Hatching
        ) {
            return;
        }
        self.pos = self.yoshi.pos;
        self.prev_pos = self.pos;
        self.face_yaw = self.yoshi.yaw;
        self.model_yaw = self.face_yaw;
        self.yoshi_ride();
        // `onFlag(MARIO_FLAG_HAS_FLUDD)`: his spit goes through FLUDD's nozzle code.
        if self.fludd.is_none() {
            self.give_fludd();
        }
        let p = self.params.clone();
        if let Some(f) = self.fludd.as_mut() {
            f.change_nozzle(NozzleKind::Yoshi, true, &p);
        }
        self.change_status(Status::Wait, 0, false);
    }

    /// `keepDistance(target, radius, 0)`: push Mario out of a cylinder.
    pub(crate) fn keep_distance(&mut self, host: &mut dyn Host, target: Vec3, radius: f32) {
        let (mut dx, dz) = (self.pos.x - target.x, self.pos.z - target.z);
        let thresh = radius + self.radius;
        let mut dist = (dx * dx + dz * dz).sqrt();
        if dist == 0.0 {
            dist = 1.0;
            dx = 1.0;
        }
        if dist >= thresh {
            return;
        }
        if !self.on_yoshi() {
            let clash = self.params.mario.clash_speed;
            let speed = (self.vel.x * self.vel.x + self.vel.z * self.vel.z).sqrt();
            if self.forward_vel > clash || speed > clash {
                self.change_status(Status::JumpShortBackDown, 0, false);
                return;
            }
        }
        let angle = matan(dz, dx);
        let mut next = Vec3::new(
            target.x + thresh * sin(angle),
            self.pos.y,
            target.z + thresh * cos(angle),
        );
        let r = self.radius;
        self.check_wall_plane(host, &mut next, 60.0, r);
        let (_, g) = self.check_ground_plane(host, next);
        if g.flags.has(SurfaceFlags::ILLEGAL) {
            return;
        }
        let diff = next - self.pos;
        let step = diff.length().min(50.0);
        if step > 0.0 {
            self.pos += diff.normalize() * step;
        }
    }

    /// `checkYoshiGetOff`: X.
    pub(crate) fn check_yoshi_get_off(&mut self) {
        if self.on_yoshi() && self.pad.x && !self.pad_prev.x {
            self.get_off_yoshi(false);
        }
    }

    /// `thinkYoshiHeadCollision`: his head (`mHeadFront` ahead, `mHeadRadius`) can't go into
    /// walls either.
    pub(crate) fn think_yoshi_head_collision(&mut self, host: &mut dyn Host) {
        if !self.on_yoshi() {
            return;
        }
        let p = &self.params.yoshi;
        let head = Vec3::new(
            self.pos.x + p.head_front * sin(self.face_yaw),
            self.pos.y + 100.0,
            self.pos.z + p.head_front * cos(self.face_yaw),
        );
        let mut walls = Vec::new();
        let moved = host.walls(head, p.head_radius, 4, &mut walls);
        if !walls.is_empty() {
            let d = Vec3::new(moved.x - head.x, 0.0, moved.z - head.z);
            let len = d.length();
            if len > 0.0 {
                self.pos += d / len * len.min(50.0);
            }
        }
    }

    /// `thinkHoldOut`: the flutter. Holding A while falling faster than 5 pushes up by
    /// `FLUTTER_ACCEL` a tick for up to `FLUTTER_TIME` ticks, once per jump.
    pub(crate) fn think_hold_out(&mut self) {
        match self.yoshi.flutter {
            0 => {
                if self.vel.y < FLUTTER_START_VY && self.pad.a {
                    self.yoshi.flutter = 1;
                }
            }
            1 => {
                if self.vel.y < 0.0 && 0.0 <= FLUTTER_ACCEL + self.vel.y {
                    let p = self.yoshi.pos;
                    self.yoshi_sound("MSD_SE_YV_FUNBARI", p);
                }
                if self.yoshi.flutter_timer != 0 {
                    self.yoshi.flutter_timer -= 1;
                    self.vel.y += FLUTTER_ACCEL;
                } else {
                    self.yoshi.flutter = 2;
                }
                if !self.pad.a {
                    self.yoshi.flutter = 2;
                }
            }
            _ => self.yoshi.flutter_timer = 0,
        }
    }

    /// Whether Yoshi is fluttering (`mFlutterState == 1`).
    pub(crate) fn fluttering(&self) -> bool {
        self.on_yoshi() && self.yoshi.flutter == 1
    }

    /// `emitTongue`: shoot, then run it up to 10 steps at once until it is `TONGUE_THROW` out.
    fn emit_tongue(&mut self, host: &mut dyn Host) {
        let (pos, dir) = self.yoshi.mouth_world();
        self.yoshi.tongue.emit(pos, dir, self.vel);
        for _ in 0..10 {
            self.yoshi.tongue.movement(host, &mut self.events);
            if (self.yoshi.tongue.tip - pos).length() > TONGUE_THROW {
                break;
            }
        }
        self.yoshi.search = 3;
    }

    fn search_wait(&mut self) -> i16 {
        let (lo, hi) = SEARCH_WAIT;
        ((hi - lo) * self.rand() + lo) as i16
    }

    /// `doSearch`: alone, now and then he turns to fruit in reach and tongues it.
    fn yoshi_search(&mut self, host: &mut dyn Host) {
        match self.yoshi.search {
            0 => {
                self.yoshi.search_timer -= 1;
                if self.yoshi.search_timer <= 0 {
                    if let Some(t) = self.yoshi.tongue.find_target(host, false, false) {
                        let d = t.pos - self.yoshi.pos;
                        self.yoshi.search_yaw = matan(d.z, d.x);
                        self.yoshi.search = 1;
                        return;
                    }
                    self.yoshi.search_timer = self.search_wait();
                }
            }
            1 => {
                // The decomp turns by the unwrapped difference.
                let delta = (SEARCH_TURN
                    * (self.yoshi.search_yaw as i32 - self.yoshi.yaw as i32) as f32)
                    as i16;
                self.yoshi.yaw = self.yoshi.yaw.wrapping_add(delta);
                if delta > -256 && delta < 256 {
                    self.emit_tongue(host);
                    self.yoshi_change_anim(clip::EAT);
                }
            }
            _ => {
                self.yoshi_think_eat();
                if self.yoshi.tongue.state == TongueState::Idle {
                    self.yoshi.search_timer = self.search_wait();
                    self.yoshi.search = 0;
                    self.yoshi_change_anim(clip::WAIT_ALONE);
                }
            }
        }
    }

    /// `doEat`: fruit sets the juice colour and fills him up.
    fn yoshi_do_eat(&mut self, fruit: Option<Fruit>) {
        if let Some(c) = color_for(fruit) {
            self.yoshi.color = c;
            self.yoshi.juice = JUICE_MAX;
            let p = self.yoshi.tongue.tip;
            self.yoshi_sound("MSD_SE_YO_TONGUE_GOKKUN", p);
        }
    }

    /// `thinkEat`.
    fn yoshi_think_eat(&mut self) {
        if let Some(kind) = self.yoshi.tongue.in_mouth.take() {
            self.yoshi_do_eat(kind);
        }
    }

    /// `TYoshi::movement`, after Mario's own.
    pub(crate) fn yoshi_movement(&mut self, host: &mut dyn Host) {
        if self.yoshi.juice > 0 {
            self.yoshi.juice -= 1;
        }
        self.yoshi_think_eat();

        match self.yoshi.state {
            YoshiState::Hatching => {
                if self.yoshi.ends_next() {
                    self.yoshi.state = YoshiState::Unmounted;
                    self.yoshi_change_anim(clip::WAIT_ALONE);
                    let p = self.yoshi.pos;
                    self.yoshi_sound("MSD_SE_YV_YOSHI1", p);
                }
            }
            YoshiState::Unmounted => {
                if self.yoshi.ends_next() {
                    self.yoshi_set_clip(clip::WAIT_ALONE);
                }
                let at = self.yoshi.pos + Vec3::Y * 200.0;
                let g = host.ground(at);
                self.yoshi.vel_y -= self.params.jump.gravity;
                self.yoshi.pos.y += self.yoshi.vel_y;
                match g {
                    Some(g) if g.height > self.yoshi.pos.y => {
                        if g.flags
                            .has(SurfaceFlags::ILLEGAL | SurfaceFlags::WATER_SURFACE)
                        {
                            self.yoshi_disappear();
                        } else {
                            self.yoshi.pos.y = g.height;
                            self.yoshi.vel_y = 0.0;
                        }
                    }
                    None => {
                        self.yoshi_disappear();
                    }
                    _ => {}
                }
                if self.yoshi.state == YoshiState::Unmounted {
                    self.yoshi_search(host);
                    if self.yoshi.juice <= 0 {
                        self.yoshi_disappear();
                    }
                }
            }
            YoshiState::Mounted => {
                self.yoshi.pos = self.pos;
                self.yoshi.yaw = self.face_yaw;
                if self.pad.b && !self.pad_prev.b {
                    self.emit_tongue(host);
                }
                // Riding onto deep water (the sim has no swimming: a water-surface floor).
                if self.ground.flags.has(SurfaceFlags::WATER_SURFACE)
                    && self.pos.y <= self.floor_y + 1.0
                {
                    self.in_water = true;
                    self.yoshi_disappear();
                    self.in_water = false;
                } else if self.yoshi.juice <= 0 {
                    self.yoshi_disappear();
                }
            }
            YoshiState::Drowning => {
                if self.yoshi.ends_next() {
                    self.yoshi.state = YoshiState::Gone;
                }
            }
            YoshiState::Dying => {
                self.yoshi.state = YoshiState::Gone;
            }
            YoshiState::Gone => {
                self.yoshi.state = YoshiState::Egg;
                self.yoshi.tongue = Tongue::default();
                if let Some(i) = self.yoshi.egg {
                    self.restart_egg(i);
                }
            }
            YoshiState::Egg => {}
        }

        if self.yoshi.is_hatched() {
            let target = BODY_COLORS[self.yoshi.color as usize];
            let target = Vec3::new(target[0] as f32, target[1] as f32, target[2] as f32);
            self.yoshi.body_color += (target - self.yoshi.body_color) * COLOR_RATE;
            let mut events = std::mem::take(&mut self.events);
            self.yoshi.tongue.movement(host, &mut events);
            self.events = events;
            // `entry`: in the last 5 s the juice colour is gone already.
            if self.yoshi.juice < 600 {
                self.yoshi.color = 0;
            }
        }
    }

    /// `thinkAnimation`: Yoshi's clip from Mario's status while ridden.
    fn yoshi_think_animation(&mut self) {
        use Status::*;
        let y = &self.yoshi;
        let cur = if y.clip == clip::WALK {
            clip::RUN
        } else {
            y.clip
        };
        let mut rate = self.anim.rate * 0.5;
        let new = if self.status.kind() == crate::mario::Kind::Running && self.status != Slip {
            if cur == clip::JUMP_FALL {
                clip::JUMP_END
            } else if matches!(self.status, Catch | SlipFore | SlipBack) {
                clip::SLIDE_POSE
            } else {
                clip::RUN
            }
        } else if self.status.is_jumping() {
            if y.flutter == 1 {
                clip::HOLD_JUMP
            } else if self.status == HipDrop && self.status_state < 2 {
                clip::HIP_START
            } else if self.status == HipDrop && self.status_state < 4 {
                clip::HIP_POSE
            } else if self.vel.y > 0.0 {
                clip::JUMP
            } else {
                clip::JUMP_FALL
            }
        } else if matches!(self.status, CatchLost | BrakeEnd | SlipEnd) {
            clip::SLIDE_END
        } else if self.status == HipAttackEnd {
            clip::HIP_END
        } else {
            clip::WAIT
        };
        if new != cur {
            self.yoshi_set_clip(new);
            self.yoshi_think_btp(new);
        }
        if new == clip::RUN {
            let (lo, hi) = RUN_BLEND;
            let ratio = ((self.forward_vel - lo) / (hi - lo)).clamp(0.0, 1.0);
            self.yoshi.blend = 1.0 - ratio;
            rate = RUN_RATE.0 + RUN_RATE.1 * self.forward_vel;
        } else {
            self.yoshi.blend = 0.0;
        }
        self.yoshi.ctrl.rate = rate;
    }

    /// `thinkUpper`: the head eats while the tongue is out or juice is spraying.
    fn yoshi_think_upper(&mut self) {
        if !self.on_yoshi() {
            return;
        }
        if let Some((_, c)) = self.yoshi.eat.as_mut() {
            c.update();
        }
        let busy = self.yoshi.tongue.state != TongueState::Idle
            || self
                .fludd
                .as_ref()
                .is_some_and(|f| f.can_spray(&self.params));
        let cur = self.yoshi.eat.map(|e| e.0);
        let start = |c: u8, t: &AnimTable| {
            let mut f = FrameCtrl::new(t.get(CLIPS[c as usize]));
            f.rate = 1.0;
            (c, f)
        };
        if busy {
            if cur != Some(clip::EAT) {
                self.yoshi.eat = Some(start(clip::EAT, &self.anims));
            }
        } else if cur == Some(clip::EAT) {
            self.yoshi.eat = Some(start(clip::EAT_END, &self.anims));
        } else if let Some((clip::EAT_END, c)) = self.yoshi.eat
            && c.done
        {
            self.yoshi.eat = None;
        }
    }

    /// `TYoshi::calcAnim`, after Mario's.
    pub(crate) fn yoshi_calc_anim(&mut self) {
        // `TMario::calcAnim` sets the rate first: the run clip follows Mario's.
        self.yoshi.ctrl.rate = if self.yoshi.clip == clip::RUN {
            self.anim.rate * 0.5
        } else {
            0.5
        };
        if self.on_yoshi() {
            self.yoshi_think_animation();
        }
        if !self.yoshi.is_hatched() {
            return;
        }
        self.yoshi_think_upper();
        self.yoshi.ctrl.update();
        self.yoshi.btp_ctrl.update();
        let (head, dir) = self.yoshi.mouth_world();
        self.yoshi.tongue.head = head;
        self.yoshi.tongue.head_dir = dir;
    }
}

/// Clip lengths for Yoshi (`assets/yoshi/*.bck`, `*.btp`) and his egg
/// (`assets/scene/mapobj/eggyoshi_*.bck`), keyed by file stem.
pub fn insert_yoshi_clips(
    table: &mut AnimTable,
    yoshi: &std::path::Path,
    mapobj: &std::path::Path,
) {
    for (dir, prefix) in [(yoshi, "yoshi_"), (mapobj, "eggyoshi_")] {
        let Ok(rd) = std::fs::read_dir(dir) else {
            continue;
        };
        for e in rd.flatten() {
            let path = e.path();
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if !stem.starts_with(prefix) {
                continue;
            }
            let info = match path.extension().and_then(|s| s.to_str()) {
                Some("bck") => std::fs::read(&path)
                    .ok()
                    .and_then(|b| AnimTable::parse_bck(&b)),
                Some("btp") => std::fs::read(&path).ok().and_then(|b| parse_btp(&b)),
                _ => None,
            };
            if let Some(info) = info {
                table.insert(stem, info);
            }
        }
    }
}

/// Length and loop mode of a BTP (`J3D1btp1`, first chunk `TPT1`).
fn parse_btp(d: &[u8]) -> Option<crate::anim::ClipInfo> {
    if d.get(0..8)? != b"J3D1btp1" || d.get(0x20..0x24)? != b"TPT1" {
        return None;
    }
    let mode = match d.get(0x28)? {
        0 => crate::anim::LoopMode::Once,
        1 => crate::anim::LoopMode::OnceReset,
        2 => crate::anim::LoopMode::Loop,
        3 => crate::anim::LoopMode::MirrorOnce,
        4 => crate::anim::LoopMode::MirrorLoop,
        _ => return None,
    };
    let frames = u16::from_be_bytes([*d.get(0x2a)?, *d.get(0x2b)?]) as f32;
    Some(crate::anim::ClipInfo { frames, mode })
}
