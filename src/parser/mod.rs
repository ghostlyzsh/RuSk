use std::io;

use anyhow::{Result, anyhow, Error};

use crate::lexer::tokens::{Tokens, TokenType, Token};

use self::ptr::P;

pub struct Parser {
    pub tokens: Tokens,
    pub exprs: Vec<P<Expr>>,
    pub filename: String,
    pub file: String,
}

impl Parser {
    pub fn new(tokens: Tokens, filename: String, contents: String) -> Self {
        Self {
            tokens,
            exprs: Vec::new(),
            filename,
            file: contents,
        }
    }
    pub fn parse(&mut self) -> Result<()> {
        let mut errored: (bool, String) = (false, String::new());
        loop {
            match self.statement() {
                Ok(tree) => {
                    self.exprs.push(P(tree));
                }
                Err(e) => {
                    let message = e.to_string();
                    match e.downcast::<io::Error>() {
                        Ok(e) => e,
                        Err(_) => {
                            errored.1.push_str(&message);
                            errored.0 = true;
                            continue;
                        }
                    };
                }
            };
            if self.tokens.is_at_end() {
                break;
            }
        }
        if errored.0 {
            return Err(anyhow!(errored.1));
        }
        Ok(())
    }
    pub fn statement(&mut self) -> Result<Expr> {
        match self.tokens.peek()?.token_type {
            TokenType::Ident(s) => {
                if s == "if" {
                    self.if_statement()
                } else if s == "set" {
                    self.set_statement()
                } else {
                    self.expression()
                }
            }
            _ => {
                self.expression()
            }
        }
    }
    pub fn if_statement(&mut self) -> Result<Expr> {
        let token = self.tokens.read()?;
        if let TokenType::Ident(i) = token.clone().token_type {
            if i == "if" {
                let condition = self.expression()?;
                let colon = self.tokens.read()
                    .or(Err(self.error(1001, "Expected \":\" after condition".to_string(), None, token)))?;
                if let TokenType::Colon = colon.token_type {
                    println!("colon");
                    self.tokens.read()?;
                    // placeholder if return
                    return Ok(Expr {
                        kind: ExprKind::Ident(Ident("if".to_string()))
                    });
                } else {
                    return Err(self.error(1001, "Expected \":\" after condition".to_string(), None, colon));
                }
            }
        }
        Err(anyhow!("Parser Error: if"))
    }
    pub fn set_statement(&mut self) -> Result<Expr> {
        let token = self.tokens.read()?;
        if let TokenType::Ident(i) = token.token_type {
            if i == "set" {

            }
        }
        Err(anyhow!("Parser Error: set"))
    }
    pub fn expression(&mut self) -> Result<Expr> {
        return self.equality();
    }
    pub fn equality(&mut self) -> Result<Expr> {
        let mut expr = self.comparison()?;

        while let TokenType::BangEqual | TokenType::Equals = self.tokens.peek()?.token_type {
            let operator = self.tokens.read()?;
            let right = self.comparison()?;
            if let TokenType::Equals = operator.token_type {
                expr = Expr {
                    kind: ExprKind::Binary(BinOp::Eq, P(expr), P(right))
                }
            } else {
                expr = Expr {
                    kind: ExprKind::Binary(BinOp::Ne, P(expr), P(right))
                }
            }
        }

        Ok(expr)
    }
    pub fn comparison(&mut self) -> Result<Expr> {
        let mut expr = self.term()?;

        while let TokenType::LeftAngle | TokenType::LessEqual | TokenType::RightAngle | TokenType::GreaterEqual =
            self.tokens.peek()?.token_type {
            let operator = self.tokens.read()?;
            let right = self.term()?;
            match operator.token_type {
                TokenType::LeftAngle => {
                    expr = Expr {
                        kind: ExprKind::Binary(BinOp::Ls, P(expr), P(right))
                    }
                }
                TokenType::LessEqual => {
                    expr = Expr {
                        kind: ExprKind::Binary(BinOp::Le, P(expr), P(right))
                    }
                }
                TokenType::RightAngle => {
                    expr = Expr {
                        kind: ExprKind::Binary(BinOp::Gr, P(expr), P(right))
                    }
                }
                _ => {
                    expr = Expr {
                        kind: ExprKind::Binary(BinOp::Ge, P(expr), P(right))
                    }
                }
            }
        }

        Ok(expr)
    }
    pub fn term(&mut self) -> Result<Expr> {
        let mut expr = self.factor()?;
        
        while let TokenType::Minus | TokenType::Plus = self.tokens.peek()?.token_type {
            let operator = self.tokens.read()?;
            let right = self.factor()?;
            if let TokenType::Plus = operator.token_type {
                expr = Expr {
                    kind: ExprKind::Binary(BinOp::Add, P(expr), P(right))
                }
            } else {
                expr = Expr {
                    kind: ExprKind::Binary(BinOp::Sub, P(expr), P(right))
                }
            }
        }

        Ok(expr)
    }
    pub fn factor(&mut self) -> Result<Expr> {
        let mut expr = self.unary()?;

        while let TokenType::Star | TokenType::Slash = self.tokens.peek()?.token_type {
            let operator = self.tokens.read()?;
            let right = self.factor()?;
            if let TokenType::Star = operator.token_type {
                expr = Expr {
                    kind: ExprKind::Binary(BinOp::Mul, P(expr), P(right))
                }
            } else {
                expr = Expr {
                    kind: ExprKind::Binary(BinOp::Div, P(expr), P(right))
                }
            }
        }

        Ok(expr)
    }
    pub fn unary(&mut self) -> Result<Expr> {
        if let TokenType::Bang | TokenType::Minus = self.tokens.peek()?.token_type {
            let operator = self.tokens.read()?;
            let right = self.unary()?;
            return Ok(if let TokenType::Bang = operator.token_type {
                Expr {
                    kind: ExprKind::Unary(UnOp::Not, P(right))
                }
            } else {
                Expr {
                    kind: ExprKind::Unary(UnOp::Neg, P(right))
                }
            })
        }

        Ok(self.primary()?)
    }
    pub fn primary(&mut self) -> Result<Expr> {
        if let TokenType::Boolean(b) = self.tokens.peek()?.token_type {
            self.tokens.read()?;
            return Ok(Expr {
                kind: ExprKind::Lit(Literal::Boolean(b))
            });
        }

        if let TokenType::Number(n) = self.tokens.peek()?.token_type {
            self.tokens.read()?;
            return Ok(Expr {
                kind: ExprKind::Lit(Literal::Number(n))
            });
        }
        if let TokenType::Text(s) = self.tokens.peek()?.token_type {
            self.tokens.read()?;
            return Ok(Expr {
                kind: ExprKind::Lit(Literal::Text(s))
            });
        }

        if let TokenType::Ident(name) = self.tokens.peek()?.token_type {
            self.tokens.read()?;
            return Ok(Expr {
                kind: ExprKind::Ident(Ident(name))
            });
        }

        if let TokenType::LeftParen = self.tokens.peek()?.token_type {
            let token = self.tokens.read()?;
            let expr = self.expression()?;
            match self.tokens.read() {
                Ok(t) => {
                    if let TokenType::RightParen = t.token_type {
                        return Ok(expr);
                    } else {
                        return Err(self.error(1001, "Expect ')' after expression".to_string(), None, t))
                    }
                }
                Err(_) => {
                    return Err(self.error(1001, "Expect ')' after expression".to_string(), None, token))
                }
            }
        }

        self.tokens.read()?;
        Err(anyhow!("Parser Error"))
    }
    pub fn error(&mut self, code: u32, message: String, help: Option<String>, token: Token) -> Error {
        let add = self.file[token.index as usize..].chars().position(|s| s == '\n').unwrap_or(self.file.len()-1);
        let line_start = token.index - self.file[..(token.index-1) as usize].chars().rev().position(|s| s == '\n').unwrap_or(token.index as usize) as u64;
        let line_end;
        if token.lexeme == "\n" {
            line_end = token.index;
        } else {
            line_end = token.index + add as u64;
        }
        let line_space: String = vec![' '; token.line.to_string().len() as usize].into_iter().collect();

        let column_space: String;
        column_space = vec![' '; token.column as usize].into_iter().collect();
        anyhow!(format!("\x1b[1;91merror[E{:0>4}]\x1b[0m: {} in \x1b[1;94m{}\x1b[0m at line {}:\n\
              \x1b[1;94m{3} |\x1b[0m  {}\n\
              \x1b[1;94m{} |\x1b[0m {}\x1b[1;91m^ {}\x1b[0m\n", code, message, self.filename, token.line+1,
              String::from_utf8(self.file.clone()[line_start as usize..line_end as usize].into()).unwrap().trim(),
              line_space, column_space, help.unwrap_or("".to_string()),
              ))
    }
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Set(Ident, P<Expr>),
    Binary(BinOp, P<Expr>, P<Expr>),
    Unary(UnOp, P<Expr>),
    If(P<Expr>, P<Block>),
    Switch(P<Expr>, Vec<Arm>),
    Block(P<Block>),
    Lit(Literal),
    Ident(Ident),
}

#[derive(Clone, Debug)]
pub struct Arm {
    pub pat: P<Expr>,
    pub body: P<Expr>,
}

#[derive(Clone, Debug)]
pub struct Block {
    pub exprs: P<Expr>,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum BinOp {
    Add, // +
    Sub, // -
    Mul, // *
    Div, // /
    Exp, // **
    And, // &&
    Or, // ||
    Eq, // ==
    Ne, // !=
    Ls, // <
    Le, // <=
    Gr, // >
    Ge, // >=
    BitAnd, // &
    BitOr, // |
    Shl, // <<
    Shr, // >>
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum UnOp {
    Not,
    Neg,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct Ident(String);

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum Literal {
    Text(String),
    Number(f64),
    Boolean(bool),
}

mod ptr;