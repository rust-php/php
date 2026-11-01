//! Tokenizer for a small PHP subset: inline HTML, `<?php`, `echo`, numbers, strings, operators.

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Html(String),
    Echo,
    Int(i64),
    Float(f64),
    Str(String),
    Op(char),
    Semi,
    LParen,
    RParen,
}

pub fn lex(src: &str) -> Result<Vec<Token>, String> {
    let mut out = Vec::new();
    let mut rest = src;
    loop {
        match rest.find("<?php") {
            Some(i) => {
                if i > 0 {
                    out.push(Token::Html(rest[..i].to_string()));
                }
                rest = &rest[i + 5..];
            }
            None => {
                if !rest.is_empty() {
                    out.push(Token::Html(rest.to_string()));
                }
                return Ok(out);
            }
        }
        let (code, after) = match rest.find("?>") {
            Some(j) => (&rest[..j], &rest[j + 2..]),
            None => (rest, ""),
        };
        lex_code(code, &mut out)?;
        // A single newline directly after `?>` is swallowed, as in PHP.
        rest = after.strip_prefix('\n').unwrap_or(after);
        if after.is_empty() {
            return Ok(out);
        }
    }
}

fn lex_code(code: &str, out: &mut Vec<Token>) -> Result<(), String> {
    let c: Vec<char> = code.chars().collect();
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        match ch {
            _ if ch.is_whitespace() => i += 1,
            ';' => {
                out.push(Token::Semi);
                i += 1;
            }
            '(' => {
                out.push(Token::LParen);
                i += 1;
            }
            ')' => {
                out.push(Token::RParen);
                i += 1;
            }
            '+' | '-' | '*' | '/' | '.' | ',' if !(ch == '.' && c.get(i + 1).is_some_and(|d| d.is_ascii_digit())) => {
                out.push(Token::Op(ch));
                i += 1;
            }
            '\'' | '"' => {
                let q = ch;
                let mut s = String::new();
                i += 1;
                while i < c.len() && c[i] != q {
                    if c[i] == '\\' && i + 1 < c.len() {
                        let n = c[i + 1];
                        match (q, n) {
                            (_, '\\') => s.push('\\'),
                            (_, x) if x == q => s.push(q),
                            ('"', 'n') => s.push('\n'),
                            ('"', 't') => s.push('\t'),
                            _ => {
                                s.push('\\');
                                s.push(n);
                            }
                        }
                        i += 2;
                    } else {
                        s.push(c[i]);
                        i += 1;
                    }
                }
                if i >= c.len() {
                    return Err("unterminated string".into());
                }
                i += 1;
                out.push(Token::Str(s));
            }
            _ if ch.is_ascii_digit() || ch == '.' => {
                let start = i;
                let mut float = false;
                while i < c.len() && (c[i].is_ascii_digit() || c[i] == '.') {
                    float |= c[i] == '.';
                    i += 1;
                }
                let t: String = c[start..i].iter().collect();
                if float {
                    out.push(Token::Float(t.parse().map_err(|_| format!("bad number {t}"))?));
                } else {
                    out.push(Token::Int(t.parse().map_err(|_| format!("bad number {t}"))?));
                }
            }
            _ if ch.is_alphabetic() || ch == '_' => {
                let start = i;
                while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_') {
                    i += 1;
                }
                let w: String = c[start..i].iter().collect();
                if w.eq_ignore_ascii_case("echo") {
                    out.push(Token::Echo);
                } else {
                    return Err(format!("unexpected identifier {w}"));
                }
            }
            _ => return Err(format!("unexpected character {ch}")),
        }
    }
    Ok(())
}
