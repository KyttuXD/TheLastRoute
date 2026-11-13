//! The per-region room and prop tables (`lr.region_rooms/1`, src/bin/last_route/data/region_rooms/<region id>.json).
//!
//! One file per region of KT's map concept (the region ids are the data file names of data/regions/). A file holds:
//! - `rooms`: the room types of the region. A room is either `"template": "code"` (furnished by a template that
//!   `furnished.rs` already has, by role name) or `"template": "data"` (its `recipe` is a list of items, expanded here
//!   into `Spec`s). Each room carries its draw `weight`, its size profile (`min`, `preferred`, `max`, in cells, `tables`).
//! - `props`: the prop table of the region (type, authored size, solid, weight), a superset of the region catalogue's
//!   `prop_set` (data/regions/<id>.json).
//! - `tunnel_dressing` (tunnels only): what the mineshaft corridors carry, by spacing and chance.
//!
//! Recipe items: `{type, side, depth, facing, wall, mount, loose, mount_y, mirror, claim, chance, min_side_span,
//! min_depth_span}`. `side` and `depth` are an expression or a range `[from, to, step]` (to is included); the
//! expression names are `half`, `front`, `mid`, `back`, `side_span`, `depth_span` with `+ - * /` and whole numbers.
//! A spec that would land outside the room's interior is dropped here, so data cannot put furniture in a wall.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use super::{sp, Spec};
use crate::world::rng::Rng;

/// The schema this module reads.
pub const SCHEMA: &str = "lr.region_rooms/1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Template {
    /// Furnished by the code template of the same role name (furnished.rs `template_for`).
    Code,
    /// Furnished by the room's own `recipe`.
    Data,
}

#[derive(Clone, Copy, Debug)]
enum Var {
    Half,
    Front,
    Mid,
    Back,
    SideSpan,
    DepthSpan,
}

#[derive(Clone, Debug)]
enum Atom {
    Num(i32),
    Var(Var),
}

/// A sum of products: `sign * a (op b)...`.
#[derive(Clone, Debug)]
struct Expr(Vec<(i32, Vec<(char, Atom)>)>);

struct Ctx {
    half: i32,
    front: i32,
    mid: i32,
    side_span: i32,
    depth_span: i32,
}

impl Expr {
    fn parse(text: &str) -> Result<Expr, String> {
        let s: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        if s.is_empty() {
            return Err("empty expression".into());
        }
        let mut terms: Vec<(i32, Vec<(char, Atom)>)> = Vec::new();
        let mut sign = 1;
        let mut cur = String::new();
        let flush = |sign: i32, cur: &mut String, terms: &mut Vec<(i32, Vec<(char, Atom)>)>| -> Result<(), String> {
            if cur.is_empty() {
                return Ok(());
            }
            let mut factors: Vec<(char, Atom)> = Vec::new();
            let mut op = '*';
            let mut word = String::new();
            for ch in cur.chars().chain(std::iter::once('\0')) {
                if ch == '*' || ch == '/' || ch == '\0' {
                    factors.push((op, atom(&word)?));
                    word.clear();
                    op = ch;
                } else {
                    word.push(ch);
                }
            }
            terms.push((sign, factors));
            cur.clear();
            Ok(())
        };
        for ch in s.chars() {
            if ch == '+' || ch == '-' {
                flush(sign, &mut cur, &mut terms)?;
                sign = if ch == '-' { -1 } else { 1 };
            } else {
                cur.push(ch);
            }
        }
        flush(sign, &mut cur, &mut terms)?;
        if terms.is_empty() {
            return Err(format!("bad expression \"{text}\""));
        }
        Ok(Expr(terms))
    }

