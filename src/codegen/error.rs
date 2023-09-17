use std::{fmt, error};

#[derive(Debug)]
pub struct CodeGenError {
    pub kind: ErrorKind,
    pub message: String,
    pub line: u32,
}
impl CodeGenError {
    pub fn into_string(&self) -> String {
        let code = match self.kind {
            ErrorKind::MismatchedTypes => 3001,
            ErrorKind::Null => 3002,
            ErrorKind::NotInScope => 3003,
            ErrorKind::InvalidArgs => 3004,
            ErrorKind::InvalidType => 3005,
            ErrorKind::Invalid => 0,
        };

        format!("\x1b[1;91merror[E{:0>4}]\x1b[0m: {} at line {}\n",
                code, self.message, self.line+1, /*self.filename, self.line+1,
              self.line_str,*/
              //line_space, column_space, self.help.clone().unwrap_or("".to_string()),
              )
    }
}
impl fmt::Display for CodeGenError {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.write_str(&self.into_string())
    }
}
impl error::Error for CodeGenError {}

#[derive(Debug)]
pub enum ErrorKind {
    MismatchedTypes,
    Null,
    NotInScope,
    InvalidArgs,
    InvalidType,
    Invalid,
}
impl From<u32> for ErrorKind {
    fn from(value: u32) -> Self {
        match value {
            3001 => Self::MismatchedTypes,
            3002 => Self::Null,
            3003 => Self::NotInScope,
            3004 => Self::InvalidArgs,
            3005 => Self::InvalidType,
            _ => Self::Invalid,
        }
    }
}

impl ErrorKind {
    pub fn as_str(&self) -> &'static str {
        use ErrorKind::*;
        match *self {
            MismatchedTypes => "mismatched types",
            Null => "found null",
            NotInScope => "not in scope",
            InvalidArgs => "invalid arguments",
            InvalidType => "invalid type",
            Invalid => "invalid",
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.write_str(self.as_str())
    }
}
