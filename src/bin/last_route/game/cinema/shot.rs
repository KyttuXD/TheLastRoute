//! Shot file format (the port of the Godot playtest sequencer): a stage, actors, a camera path and timing.
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct CamPoint {
    pub t: f32,
    pub pos: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Actor {
    pub id: String,
    pub kind: String,
    pub pos: [f32; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Shot {
    pub id: String,
    pub stage: String,
    pub seed: u64,
    pub duration: f32,
    pub actors: Vec<Actor>,
    pub camera: Vec<CamPoint>,
}

fn field<'a>(v: &'a Value, k: &str) -> Result<&'a Value, String> {
    v.get(k).ok_or_else(|| format!("missing field {k}"))
}

fn text(v: &Value, k: &str) -> Result<String, String> {
    field(v, k)?.as_str().map(str::to_string).ok_or_else(|| format!("field {k} is not text"))
}

fn num(v: &Value, k: &str) -> Result<f32, String> {
    field(v, k)?.as_f64().map(|n| n as f32).ok_or_else(|| format!("field {k} is not a number"))
}

impl Shot {
    pub fn parse(src: &str) -> Result<Shot, String> {
        let v: Value = serde_json::from_str(src).map_err(|e| e.to_string())?;
        let mut actors = Vec::new();
        for a in field(&v, "actors")?.as_array().ok_or("field actors is not a list")? {
            actors.push(Actor {
                id: text(a, "id")?,
                kind: text(a, "kind")?,
                pos: [num(a, "x")?, num(a, "y")?],
            });
        }
        let mut camera = Vec::new();
        for p in field(&v, "camera")?.as_array().ok_or("field camera is not a list")? {
            camera.push(CamPoint {
                t: num(p, "t")?,
                pos: [num(p, "x")?, num(p, "y")?, num(p, "z")?],
                yaw: num(p, "yaw")?,
                pitch: num(p, "pitch")?,
            });
        }
        if camera.is_empty() {
            return Err("camera path is empty".into());
        }
        if camera.windows(2).any(|w| w[1].t < w[0].t) {
            return Err("camera times must not go backwards".into());
        }
        Ok(Shot {
            id: text(&v, "id")?,
            stage: text(&v, "stage")?,
            seed: field(&v, "seed")?.as_u64().ok_or("field seed is not a whole number")?,
            duration: num(&v, "duration")?,
            actors,
            camera,
        })
    }

    pub fn to_json(&self) -> String {
        let actors: Vec<Value> = self
            .actors
            .iter()
            .map(|a| serde_json::json!({"id": a.id, "kind": a.kind, "x": a.pos[0], "y": a.pos[1]}))
            .collect();
        let camera: Vec<Value> = self
            .camera
            .iter()
            .map(|c| serde_json::json!({"t": c.t, "x": c.pos[0], "y": c.pos[1], "z": c.pos[2], "yaw": c.yaw, "pitch": c.pitch}))
            .collect();
        serde_json::json!({"id": self.id, "stage": self.stage, "seed": self.seed, "duration": self.duration, "actors": actors, "camera": camera}).to_string()
    }
}

/// Every embedded shot (data/cinema/*.json except the trailer list).
pub fn embedded_shots() -> Vec<Shot> {
    crate::content::embedded::data_under("cinema/")
        .filter(|(p, _)| !p.ends_with("trailer.json"))
        .filter_map(|(_, t)| Shot::parse(t).ok())
        .collect()
}

/// The trailer list: the seed and the ordered shot ids.
pub fn trailer() -> Result<(u64, Vec<String>), String> {
    let (_, t) = crate::content::embedded::data_under("cinema/")
        .find(|(p, _)| p.ends_with("trailer.json"))
        .ok_or("no trailer.json")?;
    let v: Value = serde_json::from_str(t).map_err(|e| e.to_string())?;
    let seed = field(&v, "seed")?.as_u64().ok_or("seed")?;
    let ids = field(&v, "shots")?
        .as_array()
        .ok_or("shots")?
        .iter()
        .filter_map(|s| s.as_str().map(str::to_string))
        .collect();
    Ok((seed, ids))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_missing_field() {
        let s = embedded_shots();
        assert!(!s.is_empty());
        for shot in &s {
            assert_eq!(Shot::parse(&shot.to_json()).unwrap(), *shot);
        }
        let err = Shot::parse(r#"{"id":"a","stage":"b","seed":1,"duration":2,"camera":[]}"#).unwrap_err();
        assert!(err.contains("actors"));
    }

    #[test]
    fn trailer_names_only_existing_shots() {
        let (_, ids) = trailer().unwrap();
        assert!(!ids.is_empty());
        let shots = embedded_shots();
        for id in ids {
            assert!(shots.iter().any(|s| s.id == id), "unknown shot {id}");
        }
    }
}
