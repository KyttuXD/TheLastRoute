//! Assertions over JSON documents, shared by `step_until` and the test-suite runner.
//!
//! An expectation is `{"path": "run.relays", "op": ">=", "value": 1}`. Paths are dot
//! separated; numbers index arrays; a final `len` gives an array / string / object length.
//! `value` may be replaced by `"var": "name.path"` (a reply saved with `save_as`).
//! Ops: == != < <= > >= approx (with "tol") exists missing truthy falsy contains in
//! (value is a list) matches (substring of a string). Quantifiers over an array path:
//! `{"any": "hunters", "where": [..]}`, `{"all": ...}`, `{"none": ...}`,
//! `{"count": "hunters", "where": [..], "op": ">=", "value": 2}`.

use serde_json::{Map, Value};

pub fn get<'a>(doc: &'a Value, path: &str) -> Option<Value> {
    if path.is_empty() || path == "." {
        return Some(doc.clone());
    }
    let mut cur = doc;
    let parts: Vec<&str> = path.split('.').collect();
    for (i, p) in parts.iter().enumerate() {
        let last = i == parts.len() - 1;
        if last && (*p == "len" || *p == "#") {
            return match cur {
                Value::Array(a) => Some(Value::from(a.len())),
                Value::String(s) => Some(Value::from(s.chars().count())),
                Value::Object(o) if !o.contains_key(*p) => Some(Value::from(o.len())),
                Value::Object(o) => o.get(*p).cloned(),
                Value::Null => Some(Value::from(0)),
                _ => None,
            };
        }
        cur = match cur {
            Value::Object(o) => o.get(*p)?,
            Value::Array(a) => {
                let idx: i64 = p.parse().ok()?;
                let idx = if idx < 0 { a.len() as i64 + idx } else { idx };
                a.get(idx as usize)?
            }
            _ => return None,
        };
    }
    Some(cur.clone())
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

fn num(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_bool().map(|b| if b { 1.0 } else { 0.0 }))
}

fn short(v: &Option<Value>) -> String {
    match v {
        None => "<missing>".into(),
        Some(v) => {
            let s = v.to_string();
            if s.len() > 160 {
                format!("{}...", &s[..160])
            } else {
                s
            }
        }
    }
}

fn compare(actual: &Option<Value>, op: &str, want: &Value, tol: f64) -> Result<bool, String> {
    Ok(match op {
        "exists" => actual.as_ref().map(|v| !v.is_null()).unwrap_or(false),
        "missing" => actual.as_ref().map(|v| v.is_null()).unwrap_or(true),
        "truthy" => actual.as_ref().map(truthy).unwrap_or(false),
        "falsy" => !actual.as_ref().map(truthy).unwrap_or(false),
        "==" | "eq" => match (actual, want) {
            (Some(a), w) if a.is_number() && w.is_number() => {
                (num(a).unwrap() - num(w).unwrap()).abs() < 1e-9
            }
            (Some(a), w) => a == w,
            (None, Value::Null) => true,
            (None, _) => false,
        },
        "!=" | "ne" => match actual {
            Some(a) if a.is_number() && want.is_number() => {
                (num(a).unwrap() - num(want).unwrap()).abs() >= 1e-9
            }
            Some(a) => a != want,
            None => !want.is_null(),
        },
        "<" | "<=" | ">" | ">=" | "approx" => {
            let Some(a) = actual.as_ref().and_then(num) else {
                return Ok(false);
            };
            let Some(w) = num(want) else {
                return Err(format!("op {op} needs a number value"));
            };
            match op {
                "<" => a < w,
                "<=" => a <= w,
                ">" => a > w,
                ">=" => a >= w,
                _ => (a - w).abs() <= tol,
            }
        }
        "contains" => match actual {
            Some(Value::Array(a)) => a.iter().any(|x| x == want),
            Some(Value::String(s)) => want.as_str().map(|w| s.contains(w)).unwrap_or(false),
            Some(Value::Object(o)) => want.as_str().map(|w| o.contains_key(w)).unwrap_or(false),
            _ => false,
        },
        "matches" => match (actual, want.as_str()) {
            (Some(Value::String(s)), Some(w)) => s.to_lowercase().contains(&w.to_lowercase()),
            _ => false,
        },
        "in" => match want {
            Value::Array(w) => actual.as_ref().map(|a| w.contains(a)).unwrap_or(false),
            _ => return Err("op in needs a list value".into()),
        },
        _ => return Err(format!("unknown op '{op}'")),
    })
}

