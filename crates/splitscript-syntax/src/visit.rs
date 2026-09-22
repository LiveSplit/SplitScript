//! Shared syntax-tree traversal utilities.
//!
//! [`Visitor`] provides immutable preorder traversal. Override a hook to
//! observe a node and call the matching `walk_*` function to keep descending,
//! or omit that call to define a traversal boundary. [`Folder`] is the
//! equivalent in-place mutable traversal for syntax rewrites.

use crate::ast::*;

pub trait Visitor<'ast>: Sized {
    fn visit_program(&mut self, program: &'ast Program) {
        walk_program(self, program);
    }

    fn visit_state(&mut self, state: &'ast StateDecl) {
        walk_state(self, state);
    }

    fn visit_state_field(&mut self, field: &'ast StateField) {
        walk_state_field(self, field);
    }

    fn visit_setting(&mut self, setting: &'ast SettingDecl) {
        walk_setting(self, setting);
    }

    fn visit_setting_family(&mut self, family: &'ast SettingFamilyDecl) {
        walk_setting_family(self, family);
    }

    fn visit_struct(&mut self, structure: &'ast StructDecl) {
        walk_struct(self, structure);
    }

    fn visit_enum(&mut self, enumeration: &'ast EnumDecl) {
        walk_enum(self, enumeration);
    }

    fn visit_managed_image(&mut self, image: &'ast ManagedImageDecl) {
        walk_managed_image(self, image);
    }

    fn visit_managed_namespace(&mut self, namespace: &'ast ManagedNamespaceDecl) {
        walk_managed_namespace(self, namespace);
    }

    fn visit_managed_class(&mut self, class: &'ast ManagedClassDecl) {
        walk_managed_class(self, class);
    }

    fn visit_managed_field(&mut self, field: &'ast ManagedFieldDecl) {
        self.visit_type_ref(&field.ty);
    }

    fn visit_array_type(&mut self, array: &'ast ArrayTypeDecl) {
        self.visit_type_ref(&array.element);
    }

    fn visit_option_type(&mut self, option: &'ast OptionTypeDecl) {
        self.visit_type_ref(&option.value);
    }

    fn visit_result_type(&mut self, result: &'ast ResultTypeDecl) {
        self.visit_type_ref(&result.value);
    }

    fn visit_callable_type(&mut self, callable: &'ast CallableTypeDecl) {
        for parameter in &callable.parameters {
            self.visit_type_ref(parameter);
        }
        self.visit_type_ref(&callable.result);
    }

    fn visit_type_application(&mut self, application: &'ast TypeApplicationDecl) {
        for argument in &application.arguments {
            self.visit_type_ref(argument);
        }
    }

    fn visit_function(&mut self, function: &'ast FunctionDecl) {
        walk_function(self, function);
    }

    /// Visits a value parameter independently of whether it belongs to a
    /// named function or a closure expression.
    fn visit_parameter(&mut self, parameter: &'ast Parameter) {
        walk_parameter(self, parameter);
    }

    /// Visits the shared destructuring pattern used by executable binding
    /// sites such as parameters, variables, and `for` loops.
    fn visit_binding_pattern(&mut self, binding: &'ast BindingPattern) {
        walk_binding_pattern(self, binding);
    }

    fn visit_action(&mut self, action: &'ast Action) {
        self.visit_block(&action.body);
    }

    fn visit_variable(&mut self, variable: &'ast VariableDecl) {
        walk_variable(self, variable);
    }

    fn visit_suspension_binding(&mut self, binding: &'ast SuspensionBinding) {
        if let Some(annotation) = &binding.annotation {
            self.visit_type_ref(annotation);
        }
    }

    fn visit_for_binding(&mut self, binding: &'ast ForBinding) {
        self.visit_binding_pattern(&binding.binding);
    }

    fn visit_block(&mut self, block: &'ast Block) {
        walk_block(self, block);
    }

    fn visit_stmt(&mut self, statement: &'ast Stmt) {
        walk_stmt(self, statement);
    }

    fn visit_expr(&mut self, expression: &'ast Expr) {
        walk_expr(self, expression);
    }

    fn visit_match_arm(&mut self, arm: &'ast MatchArm) {
        walk_match_arm(self, arm);
    }

    fn visit_pattern(&mut self, pattern: &'ast MatchPattern) {
        walk_pattern(self, pattern);
    }

    fn visit_type_ref(&mut self, _ty: &'ast TypeRef) {}
}

