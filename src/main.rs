#![allow(non_snake_case)]

use std::time::Instant;

use anyhow::Result;
use argh::FromArgs;
use lexer::Lexer;

use crate::parser::Parser;

pub(crate) mod lexer;
pub(crate) mod parser;

#[derive(FromArgs)]
/// A compiler for skript
struct Options {
    #[argh(positional)]
    file: String
}

fn main() -> Result<()> {
    let options: Options = argh::from_env();

    let contents = std::fs::read_to_string(options.file.clone())?;

    let mut lexer = Lexer::new(options.file.clone(), contents.clone());
    match lexer.process() {
        Ok(_) => {}
        Err(e) => {
            eprint!("{}", e);
            std::process::exit(1);
        }
    };
    let mut parser = Parser::new(lexer.tokens, options.file, contents);
    let now = Instant::now();
    match parser.parse() {
        Ok(_) => {}
        Err(e) => {
            eprint!("{}", e);
            std::process::exit(1);
        }
    };
    let elapsed_time = now.elapsed();
    println!("{:#?}", parser.exprs);
    println!("Took {} us", elapsed_time.as_micros());
    Ok(())
}
