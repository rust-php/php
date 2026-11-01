//! PHP tokenizer: inline HTML, open/close tags, variables, numbers, strings (with interpolation), heredoc.

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Html(String),
    Var(String),
    Ident(String),
    Int(i64),
    Float(f64),
    Str(String),
    Template(Vec<Part>),
    Op(&'static str),
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    Lit(String),
    Code(String),
}

const OPS: &[&str] = &[
    "<=>", "===", "!==", "**=", "<<=", ">>=", "??=", "...", "?->", "<<", ">>", "==", "!=", "<>", "<=", ">=", "&&",
    "||", "++", "--", "+=", "-=", "*=", "/=", ".=", "%=", "&=", "|=", "^=", "->", "=>", "::", "??", "**", "+", "-",
    "*", "/", "%", ".", "=", "<", ">", "!", "&", "|", "^", "~", "?", ":", ";", ",", "(", ")", "[", "]", "{", "}", "@",
    "$",
];

fn starts(c: &[char], i: usize, pat: &str) -> bool {
    pat.chars().enumerate().all(|(k, p)| c.get(i + k).is_some_and(|x| x.eq_ignore_ascii_case(&p)))
}

fn is_id_start(ch: char) -> bool {
    ch.is_alphabetic() || ch == '_' || (ch as u32) >= 0x80
}

fn is_id(ch: char) -> bool {
    is_id_start(ch) || ch.is_ascii_digit()
}

pub fn lex(src: &str) -> Result<Vec<Tok>, String> {
    let c: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    loop {
        let start = i;
        let mut open = None;
        while i < c.len() {
            if c[i] == '<' && c.get(i + 1) == Some(&'?') {
                if starts(&c, i, "<?php") {
                    open = Some(5);
                    break;
                }
                if starts(&c, i, "<?=") {
                    open = Some(3);
                    break;
                }
            }
            i += 1;
        }
        if i > start {
            out.push(Tok::Html(c[start..i].iter().collect()));
        }
        let Some(n) = open else { break };
        i += n;
        if n == 3 {
            out.push(Tok::Ident("echo".into()));
        } else if c.get(i) == Some(&'\r') && c.get(i + 1) == Some(&'\n') {
            i += 2;
        } else if i < c.len() && c[i].is_whitespace() {
            i += 1;
        }
        i = lex_code(&c, i, &mut out)?;
    }
    out.push(Tok::Eof);
    Ok(out)
}

