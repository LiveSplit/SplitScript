//! catalogs types integration tests.

use super::*;

#[test]
fn provider_profiles_accept_constant_constructors_but_not_runtime_state() {
    let compile = |body: &str| {
        splitscript::compile(&format!(
            "{body}\nstate Unity.mono(pick()) [\"game.exe\"] {{}}"
        ))
    };
    for body in [
        "fn pick() -> MonoVersion { return MonoVersion.V2 }",
        "fn identity(value: MonoVersion) -> MonoVersion { return value } fn pick() -> MonoVersion { return identity(MonoVersion.V2) }",
    ] {
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&compile(body).expect("a constant constructor is valid configuration"))
            .unwrap();
    }
    for body in [
        "let version = MonoVersion.V2; fn pick() -> MonoVersion { return version }",
        "fn pick() -> MonoVersion { print(1); return MonoVersion.V2 }",
        "fn pick() -> MonoVersion { return pick() }",
        "fn pick() -> MonoVersion { return other() } fn other() -> MonoVersion { return pick() }",
    ] {
        let errors = compile(body).expect_err("configuration cannot read or change runtime state");
        assert!(
            errors.iter().any(|error| error
                .message
                .contains("configuration must be a compile-time constant")),
            "{errors:?}"
        );
    }
}

#[test]
fn source_defined_library_bodies_compile_without_leaking_hidden_declarations() {
    let library = StandardLibrary::new();
    for item in [
        StdlibItemId::DolphinCoreBase,
        StdlibItemId::DurationFromFrames,
        StdlibItemId::DurationFromMilliseconds,
        StdlibItemId::DurationFromParts,
        StdlibItemId::DurationFromSeconds,
        StdlibItemId::DurationFromNanoseconds,
        StdlibItemId::DurationWholeSeconds,
        StdlibItemId::DurationSubsecondNanoseconds,
        StdlibItemId::DurationTotalSeconds,
        StdlibItemId::DurationTotalMilliseconds,
        StdlibItemId::DurationAdd,
        StdlibItemId::DurationSubtract,
        StdlibItemId::DurationLessThan,
        StdlibItemId::DurationLessThanOrEqual,
        StdlibItemId::DurationGreaterThan,
        StdlibItemId::DurationGreaterThanOrEqual,
        StdlibItemId::DurationZero,
        StdlibItemId::InstantAdd,
        StdlibItemId::InstantDurationSince,
        StdlibItemId::InstantElapsed,
        StdlibItemId::InstantHasElapsed,
        StdlibItemId::FloatRoundTo,
        StdlibItemId::FloatIsNaN,
        StdlibItemId::FloatIsFinite,
        StdlibItemId::NumericClamp,
        StdlibItemId::NumericSquared,
        StdlibItemId::SignedAbs,
        StdlibItemId::ArrayIsEmpty,
        StdlibItemId::ArrayContains,
        StdlibItemId::ArrayIndexOf,
        StdlibItemId::ArrayExtend,
        StdlibItemId::ArrayRemove,
        StdlibItemId::ArrayPop,
        StdlibItemId::ResultDiscardError,
        StdlibItemId::AddressOffset,
        StdlibItemId::ModulePeOptionalHeader,
        StdlibItemId::UnityIl2Cpp,
        StdlibItemId::UnityProviderContext,
        StdlibItemId::UnitySceneManager,
        StdlibItemId::UnityScenesSnapshot,
        StdlibItemId::UnityScenesActive,
        StdlibItemId::UnityScenesLoaded,
        StdlibItemId::UnityScenesPersistent,
        StdlibItemId::GBAEmulatorResolve64BitMemoryPointer,
        StdlibItemId::MonoLayoutForVersion,
        StdlibItemId::GBAEmulatorDiscover,
        StdlibItemId::PS2EmulatorDiscover,
        StdlibItemId::PS1EmulatorDiscover,
        StdlibItemId::SMSEmulatorDiscover,
        StdlibItemId::GenesisEmulatorDiscover,
        StdlibItemId::GCNEmulatorDiscover,
        StdlibItemId::WiiEmulatorDiscover,
    ] {
        assert!(matches!(
            library.item(item).implementation,
            Implementation::LibraryBody { .. } | Implementation::LibraryOverloads { .. }
        ));
    }

    let source = r#"
        state "game.exe" {}
        gameTime {
            return Duration.fromFrames(125, 60)
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("a source-defined library function should use the ordinary checker");
    assert!(checked.syntax().functions.is_empty());
    assert_eq!(checked.typed_hir().function_bodies().count(), 0);
    assert_eq!(checked.semantics().calls().count(), 1);

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::compile(source).unwrap())
        .expect("a call through a source-defined library body should produce valid Wasm");
}

#[test]
fn growable_arrays_store_non_null_standard_library_gc_values() {
    let source = r#"
        state "game.exe" {}

        whileAttached {
            let durations: [Duration] = []
            durations.push(Duration.fromSeconds(1))
            let first: Duration = durations[0]
            print(first.wholeSeconds())
        }
    "#;

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::compile(source).unwrap())
        .expect("array backing slots should preserve non-null source values");
}

#[test]
fn unity_context_exposes_immutable_scene_snapshots() {
    let source = r#"
        state Unity ["game.exe", "game-demo.exe"] {
            activeScene = unity.scenes.active();
            loadedScenes = unity.scenes.loaded();
            persistentScene = unity.scenes.persistent();
        }

        whileAttached {
            print(current.activeScene.name)
            print(current.activeScene.index)
            print(current.activeScene.address)
            print(current.loadedScenes.length())
            print(current.persistentScene.address)
        }
    "#;

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::compile(source).unwrap())
        .expect("source-defined Unity scene snapshots should produce valid Wasm GC");

    let diagnostics = splitscript::compile(
        r#"
            state Unity ["game.exe"] {}
            onDetach { let scenes = unity.scenes }
        "#,
    )
    .expect_err("the Unity context must not escape its process attachment");
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.message.contains("requires an attached process")
                && diagnostic.message.contains("onDetach")
        }),
        "{diagnostics:#?}"
    );
}

#[test]
fn private_standard_library_helpers_are_checked_but_not_user_visible() {
    let library = StandardLibrary::new();
    let layout = library.type_decl(StdlibTypeId::MonoLayout);
    assert_eq!(layout.visibility, TypeVisibility::LibraryPrivate);
    assert!(library.type_by_name("MonoLayout").is_none());
    assert!(library.types().all(|ty| ty.id != StdlibTypeId::MonoLayout));
    for (id, name) in [
        (StdlibTypeId::MonoModule, "MonoModule"),
        (StdlibTypeId::MonoImage, "MonoImage"),
        (StdlibTypeId::MonoClass, "MonoClass"),
        (StdlibTypeId::UnityModule, "UnityModule"),
        (StdlibTypeId::UnityImage, "UnityImage"),
        (StdlibTypeId::UnityClass, "UnityClass"),
        (StdlibTypeId::UnityField, "UnityField"),
    ] {
        assert_eq!(
            library.type_decl(id).visibility,
            TypeVisibility::LibraryPrivate
        );
        assert!(library.type_by_name(name).is_none());
        assert!(library.types().all(|ty| ty.id != id));
    }
    for (id, qualified_name) in [
        (StdlibItemId::DolphinCoreBase, "dolphinCoreBase"),
        (
            StdlibItemId::ModulePeOptionalHeader,
            "Module.peOptionalHeader",
        ),
        (StdlibItemId::UnityScenesSnapshot, "UnityScenes.snapshot"),
        (
            StdlibItemId::GBAEmulatorResolve64BitMemoryPointer,
            "GBAEmulator.resolve64BitMemoryPointer",
        ),
        (StdlibItemId::MonoLayoutForVersion, "MonoLayout.forVersion"),
        (StdlibItemId::GBAEmulatorDiscover, "GBAEmulator.discover"),
        (StdlibItemId::PS2EmulatorDiscover, "PS2Emulator.discover"),
        (StdlibItemId::PS1EmulatorDiscover, "PS1Emulator.discover"),
        (StdlibItemId::SMSEmulatorDiscover, "SMSEmulator.discover"),
        (
            StdlibItemId::GenesisEmulatorDiscover,
            "GenesisEmulator.discover",
        ),
        (StdlibItemId::GCNEmulatorDiscover, "GCNEmulator.discover"),
        (StdlibItemId::WiiEmulatorDiscover, "WiiEmulator.discover"),
        (StdlibItemId::MonoModuleDiscover, "MonoModule.discover"),
        (
            StdlibItemId::MonoModuleClassAnyInImage,
            "MonoModule.classAnyInImage",
        ),
        (
            StdlibItemId::MonoModuleFieldInClass,
            "MonoModule.fieldInClass",
        ),
        (
            StdlibItemId::MonoModuleStaticTableForClass,
            "MonoModule.staticTableForClass",
        ),
    ] {
        let helper = library.item(id);
        assert_eq!(helper.visibility, ItemVisibility::LibraryPrivate);
        assert!(matches!(
            helper.implementation,
            Implementation::LibraryBody { .. }
        ));
        assert!(library.items().all(|item| item.id != id));
        assert!(library.item_by_name(qualified_name).is_none());
    }
    assert!(
        library
            .methods_for_type(&TypeKind::Standard(StdlibTypeId::UnityScenes))
            .into_iter()
            .all(|item| item.id != StdlibItemId::UnityScenesSnapshot)
    );
    assert!(
        library
            .methods_for_type(&TypeKind::Standard(StdlibTypeId::MonoModule))
            .into_iter()
            .all(|item| !matches!(
                item.id,
                StdlibItemId::MonoModuleClassAnyInImage
                    | StdlibItemId::MonoModuleFieldInClass
                    | StdlibItemId::MonoModuleStaticTableForClass
            ))
    );

    let diagnostics = splitscript::compile(
        r#"
            let layout: MonoLayout
            state "game.exe" {}
        "#,
    )
    .expect_err("user code must not name private standard-library types");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message.contains("unknown type `MonoLayout`")
            && !diagnostic.message.contains("private")
    }));

    let diagnostics = splitscript::compile(
        r#"
            state Unity ["game.exe"] {}
            onAttach {
                let scene = unity.scenes.snapshot(0x1000)
            }
        "#,
    )
    .expect_err("user code must not call private standard-library helpers");
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("type `UnityScenes` has no method `snapshot`")
                && !diagnostic.message.contains("private")
        }),
        "{diagnostics:#?}"
    );

    let diagnostics = splitscript::compile(
        r#"
            state "game.exe" {}
            onAttach {
                let core = await dolphinCoreBase()
            }
        "#,
    )
    .expect_err("user code must not call private root helpers");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("unknown function `dolphinCoreBase`")
            && !diagnostic.message.contains("private")
    }));

    let diagnostics = splitscript::compile(
        r#"
            state "game.exe" {}
            onAttach {
                let emulator = await GBAEmulator.discover()
            }
        "#,
    )
    .expect_err("user code must not call state-provider attachment helpers");
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("unknown variable `GBAEmulator`")
                && !diagnostic.message.contains("private")
        }),
        "{diagnostics:#?}"
    );

    let diagnostics = splitscript::compile(
        r#"
            state "game.exe" {}
            onAttach {
                let mono = await Unity.mono(MonoVersion.V2)
            }
        "#,
    )
    .expect_err("user code must use managed schemas instead of manual traversal");
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("declare Unity managed metadata with an `image` schema")
                && diagnostic.migration_topic() == Some("asl.unity.managed-schema")
        }),
        "{diagnostics:#?}"
    );
}

#[test]
fn discarding_an_error_is_source_defined_and_preserves_generic_inference() {
    let library = StandardLibrary::new();
    assert_eq!(
        library.render_signature(StdlibItemId::ResultDiscardError),
        "T!.discardError() -> T?"
    );
    assert!(matches!(
        library
            .item(StdlibItemId::ResultDiscardError)
            .implementation,
        Implementation::LibraryBody { .. }
    ));

    let source = r#"
        state "game.exe" {
            optional: i32? = process.read<i32>(0x1000).discardError()
        }

        whileAttached {
            let text = match current.optional {
                Some(value) => value as String,
                None => "missing",
            }
            print(text)
        }
    "#;
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::compile(source).unwrap())
        .expect("source-defined Result-to-Option conversion should produce valid Wasm GC");
}

#[test]
fn must_use_obligations_are_catalog_owned() {
    let library = StandardLibrary::new();
    assert!(
        library
            .type_constructor(StdlibTypeConstructorId::Option)
            .must_use
            .expect("Option values should carry a use obligation")
            .contains("inspected")
    );
    assert!(
        library
            .type_constructor(StdlibTypeConstructorId::Result)
            .must_use
            .expect("Result values should carry a use obligation")
            .contains("failures")
    );
    assert!(
        library
            .item(StdlibItemId::StringReplaceAll)
            .must_use
            .expect("immutable replacement should explain its returned value")
            .contains("immutable")
    );
    assert!(
        library
            .item(StdlibItemId::StringToAsciiLowerCase)
            .must_use
            .expect("immutable case conversion should explain its returned value")
            .contains("immutable")
    );
    assert!(
        library
            .must_use(StdlibItemId::F32FromBits)
            .expect("float construction should carry a use obligation")
            .contains("constructed")
    );
    assert!(
        library
            .must_use(StdlibItemId::ProcessRead)
            .expect("non-mutating reads should receive the catalog default")
            .contains("only produces")
    );
    assert_eq!(library.must_use(StdlibItemId::SetInsert), None);
}