fn want_value(e: &Value, vars: &Map<String, Value>) -> Result<Value, String> {
    if let Some(v) = e.get("var").and_then(|v| v.as_str()) {
        let (name, rest) = v.split_once('.').unwrap_or((v, ""));
        let base = vars
            .get(name)
            .ok_or_else(|| format!("no saved variable '{name}'"))?;
        return get(base, rest).ok_or_else(|| format!("saved variable '{v}' has no such path"));
    }
    Ok(e.get("value").cloned().unwrap_or(Value::Null))
}

/// Checks one expectation; Err carries a readable failure.
pub fn check(doc: &Value, e: &Value, vars: &Map<String, Value>) -> Result<(), String> {
    let label = e
        .get("label")
        .and_then(|v| v.as_str())
        .map(|s| format!("{s}: "))
        .unwrap_or_default();
    for q in ["any", "all", "none", "count"] {
        if let Some(p) = e.get(q).and_then(|v| v.as_str()) {
            let list = match get(doc, p) {
                Some(Value::Array(a)) => a,
                other => return Err(format!("{label}{q} {p}: not a list ({})", short(&other))),
            };
            let conds: Vec<Value> = match e.get("where") {
                Some(Value::Array(a)) => a.clone(),
                Some(v) => vec![v.clone()],
                None => vec![],
            };
            let n = list
                .iter()
                .filter(|item| conds.iter().all(|c| check(item, c, vars).is_ok()))
                .count();
            let ok = match q {
                "any" => n > 0,
                "all" => n == list.len(),
                "none" => n == 0,
                _ => {
                    let op = e.get("op").and_then(|v| v.as_str()).unwrap_or(">=");
                    let want = want_value(e, vars)?;
                    compare(&Some(Value::from(n)), op, &want, 0.0)?
                }
            };
            return if ok {
                Ok(())
            } else {
                Err(format!(
                    "{label}{q} {p} where {}: {n} of {} matched",
                    Value::Array(conds),
                    list.len()
                ))
            };
        }
    }
    let path = e
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("{label}expectation needs 'path' (or any/all/none/count): {e}"))?;
    let op = e.get("op").and_then(|v| v.as_str()).unwrap_or(
        if e.get("value").is_some() || e.get("var").is_some() {
            "=="
        } else {
            "truthy"
        },
    );
    let want = want_value(e, vars)?;
    let tol = e.get("tol").and_then(|v| v.as_f64()).unwrap_or(0.01);
    let actual = get(doc, path);
    if compare(&actual, op, &want, tol)? {
        Ok(())
    } else {
        Err(format!(
            "{label}{path} {op} {} (got {})",
            if matches!(op, "exists" | "missing" | "truthy" | "falsy") {
                String::new()
            } else {
                want.to_string()
            },
            short(&actual)
        ))
    }
}

/// All expectations in `list` (an object or an array of objects).
pub fn check_all(doc: &Value, list: &Value, vars: &Map<String, Value>) -> Vec<String> {
    let items: Vec<Value> = match list {
        Value::Array(a) => a.clone(),
        Value::Null => vec![],
        v => vec![v.clone()],
    };
    items
        .iter()
        .filter_map(|e| check(doc, e, vars).err())
        .collect()
}

/// The top-level sections an expectation list reads (so a probe builds only those).
pub fn sections(list: &Value, out: &mut std::collections::HashSet<String>) {
    match list {
        Value::Array(a) => a.iter().for_each(|e| sections(e, out)),
        Value::Object(o) => {
            for k in ["path", "any", "all", "none", "count"] {
                if let Some(p) = o.get(k).and_then(|v| v.as_str()) {
                    out.insert(p.split('.').next().unwrap_or("").to_string());
                }
            }
        }
        _ => {}
    }
}
