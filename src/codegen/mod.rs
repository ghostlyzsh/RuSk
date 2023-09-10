use std::{ffi::CString, collections::HashMap};

use crate::parser::{ptr::P, Expr, ExprKind, BinOp, Literal, Variable, Block, Ident, Type};
use anyhow::{Result, anyhow};
use llvm_sys::{prelude::*, core::*, transforms::{scalar::{LLVMAddReassociatePass, LLVMAddInstructionCombiningPass, LLVMAddGVNPass, LLVMAddCFGSimplificationPass}, util::LLVMAddPromoteMemoryToRegisterPass}, LLVMRealPredicate, LLVMTypeKind, target::*, target_machine::{LLVMGetDefaultTargetTriple, LLVMGetTargetFromTriple, LLVMCreateTargetDataLayout, LLVMCreateTargetMachine, LLVMCodeModel, LLVMRelocMode, LLVMCodeGenOptLevel, LLVMTargetRef, LLVMGetFirstTarget, LLVMTargetMachineRef, LLVMGetHostCPUFeatures}};

use self::error::{CodeGenError, ErrorKind};

pub struct CodeGen {
    pub exprs: Vec<P<Expr>>,
    context: LLVMContextRef,
    pub module: LLVMModuleRef,
    builder: LLVMBuilderRef,
    opt_passes: LLVMPassManagerRef,
    scopes: Vec<HashMap<String, (LLVMTypeRef, LLVMValueRef)>>, // Name, (Value, Type, Alloc)
    functions: HashMap<String, (LLVMTypeRef, bool)>,
    pub machine: LLVMTargetMachineRef,
}

impl CodeGen {
    pub unsafe fn new(exprs: Vec<P<Expr>>) -> Self {
        let target_triple = LLVMGetDefaultTargetTriple();
        LLVM_InitializeAllTargetInfos();
        LLVM_InitializeAllTargets();
        LLVM_InitializeAllTargetMCs();
        LLVM_InitializeAllAsmParsers();
        LLVM_InitializeAllAsmPrinters();

        let mut target: LLVMTargetRef = LLVMGetFirstTarget().cast();
        let mut error = "\0".to_string().as_mut_ptr().cast();
        if LLVMGetTargetFromTriple(target_triple, &mut target, &mut error) == 1 {
            eprintln!("Couldn't get target from triple: {:?}", error);
            std::process::exit(1);
        }

        let cpu = "generic\0".as_ptr() as *const _;
        //let features = "\0".as_ptr() as *const _;
        let model = LLVMCodeModel::LLVMCodeModelDefault;
        let reloc = LLVMRelocMode::LLVMRelocDefault;
        let level = LLVMCodeGenOptLevel::LLVMCodeGenLevelDefault;
        let target_machine = LLVMCreateTargetMachine(target, target_triple,
            cpu, LLVMGetHostCPUFeatures(), level, reloc, model);

        let context = LLVMContextCreate();
        let module = LLVMModuleCreateWithNameInContext(b"RuSk_codegen\0".as_ptr() as *const _, context);
        LLVMSetTarget(module, target_triple);
        LLVMSetDataLayout(module, LLVMCopyStringRepOfTargetData(LLVMCreateTargetDataLayout(target_machine)));

        let builder = LLVMCreateBuilderInContext(context);
        let opt_passes = LLVMCreateFunctionPassManager(LLVMCreateModuleProviderForExistingModule(module));
        LLVMAddPromoteMemoryToRegisterPass(opt_passes);
        LLVMAddInstructionCombiningPass(opt_passes);
        LLVMAddReassociatePass(opt_passes);
        LLVMAddGVNPass(opt_passes);
        LLVMAddCFGSimplificationPass(opt_passes);

        CodeGen {
            context,
            module,
            builder,
            exprs,
            opt_passes,
            scopes: vec![HashMap::new()],
            functions: HashMap::new(),
            machine: target_machine,
        }
    }

    pub unsafe fn end(&mut self) {
        LLVMDisposeModule(self.module);
        LLVMDisposeBuilder(self.builder);
        LLVMContextDispose(self.context);
    }

