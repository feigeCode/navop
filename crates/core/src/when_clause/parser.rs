use super::tokenizer::Token;
use super::{WhenClauseError, WhenContext, invalid};
use serde_json::Value as JsonValue;

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Expr {
    Value(WhenValue),
    Not(Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Eq(Box<Expr>, Box<Expr>),
    Ne(Box<Expr>, Box<Expr>),
    In(Box<Expr>, Vec<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum WhenValue {
    Null,
    Bool(bool),
    String(String),
    Path(String),
}

impl Expr {
    pub(super) fn eval(&self, context: &WhenContext) -> WhenValue {
        match self {
            Expr::Value(value) => value.resolve(context),
            Expr::Not(expr) => WhenValue::Bool(!expr.eval(context).truthy()),
            Expr::And(left, right) => {
                WhenValue::Bool(left.eval(context).truthy() && right.eval(context).truthy())
            }
            Expr::Or(left, right) => {
                WhenValue::Bool(left.eval(context).truthy() || right.eval(context).truthy())
            }
            Expr::Eq(left, right) => WhenValue::Bool(left.eval(context) == right.eval(context)),
            Expr::Ne(left, right) => WhenValue::Bool(left.eval(context) != right.eval(context)),
            Expr::In(value, items) => eval_in(value, items, context),
        }
    }
}

fn eval_in(value: &Expr, items: &[Expr], context: &WhenContext) -> WhenValue {
    let value = value.eval(context);
    WhenValue::Bool(items.iter().any(|item| item.eval(context) == value))
}

impl WhenValue {
    fn resolve(&self, context: &WhenContext) -> Self {
        match self {
            WhenValue::Path(path) => context
                .get_path(path)
                .map(Self::from_json)
                .unwrap_or(WhenValue::Null),
            other => other.clone(),
        }
    }

    fn from_json(value: &JsonValue) -> Self {
        match value {
            JsonValue::Bool(value) => WhenValue::Bool(*value),
            JsonValue::String(value) => WhenValue::String(value.clone()),
            JsonValue::Number(value) => WhenValue::String(value.to_string()),
            _ => WhenValue::Null,
        }
    }

    pub(super) fn truthy(&self) -> bool {
        match self {
            WhenValue::Bool(value) => *value,
            WhenValue::String(value) => !value.is_empty(),
            WhenValue::Path(_) | WhenValue::Null => false,
        }
    }
}

pub(super) struct Parser {
    input: String,
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub(super) fn new(input: &str, tokens: Vec<Token>) -> Self {
        Self {
            input: input.to_string(),
            tokens,
            pos: 0,
        }
    }

    pub(super) fn parse_expr(&mut self) -> Result<Expr, WhenClauseError> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr, WhenClauseError> {
        let mut expr = self.parse_and()?;
        while self.consume(&Token::Or) {
            expr = Expr::Or(Box::new(expr), Box::new(self.parse_and()?));
        }
        Ok(expr)
    }

    fn parse_and(&mut self) -> Result<Expr, WhenClauseError> {
        let mut expr = self.parse_compare()?;
        while self.consume(&Token::And) {
            expr = Expr::And(Box::new(expr), Box::new(self.parse_compare()?));
        }
        Ok(expr)
    }

    fn parse_compare(&mut self) -> Result<Expr, WhenClauseError> {
        let left = self.parse_unary()?;
        if self.consume(&Token::Eq) {
            return Ok(Expr::Eq(Box::new(left), Box::new(self.parse_unary()?)));
        }
        if self.consume(&Token::Ne) {
            return Ok(Expr::Ne(Box::new(left), Box::new(self.parse_unary()?)));
        }
        if self.consume(&Token::In) {
            return Ok(Expr::In(Box::new(left), self.parse_array()?));
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, WhenClauseError> {
        if self.consume(&Token::Bang) {
            return Ok(Expr::Not(Box::new(self.parse_unary()?)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Expr, WhenClauseError> {
        match self.next() {
            Some(Token::Ident(value)) => Ok(Expr::Value(WhenValue::Path(value))),
            Some(Token::Str(value)) => Ok(Expr::Value(WhenValue::String(value))),
            Some(Token::Bool(value)) => Ok(Expr::Value(WhenValue::Bool(value))),
            Some(Token::Null) => Ok(Expr::Value(WhenValue::Null)),
            Some(Token::LParen) => self.parse_group(),
            other => Err(self.err(format!("expected expression, got {other:?}"))),
        }
    }

    fn parse_group(&mut self) -> Result<Expr, WhenClauseError> {
        let expr = self.parse_expr()?;
        self.expect(Token::RParen)?;
        Ok(expr)
    }

    fn parse_array(&mut self) -> Result<Vec<Expr>, WhenClauseError> {
        self.expect(Token::LBracket)?;
        let mut items = Vec::new();
        if self.consume(&Token::RBracket) {
            return Ok(items);
        }
        loop {
            items.push(self.parse_primary()?);
            if self.consume(&Token::RBracket) {
                return Ok(items);
            }
            self.expect(Token::Comma)?;
        }
    }

    pub(super) fn expect_end(&self) -> Result<(), WhenClauseError> {
        if self.pos == self.tokens.len() {
            Ok(())
        } else {
            Err(self.err("unexpected trailing token"))
        }
    }

    fn expect(&mut self, token: Token) -> Result<(), WhenClauseError> {
        if self.consume(&token) {
            Ok(())
        } else {
            Err(self.err(format!("expected {token:?}")))
        }
    }

    fn consume(&mut self, token: &Token) -> bool {
        let consumed = self.peek() == Some(token);
        if consumed {
            self.pos += 1;
        }
        consumed
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.pos).cloned();
        if token.is_some() {
            self.pos += 1;
        }
        token
    }

    fn err(&self, message: impl Into<String>) -> WhenClauseError {
        invalid(&self.input, message)
    }
}
