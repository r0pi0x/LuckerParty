//! KeyValues text as Steam writes it (`libraryfolders.vdf`,
//! `appmanifest_*.acf`) and as our own saved files use it
//! (`serverbrowser.vdf`): quoted strings with `\"` and `\\` escapes, bare
//! words, braces, `//` comments.

/// A KeyValues node: a value or a block of named children, in order.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Value(String),
    Block(Vec<(String, Node)>),
}

impl Node {
    /// The first child named `key` (any case), in a block.
    pub fn get(&self, key: &str) -> Option<&Node> {
        match self {
            Node::Block(children) => children.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v),
            Node::Value(_) => None,
        }
    }

    /// The value of child `key`, when it is a value.
    pub fn value(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Node::Value(v) => Some(v),
            Node::Block(_) => None,
        }
    }

    /// A block's children (empty for a value).
    pub fn children(&self) -> &[(String, Node)] {
        match self {
            Node::Block(children) => children,
            Node::Value(_) => &[],
        }
    }
}

/// The tokens of KeyValues text: quoted strings (unescaped), bare words,
/// braces; comments skipped.
pub fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' | '}' => out.push(c.to_string()),
            '"' => {
                let mut s = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            if let Some(n) = chars.next() {
                                s.push(n);
                            }
                        }
                        '"' => break,
                        _ => s.push(c),
                    }
                }
                out.push(s);
            }
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            c if c.is_whitespace() => {}
            c => {
                let mut s = c.to_string();
                while let Some(&n) = chars.peek() {
                    if n.is_whitespace() || n == '{' || n == '}' || n == '"' {
                        break;
                    }
                    s.push(n);
                    chars.next();
                }
                out.push(s);
            }
        }
    }
    out
}

/// Parse KeyValues text into a block of its top-level entries.
pub fn parse(text: &str) -> Node {
    let t = tokens(text);
    let mut i = 0;
    Node::Block(block(&t, &mut i))
}

fn block(t: &[String], i: &mut usize) -> Vec<(String, Node)> {
    let mut out = Vec::new();
    while *i < t.len() {
        if t[*i] == "}" {
            *i += 1;
            break;
        }
        let key = t[*i].clone();
        *i += 1;
        match t.get(*i).map(String::as_str) {
            Some("{") => {
                *i += 1;
                out.push((key, Node::Block(block(t, i))));
            }
            Some("}") | None => {}
            Some(v) => {
                out.push((key, Node::Value(v.to_string())));
                *i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_blocks_and_comments() {
        let root = parse("// c\n\"a\" { \"path\" \"C:\\\\Steam \\\"x\\\"\" b { c d } }");
        let a = root.get("A").unwrap();
        assert_eq!(a.value("path"), Some("C:\\Steam \"x\""));
        assert_eq!(a.get("b").unwrap().value("c"), Some("d"));
        assert_eq!(a.children().len(), 2);
    }
}
