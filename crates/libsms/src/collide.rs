//! Mario against enemies: `TMario::checkCollision`'s actor loop and `hitNormal`
//! (Player/MarioCheckCol.cpp), `trampleExec` (Player/MarioCollision.cpp) and `keepDistance`
//! (Player/MarioPhysics.cpp).
//!
//! Sunshine fills Mario's collision list in its hit check (`TObjHitCheck`): every actor whose
//! damage cylinder his attack cylinder (`mTrampleRadius` wide, `mAttackHeight` tall) reaches,
//! up to five. Here the host's [`Host::enemies`] supplies the candidates and the sim tests the
//! cylinders (`checkDistance`). The enemy's side of the exchange (its hit points, its attacks on
//! Mario) is the host's; what Mario sends it comes out as [`Event::EnemyHit`].

use crate::event::{EnemyHitKind, Event};
use crate::host::{Enemy, Host};
use crate::mario::{Mario, Status};

/// `initHitActor(0x80000001, 5, ...)`: Mario's collision list holds five actors.
const MAX_COLLISIONS: usize = 5;

impl Mario {
    /// The enemy part of `checkCollision`: each enemy Mario's attack cylinder reaches gets
    /// `hitNormal`; with [`crate::Options::enemy_push`] he is then kept out of its body.
    pub(crate) fn check_enemy_collision(&mut self, host: &mut dyn Host) {
        if matches!(self.status, Status::DownLoser | Status::WinDemo) {
            return;
        }
        let radius = self.params.mario.trample_radius;
        let height = self.params.mario.attack_height;
        let mut found: Vec<Enemy> = Vec::new();
        host.enemies(self.pos, radius + height, &mut found);
        found.retain(|e| e.reached_by(self.pos, radius, height));
        found.truncate(MAX_COLLISIONS);
        for e in found {
            let stomped = self.hit_normal(host, &e);
            // `keepDistance(actor, 0)`: out of the enemy's cylinder, or knocked back for
            // running into it too fast.
            if self.options.enemy_push && !stomped {
                self.keep_distance(host, e.pos, e.radius);
            }
        }
    }

    /// `hitNormal`: falling onto an enemy from above pounds or stomps it; a dive or slide into
    /// it punches it. Returns whether Mario stomped or pounded it (and so is above it, not
    /// beside it).
    fn hit_normal(&mut self, host: &mut dyn Host, e: &Enemy) -> bool {
        if self.status.is_jumping() && self.vel.y < 0.0 && e.pos.y < self.pos.y {
            if self.status == Status::HipDrop {
                self.report_hit(host, e.id, EnemyHitKind::HipDrop);
                return true;
            }
            if self.trample_exec(host, e) {
                return true;
            }
        }
        // `MARIO_STATUS_CATCH` / `OIL_SLIP` / `JUMP_CATCH`: PUNCH, then TRAMPLE.
        if matches!(self.status, Status::Catch | Status::JumpCatch) {
            let kind = if self.status == Status::JumpCatch && self.corkscrew > 0 {
                EnemyHitKind::Corkscrew
            } else {
                EnemyHitKind::Punch
            };
            self.report_hit(host, e.id, kind);
        }
        false
    }

    /// [`Event::EnemyHit`], once per enemy per status (see `enemies_hit`).
    fn report_hit(&mut self, host: &mut dyn Host, id: u64, kind: EnemyHitKind) {
        if !self.enemies_hit.contains(&id) {
            self.enemies_hit.push(id);
            host.event(Event::EnemyHit { id, kind });
        }
    }

    /// `trampleExec`: bounce off the enemy's head. A long jump turns into a back flip;
    /// otherwise each stomp in a row climbs `step1` → `step2` → `step3` (higher each time,
    /// `mTramplePowStep1..3`). Every enemy accepts the stomp here.
    fn trample_exec(&mut self, host: &mut dyn Host, e: &Enemy) -> bool {
        if !self.status.is_jumping() {
            return false;
        }
        // The bounce changes status, which clears `enemies_hit`: stomping the same enemy
        // again on the way back down counts again.
        self.report_hit(host, e.id, EnemyHitKind::Trample);
        // The stomp's burst.
        for n in ["ms_fumi_a", "ms_fumi_b", "ms_fumi_c"] {
            self.fx_once(n, 0);
        }
        if self.status == Status::BroadJump {
            self.change_status(Status::BackJump, 0, false);
            return true;
        }
        let step: u32 = match self.anim.clip {
            "step1" => 1,
            "step2" | "step3" => 2,
            _ => 0,
        };
        // `setAnimation(ANIM_STEP3)` then `setFrame(0)`: a third stomp restarts the clip.
        if step == 2 && self.anim.clip == "step3" {
            self.anim.frame = 0.0;
        }
        self.set_anim(TRAMPLE_CLIPS[step as usize], 1.0);
        self.change_status(Status::Trample, step, true);
        true
    }
}

/// The stomp clips (`ANIM_STEP1..3`).
pub(crate) const TRAMPLE_CLIPS: [&str; 3] = ["step1", "step2", "step3"];
