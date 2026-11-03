//! The original's own goldens (tests/validate_audio.gd 193-207): `hash(stream.data)` of rendered cues. Godot hashes a
//! PackedByteArray with MurmurHash3 x86_32, seed 0x7F07C65 (core/templates/hashfuncs.h `hash_murmur3_buffer`).

use super::*;

fn murmur3(data: &[u8], seed: u32) -> u32 {
    let (c1, c2) = (0xcc9e_2d51u32, 0x1b87_3593u32);
    let mut h = seed;
    let blocks = data.len() / 4;
    for i in 0..blocks {
        let mut k = u32::from_le_bytes([
            data[i * 4],
            data[i * 4 + 1],
            data[i * 4 + 2],
            data[i * 4 + 3],
        ]);
        k = k.wrapping_mul(c1).rotate_left(15).wrapping_mul(c2);
        h ^= k;
        h = h.rotate_left(13).wrapping_mul(5).wrapping_add(0xe654_6b64);
    }
    let tail = &data[blocks * 4..];
    let mut k: u32 = 0;
    if tail.len() >= 3 {
        k ^= (tail[2] as u32) << 16;
    }
    if tail.len() >= 2 {
        k ^= (tail[1] as u32) << 8;
    }
    if !tail.is_empty() {
        k ^= tail[0] as u32;
        h ^= k.wrapping_mul(c1).rotate_left(15).wrapping_mul(c2);
    }
    h ^= data.len() as u32;
    h ^= h >> 16;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^ (h >> 16)
}

/// `hash(Sound.render_cue(cue, pan, intensity).data)` as the original computes it.
fn data_hash(cue: &str, pan: f64, intensity: f64) -> u32 {
    let frames = render_pcm16(cue, pan, intensity).expect("known cue");
    let mut bytes = Vec::with_capacity(frames.len() * 4);
    for f in &frames {
        bytes.extend_from_slice(&f[0].to_le_bytes());
        bytes.extend_from_slice(&f[1].to_le_bytes());
    }
    murmur3(&bytes, 0x07F0_7C65)
}

fn sha256_hex(msg: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut data = msg.to_vec();
    let bit_len = (msg.len() as u64) * 8;
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in data.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [
                t1.wrapping_add(t2),
                v[0],
                v[1],
                v[2],
                v[3].wrapping_add(t1),
                v[4],
                v[5],
                v[6],
            ];
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(v[i]);
        }
    }
    h.iter().map(|x| format!("{:08x}", x)).collect()
}

#[test]
fn cue_goldens_match_the_original() {
    let goldens: [(&str, f64, f64, u32); 8] = [
        ("step:walk:concrete:1", 0.05, 1.0, 3808229991),
        ("step:sprint:metal:2", -0.05, 1.0, 553711646),
        ("heartbeat", 0.0, 0.65, 456396346),
        ("music_box_tune", 0.0, 1.0, 642208190),
        ("glyra_hum", 0.0, 1.0, 3426237372),
        ("carnival_carousel_tune", 0.0, 1.0, 2758972945),
        ("step:sprint:carpet:1", 0.05, 1.0, 3790847878),
        ("step:sprint:water:1", 0.05, 1.0, 3697065950),
    ];
    for (cue, pan, intensity, want) in goldens {
        assert_eq!(
            data_hash(cue, pan, intensity),
            want,
            "{cue} differs from the original's bytes"
        );
    }
}

#[test]
fn step_and_heartbeat_aggregates_match_the_original() {
    let mut steps = Vec::new();
    for gait in ["walk", "sprint", "crouch"] {
        for surface in SURFACES {
            for variant in 0..4 {
                let pan = if variant % 2 == 0 { -0.05 } else { 0.05 };
                steps.push(
                    data_hash(&format!("step:{gait}:{surface}:{variant}"), pan, 1.0).to_string(),
                );
            }
        }
    }
    assert_eq!(
        sha256_hex(steps.join(",").as_bytes()),
        "84f0549c04e2939292b01e330a5325977c6a0c1085a45c5586e7c96af6993555"
    );
    let hearts: Vec<String> = (1..=20)
        .map(|i| data_hash("heartbeat", 0.0, i as f64 * 0.05).to_string())
        .collect();
    assert_eq!(
        sha256_hex(hearts.join(",").as_bytes()),
        "e7299c0b304aeabae8646a91e692e171df98907c445b9d178266fff4f1a0e82a"
    );
}

#[test]
fn synth_matches_footstep_keys() {
    // Sound.footstep: variant 1 is the right foot (+0.045 -> key pan 0.05)
    let (name, pan, _) = default_call("step:walk:concrete", 1).unwrap();
    assert_eq!(cache_key(&name, pan, 1.0), "step:walk:concrete:1|0.05|1.00");
    let (_, pan, _) = default_call("footstep_walk", 2).unwrap();
    assert_eq!(
        cache_key("footstep_walk", pan, 1.0),
        "footstep_walk|-0.05|1.00"
    );
}

#[test]
fn every_listed_cue_renders() {
    for name in cue_names() {
        let frames = synth(name, 0).unwrap_or_else(|| panic!("{name} did not render"));
        assert!(frames.len() > 100, "{name} is empty");
    }
    assert!(synth("no_such_cue", 0).is_none());
}

#[test]
fn ambience_is_silent_on_the_finite_route() {
    let bed = ambience("hospital", 2.0);
    assert_eq!(bed.len(), 88200);
    assert!(bed.iter().all(|f| f[0] == 0.0 && f[1] == 0.0));
}

#[test]
fn front_end_throttles_like_the_original() {
    let mut front = rules::SoundFront::new();
    // validate_audio.gd 108-125: one step per gap, none into a wall, faster speed shortens the gap
    assert!(front.footstep("walk", "", -1.0, 0).is_some());
    assert!(front.footstep("walk", "", -1.0, 0).is_none());
    assert!(front.footstep("walk", "", 0.05, 1_000_000).is_none());
    assert!(front.footstep("walk", "", 4.0, 200_000).is_none());
    assert!(front.footstep("walk", "", 8.0, 200_000).is_some());
    assert!((rules::footstep_gap("sprint", -1.0) - 0.205).abs() < 1e-9);
    let first = front.heartbeat(0.5, 0).unwrap();
    assert_eq!(first.key(), "heartbeat|0.00|0.50");
    assert!(front.heartbeat(0.5, 600_000).is_none()); // gap 1.08 - .365 = .715 s
    assert!(front.heartbeat(0.5, 720_000).is_some());
    assert!(front.heartbeat(0.03, 9_000_000).is_none());
    let step = front.hunter_footstep(0.3, 0.09, "");
    assert_eq!((step.pan, step.intensity), (0.25, 1.0));
    assert!((step.gain - 1.8).abs() < 1e-6);
    assert_eq!(rules::recharge(25.0).key(), "recharge|0.00|2.50");
    assert_eq!(front.menu_blip(0).unwrap().bus, rules::Bus::Ui);
}

#[test]
fn music_box_winds_down() {
    assert_eq!(music_box::pitch_scale(30.0, 0.0), Some(1.0));
    assert_eq!(music_box::pitch_scale(0.0, 0.0), None);
    let p = music_box::pitch_scale(7.5, 0.0).unwrap();
    assert!((p - 0.88).abs() < 1e-6);
    let buf = music_box::winddown(0.0, 0.0, 2.0);
    assert_eq!(buf.len(), 88200);
    assert!(buf.iter().any(|f| f[0].abs() > 0.01));
}
