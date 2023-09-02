use std::ffi::CString;

use crate::parser::{ptr::P, Expr, ExprKind, BinOp, Literal};
use anyhow::{Result, anyhow};
use llvm_sys::{prelude::*, core::*};

use self::error::{CodeGenError, ErrorKind};

pub struct CodeGen {
    pub exprs: Vec<P<Expr>>,
    context: LLVMContextRef,
    pub module: LLVMModuleRef,
    builder: LLVMBuilderRef,
}

impl CodeGen {
    pub unsafe fn new(exprs: Vec<P<Expr>>) -> Self {
        let context = LLVMContextCreate();
        let module = LLVMModuleCreateWithNameInContext(b"RuSk_codegen\0".as_ptr() as *const _, context);
        let builder = LLVMCreateBuilderInContext(context);
        CodeGen {
            context,
            module,
            builder,
            exprs,
        }
    }

    pub unsafe fn end(&mut self) {
        LLVMDisposeModule(self.module);
        LLVMDisposeBuilder(self.builder);
        LLVMContextDispose(self.context);
    }

    pub unsafe fn gen_code(&mut self) -> Result<LLVMValueRef> {
        let void = LLVMVoidTypeInContext(self.context);
        let function_type = LLVMFunctionType(void, std::ptr::null_mut(), 0, 0);
        let function = LLVMAddFunction(self.module, b"__anon_expr\0".as_ptr() as *const _, function_type);
        let bb = LLVMAppendBasicBlockInContext(self.context,
            function,
            b"entry\0".as_ptr() as *const _,
        );
        LLVMPositionBuilderAtEnd(self.builder, bb);
        let mut errored = (false, String::new());
        for p_expr in self.exprs.clone() {
            let expr = p_expr.into_inner();

            match self.match_expr(expr) {
                Ok(value) => {
                }
                Err(e) => {
                    errored.0 = true;
                    errored.1.push_str(e.to_string().as_str());
                }
            };
        }
        if errored.0 {
            return Err(anyhow!(errored.1));
        }
        LLVMBuildRetVoid(self.builder);
        Ok(function)
    }

    pub unsafe fn match_expr(&mut self, expr: Expr) -> Result<LLVMValueRef> {
        match expr.kind {
            ExprKind::Binary(binop, p_lhs, p_rhs) => {
                println!("binary");
                let lhs: Expr = p_lhs.into_inner();
                let rhs: Expr = p_rhs.into_inner();

                self.gen_binary(binop, lhs, rhs)
            }
            ExprKind::Lit(lit) => {
                println!("literal");
                self.gen_lit(lit)
            }
            _ => {
                Err(CodeGenError {
                    kind: ErrorKind::Invalid,
                    line: expr.line,
                    message: "Expression type not handled".to_string(),
                }.into())
            }
        }
    }
    
    pub unsafe fn gen_binary(&mut self, binop: BinOp, e_lhs: Expr, e_rhs: Expr) -> Result<LLVMValueRef> {
        let lhs = self.match_expr(e_lhs.clone())?;
        let rhs = self.match_expr(e_rhs)?;
        if lhs.is_null() || rhs.is_null() {
            return Err(CodeGenError {
                kind: ErrorKind::Null,
                line: e_lhs.line,
                message: "Found null value in binary operation".to_string(),
            }.into());
        }

        match binop {
            BinOp::Add => {
                Ok(LLVMBuildFAdd(self.builder, lhs, rhs, b"addtmp\0".as_ptr() as *const _))
            }
            BinOp::Sub => {
                Ok(LLVMBuildFSub(self.builder, lhs, rhs, b"subtmp\0".as_ptr() as *const _))
            }
            BinOp::Mul => {
                Ok(LLVMBuildFMul(self.builder, lhs, rhs, b"multmp\0".as_ptr() as *const _))
            }
            BinOp::Div => {
                Ok(LLVMBuildFDiv(self.builder, lhs, rhs, b"divtmp\0".as_ptr() as *const _))
            }
            _ => {
                Err(CodeGenError {
                    kind: ErrorKind::Invalid,
                    line: e_lhs.line,
                    message: "Case not handled in binary operation".to_string(),
                }.into())
            }
        }
    }

    pub unsafe fn gen_lit(&mut self, lit: Literal) -> Result<LLVMValueRef> {
        match lit {
            Literal::Text(text) => {
                let c_str = CString::new(text.clone()).unwrap();
                Ok(LLVMConstStringInContext(self.context, c_str.as_ptr(), text.len() as u32, 0))
            }
            Literal::Number(num) => {
                println!("number: {}", num);
                Ok(LLVMConstReal(LLVMFloatTypeInContext(self.context), num))
            }
            Literal::Boolean(boolean) => {
                Ok(LLVMConstReal(LLVMFloatTypeInContext(self.context), boolean as u64 as f64))
            }
        }
    }
}

pub mod error;
