use std::io::{Cursor, Read, BufRead, ErrorKind};

use self::tokens::{Tokens, Token, TokenType};

pub struct Lexer {
    pub tokens: Tokens,
    pub file: Cursor<Vec<u8>>,
    pub line: u32,
    pub line_start: u64,
    pub start: u64,
    pub filename: String,
    pub prev_indent: u32,
    pub indent_stack: Vec<u32>,
}

impl Lexer {
    pub fn new(filename: String, contents: String) -> Self {
        Self {
            tokens: Tokens::new(),
            file: Cursor::new(contents.as_bytes().to_vec()),
            line: 0,
            line_start: 0,
            start: 0,
            filename,
            indent_stack: vec![0],
            prev_indent: 0,
        }
    }
    
    pub fn process(&mut self) -> Result<(), String> {
        let mut errored: (bool, String) = (false, String::new());
        loop {
            self.start = self.file.position();
            let mut c = [0u8; 1];
            if self.file.read(&mut c).unwrap() == 0 {
                break;
            }
            if self.tokens.len() != 0 {
                if let TokenType::Newline = self.tokens.last().token_type {
                    let mut indent: u32 = 0;
                    while c[0] == b'\t' || c[0] == b' ' {
                        if c[0] == b'\t' {
                            indent += 4;
                        } else {
                            indent += 1;
                        }
                        self.file.read(&mut c).unwrap();
                    }
                    let sign = (indent as i64 - self.prev_indent as i64).signum();
                    if sign == -1 {
                        loop {
                            match self.indent_stack.pop() {
                                Some(i) => i,
                                None => break 
                            };
                            if *self.indent_stack.last().unwrap() <= indent { break }
                        }
                        if *self.indent_stack.last().unwrap() != indent {
                            return Err(self.error(3, "Mismatching indent".to_string(), 0, None));
                        }
                        self.tokens.write_one(self.token_from_type(TokenType::Dedent));
                    } else if sign == 1 {
                        self.indent_stack.push(indent);
                        self.tokens.write_one(self.token_from_type(TokenType::Indent));
                    }
                    self.prev_indent = indent;
                }
            }

            match c[0] as char {
                '%' => self.tokens.write_one(self.token_from_type(TokenType::Percent)),
                '+' => self.tokens.write_one(self.token_from_type(TokenType::Plus)),
                '-' => self.tokens.write_one(self.token_from_type(TokenType::Minus)),
                '^' => self.tokens.write_one(self.token_from_type(TokenType::BitXor)),
                '~' => self.tokens.write_one(self.token_from_type(TokenType::Tilde)),
                ':' => {
                    self.file.read(&mut c).unwrap();
                    if c[0] == b':' {
                        self.tokens.write_one(self.token_from_type(TokenType::ColonColon));
                    } else {
                        self.file.set_position(self.file.position()-1);
                        self.tokens.write_one(self.token_from_type(TokenType::Colon));
                    }
                },
                '*' => {
                    self.file.read(&mut c).unwrap();
                    if c[0] == b'*' {
                        self.tokens.write_one(self.token_from_type(TokenType::Exp));
                    } else {
                        self.file.set_position(self.file.position()-1);
                        self.tokens.write_one(self.token_from_type(TokenType::Star));
                    }
                },
                '(' => self.tokens.write_one(self.token_from_type(TokenType::LeftParen)),
                ')' => self.tokens.write_one(self.token_from_type(TokenType::RightParen)),
                '=' => self.tokens.write_one(self.token_from_type(TokenType::Equals)),
                ',' => self.tokens.write_one(self.token_from_type(TokenType::Comma)),
                '.' => {
                    self.file.read(&mut c).unwrap();
                    if c[0] == b'.' {
                        self.file.read(&mut c).unwrap();
                        if c[0] == b'.' {
                            self.tokens.write_one(self.token_from_type(TokenType::Ellipsis))
                        }
                    }
                }
                '<' => {
                    self.file.read(&mut c).unwrap();
                    if c[0] == b'=' {
                        self.tokens.write_one(self.token_from_type(TokenType::LessEqual));
                    } else if c[0] == b'<' {
                        self.tokens.write_one(self.token_from_type(TokenType::Shl));
                    } else {
                        self.file.set_position(self.file.position()-1);
                        self.tokens.write_one(self.token_from_type(TokenType::LeftAngle));
                    }
                }
                '>' => {
                    self.file.read(&mut c).unwrap();
                    if c[0] == b'=' {
                        self.tokens.write_one(self.token_from_type(TokenType::GreaterEqual));
                    } else if c[0] == b'>' {
                        self.tokens.write_one(self.token_from_type(TokenType::Shr));
                    } else {
                        self.file.set_position(self.file.position()-1);
                        self.tokens.write_one(self.token_from_type(TokenType::RightAngle));
                    }
                }
                '|' => {
                    self.file.read(&mut c).unwrap();
                    if c[0] == b'|' {
                        self.tokens.write_one(self.token_from_type(TokenType::Or));
                    } else {
                        self.file.set_position(self.file.position()-1);
                        self.tokens.write_one(self.token_from_type(TokenType::BitOr));
                    }
                }
                '&' => {
                    self.file.read(&mut c).unwrap();
                    if c[0] == b'&' {
                        self.tokens.write_one(self.token_from_type(TokenType::And));
                    } else {
                        self.file.set_position(self.file.position()-1);
                        self.tokens.write_one(self.token_from_type(TokenType::BitAnd));
                    }
                }
                '/' => self.tokens.write_one(self.token_from_type(TokenType::Slash)),
                '!' => {
                    self.file.read(&mut c).unwrap();
                    if c[0] == b'=' {
                        self.tokens.write_one(self.token_from_type(TokenType::BangEqual));
                    } else {
                        self.file.set_position(self.file.position()-1);
                        self.tokens.write_one(self.token_from_type(TokenType::Bang));
                    }
                }
                '#' => {
                    self.file.read_until(b'\n', &mut Vec::new()).unwrap();
                    self.line_start = self.file.position();
                    self.line += 1;
                }
                '{' => self.tokens.write_one(self.token_from_type(TokenType::LeftBrace)),
                '}' => self.tokens.write_one(self.token_from_type(TokenType::RightBrace)),
                '"' => {
                    self.process_string().unwrap_or_else(|e| { errored.1.push_str(&e); errored.0 = true; });
                }
                '\n' => {
                    self.tokens.write_one(self.token_from_type(TokenType::Newline));
                    self.line_start = self.file.position();
                    self.line += 1;
                }
                ' ' | '\r' => {}
                c => {
                    if c.is_ascii_digit() {
                        self.file.set_position(self.file.position()-1);
                        self.process_number().unwrap_or_else(|e| { errored.1.push_str(&e); errored.0 = true; });
                        
                    } else if c.is_ascii_alphabetic() {
                        self.file.set_position(self.file.position()-1);
                        self.identifier();
                    } else {
                        errored.1.push_str(self.error(1,
                            format!("Unrecognized character \"{}\"", c), 0, None
                        ).as_str());
                        errored.0 = true;
                    }
                }
            }
        }
        if errored.0 {
            return Err(errored.1);
        }
        Ok(())
    }

