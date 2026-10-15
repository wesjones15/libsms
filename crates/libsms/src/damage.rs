//! Getting hurt and healing (Player/MarioCollision.cpp `damageExec`, `decHP`, `incHP`,
//! `loserExec`; MarioReceiveMsg.cpp `receiveMessage`, `getCoin*`; MarioRun.cpp `loserDown`;
//! MarioMove.cpp `thinkParams`).
//!
//! Mario has `mHPMax` (8) health. A hit knocks him over facing the attacker or away from it
//! (the `*_BACK_DOWN` / `*_FORE_DOWN` statuses, by the hit's `mDownType`), takes its
//! `mDamage`, and leaves him invincible for `mInvincibleTime` ticks while he flashes pink. On
//! Yoshi a hit costs no health: Mario is thrown off and Yoshi cries out. At 0 health he falls
//! down for good (`DOWN_LOSER`) and the host decides what happens next. Coins heal: a gold
//! coin 1, a red or blue one 2.
//!
//! Hosts call [`Mario::damage`] when one of their enemies touches Mario (the game's
//! `HIT_MESSAGE_ATTACK` from the enemy) and [`Mario::get_coin`] when he touches a coin.

use glam::Vec3;

use crate::event::{Event, SoundCue};
use crate::mario::{Mario, Status, UpperState};
use crate::math::{diff, matan};
use crate::params::Damage;

/// Ticks a host's knock-down keeps the next from landing, at most (the disc's own flash is 300).
const KNOCK_DOWN_GUARD: i16 = 120;

/// Which `/Mario/Dmg*.prm` a hit uses: `receiveMessage` picks it by the attacker's actor type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum DamageKind {
    /// Most enemies (`mDmgParamsEnemyCommon`).
    EnemyCommon,
    /// The Strollin' Stu (`THamuKuri`): a stumble, 1 damage.
    Hamukuri,
    /// Swoopin' Stu / goop blobs (`TNameKuri`).
    Namekuri,
    /// Fire Stu (`THinoKuri`): 3 damage, costs water.
    Hinokuri,
    Fire,
    BgTentacle,
    BossEel,
    HanachanBoss,
    Poihana,
    /// Bullet Bill (`TKiller`).
    Killer,
    LampTrapIron,
    LampTrapSpike,
    /// Shadow Mario (`TEnemyMario`).
    EnemyMario,
}

impl DamageKind {
    pub const ALL: [DamageKind; 13] = [
        DamageKind::EnemyCommon,
        DamageKind::Hamukuri,
        DamageKind::Namekuri,
        DamageKind::Hinokuri,
        DamageKind::Fire,
        DamageKind::BgTentacle,
        DamageKind::BossEel,
        DamageKind::HanachanBoss,
        DamageKind::Poihana,
        DamageKind::Killer,
        DamageKind::LampTrapIron,
        DamageKind::LampTrapSpike,
        DamageKind::EnemyMario,
    ];
}

/// Coins (`TCoin`, `TCoinRed`, `TCoinBlue`; Item.cpp).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Coin {
    Gold,
    Red,
    Blue,
}

/// `damageExec`'s status table, by `mDownType + 4 * airborne + 8 * hit from behind`.
const DOWNS: [Status; 16] = [
    Status::SafeBackDown,
    Status::ShortBackDown,
    Status::BackDown,
    Status::Wait,
    Status::JumpShortBackDown,
    Status::JumpShortBackDown,
    Status::JumpBackDown,
    Status::Wait,
    Status::SafeForeDown,
    Status::ShortForeDown,
    Status::ForeDown,
    Status::Wait,
    Status::JumpShortForeDown,
    Status::JumpShortForeDown,
    Status::JumpForeDown,
    Status::Wait,
];

impl Mario {
    pub(crate) fn cue(&mut self, name: &'static str) {
        self.events.push(Event::Sound(SoundCue {
            name,
            pos: self.pos,
            info: None,
        }));
    }

    /// `isInvincible`: still flashing from the last hit.
    pub fn is_invincible(&self) -> bool {
        self.invincible > 0
    }

