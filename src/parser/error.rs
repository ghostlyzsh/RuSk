use std::{fmt, error};

#[derive(Debug)]
pub struct ParserError {
    pub kind: ErrorKind,
    pub message: String,
    pub line: u32,
    pub column: u64,
    pub line_str: String,
    pub offset: i32,
    pub filename: String,
    pub help: Option<String>,
}
impl ParserError {
    pub fn into_string(&self) -> String {
        let line_space: String = vec![' '; self.line.to_string().len() as usize].into_iter().collect();

        let column_space: String;
        if self.offset.signum() == -1 {
            column_space = vec![' '; self.column as usize - (-self.offset) as usize].into_iter().collect();
        } else {
            column_space = vec![' '; self.column as usize + self.offset as usize].into_iter().collect();
        }
        let code = match self.kind {
            ErrorKind::UnexpectedCharacter => 1001,
            ErrorKind::ExpectedCharacter => 1002,
            ErrorKind::IndentChange => 1003,
            ErrorKind::UnexpectedEof => 1004,
            ErrorKind::Invalid => 0,
        };

        format!("\x1b[1;91merror[E{:0>4}]\x1b[0m: {} in \x1b[1;94m{}\x1b[0m at line {}:\n\
              \x1b[1;94m{3} |\x1b[0m  {}\n\
              \x1b[1;94m{} |\x1b[0m {}\x1b[1;91m^ {}\x1b[0m\n", code, self.message, self.filename, self.line+1,
              self.line_str,
              line_space, column_space, self.help.clone().unwrap_or("".to_string()),
              )
    }
}
impl fmt::Display for ParserError {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.write_str(&self.into_string())
    }
}
impl error::Error for ParserError {}

#[derive(Debug)]
pub enum ErrorKind {
    UnexpectedCharacter,
    ExpectedCharacter,
    IndentChange,
    UnexpectedEof,
    Invalid,
}
impl From<u32> for ErrorKind {
    fn from(value: u32) -> Self {
        match value {
            1001 => Self::UnexpectedCharacter,
            1002 => Self::ExpectedCharacter,
            1003 => Self::IndentChange,
            1004 => Self::UnexpectedEof,
            _ => Self::Invalid,
        }
    }
}

impl ErrorKind {
    pub fn as_str(&self) -> &'static str {
        use ErrorKind::*;
        match *self {
            UnexpectedCharacter => "unexpected character",
            ExpectedCharacter => "expected character",
            IndentChange => "unexpected indent change",
            UnexpectedEof => "unexpected EOF",
            Invalid => "invalid",
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.write_str(self.as_str())
    }
}
