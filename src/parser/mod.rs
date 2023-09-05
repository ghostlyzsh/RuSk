use anyhow::{Result, anyhow, Error};

use crate::lexer::tokens::{Tokens, TokenType, Token, TokenError};

use self::{ptr::P, error::ParserError};

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
                    match e.downcast::<TokenError>() {
                        Ok(_) => {}
                        Err(e) => {
                            match e.downcast::<ParserError>() {
                                Ok(e) => {
                                    errored.1.push_str(&e.into_string());
                                    errored.0 = true;
                                    break;
                                }
                                Err(e) => {
                                    panic!("Parser broke: {}", e);
                                }
                            };
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
                } else if s == "native" {
                    self.native_function()
                } else {
                    self.call_expression()
                }
            }
            TokenType::Indent => {
                let token = self.tokens.read()?;
                Err(self.error(1003, "Unexpected indent".to_string(), None, -1, token))
            }
            TokenType::Dedent => {
                let token = self.tokens.read()?;
                Err(self.error(1003, "Unexpected dedent".to_string(), None, 0, token))
            }
            TokenType::Newline => {
                self.tokens.read()?;
                self.statement()
            }
            _ => {
                self.expression_statement()
            }
        }
    }
    pub fn if_statement(&mut self) -> Result<Expr> {
        let token = self.tokens.read()?;
        if let TokenType::Ident(i) = token.clone().token_type {
            if i == "if" {
                let condition = self.call_expression()?;
                let colon = self.tokens.read()
                    .or(Err(self.error(1001, "Expected \":\" after condition".to_string(), None, 0, token.clone())))?;
                if let TokenType::Colon = colon.token_type {
                    self.consume(TokenType::Newline, "Expected new line after statement".to_string())?;
                    self.consume(TokenType::Indent, "Expected indent after \"if\"".to_string())?;
                    let expr = self.block()?;
                    return Ok(Expr {
                        kind: ExprKind::If(P(condition), P(expr)),
                        line: token.line,
                    });
                } else {
                    return Err(self.error(1001, "Expected \":\" after condition".to_string(), None, 0, colon));
                }
            }
        }
        Err(anyhow!("Parser Error: if"))
    }
    pub fn set_statement(&mut self) -> Result<Expr> {
        let token = self.tokens.read()?;
        if let TokenType::Ident(i) = token.clone().token_type {
            if i == "set" {
                if let TokenType::LeftBrace = self.tokens.peek()?.token_type {
                    self.tokens.read()?;
                    let variable = self.handle_variable()?;
                    self.consume(TokenType::Ident("to".to_string()), "Expected \"to\" after variable".to_string())?;
                    let expression = self.statement()?;
                    return Ok(Expr { kind: ExprKind::Set(variable, P(expression)), line: token.line })
                } else {
                    return Err(self.error(1002, "Expected variable".to_string(), None, 1, token))
                }
            }
        }
        Err(anyhow!("Parser Error: set"))
    }
    pub fn native_function(&mut self) -> Result<Expr> {
        let token = self.tokens.read()?;
        if let TokenType::Ident(i) = token.clone().token_type {
            if i == "native" {
                self.consume(TokenType::Ident("function".to_string()), "Expected \"function\"".to_string())?;
                if let TokenType::Ident(name) = self.tokens.peek()?.token_type {
                    self.tokens.read()?;
                    self.consume(TokenType::LeftParen, "Expected \"(\"".to_string())?;
                    let args = self.types()?;
                    let mut var = false;
                    if let TokenType::Ellipsis = self.tokens.peek()?.token_type {
                        self.tokens.read()?;
                        var = true;
                    }
                    self.consume(TokenType::RightParen, "Expected \")\" after arguments".to_string())?;
                    if let TokenType::ColonColon = self.tokens.peek()?.token_type {
                        self.tokens.read()?;
                        let ret_type = Expr {
                            kind: ExprKind::Type(self.match_type()?),
                            line: token.line,
                        };
                        return Ok(Expr { kind: ExprKind::Native(Ident(name), args, var, Some(P(ret_type))), line: token.line })
                    }
                    return Ok(Expr { kind: ExprKind::Native(Ident(i), args, var, None), line: token.line })
                } else {
                    return Err(self.error(1005, "Expected function name".to_string(), None, 0, token));
                }
            }
        }
        Err(anyhow!("Parser Error: native function"))
    }
    pub fn types(&mut self) -> Result<Vec<P<Expr>>> {
        let mut exprs = Vec::new();
        while let TokenType::Ident(ident) = self.tokens.peek()?.token_type {
            let token = self.tokens.read()?;
            self.consume(TokenType::Colon, "Expected \":\" in type declaration".to_string())?;
            if let TokenType::Ident(t) = self.tokens.peek()?.token_type {
                self.tokens.read()?;
                let tt;
                match t.as_str() {
                    "text" => {
                        tt = Type::Text;
                    }
                    "number" => {
                        tt = Type::Number;
                    }
                    "int" => {
                        tt = Type::Integer;
                    }
                    "integer" => {
                        tt = Type::Integer;
                    }
                    "bool" => {
                        tt = Type::Boolean;
                    }
                    "boolean" => {
                        tt = Type::Boolean;
                    }
                    t => {
                        return Err(self.error(1002, format!("Invalid type \"{}\"", t), None, 0, token));
                    }
                }
                exprs.push(P(Expr {
                    kind: ExprKind::Arg(Ident(ident), tt),
                    line: token.line,
                }));
                if let TokenType::Comma = self.tokens.peek()?.token_type {
                    self.tokens.read()?;
                } else {
                    return Ok(exprs);
                }
            } else {
                return Err(self.error(1002, "Expected identifier".to_string(), None, 0, token));
            }
        }
        return Ok(exprs);
    }
    pub fn match_type(&mut self) -> Result<Type> {
        if let TokenType::Ident(ident) = self.tokens.peek()?.token_type {
            let token = self.tokens.read()?;
            let tt;
            match ident.as_str() {
                "text" => {
                    tt = Type::Text;
                }
                "number" => {
                    tt = Type::Number;
                }
                "int" => {
                    tt = Type::Integer;
                }
                "integer" => {
                    tt = Type::Integer;
                }
                "bool" => {
                    tt = Type::Boolean;
                }
                "boolean" => {
                    tt = Type::Boolean;
                }
                t => {
                    return Err(self.error(1002, format!("Invalid type \"{}\"", t), None, 0, token));
                }
            }
            Ok(tt)
        } else {
            Err(self.error(1002, "Expected type".to_string(), None, 0, self.tokens.peek()?))
        }
    }

    pub fn block(&mut self) -> Result<Block> {
        let mut block = Block {
            exprs: Vec::new()
        };
        let mut errored = (false, "".to_string());
        while self.tokens.peek()?.token_type != TokenType::Dedent {
            match self.statement() {
                Ok(tree) => {
                    block.exprs.push(P(tree));
                }
                Err(e) => {
                    let message = e.to_string();
                    match e.downcast::<TokenError>() {
                        Ok(e) => {
                            errored.1.push_str(&self.error_tok(1004, "Unexpected end of file".to_string(), None, e).to_string());
                            errored.0 = true;
                            break;
                        }
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
        match self.consume(TokenType::Dedent, "Expected dedent at end of code block".to_string()) {
            _ => {}
        };
        if errored.0 {
            return Err(anyhow!(errored.1));
        }

        Ok(block)
    }
    pub fn call_expression(&mut self) -> Result<Expr> {
        if let TokenType::Ident(string) = self.tokens.peek()?.token_type {
            let token = self.tokens.read()?;
            if let TokenType::LeftParen = self.tokens.peek()?.token_type {
                self.tokens.read()?;
                let expr = self.arguments()?;
                self.consume(TokenType::RightParen, "Expected closing parenthesis in function call".to_string())?;
                Ok(Expr { kind: ExprKind::Call(Ident(string), expr), line: token.line })
            } else {
                Ok(Expr {
                    kind: ExprKind::Ident(Ident(string)),
                    line: token.line,
                })
            }
        } else {
            self.expression()
        }
    }
    pub fn arguments(&mut self) -> Result<Vec<P<Expr>>> {
        let mut exprs = Vec::new();
        while TokenType::RightParen != self.tokens.peek()?.token_type {
            exprs.push(P(self.expression()?));
            if let TokenType::Comma = self.tokens.peek()?.token_type {
                self.tokens.read()?;
            } else {
                return Ok(exprs);
            }
        }
        return Ok(exprs);
    }
    pub fn expression_statement(&mut self) -> Result<Expr> {
        let expr = self.expression()?;
        let token = self.tokens.read()?;
        if let TokenType::Newline = token.token_type {} else {
            return Err(self.error(1002, "Expected new line after statement".to_string(), None, 0, token));
        }
        Ok(expr)
    }
    pub fn expression(&mut self) -> Result<Expr> {
        let expr = self.variable()?;
        Ok(expr)
    }
    pub fn variable(&mut self) -> Result<Expr> {
        if let TokenType::LeftBrace = self.tokens.peek()?.token_type {
            let token = self.tokens.read()?;
            Ok(Expr {
                kind: ExprKind::Var(self.handle_variable()?),
                line: token.line
            })
        } else {
            self.logical_or()
        }
    }
    pub fn handle_variable(&mut self) -> Result<Variable> {
        let visibility = match self.tokens.peek()?.token_type {
            TokenType::Star => {
                self.tokens.read()?;
                VisibilityMode::Global
            }
            TokenType::Tilde => {
                self.tokens.read()?;
                VisibilityMode::Module
            }
            _ => {
                VisibilityMode::Local
            }
        };
        let ident = self.tokens.read()?;
        if let TokenType::Ident(name) = ident.token_type {
            if let TokenType::ColonColon = self.tokens.peek()?.token_type {
                self.tokens.read()?;
                if let TokenType::Star = self.tokens.peek()?.token_type {
                    self.tokens.read()?;
                    self.consume(TokenType::RightBrace, "Expected right brace after name".to_string())?;
                    Ok(Variable {
                        name: Ident(name),
                        list_mode: Some(VariableListMode::All),
                        visibility,
                        mutable: false,
                    })
                } else {
                    let expr = self.expression()?;
                    self.consume(TokenType::RightBrace, "Expected right brace after name".to_string())?;
                    Ok(Variable {
                        name: Ident(name),
                        list_mode: Some(VariableListMode::Index(P(expr))),
                        visibility,
                        mutable: false,
                    })
                }
            } else {
                self.consume(TokenType::RightBrace, "Expected right brace after name".to_string())?;
                Ok(Variable {
                    name: Ident(name),
                    list_mode: None,
                    visibility,
                    mutable: false,
                })
            }
        } else {
            Err(self.error(1005, "Expected a variable name".to_string(), None, 0, ident))
        }
    }
    pub fn logical_or(&mut self) -> Result<Expr> {
        let mut expr = self.logical_and()?;

        while let TokenType::Or = self.tokens.peek()?.token_type {
            let operator = self.tokens.read()?;
            let right = self.logical_and()?;
            if let TokenType::Or = operator.token_type {
                expr = Expr {
                    kind: ExprKind::Binary(BinOp::Or, P(expr.clone()), P(right)),
                    line: expr.line,
                }
            }
        }
        
        Ok(expr)
    }
    pub fn logical_and(&mut self) -> Result<Expr> {
        let mut expr = self.bit_or()?;

        while let TokenType::And = self.tokens.peek()?.token_type {
            let operator = self.tokens.read()?;
            let right = self.bit_or()?;
            if let TokenType::And = operator.token_type {
                expr = Expr {
                    kind: ExprKind::Binary(BinOp::And, P(expr.clone()), P(right)),
                    line: expr.line,
                }
            }
        }
        
        Ok(expr)
    }
    pub fn bit_or(&mut self) -> Result<Expr> {
        let mut expr = self.bit_xor()?;

        while let TokenType::BitOr = self.tokens.peek()?.token_type {
            let operator = self.tokens.read()?;
            let right = self.bit_xor()?;
            if let TokenType::BitOr = operator.token_type {
                expr = Expr {
                    kind: ExprKind::Binary(BinOp::BitOr, P(expr.clone()), P(right)),
                    line: expr.line,
                }
            }
        }
        
        Ok(expr)
    }
    pub fn bit_xor(&mut self) -> Result<Expr> {
        let mut expr = self.bit_and()?;

        while let TokenType::BitXor = self.tokens.peek()?.token_type {
            let operator = self.tokens.read()?;
            let right = self.bit_and()?;
            if let TokenType::BitXor = operator.token_type {
                expr = Expr {
                    kind: ExprKind::Binary(BinOp::BitXor, P(expr.clone()), P(right)),
                    line: expr.line,
                }
            }
        }
        
        Ok(expr)
    }
    pub fn bit_and(&mut self) -> Result<Expr> {
        let mut expr = self.equality()?;

        while let TokenType::BitAnd = self.tokens.peek()?.token_type {
            let operator = self.tokens.read()?;
            let right = self.equality()?;
            if let TokenType::BitAnd = operator.token_type {
                expr = Expr {
                    kind: ExprKind::Binary(BinOp::BitAnd, P(expr.clone()), P(right)),
                    line: expr.line,
                }
            }
        }
        
        Ok(expr)
    }
    pub fn equality(&mut self) -> Result<Expr> {
        let mut expr = self.comparison()?;

        while let TokenType::BangEqual | TokenType::Equals = self.tokens.peek()?.token_type {
            let operator = self.tokens.read()?;
            let right = self.comparison()?;
            if let TokenType::Equals = operator.token_type {
                expr = Expr {
                    kind: ExprKind::Binary(BinOp::Eq, P(expr.clone()), P(right)),
                    line: expr.line,
                }
            } else {
                expr = Expr {
                    kind: ExprKind::Binary(BinOp::Ne, P(expr.clone()), P(right)),
                    line: expr.line,
                }
            }
        }

        Ok(expr)
    }
    pub fn comparison(&mut self) -> Result<Expr> {
        let mut expr = self.shift()?;

        while let TokenType::LeftAngle | TokenType::LessEqual | TokenType::RightAngle | TokenType::GreaterEqual =
            self.tokens.peek()?.token_type {
            let operator = self.tokens.read()?;
            let right = self.shift()?;
            match operator.token_type {
                TokenType::LeftAngle => {
                    expr = Expr {
                        kind: ExprKind::Binary(BinOp::Ls, P(expr.clone()), P(right)),
                        line: expr.line,
                    }
                }
                TokenType::LessEqual => {
                    expr = Expr {
                        kind: ExprKind::Binary(BinOp::Le, P(expr.clone()), P(right)),
                        line: expr.line,
                    }
                }
                TokenType::RightAngle => {
                    expr = Expr {
                        kind: ExprKind::Binary(BinOp::Gr, P(expr.clone()), P(right)),
                        line: expr.line,
                    }
                }
                _ => {
                    expr = Expr {
                        kind: ExprKind::Binary(BinOp::Ge, P(expr.clone()), P(right)),
                        line: expr.line,
                    }
                }
            }
        }

        Ok(expr)
    }
    pub fn shift(&mut self) -> Result<Expr> {
        let mut expr = self.term()?;

        while let TokenType::Shl | TokenType::Shr = self.tokens.peek()?.token_type {
            let operator = self.tokens.read()?;
            let right = self.term()?;
            if let TokenType::Shl = operator.token_type {
                expr = Expr {
                    kind: ExprKind::Binary(BinOp::Shl, P(expr.clone()), P(right)),
                    line: expr.line,
                }
            } else {
                expr = Expr {
                    kind: ExprKind::Binary(BinOp::Shr, P(expr.clone()), P(right)),
                    line: expr.line,
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
                    kind: ExprKind::Binary(BinOp::Add, P(expr.clone()), P(right)),
                    line: expr.line,
                }
            } else {
                expr = Expr {
                    kind: ExprKind::Binary(BinOp::Sub, P(expr.clone()), P(right)),
                    line: expr.line,
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
                    kind: ExprKind::Binary(BinOp::Mul, P(expr.clone()), P(right)),
                    line: expr.line,
                }
            } else {
                expr = Expr {
                    kind: ExprKind::Binary(BinOp::Div, P(expr.clone()), P(right)),
                    line: expr.line,
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
                    kind: ExprKind::Unary(UnOp::Not, P(right)),
                    line: operator.line,
                }
            } else {
                Expr {
                    kind: ExprKind::Unary(UnOp::Neg, P(right)),
                    line: operator.line,
                }
            })
        }

        Ok(self.primary()?)
    }
    pub fn primary(&mut self) -> Result<Expr> {
        if let TokenType::Boolean(b) = self.tokens.peek()?.token_type {
            let token = self.tokens.read()?;
            return Ok(Expr {
                kind: ExprKind::Lit(Literal::Boolean(b)),
                line: token.line,
            });
        }

        if let TokenType::Number(n) = self.tokens.peek()?.token_type {
            let token = self.tokens.read()?;
            return Ok(Expr {
                kind: ExprKind::Lit(Literal::Number(n)),
                line: token.line,
            });
        }
        if let TokenType::Integer(n) = self.tokens.peek()?.token_type {
            let token = self.tokens.read()?;
            return Ok(Expr {
                kind: ExprKind::Lit(Literal::Integer(n)),
                line: token.line,
            });
        }
        if let TokenType::Text(s) = self.tokens.peek()?.token_type {
            let token = self.tokens.read()?;
            return Ok(Expr {
                kind: ExprKind::Lit(Literal::Text(s)),
                line: token.line,
            });
        }

        if let TokenType::Ident(name) = self.tokens.peek()?.token_type {
            let token = self.tokens.read()?;
            return Ok(Expr {
                kind: ExprKind::Ident(Ident(name)),
                line: token.line,
            });
        }

        if let TokenType::LeftBrace = self.tokens.peek()?.token_type {
            let token = self.tokens.read()?;
            let variable = self.handle_variable()?;
            return Ok(Expr {
                kind: ExprKind::Var(variable),
                line: token.line,
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
                        return Err(self.error(1001, "Expect ')' after expression".to_string(), None, 0, t))
                    }
                }
                Err(_) => {
                    return Err(self.error(1001, "Expect ')' after expression".to_string(), None, 0, token))
                }
            }
        }

        let token = self.tokens.read()?;
        Err(self.error(1002, "Unrecognized syntax".to_string(), None, 0, token))
    }

    pub fn consume(&mut self, token_type: TokenType, message: String) -> Result<Token> {
        let token = match self.tokens.read() {
            Ok(t) => t,
            Err(e) => {
                match e.downcast::<TokenError>() {
                    Ok(e) => {
                        return Err(self.error_tok(1002, message, None, e))
                    }
                    Err(_) => {
                        panic!("Parser broke");
                    }
                };
            }
        };
        if token_type == token.clone().token_type { Ok(token) } else {
            Err(self.error(1002, message, None, 0, token))
        }
    }
    
    pub fn error(&mut self, code: u32, message: String, help: Option<String>, offset: i32, token: Token) -> Error {
        let add = self.file[token.index as usize..].chars().position(|s| s == '\n').unwrap_or(self.file.len());
        let line_start = token.index - self.file[..(token.index-1) as usize].chars().rev().position(|s| s == '\n').unwrap_or(token.index as usize) as u64;
        let line_end;
        if token.token_type == TokenType::Newline {
            if token.index-1 < line_start {
                line_end = token.index;
            } else {
                line_end = token.index-1;
            }
        } else {
            line_end = token.index + add as u64;
        }
        ParserError {
            kind: code.into(),
            message,
            line: token.line,
            column: token.column,
            line_str: String::from_utf8(self.file.clone()[line_start as usize..line_end as usize].into()).unwrap(),
            offset,
            filename: self.filename.clone(),
            help,
        }.into()
        /*anyhow!(format!("\x1b[1;91merror[E{:0>4}]\x1b[0m: {} in \x1b[1;94m{}\x1b[0m at line {}:\n\
              \x1b[1;94m{3} |\x1b[0m  {}\n\
              \x1b[1;94m{} |\x1b[0m {}\x1b[1;91m^ {}\x1b[0m\n", code, message, self.filename, token.line+1,
              String::from_utf8(self.file.clone()[line_start as usize..line_end as usize].into()).unwrap(),
              line_space, column_space, help.unwrap_or("".to_string()),
              ))*/
    }
    pub fn error_tok(&mut self, code: u32, message: String, help: Option<String>, token: TokenError) -> Error {
        //let add = self.file[token.index as usize..].chars().position(|s| s == '\n').unwrap_or(self.file.len()-1);
        let line_start = token.index - self.file[..(token.index-1) as usize].chars().rev().position(|s| s == '\n').unwrap_or(token.index as usize) as u64;
        let line_end = token.index-1;
        ParserError {
            kind: code.into(),
            message,
            line: token.line,
            column: token.column,
            line_str: String::from_utf8(self.file.clone()[line_start as usize..line_end as usize].into()).unwrap(),
            offset: 0,
            filename: self.filename.clone(),
            help,
        }.into()
        /*anyhow!(format!("\x1b[1;91merror[E{:0>4}]\x1b[0m: {} in \x1b[1;94m{}\x1b[0m at line {}:\n\
              \x1b[1;94m{3} |\x1b[0m  {}\n\
              \x1b[1;94m{} |\x1b[0m {}\x1b[1;91m^ {}\x1b[0m\n", code, message, self.filename, token.line+1,
              String::from_utf8(self.file.clone()[line_start as usize..line_end as usize].into()).unwrap(),
              line_space, column_space, help.unwrap_or("".to_string()),
              ))*/
    }
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub line: u32,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum ExprKind {
    Call(Ident, Vec<P<Expr>>),
    Set(Variable, P<Expr>),
    Binary(BinOp, P<Expr>, P<Expr>),
    Unary(UnOp, P<Expr>),
    If(P<Expr>, P<Block>),
    Switch(P<Expr>, Vec<Arm>),
    Native(Ident, Vec<P<Expr>>, bool, Option<P<Expr>>),
    Block(P<Block>),
    Var(Variable),
    Lit(Literal),
    Ident(Ident),
    Arg(Ident, Type),
    Type(Type),
}

#[derive(Clone, Debug)]
pub struct Arm {
    pub pat: P<Expr>,
    pub body: P<Expr>,
}

#[derive(Clone, Debug)]
pub struct Block {
    pub exprs: Vec<P<Expr>>,
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
    BitXor, // ^
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
pub struct Ident(pub String);

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum Literal {
    Text(String),
    Number(f64),
    Integer(i64),
    Boolean(bool),
}

#[derive(Clone, Debug)]
pub enum Type {
    Text,
    Number,
    Integer,
    Boolean,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct Variable {
    pub name: Ident,
    pub list_mode: Option<VariableListMode>,
    pub visibility: VisibilityMode,
    pub mutable: bool,
}

#[derive(Clone, Debug)]
pub enum VariableListMode {
    All,
    Index(P<Expr>)
}

#[derive(Clone, Debug)]
pub enum VisibilityMode {
    Local,
    Module,
    Global,
}

pub mod ptr;
mod error;
