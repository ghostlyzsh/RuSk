use llvm_sys::{prelude::*, core::*, LLVMTypeKind};

use crate::parser::ptr::P;

#[derive(Clone, Debug)]
pub enum Type {
    Integer,
    Float,
    Boolean,
    Null,
    Char,
    List(Vec<P<Type>>),
    Pointer(P<Type>),
}
impl Type {
    pub fn surface_eq(&self, other: &Type) -> bool {
        use Type::*;
        match (self.clone(), other.clone()) {
            (Integer, Integer) => true,
            (Float, Float) => true,
            (Boolean, Boolean) => true,
            (Null, Null) => true,
            (Char, Char) => true,
            (List(_), List(_)) => true,
            (Pointer(_), Pointer(_)) => true,
            _ => false,
        }
    }
    pub fn element_eq(&self, other: &Type) -> bool {
        use Type::*;
        match self.clone() {
            List(tys) => tys[0].clone().into_inner().surface_eq(other),
            Pointer(ty) => ty.surface_eq(other),
            _ => panic!("Expected list type in codegen")
        }
    }
    pub fn is_list(&self) -> bool {
        if let Type::List(_) = self {
            true
        } else {
            false
        }
    }
    pub fn get_element(&self) -> Self {
        if let Type::List(a) = self {
            a[0].clone().into_inner()
        } else if let Type::Pointer(a) = self {
            a.clone().into_inner()
        } else {
            panic!("Codegen error: expected list");
        }
    }
}
impl PartialEq<Type> for Type {
    fn eq(&self, other: &Type) -> bool {
        use Type::*;
        match (self.clone(), other.clone()) {
            (Integer, Integer) => true,
            (Float, Float) => true,
            (Boolean, Boolean) => true,
            (Null, Null) => true,
            (Char, Char) => true,
            (List(a), List(b)) => {
                if a.len() != b.len() {
                    return false
                }
                let mut equal = true;
                for i in 0..a.len() {
                    if a[i].clone().into_inner() != b[i].clone().into_inner() {
                        equal = false;
                        break;
                    }
                }
                equal
            }
            //(Pointer, Pointer) => true,
            _ => false,
        }
    }
}
impl From<LLVMTypeRef> for Type {
    fn from(ty: LLVMTypeRef) -> Self {
        unsafe {
            let kind = LLVMGetTypeKind(ty);
            match kind {
                LLVMTypeKind::LLVMIntegerTypeKind => {
                    let width = LLVMGetIntTypeWidth(ty);
                    if width == 64 {
                        Self::Integer
                    } else if width == 1 {
                        Self::Boolean
                    } else {
                        panic!("Unexpected LLVM type");
                    }
                }
                LLVMTypeKind::LLVMDoubleTypeKind => Self::Float,
                LLVMTypeKind::LLVMVoidTypeKind => Self::Null,
                LLVMTypeKind::LLVMPointerTypeKind => Self::Pointer(P(Type::Null)),
                _ => {
                    panic!("Unexpected LLVM type {:?}", kind);
                }
            }
        }
    }
}
impl From<Type> for LLVMTypeRef {
    fn from(ty: Type) -> Self {
        use Type::*;
        unsafe {
            match ty {
                Null => LLVMVoidType(),
                Integer => LLVMInt64Type(),
                Float => LLVMDoubleType(),
                Boolean => LLVMInt1Type(),
                Char => LLVMInt8Type(),
                Pointer(inner) => LLVMPointerType(inner.into_inner().into(), 0),
                List(inners) => {
                    LLVMPointerType(inners[0].clone().into_inner().into(), 0)
                }
            }
        }
    }
}
