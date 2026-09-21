use wasmparser::{Parser, Payload, TypeRef, Validator, WasmFeatures};

use splitscript::compiler::stdlib::semantic::StandardLibrarySemanticExt;
use splitscript::{
    compiler::{
        abi::{AbiCatalog, AbiEffect, AbiImportId, AbiOwnership},
        semantic::{
            ResolvedCall, ResolvedEnumVariantId, ResolvedMember, ResolvedReceiver,
            ResolvedStructFieldId, ResolvedValue,
        },
        stdlib::{
            Availability, CancellationKind, CoreTypeId, Effect, FieldVisibility, Implementation,
            IntrinsicId, ItemVisibility, StandardBinaryOperator, StandardLibrary,
            StandardUnaryOperator, StdlibFieldId, StdlibItemId, StdlibOwner, StdlibStateProviderId,
            StdlibTypeConstructorId, StdlibTypeId, StdlibVariantId, SuspensionKind, TypeVisibility,
        },
        types::{BuiltinType, TypeKind},
    },
    tooling::language::{LanguageCatalog, LanguageItemId, LanguageItemKind},
};

const EXAMPLE: &str = include_str!("../examples/lunistice.split");
const HELLO: &str = include_str!("../examples/hello_lunistice.split");
const SETTINGS_EXAMPLE: &str = include_str!("../examples/lso_desktop_settings.split");

// Runtime strings may live in linear data or be constructed by GC byte-array
// instructions. Reachability assertions must inspect both representations.
fn contains_string_literal(wasm: &[u8], needle: &[u8]) -> bool {
    if wasm.windows(needle.len()).any(|bytes| bytes == needle) {
        return true;
    }
    for payload in Parser::new(0).parse_all(wasm) {
        if let Payload::CodeSectionEntry(body) = payload.unwrap() {
            let mut bytes = Vec::new();
            for operator in body.get_operators_reader().unwrap() {
                match operator.unwrap() {
                    wasmparser::Operator::I32Const { value } if (0..=255).contains(&value) => {
                        bytes.push(value as u8);
                    }
                    wasmparser::Operator::ArrayNewFixed { .. } => {
                        if bytes.windows(needle.len()).any(|part| part == needle) {
                            return true;
                        }
                        bytes.clear();
                    }
                    _ => bytes.clear(),
                }
            }
        }
    }
    false
}

#[path = "compiler/async_runtime.rs"]
mod async_runtime;
#[path = "compiler/catalogs_types.rs"]
mod catalogs_types;
#[path = "compiler/cli.rs"]
mod cli;
#[path = "compiler/closures.rs"]
mod closures;
#[path = "compiler/compiler_queries.rs"]
mod compiler_queries;
#[path = "compiler/diagnostics_migration.rs"]
mod diagnostics_migration;
#[path = "compiler/expressions_control.rs"]
mod expressions_control;
#[path = "compiler/failure_semantics.rs"]
mod failure_semantics;
#[path = "compiler/file_runtime.rs"]
mod file_runtime;
#[path = "compiler/inference_language.rs"]
mod inference_language;
#[path = "compiler/iterators.rs"]
mod iterators;
#[path = "compiler/maps.rs"]
mod maps;
#[path = "compiler/parser_recovery.rs"]
mod parser_recovery;
#[path = "compiler/port_review.rs"]
mod port_review;
#[path = "compiler/profiles_codegen.rs"]
mod profiles_codegen;
#[path = "compiler/ranges.rs"]
mod ranges;
#[path = "compiler/sets.rs"]
mod sets;
#[path = "compiler/snapshots.rs"]
mod snapshots;
#[path = "compiler/state_shapes.rs"]
mod state_shapes;