pub fn walk_program<'ast, V: Visitor<'ast>>(visitor: &mut V, program: &'ast Program) {
    if let Some(state) = &program.state {
        visitor.visit_state(state);
    }
    for setting in program
        .settings
        .iter()
        .filter(|setting| setting.source_visible)
    {
        visitor.visit_setting(setting);
    }
    for family in &program.setting_families {
        visitor.visit_setting_family(family);
    }
    for global in &program.globals {
        visitor.visit_variable(global);
    }
    for structure in &program.structs {
        visitor.visit_struct(structure);
    }
    for enumeration in &program.enums {
        visitor.visit_enum(enumeration);
    }
    for image in &program.managed_images {
        visitor.visit_managed_image(image);
    }
    for array in &program.array_types {
        visitor.visit_array_type(array);
    }
    for option in &program.option_types {
        visitor.visit_option_type(option);
    }
    for result in &program.result_types {
        visitor.visit_result_type(result);
    }
    for callable in &program.callable_types {
        visitor.visit_callable_type(callable);
    }
    for application in &program.type_applications {
        visitor.visit_type_application(application);
    }
    for function in &program.functions {
        visitor.visit_function(function);
    }
    for action in &program.actions {
        visitor.visit_action(action);
    }
}

pub fn walk_state<'ast, V: Visitor<'ast>>(visitor: &mut V, state: &'ast StateDecl) {
    if let Some(selector) = state
        .provider
        .as_ref()
        .and_then(|provider| provider.selector.as_ref())
    {
        for argument in &selector.arguments {
            visitor.visit_expr(argument);
        }
    }
    for alternative in &state.provider_alternatives {
        if let Some(selector) = &alternative.provider.selector {
            for argument in &selector.arguments {
                visitor.visit_expr(argument);
            }
        }
        for field in &alternative.fields {
            visitor.visit_state_field(field);
        }
    }
    for field in &state.fields {
        visitor.visit_state_field(field);
    }
    for group in &state.conditional_fields {
        if let Some(condition) = &group.condition {
            visitor.visit_expr(condition);
        }
        for field in &group.fields {
            visitor.visit_state_field(field);
        }
    }
    if let Some(enumeration) = &state.provider_enum {
        visitor.visit_enum(enumeration);
    }
}

pub fn walk_state_field<'ast, V: Visitor<'ast>>(visitor: &mut V, field: &'ast StateField) {
    if let Some(annotation) = &field.annotation {
        visitor.visit_type_ref(annotation);
    }
    match &field.source {
        StateSource::Expression(expression) => visitor.visit_expr(expression),
        StateSource::Pointer(path) => {
            if let PointerPathBase::Expression(expression) = &path.base {
                visitor.visit_expr(expression);
            }
        }
    }
    if let Some(transform) = &field.transform {
        visitor.visit_expr(&transform.expression);
    }
}

