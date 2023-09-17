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
    scopes: Vec<HashMap<String, (LLVMTypeRef, LLVMValueRef)>>, // Name, (LLVMType, Alloc)
    functions: HashMap<String, (LLVMTypeRef, bool)>, // Name, (function_type, has_var_args)
    pub machine: LLVMTargetMachineRef,
    optimize: bool,
}

impl CodeGen {
    pub unsafe fn new(exprs: Vec<P<Expr>>, optimize: bool) -> Self {
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
            optimize,
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
        //LLVMPositionBuilderAtEnd(self.builder, bb);
        LLVMBuildRet(self.builder, LLVMConstInt(LLVMInt32TypeInContext(self.context), 0, 0));

        if self.optimize {
            LLVMRunFunctionPassManager(self.opt_passes, function);
        }

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
            ExprKind::Array(array) => {
                let mut array = self.process_array(array)?;
                if array.len() == 0 {
                    return Ok(LLVMConstArray(LLVMVoidType(), array.as_mut_ptr(), 0))
                }
                Ok(LLVMConstArray(LLVMTypeOf(array[0]), array.as_mut_ptr(), array.len() as u32))
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
            ExprKind::While(cond, block) => {
                self.gen_while(cond.into_inner(), block.into_inner())
            }
            ExprKind::Call(ident, args) => {
                self.gen_call(ident, args, expr.line)
            }
            ExprKind::Native(name, args, var, ret) => {
                self.gen_native(name, args, var, ret)
            }
            ExprKind::Block(block) => {
                self.scopes.push(HashMap::new());
                self.gen_block(block.into_inner())
            }
            ExprKind::Function(name, args, block, ret) => {
                self.gen_function(name, args, block, ret)
            }
            /*ExprKind::Return(expr) => {
                self.gen_return(expr)
            }*/
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
                    _ => { // add array functionality later
                        return Err(CodeGenError {
                            kind: ErrorKind::InvalidArgs,
                            message: "Codegen error: function arg".to_string(),
                            line: arg.line,
                        }.into())
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
                _ => { // add array functionality later
                    return Err(CodeGenError {
                        kind: ErrorKind::InvalidArgs,
                        message: "Codegen error: function arg".to_string(),
                        line: args[0].line,
                    }.into())
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
                    _ => { // add array functionality later
                        return Err(CodeGenError {
                            kind: ErrorKind::InvalidArgs,
                            message: "Codegen error: function arg".to_string(),
                            line: args[0].line,
                        }.into())
                    }
                }
                let alloca = LLVMBuildAlloca(self.builder, arg_ty, (name.0.clone() + "\0").as_ptr() as *const _);
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

        if self.optimize {
            LLVMRunFunctionPassManager(self.opt_passes, function);
        }

        self.functions.insert(name.0, (function_type, false));

        Ok(ret_val)
    }
    /*pub unsafe fn gen_return(&mut self, expr: Option<P<Expr>>) -> Result<LLVMValueRef> {
        if let Some(expr) = expr {
            Ok(LLVMBuildRet(self.builder, self.match_expr(expr.into_inner())?))
        } else {
            Ok(LLVMBuildRetVoid(self.builder))
        }
    }*/

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
                                          condition, zero, "\0".as_ptr() as *const _);

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
            self.scopes.push(HashMap::new());
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
                                          condition, zero, "ifcond\0".as_ptr() as *const _);
            
            let function = LLVMGetBasicBlockParent(LLVMGetInsertBlock(self.builder));
            let thenBB = LLVMAppendBasicBlockInContext(self.context, function,
                                                       b"then\0".as_ptr() as *const _);
            let elseBB = LLVMCreateBasicBlockInContext(self.context,
                                                       b"else\0".as_ptr() as *const _);
            let mergeBB = LLVMCreateBasicBlockInContext(self.context,
                                                       b"ifcont\0".as_ptr() as *const _);
            LLVMBuildCondBr(self.builder, condition, thenBB, elseBB);

