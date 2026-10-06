//! Which hand models Mario shows per clip.
//!
//! `TMario::setAnimation` (MarioDraw.cpp) calls `changeHand(gMarioAnimeData[id].unk5)` for the
//! animation it starts: 0 draws the hands built into the body (shapes 5 and 6), 1 hides those
//! and draws the fists (`ma_hnd3*`), 2 draws the open hands (`ma_hnd2*`). Below are the clip
//! files whose first animation entry picks 1 or 2; every other clip uses 0.

/// Hand set for a clip (by its `ma_<name>.bck` stem).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hands {
    Body,
    Fists,
    Open,
}

const FISTS: &[&str] = &[
    "door_kick",
    "fence_catch",
    "fence_jcatch",
    "fence_move_down",
    "fence_move_l",
    "fence_move_r",
    "fence_move_up",
    "fence_punch",
    "fence_wait",
    "hang",
    "hang_to_hold",
    "hgdwn",
    "hgjmp",
    "hgup",
    "hmov_l",
    "hmov_r",
    "hold",
    "hold_back",
    "hold_drag",
    "hold_hang",
    "hold_move_l",
    "hold_move_r",
    "hold_return",
    "hold_to_hang",
    "hold_wait",
    "jmped",
    "ladder_hang_catch",
    "ladder_hang_kick",
    "ladder_hang_move_l",
    "ladder_hang_move_r",
    "ladder_hang_wait_l",
    "ladder_hang_wait_r",
    "ladder_roll_down",
    "ladder_roll_up",
    "put",
    "raise",
    "rope_hmovl",
    "rope_hmovr",
    "run1",
    "slip",
    "slped",
    "slpla",
    "sqend",
    "sqsta",
    "sqwat",
    "throw",
    "tree_catch",
    "tree_climb",
    "tree_wait",
    "trned",
    "turn",
];

const OPEN: &[&str] = &[
    "demo_gate_out_appear_get",
    "dgrun",
    "giant_rolling",
    "hipat",
    "hiped",
    "hipsr",
    "jkick",
    "jump",
    "jump_rolling",
    "monteman_wait",
    "roll",
    "run2",
    "step1",
    "step2",
    "step3",
    "tjmp1",
    "tjmp2",
    "turbo_dash",
];

pub fn hands_for_clip(clip: &str) -> Hands {
    if FISTS.contains(&clip) {
        Hands::Fists
    } else if OPEN.contains(&clip) {
        Hands::Open
    } else {
        Hands::Body
    }
}

/// Body shapes hidden at load (`TMario::initModel` sets shape 4's flag) and the body's own
/// hands (`changeHand`).
pub const HIDDEN_SHAPE: usize = 4;
pub const BODY_HAND_SHAPES: [usize; 2] = [5, 6];
/// The Shine Shirt over the usual shirt and overalls: `TMario::thinkAloha` (MarioMain.cpp)
/// hides it unless Mario has `MARIO_FLAG_HAS_SHIRT`.
pub const SHIRT_SHAPE: usize = 10;
