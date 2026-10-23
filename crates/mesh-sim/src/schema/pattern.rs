//! The regular-expression subset used by JSON Schema `pattern` keywords in the labrun/1
//! schema (ECMA-262 semantics, unanchored search): literals, `.`, escapes (`\d \w \s`
//! and escaped punctuation), classes with ranges and negation, groups with
//! alternation, quantifiers `* + ? {n} {n,} {n,m}`, and the anchors `^ $`. Anything
//! else is rejected when the pattern is compiled, so an unsupported schema fails loudly
//! instead of validating loosely.

#[derive(Debug, Clone, PartialEq)]
enum Node {
    Char(char),
    Any,
    Class {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
    Start,
    End,
    Group(Vec<Vec<Node>>),
    Repeat {
        node: Box<Node>,
        min: u32,
        max: Option<u32>,
    },
}

/// A compiled pattern.
#[derive(Debug, Clone, PartialEq)]
pub struct Pattern {
    alts: Vec<Vec<Node>>,
}

struct Parser<'a> {
    s: &'a [char],
    i: usize,
}

fn class_escape(c: char) -> Option<Vec<(char, char)>> {
    Some(match c {
        'd' => vec![('0', '9')],
        'w' => vec![('a', 'z'), ('A', 'Z'), ('0', '9'), ('_', '_')],
        's' => vec![
            (' ', ' '),
            ('\t', '\t'),
            ('\n', '\n'),
            ('\r', '\r'),
            ('\x0B', '\x0C'),
        ],
        _ => return None,
    })
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        self.i += 1;
        c
    }

    fn alternation(&mut self) -> Result<Vec<Vec<Node>>, String> {
        let mut alts = vec![self.sequence()?];
        while self.peek() == Some('|') {
            self.i += 1;
            alts.push(self.sequence()?);
        }
        Ok(alts)
    }

    fn sequence(&mut self) -> Result<Vec<Node>, String> {
        let mut seq = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            let atom = self.atom()?;
            let atom = self.quantified(atom)?;
            seq.push(atom);
        }
        Ok(seq)
    }

    fn atom(&mut self) -> Result<Node, String> {
        let c = self.bump().ok_or("unexpected end")?;
        Ok(match c {
            '^' => Node::Start,
            '$' => Node::End,
            '.' => Node::Any,
            '(' => {
                if self.peek() == Some('?') {
                    return Err("group modifiers are not supported".into());
                }
                let alts = self.alternation()?;
                if self.bump() != Some(')') {
                    return Err("unclosed group".into());
                }
                Node::Group(alts)
            }
            '[' => self.class()?,
            '\\' => {
                let e = self.bump().ok_or("dangling escape")?;
                match class_escape(e) {
                    Some(ranges) => Node::Class {
                        negated: false,
                        ranges,
                    },
                    None if e.is_ascii_alphanumeric() => {
                        return Err(format!("unsupported escape \\{e}"));
                    }
                    None => Node::Char(e),
                }
            }
            '*' | '+' | '?' | '{' | '}' | ']' => {
                return Err(format!("unexpected '{c}'"));
            }
            c => Node::Char(c),
        })
    }

    fn class(&mut self) -> Result<Node, String> {
        let negated = self.peek() == Some('^');
        if negated {
            self.i += 1;
        }
        let mut ranges = Vec::new();
        let mut first = true;
        loop {
            let c = self.bump().ok_or("unclosed class")?;
            if c == ']' && !first {
                break;
            }
            first = false;
            let lo = if c == '\\' {
                let e = self.bump().ok_or("dangling escape")?;
                if let Some(r) = class_escape(e) {
                    ranges.extend(r);
                    continue;
                }
                if e.is_ascii_alphanumeric() {
                    return Err(format!("unsupported escape \\{e}"));
                }
                e
            } else {
                c
            };
            if self.peek() == Some('-') && self.s.get(self.i + 1).is_some_and(|n| *n != ']') {
                self.i += 1;
                let hi = self.bump().ok_or("unclosed range")?;
                if hi == '\\' || hi < lo {
                    return Err("bad class range".into());
                }
                ranges.push((lo, hi));
            } else {
                ranges.push((lo, lo));
            }
        }
        Ok(Node::Class { negated, ranges })
    }

    fn number(&mut self) -> Option<u32> {
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.i += 1;
        }
        let s: String = self.s[start..self.i].iter().collect();
        s.parse().ok()
    }

    fn quantified(&mut self, atom: Node) -> Result<Node, String> {
        let (min, max) = match self.peek() {
            Some('*') => (0, None),
            Some('+') => (1, None),
            Some('?') => (0, Some(1)),
            Some('{') => {
                self.i += 1;
                let min = self.number().ok_or("bad quantifier")?;
                let max = if self.peek() == Some(',') {
                    self.i += 1;
                    if self.peek() == Some('}') {
                        None
                    } else {
                        Some(self.number().ok_or("bad quantifier")?)
                    }
                } else {
                    Some(min)
                };
                if self.peek() != Some('}') || max.is_some_and(|m| m < min) {
                    return Err("bad quantifier".into());
                }
                (min, max)
            }
            _ => return Ok(atom),
        };
        self.i += 1;
        if matches!(self.peek(), Some('?' | '+')) {
            return Err("lazy/possessive quantifiers are not supported".into());
        }
        if matches!(atom, Node::Start | Node::End) {
            return Err("quantified anchor".into());
        }
        Ok(Node::Repeat {
            node: Box::new(atom),
            min,
            max,
        })
    }
}