            LLVMPositionBuilderAtEnd(self.builder, thenBB);
            self.scopes.push(HashMap::new());
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
    pub unsafe fn gen_while(&mut self, cond: Expr, block: Block) -> Result<LLVMValueRef> {
        let function = LLVMGetBasicBlockParent(LLVMGetInsertBlock(self.builder));

        let v_condition = self.match_expr(cond.clone())?;
        if v_condition.is_null() {
            return Err(CodeGenError {
                kind: ErrorKind::Null,
                message: "\"while\" cannot have null condition".to_string(),
                line: cond.line,
            }.into())
        }

        let loopBB = LLVMAppendBasicBlockInContext(self.context, function, "\0".as_ptr() as *const _);
        let afterBB = LLVMCreateBasicBlockInContext(self.context, "\0".as_ptr() as *const _);

        LLVMBuildCondBr(self.builder, v_condition, loopBB, afterBB);
        LLVMPositionBuilderAtEnd(self.builder, loopBB);

        self.scopes.push(HashMap::new());
        self.gen_block(block.clone())?;
        let v_condition = self.match_expr(cond.clone())?;
        if v_condition.is_null() {
            return Err(CodeGenError {
                kind: ErrorKind::Null,
                message: "\"while\" cannot have null condition".to_string(),
                line: cond.line,
            }.into())
        }
        LLVMBuildCondBr(self.builder, v_condition, loopBB, afterBB);

        LLVMAppendExistingBasicBlock(function, afterBB);
        LLVMPositionBuilderAtEnd(self.builder, afterBB);

        Ok(LLVMConstNull(LLVMVoidType()))
    }

    pub unsafe fn gen_block(&mut self, block: Block) -> Result<LLVMValueRef> {
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
        self.scopes.pop();
        if errored.0 {
            return Err(anyhow!(errored.1));
        }

        Ok(last)
    }

