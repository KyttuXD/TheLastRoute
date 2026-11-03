//! Port of `backend/env_file.mjs`: a small dotenv reader. `index.js` loads `backend/.env` and then `./.env` before
//! anything reads the environment; variables that are already set (Pterodactyl panel variables, variables given to
//! the process) always win.

#[cfg(test)]
use std::collections::HashMap;
use std::path::Path;

/// `decodeDoubleQuoted`: `\n`, `\r`, `\t`, `\\` and `\"` escapes, left to right; any other backslash stays.
fn decode_double_quoted(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek().copied() {
                Some('n') => {
                    chars.next();
                    out.push('\n');
                }
                Some('r') => {
                    chars.next();
                    out.push('\r');
                }
                Some('t') => {
                    chars.next();
                    out.push('\t');
                }
                Some(e @ ('\\' | '"')) => {
                    chars.next();
                    out.push(e);
                }
                _ => out.push(c),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// JavaScript `\s` (Unicode white space plus U+FEFF).
fn js_space(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}'
}

/// `value.replace(/\s+#.*$/, "")`: the first whitespace run followed by `#` whose `.*` reaches the end of the line
/// (`.` does not cross `\r`, U+2028 or U+2029).
fn strip_inline_comment(value: &str) -> &str {
    let line_break = |c: char| matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}');
    let chars: Vec<(usize, char)> = value.char_indices().collect();
    let mut i = 0;
    while i < chars.len() {
        if js_space(chars[i].1) {
            // the regex engine tries the leftmost start first and \s+ is greedy, backtracking to find a "#"
            let mut j = i;
            while j < chars.len() && js_space(chars[j].1) {
                j += 1;
            }
            // greedy \s+ then "#": the "#" must directly follow some prefix of the run; only the end of the run can
            // be followed by a non-space "#"
            if j < chars.len() && chars[j].1 == '#' && !chars[j + 1..].iter().any(|(_, c)| line_break(*c)) {
                return &value[..chars[i].0];
            }
            i += 1;
        } else {
            i += 1;
        }
    }
    value
}

fn js_trim_end(text: &str) -> &str {
    text.trim_end_matches(js_space)
}

fn js_trim_start(text: &str) -> &str {
    text.trim_start_matches(js_space)
}

fn valid_key(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `parseEnvironmentFile(contents)`: the key/value pairs in file order (a repeated key keeps its first position and
/// takes the last value, like the JS `Map`).
pub fn parse_environment_file(contents: &str) -> Vec<(String, String)> {
    let mut parsed: Vec<(String, String)> = Vec::new();
    let text = contents.strip_prefix('\u{feff}').unwrap_or(contents);
    for original_line in text.split('\n') {
        let original_line = original_line.strip_suffix('\r').unwrap_or(original_line);
        let mut line = crate::util::js_trim(original_line);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("export ") {
            line = js_trim_start(rest);
        }
        let separator = match line.find('=') {
            Some(index) if index >= 1 => index,
            _ => continue,
        };
        let key = crate::util::js_trim(&line[..separator]);
        if !valid_key(key) {
            continue;
        }
        let mut value = crate::util::js_trim(&line[separator + 1..]).to_string();
        if value.starts_with('"') {
            if let Some(closing) = value.rfind('"') {
                if closing > 0 {
                    value = decode_double_quoted(&value[1..closing]);
                }
            }
        } else if value.starts_with('\'') {
            if let Some(closing) = value.rfind('\'') {
                if closing > 0 {
                    value = value[1..closing].to_string();
                }
            }
        } else {
            value = js_trim_end(strip_inline_comment(&value)).to_string();
        }
        if let Some(entry) = parsed.iter_mut().find(|(k, _)| k == key) {
            entry.1 = value;
        } else {
            parsed.push((key.to_string(), value));
        }
    }
    parsed
}

/// `loadEnvironmentFile(path, environment)` with an explicit environment map: Ok(false) when the file does not
/// exist; keys already present in `environment` are left alone.
#[cfg(test)]
pub fn load_environment_file_into(path: &Path, environment: &mut HashMap<String, String>) -> std::io::Result<bool> {
    let contents = match read_text(path)? {
        Some(text) => text,
        None => return Ok(false),
    };
    for (key, value) in parse_environment_file(&contents) {
        environment.entry(key).or_insert(value);
    }
    Ok(true)
}

/// `loadEnvironmentFile(path)` against the process environment (what `index.js` does): sets only variables that
/// are not already set (an empty value counts as set).
pub fn load_environment_file(path: &Path) -> std::io::Result<bool> {
    let contents = match read_text(path)? {
        Some(text) => text,
        None => return Ok(false),
    };
    for (key, value) in parse_environment_file(&contents) {
        if std::env::var_os(&key).is_none() {
            std::env::set_var(&key, value);
        }
    }
    Ok(true)
}

/// `readFile(path, "utf8")`; None for ENOENT. Invalid UTF-8 becomes U+FFFD like Node's decoder.
fn read_text(path: &Path) -> std::io::Result<Option<String>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(String::from_utf8_lossy(&bytes).into_owned())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get<'a>(parsed: &'a [(String, String)], key: &str) -> Option<&'a str> {
        parsed.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    #[test]
    fn environment_files_support_common_dotenv_syntax() {
        let parsed = parse_environment_file(
            "\n# deployment settings\nLASTROUTE_HTTP_PORT=8105\nexport LASTROUTE_ADMIN_PASSWORD=\"secret value\"\nSINGLE='literal # value'\nPLAIN=value # comment\nMULTILINE=\"first\\nsecond\"\n",
        );
        assert_eq!(get(&parsed, "LASTROUTE_HTTP_PORT"), Some("8105"));
        assert_eq!(get(&parsed, "LASTROUTE_ADMIN_PASSWORD"), Some("secret value"));
        assert_eq!(get(&parsed, "SINGLE"), Some("literal # value"));
        assert_eq!(get(&parsed, "PLAIN"), Some("value"));
        assert_eq!(get(&parsed, "MULTILINE"), Some("first\nsecond"));
    }

    #[test]
    fn existing_panel_variables_take_precedence_over_an_env_file() {
        let directory = std::env::temp_dir().join(format!("lastroute-env-{}-{}", std::process::id(), crate::util::hex(&crate::util::random_bytes(4))));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join(".env");
        std::fs::write(&path, "PANEL_VALUE=from-file\nFILE_ONLY=loaded\n").unwrap();
        let mut environment = HashMap::new();
        environment.insert("PANEL_VALUE".to_string(), "from-panel".to_string());

        assert!(load_environment_file_into(&path, &mut environment).unwrap());
        assert_eq!(environment["PANEL_VALUE"], "from-panel");
        assert_eq!(environment["FILE_ONLY"], "loaded");
        assert!(!load_environment_file_into(&directory.join("missing.env"), &mut environment).unwrap());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn edge_cases_match_the_node_parser() {
        let parsed = parse_environment_file("\u{feff}A=1\r\nB=\"x\\qy\"\nC=\"open\n=skip\nD='a'b'\nE=one#two\nA=2\nF=\"\"\n");
        assert_eq!(parsed[0], ("A".to_string(), "2".to_string()));
        assert_eq!(get(&parsed, "B"), Some("x\\qy"));
        assert_eq!(get(&parsed, "C"), Some("\"open"));
        assert_eq!(get(&parsed, "D"), Some("a'b"));
        assert_eq!(get(&parsed, "E"), Some("one#two"));
        assert_eq!(get(&parsed, "F"), Some(""));
    }
}