#[test]
fn duration_convenience_constructors_are_source_defined() {
    let library = StandardLibrary::new();
    assert_eq!(
        library.render_signature(StdlibItemId::DurationZero),
        "Duration.zero() -> Duration"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::DurationFromMilliseconds),
        "Duration.fromMilliseconds<T>(milliseconds: T) -> Duration where T: Numeric"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::DurationFromMinutes),
        "Duration.fromMinutes<T>(minutes: T) -> Duration where T: Numeric"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::DurationFromHours),
        "Duration.fromHours<T>(hours: T) -> Duration where T: Numeric"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::DurationFromDays),
        "Duration.fromDays<T>(days: T) -> Duration where T: Numeric"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::DurationFromNanoseconds),
        "Duration.fromNanoseconds<T>(nanoseconds: T) -> Duration where T: Numeric"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::DurationAdd),
        "Duration.add(other: Duration) -> Duration"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::DurationTotalSeconds),
        "Duration.totalSeconds() -> f64"
    );
    assert!(library.type_has_capability(
        splitscript::compiler::stdlib::StdlibTypeId::Duration,
        splitscript::compiler::stdlib::StdlibCapabilityId::Equatable,
    ));

    for expression in [
        "Duration.zero()",
        "Duration.fromSeconds(-12)",
        "Duration.fromMilliseconds(1_500)",
        "Duration.fromNanoseconds(-1_250_000_100)",
        "Duration.fromMilliseconds(1_500.25)",
        "Duration.fromMilliseconds(1_500.25 as f32)",
        "Duration.fromMinutes(1.5)",
        "Duration.fromMinutes(1.5 as f32)",
        "Duration.fromHours(1.25)",
        "Duration.fromDays(1.5)",
        "Duration.fromSeconds(1.25)",
        "Duration.fromSeconds(1.25 as f32)",
    ] {
        let source = format!(
            r#"
                state "game.exe" {{}}
                gameTime {{
                    return {expression}
                }}
            "#
        );
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&splitscript::compile(&source).unwrap())
            .expect("source-defined duration convenience constructors should produce valid Wasm");
    }
}

#[test]
fn instant_uses_one_monotonic_host_boundary_and_source_defined_arithmetic() {
    let library = StandardLibrary::new();
    assert_eq!(
        library.render_signature(StdlibItemId::InstantNow),
        "Instant.now() -> Instant"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::InstantAdd),
        "Instant.add(duration: Duration) -> Instant"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::InstantDurationSince),
        "Instant.durationSince(earlier: Instant) -> Duration"
    );
    assert!(matches!(
        library.item(StdlibItemId::InstantNow).implementation,
        Implementation::Intrinsic(IntrinsicId::InstantNow)
    ));
    for item in [
        StdlibItemId::InstantAdd,
        StdlibItemId::InstantDurationSince,
        StdlibItemId::InstantElapsed,
        StdlibItemId::InstantHasElapsed,
    ] {
        assert!(matches!(
            library.item(item).implementation,
            Implementation::LibraryBody { .. }
        ));
    }
    let now_effects = library.operation_metadata(StdlibItemId::InstantNow).effects;
    assert!(now_effects.contains(&splitscript::compiler::stdlib::Effect::ReadsRuntime));
    assert!(now_effects.contains(&splitscript::compiler::stdlib::Effect::Allocates));

    let source = r#"
        state "game.exe" {}
        whileAttached {
            let startedAt = Instant.now()
            let deadline: Instant = startedAt
            deadline += Duration.fromMilliseconds(250)
            if startedAt.hasElapsed(Duration.zero()) && deadline != startedAt {
                print("ready")
            }
        }
    "#;
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::compile(source).unwrap())
        .expect("monotonic instants and their source-defined methods should produce valid Wasm");
}

#[test]
fn binary_syntax_resolves_through_catalog_declared_methods() {
    let library = StandardLibrary::new();
    assert_eq!(
        library.item(StdlibItemId::InstantAdd).binary_operator,
        Some(StandardBinaryOperator::Add)
    );
    assert_eq!(
        library.item(StdlibItemId::DurationAdd).binary_operator,
        Some(StandardBinaryOperator::Add)
    );
    assert_eq!(
        library.item(StdlibItemId::DurationSubtract).binary_operator,
        Some(StandardBinaryOperator::Subtract)
    );
    assert_eq!(
        library.item(StdlibItemId::DurationLessThan).binary_operator,
        Some(StandardBinaryOperator::LessThan)
    );
    assert_eq!(
        library
            .item(StdlibItemId::DurationGreaterThanOrEqual)
            .binary_operator,
        Some(StandardBinaryOperator::GreaterThanOrEqual)
    );
    assert_eq!(
        library.item(StdlibItemId::NumericAdd).binary_operator,
        Some(StandardBinaryOperator::Add)
    );
    assert_eq!(
        library.item(StdlibItemId::NumericMultiply).binary_operator,
        Some(StandardBinaryOperator::Multiply)
    );
    assert_eq!(
        library.item(StdlibItemId::NumericDivide).binary_operator,
        Some(StandardBinaryOperator::Divide)
    );
    assert_eq!(
        library.item(StdlibItemId::IntegerRemainder).binary_operator,
        Some(StandardBinaryOperator::Remainder)
    );
    for (item, operator) in [
        (StdlibItemId::IntegerBitOr, StandardBinaryOperator::BitOr),
        (StdlibItemId::IntegerBitXor, StandardBinaryOperator::BitXor),
        (StdlibItemId::IntegerBitAnd, StandardBinaryOperator::BitAnd),
        (
            StdlibItemId::IntegerShiftLeft,
            StandardBinaryOperator::ShiftLeft,
        ),
        (
            StdlibItemId::IntegerShiftRight,
            StandardBinaryOperator::ShiftRight,
        ),
    ] {
        assert_eq!(library.item(item).binary_operator, Some(operator));
    }
    assert_eq!(
        library.item(StdlibItemId::EquatableEquals).binary_operator,
        Some(StandardBinaryOperator::Equal)
    );
    assert_eq!(
        library
            .item(StdlibItemId::EquatableNotEquals)
            .binary_operator,
        Some(StandardBinaryOperator::NotEqual)
    );

    let source = r#"
        state "game.exe" {}
        whileAttached {
            let number = 20 + 22
            let product = number * 2
            let quotient = product / 3
            let remainder = quotient % 5
            let direct = 6.multiply(7).divide(3).remainder(5)
            let bits = ((0x10 | 0x04) ^ 0x01) & 0x1f
            let shifted = bits << 2 >> 1
            let directBits = 1u32.shiftLeft(5).bitOr(3).shiftRight(1).bitXor(2).bitAnd(0xff)
            let same = number.equals(42)
            let different = number.notEquals(41)
            let unitEqual = None.equals(None)
            let duration = Duration.fromSeconds(1.5) + Duration.fromSeconds(0.5)
            let difference = duration - Duration.fromSeconds(1)
            let ordered = difference < duration && duration >= difference
            print(number)
            print(remainder)
            print(direct)
            print(shifted)
            print(directBits)
            print(same && different)
            print(unitEqual)
            print(difference.wholeSeconds())
            if ordered {
                print("ordered")
            }
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("catalog-declared operators should type-check as ordinary calls");
    let resolved_items = checked
        .semantics()
        .calls()
        .filter_map(|(_, call)| match call {
            ResolvedCall::StandardLibrary { item, .. } => Some(*item),
            _ => None,
        })
        .collect::<Vec<_>>();
    for expected in [
        StdlibItemId::NumericAdd,
        StdlibItemId::NumericMultiply,
        StdlibItemId::NumericDivide,
        StdlibItemId::IntegerRemainder,
        StdlibItemId::IntegerBitOr,
        StdlibItemId::IntegerBitXor,
        StdlibItemId::IntegerBitAnd,
        StdlibItemId::IntegerShiftLeft,
        StdlibItemId::IntegerShiftRight,
        StdlibItemId::EquatableEquals,
        StdlibItemId::EquatableNotEquals,
        StdlibItemId::DurationAdd,
        StdlibItemId::DurationSubtract,
        StdlibItemId::DurationLessThan,
        StdlibItemId::DurationGreaterThanOrEqual,
    ] {
        assert!(
            resolved_items.contains(&expected),
            "operator syntax should resolve to {expected:?}"
        );
    }
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::compile(source).unwrap())
        .expect("catalog-declared operators should lower to valid Wasm");
}

#[test]
fn unary_syntax_resolves_through_catalog_declared_methods() {
    let library = StandardLibrary::new();
    assert_eq!(
        library.item(StdlibItemId::BoolNot).unary_operator,
        Some(StandardUnaryOperator::Not)
    );
    assert_eq!(
        library.item(StdlibItemId::SignedNegate).unary_operator,
        Some(StandardUnaryOperator::Negate)
    );
    assert_eq!(
        library.item(StdlibItemId::IntegerBitNot).unary_operator,
        Some(StandardUnaryOperator::Not)
    );
    assert_eq!(
        library.render_signature(StdlibItemId::BoolNot),
        "bool.not() -> bool"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::SignedNegate),
        "Self.negate() -> Self where Self: Signed"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::IntegerBitNot),
        "Self.bitNot() -> Self where Self: Integer"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::NumericSwapBytes),
        "Self.swapBytes() -> Self where Self: Numeric"
    );

    let source = r#"
        state "game.exe" {}
        whileAttached {
            let offset: i32 = -42
            let reverse = offset.negate()
            let ready = !false
            let disabled = ready.not()
            let byte: u8 = !1
            let original = byte.bitNot()
            let full: u32 = !0u32
            let inferred = !1
            print(`{offset}:{reverse}:{ready}:{disabled}:{byte}:{original}:{full}:{inferred}`)
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("catalog-declared unary operators should type-check as ordinary calls");
    let resolved_items = checked
        .semantics()
        .calls()
        .filter_map(|(_, call)| match call {
            ResolvedCall::StandardLibrary { item, .. } => Some(*item),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        resolved_items
            .iter()
            .filter(|item| **item == StdlibItemId::SignedNegate)
            .count(),
        1
    );
    assert_eq!(
        resolved_items
            .iter()
            .filter(|item| **item == StdlibItemId::BoolNot)
            .count(),
        2
    );
    assert_eq!(
        resolved_items
            .iter()
            .filter(|item| **item == StdlibItemId::IntegerBitNot)
            .count(),
        4
    );
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("catalog-declared unary operators should lower to valid Wasm");
}

#[test]
fn compound_assignment_resolves_through_the_same_catalog_method() {
    let source = r#"
        state "game.exe" {}
        whileAttached {
            let elapsed = Duration.zero()
            elapsed += Duration.fromSeconds(1)
            print(elapsed.wholeSeconds())
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("Duration compound assignment should type-check through its operator binding");
    let assignment = checked
        .typed_hir()
        .action_body(splitscript::compiler::ast::ActionKind::WhileAttached)
        .unwrap()
        .statements
        .iter()
        .find_map(|statement| match &statement.kind {
            splitscript::compiler::hir::TypedStatementKind::Assign { assignment, .. } => {
                Some(assignment)
            }
            _ => None,
        })
        .expect("fixture should contain a compound assignment");
    assert!(matches!(
        assignment.operator,
        Some(ResolvedCall::StandardLibrary {
            item: StdlibItemId::DurationAdd,
            ..
        })
    ));
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("source-defined compound operators should produce valid Wasm");
}

#[test]
fn standard_library_bodies_can_read_only_their_own_private_representation() {
    let source = r#"
        state "game.exe" {}
        whileAttached {
            let duration = Duration.zero()
            let leaked = duration.seconds
        }
    "#;
    let diagnostics = splitscript::compile(source)
        .expect_err("user source must not read private standard-library representation fields");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == splitscript::DiagnosticCode::Type
            && diagnostic
                .message
                .contains("Duration has no field `seconds`")
    }));
}

#[test]
fn generic_library_bodies_emit_only_reachable_concrete_instances() {
    let defined_functions = |wasm: &[u8]| {
        Parser::new(0)
            .parse_all(wasm)
            .find_map(
                |payload| match payload.expect("generated Wasm should parse") {
                    Payload::FunctionSection(section) => Some(section.count()),
                    _ => None,
                },
            )
            .expect("generated Wasm has a function section")
    };
    let baseline = splitscript::compile(
        r#"
            state "game.exe" {}
            whileAttached {
                let signed: i32 = 10
                let wide: f64 = 10.0
            }
        "#,
    )
    .expect("baseline should compile");
    let specialized = splitscript::compile(
        r#"
            state "game.exe" {}
            whileAttached {
                let signed: i32 = 10
                let wide: f64 = 10.0
                let boundedSigned = signed.clamp(0, 7)
                let boundedWide = wide.clamp(0.0, 7.0)
            }
        "#,
    )
    .expect("generic source-body instances should compile");

    assert_eq!(
        defined_functions(&specialized),
        defined_functions(&baseline) + 2,
        "only the i32 and f64 clamp instances should survive reachability"
    );
}

#[test]
fn generic_array_library_bodies_share_demand_driven_specialization() {
    let defined_functions = |wasm: &[u8]| {
        Parser::new(0)
            .parse_all(wasm)
            .find_map(
                |payload| match payload.expect("generated Wasm should parse") {
                    Payload::FunctionSection(section) => Some(section.count()),
                    _ => None,
                },
            )
            .expect("generated Wasm has a function section")
    };
    let baseline = splitscript::compile(
        r#"
            state "game.exe" {}
            whileAttached {
                let numbers: [i32] = []
                let names: [String] = []
                let numberCount = numbers.length()
                let nameCount = names.length()
            }
        "#,
    )
    .expect("the intrinsic baseline should compile");
    let specialized = splitscript::compile(
        r#"
            state "game.exe" {}
            whileAttached {
                let numbers: [i32] = []
                let names: [String] = []
                let noNumbers = numbers.isEmpty()
                let noNames = names.isEmpty()
            }
        "#,
    )
    .expect("generic array source bodies should compile");

    assert_eq!(
        defined_functions(&specialized),
        defined_functions(&baseline) + 2,
        "one concrete isEmpty body should be emitted for each reachable array element type"
    );
}

#[test]
fn array_search_methods_are_source_defined_and_preserve_element_constraints() {
    let library = StandardLibrary::new();
    assert_eq!(
        library.render_signature(StdlibItemId::ArrayContains),
        "[T].contains(value: T) -> bool where T: Equatable"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::ArrayIndexOf),
        "[T].indexOf(value: T) -> u32? where T: Equatable"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::ArrayRemove),
        "[T].remove(value: T) -> bool where T: Equatable"
    );
    for item in [
        StdlibItemId::ArrayContains,
        StdlibItemId::ArrayIndexOf,
        StdlibItemId::ArrayRemove,
    ] {
        assert!(matches!(
            library.item(item).implementation,
            Implementation::LibraryBody { .. }
        ));
    }

    for expression in [
        "[2, 4, 6].contains(4)",
        "[2, 4, 6].indexOf(6) else 99",
        "[\"Moon\", \"Sun\"].contains(\"Sun\")",
        "[\"Moon\", \"Sun\"].indexOf(\"missing\") else 99",
    ] {
        let source = format!(
            r#"
                state "game.exe" {{}}
                whileAttached {{
                    print({expression})
                }}
            "#
        );
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&splitscript::compile(&source).unwrap())
            .unwrap_or_else(|error| panic!("`{expression}` generated invalid Wasm GC: {error}"));
    }

    let nested = splitscript::compile(
        r#"
            struct Marker { values: [i32] }
            state "game.exe" {}
            whileAttached {
                let markers: [Marker; 1] = [Marker { values: [1] }]
                print(markers.contains(Marker { values: [1] }))
            }
        "#,
    )
    .expect("array equality should let structs containing equatable arrays derive equality");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&nested)
        .expect("nested structural array equality should produce valid Wasm GC");
}

