//! Top-level declarations and the state/settings domain grammars.

//! Declaration grammar.

use super::{
    Action, ActionKind, ConditionalFieldsDecl, Diagnostic, EnumDecl, EnumId, EnumReference,
    EnumVariant, Expr, FunctionDecl, FunctionId, ManagedClassDecl, ManagedClassId,
    ManagedFieldDecl, ManagedFieldId, ManagedImageDecl, ManagedImageId, ManagedItemDecl,
    ManagedMetadataName, ManagedMetadataNames, ManagedNamespaceDecl, ManagedNamespaceId, Parameter,
    Parser, PointerPath, PointerPathBase, SettingChoiceOption, SettingDecl, SettingExternalKey,
    SettingFamilyDecl, SettingFileFilter, SettingKind, SettingTextPart, SettingTextPattern, Span,
    StateDecl, StateField, StateMemoryDecoder, StateProviderAlternativeDecl, StateProviderRef,
    StateProviderSelectorRef, StateSource, StateTransform, StructDecl, StructField, StructId,
    TickRateDecl, TickRateValue, TokenKind, TypeRef,
};
use crate::{
    diagnostic::{DiagnosticFix, FixApplicability, TextEdit},
    migration::{ASL_STRING_N_FIELD_DIAGNOSTIC, legacy_lifecycle_diagnostic},
    parser::parse_integer,
};