fn lex_code(c: &[char], mut i: usize, out: &mut Vec<Tok>) -> Result<usize, String> {
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
            continue;
        }
        if ch == '?' && c.get(i + 1) == Some(&'>') {
            out.push(Tok::Op(";"));
            i += 2;
            if c.get(i) == Some(&'\n') {
                i += 1;
            } else if c.get(i) == Some(&'\r') && c.get(i + 1) == Some(&'\n') {
                i += 2;
            }
            return Ok(i);
        }
        let next = c.get(i + 1).copied();
        if (ch == '#' && next != Some('[')) || (ch == '/' && next == Some('/')) {
            while i < c.len() && c[i] != '\n' && !(c[i] == '?' && c.get(i + 1) == Some(&'>')) {
                i += 1;
            }
            continue;
        }
        if ch == '/' && next == Some('*') {
            i += 2;
            while i < c.len() && !(c[i] == '*' && c.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2;
            continue;
        }
        if ch == '#' && next == Some('[') {
            // Attribute: skip to the matching bracket.
            let mut depth = 0;
            while i < c.len() {
                match c[i] {
                    '[' => depth += 1,
                    ']' => {
                        depth -= 1;
                        if depth == 0 {
                            i += 1;
                            break;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            continue;
        }
        if ch == '$' && next.is_some_and(is_id_start) {
            let s = i + 1;
            i = s;
            while i < c.len() && is_id(c[i]) {
                i += 1;
            }
            out.push(Tok::Var(c[s..i].iter().collect()));
            continue;
        }
        if is_id_start(ch) || (ch == '\\' && next.is_some_and(is_id_start)) {
            // Namespaces are flattened: `A\B\C` and `\C` both lex as the last segment `C`.
            let mut s = if ch == '\\' { i + 1 } else { i };
            i = s;
            loop {
                while i < c.len() && is_id(c[i]) {
                    i += 1;
                }
                if c.get(i) == Some(&'\\') && c.get(i + 1).copied().is_some_and(is_id_start) {
                    i += 1;
                    s = i;
                } else {
                    break;
                }
            }
            out.push(Tok::Ident(c[s..i].iter().collect()));
            continue;
        }
        if ch.is_ascii_digit() || (ch == '.' && next.is_some_and(|d| d.is_ascii_digit())) {
            out.push(lex_number(c, &mut i)?);
            continue;
        }
        if ch == '\'' {
            let mut s = String::new();
            i += 1;
            while i < c.len() && c[i] != '\'' {
                if c[i] == '\\' && matches!(c.get(i + 1), Some('\\') | Some('\'')) {
                    s.push(c[i + 1]);
                    i += 2;
                } else {
                    s.push(c[i]);
                    i += 1;
                }
            }
            if i >= c.len() {
                return Err("syntax error, unterminated string".into());
            }
            i += 1;
            out.push(Tok::Str(s));
            continue;
        }
        if ch == '"' {
            i += 1;
            let s = i;
            while i < c.len() && c[i] != '"' {
                if c[i] == '\\' {
                    i += 1;
                }
                i += 1;
            }
            if i >= c.len() {
                return Err("syntax error, unterminated string".into());
            }
            out.push(interp(&c[s..i], true));
            i += 1;
            continue;
        }
        if starts(c, i, "<<<") {
            i = lex_heredoc(c, i, out)?;
            continue;
        }
        match OPS.iter().find(|o| starts(c, i, o)) {
            Some(o) => {
                out.push(Tok::Op(o));
                i += o.len();
            }
            None => return Err(format!("syntax error, unexpected character '{ch}'")),
        }
    }
    Ok(i)
}

fn lex_number(c: &[char], i: &mut usize) -> Result<Tok, String> {
    let clean = |s: &[char]| -> String { s.iter().filter(|x| **x != '_').collect() };
    if c[*i] == '0' && matches!(c.get(*i + 1), Some('x' | 'X' | 'b' | 'B' | 'o' | 'O')) {
        let radix = match c[*i + 1].to_ascii_lowercase() {
            'x' => 16,
            'b' => 2,
            _ => 8,
        };
        let s = *i + 2;
        let mut j = s;
        while j < c.len() && (c[j].is_digit(radix) || c[j] == '_') {
            j += 1;
        }
        *i = j;
        return radix_tok(&clean(&c[s..j]), radix);
    }
    let s = *i;
    let mut j = s;
    let mut float = false;
    while j < c.len() && (c[j].is_ascii_digit() || c[j] == '_') {
        j += 1;
    }
    if c.get(j) == Some(&'.') && c.get(j + 1).is_some_and(|d| d.is_ascii_digit()) || (j == s && c.get(j) == Some(&'.')) {
        float = true;
        j += 1;
        while j < c.len() && (c[j].is_ascii_digit() || c[j] == '_') {
            j += 1;
        }
    }
    if matches!(c.get(j), Some('e' | 'E')) {
        let mut k = j + 1;
        if matches!(c.get(k), Some('+' | '-')) {
            k += 1;
        }
        if c.get(k).is_some_and(|d| d.is_ascii_digit()) {
            while k < c.len() && c[k].is_ascii_digit() {
                k += 1;
            }
            float = true;
            j = k;
        }
    }
    *i = j;
    let t = clean(&c[s..j]);
    if float {
        return t.parse().map(Tok::Float).map_err(|_| format!("bad number {t}"));
    }
    if t.len() > 1 && t.starts_with('0') && t.chars().all(|d| d.is_digit(8)) {
        return radix_tok(&t[1..], 8);
    }
    Ok(t.parse().map(Tok::Int).unwrap_or_else(|_| Tok::Float(t.parse().unwrap_or(f64::INFINITY))))
}

fn radix_tok(digits: &str, radix: u32) -> Result<Tok, String> {
    match i64::from_str_radix(digits, radix) {
        Ok(v) => Ok(Tok::Int(v)),
        Err(_) => {
            let f = digits.chars().fold(0.0, |a, d| a * radix as f64 + d.to_digit(radix).unwrap_or(0) as f64);
            Ok(Tok::Float(f))
        }
    }
}

fn lex_heredoc(c: &[char], mut i: usize, out: &mut Vec<Tok>) -> Result<usize, String> {
    i += 3;
    while c.get(i) == Some(&' ') || c.get(i) == Some(&'\t') {
        i += 1;
    }
    let quote = match c.get(i) {
        Some(q @ ('\'' | '"')) => {
            i += 1;
            Some(*q)
        }
        _ => None,
    };
    let s = i;
    while i < c.len() && is_id(c[i]) {
        i += 1;
    }
    let label: Vec<char> = c[s..i].to_vec();
    if label.is_empty() {
        return Err("syntax error, bad heredoc label".into());
    }
    if quote.is_some() {
        i += 1;
    }
    if c.get(i) == Some(&'\r') {
        i += 1;
    }
    if c.get(i) != Some(&'\n') {
        return Err("syntax error, heredoc must start a new line".into());
    }
    i += 1;
    let mut lines: Vec<Vec<char>> = Vec::new();
    loop {
        if i > c.len() {
            return Err("syntax error, unterminated heredoc".into());
        }
        let ls = i;
        while i < c.len() && c[i] != '\n' {
            i += 1;
        }
        let line = &c[ls..i];
        let indent = line.iter().take_while(|x| **x == ' ' || **x == '\t').count();
        let rest = &line[indent..];
        if rest.starts_with(&label) && !rest.get(label.len()).is_some_and(|x| is_id(*x)) {
            let body: Vec<Vec<char>> = lines.iter().map(|l| l[l.len().min(indent)..].to_vec()).collect();
            let text: Vec<char> = body.join(&'\n');
            out.push(if quote == Some('\'') { Tok::Str(text.iter().collect()) } else { interp(&text, false) });
            return Ok(ls + indent + label.len());
        }
        if i >= c.len() {
            return Err("syntax error, unterminated heredoc".into());
        }
        let mut l = line.to_vec();
        if l.last() == Some(&'\r') {
            l.pop();
        }
        lines.push(l);
        i += 1;
    }
}

/// Process escapes and `$var` / `{$expr}` interpolation in double-quoted or heredoc text.
fn interp(raw: &[char], dq: bool) -> Tok {
    let mut parts = Vec::new();
    let mut lit = String::new();
    let mut i = 0;
    while i < raw.len() {
        let ch = raw[i];
        match ch {
            '\\' if i + 1 < raw.len() => {
                let n = raw[i + 1];
                i += 2;
                match n {
                    'n' => lit.push('\n'),
                    't' => lit.push('\t'),
                    'r' => lit.push('\r'),
                    'v' => lit.push('\x0b'),
                    'e' => lit.push('\x1b'),
                    'f' => lit.push('\x0c'),
                    '\\' => lit.push('\\'),
                    '$' => lit.push('$'),
                    '"' if dq => lit.push('"'),
                    'x' if raw.get(i).is_some_and(|d| d.is_ascii_hexdigit()) => {
                        let mut v = 0;
                        let mut k = 0;
                        while k < 2 && raw.get(i).is_some_and(|d| d.is_ascii_hexdigit()) {
                            v = v * 16 + raw[i].to_digit(16).unwrap();
                            i += 1;
                            k += 1;
                        }
                        lit.push(char::from_u32(v).unwrap_or('?'));
                    }
                    'u' if raw.get(i) == Some(&'{') => {
                        let mut j = i + 1;
                        let mut v = 0;
                        while j < raw.len() && raw[j] != '}' {
                            v = v * 16 + raw[j].to_digit(16).unwrap_or(0);
                            j += 1;
                        }
                        lit.push(char::from_u32(v).unwrap_or('?'));
                        i = j + 1;
                    }
                    '0'..='7' => {
                        let mut v = n.to_digit(8).unwrap();
                        let mut k = 1;
                        while k < 3 && raw.get(i).is_some_and(|d| d.is_digit(8)) {
                            v = v * 8 + raw[i].to_digit(8).unwrap();
                            i += 1;
                            k += 1;
                        }
                        lit.push(char::from_u32(v & 0xff).unwrap_or('?'));
                    }
                    other => {
                        lit.push('\\');
                        lit.push(other);
                    }
                }
            }
            '$' if raw.get(i + 1).copied().is_some_and(is_id_start) => {
                let s = i + 1;
                let mut j = s;
                while j < raw.len() && is_id(raw[j]) {
                    j += 1;
                }
                let name: String = raw[s..j].iter().collect();
                let mut code = format!("${name}");
                if raw.get(j) == Some(&'[') {
                    if let Some(close) = raw[j..].iter().position(|x| *x == ']') {
                        let key: String = raw[j + 1..j + close].iter().collect();
                        let key_code = if key.starts_with('$') || key.parse::<i64>().is_ok() {
                            key
                        } else {
                            format!("'{key}'")
                        };
                        code = format!("{code}[{key_code}]");
                        j += close + 1;
                    }
                } else if raw.get(j) == Some(&'-') && raw.get(j + 1) == Some(&'>') && raw.get(j + 2).copied().is_some_and(is_id_start) {
                    let ps = j + 2;
                    let mut k = ps;
                    while k < raw.len() && is_id(raw[k]) {
                        k += 1;
                    }
                    let prop: String = raw[ps..k].iter().collect();
                    code = format!("{code}->{prop}");
                    j = k;
                }
                flush(&mut parts, &mut lit);
                parts.push(Part::Code(code));
                i = j;
            }
            '{' if raw.get(i + 1) == Some(&'$') => {
                let mut depth = 0;
                let mut j = i;
                while j < raw.len() {
                    match raw[j] {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                flush(&mut parts, &mut lit);
                parts.push(Part::Code(raw[i + 1..j.min(raw.len())].iter().collect()));
                i = j + 1;
            }
            _ => {
                lit.push(ch);
                i += 1;
            }
        }
    }
    if parts.is_empty() {
        return Tok::Str(lit);
    }
    flush(&mut parts, &mut lit);
    Tok::Template(parts)
}

fn flush(parts: &mut Vec<Part>, lit: &mut String) {
    if !lit.is_empty() {
        parts.push(Part::Lit(std::mem::take(lit)));
    }
}