#[test]
fn source_defined_library_bodies_publish_compiler_derived_operation_metadata() {
    let library = StandardLibrary::new();
    let clamp = library.item(StdlibItemId::NumericClamp);
    assert!(matches!(
        clamp.implementation,
        Implementation::LibraryBody { .. }
    ));
    assert_eq!(
        library
            .effects(clamp.id)
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        [Effect::Pure]
    );

    let timer = library.item(StdlibItemId::TimerIsRunning);
    assert!(matches!(
        timer.implementation,
        Implementation::LibraryBody { .. }
    ));
    assert!(library.effects(timer.id).contains(&Effect::ReadsTimer));
    assert!(!library.effects(timer.id).contains(&Effect::Pure));
    assert_eq!(
        library.render_operation_semantics(timer.id),
        "available everywhere; synchronous"
    );

    let relative = library.item(StdlibItemId::ModuleReadRelative32);
    assert!(matches!(
        relative.implementation,
        Implementation::LibraryBody { .. }
    ));
    let operation = library.operation_semantics(relative.id);
    assert!(library.effects(relative.id).contains(&Effect::ReadsProcess));
    assert!(operation.requires_attached_process);
    assert_eq!(operation.availability, Availability::Everywhere);
    assert_eq!(operation.suspension, SuspensionKind::None);
    assert_eq!(operation.cancellation, CancellationKind::None);

    let source = r#"
        state "game.exe" {}
        fn resolveRelative(module: Module) -> address! {
            return module.readRelative32(0)
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("effectful library bodies should type check as ordinary calls");
    let function = checked.syntax().functions[0].id;
    let inferred = checked.effects().function(function);
    assert!(inferred.effects.contains(&Effect::ReadsProcess));
    assert!(inferred.requires_attached_process);
}

#[test]
fn user_code_cannot_construct_runtime_private_standard_library_structs() {
    let parsed = splitscript::parse(
        r#"
        state "game.exe" {}
        gameTime {
            return Duration { seconds: 1, nanoseconds: 0 }
        }
        "#,
    )
    .unwrap();
    let diagnostics = splitscript::check(splitscript::lower(parsed)).unwrap_err();
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message.contains(
            "standard-library type `Duration` can only be constructed by standard-library source",
        )
    }));
}

#[test]
fn compiler_owned_library_function_names_are_reserved() {
    let parsed = splitscript::parse(
        r#"
        state "game.exe" {}
        fn __splitscript_stdlib_fake() {}
        "#,
    )
    .unwrap();
    let diagnostics = splitscript::check(splitscript::lower(parsed)).unwrap_err();
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("`__splitscript_stdlib_` are reserved")
    }));
}

#[test]
fn state_providers_are_catalog_owned_and_resolved_after_parsing() {
    let library = StandardLibrary::new();
    let declared_process_override = splitscript::check(splitscript::lower(
        splitscript::parse(r#"state GBA ["mGBA.exe"] {}"#).unwrap(),
    ))
    .expect_err("providers with catalog-owned process lists reject source overrides");
    assert!(declared_process_override.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("state provider `GBA` declares its supported processes")
    }));

    let gba = library
        .state_provider_by_name("GBA")
        .expect("the bundled GBA provider should be discoverable by source name");
    assert_eq!(gba.id, StdlibStateProviderId::Gba);
    assert_eq!(gba.value_name, "gba");
    assert_eq!(gba.process_type, StdlibTypeId::GBAEmulator);
    assert_eq!(gba.direct_read, StdlibItemId::GBAEmulatorRead);
    assert_eq!(
        gba.attachment,
        splitscript::compiler::stdlib::StateProviderAttachment::Callable(
            StdlibItemId::GBAEmulatorDiscover,
        )
    );
    assert!(matches!(
        library
            .item(StdlibItemId::GBAEmulatorDiscover)
            .implementation,
        Implementation::LibraryBody { .. }
    ));
    assert_eq!(
        library.item(StdlibItemId::GBAEmulatorDiscover).visibility,
        ItemVisibility::LibraryPrivate
    );
    assert!(matches!(
        gba.processes,
        splitscript::compiler::stdlib::StateProviderProcesses::Declared(processes)
            if processes.contains(&"mGBA.exe")
    ));

    let lowered = splitscript::lower(splitscript::parse("state GBA {}").unwrap());
    let state = lowered.syntax().state.as_ref().unwrap();
    assert_eq!(
        state
            .provider
            .as_ref()
            .map(|provider| provider.name.as_str()),
        Some("GBA")
    );
    assert!(state.processes.is_empty());
    let checked = splitscript::check(lowered).unwrap();
    assert_eq!(
        checked.semantics().state_provider(),
        Some(StdlibStateProviderId::Gba)
    );

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::compile("state GBA { room: u8 at 0x03000010 }").unwrap())
        .expect("provider-backed attachment should compile to valid Wasm");

    let invalid = splitscript::parse("state GBA { room: u8 = process.read(0) else 0 }")
        .map(splitscript::lower)
        .unwrap();
    let diagnostics = splitscript::check(invalid)
        .expect_err("native process operations should not be available in a GBA state");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("`process` is unavailable under `state GBA`; use `gba` instead")
    }));

    let invalid = splitscript::parse("state GBA { room: u8 at \"game.exe\", 0x10 }")
        .map(splitscript::lower)
        .unwrap();
    let diagnostics = splitscript::check(invalid)
        .expect_err("provider direct reads should reject native module paths");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("direct reads use hardware addresses and cannot name a module")
    }));

    let source = "state GBA { room: u8 at 0x03000010 }";
    let mut database = splitscript::tooling::database::CompilerDatabase::new(source);
    let provider_offset = source.find("GBA").unwrap() + 1;
    assert_eq!(
        database.definition_at(provider_offset).unwrap(),
        Some(
            splitscript::tooling::database::DefinitionTarget::StandardLibrarySymbol(
                splitscript::compiler::stdlib::StdlibSymbolId::StateProvider(
                    StdlibStateProviderId::Gba
                )
            )
        )
    );
    let hover = database
        .hover(provider_offset)
        .unwrap()
        .expect("the provider name should have catalog documentation");
    assert!(hover.markdown.contains("state GBA { ... }"));
    assert!(hover.markdown.contains("gba: GBAEmulator"));
}

#[test]
fn named_state_provider_alternatives_compile() {
    let source = r#"
        state {
            provider Windows: Native ["game.exe"] {
                value: u8 at 0x10;
                windowsOnly: u8 at 0x11
            },
            provider Advance: GBA {
                value: u8 at 0x03000010;
                advanceOnly: u8 at 0x03000011
            },
        }

        split {
            return match provider {
                StateProvider.Windows => current.value == 1 && current.windowsOnly == 2,
                StateProvider.Advance => current.value == 2 && current.advanceOnly == 3,
            }
        }
    "#;
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::compile(source).unwrap())
        .expect("multi-provider state should compile to valid Wasm");
    let mut database = splitscript::tooling::database::CompilerDatabase::new(source);
    let keyword = source.find("provider Windows").unwrap() + 1;
    let hover = database
        .hover(keyword)
        .unwrap()
        .expect("the contextual provider keyword should have documentation");
    assert!(hover.markdown.contains("runtime alternative"));
    let value = source.find("match provider").unwrap() + "match ".len() + 1;
    let hover = database
        .hover(value)
        .unwrap()
        .expect("the selected provider value should have inferred hover information");
    assert!(hover.markdown.contains("provider: StateProvider"));

    let diagnostics = splitscript::compile(
        r#"
            state {
                provider Windows: Native ["game.exe"] { windowsOnly: u8 at 0x10 },
                provider Advance: GBA { advanceOnly: u8 at 0x03000010 },
            }
            split { return current.windowsOnly == 1 }
        "#,
    )
    .expect_err("provider-specific fields require provider refinement");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message.contains("state field `windowsOnly`")
            || diagnostic
                .message
                .contains("unknown state field `windowsOnly`")
    }));
}

#[test]
fn unknown_state_providers_list_valid_choices_and_fix_clear_typos() {
    let diagnostics = splitscript::check(splitscript::lower(
        splitscript::parse(r#"state Untiy ["game.exe"] {}"#).unwrap(),
    ))
    .expect_err("the misspelled provider should be rejected");
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.message.contains("unknown state provider"))
        .expect("resolution should explain the unknown provider");

    assert_eq!(
        diagnostic.message,
        "unknown state provider `Untiy`; did you mean `Unity`?"
    );
    assert!(diagnostic.notes.iter().any(|note| {
        note == "valid state providers are `GBA`, `GCN`, `Genesis`, `Native`, `PS1`, `PS2`, `SMS`, `Unity`, `Wii`"
    }));
    assert_eq!(
        diagnostic.documentation_uri(),
        Some("/stdlib/state-providers/index.md")
    );
    assert_eq!(diagnostic.fixes.len(), 1);
    assert_eq!(diagnostic.fixes[0].edits[0].replacement, "Unity");
}

#[test]
fn unsupported_state_provider_names_are_not_recast_as_typos() {
    let diagnostics = splitscript::check(splitscript::lower(
        splitscript::parse("state SNES {}").unwrap(),
    ))
    .expect_err("an unsupported provider should be rejected");
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.message.contains("unknown state provider"))
        .expect("resolution should explain the unknown provider");

    assert_eq!(diagnostic.message, "unknown state provider `SNES`");
    assert!(diagnostic.fixes.is_empty());
    assert_eq!(
        diagnostic.documentation_uri(),
        Some("/stdlib/state-providers/index.md")
    );
}

#[test]
fn unity_provider_preparation_is_selected_typed_and_lowered_before_attachment() {
    let library = StandardLibrary::new();
    let unity = library
        .state_provider_by_name("Unity")
        .expect("the bundled Unity provider should be discoverable");
    assert_eq!(unity.id, StdlibStateProviderId::Unity);
    assert_eq!(unity.value_name, "process");
    assert_eq!(unity.process_type, StdlibTypeId::Process);
    assert_eq!(unity.preparation, Some(StdlibItemId::UnityProviderAuto));
    assert_eq!(unity.contexts.len(), 1);
    assert_eq!(unity.contexts[0].name, "unity");
    assert_eq!(unity.contexts[0].ty, StdlibTypeId::UnityContext);
    assert_eq!(
        unity.contexts[0].preparation,
        StdlibItemId::UnityProviderContext
    );
    assert_eq!(unity.selectors.len(), 2);
    assert_eq!(
        unity.selectors[0].preparation,
        StdlibItemId::UnityProviderIl2Cpp
    );
    assert_eq!(
        unity.selectors[1].preparation,
        StdlibItemId::UnityProviderMono
    );

    for (source, selector) in [
        (r#"state Unity ["game.exe"] {}"#, None),
        (
            r#"state Unity.il2cpp(Il2CppProfile.unity2021_3_11f1X64()) ["game.exe"] {}"#,
            Some(0),
        ),
        (
            r#"state Unity.mono(MonoVersion.V3) ["game.exe"] {}"#,
            Some(1),
        ),
    ] {
        let checked = splitscript::check(splitscript::parse(source).unwrap())
            .expect("each Unity provider configuration should type-check");
        assert_eq!(
            checked.semantics().state_provider(),
            Some(StdlibStateProviderId::Unity)
        );
        assert_eq!(checked.semantics().state_provider_selector(), selector);
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&splitscript::codegen(&checked))
            .expect("Unity provider preparation should emit valid Wasm");
    }

    let schema = r#"
        enum Edition {
            BaseGame,
            Demo,
        }

        let edition: Edition

        image "Assembly-CSharp" {
            class Player {
                u32 score;
            }

            class GameManager from ["Manager", "GameManager"] {
                static GameManager instance;
                Player player;

                if edition == Edition.BaseGame {
                    u32 level;
                }

                if edition == Edition.Demo {
                    u32 scene;
                }
            }
        }
        state Unity ["game.exe"] {
            score: u32 = GameManager.instance?.player?.score?
        }

        onAttach {
            edition = Edition.BaseGame
        }

        whileAttached {
            let manager = GameManager.instance else return
            if edition == Edition.BaseGame {
                print(manager.level else 0)
            }
            if edition == Edition.Demo {
                print(manager.scene else 0)
            }
        }

    "#;
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::compile(schema).unwrap())
        .expect("a Unity schema should be bound by the provider preparation phase");

    let native = schema.replace("state Unity [\"game.exe\"]", "state \"game.exe\"");
    let diagnostics = splitscript::compile(&native)
        .expect_err("live managed reads need the Unity attachment provider");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("live managed fields require a Unity state provider")
    }));

    let wrong_layout = schema.replace("print(manager.scene else 0)", "print(manager.level else 0)");
    let diagnostics = splitscript::compile(&wrong_layout)
        .expect_err("layout-only managed fields must remain narrowed to their own match arm");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("managed field `GameManager.level` is conditional")
    }));
}