    pub unsafe fn gen_code(&mut self) -> Result<LLVMValueRef> {
        //let void = LLVMVoidTypeInContext(self.context);
        let int32 = LLVMInt32TypeInContext(self.context);

        let function_type = LLVMFunctionType(int32, LLVMInt32TypeInContext(self.context).cast(), 0, 0);
        let function = LLVMAddFunction(self.module, b"main\0".as_ptr() as *const _, function_type);
        let bb = LLVMAppendBasicBlockInContext(self.context,
            function,
            b"\0".as_ptr() as *const _,
        );

        let mut errored = (false, String::new());
        for p_expr in self.exprs.clone() {
            LLVMPositionBuilderAtEnd(self.builder, bb);
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
        LLVMPositionBuilderAtEnd(self.builder, bb);
        LLVMBuildRet(self.builder, LLVMConstInt(LLVMInt32TypeInContext(self.context), 0, 0));

        LLVMRunFunctionPassManager(self.opt_passes, function);

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
            ExprKind::If(cond, block, el) => {
                self.gen_if(cond.into_inner(), block.into_inner(), el)
            }
            ExprKind::Call(ident, args) => {
                self.gen_call(ident, args, expr.line)
            }
            ExprKind::Native(name, args, var, ret) => {
                self.gen_native(name, args, var, ret)
            }
            ExprKind::Block(block) => {
                self.gen_block(block.into_inner())
            }
            ExprKind::Function(name, args, block, ret) => {
                self.gen_function(name, args, block, ret)
            }
            _ => {
                Err(CodeGenError {
                    kind: ErrorKind::Invalid,
                    line: expr.line,
                    message: format!("Expression type not handled {:?}", expr.kind),
                }.into())
            }
        }
    }

    pub unsafe fn gen_function(&mut self, name: Ident, args: Vec<P<Expr>>, block: P<Block>, ret: Option<Type>) -> Result<LLVMValueRef> {
        let mut f_args = Vec::with_capacity(args.len());
        for arg in args.clone() {
            if let ExprKind::Arg(_name, ty) = arg.kind.clone() {
                match ty {
                    Type::Text => {
                        f_args.push(LLVMPointerType(LLVMInt8TypeInContext(self.context), 0));
                    }
                    Type::Number => {
                        f_args.push(LLVMFloatTypeInContext(self.context));
                    }
                    Type::Integer => {
                        f_args.push(LLVMInt64TypeInContext(self.context));
                    }
                    Type::Boolean => {
                        f_args.push(LLVMInt1TypeInContext(self.context));
                    }
                };
            } else {
                return Err(CodeGenError {
                    kind: ErrorKind::InvalidArgs,
                    message: "Codegen error: function arg".to_string(),
                    line: arg.line,
                }.into())
            }
        }
        let ret_type;
        if let Some(ty) = ret {
            match ty {
                Type::Text => {
                    ret_type = LLVMPointerType(LLVMInt8TypeInContext(self.context), 0);
                }
                Type::Number => {
                    ret_type = LLVMFloatTypeInContext(self.context);
                }
                Type::Integer => {
                    ret_type = LLVMInt64TypeInContext(self.context);
                }
                Type::Boolean => {
                    ret_type = LLVMInt1TypeInContext(self.context);
                }
            }
        } else {
            ret_type = LLVMVoidTypeInContext(self.context);
        }
        let function_type = LLVMFunctionType(ret_type, f_args.as_mut_ptr(), f_args.len() as u32, 0);

        let function = LLVMAddFunction(self.module, (name.0.clone() + "\0").as_ptr() as *const _, function_type);
        LLVMSetLinkage(function, llvm_sys::LLVMLinkage::LLVMExternalLinkage);

        let bb = LLVMAppendBasicBlockInContext(self.context, function, format!("{}\0", name.0).as_ptr() as *const _);
        LLVMPositionBuilderAtEnd(self.builder, bb);

        self.scopes.push(HashMap::new());
        for (i, arg) in args.clone().iter().enumerate() {
            if let ExprKind::Arg(name, ty) = arg.kind.clone() {
                let arg_ty;
                match ty {
                    Type::Text => {
                        arg_ty = LLVMPointerType(LLVMInt8TypeInContext(self.context), 0);
                    }
                    Type::Number => {
                        arg_ty = LLVMFloatTypeInContext(self.context);
                    }
                    Type::Integer => {
                        arg_ty = LLVMInt64TypeInContext(self.context);
                    }
                    Type::Boolean => {
                        arg_ty = LLVMInt1TypeInContext(self.context);
                    }
                }
                let alloca = LLVMBuildAlloca(self.builder, arg_ty, name.0.as_ptr() as *const _);
                LLVMBuildStore(self.builder, LLVMGetParam(function, i as u32), alloca);
                self.scopes.last_mut().unwrap().insert(name.0.clone(), (arg_ty, alloca));
            } else {
                return Err(CodeGenError {
                    kind: ErrorKind::InvalidArgs,
                    message: "Codegen error: function arg".to_string(),
                    line: arg.line,
                }.into())
            }
        }
        let ret_val = self.gen_block(block.clone().into_inner())?;
        if ret_type != LLVMVoidTypeInContext(self.context) &&
            LLVMGetTypeKind(LLVMTypeOf(ret_val)) != LLVMGetTypeKind(ret_type) {
            return Err(CodeGenError {
                kind: ErrorKind::MismatchedTypes,
                message: format!("Mismatched return type on function {}", name.0),
                line: block.exprs.last().unwrap_or(&args[0]).line,
            }.into())
        }
        if ret_type != LLVMVoidTypeInContext(self.context) {
            LLVMBuildRet(self.builder, ret_val);
        } else {
            LLVMBuildRetVoid(self.builder);
        }

        LLVMRunFunctionPassManager(self.opt_passes, function);

        self.functions.insert(name.0, (function_type, false));

        Ok(ret_val)
    }

