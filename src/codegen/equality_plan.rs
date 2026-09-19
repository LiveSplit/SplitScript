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