#[test]
fn ps2_provider_is_source_defined_and_supports_direct_and_pointer_path_reads() {
    let library = StandardLibrary::new();
    let ps2 = library
        .state_provider_by_name("PS2")
        .expect("the bundled PS2 provider should be discoverable by source name");
    assert_eq!(ps2.id, StdlibStateProviderId::Ps2);
    assert_eq!(ps2.value_name, "ps2");
    assert_eq!(ps2.process_type, StdlibTypeId::PS2Emulator);
    assert_eq!(ps2.direct_read, StdlibItemId::PS2EmulatorRead);
    assert_eq!(
        ps2.readable_ranges,
        &[splitscript::compiler::stdlib::StateProviderMemoryRange {
            start: 0x0010_0000,
            end: 0x0200_0000,
        }]
    );
    assert_eq!(
        ps2.attachment,
        splitscript::compiler::stdlib::StateProviderAttachment::Callable(
            StdlibItemId::PS2EmulatorDiscover,
        )
    );
    assert!(matches!(
        library
            .item(StdlibItemId::PS2EmulatorDiscover)
            .implementation,
        Implementation::LibraryBody { .. }
    ));
    assert!(matches!(
        ps2.processes,
        splitscript::compiler::stdlib::StateProviderProcesses::Declared(processes)
            if processes.contains(&"pcsx2-qt.exe")
                && processes.contains(&"retroarch.exe")
    ));

    let source = r#"
        state PS2 {
            direct: u16 at 0x00100000;
            pointer: u32 at 0x00100100, 0x20, -0x8;
        }

        whileAttached {
            let value: u32 = ps2.read(0x00100200) else 0
            print(value)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("PS2 direct reads and provider-relative pointer paths should type-check");
    assert_eq!(
        checked.semantics().state_provider(),
        Some(StdlibStateProviderId::Ps2)
    );
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("PS2 direct reads and provider-relative pointer paths should emit valid Wasm");
}

#[test]
fn ps2_literal_guest_reads_are_checked_against_the_catalogued_domain() {
    for (source, literal, expected_size) in [
        (r#"state PS2 { value: u8 at 0x000fffff; }"#, "0x000fffff", 1),
        (r#"state PS2 { value: u8 at 0x02000000; }"#, "0x02000000", 1),
        (
            r#"state PS2 { value: u32 at 0x01fffffe; }"#,
            "0x01fffffe",
            4,
        ),
        (
            r#"
                state PS2 {}
                whileAttached {
                    let value: u16 = ps2.read(0x02000000) else 0
                    print(value)
                }
            "#,
            "0x02000000",
            2,
        ),
    ] {
        let diagnostics = splitscript::compile(source)
            .expect_err("a literal PS2 read outside main RAM must fail before code generation");
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic
                    .message
                    .contains("literal guest-memory read is outside `state PS2`")
            })
            .unwrap_or_else(|| panic!("missing focused guest-memory diagnostic: {diagnostics:#?}"));
        assert_eq!(&source[diagnostic.span.start..diagnostic.span.end], literal);
        assert!(diagnostic.labels.iter().any(|label| {
            label
                .message
                .as_deref()
                .is_some_and(|message| message.contains(&format!("reads {expected_size} byte")))
        }));
        assert!(diagnostic.notes.iter().any(|note| {
            note.contains("0x00100000..<0x02000000") && note.contains("must lie entirely within")
        }));
    }

    let dynamic = r#"
        state PS2 {
            address: u32 = 0;
            value: u8 at address;
        }

        fn readAt(address: u32) -> u8! {
            return ps2.read(address)
        }

        whileAttached {
            print(readAt(current.address) else 0)
        }
    "#;
    splitscript::compile(dynamic).expect("computed guest addresses remain fallible runtime reads");
}

#[test]
fn ps1_provider_covers_asr_backends_and_guest_pointer_paths() {
    let library = StandardLibrary::new();
    let ps1 = library
        .state_provider_by_name("PS1")
        .expect("the bundled PS1 provider should be discoverable by source name");
    assert_eq!(ps1.id, StdlibStateProviderId::Ps1);
    assert_eq!(ps1.value_name, "ps1");
    assert_eq!(ps1.process_type, StdlibTypeId::PS1Emulator);
    assert_eq!(ps1.direct_read, StdlibItemId::PS1EmulatorRead);
    assert_eq!(
        ps1.attachment,
        splitscript::compiler::stdlib::StateProviderAttachment::Callable(
            StdlibItemId::PS1EmulatorDiscover,
        )
    );
    assert!(matches!(
        library
            .item(StdlibItemId::PS1EmulatorDiscover)
            .implementation,
        Implementation::LibraryBody { .. }
    ));
    assert!(matches!(
        ps1.processes,
        splitscript::compiler::stdlib::StateProviderProcesses::Declared(processes)
            if processes.contains(&"ePSXe.exe")
                && processes.contains(&"retroarch.exe")
                && processes.contains(&"pcsx-redux.main")
    ));

    let source = r#"
        state PS1 {
            direct: u16 at 0x80000000;
            pointer: u32 at 0x80000100, 0x20, -0x8;
        }

        fn readName(reader, address) -> String! {
            return reader.readUtf8(address, 32)
        }

        fn readValue(reader, address) -> u32! {
            return reader.read<u32>(address)
        }

        whileAttached {
            let value: u32 = ps1.read(0x80000200) else 0
            let genericValue = readValue(ps1, 0x80000200) else 0
            let name = readName(ps1, 0x80000300) else "Unknown"
            let wide = ps1.readUtf16Le(0x80000400, 32) else "Unknown"
            print(value)
            print(genericValue)
            print(name)
            print(wide)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("PS1 direct reads and guest pointer paths should type-check");
    assert_eq!(
        checked.semantics().state_provider(),
        Some(StdlibStateProviderId::Ps1)
    );
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("PS1 direct reads and guest pointer paths should emit valid Wasm");

    let provider_source = "state PS1 {}";
    let mut database = splitscript::tooling::database::CompilerDatabase::new(provider_source);
    let hover = database
        .hover(provider_source.find("PS1").unwrap() + 1)
        .unwrap()
        .expect("the PS1 provider should expose catalog documentation");
    assert!(hover.markdown.contains("state PS1 { ... }"));
    assert!(hover.markdown.contains("ps1: PS1Emulator"));

    let completion_source = "state PS1 {}\nwhileAttached { ps1. }";
    let mut database = splitscript::tooling::database::CompilerDatabase::new(completion_source);
    let completion = database
        .completions(completion_source.find("ps1.").unwrap() + "ps1.".len())
        .unwrap();
    for method in ["read", "readUtf8", "readUtf16Le"] {
        assert!(
            completion.items.iter().any(|item| item.label == method),
            "PS1 MemoryReader completion is missing `{method}`: {:#?}",
            completion.items
        );
    }
}

#[test]
fn sms_provider_covers_asr_backends_and_guest_pointer_paths() {
    let library = StandardLibrary::new();
    let sms = library
        .state_provider_by_name("SMS")
        .expect("the bundled SMS provider should be discoverable by source name");
    assert_eq!(sms.id, StdlibStateProviderId::Sms);
    assert_eq!(sms.value_name, "sms");
    assert_eq!(sms.process_type, StdlibTypeId::SMSEmulator);
    assert_eq!(sms.direct_read, StdlibItemId::SMSEmulatorRead);
    assert_eq!(
        sms.attachment,
        splitscript::compiler::stdlib::StateProviderAttachment::Callable(
            StdlibItemId::SMSEmulatorDiscover,
        )
    );
    assert!(matches!(
        library
            .item(StdlibItemId::SMSEmulatorDiscover)
            .implementation,
        Implementation::LibraryBody { .. }
    ));
    assert!(matches!(
        sms.processes,
        splitscript::compiler::stdlib::StateProviderProcesses::Declared(processes)
            if processes.contains(&"Fusion.exe")
                && processes.contains(&"blastem.exe")
                && processes.contains(&"retroarch.exe")
    ));

    let source = r#"
        state SMS {
            direct: u8 at 0xc000;
            pointer: u16 at 0xc100, 0x20, -0x8;
        }

        whileAttached {
            let value: u16 = sms.read(0xc200) else 0
            print(value)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("SMS direct reads and guest pointer paths should type-check");
    assert_eq!(
        checked.semantics().state_provider(),
        Some(StdlibStateProviderId::Sms)
    );
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("SMS direct reads and guest pointer paths should emit valid Wasm");

    let provider_source = "state SMS {}";
    let mut database = splitscript::tooling::database::CompilerDatabase::new(provider_source);
    let hover = database
        .hover(provider_source.find("SMS").unwrap() + 1)
        .unwrap()
        .expect("the SMS provider should expose catalog documentation");
    assert!(hover.markdown.contains("state SMS { ... }"));
    assert!(hover.markdown.contains("sms: SMSEmulator"));
}

#[test]
fn genesis_provider_normalizes_word_swapped_unaligned_reads_and_guest_pointer_paths() {
    let library = StandardLibrary::new();
    let genesis = library
        .state_provider_by_name("Genesis")
        .expect("the bundled Genesis provider should be discoverable by source name");
    assert_eq!(genesis.id, StdlibStateProviderId::Genesis);
    assert_eq!(genesis.value_name, "genesis");
    assert_eq!(genesis.process_type, StdlibTypeId::GenesisEmulator);
    assert_eq!(genesis.direct_read, StdlibItemId::GenesisEmulatorRead);
    assert_eq!(
        genesis.attachment,
        splitscript::compiler::stdlib::StateProviderAttachment::Callable(
            StdlibItemId::GenesisEmulatorDiscover,
        )
    );
    assert!(matches!(
        library
            .item(StdlibItemId::GenesisEmulatorDiscover)
            .implementation,
        Implementation::LibraryBody { .. }
    ));
    assert!(matches!(
        genesis.processes,
        splitscript::compiler::stdlib::StateProviderProcesses::Declared(processes)
            if processes.contains(&"Fusion.exe")
                && processes.contains(&"gens.exe")
                && processes.contains(&"retroarch.exe")
    ));

    let source = r#"
        struct Snapshot {
            score: u32,
            velocity: i16,
            samples: [u16; 2],
        }

        state Genesis {
            snapshot: Snapshot at 0x1201;
            pointer: u32 at 0x2000, 0x20, -0x8;
        }

        whileAttached {
            let value: f32 = genesis.read(0x3001) else 0.0
            if value > 0.0 { print("positive") }
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("Genesis direct reads and guest pointer paths should type-check");
    assert_eq!(
        checked.semantics().state_provider(),
        Some(StdlibStateProviderId::Genesis)
    );
    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("Genesis normalized reads and guest pointer paths should emit valid Wasm");
    let operators = wasmparser::Parser::new(0)
        .parse_all(&wasm)
        .filter_map(Result::ok)
        .filter_map(|payload| match payload {
            wasmparser::Payload::CodeSectionEntry(body) => Some(body),
            _ => None,
        })
        .flat_map(|body| body.get_operators_reader().unwrap().into_iter())
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    assert!(
        operators
            .iter()
            .any(|operator| matches!(operator, wasmparser::Operator::I32Store8 { .. }))
    );

    let provider_source = "state Genesis {}";
    let mut database = splitscript::tooling::database::CompilerDatabase::new(provider_source);
    let hover = database
        .hover(provider_source.find("Genesis").unwrap() + 1)
        .unwrap()
        .expect("the Genesis provider should expose catalog documentation");
    assert!(hover.markdown.contains("state Genesis { ... }"));
    assert!(hover.markdown.contains("genesis: GenesisEmulator"));
}

#[test]
fn gcn_provider_decodes_big_endian_structs_arrays_and_guest_pointer_paths() {
    let library = StandardLibrary::new();
    let gcn = library
        .state_provider_by_name("GCN")
        .expect("the bundled GameCube provider should be discoverable by source name");
    assert_eq!(gcn.id, StdlibStateProviderId::Gcn);
    assert_eq!(gcn.value_name, "gcn");
    assert_eq!(gcn.process_type, StdlibTypeId::GCNEmulator);
    assert_eq!(gcn.direct_read, StdlibItemId::GCNEmulatorRead);
    assert_eq!(
        gcn.attachment,
        splitscript::compiler::stdlib::StateProviderAttachment::Callable(
            StdlibItemId::GCNEmulatorDiscover,
        )
    );
    assert!(matches!(
        library
            .item(StdlibItemId::GCNEmulatorDiscover)
            .implementation,
        Implementation::LibraryBody { .. }
    ));
    assert!(matches!(
        gcn.processes,
        splitscript::compiler::stdlib::StateProviderProcesses::Declared(processes)
            if processes.contains(&"Dolphin.exe") && processes.contains(&"retroarch.exe")
    ));

    let source = r#"
        struct Snapshot {
            health: u16,
            velocity: i32,
            samples: [u16; 2],
        }

        state GCN {
            snapshot: Snapshot at 0x80001000;
            pointer: u32 at 0x80002000, 0x20, -0x8;
        }

        whileAttached {
            let value: f32 = gcn.read(0x80003000) else 0.0
            if value > 0.0 { print("positive") }
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("GameCube big-endian values and guest pointer paths should type-check");
    assert_eq!(
        checked.semantics().state_provider(),
        Some(StdlibStateProviderId::Gcn)
    );
    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("GameCube big-endian values and guest pointer paths should emit valid Wasm");
    let operators = wasmparser::Parser::new(0)
        .parse_all(&wasm)
        .filter_map(Result::ok)
        .filter_map(|payload| match payload {
            wasmparser::Payload::CodeSectionEntry(body) => Some(body),
            _ => None,
        })
        .flat_map(|body| body.get_operators_reader().unwrap().into_iter())
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    assert!(
        operators
            .iter()
            .filter(|operator| matches!(operator, wasmparser::Operator::I32Load8U { .. }))
            .count()
            >= 12
    );
    assert!(
        operators
            .iter()
            .any(|operator| matches!(operator, wasmparser::Operator::F32ReinterpretI32))
    );

    let provider_source = "state GCN {}";
    let mut database = splitscript::tooling::database::CompilerDatabase::new(provider_source);
    let hover = database
        .hover(provider_source.find("GCN").unwrap() + 1)
        .unwrap()
        .expect("the GameCube provider should expose catalog documentation");
    assert!(hover.markdown.contains("state GCN { ... }"));
    assert!(hover.markdown.contains("gcn: GCNEmulator"));
}

#[test]
fn wii_provider_covers_mem1_mem2_and_big_endian_guest_pointer_paths() {
    let library = StandardLibrary::new();
    let wii = library
        .state_provider_by_name("Wii")
        .expect("the bundled Wii provider should be discoverable by source name");
    assert_eq!(wii.id, StdlibStateProviderId::Wii);
    assert_eq!(wii.value_name, "wii");
    assert_eq!(wii.process_type, StdlibTypeId::WiiEmulator);
    assert_eq!(wii.direct_read, StdlibItemId::WiiEmulatorRead);
    assert_eq!(
        wii.attachment,
        splitscript::compiler::stdlib::StateProviderAttachment::Callable(
            StdlibItemId::WiiEmulatorDiscover,
        )
    );
    assert!(matches!(
        library
            .item(StdlibItemId::WiiEmulatorDiscover)
            .implementation,
        Implementation::LibraryBody { .. }
    ));

    let source = r#"
        struct Player {
            health: u16,
            position: [f32; 3],
        }

        state Wii {
            mem1: Player at 0x80001000;
            mem2: u32 at 0x90002000;
            pointer: u16 at 0x90003000, 0x20, -0x8;
        }

        whileAttached {
            let value: i32 = wii.read(0x80004000) else 0
            if value > 0 { print("positive") }
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("Wii MEM1, MEM2, and guest pointer paths should type-check");
    assert_eq!(
        checked.semantics().state_provider(),
        Some(StdlibStateProviderId::Wii)
    );
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("Wii MEM1, MEM2, and guest pointer paths should emit valid Wasm");

    let provider_source = "state Wii {}";
    let mut database = splitscript::tooling::database::CompilerDatabase::new(provider_source);
    let hover = database
        .hover(provider_source.find("Wii").unwrap() + 1)
        .unwrap()
        .expect("the Wii provider should expose catalog documentation");
    assert!(hover.markdown.contains("state Wii { ... }"));
    assert!(hover.markdown.contains("wii: WiiEmulator"));
}

#[test]
fn native_processes_are_typed_provider_values_and_methods_use_the_receiver() {
    let source = r#"
        state "game.exe" {}

        fn keepProcess(value: Process) -> Process {
            return value
        }

        whileAttached {
            let attached = keepProcess(process)
            let name = attached.name()
            let value: u32! = attached.read<u32>(0x1000)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let attached = checked
        .syntax()
        .actions
        .iter()
        .flat_map(|action| &action.body.statements)
        .find_map(|statement| match statement {
            splitscript::compiler::ast::Stmt::Variable(variable) if variable.name == "attached" => {
                Some(variable.id)
            }
            _ => None,
        })
        .expect("the attached-process binding should exist");
    let ty = checked
        .semantics()
        .value_type(attached)
        .expect("the provider value should flow through ordinary inference");
    assert_eq!(
        checked.semantics().types().kind(ty),
        &TypeKind::Standard(StdlibTypeId::Process)
    );
    let name = checked
        .syntax()
        .actions
        .iter()
        .flat_map(|action| &action.body.statements)
        .find_map(|statement| match statement {
            splitscript::compiler::ast::Stmt::Variable(variable) if variable.name == "name" => {
                Some(variable.id)
            }
            _ => None,
        })
        .expect("the attached-process name binding should exist");
    assert_eq!(
        checked.semantics().types().kind(
            checked
                .semantics()
                .value_type(name)
                .expect("process.name should have a semantic type")
        ),
        &TypeKind::Standard(StdlibTypeId::String)
    );
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("captured Process receivers should lower to valid Wasm");
}

#[test]
fn abi_catalog_drives_wasm_imports_and_the_internal_reference() {
    let catalog = AbiCatalog::new();
    assert_eq!(catalog.validate(), Vec::<String>::new());
    assert_eq!(
        catalog.render_signature(AbiImportId::ProcessRead),
        "(i64, i64, i32, i32) -> i32"
    );
    let process_read = catalog.import(AbiImportId::ProcessRead);
    assert_eq!(
        process_read.parameters[0].ownership,
        AbiOwnership::BorrowedHandle
    );
    assert_eq!(
        process_read.parameters[2].ownership,
        AbiOwnership::OutputMemory
    );
    assert!(process_read.effects.contains(&AbiEffect::ReadsProcess));
    assert_eq!(
        catalog.import(AbiImportId::ProcessAttach).results[0].ownership,
        AbiOwnership::OwnedHandle
    );
    assert_eq!(
        catalog.import(AbiImportId::ProcessListByName).parameters[3].ownership,
        AbiOwnership::InputOutputMemory
    );

    let wasm = splitscript::compile("state \"game.exe\" {}").unwrap();
    let mut emitted = Vec::new();
    for payload in Parser::new(0).parse_all(&wasm) {
        if let Payload::ImportSection(section) = payload.unwrap() {
            for import in section.into_imports() {
                let import = import.unwrap();
                assert!(matches!(import.ty, TypeRef::Func(_)));
                emitted.push((import.module.to_owned(), import.name.to_owned()));
            }
        }
    }
    let required = [
        AbiImportId::RuntimeSetTickRate,
        AbiImportId::ProcessAttach,
        AbiImportId::ProcessDetach,
        AbiImportId::ProcessIsOpen,
    ];
    let declared = catalog
        .imports()
        .filter(|import| required.contains(&import.id))
        .map(|import| (import.module.to_owned(), import.name.to_owned()))
        .collect::<Vec<_>>();
    assert_eq!(emitted, declared);

    let selected =
        splitscript::compile(r#"state "game.exe" {} selectProcess { return true }"#).unwrap();
    let selected_imports = Parser::new(0)
        .parse_all(&selected)
        .filter_map(|payload| match payload.unwrap() {
            Payload::ImportSection(section) => Some(
                section
                    .into_imports()
                    .map(|import| import.unwrap().name.to_owned())
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .flatten()
        .collect::<Vec<_>>();
    assert!(
        selected_imports
            .iter()
            .any(|name| name == "process_list_by_name")
    );
    assert!(
        selected_imports
            .iter()
            .any(|name| name == "process_attach_by_pid")
    );
    assert!(!selected_imports.iter().any(|name| name == "process_attach"));

    let abi_document = include_str!("../../docs/ABI.md");
    let table_start = abi_document
        .find("| Import | WebAssembly type |")
        .expect("ABI reference should contain its generated import table");
    let table_end = abi_document[table_start..]
        .find("\n\n")
        .map_or(abi_document.len(), |offset| table_start + offset);
    assert_eq!(
        abi_document[table_start..table_end].trim_end(),
        catalog.render_import_table().trim_end(),
        "docs/ABI.md import table must remain a verified catalog view"
    );
}

#[test]
fn settings_host_imports_are_filtered_by_setting_kind() {
    let wasm = splitscript::compile(
        r#"
            state "game.exe" {}
            settings {
                "Enabled" => enabled: true
            }
        "#,
    )
    .expect("Boolean settings should compile");
    let mut imports = Vec::new();
    for payload in Parser::new(0).parse_all(&wasm) {
        if let Payload::ImportSection(section) = payload.unwrap() {
            imports.extend(
                section
                    .into_imports()
                    .map(|import| import.unwrap().name.to_owned()),
            );
        }
    }

    assert!(imports.iter().any(|name| name == "user_settings_add_bool"));
    assert!(imports.iter().any(|name| name == "setting_value_get_bool"));
    assert!(
        !imports
            .iter()
            .any(|name| name == "setting_value_get_string")
    );
    assert!(
        !imports
            .iter()
            .any(|name| name == "user_settings_add_choice")
    );
    assert!(
        !imports
            .iter()
            .any(|name| name == "user_settings_add_file_select")
    );
    assert!(
        !imports
            .iter()
            .any(|name| name == "user_settings_add_text_input")
    );
}

#[test]
fn text_settings_import_only_the_text_registration_and_string_reader() {
    let wasm = splitscript::compile(
        r#"
            state "game.exe" {}
            settings {
                "Profile" => profile: "Runner"
            }
            whileAttached { print(settings.profile) }
        "#,
    )
    .expect("text settings should compile");
    let mut imports = Vec::new();
    for payload in Parser::new(0).parse_all(&wasm) {
        if let Payload::ImportSection(section) = payload.unwrap() {
            imports.extend(
                section
                    .into_imports()
                    .map(|import| import.unwrap().name.to_owned()),
            );
        }
    }

    assert!(
        imports
            .iter()
            .any(|name| name == "user_settings_add_text_input")
    );
    assert!(
        imports
            .iter()
            .any(|name| name == "setting_value_get_string")
    );
    assert!(!imports.iter().any(|name| name == "user_settings_add_bool"));
    assert!(
        !imports
            .iter()
            .any(|name| name == "user_settings_add_file_select")
    );
    assert!(
        !imports
            .iter()
            .any(|name| name == "user_settings_add_choice")
    );
}

#[derive(Default)]
pub(super) struct TypedExpressionCounter(pub usize);

impl splitscript::compiler::hir::TypedVisitor for TypedExpressionCounter {
    fn visit_expression(
        &mut self,
        expression: &splitscript::compiler::hir::TypedExpression,
        program: &splitscript::compiler::hir::TypedProgram,
    ) {
        self.0 += 1;
        splitscript::compiler::hir::walk_typed_expression(self, expression, program);
    }
}

#[test]
fn standard_library_catalog_is_valid_documented_and_compilable() {
    let library = StandardLibrary::new();
    assert_eq!(library.validate(), Vec::<String>::new());
    assert!(library.core_type_has_capability(
        splitscript::compiler::stdlib::CoreTypeId::I32,
        splitscript::compiler::stdlib::StdlibCapabilityId::MemoryReadable,
    ));
    assert!(library.core_type_has_capability(
        splitscript::compiler::stdlib::CoreTypeId::F64,
        splitscript::compiler::stdlib::StdlibCapabilityId::Float,
    ));
    assert!(library.core_type_has_capability(
        splitscript::compiler::stdlib::CoreTypeId::F64,
        splitscript::compiler::stdlib::StdlibCapabilityId::Display,
    ));
    assert!(library.core_type_has_capability(
        splitscript::compiler::stdlib::CoreTypeId::Char,
        splitscript::compiler::stdlib::StdlibCapabilityId::Equatable,
    ));
    assert!(library.core_type_has_capability(
        splitscript::compiler::stdlib::CoreTypeId::Char,
        splitscript::compiler::stdlib::StdlibCapabilityId::Display,
    ));
    assert!(!library.core_type_has_capability(
        splitscript::compiler::stdlib::CoreTypeId::Char,
        splitscript::compiler::stdlib::StdlibCapabilityId::Integer,
    ));
    assert!(!library.core_type_has_capability(
        splitscript::compiler::stdlib::CoreTypeId::Char,
        splitscript::compiler::stdlib::StdlibCapabilityId::MemoryReadable,
    ));
    assert!(library.type_has_capability(
        splitscript::compiler::stdlib::StdlibTypeId::String,
        splitscript::compiler::stdlib::StdlibCapabilityId::Display,
    ));
    assert_eq!(
        library.item_by_name("Numeric.clamp").map(|item| item.id),
        Some(StdlibItemId::NumericClamp)
    );
    assert_eq!(
        library.render_signature(StdlibItemId::NumericClamp),
        "Self.clamp(minimum: Self, maximum: Self) -> Self where Self: Numeric"
    );
    assert_eq!(
        library.item_by_name("setVariable").map(|item| item.id),
        Some(StdlibItemId::SetVariable)
    );
    assert_eq!(
        library.render_signature(StdlibItemId::SetTickRate),
        "setTickRate<T>(hz: T) -> None where T: Numeric"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::FutureRace),
        "future.race<T>(operations: [async T]) -> async T"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::FutureTimeout),
        "future.timeout<T>(operation: async T, duration: Duration) -> async T!"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::ProcessScanOnce),
        "Process.scanOnce(address: address, size: u64, signature: Signature) -> async address?"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::DurationFromSeconds),
        "Duration.fromSeconds<T>(seconds: T) -> Duration where T: Numeric"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::DurationFromFrames),
        "Duration.fromFrames<T>(frames: T, framesPerSecond: T) -> Duration where T: Integer"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringContains),
        "String.contains(substring: String) -> bool"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringIndexOf),
        "String.indexOf(substring: String) -> u32?"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringLastIndexOf),
        "String.lastIndexOf(substring: String) -> u32?"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringPadStart),
        "String.padStart(width: u32, fill: char) -> String"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringPadEnd),
        "String.padEnd(width: u32, fill: char) -> String"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringStartsWith),
        "String.startsWith(prefix: String) -> bool"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringEndsWith),
        "String.endsWith(suffix: String) -> bool"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringEqualsIgnoreAsciiCase),
        "String.equalsIgnoreAsciiCase(other: String) -> bool"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringToAsciiLowerCase),
        "String.toAsciiLowerCase() -> String"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringIsEmpty),
        "String.isEmpty() -> bool"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringToAsciiUpperCase),
        "String.toAsciiUpperCase() -> String"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringTrimAsciiWhitespace),
        "String.trimAsciiWhitespace() -> String"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringIsBlank),
        "String.isBlank() -> bool"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringJoin),
        "String.join(values: [String], separator: String) -> String"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringReplaceAll),
        "String.replaceAll(search: String, replacement: String) -> String!"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringSplit),
        "String.split(delimiter: String) -> [String]!"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringParse),
        "String.parse<T>() -> T! where T: Numeric"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringByteAt),
        "String.byteAt(byteIndex: u32) -> u8!"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringCharAt),
        "String.charAt(byteIndex: u32) -> char!"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::StringSlice),
        "String.slice(start: u32, end: u32) -> String!"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::SignedAbs),
        "Self.abs() -> Self where Self: Signed"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::NumericSquared),
        "Self.squared() -> Self where Self: Numeric"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::IntegerToString),
        "Self.toString(radix: u32) -> String! where Self: Integer"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::FloatSqrt),
        "Self.sqrt() -> Self where Self: Float"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::FloatTruncate),
        "Self.truncate() -> Self where Self: Float"
    );
    assert!(matches!(
        library.item(StdlibItemId::FloatRound).implementation,
        Implementation::Intrinsic { .. }
    ));
    assert_eq!(
        library.render_signature(StdlibItemId::FloatRoundTo),
        "Self.roundTo(decimalPlaces: u32) -> Self where Self: Float"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::FloatIsFinite),
        "Self.isFinite() -> bool where Self: Float"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::F32NaN),
        "f32.NaN: f32"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::F32PositiveInfinity),
        "f32.positiveInfinity: f32"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::F64NegativeInfinity),
        "f64.negativeInfinity: f64"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::F32FromBits),
        "f32.fromBits(bits: u32) -> f32"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::F32ToBits),
        "f32.toBits() -> u32"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::F64FromBits),
        "f64.fromBits(bits: u64) -> f64"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::F64ToBits),
        "f64.toBits() -> u64"
    );
    for removed in [
        "timer.setVariable",
        "runtime.setTickRate",
        "Duration.saturatingSecondsF32",
    ] {
        assert!(
            library.item_by_name(removed).is_none(),
            "prototype alias `{removed}` must not remain in the catalog"
        );
    }

    let mut broken_examples = Vec::new();
    let summarize_errors = |errors: Vec<splitscript::Diagnostic>| {
        errors
            .into_iter()
            .map(|error| error.message)
            .collect::<Vec<_>>()
            .join("; ")
    };
    for item in library.items() {
        assert!(!item.documentation.summary.is_empty());
        for example in item.documentation.examples {
            if let Err(errors) = splitscript::compile(&example.validation_program()) {
                broken_examples.push(format!(
                    "standard-library example `{}: {}` failed: {}",
                    item.qualified_name,
                    example.title,
                    summarize_errors(errors)
                ));
            }
        }
    }
    for provider in library.state_providers() {
        assert!(!provider.documentation.summary.is_empty());
        for example in provider.documentation.examples {
            if let Err(errors) = splitscript::compile(&example.validation_program()) {
                broken_examples.push(format!(
                    "state-provider example `{}: {}` failed: {}",
                    provider.name,
                    example.title,
                    summarize_errors(errors)
                ));
            }
        }
    }
    let declaration_documentation = library
        .namespaces()
        .iter()
        .map(|value| (value.name, value.documentation))
        .chain(
            library
                .capabilities()
                .iter()
                .map(|value| (value.name, value.documentation)),
        )
        .chain(
            library
                .public_type_constructors()
                .map(|value| (value.name, value.documentation)),
        )
        .chain(
            library
                .types()
                .map(|value| (value.name, value.documentation)),
        )
        .chain(
            library
                .fields()
                .iter()
                .filter(|field| match field.owner {
                    StdlibOwner::Type(owner) => {
                        library.type_decl(owner).visibility == TypeVisibility::Public
                    }
                    _ => true,
                })
                .map(|value| (value.name, value.documentation)),
        )
        .chain(
            library
                .public_variants()
                .map(|value| (value.name, value.documentation)),
        );
    let mut checked_declaration_examples = 0;
    for (name, documentation) in declaration_documentation {
        for example in documentation.examples {
            checked_declaration_examples += 1;
            if let Err(errors) = splitscript::compile(&example.validation_program()) {
                broken_examples.push(format!(
                    "standard-library declaration example `{name}: {}` failed: {}",
                    example.title,
                    summarize_errors(errors)
                ));
            }
        }
    }
    assert!(
        broken_examples.is_empty(),
        "{}",
        broken_examples.join("\n\n")
    );
    assert!(
        checked_declaration_examples >= 7,
        "representative non-callable declarations should retain checked examples"
    );
    let missing_declaration_examples = library
        .namespaces()
        .iter()
        .map(|value| ("namespace", value.name, value.documentation))
        .chain(
            library
                .capabilities()
                .iter()
                .map(|value| ("capability", value.name, value.documentation)),
        )
        .chain(
            library
                .public_type_constructors()
                .map(|value| ("type constructor", value.name, value.documentation)),
        )
        .chain(
            library
                .types()
                .map(|value| ("type", value.name, value.documentation)),
        )
        .chain(
            library
                .fields()
                .iter()
                .filter(|value| {
                    value.visibility == FieldVisibility::Public
                        && match value.owner {
                            StdlibOwner::Type(owner) => {
                                library.type_decl(owner).visibility == TypeVisibility::Public
                            }
                            _ => true,
                        }
                })
                .map(|value| ("field", value.name, value.documentation)),
        )
        .chain(
            library
                .public_variants()
                .map(|value| ("variant", value.name, value.documentation)),
        )
        .filter(|(_, _, documentation)| documentation.examples.is_empty())
        .map(|(kind, name, _)| format!("{kind} {name}"))
        .collect::<Vec<_>>();
    assert_eq!(missing_declaration_examples, Vec::<String>::new());
}

