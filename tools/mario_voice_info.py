"""What each of Mario's voice cues (and the sounds around them) is for.

Shared by luigi_voice_sorter.py (shows it beside every Mario reference you play) and
mario_voice_recorder.py (shows it while you record your own take). Written from the Sunshine
decomp, the behavioural spec: the call sites of `startVoice` / `startMarioVoice` in
`sms-main/src/Player/Mario*.cpp` and the swap table in `MSound.cpp` (the tired variants and the
random `MA_VO_*` alternates). RoR2 Mario runs the same sim, so the same moments trigger them
(libsms: damage.rs, air.rs, dirty.rs, hang.rs, carry.rs; the RoR2 jump kinds are mapped in
libsms-present/src/audio.rs). Standard library only.
"""
import re

# (cue-name prefix, short title, when it plays). The longest matching prefix wins; a clip name
# like MV21_JUMP_SMALL_03 matches its prefix MV21_JUMP_SMALL, so every numbered variant of a cue
# shares one text. Variants are picked at random by the game, so you want several takes.
CUES = [
    ("MV01_DAMAGE_BIG", "Big hit", "Hurt for 2 or more damage (damageExec, MarioCollision.cpp; libsms damage.rs). 25% of the time the game swaps in MA_VO_DAMAGE_BIG."),
    ("MV02_DAMAGE_MID", "Medium hit", "Hurt for 1 damage with the medium knock-back reaction (the 'down type' is not 0). 25% swapped for MA_VO_DAMAGE_SMALL."),
    ("MV03_DAMAGE_LITLE", "Small hit / small tumble", "Hurt for 1 damage with the little flinch. Also his grunt for tumbling: "
        "getting up from a back-fall, landing from being thrown, slipping back on goop, and falling on his feet from a foot-down. "
        "Loud and frequent: the one you hear most. 25% swapped for MA_VO_DAMAGE_SMALL."),
    ("MV04_DAMAGE_ELEC", "Electric shock", "Zapped by electricity (electricDamage, MarioAutodemo.cpp)."),
    ("MV05_DAMAGE_FIRE", "Catching fire", "Set on fire: plays when the burning bounce starts (fireDowning / fireJumping). _02 is the second line used while he runs around on fire."),
    ("MV06_DAMAGE_WATER", "Picked up / carried off", "Plays when something grabs and takes him (status TAKEN: a bird, a monster's grip). Despite the name it is not about water."),
    ("MV07_DAMAGE_REACT", "Reacting to a hit", "The follow-up yelp after a damage animation: the second half of a shock, and when the fire bounce lands. "
        "At 2 health or less the game plays MV33_T instead."),
    ("MV08A_DOWN", "Dying", "Mario loses his last life meter and falls (loserDown): the death cry. 15% swapped for MA_VO_DEAD."),
    ("MV08B_DOWN_REACT", "Dying, second beat", "A short second cry partway through the death animation (loserDown, state 3)."),
    ("MV10A_CRY_SHORT", "Sinking in goop", "Whimper when he sinks into pollution or is stuck in the sludge (checkSink). A random cry-filler too, "
        "so it repeats now and then while he is in trouble."),
    ("MV10B_CRY_JUMP", "Crying while thrown", "A cry as he is flung through the air: when a Bowser-sized grab throws him, "
        "and about a second into being thrown down by one (MarioSound thrown-down check)."),
    ("MV12_REACT", "Waking up", "Mario is asleep (the long idle) and you press something: _03 is the yelp as he wakes (MarioWait sleeping). At 2 health or less the game plays MV33_T instead."),
    ("MV13_ACTION_SMALL", "Small action (dive, door, catch)", "Short effort for the belly-slide dive start, grabbing a thing mid-jump (jump catch), "
        "and opening a door in the auto-demo. In RoR2 this is the dive. Tired: MV34_ACTION_T."),
    ("MV15_EXERT_INST", "Instant effort", "A short 'hup' for pulling himself up: grabbing a ledge, hanging, "
        "climbing, pulling a plug or a lever, raising a thing, the Turbo nozzle's launch, and starting a ground pound. _02 is the variant used when "
        "he swings onto a wire or hangs. At 2 health or less: MV38_EXERT_INST_T / MV34_ACTION_T_02."),
    ("MV15_REACT_SMALL", "Small effort reaction", "On the disc and in the sound table; no call in the decomp player code, so it is probably unused. Record it last."),
    ("MV16_EXERT_CONT", "Continuous effort", "The grunt while he pushes a wall, or carries something heavy "
        "(doPushingAnimation, and holding an object). Loops in the game by random replay, so make it a short grunt."),
    ("MV17_EXERT_REACT", "Effort reaction / waking", "A short grunt: _02 is the groan when he wakes from a sleep that was already deep; _01 is keyed in his sitting animation (ma_sit). Tired: MV34_ACTION_T."),
    ("MV18_FALL_AFTER", "After a fall", "Breath after hitting the ground; keyed to the end of the gate-exit (pipe out) animation."),
    ("MV19_JMP_PT", "Jump off a pad / trampoline", "Springing off something springy: plays instead of the normal small jump when the surface he jumped from is deep/soft "
        "(setStatusToJumping, 'feel deep')."),
    ("MV20_JMP_PT_REACT", "Landing off a pad", "Follow-up after a pad jump; keyed to the pipe-exit (gate-out) animation."),
    ("MV21_JUMP_SMALL", "Jump - small", "The first, ordinary jump; also a miss-jump, Hover nozzle launch, and a trample-up's first step. "
        "In RoR2 this is the single A-press jump. 30% swapped for MA_VO_JUMP_SMALL. Tired: MV41_JUMP_T."),
    ("MV22_JUMP_MID", "Jump - medium", "Second jump of the triple-jump chain, a wall jump, a broad (long) jump, a forced jump, a trample step. "
        "70% swapped for MA_VO_JUMP_MID. Tired: MV41_JUMP_T."),
    ("MV23_JUMP_LARGE", "Jump - large", "Third jump of the chain, the back flip, the turn-around jump, the right-rotate (spin) jump, a higher trample step. "
        "50% swapped for MA_VO_JUMP_BIG. Tired: MV41_JUMP_T."),
    ("MV24_JUMP_SPECIAL", "Jump - special / spin / rocket", "The spin jump, the Rocket nozzle launch, a forced bounce (a spring or a spin-bounce), "
        "wall-spin and the hang-to-air jump. 50% swapped for MA_VO_JUMP_SPECIAL. Tired: MV41_JUMP_T / MV42_JUMP_HUGE_T_02."),
    ("MV25A_JUMP_HUGE", "Jump - huge / thrown", "The big yell: thrown down by a heavy object after about half a second in the air, "
        "or an extreme launch. Tired: MV42_JUMP_HUGE_T."),
    ("MV25A_WIRE_ROLL", "Wire swing roll", "Swinging all the way round a wire or rope: each full turn plays an A/B voice, randomly the 01 or 02 set (wireSwinging in MarioSpecial.cpp). Wire stages only; rare."),
    ("MV26_JUMP_REACT", "Landing reaction", "Landing after a jump: a 'hoo' as he lands. 50% swapped for MA_VO_LAND_LOW / MA_VO_LAND_HI. "
        "Tired: MV33_T. It is the quiet sound under every hard landing."),
    ("MV27_SPRISE", "Surprised (big)", "Startled: slipping on a slope (slip fore/back), a hit from a blocking actor, and the first beat of a fire hit. "
        "Slipping uses it, so he says it every time he slides off a steep slope."),
    ("MV28_SPRISE_SMALL", "Surprised (small)", "Small startle: slipping on goop (oil slip/slope, the oil run), a super ground pound landing "
        "(hipAttacking), a bump from certain actors. Keyed (_02) into his rope-hang animation."),
    ("MV29_SPRISE_REACT", "Surprise follow-up", "The extra yelp that the voice system allows after a surprise (MSound.cpp groups it with the other surprise cues); no direct call from the player code."),
    ("MV30_FRIGHT", "Frightened", "Swinging or wobbling on a wire, and letting go of something he was pulling (pulling, wireWait, wireSWait). Keyed in the hanging-fall animation."),
    ("MV31_OPEN_DOOR", "Opening a door", "Pushing a door open (the left/right door animations, MarioReceiveMsg). Also swapped in for stage-change lines."),
    ("MV33_T", "Tired - reaction", "The low-health version of MV07 / MV12 / MV26: plays instead when his health is 2 or less. _05 and _06 are the low-health yelps for latching onto a wire."),
    ("MV34_ACTION_T", "Tired - action", "Low-health version of MV13 / MV17 (and MV15_EXERT_INST_02 for hanging)."),
    ("MV38_EXERT_INST_T", "Tired - effort", "Low-health version of MV15_EXERT_INST; also his yawn animation."),
    ("MV41_JUMP_T", "Tired - jump", "Low-health version of the small/mid/large/special jumps (_03 also the low-health pull-up)."),
    ("MV42_JUMP_HUGE_T", "Tired - huge jump", "Low-health version of the huge jump and of forced jumps."),
    ("MV46A_OPEN_DOOR_T", "Tired - door", "Low-health version of the door-opening voice."),
    ("MV46B_NEXT_STG_T", "Tired - next stage", "At 2 health or less the stage-change call plays this instead of the MV31 door voice (MSound.cpp, id -2)."),
    ("MV_BYEBYE", "Bye bye", "A spoken farewell, used in cutscenes (not a gameplay sound)."),
    ("MV_CHAO", "Chao!", "'Chao!' from the GameCube logo screen at startup (GCLogoDir.cpp). Not used in gameplay."),
    ("MV_COIN", "Coin voice", "On the disc; no call in the player code. Not a gameplay sound."),
    ("MV_DOOR", "Door", "Keyed on the 'gacha' door animations (ma_door_gacha_l/r), a short voice as he swings the door."),
    ("MV_IBIKI", "Snore", "Snoring in the sleeping/sitting idle (ma_sleep_wait, ma_sit_wait): the idle sleep that begins after a long wait. Two variants alternate."),
    ("MV_UNTEI", "Ladder / bars", "Grunts while he climbs hand over hand on a ladder or bars (ma_ladder_hang_move). _03 also plays when he lands after a slide (LAND_SLIP -> LANDING)."),
    ("MV_YOSHI_AND_MARIO", "Yoshi and Mario", "On the disc; no call in the player code (a cutscene / jingle line). Not a gameplay sound."),
    ("MA_VO_DAMAGE_BIG", "Alternate big hit", "Random replacement for MV01_DAMAGE_BIG (25%): a second voice style."),
    ("MA_VO_DAMAGE_SMALL", "Alternate small hit", "Random replacement for MV02 / MV03 (25%)."),
    ("MA_VO_DEAD", "Alternate death", "15% of the time instead of MV08A_DOWN when he dies."),
    ("MA_VO_FOO", "Hot-air puff", "A long breath; keyed in the hot-weather idle (ma_hot_wait)."),
    ("MA_VO_JUMP_BIG", "Alternate large jump", "50% of MV23_JUMP_LARGE; also the line in the Shine Get pose (arms up)."),
    ("MA_VO_JUMP_MID", "Alternate medium jump", "70% of MV22_JUMP_MID (the most common medium-jump voice)."),
    ("MA_VO_JUMP_SMALL", "Alternate small jump", "30% of MV21_JUMP_SMALL; also the low-effort grab on a pull when his health is high."),
    ("MA_VO_JUMP_SPECIAL", "Alternate special jump", "50% of MV24_JUMP_SPECIAL."),
    ("MA_VO_LAND_HI", "Alternate landing (high)", "Alternate to MV26 when the fall was from a high drop."),
    ("MA_VO_LAND_LOW", "Alternate landing (low)", "50% of MV26_JUMP_REACT on an ordinary landing."),
]