    fn eval(&self, c: &Ctx) -> i32 {
        let mut total = 0;
        for (sign, factors) in &self.0 {
            let mut v = 1;
            for (op, a) in factors {
                let n = match a {
                    Atom::Num(n) => *n,
                    Atom::Var(Var::Half) => c.half,
                    Atom::Var(Var::Front) => c.front,
                    Atom::Var(Var::Mid) => c.mid,
                    Atom::Var(Var::Back) => 0,
                    Atom::Var(Var::SideSpan) => c.side_span,
                    Atom::Var(Var::DepthSpan) => c.depth_span,
                };
                if *op == '/' {
                    v = if n == 0 { 0 } else { v / n };
                } else {
                    v *= n;
                }
            }
            total += sign * v;
        }
        total
    }
}

fn atom(word: &str) -> Result<Atom, String> {
    if let Ok(n) = word.parse::<i32>() {
        return Ok(Atom::Num(n));
    }
    Ok(Atom::Var(match word {
        "half" => Var::Half,
        "front" => Var::Front,
        "mid" => Var::Mid,
        "back" => Var::Back,
        "side_span" => Var::SideSpan,
        "depth_span" => Var::DepthSpan,
        _ => return Err(format!("unknown name \"{word}\" in an expression")),
    }))
}

#[derive(Clone, Debug)]
enum Axis {
    One(Expr),
    Range(Expr, Expr, i32),
}

impl Axis {
    fn parse(v: &serde_json::Value, what: &str) -> Result<Axis, String> {
        use serde_json::Value;
        match v {
            Value::Number(n) => Ok(Axis::One(Expr::parse(&n.to_string())?)),
            Value::String(s) => Ok(Axis::One(Expr::parse(s)?)),
            Value::Array(a) if a.len() == 3 => {
                let e = |x: &Value| match x {
                    Value::Number(n) => Expr::parse(&n.to_string()),
                    Value::String(s) => Expr::parse(s),
                    _ => Err(format!("{what}: a range holds numbers or expressions")),
                };
                let step = a[2].as_i64().filter(|s| *s > 0 && *s < 64).ok_or_else(|| format!("{what}: the step of a range is a whole number from 1 to 63"))?;
                Ok(Axis::Range(e(&a[0])?, e(&a[1])?, step as i32))
            }
            _ => Err(format!("{what}: expected a number, an expression or [from, to, step]")),
        }
    }

    fn values(&self, c: &Ctx) -> Vec<i32> {
        match self {
            Axis::One(e) => vec![e.eval(c)],
            Axis::Range(a, b, step) => {
                let (from, to) = (a.eval(c), b.eval(c));
                let mut out = Vec::new();
                let mut v = from;
                while v <= to {
                    out.push(v);
                    v += step;
                }
                out
            }
        }
    }
}

#[derive(Clone, Debug)]
struct Item {
    ty: &'static str,
    side: Axis,
    depth: Axis,
    facing: &'static str,
    wall: bool,
    mount: bool,
    loose: bool,
    mount_y: f64,
    mirror: bool,
    claim: bool,
    chance: Option<f64>,
    min_side_span: i32,
    min_depth_span: i32,
}

/// One room type of a region.
#[derive(Clone, Debug)]
pub struct RoomEntry {
    pub role: String,
    /// Draw weight: the role appears this many times in the region's `room_roles` list.
    pub weight: u32,
    pub template: Template,
    pub min: [i32; 2],
    pub preferred: [i32; 2],
    pub max: [i32; 2],
    /// The least and most tables a room of this type holds (0, 0 when it has none).
    pub tables: [i32; 2],
    recipe: Vec<Item>,
}

/// One entry of a region's prop table.
#[derive(Clone, Debug, PartialEq)]
pub struct PropEntry {
    pub prop_type: String,
    pub size: [f32; 3],
    pub solid: bool,
    pub weight: u32,
}

/// One mineshaft corridor decoration: every `every` cells along the tunnel (or by `chance` per cell when `every` is 0).
#[derive(Clone, Debug, PartialEq)]
pub struct TunnelRule {
    pub prop_type: String,
    /// `edge` (against a tunnel wall), `centre` (on the walking line) or `ceiling`.
    pub placement: String,
    pub every: i32,
    pub chance: f64,
}

