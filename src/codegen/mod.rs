mod ty;

use std::collections::HashMap;

use crate::parser::{ptr::P, Expr, ExprKind, BinOp, Literal, Variable, Block, Ident, Type as PType};
use anyhow::{Result, anyhow};
use llvm_sys::{prelude::*, core::*, transforms::{scalar::{LLVMAddReassociatePass, LLVMAddGVNPass, LLVMAddCFGSimplificationPass, LLVMAddTailCallEliminationPass, LLVMAddInstructionCombiningPass, LLVMAddDCEPass, LLVMAddDeadStoreEliminationPass}, util::LLVMAddPromoteMemoryToRegisterPass}, target::*, target_machine::{LLVMGetDefaultTargetTriple, LLVMGetTargetFromTriple, LLVMCreateTargetDataLayout, LLVMCreateTargetMachine, LLVMCodeModel, LLVMRelocMode, LLVMCodeGenOptLevel, LLVMTargetRef, LLVMGetFirstTarget, LLVMTargetMachineRef, LLVMGetHostCPUFeatures}};

use self::error::{CodeGenError, ErrorKind};
use ty::*;

pub struct CodeGen {
    pub exprs: Vec<P<Expr>>,
    context: LLVMContextRef,
    pub module: LLVMModuleRef,
    builder: LLVMBuilderRef,
    opt_passes: LLVMPassManagerRef,
    scopes: Vec<HashMap<String, (LLVMTypeRef, Type, LLVMValueRef)>>, // Name, (LLVMType, Type, Alloc)
    functions: HashMap<String, (LLVMTypeRef, Type, bool)>, // Name, (function_type, ret_type, has_var_args)
    structs: HashMap<String, (LLVMTypeRef, Type)>,
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
        LLVMAddTailCallEliminationPass(opt_passes);
        
        LLVMAddPromoteMemoryToRegisterPass(opt_passes);
        LLVMAddInstructionCombiningPass(opt_passes);
        LLVMAddDCEPass(opt_passes);
        LLVMAddDeadStoreEliminationPass(opt_passes);
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
            structs: HashMap::new(),
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
        LLVMPositionBuilderAtEnd(self.builder, bb);

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
        //LLVMPositionBuilderAtEnd(self.builder, bb);
        LLVMBuildRet(self.builder, LLVMConstInt(LLVMInt32TypeInContext(self.context), 0, 0));

        if self.optimize {
            LLVMRunFunctionPassManager(self.opt_passes, function);
        }

