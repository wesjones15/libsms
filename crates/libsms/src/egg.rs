//! Yoshi's egg (`TEggYoshi`, MoveBG/Item.cpp; `eggYoshi` in MapObjInit.cpp).
//!
//! The egg wants one fruit, shown in its thought bubble (`decideRandomLoveFruit`: banana,
//! papaya, pine or coconut). Touching it with fruit, held by Mario or loose, plays
//! `eggyoshi_true` for the wanted kind and `eggyoshi_false` for any other. When `true` ends the
//! egg breaks (`eggyoshi_broken`) and Yoshi hatches from it, eating that fruit
//! (`TYoshi::appearFromEgg`); the broken shell vanishes when its clip ends. When Yoshi runs out
//! of juice or drowns, the egg comes back where it was with a new wish (`startFruit`).
//!
//! Touching is the game's hit check against the egg's cylinder: radius 300, height 100
//! (`eggYoshi_hit_data_table`).

use glam::Vec3;

use crate::anim::{AnimTable, FrameCtrl};
use crate::event::{Event, SoundCue};
use crate::host::{Fruit, Host};
use crate::mario::Mario;
use crate::math::{Angle, matan};

/// `eggYoshi_hit_data_table`: the cylinder fruit has to reach.
pub const TOUCH_RADIUS: f32 = 300.0;
pub const TOUCH_HEIGHT: f32 = 100.0;

/// The egg's clips (`eggYoshi_anim_data`, by `startAnim` index).
pub const CLIPS: [&str; 4] = [
    "eggyoshi_wait",
    "eggyoshi_true",
    "eggyoshi_false",
    "eggyoshi_broken",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EggState {
    /// Waiting for fruit (`STATE_NORMAL`).
    Waiting,
    /// Shaking its head at the wrong fruit (state 0xD).
    Wrong,
    /// Got the right fruit (0xB).
    Right,
    /// Breaking open as Yoshi appears (0xC).
    Hatching,
    /// Gone until Yoshi does (`STATE_DEAD`).
    Gone,
}

#[derive(Debug, Clone)]
pub struct Egg {
    /// Where it was placed; it comes back here.
    pub pos: Vec3,
    /// Facing (`mRotation.y`); turned toward the fruit that hatches it.
    pub yaw: Angle,
    pub state: EggState,
    /// The fruit it wants (`unk14C`).
    pub wants: Fruit,
    /// Index into [`CLIPS`] and its frame controller.
    pub clip: usize,
    pub ctrl: FrameCtrl,
    /// The bubble's texture frame (`eggyoshi_fukidashi.btp`): the wanted fruit, or a
    /// heart for the right one and a cross for a wrong one.
    pub bubble_frame: f32,
    /// The fruit that hatches it (`unk150`), by host id.
    fruit: Option<(u64, Fruit)>,
}

/// What to draw for an egg.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EggLook {
    pub pos: Vec3,
    pub yaw: Angle,
    /// `eggYoshi_broken.bmd` instead of `eggYoshi_normal.bmd`.
    pub broken: bool,
    pub clip: &'static str,
    pub frame: f32,
    /// The thought bubble and its texture frame, when shown.
    pub bubble: Option<f32>,
}

impl Egg {
    pub fn new(pos: Vec3, yaw: Angle, wants: Fruit, table: &AnimTable) -> Egg {
        let mut e = Egg {
            pos,
            yaw,
            state: EggState::Waiting,
            wants,
            clip: 0,
            ctrl: FrameCtrl::new(table.get(CLIPS[0])),
            bubble_frame: 0.0,
            fruit: None,
        };
        e.start_anim(0, table);
        e.start_balloon_anim();
        e
    }

    /// `TMapObjBase::startAnim`: the clip at 0.5 frames a tick (60 fps).
    fn start_anim(&mut self, i: usize, table: &AnimTable) {
        self.clip = i;
        self.ctrl = FrameCtrl::new(table.get(CLIPS[i]));
        self.ctrl.rate = 0.5;
    }

    /// `animIsFinished` (`curAnmEndsNext`).
    fn anim_finished(&self) -> bool {
        self.ctrl.frame + self.ctrl.rate + 0.1 >= self.ctrl.end
    }

    /// `startBalloonAnim`: the bubble shows the wanted fruit.
    fn start_balloon_anim(&mut self) {
        self.bubble_frame = match self.wants {
            Fruit::Banana => 1.0,
            Fruit::Durian => 3.0,
            Fruit::Papaya => 5.0,
            Fruit::Pine => 7.0,
            Fruit::Coconut => 9.0,
        };
    }

    pub fn look(&self) -> Option<EggLook> {
        if self.state == EggState::Gone {
            return None;
        }
        Some(EggLook {
            pos: self.pos,
            yaw: self.yaw,
            broken: self.clip == 3,
            clip: CLIPS[self.clip],
            frame: self.ctrl.frame,
            bubble: (self.state != EggState::Hatching).then_some(self.bubble_frame),
        })
    }