#[derive(Clone, Debug)]
pub struct RegionRooms {
    pub region: String,
    /// The map concept colour of the region, six hex digits.
    pub colour: String,
    /// Relative draw weight against a normal region (1.0). Below 1 is rarer.
    pub relative_weight: f64,
    /// True for the region KT has not decided on (7F3F3F): a mixed utility region.
    pub undecided: bool,
    /// Tunnels only: the corridor width in cells (0 for every other region).
    pub tunnel_width: i32,
    /// Region-catalogue room roles (data/regions/<id>.json) that this table replaces with other roles.
    pub replaces: Vec<String>,
    pub rooms: Vec<RoomEntry>,
    pub props: Vec<PropEntry>,
    pub tunnel_dressing: Vec<TunnelRule>,
}

fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

fn int2(v: &serde_json::Value, what: &str) -> Result<[i32; 2], String> {
    let a = v.as_array().filter(|a| a.len() == 2).ok_or_else(|| format!("{what}: expected [x, z]"))?;
    let n = |x: &serde_json::Value| x.as_i64().filter(|n| (0..=64).contains(n)).map(|n| n as i32).ok_or_else(|| format!("{what}: expected whole numbers from 0 to 64"));
    Ok([n(&a[0])?, n(&a[1])?])
}

const FACINGS: [&str; 5] = ["open", "back", "left", "right", "center"];

fn parse_item(v: &serde_json::Value, role: &str) -> Result<Item, String> {
    let what = |k: &str| format!("room {role} item {k}");
    let o = v.as_object().ok_or_else(|| format!("room {role}: a recipe item is an object"))?;
    let ty = o.get("type").and_then(|t| t.as_str()).filter(|t| !t.is_empty()).ok_or_else(|| format!("room {role}: an item needs a \"type\""))?;
    let facing = o.get("facing").and_then(|f| f.as_str()).unwrap_or("open");
    if !FACINGS.contains(&facing) {
        return Err(format!("room {role}: facing \"{facing}\" is not one of {FACINGS:?}"));
    }
    let flag = |k: &str| o.get(k).and_then(|b| b.as_bool()).unwrap_or(false);
    let chance = match o.get("chance") {
        None => None,
        Some(c) => Some(c.as_f64().filter(|c| (0.0..=1.0).contains(c)).ok_or_else(|| what("chance (0 to 1)"))?),
    };
    Ok(Item {
        ty: leak(ty),
        side: Axis::parse(o.get("side").ok_or_else(|| what("side"))?, &what("side"))?,
        depth: Axis::parse(o.get("depth").ok_or_else(|| what("depth"))?, &what("depth"))?,
        facing: leak(facing),
        wall: flag("wall"),
        mount: flag("mount"),
        loose: flag("loose"),
        mount_y: o.get("mount_y").and_then(|n| n.as_f64()).unwrap_or(0.0),
        mirror: flag("mirror"),
        claim: flag("claim"),
        chance,
        min_side_span: o.get("min_side_span").and_then(|n| n.as_i64()).unwrap_or(0) as i32,
        min_depth_span: o.get("min_depth_span").and_then(|n| n.as_i64()).unwrap_or(0) as i32,
    })
}

