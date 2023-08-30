use std::io::{self, Error, ErrorKind};


#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum TokenType {
    Colon, LeftParen, RightParen, Percent, Plus, Minus, Star, Tab, Newline,
    Equals, Comma, BangEqual, Slash, LeftAngle, RightAngle,

    Ident(String), Number(f64), Text(String), Variable(String),

    EOF,
}

#[derive(Debug)]
pub struct Token {
    pub token_type: TokenType,
    pub lexeme: String,
    pub line: u32,
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

    pub fn read_exact<'a>(&'a mut self, mut buf: &'a mut [Token]) -> io::Result<usize> {
        let length = buf.len();
        if length > self.inner.len() {
            return Err(Error::new(ErrorKind::UnexpectedEof, "failed to fill whole buffer"));
        }
        buf = &mut self.inner[0..length];
        self.pos += length as u64;
        Ok(length)
    }

    pub fn write_one(&mut self, buf: Token) {
        /*if buf.len() == 0 {
            return Err(Error::new(ErrorKind::WriteZero, "failed to write whole buffer"));
        }*/
        self.inner.push(buf);
    }
}