    pub unsafe fn gen_set(&mut self, variable: Variable, eval: Expr) -> Result<LLVMValueRef> {
        let scopes = self.scopes.clone();
        let scope = scopes.iter().fold(HashMap::new(), |sum, v| sum.into_iter().chain(v).collect());
        let value: LLVMValueRef;
        if let ExprKind::Array(array) = eval.clone().kind {
            // set {variable::1} to [1, 2, 3] will NOT work right now.
            // TODO fix that
            if variable.mutable {
                let mut v_array = self.process_array(array)?;
                let arr_type;
                if v_array.len() == 0 {
                    arr_type = LLVMVoidType();
                } else {
                    arr_type = LLVMTypeOf(v_array[0]);
                }
                if scope.get(&variable.name.0).is_some() {
                    let alloc = scope.get(&variable.name.0).unwrap();
                    let value = LLVMConstArray(arr_type, v_array.as_mut_ptr(), v_array.len() as u32);
                    LLVMBuildStore(self.builder, value, alloc.1);
                    return Ok(value);
                }
                let alloc = LLVMBuildAlloca(self.builder, LLVMPointerType(arr_type, 0), (variable.name.0.clone() + "\0").as_ptr() as *const _);
                LLVMBuildStore(self.builder, LLVMConstArray(arr_type, v_array.as_mut_ptr(), v_array.len() as u32), alloc);
                self.scopes.last_mut().unwrap().insert(variable.name.0, (LLVMArrayType(arr_type, v_array.len() as u32), alloc));
                return Ok(alloc);
            } else {
                value = self.match_expr(eval.clone())?;
            }
        } else {
            value = self.match_expr(eval.clone())?;
        }

        if scope.get(&variable.name.0).is_some() {
            let alloc = scope.get(&variable.name.0).unwrap();
            if let Some(index) = variable.index {
                let index = self.match_expr(index.into_inner())?;
                if LLVMTypeKind::LLVMIntegerTypeKind != LLVMGetTypeKind(LLVMTypeOf(index)) {
                }
                let mut indices = [LLVMConstInt(LLVMInt64TypeInContext(self.context), 0, 0), index];
                let ptr = LLVMBuildInBoundsGEP2(self.builder, alloc.0, alloc.1, indices.as_mut_ptr(), 2, "\0".as_ptr() as *const _);
                return Ok(LLVMBuildStore(self.builder, value, ptr))
            }
            LLVMBuildStore(self.builder, value, alloc.1);
            return Ok(value);
        }
        if let Some(_) = variable.index {
            return Err(CodeGenError {
                kind: ErrorKind::Null,
                message: "Cannot set index of uninitialized list".to_string(),
                line: eval.line,
            }.into())
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

    pub unsafe fn gen_native(&mut self, name: Ident, args: Vec<P<Expr>>, var: bool, ret: Option<P<Expr>>) -> Result<LLVMValueRef> {
        let mut args_type = Vec::new();
        for arg in args.clone() {
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
                    _ => { // add array functionality later
                        return Err(CodeGenError {
                            kind: ErrorKind::InvalidArgs,
                            message: "Codegen error: function arg".to_string(),
                            line: args[0].line,
                        }.into())
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
                    _ => { // add array functionality later
                        return Err(CodeGenError {
                            kind: ErrorKind::InvalidArgs,
                            message: "Codegen error: function arg".to_string(),
                            line: args[0].line,
                        }.into())
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
        let scopes = self.scopes.clone();
        scopes.iter().for_each(|scope| {
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

        if let Some(index) = variable.index {
            let index = self.match_expr(index.into_inner())?;
            if LLVMTypeKind::LLVMIntegerTypeKind != LLVMGetTypeKind(LLVMTypeOf(index)) {
            }
            let element_type = LLVMGetElementType(var.0);
            //let mut indices = [index, LLVMConstInt(LLVMInt64TypeInContext(self.context), 0, 0)];
            let mut indices = [LLVMConstInt(LLVMInt64TypeInContext(self.context), 0, 0), index];
            let ptr = LLVMBuildInBoundsGEP2(self.builder, var.0, var.1, indices.as_mut_ptr(), 2, "\0".as_ptr() as *const _);
            Ok(LLVMBuildLoad2(self.builder, element_type, ptr, "\0".as_ptr() as *const _))
        } else {
            Ok(LLVMBuildLoad2(self.builder, var.0, var.1, "\0".as_ptr() as *const _))
        }
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
                    //let num_indices = LLVMGetArrayLength(LLVMTypeOf(arg));
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

        if let LLVMTypeKind::LLVMIntegerTypeKind = LLVMGetTypeKind(LLVMTypeOf(lhs)) {
            if let LLVMTypeKind::LLVMIntegerTypeKind = LLVMGetTypeKind(LLVMTypeOf(rhs)) {
                if LLVMGetIntTypeWidth(LLVMTypeOf(lhs)) == 64 {
                    if LLVMGetIntTypeWidth(LLVMTypeOf(rhs)) == 64 {
                        match binop {
                            BinOp::Add => {
                                return Ok(LLVMBuildAdd(self.builder, lhs, rhs, b"\0".as_ptr() as *const _))
                            }
                            BinOp::Sub => {
                                return Ok(LLVMBuildSub(self.builder, lhs, rhs, b"\0".as_ptr() as *const _))
                            }
                            BinOp::Mul => {
                                return Ok(LLVMBuildMul(self.builder, lhs, rhs, b"\0".as_ptr() as *const _))
                            }
                            BinOp::Shl => {
                                return Ok(LLVMBuildShl(self.builder, lhs, rhs, b"\0".as_ptr() as *const _))
                            }
                            BinOp::Shr => {
                                return Ok(LLVMBuildAShr(self.builder, lhs, rhs, b"\0".as_ptr() as *const _))
                            }
                            BinOp::BitOr => {
                                return Ok(LLVMBuildOr(self.builder, lhs, rhs, b"\0".as_ptr() as *const _))
                            }
                            BinOp::BitAnd => {
                                return Ok(LLVMBuildAnd(self.builder, lhs, rhs, b"\0".as_ptr() as *const _))
                            }
                            BinOp::BitXor => {
                                return Ok(LLVMBuildXor(self.builder, lhs, rhs, b"\0".as_ptr() as *const _))
                            }
                            BinOp::Eq => {
                                return Ok(LLVMBuildICmp(self.builder, llvm_sys::LLVMIntPredicate::LLVMIntEQ, lhs, rhs, b"\0".as_ptr() as *const _))
                            }
                            BinOp::Ne => {
                                return Ok(LLVMBuildICmp(self.builder, llvm_sys::LLVMIntPredicate::LLVMIntNE, lhs, rhs, b"\0".as_ptr() as *const _))
                            }
                            BinOp::Gr => {
                                return Ok(LLVMBuildICmp(self.builder, llvm_sys::LLVMIntPredicate::LLVMIntSGT, lhs, rhs, b"\0".as_ptr() as *const _))
                            }
                            BinOp::Ge => {
                                return Ok(LLVMBuildICmp(self.builder, llvm_sys::LLVMIntPredicate::LLVMIntSGE, lhs, rhs, b"\0".as_ptr() as *const _))
                            }
                            BinOp::Ls => {
                                return Ok(LLVMBuildICmp(self.builder, llvm_sys::LLVMIntPredicate::LLVMIntSLT, lhs, rhs, b"\0".as_ptr() as *const _))
                            }
                            BinOp::Le => {
                                return Ok(LLVMBuildICmp(self.builder, llvm_sys::LLVMIntPredicate::LLVMIntSLE, lhs, rhs, b"\0".as_ptr() as *const _))
                            }
                            _ => {
                                return Err(CodeGenError {
                                    kind: ErrorKind::Invalid,
                                    line: e_lhs.line,
                                    message: "Case not handled in binary operation".to_string(),
                                }.into())
                            }
                        }
                    }
                } else if LLVMGetIntTypeWidth(LLVMTypeOf(lhs)) == 1 {
                    if LLVMGetIntTypeWidth(LLVMTypeOf(rhs)) == 1 {
                        match binop {
                            BinOp::Or => {
                                return Ok(LLVMBuildOr(self.builder, lhs, rhs, b"\0".as_ptr() as *const _))
                            }
                            BinOp::And => {
                                return Ok(LLVMBuildAnd(self.builder, lhs, rhs, b"\0".as_ptr() as *const _))
                            }
                            BinOp::Eq => {
                                return Ok(LLVMBuildICmp(self.builder, llvm_sys::LLVMIntPredicate::LLVMIntEQ, lhs, rhs, b"\0".as_ptr() as *const _))
                            }
                            BinOp::Ne => {
                                return Ok(LLVMBuildICmp(self.builder, llvm_sys::LLVMIntPredicate::LLVMIntNE, lhs, rhs, b"\0".as_ptr() as *const _))
                            }
                            _ => {
                                return Err(CodeGenError {
                                    kind: ErrorKind::Invalid,
                                    line: e_lhs.line,
                                    message: "Case not handled in binary operation".to_string(),
                                }.into())
                            }
                        }
                    }
                    return Err(CodeGenError {
                        kind: ErrorKind::Invalid,
                        line: e_lhs.line,
                        message: "Mismatching type".to_string(),
                    }.into())
                } else {
                    return Err(CodeGenError {
                        kind: ErrorKind::Invalid,
                        line: e_lhs.line,
                        message: "Unexpected integer width".to_string(),
                    }.into())
                }
            }
            return Err(CodeGenError {
                kind: ErrorKind::MismatchedTypes,
                line: e_lhs.line,
                message: "Mismatched type".to_string(),
            }.into())
        } else if LLVMGetTypeKind(LLVMTypeOf(lhs)) == LLVMTypeKind::LLVMFloatTypeKind {
            if LLVMGetTypeKind(LLVMTypeOf(rhs)) == LLVMTypeKind::LLVMFloatTypeKind {
                match binop {
                    BinOp::Add => {
                        return Ok(LLVMBuildFAdd(self.builder, lhs, rhs, b"\0".as_ptr() as *const _))
                    }
                    BinOp::Sub => {
                        return Ok(LLVMBuildFSub(self.builder, lhs, rhs, b"\0".as_ptr() as *const _))
                    }
                    BinOp::Mul => {
                        return Ok(LLVMBuildFMul(self.builder, lhs, rhs, b"\0".as_ptr() as *const _))
                    }
                    BinOp::Div => {
                        return Ok(LLVMBuildFDiv(self.builder, lhs, rhs, b"\0".as_ptr() as *const _))
                    }
                    BinOp::Shl => {
                        return Ok(LLVMBuildShl(self.builder, lhs, rhs, b"\0".as_ptr() as *const _))
                    }
                    BinOp::Shr => {
                        return Ok(LLVMBuildAShr(self.builder, lhs, rhs, b"\0".as_ptr() as *const _))
                    }
                    BinOp::BitOr => {
                        return Ok(LLVMBuildOr(self.builder, lhs, rhs, b"\0".as_ptr() as *const _))
                    }
                    BinOp::BitAnd => {
                        return Ok(LLVMBuildAnd(self.builder, lhs, rhs, b"\0".as_ptr() as *const _))
                    }
                    BinOp::BitXor => {
                        return Ok(LLVMBuildXor(self.builder, lhs, rhs, b"\0".as_ptr() as *const _))
                    }
                    BinOp::Eq => {
                        return Ok(LLVMBuildFCmp(self.builder, llvm_sys::LLVMRealPredicate::LLVMRealOEQ, lhs, rhs, b"\0".as_ptr() as *const _))
                    }
                    BinOp::Ne => {
                        return Ok(LLVMBuildFCmp(self.builder, llvm_sys::LLVMRealPredicate::LLVMRealONE, lhs, rhs, b"\0".as_ptr() as *const _))
                    }
                    BinOp::Gr => {
                        return Ok(LLVMBuildFCmp(self.builder, llvm_sys::LLVMRealPredicate::LLVMRealOGT, lhs, rhs, b"\0".as_ptr() as *const _))
                    }
                    BinOp::Ge => {
                        return Ok(LLVMBuildFCmp(self.builder, llvm_sys::LLVMRealPredicate::LLVMRealOGE, lhs, rhs, b"\0".as_ptr() as *const _))
                    }
                    BinOp::Ls => {
                        return Ok(LLVMBuildFCmp(self.builder, llvm_sys::LLVMRealPredicate::LLVMRealOLT, lhs, rhs, b"\0".as_ptr() as *const _))
                    }
                    BinOp::Le => {
                        return Ok(LLVMBuildFCmp(self.builder, llvm_sys::LLVMRealPredicate::LLVMRealOLE, lhs, rhs, b"\0".as_ptr() as *const _))
                    }
                    _ => {
                        return Err(CodeGenError {
                            kind: ErrorKind::Invalid,
                            line: e_lhs.line,
                            message: "Case not handled in binary operation".to_string(),
                        }.into())
                    }
                }
            }
            return Err(CodeGenError {
                kind: ErrorKind::MismatchedTypes,
                line: e_lhs.line,
                message: "Mismatched type".to_string(),
            }.into())
        }
        Err(CodeGenError {
            kind: ErrorKind::MismatchedTypes,
            line: e_lhs.line,
            message: "Type not evaluated".to_string(),
        }.into())
    }

    pub unsafe fn process_array(&mut self, array: Vec<P<Expr>>) -> Result<Vec<LLVMValueRef>> {
        let mut array_values = Vec::new();
        for element in array {
            let element = element.into_inner();
            array_values.push(self.match_expr(element)?);
        }

        Ok(array_values)
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
