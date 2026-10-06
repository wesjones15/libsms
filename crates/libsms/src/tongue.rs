//! Yoshi's tongue (`TYoshiTongue`, Player/Tongue.cpp).
//!
//! B on Yoshi (or Yoshi alone spotting fruit) shoots the tip out of his `null_tongue` joint at
//! `mInitialSpeed` along the joint's X axis plus half Mario's velocity. While it flies it homes
//! on the nearest fruit in reach and closes on it at `mExtendAmount` of the gap a tick; within
//! 200 it takes the fruit (`HIT_MESSAGE_TAKE`) and starts back. Reeling in, the tip closes
//! `mRetractAmount` of the way each tick, the fruit shrinks by `mElasticity` a tick, and once
//! the tip is within `mRetractedLength` of the mouth the fruit is swallowed (`mActorTypeInMouth`,
//! which `TYoshi::thinkEat` turns into a juice colour). It gives up after `mMaxProgress` ticks,
//! when it would pass behind the mouth, or when it touches a wall.
//!
//! Its hit cylinder is the game's way to find targets: radius 300 at rest, 1000 while flying,
//! 10 reeling in (`mAttackRadius`, `calcEntryRadius`). Hosts answer through
//! [`Host::takeables`] (fruit) and, when Mario shoots it, [`Host::enemies`] too
//! (`allowExtra`); an enemy that lets itself be taken is swallowed like fruit, without the
//! juice.

use glam::Vec3;

use crate::event::{Event, SoundCue};
use crate::host::{Fruit, Host};

/// `mInitialSpeed`.
pub const INITIAL_SPEED: f32 = 10.0;
/// `mExtendAmount`: share of the gap to a target closed each tick.
pub const EXTEND_AMOUNT: f32 = 0.05;
/// `mRetractAmount`: share of the length kept each tick reeling in.
pub const RETRACT_AMOUNT: f32 = 0.7;
/// `mRetractedLength`: back in the mouth.
pub const RETRACTED_LENGTH: f32 = 80.0;
/// `mMaxReach` (500 + `mRetractedLength`).
pub const MAX_REACH: f32 = 500.0 + RETRACTED_LENGTH;
/// `mElasticity`: the caught fruit's scale per tick reeling in.
pub const ELASTICITY: f32 = 0.8;
/// `mMaxProgress`: ticks before it gives up.
pub const MAX_PROGRESS: u16 = 60;
/// `mAttackRadius` by state (`movement`).
pub const RADIUS_IDLE: f32 = 300.0;
pub const RADIUS_FLYING: f32 = 1000.0;
pub const RADIUS_REELING: f32 = 10.0;
/// Ticks a grabbed fruit keeps the tip's momentum before reeling in.
const GRAB_TICKS: u16 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TongueState {
    #[default]
    Idle,
    Extending,
    /// Holding a fruit, still coasting (`STATE_GRABBED`).
    Grabbed,
    Retracting,
}

/// What is on the tongue (`mHeldObject`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Held {
    /// A takeable object (fruit), by host id.
    Object { id: u64, fruit: Option<Fruit> },
    /// An enemy, by host id.
    Enemy { id: u64 },
}

/// A target `findTarget` picked, and where the tongue aims on it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Target {
    pub held: Held,
    pub pos: Vec3,
    pub aim: Vec3,
}

#[derive(Debug, Clone, Default)]
pub struct Tongue {
    pub state: TongueState,
    /// The mouth (`mHeadPos`) and the joint's X axis (`mHeadDir`).
    pub head: Vec3,
    pub head_dir: Vec3,
    /// The tip (`mTipPos`).
    pub tip: Vec3,
    velocity: Vec3,
    progress: u16,
    /// What is on the tip.
    pub held: Option<Held>,
    /// The held fruit's scale (`mHeldObject->mScaling`).
    pub held_scale: f32,
    /// What was just swallowed (`mActorTypeInMouth`).
    pub(crate) in_mouth: Option<Option<Fruit>>,
}

impl Tongue {
    /// `mAttackRadius` for the current state.
    pub fn reach(&self) -> f32 {
        match self.state {
            TongueState::Idle | TongueState::Grabbed => RADIUS_IDLE,
            TongueState::Extending => RADIUS_FLYING,
            TongueState::Retracting => RADIUS_REELING,
        }
    }

    /// `emit`: shoot from `src` along `dir`, carrying half of `vel`.
    pub fn emit(&mut self, src: Vec3, dir: Vec3, vel: Vec3) {
        if self.state != TongueState::Idle {
            return;
        }
        self.progress = 0;
        self.state = TongueState::Extending;
        self.tip = src;
        self.head = src;
        self.head_dir = dir;
        self.velocity = dir * INITIAL_SPEED + vel * 0.5;
        self.velocity.y = self.velocity.y.clamp(-50.0, 50.0);
    }

