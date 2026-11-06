//! A reader for the small YAML subset the KyttusLab-LORE story files use: nested `key: value` maps, `- item` lists,
//! plain or single/double-quoted scalars, and full-line `#` comments. Anything else is an error, so an unusual file
//! is reported instead of being read wrong. No dependency: the crate has no YAML parser.

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Map(Vec<(String, Node)>),
    List(Vec<Node>),
    Str(String),
    Null,
}

impl Node {
    /// The value under `key` when this is a map.
    pub fn get(&self, key: &str) -> Option<&Node> {
        match self {
            Node::Map(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn entries(&self) -> &[(String, Node)] {
        match self {
            Node::Map(entries) => entries,
            _ => &[],
        }
    }
}

struct Line {
    indent: usize,
    text: String,
}

/// Parses a whole document. The root is a map.
pub fn parse(src: &str) -> Result<Node, String> {
    let lines: Vec<Line> = src
        .lines()
        .enumerate()
        .filter_map(|(n, raw)| {
            let trimmed = raw.trim_start();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                return None;
            }
            Some((n + 1, Line { indent: raw.len() - trimmed.len(), text: raw.trim_end().trim_start().to_string() }))
        })
        .map(|(n, mut l)| {
            l.text = format!("{}\u{0}{}", n, l.text); // keep the line number for error messages
            l
        })
        .collect();
    if lines.is_empty() {
        return Ok(Node::Null);
    }
    let mut p = Parser { lines, at: 0 };
    let root_indent = p.lines[0].indent;
    let node = p.block(root_indent)?;
    if p.at != p.lines.len() {
        return Err(p.err("unexpected indentation"));
    }
    Ok(node)
}

struct Parser {
    lines: Vec<Line>,
    at: usize,
}

impl Parser {
    fn err(&self, what: &str) -> String {
        match self.lines.get(self.at) {
            Some(l) => {
                let (n, text) = l.text.split_once('\u{0}').unwrap_or(("?", &l.text));
                format!("line {n}: {what}: {text}")
            }
            None => format!("end of file: {what}"),
        }
    }

    fn text(&self) -> &str {
        let t = &self.lines[self.at].text;
        t.split_once('\u{0}').map_or(t.as_str(), |(_, s)| s)
    }

    /// A block whose lines sit at `indent`: a list if it starts with `- `, otherwise a map.
    fn block(&mut self, indent: usize) -> Result<Node, String> {
        if self.at >= self.lines.len() || self.lines[self.at].indent != indent {
            return Ok(Node::Null);
        }
        if self.text() == "-" || self.text().starts_with("- ") {
            self.list(indent)
        } else {
            self.map(indent)
        }
    }

    fn list(&mut self, indent: usize) -> Result<Node, String> {
        let mut items = Vec::new();
        while self.at < self.lines.len() && self.lines[self.at].indent == indent {
            let text = self.text().to_string();
            if text != "-" && !text.starts_with("- ") {
                return Err(self.err("expected a list item"));
            }
            let rest = text.get(1..).unwrap_or("").trim_start().to_string();
            let at = self.at;
            self.at += 1;
            let item = if rest.is_empty() {
                let child = self.lines.get(self.at).map_or(0, |l| l.indent);
                if child > indent {
                    self.block(child)?
                } else {
                    Node::Null
                }
            } else if let Some((key, value)) = split_key(&rest) {
                // `- key: value` starts a map. Further `key: value` lines at the item's content column (the column
                // after `- `, indent + 2 in the usual form) belong to the same item, as in a LORE `Choices:` entry.
                // With none, it is the one-entry map the speaker lines use.
                let column = indent + (text.len() - rest.len());
                let node = self.value(&value, column).map_err(|e| self.at_line(at, &e))?;
                let mut pairs = vec![(key, node)];
                while self.at < self.lines.len() && self.lines[self.at].indent == column && !is_item(self.text()) {
                    let line = self.text().to_string();
                    let Some((k, v)) = split_key(&line) else {
                        return Err(self.err("expected 'key: value'"));
                    };
                    let at_k = self.at;
                    self.at += 1;
                    let node = self.value(&v, column).map_err(|e| self.at_line(at_k, &e))?;
                    pairs.push((k, node));
                }
                Node::Map(pairs)
            } else {
                Node::Str(unquote(&rest).map_err(|e| self.at_line(at, &e))?)
            };
            items.push(item);
        }
        Ok(Node::List(items))
    }

    fn map(&mut self, indent: usize) -> Result<Node, String> {
        let mut entries = Vec::new();
        while self.at < self.lines.len() && self.lines[self.at].indent == indent {
            let text = self.text().to_string();
            let Some((key, value)) = split_key(&text) else {
                return Err(self.err("expected 'key: value'"));
            };
            let at = self.at;
            self.at += 1;
            let node = self.value(&value, indent).map_err(|e| self.at_line(at, &e))?;
            entries.push((key, node));
        }
        Ok(Node::Map(entries))
    }

    /// Prefixes an error with the line it came from.
    fn at_line(&self, at: usize, what: &str) -> String {
        let t = &self.lines[at].text;
        let n = t.split_once('\u{0}').map_or("?", |(n, _)| n);
        format!("line {n}: {what}")
    }

    /// The value after `key:`. An empty value is the block nested under the key (or null).
    fn value(&mut self, raw: &str, parent_indent: usize) -> Result<Node, String> {
        if !raw.is_empty() {
            return Ok(Node::Str(unquote(raw)?));
        }
        let child = self.lines.get(self.at).map_or(0, |l| l.indent);
        if child > parent_indent {
            self.block(child)
        } else {
            Ok(Node::Null)
        }
    }
}

/// Whether a line starts a list item (`- ...` or a bare `-`).
fn is_item(text: &str) -> bool {
    text == "-" || text.starts_with("- ")
}

/// Splits `key: value` (or `key:`) at the first colon that is followed by a space or ends the line. The key has no
/// colon-space in it, so speaker lines such as `Felix: "text: with colons"` split at the speaker.
fn split_key(text: &str) -> Option<(String, String)> {
    if let Some(stripped) = text.strip_suffix(':') {
        return Some((stripped.trim().to_string(), String::new()));
    }
    let at = text.find(": ")?;
    let key = text[..at].trim();
    if key.is_empty() || key.starts_with('\'') || key.starts_with('"') {
        return None;
    }
    Some((key.to_string(), text[at + 2..].trim().to_string()))
}

/// A scalar: `'...'` (with `''` for an apostrophe), `"..."` (with `\"` and `\\`), or plain text.
fn unquote(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw.len() >= 2 && raw.starts_with('\'') && raw.ends_with('\'') {
        return Ok(raw[1..raw.len() - 1].replace("''", "'"));
    }
    if raw.len() >= 2 && raw.starts_with('"') && raw.ends_with('"') {
        let inner = &raw[1..raw.len() - 1];
        let mut out = String::new();
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some(e) => out.push(e),
                    None => return Err("a double-quoted scalar ends in a backslash".to_string()),
                }
            } else {
                out.push(c);
            }
        }
        return Ok(out);
    }
    if raw.starts_with('\'') || raw.starts_with('"') {
        return Err(format!("unterminated quote: {raw}"));
    }
    Ok(raw.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_nested_maps_lists_and_quoted_scalars() {
        let src = "Top:\n  Story:\n    Chapter 1:\n      Title: Prep\n      Summary:\n        - ???: It's late.\n        - Felix: '\"Don''t tell.\"'\n";
        let root = parse(src).unwrap();
        let chapter = root.get("Top").unwrap().get("Story").unwrap().get("Chapter 1").unwrap();
        assert_eq!(chapter.get("Title"), Some(&Node::Str("Prep".into())));
        let Some(Node::List(items)) = chapter.get("Summary") else { panic!("summary is a list") };
        assert_eq!(items[0], Node::Map(vec![("???".into(), Node::Str("It's late.".into()))]));
        assert_eq!(items[1], Node::Map(vec![("Felix".into(), Node::Str("\"Don't tell.\"".into()))]));
    }

    #[test]
    fn colons_inside_the_text_do_not_split_the_speaker() {
        let node = parse("S:\n  - Renata: \"Time: twelve.\"\n").unwrap();
        let Some(Node::List(items)) = node.get("S") else { panic!() };
        assert_eq!(items[0], Node::Map(vec![("Renata".into(), Node::Str("Time: twelve.".into()))]));
    }

    #[test]
    fn comments_and_blank_lines_are_skipped() {
        let node = parse("# top\nA:\n\n  # inside\n  B: 1\n").unwrap();
        assert_eq!(node.get("A").unwrap().get("B"), Some(&Node::Str("1".into())));
    }

    #[test]
    fn a_list_item_continues_with_keys_at_its_content_column() {
        let src = "C:\n  Choices:\n    - Text: \"Go left\"\n      Needs:\n        - lantern\n        - door_open\n      Sets:\n        - went_left\n      Goto: Chapter 2\n    - Text: 'Stay'\n";
        let node = parse(src).unwrap();
        let Some(Node::List(items)) = node.get("C").unwrap().get("Choices") else { panic!("choices is a list") };
        assert_eq!(items.len(), 2, "the second item starts at the dash, not the content column");
        assert_eq!(
            items[0],
            Node::Map(vec![
                ("Text".into(), Node::Str("Go left".into())),
                ("Needs".into(), Node::List(vec![Node::Str("lantern".into()), Node::Str("door_open".into())])),
                ("Sets".into(), Node::List(vec![Node::Str("went_left".into())])),
                ("Goto".into(), Node::Str("Chapter 2".into())),
            ])
        );
        assert_eq!(items[1], Node::Map(vec![("Text".into(), Node::Str("Stay".into()))]));
    }

    #[test]
    fn a_one_key_item_stays_a_one_entry_map() {
        let node = parse("S:\n  - Felix: \"Hi.\"\n  - ???: \"Dark.\"\n").unwrap();
        let Some(Node::List(items)) = node.get("S") else { panic!() };
        assert_eq!(items[0], Node::Map(vec![("Felix".into(), Node::Str("Hi.".into()))]));
        assert_eq!(items[1].entries().len(), 1);
    }

    #[test]
    fn an_unterminated_quote_is_an_error_with_its_line() {
        let err = parse("A:\n  B: 'open\n").unwrap_err();
        assert!(err.contains("line 2"), "{err}");
    }
}
