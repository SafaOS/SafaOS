#[derive(Debug, Clone, Copy)]
pub enum Operation {
    Number(u8),
    Remove,
    Clear,
    Add,
    Mul,
    Div,
    Sub,
    Dot,
    Results,
}

#[derive(Debug, Clone)]
enum Token {
    Plus,           // +
    Minus,          // -
    Mul,            // *
    Div,            // /
    Number(String), // num.num
}

impl Token {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Mul => "*",
            Self::Plus => "+",
            Self::Minus => "-",
            Self::Div => "/",
            Self::Number(s) => &s,
        }
    }
}

pub struct Eval<'a> {
    data: &'a [Token],
    at: usize,
}

impl<'a> Eval<'a> {
    #[inline]
    fn peek(&self, from: usize) -> Option<&Token> {
        self.data.get(self.at + from)
    }

    #[inline]
    fn next(&mut self) -> Option<&Token> {
        let r = self.data.get(self.at)?;
        self.at += 1;
        Some(r)
    }

    pub fn eval_next_num(&mut self) -> Result<f64, &'static str> {
        match self.next() {
            Some(Token::Number(n)) => n.parse::<f64>().map_err(|_| "Parse Error"),
            _ => Err("Unexpected Token"),
        }
    }

    pub fn eval_next_multiplicative(&mut self) -> Result<f64, &'static str> {
        let mut lhs = self.eval_next_num()?;
        while let Some(t) = self.peek(0) {
            match t {
                Token::Div => {
                    self.next();
                    let rhs = self.eval_next_num()?;
                    lhs = lhs / rhs;
                    if !lhs.is_normal() {
                        return Err("Inf");
                    }
                }
                Token::Mul => {
                    self.next();
                    let rhs = self.eval_next_num()?;
                    lhs = lhs * rhs;
                    if !lhs.is_normal() {
                        return Err("Inf");
                    }
                }
                _ => break,
            }
        }
        Ok(lhs)
    }

    pub fn eval_next_additive(&mut self) -> Result<f64, &'static str> {
        let mut lhs = self.eval_next_multiplicative()?;
        while let Some(t) = self.peek(0) {
            match t {
                Token::Plus => {
                    self.next();
                    let rhs = self.eval_next_multiplicative()?;
                    lhs = lhs + rhs;
                    if !lhs.is_normal() {
                        return Err("Inf");
                    }
                }
                Token::Minus => {
                    self.next();
                    let rhs = self.eval_next_multiplicative()?;
                    lhs = lhs - rhs;
                    if !lhs.is_normal() {
                        return Err("Inf");
                    }
                }
                _ => break,
            }
        }
        Ok(lhs)
    }

    pub fn eval(mut self) -> Result<f64, &'static str> {
        self.eval_next_additive()
    }
}

#[derive(Debug, Default)]
pub struct LexerData {
    tokens: Vec<Token>,
}

impl LexerData {
    pub fn view_to(&self, buf: &mut String) {
        buf.clear();
        for tok in &self.tokens {
            buf.push_str(tok.as_str());
            buf.push(' ');
        }
    }

    /// Eval results or Errors.
    pub fn eval(&self) -> Result<f64, &'static str> {
        Eval {
            data: &self.tokens,
            at: 0,
        }
        .eval()
    }

    fn add_op(&mut self, t: Token) {
        match self.tokens.last() {
            Some(Token::Div | Token::Minus | Token::Mul | Token::Plus) | None => {}
            _ => self.tokens.push(t),
        }
    }

    pub fn execute(&mut self, op: &Operation) -> Result<(), &'static str> {
        match op {
            Operation::Results => {
                let r = self.eval()?;
                self.tokens.clear();
                self.tokens.push(Token::Number(r.to_string()));
            }
            Operation::Add => self.add_op(Token::Plus),
            Operation::Div => self.add_op(Token::Div),
            Operation::Mul => self.add_op(Token::Mul),
            Operation::Sub => self.add_op(Token::Minus),
            Operation::Clear => self.tokens.clear(),
            Operation::Remove => match self.tokens.pop() {
                Some(Token::Number(mut n)) => {
                    if n.pop().is_some() && !n.is_empty() {
                        self.tokens.push(Token::Number(n));
                    }
                }
                _ => {}
            },
            Operation::Dot => {
                if let Some(Token::Number(n)) = self.tokens.last_mut()
                    && n.len() < 15
                {
                    n.push('.');
                }
            }
            Operation::Number(digit) => {
                if let Some(Token::Number(n)) = self.tokens.last_mut() {
                    if n.len() < 15 {
                        n.push(((*digit) + b'0') as char);
                    }
                } else {
                    self.tokens.push(Token::Number(digit.to_string()));
                }
            }
        }

        Ok(())
    }
}
