//! Stable compiler-facing API and inspectable stage products.
//!
//! The crate root keeps the convenient one-shot and staged functions. This
//! facade classifies the data models that compiler integrations may inspect;
//! implementation passes and registries remain private to the crate.

pub use crate::codegen::{BackendProgram, CodegenReport};
pub use crate::{
    BuildProfile, CheckedProgram, CompilerContext, CompilerOptions, LoweredProgram, ParsedProgram,
    RecoveredCheck, RecoveredParse, WarningLevel, WarningPolicy, check, check_recovering, compile,
    compile_named_with_context_and_options_diagnostics, compile_with_context,
    compile_with_context_and_options, compile_with_context_and_options_diagnostics,
    compile_with_options, lower, lower_wasm, lower_wasm_with_options, parse, parse_named,
    parse_named_with_context, parse_recovering, parse_recovering_named_with_context,
    parse_recovering_with_context, parse_with_context,
};

/// Generates WebAssembly from a successfully checked program.
pub fn codegen(checked: &CheckedProgram) -> Vec<u8> {
    crate::codegen(checked)
}

/// Generates WebAssembly with explicit profile-sensitive options.
pub fn codegen_with_options(checked: &CheckedProgram, options: CompilerOptions) -> Vec<u8> {
    crate::codegen_with_options(checked, options)
}

/// Emits the ordinary module and a sidecar describing its retained functions
/// and linear-memory reservations. The report does not change the artifact.
pub fn codegen_with_report(
    checked: &CheckedProgram,
    options: CompilerOptions,
) -> (Vec<u8>, CodegenReport) {
    crate::codegen::compile_with_report(crate::lower_wasm_with_options(checked, options))
}

/// Host ABI contracts available for backend inspection.
pub mod abi {
    pub use crate::abi::*;
}

/// Lossless syntactic program model.
pub mod ast {
    pub use crate::ast::*;
}

/// Derived semantic capabilities such as equality and memory readability.
pub mod capabilities {
    pub use crate::capabilities::*;
}

/// Shared documentation metadata used by public compiler catalogs.
pub mod catalog {
    pub use crate::catalog::*;
}

/// Interprocedural operation-effect facts.
pub mod effects {
    pub use crate::effects::*;
}

/// Equality capability analysis.
pub mod equality {
    pub use crate::equality::*;
}

/// Declaration index and typed semantic HIR.
pub mod hir {
    pub use crate::hir::*;
}

/// Typed process-memory layouts.
pub mod memory {
    pub use crate::memory::*;
}

/// Resolved semantic identities and inferred facts.
pub mod semantic {
    pub use crate::semantic::*;
}

/// Validated standard-library graph API.
pub mod stdlib {
    pub use crate::stdlib::*;

    /// Compiler-semantic candidate and applicability queries layered over the
    /// backend-neutral catalog graph.
    pub mod semantic {
        pub use crate::stdlib_semantic::*;
    }
}

/// Lossless source document, recovery, and token-span model.
pub mod syntax {
    pub use crate::syntax::*;
}

/// Inferred semantic type universe.
pub mod types {
    pub use crate::types::*;
}

/// Generic syntax visitor utilities.
pub mod visit {
    pub use crate::visit::*;
}

/// Completed Wasm-oriented control-flow and expression IR.
pub mod wasm_ir {
    pub use crate::wasm_ir::*;
}

/// Versioned, transport-neutral compiler service used by embedded hosts.
pub mod service {
    pub use crate::service::*;
}