#[test]
fn language_catalog_is_valid_documented_and_compilable() {
    let language = LanguageCatalog::new();
    assert_eq!(language.validate(), Vec::<String>::new());
    let retry = language
        .item_by_name("retry")
        .expect("retry should be a catalog-backed language construct");
    assert_eq!(retry.id, LanguageItemId::Retry);
    assert_eq!(retry.kind, LanguageItemKind::Keyword);
    assert_eq!(
        retry.form,
        "let value = retry fallibleExpression | let value = retry { ... }"
    );
    assert!(retry.documentation.details.contains("T!"));
    assert_eq!(
        language
            .builtin_type(splitscript::compiler::types::BuiltinType::I32)
            .map(|item| item.id),
        Some(LanguageItemId::BuiltinType(
            splitscript::compiler::types::BuiltinType::I32
        ))
    );
    assert_eq!(
        language
            .item_for_source_token("Address")
            .map(|item| item.id),
        Some(LanguageItemId::BuiltinType(
            splitscript::compiler::types::BuiltinType::Address
        ))
    );
    assert_eq!(
        language.item_for_source_token("[").map(|item| item.id),
        Some(LanguageItemId::ArrayType)
    );
    for contextual in ["at", "key", "choice", "default", "file", "mime", "in"] {
        assert!(
            language.item_for_source_token(contextual).is_none(),
            "`{contextual}` needs its grammatical context"
        );
    }
    let fixed_array = language.item(LanguageItemId::ArrayType);
    assert!(fixed_array.documentation.details.contains("4,096 elements"));
    assert!(fixed_array.documentation.details.contains("65,536 bytes"));
    assert!(
        fixed_array
            .documentation
            .details
            .contains("expression-valued [`state`] field")
    );
    let state_path = language.item(LanguageItemId::StatePointerField);
    assert!(state_path.documentation.details.contains("4,096 elements"));
    assert!(
        state_path
            .documentation
            .details
            .contains("growable [`[T]`]")
    );
    assert!(language.item_for_source_token("Array").is_none());
    assert_eq!(
        StandardLibrary::new()
            .field(StdlibFieldId::ModuleAddress)
            .name,
        "address"
    );
    assert_eq!(
        StandardLibrary::new()
            .variant(StdlibVariantId::TimerStateRunning)
            .name,
        "Running"
    );

    for action in [
        splitscript::compiler::ast::ActionKind::Setup,
        splitscript::compiler::ast::ActionKind::SelectProcess,
        splitscript::compiler::ast::ActionKind::OnDetach,
        splitscript::compiler::ast::ActionKind::OnAttach,
        splitscript::compiler::ast::ActionKind::OnStateReady,
        splitscript::compiler::ast::ActionKind::OnStart,
        splitscript::compiler::ast::ActionKind::OnReset,
        splitscript::compiler::ast::ActionKind::WhileAttached,
        splitscript::compiler::ast::ActionKind::Start,
        splitscript::compiler::ast::ActionKind::Split,
        splitscript::compiler::ast::ActionKind::Reset,
        splitscript::compiler::ast::ActionKind::IsLoading,
        splitscript::compiler::ast::ActionKind::GameTime,
    ] {
        let item = language.action(action);
        assert_eq!(item.name, action.name());
        assert_eq!(item.kind, LanguageItemKind::Action(action));
    }

    for item in language.items() {
        assert!(!item.documentation.summary.is_empty());
        for example in item.documentation.examples {
            let validation = example.validation_program();
            if example.is_complete_program() {
                assert_eq!(
                    validation, example.source,
                    "complete language examples must validate the visible source itself"
                );
            }
            let formatted = splitscript::format_source(&validation).unwrap_or_else(|errors| {
                panic!(
                    "language example `{}: {}` did not parse and format: {errors:#?}",
                    item.name, example.title
                )
            });
            splitscript::compile(&formatted).unwrap_or_else(|errors| {
                panic!(
                    "language example `{}: {}` failed: {errors:#?}",
                    item.name, example.title
                )
            });
        }
    }
}

