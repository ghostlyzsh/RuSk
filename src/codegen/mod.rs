use std::{ffi::CString, collections::HashMap};

use crate::parser::{ptr::P, Expr, ExprKind, BinOp, Literal, Variable, Block, Ident};
use anyhow::{Result, anyhow};
use llvm_sys::{prelude::*, core::*, transforms::scalar::{LLVMAddReassociatePass, LLVMAddInstructionCombiningPass}, LLVMRealPredicate, LLVMTypeKind};

use self::error::{CodeGenError, ErrorKind};

pub struct CodeGen {
    pub exprs: Vec<P<Expr>>,
    context: LLVMContextRef,
    pub module: LLVMModuleRef,
    builder: LLVMBuilderRef,
    opt_passes: LLVMPassManagerRef,
    scopes: Vec<HashMap<String, (LLVMTypeRef, LLVMValueRef)>>, // Name, (Type, Alloc)
}

impl CodeGen {
    pub unsafe fn new(exprs: Vec<P<Expr>>) -> Self {
        let context = LLVMContextCreate();
        let module = LLVMModuleCreateWithNameInContext(b"RuSk_codegen\0".as_ptr() as *const _, context);
        let builder = LLVMCreateBuilderInContext(context);
        let opt_passes = LLVMCreateFunctionPassManager(LLVMCreateModuleProviderForExistingModule(module));
        LLVMAddInstructionCombiningPass(opt_passes);
        LLVMAddReassociatePass(opt_passes);
        CodeGen {
            context,
            module,
            builder,
            exprs,
            opt_passes,
            scopes: vec![HashMap::new()],
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
        let params = LLVMPointerType(LLVMInt8TypeInContext(self.context), 0);
        let print_type = LLVMFunctionType(void, [params].as_mut_ptr(), 1, 0);
        LLVMAddFunction(self.module, "print\0".as_ptr() as *const _, print_type);
        LLVMDumpType(print_type);
        print!("\n");
        let mut errored = (false, String::new());
        for p_expr in self.exprs.clone() {
            let expr = p_expr.into_inner();

            match self.match_expr(expr) {
                Ok(_) => {
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

        //LLVMRunFunctionPassManager(self.opt_passes, function);

        Ok(function)
    }

    pub unsafe fn match_expr(&mut self, expr: Expr) -> Result<LLVMValueRef> {
        match expr.kind {
            ExprKind::Binary(binop, p_lhs, p_rhs) => {
                let lhs: Expr = p_lhs.into_inner();
                let rhs: Expr = p_rhs.into_inner();

                self.gen_binary(binop, lhs, rhs)
            }
            ExprKind::Lit(lit) => {
                self.gen_lit(lit)
            }
            ExprKind::Var(name) => {
                self.gen_variable(name, expr.line)
            }
            ExprKind::Set(variable, expr) => {
                self.gen_set(variable, expr.into_inner())
            }
            ExprKind::If(cond, block) => {
                self.gen_if(cond.into_inner(), block.into_inner())
            }
            ExprKind::Call(ident, args) => {
                self.gen_call(ident, args, expr.line)
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

    pub unsafe fn gen_if(&mut self, e_condition: Expr, block: Block) -> Result<LLVMValueRef> {
        let condition = self.match_expr(e_condition.clone())?;
        if condition.is_null() {
            return Err(CodeGenError {
                kind: ErrorKind::Null,
                message: "\"if\" cannot have null condition".to_string(),
                line: e_condition.line,
            }.into())
        }

        let zero = LLVMConstReal(LLVMFloatTypeInContext(self.context), 0.);
        let condition = LLVMBuildFCmp(self.builder, LLVMRealPredicate::LLVMRealONE,
                                      condition, zero, "ifcond".as_ptr() as *const _);

        let function = LLVMGetBasicBlockParent(LLVMGetInsertBlock(self.builder));

        let thenBB = LLVMAppendBasicBlockInContext(self.context, function,
                                                   b"then\0".as_ptr() as *const _);
        let elseBB = LLVMCreateBasicBlockInContext(self.context,
                                                   b"else\0".as_ptr() as *const _);
        let mergeBB = LLVMCreateBasicBlockInContext(self.context,
                                                   b"ifcont\0".as_ptr() as *const _);
        LLVMBuildCondBr(self.builder, condition, thenBB, elseBB);

        LLVMPositionBuilderAtEnd(self.builder, thenBB);

        // then
        let then = self.gen_block(block)?;
        if then.is_null() {
            return Err(CodeGenError {
                kind: ErrorKind::Null,
                message: "\"if\" cannot have return nothing in last expression".to_string(),
                line: e_condition.line,
            }.into())
        }
        LLVMBuildBr(self.builder, mergeBB);
        let thenBB = LLVMGetInsertBlock(self.builder);

        // else
        LLVMAppendExistingBasicBlock(function, elseBB);
        LLVMPositionBuilderAtEnd(self.builder, elseBB);
        // else block

        LLVMBuildBr(self.builder, mergeBB);
        let elseBB = LLVMGetInsertBlock(self.builder);

        LLVMAppendExistingBasicBlock(function, mergeBB);
        LLVMPositionBuilderAtEnd(self.builder, mergeBB);
        let phi = LLVMBuildPhi(self.builder, LLVMTypeOf(then), b"iftmp\0".as_ptr() as *const _);
        LLVMAddIncoming(phi, then.cast(), thenBB.cast(), 0);
        LLVMAddIncoming(phi, LLVMTypeOf(then).cast(), elseBB.cast(), 0);
        Ok(phi)
    }

    pub unsafe fn gen_block(&mut self, block: Block) -> Result<LLVMValueRef> {
        let mut errored = (false, String::new());
        let mut last: LLVMValueRef = LLVMConstNull(LLVMVoidType());
        for p_expr in block.exprs.clone() {
            let expr = p_expr.into_inner();

            match self.match_expr(expr) {
                Ok(v) => {
                    last = v;
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

        Ok(last)
    }

    pub unsafe fn gen_set(&mut self, variable: Variable, eval: Expr) -> Result<LLVMValueRef> {
        let scopes = self.scopes.clone();
        let scope = scopes.last().unwrap();
        let value = self.match_expr(eval.clone())?;

        if scope.get(&variable.name.0).is_some() {
            let alloc = scope.get(&variable.name.0).unwrap();
            LLVMBuildStore(self.builder, value, alloc.1);
            return Ok(value);
        }
        if value.is_null() {
            return Err(CodeGenError {
                kind: ErrorKind::Null,
                message: "Cannot set variable to null".to_string(),
                line: eval.line,
            }.into())
        }
        let value_type = LLVMTypeOf(value);

        let name = variable.name.0;
        let alloc = LLVMBuildAlloca(self.builder, value_type, (name.clone() + "\0").as_ptr() as *const _);
        LLVMBuildStore(self.builder, value, alloc);
        self.scopes.last_mut().unwrap().insert(name, (LLVMTypeOf(value), alloc));

        Ok(value)
    }

    pub unsafe fn gen_variable(&mut self, variable: Variable, line: u32) -> Result<LLVMValueRef> {
        println!("scope: {:?}", self.scopes.last());
        let var = match self.scopes.last().unwrap().get(&variable.name.0) {
            Some(v) => v,
            None => return Err(CodeGenError {
                kind: ErrorKind::NotInScope,
                message: "Variable not in scope".to_string(),
                line,
            }.into())
        };
        println!("variable: {:?}", var);
        if var.1.is_null() {
            return Err(CodeGenError {
                kind: ErrorKind::NotInScope,
                message: "Variable not in scope".to_string(),
                line,
            }.into())
        }

        Ok(LLVMBuildLoad2(self.builder, var.0, var.1, variable.name.0.as_ptr() as *const _))
    }

    pub unsafe fn gen_call(&mut self, ident: Ident, args: Vec<P<Expr>>, line: u32) -> Result<LLVMValueRef> {
        let function = LLVMGetNamedFunction(self.module, (ident.0 + "\0").as_ptr() as *const _);
        if function.is_null() {
            return Err(CodeGenError {
                kind: ErrorKind::NotInScope,
                message: "Function not found".to_string(),
                line,
            }.into())
        }

        if LLVMCountParams(function) != args.len() as u32 {
            return Err(CodeGenError {
                kind: ErrorKind::InvalidArgs,
                message: "Wrong number of arguments in function call".to_string(),
                line,
            }.into())
        }

        let mut argsV = Vec::new();
        for arg in args {
            let arg = arg.into_inner();
            let mut arg = self.match_expr(arg)?;
            if arg.is_null() {
                return Err(CodeGenError {
                    kind: ErrorKind::Null,
                    message: "Cannot have null argument".to_string(),
                    line,
                }.into())
            }
            if LLVMGetTypeKind(LLVMTypeOf(arg)) == LLVMTypeKind::LLVMArrayTypeKind {
                let element_type = LLVMGetElementType(LLVMTypeOf(arg));
                arg = LLVMConstPointerCast(arg, LLVMPointerType(element_type, 0));
            }
            argsV.push(arg);
        }
        LLVMDumpValue(argsV[0]);
        print!("\n");
        LLVMDumpType(LLVMTypeOf(function));
        print!("\n");

        Ok(LLVMBuildCall2(self.builder, LLVMTypeOf(function), function,
            argsV.as_mut_ptr(), argsV.len() as u32, "calltmp\0".as_ptr() as *const _))
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
                Ok(LLVMConstReal(LLVMFloatTypeInContext(self.context), num))
            }
            Literal::Boolean(boolean) => {
                Ok(LLVMConstReal(LLVMFloatTypeInContext(self.context), boolean as u64 as f64))
            }
        }
    }
}

pub mod error;