    /// `canGo`: the tip stays in front of the mouth and out of walls, floors and ceilings.
    fn can_go(&mut self, host: &dyn Host) -> bool {
        if (self.tip - self.head).dot(self.head_dir) < 0.0 {
            return false;
        }
        let mut walls = Vec::new();
        let c = host.walls(self.tip + Vec3::Y * 10.0, 50.0, 1, &mut walls);
        if !walls.is_empty() {
            self.tip.x = c.x;
            self.tip.z = c.z;
            return false;
        }
        if let Some(g) = host.ground(self.tip)
            && g.height + 50.0 > self.tip.y
        {
            self.tip.y = g.height + 50.0;
            return true;
        }
        if let Some(r) = host.roof(self.tip)
            && r.height - 50.0 < self.tip.y
        {
            self.tip.y = r.height - 50.0;
        }
        true
    }

    /// `findTarget`: the nearest fruit in reach of the tip, and with `enemies` the nearest
    /// enemy too (`allowExtra`), optionally only ahead of the mouth (within 60 degrees).
    pub(crate) fn find_target(
        &self,
        host: &dyn Host,
        enemies: bool,
        forward: bool,
    ) -> Option<Target> {
        let mut candidates = Vec::new();
        let mut found = Vec::new();
        host.takeables(self.tip, self.reach(), &mut found);
        for t in found {
            if t.fruit.is_some() {
                // Aim at the middle of its height (`mDamageHeight * 0.5`).
                candidates.push(Target {
                    held: Held::Object {
                        id: t.id,
                        fruit: t.fruit,
                    },
                    pos: t.pos,
                    aim: t.pos + Vec3::Y * t.radius,
                });
            }
        }
        if enemies {
            let mut found = Vec::new();
            host.enemies(self.tip, self.reach(), &mut found);
            for e in found {
                candidates.push(Target {
                    held: Held::Enemy { id: e.id },
                    pos: e.pos,
                    aim: e.pos + Vec3::Y * (0.5 * e.height),
                });
            }
        }
        let mut best: Option<(f32, Target)> = None;
        for t in candidates {
            let d = t.aim - self.tip;
            if d == Vec3::ZERO {
                continue;
            }
            let dist = d.length();
            if forward && d.normalize().dot(self.head_dir) <= 0.5 {
                continue;
            }
            if best.as_ref().is_none_or(|b| dist < b.0) {
                best = Some((dist, t));
            }
        }
        best.map(|b| b.1)
    }

    /// `movement`. `events` receives the grab sound and the swallowed fruit.
    pub(crate) fn movement(&mut self, host: &mut dyn Host, events: &mut Vec<Event>) {
        match self.state {
            TongueState::Extending => {
                self.progress += 1;
                if self.progress >= MAX_PROGRESS || !self.can_go(host) {
                    self.state = TongueState::Retracting;
                }
            }
            TongueState::Retracting => {
                let len = (self.tip - self.head).length();
                if self.held.is_some() && len < MAX_REACH {
                    self.held_scale = (self.held_scale * ELASTICITY).max(0.01);
                }
                if len < RETRACTED_LENGTH {
                    // HIT_MESSAGE_UNK8 then UNKB: let go and vanish.
                    match self.held.take() {
                        Some(Held::Object { id, fruit }) => {
                            self.in_mouth = Some(fruit);
                            events.push(Event::Eaten { id });
                        }
                        Some(Held::Enemy { id }) => {
                            self.in_mouth = Some(None);
                            events.push(Event::EnemyEaten { id });
                        }
                        None => {}
                    }
                    self.state = TongueState::Idle;
                }
            }
            TongueState::Idle | TongueState::Grabbed => {}
        }

        match self.state {
            TongueState::Idle => self.tip = self.head,
            TongueState::Extending => match self.find_target(host, true, true) {
                Some(t) if self.held.is_none() => {
                    let step = (t.aim - self.tip) * EXTEND_AMOUNT;
                    self.tip += step;
                    self.velocity = step;
                    let taken = (t.aim - self.tip).length() < 200.0
                        && match t.held {
                            Held::Object { id, .. } => host.take(id),
                            Held::Enemy { id } => host.take_enemy(id),
                        };
                    if taken {
                        self.held = Some(t.held);
                        self.held_scale = 1.0;
                        events.push(Event::Sound(SoundCue {
                            name: "MSD_SE_YV_PERON",
                            pos: self.tip,
                            info: None,
                        }));
                        self.progress = 0;
                        self.state = TongueState::Grabbed;
                    }
                }
                _ => self.tip += self.velocity,
            },
            TongueState::Grabbed => {
                self.tip += self.velocity;
                self.progress += 1;
                if self.progress > GRAB_TICKS {
                    self.state = TongueState::Retracting;
                }
            }
            TongueState::Retracting => {
                self.tip = self.head + (self.tip - self.head) * RETRACT_AMOUNT;
            }
        }
    }
}

impl Held {
    /// The host's id for it.
    pub fn id(self) -> u64 {
        match self {
            Held::Object { id, .. } | Held::Enemy { id } => id,
        }
    }
}
