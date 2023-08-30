use std::time::Instant;

use anyhow::Result;
use argh::FromArgs;
use lexer::Lexer;

mod lexer;

#[derive(FromArgs)]
/// A compiler for skript
struct Options {
    #[argh(positional)]
    file: String
}

fn main() -> Result<()> {
    let options: Options = argh::from_env();

    let contents = std::fs::read_to_string(options.file.clone())?;

    let now = Instant::now();
    let mut lexer = Lexer::new(options.file, contents);
    match lexer.process() {
        Ok(_) => {}
        Err(e) => {
            eprint!("{}", e);
            std::process::exit(1);
        }
    };
    let elapsed_time = now.elapsed();
    println!("Tokens: {:#?}", lexer.tokens);
    println!("Took {} us", elapsed_time.as_micros());
    Ok(())
}
