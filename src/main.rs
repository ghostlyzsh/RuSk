#![allow(non_snake_case)]

use std::ffi::CString;

use anyhow::Result;
use argh::FromArgs;
use lexer::Lexer;
use llvm_sys::target_machine::{LLVMCodeGenFileType, LLVMTargetMachineEmitToFile};

use crate::{parser::Parser, codegen::CodeGen};

pub mod lexer;
pub mod parser;
pub mod codegen;

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
    //println!("Tokens: {:#?}", lexer.tokens);
    let mut parser = Parser::new(lexer.tokens, options.file, contents);
    match parser.parse() {
        Ok(_) => {}
        Err(e) => {
            eprint!("{}", e);
            std::process::exit(1);
        }
    };

    unsafe {
        let output = "output.o";
        let mut codegen = CodeGen::new(parser.exprs);
        match codegen.gen_code() {
            Ok(_) => {}
            Err(e) => {
                println!("error");
                eprint!("{}", e);
                std::process::exit(1);
            }
        };
        let mut error = std::ptr::null_mut();
        let codegen_type = LLVMCodeGenFileType::LLVMObjectFile;
        if LLVMTargetMachineEmitToFile(codegen.machine, codegen.module, output.to_string().as_mut_ptr() as *mut _, codegen_type, &mut error) == 1 {
            eprintln!("Couldn't output file: {:?}", CString::from_raw(error as *mut _));
        }
        codegen.end();
    }
    Ok(())
}
