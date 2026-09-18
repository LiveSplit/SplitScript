# Compiler architecture

SplitScript exposes its front-end stages as separate inspectable products:

```text
CompilerContext (immutable compiler services and catalog graph) + source
  -> parse -> ParsedProgram (syntax AST)
  -> lower -> LoweredProgram (syntax + resolution tables + DeclarationIndex + diagnostics)
  -> type check -> typed HIR + inferred layouts
  -> validate -> capabilities + effects + semantic diagnostics
  -> check -> CheckedProgram
  -> lower_wasm -> BackendProgram (Wasm IR + coherent checked backend inputs)
  -> codegen -> WebAssembly GC
```

`CompilerOptions` selects `BuildProfile::Debug` or `BuildProfile::Release` and
contains the host-selected `WarningPolicy`. Profile-aware lowering and codegen
consume the profile; complete `compile_with_options` products additionally
apply the warning policy before generating an artifact. The selected profile is
stored on the Wasm-oriented program so profile erasure can remain a semantic
lowering pass.
The convenience entry points default to debug. Until debug-only syntax is
encountered, both profiles emit identical modules. Debug statements, bindings,
globals, and functions remain in typed HIR for diagnostics in every profile.
Release Wasm lowering filters them before local planning, async-state
construction, global storage planning, and backend reachability. The backend
consumes the resulting Wasm-IR global set rather than making its own profile
decision. `BackendProgram` dereferences to its inspectable `wasm_ir::Program`
for control-flow queries, but also carries the matching syntax, semantic model,
constructed layouts, and validated capability analyses as one
coherent backend input. Binary encoding accepts only this product, so callers
cannot accidentally combine stage results from different compilations.

Every staged product also retains the `CompilerContext` that created it. The
default context owns the process-wide validated standard-library graph;
parsing, type checking, recovery, typed HIR and semantic analyses,
`CompilerDatabase` tooling queries, documentation, Wasm IR, backend planning,
and binary emission consume that same handle rather than reconstructing a
catalog. Compatibility entry points may still select the default context, but
an active compilation never changes catalogs between stages.

`lower` establishes a deterministic `hir::DeclarationIndex` with stable
IDs for named types, functions, values, and lifecycle actions. Tools can query
this deliberately narrow index even when type checking would fail; it is not
presented as a resolved body HIR. `LoweredProgram` also owns
`resolution::ProgramResolutions`, a side table for catalog and nominal
identities that must not be written into source syntax. State-provider names
and source-written nominal type references therefore remain unchanged across
lowering; checking publishes the selected provider in `SemanticModel`, and the
backend derives its attachment process list from the provider declaration.
`LoweredProgram` as a whole is the resolution product. Nominal declaration conflicts and
unknown type names are retained separately from parser recovery diagnostics:
they do not invalidate the syntax tree or prevent formatting, but strict
checking reports them before inference. Struct literals retain their written
name in syntax and publish their resolved `StructId` through the semantic
model instead of requiring parser-time nominal lookup. Enum-qualified paths,
calls, match patterns, and choice settings also remain in their original
path/reference syntax. `resolution::resolve_program` records their enum
identities by stable expression, pattern, and setting IDs; checking publishes
the selected variants in `SemanticModel`, and typed HIR owns the resolved enum
constructor and pattern forms. The parser performs no declaration pre-scan and
has no standard-library dependency.

Source `TypeRef` is likewise syntax-only: it contains primitive spellings,
unresolved nominal `TypeNameId`s, and IDs for source-written constructed type
expressions. Lowering maps nominal names to `ResolvedTypeRef` values in
`ProgramResolutions`; inference and checked GC layouts use that semantic form.
Consequently parsed nodes cannot contain `StdlibTypeId`, resolved struct/enum
identities, or the cross-source `EnumTypeId` union. Primitive spellings and
their stable identity are owned by the dependency-light syntax crate, while
the standard-library graph attaches capabilities and runtime layout facts.

The complete ordinary AST, its stable IDs, generic visitor/folder,
parser/recovery grammar, structured syntax diagnostics, and lossless source document are owned by
`splitscript-syntax`. The compiler facade re-exports those exact types; there
is no mirrored compiler AST or conversion layer. The privileged library parser
uses the same lexer and cursor infrastructure under an explicit syntax mode.

Decimal-number lexing recognizes optional fractional and exponent components
without treating `e` as an integer suffix. Parsing rejects values outside the
finite `f64` domain or nonzero significands that become zero. After inference,
the finalization pass performs the corresponding range check for expressions
resolved as `f32`; this preserves target-dependent inference while preventing
code generation from silently emitting zero or infinity. Representable
subnormals continue to the ordinary Wasm `f32.const`/`f64.const` lowering.

Every successful compiler stage also retains a `SourceDocument` produced by
the same lexer pass used by the parser. Its ordered lexeme stream includes
ordinary line and block comments, whitespace, documentation comments, and
semantic tokens with exact byte spans, so concatenating their raw source slices
reproduces the input byte-for-byte. Formatter and editor consumers can use this
layer without reconstructing spelling or layout from the semantic AST.

`parse_recovering` is the editor-facing entry point. It returns the lossless
document, a partial syntax AST, every diagnostic recoverable at top-level
declaration boundaries, and zero-width missing or source-backed error recovery
nodes. The ordinary `parse` entry point uses the same recovery pass but remains
strict only about syntax: any collected grammar diagnostic makes the batch
parse fail, while resolution diagnostics travel with the parsed/lowered
products until checking.

Statement blocks also recover at semicolons, their closing brace, or the next
plausible statement on a later line. Nested blocks recover independently, so a
bad local declaration does not discard later statements in the same function
or lifecycle action. Failed expectations leave the unexpected token untouched,
which prevents a missing delimiter from consuming a valid recovery boundary.

Struct and enum bodies use the same principle at member boundaries. Invalid
fields or variants are represented by recovery nodes, while later valid
members remain in their enclosing declaration and retain ordinary stable IDs.
State declarations recover field-by-field as well, for both the current DSL
and the older constructor-shaped syntax accepted by the prototype.
Settings recover entry-by-entry in the current simple and nested documentation
DSLs as well as the older constructor-shaped syntax. Documentation comments
following an invalid entry remain attached to the next valid setting.
Choice options and file filters are recovery boundaries too; their containing
setting remains available when at least its other entries are valid.
Match expressions recover arm-by-arm, retaining later patterns and the
enclosing expression, statement, block, and function.
Function parameter lists likewise retain valid parameters and the function
body after an invalid parameter or a missing closing parenthesis.
Array literals and function calls recover each comma-delimited expression
independently. A malformed element or argument therefore produces a localized
recovery region while later values and the enclosing statement remain in the
partial syntax tree.
Struct literals apply the same behavior at field boundaries. Template strings
recover within each interpolation boundary, so invalid embedded syntax does
not discard later text, later interpolations, or the call containing the
template.
Missing unary or binary operands and malformed parenthesized expressions use
an explicit syntax-only error expression. This keeps the surrounding operator,
initializer, and later statements structurally available to editor tooling.
The checker rejects recovery expressions, so they cannot enter typed HIR or
code generation.
Expression-valued `if` uses the same placeholder for missing conditions, empty
or malformed branches, and a missing required `else`. Consequently the
conditional itself and later statements remain queryable. Every missing
recovery node is represented by a zero-width source span.
At declaration and statement roots, failed expressions synchronize before a
semicolon, closing delimiter, or the next source line and become error
expressions. This retains globals, expression-backed state fields, locals,
assignments, conditions, throws, suspensions, standalone expression
statements, and matches with missing scrutinees. Newline synchronization does
not require the next token itself to look valid, so consecutive malformed
statements remain distinct syntax nodes and diagnostics.

Diagnostics are backend-independent values defined in
[`crates/splitscript-syntax/src/diagnostic.rs`](../crates/splitscript-syntax/src/diagnostic.rs).
They carry a stable code and severity. Compiler-stage errors use `SS0001` for
lexing, `SS0002` for parsing, `SS0003` for type checking, and `SS0004` for
post-type semantic validation. Actionable warnings use their own `SS1xxx`
namespace: `SS1001` for discarded must-use values, `SS1002` for unread local
bindings, `SS1003` for unreachable declarations, and `SS1004` for unused state
fields, settings, struct fields, or enum variants. `SS1009` identifies normal
locals, globals, and functions reached exclusively from debug-only code, where
release builds retain work without a release consumer. Clients can therefore
configure and present a warning without parsing its human-readable message.
The same value owns its primary and secondary labels, notes, and fixes. Fixes
have an explicit applicability and may contain multiple source edits. The
repeated optional/result-postfix diagnostic already supplies a
machine-applicable edit that removes the extra postfix, while unused local
bindings provide a multi-edit underscore-suppression action. For unused
declarations, state fields, and nominal members, the LSP reuses the
identity-aware Rename query to update every reference and shared source
declaration, tries extra underscores on collisions, and type-checks the
candidate before offering the action. Unused settings receive a targeted
source-name edit that preserves their host-visible key. The LSP publishes the
same diagnostic codes as the CLI/compiler service. The native CLI converts this
shared value to `codespan-reporting` only at its terminal boundary. Errors and
warnings are therefore rendered with annotated source snippets, terminal-aware
color, and every primary and secondary label without coupling compiler passes
to a particular presentation library. Formatter failures and watch-mode
rebuilds use the same renderer. Spans remain byte offsets within the one source
file; a `FileId` and source map are deliberately deferred until the language
gains modules or another feature that accepts multiple sources.

Unused declarations use one profile-labelled reachability graph. Ordinary
unused warnings inspect whether any profile reaches a declaration, while
`SS1009` checks whether that same path includes release. Edge profiles are
intersected as reachability crosses debug statements and source declarations,
which makes transitive helpers and named function values behave consistently.
The compiler offers a machine-applicable `debug` insertion only when erasing
the declaration cannot leave a release-visible assignment behind.

`WarningPolicy` independently configures each `SS100x` code as `allow`, `warn`,
or `deny`. Policy is deliberately applied after semantic checking. `allow`
omits the diagnostic from the configured product, `warn` preserves its normal
non-fatal severity, and `deny` rejects artifact generation while retaining the
warning code, labels, fixes, and origin. A denied warning is therefore never
reclassified as an `SS0002` parser error or `SS0003` type error. The incremental
database applies policy only to its diagnostic cache, so hover, completion,
rename, and other semantic queries remain available. The same serializable
policy crosses the embedded compiler-service boundary. A future persistent
project format can own this value without changing compiler passes or protocol
semantics.

Post-type semantic checks live in [`src/validation.rs`](../src/validation.rs),
not in the public pipeline glue. One `ValidationOutput` derives capability and
operation-effect facts and owns detached-call, equality, memory-readability,
and generic-capability diagnostics. Strict and recovering checks invoke this
same stage; recovery retains derived effects when validation reports an error,
so hover and other editor features do not reconstruct a weaker analysis path.

Catalog-owned SplitScript bodies enter the same typed-function pipeline through
a private compilation view. Each source-defined catalog item contributes one
hidden function template, indexed in typed HIR by its stable catalog item ID
rather than by its generated source name. Concrete catalog calls retain their
exact declared receiver, parameter, and result signature. Wasm lowering maps
that signature onto the hidden function's inferred type scheme and creates the
same `FunctionInstance` used by ordinary source functions. Reachability thus
materializes only the concrete instances selected by user or library calls,
including constructed types such as `[T]` and `[T; N]`, without an eager
catalog-specific substitution tier.