/// Parses one `lr.region_rooms/1` file.
pub fn parse(file: &str, text: &str) -> Result<RegionRooms, String> {
    use serde_json::Value;
    let v: Value = serde_json::from_str(text).map_err(|e| format!("{file}: {e}"))?;
    let o = v.as_object().ok_or_else(|| format!("{file}: not an object"))?;
    let err = |m: String| format!("{file}: {m}");
    if o.get("schema").and_then(|s| s.as_str()) != Some(SCHEMA) {
        return Err(err(format!("schema must be \"{SCHEMA}\"")));
    }
    let region = o.get("region").and_then(|s| s.as_str()).filter(|s| !s.is_empty()).ok_or_else(|| err("\"region\" is missing".into()))?.to_string();
    let colour = o.get("colour").and_then(|s| s.as_str()).unwrap_or("").to_string();
    let relative_weight = o.get("relative_weight").and_then(|n| n.as_f64()).unwrap_or(1.0);
    if !(relative_weight > 0.0 && relative_weight <= 4.0) {
        return Err(err("relative_weight is a number above 0 up to 4".into()));
    }
    let undecided = o.get("undecided").and_then(|b| b.as_bool()).unwrap_or(false);
    let tunnel_width = o.get("tunnel_width").and_then(|n| n.as_i64()).filter(|n| (0..=9).contains(n)).unwrap_or(0) as i32;
    let replaces: Vec<String> = o.get("replaces").and_then(|r| r.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
    let mut rooms = Vec::new();
    for r in o.get("rooms").and_then(|r| r.as_array()).ok_or_else(|| err("\"rooms\" is missing".into()))? {
        let ro = r.as_object().ok_or_else(|| err("a room is an object".into()))?;
        let role = ro.get("role").and_then(|s| s.as_str()).filter(|s| !s.is_empty()).ok_or_else(|| err("a room needs a \"role\"".into()))?.to_string();
        let weight = ro.get("weight").and_then(|n| n.as_u64()).filter(|n| (1..=32).contains(n)).ok_or_else(|| err(format!("room {role}: weight is a whole number from 1 to 32")))? as u32;
        let template = match ro.get("template").and_then(|s| s.as_str()) {
            Some("code") => Template::Code,
            Some("data") => Template::Data,
            _ => return Err(err(format!("room {role}: template is \"code\" or \"data\""))),
        };
        let size = ro.get("size").and_then(|s| s.as_object()).ok_or_else(|| err(format!("room {role}: size is missing")))?;
        let get = |k: &str| int2(size.get(k).unwrap_or(&Value::Null), &format!("{file} room {role} size.{k}"));
        let (min, preferred, max) = (get("min")?, get("preferred")?, get("max")?);
        if (0..2).any(|i| !(min[i] <= preferred[i] && preferred[i] <= max[i])) {
            return Err(err(format!("room {role}: size needs min <= preferred <= max")));
        }
        let tables = match ro.get("tables") {
            None => [0, 0],
            Some(t) => int2(t, &format!("{file} room {role} tables"))?,
        };
        let mut recipe = Vec::new();
        for item in ro.get("recipe").and_then(|r| r.as_array()).map(|a| a.as_slice()).unwrap_or(&[]) {
            recipe.push(parse_item(item, &role).map_err(err)?);
        }
        if template == Template::Data && recipe.is_empty() {
            return Err(err(format!("room {role}: a data room needs a recipe")));
        }
        rooms.push(RoomEntry { role, weight, template, min, preferred, max, tables, recipe });
    }
    let mut props = Vec::new();
    for p in o.get("props").and_then(|r| r.as_array()).ok_or_else(|| err("\"props\" is missing".into()))? {
        let po = p.as_object().ok_or_else(|| err("a prop is an object".into()))?;
        let ty = po.get("type").and_then(|s| s.as_str()).filter(|s| !s.is_empty()).ok_or_else(|| err("a prop needs a \"type\"".into()))?;
        let sz = po.get("size").and_then(|s| s.as_array()).filter(|a| a.len() == 3).ok_or_else(|| err(format!("prop {ty}: size is [x, y, z]")))?;
        let n = |i: usize| sz[i].as_f64().filter(|n| *n > 0.0 && *n < 40.0).map(|n| n as f32).ok_or_else(|| err(format!("prop {ty}: size numbers are above 0")));
        props.push(PropEntry {
            prop_type: ty.to_string(),
            size: [n(0)?, n(1)?, n(2)?],
            solid: po.get("solid").and_then(|b| b.as_bool()).unwrap_or(true),
            weight: po.get("weight").and_then(|n| n.as_u64()).filter(|n| (1..=32).contains(n)).unwrap_or(1) as u32,
        });
    }
    let mut tunnel_dressing = Vec::new();
    for t in o.get("tunnel_dressing").and_then(|r| r.as_array()).map(|a| a.as_slice()).unwrap_or(&[]) {
        let to = t.as_object().ok_or_else(|| err("a tunnel rule is an object".into()))?;
        let placement = to.get("placement").and_then(|s| s.as_str()).unwrap_or("");
        if !["edge", "centre", "ceiling"].contains(&placement) {
            return Err(err("tunnel placement is edge, centre or ceiling".into()));
        }
        tunnel_dressing.push(TunnelRule {
            prop_type: to.get("type").and_then(|s| s.as_str()).filter(|s| !s.is_empty()).ok_or_else(|| err("a tunnel rule needs a type".into()))?.to_string(),
            placement: placement.to_string(),
            every: to.get("every").and_then(|n| n.as_i64()).filter(|n| (0..=256).contains(n)).unwrap_or(0) as i32,
            chance: to.get("chance").and_then(|n| n.as_f64()).filter(|c| (0.0..=1.0).contains(c)).unwrap_or(0.0),
        });
    }
    Ok(RegionRooms { region, colour, relative_weight, undecided, tunnel_width, replaces, rooms, props, tunnel_dressing })
}

/// Every built-in table, by region id.
pub fn all() -> &'static BTreeMap<String, RegionRooms> {
    static TABLES: OnceLock<BTreeMap<String, RegionRooms>> = OnceLock::new();
    TABLES.get_or_init(|| {
        let mut out = BTreeMap::new();
        for (path, text) in crate::content::embedded::data_under("region_rooms/") {
            let t = parse(path, text).unwrap_or_else(|e| panic!("built-in room table: {e}"));
            out.insert(t.region.clone(), t);
        }
        out
    })
}

