//! Function indices reserved for structural equality implementations.

use std::collections::HashMap;

use crate::{
    ast::{
        ArrayTypeId, EnumId, ManagedClassId, OptionTypeId, ResultTypeId, StructId,
        TypeApplicationId,
    },
    stdlib::{StandardLibrary, StdlibTypeId},
};

#[derive(Default)]
pub(super) struct EqualityFunctions {
    pub standard_library: StandardLibrary,
    pub budget: Option<ComparisonBudget>,
    pub string: Option<u32>,
    pub standard_structs: HashMap<StdlibTypeId, u32>,
    pub structs: HashMap<StructId, u32>,
    pub managed_classes: HashMap<ManagedClassId, u32>,
    pub enums: HashMap<EnumId, u32>,
    pub arrays: HashMap<ArrayTypeId, u32>,
    pub options: HashMap<OptionTypeId, u32>,
    pub results: HashMap<ResultTypeId, u32>,
    pub sets: HashMap<TypeApplicationId, u32>,
    pub maps: HashMap<TypeApplicationId, u32>,
}

/// Only managed duplicate detection uses this signature and shared context.
#[derive(Clone, Copy)]
pub(super) struct ComparisonBudget {
    pub charge: u32,
    pub context_type: u32,
}

impl EqualityFunctions {
    pub fn local(&self, ordinary_index: u32) -> u32 {
        ordinary_index + u32::from(self.budget.is_some())
    }

    pub fn charge(&self, f: &mut wasm_encoder::Function) {
        if let Some(budget) = self.budget {
            budget.emit(f);
        }
    }
}

impl ComparisonBudget {
    pub fn emit(self, f: &mut wasm_encoder::Function) {
        use wasm_encoder::{BlockType, Instruction as I};
        f.instruction(&I::LocalGet(2))
            .instruction(&I::Call(self.charge))
            .instruction(&I::I32Eqz)
            .instruction(&I::If(BlockType::Empty))
            // Keep exhaustion distinct from inequality, including at the exact
            // limit. The root decoder checks this sticky marker before insertion.
            .instruction(&I::LocalGet(2))
            .instruction(&I::I32Const(2))
            .instruction(&I::I64Const(crate::managed_read::MAX_SNAPSHOT_WORK + 1))
            .instruction(&I::ArraySet(self.context_type))
            .instruction(&I::I32Const(0))
            .instruction(&I::Return)
            .instruction(&I::End);
    }
}