Ordinary source functions have inferred Hindley-Milner-style type schemes with
capability constraints. The type checker builds a deterministic declaration
dependency graph: free calls add exact edges and method syntax conservatively
adds edges to source methods with the selected member name. Tarjan components
are checked callee-first. Calls inside one component remain monomorphic while
it is solved, then declaration-local roots are generalized at the component
boundary. Each external call instantiates fresh roots while preserving shared
variables, numeric-literal constraints, capability requirements, and nested
`[T]`, `T?`, and `T!` shapes. Mutually recursive generic functions are
supported; an occurs check rejects polymorphic recursion that would require an
infinite type with a focused diagnostic.

Expected-type checking retains source provenance separately from the inferred
type graph. An explicit parameter, result, variable, state field, struct field,
or enum payload therefore contributes both its semantic type and the span and
wording of the declaration that imposed it. Nested collection and wrapper
checking preserves that provenance. A mismatch labels the supplied expression
as primary and the declaring contract as secondary; capability constraints and
literal range failures keep their more specific explanations instead of being
collapsed into a generic type mismatch.

`semantic::FunctionInstance` separates a declaration from a concrete body. Its
structural identity contains the `FunctionId`, inferred type arguments, and the
exact concrete parameter/result signature. The signature is required because
nominal GC layouts such as two independently interned `[String]` types
cannot be reconstructed from generic arguments alone. Resolved calls, typed
function owners, Wasm body owners, reachability, function-index planning, and
emission all carry this same identity.

The checked semantic model and typed HIR retain one syntax-keyed template body
for editor stability. Demand-driven backend reachability traverses that body in
the context of each concrete owner and specializes expression types, locals,
conversions, nested calls, wrappers, matches, and intrinsic arguments without
overwriting source facts. Before layout planning, a backend-private clone of
the semantic and constructed-type arenas materializes any concrete
`[T]`/`T?`/`T!` layouts that occur only inside a generic body. Roots
come from actions, states, globals, and their transitive concrete calls. The
instance graph is structurally deduplicated and imposes deterministic generic
expansion-depth and total-instance limits; ordinary monomorphic call chains do
not consume those limits. Function effects remain definition-level because
they are independent of type instantiation, while inferred capability bounds
remain visible to validation, hover, and completion.

The [`src/database`](../src/database) module family provides the revisioned
single-source query facade used as the foundation for editor tooling.
`queries.rs` owns source revisions and query orchestration, `cache.rs` owns
per-revision storage, `snapshot.rs` owns the strict-or-recovered semantic view,
and position analysis, value references, and rename validation have independent
products. Recovering parse,
strict parse, declaration lowering, semantic checking, and diagnostics are
cached as shared results. Supplying identical text preserves both the revision
and cached allocations; a changed buffer invalidates all dependent stages.
Diagnostics return the first failing stage, while the recovering parse remains
independently queryable after syntax errors. This is intentionally explicit
invalidation rather than a dependency framework: more granular machinery can
be justified later by measurements and additional consumers.

The database also owns the editor-facing semantic query surface. Clients can
look up declarations by name; request inferred expression, value, and function
result types; inspect semantic type shapes; resolve calls, paths, and
assignment targets; and obtain a cached reference index without navigating the
checked program's internal products. References are deterministically ordered,
distinguish reads from writes, and include value paths and method receivers.
The same source-identity index powers document highlights, while inferred
expression types navigate either to their source declaration or to the
canonical compiler-owned documentation page for catalog types. Neither LSP
feature reconstructs semantic meaning in the protocol adapter.
The recovering AST can also be lowered into declaration HIR when strict parsing
fails, preserving valid globals, types, functions, and actions for outline and
name queries. For checked source, `analysis_at` selects the smallest typed
expression containing a byte offset and returns its inferred type shape and
semantic resolution. When syntax is valid but type checking reports errors,
`recovering_check` keeps the diagnostics and publishes all semantic facts that
could still be resolved. One cached `SemanticSnapshot` selects the strict or
recovered product and gives every editor query the same view. Database type,
call/path, assignment, position, definition, hover, and signature queries use
that shared product, so an unrelated bad expression does not disable hover or
navigation elsewhere. Recovery-only
fallback types merely make the semantic model representable; strict `check`
continues to reject the program and typed HIR and Wasm lowering are never built
from those fallbacks. Exact identifier segments connect recovered resolutions
to the same source and compiler-catalog definition targets as checked source.
The sibling `format` query reuses the cached strict parse and canonical
formatter, returning shared formatted text or the same syntax diagnostics.
The native and WebAssembly-backed language servers expose this query as
whole-document LSP formatting.

After inference, `check` materializes a `TypedProgram` body HIR. Every
`TypedExpression` contains its stable `ExprId`, source span, and semantic
`TypeId`. Its expression kind owns literal data, operators, and child `ExprId`s;
typed blocks likewise own variables, assignments, branches, returns, suspensions,
and expression statements. Match arms own their pattern, optional guard, and
result expression. Type-directed resolutions are attached to the relevant node:
value-path roots and member chains, user or standard-library calls, struct
field order, and enum constructors. Assignments and match patterns similarly
carry their resolved targets, while choice settings carry resolved default and
option variants. Node enumeration is stable-ID ordered for deterministic LSP
and documentation consumers. Implicit conversions live on typed operand edges;
for example, a non-string template interpolation carries an explicit
`ToString` conversion and retains the operand's original `TypeId`.

Global initializers are checked before function and action bodies. Before that
pass, the checker identifies unannotated bare-`None` globals with a later plain
non-`None` assignment and seeds them as `T?`; the assignment then constrains
`T` through the ordinary option-lift rule. Bare `None` globals without such an
assignment retain the unit type, so this declaration-shape pass does not make
every `None` initializer an ambiguous option.

Focused front-end snapshots render declaration identities, inferred function
signatures, typed body edges, expression types, semantic resolutions, and
implicit conversions without byte offsets or backend indices. A sibling
diagnostic snapshot records ordered, source-located error messages. These
fixtures make intentional semantic changes reviewable while keeping backend
tests centered on validated Wasm and observable runtime behavior.

[`examples/compiler_baseline.rs`](../examples/compiler_baseline.rs) measures
release-mode one-shot compiler latency and deterministic output size across the
minimal, Lunistice, cancellation, and settings fixtures. Results and the exact
methodology live in [`docs/BASELINES.md`](BASELINES.md); timings remain trend
signals rather than platform-sensitive test thresholds.

Wasm lowering consumes typed HIR once and publishes a backend-owned expression
DAG plus structured control flow. Local and async-frame layout discovery, match
and intrinsic scratch planning, string/signature discovery, statement and
expression emission, global constant expressions, and async polling consume
that lowered product. Calls, paths, assignments, constructors, and match
patterns therefore arrive with semantic targets already selected; encoding no longer descends
through syntax expressions or re-queries expression-resolution side tables.
Parsed declarations still provide source-level state-pointer and settings
metadata.

Public language metadata is also backend-independent. `StandardLibrary`
catalogs callable APIs, while `LanguageCatalog` catalogs keywords, lifecycle
actions, settings forms, and syntax such as `retry expression`. Their stable
IDs remain distinct, but both use the shared `catalog::Documentation` and
`catalog::Example` model. Catalog validation checks identity and documentation
integrity, and the compiler test suite parses, checks, and lowers every embedded
example before editor or generated-documentation clients can expose it.

Contextual syntax keeps its exact keyword spans in the shared syntax tree.
Compiler queries resolve `at` and `key`, settings `choice` / `default` /
`file` / `mime`, and the `in` in a `for` loop to catalog entries only at those
grammar positions. Semantic highlighting consumes the same spans. These
spellings are therefore documented in their DSL roles without globally
reserving them or misclassifying an ordinary parameter, local, field, or
function with the same name. Globally meaningful keywords continue to resolve
directly from their source token; standard-library syntax such as `utf8` and
state-provider names continues through the standard symbol graph.

The bundled standard library is authored hierarchically once in privileged
SplitScript source at
[`stdlib/standard.split`](../stdlib/standard.split). Each namespace, nominal
type, capability, and type constructor owns its fields, variants, associated
functions, methods, documentation, focused examples, and trusted binding name.
The dependency-light [`splitscript-syntax`](../crates/splitscript-syntax)
crate owns the shared lexer, spans, tokens, explicit ordinary/privileged modes,
token cursor, documentation-comment collection, and privileged declaration
grammar. Both grammars therefore share position, lookahead, EOF, and token
matching behavior while retaining their domain-specific recovery policies. The
[`splitscript-stdlib-loader`](../crates/splitscript-stdlib-loader) consumes that
syntax tree during the Cargo build and directly generates both opaque catalog
IDs and the final typed declaration arrays. Privileged type expressions are a
structured syntax tree, so generation never recovers `T!`, `[T]`, or generic
applications by parsing strings. The loader validates source-level names,
constructor arity, attributes, layouts, capabilities, providers, and examples
before emission. There is no parallel lexer, Rust declaration grammar,
normalization macro, or compiler-side runtime reparse of the same source.
Catalog IDs remain open `u32` newtypes with
generated well-known constants, while the closed Rust-owned `IntrinsicId`
registry remains the trust boundary. Dependency-light normalized shapes live
in `declarations.rs` and `schema.rs`, while cross-declaration checks live above
them in `validation.rs`.
[`src/stdlib/graph.rs`](../src/stdlib/graph.rs) validates identity, names,
paths, referenced owners, and namespace parents once, then indexes declarations
by identity/name/path plus owner-to-child, field, variant, and method edges.
Compiler and tooling queries consume this immutable graph; flat tables remain
deterministic iteration views. Architecture tests enforce the one-way producer
layers and reject reintroduction of retired parallel registries or positional
callable factories.

The loader is only a build dependency of the compiler. At runtime the compiler
loads the generated declarations, builds the indexed graph once, and validates
that graph directly against trusted intrinsic contracts. This keeps malformed
source diagnostics at the build boundary and avoids maintaining a second
source-to-catalog comparison implementation inside the compiler.

Callable signatures use a backend-neutral declaration type expression. Core
types, nominal library types, and type parameters are atomic references; every
generic shape is the same recursive application node keyed by an open catalog
constructor ID. `[T]`, `T?`, `T!`, and `Set<T>` are unary constructor declarations,
so adding a future generic constructor does not add a parallel type-expression
variant. Catalog validation resolves constructor IDs, checks arity and nested
arguments, preserves constructor-parameter capability constraints, and rejects
references to undeclared type parameters before the semantic adapter
instantiates the supported constructors. Source-written `Set<T>` applications
retain every delimiter occurrence for formatting and editor tooling while
sharing one structural syntax identity for inference and lowering.

The magical scalar spellings and identities are defined once as
`splitscript_syntax::PrimitiveType`. `CoreTypeId` and semantic `BuiltinType`
are aliases of that identity rather than parallel enums. The standard-library
core table attaches capability and scalar-memory facts, while
`with_core_types!` mechanically derives the backend's deliberately physical
primitive variants/conversion. Constructed syntax, semantic, and physical
types remain separate because they carry genuinely different stage-specific
facts. Stable syntax ID constructors are private to the syntax crate.
Parser-written and inference-created constructed layouts share one explicit
per-program allocator, and nominal type-name resolution consumes AST-owned
identity iteration rather than recreating IDs from vector positions.

Generic bounds store catalog capability IDs directly. Inference accumulates a
deduplicated set of those IDs rather than maintaining a fixed bit assignment.
Capability declarations select their evaluation behavior: declared membership,
recursive equality, or recursive process-memory layout. Candidate discovery,
inference admissibility, and final semantic validation consult that descriptor,
so adding a marker capability does not add a compiler switch. A future
privileged custom behavior will be registered at the same trust boundary as
intrinsics when a real standard-library use case requires one.

