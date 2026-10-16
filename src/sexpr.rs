//! A tiny reader for S-expressions, the bracket format KiCad uses for its files:
//!
//!     (comp (ref "R1") (value "1k") (footprint "Resistor_THT:R_Axial_..."))
//!
//! Everything is either an atom (a word or a "quoted string") or a list of
//! things in brackets. By KiCad convention the first atom of a list names it.

#[derive(Debug)]
pub enum Sexpr {
    Atom(String),
    List(Vec<Sexpr>),
}

impl Sexpr {
    /// The name of a list: the `comp` in `(comp ...)`.
    pub fn head(&self) -> Option<&str> {
        match self {
            Sexpr::List(items) => match items.first() {
                Some(Sexpr::Atom(name)) => Some(name),
                _ => None,
            },
            Sexpr::Atom(_) => None,
        }
    }

    /// The sub-lists called `name`: every `(node ...)` inside a `(net ...)`.
    pub fn children<'a>(&'a self, name: &str) -> impl Iterator<Item = &'a Sexpr> {
        let items = match self {
            Sexpr::List(items) => &items[..],
            Sexpr::Atom(_) => &[],
        };
        items.iter().filter(move |item| item.head() == Some(name))
    }

    pub fn child(&self, name: &str) -> Option<&Sexpr> {
        self.children(name).next()
    }

    /// The text in `(name "text")`: `comp.value("ref")` gives "R1".
    pub fn value(&self, name: &str) -> Option<&str> {
        match self.child(name)? {
            Sexpr::List(items) => match items.get(1) {
                Some(Sexpr::Atom(text)) => Some(text),
                _ => None,
            },
            Sexpr::Atom(_) => None,
        }
    }
}

pub fn parse(text: &str) -> Result<Sexpr, String> {
    let mut chars = text.chars().peekable();
    // Lists we are still inside of. The innermost one is last.
    let mut open: Vec<Vec<Sexpr>> = Vec::new();
    while let Some(&ch) = chars.peek() {
        let finished = match ch {
            _ if ch.is_whitespace() => {
                chars.next();
                continue;
            }
            '(' => {
                chars.next();
                open.push(Vec::new());
                continue;
            }
            ')' => {
                chars.next();
                Sexpr::List(open.pop().ok_or("too many ')'")?)
            }
            '"' => {
                chars.next();
                let mut s = String::new();
                loop {
                    match chars.next().ok_or("a quoted string never ends")? {
                        '"' => break,
                        '\\' => s.push(chars.next().ok_or("a quoted string never ends")?),
                        c => s.push(c),
                    }
                }
                Sexpr::Atom(s)
            }
            _ => {
                let mut s = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_whitespace() || c == '(' || c == ')' || c == '"' {
                        break;
                    }
                    s.push(c);
                    chars.next();
                }
                Sexpr::Atom(s)
            }
        };
        match open.last_mut() {
            Some(parent) => parent.push(finished),
            None => return Ok(finished), // closed the outermost list: done
        }
    }
    Err("missing ')' at the end".into())
}
