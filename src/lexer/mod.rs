use std::io::{Cursor, Read, BufRead, ErrorKind};

use self::tokens::{Tokens, Token, TokenType};


pub struct Lexer {
    pub tokens: Tokens,
    pub file: Cursor<Vec<u8>>,
    pub line: u32,
    pub line_start: u64,
    pub start: u64,
    pub filename: String,
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
            match c[0] as char {
                ':' => self.tokens.write_one(self.token_from_type(TokenType::Colon)),
                '%' => self.tokens.write_one(self.token_from_type(TokenType::Percent)),
                '+' => self.tokens.write_one(self.token_from_type(TokenType::Plus)),
                '-' => self.tokens.write_one(self.token_from_type(TokenType::Minus)),
                '*' => self.tokens.write_one(self.token_from_type(TokenType::Star)),
                '#' => {
                    self.file.read_until(b'\n', &mut Vec::new()).unwrap();
                    self.line += 1;
                    self.tokens.write_one(self.token_from_type(TokenType::Star))
                }
                '{' => {
                    self.file.set_position(self.file.position()-1);
                    self.process_variable()
                }
                '"' => {
                    self.file.set_position(self.file.position()-1);
                    self.process_string()
                }
                '\n' => {
                    self.line_start = self.file.position();
                    self.line += 1;
                }
                c => {
                    if c.is_ascii_digit() {
                        self.file.set_position(self.file.position()-1);
                        self.process_number().unwrap_or_else(|e| errored.1.push_str(&e));
                        errored.0 = true;
                    } else if c.is_ascii_alphabetic() {
                        self.file.set_position(self.file.position()-1);
                        self.identifier();
                    } else {
                        errored.1.push_str(self.error(
                            format!("Unrecognized character \"{}\"", c), None
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
        Token {
            token_type,
            line: self.line,
            lexeme: String::from_utf8(self.file.clone().into_inner()[self.start as usize..self.file.position() as usize].to_vec()).unwrap(),
        }
    }

    fn process_variable(&mut self) {
        let mut name = String::new();
        let mut c = [0u8; 1];
        self.file.read(&mut c).unwrap();
        while c[0] != b'}' {
            name.push(c[0] as char);
            if c[0] == b'\n' {
                self.line_start = self.file.position();
                self.line += 1;
                break;
            };
            self.file.read(&mut c).unwrap();
        }

        self.tokens.write_one(self.token_from_type(TokenType::Variable(name)));
    }
    fn process_string(&mut self) {
        let mut name = String::new();
        let mut c = [0u8; 1];
        self.file.read(&mut c).unwrap();
        while c[0] != b'"' {
            name.push(c[0] as char);
            if c[0] == b'\n' {
                self.line_start = self.file.position();
                self.line += 1;
                break;
            };
            self.file.read(&mut c).unwrap();
        }

        self.tokens.write_one(self.token_from_type(TokenType::Variable(name)));
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
            if self.file.clone().into_inner().get(self.file.position() as usize).unwrap_or(&0).is_ascii_digit() {
                self.file.read(&mut c).unwrap();
                while c[0].is_ascii_digit() {
                    number.push(c[0] as char);
                    self.file.read(&mut c).unwrap();
                }
            }
            else {
                self.file.set_position(self.file.position()+1);
                return Err(self.error("Unexpected end of number".to_string(), Some("help: remove trailing decimal point".to_string())));
            }
        }
        self.file.set_position(self.file.position()-1);

        self.tokens.write_one(self.token_from_type(TokenType::Number(number.parse().unwrap())));
        Ok(())
    }
    fn identifier(&mut self) {
        let mut ident = String::new();
        let mut c = [0u8; 1];
        self.file.read(&mut c).unwrap();
        while c[0].is_ascii_alphabetic() {
            ident.push(c[0] as char);
            self.file.read(&mut c).unwrap();
        }
        self.file.set_position(self.file.position()-1);

        self.tokens.write_one(self.token_from_type(TokenType::Ident(ident)));
    }
    fn error(&self, message: String, help: Option<String>) -> String {
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
        let column = self.file.position()-self.line_start;
        let column_space: String = vec![' '; column as usize].into_iter().collect();
        format!("\x1b[1;91merror\x1b[0m: {} in \x1b[1;94m{}\x1b[0m at line {}:\n\
              \x1b[1;94m{2} |\x1b[0m  {}\n\
              \x1b[1;94m{} |\x1b[0m {}\x1b[1;91m^ {}\x1b[0m\n", message, self.filename, self.line+1, 
              String::from_utf8(self.file.clone().into_inner()[self.line_start as usize..line_end as usize].into()).unwrap().trim(),
              line_space, column_space, help.unwrap_or("".to_string()),
        )
    }
}


mod tokens;