    /// Does a cylinder at `p` (bottom) of `height` reach the egg's?
    fn touches(&self, p: Vec3, height: f32) -> bool {
        let d = p - self.pos;
        (d.x * d.x + d.z * d.z).sqrt() < TOUCH_RADIUS
            && p.y < self.pos.y + TOUCH_HEIGHT
            && p.y + height > self.pos.y
    }
}

impl Mario {
    /// Places a Yoshi egg (the stage's `eggYoshi` object) and returns its index.
    pub fn add_egg(&mut self, pos: Vec3, yaw: Angle) -> usize {
        let wants = self.random_love_fruit();
        self.eggs.push(Egg::new(pos, yaw, wants, &self.anims));
        self.eggs.len() - 1
    }

    /// `decideRandomLoveFruit` (the stage-specific overrides aside).
    fn random_love_fruit(&mut self) -> Fruit {
        match (4.0 * self.rand()) as i32 {
            0 => Fruit::Banana,
            1 => Fruit::Papaya,
            2 => Fruit::Pine,
            _ => Fruit::Coconut,
        }
    }

    /// `startFruit` (`HIT_MESSAGE_UNK10`): the egg is back with a new wish.
    pub(crate) fn restart_egg(&mut self, i: usize) {
        let wants = self.random_love_fruit();
        let table = std::mem::take(&mut self.anims);
        if let Some(e) = self.eggs.get_mut(i) {
            e.wants = wants;
            e.fruit = None;
            e.state = EggState::Waiting;
            e.start_anim(0, &table);
            e.start_balloon_anim();
        }
        self.anims = table;
    }

    /// `touchFruit`.
    fn egg_touch_fruit(&mut self, i: usize, id: u64, kind: Fruit, at: Vec3) {
        let table = std::mem::take(&mut self.anims);
        let e = &mut self.eggs[i];
        let mut sound = None;
        if kind == e.wants {
            e.start_anim(1, &table);
            e.bubble_frame = 11.0;
            e.yaw = matan(at.z - e.pos.z, at.x - e.pos.x);
            e.state = EggState::Right;
            e.fruit = Some((id, kind));
            sound = Some("MSD_SE_SY_COLLECT_PRETTY");
        } else if e.anim_finished() {
            e.start_anim(2, &table);
            e.bubble_frame = 12.0;
            e.state = EggState::Wrong;
            sound = Some("MSD_SE_SY_NOT_COLLECT_YOSHI");
        }
        let pos = e.pos;
        self.anims = table;
        if let Some(name) = sound {
            self.events.push(Event::Sound(SoundCue {
                name,
                pos,
                info: None,
            }));
        }
    }

    /// One tick of every egg: `touchActor` from the hit check, then `control` and the clip.
    pub(crate) fn eggs_tick(&mut self, host: &mut dyn Host) {
        for i in 0..self.eggs.len() {
            if matches!(self.eggs[i].state, EggState::Waiting | EggState::Wrong) {
                // Mario carrying fruit into it.
                if let (Some(id), Some(kind)) = (self.held, self.held_fruit)
                    && self.eggs[i].touches(self.pos, crate::mario::HEIGHT)
                {
                    self.egg_touch_fruit(i, id, kind, self.pos);
                }
            }
            if matches!(self.eggs[i].state, EggState::Waiting | EggState::Wrong) {
                // Loose fruit (thrown, or lying in reach).
                let mut found = Vec::new();
                host.takeables(self.eggs[i].pos, TOUCH_RADIUS, &mut found);
                for t in found {
                    if let Some(kind) = t.fruit
                        && self.eggs[i].touches(t.pos, 2.0 * t.radius)
                        && matches!(self.eggs[i].state, EggState::Waiting | EggState::Wrong)
                    {
                        self.egg_touch_fruit(i, t.id, kind, t.pos);
                    }
                }
            }

            let table = std::mem::take(&mut self.anims);
            let mut hatch = None;
            let e = &mut self.eggs[i];
            match e.state {
                EggState::Wrong if e.anim_finished() => {
                    e.start_anim(0, &table);
                    e.start_balloon_anim();
                    e.state = EggState::Waiting;
                }
                EggState::Right if e.anim_finished() => {
                    e.start_anim(3, &table);
                    hatch = Some((e.pos, e.yaw, e.fruit));
                    e.state = EggState::Hatching;
                }
                EggState::Hatching if e.anim_finished() => {
                    e.state = EggState::Gone;
                }
                _ => {}
            }
            e.ctrl.update();
            self.anims = table;
            if let Some((pos, yaw, fruit)) = hatch
                && !self.yoshi.is_hatched()
            {
                self.appear_from_egg(pos, yaw, i, fruit);
            }
        }
    }
}