Callable signatures record asynchronous completion directly as `async T` in
privileged standard-library source. This is a type fact rather than an
operation category: [`await`](LANGUAGE.md#await) accepts an `async T`
expression, while [`retry`](LANGUAGE.md#retry) accepts synchronous
fallible `T!` work. Privileged intrinsic declarations separately record only
the runtime context that cannot be recovered from a body: attached-process and
state-snapshot requirements, suspending attachment availability, and process-close
cancellation. The loader validates those attributes and the closed Rust
registry independently verifies that they match the trusted lowering contract.

Callable effects are exposed as a canonical `EffectSet`, not declaration-order
slices. Iteration is deterministic and duplicate effects are impossible.
Intrinsic observable effects come from the closed trust registry, with
signature asyncness and the narrow source-authored context merged into the
frontend view. Source-defined standard-library bodies contain no placeholder
effect metadata: the compiler checks the library as a standalone synthetic
unit once per catalog graph, derives transitive operation metadata through the
ordinary typed call graph, and caches that immutable overlay. User
compilations recheck injected bodies against the cached result, so metadata is
user-independent without creating a second effect language in the loader.
Before operation analysis,
[`validation/stdlib_bodies.rs`](../src/validation/stdlib_bodies.rs) proves every
catalog call shape is a consistent instance of the hidden body's inferred type
scheme.
This directional check permits a body to infer a safely more-general template,
while rejecting concrete narrowing, inconsistent receiver/parameter/result
relationships, nested wrapper mismatches, and capability requirements not
guaranteed by the public declaration. A malformed privileged body is therefore
a catalog-construction error; it cannot survive until demand-driven
specialization or backend emission.
Catalog validation rejects empty sets, purity mixed with observable behavior,
cancellation without an async result or attachment, process reads without
attachment, and synchronous calls to attachment-suspending operations.
`OperationSemantics` is the shared normalized view used by checking, hover,
async planning, and documentation. The trusted intrinsic registry enforces the
intrinsic implementation contract; source-body conformance is enforced against
the standalone compiler-derived result.

Compiler-implemented calls also have an independent trusted registry in
`intrinsic_registry.rs`. `IntrinsicId::ALL` is generated with the public IDs,
while an exhaustive Rust match requires one contract for every implementation.
Before user code is checked, catalog validation compares each public binding's
callable kind, explicit and receiver-inherited generic parameters, method
receiver, recursive parameter/completion types, literal-only parameter rules,
async result shape, contextual requirements, cancellation, and availability
with that contract. Generic parameters are matched by ordinal, so cosmetic
names do not become ABI. The contract also classifies host-boundary,
representation, retryable, and suspending lowering. `Retryable` here is only a
backend implementation strategy for producing a `T!`; it does not make the
call a distinct frontend operation kind. Direct generated-helper and
host-import roots live in the same contract, and the
backend dependency planner interprets those roots instead of dispatching on
`IntrinsicId`. Synchronous and suspension scratch policies also live in the
contract as typed core/expression/`T!`-payload slots, so Wasm-IR local
planning no longer rematches every intrinsic. The runtime-helper registry
recursively derives observable effects through helper and ABI roots and checks
them against the trusted contract before Wasm emission, so public declarations
cannot understate their implementation.

Wasm-IR lowering is also the single semantic-to-backend call boundary. It
converts front-end `ResolvedCall` values into its own `CallTarget`; intrinsic
targets retain the public item ID, trusted intrinsic ID, concrete type
arguments, and resolved receiver path. Backend reachability, dependency
planning, async polling, and expression emission no longer reopen the catalog
or import the front-end call enum to rediscover implementation identity.

All standard-library documentation links use `StdlibSymbolId`, including
callables. Validation resolves related links across namespaces, capabilities,
constructors, types, fields, variants, and callable items, so tooling and a
future HTML renderer share one cross-kind identity space.

The normalized catalog layer is backend-neutral: it does not import inference
or semantic type representations. [`src/stdlib_semantic.rs`](../src/stdlib_semantic.rs)
is the explicit one-way adapter that turns semantic `TypeKind` values into
applicable catalog methods and typed call candidates. Type checking,
completion, and hover opt into that adapter; documentation, graph validation,
and future catalog loaders remain independent of compiler-semantic types. An
architecture test guards this dependency direction.

Privileged parsing is an explicit mode of the shared syntax layer. Only the
compiler-created build path may use reserved declarations and
decorators for intrinsic bindings, representation hooks, runtime-private
fields, state-provider attachment, or literal/selector rules. Ordinary source
rejects that decorator syntax; there is no CLI switch, import, or shadowable
module name that grants this authority. Before the default graph is exposed,
the compiler validates every privileged binding's callable shape, signature,
effects, availability, suspension and cancellation behavior against the small
Rust trust registry. Future modules and ordinary generic function bodies can
split the currently monolithic source by runtime domain without changing this
security boundary.

The lower-level host boundary has a separate `AbiCatalog`. It is intentionally
not an LSP or public standard-library input. Each host import records its stable
ID, module and field name, Wasm parameters/results, memory or handle ownership,
lifetime contract, effects, and internal documentation. Wasm import types and
function indices are generated by iterating this catalog; backend call sites
retain only the resulting ID-to-index binding. A conformance test parses the
emitted module and verifies `docs/ABI.md` against the catalog-rendered table, so
the runtime contract cannot silently drift across code and prose.

Compile-time settings families preserve two deliberately separate views in the
syntax tree. `Program::setting_families` retains the written range, templates,
binding spans, and documentation for formatting and editor queries. Parsing
also expands the finite range into ordinary non-source-visible `SettingDecl`
values in `Program::settings`. Validation, storage planning, static-data
collection, host registration, refresh, tooltips, and `settings.enabled(key)`
therefore consume one concrete declaration model. Source symbols, member
completion, rename, and document outlines filter the generated declarations so
implementation names never leak into the language.

The declaration pass also owns an exact runtime-key index with each key's
setting kind, declaration span, and optional source-visible member name.
Resolved calls to `SettingsView.enabled` and `contains` validate ordinary string
literals against that index, while computed strings deliberately remain runtime
lookups. Completion uses the same declarations to offer only compatible keys
inside the quoted argument and replaces only the literal's contents.

The first physical backend split follows that boundary:
[`src/codegen/imports.rs`](../src/codegen/imports.rs) owns host-import type
emission and the catalog-order-to-function-index mapping. It returns the import
section, concrete ABI indices, imported-function count, and next free type
index to the final module orchestrator. This keeps catalog interpretation out
of the larger code generator without exposing backend indices to semantic or
editor layers.

[`src/codegen/expression.rs`](../src/codegen/expression.rs) owns structured
block and assignment emission, resolved receiver/path access, every Wasm-IR
expression variant, casts and comparisons, `T!`/`T?` control flow, match
lowering, and standard-library intrinsic dispatch. Its parent-facing surface is
the expression context plus entry points for a block, assignment, expression,
or receiver. Action, state-read, and async encoders can therefore reuse
ordinary expression semantics without reaching into their implementation or
re-querying typed HIR.

[`src/codegen/async_state.rs`](../src/codegen/async_state.rs) owns lifecycle
action and source-future state-machine emission. It discovers the already-numbered Wasm-IR poll and
resume states, traverses nested continuation blocks, emits builtin suspension
polls and arbitrary `retry T!` polling, stores live values in the GC frame, and
applies the attachment-lifetime cancellation region. Only the completed async
action and future entry points are visible to the final orchestrator;
ordinary values and receivers are delegated through the expression module.

[`src/codegen/gc_types.rs`](../src/codegen/gc_types.rs) owns deterministic GC
layout construction. It emits one recursive group containing the state and
built-in runtime types, the async continuation frame, and only reachable
nominal structs, enums, inferred arrays, Options, and Results. `GcLayout` owns
their compact deterministic order and the encoder consumes that same order.
It also derives standard type indices, standard field slots, enum variant
indices, physical value representations, and the continuation-frame position
from the compilation's catalog graph. No emitter is allowed to rediscover
those facts from the process-wide default catalog.
The resulting plan contains the completed type section and first free type
index, which feeds host-import and generated-function type assignment without
making the final orchestrator understand individual GC fields.

[`src/codegen/settings.rs`](../src/codegen/settings.rs) owns the complete
settings lifecycle: host widget registration, nested titles, text inputs,
choices, file filters and tooltips, settings-map acquisition/freeing, String decoding,
default fallback, and atomic current/old value rotation. It also emits the
start routine that initializes enum globals, source-level state defaults, and
the async frame before registration. Final assembly sees only generated bodies
for String decoding, refresh, and start rather than individual setting kinds.

[`src/codegen/update.rs`](../src/codegen/update.rs) owns the exported per-tick
runtime behavior. It emits process attachment and one-shot detached callbacks,
process-close cancellation/reset, settings refresh, persistent state-field
polling, first-snapshot seeding, snapshot rotation, lifecycle action ordering, nullable loading and
game-time handling, and timer start/split/reset commands. Final assembly passes
only resolved read/action function indices and receives one generated update
body, keeping host lifecycle policy out of the section orchestrator.

[`src/codegen/runtime_helper_registry.rs`](../src/codegen/runtime_helper_registry.rs)
is the sole generated-helper registry. Each descriptor owns the helper's
`RuntimeHelperId`, symbolic Wasm signature, direct helper and ABI dependencies,
and body-builder callback. Descriptor order is canonical and dependency-first;
there is no parallel helper-order list, signature match, dependency match, or
body dispatch. Intrinsic contracts refer to this same helper identity rather
than mirroring it in an intrinsic-only enum.

[`src/codegen/runtime_helpers.rs`](../src/codegen/runtime_helpers.rs) is the
small descriptor-callback orchestrator. Implementations are split by domain in
[`src/codegen/runtime_helpers/`](../src/codegen/runtime_helpers/): String
conversion/formatting, structural equality, process memory/signature scanning/
bounded native UTF-8 and UTF-16LE, managed UTF-16, and Unity type and field
discovery. Native UTF-16LE has a reusable scratch decoder separated from its
process-read wrapper; the input and worst-case replacement-encoded UTF-8 output
occupy distinct, explicitly sized alias banks. Each
domain has explicit imports and a narrow builder surface. Runtime bodies are
built by iterating the ordered `RuntimeHelperPlan`; settings adapters use the
same path, while type-directed structural equality remains a separate
generated-function family.

[`src/codegen/function_plan.rs`](../src/codegen/function_plan.rs) owns the
single generated-function index space. Starting after the catalog-driven host
imports, it resolves descriptor signatures and records helper indices in one
ordered `RuntimeHelperPlan`, then declares structural equality bodies, user
functions, state readers, lifecycle actions, and exported entry points in body
emission order. Emitters receive named indices and cannot advance the type or
function counters themselves.

[`src/codegen/global_plan.rs`](../src/codegen/global_plan.rs) allocates runtime,
source, and settings globals. Its `RuntimeGlobals` result names the process
handle, current/old snapshots, attach readiness, async frame, async action
completion, and detached
entry latch. All emitters consume those roles rather than relying on raw global
numbers or declaration order.

[`src/codegen/script_functions.rs`](../src/codegen/script_functions.rs) emits
ordinary source-defined bodies: fallible pointer and expression state
reads, user functions, and non-async actions. It also owns deterministic
Wasm-local assignment for values, match storage, fallback values, intrinsic
scratch space, and suspension scratch space; expression and async lowering
share that assignment routine without owning index policy.

The syntax AST represents a static pointer root separately from its offsets:
an absolute root is a full unsigned `u64`, while a module root carries a signed
`i64` initial displacement. All offsets after a pointer dereference are signed
`i64`. State-reader code generation lowers displacement addition to Wasm
`i64.add`, whose modulo-2^64 behavior is the language's specified wrapping
address arithmetic. The generic source-defined `address.offset` casts its
integer displacement to the address representation and delegates to intrinsic
`address.add`; it needs no additional compiler special case. Keeping the root
distinction in the AST prevents a high absolute address from being
reinterpreted as a negative offset while allowing ordinary negative paths
without casts.

[`src/codegen/data_plan.rs`](../src/codegen/data_plan.rs) discovers and interns
all static UTF-8 text and parsed signature needles/masks before body emission.
It exposes immutable lookup pools to emitters and alone encodes their linear
memory data segments, keeping memory offsets deterministic and preventing the
final assembler from rediscovering source dependencies. Pools retain relative
offsets until scratch requirements are known, then relocate together after the
page-aligned runtime area centralized in
[`src/codegen/memory_plan.rs`](../src/codegen/memory_plan.rs). The memory plan
checks wasm32 bounds and derives the module's minimum page count from the
actual static payload, so large sources cannot overlap scratch storage or
produce out-of-bounds active data segments.
The plan packs typed settings, scanning, C-string, managed-UTF, and ABI-read
roles into explicit primary/companion alias classes. The primary bank is sized
from the largest analyzed readable layout, logical API capacities, and actual
signature overlap; the companion bank holds values that must coexist with it.
A readable struct larger than one page therefore moves static data rather than
overflowing a historical buffer. Unbounded host String staging begins at the
first page after immutable data and grows memory before writing, preventing
long runtime messages from corrupting static strings or signatures. A distinct aligned
`AbiReadScratch` role owns synchronous process-read output across source-state,
expression, async, process-helper, and Unity-helper emission. Each call checks
its complete size, decoding uses the named base, and callers must materialize a
value before any nested read reuses the buffer; an architecture test rejects
anonymous destinations and raw address-zero loads.

[`src/codegen/unity_layout.rs`](../src/codegen/unity_layout.rs) is the sole
target-layout authority for the implemented 64-bit IL2CPP family. It declares
the versioned class offsets alongside the invariant assembly/image/class/field
schema and object-layout facts needed by low-level metadata helpers. The
high-level `Unity.il2cpp` discovery algorithm, supported-version policy,
signatures, scan windows, and instruction displacements live together in its
standard-library source body. Compiler-start validation and architecture tests
reject malformed, duplicated, misaligned, incomplete, or redeclared backend
layout facts before an emitter can silently drift.

[`src/shape_selection.rs`](../src/shape_selection.rs) derives the bounded,
backend-independent decision plan for attachment-wide shape dimensions.
Conditional managed fields contribute exact presence evidence. The same plan
drives semantic validation and Wasm emission, so automatic selection cannot
silently disagree with the conditions used for member refinement. Payload
dimensions, indistinguishable evidence, unavailable provider probes, and
oversized products retain explicit user selection through [`onAttach`].

[`src/codegen/dependencies.rs`](../src/codegen/dependencies.rs) scans resolved
standard-library calls in Wasm IR and closes descriptor-declared helper and ABI
dependencies transitively. The first consumer is static-data planning: Unity's
module name and IL2CPP signatures are discovered from the reachable
standard-library body and remain absent unless `Unity.il2cpp` is used. Function
planning and body generation traverse the same filtered
descriptor order and the latter consumes the former's concrete plan; checked
helper-index lookups catch missing transitive edges, and no placeholder
signatures or bodies remain. Settings-free programs also omit their two
map-decoding adapters and refresh calls. Tests assert
observable data-segment and function-count behavior rather than exact indices.

The same dependency plan owns required host imports. Fixed process lifecycle
behavior, present action kinds, pointer-backed state fields, resolved
intrinsics, generated helpers, and individual settings kinds contribute stable
`AbiImportId`s. `codegen/imports.rs` filters the declarative ABI catalog by
that set while retaining catalog order, and all body emitters use checked ID
lookups. Consequently a minimal script imports only timer-state inspection and
the three process-lifecycle operations, while richer scripts pull in precisely
the timer, memory, logging, and settings APIs their emitted bodies call.
`abi.rs` authors each import once; that declaration list generates the stable
`AbiImportId` variants, `ALL`/`COUNT`, and normalized import table together, so
identity and table order cannot drift.

`language.rs` likewise authors ordinary syntax, built-in types,
compiler-provided snapshot roots, and lifecycle actions in one grouped source.
It generates `LanguageItemId` and the ordered documentation table together
while preserving typed core-type and `ActionKind` payload relationships;
language syntax remains deliberately separate from the standard library.

[`src/codegen/reachability.rs`](../src/codegen/reachability.rs) computes the
source-level live graph before backend dependencies are selected. Lifecycle
actions, expression-backed state fields, and global initializers are roots;
resolved user-function and user-method calls add bodies transitively, including
recursive call cycles. Function planning/body emission and static-data/helper/
import analysis all consume the same result. Dead functions therefore cannot
retain their strings, signature literals, generated helpers, or host imports.
Reachable equality operands additionally close over nested source or catalog
struct fields, enum payloads, and `T?`/`T!` values; `T!` errors retain
String equality. Thus
structural-equality signatures/bodies and the String equality helper are emitted
only when a live comparison can call them. The same analysis
collects GC-type roots from state and poll storage, globals, settings, emitted
function signatures/locals, async frames, and live expressions, then closes
over every aggregate member. Compiler-generated layouts such as interpolation's
`[String]` are explicit roots rather than accidental side effects.

[`src/codegen/global_plan.rs`](../src/codegen/global_plan.rs) assigns runtime,
source-global, and current/old settings storage. Its result contains the global
section plus the value-to-index/type maps used by lowering, so body generators
cannot allocate globals opportunistically.

Initialized source globals have two backend forms. Literal scalar expressions
remain Wasm global constant expressions. Every other closed, synchronous, pure
initializer is normalized by `wasm_ir.rs` into an ordinary control-flow block
ending in a global store, with its locals planned through the same machinery as
function bodies. `_start` executes those plans in source order before setup and
lifecycle scaffolding. This keeps value blocks, source-defined helper calls,
and allocating standard-library operations on the normal lowering path rather
than maintaining a second constant evaluator. Operational analysis tracks
transitive global reads and writes alongside effects and availability; semantic
validation rejects any initializer that can observe another global or runtime
context, mutate global state, or suspend, so execution order cannot expose
partially initialized state.

GC type construction also returns an explicit `GcLayout`. The map is the sole
source of physical indices and reference/storage types for recursive GC fields,
globals, generated function signatures, ordinary/async locals, aggregate
instructions, memory decoding, settings initialization, runtime helpers, and
failure propagation. The fixed built-in type conversion rejects dynamic GC
types, preventing semantic IDs from being mistaken for physical Wasm indices.
`GcLayout` compactly orders only the reachable structs, enums, arrays, Options,
and Results and exposes that same order to type encoding. Removing an earlier
semantic layout therefore remaps all physical Wasm indices centrally without
changing individual emitters.

[`src/codegen/module_assembly.rs`](../src/codegen/module_assembly.rs) is the
final binary boundary. It receives completed type, import, function, global,
code, and static-data plans; adds the planned memory, `_start`/`update` exports,
and SplitScript metadata; and serializes sections in canonical Wasm order.
The top-level encoder is now a deterministic orchestrator of these plans.

The checker borrows syntax immutably: inferred array layouts are returned as
part of `CheckedProgram` and passed explicitly to code generation rather than
being appended to the parsed AST.

[`src/wasm_ir.rs`](../src/wasm_ir.rs) is the first backend-owned lowering
product. Its bodies contain structured statements plus explicit
`Fallthrough`, `Return`, and `Suspend` terminators. A suspend records whether
its source operation is `await` or `retry` and owns its continuation, so
async lifecycle state-machine construction no longer rediscovers suspension
boundaries from syntax or typed HIR. The same product plans declared
values, match inputs and payload bindings, and numeric-intrinsic scratch locals
with semantic `TypeId`s. Code generation assigns their concrete Wasm local or
GC-frame indices.

Unity metadata demand is derived from resolved, profile-filtered reachability
before generated provider preparation becomes a root. Live field reads,
complete snapshot reads, instances/components, and automatic shape evidence
retain their required bindings. The backend privately prunes unused generated
lookup statements and binding-struct fields, then rebuilds HIR and Wasm control
flow with the same semantic identities. This removes lookup awaits and their
frame storage together without mutating the checked products used by tooling
or eliminating user-authored effects. Static-read transaction caches also
exclude unreachable reads.

Async storage is selected by backward liveness over the lowered suspension
segments. Every `Suspend` records, in deterministic declaration order, the
source locals required by its continuation. The generated GC frame contains
the union of those per-suspension sets; values used only before a suspension,
defined after it, or overwritten before their next use stay in ordinary Wasm
locals. Await bindings are frame-backed only when a later segment reads them.
Keeping the per-suspension sets in the IR also permits future physical slot
coalescing without making the encoder reconstruct data flow.

Suspending `onAttach` and `whileAttached` bodies own an `AttachmentLifetime`
cancellation region representing the logical state-provider attachment. Awaited
catalog operations whose normalized `OperationSemantics` cancel on process
close attach their `Suspend`
terminator to that region. The generated runtime consumes the body-level region
when the provider detaches: it atomically resets both attach readiness and the
entire continuation frame. Process closure also releases the host handle;
emulator mapping invalidation retains it for provider rediscovery. A later
attachment therefore enters the first suspension segment with fresh values. This ownership is
represented before encoding rather than being rediscovered separately by each
process or Unity intrinsic.

The checker consumes that same normalized catalog query to decide whether an
operation is awaitable and whether it is valid in the current lifecycle. For
example, a process read in `onDetach` is rejected because the catalog requires
an attached process. After typed HIR is built, `OperationAnalysis` computes a
fixed point over resolved user-function and method calls. The process
requirement therefore propagates through forward calls, methods, and recursive
call graphs, and the action boundary rejects the outer invalid call. The result
is exposed by `CheckedProgram::effects` to hover and other semantic queries. The
public catalog rendering query similarly prevents documentation and LSP clients
from reconstructing these rules independently.

Async control flow is no longer encoded from source statement indices. Lowering
assigns every suspension a stable poll state and a distinct continuation state,
then the encoder emits one dispatcher loop over those states. Reaching a
suspension sets
the poll state before invoking the host operation, so a pending poll resumes at
the operation itself rather than replaying preceding assignments, conditions,
or side effects. A successful poll selects its continuation and redispatches in
the same runtime tick. Conditional branches use continuation-style lowered
tails, allowing a suspension inside either branch to rejoin code after the
conditional without source-level special cases.

Source-defined async calls have a split initializer/poll ABI. The initializer
captures the receiver and arguments exactly once and returns an `async T`
reference. Every reachable generic `FunctionInstance` receives its own final
GC frame subtype containing a mutable program counter, stable producer tag,
parameters, locals and compiler temporaries live across suspension, nested
future handles, and a typed completion slot. The erased `async T` supertype
contains only the state/tag header used at storage boundaries; primitive
completion values remain unboxed in concrete frames. A poll function takes its
concrete frame and returns `0` for pending or `1` for ready. On completion the
counter becomes the completed sentinel, making repeated polls idempotent.

Awaiting an arbitrary future stores that identity in the caller's frame,
dispatches its producer tag to the matching typed poll function, copies the
ready completion into the continuation destination, and releases only the
caller's temporary child reference. Source functions and intrinsically
suspending catalog calls are both concrete producers under the same erased
header. Intrinsic leaf frames capture receivers and runtime arguments once and
reuse the ordinary suspension emitter for host polling; literal-only arguments
remain static data. The original future can remain in a local or aggregate and
be awaited again. Process closure drops the host-owned root frame and therefore
its complete nested continuation tree. Semantic validation prevents
attachment-lifetime future references, including references nested in aggregates,
from escaping into globals.

`[T]` `for` loops lower through compiler-owned iterable, `u32` index, and
element-binding values. Ordinary bodies become structured Wasm block/loop
control flow; the iterable is evaluated once and the index advances before the
body so `continue` cannot repeat an element. A body containing suspension uses
dedicated async header and exit states. Liveness retains the iterable and index
across every poll and retains the current binding only when a suspended
continuation needs it, so an `await` neither reconstructs nor restarts the
iteration.

`nextTick()` is a lowering intrinsic rather than a host call. Reaching it stores
its poll state and returns pending immediately; dispatching that poll state on
the next update selects the continuation without replaying the preceding
segment. Its catalog cancellation behavior still attaches it to the same
attachment-lifetime region as process-backed futures.

`retry expression` uses the same state machine but is not a catalog intrinsic.
The checker requires the expression to have type `T!`, the Wasm IR records the
`Retry` suspension mode and plans a typed `T!` scratch local, and polling
re-evaluates the ordinary lowered expression. An error returns pending; a
success extracts the `T!` payload into the continuation frame when the
binding remains live. This makes user-defined `T!`-returning functions
retryable without giving the backend special knowledge of those functions.
Immediate process-memory APIs consistently expose this path: generic reads,
pointer following, relative-address decoding, and managed-string decoding all
return `T!`. Their low-level helpers may still use zero or null sentinels at the
ABI boundary, but expression lowering converts those sentinels exactly once
into the standard `T!` GC representation. Discovery APIs such as module,
signature, and low-level Unity metadata lookup remain intrinsic suspensions:
temporary absence means pending, while process closure cancels their region.
Higher-level discovery such as `Unity.il2cpp` is an ordinary source-defined
async function composed from those bounded suspension points.

The Wasm IR now owns a stable-ID expression plan alongside its control-flow and
storage plans. Scalar literals, resolved value/member paths, unary and binary
operations, explicit casts, result `TypeId`s, and implicit `T?`/`T!` lift
edges are copied into backend-owned nodes. Ordinary emission consumes those
nodes without asking typed HIR which operation or path was selected. String
literals and interpolation, compile-time signatures, arrays, and resolved
struct/enum constructors have moved to the same plan; static string/signature
data collection consumes it as well. Calls now carry their argument IDs and
complete resolved target in this plan: user-function IDs, method receiver
paths, standard-library item IDs, and inferred generic arguments are no longer
recovered from typed HIR by ordinary or suspending emission. Expression-valued
`if` uses a native branch node with typed result arms. `else` fallback nodes
preserve value versus returning branches, while postfix `?` nodes carry the
exact inferred `T!` boundary used for failure transfer. `match` uses
resolved Wasm-IR arms: enum patterns retain stable variant and payload
binding IDs; `None`/`Some` patterns retain `T?` state plus the present-value
binding; `Ok`/`Err` patterns retain `T!` state plus the corresponding
value or error binding. Literal/wildcard patterns retain guards and result
expressions. Binding extraction and guard evaluation are nested behind the
pattern condition, so an inactive enum variant or wrapper state is never
accessed. The temporary `TypedHir` expression variant is gone, calls are
encoded directly from their Wasm-IR target, and ordinary expression emission
no longer receives typed HIR. This completes the expression-plan migration
without introducing a second expression identity or a general SSA optimizer.

Synchronous-body local planning also recognizes repeated projections of
immutable managed snapshots already stored in `current` or `old`. Three or
more uses can share the state field itself; two or more uses can share a direct
field projected from that managed snapshot. Both become compiler-owned Wasm
locals, avoiding repeated state and managed-object GC field loads. This is
intentionally a narrow snapshot-stability rule rather than general
common-subexpression elimination: live process reads may observe concurrent
mutation and are never shared by this pass, async bodies may cross a tick and
are excluded, and assigning a `current` field disables every reuse rooted at
that field throughout the program. `old` remains immutable by construction.
The plan is recorded as an explicit Wasm-IR local purpose, so emission does not
rediscover the optimization from source syntax.

Process-read failure will remain expressed through the language's real `T!`
result type and ordinary result control flow rather than through a new
backend-only failure channel. This keeps semantic facts and backend physical
types separate.

Error payload construction is planned by a conservative whole-program demand
analysis after reachability and specialization. Every `T!` keeps the same GC
layout and failure flag; only its nullable payload field may be left empty when
no reachable operation can observe it. Demand includes `Err(error)` pattern
bindings, result equality, derived debug/display formatting, and errors
forwarded through `?` or managed-result adapters. Demand is joined across each
existing result identity: the backend never clones a function merely because
one caller discards its error while another observes it. Compiler-generated
messages and inert source string literals can therefore disappear completely;
other source error expressions still execute and are dropped when their value
is unused, preserving side effects. This pass changes neither source semantics
nor the physical result ABI and remains independent of any future structured
error design.

`T?` and `T!` are first-class constructed type annotations. Parsing assigns
stable layout identities, inference preserves their value types, and the
semantic store exposes `TypeKind::Option` and `TypeKind::Result` for the
backend and editor tooling. The backend emits a monomorphized Wasm GC
struct layout for each used wrapper. `None`, `Some(value)`, `Ok(value)`,
`Err(message)`, and implicit optional/successful lifts are ordinary typed
values: every lift is recorded as
a source/target `TypeId` conversion during checking, copied onto its Wasm-IR
expression edge, and then lowered to GC construction. Payload-bearing `Some`
and `Ok` constructors allocate provisional layouts when no expected wrapper is
available. After inference resolves their payload types, equivalent provisional
and declared layouts canonicalize to one stable nominal Wasm GC identity.
Payload-free `None` and success-type-free `Err` still require expected-type
context. Wrapper match patterns are resolved to stable semantic `T?`/`T!`
layout IDs before lowering; the backend does not rediscover their meaning from
source spelling. Fallible control flow remains a separate semantic step;
none of these physical layouts introduce a private failure protocol.

## Debug metadata and source identity

Debug builds contain a WebAssembly `name` section derived from final function
indices. It names host imports, generated helpers, concrete generic
specializations, async initializer/poller pairs, state readers and transforms,
lifecycle blocks, `_start`, and `update`.

They also contain initial DWARF v5 compilation-unit, subprogram, and line-table
metadata. Source-backed function bodies map emitted instruction boundaries to
the original expression and statement lines, including bare control-flow
statements and source statements moved into async poll continuations.
Compiler-generated scaffolding deliberately has no source location. Source
parameters and primitive scalar locals in direct synchronous functions receive
Wasm local names, DWARF base types, declaration metadata, lexical visibility
ranges, and `DW_OP_WASM_location` expressions. Values moved into async GC
frames are omitted until location changes across suspension can be represented
honestly. Reachable source globals receive WebAssembly global names and scalar
globals receive `DW_OP_WASM_location` global locations.

Source identity flows through every compiler stage: the CLI records an
absolute `.split` path, the extension records VS Code's native file path, and
non-file editor documents retain their URI. APIs that intentionally compile
only an in-memory string use `input.split`. Release builds omit both the name
section and every `.debug_*` section.

## Numeric parsing implementation

The Wasm implementation uses allocation-free Simple Decimal Conversion with a
reused 768-digit scratch buffer. It does not parse through an intermediate
`f64` when the target is `f32`, avoiding double rounding, and it does not call a
locale-sensitive host routine.

## Standard-library catalog and tooling model

The library surface is described by a backend-independent catalog. Each entry
has a stable ID, canonical name, callable kind, generic type scheme,
constraints, documentation, parameter documentation, examples, related items,
deprecation information, and an implementation key.
Type checking resolves source calls to these IDs. WebAssembly generation and
async lowering only handle the resolved implementation key; neither resolves
source names. Operational metadata has exactly one authority per implementation:
the closed intrinsic registry for intrinsic leaves and compiler analysis for
source-defined bodies.

Every callable has exactly one implementation: either a trusted intrinsic
declaration ending in `;` or an ordinary SplitScript body. Intrinsics remain
necessary for host ABI access, physical representations, runtime helpers, and
special lowering primitives. Higher-level operations are composed in library
source and checked and lowered through the same resolver, inference, typed HIR,
effect analysis, reachability, Wasm IR, and encoder as user functions. Merely
being a library body grants no privilege; it can reach host behavior only by
calling a validated intrinsic declaration.

The implemented synchronous body tier uses ordinary inferred function
templates and demand-driven monomorphization. A catalog call supplies its exact
concrete receiver, parameter, and result signature, so constructed layouts and
source-owned types use the same `FunctionInstance` machinery as user-defined
generic functions. Reachability emits only called concrete instances.
`Numeric.clamp` composes the primitive `min` and `max` operations, while
`[T].isEmpty()` composes `length()` across arbitrary array element types.
`[T].contains(value)` and `[T].indexOf(value)` are source-defined search loops;
their declarations add `Equatable` only to the inherited `T` used by those
methods, leaving operations such as `length()` available for every element
type. None of these helpers has dedicated intrinsic dispatch or backend
lowering.

Bodies may perform any operation reachable through their permitted intrinsic
leaves. All `Duration` constructors are source-defined: their arithmetic and
physical GC-struct construction live in `standard.split`, so the type has no
intrinsic operations. This includes zero, exact integer and fractional unit
constructors, frames, and parts. Capability-directed implementation cases let
one public `T: Numeric` constructor retain separate `Integer` and `Float`
source bodies; selection occurs only after generic specialization and is not a
general user-visible overload system. A catalog-owned body may construct its
owning GC struct, including runtime-private fields, without making that
constructor syntax available to user code or unrelated library functions.
`address.offset` widens its argument and delegates to primitive full-width
address addition. `timer.isRunning` and `Module.readRelative32` demonstrate
effectful composition over timer and process intrinsics entirely in library
source.

Each independently owned catalog graph runs a standalone compilation of its
source bodies once. Ordinary typed call-graph analysis derives a canonical
effect set, availability, attachment requirement, suspension, and cancellation
metadata without consulting a user program. The immutable result is cached on
the graph and supplies checking and editor queries. Source entries contain no
fake `pure` fields that a consumer could accidentally treat as authoritative.
Normal user compilations still type-check the injected bodies and verify their
inferred metadata against the standalone result. Actual suspending library
bodies remain the next body tier.

`CompilerContext` owns the selected catalog through a cloneable
`StandardLibrary` handle backed by an immutable `Arc` graph. Compiler passes
borrow that graph; parsed, checked, tooling, and backend products clone the
owner only when they must retain it. The bundled graph is cached, while tests
also construct and inject an independently owned validated graph through the
same context path. This makes catalog identity and lifetime explicit without
requiring each pass to reconstruct global state. Its declaration producer is
the build-only privileged SplitScript loader: `stdlib/standard.split` is parsed
and source-validated once during the Cargo build, then emitted as typed catalog
data. Callable body blocks are retained only for source-defined
implementations. The compiler injects one hidden inferred function template per
catalog source body after the user file and parses it with the ordinary program
parser. Resolved catalog call signatures drive concrete instances through
ordinary demand specialization. Public syntax, HIR, semantic
iterators, symbols, and editor features retain the user-only view, while the
backend sees the complete unit. Generated function names use a reserved prefix
that user programs cannot declare.

Effects distinguish ordinary process reads from operations that require an
attached process, suspend or retry, and cancel when that process closes.
`RequiresAttachedProcess` and `CancelsOnProcessClose` are catalog facts shared
with type checking and async lowering. `StandardLibrary::operation_semantics`
normalizes them into `SuspensionKind`, `CancellationKind`, availability, and a
process requirement; `render_operation_semantics` provides their common human
presentation. Catalog validation rejects incompatible declarations such as a
non-awaitable cancellable item. Documentation and editor tooling consume these
queries directly rather than reading implementation-specific storage.

Language-only constructs live in a sibling `LanguageCatalog`, rather than as
fake standard-library functions. It gives keywords such as `await`, `retry`,
and `as`; lifecycle actions; source-spellable built-in and constructed types;
wrapper/literal syntax; snapshot roots; compiler-provided fields and
the settings DSL stable IDs, compact source forms, documentation, and checked
examples. Standard-library namespaces, nominal types (including
`TimerState`), fields, variants, and callables remain exclusively in the
standard-library catalog. Both catalogs
share the generic documentation and example model in `catalog.rs`, so generated
reference pages and LSP hover/completion can consume one metadata shape while
preserving the semantic difference between syntax and callable APIs.

Resolved user functions and methods inherit the attached-process requirement
through a fixed-point call-graph analysis. This includes recursive call graphs;
no source annotation is required. The inferred result is available through the
checked compiler product and prevents a process-dependent helper from hiding an
invalid `onDetach` operation.

Catalog signatures no longer depend on parser AST types: built-in catalog
types use `BuiltinType`, and a checked call exposes its inferred generic
arguments as interned `TypeId` values. `TypeStore` lets documentation and editor
clients inspect `TypeKind` directly, including constructed arrays, without
depending on inference-only AST variants. Inferred declaration types are also
semantic facts: `SemanticModel::value_type` covers globals, parameters, locals,
await bindings, state fields, settings, and match payload bindings, while
`SemanticModel::function_result` covers function results. Optional source
annotations remain optional in checked syntax instead of being overwritten by
inference. Struct fields, enum payloads, and array elements likewise publish
their resolved `TypeId` layouts through `StructFieldId`, `EnumVariantId`, and
dedicated constructed-type identities; WebAssembly GC layout construction reads those
semantic queries rather than the AST annotations. `TypeKind::Array` retains
its `ArrayTypeId` layout identity, element `TypeId`, and optional exact length,
so code generation never needs to reconstruct this information from syntax.
`TypeKind::Set` similarly retains the source generic-application identity,
element type, and compiler-selected backing-array identity.
`TypeStore` has no
parallel legacy type representation: Wasm storage/value selection lowers
`TypeId` / `TypeKind` directly into backend-local physical categories.

The semantic `TypeStore` is created before inference. Core primitives,
standard-library types, and source struct/enum types enter inference as their
canonical `TypeId` and retain that exact identity through checked publication,
rather than passing through parallel enums and post-inference conversion
tables. Only inference variables and temporarily unresolved `[T]`, `T?`, and
`T!` constructor terms remain solver-local. Namespace, nominal-type, field,
variant, and callable IDs are generated from the same declaration rows as their
names, ownership, documentation, and representation metadata.

Catalog-declared capabilities are executable contracts rather than labels.
Capabilities can build on other capabilities in the privileged source. For
example, `Numeric<T: Equatable>` makes equality part of the numeric
contract, while `Integer<T: Numeric + Display>` makes every integer numeric,
equatable through that numeric relationship, and displayable. The
compiler follows this hierarchy transitively for type checking and method
completion, while inferred constraints and rendered signatures retain only the
strongest non-redundant capabilities. Concrete integer types consequently
declare `Integer` once instead of separately repeating those memberships.
Capabilities may also declare structural method requirements in privileged
source. `Display` requires `fn toString() -> String`; user structs and enums
derive a structural implementation by default, while a corresponding
`fn Type.toString()` method overrides it. The checker matches the semantic
receiver, parameters, and result against the catalog requirement, and implicit
conversions enter the ordinary source call graph so effects and reachability
remain accurate. Structural shape is shared with equality analysis, and the
backend plans both kinds of generated helper from reachable uses rather than
emitting one for every declaration. This intentionally avoids requiring
`impl` blocks in short user scripts.

Structural requirements may include ordinary generic method parameters and
associated result types. The checker infers a source implementation only when
all required methods match as one complete contract, including generic bounds;
it then publishes the inferred associated types in the semantic model. Generic
projections and concrete source projections use the same inference relations,
so a `for` loop over a source-defined `Iterable` obtains its element type as
`Iterable.Iterator.Item` without a checker-only guess or a duplicate
`Iterable.Item` declaration. Incomplete or mismatched contracts are recovered
during inference and diagnosed at the capability use site, with the candidate
method identified when available. The catalog remains the single source of
truth for the contract itself.

The initial public policy deliberately distinguishes behavioral and
representation-sensitive capabilities. Exact ordinary source methods can
satisfy behavioral contracts such as iteration and generic memory-reader
dispatch. `Debug` is universally derived, `Display` may be overridden by an
exact `toString` method and otherwise falls back to `Debug`, and `Equatable`
remains structurally derived. Physical `MemoryReadable` layout and the numeric
capability hierarchy remain compiler-sealed. SplitScript does not expose
`impl` blocks or user-declared capability syntax; those features remain
deferred until a concrete script needs them.

Nominal standard-library types remain explicit. They connect a parameterless
`String`-returning source method to `Display` with the private `@display`
annotation. The generated type declaration owns that implementation identity,
catalog validation checks its receiver and signature, and reachability treats
conversions as calls to the ordinary hidden library body.
`FileVersion.toString()` is the first such implementation: casts,
interpolation, `print`, and `setVariable` all dispatch to it, while codegen has
no `FileVersion` formatting case.
`MemoryReadable` GC structs derive their naturally aligned field layout from
catalog field declarations through the same semantic-`TypeId` layout engine as
source structs. `Equatable` catalog structs similarly receive generated
structural equality helpers whose dependencies close over nested declared
fields. Catalog validation rejects readable or equatable declarations whose
representation and fields cannot satisfy those contracts. A test-only ordinary
struct, with no intrinsic implementation, passes through name resolution,
checking, memory layout, equality, hover, completion, and Wasm GC generation;
this guards the promise that future ordinary types do not need compiler-wide
type matches.

Source annotations, cast targets, and integer suffixes use the separate,
inference-free `ast::TypeRef`. It contains no catalog or resolved nominal IDs;
lowering publishes those as `types::ResolvedTypeRef`. Parser-owned constructed
type expressions and checker-owned resolved layouts are distinct as well.
Expressions have no checker-owned
type slot: pending types are recorded directly by `ExprId` and resolved when
the semantic model is finalized. The dedicated inference context owns type
variables, union/find unification, requirement composition, integer-literal
bounds, numeric defaulting, and inferred array layouts. The checker translates
solver failures into source diagnostics; syntax and editor APIs never expose
these temporary types.

Free functions and type-directed methods are exposed as
declarative `CallCandidate` values. `process.read(address)` leaves its named
generic parameter open for bidirectional inference, while an explicit generic
call such as `process.read<u16>(address)` seeds `T = u16`. Method candidates
carry their receiver type scheme and
capability constraints, and retain the selected receiver value through Wasm
lowering. The checker uses
one catalog-call path to instantiate that scheme and submit receiver,
argument, expected-result, and capability constraints to the same inference
context used by ordinary expressions. Candidate discovery itself commits no
new constraints, and ambiguous applicable candidates produce a diagnostic
before a candidate is committed. The catalog validates duplicate callable
shapes so accidental indistinguishable overloads are caught by tests.
Binary and unary operator syntax uses this same path. Privileged declarations
bind methods to operator identities with `@operator`, so `value + other`,
`value == other`, `-value`, and `!value` resolve to the same items exposed as
ordinary documented methods. Short-circuit `&&` and `||` remain language-level
control flow rather than eager method calls.

Read-only queries cover item enumeration, exact canonical lookup, path and
method candidate lookup, signature rendering, and the documentation stored on
each item. `StandardLibraryDocumentation` turns those facts into one canonical
generic or call-site-substituted reference entry. Completion, hover, signature
help, the browsable reference, and terminal documentation already consume that
entry; a future machine-readable exporter must use the same payload.

Every parsed expression also has a stable per-program `ExprId`. Checked
expression types and standard-library call resolutions are queried by this ID,
not by source spans. Syntax expressions are never mutated with inferred types;
WebAssembly lowering reads the semantic `TypeId`. This keeps syntax suitable
for future incremental editor analysis while inference internals continue to
be migrated.

User functions and methods likewise have stable per-program `FunctionId`
values. `ResolvedCall` distinguishes user functions, user methods, and catalog
items, so editor navigation and WebAssembly lowering consume the same checked
call target. Method-call facts also retain the resolved receiver root and its
semantic type, allowing hover/navigation and lowering to agree about the value
on which a method operates. Backend callable dispatch is entirely ID-based;
member resolution is being migrated independently as described below.

Globals, parameters, ordinary locals, awaited bindings, state fields, and
settings now carry `ValueId` values as well. Path expressions publish a
`ResolvedValue` root for go-to-definition, including the temporal distinction
between `current`/`old` and `settings`/`oldSettings`. Backend reads use ID-keyed
Wasm locals, continuation-frame fields, state slots, setting globals, and user
globals. Assignment statements have stable `AssignmentId` values and publish
their `ValueId` targets too, so backend writes use the same ID-keyed storage.
Match payload bindings also have `ValueId` values. Method receivers are lowered
from their semantic `ResolvedValue` roots, so compiler-created receiver paths
and their former local/global/setting name maps are gone. Structs, enums,
struct fields, enum variants, and compiler-provided fields have distinct typed
IDs. Path expressions publish ordered `ResolvedMember` chains, while struct
literals and enum constructors publish the selected field/variant IDs. The
backend therefore does not reinterpret member-path or constructor spelling.
Match arms have `PatternId` values, and choice options have
`SettingChoiceOptionId` values; their resolved enum variants, including choice
defaults, are semantic queries consumed directly by match and settings
lowering. Variant text remains in settings lowering only as the external value
understood by the host settings map.

The same catalog drives generated documentation, LSP completion, hover, and
signature help. Tools can parse and check a program and inspect its
semantic call resolutions without constructing the WebAssembly backend. All
currently implemented functions and type-directed methods—including numeric,
array, process, duration, address, and Unity APIs—are catalog-backed. Compiled
catalog examples keep their user-facing snippets separate from complete
validation programs. A line prefixed with `# ` inside a `splitscript` example
fence participates in parsing, type checking, semantic highlighting, and
definition resolution, but is omitted from rendered documentation. This is the
standard way to supply lifecycle scaffolding and local declarations without
distracting from the focused snippet. The generated reference maps semantic
spans from the complete fixture back onto the visible lines, so links never
depend on identifier text alone.

Catalog entries can also declare a use obligation with
`@mustUse("reason")`. A bare expression statement that discards such a return
value produces a warning while compilation and Wasm generation still succeed.
The marker may be attached to a callable for an operation-specific explanation,
or to a type form such as `T?` and `T!` so the obligation
follows values returned by user functions as well. `String.toAsciiLowerCase`,
`String.replaceAll`, and `String.split` use callable-specific reasons because
strings are immutable: they return transformed values and never change their
receivers. The ASCII conversion's runtime helper reuses the receiver when no
byte changes, while still conservatively advertising its possible allocation
effect. The checked compiler product retains these warnings, and the database,
LSP, compiler service, CLI, and watch workflow all
publish the same structured diagnostics.


## Syntax traversal

[`src/visit.rs`](../src/visit.rs) is the single exhaustive definition of AST
child traversal. `Visitor` provides immutable preorder traversal for analysis
and collection. `Folder` provides the equivalent in-place mutable traversal
for syntax rewrites. Both cover top-level declarations, blocks, statements,
expressions, match arms and patterns, and written type references.

Each hook descends by default. When overriding a hook, call its corresponding
`walk_*` or `walk_*_mut` function to continue into children. Deliberately omit
that call to establish a boundary. For example, an analysis interested only in
statements can override expression traversal with an empty method, while an
async pass can stop below a suspension statement.

Compiler passes should use these utilities when they need ordinary structural
traversal. A bespoke recursive walker remains appropriate when traversal alone
does not express the algorithm. Definite-return analysis is one example: an
`if` returns on every path only when both branches return, so it needs explicit
control-flow composition rather than a flat node visit.

The typed-HIR traversal users include:

- string and signature literal collection;
- async continuation-frame local collection;
- ordinary Wasm local collection;
- match and numeric-intrinsic temporary planning;
- backend body emission through stable child IDs.

Typed HIR has a sibling `TypedVisitor` whose root traversal covers global
initializers, expression-backed state fields, function bodies, and action
bodies exactly once. AST visitors must not be extended to inspect semantic side
tables implicitly; passes that need types or resolutions should carry the
appropriate checked product explicitly. That keeps syntax traversal useful to
the formatter and early LSP features, which run before successful type checking.

Lowered Wasm IR has its own `wasm_ir::Visitor`. It traverses structured blocks,
statements, terminators, and the expression DAG through stable IDs. The sibling
`visit_expression_children` query is the one exhaustive definition of direct
expression edges for analyses that own a deduplicating worklist. Reachability,
local/scratch planning, and suspension-frame liveness use these APIs; adding an
expression form does not require another recursive child-shape match. Dependency
and static-data planning scan the flat reachable expression table and match only
the semantic payloads they consume.

## Formatting

[`src/formatter.rs`](../src/formatter.rs) exposes `format_source` as a reusable
library operation. It first invokes the strict compiler parser, then lays out
the parser's lossless token and trivia stream; there is no formatter-specific
lexer or grammar to drift from the compiler. Literal spellings, ordinary line
and block comments, and `///` setting documentation remain source-backed while
whitespace, indentation, and operator spacing become canonical. Invalid input
returns the compiler's structured parser diagnostics instead of producing a
potentially destructive rewrite.

Parsed header spans distinguish block braces from other braces. A one-line
`if`, `while`, function, or lifecycle header keeps its opening brace on that
line. When the header wraps, continuation lines receive one extra indentation
level and the opening brace moves to its own line at the header indentation;
the body therefore remains visibly nested beneath the brace. Formatter tests
require idempotence and reparsing for the main language showcases.

Formatting metadata is derived from parsed syntax rather than guessed solely
from punctuation. Settings, state fields, and ordinary statement spans
establish continuation regions; these stop at nested block braces so block
bodies are not indented twice. Match, choice, and file blocks identify commas
that terminate domain-specific entries. A delimiter pass separately records
line breaks made directly inside each parenthesized or bracketed expression.
Closed comma-separated lists receive a trailing comma when formatted across
multiple lines, while compact lists do not. State fields instead receive a
trailing semicolon because commas already join offsets in their pointer paths.
These rules keep multiline process-read arguments one level inside their call without
over-indenting a call merely because a nested argument is multiline. Braces
in the settings DSL keep each label and `=>` on the same line, including
boolean, text-input, choice, file, choice-option, and file-filter entries; nested block
contents and closing braces are then anchored from that line. Interpolated
string chunks remain byte-for-byte source text, while expressions inside
`{...}` use the ordinary spacing rules. Struct fields and enum variants are
always laid out one per line, including their trailing commas.

The command-line frontend exposes the same operation as `splitc fmt <file>`.
It writes only after parsing and formatting succeed and skips unchanged files.
`splitc fmt <file> --check` performs no write and returns failure when the
canonical text differs, making it suitable for CI.

The native frontend derives its command model, validation, and help from Clap;
the dependency is native-only and is not pulled into the embedded compiler
Wasm. `splitc --help` and `splitc -h` print the complete command surface, while
`splitc help watch`, `splitc watch --help`, and their formatting equivalents
show command-specific behavior. `splitc --version` and `splitc -V` print the
package version. These successful queries use standard output; malformed or
incomplete invocations keep exit status 2 and a concise standard-error usage
message, so scripts can distinguish discovery from misuse.

## Language server

[`src/lsp.rs`](../src/lsp.rs) owns transport-independent routing and lifecycle
state, while typed incoming DTOs, open-document ownership, compiler-to-LSP
conversion, and protocol tests live in the sibling `lsp` modules. Malformed
request envelopes and method parameters therefore have one decoding boundary
and consistently produce JSON-RPC `-32600` and `-32602` errors. The
[`src/bin/splitls.rs`](../src/bin/splitls.rs) provides standard
`Content-Length`-framed JSON-RPC over stdin and stdout. This remains a module
and sibling binary in the compiler crate for now, following the architecture
rule that crate boundaries should be introduced only after interfaces have
multiple proven consumers.

Each open URI owns one revisioned `CompilerDatabase`. Full-sync open and change
notifications update that database, publish diagnostics tagged with the
document version, and convert byte spans to LSP's UTF-16 line/character
positions. Diagnostic codes, severities, label messages, notes, and structured
fix edits come from the compiler's existing diagnostic model. Formatting
requests call the cached database formatter and return one whole-document edit;
syntax-invalid documents retain their diagnostics and receive no destructive
formatting edit. The server advertises only implemented capabilities during
`initialize` and observes the standard `shutdown`/`exit` lifecycle.

Completion recognizes type grammar before falling back to ordinary expression
scope. One lexical type-prefix parser remains usable while the recovering AST
contains a missing or partial type and covers annotations, return arrows,
casts, enum payloads, nested arrays and generic arguments. All of these sites
consume one catalog-backed candidate builder, so primitives, standard-library
types and named constructors, source structs and enums, and structural type
syntax cannot drift between grammar positions. Declaration-role checks keep
struct-literal fields and other value colons on the expression-completion path.

Semantic highlighting is a compiler query rather than an LSP-specific AST
walk. [`src/highlight.rs`](../src/highlight.rs) merges lossless lexer tokens,
syntax declarations, and semantic resolutions into a sorted index of byte
spans, token kinds, and modifiers. This gives domain constructs—including
settings titles, state fields, lifecycle blocks, signatures, enum variants,
and debug-erased regions—stable classifications that future editor clients can
share. Recoverable syntax errors retain lexical and declaration highlighting;
valid syntax with type errors additionally uses the recovering semantic model.
The LSP layer only splits multiline spans and delta-encodes UTF-16 positions
according to the advertised semantic-token legend.

Inferred-type inlay hints have the same compiler-owned boundary.
[`src/inlay_hints.rs`](../src/inlay_hints.rs) walks resolved declarations and
emits byte positions plus source-shaped type labels only where the source omits
an annotation. It covers globals, locals, parameters, function results, state
fields, and suspension bindings, filters to the client-requested range, and
remains available through the recovering semantic snapshot. Hover and inlay
hints both use [`src/type_display.rs`](../src/type_display.rs), so nominal and
constructed types cannot acquire different spellings between editor features.
The LSP adapter only converts positions to UTF-16 and marks each result as a
standard type hint.

Completion follows the same ownership rule. [`src/completion.rs`](../src/completion.rs)
returns editor-neutral candidate kinds, snippets, documentation, and byte-span
replacements. Root candidates combine the `LanguageCatalog`, `StandardLibrary`,
source declarations, and bindings visible at the cursor. The lexical walk
includes function parameters, preceding ordinary and suspension bindings,
enclosing-block locals, and active match-pattern bindings while preserving
shadowing and excluding later or sibling declarations. Root completion also
uses the same exhaustive lifecycle attachment predicate as validation. In a
detached context it omits the selected `process`/`gba` provider, catalog
operations that require an attachment, and user functions whose transitive
`OperationAnalysis` carries that requirement. A temporary `None` probe replaces
an unfinished root identifier when necessary, retaining those function facts
while the user is typing instead of introducing a second effect analysis.
Member completion handles snapshots and nominal enum names directly, then uses
a temporary source probe with the unfinished member suffix removed to recover
the receiver's semantic `TypeKind`. That supports
struct fields, compiler-provided fields, user methods, and catalog methods even
for the normal mid-edit spelling `receiver.`. The LSP adapter converts the
replacement span to UTF-16 and emits standard snippet text edits; it does not
reimplement candidate or documentation rules.

Source navigation is likewise a compiler query. `DefinitionIndex` records the
exact declaration identifier and every exact identifier-token reference by
stable `ValueId`, `FunctionId`, `StructId`, `StructFieldId`, `EnumId`, or
`EnumVariantId`. It covers declarations, annotations, patterns, paths, calls,
constructors, struct literals, and assignment targets, including recovered
semantic models. `CompilerDatabase::definition_at` resolves the token under the
cursor, while `CompilerDatabase::references_at` filters the same index by
identity and optionally excludes its declaration. The LSP adapter emits
single-document `Location` values and performs only byte-to-UTF-16 conversion.
Editor selection is centralized in the lossless `SourceDocument` but reflects
the two LSP position conventions. Hover is character-based, so an exact token
beginning at the pointer wins over an identifier ending there. Definition,
references, and rename are caret-based and keep selecting a word whose
half-open span ends at the caret, including before adjacent punctuation;
semantic postfix `?`/`!` tokens retain exact precedence. Neither policy skips
whitespace, and all remain well-defined at line endings and EOF.
Standard-library and language-catalog definitions navigate to their exact
`splitscript-docs:` page rather than exposing the privileged implementation in
`standard.split`. User declarations retain ordinary source locations, so this
documentation target does not weaken source identity.

Rename builds directly on that navigation boundary. `rename_target_at` returns
the exact selected occurrence for LSP `prepareRename`, while `rename_at`
validates the proposed ASCII identifier and rejects language or standard-library
root names. It then applies the proposed edits to an in-memory candidate,
requires the candidate to type-check, and maps every old reference span through
the edit deltas. Each mapped reference must retain its original stable source
ID. Consequently a rename cannot silently capture a global, parameter, local,
field, type, or callable even when the captured program would still be valid.
The LSP layer only converts the returned spans into one same-document
`WorkspaceEdit`; catalog symbols are not renameable.

[`src/symbols.rs`](../src/symbols.rs) owns the editor-neutral document outline.
It converts the recovered syntax tree into source-ordered symbols even though
the AST stores declarations by category. State and settings are domain
containers, setting-title spans recover their nested hierarchy, structs and
enums contain fields and variants, and methods and lifecycle blocks retain
distinct kinds. `CompilerDatabase::document_symbols` caches this result; the
LSP adapter recursively converts its byte ranges and selection ranges to UTF-16
`DocumentSymbol` values.

Code actions require no parallel repair model. Compiler diagnostics already
carry titled `DiagnosticFix` values with applicability and one or more exact
`TextEdit`s. The LSP code-action endpoint filters current compiler diagnostics
to the requested source range, honors `context.only`, and maps those edits into
same-document quick-fix workspace edits. The diagnostic attached to an action
is rendered through the same conversion used by publication, keeping its code,
notes, labels, and fix metadata consistent.

[`src/insight.rs`](../src/insight.rs) provides the corresponding hover and
signature-help queries. Hover first uses the shared definition and position
analysis to select a stable source, standard-library, or language-catalog
identity. Source definitions are joined with semantic type facts, so globals,
locals, parameters, state and setting fields, struct fields, functions and
methods, structs, enums, and variants show their inferred source-level types.
User-function and method hover additionally consumes the interprocedural
`OperationAnalysis`, exposing transitive catalog effects, attachment
constraints, synchronous behavior, and debug-only build availability. Recovery
retains this analysis when type inference succeeds but a later semantic rule,
such as calling an attached-process helper from `onDetach`, rejects the
program.
For a resolved generic call, semantic `TypeId` arguments are rendered back to
source-level names and substituted into the catalog signature. The resulting
Markdown includes parameter documentation, effects and availability, and the
same compiler-validated examples used by generated documentation. Signature
help finds the active unmatched call delimiter and counts commas only at its
own nesting level. If an unfinished method call cannot yet be checked, a
temporary probe removes the partial call, infers its receiver, and selects the
applicable method from `StandardLibrary::methods_for_type`. The LSP adapter only
performs UTF-16 position conversion and JSON shaping.

Standard-library prose is normalized one step earlier in
[`src/documentation.rs`](../src/documentation.rs). A
`StandardLibraryDocumentation` entry contains the canonical generic or
substituted signature, summary and details, parameters, effects, runtime
behavior, deprecation, and compiler-validated examples. It can render the
compact completion description, the signature-help body, or the full hover
payload. An end-to-end JSON-RPC test requests numeric `.clamp` completion,
updates the same open document to a resolved call, and verifies both completion
and hover exactly against the corresponding generated entries. The future HTML
and machine-readable documentation exporters should consume this model instead
of rebuilding catalog presentation.

[`src/documentation/reference.rs`](../src/documentation/reference.rs) joins
that standard-library model with `LanguageCatalog`, `MigrationCatalog`, and the
self-contained maintained guides from
[`src/documentation/bundled.rs`](../src/documentation/bundled.rs) into one
renderer-independent hierarchy. Stable
virtual paths identify language constructs, migration concepts, cookbook
anchors, namespaces, types, capabilities, members, variants, and operators.
Migration pages resolve their canonical targets back through the same language
and standard-library catalogs, while catalog examples retain semantic tokens
and exact definition links, including links from language keywords such as
`fn`, `let`, `await`, and `return`. Documentation prose supports rustdoc-style
intra-doc links such as ``[`Process.read`]`` and ``[`await`]``. A custom label
can name a different target with ``[`*`](operator@Numeric.multiply)``, while optional
Rustdoc-style disambiguators such as `keyword@await`, `method@Process.read`,
`operator@Numeric.multiply`, and `type@Duration` make the intended symbol kind
explicit. Resolution is exact and ambiguity-safe: qualified catalog identities
and unique short names become page-relative links, while an unknown or
ambiguous spelling remains visible for validation instead of silently choosing
a destination. Compact
hover and completion Markdown reduces this reference-only markup to ordinary
code spans. Standard-library declarations, language entries, migration
concepts, and bundled guides author every exact known-symbol mention this way.
Graph validation rejects both unresolved intra-doc links and plain code spans
whose text has an unambiguous documentation identity, preventing new inert
mentions from accumulating. Parameter names and code fences remain ordinary
code because they are not prose references. The LSP serves both the searchable
index and pages; the VS Code client only presents those Markdown documents.
Structured migration diagnostics carry their concept identity through native
CLI, LSP, and embedded compiler responses, so frontends do not infer a
documentation destination from
error prose. Native `splitc docs` resolves an exact catalog title, stable
migration identity, or virtual path and renders the same Markdown. Guide links
are intentionally self-contained: focused snippets explain individual concepts,
while complete `.split` examples are neither bundled nor required navigation.

`cargo xtask docs` renders this exact hierarchy as a static HTML site under the
ignored `target/generated-docs` directory; an alternative repository-relative
output directory may be passed explicitly. The site preserves compiler-produced
semantic SplitScript highlighting and symbol links, and adds shared responsive
navigation and client-side search without introducing another documentation
catalog. Hidden rustdoc-style example scaffolding is removed by the bundled
guide renderer before export.

`cargo xtask docs --check` renders the site in memory and validates every page,
local link, and heading anchor. The repository verification matrix runs that
check, while the documentation workflow regenerates the output and publishes it
to GitHub Pages. No generated reference pages are committed. Future
machine-readable output should remain another rendering mode over the same
hierarchy rather than define another catalog.

Catalog-backed completion and hover results carry the same stable page URI and
append an **Open full documentation** command link. The VS Code clients trust
only that one command in language-server Markdown, and the command opens the
exact page beside the script; other command URIs remain inert. Standard-library
and language go-to-definition responses use the same URI directly.

[`src/documentation/validation.rs`](../src/documentation/validation.rs) checks
the complete rendered graph rather than a hand-picked set of pages. It composes
the three catalog validators, requires complete and unique index metadata,
renders every indexed page, resolves Markdown and semantic-code HTML links
relative to their virtual document, and verifies heading fragments. A stable
fingerprint snapshots the ordered reference index. Native LSP tests compare all
pages with this model, while packaged desktop and browser worker tests exercise
the same page endpoint through the compiled adapter.

## VS Code client

[`editors/vscode`](../editors/vscode) is a thin TypeScript extension around two
isolated services from the same bundled core-Wasm compiler. The accepted
architecture and its native/embedded conformance boundary are recorded in
[ADR 0001](adr/0001-portable-toolchain-workers.md). Activation wiring,
language-client lifecycle, compiler-task management, and the identity-safe
exclusive task state are separate modules. The language
client selects SplitScript documents from any workspace provider and starts the
same Rust [`LanguageServer`](../src/lsp.rs) inside a dedicated worker backed by
the bundled compiler WebAssembly. `vscode-languageclient` communicates with it
through standard JSON-message transports, so formatting and every other dynamic
editor feature remain LSP registrations rather than parallel VS Code providers.
Restart replaces the worker and its isolated language-server state. Native
`splitls` remains an independent stdio shell for non-VS-Code clients.

Compilation no longer discovers or spawns `splitc`. The optimized compiler Wasm
and a Node worker adapter are packaged under the extension's ignored `dist`
directory. One long-lived worker initializes the module once and accepts exact
source snapshots tagged with their VS Code document revision. Build Release
uses the release profile and rejects output when the document changes during
the build. Debug watch listens for saves, coalesces them while a build is in
flight, compiles with the debug profile, and discards an older result whenever
a newer save is queued. Both write the neighboring `.wasm` through
`vscode.workspace.fs` using a temporary sibling and overwrite rename, so failed
or superseded builds preserve the last successful module. Release and watch
share one discriminated task owner and cannot overwrite each other. The worker
protocol transfers raw artifact buffers rather than serializing them, reports
structured compiler diagnostics, rejects pending requests if the worker exits,
and terminates with the extension. Tests cover task ownership, envelope
validation, direct Wasm compilation, and compilation through the real worker.

The portable-extension migration now also has an in-memory compiler product.
[`compiler::service`](../src/service.rs) accepts a versioned request containing
the source URI, exact document revision, source text, and debug/release profile.
It returns either structured diagnostics (including labels and fixes) or the
generated module bytes for the same revision. It deliberately knows nothing
about files, polling, child processes, VS Code, or terminal rendering. The
native `splitc` and `splitls` binaries remain independent shells around the
shared compiler/tooling implementation and remain publishable native products.
Every response also carries the canonical compiler package version and optional
full Git revision. Generated autosplitters embed that identity in their
`splitscript` custom section as JSON, while native version output and LSP server
information use the same build-time source.

The shared compiler now also exposes a cloneable `CompilationCancellation`
token and a typed `CompilationFailure::Cancelled` outcome. Stable checkpoints
separate analysis, Wasm-IR lowering, binary encoding, and publication; a
cancelled request never masquerades as a source diagnostic or publishes an
artifact completed after cancellation. The compiler service maps this to a
distinct `cancelled` service error. The embedded adapter retains opaque
analysis and Wasm-IR products across worker event-loop yields. Debug watch can
therefore discard a superseded revision before lowering or encoding, report a
typed cancellation outcome, and immediately compile the newest queued save.
No partial response or artifact is published by a discarded stage.

[`crates/splitscript-vscode-wasm`](../crates/splitscript-vscode-wasm) is the
first direct-WebAssembly adapter for that service. It is a separate unpublished
`cdylib`, so the main compiler library remains an ordinary Rust `rlib` for
native consumers. Its compact ABI accepts request metadata as JSON but returns
the generated autosplitter module as raw bytes after a JSON metadata prefix;
large artifacts are never expanded into JSON number arrays or base64. The
platform-neutral TypeScript binding lives in
[`embeddedCompiler.ts`](../editors/vscode/src/embeddedCompiler.ts). It currently
serves both the direct protocol tests and the dedicated build worker. The same
adapter exposes a direct JSON-message ABI around the editor-neutral Rust LSP
handler; the extension runs it in a second, isolated worker and connects it to
`vscode-languageclient` without stdio framing. Desktop adapters use Node
`worker_threads`; browser adapters use the Web Worker API over the same
messages and compiler ABI.

The browser host slice exists alongside the desktop adapter. The manifest
points `browser` at one esbuild-produced extension bundle, which imports
`vscode-languageclient/browser`; two separately bundled browser workers host the
compiler service and direct JSON-message LSP service. Shared activation and
build orchestration receive host-specific worker factories and otherwise use
only `ExtensionContext.extensionUri`, URI operations, and `workspace.fs`.
Generated-worker tests execute the actual bundles against the packaged compiler
Wasm and audit the browser entry's external imports. A real
`@vscode/test-web`/virtual-workspace suite remains the acceptance boundary for
declaring web delivery complete.

The extension manifest owns the `.split` association, declarative language
configuration, and fast-startup TextMate grammar. Semantic highlighting is
enabled by default and the compiler's custom `setting`, `settingTitle`,
`stateField`, `lifecycle`, `signature`, and `debug` token types plus the `debug`
modifier are contributed with standard supertypes and fallback theme scopes.
An integration test compares those manifest IDs against
`SemanticTokenKind::ALL` and `SEMANTIC_TOKEN_MODIFIERS`, preventing protocol and
extension drift. Root VS Code tasks build the Rust and TypeScript halves,
compile or watch the active script, and launch an Extension Development Host.
The packaged client additionally contributes fixed debug-watch and release-build
commands to the editor title and context menu, so users do not need the
repository tasks or understand compiler profile flags.

## Module responsibility policy

Source files above roughly 1,000 lines receive an ownership review. This is a
soft design signal, not a formatting gate: a split is useful only when the new
modules communicate through a named product or a narrow context. The parser,
type checker, generated runtime, compiler database, LSP handler, and compiler
integration suite have all been decomposed on those terms. Their roots now
orchestrate grammar domains, semantic passes, host domains, cached queries,
protocol services, and subsystem suites respectively.

The current modules above that signal are `codegen/expression.rs`,
`wasm_ir.rs`, `hir.rs`, `language.rs`, `formatter.rs`,
`codegen/async_state.rs`, and `completion.rs`. They remain cohesive for now:
expression and async emission share backend plans, the IR and HIR define stage
products and visitors, the language catalog is one authored declaration,
formatting is one document transform, and completion is one query pipeline.
Future work should split one of these only when a concrete feature exposes a
stable boundary—for example call emission versus structural expression
emission—not by moving methods that still share the same mutable context.

## Package API boundary

The package exposes two classified module trees. [`compiler`](../src/compiler.rs)
contains the staged compiler API and deliberately inspectable syntax, HIR,
semantic, standard-library, memory-layout, effect, and Wasm-IR products.
[`tooling`](../src/tooling.rs) contains the revisioned database, completion,
hover/signature, highlighting, document-symbol, language-catalog, documentation,
and in-process LSP products. The crate root retains convenient compile/check/
format functions and their product types.

Implementation passes, inference/resolution/validation machinery, intrinsic
and helper registries, protocol DTOs, and code emitters are private. Integration
tests consume the same facades as external tools, preventing test convenience
from accidentally making every source module part of the package contract. A
new public item should therefore answer whether it is a compiler stage product
or an editor-neutral tooling query; if neither applies, it stays internal.
