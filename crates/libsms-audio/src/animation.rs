//! Animation sound playback, from `JAIAnimeSound::{setAnimSoundActor,playActorAnimSound}`
//! and `MAnmSound::setSpeedModifySound`. The host resets on clip changes and samples the
//! clock often enough to observe each wrap (the viewer does this at 120 Hz).

use crate::{SoundHandle, SoundSystem};
use glam::Vec3;
use libsms_assets::sound::{AnimSound, AnimSoundTable};

#[derive(Default)]
struct Cursor {
    previous: Option<f32>,
    loops: u16,
}

impl Cursor {
    fn crossed(&mut self, table: &AnimSoundTable, frame: f32, speed: f32) -> Vec<AnimSound> {
        let reverse = speed < 0.0;
        let wrapped = self
            .previous
            .is_some_and(|p| if reverse { frame > p } else { frame < p });
        let old_loop = self.loops;
        if wrapped {
            self.loops = self.loops.saturating_add(1).min(256);
        }
        let mut keys: Vec<_> = table
            .sounds
            .iter()
            .copied()
            .filter(|s| {
                if s.flags & (if reverse { 1 } else { 2 }) != 0 {
                    return false;
                }
                let (crossed, lap) = match self.previous {
                    None => (
                        if reverse {
                            s.start_frame >= frame
                        } else {
                            s.start_frame <= frame
                        },
                        self.loops,
                    ),
                    Some(p) if wrapped => {
                        let tail = if reverse {
                            s.start_frame < p
                        } else {
                            s.start_frame > p
                        };
                        let head = if reverse {
                            s.start_frame >= frame
                        } else {
                            s.start_frame <= frame
                        };
                        (tail || head, if tail { old_loop } else { self.loops })
                    }
                    Some(p) => (
                        if reverse {
                            s.start_frame < p && s.start_frame >= frame
                        } else {
                            s.start_frame > p && s.start_frame <= frame
                        },
                        self.loops,
                    ),
                };
                crossed && (s.flags & 8 == 0 || lap == s.loop_count as u16)
            })
            .collect();
        // On wrap, finish the old lap before starting the new one.
        keys.sort_by(|a, b| {
            let tail = |s: &AnimSound| {
                wrapped
                    && self.previous.is_some_and(|p| {
                        if reverse {
                            s.start_frame < p
                        } else {
                            s.start_frame > p
                        }
                    })
            };
            tail(b).cmp(&tail(a)).then_with(|| {
                if reverse {
                    b.start_frame.total_cmp(&a.start_frame)
                } else {
                    a.start_frame.total_cmp(&b.start_frame)
                }
            })
        });
        self.previous = Some(frame);
        keys
    }
}

struct Active {
    key: AnimSound,
    handle: SoundHandle,
}

/// One animated emitter, with the original eight animation sound slots.
#[derive(Default)]
pub struct AnimationPlayer {
    cursor: Cursor,
    active: Vec<Active>,
}

impl AnimationPlayer {
    /// A new clip starts a fresh cue cursor. Flag 4 stops a sound on animation change;
    /// other one-shots finish naturally. Held sounds are no longer refreshed.
    pub fn reset(&mut self, sys: &mut SoundSystem) {
        for a in self.active.drain(..) {
            if a.key.flags & 4 != 0 {
                sys.stop(a.handle, 1);
            }
        }
        self.cursor = Cursor::default();
    }

    /// Stops all animation sounds, for despawn/respawn.
    pub fn stop(&mut self, sys: &mut SoundSystem) {
        for a in self.active.drain(..) {
            sys.stop(a.handle, 0);
        }
        self.cursor = Cursor::default();
    }