#[test]
fn compiler_database_resolves_language_catalog_syntax() {
    use splitscript::{
        compiler::types::BuiltinType,
        tooling::database::{CompilerDatabase, DefinitionTarget, SourceDefinitionId},
        tooling::language::LanguageItemId,
    };

    let source = r#"
        enum Mode {
            A,
            B
        }

        settings {
            /// Select the active mode.
            "Mode" => selected key "selected-mode": choice {
                "First" => Mode.A default,
                "Second" => Mode.B
            },
            /// Select an input file.
            "Input" => input: file {
                mime => "application/octet-stream"
            }
        }

        state "game.exe" {
            level: i32 at 0x1000;
            mapName at 0x2000 as utf8(32);
            chapterName at 0x3000 as utf16le(64)
        }

        fn maybe(value: i32) -> i32? {
            return Some(value)
        }

        fn preserveBytes(value: [u8]) -> [u8] {
            return value
        }

        fn fallible() -> i32! {
            return Err("unavailable")
        }

        fn propagated() -> i32! {
            return fallible()?
        }

        setup {}

        onAttach {
            let module = await process.module("GameAssembly.dll")
            print(module.address as String)
        }

        onStateReady {
            print(current.level)
        }

        whileAttached {
            let firstByte = preserveBytes([1u8])[0]
            for byte in [firstByte] {
                print(byte)
            }
            let timerState = TimerState.Running
            let levelChanged = current.level != old.level
            let settingChanged = settings.selected != oldSettings.selected
            let value = match maybe(1) {
                Some(value) => value,
                None => 0
            }
            print(`value {value}`)
        }
    "#;
    let mut database = CompilerDatabase::new(source);
    let checked = database.check().unwrap();
    let mode = checked.syntax().enums[0].id;
    let variant = checked.syntax().enums[0].variants[0].id;

    let i32_type = source.find("value: i32").unwrap() + "value: ".len();
    assert_eq!(
        database.definition_at(i32_type).unwrap(),
        Some(DefinitionTarget::Language(LanguageItemId::BuiltinType(
            BuiltinType::I32
        )))
    );
    let option_type = source.find("i32?").unwrap() + "i32".len();
    assert_eq!(
        database.definition_at(option_type).unwrap(),
        Some(DefinitionTarget::Language(LanguageItemId::OptionType))
    );
    let result_type = source.find("i32!").unwrap() + "i32".len();
    assert_eq!(
        database.definition_at(result_type).unwrap(),
        Some(DefinitionTarget::Language(LanguageItemId::ResultType))
    );
    let array_type = source.find("[u8]").unwrap();
    assert_eq!(
        database.definition_at(array_type).unwrap(),
        Some(DefinitionTarget::Language(LanguageItemId::ArrayType))
    );
    let array_index = source.find("[0]").unwrap();
    assert_eq!(
        database.definition_at(array_index).unwrap(),
        Some(DefinitionTarget::Language(LanguageItemId::ArrayIndex))
    );
    let propagation = source.find("fallible()?").unwrap() + "fallible()".len();
    assert_eq!(
        database.definition_at(propagation).unwrap(),
        Some(DefinitionTarget::Language(LanguageItemId::Propagate))
    );
    let utf8_decoder = source.find("utf8(32)").unwrap();
    assert_eq!(
        database.definition_at(utf8_decoder).unwrap(),
        Some(DefinitionTarget::Language(
            LanguageItemId::NativeStringDecoder
        ))
    );
    let decoder_hover = database
        .hover(utf8_decoder)
        .unwrap()
        .expect("the state decoder should have language documentation");
    assert!(decoder_hover.markdown.contains("bounded native UTF-8"));
    assert!(decoder_hover.markdown.contains("not a string-size type"));
    let utf16_decoder = source.find("utf16le(64)").unwrap();
    assert_eq!(
        database.definition_at(utf16_decoder).unwrap(),
        Some(DefinitionTarget::Language(
            LanguageItemId::NativeUtf16LeDecoder
        ))
    );
    let decoder_hover = database
        .hover(utf16_decoder)
        .unwrap()
        .expect("the UTF-16LE state decoder should have language documentation");
    assert!(decoder_hover.markdown.contains("bounded native UTF-16LE"));
    assert!(decoder_hover.markdown.contains("replacement character"));

    let module_field = source.find("module.address").unwrap() + "module.".len();
    assert_eq!(
        database.definition_at(module_field).unwrap(),
        Some(DefinitionTarget::StandardLibrarySymbol(
            splitscript::compiler::stdlib::StdlibSymbolId::Field(StdlibFieldId::ModuleAddress)
        ))
    );
    let module_field_hover = database
        .hover(module_field)
        .unwrap()
        .expect("standard-library field hover");
    assert!(module_field_hover.markdown.contains("**Examples**"));
    assert!(
        module_field_hover
            .markdown
            .contains("let baseAddress = executable.address")
    );
    let timer_state = source.find("TimerState.Running").unwrap();
    assert_eq!(
        database.definition_at(timer_state).unwrap(),
        Some(DefinitionTarget::StandardLibrarySymbol(
            splitscript::compiler::stdlib::StdlibSymbolId::Type(StdlibTypeId::TimerState)
        ))
    );
    assert_eq!(
        database
            .definition_at(timer_state + "TimerState.".len())
            .unwrap(),
        Some(DefinitionTarget::StandardLibrarySymbol(
            splitscript::compiler::stdlib::StdlibSymbolId::Variant(
                StdlibVariantId::TimerStateRunning
            )
        ))
    );
    let running_variant_hover = database
        .hover(timer_state + "TimerState.".len())
        .unwrap()
        .expect("standard-library variant hover");
    assert!(
        running_variant_hover
            .markdown
            .contains("timer.state() == TimerState.Running")
    );

    for (root, expected) in [
        ("current.level", SourceDefinitionId::State),
        ("old.level", SourceDefinitionId::State),
        ("settings.selected", SourceDefinitionId::Settings),
        ("oldSettings.selected", SourceDefinitionId::Settings),
    ] {
        let offset = source.find(root).unwrap();
        assert!(
            matches!(
                database.definition_at(offset).unwrap(),
                Some(DefinitionTarget::Source(definition)) if definition.id == expected
            ),
            "wrong snapshot declaration target for `{root}`"
        );
    }
    for (root, expected) in [
        ("current.level", "current / old: state snapshot"),
        ("settings.selected", "settings / oldSettings: settings view"),
    ] {
        let hover = database
            .hover(source.find(root).unwrap())
            .unwrap()
            .expect("snapshot roots should have source-aware documentation");
        assert!(hover.markdown.contains(expected));
    }

    for (spelling, expected) in [
        ("Some(value)", LanguageItemId::SomeConstructor),
        ("Err(\"unavailable\")", LanguageItemId::ErrorConstructor),
        ("None =>", LanguageItemId::BuiltinType(BuiltinType::None)),
        ("choice {", LanguageItemId::ChoiceSetting),
        ("default", LanguageItemId::ChoiceSetting),
        ("file {", LanguageItemId::FileSetting),
        ("mime =>", LanguageItemId::FileSetting),
        ("at 0x1000", LanguageItemId::StatePointerField),
        ("key \"selected-mode\"", LanguageItemId::StableSettingKey),
        ("in [firstByte]", LanguageItemId::For),
        ("setup", LanguageItemId::Setup),
        ("onStateReady", LanguageItemId::OnStateReady),
        ("whileAttached", LanguageItemId::WhileAttached),
    ] {
        let offset = source.find(spelling).unwrap();
        assert_eq!(
            database.definition_at(offset).unwrap(),
            Some(DefinitionTarget::Language(expected)),
            "wrong catalog target for `{spelling}`"
        );
        let hover = database
            .hover(offset)
            .unwrap()
            .unwrap_or_else(|| panic!("missing catalog hover for `{spelling}`"));
        assert!(hover.markdown.contains("```splitscript"));
        assert!(hover.markdown.contains("**Examples**"));
    }

    let doc_comment = source.find("/// Select").unwrap();
    assert_eq!(
        database.definition_at(doc_comment).unwrap(),
        Some(DefinitionTarget::Language(
            LanguageItemId::DocumentationComment
        ))
    );
    let doc_hover = database
        .hover(doc_comment)
        .unwrap()
        .expect("documentation comments should have language hover");
    assert!(
        doc_hover
            .markdown
            .contains("source declaration, state field")
    );
    assert!(doc_hover.markdown.contains("functions and methods"));
    assert!(doc_hover.markdown.contains("Document a source symbol"));
    assert!(doc_hover.markdown.contains("Add a setting tooltip"));
    let template = source.find('`').unwrap();
    assert_eq!(
        database.definition_at(template).unwrap(),
        Some(DefinitionTarget::Language(LanguageItemId::TemplateString))
    );

    let choice = source.find("Mode.A default").unwrap();
    assert!(matches!(
        database.definition_at(choice).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Enum(mode)
    ));
    assert!(matches!(
        database.definition_at(choice + "Mode.".len()).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::EnumVariant(variant)
    ));
}