        Ok(function)
    }

    pub unsafe fn match_expr(&mut self, expr: Expr) -> Result<(Type, LLVMValueRef)> {
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
                if array.1.len() == 0 {
                    return Ok((Type::List(vec![]), LLVMConstArray(LLVMVoidType(), [].as_mut_ptr(), 0)))
                }
                Ok((array.0, LLVMConstArray(LLVMTypeOf(array.1[0]), array.1.as_mut_ptr(), array.1.len() as u32)))
            }
            ExprKind::Var(name) => {
                self.gen_variable(name, expr.line)
            }
            ExprKind::Set(variable, expr) => {
                self.gen_set(variable, expr.into_inner())
            }
            ExprKind::Add(value, variable) => {
                self.gen_add(value.into_inner(), variable)
            }
            ExprKind::Pop(variable) => {
                self.gen_pop(variable, expr.line)
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
            ExprKind::Struct(name, fields) => {
                self.gen_struct(name, fields, expr.line)
            }
            ExprKind::StructInit(name, fields) => {
                self.gen_struct_init(name, fields, expr.line)
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

    pub unsafe fn gen_function(&mut self, name: Ident, args: Vec<P<Expr>>, block: P<Block>, ret: Option<PType>) -> Result<(Type, LLVMValueRef)> {
        let mut f_args = Vec::with_capacity(args.len());
        for arg in args.clone() {
            if let ExprKind::Arg(_name, ty) = arg.kind.clone() {
                let ty: Type = ty.into();
                f_args.push(ty.into());
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
            let ty: Type = ty.into();
            ret_type = ty.into();
        } else {
            ret_type = LLVMVoidTypeInContext(self.context);
        }
        let function_type = LLVMFunctionType(ret_type, f_args.as_mut_ptr(), f_args.len() as u32, 0);

        let function = LLVMAddFunction(self.module, (name.0.clone() + "\0").as_ptr() as *const _, function_type);
        LLVMSetLinkage(function, llvm_sys::LLVMLinkage::LLVMExternalLinkage);

        let orig_bb = LLVMGetInsertBlock(self.builder);
        let bb = LLVMAppendBasicBlockInContext(self.context, function, "\0".as_ptr() as *const _);
        LLVMPositionBuilderAtEnd(self.builder, bb);

        self.scopes.push(HashMap::new());
        for (i, arg) in args.clone().iter().enumerate() {
            if let ExprKind::Arg(name, ty) = arg.kind.clone() {
                let ty: Type = ty.into();
                let arg_ty = ty.into();

                let alloca = LLVMBuildAlloca(self.builder, arg_ty, (name.0.clone() + "\0").as_ptr() as *const _);
                LLVMBuildStore(self.builder, LLVMGetParam(function, i as u32), alloca);
                // TODO replace with list logic
                self.scopes.last_mut().unwrap().insert(name.0.clone(), (arg_ty, Type::Pointer(P(Type::Null)), alloca));
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
            LLVMGetTypeKind(LLVMTypeOf(ret_val.1)) != LLVMGetTypeKind(ret_type) {
            return Err(CodeGenError {
                kind: ErrorKind::MismatchedTypes,
                message: format!("Mismatched return type on function {}", name.0),
                line: block.exprs.last().unwrap_or(&args[0]).line,
            }.into())
        }
        if ret_type != LLVMVoidTypeInContext(self.context) {
            LLVMBuildRet(self.builder, ret_val.1);
        } else {
            LLVMBuildRetVoid(self.builder);
        }

        if self.optimize {
            LLVMRunFunctionPassManager(self.opt_passes, function);
        }

        self.functions.insert(name.0, (function_type, ret_val.clone().0, false));

        LLVMPositionBuilderAtEnd(self.builder, orig_bb);

        Ok(ret_val)
    }
    /*pub unsafe fn gen_return(&mut self, expr: Option<P<Expr>>) -> Result<LLVMValueRef> {
        if let Some(expr) = expr {
            Ok(LLVMBuildRet(self.builder, self.match_expr(expr.into_inner())?))
        } else {
            Ok(LLVMBuildRetVoid(self.builder))
        }
    }*/
    pub unsafe fn gen_struct(&mut self, name: Ident, fields: Vec<(Ident, PType)>, line: u32) -> Result<(Type, LLVMValueRef)> {
        let mut v_fields: HashMap<String, (u32, Type)> = HashMap::new();
        let mut v_types: Vec<LLVMTypeRef> = Vec::with_capacity(fields.len());
        for (i, (ident, ty)) in fields.iter().enumerate() {
            if let PType::Struct(name) = ty {
                let ty = if let Some(t) = self.structs.get(&name.0) {
                    t.1.clone()
                } else {
                    return Err(CodeGenError {
                        kind: ErrorKind::InvalidType,
                        message: format!("struct \"{}\" not found", name.0),
                        line,
                    }.into())
                };
                v_fields.insert(ident.clone().0, (i as u32, ty.clone()));
                v_types.push(ty.into());
            } else {
                v_fields.insert(ident.clone().0, (i as u32, Type::from(ty.clone())));
                v_types.push(Type::from(ty.clone()).into());
            }
        }

        let ty = Type::Struct(name.0.clone(), v_fields.clone());
        let llvm_ty = LLVMStructCreateNamed(self.context, name.0.as_ptr() as *const _);
        LLVMStructSetBody(llvm_ty, v_types.as_mut_ptr(), v_types.len() as u32, 0);

        self.structs.insert(name.0.to_string(), (llvm_ty, ty));

        Ok((Type::Null, LLVMConstNull(LLVMVoidType())))
    }
    pub unsafe fn gen_struct_init(&mut self, name: Ident, fields: Vec<(Ident, Expr)>, line: u32) -> Result<(Type, LLVMValueRef)> {
        let mut v_fields: HashMap<String, (Type, LLVMValueRef)> = HashMap::new();
        for (ident, expr) in fields {
            v_fields.insert(ident.0, self.match_expr(expr)?);
        }
        let ty = if let Some(t) = self.structs.get(&name.0) { t } else {
            return Err(CodeGenError {
                kind: ErrorKind::InvalidType,
                message: format!("struct \"{}\" not found", name.0),
                line,
            }.into())
        };
        let fields = if let Type::Struct(_, value) = ty.1.clone() {
            value
        } else {
            panic!("Codegen Error: Struct Init");
        };

        let mut llvm_fields = vec![LLVMConstNull(LLVMVoidType()); fields.len()];
        for (key, (field_ty, value)) in v_fields {
            if !fields.contains_key(&key) {
                return Err(CodeGenError {
                    kind: ErrorKind::FieldNotFound,
                    message: format!("field \"{}\" not found", key),
                    line,
                }.into())
            }
            let field = fields.get(&key).unwrap();
            if !field.1.surface_eq(&field_ty) {
                return Err(CodeGenError {
                    kind: ErrorKind::MismatchedTypes,
                    message: format!("field \"{}\" does not match struct type", key),
                    line,
                }.into())
            }
            llvm_fields[field.0 as usize] = value;

            //let zero = LLVMConstInt(LLVMInt32Type(), 0, 0);
            //let field_ptr = LLVMBuildInBoundsGEP2(self.builder, ty.0, Pointer, [zero], 2, "\0".as_ptr() as *const _);
            //LLVMConstNamedStruct(ty.0, ConstantVals, v_fields.len() as u32);
        }
        let value = LLVMConstNamedStruct(ty.0, llvm_fields.as_mut_ptr(), llvm_fields.len() as u32);

        Ok((ty.1.clone(), value))
    }

    pub unsafe fn gen_if(&mut self, e_condition: Expr, block: Block, el: Option<P<Expr>>) -> Result<(Type, LLVMValueRef)> {
        let condition = self.match_expr(e_condition.clone())?;
        if condition.1.is_null() {
            return Err(CodeGenError {
                kind: ErrorKind::Null,
                message: "\"if\" cannot have null condition".to_string(),
                line: e_condition.line,
            }.into())
        }

        if let Some(expr) = el {
            //let zero = LLVMConstInt(LLVMInt1TypeInContext(self.context), 0, 0);
            //let condition = LLVMBuildICmp(self.builder, LLVMIntPredicate::LLVMIntNE,
            //                              condition.1, zero, "\0".as_ptr() as *const _);

            let function = LLVMGetBasicBlockParent(LLVMGetInsertBlock(self.builder));

            let thenBB = LLVMAppendBasicBlockInContext(self.context, function,
                                                       b"then\0".as_ptr() as *const _);
            let elseBB = LLVMCreateBasicBlockInContext(self.context,
                                                       b"else\0".as_ptr() as *const _);
            let mergeBB = LLVMCreateBasicBlockInContext(self.context,
                                                       b"ifcont\0".as_ptr() as *const _);
            LLVMBuildCondBr(self.builder, condition.1, thenBB, elseBB);

            LLVMPositionBuilderAtEnd(self.builder, thenBB);

            // then
            self.scopes.push(HashMap::new());
            let mut then = self.gen_block(block.clone())?;
            if then.1.is_null() {
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
            if else_v.1.is_null() {
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
            let phi = LLVMBuildPhi(self.builder, LLVMTypeOf(then.1), b"iftmp\0".as_ptr() as *const _);
            LLVMAddIncoming(phi, &mut then.1, &mut thenBB, 1);
            LLVMAddIncoming(phi, &mut else_v.1, &mut elseBB, 1);
            Ok((then.0, phi))
        } else {
            //let zero = LLVMConstReal(LLVMFloatTypeInContext(self.context), 0.);
            //let condition = LLVMBuildFCmp(self.builder, LLVMRealPredicate::LLVMRealONE,
            //                              condition.1, zero, "ifcond\0".as_ptr() as *const _);
            
            let function = LLVMGetBasicBlockParent(LLVMGetInsertBlock(self.builder));
            let thenBB = LLVMAppendBasicBlockInContext(self.context, function,
                                                       b"then\0".as_ptr() as *const _);
            let elseBB = LLVMCreateBasicBlockInContext(self.context,
                                                       b"else\0".as_ptr() as *const _);
            let mergeBB = LLVMCreateBasicBlockInContext(self.context,
                                                       b"ifcont\0".as_ptr() as *const _);
            LLVMBuildCondBr(self.builder, condition.1, thenBB, elseBB);

            LLVMPositionBuilderAtEnd(self.builder, thenBB);
            self.scopes.push(HashMap::new());
            let mut then = self.gen_block(block.clone())?;
            if then.1.is_null() {
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
            let phi = LLVMBuildPhi(self.builder, LLVMTypeOf(then.1), b"iftmp\0".as_ptr() as *const _);
            LLVMAddIncoming(phi, &mut then.1, &mut thenBB, 1);
            LLVMAddIncoming(phi, &mut LLVMConstNull(LLVMTypeOf(then.1)), &mut elseBB, 1);
            Ok(then)
        }

    }
    pub unsafe fn gen_while(&mut self, cond: Expr, block: Block) -> Result<(Type, LLVMValueRef)> {
        let function = LLVMGetBasicBlockParent(LLVMGetInsertBlock(self.builder));

        let v_condition = self.match_expr(cond.clone())?;
        if v_condition.1.is_null() {
            return Err(CodeGenError {
                kind: ErrorKind::Null,
                message: "\"while\" cannot have null condition".to_string(),
                line: cond.line,
            }.into())
        }

        let loopBB = LLVMAppendBasicBlockInContext(self.context, function, "\0".as_ptr() as *const _);
        let afterBB = LLVMCreateBasicBlockInContext(self.context, "\0".as_ptr() as *const _);

        LLVMBuildCondBr(self.builder, v_condition.1, loopBB, afterBB);
        LLVMPositionBuilderAtEnd(self.builder, loopBB);

        self.scopes.push(HashMap::new());
        self.gen_block(block.clone())?;
        let v_condition = self.match_expr(cond.clone())?;
        if v_condition.1.is_null() {
            return Err(CodeGenError {
                kind: ErrorKind::Null,
                message: "\"while\" cannot have null condition".to_string(),
                line: cond.line,
            }.into())
        }
        LLVMBuildCondBr(self.builder, v_condition.1, loopBB, afterBB);

        LLVMAppendExistingBasicBlock(function, afterBB);
        LLVMPositionBuilderAtEnd(self.builder, afterBB);

        Ok((Type::Null, LLVMConstNull(LLVMVoidType())))
    }

    pub unsafe fn gen_block(&mut self, block: Block) -> Result<(Type, LLVMValueRef)> {
        let mut errored = (false, String::new());
        let mut last = (Type::Null, LLVMConstNull(LLVMVoidType()));
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

    pub unsafe fn gen_set(&mut self, variable: Variable, eval: Expr) -> Result<(Type, LLVMValueRef)> {
        let scopes = self.scopes.clone();
        let scope = scopes.iter().fold(HashMap::new(), |sum, v| sum.into_iter().chain(v).collect());
        let value: LLVMValueRef;
        let ty: Type;
        if let ExprKind::Array(_) | ExprKind::Lit(Literal::Text(_)) = eval.clone().kind {
            // set {variable::1} to [1, 2, 3] will NOT work right now.
            // TODO fix that
            let mut v_array = if let ExprKind::Array(array) = eval.clone().kind {
                self.process_array(array)?
            } else if let ExprKind::Lit(Literal::Text(text)) = eval.clone().kind {
                self.process_text(text)?
            } else {
                panic!("Codegen error: found invalid list type");
            };
            let arr_type;
            if v_array.1.len() == 0 {
                arr_type = LLVMVoidType();
            } else {
                arr_type = LLVMTypeOf(v_array.1[0]);
            }
            let vec_ty = if let Some(s) = self.structs.get(&"Vec".to_string()) {
                s.clone()
            } else {
                let llvm_ty = LLVMStructCreateNamed(self.context, "Vec\0".as_ptr() as *const _);
                LLVMStructSetBody(llvm_ty, [LLVMPointerType(LLVMVoidType(), 0), LLVMInt64TypeInContext(self.context)].as_mut_ptr(), 2, 0);
                let ty = Type::Struct("Vec".to_string(),
                        HashMap::from([ ("data".into(), (0, Type::Pointer(P(Type::Null)))), ("len".into(), (1, Type::Integer)) ]));
                self.structs.insert("Vec".to_string(),
                    (llvm_ty, ty.clone()));
                (llvm_ty, ty)
            };

            let zero = LLVMConstInt(LLVMInt32Type(), 0, 0);
            if scope.get(&variable.name.0).is_some() {
                // set variable that already exists
                let alloc = scope.get(&variable.name.0).unwrap();

                let element_type = LLVMGetElementType(alloc.0);
                let mut indices = [zero, zero];
                let ptr = LLVMBuildInBoundsGEP2(self.builder, vec_ty.0, alloc.2, indices.as_mut_ptr(), 2, "\0".as_ptr() as *const _);
                let malloc = LLVMBuildLoad2(self.builder, LLVMPointerType(element_type, 0), ptr, "\0".as_ptr() as *const _);

                let value = LLVMConstArray(arr_type, v_array.1.as_mut_ptr(), v_array.1.len() as u32);
                LLVMBuildStore(self.builder, value, malloc);
                return Ok((v_array.0, value));
            }
            let alloc = LLVMBuildAlloca(self.builder, vec_ty.0, (variable.name.0.clone() + "\0").as_ptr() as *const _);
            if v_array.1.len() == 0 {
                self.scopes.last_mut().unwrap().insert(variable.name.0,
                                                       (LLVMArrayType(arr_type, v_array.1.len() as u32), Type::List(vec![P(arr_type.into()); v_array.1.len()]), alloc));
                return Ok((v_array.0, alloc));
            }

            let (malloc_ty, malloc) = self.get_or_create_function("malloc".to_string(), vec![LLVMInt64TypeInContext(self.context)], LLVMPointerType(LLVMVoidType(), 0));

            // call malloc
            let malloc_amount = if v_array.0.get_element() == Type::Char {
                v_array.1.len() as u64
            } else {
                v_array.1.len() as u64 * 8
            };
            let ret = LLVMBuildCall2(self.builder, malloc_ty, malloc, [LLVMConstInt(LLVMInt64Type(), malloc_amount, 0)].as_mut_ptr(), 1, "\0".as_ptr() as *const _);

            // get length part of Vec
            let mut indices = [zero, LLVMConstInt(LLVMInt32TypeInContext(self.context), 1, 0)];
            let ptr = LLVMBuildInBoundsGEP2(self.builder, vec_ty.0, alloc, indices.as_mut_ptr(), 2, "\0".as_ptr() as *const _);
            LLVMBuildStore(self.builder, LLVMConstInt(LLVMInt64Type(), v_array.1.len() as u64, 0), ptr);
            // store pointer to malloc call in Vec
            indices[1] = LLVMConstInt(LLVMInt32TypeInContext(self.context), 0, 0);
            let ptr = LLVMBuildInBoundsGEP2(self.builder, vec_ty.0, alloc, indices.as_mut_ptr(), 2, "\0".as_ptr() as *const _);
            LLVMBuildStore(self.builder, ret, ptr);
            let value = LLVMConstArray(arr_type, v_array.1.as_mut_ptr(), v_array.1.len() as u32);
            LLVMBuildStore(self.builder, value, ret);
            /*for (i, value) in v_array.1.iter().enumerate() {
                let mut indices = [LLVMConstInt(LLVMInt64Type(), i as u64, 0)];
                let ptr = LLVMBuildInBoundsGEP2(self.builder, arr_type, ret, indices.as_mut_ptr(), 1, "\0".as_ptr() as *const _);
                LLVMBuildStore(self.builder, *value, ptr);
            }*/
            if let Type::List(_) = v_array.0 {
                self.scopes.last_mut().unwrap().insert(variable.name.0,
                                                       (LLVMArrayType(arr_type, v_array.1.len() as u32), Type::List(vec![P(arr_type.into()); v_array.1.len()]), alloc));
            } else if let Type::Text(len) = v_array.0 {
                self.scopes.last_mut().unwrap().insert(variable.name.0,
                                                       (LLVMArrayType(arr_type, v_array.1.len() as u32), Type::Text(len), alloc));
            }
            return Ok((v_array.0, alloc));
        } else {
            let expr = self.match_expr(eval.clone())?;
            ty = expr.0;
            value = expr.1;
        }

        if scope.get(&variable.name.0).is_some() {
            let alloc = scope.get(&variable.name.0).unwrap();
            if let Some(indices) = variable.index {
                if let ExprKind::Ident(i) = indices[0].clone().kind {
                    if let Type::Struct(name, fields) = alloc.1.clone() {
                        let mut field = i.0;
                        let zero = LLVMConstInt(LLVMInt32Type(), 0, 0);
                        let mut v_indices = Vec::with_capacity(indices.len());
                        v_indices.push(zero);
                        let mut v_fields = fields.clone();
                        let mut next_type = alloc.1.clone();
                        let mut last_i = 0;
                        for (i, index) in indices.iter().enumerate() {
                            if next_type.surface_eq(&Type::Struct(name.clone(), fields.clone())) {
                                let v_index = if let Some(v) = v_fields.get(&field) {
                                    v
                                } else {
                                    return Err(CodeGenError {
                                        kind: ErrorKind::FieldNotFound,
                                        message: format!("Field \"{}\" not found in struct {}", field, name),
                                        line: eval.line,
                                    }.into())
                                };
                                println!("{}", field);
                                v_indices.push(LLVMConstInt(LLVMInt32Type(), v_index.0 as u64, 0));
                                last_i = v_index.0;
                                if let Type::Struct(name, fields) = &v_index.1 {
                                    if i < indices.len()-1 {
                                        if let ExprKind::Ident(i) = &indices[i+1].kind {
                                            field = i.0.clone();
                                        }
                                    }
                                    next_type = Type::Struct(name.clone(), fields.clone()); 
                                    v_fields = fields.clone();
                                }
                            }
                        }
                        let mut ty = next_type.clone();
                        if let Type::Struct(_, fields) = next_type {
                            let value = fields.iter().find(|(_, (i, _))| *i == last_i).unwrap();
                            ty = value.1.1.clone();
                        }
                        println!("{:?}", v_indices);
                        /*let index = if let Some(v) = v_fields.get(&field) {
                            v
                        } else {
                            return Err(CodeGenError {
                                kind: ErrorKind::FieldNotFound,
                                message: format!("Field \"{}\" not found in struct {}", field, name),
                                line: eval.line,
                            }.into())
                        };*/
                        let ptr = LLVMBuildInBoundsGEP2(self.builder, alloc.0, alloc.2, v_indices.as_mut_ptr(), v_indices.len() as u32, "\0".as_ptr() as *const _);
                        /*let mut types = Vec::with_capacity(v_fields.len());
                        LLVMGetStructElementTypes(alloc.0, types.as_mut_ptr());
                        types.set_len(v_fields.len());*/
                        let value = LLVMBuildStore(self.builder, value, ptr); 
                        return Ok((ty, value))
                    } else {
                        return Err(CodeGenError {
                            kind: ErrorKind::MismatchedTypes,
                            message: "Exprected struct type to set a field".to_string(),
                            line: eval.line,
                        }.into())
                    }
                }

                let index = self.match_expr(indices[0].clone())?;
                let mut indices = [LLVMConstInt(LLVMInt64TypeInContext(self.context), 0, 0), index.1];
                let ptr = LLVMBuildInBoundsGEP2(self.builder, alloc.0, alloc.2, indices.as_mut_ptr(), 2, "\0".as_ptr() as *const _);
                return Ok((ty, LLVMBuildStore(self.builder, value, ptr)))
            }
            LLVMBuildStore(self.builder, value, alloc.2);
            return Ok((ty, value));
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
        self.scopes.last_mut().unwrap().insert(name, (LLVMTypeOf(value), ty.clone(), alloc));

        Ok((ty, value))
    }
    pub unsafe fn gen_add(&mut self, e_value: Expr, variable: Variable) -> Result<(Type, LLVMValueRef)> {
        let value: LLVMValueRef;
        let ty: Type;
        let mut vec = None;
        let scopes = self.scopes.clone();
        scopes.iter().for_each(|scope| {
            match scope.get(&variable.name.0) {
                Some(v) => {
                    vec = Some(v)
                }
                None => {}
            };
        });
        let vec = match vec {
            Some(v) => v,
            None => return Err(CodeGenError {
                kind: ErrorKind::NotInScope,
                message: "Variable not in scope".to_string(),
                line: e_value.line,
            }.into())
        };
        if vec.0.is_null() {
            return Err(CodeGenError {
                kind: ErrorKind::NotInScope,
                message: "Variable not in scope".to_string(),
                line: e_value.line,
            }.into())
        }
        //if let ExprKind::Array(_array) = e_value.kind {
        //    todo!();
        //} else {
            let expr = self.match_expr(e_value.clone())?;
            ty = expr.0;
            value = expr.1;
        //}
        let vec_ty = if let Some(s) = self.structs.get("Vec") {
            s.clone()
        } else {
            return Err(CodeGenError {
                kind: ErrorKind::NotInScope,
                message: "Variable not in scope".to_string(),
                line: e_value.line,
            }.into())
        };
        
        let mut first_tys = Vec::with_capacity(LLVMCountStructElementTypes(vec_ty.0) as usize);
        LLVMGetStructElementTypes(vec_ty.0, first_tys.as_mut_ptr());
        let mut second_tys = Vec::with_capacity(LLVMCountStructElementTypes(vec.0) as usize);
        LLVMGetStructElementTypes(vec.0, second_tys.as_mut_ptr());

        if first_tys != second_tys {
            panic!("Codegen error: add (vec type)");
        }
        let zero = LLVMConstInt(LLVMInt32TypeInContext(self.context), 0, 0);
        let s_array_ptr = LLVMBuildInBoundsGEP2(self.builder, vec_ty.0, vec.2, [zero, zero].as_mut_ptr(), 2, "\0".as_ptr() as *const _);
        let size_ptr = LLVMBuildInBoundsGEP2(self.builder, vec_ty.0, vec.2, [zero, LLVMConstInt(LLVMInt32Type(), 1, 0)].as_mut_ptr(), 2, "\0".as_ptr() as *const _);
        let array_ptr = LLVMBuildLoad2(self.builder, LLVMPointerType(LLVMVoidType(), 0), s_array_ptr, "\0".as_ptr() as *const _);
        let size = LLVMBuildLoad2(self.builder, LLVMInt64Type(), size_ptr, "\0".as_ptr() as *const _);

        // add to array 
        let void_ptr = LLVMPointerType(LLVMVoidTypeInContext(self.context), 0);
        let (alloc_ty, alloc);
        if vec.1 == Type::List(vec![]) {
            // add to empty array
            (alloc_ty, alloc) = self.get_or_create_function("malloc".to_string(), vec![LLVMInt64TypeInContext(self.context)], LLVMPointerType(LLVMVoidType(), 0));
            let eight = LLVMConstInt(LLVMInt64Type(), 8, 0);
            let one = LLVMConstInt(LLVMInt64Type(), 1, 0);
            let ptr = LLVMBuildCall2(self.builder, alloc_ty, alloc, [eight].as_mut_ptr(), 1, "\0".as_ptr() as *const _);
            LLVMBuildStore(self.builder, one, size_ptr);
            LLVMBuildStore(self.builder, ptr, s_array_ptr);

            // set last index to value
            let mut indices = [zero];
            let last_ptr = LLVMBuildInBoundsGEP2(self.builder, LLVMInt64TypeInContext(self.context), ptr, indices.as_mut_ptr(), 1, "\0".as_ptr() as *const _);
            LLVMBuildStore(self.builder, value, last_ptr);

            let mut ret = None;
            scopes.iter().enumerate().rev().for_each(|(i, scope)| {
                if scope.contains_key(&variable.name.0) {
                    let len = LLVMGetArrayLength(vec.0) + 1;
                    let el_ty = LLVMGetElementType(vec.0);
                    let vec_ty = LLVMArrayType(LLVMTypeOf(value), len);
                    if let Type::List(mut v) = vec.1.clone() {
                        v.push(P(el_ty.into()));
                        *self.scopes.get_mut(i).unwrap().get_mut(&variable.name.0).unwrap() = (vec_ty, Type::List(v), vec.2);
                    } else {
                        ret = Some(Err(CodeGenError {
                            kind: ErrorKind::NotInScope,
                            message: "Expected list variable".to_string(),
                            line: e_value.line,
                        }.into()));
                    }
                }
            });
            if let Some(e) = ret {
                return e;
            }
        } else if let Type::List(_) = vec.1 {
            (alloc_ty, alloc) = self.get_or_create_function("realloc".to_string(), vec![void_ptr, LLVMInt64TypeInContext(self.context)], LLVMPointerType(LLVMVoidType(), 0));

            // type check
            if !vec.1.element_eq(&ty) {
                return Err(CodeGenError {
                    kind: ErrorKind::MismatchedTypes,
                    message: "Mismatching element type".to_string(),
                    line: e_value.line,
                }.into())
            }

            // add to list
            let last_index = LLVMBuildNSWAdd(self.builder, size, LLVMConstInt(LLVMInt64Type(), 1, 0), "\0".as_ptr() as *const _);
            let new_size = if vec.1.element_eq(&Type::Char) {
                last_index
            } else {
                LLVMBuildNSWMul(self.builder, last_index, LLVMConstInt(LLVMInt64Type(), 8, 0), "\0".as_ptr() as *const _)
            };
            let new_ptr = LLVMBuildCall2(self.builder, alloc_ty, alloc, [array_ptr, new_size].as_mut_ptr(), 2, "\0".as_ptr() as *const _);
            LLVMBuildStore(self.builder, last_index, size_ptr);
            LLVMBuildStore(self.builder, new_ptr, s_array_ptr);

            let mut indices = [size];
            let last_ptr = LLVMBuildInBoundsGEP2(self.builder, ty.clone().into(), new_ptr, indices.as_mut_ptr(), 1, "\0".as_ptr() as *const _);
            LLVMBuildStore(self.builder, value, last_ptr);
            // set new size
            scopes.iter().enumerate().rev().for_each(|(i, scope)| {
                if scope.contains_key(&variable.name.0) {
                    let len = LLVMGetArrayLength(vec.0) + 1;
                    let el_ty = LLVMGetElementType(vec.0);
                    let vec_ty = LLVMArrayType(el_ty, len);
                    *self.scopes.get_mut(i).unwrap().get_mut(&variable.name.0).unwrap() = (vec_ty, vec.1.clone(), vec.2);
                }
            });
        } else {
            (alloc_ty, alloc) = self.get_or_create_function("realloc".to_string(), vec![void_ptr, LLVMInt64TypeInContext(self.context)], LLVMPointerType(LLVMVoidType(), 0));

            if !vec.1.surface_eq(&ty) {
                return Err(CodeGenError {
                    kind: ErrorKind::MismatchedTypes,
                    message: "Mismatching element type".to_string(),
                    line: e_value.line,
                }.into())
            }
            let other_len = if let Type::Text(len) = ty {
                len
            } else {
                panic!("Codegen error: Mismatching types in \"add\" of text types");
            };

            let new_size = LLVMBuildNSWAdd(self.builder, size, LLVMConstInt(LLVMInt64Type(), other_len as u64, 0), "\0".as_ptr() as *const _);
            let new_ptr = LLVMBuildCall2(self.builder, alloc_ty, alloc, [array_ptr, new_size].as_mut_ptr(), 2, "\0".as_ptr() as *const _);
            LLVMBuildStore(self.builder, new_size, size_ptr);
            LLVMBuildStore(self.builder, new_ptr, s_array_ptr);

            let (memcpy_ty, memcpy) = self.get_or_create_function("llvm.memcpy.p0.p0.i32".to_string(),
                vec![void_ptr, void_ptr, LLVMInt32Type(), LLVMInt1Type()], LLVMVoidType());

            let cpy_ptr = LLVMBuildGEP2(self.builder, LLVMInt8Type(), new_ptr, [size].as_mut_ptr(), 1, "\0".as_ptr() as *const _);
            LLVMBuildCall2(self.builder, memcpy_ty, memcpy, [cpy_ptr, value, LLVMConstInt(LLVMInt32Type(), other_len as u64, 0), LLVMConstInt(LLVMInt1Type(), 0, 0)].as_mut_ptr(), 4, "\0".as_ptr() as *const _);

            scopes.iter().enumerate().rev().for_each(|(i, scope)| {
                if scope.contains_key(&variable.name.0) {
                    let len = LLVMGetArrayLength(vec.0) + other_len;
                    let el_ty = LLVMGetElementType(vec.0);
                    let vec_ty = LLVMArrayType(el_ty, len);
                    *self.scopes.get_mut(i).unwrap().get_mut(&variable.name.0).unwrap() = (vec_ty, vec.1.clone(), vec.2);
                }
            });
        }

        Ok((ty, value))
    }
    pub unsafe fn gen_pop(&mut self, variable: Variable, line: u32) -> Result<(Type, LLVMValueRef)> {
        let mut vec = None;
        let scopes = self.scopes.clone();
        scopes.iter().for_each(|scope| {
            match scope.get(&variable.name.0) {
                Some(v) => {
                    vec = Some(v)
                }
                None => {}
            };
        });
        let vec = match vec {
            Some(v) => v,
            None => return Err(CodeGenError {
                kind: ErrorKind::NotInScope,
                message: "Variable not in scope".to_string(),
                line,
            }.into())
        };
        if vec.0.is_null() {
            return Err(CodeGenError {
                kind: ErrorKind::NotInScope,
                message: "Variable not in scope".to_string(),
                line,
            }.into())
        }
        if !vec.1.is_list() {
            return Err(CodeGenError {
                kind: ErrorKind::MismatchedTypes,
                message: "Expected list type in pop".to_string(),
                line,
            }.into())
        }
        let vec_ty = if let Some(s) = self.structs.get("Vec") {
            s.clone()
        } else {
            return Err(CodeGenError {
                kind: ErrorKind::NotInScope,
                message: "Variable not in scope".to_string(),
                line,
            }.into())
        };
        
        let mut first_tys = Vec::with_capacity(LLVMCountStructElementTypes(vec_ty.0) as usize);
        LLVMGetStructElementTypes(vec_ty.0, first_tys.as_mut_ptr());
        let mut second_tys = Vec::with_capacity(LLVMCountStructElementTypes(vec.0) as usize);
        LLVMGetStructElementTypes(vec.0, second_tys.as_mut_ptr());

        if first_tys != second_tys {
            panic!("Codegen error: add (vec type)");
        }
        let zero = LLVMConstInt(LLVMInt32TypeInContext(self.context), 0, 0);
        let s_array_ptr = LLVMBuildInBoundsGEP2(self.builder, vec_ty.0, vec.2, [zero].as_mut_ptr(), 1, "\0".as_ptr() as *const _);
        let size_ptr = LLVMBuildInBoundsGEP2(self.builder, vec_ty.0, vec.2, [zero, LLVMConstInt(LLVMInt32Type(), 1, 0)].as_mut_ptr(), 2, "\0".as_ptr() as *const _);
        let array_ptr = LLVMBuildLoad2(self.builder, LLVMPointerType(LLVMVoidType(), 0), s_array_ptr, "\0".as_ptr() as *const _);
        let size = LLVMBuildLoad2(self.builder, LLVMInt64Type(), size_ptr, "\0".as_ptr() as *const _);

        // add to array 
        // set last index to value

        let void_ptr = LLVMPointerType(LLVMVoidTypeInContext(self.context), 0);
        let (alloc_ty, alloc);
        if vec.1 == Type::List(vec![]) {
            return Err(CodeGenError {
                kind: ErrorKind::MismatchedTypes,
                message: "Expected non-empty list".to_string(),
                line,
            }.into());
        } else {
            (alloc_ty, alloc) = self.get_or_create_function("realloc".to_string(), vec![void_ptr, LLVMInt64TypeInContext(self.context)], LLVMPointerType(LLVMVoidType(), 0));
            // add to list
            let last_index = LLVMBuildNSWSub(self.builder, size, LLVMConstInt(LLVMInt64Type(), 1, 0), "\0".as_ptr() as *const _);
            let new_size = if vec.1.element_eq(&Type::Char) {
                last_index
            } else {
                LLVMBuildNSWMul(self.builder, last_index, LLVMConstInt(LLVMInt64Type(), 8, 0), "\0".as_ptr() as *const _)
            };

            let mut indices = [last_index];
            let last_ptr = LLVMBuildInBoundsGEP2(self.builder, vec.1.get_element().into(),
                            array_ptr, indices.as_mut_ptr(), 1, "\0".as_ptr() as *const _);
            let last_value = LLVMBuildLoad2(self.builder, vec.1.get_element().into(), last_ptr, "\0".as_ptr() as *const _);
            LLVMBuildStore(self.builder, zero, last_ptr);

            let new_ptr = LLVMBuildCall2(self.builder, alloc_ty, alloc, [array_ptr, new_size].as_mut_ptr(), 2, "\0".as_ptr() as *const _);
            LLVMBuildStore(self.builder, last_index, size_ptr);
            LLVMBuildStore(self.builder, new_ptr, s_array_ptr);

            // set new size
            scopes.iter().enumerate().rev().for_each(|(i, scope)| {
                if scope.contains_key(&variable.name.0) {
                    let len = LLVMGetArrayLength(vec.0) + 1;
                    let el_ty = LLVMGetElementType(vec.0);
                    let vec_ty = LLVMArrayType(el_ty, len);
                    *self.scopes.get_mut(i).unwrap().get_mut(&variable.name.0).unwrap() = (vec_ty, vec.1.clone(), vec.2);
                }
            });
            Ok((vec.1.clone(), last_value))
        }
    }

    pub unsafe fn gen_native(&mut self, name: Ident, args: Vec<P<Expr>>, var: bool, ret: Option<P<Expr>>) -> Result<(Type, LLVMValueRef)> {
        let mut args_type = Vec::new();
        for arg in args.clone() {
            let arg = arg.into_inner();
            if let ExprKind::Arg(ident, t) = arg.kind {
                let tt;
                match t {
                    PType::Text => {
                        tt = LLVMPointerType(LLVMInt8TypeInContext(self.context), 0);
                    }
                    PType::Number => {
                        tt = LLVMFloatType();
                    }
                    PType::Integer => {
                        tt = LLVMInt64Type();
                    }
                    PType::Boolean => {
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
                    PType::Text => {
                        ret_type = LLVMPointerType(LLVMInt8TypeInContext(self.context), 0);
                    }
                    PType::Number => {
                        ret_type = LLVMFloatType();
                    }
                    PType::Integer => {
                        ret_type = LLVMInt64Type();
                    }
                    PType::Boolean => {
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
        self.functions.insert(name.0, (function_type, Type::from(ret_type), var));

        Ok((Type::Null, LLVMConstNull(LLVMVoidType())))
    }

    pub unsafe fn gen_variable(&mut self, variable: Variable, line: u32) -> Result<(Type, LLVMValueRef)> {
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

        if let Some(indices) = variable.index {
            let zero = LLVMConstInt(LLVMInt32Type(), 0, 0);
            if let ExprKind::Ident(i) = indices[0].clone().kind {
                if let Type::Struct(name, fields) = var.1.clone() {
                    let mut field = i.0;
                    let zero = LLVMConstInt(LLVMInt32Type(), 0, 0);
                    let mut v_indices = Vec::with_capacity(indices.len());
                    v_indices.push(zero);
                    let mut v_fields = fields.clone();
                    let mut next_type = var.1.clone();
                    let mut last_i = 0;
                    for (i, index) in indices.iter().enumerate() {
                        if next_type.surface_eq(&Type::Struct(name.clone(), fields.clone())) {
                            let v_index = if let Some(v) = v_fields.get(&field) {
                                v
                            } else {
                                return Err(CodeGenError {
                                    kind: ErrorKind::FieldNotFound,
                                    message: format!("Field \"{}\" not found in struct {}", field, name),
                                    line,
                                }.into())
                            };
                            println!("{}", field);
                            v_indices.push(LLVMConstInt(LLVMInt32Type(), v_index.0 as u64, 0));
                            last_i = v_index.0;
                            if let Type::Struct(name, fields) = &v_index.1 {
                                if i < indices.len()-1 {
                                    if let ExprKind::Ident(i) = &indices[i+1].kind {
                                        field = i.0.clone();
                                    }
                                }
                                next_type = Type::Struct(name.clone(), fields.clone()); 
                                v_fields = fields.clone();
                            }
                        }
                    }
                    let mut ty = next_type.clone();
                    if let Type::Struct(_, fields) = next_type {
                        let value = fields.iter().find(|(_, (i, _))| *i == last_i).unwrap();
                        ty = value.1.1.clone();
                    }
                    println!("{:?}", v_indices);
                    let ptr = LLVMBuildInBoundsGEP2(self.builder, var.0, var.2, v_indices.as_mut_ptr(), v_indices.len() as u32, "\0".as_ptr() as *const _);
                    /*let mut types = Vec::with_capacity(v_fields.len());
                    LLVMGetStructElementTypes(alloc.0, types.as_mut_ptr());
                    types.set_len(v_fields.len());*/
                    let value = LLVMBuildLoad2(self.builder, ty.clone().into(), ptr, "\0".as_ptr() as *const _); 
                    return Ok((ty, value))
                } else {
                    return Err(CodeGenError {
                        kind: ErrorKind::MismatchedTypes,
                        message: "Exprected struct type to set a field".to_string(),
                        line,
                    }.into())
                }
            }

            //let index = self.match_expr(index.into_inner())?;
            let mut v_indices = Vec::with_capacity(indices.len());
            for index in indices {
                v_indices.push(self.match_expr(index)?.1);
            }
            let element_type = LLVMGetElementType(var.0);
            //let mut indices = [index, LLVMConstInt(LLVMInt64TypeInContext(self.context), 0, 0)];
            let mut indices = [zero, zero];
            let vec_ty = if let Some(s) = self.structs.get("Vec") {
                s
            } else {
                panic!("Codegen error: Vec struct not created");
            }.clone();
            let ptr = LLVMBuildInBoundsGEP2(self.builder, vec_ty.0, var.2, indices.as_mut_ptr(), 2, "\0".as_ptr() as *const _);
            let malloc = LLVMBuildLoad2(self.builder, LLVMPointerType(element_type, 0), ptr, "\0".as_ptr() as *const _);
            //let mut indices = [index.1];
            let ptr = LLVMBuildInBoundsGEP2(self.builder, element_type, malloc, indices.as_mut_ptr(), 1, "\0".as_ptr() as *const _);
            let load = LLVMBuildLoad2(self.builder, element_type, ptr, "\0".as_ptr() as *const _);
            // TODO var.1 isnt inner element type, fix later
            Ok((var.1.get_element(), load))
        } else if let Type::Text(_) = var.1 {
            let zero = LLVMConstInt(LLVMInt32Type(), 0, 0);
            let vec_ty = if let Some(s) = self.structs.get("Vec") {
                s
            } else {
                panic!("Codegen error: Vec struct not created");
            }.clone();
            let mut indices = [zero, zero];
            let ptr = LLVMBuildInBoundsGEP2(self.builder, vec_ty.0, var.2, indices.as_mut_ptr(), 2, "\0".as_ptr() as *const _);
            let load = LLVMBuildLoad2(self.builder, LLVMPointerType(LLVMInt8Type(), 0), ptr, "\0".as_ptr() as *const _);
            Ok((var.1.clone(), load))
        } else {
            Ok((var.1.clone(), LLVMBuildLoad2(self.builder, var.0, var.2, "\0".as_ptr() as *const _)))
        }
    }

    pub unsafe fn gen_call(&mut self, ident: Ident, args: Vec<P<Expr>>, line: u32) -> Result<(Type, LLVMValueRef)> {
        let function = LLVMGetNamedFunction(self.module, (ident.0.clone() + "\0").as_ptr() as *const _);
        let function_type = match self.functions.get(&ident.0) {
            Some(t) => t,
            None => {
                return Err(CodeGenError {
                    kind: ErrorKind::NotInScope,
                    message: "Function not found".to_string(),
                    line,
                }.into())
            }
        }.clone();
        if function.is_null() {
            return Err(CodeGenError {
                kind: ErrorKind::NotInScope,
                message: "Function not found".to_string(),
                line,
            }.into())
        }
        let param_num = LLVMCountParamTypes(function_type.0);

        let mut arg_types = Vec::with_capacity(param_num as usize);
        if !function_type.2 {
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
            let mut arg = self.match_expr(arg.clone())?; // hang
            if arg.1.is_null() {
                return Err(CodeGenError {
                    kind: ErrorKind::Null,
                    message: "Cannot have null argument".to_string(),
                    line,
                }.into())
            }
            if !function_type.2 {
                if let Type::List(ref _inner) = arg.0 {
                    let element_type = LLVMGetElementType(LLVMTypeOf(arg.1));
                    //let num_indices = LLVMGetArrayLength(LLVMTypeOf(arg));
                    let mut int = LLVMConstInt(LLVMInt32Type(), 0, 0);
                    arg.1 = LLVMBuildInBoundsGEP2(self.builder, LLVMPointerType(element_type, 0), arg.1, &mut int, 0, "\0".as_ptr() as *const _);
                }

                if LLVMTypeOf(arg.1) != arg_types[i] {
                    return Err(CodeGenError {
                        kind: ErrorKind::MismatchedTypes,
                        message: format!("Argument {} has wrong type", i),
                        line,
                    }.into())
                }
            }
            argsV.push(arg);
        }

        let mut argsV: Vec<LLVMValueRef> = argsV.iter().map(|(_, v)| { *v }).collect();
        let ret = LLVMBuildCall2(self.builder, function_type.0, function,
            argsV.as_mut_ptr(), argsV.len() as u32, "\0".as_ptr() as *const _);
        Ok((function_type.1, ret))
    }
    
    pub unsafe fn gen_binary(&mut self, binop: BinOp, e_lhs: Expr, e_rhs: Expr) -> Result<(Type, LLVMValueRef)> {
        let lhs = self.match_expr(e_lhs.clone())?;
        let rhs = self.match_expr(e_rhs)?;
        if lhs.0 == Type::Null || rhs.0 == Type::Null {
            return Err(CodeGenError {
                kind: ErrorKind::Null,
                line: e_lhs.line,
                message: "Found null value in binary operation".to_string(),
            }.into());
        }

        if lhs.0 == Type::Integer && rhs.0 == Type::Integer {
            match binop {
                BinOp::Add => {
                    return Ok((lhs.0, LLVMBuildAdd(self.builder, lhs.1, rhs.1, b"\0".as_ptr() as *const _)))
                }
                BinOp::Sub => {
                    return Ok((lhs.0, LLVMBuildSub(self.builder, lhs.1, rhs.1, b"\0".as_ptr() as *const _)))
                }
                BinOp::Mul => {
                    return Ok((lhs.0, LLVMBuildMul(self.builder, lhs.1, rhs.1, b"\0".as_ptr() as *const _)))
                }
                BinOp::Shl => {
                    return Ok((lhs.0, LLVMBuildShl(self.builder, lhs.1, rhs.1, b"\0".as_ptr() as *const _)))
                }
                BinOp::Shr => {
                    return Ok((lhs.0, LLVMBuildAShr(self.builder, lhs.1, rhs.1, b"\0".as_ptr() as *const _)))
                }
                BinOp::BitOr => {
                    return Ok((lhs.0, LLVMBuildOr(self.builder, lhs.1, rhs.1, b"\0".as_ptr() as *const _)))
                }
                BinOp::BitAnd => {
                    return Ok((lhs.0, LLVMBuildAnd(self.builder, lhs.1, rhs.1, b"\0".as_ptr() as *const _)))
                }
                BinOp::BitXor => {
                    return Ok((lhs.0, LLVMBuildXor(self.builder, lhs.1, rhs.1, b"\0".as_ptr() as *const _)))
                }
                BinOp::Eq => {
                    return Ok((lhs.0, LLVMBuildICmp(self.builder, llvm_sys::LLVMIntPredicate::LLVMIntEQ, lhs.1, rhs.1, b"\0".as_ptr() as *const _)))
                }
                BinOp::Ne => {
                    return Ok((lhs.0, LLVMBuildICmp(self.builder, llvm_sys::LLVMIntPredicate::LLVMIntNE, lhs.1, rhs.1, b"\0".as_ptr() as *const _)))
                }
                BinOp::Gr => {
                    return Ok((lhs.0, LLVMBuildICmp(self.builder, llvm_sys::LLVMIntPredicate::LLVMIntSGT, lhs.1, rhs.1, b"\0".as_ptr() as *const _)))
                }
                BinOp::Ge => {
                    return Ok((lhs.0, LLVMBuildICmp(self.builder, llvm_sys::LLVMIntPredicate::LLVMIntSGE, lhs.1, rhs.1, b"\0".as_ptr() as *const _)))
                }
                BinOp::Ls => {
                    return Ok((lhs.0, LLVMBuildICmp(self.builder, llvm_sys::LLVMIntPredicate::LLVMIntSLT, lhs.1, rhs.1, b"\0".as_ptr() as *const _)))
                }
                BinOp::Le => {
                    return Ok((lhs.0, LLVMBuildICmp(self.builder, llvm_sys::LLVMIntPredicate::LLVMIntSLE, lhs.1, rhs.1, b"\0".as_ptr() as *const _)))
                }
                _ => {
                    return Err(CodeGenError {
                        kind: ErrorKind::Invalid,
                        line: e_lhs.line,
                        message: "Case not handled in binary operation".to_string(),
                    }.into())
                }
            }
        } else if lhs.0 == Type::Boolean && rhs.0 == Type::Boolean {
            match binop {
                BinOp::Or => {
                    return Ok((lhs.0, LLVMBuildOr(self.builder, lhs.1, rhs.1, b"\0".as_ptr() as *const _)))
                }
                BinOp::And => {
                    return Ok((lhs.0, LLVMBuildAnd(self.builder, lhs.1, rhs.1, b"\0".as_ptr() as *const _)))
                }
                BinOp::Eq => {
                    return Ok((lhs.0, LLVMBuildICmp(self.builder, llvm_sys::LLVMIntPredicate::LLVMIntEQ, lhs.1, rhs.1, b"\0".as_ptr() as *const _)))
                }
                BinOp::Ne => {
                    return Ok((lhs.0, LLVMBuildICmp(self.builder, llvm_sys::LLVMIntPredicate::LLVMIntNE, lhs.1, rhs.1, b"\0".as_ptr() as *const _)))
                }
                _ => {
                    return Err(CodeGenError {
                        kind: ErrorKind::Invalid,
                        line: e_lhs.line,
                        message: "Case not handled in binary operation".to_string(),
                    }.into())
                }
            }
        } else if lhs.0 == Type::Float && rhs.0 == Type::Float {
            match binop {
                BinOp::Add => {
                    return Ok((lhs.0, LLVMBuildFAdd(self.builder, lhs.1, rhs.1, b"\0".as_ptr() as *const _)))
                }
                BinOp::Sub => {
                    return Ok((lhs.0, LLVMBuildFSub(self.builder, lhs.1, rhs.1, b"\0".as_ptr() as *const _)))
                }
                BinOp::Mul => {
                    return Ok((lhs.0, LLVMBuildFMul(self.builder, lhs.1, rhs.1, b"\0".as_ptr() as *const _)))
                }
                BinOp::Div => {
                    return Ok((lhs.0, LLVMBuildFDiv(self.builder, lhs.1, rhs.1, b"\0".as_ptr() as *const _)))
                }
                BinOp::Shl => {
                    return Ok((lhs.0, LLVMBuildShl(self.builder, lhs.1, rhs.1, b"\0".as_ptr() as *const _)))
                }
                BinOp::Shr => {
                    return Ok((lhs.0, LLVMBuildAShr(self.builder, lhs.1, rhs.1, b"\0".as_ptr() as *const _)))
                }
                BinOp::BitOr => {
                    return Ok((lhs.0, LLVMBuildOr(self.builder, lhs.1, rhs.1, b"\0".as_ptr() as *const _)))
                }
                BinOp::BitAnd => {
                    return Ok((lhs.0, LLVMBuildAnd(self.builder, lhs.1, rhs.1, b"\0".as_ptr() as *const _)))
                }
                BinOp::BitXor => {
                    return Ok((lhs.0, LLVMBuildXor(self.builder, lhs.1, rhs.1, b"\0".as_ptr() as *const _)))
                }
                BinOp::Eq => {
                    return Ok((lhs.0, LLVMBuildFCmp(self.builder, llvm_sys::LLVMRealPredicate::LLVMRealOEQ, lhs.1, rhs.1, b"\0".as_ptr() as *const _)))
                }
                BinOp::Ne => {
                    return Ok((lhs.0, LLVMBuildFCmp(self.builder, llvm_sys::LLVMRealPredicate::LLVMRealONE, lhs.1, rhs.1, b"\0".as_ptr() as *const _)))
                }
                BinOp::Gr => {
                    return Ok((lhs.0, LLVMBuildFCmp(self.builder, llvm_sys::LLVMRealPredicate::LLVMRealOGT, lhs.1, rhs.1, b"\0".as_ptr() as *const _)))
                }
                BinOp::Ge => {
                    return Ok((lhs.0, LLVMBuildFCmp(self.builder, llvm_sys::LLVMRealPredicate::LLVMRealOGE, lhs.1, rhs.1, b"\0".as_ptr() as *const _)))
                }
                BinOp::Ls => {
                    return Ok((lhs.0, LLVMBuildFCmp(self.builder, llvm_sys::LLVMRealPredicate::LLVMRealOLT, lhs.1, rhs.1, b"\0".as_ptr() as *const _)))
                }
                BinOp::Le => {
                    return Ok((lhs.0, LLVMBuildFCmp(self.builder, llvm_sys::LLVMRealPredicate::LLVMRealOLE, lhs.1, rhs.1, b"\0".as_ptr() as *const _)))
                }
                _ => {
                    return Err(CodeGenError {
                        kind: ErrorKind::Invalid,
                        line: e_lhs.line,
                        message: "Case not handled in binary operation".to_string(),
                    }.into())
                }
            }
        } else {
            return Err(CodeGenError {
                kind: ErrorKind::MismatchedTypes,
                line: e_lhs.line,
                message: "Mismatching types".to_string(),
            }.into())
        }
    }

    pub unsafe fn process_array(&mut self, array: Vec<P<Expr>>) -> Result<(Type, Vec<LLVMValueRef>)> {
        let mut array_values = Vec::new();
        let mut first = Type::Null;
        for (i, element) in array.iter().enumerate() {
            let element = element.clone().into_inner();
            let value = self.match_expr(element.clone())?;
            if i == 0 {
                first = value.0.clone();
            }
            if !first.surface_eq(&value.0) {
                return Err(CodeGenError {
                    kind: ErrorKind::MismatchedTypes,
                    message: format!("Found element of type {:?} expected {:?} in list", value.0, first),
                    line: element.line,
                }.into());
            }
            array_values.push(value.1);
        }

        Ok((Type::List(vec![P(first); array_values.len()]), array_values))
    }
    pub unsafe fn process_text(&mut self, text: String) -> Result<(Type, Vec<LLVMValueRef>)> {
        let mut array_values = Vec::new();
        for element in text.chars() {
            array_values.push(LLVMConstInt(LLVMInt8Type(), element as u64, 0));
        }

        Ok((Type::Text(array_values.len() as u32), array_values))
    }

    pub unsafe fn gen_lit(&mut self, lit: Literal) -> Result<(Type, LLVMValueRef)> {
        match lit {
            Literal::Text(text) => {
                // make it so two strings of same value doesnt make two global pointers
                Ok((Type::Text(text.len() as u32), LLVMBuildGlobalStringPtr(self.builder, (text + "\0").as_ptr() as *const _, "\0".as_ptr() as *const _)))
            }
            Literal::Number(num) => {
                Ok((Type::Float, LLVMConstReal(LLVMDoubleTypeInContext(self.context), num)))
            }
            Literal::Integer(num) => {
                Ok((Type::Integer, LLVMConstInt(LLVMInt64TypeInContext(self.context), std::mem::transmute(num), 1)))
            }
            Literal::Boolean(boolean) => {
                Ok((Type::Char, LLVMConstInt(LLVMInt8TypeInContext(self.context), boolean as u64, 0)))
            }
        }
    }

    pub unsafe fn get_or_create_function(&mut self, name: String, mut args: Vec<LLVMTypeRef>, ret: LLVMTypeRef) -> (LLVMTypeRef, LLVMValueRef) {
        let mut function = LLVMGetNamedFunction(self.module, (name.clone() + "\0").as_ptr() as *const _);
        let function_ty = LLVMFunctionType(ret,
                                         args.as_mut_ptr(), args.len() as u32, 0);
        if function.is_null() {
            function = LLVMAddFunction(self.module, (name.clone() + "\0").as_ptr() as *const _, function_ty);
            self.functions.insert(name, (function_ty, ret.into(), false));
        }
        (function_ty, function)
    }
}

pub mod error;