    /// `speed` is the BCK frame-controller rate (Mario's authored rate * 0.5).
    pub fn update(
        &mut self,
        sys: &mut SoundSystem,
        table: &AnimSoundTable,
        frame: f32,
        speed: f32,
        pos: Vec3,
        owner: u64,
    ) {
        self.active.retain_mut(|a| {
            if !sys.is_playing(a.handle) {
                return false;
            }
            let k = a.key;
            sys.set_position(a.handle, pos);
            if k.flags & 0x10 != 0
                && (if speed < 0.0 {
                    frame <= k.end_frame
                } else {
                    frame >= k.end_frame
                })
            {
                sys.stop(a.handle, 1);
                return false;
            }
            if k.id & 0xC00 == 0 {
                let inside = in_window(k, frame, speed < 0.0);
                if !inside {
                    return false;
                }
                if speed != 0.0 || k.flags & 0x20 == 0 {
                    sys.refresh(a.handle, pos);
                }
            }
            if sys
                .data()
                .init
                .sound_table
                .info(k.id)
                .is_some_and(|i| i.sw_bit & 0x0010_0000 != 0)
            {
                parameters(sys, a.handle, k, speed, true);
            }
            true
        });
        for key in self.cursor.crossed(table, frame, speed) {
            if key.id & 0xC00 == 0 && self.active.iter().any(|a| a.key.id == key.id) {
                continue;
            }
            if self.active.len() == 8 {
                continue;
            }
            if let Some(handle) = sys.play_anim_sound(key.id, pos, owner) {
                parameters(sys, handle, key, speed, false);
                self.active.push(Active { key, handle });
            }
        }
    }
}

fn in_window(k: AnimSound, frame: f32, reverse: bool) -> bool {
    let (start, end, f) = if reverse {
        (-k.start_frame, -k.end_frame, -frame)
    } else {
        (k.start_frame, k.end_frame, frame)
    };
    start == end
        || if start < end {
            f >= start && f < end
        } else {
            f > start || f < end
        }
}

fn parameters(sys: &mut SoundSystem, h: SoundHandle, k: AnimSound, speed: f32, modify: bool) {
    let volume = if modify && k.pitch_scale != 0 {
        (k.volume as f32 + (k.volume_scale as f32 * 2.0 * (speed - 1.0)).trunc()).clamp(0.0, 127.0)
    } else {
        k.volume as f32
    };
    sys.set_volume(h, volume / 127.0, 0, 5);
    sys.set_pitch(
        h,
        k.pitch + k.pitch_scale as f32 * (speed - 1.0) / 32.0,
        0,
        5,
    );
    if !modify {
        sys.set_pan(h, k.pan as f32 / 127.0, 0, 5);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(start: f32) -> AnimSound {
        AnimSound {
            id: 0x800,
            start_frame: start,
            end_frame: start,
            pitch: 1.0,
            flags: 0,
            volume: 127,
            pitch_scale: 0,
            loop_count: 0,
            pan: 64,
            volume_scale: 0,
        }
    }
    #[test]
    fn initial_skipped_frames_wrap_and_pause() {
        let t = AnimSoundTable {
            sounds: vec![key(0.0), key(2.0), key(9.0)],
        };
        let mut c = Cursor::default();
        assert_eq!(c.crossed(&t, 0.5, 1.0), vec![key(0.0)]);
        assert_eq!(c.crossed(&t, 8.5, 1.0), vec![key(2.0)]);
        assert_eq!(c.crossed(&t, 1.0, 1.0), vec![key(9.0), key(0.0)]);
        assert!(c.crossed(&t, 1.0, 0.0).is_empty());
        assert_eq!(c.crossed(&t, 2.0, 1.0), vec![key(2.0)]);
    }
    #[test]
    fn direction_and_exact_loop_flags() {
        let mut k = key(2.0);
        k.flags = 8;
        k.loop_count = 1;
        let t = AnimSoundTable { sounds: vec![k] };
        let mut c = Cursor::default();
        assert!(c.crossed(&t, 3.0, 1.0).is_empty());
        assert_eq!(c.crossed(&t, 2.0, 1.0), vec![k]);
        c.crossed(&t, 8.0, 1.0);
        assert!(c.crossed(&t, 2.0, 1.0).is_empty());
        k.flags = 1;
        let t = AnimSoundTable { sounds: vec![k] };
        assert!(Cursor::default().crossed(&t, 1.0, -1.0).is_empty());
    }
    #[test]
    fn held_windows_can_cross_loop_boundary() {
        let mut k = key(8.0);
        k.end_frame = 2.0;
        assert!(in_window(k, 9.0, false));
        assert!(in_window(k, 0.0, false));
        assert!(!in_window(k, 2.0, false));
        assert!(!in_window(k, 5.0, false));
    }
}