#[test]
fn managed_string_policy_is_documented_at_its_keyword() {
    use splitscript::tooling::{
        database::{CompilerDatabase, DefinitionTarget},
        language::LanguageItemId,
    };

    let source = r#"
        image "Assembly-CSharp" {
            class GameManager {
                String scene maxLength 64;
            }
        }
        state Unity ["game.exe"] {}
    "#;
    let mut database = CompilerDatabase::new(source);
    let offset = source.find("maxLength").unwrap();
    assert_eq!(
        database.definition_at(offset).unwrap(),
        Some(DefinitionTarget::Language(
            LanguageItemId::ManagedStringMaxLength
        ))
    );
    let hover = database.hover(offset).unwrap().expect("maxLength hover");
    assert!(hover.markdown.contains("UTF-16"), "{}", hover.markdown);
}

#[test]
fn checked_program_exposes_resolved_standard_library_calls_without_codegen() {
    let source = r#"
        state "game.exe" {}
        whileAttached {
            let value: i32 = 10
            let bounded = value.clamp(0, 7)
        }
    "#;
    let parsed = splitscript::parse(source).expect("source should parse");
    let checked = splitscript::check(parsed).expect("source should type-check");
    let calls = checked.semantics().calls().collect::<Vec<_>>();
    assert_eq!(calls.len(), 1);
    let ResolvedCall::StandardLibrary {
        item,
        type_arguments,
        ..
    } = calls[0].1
    else {
        panic!("the call should resolve to the standard library");
    };
    assert_eq!(*item, StdlibItemId::NumericClamp);
    assert_eq!(type_arguments.len(), 1);
    assert_eq!(
        checked.semantics().types().kind(type_arguments[0]),
        &TypeKind::Builtin(BuiltinType::I32)
    );

    let action = checked
        .syntax()
        .actions
        .first()
        .expect("whileAttached action");
    let splitscript::compiler::ast::Stmt::Variable(bounded) = &action.body.statements[1] else {
        panic!("the second statement should declare the bounded value");
    };
    assert_eq!(calls[0].0, bounded.value.as_ref().unwrap().id);
    let result_type = checked
        .semantics()
        .expression_type(bounded.value.as_ref().unwrap().id)
        .expect("every checked expression has a semantic type");
    assert_eq!(
        checked.semantics().types().kind(result_type),
        &TypeKind::Builtin(BuiltinType::I32)
    );
}

#[test]
fn expression_ids_distinguish_nested_nodes_and_expose_their_types() {
    let source = r#"
        state "game.exe" {}
        whileAttached {
            let sum: i32 = 1 + 2
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let splitscript::compiler::ast::Stmt::Variable(sum) =
        &checked.syntax().actions[0].body.statements[0]
    else {
        panic!("expected the sum variable");
    };
    let splitscript::compiler::ast::ExprKind::Binary { left, right, .. } =
        &sum.value.as_ref().unwrap().kind
    else {
        panic!("expected a binary expression");
    };
    assert_ne!(sum.value.as_ref().unwrap().id, left.id);
    assert_ne!(sum.value.as_ref().unwrap().id, right.id);
    assert_ne!(left.id, right.id);
    for expression in [sum.value.as_ref().unwrap(), left.as_ref(), right.as_ref()] {
        let ty = checked
            .semantics()
            .expression_type(expression.id)
            .expect("the expression should have a semantic type");
        assert_eq!(
            checked.semantics().types().kind(ty),
            &TypeKind::Builtin(BuiltinType::I32)
        );
    }
}

#[test]
fn inferred_declaration_types_are_semantic_and_syntax_annotations_stay_optional() {
    let source = r#"
        let seed = 7

        state "game.exe" {
            score = seed
        }

        fn identity(value) {
            return value
        }

        onAttach {
            let module = await process.module("GameAssembly.dll")
            let copy = identity(seed)
            let address = module.address
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let syntax = checked.syntax();
    let semantics = checked.semantics();

    let assert_builtin = |value, expected| {
        let ty = semantics
            .value_type(value)
            .expect("every value declaration should have a semantic type");
        assert_eq!(semantics.types().kind(ty), &TypeKind::Builtin(expected));
    };
    let assert_standard = |value, expected| {
        let ty = semantics
            .value_type(value)
            .expect("every value declaration should have a semantic type");
        assert_eq!(semantics.types().kind(ty), &TypeKind::Standard(expected));
    };

    let global = &syntax.globals[0];
    assert_eq!(global.annotation, None);
    assert_builtin(global.id, BuiltinType::I32);

    let state_field = &syntax.state.as_ref().unwrap().fields[0];
    assert_eq!(state_field.annotation, None);
    assert_builtin(state_field.id, BuiltinType::I32);

    let function = &syntax.functions[0];
    assert_eq!(function.params[0].annotation, None);
    assert_eq!(function.return_annotation, None);
    let parameter = semantics
        .value_type(function.params[0].id)
        .expect("inferred function parameters have semantic types");
    assert!(matches!(
        semantics.types().kind(parameter),
        TypeKind::GenericParameter { owner, index: 0 } if *owner == function.id
    ));
    let result = semantics
        .function_result(function.id)
        .expect("every function should have a semantic result type");
    assert_eq!(result, parameter);
    assert_eq!(semantics.function_type_parameters(function.id), [parameter]);

    let statements = &syntax.actions[0].body.statements;
    let splitscript::compiler::ast::Stmt::Variable(module) = &statements[0] else {
        panic!("expected an awaited module binding");
    };
    assert!(matches!(
        module.value.as_ref().unwrap().kind,
        splitscript::compiler::ast::ExprKind::Suspend { .. }
    ));
    assert_eq!(module.annotation, None);
    assert_standard(module.id, StdlibTypeId::Module);

    let splitscript::compiler::ast::Stmt::Variable(copy) = &statements[1] else {
        panic!("expected the inferred function-call binding");
    };
    assert_eq!(copy.annotation, None);
    assert_builtin(copy.id, BuiltinType::I32);

    let splitscript::compiler::ast::Stmt::Variable(address) = &statements[2] else {
        panic!("expected the inferred member-path binding");
    };
    assert_eq!(address.annotation, None);
    assert_builtin(address.id, BuiltinType::Address);

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("semantic declaration types should drive valid Wasm lowering");
}

#[test]
fn parsed_type_references_are_inference_free_syntax() {
    let source = r#"
        state "game.exe" {
            level: u16 at "game.exe", 0x10
        }

        struct Buffer {
            values: [u32]
        }

        fn widen(value: i32) -> u64 {
            return value as u64
        }

        whileAttached {
            let count: i64 = 1i64
        }
    "#;
    let parsed = splitscript::parse(source).unwrap();
    {
        use splitscript::compiler::ast::TypeRef;

        let syntax = parsed.syntax();
        assert_eq!(
            syntax.state.as_ref().unwrap().fields[0].annotation,
            Some(TypeRef::core(CoreTypeId::U16))
        );

        let values = &syntax.structs[0].fields[0];
        let TypeRef::Array(array) = values.ty else {
            panic!("the struct field should retain its parsed array reference");
        };
        assert_eq!(
            syntax
                .array_types
                .iter()
                .find(|declaration| declaration.id == array)
                .unwrap()
                .element,
            TypeRef::core(CoreTypeId::U32)
        );

        let function = &syntax.functions[0];
        assert_eq!(
            function.params[0].annotation,
            Some(TypeRef::core(CoreTypeId::I32))
        );
        assert_eq!(
            function.return_annotation,
            Some(TypeRef::core(CoreTypeId::U64))
        );
        let splitscript::compiler::ast::Stmt::Expression(splitscript::compiler::ast::Expr {
            kind: splitscript::compiler::ast::ExprKind::Return(Some(cast)),
            ..
        }) = &function.body.statements[0]
        else {
            panic!("expected the cast return expression");
        };
        let splitscript::compiler::ast::ExprKind::Cast { target, .. } = &cast.kind else {
            panic!("expected a parsed cast");
        };
        assert_eq!(*target, TypeRef::core(CoreTypeId::U64));

        let splitscript::compiler::ast::Stmt::Variable(count) =
            &syntax.actions[0].body.statements[0]
        else {
            panic!("expected the annotated local");
        };
        assert_eq!(count.annotation, Some(TypeRef::core(CoreTypeId::I64)));
        let splitscript::compiler::ast::ExprKind::Int { suffix, .. } =
            &count.value.as_ref().unwrap().kind
        else {
            panic!("expected the suffixed integer literal");
        };
        assert_eq!(*suffix, Some(TypeRef::core(CoreTypeId::I64)));
    }

    let checked = splitscript::check(parsed).unwrap();
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("syntax type references should adapt to semantic types and valid Wasm");
}

#[test]
fn source_standard_type_names_resolve_after_parsing() {
    use splitscript::compiler::ast::TypeRef;

    let parsed = splitscript::parse(
        r#"
            state "game.exe" {}
            fn base(module: Module) -> address { return module.address }
        "#,
    )
    .unwrap();
    let function = &parsed.syntax().functions[0];
    let Some(TypeRef::Named(name)) = function.params[0].annotation else {
        panic!("source nominal annotations should retain a name identity");
    };
    assert_eq!(parsed.syntax().type_name(name), "Module");

    let checked = splitscript::check(parsed).unwrap();
    let parameter_type = checked
        .semantics()
        .value_type(checked.syntax().functions[0].params[0].id)
        .unwrap();
    assert_eq!(
        parameter_type,
        checked
            .semantics()
            .types()
            .id_for_standard(StdlibTypeId::Module),
        "name resolution, inference, and published semantics must preserve one standard TypeId",
    );
    assert_eq!(
        checked.semantics().types().kind(parameter_type),
        &TypeKind::Standard(StdlibTypeId::Module)
    );
}

#[test]
fn source_struct_and_enum_annotations_resolve_after_parsing() {
    use splitscript::compiler::ast::TypeRef;

    let parsed = splitscript::parse(
        r#"
            state "game.exe" {}
            struct Point {
                x: i32
            }
            enum Location {
                Known(Point),
                Unknown
            }
            fn identity(point: Point) -> Point { return point }
            fn keep(location: Location) -> Location { return location }
        "#,
    )
    .unwrap();
    for (function, expected_name) in parsed.syntax().functions.iter().zip(["Point", "Location"]) {
        let Some(TypeRef::Named(parameter_name)) = function.params[0].annotation else {
            panic!("source nominal parameters should retain name identities");
        };
        let Some(TypeRef::Named(result_name)) = function.return_annotation else {
            panic!("source nominal results should retain name identities");
        };
        assert_eq!(parameter_name, result_name);
        assert_eq!(parsed.syntax().type_name(parameter_name), expected_name);
    }

    let checked = splitscript::check(parsed).unwrap();
    let point_parameter = checked.syntax().functions[0].params[0].id;
    let location_parameter = checked.syntax().functions[1].params[0].id;
    assert_eq!(
        checked.semantics().value_type(point_parameter).unwrap(),
        checked
            .semantics()
            .types()
            .id_for_struct(checked.syntax().structs[0].id),
    );
    assert_eq!(
        checked.semantics().value_type(location_parameter).unwrap(),
        checked
            .semantics()
            .types()
            .id_for_enum(checked.syntax().enums[0].id),
    );
    assert_eq!(
        checked
            .semantics()
            .types()
            .kind(checked.semantics().value_type(point_parameter).unwrap()),
        &TypeKind::Struct(checked.syntax().structs[0].id)
    );
    assert_eq!(
        checked
            .semantics()
            .types()
            .kind(checked.semantics().value_type(location_parameter).unwrap()),
        &TypeKind::Enum(checked.syntax().enums[0].id)
    );
}

#[test]
fn semantic_capabilities_query_declared_and_derived_types_by_type_id() {
    let checked = splitscript::check(
        splitscript::parse(
            r#"
            state "game.exe" {}
            struct Pair {
                left: i32,
                right: i32
            }
            enum MaybePair {
                Pair(Pair),
                Empty
            }
            fn keep(value: MaybePair) -> MaybePair { return value }
        "#,
        )
        .expect("the capability fixture should parse"),
    )
    .expect("the capability fixture should type-check");
    let types = checked.semantics().types();
    let pair = types
        .iter()
        .find_map(|(id, kind)| {
            matches!(kind, TypeKind::Struct(structure) if *structure == checked.syntax().structs[0].id)
                .then_some(id)
        })
        .expect("the source struct should have a semantic type");
    let maybe_pair = types
        .iter()
        .find_map(|(id, kind)| {
            matches!(kind, TypeKind::Enum(enumeration) if *enumeration == checked.syntax().enums[0].id)
                .then_some(id)
        })
        .expect("the source enum should have a semantic type");
    let string = types.id_for_standard(StdlibTypeId::String);
    let capabilities = checked.capabilities();

    assert!(capabilities.has(
        pair,
        splitscript::compiler::stdlib::StdlibCapabilityId::Equatable,
        checked.semantics(),
    ));
    assert!(capabilities.has(
        pair,
        splitscript::compiler::stdlib::StdlibCapabilityId::MemoryReadable,
        checked.semantics(),
    ));
    assert!(capabilities.has(
        maybe_pair,
        splitscript::compiler::stdlib::StdlibCapabilityId::Equatable,
        checked.semantics(),
    ));
    assert!(!capabilities.has(
        maybe_pair,
        splitscript::compiler::stdlib::StdlibCapabilityId::MemoryReadable,
        checked.semantics(),
    ));
    assert!(capabilities.has(
        string,
        splitscript::compiler::stdlib::StdlibCapabilityId::Display,
        checked.semantics(),
    ));
}

#[test]
fn source_methods_structurally_satisfy_display() {
    let source = r#"
        state "game.exe" {}

        struct Position {
            x: i32,
            y: i32,
        }

        enum Mode {
            Running,
            Paused,
        }

        fn Position.toString() -> String {
            return `({self.x}, {self.y})`
        }

        fn Mode.toString() {
            return match self {
                Mode.Running => "running",
                Mode.Paused => "paused",
            }
        }

        whileAttached {
            let position = Position { x: 3, y: 5 }
            print(position)
            print(position as String)
            print(`position = {position}`)
            setVariable("Position", position)
            print(Mode.Running)
        }
    "#;

    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("matching source methods should satisfy Display structurally");
    let position = checked
        .semantics()
        .types()
        .id_for_struct(checked.syntax().structs[0].id);
    let mode = checked
        .semantics()
        .types()
        .id_for_enum(checked.syntax().enums[0].id);
    for ty in [position, mode] {
        assert!(checked.capabilities().has(
            ty,
            splitscript::compiler::stdlib::StdlibCapabilityId::Display,
            checked.semantics(),
        ));
    }

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::compile(source).unwrap())
        .expect("source-defined Display methods should lower to valid Wasm");
}

#[test]
fn structural_display_derives_by_default_and_reports_mismatched_overrides() {
    let derived = r#"
        state "game.exe" {}
        struct Position { x: i32, }
        enum Location { Unknown, Known(Position), }
        whileAttached {
            let position = Position { x: 3 }
            print(position)
            print(position as String)
            print(`position {position}`)
            print(Location.Known(position))
        }
    "#;
    let checked = splitscript::check(splitscript::parse(derived).unwrap())
        .expect("source aggregates should derive Display without boilerplate");
    for ty in [
        checked
            .semantics()
            .types()
            .id_for_struct(checked.syntax().structs[0].id),
        checked
            .semantics()
            .types()
            .id_for_enum(checked.syntax().enums[0].id),
    ] {
        assert!(checked.capabilities().has(
            ty,
            splitscript::compiler::stdlib::StdlibCapabilityId::Display,
            checked.semantics(),
        ));
    }
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::compile(derived).unwrap())
        .expect("lazily derived Display helpers should form valid Wasm");

    for method in ["fn Position.toString() -> i32 { return self.x }"] {
        for consumer in [
            "print(Position { x: 3 })",
            "let text = Position { x: 3 } as String",
            "let text = `position {Position { x: 3 }}`",
        ] {
            let source = format!(
                r#"
                    state "game.exe" {{}}
                    struct Position {{ x: i32, }}
                    {method}
                    whileAttached {{ {consumer} }}
                "#
            );
            let diagnostics = splitscript::check(splitscript::parse(&source).unwrap())
                .expect_err("an invalid structural Display implementation must fail");
            let diagnostic = diagnostics
                .iter()
                .find(|diagnostic| {
                    diagnostic.message.contains("does not match")
                        && diagnostic.message.contains("Display")
                        && diagnostic.message.contains("toString")
                })
                .unwrap_or_else(|| panic!("{diagnostics:#?}"));
            assert!(
                diagnostic.labels.iter().any(|label| {
                    label.style == splitscript::DiagnosticLabelStyle::Secondary
                        && label
                            .message
                            .as_deref()
                            .is_some_and(|message| message.contains("this method was considered"))
                }),
                "{diagnostic:#?}"
            );
        }
    }
}