    fn token_from_type(&self, token_type: TokenType) -> Token {
        let mut start = self.line_start;
        loop {
            match match self.file.clone().into_inner().get(start as usize) {
                Some(s) => s,
                None => {break}
            } {
                b'\r' | b'\t' | b'\n' => {start += 1}
                _ => break
            }
        }
        let column = (self.file.position()).abs_diff(start);
        Token {
            token_type,
            line: self.line,
            column,
            lexeme: String::from_utf8(self.file.clone().into_inner()[self.start as usize..self.file.position() as usize].to_vec()).unwrap(),
            index: self.file.position(),
        }
    }

    /*fn process_variable(&mut self) -> Result<(), String> {
        let mut name = String::new();
        let mut c = [0u8; 1];
        self.file.read(&mut c).unwrap();
        while c[0] != b'}' {
            name.push(c[0] as char);
            if c[0] == b'\n' {
                self.tokens.write_one(self.token_from_type(TokenType::Newline));
                self.line_start = self.file.position();
                self.line += 1;
                return Err(self.error(2, "Unexpected end of line".to_string(), 1, Some("help: remove new line".to_string())));
            };
            self.file.read(&mut c).unwrap();
        }

        self.tokens.write_one(self.token_from_type(TokenType::Variable(name)));
        Ok(())
    }*/
    fn process_string(&mut self) -> Result<(), String> {
        let mut name = String::new();
        let mut c = [0u8; 1];
        self.file.read(&mut c).unwrap();
        while c[0] != b'"' {
            name.push(c[0] as char);
            if c[0] == b'\n' {
                self.tokens.write_one(self.token_from_type(TokenType::Newline));
                self.file.set_position(self.file.position()-1);
                return Err(self.error(2, "Unexpected end of line".to_string(), 1, Some("help: remove new line".to_string())));
            };
            self.file.read(&mut c).unwrap();
        }

        self.tokens.write_one(self.token_from_type(TokenType::Text(name)));
        Ok(())
    }
    fn process_number(&mut self) -> Result<(), String> {
        let mut number = String::new();
        let mut c = [0u8; 1];
        self.file.read(&mut c).unwrap();
        while c[0].is_ascii_digit() {
            number.push(c[0] as char);
            self.file.read(&mut c).unwrap();
        }

        if c[0] == b'.' {
            number.push('.');
            if self.file.clone().into_inner().get(self.file.position() as usize).unwrap_or(&0).is_ascii_digit() {
                self.file.read(&mut c).unwrap();
                while c[0].is_ascii_digit() {
                    number.push(c[0] as char);
                    self.file.read(&mut c).unwrap();
                }
                self.file.set_position(self.file.position()-1);

                self.tokens.write_one(self.token_from_type(TokenType::Number(number.parse().unwrap())));
            }
            else {
                self.file.set_position(self.file.position()+1);
                return Err(self.error(3, "Unexpected end of number".to_string(), 0, Some("help: remove trailing decimal point".to_string())));
            }
        } else {
            self.file.set_position(self.file.position()-1);

            self.tokens.write_one(self.token_from_type(TokenType::Integer(number.parse().unwrap())));
        }
        Ok(())
    }
    fn identifier(&mut self) {
        let mut ident = String::new();
        let mut c = [0u8; 1];
        self.file.read(&mut c).unwrap();
        while c[0].is_ascii_alphabetic() || c[0] == b'\'' || c[0] == b'_' {
            ident.push(c[0] as char);
            self.file.read(&mut c).unwrap();
        }
        self.file.set_position(self.file.position()-1);

        if ident == "false" {
            self.tokens.write_one(self.token_from_type(TokenType::Boolean(false)));
        } else if ident == "true" {
            self.tokens.write_one(self.token_from_type(TokenType::Boolean(true)));
        } else {
            self.tokens.write_one(self.token_from_type(TokenType::Ident(ident)));
        }
    }
    fn error(&self, code: u32, message: String, offset: i32, help: Option<String>) -> String {
        let add = match self.file.clone().read_until(b'\n', &mut Vec::new()) {
            Ok(a) => a,
            Err(e) => {
                if let ErrorKind::UnexpectedEof = e.kind() {
                    self.file.clone().into_inner().len()
                } else {
                    panic!("{}", e);
                }
            }
        }; 
        let line_end = self.file.position() + add as u64;
        let line_space: String = vec![' '; self.line.to_string().len() as usize].into_iter().collect();
        let mut start = self.line_start;
        loop {
            match self.file.clone().into_inner()[start as usize] {
                b'\r' | b'\t' | b'\n' => {start += 1}
                _ => break
            }
        }
        let column = self.file.position()-start;
        let column_space: String;
        if offset.signum() == -1 {
            column_space = vec![' '; column as usize - (-offset) as usize].into_iter().collect();
        } else {
            column_space = vec![' '; column as usize + offset as usize].into_iter().collect();
        }
        format!("\x1b[1;91merror[E{:0>4}]\x1b[0m: {} in \x1b[1;94m{}\x1b[0m at line {}:\n\
              \x1b[1;94m{3} |\x1b[0m  {}\n\
              \x1b[1;94m{} |\x1b[0m {}\x1b[1;91m^ {}\x1b[0m\n", code, message, self.filename, self.line+1, 
              String::from_utf8(self.file.clone().into_inner()[self.line_start as usize..line_end as usize].into()).unwrap().trim(),
              line_space, column_space, help.unwrap_or("".to_string()),
        )
        /*format!("\x1b[1;91mLine {}: \x1b[0m({})\n\
            \t\x1b[0;91m{}\n\
            \t\x1b[0;33mLine: \x1b[0m{}\n\n",
              self.line, self.filename, message, 
              String::from_utf8(self.file.clone().into_inner()[self.line_start as usize..line_end as usize].into()).unwrap().trim(),
        )*/
    }
}


pub(crate) mod tokens;