type Cont<'a> = &'a mut dyn FnMut(usize) -> bool;

fn match_node(n: &Node, s: &[char], i: usize, k: Cont<'_>) -> bool {
    match n {
        Node::Char(c) => i < s.len() && s[i] == *c && k(i + 1),
        Node::Any => i < s.len() && s[i] != '\n' && s[i] != '\r' && k(i + 1),
        Node::Class { negated, ranges } => {
            i < s.len()
                && {
                    let c = s[i];
                    ranges.iter().any(|(lo, hi)| (*lo..=*hi).contains(&c)) != *negated
                }
                && k(i + 1)
        }
        Node::Start => i == 0 && k(i),
        Node::End => i == s.len() && k(i),
        Node::Group(alts) => alts.iter().any(|seq| match_seq(seq, s, i, k)),
        Node::Repeat { node, min, max } => repeat(node, *min, *max, 0, s, i, k),
    }
}

fn repeat(
    n: &Node,
    min: u32,
    max: Option<u32>,
    count: u32,
    s: &[char],
    i: usize,
    k: Cont<'_>,
) -> bool {
    if max.is_none_or(|m| count < m)
        && match_node(n, s, i, &mut |j| {
            j != i && repeat(n, min, max, count + 1, s, j, k)
        })
    {
        return true;
    }
    count >= min && k(i)
}

fn match_seq(seq: &[Node], s: &[char], i: usize, k: Cont<'_>) -> bool {
    match seq.split_first() {
        None => k(i),
        Some((first, rest)) => match_node(first, s, i, &mut |j| match_seq(rest, s, j, k)),
    }
}

impl Pattern {
    /// Compiles `src`, refusing constructs outside the supported subset.
    pub fn new(src: &str) -> Result<Pattern, String> {
        let chars: Vec<char> = src.chars().collect();
        let mut p = Parser { s: &chars, i: 0 };
        let alts = p.alternation()?;
        if p.i != chars.len() {
            return Err(format!("unbalanced ')' in {src:?}"));
        }
        Ok(Pattern { alts })
    }

    /// ECMA-262 `test`: a match anywhere in `text`.
    pub fn is_match(&self, text: &str) -> bool {
        let s: Vec<char> = text.chars().collect();
        (0..=s.len()).any(|start| {
            self.alts
                .iter()
                .any(|seq| match_seq(seq, &s, start, &mut |_| true))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labrun_patterns() {
        let run = Pattern::new("^r-[0-9]{8}-[0-9]{4}-[0-9a-z]{4}$").unwrap();
        assert!(run.is_match("r-20261023-0912-ab12"));
        assert!(!run.is_match("r-20261023-0912-ab1"));
        assert!(!run.is_match("r-20261023-0912-AB12"));
        assert!(!run.is_match("xr-20261023-0912-ab12"));
        let ts = Pattern::new(
            "^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\\.[0-9]{1,6})?Z$",
        )
        .unwrap();
        assert!(ts.is_match("2026-10-23T09:12:03.120Z"));
        assert!(ts.is_match("2026-10-23T09:12:03Z"));
        assert!(!ts.is_match("2026-10-23T09:12:03.1234567Z"));
        assert!(!ts.is_match("2026-10-23 09:12:03Z"));
        let ev = Pattern::new("^[a-z][A-Za-z0-9]*$").unwrap();
        assert!(ev.is_match("linkDown"));
        assert!(!ev.is_match("LinkDown"));
        assert!(!ev.is_match("link-down"));
    }

    #[test]
    fn general_subset() {
        let p = Pattern::new("ab|c(d|e)+").unwrap();
        assert!(p.is_match("xxab"));
        assert!(p.is_match("cdede"));
        assert!(!p.is_match("cx"));
        assert!(Pattern::new("a{2,}b").unwrap().is_match("caaab"));
        assert!(!Pattern::new("^a{2,3}$").unwrap().is_match("aaaa"));
        assert!(Pattern::new("[^0-9]\\d\\.").unwrap().is_match("x1."));
        assert!(Pattern::new("^(a*)*$").unwrap().is_match("aaa"));
        for bad in ["(?:a)", "a+?", "\\p{L}", "[z-a]", "(a", "a)", "a{3,1}"] {
            assert!(Pattern::new(bad).is_err(), "{bad}");
        }
    }
}
