//! The hunter footstep cadence: HunterView3D's footfall rate (game/hunters/view/hunter_view_3d.gd 1662-1686), which
//! main_3d's hunter-step clock runs on (`rules::HunterStepClock`). A travel clip (walk / chase / sprint) plays two
//! footfalls per cycle at a stride-matched speed_scale (HunterStride.choose, hunter_stride.gd 61-76: ground speed / the
//! clip's measured stride speed, swapping clip when the pace is outside 0.55-1.9x, clamped 0.2-2.6x, eased in with
//! `1 - exp(-10 dt)`), so `footfall_rate = 2 * |speed_scale| / clip_length`. The view's ground speed is itself eased
//! (`lerp(speed, step / frame, .3)` per frame, hunter_view_3d.gd 1389-1400) and the clip only walks once that speed passes
//! 0.35 m/s, dropping back to idle after 0.35 s under 0.15 m/s (1764-1773). Clip choice by brain state: CHASE = sprint
//! while she sees the runner and is not winded, else chase; WANDER / INVESTIGATE / SEARCH = walk; anything else idle.
//!
//! STRIDE: assets/hunters_blocky/stride_table.json (ref m/s and clip length s per clip), times HunterRoster.MODEL_SCALE.

/// (hunter id, walk (ref, cycle), chase (ref, cycle), sprint (ref, cycle)) from stride_table.json.
pub const STRIDE: [(&str, (f32, f32), (f32, f32), (f32, f32)); 38] = [
    ("aki", (1.198, 1.111), (3.131, 0.556), (2.802, 0.741)),
    ("briselle", (1.335, 1.111), (3.465, 0.556), (3.08, 0.741)),
    ("ceryth", (1.497, 1.0), (3.813, 0.5), (3.032, 0.667)),
    ("dovelle", (1.567, 1.0), (3.976, 0.5), (3.152, 0.667)),
    ("elise", (2.116, 0.833), (5.258, 0.417), (4.559, 0.556)),
    ("ferryn", (1.46, 0.909), (3.664, 0.455), (3.213, 0.606)),
    ("glyra", (1.266, 1.176), (3.303, 0.588), (2.871, 0.784)),
    ("haelith", (1.943, 0.909), (4.756, 0.455), (4.083, 0.606)),
    ("ilyra", (1.335, 1.111), (3.465, 0.556), (3.08, 0.741)),
    ("javelle", (1.943, 0.909), (4.756, 0.455), (4.083, 0.606)),
    ("kathy", (1.567, 1.0), (3.976, 0.5), (3.152, 0.667)),
    ("loxelle", (1.43, 1.053), (3.778, 0.526), (3.071, 0.702)),
    ("mavrine", (1.497, 1.0), (3.813, 0.5), (3.032, 0.667)),
    ("nyxie", (1.77, 1.0), (4.429, 0.5), (3.476, 0.667)),
    ("opalie", (1.788, 0.909), (4.417, 0.455), (3.822, 0.606)),
    ("porcelle", (1.137, 1.176), (2.987, 0.588), (2.611, 0.784)),
    ("quenelle", (1.335, 1.111), (3.465, 0.556), (3.08, 0.741)),
    ("ravelle", (1.866, 0.909), (4.59, 0.455), (3.956, 0.606)),
    ("solyth", (2.091, 0.909), (5.067, 0.455), (4.312, 0.606)),
    ("thessine", (1.426, 1.0), (3.645, 0.5), (2.907, 0.667)),
    ("umbriel", (1.202, 1.176), (3.147, 0.588), (2.744, 0.784)),
    ("verrine", (1.335, 1.111), (3.465, 0.556), (3.08, 0.741)),
    ("void_black", (2.295, 0.87), (5.452, 0.435), (4.889, 0.58)),
    ("void_blue", (1.682, 1.0), (4.285, 0.5), (3.407, 0.667)),
    ("void_brown", (2.258, 0.952), (5.945, 0.476), (4.565, 0.635)),
    ("void_cyan", (2.295, 0.87), (5.452, 0.435), (4.889, 0.58)),
    ("void_gray", (1.861, 1.053), (4.831, 0.526), (3.877, 0.702)),
    ("void_green", (2.295, 0.87), (5.452, 0.435), (4.889, 0.58)),
    (
        "void_orange",
        (2.759, 0.769),
        (6.547, 0.385),
        (5.943, 0.513),
    ),
    ("void_pink", (1.918, 1.053), (4.98, 0.526), (3.996, 0.702)),
    (
        "void_purple",
        (2.378, 0.833),
        (5.908, 0.417),
        (5.122, 0.556),
    ),
    ("void_red", (2.759, 0.769), (6.547, 0.385), (5.943, 0.513)),
    ("void_white", (1.918, 1.053), (4.98, 0.526), (3.996, 0.702)),
    ("void_yellow", (2.47, 0.833), (6.106, 0.417), (5.273, 0.556)),
    ("wynelle", (1.402, 1.111), (3.624, 0.556), (3.21, 0.741)),
    ("xivvy", (2.21, 0.8), (5.11, 0.4), (4.445, 0.533)),
    ("ysolde", (1.202, 1.176), (3.147, 0.588), (2.744, 0.784)),
    ("zellia", (1.718, 0.952), (4.613, 0.476), (3.595, 0.635)),
];