    pub unsafe fn gen_if(&mut self, e_condition: Expr, block: Block, el: Option<P<Expr>>) -> Result<LLVMValueRef> {
        let condition = self.match_expr(e_condition.clone())?;
        if condition.is_null() {
            return Err(CodeGenError {
                kind: ErrorKind::Null,
                message: "\"if\" cannot have null condition".to_string(),
                line: e_condition.line,
            }.into())
        }

        if let Some(expr) = el {
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
            let mut then = self.gen_block(block.clone())?;
            if then.is_null() {
                return Err(CodeGenError {
                    kind: ErrorKind::Null,
                    message: "\"if\" cannot have return nothing in last expression".to_string(),
                    line: e_condition.line,
                }.into())
            }
            LLVMBuildBr(self.builder, mergeBB);
            let mut thenBB = LLVMGetInsertBlock(self.builder);

            LLVMAppendExistingBasicBlock(function, elseBB);
            LLVMPositionBuilderAtEnd(self.builder, elseBB);
            let mut else_v = self.match_expr(expr.into_inner())?;
            if else_v.is_null() {
                return Err(CodeGenError {
                    kind: ErrorKind::Null,
                    message: "\"if\" cannot have return nothing in last expression".to_string(),
                    line: e_condition.line,
                }.into())
            }
            LLVMBuildBr(self.builder, mergeBB);
            let mut elseBB = LLVMGetInsertBlock(self.builder);

            LLVMAppendExistingBasicBlock(function, mergeBB);
            LLVMPositionBuilderAtEnd(self.builder, mergeBB);
            let phi = LLVMBuildPhi(self.builder, LLVMTypeOf(then), b"iftmp\0".as_ptr() as *const _);
            LLVMAddIncoming(phi, &mut then, &mut thenBB, 1);
            LLVMAddIncoming(phi, &mut else_v, &mut elseBB, 1);
            Ok(phi)
        } else {
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
            let mut then = self.gen_block(block.clone())?;
            if then.is_null() {
                return Err(CodeGenError {
                    kind: ErrorKind::Null,
                    message: "\"if\" cannot have return nothing in last expression".to_string(),
                    line: e_condition.line,
                }.into())
            }
            LLVMBuildBr(self.builder, mergeBB);
            let mut thenBB = LLVMGetInsertBlock(self.builder);

            LLVMAppendExistingBasicBlock(function, elseBB);
            LLVMPositionBuilderAtEnd(self.builder, elseBB);
            LLVMBuildBr(self.builder, mergeBB);
            let mut elseBB = LLVMGetInsertBlock(self.builder);

            LLVMAppendExistingBasicBlock(function, mergeBB);
            LLVMPositionBuilderAtEnd(self.builder, mergeBB);
            let phi = LLVMBuildPhi(self.builder, LLVMTypeOf(then), b"iftmp\0".as_ptr() as *const _);
            LLVMAddIncoming(phi, &mut then, &mut thenBB, 1);
            LLVMAddIncoming(phi, &mut LLVMConstNull(LLVMTypeOf(then)), &mut elseBB, 1);
            Ok(then)
        }

    }

