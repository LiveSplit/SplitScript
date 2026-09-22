//! Recursive pattern usefulness, reachability, and exhaustiveness.

use std::collections::{BTreeSet, HashMap};

use crate::{
    inference::Type,
    semantic::{ResolvedEnumVariantId, ResolvedStructFieldId, ResolvedStructId},
    stdlib::{FieldVisibility, StdlibTypeId},
};

use super::Checker;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IntegerInterval {
    start: i128,
    end: i128,
}

impl IntegerInterval {
    fn intersect(self, other: Self) -> Option<Self> {
        let interval = Self {
            start: self.start.max(other.start),
            end: self.end.min(other.end),
        };
        (interval.start <= interval.end).then_some(interval)
    }

    fn contains(self, other: Self) -> bool {
        self.start <= other.start && other.end <= self.end
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum PatternCoverage {
    Irrefutable,
    Enum {
        variant: ResolvedEnumVariantId,
        name: String,
        payload: Box<Self>,
    },
    Struct {
        structure: ResolvedStructId,
        name: String,
        fields: Vec<(ResolvedStructFieldId, String, Self)>,
    },
    Bool(bool),
    Char(char),
    String(String),
    Int {
        value: u64,
        negative: bool,
    },
    IntRange {
        start: i128,
        end: i128,
        kind: crate::ast::RangeKind,
    },
    FileVersion([u16; 4]),
    OptionNone,
    OptionSome(Box<Self>),
    IteratorEnd,
    IteratorItem(Box<Self>),
    ResultSuccess(Box<Self>),
    ResultError(Box<Self>),
    Array {
        prefix: Vec<Self>,
        rest: bool,
        suffix: Vec<Self>,
    },
    Alternation(Vec<Self>),
    Invalid(crate::ast::PatternId),
}

impl PatternCoverage {
    pub(super) fn is_irrefutable(&self) -> bool {
        matches!(self, Self::Irrefutable)
    }

    pub(super) fn display(&self) -> String {
        match self {
            Self::Irrefutable => "_".to_owned(),
            Self::Enum { name, payload, .. } if payload.is_irrefutable() => name.clone(),
            Self::Enum { name, payload, .. } => format!("{name}({})", payload.display()),
            Self::Struct { name, fields, .. } => format!(
                "{name} {{ {} }}",
                fields
                    .iter()
                    .map(|(_, field, pattern)| format!("{field}: {}", pattern.display()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Bool(value) => value.to_string(),
            Self::Char(value) => format!("{value:?}"),
            Self::String(value) => format!("{value:?}"),
            Self::Int { value, negative } => {
                format!("{}{value}", if *negative { "-" } else { "" })
            }
            Self::IntRange { start, end, kind } => {
                format!("{start}{}{end}", kind.operator())
            }
            Self::FileVersion(parts) => {
                format!("v\"{}.{}.{}.{}\"", parts[0], parts[1], parts[2], parts[3])
            }
            Self::OptionNone => "None".to_owned(),
            Self::OptionSome(payload) if payload.is_irrefutable() => "Some(value)".to_owned(),
            Self::OptionSome(payload) => format!("Some({})", payload.display()),
            Self::IteratorEnd => "End".to_owned(),
            Self::IteratorItem(payload) if payload.is_irrefutable() => "Item(value)".to_owned(),
            Self::IteratorItem(payload) => format!("Item({})", payload.display()),
            Self::ResultSuccess(payload) if payload.is_irrefutable() => "Ok(value)".to_owned(),
            Self::ResultSuccess(payload) => format!("Ok({})", payload.display()),
            Self::ResultError(payload) if payload.is_irrefutable() => "Err(error)".to_owned(),
            Self::ResultError(payload) => format!("Err({})", payload.display()),
            Self::Array {
                prefix,
                rest,
                suffix,
            } => {
                let mut elements = prefix.iter().map(Self::display).collect::<Vec<_>>();
                if *rest {
                    elements.push("..".to_owned());
                }
                elements.extend(suffix.iter().map(Self::display));
                format!("[{}]", elements.join(", "))
            }
            Self::Alternation(alternatives) => alternatives
                .iter()
                .map(Self::display)
                .collect::<Vec<_>>()
                .join(" | "),
            Self::Invalid(_) => "<invalid pattern>".to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum PatternConstructor {
    Enum {
        variant: ResolvedEnumVariantId,
        name: String,
    },
    Struct {
        structure: ResolvedStructId,
        name: String,
        fields: Vec<(ResolvedStructFieldId, String, Type)>,
    },
    Bool(bool),
    Char(char),
    String(String),
    Int {
        value: u64,
        negative: bool,
    },
    FileVersion([u16; 4]),
    OptionNone,
    OptionSome,
    IteratorEnd,
    IteratorItem,
    ResultSuccess,
    ResultError,
}

#[derive(Debug, Clone, Copy)]
enum Inhabitedness {
    Visiting,
    Known(bool),
}

type StructPatternShape = (
    ResolvedStructId,
    String,
    Vec<(ResolvedStructFieldId, String, Type)>,
);

impl PatternConstructor {
    fn of(pattern: &PatternCoverage) -> Option<Self> {
        match pattern {
            PatternCoverage::Enum { variant, name, .. } => Some(Self::Enum {
                variant: *variant,
                name: name.clone(),
            }),
            PatternCoverage::Struct {
                structure, name, ..
            } => Some(Self::Struct {
                structure: *structure,
                name: name.clone(),
                fields: Vec::new(),
            }),
            PatternCoverage::Bool(value) => Some(Self::Bool(*value)),
            PatternCoverage::Char(value) => Some(Self::Char(*value)),
            PatternCoverage::String(value) => Some(Self::String(value.clone())),
            PatternCoverage::Int { value, negative } => Some(Self::Int {
                value: *value,
                negative: *negative,
            }),
            PatternCoverage::FileVersion(parts) => Some(Self::FileVersion(*parts)),
            PatternCoverage::OptionNone => Some(Self::OptionNone),
            PatternCoverage::OptionSome(_) => Some(Self::OptionSome),
            PatternCoverage::IteratorEnd => Some(Self::IteratorEnd),
            PatternCoverage::IteratorItem(_) => Some(Self::IteratorItem),
            PatternCoverage::ResultSuccess(_) => Some(Self::ResultSuccess),
            PatternCoverage::ResultError(_) => Some(Self::ResultError),
            PatternCoverage::Irrefutable
            | PatternCoverage::Array { .. }
            | PatternCoverage::IntRange { .. }
            | PatternCoverage::Alternation(_)
            | PatternCoverage::Invalid(_) => None,
        }
    }

    fn same_identity(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::Struct {
                    structure: left, ..
                },
                Self::Struct {
                    structure: right, ..
                },
            ) => left == right,
            _ => self == other,
        }
    }
}

impl Checker {
    fn struct_pattern_shape(&mut self, ty: Type) -> Option<StructPatternShape> {
        let ty = self.shallow_type(ty);
        if let Some(structure) = self.source_struct_id(ty) {
            let declaration = self
                .declarations
                .structs
                .iter()
                .find(|declaration| declaration.id == structure)?
                .clone();
            let fields = declaration
                .fields
                .iter()
                .map(|field| {
                    (
                        ResolvedStructFieldId::Source(field.id),
                        field.name.clone(),
                        self.syntax_type(field.ty),
                    )
                })
                .collect();
            return Some((
                ResolvedStructId::Source(structure),
                declaration.name,
                fields,
            ));
        }

        let application = match ty {
            Type::Application(application) => application,
            Type::Known(id) => match self.inference.type_store().kind(id) {
                crate::types::TypeKind::Application { layout, .. } => *layout,
                _ => return None,
            },
            _ => return None,
        };
        let constructor = self.inference.application_constructor(application);
        let declaration = *self.standard_library.type_constructor(constructor);
        let variables = declaration
            .parameters
            .iter()
            .zip(self.inference.application_arguments(application).to_vec())
            .map(|(parameter, argument)| (parameter.name, argument))
            .collect::<HashMap<_, _>>();
        let fields = self
            .standard_library
            .fields_of_constructor(constructor)
            .filter(|field| field.visibility == FieldVisibility::Public)
            .copied()
            .collect::<Vec<_>>();
        if fields.is_empty() {
            return None;
        }
        let fields = fields
            .into_iter()
            .map(|field| {
                (
                    ResolvedStructFieldId::Standard(field.id),
                    field.name.to_owned(),
                    self.catalog_type(field.ty, &variables),
                )
            })
            .collect();
        Some((
            ResolvedStructId::StandardConstructor(application),
            declaration.name.to_owned(),
            fields,
        ))
    }

    /// Computes the least fixed point of source-value construction. Encountering
    /// a recursive type again does not by itself prove a value exists: a cycle
    /// becomes inhabited only when another path reaches a concrete constructor
    /// such as an empty enum variant, `None`, or an empty collection.
    fn type_is_inhabited(&mut self, ty: Type) -> bool {
        fn visit(
            checker: &mut Checker,
            ty: Type,
            states: &mut HashMap<Type, Inhabitedness>,
        ) -> bool {
            let ty = checker.shallow_type(ty);
            match states.get(&ty).copied() {
                Some(Inhabitedness::Visiting) => return false,
                Some(Inhabitedness::Known(value)) => return value,
                None => {}
            }
            states.insert(ty, Inhabitedness::Visiting);

            let inhabited = if checker.is_never_type(ty) {
                false
            } else if let Type::Array(array) = ty {
                match checker.inference.array_length(array) {
                    Some(0) | None => true,
                    Some(_) => visit(checker, checker.inference.array_element(array), states),
                }
            } else if let Some(structure) = checker.source_struct_id(ty) {
                let fields = checker
                    .declarations
                    .structs
                    .iter()
                    .find(|declaration| declaration.id == structure)
                    .map(|declaration| {
                        declaration
                            .fields
                            .iter()
                            .map(|field| checker.syntax_type(field.ty))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                fields
                    .into_iter()
                    .all(|field| visit(checker, field, states))
            } else if let Some((_, enumeration)) = checker.enum_info_for_type(ty) {
                enumeration.variants.into_iter().any(|variant| {
                    variant
                        .payload
                        .is_none_or(|payload| visit(checker, payload, states))
                })
            } else {
                // Options, results, iterator steps, growable containers, scalar
                // values, functions, futures, and unresolved recovery types all
                // have at least one concrete runtime representation.
                true
            };
            states.insert(ty, Inhabitedness::Known(inhabited));
            inhabited
        }

        visit(self, ty, &mut HashMap::new())
    }

    fn pattern_constructors(&mut self, ty: Type) -> Option<Vec<PatternConstructor>> {
        let ty = self.shallow_type(ty);
        if !self.type_is_inhabited(ty) {
            return Some(Vec::new());
        }
        if ty == self.core_type(crate::stdlib::CoreTypeId::Bool) {
            return Some(vec![
                PatternConstructor::Bool(false),
                PatternConstructor::Bool(true),
            ]);
        }
        match ty {
            Type::Option(_) => Some(vec![
                PatternConstructor::OptionNone,
                PatternConstructor::OptionSome,
            ]),
            Type::Result(_) => Some(vec![
                PatternConstructor::ResultSuccess,
                PatternConstructor::ResultError,
            ]),
            Type::Application(step)
                if self.inference.application_constructor(step)
                    == crate::stdlib::StdlibTypeConstructorId::IteratorStep =>
            {
                Some(vec![
                    PatternConstructor::IteratorItem,
                    PatternConstructor::IteratorEnd,
                ])
            }
            other => {
                if let Some((structure, name, fields)) = self.struct_pattern_shape(other) {
                    Some(vec![PatternConstructor::Struct {
                        structure,
                        name,
                        fields,
                    }])
                } else if matches!(other, Type::Known(_)) {
                    self.enum_info_for_type(other).map(|(_, enumeration)| {
                        enumeration
                            .variants
                            .into_iter()
                            .map(|variant| PatternConstructor::Enum {
                                variant: variant.id,
                                name: variant.name,
                            })
                            .collect()
                    })
                } else {
                    None
                }
            }
        }
    }

    fn pattern_constructor_argument_types(
        &mut self,
        constructor: &PatternConstructor,
        ty: Type,
    ) -> Vec<Type> {
        let ty = self.shallow_type(ty);
        match constructor {
            PatternConstructor::Enum { variant, .. } => self
                .enum_info_for_type(ty)
                .and_then(|(_, enumeration)| {
                    enumeration
                        .variants
                        .into_iter()
                        .find(|candidate| candidate.id == *variant)
                })
                .and_then(|variant| variant.payload)
                .into_iter()
                .collect(),
            PatternConstructor::Struct { fields, .. } => {
                fields.iter().map(|(_, _, ty)| *ty).collect()
            }
            PatternConstructor::OptionSome => match ty {
                Type::Option(option) => vec![self.inference.option_value(option)],
                _ => Vec::new(),
            },
            PatternConstructor::IteratorItem => match ty {
                Type::Application(step) => self
                    .inference
                    .application_arguments(step)
                    .first()
                    .copied()
                    .into_iter()
                    .collect(),
                _ => Vec::new(),
            },
            PatternConstructor::ResultSuccess => match ty {
                Type::Result(result) => vec![self.inference.result_value(result)],
                _ => Vec::new(),
            },
            PatternConstructor::ResultError => {
                vec![self.standard_type(StdlibTypeId::String)]
            }
            PatternConstructor::Bool(_)
            | PatternConstructor::Char(_)
            | PatternConstructor::String(_)
            | PatternConstructor::Int { .. }
            | PatternConstructor::FileVersion(_)
            | PatternConstructor::OptionNone
            | PatternConstructor::IteratorEnd => Vec::new(),
        }
    }

    fn pattern_constructor_arguments(
        &self,
        pattern: &PatternCoverage,
        constructor: &PatternConstructor,
        arity: usize,
    ) -> Vec<PatternCoverage> {
        let mut arguments = match (pattern, constructor) {
            (PatternCoverage::Irrefutable, _) => vec![PatternCoverage::Irrefutable; arity],
            (PatternCoverage::Enum { payload, .. }, PatternConstructor::Enum { .. }) => (arity
                != 0)
                .then(|| (**payload).clone())
                .into_iter()
                .collect(),
            (
                PatternCoverage::Struct { fields, .. },
                PatternConstructor::Struct {
                    fields: declared_fields,
                    ..
                },
            ) => declared_fields
                .iter()
                .map(|(field, _, _)| {
                    fields
                        .iter()
                        .find(|(candidate, _, _)| candidate == field)
                        .map(|(_, _, pattern)| pattern.clone())
                        .unwrap_or(PatternCoverage::Irrefutable)
                })
                .collect(),
            (PatternCoverage::OptionSome(payload), PatternConstructor::OptionSome)
            | (PatternCoverage::IteratorItem(payload), PatternConstructor::IteratorItem)
            | (PatternCoverage::ResultSuccess(payload), PatternConstructor::ResultSuccess)
            | (PatternCoverage::ResultError(payload), PatternConstructor::ResultError) => {
                vec![(**payload).clone()]
            }
            _ => Vec::new(),
        };
        arguments.resize(arity, PatternCoverage::Irrefutable);
        arguments.truncate(arity);
        arguments
    }

    fn expand_pattern_alternatives(pattern: &PatternCoverage, output: &mut Vec<PatternCoverage>) {
        if let PatternCoverage::Alternation(alternatives) = pattern {
            for alternative in alternatives {
                Self::expand_pattern_alternatives(alternative, output);
            }
        } else {
            output.push(pattern.clone());
        }
    }

    fn specialize_pattern_matrix(
        &self,
        matrix: &[Vec<PatternCoverage>],
        constructor: &PatternConstructor,
        arity: usize,
    ) -> Vec<Vec<PatternCoverage>> {
        let mut specialized = Vec::new();
        for row in matrix {
            let Some((head, tail)) = row.split_first() else {
                continue;
            };
            let mut alternatives = Vec::new();
            Self::expand_pattern_alternatives(head, &mut alternatives);
            for head in alternatives {
                if matches!(head, PatternCoverage::Invalid(_)) {
                    continue;
                }
                if head.is_irrefutable()
                    || PatternConstructor::of(&head)
                        .as_ref()
                        .is_some_and(|head| head.same_identity(constructor))
                {
                    let mut row = self.pattern_constructor_arguments(&head, constructor, arity);
                    row.extend_from_slice(tail);
                    specialized.push(row);
                }
            }
        }
        specialized
    }

    fn default_pattern_matrix(&self, matrix: &[Vec<PatternCoverage>]) -> Vec<Vec<PatternCoverage>> {
        let mut default = Vec::new();
        for row in matrix {
            let Some((head, tail)) = row.split_first() else {
                continue;
            };
            let mut alternatives = Vec::new();
            Self::expand_pattern_alternatives(head, &mut alternatives);
            for head in alternatives {
                if head.is_irrefutable() {
                    default.push(tail.to_vec());
                }
            }
        }
        default
    }

    fn pattern_from_constructor(
        &self,
        constructor: &PatternConstructor,
        arguments: Vec<PatternCoverage>,
    ) -> PatternCoverage {
        match constructor {
            PatternConstructor::Enum { variant, name } => PatternCoverage::Enum {
                variant: *variant,
                name: name.clone(),
                payload: Box::new(
                    arguments
                        .into_iter()
                        .next()
                        .unwrap_or(PatternCoverage::Irrefutable),
                ),
            },
            PatternConstructor::Struct {
                structure,
                name,
                fields,
            } => {
                let fields = fields
                    .iter()
                    .zip(arguments)
                    .map(|((field, name, _), pattern)| (*field, name.clone(), pattern))
                    .collect();
                PatternCoverage::Struct {
                    structure: *structure,
                    name: name.clone(),
                    fields,
                }
            }
            PatternConstructor::Bool(value) => PatternCoverage::Bool(*value),
            PatternConstructor::Char(value) => PatternCoverage::Char(*value),
            PatternConstructor::String(value) => PatternCoverage::String(value.clone()),
            PatternConstructor::Int { value, negative } => PatternCoverage::Int {
                value: *value,
                negative: *negative,
            },
            PatternConstructor::FileVersion(parts) => PatternCoverage::FileVersion(*parts),
            PatternConstructor::OptionNone => PatternCoverage::OptionNone,
            PatternConstructor::OptionSome => PatternCoverage::OptionSome(Box::new(
                arguments
                    .into_iter()
                    .next()
                    .unwrap_or(PatternCoverage::Irrefutable),
            )),
            PatternConstructor::IteratorEnd => PatternCoverage::IteratorEnd,
            PatternConstructor::IteratorItem => PatternCoverage::IteratorItem(Box::new(
                arguments
                    .into_iter()
                    .next()
                    .unwrap_or(PatternCoverage::Irrefutable),
            )),
            PatternConstructor::ResultSuccess => PatternCoverage::ResultSuccess(Box::new(
                arguments
                    .into_iter()
                    .next()
                    .unwrap_or(PatternCoverage::Irrefutable),
            )),
            PatternConstructor::ResultError => PatternCoverage::ResultError(Box::new(
                arguments
                    .into_iter()
                    .next()
                    .unwrap_or(PatternCoverage::Irrefutable),
            )),
        }
    }

    fn array_pattern_dimensions(pattern: &PatternCoverage) -> Option<(usize, usize)> {
        match pattern {
            PatternCoverage::Array { prefix, suffix, .. } => Some((prefix.len(), suffix.len())),
            _ => None,
        }
    }

    fn array_pattern_at_length(
        pattern: &PatternCoverage,
        length: usize,
    ) -> Option<Vec<PatternCoverage>> {
        match pattern {
            PatternCoverage::Irrefutable => Some(vec![PatternCoverage::Irrefutable; length]),
            PatternCoverage::Array {
                prefix,
                rest,
                suffix,
            } => {
                let explicit_length = prefix.len() + suffix.len();
                if (*rest && explicit_length <= length) || (!*rest && explicit_length == length) {
                    let mut elements = Vec::with_capacity(length);
                    elements.extend(prefix.iter().cloned());
                    elements.resize(length - suffix.len(), PatternCoverage::Irrefutable);
                    elements.extend(suffix.iter().cloned());
                    Some(elements)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn specialize_array_matrix(
        &self,
        matrix: &[Vec<PatternCoverage>],
        length: usize,
    ) -> Vec<Vec<PatternCoverage>> {
        let mut specialized = Vec::new();
        for row in matrix {
            let Some((head, tail)) = row.split_first() else {
                continue;
            };
            let mut alternatives = Vec::new();
            Self::expand_pattern_alternatives(head, &mut alternatives);
            for alternative in alternatives {
                if let Some(mut elements) = Self::array_pattern_at_length(&alternative, length) {
                    elements.extend_from_slice(tail);
                    specialized.push(elements);
                }
            }
        }
        specialized
    }

    fn useful_array_witness(
        &mut self,
        matrix: &[Vec<PatternCoverage>],
        candidate: &PatternCoverage,
        tail: &[PatternCoverage],
        remaining_types: &[Type],
        array: crate::ast::ArrayTypeId,
    ) -> Option<Vec<PatternCoverage>> {
        let lengths = if let Some(length) = self.inference.array_length(array) {
            vec![length as usize]
        } else {
            let mut prefix_lengths = vec![0];
            let mut suffix_lengths = vec![0];
            if let Some((prefix, suffix)) = Self::array_pattern_dimensions(candidate) {
                prefix_lengths.push(prefix);
                suffix_lengths.push(suffix);
            }
            for row in matrix {
                let Some(head) = row.first() else {
                    continue;
                };
                let mut alternatives = Vec::new();
                Self::expand_pattern_alternatives(head, &mut alternatives);
                for alternative in alternatives {
                    if let Some((prefix, suffix)) = Self::array_pattern_dimensions(&alternative) {
                        prefix_lengths.push(prefix);
                        suffix_lengths.push(suffix);
                    }
                }
            }
            prefix_lengths.sort_unstable();
            prefix_lengths.dedup();
            suffix_lengths.sort_unstable();
            suffix_lengths.dedup();

            // Prefix positions are measured from the start and suffix positions
            // from the end. Their relative geometry can change only when one of
            // these lengths crosses a prefix-plus-suffix boundary. Test both
            // sides of every such boundary, plus one stable longer length,
            // instead of enumerating an unbounded growable-array domain.
            let mut representative_lengths = BTreeSet::new();
            for prefix in &prefix_lengths {
                for suffix in &suffix_lengths {
                    let boundary = prefix.saturating_add(*suffix);
                    representative_lengths.insert(boundary);
                    representative_lengths.insert(boundary.saturating_add(1));
                }
            }

            match candidate {
                PatternCoverage::Array {
                    prefix,
                    rest: false,
                    suffix,
                } => vec![prefix.len() + suffix.len()],
                PatternCoverage::Array {
                    prefix,
                    rest: true,
                    suffix,
                } => {
                    let minimum = prefix.len() + suffix.len();
                    representative_lengths
                        .into_iter()
                        .filter(|length| *length >= minimum)
                        .collect()
                }
                PatternCoverage::Irrefutable => representative_lengths.into_iter().collect(),
                _ => return None,
            }
        };

        let element_type = self.inference.array_element(array);
        for length in lengths {
            let Some(mut specialized_candidate) = Self::array_pattern_at_length(candidate, length)
            else {
                continue;
            };
            specialized_candidate.extend_from_slice(tail);
            let specialized = self.specialize_array_matrix(matrix, length);
            let mut specialized_types = vec![element_type; length];
            specialized_types.extend_from_slice(remaining_types);
            if let Some(mut witness) = self.useful_pattern_witness(
                &specialized,
                &specialized_candidate,
                &specialized_types,
            ) {
                let tail = witness.split_off(length);
                let mut result = vec![PatternCoverage::Array {
                    prefix: witness,
                    rest: false,
                    suffix: Vec::new(),
                }];
                result.extend(tail);
                return Some(result);
            }
        }
        None
    }

    fn integer_pattern_domain(&mut self, ty: Type) -> Option<IntegerInterval> {
        let ty = self.shallow_type(ty);
        let bounds = [
            (
                crate::stdlib::CoreTypeId::I8,
                i8::MIN as i128,
                i8::MAX as i128,
            ),
            (crate::stdlib::CoreTypeId::U8, 0, u8::MAX as i128),
            (
                crate::stdlib::CoreTypeId::I16,
                i16::MIN as i128,
                i16::MAX as i128,
            ),
            (crate::stdlib::CoreTypeId::U16, 0, u16::MAX as i128),
            (
                crate::stdlib::CoreTypeId::I32,
                i32::MIN as i128,
                i32::MAX as i128,
            ),
            (crate::stdlib::CoreTypeId::U32, 0, u32::MAX as i128),
            (
                crate::stdlib::CoreTypeId::I64,
                i64::MIN as i128,
                i64::MAX as i128,
            ),
            (crate::stdlib::CoreTypeId::U64, 0, u64::MAX as i128),
            (crate::stdlib::CoreTypeId::Address, 0, u64::MAX as i128),
        ];
        for (core, start, end) in bounds {
            if ty == self.core_type(core) {
                return Some(IntegerInterval { start, end });
            }
        }
        self.inference.is_integer(ty).then_some(IntegerInterval {
            start: -(u64::MAX as i128),
            end: u64::MAX as i128,
        })
    }

    fn integer_interval(
        &self,
        pattern: &PatternCoverage,
        domain: IntegerInterval,
    ) -> Option<IntegerInterval> {
        let interval = match pattern {
            PatternCoverage::Irrefutable => domain,
            PatternCoverage::Int { value, negative } => {
                let value = if *negative && *value != 0 {
                    -i128::from(*value)
                } else {
                    i128::from(*value)
                };
                IntegerInterval {
                    start: value,
                    end: value,
                }
            }
            PatternCoverage::IntRange { start, end, kind } => IntegerInterval {
                start: *start,
                end: match kind {
                    crate::ast::RangeKind::Exclusive => end - 1,
                    crate::ast::RangeKind::Inclusive => *end,
                },
            },
            _ => return None,
        };
        interval.intersect(domain)
    }

    fn integer_interval_pattern(interval: IntegerInterval) -> PatternCoverage {
        if interval.start == interval.end {
            let negative = interval.start < 0;
            let value = if negative {
                (-interval.start) as u64
            } else {
                interval.start as u64
            };
            PatternCoverage::Int { value, negative }
        } else {
            PatternCoverage::IntRange {
                start: interval.start,
                end: interval.end,
                kind: crate::ast::RangeKind::Inclusive,
            }
        }
    }

    fn specialize_integer_matrix(
        &self,
        matrix: &[Vec<PatternCoverage>],
        cell: IntegerInterval,
        domain: IntegerInterval,
    ) -> Vec<Vec<PatternCoverage>> {
        let mut specialized = Vec::new();
        for row in matrix {
            let Some((head, tail)) = row.split_first() else {
                continue;
            };
            let mut alternatives = Vec::new();
            Self::expand_pattern_alternatives(head, &mut alternatives);
            if alternatives.into_iter().any(|alternative| {
                self.integer_interval(&alternative, domain)
                    .is_some_and(|interval| interval.contains(cell))
            }) {
                specialized.push(tail.to_vec());
            }
        }
        specialized
    }

    fn useful_integer_witness(
        &mut self,
        matrix: &[Vec<PatternCoverage>],
        candidate: IntegerInterval,
        tail: &[PatternCoverage],
        remaining_types: &[Type],
        domain: IntegerInterval,
    ) -> Option<Vec<PatternCoverage>> {
        let mut boundaries = vec![candidate.start, candidate.end + 1];
        for row in matrix {
            let Some(head) = row.first() else {
                continue;
            };
            let mut alternatives = Vec::new();
            Self::expand_pattern_alternatives(head, &mut alternatives);
            for alternative in alternatives {
                if let Some(interval) = self
                    .integer_interval(&alternative, domain)
                    .and_then(|interval| interval.intersect(candidate))
                {
                    boundaries.push(interval.start);
                    boundaries.push(interval.end + 1);
                }
            }
        }
        boundaries.sort_unstable();
        boundaries.dedup();

        for pair in boundaries.windows(2) {
            let cell = IntegerInterval {
                start: pair[0],
                end: pair[1] - 1,
            };
            if cell.start > cell.end {
                continue;
            }
            let specialized = self.specialize_integer_matrix(matrix, cell, domain);
            if let Some(mut witness) =
                self.useful_pattern_witness(&specialized, tail, remaining_types)
            {
                witness.insert(0, Self::integer_interval_pattern(cell));
                return Some(witness);
            }
        }
        None
    }

    fn useful_pattern_witness(
        &mut self,
        matrix: &[Vec<PatternCoverage>],
        candidate: &[PatternCoverage],
        types: &[Type],
    ) -> Option<Vec<PatternCoverage>> {
        // A row of wildcards covers the complete product represented by this
        // matrix. Besides avoiding needless constructor expansion, this is
        // essential for recursive product types: expanding `Node { next: _ }`
        // back into `Node` can otherwise recurse forever even though the
        // existing row has already proved that every value is covered.
        if matrix.iter().any(|row| {
            row.len() == candidate.len() && row.iter().all(PatternCoverage::is_irrefutable)
        }) {
            return None;
        }
        let Some((head, tail)) = candidate.split_first() else {
            return matrix.is_empty().then(Vec::new);
        };
        let (&ty, remaining_types) = types.split_first()?;
        if !self.type_is_inhabited(ty) {
            return None;
        }
        if let PatternCoverage::Alternation(alternatives) = head {
            for alternative in alternatives {
                let mut branch = vec![alternative.clone()];
                branch.extend_from_slice(tail);
                if let Some(witness) = self.useful_pattern_witness(matrix, &branch, types) {
                    return Some(witness);
                }
            }
            return None;
        }
        if matches!(head, PatternCoverage::Invalid(_)) {
            return Some(candidate.to_vec());
        }

        if let Type::Array(array) = self.shallow_type(ty) {
            return self.useful_array_witness(matrix, head, tail, remaining_types, array);
        }

        if let Some(domain) = self.integer_pattern_domain(ty)
            && let Some(interval) = self.integer_interval(head, domain)
        {
            return self.useful_integer_witness(matrix, interval, tail, remaining_types, domain);
        }

        if let Some(mut constructor) = PatternConstructor::of(head) {
            if matches!(constructor, PatternConstructor::Struct { .. })
                && let Some(domain_constructor) =
                    self.pattern_constructors(ty).and_then(|constructors| {
                        constructors
                            .into_iter()
                            .find(|candidate| candidate.same_identity(&constructor))
                    })
            {
                constructor = domain_constructor;
            }
            let argument_types = self.pattern_constructor_argument_types(&constructor, ty);
            let arity = argument_types.len();
            let specialized = self.specialize_pattern_matrix(matrix, &constructor, arity);
            let mut specialized_candidate =
                self.pattern_constructor_arguments(head, &constructor, arity);
            specialized_candidate.extend_from_slice(tail);
            let mut specialized_types = argument_types;
            specialized_types.extend_from_slice(remaining_types);
            let mut witness = self.useful_pattern_witness(
                &specialized,
                &specialized_candidate,
                &specialized_types,
            )?;
            let tail = witness.split_off(arity);
            let pattern = self.pattern_from_constructor(&constructor, witness);
            let mut result = vec![pattern];
            result.extend(tail);
            return Some(result);
        }

        if let Some(constructors) = self.pattern_constructors(ty) {
            for constructor in constructors {
                let argument_types = self.pattern_constructor_argument_types(&constructor, ty);
                let arity = argument_types.len();
                let specialized = self.specialize_pattern_matrix(matrix, &constructor, arity);
                let mut specialized_candidate = vec![PatternCoverage::Irrefutable; arity];
                specialized_candidate.extend_from_slice(tail);
                let mut specialized_types = argument_types;
                specialized_types.extend_from_slice(remaining_types);
                if let Some(mut witness) = self.useful_pattern_witness(
                    &specialized,
                    &specialized_candidate,
                    &specialized_types,
                ) {
                    let tail = witness.split_off(arity);
                    let pattern = self.pattern_from_constructor(&constructor, witness);
                    let mut result = vec![pattern];
                    result.extend(tail);
                    return Some(result);
                }
            }
            None
        } else {
            let default = self.default_pattern_matrix(matrix);
            let mut witness = self.useful_pattern_witness(&default, tail, remaining_types)?;
            witness.insert(0, PatternCoverage::Irrefutable);
            Some(witness)
        }
    }

    pub(super) fn pattern_is_useful(
        &mut self,
        previous: &[PatternCoverage],
        candidate: &PatternCoverage,
        ty: Type,
    ) -> bool {
        let matrix = previous
            .iter()
            .cloned()
            .map(|pattern| vec![pattern])
            .collect::<Vec<_>>();
        self.useful_pattern_witness(&matrix, std::slice::from_ref(candidate), &[ty])
            .is_some()
    }

    pub(super) fn missing_patterns(
        &mut self,
        previous: &[PatternCoverage],
        ty: Type,
    ) -> Vec<PatternCoverage> {
        // A binding or wildcard already covers every value, including an
        // uninhabited type. Avoid expanding its constructors just to prove it again.
        if previous.iter().any(PatternCoverage::is_irrefutable) {
            return Vec::new();
        }
        let matrix = previous
            .iter()
            .cloned()
            .map(|pattern| vec![pattern])
            .collect::<Vec<_>>();
        let ty = self.shallow_type(ty);
        if !self.type_is_inhabited(ty) {
            return Vec::new();
        }
        if let Some(constructors) = self.pattern_constructors(ty) {
            constructors
                .into_iter()
                .filter_map(|constructor| {
                    let arity = self
                        .pattern_constructor_argument_types(&constructor, ty)
                        .len();
                    let candidate = self.pattern_from_constructor(
                        &constructor,
                        vec![PatternCoverage::Irrefutable; arity],
                    );
                    self.useful_pattern_witness(&matrix, std::slice::from_ref(&candidate), &[ty])
                        .and_then(|mut witness| (!witness.is_empty()).then(|| witness.remove(0)))
                })
                .collect()
        } else {
            self.useful_pattern_witness(&matrix, &[PatternCoverage::Irrefutable], &[ty])
                .and_then(|mut witness| (!witness.is_empty()).then(|| witness.remove(0)))
                .into_iter()
                .collect()
        }
    }
}