pub const MODEL_SCALE: f32 = 1.5;
const MIN_RATIO: f32 = 0.55;
const MAX_RATIO: f32 = 1.9;
const HARD_MIN: f32 = 0.2;
const HARD_MAX: f32 = 2.6;
const GAIT_ENTER_SPEED: f32 = 0.35;
const STILL_SPEED: f32 = 0.15;
const STILL_CLIP_AFTER: f32 = 0.35;
const TELEPORT_DISTANCE: f32 = 4.0;
/// HunterStride.FALLBACK refs for a model without a table entry (clip length then unknown: 1 s).
const FALLBACK: [f32; 3] = [1.4, 3.0, 4.2];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clip {
    Walk = 0,
    Chase = 1,
    Sprint = 2,
}

/// The brain state's travel clip (hunter_view_3d.gd 1758-1763); `None` = idle.
pub fn preferred_clip(
    chasing: bool,
    wandering_or_searching: bool,
    direct_visible: bool,
    winded: bool,
) -> Option<Clip> {
    if chasing {
        Some(if direct_visible && !winded {
            Clip::Sprint
        } else {
            Clip::Chase
        })
    } else if wandering_or_searching {
        Some(Clip::Walk)
    } else {
        None
    }
}

fn entry(id: &str, clip: Clip) -> (f32, f32) {
    match STRIDE.iter().find(|e| e.0 == id) {
        Some(e) => [e.1, e.2, e.3][clip as usize],
        None => (FALLBACK[clip as usize], 1.0),
    }
}

/// HunterStride.ref_speed: the clip's stride speed in m/s at speed_scale 1.
pub fn ref_speed(id: &str, clip: Clip) -> f32 {
    entry(id, clip).0.max(0.3) * MODEL_SCALE
}

/// HunterStride.choose: the clip actually played and its target speed_scale.
pub fn choose(id: &str, preferred: Clip, speed: f32) -> (Clip, f32) {
    let mut clip = preferred;
    let ratio = speed / ref_speed(id, preferred);
    if !(MIN_RATIO..=MAX_RATIO).contains(&ratio) {
        let mut best = f32::INFINITY;
        for c in [Clip::Walk, Clip::Chase, Clip::Sprint] {
            let gap = (speed.max(0.05) / ref_speed(id, c)).ln().abs();
            if gap < best {
                best = gap;
                clip = c;
            }
        }
    }
    (
        clip,
        (speed / ref_speed(id, clip)).clamp(HARD_MIN, HARD_MAX),
    )
}

/// One hunter view's gait state.
#[derive(Clone, Debug)]
pub struct HunterGait {
    move_speed: f32,
    still_time: f32,
    gait_moving: bool,
    stride_scale: f32,
    primed: bool,
    last_pos: (f32, f32),
    pub footfall_rate: f32,
}

impl Default for HunterGait {
    fn default() -> Self {
        HunterGait {
            move_speed: 0.0,
            still_time: 0.0,
            gait_moving: false,
            stride_scale: 1.0,
            primed: false,
            last_pos: (0.0, 0.0),
            footfall_rate: 0.0,
        }
    }
}

impl HunterGait {
    pub fn new() -> Self {
        Self::default()
    }

    /// One frame of the view: `pos` the brain position (m), `preferred` the state's clip, `can_step` false while
    /// inactive, stunned, in the vents or climbing. Returns the footfall rate (footfalls per second).
    pub fn update(
        &mut self,
        id: &str,
        pos: (f32, f32),
        dt: f32,
        preferred: Option<Clip>,
        can_step: bool,
    ) -> f32 {
        let frame = dt.max(1.0 / 120.0);
        let step = ((pos.0 - self.last_pos.0).powi(2) + (pos.1 - self.last_pos.1).powi(2)).sqrt();
        if !self.primed || step > TELEPORT_DISTANCE {
            self.move_speed = 0.0;
            if !self.primed {
                self.still_time = STILL_CLIP_AFTER + 0.01;
            }
        } else {
            self.move_speed += (step / frame - self.move_speed) * 0.3;
        }
        self.primed = true;
        self.last_pos = pos;
        self.still_time = if self.move_speed < STILL_SPEED {
            self.still_time + frame
        } else {
            0.0
        };
        self.footfall_rate = 0.0;
        let Some(preferred) = preferred else {
            return 0.0;
        };
        if self.gait_moving && self.still_time > STILL_CLIP_AFTER {
            self.gait_moving = false;
        } else if !self.gait_moving && self.move_speed > GAIT_ENTER_SPEED {
            self.gait_moving = true;
        }
        if !self.gait_moving {
            return 0.0; // the idle clip: no footfalls
        }
        let (clip, scale) = choose(id, preferred, self.move_speed);
        self.stride_scale += (scale - self.stride_scale) * (1.0 - (-10.0 * dt).exp());
        if !can_step {
            return 0.0;
        }
        let length = entry(id, clip).1.max(0.1);
        self.footfall_rate = 2.0 * self.stride_scale.abs() / length;
        self.footfall_rate
    }
}