    pub unsafe fn gen_block(&mut self, block: Block) -> Result<LLVMValueRef> {
        self.scopes.push(HashMap::new());
        let mut errored = (false, String::new());
        let mut last = LLVMConstNull(LLVMVoidType());
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
        self.scopes.pop();

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
        let alloc = LLVMBuildAlloca(self.builder, value_type, "\0".as_ptr() as *const _);
        LLVMBuildStore(self.builder, value, alloc);
        self.scopes.last_mut().unwrap().insert(name, (LLVMTypeOf(value), alloc));

        Ok(value)
    }

    pub unsafe fn gen_native(&mut self, name: Ident, args: Vec<P<Expr>>, var: bool, ret: Option<P<Expr>>) -> Result<LLVMValueRef> {
        let mut args_type = Vec::new();
        for arg in args {
            let arg = arg.into_inner();
            if let ExprKind::Arg(ident, t) = arg.kind {
                let tt;
                match t {
                    Type::Text => {
                        tt = LLVMPointerType(LLVMInt8TypeInContext(self.context), 0);
                    }
                    Type::Number => {
                        tt = LLVMFloatType();
                    }
                    Type::Integer => {
                        tt = LLVMInt64Type();
                    }
                    Type::Boolean => {
                        tt = LLVMInt1Type();
                    }
                }
                args_type.push((ident.0, tt));
            } else {
                return Err(CodeGenError {
                    kind: ErrorKind::Invalid,
                    message: "Parser broke".to_string(),
                    line: arg.line,
                }.into())
            }
        }
        let mut ret_type = LLVMVoidType();
        if let Some(ret) = ret {
            let ret = ret.into_inner();
            if let ExprKind::Type(t) = ret.kind {
                match t {
                    Type::Text => {
                        ret_type = LLVMPointerType(LLVMInt8TypeInContext(self.context), 0);
                    }
                    Type::Number => {
                        ret_type = LLVMFloatType();
                    }
                    Type::Integer => {
                        ret_type = LLVMInt64Type();
                    }
                    Type::Boolean => {
                        ret_type = LLVMInt1Type();
                    }
                }
            } else {
                return Err(CodeGenError {
                    kind: ErrorKind::Invalid,
                    message: "Parser broke".to_string(),
                    line: ret.line,
                }.into())
            }
        }
        let function_type = LLVMFunctionType(ret_type, args_type.iter().map(|v| v.1).collect::<Vec<_>>().as_mut_ptr(), args_type.len() as u32, var as i32);

        LLVMAddFunction(self.module, (name.0.clone() + "\0").as_ptr() as *const _, function_type);
        self.functions.insert(name.0, (function_type, var));

        Ok(LLVMConstNull(LLVMFloatTypeInContext(self.context)))
    }

