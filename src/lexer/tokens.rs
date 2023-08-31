use std::io::{self, Error, ErrorKind};


#[derive(Clone, Debug)]
pub enum TokenType {
    Bang, Colon, LeftParen, RightParen, Percent, Plus, Minus, Star, Newline,
    Equals, Comma, Slash, LeftAngle, RightAngle, BitAnd, BitOr, BitXor,

    Exp, BangEqual, LessEqual, GreaterEqual, Shl, Shr, And, Or,

    Ident(String), Number(f64), Text(String), Boolean(bool), Variable(String),

    Indent, Dedent,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub token_type: TokenType,
    pub lexeme: String,
    pub column: u64,
    pub line: u32,
    pub index: u64,
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct Tokens {
    inner: Vec<Token>,
    pos: u64,
}

#[allow(dead_code)]
impl Tokens {
    pub fn new() -> Self {
        Self { pos: 0, inner: Vec::new() }
    }
    
    pub fn into_inner(self) -> Vec<Token> {
        self.inner
    }
    pub fn get_ref(&self) -> &Vec<Token> {
        &self.inner
    }
    pub fn get_mut(&mut self) -> &mut Vec<Token> {
        &mut self.inner
    }
    pub fn is_at_end(&mut self) -> bool {
        (self.pos+1) as usize >= self.inner.len()
    }
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn last(&self) -> Token {
        self.inner.last().unwrap().clone()
    }

    pub fn read(&mut self) -> io::Result<Token> {
        if self.inner.len() < (self.pos+1) as usize {
            return Err(Error::new(ErrorKind::UnexpectedEof, "failed to fill whole buffer"));
        }
        let token = &self.inner[self.pos as usize];
        self.pos += 1;
        Ok(token.clone())
    }

    pub fn peek(&self) -> io::Result<Token> {
        if self.inner.len() < (self.pos+1) as usize {
            return Err(Error::new(ErrorKind::UnexpectedEof, "failed to fill whole buffer"));
        }
        let token = &self.inner[(self.pos) as usize];
        Ok(token.clone())
    }

    pub fn write_one(&mut self, buf: Token) {
        /*if buf.len() == 0 {
            return Err(Error::new(ErrorKind::WriteZero, "failed to write whole buffer"));
        }*/
        self.inner.push(buf);
    }
}

