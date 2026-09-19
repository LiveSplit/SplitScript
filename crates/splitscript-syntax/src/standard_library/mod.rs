//! Syntax tree and parser for privileged SplitScript standard-library source.
//!
//! This crate intentionally knows nothing about compiler intrinsics or runtime
//! representations. It only turns source into an owned declaration tree. The
//! The compiler validates every reserved binding against its closed trust
//! registries before accepting the resulting library graph.

mod parser;

use std::fmt;

pub use parser::parse;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Library {
    pub declarations: Vec<Declaration>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Declaration {
    Struct(StructDeclaration),
    Enum(EnumDeclaration),
    IntrinsicType(StructDeclaration),
    Root(CallableOwnerDeclaration),
    Namespace(CallableOwnerDeclaration),
    Capability(CallableOwnerDeclaration),
    TypeConstructor(CallableOwnerDeclaration),
    CoreExtension(CallableOwnerDeclaration),
    StateProvider(StateProviderDeclaration),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateProviderDeclaration {
    pub name: String,
    pub value_name: String,
    pub processes: Vec<String>,
    /// Additional attachment-scoped values exposed by this provider.
    pub contexts: Vec<StateProviderContextDeclaration>,
    /// Qualified configurations accepted after the provider name, such as
    /// `Unity.il2cpp(Il2CppProfile.unity2021_3_11f1X64())`.
    pub selectors: Vec<StateProviderSelectorDeclaration>,
    pub documentation: Documentation,
    pub attributes: Vec<Attribute>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateProviderContextDeclaration {
    pub name: String,
    pub ty: Type,
    pub documentation: Documentation,
    pub attributes: Vec<Attribute>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateProviderSelectorDeclaration {
    pub name: String,
    pub parameters: Vec<Parameter>,
    pub documentation: Documentation,
    pub attributes: Vec<Attribute>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallableOwnerDeclaration {
    pub name: String,
    /// Whether this type constructor is available only to privileged
    /// standard-library source.
    pub private: bool,
    /// The public source form of a type constructor. This is present only for
    /// `typeConstructor` declarations; `name` remains its stable catalog ID.
    pub type_constructor_syntax: Option<TypeConstructorSyntax>,
    pub type_parameters: Vec<TypeParameter>,
    pub documentation: Documentation,
    pub attributes: Vec<Attribute>,
    /// Immutable instance fields declared by a structural type constructor.
    /// Other callable-owner forms keep this empty.
    pub fields: Vec<FieldDeclaration>,
    /// Associated type requirements (on capabilities) or their concrete
    /// standard-library definitions (on type constructors).
    pub associated_types: Vec<AssociatedTypeDeclaration>,
    pub functions: Vec<FunctionDeclaration>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssociatedTypeDeclaration {
    pub name: String,
    pub constraints: Vec<String>,
    pub value: Option<Type>,
    pub documentation: Documentation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeConstructorSyntax {
    Named,
    Array,
    Optional,
    Fallible,
    ExclusiveRange,
    InclusiveRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructDeclaration {
    pub name: String,
    /// Whether this type is available only to trusted standard-library source.
    pub private: bool,
    pub documentation: Documentation,
    pub attributes: Vec<Attribute>,
    /// Concrete associated-type definitions used to satisfy capabilities.
    pub associated_types: Vec<AssociatedTypeDeclaration>,
    pub fields: Vec<FieldDeclaration>,
    pub functions: Vec<FunctionDeclaration>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumDeclaration {
    pub name: String,
    /// Whether this type is available only to trusted standard-library source.
    pub private: bool,
    pub documentation: Documentation,
    pub attributes: Vec<Attribute>,
    pub variants: Vec<VariantDeclaration>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariantDeclaration {
    pub name: String,
    pub documentation: Documentation,
    pub attributes: Vec<Attribute>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDeclaration {
    pub name: String,
    pub ty: Type,
    pub private: bool,
    pub documentation: Documentation,
    pub attributes: Vec<Attribute>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionDeclaration {
    pub name: String,
    /// Whether this declaration is an associated constant whose source
    /// initializer is represented internally as a zero-argument body.
    pub is_constant: bool,
    /// Whether this callable is available only to trusted standard-library
    /// source bodies.
    pub private: bool,
    pub type_parameters: Vec<TypeParameter>,
    /// Additional constraints on type parameters inherited from the callable
    /// owner or declared directly by this function.
    pub where_constraints: Vec<TypeParameter>,
    pub parameters: Vec<Parameter>,
    /// Whether the explicitly declared result is `async T` rather than `T`.
    pub result_is_async: bool,
    pub result: Type,
    pub is_static: bool,
    pub documentation: Documentation,
    pub attributes: Vec<Attribute>,
    /// An ordinary SplitScript block for a source-defined implementation.
    /// Intrinsic declarations end in `;` and therefore have no body.
    pub body: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parameter {
    pub name: String,
    pub ty: Type,
    pub documentation: Documentation,
    pub attributes: Vec<Attribute>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeParameter {
    pub name: String,
    pub constraints: Vec<String>,
}

/// A type expression in privileged standard-library source.
///
/// Unlike ordinary program type syntax, this tree does not need stable node
/// identities: it is consumed once by the build-time catalog compiler. It is
/// nevertheless structured so catalog generation never has to parse type
/// names back out of strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    Name(String),
    Async(Box<Type>),
    Iterator(Box<Type>),
    Application {
        constructor: String,
        arguments: Vec<Type>,
    },
    Array(Box<Type>),
    FixedArray {
        element: Box<Type>,
        length: u32,
    },
    Option(Box<Type>),
    Result(Box<Type>),
    ExclusiveRange(Box<Type>),
    InclusiveRange(Box<Type>),
    Callable {
        parameters: Vec<Type>,
        result: Box<Type>,
    },
}

impl fmt::Display for Type {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Name(name) => formatter.write_str(name),
            Self::Async(value) => write!(formatter, "async {value}"),
            Self::Iterator(item) => write!(formatter, "iterator {item}"),
            Self::Application {
                constructor,
                arguments,
            } => {
                write!(formatter, "{constructor}<")?;
                for (index, argument) in arguments.iter().enumerate() {
                    if index != 0 {
                        formatter.write_str(", ")?;
                    }
                    argument.fmt(formatter)?;
                }
                formatter.write_str(">")
            }
            Self::Array(element) => write!(formatter, "[{element}]"),
            Self::FixedArray { element, length } => {
                write!(formatter, "[{element}; {length}]")
            }
            Self::Option(value) => write!(formatter, "{value}?"),
            Self::Result(value) => write!(formatter, "{value}!"),
            Self::ExclusiveRange(value) => write!(formatter, "{value}..<{value}"),
            Self::InclusiveRange(value) => write!(formatter, "{value}..={value}"),
            Self::Callable { parameters, result } => {
                formatter.write_str("(")?;
                for (index, parameter) in parameters.iter().enumerate() {
                    if index != 0 {
                        formatter.write_str(", ")?;
                    }
                    parameter.fmt(formatter)?;
                }
                write!(formatter, ") -> {result}")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    pub name: String,
    pub arguments: Vec<AttributeArgument>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttributeArgument {
    Name(String),
    String(String),
    Integer(u64),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Documentation {
    pub summary: String,
    pub details: String,
    pub examples: Vec<Example>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Example {
    pub title: String,
    /// The concise snippet rendered to users. Lines prefixed with `# ` inside
    /// the source fence are omitted here, following rustdoc's hidden-line
    /// convention.
    pub source: String,
    /// A complete compiler fixture assembled from every fence line after
    /// stripping the hidden-line marker. This is present only when the author
    /// supplied hidden context explicitly.
    pub validation_source: Option<String>,
    /// Optional state-provider name used only to compile-check the focused
    /// snippet. It is not part of rendered documentation.
    pub state_provider: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub message: String,
    pub start: usize,
    pub end: usize,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} at {}..{}",
            self.message, self.start, self.end
        )
    }
}

impl std::error::Error for Error {}