#[test]
fn structural_debug_can_be_overridden_and_rejects_mismatched_methods() {
    let source = r#"
        state "game.exe" {}
        struct Position { x: i32, }
        fn Position.debugString() -> String { return `point:{self.x}` }
        whileAttached {
            print(Position { x: 3 })
            print([Position { x: 4 }])
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("an exact debugString method should override derived Debug");
    let position = checked
        .semantics()
        .types()
        .id_for_struct(checked.syntax().structs[0].id);
    assert!(checked.capabilities().has(
        position,
        splitscript::compiler::stdlib::StdlibCapabilityId::Debug,
        checked.semantics(),
    ));
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::compile(source).unwrap())
        .expect("custom Debug calls should lower to valid Wasm");

    let diagnostics = splitscript::check(
        splitscript::parse(
            r#"
                state "game.exe" {}
                struct Position { x: i32, }
                fn Position.debugString() -> i32 { return self.x }
                whileAttached { print(Position { x: 3 }) }
            "#,
        )
        .unwrap(),
    )
    .expect_err("a malformed debugString override must not silently derive Debug");
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.message.contains("does not match")
                && diagnostic.message.contains("Debug")
                && diagnostic.message.contains("debugString")
        }),
        "{diagnostics:#?}"
    );
}

#[test]
fn implicit_display_calls_propagate_source_method_effects() {
    let source = r#"
        state "game.exe" {}
        struct ProcessLabel { prefix: String, }
        struct WrappedLabel { label: ProcessLabel, }

        fn ProcessLabel.toString() -> String {
            return `{self.prefix}: {process.name()}`
        }

        fn label(value: ProcessLabel) -> String {
            return value as String
        }

        setup {
            print(label(ProcessLabel { prefix: "game" }))
            print(WrappedLabel { label: ProcessLabel { prefix: "game" } })
        }
    "#;
    let diagnostics = splitscript::check(splitscript::parse(source).unwrap())
        .expect_err("implicit Display calls must preserve attached-process effects");
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.message.contains("requires an attached process")
                && diagnostic.message.contains("setup")
        }),
        "{diagnostics:#?}"
    );
}

#[test]
fn implicit_debug_calls_propagate_source_method_effects() {
    let source = r#"
        state "game.exe" {}
        struct ProcessLabel { prefix: String, }

        fn ProcessLabel.debugString() -> String {
            return `{self.prefix}: {process.name()}`
        }

        setup {
            let text = [ProcessLabel { prefix: "game" }] as String
        }
    "#;
    let diagnostics = splitscript::check(splitscript::parse(source).unwrap())
        .expect_err("nested Debug calls must preserve attached-process effects");
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.message.contains("requires an attached process")
                && diagnostic.message.contains("setup")
        }),
        "{diagnostics:#?}"
    );
}

#[test]
fn implicit_debug_calls_propagate_state_snapshot_effects() {
    let source = r#"
        state "game.exe" {
            health: u32 at 0x100;
        }
        struct SnapshotLabel { prefix: String, }

        fn SnapshotLabel.debugString() -> String {
            return `{self.prefix}: {current.health}`
        }

        onAttach {
            let text = [SnapshotLabel { prefix: "health" }] as String
        }
    "#;
    let diagnostics = splitscript::check(splitscript::parse(source).unwrap())
        .expect_err("nested Debug calls must preserve state-snapshot effects");
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.message.contains("requires state snapshots")
                && diagnostic.message.contains("onAttach")
        }),
        "{diagnostics:#?}"
    );
}

#[test]
fn semantic_type_ids_intern_constructed_generic_arguments() {
    let source = r#"
        state "game.exe" {}
        whileAttached {
            let matrix = [[1i32], [2i32]]
            let row = matrix[0]
        }
    "#;
    let parsed = splitscript::parse(source).expect("source should parse");
    let checked = splitscript::check(parsed).expect("source should type-check");
    let row = checked
        .syntax()
        .actions
        .iter()
        .flat_map(|action| &action.body.statements)
        .find_map(|statement| match statement {
            splitscript::compiler::ast::Stmt::Variable(variable) if variable.name == "row" => {
                Some(variable.id)
            }
            _ => None,
        })
        .expect("the row binding should exist");
    let row = checked
        .semantics()
        .value_type(row)
        .expect("the row binding should have a semantic type");
    let TypeKind::Array { element, .. } = checked.semantics().types().kind(row) else {
        panic!("the indexed row should retain its interned array type");
    };
    assert_eq!(
        checked.semantics().types().kind(*element),
        &TypeKind::Builtin(BuiltinType::I32)
    );
}

#[test]
fn option_and_result_annotations_are_distinct_interned_semantic_types() {
    let source = r#"
        state "game.exe" {}

        struct Wrappers {
            maybe: i32?,
            attempt: String!
        }

        fn inspect(maybe: i32?, attempt: String!) {}
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let syntax = checked.syntax();
    let semantics = checked.semantics();
    let function = &syntax.functions[0];

    let maybe = semantics.value_type(function.params[0].id).unwrap();
    let TypeKind::Option {
        layout: maybe_layout,
        value: maybe_value,
    } = semantics.types().kind(maybe)
    else {
        panic!("`i32?` should remain a first-class option type");
    };
    assert_eq!(
        semantics.types().kind(*maybe_value),
        &TypeKind::Builtin(BuiltinType::I32)
    );
    let splitscript::compiler::ast::TypeRef::Option(parsed_maybe_layout) =
        function.params[0].annotation.unwrap()
    else {
        panic!("the parsed annotation should retain its option layout");
    };
    assert_eq!(*maybe_layout, parsed_maybe_layout);

    let attempt = semantics.value_type(function.params[1].id).unwrap();
    let TypeKind::Result {
        layout: attempt_layout,
        value: attempt_value,
    } = semantics.types().kind(attempt)
    else {
        panic!("`String!` should remain a first-class result type");
    };
    assert_eq!(
        semantics.types().kind(*attempt_value),
        &TypeKind::Standard(StdlibTypeId::String)
    );
    let splitscript::compiler::ast::TypeRef::Result(parsed_attempt_layout) =
        function.params[1].annotation.unwrap()
    else {
        panic!("the parsed annotation should retain its result layout");
    };
    assert_eq!(*attempt_layout, parsed_attempt_layout);

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("option and result annotations should have valid Wasm GC layouts");
}

#[test]
fn option_and_result_equality_is_structural_and_payload_checked() {
    let source = include_str!("../wrapper_equality.split");
    let wasm = splitscript::compile(source).expect("wrapper equality should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("Option and Result equality helpers should produce valid Wasm GC");

    let invalid = r#"
        struct Callback { run: () -> i32 }
        state "game.exe" {}

        fn same(left: Callback?, right: Callback?) -> bool {
            return left == right
        }

        whileAttached {
            let value = same(None, None)
        }
    "#;
    let diagnostics = splitscript::compile(invalid)
        .expect_err("wrappers should require equality on their contained values");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("optional value does not support equality")
    }));
}

#[test]
fn option_and_result_matches_bind_values_and_require_exhaustiveness() {
    let source = include_str!("../wrapper_match.split");
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("wrapper matches should type-check");
    assert!(
        checked
            .typed_hir()
            .patterns()
            .any(|pattern| pattern.wrapper.is_some())
    );
    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("wrapper match lowering should produce valid Wasm GC");

    for (source, missing) in [
        (
            r#"
                state "game.exe" {}
                fn unwrap(value: i32?) -> i32 {
                    return match value { Some(present) => present }
                }
            "#,
            "missing `None`",
        ),
        (
            r#"
                state "game.exe" {}
                fn unwrap(value: i32!) -> i32 {
                    return match value { Ok(success) => success }
                }
            "#,
            "missing `Err(error)`",
        ),
    ] {
        let diagnostics = splitscript::compile(source)
            .expect_err("wrapper matches without both states must be rejected");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains(missing)),
            "expected `{missing}` in {diagnostics:?}"
        );
    }

    let diagnostics = splitscript::compile(
        r#"
            state "game.exe" {}
            fn unwrap(value: i32?) -> i32 {
                return match value {
                    present => present,
                    None => 0
                }
            }
        "#,
    )
    .expect_err("bare wrapper bindings should be rejected as misleading");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("use `Some(present)` or `Ok(present)`")
    }));
}

#[test]
fn source_structs_can_implement_the_generic_memory_reader_contract() {
    let wasm = splitscript::compile(
        r#"
            state "game.exe" {}

            struct Reader {}

            fn Reader.read(address: address) {
                return process.read(address)
            }

            fn readU32(reader, address) -> u32! {
                return reader.read(address)
            }

            whileAttached {
                let value = readU32(Reader {}, 0x1000) else 0
                print(value)
            }
        "#,
    )
    .expect("a source method should satisfy MemoryReader with its inferred generic read type");
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&wasm)
        .expect("source-defined generic capability dispatch should emit valid Wasm");
}
