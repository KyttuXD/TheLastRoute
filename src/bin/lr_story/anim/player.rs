//! Godot 4's AnimationPlayer as the hunters use it (scene/animation/animation_player.cpp + animation_mixer.cpp):
//!
//! * `play(name, custom_blend)`: with a blend > 0 the clip that was current keeps playing as a fading `Blend` entry
//!   (blend_left starts at the current blend amount), without one every fading clip is dropped (an instant switch).
//!   Same clip again: nothing is reset, except a finished one-shot starts over.
//! * `advance(delta)`: position += delta x speed_scale x the clip's own speed (loops wrap with fposmod, one-shots clamp
//!   and stop at the end); every fading clip loses |speed_scale x delta| / blend_time of its weight and keeps playing.
//! * `pose()`: the deterministic mixer. Every bone starts at its rest value (the cache's init value, there is no RESET
//!   clip), then each playing clip in order -- the current one first with weight 1 - sum(blend_left), then the fading
//!   ones -- adds `loc += (clip_loc - init_loc) * w` and `rot = (rot * IDENTITY.slerp(init_rot^-1 * clip_rot, w)).normalized()`.

use super::clip::{gd_slerp, Kind, Rig};
use macroquad::prelude::*;

#[derive(Clone)]
struct Playback {
    clip: String,
    pos: f32,
}

struct Blend {
    data: Playback,
    blend_left: f32,
    blend_time: f32,
}

pub struct AnimPlayer {
    current: Option<Playback>,
    pub assigned: String,
    pub playing: bool,
    blends: Vec<Blend>,
    pub speed_scale: f32,
    /// set by play() and consumed by the next advance (Godot processes a just-started clip with delta 0)
    started: bool,
}

/// One bone's local transform (bbmodel units).
#[derive(Clone, Copy)]
pub struct Local {
    pub t: Vec3,
    pub r: Quat,
}

impl Default for AnimPlayer {
    fn default() -> Self {
        AnimPlayer {
            current: None,
            assigned: String::new(),
            playing: false,
            blends: vec![],
            speed_scale: 1.0,
            started: false,
        }
    }
}

impl AnimPlayer {
    pub fn current_animation(&self) -> &str {
        if self.playing {
            &self.assigned
        } else {
            ""
        }
    }

    pub fn position(&self) -> f32 {
        self.current.as_ref().map(|c| c.pos).unwrap_or(0.0)
    }

    fn current_blend_amount(&self) -> f32 {
        1.0 - self.blends.iter().map(|b| b.blend_left).sum::<f32>()
    }

    pub fn play(&mut self, rig: &Rig, name: &str, custom_blend: f32) {
        let Some(clip) = rig.clips.get(name) else {
            return;
        };
        if let Some(cur) = &self.current {
            // hunters: no blend_times pairs and playback_default_blend_time = 0
            let blend_time = if custom_blend >= 0.0 {
                custom_blend
            } else {
                0.0
            };
            if blend_time > 0.0 {
                let left = self.current_blend_amount();
                self.blends.push(Blend {
                    data: cur.clone(),
                    blend_left: left,
                    blend_time,
                });
            } else {
                self.blends.clear();
            }
        }
        if self.assigned != name {
            self.current = Some(Playback {
                clip: name.to_string(),
                pos: 0.0,
            });
        } else {
            let mut cur = self.current.clone().unwrap_or(Playback {
                clip: name.to_string(),
                pos: 0.0,
            });
            if cur.pos == clip.length {
                cur.pos = 0.0;
            }
            self.current = Some(cur);
        }
        self.started = true;
        self.playing = true;
        self.assigned = name.to_string();
    }

    pub fn seek(&mut self, t: f32) {
        if let Some(c) = &mut self.current {
            c.pos = t;
        }
    }

    fn step(rig: &Rig, p: &mut Playback, delta: f32, speed: f32) -> bool {
        let Some(clip) = rig.clips.get(&p.clip) else {
            return false;
        };
        // A negative length in a rig file would make clamp(0, len) panic; it is read as zero.
        let len = clip.length.max(0.0);
        let mut next = p.pos + delta * speed;
        let prev = p.pos;
        let mut ended = false;
        if clip.looping {
            next = if len.abs() < 1e-5 {
                0.0
            } else {
                next.rem_euclid(len)
            };
        } else {
            next = if len.abs() < 1e-5 {
                0.0
            } else {
                next.clamp(0.0, len)
            };
            let backwards = speed < 0.0;
            ended = (!backwards && len <= prev + 1e-5) || (backwards && prev <= 0.0);
        }
        p.pos = next;
        ended
    }

    pub fn advance(&mut self, rig: &Rig, delta: f32) {
        if !self.playing {
            return;
        }
        let Some(mut cur) = self.current.take() else {
            return;
        };
        let started = std::mem::replace(&mut self.started, false);
        let d = if started { 0.0 } else { delta };
        let ended = Self::step(rig, &mut cur, d, self.speed_scale);
        self.current = Some(cur);
        if ended {
            self.blends.clear();
            self.playing = false;
            return;
        }
        let speed = self.speed_scale;
        self.blends.retain_mut(|b| {
            b.blend_left = (b.blend_left - (speed * delta).abs() / b.blend_time).max(0.0);
            let keep = b.blend_left > 1e-5;
            if !keep {
                b.blend_left = 1e-5;
            }
            Self::step(rig, &mut b.data, delta, speed);
            keep
        });
    }

    /// The mixed local pose of every rig bone.
    pub fn pose(&self, rig: &Rig) -> Vec<Local> {
        let mut out: Vec<Local> = rig
            .bones
            .iter()
            .map(|b| Local {
                t: b.rest_t,
                r: b.rest_r,
            })
            .collect();
        let Some(cur) = &self.current else { return out };
        let mut layers: Vec<(&Playback, f32)> = vec![(cur, self.current_blend_amount())];
        for b in &self.blends {
            layers.push((&b.data, b.blend_left));
        }
        for (pb, w) in layers {
            let Some(clip) = rig.clips.get(&pb.clip) else {
                continue;
            };
            clip.sample(pb.pos, |tr, v| {
                let bone = &rig.bones[tr.bone];
                let o = &mut out[tr.bone];
                match tr.kind {
                    Kind::Pos => o.t += (v.truncate() - bone.rest_t) * w,
                    Kind::Rot => {
                        let q = Quat::from_vec4(v);
                        o.r = (o.r * gd_slerp(Quat::IDENTITY, bone.rest_r.inverse() * q, w))
                            .normalize();
                    }
                }
            });
        }
        out
    }
}