    pub unsafe fn gen_variable(&mut self, variable: Variable, line: u32) -> Result<LLVMValueRef> {
        let mut var = None;
        self.scopes.iter().for_each(|scope| {
            match scope.get(&variable.name.0) {
                Some(v) => {
                    var = Some(v)
                }
                None => {}
            };
        });
        let var = match var {
            Some(v) => v,
            None => return Err(CodeGenError {
                kind: ErrorKind::NotInScope,
                message: "Variable not in scope".to_string(),
                line,
            }.into())
        };
        if var.0.is_null() {
            return Err(CodeGenError {
                kind: ErrorKind::NotInScope,
                message: "Variable not in scope".to_string(),
                line,
            }.into())
        }

        Ok(LLVMBuildLoad2(self.builder, var.0, var.1, "\0".as_ptr() as *const _))
    }

    pub unsafe fn gen_call(&mut self, ident: Ident, args: Vec<P<Expr>>, line: u32) -> Result<LLVMValueRef> {
        let function = LLVMGetNamedFunction(self.module, (ident.0.clone() + "\0").as_ptr() as *const _);
        let function_type = *match self.functions.get(&ident.0) {
            Some(t) => t,
            None => {
                return Err(CodeGenError {
                    kind: ErrorKind::NotInScope,
                    message: "Function not found".to_string(),
                    line,
                }.into())
            }
        };
        if function.is_null() {
            return Err(CodeGenError {
                kind: ErrorKind::NotInScope,
                message: "Function not found".to_string(),
                line,
            }.into())
        }
        let param_num = LLVMCountParamTypes(function_type.0);

        let mut arg_types = Vec::with_capacity(param_num as usize);
        if !function_type.1 {
            if param_num != args.len() as u32 {
                return Err(CodeGenError {
                    kind: ErrorKind::InvalidArgs,
                    message: "Wrong number of arguments in function call".to_string(),
                    line,
                }.into())
            }

            LLVMGetParamTypes(function_type.0, arg_types.as_mut_ptr());
            arg_types.set_len(param_num as usize);
        }

        let mut argsV = Vec::new();
        for (i, arg) in args.iter().enumerate() {
            let arg = arg.clone().into_inner();
            let mut arg = self.match_expr(arg)?;
            if arg.is_null() {
                return Err(CodeGenError {
                    kind: ErrorKind::Null,
                    message: "Cannot have null argument".to_string(),
                    line,
                }.into())
            }
            if !function_type.1 {
                if LLVMGetTypeKind(LLVMTypeOf(arg)) == LLVMTypeKind::LLVMArrayTypeKind {
                    let element_type = LLVMGetElementType(LLVMTypeOf(arg));
                    let num_indices = LLVMGetArrayLength(LLVMTypeOf(arg));
                    println!("{}", num_indices);
                    let mut int = LLVMConstInt(LLVMInt32Type(), 0, 0);
                    arg = LLVMBuildInBoundsGEP2(self.builder, LLVMPointerType(element_type, 0), arg, &mut int, 0, "\0".as_ptr() as *const _);
                }

                if LLVMTypeOf(arg) != arg_types[i] {
                    return Err(CodeGenError {
                        kind: ErrorKind::MismatchedTypes,
                        message: format!("Argument {} has wrong type", i),
                        line,
                    }.into())
                }
            }
            argsV.push(arg);
        }

        let ret = LLVMBuildCall2(self.builder, function_type.0, function,
            argsV.as_mut_ptr(), argsV.len() as u32, "\0".as_ptr() as *const _);
        Ok(ret)
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
                Ok(LLVMBuildGlobalStringPtr(self.builder, c_str.as_ptr(), "\0".as_ptr() as *const _))
            }
            Literal::Number(num) => {
                Ok(LLVMConstReal(LLVMFloatTypeInContext(self.context), num))
            }
            Literal::Integer(num) => {
                Ok(LLVMConstInt(LLVMInt64TypeInContext(self.context), std::mem::transmute(num), 1))
            }
            Literal::Boolean(boolean) => {
                Ok(LLVMConstReal(LLVMFloatTypeInContext(self.context), boolean as u64 as f64))
            }
        }
    }
}

pub mod error;
