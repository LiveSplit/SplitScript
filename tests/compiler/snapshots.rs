//! snapshots integration tests.

use super::*;

#[test]
fn resolved_and_typed_hir_snapshot() {
    let source = r#"
        struct Point {
            x: i32,
            y: i32,
        }

        enum Event {
            Idle,
            Moved(Point),
        }

        state "game.exe" {
            event = Event.Idle
        }

        fn pointX(event) -> i32? {
            return match event {
                Event.Moved(point) => point.x,
                Event.Idle => None
            }
        }

        whileAttached {
            let next = Event.Moved(Point { x: 3, y: 4 })
            let x = pointX(next) else 0
            print(`x={x}`)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    assert_snapshot(
        include_str!("../snapshots/resolved_typed_hir.snap"),
        &render_typed_hir_snapshot(&checked),
    );
}

#[test]
fn diagnostic_snapshot() {
    let source = r#"
        state "game.exe" {}

        whileAttached {
            let wrong: i32 = "text"
            let impossible = true + 1
            let missing = unknown
        }
    "#;
    let diagnostics = splitscript::compile(source).expect_err("fixture is intentionally invalid");
    let rendered = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.render("diagnostics.split", source))
        .collect::<Vec<_>>()
        .join("\n");
    assert_snapshot(include_str!("../snapshots/diagnostics.snap"), &rendered);
}

#[test]
fn diagnostics_expose_stable_stage_codes_and_severity() {
    use splitscript::{DiagnosticCode, DiagnosticSeverity};

    let lexical = splitscript::parse("@").expect_err("the source contains an invalid character");
    assert_eq!(lexical[0].code, DiagnosticCode::Lexical);
    assert_eq!(lexical[0].code.as_str(), "SS0001");
    assert_eq!(lexical[0].severity, DiagnosticSeverity::Error);
    assert_eq!(
        lexical[0].render("invalid.split", "@"),
        "invalid.split:1:1: error[SS0001]: unexpected character"
    );

    let syntax = splitscript::parse(
        r#"
            state "game.exe" { broken = + }
        "#,
    )
    .expect_err("the state expression is malformed");
    assert!(
        syntax
            .iter()
            .all(|diagnostic| diagnostic.code == DiagnosticCode::Syntax)
    );
    assert_eq!(DiagnosticCode::Syntax.as_str(), "SS0002");

    let type_errors = splitscript::compile(
        r#"
            state "game.exe" {}
            whileAttached { let value: i32 = "wrong" }
        "#,
    )
    .expect_err("the initializer has the wrong type");
    assert!(
        type_errors
            .iter()
            .all(|diagnostic| diagnostic.code == DiagnosticCode::Type)
    );
    assert_eq!(DiagnosticCode::Type.as_str(), "SS0003");

    let semantic = splitscript::compile(
        r#"
            state "game.exe" {}
            onDetach { process.read<i32>(0x1000) }
        "#,
    )
    .expect_err("process access requires an attachment");
    assert!(semantic.iter().any(|diagnostic| {
        diagnostic.severity == DiagnosticSeverity::Error
            && diagnostic.code == DiagnosticCode::Semantic
    }));
    assert!(semantic.iter().any(|diagnostic| {
        diagnostic.severity == DiagnosticSeverity::Warning
            && diagnostic.code == DiagnosticCode::MustUse
    }));
    assert_eq!(DiagnosticCode::Semantic.as_str(), "SS0004");

    assert_eq!(DiagnosticCode::MustUse.as_str(), "SS1001");
    assert_eq!(DiagnosticCode::UnusedBinding.as_str(), "SS1002");
    assert_eq!(DiagnosticCode::UnusedDeclaration.as_str(), "SS1003");
    assert_eq!(DiagnosticCode::UnusedMember.as_str(), "SS1004");
    assert_eq!(DiagnosticCode::ValueBlockSemicolon.as_str(), "SS1005");
    assert_eq!(DiagnosticCode::AmbiguousRetryFallback.as_str(), "SS1006");
    assert_eq!(DiagnosticCode::StaticSettingLookup.as_str(), "SS1007");
    assert_eq!(DiagnosticCode::SuspiciousInterpolation.as_str(), "SS1008");
    assert_eq!(DiagnosticCode::DebugOnlyUse.as_str(), "SS1009");
    assert_eq!(DiagnosticCode::StructFieldShorthand.as_str(), "SS1010");
}