/// The authored size of a prop type from the tables' `props` lists (the first table, by region id, that lists it).
/// `furnished.rs` reads it for the types that have no row in the generated Godot tables.
pub fn data_prop_size(prop_type: &str) -> Option<[f32; 3]> {
    static SIZES: OnceLock<BTreeMap<String, [f32; 3]>> = OnceLock::new();
    SIZES
        .get_or_init(|| {
            let mut out = BTreeMap::new();
            for t in all().values() {
                for p in &t.props {
                    out.entry(p.prop_type.clone()).or_insert(p.size);
                }
            }
            out
        })
        .get(prop_type)
        .copied()
}

/// The table of a region.
pub fn table(region: &str) -> Option<&'static RegionRooms> {
    all().get(region)
}

/// The room entry of a role in a region.
pub fn room(region: &str, role: &str) -> Option<&'static RoomEntry> {
    table(region)?.rooms.iter().find(|r| r.role == role)
}

/// Whether the role is a data room (its recipe lives in data) of the region.
pub fn has_data_recipe(region: &str, role: &str) -> bool {
    room(region, role).is_some_and(|r| r.template == Template::Data)
}

impl RoomEntry {
    /// The prop types the room's recipe uses (a type once per item).
    pub fn recipe_types(&self) -> Vec<&'static str> {
        self.recipe.iter().map(|i| i.ty).collect()
    }
}

impl RegionRooms {
    /// The `room_roles` list this table gives: each role repeated by its weight, in table order (the shape of
    /// data/regions/<id>.json `room_roles`).
    pub fn role_list(&self) -> Vec<&str> {
        self.rooms.iter().flat_map(|r| std::iter::repeat(r.role.as_str()).take(r.weight as usize)).collect()
    }
}

/// The specs of a data room: one mirror draw, then the items in order (a `chance` item draws once per spec).
/// `side_span` runs across the room and `depth_span` from the back wall to the door, as in `furnished.rs`.
pub fn specs(region: &str, role: &str, side_span: i32, depth_span: i32, rng: &mut Rng) -> Vec<Spec> {
    let Some(entry) = room(region, role).filter(|r| r.template == Template::Data) else {
        return Vec::new();
    };
    let mirror: i32 = if rng.next_float() < 0.5 { -1 } else { 1 };
    expand(&entry.recipe, side_span, depth_span, mirror, rng)
}

