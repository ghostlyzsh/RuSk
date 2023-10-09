use llvm_sys::{prelude::*, core::*, LLVMTypeKind};

use crate::parser::{ptr::P, Type as PType};

#[derive(Clone, Debug)]
pub enum Type {
    Text(u32),
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
            (Text(_), Text(_)) => true,
            (List(_), List(_)) => true,
            (Pointer(_), Pointer(_)) => true,
            _ => false,
        }
    }
    pub fn element_eq(&self, other: &Type) -> bool {
        use Type::*;
        match self.clone() {
            List(tys) => tys[0].clone().into_inner().surface_eq(other),
            Text(_) => Type::Char.surface_eq(other),
            Pointer(ty) => ty.surface_eq(other),
            _ => panic!("Expected list type in codegen")
        }
    }
    pub fn is_list(&self) -> bool {
        if let Type::List(_) | Type::Text(_) = self {
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
        } else if let Type::Text(_) = self {
            Type::Char
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
            (Text(a), Text(b)) => a == b,
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
                    } else if width == 8 {
                        Self::Char
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
                Boolean => LLVMInt8Type(),
                Char => LLVMInt8Type(),
                Text(_) => LLVMPointerType(LLVMInt8Type(), 0),
                Pointer(inner) => LLVMPointerType(inner.into_inner().into(), 0),
                List(inners) => {
                    LLVMPointerType(inners[0].clone().into_inner().into(), 0)
                }
            }
        }
    }
}
impl From<PType> for Type {
    fn from(ty: PType) -> Self {
        match ty {
            PType::Text => {
                Self::Pointer(P(Self::Char))
            }
            PType::Number => {
                Self::Float
            }
            PType::Integer => {
                Self::Integer
            }
            PType::Boolean => {
                Self::Char
            }
            PType::List(ty) => {
                Self::Pointer(P(ty.into_inner().into()))
            }
        }
    }
}