#[test]
fn structured_diagnostics_render_labels_notes_and_multi_edit_fixes() {
    use splitscript::{
        Diagnostic, DiagnosticFix, DiagnosticLabelStyle, FixApplicability, TextEdit,
        compiler::ast::Span,
    };

    let source = "first\nsecond\n";
    let diagnostic = Diagnostic::type_error("values are reversed", Span { start: 6, end: 12 })
        .with_primary_label("this value belongs first")
        .with_secondary_label(Span { start: 0, end: 5 }, "this value belongs second")
        .with_note("the two values must appear in declaration order")
        .with_fix(DiagnosticFix {
            title: "swap the values".to_owned(),
            applicability: FixApplicability::MachineApplicable,
            edits: vec![
                TextEdit {
                    span: Span { start: 0, end: 5 },
                    replacement: "second".to_owned(),
                },
                TextEdit {
                    span: Span { start: 6, end: 12 },
                    replacement: "first".to_owned(),
                },
            ],
        });

    assert_eq!(diagnostic.labels.len(), 2);
    assert_eq!(diagnostic.labels[0].style, DiagnosticLabelStyle::Primary);
    assert_eq!(diagnostic.labels[1].style, DiagnosticLabelStyle::Secondary);
    assert_eq!(diagnostic.fixes[0].edits.len(), 2);
    assert_eq!(
        diagnostic.render("example.split", source),
        concat!(
            "example.split:2:1: error[SS0003]: values are reversed\n",
            "  = primary: this value belongs first\n",
            "  = secondary example.split:1:1: this value belongs second\n",
            "  = note: the two values must appear in declaration order\n",
            "  = help: swap the values (machine-applicable)"
        )
    );
}

fn assert_snapshot(expected: &str, actual: &str) {
    assert_eq!(actual.trim_end(), expected.trim_end());
}