    /// Down for good (`MARIO_FLAG_GAME_OVER`).
    pub fn is_down(&self) -> bool {
        self.status == Status::DownLoser
    }

    /// An enemy at `from` hits Mario with one of the disc's damage kinds (`receiveMessage`,
    /// `HIT_MESSAGE_ATTACK`). Returns whether it landed.
    pub fn damage(&mut self, from: Vec3, kind: DamageKind) -> bool {
        let p = self.params.damage(kind).clone();
        self.damage_by(from, &p)
    }

    /// `damageExec` with any damage parameters.
    pub fn damage_by(&mut self, from: Vec3, p: &Damage) -> bool {
        self.hurt(from, p, true)
    }

    /// A host's hit that costs the sim's health nothing (the host keeps the health): the
    /// knock-down of `damageExec` from `from`, with `kind`'s fall, speed and flash, its voice
    /// (the big one) and the pink flash, but no `decHP` and no water lost. Returns whether it
    /// landed (not while he is flashing from the last, down for good, or on Yoshi, who takes
    /// the hit as in the game). A dive isn't knocked out of, as in the game.
    pub fn knock_down(&mut self, from: Vec3, kind: DamageKind) -> bool {
        let mut p = self.params.damage(kind).clone();
        p.damage = 0;
        p.water_emit = 0;
        // Only there so one hit isn't followed by another before he's up.
        p.invincible_time = p.invincible_time.min(KNOCK_DOWN_GUARD);
        self.hurt(from, &p, false)
    }

    /// `damageExec`; `hp` false leaves the health and the water alone (a host's hit).
    fn hurt(&mut self, from: Vec3, p: &Damage, hp: bool) -> bool {
        if self.is_invincible() || self.is_down() || self.health <= 0 {
            return false;
        }
        if self.on_yoshi() {
            // Yoshi takes the hit: Mario is thrown off (`getOffYoshi(true)`), Yoshi plays his
            // damage clip and cries (`TYoshi::getOff`). No health is lost and no invincibility
            // starts. The game also starts `YV_DAMAGE` here at Mario; once is enough.
            self.get_off_yoshi(true);
            self.events.push(Event::Damaged {
                damage: 0,
                health: self.health,
                yoshi: true,
            });
            return true;
        }
        let airborne = self.status.is_jumping() as usize;
        // `canPlayAnimation`: a dive (`MARIO_STATUS_DIVE`) isn't knocked out of.
        if p.down_type == 3 || self.status == Status::JumpCatch {
            // `setUpperDamageRun`: a flinch with no knock-back; not drawn here.
        } else {
            let attack = matan(from.z - self.pos.z, from.x - self.pos.x);
            let turn = diff(attack, self.face_yaw);
            self.face_yaw = attack;
            if self.forward_vel < p.min_speed {
                self.set_player_velocity(p.min_speed);
            }
            let behind = if (-0x4000..=0x4000).contains(&turn) {
                self.forward_vel = -self.forward_vel;
                0
            } else {
                self.face_yaw = self.face_yaw.wrapping_add(i16::MIN);
                1
            };
            self.model_yaw = self.face_yaw;
            let to = DOWNS[(p.down_type.min(3) as usize) + airborne * 4 + behind * 8];
            // `changePlayerDropping`: let go of what he carries first.
            self.drop_object();
            if self.upper == UpperState::HoldingObject {
                self.upper = UpperState::Idle;
            }
            self.change_status(to, 0, false);
        }
        self.invincible = p.invincible_time;
        if hp {
            self.dec_hp(p.damage as i16);
        }
        let lost = p.water_emit as i32;
        let params = self.params.clone();
        if let Some(f) = self.fludd.as_mut() {
            // `TWaterGun::damage`, once per burst; the bursts' water itself isn't emitted.
            let loss = params.nozzle(f.nozzle).damage_loss;
            for _ in 0..lost {
                if f.water > 0 {
                    f.water = (f.water - loss).max(0);
                }
            }
        }
        if self.health > 0 {
            self.cue(if p.damage > 1 || !hp {
                "MSD_SE_MV01_DAMAGE_BIG_01"
            } else if p.down_type == 0 {
                "MSD_SE_MV03_DAMAGE_LITLE_01"
            } else {
                "MSD_SE_MV02_DAMAGE_MID_01"
            });
        }
        self.events.push(Event::Damaged {
            damage: p.damage,
            health: self.health,
            yoshi: false,
        });
        true
    }