impl Parser<'_> {
    pub(super) fn managed_image_decl(&mut self) -> Result<ManagedImageDecl, Diagnostic> {
        let start = self.expect_ident("image")?;
        let name_token = self.current().clone();
        let name = self.expect_string("expected a quoted managed image name after `image`")?;
        let id = ManagedImageId::from_index(self.next_managed_image_id);
        self.next_managed_image_id += 1;
        let opening_span = self.expect(
            TokenKind::LBrace,
            "expected `{` after the managed image name",
        )?;
        let items = self.managed_items("managed image");
        let closing = self
            .eat(&TokenKind::RBrace)
            .unwrap_or_else(|| self.current().span);
        Ok(ManagedImageDecl {
            id,
            keyword_span: start,
            name,
            name_span: name_token.span,
            documentation: None,
            opening_span,
            items,
            span: start.join(closing),
        })
    }

    fn managed_items(&mut self, owner: &'static str) -> Vec<ManagedItemDecl> {
        let mut items = Vec::new();
        let body_depth = self.cursor.brace_depth();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                self.record_missing_closing(match owner {
                    "namespace" => "unterminated managed namespace declaration",
                    _ => "unterminated managed image declaration",
                });
                break;
            }
            let item_start = self.cursor.position();
            let documentation = self.take_source_documentation();
            let parsed = if self.at_ident("namespace") {
                self.managed_namespace_decl().map(|mut namespace| {
                    namespace.documentation = documentation;
                    ManagedItemDecl::Namespace(namespace)
                })
            } else if self.at_ident("class") {
                self.managed_class_decl().map(|mut class| {
                    class.documentation = documentation;
                    ManagedItemDecl::Class(class)
                })
            } else {
                Err(self.error("expected a `class` or `namespace` declaration"))
            };
            if let Some(item) = self.recover_delimited_item(parsed, item_start, body_depth) {
                items.push(item);
            }
        }
        items
    }

    fn managed_namespace_decl(&mut self) -> Result<ManagedNamespaceDecl, Diagnostic> {
        let start = self.expect_ident("namespace")?;
        let (name, name_span) = self.expect_declared_ident("expected a namespace name")?;
        let id = ManagedNamespaceId::from_index(self.next_managed_namespace_id);
        self.next_managed_namespace_id += 1;
        let opening_span =
            self.expect(TokenKind::LBrace, "expected `{` after the namespace name")?;
        let items = self.managed_items("namespace");
        let closing = self
            .eat(&TokenKind::RBrace)
            .unwrap_or_else(|| self.current().span);
        Ok(ManagedNamespaceDecl {
            id,
            keyword_span: start,
            name,
            name_span,
            documentation: None,
            opening_span,
            items,
            span: start.join(closing),
        })
    }

    fn managed_class_decl(&mut self) -> Result<ManagedClassDecl, Diagnostic> {
        let start = self.expect_ident("class")?;
        let (name, name_span) = self.expect_declared_ident("expected a managed class name")?;
        let id = ManagedClassId::from_index(self.next_managed_class_id);
        self.next_managed_class_id += 1;
        let metadata_names = self.managed_metadata_names()?;
        let opening_span = self.expect(
            TokenKind::LBrace,
            "expected `{` after the managed class name",
        )?;
        let mut fields = Vec::new();
        let mut conditional_fields = Vec::new();
        let body_depth = self.cursor.brace_depth();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                self.record_missing_closing("unterminated managed class declaration");
                break;
            }
            let item_start = self.cursor.position();
            let documentation = self.take_source_documentation();
            if self.at_ident("if") {
                let parsed = self.managed_conditional_fields_decl(documentation);
                if let Some(groups) = self.recover_delimited_item(parsed, item_start, body_depth) {
                    conditional_fields.extend(groups);
                }
            } else {
                let parsed = self.managed_field_decl().map(|mut field| {
                    field.documentation = documentation;
                    field
                });
                if let Some(field) = self.recover_delimited_item(parsed, item_start, body_depth) {
                    fields.push(field);
                }
            }
        }
        let closing = self
            .eat(&TokenKind::RBrace)
            .unwrap_or_else(|| self.current().span);
        Ok(ManagedClassDecl {
            id,
            keyword_span: start,
            name,
            name_span,
            documentation: None,
            metadata_names,
            opening_span,
            fields,
            conditional_fields,
            span: start.join(closing),
        })
    }

    fn managed_conditional_fields_decl(
        &mut self,
        mut documentation: Option<String>,
    ) -> Result<Vec<ConditionalFieldsDecl<ManagedFieldDecl>>, Diagnostic> {
        let mut groups = Vec::new();
        let mut else_span = None;
        loop {
            let keyword_span = self.expect_ident("if")?;
            let condition = Some(self.root_expression_before_block());
            groups.push(self.managed_conditional_fields_branch(
                documentation.take(),
                else_span,
                keyword_span,
                condition,
            )?);
            if !self.at_ident("else") {
                break;
            }
            let found_else = self.expect_ident("else")?;
            if self.at_ident("if") {
                else_span = Some(found_else);
                continue;
            }
            groups.push(self.managed_conditional_fields_branch(
                None,
                Some(found_else),
                found_else,
                None,
            )?);
            break;
        }
        Ok(groups)
    }

    fn managed_conditional_fields_branch(
        &mut self,
        documentation: Option<String>,
        else_span: Option<Span>,
        keyword_span: Span,
        condition: Option<Expr>,
    ) -> Result<ConditionalFieldsDecl<ManagedFieldDecl>, Diagnostic> {
        if documentation.is_some() {
            self.diagnostics.push(Diagnostic::new(
                "document the fields inside a conditional managed group",
                keyword_span,
            ));
        }
        let opening_span = self.expect(
            TokenKind::LBrace,
            "expected `{` after the managed-field condition",
        )?;
        let body_depth = self.cursor.brace_depth();
        let mut fields = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                self.record_missing_closing("unterminated conditional managed fields");
                break;
            }
            let item_start = self.cursor.position();
            let field_documentation = self.take_source_documentation();
            let parsed = self.managed_field_decl().map(|mut field| {
                field.documentation = field_documentation;
                field
            });
            if let Some(field) = self.recover_delimited_item(parsed, item_start, body_depth) {
                fields.push(field);
            }
        }
        let closing = self
            .eat(&TokenKind::RBrace)
            .unwrap_or_else(|| self.current().span);
        Ok(ConditionalFieldsDecl {
            else_span,
            keyword_span,
            condition,
            opening_span,
            fields,
            span: else_span.unwrap_or(keyword_span).join(closing),
        })
    }

    fn managed_field_decl(&mut self) -> Result<ManagedFieldDecl, Diagnostic> {
        let static_span = if self.at_ident("static") {
            Some(self.bump().span)
        } else {
            None
        };
        let start = static_span.map_or(self.current().span.start, |span| span.start);
        let (ty, type_span) = self.parse_type("expected a managed field type")?;
        let (name, name_span) = self.expect_declared_ident("expected a managed field name")?;
        let metadata_names = self.managed_metadata_names()?;
        let closing = self.expect(
            TokenKind::Semicolon,
            "expected `;` after the managed field declaration",
        )?;
        let id = ManagedFieldId::from_index(self.next_managed_field_id);
        self.next_managed_field_id += 1;
        Ok(ManagedFieldDecl {
            id,
            is_static: static_span.is_some(),
            static_span,
            ty,
            type_span,
            name,
            name_span,
            documentation: None,
            metadata_names,
            span: Span {
                start,
                end: closing.end,
            },
        })
    }

    fn managed_metadata_names(&mut self) -> Result<ManagedMetadataNames, Diagnostic> {
        if !self.at_ident("from") {
            return Ok(ManagedMetadataNames::default());
        }
        let keyword_span = self.bump().span;
        let mut values = Vec::new();
        let end = if self.eat(&TokenKind::LBracket).is_some() {
            while !self.at(&TokenKind::RBracket) {
                if self.at(&TokenKind::Eof) {
                    return Err(self.error("unterminated metadata-name list"));
                }
                let token = self.current().clone();
                let value = self.expect_string("expected a quoted metadata name")?;
                values.push(ManagedMetadataName {
                    value,
                    span: token.span,
                });
                if self.eat(&TokenKind::Comma).is_none() {
                    break;
                }
            }
            self.expect(TokenKind::RBracket, "expected `]` after metadata names")?
        } else {
            let token = self.current().clone();
            let value = self.expect_string("expected a quoted metadata name after `from`")?;
            values.push(ManagedMetadataName {
                value,
                span: token.span,
            });
            token.span
        };
        if values.is_empty() {
            return Err(Diagnostic::new(
                "a metadata-name list cannot be empty",
                keyword_span.join(end),
            ));
        }
        Ok(ManagedMetadataNames {
            keyword_span: Some(keyword_span),
            values,
            span: Some(keyword_span.join(end)),
        })
    }

    pub(super) fn tick_rate_decl(&mut self) -> Result<TickRateDecl, Diagnostic> {
        let keyword_span = self.expect_ident("tickRate")?;
        self.expect(TokenKind::LBrace, "expected `{` after `tickRate`")?;
        let body_depth = self.cursor.brace_depth();
        let mut attached = None;
        let mut detached = None;

        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                self.record_missing_closing("unterminated tick-rate declaration");
                break;
            }
            let item_start = self.cursor.position();
            let parsed = (|| {
                let (name, name_span) =
                    self.expect_any_ident("expected `attached` or `detached`")?;
                self.expect(TokenKind::Colon, "expected `:` after the tick-rate name")?;
                let rate = self.tick_rate_value(name_span)?;
                let value = TickRateValue {
                    keyword_span: name_span,
                    value: rate.0,
                    span: name_span.join(rate.1),
                };
                let slot = match name.as_str() {
                    "attached" => &mut attached,
                    "detached" => &mut detached,
                    _ => {
                        return Err(Diagnostic::new(
                            "expected `attached` or `detached`",
                            name_span,
                        ));
                    }
                };
                if slot.is_some() {
                    return Err(Diagnostic::new(
                        format!("`{name}` is already declared in this tick-rate policy"),
                        name_span,
                    ));
                }
                *slot = Some(value);
                self.require_comma_between("tick rates");
                Ok(())
            })();
            self.recover_delimited_item(parsed, item_start, body_depth);
        }
        let closing = self
            .eat(&TokenKind::RBrace)
            .unwrap_or_else(|| self.current().span);
        Ok(TickRateDecl {
            keyword_span,
            attached,
            detached,
            span: keyword_span.join(closing),
        })
    }

    fn tick_rate_value(&mut self, name_span: Span) -> Result<(f64, Span), Diagnostic> {
        let minus = self.eat(&TokenKind::Minus);
        let token = self.current().clone();
        let magnitude = match &token.kind {
            TokenKind::Int(text) => {
                let (value, suffix) =
                    parse_integer(text).map_err(|message| Diagnostic::new(message, token.span))?;
                if suffix.is_some() {
                    return Err(Diagnostic::new(
                        "tick rates do not need numeric suffixes",
                        token.span,
                    ));
                }
                value as f64
            }
            TokenKind::Float(text) => text
                .replace('_', "")
                .parse::<f64>()
                .map_err(|_| Diagnostic::new("expected a finite tick rate", token.span))?,
            _ => return Err(Diagnostic::new("expected a numeric tick rate", token.span)),
        };
        let span = minus.map_or(token.span, |minus| minus.join(token.span));
        let value = if minus.is_some() {
            -magnitude
        } else {
            magnitude
        };
        if !value.is_finite() || value <= 0.0 {
            return Err(
                Diagnostic::new("a tick rate must be finite and greater than zero", span)
                    .with_secondary_label(name_span, "this lifecycle rate is invalid"),
            );
        }
        self.bump();
        Ok((value, span))
    }

    pub(super) fn enum_decl(&mut self) -> Result<EnumDecl, Diagnostic> {
        let start = self.expect_ident("enum")?.start;
        let (name, name_span) = self.expect_declared_ident("expected an enum name")?;
        let id = EnumId::from_index(self.next_enum_id);
        self.next_enum_id += 1;
        let (representation, representation_span) = if self.eat(&TokenKind::Colon).is_some() {
            let (representation, span) =
                self.parse_type("expected an integer representation after `:`")?;
            (Some(representation), Some(span))
        } else {
            (None, None)
        };
        self.expect(TokenKind::LBrace, "expected `{` after the enum name")?;
        let body_depth = self.cursor.brace_depth();
        let mut variants = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                self.record_missing_closing("unterminated enum declaration");
                break;
            }
            let item_start = self.cursor.position();
            let documentation = self.take_source_documentation();
            if self.at(&TokenKind::RBrace) {
                self.diagnostics
                    .push(self.error("a documentation comment must precede an enum variant"));
                break;
            }
            let parsed = (|| {
                let (variant_name, variant_span) =
                    self.expect_declared_ident("expected a variant name")?;
                let payload = if self.eat(&TokenKind::LParen).is_some() {
                    let (ty, _) = self.parse_type("expected a payload type")?;
                    self.expect(TokenKind::RParen, "expected `)` after the payload type")?;
                    Some(ty)
                } else {
                    None
                };
                let discriminant = if self.eat(&TokenKind::Assign).is_some() {
                    let minus = self.eat(&TokenKind::Minus);
                    let token = self.current().clone();
                    let TokenKind::Int(text) = token.kind else {
                        return Err(Diagnostic::new(
                            "expected an integer enum discriminant after `=`",
                            token.span,
                        ));
                    };
                    let (magnitude, suffix) = parse_integer(&text)
                        .map_err(|message| Diagnostic::new(message, token.span))?;
                    if suffix.is_some() {
                        return Err(Diagnostic::new(
                            "enum discriminants use the enum's declared integer representation and must not have a type suffix",
                            token.span,
                        ));
                    }
                    self.bump();
                    Some(crate::ast::EnumDiscriminant {
                        magnitude,
                        negative: minus.is_some(),
                        span: minus.map_or(token.span, |minus| minus.join(token.span)),
                    })
                } else {
                    None
                };
                let variant = EnumVariant {
                    id: self.new_enum_variant_id(),
                    name: variant_name,
                    name_span: variant_span,
                    documentation,
                    payload,
                    discriminant,
                    span: variant_span.join(self.previous().span),
                };
                self.require_comma_between("enum variants");
                Ok(variant)
            })();
            if let Some(variant) = self.recover_delimited_item(parsed, item_start, body_depth) {
                variants.push(variant);
            }
        }
        let end = self
            .eat(&TokenKind::RBrace)
            .map_or(self.current().span.end, |span| span.end);
        Ok(EnumDecl {
            id,
            name,
            documentation: None,
            name_span,
            representation,
            representation_span,
            variants,
            span: Span { start, end },
        })
    }

    pub(super) fn struct_decl(&mut self) -> Result<StructDecl, Diagnostic> {
        let start = if self.at_ident("record") {
            self.expect_ident("record")?.start
        } else {
            self.expect_ident("struct")?.start
        };
        let (name, name_span) = self.expect_declared_ident("expected a struct name")?;
        let id = StructId::from_index(self.next_struct_id);
        self.next_struct_id += 1;
        self.expect(TokenKind::LBrace, "expected `{` after the struct name")?;
        let body_depth = self.cursor.brace_depth();
        let mut fields = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                self.record_missing_closing("unterminated struct declaration");
                break;
            }
            let item_start = self.cursor.position();
            let documentation = self.take_source_documentation();
            if self.at(&TokenKind::RBrace) {
                self.diagnostics
                    .push(self.error("a documentation comment must precede a struct field"));
                break;
            }
            let parsed = (|| {
                let (field_name, field_start) =
                    self.expect_declared_ident("expected a field name")?;
                self.expect(TokenKind::Colon, "expected `:` after the field name")?;
                let (ty, type_span) = self.parse_type("expected a field type")?;
                let field = StructField {
                    id: self.new_struct_field_id(),
                    name: field_name,
                    name_span: field_start,
                    documentation,
                    ty,
                    span: field_start.join(type_span),
                };
                self.require_comma_between("struct fields");
                Ok(field)
            })();
            if let Some(field) = self.recover_delimited_item(parsed, item_start, body_depth) {
                fields.push(field);
            }
        }
        let end = self
            .eat(&TokenKind::RBrace)
            .map_or(self.current().span.end, |span| span.end);
        Ok(StructDecl {
            id,
            name,
            documentation: None,
            name_span,
            fields,
            span: Span { start, end },
        })
    }

    pub(super) fn function_decl(&mut self) -> Result<FunctionDecl, Diagnostic> {
        let id = FunctionId::from_index(self.next_function_id);
        self.next_function_id += 1;
        let start = self.bump().span.start;
        let (first_name, first_span) = self.expect_declared_ident("expected a function name")?;
        let (method_of, name, name_span) = if self.eat(&TokenKind::Dot).is_some() {
            let receiver_name = self
                .record_foreign_spelling_diagnostic(
                    first_span,
                    &first_name,
                    crate::migration::ForeignSpellingContext::Type,
                )
                .unwrap_or(&first_name);
            let receiver = self.resolve_type(receiver_name, first_span)?;
            let (method, method_span) =
                self.expect_declared_ident("expected a method name after `.`")?;
            (Some(receiver), method, method_span)
        } else {
            (None, first_name, first_span)
        };
        self.expect(TokenKind::LParen, "expected `(` after the function name")?;
        let mut params = method_of.map_or_else(Vec::new, |ty| {
            vec![Parameter {
                binding: {
                    let pattern = crate::ast::PatternNode {
                        id: self.new_pattern_id(),
                        kind: crate::ast::MatchPattern::Binding(crate::ast::PatternBinding {
                            id: self.new_value_id(),
                            name: "self".to_owned(),
                            name_span: first_span,
                        }),
                        span: first_span,
                    };
                    self.binding_pattern(pattern)
                },
                annotation: Some(ty),
                span: first_span,
            }]
        });
        let mut missing_closing_parenthesis = false;
        while !self.at(&TokenKind::RParen) {
            if self.at(&TokenKind::Eof) {
                self.record_missing_closing("unterminated parameter list");
                missing_closing_parenthesis = true;
                break;
            }
            // A leading `{` is an anonymous struct pattern. A `{` after a
            // complete parameter, however, is still the function body and
            // therefore recovers a missing closing `)` as before.
            if (self.at(&TokenKind::LBrace)
                && !matches!(self.previous().kind, TokenKind::LParen | TokenKind::Comma))
                || self.at(&TokenKind::Minus)
            {
                self.record_missing(Diagnostic::new(
                    "expected `)` after the parameters",
                    self.current().span,
                ));
                missing_closing_parenthesis = true;
                break;
            }
            let item_start = self.cursor.position();
            let parsed = (|| {
                let pattern = self.match_pattern(true)?;
                let (annotation, type_span) = if self.eat(&TokenKind::Colon).is_some() {
                    let (ty, span) = self.parse_type("expected a parameter type")?;
                    (Some(ty), span)
                } else {
                    (None, pattern.span)
                };
                let parameter = Parameter {
                    span: pattern.span.join(type_span),
                    binding: self.binding_pattern(pattern),
                    annotation,
                };
                if self.eat(&TokenKind::Comma).is_none()
                    && !self.at(&TokenKind::RParen)
                    && !self.at(&TokenKind::LBrace)
                    && !self.at(&TokenKind::Minus)
                {
                    return Err(self.error("expected `,` between parameters"));
                }
                Ok(parameter)
            })();
            if let Some(parameter) = self.recover_parameter(parsed, item_start) {
                params.push(parameter);
            }
        }
        if !missing_closing_parenthesis {
            self.expect(TokenKind::RParen, "expected `)` after the parameters")?;
        }
        let (
            return_annotation,
            return_is_async,
            return_async_span,
            return_is_iterator,
            return_iterator_span,
            return_annotation_span,
        ) = if self.eat(&TokenKind::Minus).is_some() {
            self.expect(TokenKind::Gt, "expected `>` in the return arrow `->`")?;
            let async_span = self.at_ident("async").then_some(self.current().span);
            let iterator_span = self.at_ident("iterator").then_some(self.current().span);
            let (ty, span) = self.parse_type("expected a return type")?;
            (
                Some(ty),
                matches!(ty, TypeRef::Async(_)),
                async_span,
                matches!(ty, TypeRef::Iterator(_)),
                iterator_span,
                Some(span),
            )
        } else {
            (None, false, None, false, None, None)
        };
        let body = self.block()?;
        let span = Span {
            start,
            end: body.span.end,
        };
        Ok(FunctionDecl {
            id,
            name,
            name_span,
            documentation: None,
            debug_only: false,
            method_of,
            params,
            return_annotation,
            return_is_async,
            return_async_span,
            return_is_iterator,
            return_iterator_span,
            return_annotation_span,
            body,
            span,
        })
    }

    pub(super) fn state_block_decl(&mut self) -> Result<StateDecl, Diagnostic> {
        let start = self.expect_ident("state")?.start;
        if self.at(&TokenKind::LBrace) {
            return self.multi_provider_state_decl(start);
        }
        let (provider, processes) = if matches!(self.current().kind, TokenKind::Ident(_)) {
            let (provider, processes) = self.state_provider_reference()?;
            (Some(provider), processes)
        } else {
            (None, self.process_names()?)
        };
        self.expect(
            TokenKind::LBrace,
            "expected `{` after the process name list",
        )?;
        let body_depth = self.cursor.brace_depth();
        let mut fields = Vec::new();
        let mut conditional_fields = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                self.record_missing_closing("unterminated state declaration");
                break;
            }
            let item_start = self.cursor.position();
            let documentation = self.take_source_documentation();
            if self.at(&TokenKind::RBrace) {
                self.diagnostics.push(self.error(
                    "a documentation comment must precede a state field or conditional group",
                ));
                break;
            }
            if self.at_ident("if") {
                let parsed = self.state_conditional_fields_decl(documentation);
                if let Some(groups) = self.recover_delimited_item(parsed, item_start, body_depth) {
                    conditional_fields.extend(groups);
                }
            } else {
                let parsed = self.state_field(documentation);
                if let Some(field) = self.recover_delimited_item(parsed, item_start, body_depth) {
                    fields.push(field);
                    self.require_semicolon_between("state fields");
                }
            }
        }
        let end = self
            .eat(&TokenKind::RBrace)
            .map_or(self.current().span.end, |span| span.end);
        Ok(StateDecl {
            provider,
            processes,
            provider_alternatives: Vec::new(),
            fields,
            conditional_fields,
            provider_enum: None,
            provider_value: None,
            span: Span { start, end },
        })
    }

    fn state_provider_reference(&mut self) -> Result<(StateProviderRef, Vec<String>), Diagnostic> {
        let (name, span) = self.expect_any_ident("expected a state provider name")?;
        let selector = if self.eat(&TokenKind::Dot).is_some() {
            let selector_start = self.previous().span.start;
            let (name, name_span) =
                self.expect_any_ident("expected a state-provider selector after `.`")?;
            self.expect(
                TokenKind::LParen,
                "expected `(` after the state-provider selector",
            )?;
            let mut arguments = Vec::new();
            while !self.at(&TokenKind::RParen) {
                arguments.push(self.expression(0)?);
                if self.eat(&TokenKind::Comma).is_none() {
                    break;
                }
            }
            let end = self
                .expect(
                    TokenKind::RParen,
                    "expected `)` after the state-provider selector arguments",
                )?
                .end;
            Some(StateProviderSelectorRef {
                name,
                name_span,
                arguments,
                span: Span {
                    start: selector_start,
                    end,
                },
            })
        } else {
            None
        };
        let processes = if self.at(&TokenKind::LBrace) {
            Vec::new()
        } else {
            self.process_names()?
        };
        Ok((
            StateProviderRef {
                name,
                span,
                selector,
            },
            processes,
        ))
    }

    fn multi_provider_state_decl(&mut self, start: usize) -> Result<StateDecl, Diagnostic> {
        self.expect(TokenKind::LBrace, "expected `{` after `state`")?;
        let body_depth = self.cursor.brace_depth();
        let mut alternatives = Vec::new();
        let mut variants = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                self.record_missing_closing("unterminated multi-provider state declaration");
                break;
            }
            let item_start = self.cursor.position();
            let documentation = self.take_source_documentation();
            let parsed = self.state_provider_alternative(documentation);
            if let Some((alternative, variant)) =
                self.recover_delimited_item(parsed, item_start, body_depth)
            {
                alternatives.push(alternative);
                variants.push(variant);
                self.require_comma_between("state providers");
            }
        }
        let end = self
            .eat(&TokenKind::RBrace)
            .map_or(self.current().span.end, |span| span.end);
        if alternatives.len() < 2 {
            self.diagnostics.push(
                Diagnostic::new(
                    "a multi-provider state declaration needs at least two provider alternatives",
                    Span { start, end },
                )
                .with_primary_label(
                    "use the ordinary `state Provider { ... }` form for one provider",
                ),
            );
        }
        let enum_id = EnumId::from_index(self.next_enum_id);
        self.next_enum_id += 1;
        let name_span = Span {
            start,
            end: start + "state".len(),
        };
        Ok(StateDecl {
            provider: None,
            processes: Vec::new(),
            provider_alternatives: alternatives,
            fields: Vec::new(),
            conditional_fields: Vec::new(),
            provider_enum: Some(EnumDecl {
                id: enum_id,
                name: "StateProvider".to_owned(),
                documentation: Some(
                    "The state provider selected for the attached game process.".to_owned(),
                ),
                name_span,
                representation: None,
                representation_span: None,
                variants,
                span: Span { start, end },
            }),
            provider_value: Some(self.new_value_id()),
            span: Span { start, end },
        })
    }

    fn state_provider_alternative(
        &mut self,
        documentation: Option<String>,
    ) -> Result<(StateProviderAlternativeDecl, EnumVariant), Diagnostic> {
        let keyword_span = self.expect_ident("provider")?;
        let start = keyword_span.start;
        let (name, name_span) =
            self.expect_declared_ident("expected a state-provider alternative name")?;
        self.expect(
            TokenKind::Colon,
            "expected `:` after the state-provider alternative name",
        )?;
        let (provider, processes) = self.state_provider_reference()?;
        let variant = EnumVariant {
            id: self.new_enum_variant_id(),
            name,
            name_span,
            documentation,
            payload: None,
            discriminant: None,
            span: name_span,
        };
        let opening_span = self.expect(
            TokenKind::LBrace,
            "expected `{` after the state-provider configuration",
        )?;
        let body_depth = self.cursor.brace_depth();
        let mut fields = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                self.record_missing_closing("unterminated state-provider alternative");
                break;
            }
            let item_start = self.cursor.position();
            let field_documentation = self.take_source_documentation();
            let parsed = self.state_field(field_documentation);
            if let Some(field) = self.recover_delimited_item(parsed, item_start, body_depth) {
                fields.push(field);
                self.require_semicolon_between("state fields");
            }
        }
        let end = self
            .eat(&TokenKind::RBrace)
            .map_or(self.current().span.end, |span| span.end);
        Ok((
            StateProviderAlternativeDecl {
                keyword_span,
                variant: variant.id,
                provider,
                processes,
                opening_span,
                fields,
                span: Span { start, end },
            },
            variant,
        ))
    }

    fn state_conditional_fields_decl(
        &mut self,
        mut documentation: Option<String>,
    ) -> Result<Vec<ConditionalFieldsDecl<StateField>>, Diagnostic> {
        let mut groups = Vec::new();
        let mut else_span = None;
        loop {
            let keyword_span = self.expect_ident("if")?;
            let condition = Some(self.root_expression_before_block());
            groups.push(self.state_conditional_fields_branch(
                documentation.take(),
                else_span,
                keyword_span,
                condition,
            )?);
            if !self.at_ident("else") {
                break;
            }
            let found_else = self.expect_ident("else")?;
            if self.at_ident("if") {
                else_span = Some(found_else);
                continue;
            }
            groups.push(self.state_conditional_fields_branch(
                None,
                Some(found_else),
                found_else,
                None,
            )?);
            break;
        }
        Ok(groups)
    }

    fn state_conditional_fields_branch(
        &mut self,
        documentation: Option<String>,
        else_span: Option<Span>,
        keyword_span: Span,
        condition: Option<Expr>,
    ) -> Result<ConditionalFieldsDecl<StateField>, Diagnostic> {
        if documentation.is_some() {
            self.diagnostics.push(Diagnostic::new(
                "document the fields inside a conditional state group",
                keyword_span,
            ));
        }
        let opening_span = self.expect(
            TokenKind::LBrace,
            "expected `{` after the state-field condition",
        )?;
        let body_depth = self.cursor.brace_depth();
        let mut fields = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                self.record_missing_closing("unterminated conditional state fields");
                break;
            }
            let item_start = self.cursor.position();
            let field_documentation = self.take_source_documentation();
            let parsed = self.state_field(field_documentation);
            if let Some(field) = self.recover_delimited_item(parsed, item_start, body_depth) {
                fields.push(field);
                self.require_semicolon_between("state fields");
            }
        }
        let closing = self
            .eat(&TokenKind::RBrace)
            .unwrap_or_else(|| self.current().span);
        Ok(ConditionalFieldsDecl {
            else_span,
            keyword_span,
            condition,
            opening_span,
            fields,
            span: else_span.unwrap_or(keyword_span).join(closing),
        })
    }

    fn state_field(&mut self, documentation: Option<String>) -> Result<StateField, Diagnostic> {
        let legacy_string = self.legacy_string_field_prefix();
        let (name, field_start) = if legacy_string.is_some() {
            self.bump();
            self.expect_declared_ident("expected a field name after the ASL `stringN` type")?
        } else {
            self.expect_declared_ident("expected a state field name")?
        };
        let annotation = if legacy_string.is_none() && self.eat(&TokenKind::Colon).is_some() {
            let (ty, _) = self.parse_type("expected a state field type")?;
            Some(ty)
        } else {
            None
        };
        let source = if self.eat(&TokenKind::Assign).is_some() {
            StateSource::Expression(self.root_expression())
        } else {
            let legacy_colon = if legacy_string.is_some() {
                self.eat(&TokenKind::Colon)
            } else {
                None
            };
            let at_span = if legacy_colon.is_none() {
                Some(self.expect_ident("at")?)
            } else {
                None
            };
            let module = if matches!(self.current().kind, TokenKind::String(_)) {
                let module = self.expect_string("expected a module name")?;
                self.expect(TokenKind::Comma, "expected an offset after the module")?;
                Some(module)
            } else {
                None
            };
            let base_start = self.current().span.start;
            let base = if let Some(name) = module {
                PointerPathBase::Module {
                    name,
                    offset: self.expect_i64("expected a signed module offset")?,
                }
            } else if matches!(self.current().kind, TokenKind::Int(_)) {
                PointerPathBase::Absolute(self.expect_u64("expected an unsigned absolute address")?)
            } else {
                PointerPathBase::Expression(self.required_expression(12)?)
            };
            let base_span = Span {
                start: base_start,
                end: self.previous().span.end,
            };
            let mut offsets = Vec::new();
            while self.at(&TokenKind::Comma)
                && (matches!(self.peek(1).kind, TokenKind::Int(_))
                    || (matches!(self.peek(1).kind, TokenKind::Minus)
                        && matches!(self.peek(2).kind, TokenKind::Int(_))))
            {
                self.bump();
                offsets.push(self.expect_i64("expected a signed pointer offset")?);
            }
            let pointer_end = self.previous().span.end;
            let mut decoder = if let Some(start) = self.eat_ident("as") {
                let (name, name_span) =
                    self.expect_any_ident("expected a state memory decoder after `as`")?;
                if name != "utf8" && name != "utf16le" {
                    return Err(Diagnostic::new(
                        format!("unknown state memory decoder `{name}`"),
                        name_span,
                    ));
                }
                self.expect(
                    TokenKind::LParen,
                    if name == "utf8" {
                        "expected `(` after `utf8`"
                    } else {
                        "expected `(` after `utf16le`"
                    },
                )?;
                let maximum = self.expect_u64(if name == "utf8" {
                    "expected a maximum UTF-8 byte count"
                } else {
                    "expected a maximum UTF-16 code-unit count"
                })?;
                let end = self
                    .expect(
                        TokenKind::RParen,
                        if name == "utf8" {
                            "expected `)` after the maximum UTF-8 byte count"
                        } else {
                            "expected `)` after the maximum UTF-16 code-unit count"
                        },
                    )?
                    .end;
                let Ok(maximum) = u32::try_from(maximum) else {
                    return Err(Diagnostic::new(
                        format!("the maximum `{name}` count must fit in `u32`"),
                        start.join(Span {
                            start: name_span.start,
                            end,
                        }),
                    ));
                };
                let span = Span {
                    start: start.start,
                    end,
                };
                Some(if name == "utf8" {
                    StateMemoryDecoder::Utf8 {
                        max_bytes: maximum,
                        span,
                    }
                } else {
                    StateMemoryDecoder::Utf16Le {
                        max_units: maximum,
                        span,
                    }
                })
            } else {
                None
            };
            if let Some((type_name, type_span, max_bytes)) = legacy_string {
                let mut base_edits = vec![TextEdit {
                    span: Span {
                        start: type_span.start,
                        end: field_start.start,
                    },
                    replacement: String::new(),
                }];
                if let Some(colon) = legacy_colon {
                    base_edits.push(TextEdit {
                        span: colon,
                        replacement: "at".to_owned(),
                    });
                }
                let had_decoder = decoder.is_some();
                let mut utf8_edits = base_edits.clone();
                if !had_decoder {
                    utf8_edits.push(TextEdit {
                        span: Span {
                            start: pointer_end,
                            end: pointer_end,
                        },
                        replacement: format!(" as utf8({max_bytes})"),
                    });
                    decoder = Some(StateMemoryDecoder::Utf8 {
                        max_bytes,
                        span: type_span,
                    });
                }
                let mut diagnostic = self
                    .migration_diagnostic(ASL_STRING_N_FIELD_DIAGNOSTIC, type_span)
                    .with_fix(DiagnosticFix {
                        title: format!("rewrite `{type_name}` assuming the memory contains UTF-8"),
                        applicability: FixApplicability::MaybeIncorrect,
                        edits: utf8_edits,
                    });
                if !had_decoder && max_bytes % 2 == 0 {
                    let mut utf16_edits = base_edits;
                    utf16_edits.push(TextEdit {
                        span: Span {
                            start: pointer_end,
                            end: pointer_end,
                        },
                        replacement: format!(" as utf16le({})", max_bytes / 2),
                    });
                    diagnostic = diagnostic.with_fix(DiagnosticFix {
                        title: format!(
                            "rewrite `{type_name}` assuming the memory contains UTF-16LE"
                        ),
                        applicability: FixApplicability::MaybeIncorrect,
                        edits: utf16_edits,
                    });
                }
                self.diagnostics.push(diagnostic);
            }
            StateSource::Pointer(PointerPath {
                at_span,
                base_span,
                base,
                offsets,
                decoder,
            })
        };
        let transform =
            (matches!(&source, StateSource::Pointer(_)) && self.at_ident("if")).then(|| {
                let start = self.current().span;
                let value = self.new_value_id();
                let expression = self.root_expression();
                StateTransform {
                    value,
                    span: Span {
                        start: start.start,
                        end: expression.span.end,
                    },
                    expression,
                }
            });
        let end = self.previous().span.end;
        Ok(StateField {
            id: self.new_value_id(),
            name,
            documentation,
            annotation,
            source,
            transform,
            span: Span {
                start: field_start.start,
                end,
            },
        })
    }

    fn legacy_string_field_prefix(&self) -> Option<(String, Span, u32)> {
        let TokenKind::Ident(type_name) = &self.current().kind else {
            return None;
        };
        let digits = type_name.strip_prefix("string")?;
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let separator_is_path = matches!(self.peek(2).kind, TokenKind::Colon)
            || matches!(&self.peek(2).kind, TokenKind::Ident(name) if name == "at");
        if !matches!(self.peek(1).kind, TokenKind::Ident(_)) || !separator_is_path {
            return None;
        }
        let max_bytes = digits.parse().ok()?;
        Some((type_name.clone(), self.current().span, max_bytes))
    }

    pub(super) fn process_names(&mut self) -> Result<Vec<String>, Diagnostic> {
        if matches!(self.current().kind, TokenKind::String(_)) {
            return Ok(vec![self.expect_string("expected a process name")?]);
        }
        self.expect(TokenKind::LBracket, "expected a process name or `[` list")?;
        let mut names = Vec::new();
        while !self.at(&TokenKind::RBracket) {
            names.push(self.expect_string("expected a process name string")?);
            if self.eat(&TokenKind::Comma).is_none() {
                break;
            }
        }
        self.expect(TokenKind::RBracket, "expected `]` after process names")?;
        if names.is_empty() {
            return Err(self.error("a process name list cannot be empty"));
        }
        Ok(names)
    }

    pub(super) fn settings_block_decl(
        &mut self,
    ) -> Result<(Vec<SettingDecl>, Vec<SettingFamilyDecl>), Diagnostic> {
        self.expect_ident("settings")?;
        self.expect(TokenKind::LBrace, "expected `{` after `settings`")?;
        let mut settings = Vec::new();
        let mut families = Vec::new();
        let mut heading_count = 0;
        self.settings_dsl_entries(&mut settings, &mut families, 0, &mut heading_count)?;
        if let Err(error) = self.expect(TokenKind::RBrace, "expected `}` after settings") {
            self.record_missing(error);
        }
        Ok((settings, families))
    }

    pub(super) fn settings_dsl_entries(
        &mut self,
        settings: &mut Vec<SettingDecl>,
        families: &mut Vec<SettingFamilyDecl>,
        heading_level: u32,
        heading_count: &mut u32,
    ) -> Result<(), Diagnostic> {
        let body_depth = self.cursor.brace_depth();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                self.record_missing_closing("unterminated settings group");
                return Ok(());
            }
            let item_start = self.cursor.position();
            let parsed = self.settings_dsl_entry(settings, families, heading_level, heading_count);
            self.recover_delimited_item(parsed, item_start, body_depth);
        }
        Ok(())
    }

    pub(super) fn settings_dsl_entry(
        &mut self,
        settings: &mut Vec<SettingDecl>,
        families: &mut Vec<SettingFamilyDecl>,
        heading_level: u32,
        heading_count: &mut u32,
    ) -> Result<(), Diagnostic> {
        let tooltip = self.take_doc_tooltip();
        if self.at(&TokenKind::RBrace) {
            return Err(self.error("a documentation comment must precede a setting or title"));
        }
        if self.at_ident("for") {
            return self.setting_family(settings, families, tooltip);
        }
        if matches!(self.current().kind, TokenKind::Ident(_))
            && matches!(self.peek(1).kind, TokenKind::Colon)
            && matches!(&self.peek(2).kind, TokenKind::Ident(name) if name == "bool")
            && matches!(self.peek(3).kind, TokenKind::Assign)
            && matches!(&self.peek(4).kind, TokenKind::Ident(value) if value == "true" || value == "false")
        {
            let (name, start) = self.expect_declared_ident("expected a setting name")?;
            self.bump();
            self.bump();
            self.bump();
            let default = self.expect_bool("expected a boolean default value")?;
            let description = if self.eat(&TokenKind::Comma).is_some() {
                self.expect_string("expected a quoted setting label after `,`")?
            } else {
                name.clone()
            };
            let span = Span {
                start: start.start,
                end: self.previous().span.end,
            };
            return Err(Diagnostic::new(
                "legacy `name: bool = default, \"label\"` settings syntax is not supported",
                span,
            )
            .with_machine_applicable_fix(
                "rewrite this setting using the settings DSL",
                span,
                format!("{description:?} => {name}: {default}"),
            ));
        }
        let label_token = self.current().clone();
        let TokenKind::String(description) = label_token.kind else {
            return Err(Diagnostic::new(
                "expected a quoted setting label",
                label_token.span,
            ));
        };
        self.bump();
        if self.at(&TokenKind::Comma) {
            return Err(
                self.error("setting tooltips use `///` documentation comments before the setting")
            );
        }

        if self.eat(&TokenKind::LBrace).is_some() {
            let name = format!("_heading{}", *heading_count);
            *heading_count += 1;
            let title_index = settings.len();
            settings.push(SettingDecl {
                id: self.new_value_id(),
                name,
                name_span: label_token.span,
                description,
                tooltip,
                external_key: None,
                kind: SettingKind::Title { heading_level },
                source_visible: true,
                span: label_token.span,
            });
            self.settings_dsl_entries(settings, families, heading_level + 1, heading_count)?;
            let end = self.expect(TokenKind::RBrace, "expected `}` after setting group")?;
            settings[title_index].span = label_token.span.join(end);
            self.require_comma_between("settings");
            return Ok(());
        }

        self.expect(TokenKind::FatArrow, "expected `=>` after the setting label")?;
        let (name, name_span) = self.expect_declared_ident("expected a setting name")?;
        let external_key = if self.at_ident("key") {
            let keyword_span = self.bump().span;
            let token = self.current().clone();
            let key = match token.kind {
                TokenKind::String(value) => {
                    self.bump();
                    SettingExternalKey {
                        value,
                        keyword_span,
                        span: token.span,
                    }
                }
                _ => {
                    return Err(Diagnostic::new("expected a string setting key", token.span));
                }
            };
            Some(key)
        } else {
            None
        };
        self.expect(TokenKind::Colon, "expected `:` after the setting name")?;
        let kind_token = self.current().clone();
        let kind = match kind_token.kind {
            TokenKind::String(default) => {
                self.bump();
                SettingKind::Text {
                    default,
                    default_span: kind_token.span,
                }
            }
            TokenKind::Ident(kind_name) => {
                self.bump();
                match kind_name.as_str() {
                    "true" => SettingKind::Bool { default: true },
                    "false" => SettingKind::Bool { default: false },
                    "choice" => self.choice_setting(kind_token.span)?,
                    "file" => self.file_setting(kind_token.span)?,
                    _ => {
                        return Err(Diagnostic::new(
                            "expected `true`, `false`, a string default, `choice`, or `file`",
                            kind_token.span,
                        ));
                    }
                }
            }
            _ => {
                return Err(Diagnostic::new(
                    "expected `true`, `false`, a string default, `choice`, or `file`",
                    kind_token.span,
                ));
            }
        };
        let end = self.previous().span;
        settings.push(SettingDecl {
            id: self.new_value_id(),
            name,
            name_span,
            description,
            tooltip,
            external_key,
            kind,
            source_visible: true,
            span: label_token.span.join(end).join(name_span),
        });
        self.require_comma_between("settings");
        Ok(())
    }

    fn setting_family(
        &mut self,
        settings: &mut Vec<SettingDecl>,
        families: &mut Vec<SettingFamilyDecl>,
        tooltip: Option<String>,
    ) -> Result<(), Diagnostic> {
        let start_span = self.expect_ident("for")?;
        let (binding, binding_span) =
            self.expect_declared_ident("expected a binding after `for`")?;
        let in_span = self.expect_ident("in")?;
        let (range_start, range_start_span) = self.setting_family_bound()?;
        self.expect(
            TokenKind::DotDotEq,
            "expected an inclusive `..=` range in a settings family",
        )?;
        let (range_end, range_end_span) = self.setting_family_bound()?;
        if range_start > range_end {
            return Err(Diagnostic::new(
                "a settings-family range cannot run backwards",
                range_start_span.join(range_end_span),
            ));
        }
        if range_end - range_start > 4095 {
            return Err(Diagnostic::new(
                "a settings family may declare at most 4096 settings",
                range_start_span.join(range_end_span),
            ));
        }
        self.expect(
            TokenKind::LBrace,
            "expected `{` after the settings-family range",
        )?;
        let label = self.setting_text_pattern(&binding, "expected a quoted or template label")?;
        let (key_keyword_span, key) = if self.at_ident("key") {
            let keyword_span = self.bump().span;
            (
                Some(keyword_span),
                Some(self.setting_text_pattern(
                    &binding,
                    "expected a quoted or template key after `key`",
                )?),
            )
        } else {
            (None, None)
        };
        self.expect(TokenKind::Colon, "expected `:` before the family default")?;
        let default = self.expect_bool("expected a boolean family default")?;
        self.eat(&TokenKind::Comma);
        let closing = self.expect(TokenKind::RBrace, "expected `}` after the settings family")?;
        let span = start_span.join(closing);
        let family_index = families.len();
        let family = SettingFamilyDecl {
            keyword_span: start_span,
            binding_id: self.new_value_id(),
            binding,
            binding_span,
            in_span,
            start: range_start,
            end_inclusive: range_end,
            range_span: range_start_span.join(range_end_span),
            label,
            key_keyword_span,
            key,
            default,
            tooltip,
            span,
        };
        for value in range_start..=range_end {
            let description = family.label.render(value);
            let key_pattern = family.key.as_ref().unwrap_or(&family.label);
            settings.push(SettingDecl {
                id: self.new_value_id(),
                name: format!("_setting_family_{family_index}_{value}"),
                name_span: binding_span,
                description,
                tooltip: family.tooltip.clone(),
                external_key: Some(SettingExternalKey {
                    value: key_pattern.render(value),
                    keyword_span: key_pattern.span,
                    span: key_pattern.span,
                }),
                kind: SettingKind::Bool { default },
                source_visible: false,
                span,
            });
        }
        families.push(family);
        self.require_comma_between("settings");
        Ok(())
    }

    fn setting_family_bound(&mut self) -> Result<(u32, Span), Diagnostic> {
        let token = self.current().clone();
        let TokenKind::Int(text) = token.kind else {
            return Err(Diagnostic::new(
                "expected a non-negative integer settings-family bound",
                token.span,
            ));
        };
        self.bump();
        let (value, suffix) =
            parse_integer(&text).map_err(|message| Diagnostic::new(message, token.span))?;
        if suffix.is_some() {
            return Err(Diagnostic::new(
                "settings-family bounds do not need integer suffixes",
                token.span,
            ));
        }
        let value = u32::try_from(value)
            .map_err(|_| Diagnostic::new("a settings-family bound must fit in u32", token.span))?;
        Ok((value, token.span))
    }

    fn setting_text_pattern(
        &mut self,
        binding: &str,
        message: &'static str,
    ) -> Result<SettingTextPattern, Diagnostic> {
        let token = self.current().clone();
        if let TokenKind::String(value) = token.kind {
            self.bump();
            return Ok(SettingTextPattern {
                parts: vec![SettingTextPart::Text(value)],
                span: token.span,
            });
        }
        if self.eat(&TokenKind::TemplateStart).is_none() {
            return Err(Diagnostic::new(message, token.span));
        }
        let start = token.span;
        let mut parts = Vec::new();
        loop {
            match self.current().kind.clone() {
                TokenKind::TemplateChunk(value) => {
                    self.bump();
                    parts.push(SettingTextPart::Text(value));
                }
                TokenKind::TemplateExprStart => {
                    self.record_javascript_style_interpolation();
                    self.bump();
                    let (name, span) =
                        self.expect_any_ident("expected the family binding in this template")?;
                    if name != binding {
                        return Err(Diagnostic::new(
                            format!("settings-family templates may only interpolate `{binding}`"),
                            span,
                        ));
                    }
                    self.expect(
                        TokenKind::TemplateExprEnd,
                        "expected `}` after the family binding",
                    )?;
                    parts.push(SettingTextPart::Binding { span });
                }
                TokenKind::TemplateEnd => {
                    let end = self.bump().span;
                    return Ok(SettingTextPattern {
                        parts,
                        span: start.join(end),
                    });
                }
                _ => {
                    return Err(Diagnostic::new(
                        "settings-family templates only support their integer binding",
                        self.current().span,
                    ));
                }
            }
        }
    }

    pub(super) fn take_doc_tooltip(&mut self) -> Option<String> {
        self.take_doc_comments("\n")
    }

    pub(super) fn take_source_documentation(&mut self) -> Option<String> {
        self.take_doc_comments("\n\n")
    }

    fn take_doc_comments(&mut self, paragraph_break: &str) -> Option<String> {
        let mut tooltip = String::new();
        let mut blank_lines = 0usize;
        for line in self.cursor.take_doc_comments() {
            let line = line.trim();
            if line.is_empty() {
                if !tooltip.is_empty() {
                    blank_lines += 1;
                }
                continue;
            }
            if !tooltip.is_empty() {
                if blank_lines == 0 {
                    tooltip.push(' ');
                } else {
                    for _ in 0..blank_lines {
                        tooltip.push_str(paragraph_break);
                    }
                }
            }
            blank_lines = 0;
            tooltip.push_str(line);
        }
        (!tooltip.is_empty()).then_some(tooltip)
    }

    pub(super) fn choice_setting(&mut self, keyword_span: Span) -> Result<SettingKind, Diagnostic> {
        self.expect(TokenKind::LBrace, "expected `{` after `choice`")?;
        let body_depth = self.cursor.brace_depth();
        let mut enumeration: Option<(String, Span)> = None;
        let mut options = Vec::new();
        let mut default_variant = None;
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                self.record_missing_closing("unterminated choice setting");
                break;
            }
            let item_start = self.cursor.position();
            let parsed = (|| {
                let option_start = self.current().span;
                let description = self.expect_string("expected a choice option description")?;
                self.expect(
                    TokenKind::FatArrow,
                    "expected `=>` after the option description",
                )?;
                let (enum_name, enum_span) = self.expect_any_ident("expected an enum name")?;
                if enumeration
                    .as_ref()
                    .is_some_and(|(previous, _)| previous != &enum_name)
                {
                    return Err(Diagnostic::new(
                        "all choice options must belong to the same enum",
                        enum_span,
                    ));
                }
                self.expect(TokenKind::Dot, "expected `.` before the enum variant")?;
                let (variant, variant_span) = self.expect_any_ident("expected an enum variant")?;
                let default_span = self.eat_ident("default");
                let is_default = default_span.is_some();
                if is_default && default_variant.is_some() {
                    return Err(Diagnostic::new(
                        "a choice can only have one default option",
                        variant_span,
                    ));
                }
                let span = option_start.join(self.previous().span);
                self.require_comma_between("choice options");
                Ok((
                    enum_name,
                    enum_span,
                    description,
                    variant,
                    default_span,
                    span,
                ))
            })();
            if let Some((enum_name, enum_span, description, variant, default_span, span)) =
                self.recover_delimited_item(parsed, item_start, body_depth)
            {
                enumeration.get_or_insert((enum_name, enum_span));
                if default_span.is_some() {
                    default_variant = Some(variant.clone());
                }
                options.push(SettingChoiceOption {
                    id: self.new_setting_choice_option_id(),
                    variant,
                    description,
                    default_span,
                    span,
                });
            }
        }
        self.eat(&TokenKind::RBrace);
        let Some((enum_name, enum_span)) = enumeration else {
            return Err(self.error("a choice needs at least one option"));
        };
        let default_variant = default_variant.unwrap_or_else(|| options[0].variant.clone());
        Ok(SettingKind::Choice {
            keyword_span,
            enumeration: EnumReference {
                name: enum_name,
                span: enum_span,
            },
            default_variant,
            options,
        })
    }

    pub(super) fn file_setting(&mut self, keyword_span: Span) -> Result<SettingKind, Diagnostic> {
        self.expect(TokenKind::LBrace, "expected `{` after `file`")?;
        let body_depth = self.cursor.brace_depth();
        let mut filters = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                self.record_missing_closing("unterminated file setting");
                break;
            }
            let item_start = self.cursor.position();
            let parsed = (|| {
                let filter = match self.current().kind.clone() {
                    TokenKind::String(description) => {
                        self.bump();
                        self.expect(TokenKind::FatArrow, "expected `=>` after the filter name")?;
                        let pattern = self.expect_string("expected a file-name pattern")?;
                        SettingFileFilter::Name {
                            description: Some(description),
                            pattern,
                        }
                    }
                    TokenKind::Ident(name) if name == "_" => {
                        self.bump();
                        self.expect(TokenKind::FatArrow, "expected `=>` after `_`")?;
                        let pattern = self.expect_string("expected a file-name pattern")?;
                        SettingFileFilter::Name {
                            description: None,
                            pattern,
                        }
                    }
                    TokenKind::Ident(name) if name == "mime" => {
                        let keyword_span = self.bump().span;
                        self.expect(TokenKind::FatArrow, "expected `=>` after `mime`")?;
                        SettingFileFilter::Mime {
                            value: self.expect_string("expected a MIME type")?,
                            keyword_span,
                        }
                    }
                    _ => {
                        return Err(
                            self.error("expected a named filter, `_` filter, `mime`, or `}`")
                        );
                    }
                };
                self.require_comma_between("file filters");
                Ok(filter)
            })();
            if let Some(filter) = self.recover_delimited_item(parsed, item_start, body_depth) {
                filters.push(filter);
            }
        }
        self.eat(&TokenKind::RBrace);
        Ok(SettingKind::File {
            keyword_span,
            filters,
        })
    }

    pub(super) fn action_block(&mut self) -> Result<Action, Diagnostic> {
        let (name, name_span) = self.expect_any_ident("expected an action name")?;
        let Some(kind) = ActionKind::parse(&name) else {
            if let Some(diagnostic) = legacy_lifecycle_diagnostic(&name) {
                return Err(self.migration_diagnostic(diagnostic, name_span));
            }
            return Err(Diagnostic::new(
                format!("unknown action `{name}`"),
                name_span,
            ));
        };
        let body = self.block()?;
        Ok(Action {
            kind,
            span: name_span.join(body.span),
            body,
        })
    }

    fn require_comma_between(&mut self, items: &'static str) {
        if self.previous().kind == TokenKind::Comma
            || self.eat(&TokenKind::Comma).is_some()
            || self.at(&TokenKind::RBrace)
        {
            return;
        }
        let insertion = Span {
            start: self.previous().span.end,
            end: self.previous().span.end,
        };
        self.record_missing(
            Diagnostic::new(format!("expected `,` between {items}"), self.current().span)
                .with_machine_applicable_fix("insert `,`", insertion, ","),
        );
    }

    fn require_semicolon_between(&mut self, items: &'static str) {
        if self.previous().kind == TokenKind::Semicolon
            || self.eat(&TokenKind::Semicolon).is_some()
            || self.at(&TokenKind::RBrace)
        {
            return;
        }
        let (title, edit) = if self.current().kind == TokenKind::Comma {
            ("replace `,` with `;`", self.current().span)
        } else if self.previous().kind == TokenKind::Comma {
            ("replace `,` with `;`", self.previous().span)
        } else {
            (
                "insert `;`",
                Span {
                    start: self.previous().span.end,
                    end: self.previous().span.end,
                },
            )
        };
        let consume_current_comma = self.current().kind == TokenKind::Comma;
        self.record_missing(
            Diagnostic::new(format!("expected `;` between {items}"), self.current().span)
                .with_machine_applicable_fix(title, edit, ";"),
        );
        if consume_current_comma {
            self.bump();
        }
    }

    pub(super) fn state_decl(&mut self) -> Result<StateDecl, Diagnostic> {
        let start = self.expect_ident("state")?.start;
        self.expect(TokenKind::LParen, "expected `(` after `state`")?;
        let process = self.expect_string("expected a process name string")?;
        self.expect(TokenKind::Comma, "expected `,` after the process name")?;
        self.expect(TokenKind::LBrace, "expected a state object")?;
        let body_depth = self.cursor.brace_depth();
        let mut fields = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                self.record_missing_closing("unterminated state object");
                break;
            }
            let item_start = self.cursor.position();
            let documentation = self.take_source_documentation();
            if self.at(&TokenKind::RBrace) {
                self.diagnostics
                    .push(self.error("a documentation comment must precede a state field"));
                break;
            }
            let parsed = (|| {
                let (name, field_start) =
                    self.expect_declared_ident("expected a state field name")?;
                self.expect(TokenKind::Colon, "expected `:` after the field name")?;
                self.expect_ident("memory")?;
                self.expect(TokenKind::Dot, "expected `.` after `memory`")?;
                let (type_name, type_span) = self.expect_any_ident("expected a memory type")?;
                let Some(ty) = TypeRef::parse(&type_name) else {
                    return Err(Diagnostic::new(
                        format!("`memory.{type_name}` is not a supported memory type"),
                        type_span,
                    ));
                };
                self.expect(TokenKind::LParen, "expected `(` after the memory type")?;
                let module = if matches!(self.current().kind, TokenKind::String(_)) {
                    let module = self.expect_string("expected module name")?;
                    self.expect(TokenKind::Comma, "expected an address offset")?;
                    Some(module)
                } else {
                    None
                };
                let base_start = self.current().span.start;
                let base = if let Some(name) = module {
                    PointerPathBase::Module {
                        name,
                        offset: self.expect_i64("expected a signed module offset")?,
                    }
                } else {
                    PointerPathBase::Absolute(
                        self.expect_u64("expected an unsigned absolute address")?,
                    )
                };
                let base_span = Span {
                    start: base_start,
                    end: self.previous().span.end,
                };
                let mut offsets = Vec::new();
                loop {
                    if self.eat(&TokenKind::Comma).is_none() {
                        break;
                    }
                    if self.at(&TokenKind::RParen) {
                        break;
                    }
                    offsets.push(self.expect_i64("expected a signed pointer offset")?);
                }
                let end = self
                    .expect(TokenKind::RParen, "expected `)` after the pointer path")?
                    .end;
                let field = StateField {
                    id: self.new_value_id(),
                    name,
                    documentation,
                    annotation: Some(ty),
                    source: StateSource::Pointer(PointerPath {
                        at_span: None,
                        base_span,
                        base,
                        offsets,
                        decoder: None,
                    }),
                    transform: None,
                    span: Span {
                        start: field_start.start,
                        end,
                    },
                };
                if self.eat(&TokenKind::Comma).is_none() && !self.at(&TokenKind::RBrace) {
                    return Err(self.error("expected `,` between state fields"));
                }
                Ok(field)
            })();
            if let Some(field) = self.recover_delimited_item(parsed, item_start, body_depth) {
                fields.push(field);
            }
        }
        self.eat(&TokenKind::RBrace);
        if let Err(error) = self.expect(TokenKind::RParen, "expected `)` after the state object") {
            self.record_missing(error);
        }
        self.eat(&TokenKind::Semicolon);
        Ok(StateDecl {
            provider: None,
            processes: vec![process],
            provider_alternatives: Vec::new(),
            fields,
            conditional_fields: Vec::new(),
            provider_enum: None,
            provider_value: None,
            span: Span {
                start,
                end: self.previous().span.end,
            },
        })
    }
}
