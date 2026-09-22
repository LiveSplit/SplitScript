//! Expressions, patterns, precedence, and expression-local recovery.

//! Expression grammar.

use super::{
    ArrayPattern, BinaryOp, DelimiterDepth, Diagnostic, EnumReference, Expr, ExprKind,
    InterpolatedPart, MatchArm, MatchPattern, Parser, PatternBinding, RangeKind, Span, TokenKind,
    TypeRef, UnaryOp, ambiguous_range_diagnostic, assignment_operator, parse_integer,
};
use crate::ast::{PatternNode, StructLiteralField, StructPatternField};
use crate::diagnostic::{DiagnosticCode, DiagnosticFix, FixApplicability, TextEdit};

const IF_EXPRESSION_MISSING_ELSE: &str = "an `if` expression needs an `else` branch";

impl Parser<'_> {
    pub(super) fn expression(&mut self, min_precedence: u8) -> Result<Expr, Diagnostic> {
        let mut left = self.prefix(min_precedence)?;
        let mut saw_comparison = false;
        loop {
            if self.line_break_before_current()
                && matches!(
                    &left.kind,
                    ExprKind::Suspend { value, .. } if matches!(value.kind, ExprKind::Error)
                )
            {
                // A recovered suspension operand owns only its source line.
                // Do not reinterpret an operator starting the next malformed
                // statement as a continuation of this expression.
                break;
            }
            if (self.at(&TokenKind::LParen) || self.begins_generic_call())
                && matches!(&left.kind, ExprKind::Member { .. })
            {
                let start = left.span;
                let ExprKind::Member {
                    receiver,
                    name,
                    name_span,
                } = left.kind
                else {
                    unreachable!()
                };
                let mut callee = Vec::new();
                let receiver = flatten_postfix_receiver(*receiver, &mut callee);
                callee.push(name);
                let (type_arguments, type_argument_span) = self.call_type_arguments()?;
                self.expect(TokenKind::LParen, "expected `(` after generic arguments")?;
                let (args, end) =
                    self.expression_list(TokenKind::RParen, "expected `)` after arguments", true);
                left = self.new_expr(
                    ExprKind::Call {
                        callee,
                        name_span,
                        receiver: Some(Box::new(receiver)),
                        type_arguments,
                        type_argument_span,
                        args,
                    },
                    start.join(end),
                );
                continue;
            }
            if self.eat(&TokenKind::LParen).is_some() {
                let start = left.span;
                let (args, end) =
                    self.expression_list(TokenKind::RParen, "expected `)` after arguments", true);
                left = self.new_expr(
                    ExprKind::Invoke {
                        callee: Box::new(left),
                        args,
                    },
                    start.join(end),
                );
                continue;
            }
            if self.eat(&TokenKind::Dot).is_some() {
                let (name, name_span) = self.expect_any_ident("expected a field name after `.`")?;
                let span = left.span.join(name_span);
                left = self.new_expr(
                    ExprKind::Member {
                        receiver: Box::new(left),
                        name,
                        name_span,
                    },
                    span,
                );
                continue;
            }
            if let Some(opening) = self.eat(&TokenKind::LBracket) {
                let index =
                    self.with_struct_literals(true, |parser| parser.required_expression(0))?;
                let closing = self.expect(TokenKind::RBracket, "expected `]` after the index")?;
                let span = left.span.join(closing);
                left = self.new_expr(
                    ExprKind::Index {
                        receiver: Box::new(left),
                        index: Box::new(index),
                        bracket_span: opening.join(closing),
                    },
                    span,
                );
                continue;
            }
            if let Some(question) = self.eat(&TokenKind::Question) {
                let span = left.span.join(question);
                left = self.new_expr(ExprKind::Propagate(Box::new(left)), span);
                continue;
            }
            const FALLBACK_PRECEDENCE: u8 = 0;
            if self.at_ident("else") {
                if FALLBACK_PRECEDENCE < min_precedence {
                    break;
                }
                let ambiguous_retry = match &left.kind {
                    ExprKind::Suspend {
                        mode: super::SuspensionMode::Retry,
                        value,
                        ..
                    } if self
                        .source
                        .get(left.span.start..left.span.end)
                        .is_some_and(|source| source.starts_with("retry")) =>
                    {
                        Some((left.span, value.span.start))
                    }
                    _ => None,
                };
                self.bump();
                let fallback = self.required_expression(FALLBACK_PRECEDENCE)?;
                let span = left.span.join(fallback.span);
                if let Some((retry_span, operand_start)) = ambiguous_retry {
                    self.diagnostics.push(
                        Diagnostic::warning(
                            DiagnosticCode::AmbiguousRetryFallback,
                            "`retry` binds more tightly than fallback `else`",
                            retry_span,
                        )
                        .with_primary_label("only this expression is inside the retry boundary")
                        .with_secondary_label(
                            fallback.span,
                            "this fallback is evaluated after retry completes",
                        )
                        .with_note(
                            "the current grouping is `(retry value) else fallback`; use parentheses to make either interpretation explicit",
                        )
                        .with_fix(DiagnosticFix {
                            title: "make the current retry boundary explicit".to_owned(),
                            applicability: FixApplicability::MachineApplicable,
                            edits: vec![
                                TextEdit {
                                    span: Span {
                                        start: retry_span.start,
                                        end: retry_span.start,
                                    },
                                    replacement: "(".to_owned(),
                                },
                                TextEdit {
                                    span: Span {
                                        start: retry_span.end,
                                        end: retry_span.end,
                                    },
                                    replacement: ")".to_owned(),
                                },
                            ],
                        })
                        .with_fix(DiagnosticFix {
                            title: "retry the complete fallback chain".to_owned(),
                            applicability: FixApplicability::MaybeIncorrect,
                            edits: vec![
                                TextEdit {
                                    span: Span {
                                        start: operand_start,
                                        end: operand_start,
                                    },
                                    replacement: "(".to_owned(),
                                },
                                TextEdit {
                                    span: Span {
                                        start: span.end,
                                        end: span.end,
                                    },
                                    replacement: ")".to_owned(),
                                },
                            ],
                        }),
                    );
                }
                left = self.new_expr(
                    ExprKind::Fallback {
                        value: Box::new(left),
                        fallback: Box::new(fallback),
                    },
                    span,
                );
                continue;
            }
            const RANGE_PRECEDENCE: u8 = 1;
            if matches!(
                self.current().kind,
                TokenKind::DotDot | TokenKind::DotDotLt | TokenKind::DotDotEq
            ) {
                if RANGE_PRECEDENCE < min_precedence {
                    break;
                }
                if matches!(left.kind, ExprKind::Range { .. }) {
                    return Err(self
                        .error("range operators cannot be chained; use two named ranges instead"));
                }
                let operator = self.bump().clone();
                let kind = match operator.kind {
                    TokenKind::DotDotLt => super::RangeKind::Exclusive,
                    TokenKind::DotDotEq => super::RangeKind::Inclusive,
                    TokenKind::DotDot => {
                        return Err(ambiguous_range_diagnostic(operator.span));
                    }
                    _ => unreachable!(),
                };
                let right = self.required_expression(RANGE_PRECEDENCE + 1)?;
                let span = left.span.join(right.span);
                left = self.new_expr(
                    ExprKind::Range {
                        start: Box::new(left),
                        end: Box::new(right),
                        kind,
                        operator_span: operator.span,
                    },
                    span,
                );
                continue;
            }
            const CAST_PRECEDENCE: u8 = 11;
            if self.at_ident("as") {
                if CAST_PRECEDENCE < min_precedence {
                    break;
                }
                self.bump();
                let (target, target_span) = self.parse_type("expected a type after `as`")?;
                let span = left.span.join(target_span);
                left = self.new_expr(
                    ExprKind::Cast {
                        expr: Box::new(left),
                        target,
                    },
                    span,
                );
                continue;
            }
            const IS_PRECEDENCE: u8 = 4;
            if self.at_ident("is") {
                if IS_PRECEDENCE < min_precedence {
                    break;
                }
                if saw_comparison {
                    return Err(self.error(
                        "comparison and `is` operators cannot be chained; use parentheses to disambiguate",
                    ));
                }
                let keyword_span = self.bump().span;
                let pattern = self.match_pattern(true)?;
                let span = left.span.join(pattern.span);
                left = self.new_expr(
                    ExprKind::Is {
                        value: Box::new(left),
                        pattern,
                        keyword_span,
                    },
                    span,
                );
                saw_comparison = true;
                continue;
            }
            let Some((precedence, op)) = self.binary_operator() else {
                break;
            };
            if precedence < min_precedence {
                break;
            }
            let operator_span = self.current().span;
            if let Some(spelling) = self.source.get(operator_span.start..operator_span.end)
                && matches!(spelling, "===" | "!==")
            {
                self.record_foreign_spelling_diagnostic(
                    operator_span,
                    spelling,
                    crate::migration::ForeignSpellingContext::Operator,
                );
            }
            let is_comparison = matches!(
                op,
                BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge
            );
            if is_comparison && saw_comparison {
                return Err(self.error(
                    "comparison operators cannot be chained; use parentheses to disambiguate",
                ));
            }
            self.bump();
            let right = self.required_expression(precedence + 1)?;
            let span = left.span.join(right.span);
            left = self.new_expr(
                ExprKind::Binary {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                },
                span,
            );
            saw_comparison |= is_comparison;
        }
        Ok(left)
    }

    pub(super) fn prefix(&mut self, min_precedence: u8) -> Result<Expr, Diagnostic> {
        if min_precedence == 0
            && let TokenKind::Ident(name) = self.current().kind.clone()
            && name != "_"
            && self.peek(1).kind == TokenKind::FatArrow
        {
            let (name, name_span) =
                self.expect_declared_ident("expected a closure parameter name")?;
            let arrow_span = self.bump().span;
            let pattern = super::PatternNode {
                id: self.new_pattern_id(),
                kind: super::MatchPattern::Binding(super::PatternBinding {
                    id: self.new_value_id(),
                    name,
                    name_span,
                }),
                span: name_span,
            };
            let parameter = super::Parameter {
                binding: self.binding_pattern(pattern),
                annotation: None,
                span: name_span,
            };
            let body = self.required_expression(0)?;
            let span = name_span.join(body.span);
            return Ok(self.new_expr(
                ExprKind::Closure {
                    params: vec![parameter],
                    return_annotation: None,
                    return_annotation_span: None,
                    arrow_span,
                    body: Box::new(body),
                },
                span,
            ));
        }
        if min_precedence == 0 && self.begins_parenthesized_closure() {
            let start = self.expect(TokenKind::LParen, "expected `(`")?;
            let mut params = Vec::new();
            while !self.at(&TokenKind::RParen) {
                let pattern = self.match_pattern(true)?;
                let (annotation, end) = if self.eat(&TokenKind::Colon).is_some() {
                    let (ty, span) = self.parse_type("expected a closure parameter type")?;
                    (Some(ty), span)
                } else {
                    (None, pattern.span)
                };
                params.push(super::Parameter {
                    span: pattern.span.join(end),
                    binding: self.binding_pattern(pattern),
                    annotation,
                });
                if self.eat(&TokenKind::Comma).is_none() {
                    break;
                }
                if self.at(&TokenKind::RParen) {
                    break;
                }
            }
            self.expect(TokenKind::RParen, "expected `)` after closure parameters")?;
            let (return_annotation, return_annotation_span) =
                if self.eat(&TokenKind::Minus).is_some() {
                    self.expect(
                        TokenKind::Gt,
                        "expected `>` in the closure return arrow `->`",
                    )?;
                    let (ty, span) = self.parse_type("expected a closure return type")?;
                    (Some(ty), Some(span))
                } else {
                    (None, None)
                };
            let arrow_span = self.expect(
                TokenKind::FatArrow,
                "expected `=>` after the closure signature",
            )?;
            let body = self.required_expression(0)?;
            let span = start.join(body.span);
            return Ok(self.new_expr(
                ExprKind::Closure {
                    params,
                    return_annotation,
                    return_annotation_span,
                    arrow_span,
                    body: Box::new(body),
                },
                span,
            ));
        }
        if self.eat_ident("return").is_some() {
            let start = self.previous().span;
            let value = self.optional_control_flow_value()?;
            let span = value.as_ref().map_or(start, |value| start.join(value.span));
            return Ok(self.new_expr(ExprKind::Return(value.map(Box::new)), span));
        }
        if self.eat_ident("break").is_some() {
            let start = self.previous().span;
            let value = self.optional_control_flow_value()?;
            let span = value.as_ref().map_or(start, |value| start.join(value.span));
            return Ok(self.new_expr(ExprKind::Break(value.map(Box::new)), span));
        }
        if self.eat_ident("continue").is_some() {
            let span = self.previous().span;
            return Ok(self.new_expr(ExprKind::Continue, span));
        }
        if self.eat_ident("throw").is_some() {
            let start = self.previous().span;
            let error = self.required_expression(0)?;
            let span = start.join(error.span);
            return Ok(self.new_expr(ExprKind::Throw(Box::new(error)), span));
        }
        if self.eat_ident("inspect").is_some() {
            let keyword_span = self.previous().span;
            self.expect(
                TokenKind::LParen,
                "expected `(` after `inspect`; write `inspect(expression)`",
            )?;
            let value = self.with_struct_literals(true, |parser| parser.required_expression(0))?;
            let closing =
                self.expect(TokenKind::RParen, "expected `)` after inspected expression")?;
            let label = self
                .source
                .get(value.span.start..value.span.end)
                .unwrap_or_default()
                .trim()
                .to_owned();
            let span = keyword_span.join(closing);
            return Ok(self.new_expr(
                ExprKind::Inspect {
                    label,
                    value: Box::new(value),
                    keyword_span,
                },
                span,
            ));
        }
        if self.at_ident("await") || self.at_ident("retry") {
            let mode = if self.eat_ident("await").is_some() {
                super::SuspensionMode::Await
            } else {
                self.expect_ident("retry")?;
                super::SuspensionMode::Retry
            };
            let start = self.previous().span;
            let expression_start = self.cursor.position();
            let expression_depth = self.cursor.delimiter_depth();
            let parsed = if self.expression_is_missing_before_statement() {
                Err(self.error("expected an expression"))
            } else {
                self.expression(12)
            };
            let value = self.recover_root_expression(parsed, expression_start, expression_depth);
            let span = start.join(value.span);
            let destination = self.new_value_id();
            return Ok(self.new_expr(
                ExprKind::Suspend {
                    mode,
                    destination,
                    value: Box::new(value),
                },
                span,
            ));
        }
        if self.eat_ident("if").is_some() {
            let start = self.previous().span;
            return self.if_expression(start);
        }
        if self.eat_ident("match").is_some() {
            let start = self.previous().span;
            let value = self.required_expression_before_block()?;
            self.expect(TokenKind::LBrace, "expected `{` after the matched value")?;
            let body_depth = self.cursor.brace_depth();
            let mut arms = Vec::new();
            while !self.at(&TokenKind::RBrace) {
                if self.at(&TokenKind::Eof) {
                    self.record_missing_closing("unterminated match expression");
                    break;
                }
                let item_start = self.cursor.position();
                let parsed = self.match_arm();
                if let Some(arm) = self.recover_delimited_item(parsed, item_start, body_depth) {
                    arms.push(arm);
                }
            }
            let end = self.eat(&TokenKind::RBrace).unwrap_or(self.current().span);
            return Ok(self.new_expr(
                ExprKind::Match {
                    value: Box::new(value),
                    arms,
                },
                start.join(end),
            ));
        }
        if self.eat_ident("loop").is_some() {
            let start = self.previous().span;
            let block = self.block()?;
            let span = start.join(block.span);
            return Ok(self.new_expr(ExprKind::Loop(block), span));
        }
        if self.at(&TokenKind::LBrace) {
            let block = self.block()?;
            let block = self.normalize_value_block(block);
            return Ok(self.value_block(block));
        }
        if self.eat(&TokenKind::Bang).is_some() {
            let start = self.previous().span;
            let expr = self.required_expression(12)?;
            let span = start.join(expr.span);
            return Ok(self.new_expr(
                ExprKind::Unary {
                    op: UnaryOp::Not,
                    expr: Box::new(expr),
                },
                span,
            ));
        }
        if self.at(&TokenKind::Minus) && matches!(self.peek(1).kind, TokenKind::Int(_)) {
            let start = self.current().span;
            self.bump();
            let token = self.current().clone();
            let TokenKind::Int(text) = token.kind else {
                unreachable!("a negative integer literal was checked above")
            };
            self.bump();
            let (value, suffix) =
                parse_integer(&text).map_err(|message| Diagnostic::new(message, token.span))?;
            return Ok(self.new_expr(
                ExprKind::Int {
                    value,
                    negative: true,
                    suffix,
                },
                start.join(token.span),
            ));
        }
        if self.eat(&TokenKind::Minus).is_some() {
            let start = self.previous().span;
            let expr = self.required_expression(12)?;
            let span = start.join(expr.span);
            return Ok(self.new_expr(
                ExprKind::Unary {
                    op: UnaryOp::Neg,
                    expr: Box::new(expr),
                },
                span,
            ));
        }
        if self.at(&TokenKind::Tilde) {
            let start = self.current().span;
            self.record_foreign_spelling_diagnostic(
                start,
                "~",
                crate::migration::ForeignSpellingContext::Operator,
            );
            self.bump();
            let expr = self.required_expression(12)?;
            let span = start.join(expr.span);
            return Ok(self.new_expr(
                ExprKind::Unary {
                    op: UnaryOp::Not,
                    expr: Box::new(expr),
                },
                span,
            ));
        }
        if self.eat(&TokenKind::LParen).is_some() {
            let start = self.previous().span;
            let target_depth = self.cursor.delimiter_depth();
            let mut expr =
                self.with_struct_literals(true, |parser| parser.required_expression(0))?;
            let end = if let Some(end) = self.eat(&TokenKind::RParen) {
                end
            } else {
                let error = self.error("expected `)` after expression");
                self.record_recovery_diagnostic(error);
                let skipped_start = self.current().span.start;
                self.synchronize_delimited_expression(&TokenKind::RParen, target_depth);
                self.record_error_region(skipped_start, self.current().span.start);
                self.eat(&TokenKind::RParen).unwrap_or(expr.span)
            };
            expr.span = start.join(end);
            return Ok(expr);
        }
        if self.eat(&TokenKind::LBracket).is_some() {
            let start = self.previous().span;
            let (elements, end) = self.expression_list(
                TokenKind::RBracket,
                "expected `]` after array elements",
                true,
            );
            return Ok(self.new_expr(ExprKind::Array(elements), start.join(end)));
        }
        if self.eat(&TokenKind::TemplateStart).is_some() {
            let start = self.previous().span;
            let mut parts = Vec::new();
            let mut has_expression = false;
            loop {
                match self.current().kind.clone() {
                    TokenKind::TemplateChunk(value) => {
                        self.bump();
                        parts.push(InterpolatedPart::Text(value));
                    }
                    TokenKind::TemplateExprStart => {
                        self.record_javascript_style_interpolation();
                        self.bump();
                        let expression_start = self.cursor.position();
                        if let Some(value) = self.interpolated_expression(expression_start) {
                            has_expression = true;
                            parts.push(InterpolatedPart::Expr(value));
                        }
                    }
                    TokenKind::TemplateEnd => {
                        let end = self.bump().span;
                        if has_expression {
                            return Ok(
                                self.new_expr(ExprKind::InterpolatedString(parts), start.join(end))
                            );
                        }
                        let value = parts
                            .into_iter()
                            .map(|part| match part {
                                InterpolatedPart::Text(value) => value,
                                InterpolatedPart::Expr(_) => unreachable!(),
                            })
                            .collect();
                        return Ok(self.new_expr(ExprKind::String(value), start.join(end)));
                    }
                    _ => {
                        return Err(self.error(
                            "expected template text, an interpolation, or a closing backtick",
                        ));
                    }
                }
            }
        }

        let token = self.current().clone();
        match token.kind {
            TokenKind::Ident(name) if name == "None" => {
                self.bump();
                Ok(self.new_expr(ExprKind::None, token.span))
            }
            TokenKind::Ident(name) if name == "End" => {
                self.bump();
                Ok(self.new_expr(ExprKind::IteratorEnd, token.span))
            }
            TokenKind::Ident(name) if name == "null" => {
                self.record_foreign_spelling_diagnostic(
                    token.span,
                    "null",
                    crate::migration::ForeignSpellingContext::OptionalValue,
                );
                self.bump();
                Ok(self.new_expr(ExprKind::None, token.span))
            }
            TokenKind::Ident(name) if name == "true" => {
                self.bump();
                Ok(self.new_expr(ExprKind::Bool(true), token.span))
            }
            TokenKind::Ident(name) if name == "false" => {
                self.bump();
                Ok(self.new_expr(ExprKind::Bool(false), token.span))
            }
            TokenKind::Ident(mut first) => {
                self.bump();
                if self.at(&TokenKind::Dot)
                    && let Some(replacement) = self.record_foreign_spelling_diagnostic(
                        token.span,
                        &first,
                        crate::migration::ForeignSpellingContext::StaticTypeReceiver,
                    )
                {
                    first = replacement.to_owned();
                }
                if first == "sig" {
                    let signature_span = self.current().span;
                    let value = self.expect_string("expected a quoted pattern after `sig`")?;
                    return Ok(
                        self.new_expr(ExprKind::Signature(value), token.span.join(signature_span))
                    );
                }
                if first == "v" {
                    let version_span = self.current().span;
                    let value = self.expect_string("expected a quoted version after `v`")?;
                    let components = parse_file_version(&value)
                        .map_err(|message| Diagnostic::new(message, version_span))?;
                    let args = components
                        .into_iter()
                        .map(|value| {
                            self.new_expr(
                                ExprKind::Int {
                                    value: u64::from(value),
                                    negative: false,
                                    suffix: None,
                                },
                                version_span,
                            )
                        })
                        .collect();
                    return Ok(self.new_expr(
                        ExprKind::Call {
                            callee: vec!["FileVersion".to_owned(), "fromParts".to_owned()],
                            // The source spelling is a literal, not a visible
                            // call site. Keep the lowering target out of
                            // position-based tooling while preserving the
                            // complete literal span on the expression.
                            name_span: Span {
                                start: token.span.end,
                                end: token.span.end,
                            },
                            receiver: None,
                            type_arguments: Vec::new(),
                            type_argument_span: None,
                            args,
                        },
                        token.span.join(version_span),
                    ));
                }
                let begins_struct_literal = self.struct_literals_allowed
                    && self.at(&TokenKind::LBrace)
                    && (matches!(self.peek(1).kind, TokenKind::RBrace)
                        || matches!(
                            (&self.peek(1).kind, &self.peek(2).kind),
                            (
                                TokenKind::Ident(_),
                                TokenKind::Colon | TokenKind::Comma | TokenKind::RBrace,
                            )
                        ));
                if begins_struct_literal && self.eat(&TokenKind::LBrace).is_some() {
                    let body_depth = self.cursor.brace_depth();
                    let mut fields = Vec::new();
                    while !self.at(&TokenKind::RBrace) {
                        if self.at(&TokenKind::Eof) {
                            self.record_missing_closing("unterminated struct literal");
                            break;
                        }
                        let item_start = self.cursor.position();
                        let parsed = self.struct_literal_field();
                        if let Some(field) =
                            self.recover_delimited_item(parsed, item_start, body_depth)
                        {
                            fields.push(field);
                            if self.eat(&TokenKind::Comma).is_some() {
                                continue;
                            }
                            if self.at(&TokenKind::RBrace) {
                                continue;
                            }
                            self.record_missing(Diagnostic::new(
                                "expected `,` between struct fields",
                                self.current().span,
                            ));
                            if matches!(self.current().kind, TokenKind::Ident(_)) {
                                continue;
                            }
                            self.synchronize_delimited_item(item_start, body_depth);
                        }
                    }
                    let end = self.eat(&TokenKind::RBrace).unwrap_or(self.current().span);
                    return Ok(self.new_expr(
                        ExprKind::Struct {
                            name: first,
                            name_span: token.span,
                            fields,
                        },
                        token.span.join(end),
                    ));
                }
                let mut path = vec![first];
                let mut name_span = token.span;
                while self.eat(&TokenKind::Dot).is_some() {
                    let (name, span) = self.expect_any_ident("expected a name after `.`")?;
                    path.push(name);
                    name_span = span;
                }
                if self.at(&TokenKind::LParen) || self.begins_generic_call() {
                    let (type_arguments, type_argument_span) = self.call_type_arguments()?;
                    self.expect(TokenKind::LParen, "expected `(` after generic arguments")?;
                    let (args, end) = self.expression_list(
                        TokenKind::RParen,
                        "expected `)` after arguments",
                        true,
                    );
                    Ok(self.new_expr(
                        ExprKind::Call {
                            callee: path,
                            name_span,
                            receiver: None,
                            type_arguments,
                            type_argument_span,
                            args,
                        },
                        token.span.join(end),
                    ))
                } else {
                    let end = self.previous().span;
                    Ok(self.new_expr(ExprKind::Path(path), token.span.join(end)))
                }
            }
            TokenKind::Int(text) => {
                let (value, suffix) =
                    parse_integer(&text).map_err(|message| Diagnostic::new(message, token.span))?;
                self.bump();
                Ok(self.new_expr(
                    ExprKind::Int {
                        value,
                        negative: false,
                        suffix,
                    },
                    token.span,
                ))
            }
            TokenKind::Float(text) => {
                let normalized = text.replace('_', "");
                let value: f64 = normalized
                    .parse()
                    .map_err(|_| Diagnostic::new("invalid floating-point literal", token.span))?;
                if !value.is_finite() {
                    return Err(Diagnostic::new(
                        "floating-point literal overflows the finite `f64` range",
                        token.span,
                    ));
                }
                let significand = normalized
                    .split_once(['e', 'E'])
                    .map_or(normalized.as_str(), |(significand, _)| significand);
                if value == 0.0
                    && significand
                        .bytes()
                        .any(|digit| matches!(digit, b'1'..=b'9'))
                {
                    return Err(Diagnostic::new(
                        "floating-point literal underflows `f64` to zero",
                        token.span,
                    ));
                }
                self.bump();
                Ok(self.new_expr(
                    ExprKind::Float(crate::ast::FloatLiteral { normalized, value }),
                    token.span,
                ))
            }
            TokenKind::Char(value) => {
                self.bump();
                Ok(self.new_expr(ExprKind::Char(value), token.span))
            }
            TokenKind::String(value) => {
                self.bump();
                Ok(self.new_expr(ExprKind::String(value), token.span))
            }
            _ => Err(Diagnostic::new("expected an expression", token.span)),
        }
    }

    fn begins_parenthesized_closure(&self) -> bool {
        if !self.at(&TokenKind::LParen) {
            return false;
        }
        let mut depth = 0u32;
        for offset in 0.. {
            match self.peek(offset).kind {
                TokenKind::LParen => depth += 1,
                TokenKind::RParen => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return match self.peek(offset + 1).kind {
                            TokenKind::FatArrow => true,
                            TokenKind::Minus if self.peek(offset + 2).kind == TokenKind::Gt => {
                                let mut next = offset + 3;
                                loop {
                                    match self.peek(next).kind {
                                        TokenKind::FatArrow => return true,
                                        TokenKind::Eof
                                        | TokenKind::Semicolon
                                        | TokenKind::LBrace
                                        | TokenKind::RBrace => return false,
                                        _ => next += 1,
                                    }
                                }
                            }
                            _ => false,
                        };
                    }
                }
                TokenKind::Eof => return false,
                _ => {}
            }
        }
        unreachable!()
    }

    fn optional_control_flow_value(&mut self) -> Result<Option<Expr>, Diagnostic> {
        if self.at(&TokenKind::Semicolon)
            || self.at(&TokenKind::Comma)
            || self.at(&TokenKind::RParen)
            || self.at(&TokenKind::RBracket)
            || self.at(&TokenKind::RBrace)
            || self.at(&TokenKind::Eof)
            || self.line_break_before_current()
        {
            Ok(None)
        } else {
            self.expression(0).map(Some)
        }
    }

    pub(super) fn struct_literal_field(&mut self) -> Result<StructLiteralField, Diagnostic> {
        let (name, name_span) = self.expect_any_ident("expected a struct field name")?;
        let (value, shorthand) = if self.eat(&TokenKind::Colon).is_some() {
            (self.expression(0)?, false)
        } else if self.at(&TokenKind::Comma) || self.at(&TokenKind::RBrace) {
            (
                self.new_expr(ExprKind::Path(vec![name.clone()]), name_span),
                true,
            )
        } else {
            return Err(Diagnostic::new(
                "expected `:`, `,`, or `}` after the field name",
                self.current().span,
            ));
        };
        Ok(StructLiteralField {
            name,
            name_span,
            value,
            shorthand,
        })
    }

    /// Recognizes a complete `name<T>(...)` call before committing `<` to the
    /// generic-call grammar. The closing `>` followed by `(` disambiguates it
    /// from an ordinary comparison without making whitespace significant.
    fn begins_generic_call(&self) -> bool {
        if !self.at(&TokenKind::Lt) {
            return false;
        }
        let mut offset = 1usize;
        let mut brackets = 0usize;
        let mut angles = 1usize;
        loop {
            match self.peek(offset).kind {
                TokenKind::LBracket => brackets += 1,
                TokenKind::RBracket if brackets > 0 => brackets -= 1,
                TokenKind::Lt if brackets == 0 => angles += 1,
                TokenKind::Gt if brackets == 0 => {
                    angles -= 1;
                    if angles == 0 {
                        return self.peek(offset + 1).kind == TokenKind::LParen;
                    }
                }
                TokenKind::Shr if brackets == 0 => {
                    if angles < 2 {
                        return false;
                    }
                    angles -= 2;
                    if angles == 0 {
                        return self.peek(offset + 1).kind == TokenKind::LParen;
                    }
                }
                // These contain an assignment after their leading generic
                // closers, so they cannot directly precede a call's `(`.
                TokenKind::Ge | TokenKind::ShrAssign if brackets == 0 => return false,
                TokenKind::Semicolon if brackets == 0 => return false,
                TokenKind::Eof | TokenKind::LBrace | TokenKind::RBrace => {
                    return false;
                }
                _ => {}
            }
            offset += 1;
        }
    }

    fn call_type_arguments(&mut self) -> Result<(Vec<TypeRef>, Option<Span>), Diagnostic> {
        if !self.begins_generic_call() {
            return Ok((Vec::new(), None));
        }
        let opening = self.bump().span;
        let mut arguments = Vec::new();
        loop {
            arguments.push(self.parse_type("expected a type argument after `<`")?.0);
            if self.eat(&TokenKind::Comma).is_some() {
                if let Some(closing) = self.eat_generic_close() {
                    return Ok((arguments, Some(opening.join(closing))));
                }
                continue;
            }
            let closing = self.expect_generic_close("expected `>` after type arguments")?;
            return Ok((arguments, Some(opening.join(closing))));
        }
    }

    pub(super) fn interpolated_expression(&mut self, expression_start: usize) -> Option<Expr> {
        match self.expression(0) {
            Ok(value) => {
                if self.eat(&TokenKind::TemplateExprEnd).is_none() {
                    let error = self.error("expected `}` after the interpolated expression");
                    self.record_recovery_diagnostic(error);
                    let skipped_start = self.current().span.start;
                    self.synchronize_interpolation();
                    self.record_error_region(skipped_start, self.current().span.start);
                    self.eat(&TokenKind::TemplateExprEnd);
                }
                Some(value)
            }
            Err(error) => {
                self.record_recovery_diagnostic(error);
                let skipped_start = self.cursor.tokens()[expression_start].span.start;
                self.synchronize_interpolation();
                self.record_error_region(skipped_start, self.current().span.start);
                self.eat(&TokenKind::TemplateExprEnd);
                None
            }
        }
    }

    pub(super) fn synchronize_interpolation(&mut self) {
        let mut nested_interpolations = 0u32;
        loop {
            match self.current().kind {
                TokenKind::Eof => return,
                TokenKind::TemplateExprEnd if nested_interpolations == 0 => return,
                TokenKind::TemplateExprStart => {
                    nested_interpolations += 1;
                    self.bump();
                }
                TokenKind::TemplateExprEnd => {
                    nested_interpolations -= 1;
                    self.bump();
                }
                _ => {
                    self.bump();
                }
            }
        }
    }

    pub(super) fn recover_required_expression(
        &mut self,
        parsed: Result<Expr, Diagnostic>,
        expression_start: usize,
    ) -> Result<Expr, Diagnostic> {
        match parsed {
            Ok(expression) => Ok(expression),
            Err(error) if self.is_expression_recovery_boundary() => {
                let error_span = error.span;
                self.record_recovery_diagnostic(error);
                let skipped_start = self.cursor.tokens()[expression_start].span.start;
                let skipped_end = self.current().span.start.max(skipped_start);
                self.record_error_region(skipped_start, skipped_end);
                let span = if skipped_end == skipped_start {
                    Span {
                        start: error_span.start,
                        end: error_span.start,
                    }
                } else {
                    Span {
                        start: skipped_start,
                        end: skipped_end,
                    }
                };
                Ok(self.new_expr(ExprKind::Error, span))
            }
            Err(error) => Err(error),
        }
    }

    pub(super) fn root_expression(&mut self) -> Expr {
        let expression_start = self.cursor.position();
        let expression_depth = self.cursor.delimiter_depth();
        let parsed = if self.expression_is_missing_before_statement() {
            Err(self.error("expected an expression"))
        } else {
            self.expression(0)
        };
        self.recover_root_expression(parsed, expression_start, expression_depth)
    }

    pub(super) fn root_expression_before_block(&mut self) -> Expr {
        self.with_struct_literals(false, Self::root_expression)
    }

    pub(super) fn recover_root_expression(
        &mut self,
        parsed: Result<Expr, Diagnostic>,
        expression_start: usize,
        expression_depth: DelimiterDepth,
    ) -> Expr {
        match parsed {
            Ok(expression) => expression,
            Err(error) => {
                let error_span = error.span;
                self.record_recovery_diagnostic(error);
                let skipped_start = self.cursor.tokens()[expression_start].span.start;
                self.synchronize_root_expression(expression_start, expression_depth);
                let skipped_end = self.current().span.start.max(skipped_start);
                self.record_error_region(skipped_start, skipped_end);
                let span = if skipped_end == skipped_start {
                    Span {
                        start: error_span.start,
                        end: error_span.start,
                    }
                } else {
                    Span {
                        start: skipped_start,
                        end: skipped_end,
                    }
                };
                self.new_expr(ExprKind::Error, span)
            }
        }
    }

    pub(super) fn synchronize_root_expression(
        &mut self,
        expression_start: usize,
        target_depth: DelimiterDepth,
    ) {
        loop {
            let depth = self.cursor.delimiter_depth();
            let at_same_brace_depth = depth.braces == target_depth.braces;
            if self.at(&TokenKind::Eof)
                || (self.at(&TokenKind::Semicolon) && at_same_brace_depth)
                || (self.at(&TokenKind::LBrace) && depth == target_depth)
                || (self.at(&TokenKind::RBrace) && depth.braces <= target_depth.braces)
                || (self.at(&TokenKind::RParen)
                    && at_same_brace_depth
                    && depth.parentheses <= target_depth.parentheses)
                || (self.at(&TokenKind::RBracket)
                    && at_same_brace_depth
                    && depth.brackets <= target_depth.brackets)
                || (at_same_brace_depth
                    && self.line_break_before_current()
                    && (self.cursor.position() > expression_start || self.is_statement_start()))
                || (self.cursor.position() > expression_start
                    && depth == target_depth
                    && self.is_top_level_start())
            {
                return;
            }
            self.bump();
        }
    }

    pub(super) fn required_expression(&mut self, min_precedence: u8) -> Result<Expr, Diagnostic> {
        let expression_start = self.cursor.position();
        let parsed = if self.expression_is_missing_before_statement() {
            Err(self.error("expected an expression"))
        } else {
            self.expression(min_precedence)
        };
        self.recover_required_expression(parsed, expression_start)
    }

    fn required_expression_before_block(&mut self) -> Result<Expr, Diagnostic> {
        self.with_struct_literals(false, |parser| parser.required_expression(0))
    }

    pub(super) fn expression_is_missing_before_statement(&self) -> bool {
        if !self.line_break_before_current() {
            return false;
        }
        match &self.current().kind {
            TokenKind::Ident(name) => {
                matches!(
                    name.as_str(),
                    "debug" | "let" | "const" | "var" | "while" | "for"
                ) || assignment_operator(&self.peek(1).kind).is_some()
            }
            _ => false,
        }
    }

    pub(super) fn is_expression_recovery_boundary(&self) -> bool {
        matches!(
            self.current().kind,
            TokenKind::Eof
                | TokenKind::LBrace
                | TokenKind::RParen
                | TokenKind::RBracket
                | TokenKind::RBrace
                | TokenKind::Comma
                | TokenKind::Semicolon
                | TokenKind::TemplateExprEnd
        ) || self.at_ident("else")
            || (self.line_break_before_current() && self.is_statement_start())
    }

    pub(super) fn synchronize_delimited_expression(
        &mut self,
        closing: &TokenKind,
        target_depth: DelimiterDepth,
    ) {
        loop {
            let depth = self.cursor.delimiter_depth();
            if self.at(&TokenKind::Eof)
                || (self.at(closing) && depth == target_depth)
                || self.at(&TokenKind::TemplateExprEnd)
                || (self.at(&TokenKind::LBrace) && depth == target_depth)
                || self.at_ident("else")
                || self.is_expression_list_boundary(closing, depth, target_depth)
                || (depth == target_depth
                    && self.line_break_before_current()
                    && self.is_statement_start())
            {
                return;
            }
            self.bump();
        }
    }

    pub(super) fn match_arm(&mut self) -> Result<MatchArm, Diagnostic> {
        let pattern = self.match_pattern(false)?;
        let pattern_start = pattern.span;
        let pattern_id = pattern.id;
        let guard = if self.eat_ident("if").is_some() {
            // `=>` terminates the guard rather than turning its final bare
            // identifier into a closure parameter.
            Some(self.expression(1)?)
        } else {
            None
        };
        self.expect(TokenKind::FatArrow, "expected `=>` after the pattern")?;
        let value_start = self.current().span.start;
        let starts_with_if = self.at_ident("if");
        let diagnostics_before = self.diagnostics.len();
        let recovery_nodes_before = self.recovery_nodes.len();
        let value = self.expression(0)?;
        let statement_if = starts_with_if
            && matches!(
                self.diagnostics.get(diagnostics_before..),
                Some([diagnostic]) if diagnostic.message == IF_EXPRESSION_MISSING_ELSE
            );
        if statement_if {
            self.diagnostics.truncate(diagnostics_before);
            self.recovery_nodes.truncate(recovery_nodes_before);
            return Err(self.statement_match_arm_diagnostic(value_start, value.span.end));
        }
        if assignment_operator(&self.current().kind).is_some() {
            return Err(self.statement_match_arm_diagnostic(value_start, value.span.end));
        }
        let span = pattern_start.join(value.span);
        let arm = MatchArm {
            pattern_id,
            pattern: pattern.kind,
            guard,
            value,
            span,
        };
        if self.eat(&TokenKind::Comma).is_none() && !self.at(&TokenKind::RBrace) {
            return Err(self.error("expected `,` between match arms"));
        }
        Ok(arm)
    }

    pub(super) fn match_pattern(&mut self, allow_binding: bool) -> Result<PatternNode, Diagnostic> {
        let first = self.match_range_pattern(allow_binding)?;
        if self.eat(&TokenKind::Or).is_none() {
            return Ok(first);
        }

        let pattern_start = first.span;
        let mut alternatives = vec![first];
        loop {
            alternatives.push(self.match_range_pattern(allow_binding)?);
            if self.eat(&TokenKind::Or).is_none() {
                break;
            }
        }

        // All occurrences of one name in an or-pattern denote one logical
        // arm binding. Type checking still validates that every alternative
        // binds the complete set exactly once.
        let mut binding_ids = std::collections::HashMap::new();
        for alternative in &mut alternatives {
            alternative.kind.visit_bindings_mut(&mut |binding| {
                let id = *binding_ids
                    .entry(binding.name.clone())
                    .or_insert(binding.id);
                binding.id = id;
            });
        }

        let span = pattern_start.join(
            alternatives
                .last()
                .expect("an alternation has at least two patterns")
                .span,
        );
        Ok(PatternNode {
            id: self.new_pattern_id(),
            kind: MatchPattern::Alternation(alternatives),
            span,
        })
    }

    /// Parses the range layer between atomic patterns and `|`. Keeping this
    /// precedence explicit makes `1..<5 | 10..=20` a union of two ranges and
    /// prevents expression precedence from leaking into the pattern grammar.
    fn match_range_pattern(&mut self, allow_binding: bool) -> Result<PatternNode, Diagnostic> {
        let start = self.match_pattern_atom(allow_binding)?;
        if !matches!(
            self.current().kind,
            TokenKind::DotDot | TokenKind::DotDotLt | TokenKind::DotDotEq
        ) {
            return Ok(start);
        }

        let operator = self.bump().clone();
        let kind = match operator.kind {
            TokenKind::DotDotLt => RangeKind::Exclusive,
            TokenKind::DotDotEq => RangeKind::Inclusive,
            TokenKind::DotDot => return Err(ambiguous_range_diagnostic(operator.span)),
            _ => unreachable!(),
        };
        let end = self.match_pattern_atom(false)?;
        if matches!(
            self.current().kind,
            TokenKind::DotDot | TokenKind::DotDotLt | TokenKind::DotDotEq
        ) {
            return Err(self.error(
                "range patterns cannot be chained; combine separate ranges with `|` instead",
            ));
        }
        let span = start.span.join(end.span);
        let MatchPattern::Int {
            value: start_value,
            negative: start_negative,
            suffix: start_suffix,
        } = start.kind
        else {
            return Err(Diagnostic::new(
                "an integer range pattern requires an integer literal lower bound",
                start.span,
            ));
        };
        let MatchPattern::Int {
            value: end_value,
            negative: end_negative,
            suffix: end_suffix,
        } = end.kind
        else {
            return Err(Diagnostic::new(
                "an integer range pattern requires an integer literal upper bound",
                end.span,
            ));
        };
        Ok(PatternNode {
            id: self.new_pattern_id(),
            kind: MatchPattern::IntRange {
                start: start_value,
                start_negative,
                start_suffix,
                start_span: start.span,
                end: end_value,
                end_negative,
                end_suffix,
                end_span: end.span,
                kind,
                operator_span: operator.span,
            },
            span,
        })
    }

    fn match_pattern_atom(&mut self, allow_binding: bool) -> Result<PatternNode, Diagnostic> {
        let token = self.current().clone();
        let pattern_start = token.span;
        let kind = match token.kind {
            TokenKind::Ident(name) if name == "_" => {
                self.bump();
                MatchPattern::Wildcard
            }
            TokenKind::Ident(name) if name == "true" => {
                self.bump();
                MatchPattern::Bool(true)
            }
            TokenKind::Ident(name) if name == "false" => {
                self.bump();
                MatchPattern::Bool(false)
            }
            TokenKind::Ident(name) if name == "None" => {
                self.bump();
                MatchPattern::None
            }
            TokenKind::Ident(name) if name == "End" => {
                self.bump();
                MatchPattern::IteratorEnd
            }
            TokenKind::Ident(name) if name == "v" => {
                self.bump();
                let version_span = self.current().span;
                let value = self.expect_string("expected a quoted version after `v`")?;
                MatchPattern::FileVersion(
                    parse_file_version(&value)
                        .map_err(|message| Diagnostic::new(message, version_span))?,
                )
            }
            TokenKind::Ident(name) if name == "null" => {
                self.record_foreign_spelling_diagnostic(
                    token.span,
                    "null",
                    crate::migration::ForeignSpellingContext::OptionalValue,
                );
                self.bump();
                MatchPattern::None
            }
            TokenKind::Ident(name)
                if matches!(name.as_str(), "Some" | "Ok" | "Err" | "Item")
                    && matches!(self.peek(1).kind, TokenKind::LParen) =>
            {
                self.bump();
                self.bump();
                let payload = Box::new(self.match_pattern(true)?);
                self.expect(TokenKind::RParen, "expected `)` after the wrapper pattern")?;
                match name.as_str() {
                    "Some" => MatchPattern::OptionSome(payload),
                    "Ok" => MatchPattern::ResultSuccess(payload),
                    "Err" => MatchPattern::ResultError(payload),
                    "Item" => MatchPattern::IteratorItem(payload),
                    _ => unreachable!(),
                }
            }
            TokenKind::LBrace => {
                self.bump();
                MatchPattern::Struct {
                    name: None,
                    name_span: None,
                    fields: self.struct_pattern_fields()?,
                }
            }
            TokenKind::Ident(enum_name) => {
                self.bump();
                if self.eat(&TokenKind::LBrace).is_some() {
                    MatchPattern::Struct {
                        name: Some(enum_name),
                        name_span: Some(pattern_start),
                        fields: self.struct_pattern_fields()?,
                    }
                } else if self.eat(&TokenKind::Dot).is_some() {
                    let (mut segment, _) = self.expect_any_ident("expected a variant name")?;
                    let mut enumeration = enum_name;
                    while self.eat(&TokenKind::Dot).is_some() {
                        enumeration.push('.');
                        enumeration.push_str(&segment);
                        (segment, _) = self.expect_any_ident("expected a variant name")?;
                    }
                    let variant = segment;
                    let payload = if self.eat(&TokenKind::LParen).is_some() {
                        let payload = Box::new(self.match_pattern(true)?);
                        self.expect(TokenKind::RParen, "expected `)` after the payload pattern")?;
                        Some(payload)
                    } else {
                        None
                    };
                    MatchPattern::Enum {
                        enumeration: EnumReference {
                            name: enumeration,
                            span: pattern_start,
                        },
                        variant,
                        payload,
                    }
                } else if allow_binding {
                    self.record_declared_ident_diagnostic(&enum_name, pattern_start);
                    MatchPattern::Binding(PatternBinding {
                        id: self.new_value_id(),
                        name: enum_name,
                        name_span: pattern_start,
                    })
                } else {
                    return Err(Diagnostic::new(
                        format!(
                            "bare binding `{enum_name}` would match every value; use `Some({enum_name})` or `Ok({enum_name})` to match a wrapper payload"
                        ),
                        pattern_start,
                    ));
                }
            }
            TokenKind::Int(text) => {
                self.bump();
                let (value, suffix) = parse_integer(&text)
                    .map_err(|message| Diagnostic::new(message, pattern_start))?;
                MatchPattern::Int {
                    value,
                    negative: false,
                    suffix,
                }
            }
            TokenKind::Minus if matches!(self.peek(1).kind, TokenKind::Int(_)) => {
                self.bump();
                let token = self.current().clone();
                let TokenKind::Int(text) = token.kind else {
                    unreachable!("a negative integer pattern was checked above")
                };
                self.bump();
                let (value, suffix) =
                    parse_integer(&text).map_err(|message| Diagnostic::new(message, token.span))?;
                MatchPattern::Int {
                    value,
                    negative: true,
                    suffix,
                }
            }
            TokenKind::Char(value) => {
                self.bump();
                MatchPattern::Char(value)
            }
            TokenKind::String(value) => {
                self.bump();
                MatchPattern::String(value)
            }
            TokenKind::LBracket => {
                self.bump();
                let mut prefix = Vec::new();
                let mut rest = None;
                let mut suffix = Vec::new();
                while !self.at(&TokenKind::RBracket) {
                    if self.at(&TokenKind::DotDot) {
                        let rest_span = self.bump().span;
                        if rest.replace(rest_span).is_some() {
                            return Err(Diagnostic::new(
                                "an array pattern can contain only one `..` rest marker",
                                rest_span,
                            ));
                        }
                    } else {
                        let element = self.match_pattern(true)?;
                        if rest.is_some() {
                            suffix.push(element);
                        } else {
                            prefix.push(element);
                        }
                    }
                    if self.eat(&TokenKind::Comma).is_some() {
                        if self.at(&TokenKind::RBracket) {
                            break;
                        }
                    } else if !self.at(&TokenKind::RBracket) {
                        return Err(self.error("expected `,` between array patterns"));
                    }
                }
                self.expect(TokenKind::RBracket, "expected `]` after array pattern")?;
                MatchPattern::Array(ArrayPattern {
                    prefix,
                    rest,
                    suffix,
                })
            }
            _ => {
                return Err(Diagnostic::new(
                    "expected a struct, array, enum variant, string, character, integer, file-version, boolean, `None`, `Some(value)`, `Ok(value)`, `Err(error)`, or `_` pattern",
                    pattern_start,
                ));
            }
        };
        let span = pattern_start.join(self.previous().span);
        Ok(PatternNode {
            id: self.new_pattern_id(),
            kind,
            span,
        })
    }

    fn struct_pattern_fields(&mut self) -> Result<Vec<StructPatternField>, Diagnostic> {
        let mut fields = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            let (field_name, field_span) =
                self.expect_any_ident("expected a struct pattern field name")?;
            let (pattern, shorthand) = if self.eat(&TokenKind::Colon).is_some() {
                (self.match_pattern(true)?, false)
            } else if self.at(&TokenKind::Comma) || self.at(&TokenKind::RBrace) {
                (
                    PatternNode {
                        id: self.new_pattern_id(),
                        kind: MatchPattern::Binding(PatternBinding {
                            id: self.new_value_id(),
                            name: field_name.clone(),
                            name_span: field_span,
                        }),
                        span: field_span,
                    },
                    true,
                )
            } else {
                return Err(Diagnostic::new(
                    "expected `:`, `,`, or `}` after the struct pattern field name",
                    self.current().span,
                ));
            };
            fields.push(StructPatternField {
                name: field_name,
                name_span: field_span,
                pattern,
                shorthand,
            });
            if self.eat(&TokenKind::Comma).is_some() {
                continue;
            }
            if !self.at(&TokenKind::RBrace) {
                return Err(self.error("expected `,` between struct pattern fields"));
            }
        }
        self.expect(TokenKind::RBrace, "expected `}` after the struct pattern")?;
        Ok(fields)
    }

    fn statement_match_arm_diagnostic(&self, start: usize, parsed_end: usize) -> Diagnostic {
        let (body_end, diagnostic_end) = self.match_arm_body_end(parsed_end);
        let body_span = Span {
            start,
            end: body_end,
        };
        let diagnostic_span = Span {
            start,
            end: diagnostic_end,
        };
        let body = self
            .source
            .get(body_span.start..body_span.end)
            .unwrap_or_default();
        Diagnostic::new("a statement-shaped match arm needs braces", diagnostic_span)
            .with_primary_label("this arm body is a statement, not a single expression")
            .with_note(
                "write `pattern => { ... }` around assignments and side-effecting control flow",
            )
            .with_fix(DiagnosticFix {
                title: "wrap the match arm body in braces".to_owned(),
                applicability: FixApplicability::MachineApplicable,
                edits: vec![TextEdit {
                    span: body_span,
                    replacement: format!("{{ {body} }}"),
                }],
            })
    }

    fn match_arm_body_end(&self, parsed_end: usize) -> (usize, usize) {
        let target = self.cursor.delimiter_depth();
        let mut depth = target;
        let mut diagnostic_end = parsed_end;
        for token in &self.cursor.tokens()[self.cursor.position()..] {
            if depth == target && matches!(token.kind, TokenKind::Comma | TokenKind::RBrace) {
                return (token.span.start, diagnostic_end);
            }
            diagnostic_end = token.span.end;
            depth.update(&token.kind);
        }
        (self.source.len(), diagnostic_end)
    }

    pub(super) fn expression_list(
        &mut self,
        closing: TokenKind,
        missing_closing_message: &'static str,
        allow_trailing_comma: bool,
    ) -> (Vec<Expr>, Span) {
        self.with_struct_literals(true, |parser| {
            parser.expression_list_inner(closing, missing_closing_message, allow_trailing_comma)
        })
    }

    fn expression_list_inner(
        &mut self,
        closing: TokenKind,
        missing_closing_message: &'static str,
        allow_trailing_comma: bool,
    ) -> (Vec<Expr>, Span) {
        let target_depth = self.cursor.delimiter_depth();
        let mut expressions = Vec::new();
        loop {
            let depth = self.cursor.delimiter_depth();
            if self.at(&closing) && depth == target_depth {
                return (expressions, self.bump().span);
            }
            if self.is_expression_list_boundary(&closing, depth, target_depth) {
                self.record_missing(Diagnostic::new(
                    missing_closing_message,
                    self.current().span,
                ));
                return (expressions, self.previous().span);
            }

            let item_start = self.cursor.position();
            let parsed = self.expression(0);
            if let Some(expression) =
                self.recover_expression_list_item(parsed, item_start, target_depth, &closing)
            {
                expressions.push(expression);
                if self.eat(&TokenKind::Comma).is_some() {
                    if !allow_trailing_comma
                        && self.at(&closing)
                        && self.cursor.delimiter_depth() == target_depth
                    {
                        self.record_missing(Diagnostic::new(
                            "expected an expression after `,`",
                            self.current().span,
                        ));
                    }
                    continue;
                }
                if self.at(&closing) && self.cursor.delimiter_depth() == target_depth {
                    continue;
                }
                self.record_missing(Diagnostic::new(
                    "expected `,` between expressions",
                    self.current().span,
                ));
                if self.is_expression_start() {
                    continue;
                }
                self.synchronize_expression_list_item(target_depth, &closing);
            }
        }
    }

    pub(super) fn recover_expression_list_item(
        &mut self,
        parsed: Result<Expr, Diagnostic>,
        item_start: usize,
        target_depth: DelimiterDepth,
        closing: &TokenKind,
    ) -> Option<Expr> {
        match parsed {
            Ok(expression) => Some(expression),
            Err(error) => {
                self.record_recovery_diagnostic(error);
                let skipped_start = self.cursor.tokens()[item_start].span.start;
                self.synchronize_expression_list_item(target_depth, closing);
                self.record_error_region(skipped_start, self.current().span.start);
                None
            }
        }
    }

    pub(super) fn synchronize_expression_list_item(
        &mut self,
        target_depth: DelimiterDepth,
        closing: &TokenKind,
    ) {
        loop {
            let depth = self.cursor.delimiter_depth();
            if self.at(&TokenKind::Eof)
                || (self.at(closing) && depth == target_depth)
                || self.is_expression_list_boundary(closing, depth, target_depth)
            {
                return;
            }
            if self.bump().kind == TokenKind::Comma && depth == target_depth {
                return;
            }
        }
    }

    pub(super) fn is_expression_list_boundary(
        &self,
        closing: &TokenKind,
        depth: DelimiterDepth,
        target: DelimiterDepth,
    ) -> bool {
        if self.at(&TokenKind::Eof) || (self.at(&TokenKind::Semicolon) && depth == target) {
            return true;
        }
        match self.current().kind {
            TokenKind::RParen => {
                *closing != TokenKind::RParen && depth.parentheses <= target.parentheses
            }
            TokenKind::RBracket => {
                *closing != TokenKind::RBracket && depth.brackets <= target.brackets
            }
            TokenKind::RBrace => depth.braces <= target.braces,
            _ => false,
        }
    }

    pub(super) fn if_expression(&mut self, start: Span) -> Result<Expr, Diagnostic> {
        let condition = self.required_expression_before_block()?;
        let then_expr = self.value_block_expression("expected `{` after the `if` condition")?;
        let else_expr = if self.eat_ident("else").is_none() {
            let error = Diagnostic::new(IF_EXPRESSION_MISSING_ELSE, self.current().span);
            let span = Span {
                start: error.span.start,
                end: error.span.start,
            };
            self.record_missing(error);
            self.new_expr(ExprKind::Error, span)
        } else if self.eat_ident("if").is_some() {
            let nested_start = self.previous().span;
            self.if_expression(nested_start)?
        } else {
            self.value_block_expression("expected `{` after `else`")?
        };
        let span = start.join(else_expr.span);
        Ok(self.new_expr(
            ExprKind::If {
                condition: Box::new(condition),
                then_expr: Box::new(then_expr),
                else_expr: Box::new(else_expr),
            },
            span,
        ))
    }

    pub(super) fn value_block_expression(
        &mut self,
        message: &'static str,
    ) -> Result<Expr, Diagnostic> {
        if !self.at(&TokenKind::LBrace) {
            let error = self.error(message);
            let span = Span {
                start: error.span.start,
                end: error.span.start,
            };
            self.record_missing(error);
            return Ok(self.new_expr(ExprKind::Error, span));
        }
        let block = self.block()?;
        Ok(self.parsed_value_block_expression(block))
    }

    /// Finishes a block parsed in expression position.
    ///
    /// The statement parser deliberately accepts `if` without `else`, so a
    /// leading `if` is initially represented as a statement. Once the entire
    /// block is available, an `if ... else ...` in tail position is
    /// unambiguously value-bearing. Normalize it here so every expression
    /// context (state fields, locals, arguments, retry, and so on) shares the
    /// same block semantics.
    fn normalize_value_block(&mut self, mut block: crate::ast::Block) -> crate::ast::Block {
        if let Some(statement) = block.statements.pop() {
            if let crate::ast::Stmt::If {
                condition,
                then_block,
                else_block: Some(else_block),
                span,
            } = statement
            {
                let then_expr = self.parsed_value_block_expression(then_block);
                let else_expr = self.parsed_value_block_expression(else_block);
                let expression = self.new_expr(
                    ExprKind::If {
                        condition: Box::new(condition),
                        then_expr: Box::new(then_expr),
                        else_expr: Box::new(else_expr),
                    },
                    span,
                );
                block
                    .statements
                    .push(crate::ast::Stmt::Expression(expression));
            } else {
                block.statements.push(statement);
            }
        }
        block
    }

    fn parsed_value_block_expression(&mut self, block: crate::ast::Block) -> Expr {
        let block = self.normalize_value_block(block);
        let span = block.span;
        if block.trailing_semicolon.is_none()
            && let [crate::ast::Stmt::Expression(expression)] = block.statements.as_slice()
        {
            let mut expression = expression.clone();
            expression.span = span;
            expression
        } else {
            self.value_block(block)
        }
    }

    fn value_block(&mut self, block: crate::ast::Block) -> Expr {
        if let Some(semicolon) = block.trailing_semicolon
            && matches!(
                block.statements.last(),
                Some(crate::ast::Stmt::Expression(_))
            )
        {
            self.diagnostics.push(
                Diagnostic::warning(
                    DiagnosticCode::ValueBlockSemicolon,
                    "a trailing semicolon does not discard a value block's final expression",
                    semicolon,
                )
                .with_primary_label("this expression still supplies the block's value")
                .with_note(
                    "value blocks use their final expression even when it has a semicolon; ordinary function bodies still require `return`",
                )
                .with_machine_applicable_fix("remove the trailing semicolon", semicolon, ""),
            );
        }
        let span = block.span;
        self.new_expr(ExprKind::Block(block), span)
    }

    pub(super) fn binary_operator(&self) -> Option<(u8, BinaryOp)> {
        Some(match self.current().kind {
            TokenKind::OrOr => (2, BinaryOp::Or),
            TokenKind::AndAnd => (3, BinaryOp::And),
            TokenKind::EqEq => (4, BinaryOp::Eq),
            TokenKind::BangEq => (4, BinaryOp::Ne),
            TokenKind::Lt => (4, BinaryOp::Lt),
            TokenKind::Le => (4, BinaryOp::Le),
            TokenKind::Gt => (4, BinaryOp::Gt),
            TokenKind::Ge => (4, BinaryOp::Ge),
            TokenKind::Or => (5, BinaryOp::BitOr),
            TokenKind::Caret => (6, BinaryOp::BitXor),
            TokenKind::And => (7, BinaryOp::BitAnd),
            TokenKind::Shl => (8, BinaryOp::Shl),
            TokenKind::Shr => (8, BinaryOp::Shr),
            TokenKind::Plus => (9, BinaryOp::Add),
            TokenKind::Minus => (9, BinaryOp::Sub),
            TokenKind::Star => (10, BinaryOp::Mul),
            TokenKind::Slash => (10, BinaryOp::Div),
            TokenKind::Percent => (10, BinaryOp::Rem),
            _ => return None,
        })
    }
}

fn parse_file_version(value: &str) -> Result<[u16; 4], &'static str> {
    let mut components = value.split('.');
    let mut parsed = [0; 4];
    for component in &mut parsed {
        let Some(text) = components.next() else {
            return Err("file-version literals require exactly four decimal components");
        };
        if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err("file-version components must be decimal integers");
        }
        *component = text
            .parse()
            .map_err(|_| "file-version components must fit in `u16`")?;
    }
    if components.next().is_some() {
        return Err("file-version literals require exactly four decimal components");
    }
    Ok(parsed)
}

fn flatten_postfix_receiver(receiver: Expr, members: &mut Vec<String>) -> Expr {
    let Expr { id, kind, span } = receiver;
    match kind {
        ExprKind::Member { receiver, name, .. } => {
            let receiver = flatten_postfix_receiver(*receiver, members);
            members.push(name);
            receiver
        }
        kind => Expr { id, kind, span },
    }
}