# Non-voice sounds Mario's animations key; a replacement pack needs these too. Names are the
# game's own (MSD_SE_ prefix dropped). See libsms-assets --example sound_inventory.
SFX = [
    ("MA_WALK_STONE_L_HEEL", "Footstep - left heel", "Left heel strike on stone/ordinary floor: 41 of Mario's animations key it (every walk, run and landing)."),
    ("MA_WALK_STONE_L_TIP", "Footstep - left toe", "Left toe touch-down, 29 animations."),
    ("MA_WALK_STONE_R_HEEL", "Footstep - right heel", "Right heel strike, 36 animations."),
    ("MA_WALK_STONE_R_TIP", "Footstep - right toe", "Right toe touch-down, 26 animations."),
    ("MA_WALK_SAND_L", "Footstep - sand left", "Left step on sand (and when he fills with sand)."),
    ("MA_WALK_SAND_R", "Footstep - sand right", "Right step on sand."),
    ("MA_WALK_WIRE_L1", "Footstep - wire left", "Hand/foot on a wire or rope, left."),
    ("MA_WALK_WIRE_R1", "Footstep - wire right", "Hand/foot on a wire or rope, right."),
    ("MA_WALK_METALNET_LH1", "Fence step - left hand", "Climbing a chain-link fence, left hand."),
    ("MA_WALK_METALNET_RH1", "Fence step - right hand", "Climbing a chain-link fence, right hand."),
    ("MA_WALK_WOOD_THN_LH1", "Tree climb - left", "Climbing a thin wooden pole or tree, left hand."),
    ("MA_WALK_WOOD_THN_RH1", "Tree climb - right", "Climbing a thin wooden pole or tree, right hand."),
    ("MA_TIMP_LOW", "Body impact - light", "A body hitting the floor: low impact (a small flop)."),
    ("MA_TIMP_MID", "Body impact - medium", "Medium flop onto the floor."),
    ("MA_TIMP_HI", "Body impact - heavy", "Hard flop onto the floor."),
    ("MA_SIRIMOTHI", "Butt landing", "Landing on his rear (after a backward fall or a jump-fall)."),
    ("MA_BOUND", "Bounce", "A bounce off the ground (also the hold-return pop)."),
    ("MA_ROLL", "Roll", "Rolling over on the ground (the roll animation)."),
    ("PO_FRICTION_A", "Slide friction", "Cloth/ground scrape layered under jumps, landings, slides (19 animations)."),
    ("MA_FENCE_CATCH", "Fence grab", "Catching a fence."),
    ("MA_FENCE_PUNCH", "Fence punch", "Punching a fence from behind (ladder kick, roll down)."),
    ("MA_SWIM", "Swimming stroke", "A swim stroke in the water (paddle start, swim dive, damage)."),
    ("MA_FLOAT", "Float", "Bobbing in the water."),
    ("MA_TAKE_BARREL", "Lifting a barrel", "Raising or putting down a carried thing."),
    ("MA_BELT", "Belt", "Keyed in ma_belt_up (the belt animation)."),
    ("OBJ_DOOR_GACHA", "Door rattle", "The door's own sound as he opens it."),
    ("MA_HIP_ATTACK", "Ground pound", "The impact of a ground pound, and the splash of the spin (event)."),
    ("MA_ROLL_JUMP", "Roll jump", "The rolling-jump whoosh (event)."),
    ("MA_FALL_AFTER", "Fall landing", "The thud after being thrown down (event)."),
    ("MA_WALL_COL_CMN_H", "Wall impact", "Hard hit against a wall (event)."),
]


def _best(table, name):
    key = re.sub(r"\.wav$", "", name, flags=re.I).upper()
    key = re.sub(r"^MSD_SE_", "", key)
    best = None
    for pre, title, use in table:
        if key == pre or key.startswith(pre + "_") or key.startswith(pre):
            if best is None or len(pre) > len(best[0]):
                best = (pre, title, use)
    return best


def describe(cue):
    """(title, text) for a cue or clip name like MV21_JUMP_SMALL_03; ('', '') if unknown."""
    hit = _best(CUES, cue) or _best(SFX, cue)
    return (hit[1], hit[2]) if hit else ("", "")


def line(cue):
    """One display string: 'Title: when it plays'."""
    t, u = describe(cue)
    return "%s: %s" % (t, u) if t else "(no note for this cue)"


if __name__ == "__main__":
    import glob, os, sys
    folder = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(__file__), "..", "..", "output", "mario-refs")
    missing = [os.path.basename(p) for p in sorted(glob.glob(os.path.join(folder, "*.wav"))) if not describe(os.path.basename(p))[0]]
    print("cues without a note: %s" % (missing or "none"))