fn expand(items: &[Item], side_span: i32, depth_span: i32, mirror: i32, rng: &mut Rng) -> Vec<Spec> {
    let ctx = Ctx { half: (side_span - 3) / 2, front: depth_span - 3, mid: depth_span / 2, side_span, depth_span };
    // the room's interior: `half` cells either side of the door column, from the back wall to the cell before the door row
    let (max_side, max_depth) = (ctx.half, depth_span - 2);
    let mut out = Vec::new();
    for item in items {
        if side_span < item.min_side_span || depth_span < item.min_depth_span {
            continue;
        }
        for depth in item.depth.values(&ctx) {
            for side in item.side.values(&ctx) {
                let side = if item.mirror { side * mirror } else { side };
                if let Some(chance) = item.chance {
                    if rng.next_float() >= chance {
                        continue;
                    }
                }
                if side.abs() > max_side || depth < 0 || depth > max_depth {
                    continue;
                }
                let mut s = sp(item.ty, side, depth, item.facing);
                if item.wall {
                    s = s.wall();
                }
                if item.mount {
                    s = s.mount();
                }
                if item.loose {
                    s = s.loose();
                }
                if item.mount_y != 0.0 {
                    s = s.my(item.mount_y);
                }
                if item.claim {
                    s = s.claim();
                }
                out.push(s);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(side_span: i32, depth_span: i32) -> Ctx {
        Ctx { half: (side_span - 3) / 2, front: depth_span - 3, mid: depth_span / 2, side_span, depth_span }
    }

    #[test]
    fn expressions_follow_the_room_size() {
        let c = ctx(13, 15);
        let e = |s: &str| Expr::parse(s).unwrap().eval(&c);
        assert_eq!(e("half"), 5);
        assert_eq!(e("-half+3"), -2);
        assert_eq!(e("front - 2"), 10);
        assert_eq!(e("mid"), 7);
        assert_eq!(e("2*half-1"), 9);
        assert_eq!(e("side_span/2"), 6);
        assert_eq!(e("back"), 0);
        assert!(Expr::parse("bogus").is_err());
        assert!(Expr::parse("").is_err());
    }

    #[test]
    fn ranges_include_both_ends_and_step() {
        let v = serde_json::json!(["-half+1", "half-1", 2]);
        let a = Axis::parse(&v, "t").unwrap();
        assert_eq!(a.values(&ctx(13, 13)), vec![-4, -2, 0, 2, 4]);
        assert!(Axis::parse(&serde_json::json!([1, 2, 0]), "t").is_err());
        assert!(Axis::parse(&serde_json::json!([1, 2]), "t").is_err());
    }

    fn one_room(recipe: &str) -> String {
        format!(
            r#"{{"schema":"{SCHEMA}","region":"t","rooms":[{{"role":"r","weight":2,"template":"data","size":{{"min":[7,7],"preferred":[9,9],"max":[11,11]}},"recipe":[{recipe}]}}],"props":[{{"type":"crate","size":[1,1,1]}}]}}"#
        )
    }

    #[test]
    fn a_bad_table_is_refused_with_a_reason() {
        assert!(parse("t.json", &one_room(r#"{"type":"crate","side":0,"depth":0}"#)).is_ok());
        let e = parse("t.json", &one_room(r#"{"type":"crate","side":"nope","depth":0}"#)).unwrap_err();
        assert!(e.contains("unknown name"), "{e}");
        let e = parse("t.json", &one_room(r#"{"type":"crate","side":0,"depth":0,"facing":"up"}"#)).unwrap_err();
        assert!(e.contains("facing"), "{e}");
        let e = parse("t.json", &one_room(r#"{"type":"crate","side":0}"#)).unwrap_err();
        assert!(e.contains("depth"), "{e}");
        let e = parse("t.json", &one_room("")).unwrap_err();
        assert!(e.contains("recipe"), "{e}");
        assert!(parse("t.json", "{}").is_err());
    }

    #[test]
    fn data_cannot_put_furniture_outside_the_room() {
        let t = parse("t.json", &one_room(r#"{"type":"crate","side":[-30,30,1],"depth":[-3,40,1]}"#)).unwrap();
        let mut rng = Rng::new("bounds");
        let specs = expand(&t.rooms[0].recipe, 9, 11, 1, &mut rng);
        assert!(!specs.is_empty());
        let half = (9 - 3) / 2;
        for s in &specs {
            assert!(s.side.abs() <= half, "side {}", s.side);
            assert!(s.depth >= 0 && s.depth <= 11 - 2, "depth {}", s.depth);
        }
    }

    #[test]
    fn mirror_and_chance_and_min_span_work() {
        let t = parse(
            "t.json",
            &one_room(
                r#"{"type":"a","side":2,"depth":1,"mirror":true},{"type":"b","side":0,"depth":1,"chance":0.0},{"type":"c","side":0,"depth":1,"chance":1.0},{"type":"d","side":0,"depth":1,"min_side_span":15}"#,
            ),
        )
        .unwrap();
        let mut rng = Rng::new("m");
        let on = expand(&t.rooms[0].recipe, 11, 11, 1, &mut rng);
        let off = expand(&t.rooms[0].recipe, 11, 11, -1, &mut rng);
        assert_eq!(on.iter().find(|s| s.ty == "a").unwrap().side, 2);
        assert_eq!(off.iter().find(|s| s.ty == "a").unwrap().side, -2);
        assert!(on.iter().all(|s| s.ty != "b" && s.ty != "d"));
        assert!(on.iter().any(|s| s.ty == "c"));
        let wide = expand(&t.rooms[0].recipe, 15, 11, 1, &mut rng);
        assert!(wide.iter().any(|s| s.ty == "d"));
    }

    /// The built-in prison table: the mess hall holds six tables, and changing the table's data changes the room.
    #[test]
    fn the_mess_hall_has_six_tables_and_follows_its_data() {
        let (path, text) = crate::content::embedded::data_under("region_rooms/prison.json").next().expect("prison table is embedded");
        let tables = |t: &RegionRooms, span: i32| {
            let e = t.rooms.iter().find(|r| r.role == "prison_mess_hall").unwrap();
            assert_eq!(e.tables, [6, 6]);
            expand(&e.recipe, span, span, 1, &mut Rng::new("mess")).iter().filter(|s| s.ty == "prisonCafeteriaTable").count()
        };
        let t = parse(path, text).unwrap();
        assert_eq!(tables(&t, 21), 6);
        // one row of tables instead of two: half as many
        let one_row = parse(path, &text.replace("[4, 10, 6]", "[4, 4, 6]")).unwrap();
        assert_eq!(tables(&one_row, 21), 3);
        // a smaller hall drops the columns that would reach a wall
        assert!(tables(&t, 11) < 6);
    }

    #[test]
    fn every_built_in_table_parses_and_has_rooms_and_props() {
        let mut seen = 0;
        for (path, text) in crate::content::embedded::data_under("region_rooms/") {
            let t = parse(path, text).unwrap_or_else(|e| panic!("{e}"));
            assert!(path.ends_with(&format!("/{}.json", t.region)), "{path}: the file name is the region id");
            assert!(!t.rooms.is_empty() && !t.props.is_empty(), "{}", t.region);
            assert_eq!(t.colour.len(), 6, "{}: a six digit map colour", t.region);
            seen += 1;
        }
        assert!(seen >= 11, "one table per region of the map concept");
    }

    #[test]
    fn the_role_list_repeats_each_role_by_weight() {
        let t = parse("t.json", &one_room(r#"{"type":"crate","side":0,"depth":0}"#)).unwrap();
        assert_eq!(t.role_list(), ["r", "r"]);
    }
}
