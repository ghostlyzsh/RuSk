#![allow(non_snake_case)]

use std::{time::Instant, ffi::CString};

use anyhow::Result;
use argh::FromArgs;
use lazy_static::lazy_static;
use lexer::Lexer;
use llvm_sys::{core::{LLVMDumpModule, LLVMDisposeModule, LLVMPrintModuleToString, LLVMInstructionEraseFromParent, LLVMGetTarget}, target_machine::{LLVMGetDefaultTargetTriple, LLVMGetTargetFromTriple, LLVMCodeGenFileType, LLVMTargetMachineEmitToFile, LLVMGetTargetMachineTriple}, target::*};

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

    let now = Instant::now();
    unsafe {
        let output = "output.o";
        let mut codegen = CodeGen::new(parser.exprs);
        match codegen.gen_code() {
            Ok(f) => {
            }
            Err(e) => {
                println!("error");
                eprint!("{}", e);
                std::process::exit(1);
            }
        };
        LLVMDumpModule(codegen.module);
        let error = "\0".to_string().as_mut_ptr();
        let codegen_type = LLVMCodeGenFileType::LLVMObjectFile;
        if LLVMTargetMachineEmitToFile(codegen.machine, codegen.module, output.to_string().as_mut_ptr() as *mut _, codegen_type, error.cast()) == 1 {
            eprintln!("Couldn't output file: {:?}", CString::from_raw(error as *mut _));
        }
        codegen.end();
    }
    let elapsed_time = now.elapsed();
    println!("Took {} ns", elapsed_time.as_nanos());
    Ok(())
}