fn render_typed_hir_snapshot(checked: &splitscript::CheckedProgram) -> String {
    use std::fmt::Write as _;

    let mut output = String::new();
    writeln!(output, "declarations:").unwrap();
    for declaration in checked.hir().declarations() {
        writeln!(output, "  {:?} name={}", declaration.id, declaration.name).unwrap();
    }

    writeln!(output, "signatures:").unwrap();
    for function in &checked.syntax().functions {
        let parameters = function
            .params
            .iter()
            .map(|parameter| {
                snapshot_type_name(
                    checked,
                    checked
                        .semantics()
                        .value_type(parameter.id)
                        .expect("checked parameters have types"),
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let result = snapshot_type_name(
            checked,
            checked
                .semantics()
                .function_result(function.id)
                .expect("checked functions have result types"),
        );
        writeln!(
            output,
            "  FunctionId({}) {}({parameters}) -> {result}",
            function.id.index(),
            function.name
        )
        .unwrap();
    }

    writeln!(output, "bodies:").unwrap();
    for function in checked.typed_hir().function_bodies() {
        writeln!(output, "  function {}:", function.function.function.index()).unwrap();
        render_typed_block(&mut output, &function.body, 2);
    }
    for action in checked.typed_hir().action_bodies() {
        writeln!(output, "  action {}:", action.action.name()).unwrap();
        render_typed_block(&mut output, &action.body, 2);
    }

    writeln!(output, "expressions:").unwrap();
    for expression in checked.typed_hir().expressions() {
        let ty = snapshot_type_name(checked, expression.ty);
        let kind = snapshot_expression_kind(checked, &expression.kind);
        write!(output, "  e{}: {ty} = {kind}", expression.id.index()).unwrap();
        if let Some(resolution) = &expression.resolution {
            write!(output, " resolve={}", stable_resolution_debug(resolution)).unwrap();
        }
        if let Some(conversion) = expression.conversion {
            write!(
                output,
                " convert={:?} {} -> {}",
                conversion.kind,
                snapshot_type_name(checked, conversion.source),
                snapshot_type_name(checked, conversion.target)
            )
            .unwrap();
        }
        writeln!(output).unwrap();
    }
    output
}

fn stable_resolution_debug(
    resolution: &splitscript::compiler::hir::ExpressionResolution,
) -> String {
    let mut rendered = format!("{resolution:?}");
    let mut search_from = 0;
    while let Some(relative_start) = rendered[search_from..].find("TypeId(") {
        let start = search_from + relative_start;
        let value_start = start + "TypeId(".len();
        let Some(relative_end) = rendered[value_start..].find(')') else {
            break;
        };
        let end = value_start + relative_end;
        rendered.replace_range(value_start..end, "_");
        search_from = value_start + 1;
    }
    rendered
}

fn render_typed_block(
    output: &mut String,
    block: &splitscript::compiler::hir::TypedBlock,
    depth: usize,
) {
    use splitscript::compiler::hir::TypedStatementKind;
    use std::fmt::Write as _;

    let indent = "  ".repeat(depth);
    for statement in &block.statements {
        match &statement.kind {
            TypedStatementKind::Variable {
                binding,
                initializer,
            } => {
                writeln!(
                    output,
                    "{indent}let v{} = e{}",
                    binding.value.index(),
                    initializer.index()
                )
                .unwrap();
            }
            TypedStatementKind::Assign {
                assignment,
                op,
                value,
            } => {
                writeln!(
                    output,
                    "{indent}assign a{} -> v{} op={op:?} value=e{}",
                    assignment.id.index(),
                    assignment.target.index(),
                    value.index()
                )
                .unwrap();
            }
            TypedStatementKind::StateAssign {
                assignment,
                target,
                op,
                value,
            } => {
                writeln!(
                    output,
                    "{indent}state-assign a{} -> v{} target=e{} op={op:?} value=e{}",
                    assignment.id.index(),
                    assignment.target.index(),
                    target.index(),
                    value.index()
                )
                .unwrap();
            }
            TypedStatementKind::IndexAssign {
                assignment,
                target,
                op,
                value,
            } => {
                writeln!(
                    output,
                    "{indent}index-assign a{} target=e{} op={op:?} value=e{}",
                    assignment.id.index(),
                    target.index(),
                    value.index()
                )
                .unwrap();
            }
            TypedStatementKind::If {
                condition,
                then_block,
                else_block,
            } => {
                writeln!(output, "{indent}if e{}:", condition.index()).unwrap();
                render_typed_block(output, then_block, depth + 1);
                if let Some(else_block) = else_block {
                    writeln!(output, "{indent}else:").unwrap();
                    render_typed_block(output, else_block, depth + 1);
                }
            }
            TypedStatementKind::While { condition, body } => {
                writeln!(output, "{indent}while e{}:", condition.index()).unwrap();
                render_typed_block(output, body, depth + 1);
            }
            TypedStatementKind::For {
                binding,
                iterable_value,
                index_value,
                version_value,
                iterable,
                body,
            } => {
                writeln!(
                    output,
                    "{indent}for v{} in e{} storage=v{} index=v{} version=v{}:",
                    binding.value.index(),
                    iterable.index(),
                    iterable_value.index(),
                    index_value.index(),
                    version_value.index()
                )
                .unwrap();
                render_typed_block(output, body, depth + 1);
            }
            TypedStatementKind::Suspend {
                mode,
                binding,
                value,
                ..
            } => {
                writeln!(
                    output,
                    "{indent}suspend {mode:?} binding={binding:?} value=e{}",
                    value.index()
                )
                .unwrap();
            }
            TypedStatementKind::Yield { value } => {
                writeln!(output, "{indent}yield e{}", value.index()).unwrap();
            }
            TypedStatementKind::Expression(expression) => {
                writeln!(output, "{indent}evaluate e{}", expression.index()).unwrap();
            }
        }
    }
}

fn snapshot_expression_kind(
    checked: &splitscript::CheckedProgram,
    kind: &splitscript::compiler::hir::TypedExpressionKind,
) -> String {
    use splitscript::compiler::hir::{TypedExpressionKind, TypedInterpolatedPart};

    match kind {
        TypedExpressionKind::None => "None".to_owned(),
        TypedExpressionKind::IteratorEnd => "End".to_owned(),
        TypedExpressionKind::Bool(value) => value.to_string(),
        TypedExpressionKind::Int {
            value,
            negative,
            suffix,
        } => format!(
            "int {}{value} suffix={suffix:?}",
            if *negative { "-" } else { "" }
        ),
        TypedExpressionKind::Float(literal) => format!("float {}", literal.value),
        TypedExpressionKind::Char(value) => format!("char {value:?}"),
        TypedExpressionKind::String(value) => format!("string {value:?}"),
        TypedExpressionKind::InterpolatedString(parts) => format!(
            "interpolate [{}]",
            parts
                .iter()
                .map(|part| match part {
                    TypedInterpolatedPart::Text(text) => format!("text {text:?}"),
                    TypedInterpolatedPart::Expression {
                        expression,
                        conversion,
                    } => format!(
                        "e{} conversion={}",
                        expression.index(),
                        conversion.map_or_else(
                            || "none".to_owned(),
                            |splitscript::compiler::hir::ImplicitConversion::ToString {
                                 source,
                             }| {
                                format!("ToString<{}>", snapshot_type_name(checked, source))
                            }
                        )
                    ),
                })
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypedExpressionKind::Signature(value) => format!("signature {value:?}"),
        TypedExpressionKind::Array(values) => format!("array {values:?}"),
        TypedExpressionKind::Range { start, end, kind } => format!(
            "range e{} {} e{}",
            start.index(),
            kind.operator(),
            end.index()
        ),
        TypedExpressionKind::Block { statements, value } => format!(
            "block statements={} value={value:?}",
            statements.statements.len()
        ),
        TypedExpressionKind::Loop { body } => {
            format!("loop statements={}", body.statements.len())
        }
        TypedExpressionKind::Struct { structure, fields } => {
            format!("struct {structure} fields={fields:?}")
        }
        TypedExpressionKind::Enum {
            enumeration,
            variant,
            payload,
        } => format!("enum {enumeration}.{variant} payload={payload:?}"),
        TypedExpressionKind::Match { value, arms } => format!(
            "match e{} arms=[{}]",
            value.index(),
            arms.iter()
                .map(|arm| format!(
                    "p{} {:?} guard={:?} value=e{}",
                    arm.resolution.id.index(),
                    arm.pattern,
                    arm.guard,
                    arm.value.index()
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypedExpressionKind::Is { value, pattern } => format!(
            "is e{} p{} {:?}",
            value.index(),
            pattern.resolution.id.index(),
            pattern.pattern
        ),
        TypedExpressionKind::If {
            condition,
            then_expr,
            else_expr,
        } => format!(
            "if e{} then=e{} else=e{}",
            condition.index(),
            then_expr.index(),
            else_expr.index()
        ),
        TypedExpressionKind::Fallback { value, fallback } => {
            format!("fallback e{} else=e{}", value.index(), fallback.index())
        }
        TypedExpressionKind::Break(value) => format!("break {value:?}"),
        TypedExpressionKind::Continue => "continue".to_owned(),
        TypedExpressionKind::Return(value) => format!("return {value:?}"),
        TypedExpressionKind::Throw { error, target } => format!(
            "throw e{} -> {}",
            error.index(),
            render_failure_target(*target)
        ),
        TypedExpressionKind::Inspect { label, value } => {
            format!("inspect {label:?} e{}", value.index())
        }
        TypedExpressionKind::Suspend { mode, value, .. } => {
            format!("{mode:?} e{}", value.index())
        }
        TypedExpressionKind::Propagate { value, target } => {
            format!(
                "propagate e{} -> {}",
                value.index(),
                render_failure_target(*target)
            )
        }
        TypedExpressionKind::Path(path) => format!("path {}", path.join(".")),
        TypedExpressionKind::Member { receiver, name, .. } => {
            format!("member e{}.{}", receiver.index(), name)
        }
        TypedExpressionKind::Index { receiver, index } => {
            format!("index e{}[e{}]", receiver.index(), index.index())
        }
        TypedExpressionKind::Unary { op, expression } => {
            format!("{op:?} e{}", expression.index())
        }
        TypedExpressionKind::Cast { expression, target } => {
            format!("cast e{} as {target:?}", expression.index())
        }
        TypedExpressionKind::Binary { op, left, right } => {
            format!("{op:?} e{} e{}", left.index(), right.index())
        }
        TypedExpressionKind::Call {
            source_path,
            arguments,
            ..
        } => format!("call {} args={arguments:?}", source_path.join(".")),
        TypedExpressionKind::Invoke { callee, arguments } => {
            format!("invoke e{} args={arguments:?}", callee.index())
        }
        TypedExpressionKind::Closure { parameters, body } => {
            format!("closure params={parameters:?} body=e{}", body.index())
        }
    }
}

fn render_failure_target(target: splitscript::compiler::hir::FailureTarget) -> String {
    match target {
        splitscript::compiler::hir::FailureTarget::Return(result) => {
            format!("t{}", result.index())
        }
        splitscript::compiler::hir::FailureTarget::Retry { expression, result } => {
            format!("retry e{} t{}", expression.index(), result.index())
        }
    }
}

fn snapshot_type_name(
    checked: &splitscript::CheckedProgram,
    ty: splitscript::compiler::types::TypeId,
) -> String {
    match checked.semantics().types().kind(ty) {
        TypeKind::Error => "<unknown>".to_owned(),
        TypeKind::Builtin(builtin) => builtin.to_string(),
        TypeKind::Standard(standard) => StandardLibrary::new().type_decl(*standard).name.to_owned(),
        TypeKind::StateSnapshot => "StateSnapshot".to_owned(),
        TypeKind::SettingsView => "SettingsView".to_owned(),
        TypeKind::Struct(id) => checked
            .syntax()
            .structs
            .iter()
            .find(|structure| structure.id == *id)
            .map(|structure| structure.name.clone())
            .unwrap_or_else(|| format!("struct#{id}")),
        TypeKind::Enum(id) => checked
            .enum_types()
            .iter()
            .find(|enumeration| enumeration.id == *id)
            .map(|enumeration| enumeration.name.clone())
            .unwrap_or_else(|| format!("enum#{id}")),
        TypeKind::ManagedClass(id) => checked
            .syntax()
            .managed_class(*id)
            .map(|class| class.name.clone())
            .unwrap_or_else(|| format!("class#{id}")),
        TypeKind::ManagedReference(id) => checked
            .syntax()
            .managed_class(*id)
            .map(|class| format!("{}.Ref", class.name))
            .unwrap_or_else(|| format!("class#{id}.Ref")),
        TypeKind::GenericParameter { index, .. } => match index {
            0..=25 => char::from_u32('T' as u32 + index).unwrap().to_string(),
            _ => format!("T{}", index + 1),
        },
        TypeKind::Array { element, .. } => {
            format!("[{}]", snapshot_type_name(checked, *element))
        }
        TypeKind::Set { element, .. } => {
            format!("Set<{}>", snapshot_type_name(checked, *element))
        }
        TypeKind::Application {
            constructor,
            arguments,
            ..
        } => {
            let name = StandardLibrary::new().type_constructor(*constructor).name;
            format!(
                "{name}<{}>",
                arguments
                    .iter()
                    .map(|argument| snapshot_type_name(checked, *argument))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
        TypeKind::Option { value, .. } => format!("{}?", snapshot_type_name(checked, *value)),
        TypeKind::Result { value, .. } => format!("{}!", snapshot_type_name(checked, *value)),
        TypeKind::Async { value, .. } => {
            format!("async {}", snapshot_type_name(checked, *value))
        }
        TypeKind::Iterator { item, .. } => {
            format!("iterator {}", snapshot_type_name(checked, *item))
        }
        TypeKind::Callable {
            parameters, result, ..
        } => format!(
            "({}) -> {}",
            parameters
                .iter()
                .map(|parameter| snapshot_type_name(checked, *parameter))
                .collect::<Vec<_>>()
                .join(", "),
            snapshot_type_name(checked, *result)
        ),
        TypeKind::Range { bound, kind, .. } => {
            let bound = snapshot_type_name(checked, *bound);
            format!("{bound}{}{bound}", kind.operator())
        }
    }
}
