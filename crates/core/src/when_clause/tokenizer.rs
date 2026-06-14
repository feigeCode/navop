use super::{WhenClauseError, invalid};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Token {
    Ident(String),
    Str(String),
    Bool(bool),
    Null,
    Bang,
    And,
    Or,
    Eq,
    Ne,
    In,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
}

pub(super) fn tokenize(input: &str) -> Result<Vec<Token>, WhenClauseError> {
    let chars = input.chars().collect::<Vec<_>>();
    let mut tokens = Vec::new();
    let mut pos = 0;
    while pos < chars.len() {
        match chars[pos] {
            c if c.is_whitespace() => pos += 1,
            '\'' => read_string(input, &chars, &mut pos, &mut tokens)?,
            c if is_ident_start(c) => read_ident(&chars, &mut pos, &mut tokens),
            '&' if take_pair(&chars, &mut pos, '&') => tokens.push(Token::And),
            '|' if take_pair(&chars, &mut pos, '|') => tokens.push(Token::Or),
            '=' if take_pair(&chars, &mut pos, '=') => tokens.push(Token::Eq),
            '!' if take_pair(&chars, &mut pos, '=') => tokens.push(Token::Ne),
            '!' => push_one(&mut tokens, &mut pos, Token::Bang),
            '(' => push_one(&mut tokens, &mut pos, Token::LParen),
            ')' => push_one(&mut tokens, &mut pos, Token::RParen),
            '[' => push_one(&mut tokens, &mut pos, Token::LBracket),
            ']' => push_one(&mut tokens, &mut pos, Token::RBracket),
            ',' => push_one(&mut tokens, &mut pos, Token::Comma),
            c => return Err(invalid(input, format!("unexpected character `{c}`"))),
        }
    }
    Ok(tokens)
}

fn read_string(
    input: &str,
    chars: &[char],
    pos: &mut usize,
    tokens: &mut Vec<Token>,
) -> Result<(), WhenClauseError> {
    *pos += 1;
    let start = *pos;
    while *pos < chars.len() && chars[*pos] != '\'' {
        *pos += 1;
    }
    if *pos >= chars.len() {
        return Err(invalid(input, "unterminated string literal"));
    }
    tokens.push(Token::Str(chars[start..*pos].iter().collect()));
    *pos += 1;
    Ok(())
}

fn read_ident(chars: &[char], pos: &mut usize, tokens: &mut Vec<Token>) {
    let start = *pos;
    while *pos < chars.len() && is_ident_part(chars[*pos]) {
        *pos += 1;
    }
    let value: String = chars[start..*pos].iter().collect();
    let token = match value.as_str() {
        "true" => Token::Bool(true),
        "false" => Token::Bool(false),
        "null" => Token::Null,
        "in" => Token::In,
        _ => Token::Ident(value),
    };
    tokens.push(token);
}

fn take_pair(chars: &[char], pos: &mut usize, expected: char) -> bool {
    if chars.get(*pos + 1) == Some(&expected) {
        *pos += 2;
        true
    } else {
        false
    }
}

fn push_one(tokens: &mut Vec<Token>, pos: &mut usize, token: Token) {
    tokens.push(token);
    *pos += 1;
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_ident_part(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')
}