    /// `incHP`: heals up to `mHPMax`, with the recovery chime when it did.
    pub fn inc_hp(&mut self, hp: i16) {
        let max = self.params.mario.hp_max;
        self.health += hp;
        if self.health > max {
            self.health = max;
        } else {
            self.cue("MSD_SE_SY_HP_RECOVER");
        }
    }

    /// `decHP`.
    fn dec_hp(&mut self, hp: i16) {
        self.health -= hp;
        if self.health <= 0 {
            self.health = 0;
            self.loser_exec();
        }
    }

    /// Out of health from the host's side (its own health ran out): Mario falls down for good
    /// as at 0 health (`loserExec`), with its sound and voices. Nothing if he's already down.
    pub fn knock_out(&mut self) {
        self.loser_exec();
    }

    /// `loserExec`: out of health. Yoshi, if any, runs off.
    fn loser_exec(&mut self) {
        if self.is_down() {
            return;
        }
        self.health = 0;
        self.cue("MSD_SE_SY_HP_DOWN");
        if self.yoshi.is_hatched() {
            self.yoshi_disappear();
        }
        self.change_status(Status::DownLoser, 0, true);
        self.events.push(Event::Down);
    }

    /// `loserDown`: slides to a stop lying down, then the two "down" voices.
    pub(crate) fn loser_down(&mut self) -> bool {
        // A hit in the air ends here too; the game leaves him where he is, we let him drop to
        // the floor first.
        if self.pos.y > self.floor_y {
            self.vel.y -= self.params.jump.gravity;
            self.pos.y = (self.pos.y + 0.25 * self.vel.y).max(self.floor_y);
        } else {
            self.vel.y = 0.0;
        }
        self.slope_process();
        self.forward_vel *= 0.9;
        if self.forward_vel * self.forward_vel < 1.0 {
            self.set_player_velocity(0.0);
        }
        self.set_anim("die", 1.0);
        match self.status_state {
            0 => {
                self.cue("MSD_SE_MV08A_DOWN_01");
                self.status_state = 1;
                self.status_timer = 0;
            }
            // The game waits for the first voice to end; about a second.
            1 => {
                self.status_timer += 1;
                if self.status_timer > 120 {
                    self.status_timer = 0;
                    self.status_state = 2;
                }
            }
            2 => {
                self.status_timer += 1;
                if self.status_timer > 60 {
                    self.status_state = 3;
                }
            }
            3 => {
                self.cue("MSD_SE_MV08B_DOWN_REACT_01");
                self.status_state = 4;
            }
            _ => {}
        }
        false
    }

    /// Mario touched a coin (`TCoin::taken` then `getCoin` / `getCoinRed` / `getCoinBlue`):
    /// the coin's sound, and health.
    pub fn get_coin(&mut self, coin: Coin) {
        // MarioReceiveMsg.cpp getCoin* -> MarioParticle.cpp emitGetCoinEffect.
        self.fx_once("ms_coinget_a", 0);
        self.fx_once("ms_coinget_b", 0);
        match coin {
            Coin::Gold => {
                self.coins += 1;
                self.cue("MSD_SE_SY_COIN");
                self.inc_hp(1);
            }
            Coin::Red => {
                self.cue("MSD_SE_SY_RED_COIN_GET");
                self.inc_hp(2);
            }
            Coin::Blue => {
                self.cue("MSD_SE_SY_BLUE_COIN_GET");
                self.inc_hp(2);
            }
        }
    }

    /// `thinkParams`: the invincibility countdown.
    pub(crate) fn think_params(&mut self) {
        if self.invincible > 0 {
            self.invincible -= 1;
        }
    }

    /// Full health and no flashing (`TMario::initValues` on a new life).
    pub fn restore(&mut self) {
        self.health = self.params.mario.hp_max;
        self.invincible = 0;
    }
}
