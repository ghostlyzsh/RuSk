use core::fmt;
use std::collections::HashMap;

use llvm_sys::{prelude::*, core::*};

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
    Struct(String, HashMap<String, (u32, Type)>),
    // internal
    I32,
    I1,
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
            (Struct(_, _), Struct(_, _)) => true,
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
            (Pointer(a), Pointer(b)) => a.into_inner() == b.into_inner(),
            _ => false,
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
                Struct(_name, inner) => {
                    LLVMStructType(inner.values().map(|t| t.clone().1.into()).collect::<Vec<_>>().as_mut_ptr(),
                        inner.len() as u32, 0)
                }
                I32 => LLVMInt32Type(),
                I1 => LLVMInt1Type(),
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
                Self::Boolean
            }
            PType::Char => {
                Self::Char
            }
            PType::List(ty) => {
                Self::Pointer(P(ty.into_inner().into()))
            }
            PType::Pointer(ty) => {
                Self::Pointer(P(ty.into_inner().into()))
            }
            PType::Struct(name) => {
                Self::Struct(name.0, HashMap::new())
            }
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let debug = format!("{:?}", self);
        let debug = &debug;
        let name: String = match self {
            Type::Integer => "int".to_string(),
            Type::Text(_) => "text".to_string(),
            Type::Float => "num".to_string(),
            Type::Boolean => "boolean".to_string(),
            Type::Null => "none".to_string(),
            Type::Char => "char".to_string(),
            Type::I32 => "i32".to_string(),
            Type::I1 => "i1".to_string(),
            Type::Pointer(t) => {
                format!("{} pointer", t)
            }
            _ => debug.to_string(),
        };

        write!(f, "{}", name)
    }
}