pub fn walk_setting<'ast, V: Visitor<'ast>>(_visitor: &mut V, _setting: &'ast SettingDecl) {}

pub fn walk_setting_family<'ast, V: Visitor<'ast>>(
    _visitor: &mut V,
    _family: &'ast SettingFamilyDecl,
) {
}

pub fn walk_struct<'ast, V: Visitor<'ast>>(visitor: &mut V, structure: &'ast StructDecl) {
    for field in &structure.fields {
        visitor.visit_type_ref(&field.ty);
    }
}

pub fn walk_enum<'ast, V: Visitor<'ast>>(visitor: &mut V, enumeration: &'ast EnumDecl) {
    if let Some(representation) = &enumeration.representation {
        visitor.visit_type_ref(representation);
    }
    for variant in &enumeration.variants {
        if let Some(payload) = &variant.payload {
            visitor.visit_type_ref(payload);
        }
    }
}

pub fn walk_managed_image<'ast, V: Visitor<'ast>>(visitor: &mut V, image: &'ast ManagedImageDecl) {
    walk_managed_items(visitor, &image.items);
}

fn walk_managed_items<'ast, V: Visitor<'ast>>(visitor: &mut V, items: &'ast [ManagedItemDecl]) {
    for item in items {
        match item {
            ManagedItemDecl::Namespace(namespace) => visitor.visit_managed_namespace(namespace),
            ManagedItemDecl::Class(class) => visitor.visit_managed_class(class),
        }
    }
}

pub fn walk_managed_namespace<'ast, V: Visitor<'ast>>(
    visitor: &mut V,
    namespace: &'ast ManagedNamespaceDecl,
) {
    walk_managed_items(visitor, &namespace.items);
}

pub fn walk_managed_class<'ast, V: Visitor<'ast>>(visitor: &mut V, class: &'ast ManagedClassDecl) {
    for field in &class.fields {
        visitor.visit_managed_field(field);
    }
    for group in &class.conditional_fields {
        if let Some(condition) = &group.condition {
            visitor.visit_expr(condition);
        }
        for field in &group.fields {
            visitor.visit_managed_field(field);
        }
    }
}

pub fn walk_function<'ast, V: Visitor<'ast>>(visitor: &mut V, function: &'ast FunctionDecl) {
    if let Some(receiver) = &function.method_of {
        visitor.visit_type_ref(receiver);
    }
    for parameter in &function.params {
        visitor.visit_parameter(parameter);
    }
    if let Some(result) = &function.return_annotation {
        visitor.visit_type_ref(result);
    }
    visitor.visit_block(&function.body);
}

pub fn walk_parameter<'ast, V: Visitor<'ast>>(visitor: &mut V, parameter: &'ast Parameter) {
    visitor.visit_binding_pattern(&parameter.binding);
    if let Some(annotation) = &parameter.annotation {
        visitor.visit_type_ref(annotation);
    }
}

pub fn walk_variable<'ast, V: Visitor<'ast>>(visitor: &mut V, variable: &'ast VariableDecl) {
    visitor.visit_binding_pattern(&variable.binding);
    if let Some(annotation) = &variable.annotation {
        visitor.visit_type_ref(annotation);
    }
    if let Some(value) = &variable.value {
        visitor.visit_expr(value);
    }
}

pub fn walk_binding_pattern<'ast, V: Visitor<'ast>>(
    visitor: &mut V,
    binding: &'ast BindingPattern,
) {
    visitor.visit_pattern(&binding.pattern.kind);
}

pub fn walk_block<'ast, V: Visitor<'ast>>(visitor: &mut V, block: &'ast Block) {
    for statement in &block.statements {
        visitor.visit_stmt(statement);
    }
}

pub fn walk_stmt<'ast, V: Visitor<'ast>>(visitor: &mut V, statement: &'ast Stmt) {
    match statement {
        Stmt::Debug { statement, .. } => visitor.visit_stmt(statement),
        Stmt::Variable(variable) => visitor.visit_variable(variable),
        Stmt::Assign { value, .. } | Stmt::Yield { value, .. } | Stmt::Expression(value) => {
            visitor.visit_expr(value)
        }
        Stmt::StateAssign { target, value, .. } | Stmt::IndexAssign { target, value, .. } => {
            visitor.visit_expr(target);
            visitor.visit_expr(value);
        }
        Stmt::If {
            condition,
            then_block,
            else_block,
            ..
        } => {
            visitor.visit_expr(condition);
            visitor.visit_block(then_block);
            if let Some(else_block) = else_block {
                visitor.visit_block(else_block);
            }
        }
        Stmt::While {
            condition, body, ..
        } => {
            visitor.visit_expr(condition);
            visitor.visit_block(body);
        }
        Stmt::For {
            binding,
            iterable,
            body,
            ..
        } => {
            visitor.visit_expr(iterable);
            visitor.visit_for_binding(binding);
            visitor.visit_block(body);
        }
        Stmt::Suspend { binding, value, .. } => {
            if let Some(binding) = binding {
                visitor.visit_suspension_binding(binding);
            }
            visitor.visit_expr(value);
        }
    }
}

pub fn walk_expr<'ast, V: Visitor<'ast>>(visitor: &mut V, expression: &'ast Expr) {
    match &expression.kind {
        ExprKind::InterpolatedString(parts) => {
            for part in parts {
                if let InterpolatedPart::Expr(expression) = part {
                    visitor.visit_expr(expression);
                }
            }
        }
        ExprKind::Array(elements) => {
            for element in elements {
                visitor.visit_expr(element);
            }
        }
        ExprKind::Range { start, end, .. } => {
            visitor.visit_expr(start);
            visitor.visit_expr(end);
        }
        ExprKind::Block(block) | ExprKind::Loop(block) => visitor.visit_block(block),
        ExprKind::Struct { fields, .. } => {
            for field in fields {
                visitor.visit_expr(&field.value);
            }
        }
        ExprKind::Match { value, arms } => {
            visitor.visit_expr(value);
            for arm in arms {
                visitor.visit_match_arm(arm);
            }
        }
        ExprKind::Is { value, pattern, .. } => {
            visitor.visit_expr(value);
            visitor.visit_pattern(&pattern.kind);
        }
        ExprKind::If {
            condition,
            then_expr,
            else_expr,
        } => {
            visitor.visit_expr(condition);
            visitor.visit_expr(then_expr);
            visitor.visit_expr(else_expr);
        }
        ExprKind::Fallback { value, fallback } => {
            visitor.visit_expr(value);
            visitor.visit_expr(fallback);
        }
        ExprKind::Break(Some(value))
        | ExprKind::Return(Some(value))
        | ExprKind::Throw(value)
        | ExprKind::Inspect { value, .. }
        | ExprKind::Suspend { value, .. }
        | ExprKind::Propagate(value) => visitor.visit_expr(value),
        ExprKind::Member { receiver, .. } => visitor.visit_expr(receiver),
        ExprKind::Index {
            receiver, index, ..
        } => {
            visitor.visit_expr(receiver);
            visitor.visit_expr(index);
        }
        ExprKind::Unary { expr, .. } | ExprKind::Cast { expr, .. } => visitor.visit_expr(expr),
        ExprKind::Binary { left, right, .. } => {
            visitor.visit_expr(left);
            visitor.visit_expr(right);
        }
        ExprKind::Call {
            receiver,
            type_arguments,
            args,
            ..
        } => {
            if let Some(receiver) = receiver {
                visitor.visit_expr(receiver);
            }
            for argument in type_arguments {
                visitor.visit_type_ref(argument);
            }
            for argument in args {
                visitor.visit_expr(argument);
            }
        }
        ExprKind::Invoke { callee, args } => {
            visitor.visit_expr(callee);
            for argument in args {
                visitor.visit_expr(argument);
            }
        }
        ExprKind::Closure {
            params,
            return_annotation,
            body,
            ..
        } => {
            for parameter in params {
                visitor.visit_parameter(parameter);
            }
            if let Some(result) = return_annotation {
                visitor.visit_type_ref(result);
            }
            visitor.visit_expr(body);
        }
        ExprKind::Error
        | ExprKind::None
        | ExprKind::IteratorEnd
        | ExprKind::Break(None)
        | ExprKind::Continue
        | ExprKind::Return(None)
        | ExprKind::Bool(_)
        | ExprKind::Int { .. }
        | ExprKind::Float(_)
        | ExprKind::Char(_)
        | ExprKind::String(_)
        | ExprKind::Signature(_)
        | ExprKind::Path(_) => {}
    }
}

pub fn walk_match_arm<'ast, V: Visitor<'ast>>(visitor: &mut V, arm: &'ast MatchArm) {
    visitor.visit_pattern(&arm.pattern);
    if let Some(guard) = &arm.guard {
        visitor.visit_expr(guard);
    }
    visitor.visit_expr(&arm.value);
}

pub fn walk_pattern<'ast, V: Visitor<'ast>>(visitor: &mut V, pattern: &'ast MatchPattern) {
    if let MatchPattern::Int {
        suffix: Some(suffix),
        ..
    } = pattern
    {
        visitor.visit_type_ref(suffix);
    }
    if let MatchPattern::IntRange {
        start_suffix,
        end_suffix,
        ..
    } = pattern
    {
        if let Some(suffix) = start_suffix {
            visitor.visit_type_ref(suffix);
        }
        if let Some(suffix) = end_suffix {
            visitor.visit_type_ref(suffix);
        }
    }
    match pattern {
        MatchPattern::Struct { fields, .. } => {
            for field in fields {
                visitor.visit_pattern(&field.pattern.kind);
            }
        }
        MatchPattern::Enum {
            payload: Some(payload),
            ..
        }
        | MatchPattern::OptionSome(payload)
        | MatchPattern::IteratorItem(payload)
        | MatchPattern::ResultSuccess(payload)
        | MatchPattern::ResultError(payload) => visitor.visit_pattern(&payload.kind),
        MatchPattern::Array(array) => {
            for element in array.elements() {
                visitor.visit_pattern(&element.kind);
            }
        }
        MatchPattern::Alternation(elements) => {
            for element in elements {
                visitor.visit_pattern(&element.kind);
            }
        }
        _ => {}
    }
}

pub trait Folder: Sized {
    fn fold_program(&mut self, program: &mut Program) {
        walk_program_mut(self, program);
    }

    fn fold_state(&mut self, state: &mut StateDecl) {
        walk_state_mut(self, state);
    }

    fn fold_state_field(&mut self, field: &mut StateField) {
        walk_state_field_mut(self, field);
    }

    fn fold_setting(&mut self, setting: &mut SettingDecl) {
        walk_setting_mut(self, setting);
    }

    fn fold_setting_family(&mut self, family: &mut SettingFamilyDecl) {
        walk_setting_family_mut(self, family);
    }

    fn fold_struct(&mut self, structure: &mut StructDecl) {
        walk_struct_mut(self, structure);
    }

    fn fold_enum(&mut self, enumeration: &mut EnumDecl) {
        walk_enum_mut(self, enumeration);
    }

    fn fold_managed_image(&mut self, image: &mut ManagedImageDecl) {
        walk_managed_image_mut(self, image);
    }

    fn fold_managed_namespace(&mut self, namespace: &mut ManagedNamespaceDecl) {
        walk_managed_namespace_mut(self, namespace);
    }

    fn fold_managed_class(&mut self, class: &mut ManagedClassDecl) {
        walk_managed_class_mut(self, class);
    }

    fn fold_managed_field(&mut self, field: &mut ManagedFieldDecl) {
        self.fold_type_ref(&mut field.ty);
    }

    fn fold_array_type(&mut self, array: &mut ArrayTypeDecl) {
        self.fold_type_ref(&mut array.element);
    }

    fn fold_option_type(&mut self, option: &mut OptionTypeDecl) {
        self.fold_type_ref(&mut option.value);
    }

    fn fold_result_type(&mut self, result: &mut ResultTypeDecl) {
        self.fold_type_ref(&mut result.value);
    }

    fn fold_function(&mut self, function: &mut FunctionDecl) {
        walk_function_mut(self, function);
    }

    /// Folds a value parameter independently of whether it belongs to a
    /// named function or a closure expression.
    fn fold_parameter(&mut self, parameter: &mut Parameter) {
        walk_parameter_mut(self, parameter);
    }

    fn fold_action(&mut self, action: &mut Action) {
        self.fold_block(&mut action.body);
    }

    fn fold_variable(&mut self, variable: &mut VariableDecl) {
        walk_variable_mut(self, variable);
    }

    fn fold_suspension_binding(&mut self, binding: &mut SuspensionBinding) {
        if let Some(annotation) = &mut binding.annotation {
            self.fold_type_ref(annotation);
        }
    }

    fn fold_for_binding(&mut self, binding: &mut ForBinding) {
        self.fold_pattern(&mut binding.binding.pattern.kind);
    }

    fn fold_block(&mut self, block: &mut Block) {
        walk_block_mut(self, block);
    }

    fn fold_stmt(&mut self, statement: &mut Stmt) {
        walk_stmt_mut(self, statement);
    }

    fn fold_expr(&mut self, expression: &mut Expr) {
        walk_expr_mut(self, expression);
    }

    fn fold_match_arm(&mut self, arm: &mut MatchArm) {
        walk_match_arm_mut(self, arm);
    }

    fn fold_pattern(&mut self, pattern: &mut MatchPattern) {
        walk_pattern_mut(self, pattern);
    }

    fn fold_type_ref(&mut self, _ty: &mut TypeRef) {}
}

pub fn walk_program_mut<F: Folder>(folder: &mut F, program: &mut Program) {
    if let Some(state) = &mut program.state {
        folder.fold_state(state);
    }
    for setting in program
        .settings
        .iter_mut()
        .filter(|setting| setting.source_visible)
    {
        folder.fold_setting(setting);
    }
    for family in &mut program.setting_families {
        folder.fold_setting_family(family);
    }
    for global in &mut program.globals {
        folder.fold_variable(global);
    }
    for structure in &mut program.structs {
        folder.fold_struct(structure);
    }
    for enumeration in &mut program.enums {
        folder.fold_enum(enumeration);
    }
    for image in &mut program.managed_images {
        folder.fold_managed_image(image);
    }
    for array in &mut program.array_types {
        folder.fold_array_type(array);
    }
    for option in &mut program.option_types {
        folder.fold_option_type(option);
    }
    for result in &mut program.result_types {
        folder.fold_result_type(result);
    }
    for function in &mut program.functions {
        folder.fold_function(function);
    }
    for action in &mut program.actions {
        folder.fold_action(action);
    }
}

pub fn walk_state_mut<F: Folder>(folder: &mut F, state: &mut StateDecl) {
    for alternative in &mut state.provider_alternatives {
        if let Some(selector) = &mut alternative.provider.selector {
            for argument in &mut selector.arguments {
                folder.fold_expr(argument);
            }
        }
        for field in &mut alternative.fields {
            folder.fold_state_field(field);
        }
    }
    for field in &mut state.fields {
        folder.fold_state_field(field);
    }
    for group in &mut state.conditional_fields {
        if let Some(condition) = &mut group.condition {
            folder.fold_expr(condition);
        }
        for field in &mut group.fields {
            folder.fold_state_field(field);
        }
    }
    if let Some(enumeration) = &mut state.provider_enum {
        folder.fold_enum(enumeration);
    }
}

pub fn walk_state_field_mut<F: Folder>(folder: &mut F, field: &mut StateField) {
    if let Some(annotation) = &mut field.annotation {
        folder.fold_type_ref(annotation);
    }
    match &mut field.source {
        StateSource::Expression(expression) => folder.fold_expr(expression),
        StateSource::Pointer(path) => {
            if let PointerPathBase::Expression(expression) = &mut path.base {
                folder.fold_expr(expression);
            }
        }
    }
    if let Some(transform) = &mut field.transform {
        folder.fold_expr(&mut transform.expression);
    }
}

pub fn walk_setting_mut<F: Folder>(_folder: &mut F, _setting: &mut SettingDecl) {}

pub fn walk_setting_family_mut<F: Folder>(_folder: &mut F, _family: &mut SettingFamilyDecl) {}

pub fn walk_struct_mut<F: Folder>(folder: &mut F, structure: &mut StructDecl) {
    for field in &mut structure.fields {
        folder.fold_type_ref(&mut field.ty);
    }
}

pub fn walk_enum_mut<F: Folder>(folder: &mut F, enumeration: &mut EnumDecl) {
    for variant in &mut enumeration.variants {
        if let Some(payload) = &mut variant.payload {
            folder.fold_type_ref(payload);
        }
    }
}

pub fn walk_managed_image_mut<F: Folder>(folder: &mut F, image: &mut ManagedImageDecl) {
    walk_managed_items_mut(folder, &mut image.items);
}

fn walk_managed_items_mut<F: Folder>(folder: &mut F, items: &mut [ManagedItemDecl]) {
    for item in items {
        match item {
            ManagedItemDecl::Namespace(namespace) => folder.fold_managed_namespace(namespace),
            ManagedItemDecl::Class(class) => folder.fold_managed_class(class),
        }
    }
}

pub fn walk_managed_namespace_mut<F: Folder>(folder: &mut F, namespace: &mut ManagedNamespaceDecl) {
    walk_managed_items_mut(folder, &mut namespace.items);
}

pub fn walk_managed_class_mut<F: Folder>(folder: &mut F, class: &mut ManagedClassDecl) {
    for field in &mut class.fields {
        folder.fold_managed_field(field);
    }
    for group in &mut class.conditional_fields {
        if let Some(condition) = &mut group.condition {
            folder.fold_expr(condition);
        }
        for field in &mut group.fields {
            folder.fold_managed_field(field);
        }
    }
}

pub fn walk_function_mut<F: Folder>(folder: &mut F, function: &mut FunctionDecl) {
    if let Some(receiver) = &mut function.method_of {
        folder.fold_type_ref(receiver);
    }
    for parameter in &mut function.params {
        folder.fold_parameter(parameter);
    }
    if let Some(result) = &mut function.return_annotation {
        folder.fold_type_ref(result);
    }
    folder.fold_block(&mut function.body);
}

pub fn walk_parameter_mut<F: Folder>(folder: &mut F, parameter: &mut Parameter) {
    folder.fold_pattern(&mut parameter.binding.pattern.kind);
    if let Some(annotation) = &mut parameter.annotation {
        folder.fold_type_ref(annotation);
    }
}

pub fn walk_variable_mut<F: Folder>(folder: &mut F, variable: &mut VariableDecl) {
    folder.fold_pattern(&mut variable.binding.pattern.kind);
    if let Some(annotation) = &mut variable.annotation {
        folder.fold_type_ref(annotation);
    }
    if let Some(value) = &mut variable.value {
        folder.fold_expr(value);
    }
}

pub fn walk_block_mut<F: Folder>(folder: &mut F, block: &mut Block) {
    for statement in &mut block.statements {
        folder.fold_stmt(statement);
    }
}

pub fn walk_stmt_mut<F: Folder>(folder: &mut F, statement: &mut Stmt) {
    match statement {
        Stmt::Debug { statement, .. } => folder.fold_stmt(statement),
        Stmt::Variable(variable) => folder.fold_variable(variable),
        Stmt::Assign { value, .. } | Stmt::Yield { value, .. } | Stmt::Expression(value) => {
            folder.fold_expr(value)
        }
        Stmt::StateAssign { target, value, .. } | Stmt::IndexAssign { target, value, .. } => {
            folder.fold_expr(target);
            folder.fold_expr(value);
        }
        Stmt::If {
            condition,
            then_block,
            else_block,
            ..
        } => {
            folder.fold_expr(condition);
            folder.fold_block(then_block);
            if let Some(else_block) = else_block {
                folder.fold_block(else_block);
            }
        }
        Stmt::While {
            condition, body, ..
        } => {
            folder.fold_expr(condition);
            folder.fold_block(body);
        }
        Stmt::For {
            binding,
            iterable,
            body,
            ..
        } => {
            folder.fold_expr(iterable);
            folder.fold_for_binding(binding);
            folder.fold_block(body);
        }
        Stmt::Suspend { binding, value, .. } => {
            if let Some(binding) = binding {
                folder.fold_suspension_binding(binding);
            }
            folder.fold_expr(value);
        }
    }
}

pub fn walk_expr_mut<F: Folder>(folder: &mut F, expression: &mut Expr) {
    match &mut expression.kind {
        ExprKind::InterpolatedString(parts) => {
            for part in parts {
                if let InterpolatedPart::Expr(expression) = part {
                    folder.fold_expr(expression);
                }
            }
        }
        ExprKind::Array(elements) => {
            for element in elements {
                folder.fold_expr(element);
            }
        }
        ExprKind::Range { start, end, .. } => {
            folder.fold_expr(start);
            folder.fold_expr(end);
        }
        ExprKind::Block(block) | ExprKind::Loop(block) => folder.fold_block(block),
        ExprKind::Struct { fields, .. } => {
            for field in fields {
                folder.fold_expr(&mut field.value);
            }
        }
        ExprKind::Match { value, arms } => {
            folder.fold_expr(value);
            for arm in arms {
                folder.fold_match_arm(arm);
            }
        }
        ExprKind::Is { value, pattern, .. } => {
            folder.fold_expr(value);
            folder.fold_pattern(&mut pattern.kind);
        }
        ExprKind::If {
            condition,
            then_expr,
            else_expr,
        } => {
            folder.fold_expr(condition);
            folder.fold_expr(then_expr);
            folder.fold_expr(else_expr);
        }
        ExprKind::Fallback { value, fallback } => {
            folder.fold_expr(value);
            folder.fold_expr(fallback);
        }
        ExprKind::Break(Some(value))
        | ExprKind::Return(Some(value))
        | ExprKind::Throw(value)
        | ExprKind::Inspect { value, .. }
        | ExprKind::Suspend { value, .. }
        | ExprKind::Propagate(value) => folder.fold_expr(value),
        ExprKind::Member { receiver, .. } => folder.fold_expr(receiver),
        ExprKind::Index {
            receiver, index, ..
        } => {
            folder.fold_expr(receiver);
            folder.fold_expr(index);
        }
        ExprKind::Unary { expr, .. } => folder.fold_expr(expr),
        ExprKind::Cast { expr, target } => {
            folder.fold_expr(expr);
            folder.fold_type_ref(target);
        }
        ExprKind::Binary { left, right, .. } => {
            folder.fold_expr(left);
            folder.fold_expr(right);
        }
        ExprKind::Call {
            receiver,
            type_arguments,
            args,
            ..
        } => {
            if let Some(receiver) = receiver {
                folder.fold_expr(receiver);
            }
            for argument in type_arguments {
                folder.fold_type_ref(argument);
            }
            for argument in args {
                folder.fold_expr(argument);
            }
        }
        ExprKind::Invoke { callee, args } => {
            folder.fold_expr(callee);
            for argument in args {
                folder.fold_expr(argument);
            }
        }
        ExprKind::Closure {
            params,
            return_annotation,
            body,
            ..
        } => {
            for parameter in params {
                folder.fold_parameter(parameter);
            }
            if let Some(result) = return_annotation {
                folder.fold_type_ref(result);
            }
            folder.fold_expr(body);
        }
        ExprKind::Error
        | ExprKind::None
        | ExprKind::IteratorEnd
        | ExprKind::Break(None)
        | ExprKind::Continue
        | ExprKind::Return(None)
        | ExprKind::Bool(_)
        | ExprKind::Int { .. }
        | ExprKind::Float(_)
        | ExprKind::Char(_)
        | ExprKind::String(_)
        | ExprKind::Signature(_)
        | ExprKind::Path(_) => {}
    }
}

pub fn walk_match_arm_mut<F: Folder>(folder: &mut F, arm: &mut MatchArm) {
    folder.fold_pattern(&mut arm.pattern);
    if let Some(guard) = &mut arm.guard {
        folder.fold_expr(guard);
    }
    folder.fold_expr(&mut arm.value);
}

pub fn walk_pattern_mut<F: Folder>(folder: &mut F, pattern: &mut MatchPattern) {
    if let MatchPattern::Int {
        suffix: Some(suffix),
        ..
    } = pattern
    {
        folder.fold_type_ref(suffix);
    }
    if let MatchPattern::IntRange {
        start_suffix,
        end_suffix,
        ..
    } = pattern
    {
        if let Some(suffix) = start_suffix {
            folder.fold_type_ref(suffix);
        }
        if let Some(suffix) = end_suffix {
            folder.fold_type_ref(suffix);
        }
    }
    match pattern {
        MatchPattern::Struct { fields, .. } => {
            for field in fields {
                folder.fold_pattern(&mut field.pattern.kind);
            }
        }
        MatchPattern::Enum {
            payload: Some(payload),
            ..
        }
        | MatchPattern::OptionSome(payload)
        | MatchPattern::IteratorItem(payload)
        | MatchPattern::ResultSuccess(payload)
        | MatchPattern::ResultError(payload) => folder.fold_pattern(&mut payload.kind),
        MatchPattern::Array(array) => {
            for element in array.prefix.iter_mut().chain(&mut array.suffix) {
                folder.fold_pattern(&mut element.kind);
            }
        }
        MatchPattern::Alternation(elements) => {
            for element in elements {
                folder.fold_pattern(&mut element.kind);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    const NESTED_SOURCE: &str = r#"
        state "game.exe" {}

        whileAttached {
            let value = if true {
                match 1 {
                    1 => `a{2}`,
                    _ => "b"
                }
            } else {
                "c"
            }
            print(value)
        }
    "#;

    #[derive(Default)]
    struct ExpressionIds {
        ids: Vec<ExprId>,
    }

    impl<'ast> Visitor<'ast> for ExpressionIds {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            self.ids.push(expression.id);
            walk_expr(self, expression);
        }
    }

    #[test]
    fn visitor_reaches_every_nested_expression_once() {
        let tokens = crate::lex(NESTED_SOURCE, crate::SyntaxMode::Program).unwrap();
        let parsed = crate::parser::parse(NESTED_SOURCE, tokens).unwrap();
        let mut visitor = ExpressionIds::default();
        visitor.visit_program(&parsed);

        assert_eq!(visitor.ids.len(), 10);
        assert_eq!(
            visitor.ids.iter().copied().collect::<HashSet<_>>().len(),
            visitor.ids.len()
        );
    }

    struct PrefixStrings;

    impl Folder for PrefixStrings {
        fn fold_expr(&mut self, expression: &mut Expr) {
            match &mut expression.kind {
                ExprKind::String(value) => value.insert_str(0, "folded:"),
                ExprKind::InterpolatedString(parts) => {
                    for part in parts {
                        if let InterpolatedPart::Text(value) = part {
                            value.insert_str(0, "folded:");
                        }
                    }
                }
                _ => {}
            }
            walk_expr_mut(self, expression);
        }
    }

    #[derive(Default)]
    struct StringValues {
        values: Vec<String>,
    }

    impl<'ast> Visitor<'ast> for StringValues {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            match &expression.kind {
                ExprKind::String(value) => self.values.push(value.clone()),
                ExprKind::InterpolatedString(parts) => {
                    self.values
                        .extend(parts.iter().filter_map(|part| match part {
                            InterpolatedPart::Text(value) => Some(value.clone()),
                            InterpolatedPart::Expr(_) => None,
                        }));
                }
                _ => {}
            }
            walk_expr(self, expression);
        }
    }

    #[test]
    fn folder_rewrites_nested_nodes_with_the_same_child_order() {
        let tokens = crate::lex(NESTED_SOURCE, crate::SyntaxMode::Program).unwrap();
        let mut syntax = crate::parser::parse(NESTED_SOURCE, tokens).unwrap();
        PrefixStrings.fold_program(&mut syntax);

        let mut strings = StringValues::default();
        strings.visit_program(&syntax);
        assert_eq!(strings.values, ["folded:a", "folded:b", "folded:c"]);
    }

    #[test]
    fn visitor_exposes_named_function_and_closure_parameters_uniformly() {
        let source = r#"
            state "game.exe" {}
            fn apply(value: u16, transform: (u16) -> u16) -> u16 {
                return transform(value)
            }
            whileAttached {
                print(apply(1, (left, right) => left + right))
            }
        "#;
        let tokens = crate::lex(source, crate::SyntaxMode::Program).unwrap();
        let syntax = crate::parser::parse(source, tokens).unwrap();

        #[derive(Default)]
        struct ParameterNames(Vec<String>);

        impl<'ast> Visitor<'ast> for ParameterNames {
            fn visit_parameter(&mut self, parameter: &'ast Parameter) {
                self.0.push(parameter.name.clone());
                walk_parameter(self, parameter);
            }
        }

        let mut names = ParameterNames::default();
        names.visit_program(&syntax);
        assert_eq!(names.0, ["value", "transform", "left", "right"]);
    }
}
