use wasmparser::{Validator, WasmFeatures};

#[test]
fn optional_expression_state_fields_contextualize_none() {
    let source = r#"
        state "game.exe" {
            label: String? = None;
            count: u32? = { None };
        }

        whileAttached {
            print(current.label)
            print(current.count)
        }
    "#;

    for profile in [
        splitscript::BuildProfile::Debug,
        splitscript::BuildProfile::Release,
    ] {
        let checked = splitscript::check(splitscript::parse(source).unwrap())
            .expect("bare `None` should use the optional state field's declared type");
        let wasm = splitscript::codegen_with_options(
            &checked,
            splitscript::CompilerOptions {
                profile,
                ..splitscript::CompilerOptions::default()
            },
        );
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&wasm)
            .expect("optional state expressions should emit valid Wasm GC");
    }
}

#[test]
fn none_state_expression_still_explains_a_non_optional_field() {
    let source = r#"
        state "game.exe" {
            score: u32 = None;
        }
    "#;

    let diagnostics = splitscript::check(splitscript::parse(source).unwrap())
        .expect_err("`None` must not flow into a non-optional state field");
    let mismatch = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.message.contains("expected `u32`, found `None`"))
        .expect("the mismatch should name the declared value type rather than the poll wrapper");
    assert!(mismatch.labels.iter().any(|label| {
        label
            .message
            .as_deref()
            .is_some_and(|message| message.contains("state field `score` is declared as `u32`"))
    }));
}

#[test]
fn state_fields_can_depend_on_later_siblings_and_dynamic_at_bases() {
    let source = r#"
        state "game.exe" {
            value: u32 at base, 0x20;
            copied: u32 = value;
            base: address = 0x1000;
        }

        whileAttached {
            print(current.copied)
        }
    "#;

    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("state dependencies should be independent of declaration order");
    let fields = checked
        .syntax()
        .state
        .as_ref()
        .unwrap()
        .all_fields()
        .collect::<Vec<_>>();
    let value = fields.iter().find(|field| field.name == "value").unwrap();
    let copied = fields.iter().find(|field| field.name == "copied").unwrap();
    let base = fields.iter().find(|field| field.name == "base").unwrap();
    assert_eq!(checked.semantics().state_dependencies(value.id), [base.id]);
    assert_eq!(
        checked.semantics().state_dependencies(copied.id),
        [value.id]
    );

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("dependency-ordered dynamic state reads should emit valid Wasm GC");
}

#[test]
fn state_dependency_cycles_point_to_every_participating_field() {
    let source = r#"
        state "game.exe" {
            first: u32 = second;
            second: u32 = third;
            third: u32 = first;
        }
    "#;

    let diagnostics = splitscript::check(splitscript::parse(source).unwrap())
        .expect_err("cyclic state dependencies must be rejected");
    let cycle = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.message.contains("cyclically"))
        .expect("the cycle should have a dedicated diagnostic");
    assert_eq!(cycle.labels.len(), 3);
}

#[test]
fn emulator_state_paths_accept_sibling_hardware_addresses() {
    let source = r#"
        state GBA {
            value: u8 at base, 4;
            base: u32 = 0x02000000;
        }
    "#;
    let wasm = splitscript::compile(source)
        .expect("emulator pointer paths should use their provider address type");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("dynamic emulator addresses should emit valid Wasm GC");
}

#[test]
fn managed_metadata_can_select_multiple_attachment_dimensions_automatically() {
    let source = r#"
        enum Edition { Base, Demo }
        enum Storefront { Steam, GOG }
        let edition: Edition
        let storefront: Storefront

        image "Assembly-CSharp" {
            class GameManager {
                if edition == Edition.Base && storefront == Storefront.Steam {
                    u32 baseSteamMarker;
                }
                if edition == Edition.Base && storefront == Storefront.GOG {
                    u32 baseGogMarker;
                }
                if edition == Edition.Demo && storefront == Storefront.Steam {
                    u32 demoSteamMarker;
                }
                if edition == Edition.Demo && storefront == Storefront.GOG {
                    u32 demoGogMarker;
                }
            }
        }

        state Unity ["game.exe"] {}

        onAttach {
            print(edition)
        }

        whileAttached {
            if edition == Edition.Base {
                print("base")
            }
        }
    "#;

    let wasm = splitscript::compile(source)
        .expect("distinct managed presence patterns should select every dimension");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("automatic multi-dimensional shape selection should emit valid Wasm");
}

#[test]
fn binding_free_is_patterns_refine_static_shape_predicates() {
    let source = r#"
        enum Edition { Base, Demo }
        let edition: Edition

        image "Assembly-CSharp" {
            class GameManager {
                if edition is Edition.Base {
                    u32 level;
                } else {
                    u32 scene;
                }
            }
        }

        state Unity ["game.exe"] {}
    "#;
    let wasm = splitscript::compile(source)
        .expect("a binding-free enum `is` pattern should select a static shape");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("an `is`-selected managed shape should emit valid Wasm");

    let invalid = source.replace("edition is Edition.Base", "(edition is selectedEdition)");
    let diagnostics = splitscript::compile(&invalid)
        .expect_err("static shape predicates cannot introduce runtime bindings");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("shape predicates cannot introduce conditional binding")
    }));
}

#[test]
fn automatic_shape_failure_report_names_observations_and_source_candidates() {
    let source = r#"
        enum Edition { Base, Demo }
        let edition: Edition

        image "Assembly-CSharp" {
            class GameManager {
                if edition == Edition.Base {
                    u32 level;
                } else {
                    u32 scene;
                }
            }
        }

        state Unity ["game.exe"] {}
    "#;

    let wasm = splitscript::compile(source)
        .expect("a metadata-selected shape should compile with a failure report");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("the attachment report should emit valid Wasm GC");
    let contains = |needle: &str| {
        wasm.windows(needle.len())
            .any(|window| window == needle.as_bytes())
    };
    assert!(contains(
        "Could not select the attachment shape: managed metadata did not match any declared shape"
    ));
    assert!(contains("Observed present managed fields:"));
    assert!(contains("Observed absent managed fields:"));
    assert!(contains("Assembly-CSharp::GameManager.level"));
    assert!(contains("Assembly-CSharp::GameManager.scene"));
    assert!(contains(
        "Expected attachment shape `edition = Edition.Base`"
    ));
    assert!(contains(
        "Expected attachment shape `edition = Edition.Demo`"
    ));
}

#[test]
fn explicit_shape_selection_does_not_embed_the_automatic_failure_report() {
    let source = r#"
        enum Edition { Base, Demo }
        let edition: Edition

        image "Assembly-CSharp" {
            class GameManager {
                if edition == Edition.Base { u32 level; }
                else { u32 scene; }
            }
        }

        state Unity ["game.exe"] {}

        onAttach {
            edition = Edition.Base
        }
    "#;

    let wasm = splitscript::compile(source)
        .expect("explicit shape selection should bypass the automatic report");
    assert!(
        !wasm
            .windows("Could not select the attachment shape".len())
            .any(|window| window == "Could not select the attachment shape".as_bytes())
    );
}

#[test]
fn automatic_shape_selection_requires_distinguishable_metadata_evidence() {
    let source = r#"
        enum Edition { Base, Demo }
        enum Storefront { Steam, GOG }
        let edition: Edition
        let storefront: Storefront
        image "Assembly-CSharp" {
            class GameManager {
                if (edition == Edition.Base && storefront == Storefront.Steam)
                    || (edition == Edition.Base && storefront == Storefront.GOG)
                {
                    u32 marker;
                }
            }
        }
        state Unity ["game.exe"] {}
    "#;
    let diagnostics = splitscript::compile(source)
        .expect_err("unconditional metadata cannot identify either shape");
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic
                .notes
                .iter()
                .any(|note| note.contains("do not distinguish every shape combination"))
        }),
        "{diagnostics:#?}"
    );
}

#[test]
fn conditional_state_fields_refine_multiple_attachment_dimensions() {
    let source = r#"
        enum Edition { BaseGame, DlcDemo }
        enum Storefront { Steam, GOG }
        let edition: Edition
        let storefront: Storefront

        state "game.exe" {
            common: u8 at 0x100;
            if edition == Edition.BaseGame {
                baseLevel: u8 at 0x180;
            }
            if edition == Edition.BaseGame
                && storefront == Storefront.Steam
            {
                steamLevel: u16 at 0x200;
            }
        }

        onAttach {
            edition = Edition.BaseGame
            storefront = Storefront.Steam
        }

        split {
            let steamLevelChanged = edition == Edition.BaseGame
                && storefront == Storefront.Steam
                && current.steamLevel != old.steamLevel
            let baseLevelKnown = edition == Edition.DlcDemo
                || current.common == 255
                || current.baseLevel > 0
            return steamLevelChanged || baseLevelKnown || current.common != old.common
        }
    "#;
    let wasm = splitscript::compile(source)
        .expect("shape predicates should refine and gate conditional state fields");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("conditional state polling should produce valid Wasm GC");

    let unrefined = source.replace(
        "current.common != old.common",
        "current.steamLevel != old.steamLevel",
    );
    let diagnostics =
        splitscript::compile(&unrefined).expect_err("conditional fields need refinement");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("state field `steamLevel` is conditional")
    }));
}

#[test]
fn attachment_globals_are_conditional_schema_dimensions() {
    let source = r#"
        enum Edition { BaseGame, Demo }
        let edition: Edition

        state "game.exe" {
            common: u8 at 0x100;
            if edition is Edition.BaseGame {
                level: u8 at 0x180;
            } else {
                scene: u16 at 0x200;
            }
        }

        onAttach {
            edition = Edition.BaseGame
        }

        split {
            if edition is Edition.BaseGame {
                return current.level != old.level
            } else {
                return current.scene != old.scene
            }
        }
    "#;
    let wasm = splitscript::compile(source)
        .expect("an attachment enum global should select conditional state fields");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("global schema predicates should produce valid Wasm GC");
}

#[test]
fn attachment_shape_globals_are_frozen_after_attach() {
    let source = r#"
        enum Edition { BaseGame, Demo }
        let edition: Edition
        state "game.exe" {
            if edition is Edition.BaseGame { level: u8 at 0x100; }
            else { scene: u8 at 0x200; }
        }
        onAttach { edition = Edition.BaseGame }
        whileAttached { edition = Edition.Demo }
    "#;
    let diagnostics = splitscript::compile(source)
        .expect_err("an attachment schema discriminator must remain frozen");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message == "attachment-shape global `edition` can only be assigned in `onAttach`"
    }));
}

#[test]
fn attachment_shape_selection_has_one_owner() {
    let source = r#"
        enum Edition { Base, Demo }
        enum Storefront { Steam, GOG }
        let edition: Edition
        let storefront: Storefront
        state "game.exe" {
            if edition is Edition.Base && storefront is Storefront.Steam {
                value: u8 at 0x100;
            } else {
                value: u8 at 0x200;
            }
        }
        onAttach { edition = Edition.Base }
        split { return current.value != old.value }
    "#;
    let diagnostics = splitscript::compile(source)
        .expect_err("shape selection must not mix user and provider ownership");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message
            == "`onAttach` cannot mix explicit and provider-inferred attachment shape"
            && diagnostic
                .notes
                .iter()
                .any(|note| note.contains("still provider-inferred: storefront"))
    }));
}

#[test]
fn state_enum_fields_are_dynamic_schema_dimensions() {
    let source = r#"
        enum Game { Menu, Playing }

        state "game.exe" {
            game: Game = if process.read<u8>(0x100)? == 0 {
                Game.Menu
            } else {
                Game.Playing
            };
            if game is Game.Menu {
                selection: u8 at 0x200;
            } else {
                level: u16 at 0x300;
            }
        }

        split {
            if current.game is Game.Playing {
                return current.level != old.level
            }
            return false
        }
    "#;
    let wasm = splitscript::compile(source)
        .expect("a state enum should select dynamically conditional state fields");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("dynamic state predicates should produce valid Wasm GC");
}

#[test]
fn exhaustive_shape_branches_share_compatible_state_fields() {
    let source = r#"
        enum Build { Steam, GOG }
        let build: Build
        state "game.exe" {
            if build is Build.Steam {
                value: u32 at 0x100;
                checkpoint: u8 at 0x110;
            } else {
                value: u32 at 0x200;
                checkpoint: u16 at 0x210;
            }
        }
        onAttach { build = Build.Steam }
        split {
            if old.value != current.value { return true }
            return match build {
                Build.Steam => old.checkpoint != current.checkpoint,
                Build.GOG => old.checkpoint != current.checkpoint,
            }
        }
    "#;
    let wasm = splitscript::compile(source).expect(
        "exhaustive same-typed fields should share an interface while conflicting types refine",
    );
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("compatible conditional storage should produce valid Wasm GC");
}

#[test]
fn conditional_state_field_chains_preserve_exact_remaining_shapes() {
    let source = r#"
        enum Edition { Base, Demo }
        enum Storefront { Steam, GOG }
        let edition: Edition
        let storefront: Storefront

        state "game.exe" {
            if edition == Edition.Base && storefront == Storefront.Steam {
                steamLevel: u8 at 0x100;
            } else if edition == Edition.Base {
                gogLevel: u8 at 0x200;
            } else {
                demoLevel: u8 at 0x300;
            }
        }

        onAttach {
            edition = Edition.Base
            storefront = Storefront.GOG
        }

        split {
            if edition == Edition.Base && storefront == Storefront.Steam {
                return current.steamLevel != old.steamLevel
            } else if edition == Edition.Base && storefront == Storefront.GOG {
                return current.gogLevel != old.gogLevel
            } else if edition == Edition.Demo {
                return current.demoLevel != old.demoLevel
            }
            return false
        }
    "#;
    let wasm = splitscript::compile(source)
        .expect("else-if state fields should retain the exact shapes left by earlier branches");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("conditional branch predicates should lower to valid Wasm GC");

    let insufficiently_refined = source.replace(
        "if edition == Edition.Base && storefront == Storefront.Steam {\n                return current.steamLevel != old.steamLevel\n            } else if edition == Edition.Base && storefront == Storefront.GOG {\n                return current.gogLevel != old.gogLevel\n            } else if edition == Edition.Demo {\n                return current.demoLevel != old.demoLevel\n            }\n            return false",
        "if edition == Edition.Base {\n                return current.gogLevel != old.gogLevel\n            }\n            return false",
    );
    let diagnostics = splitscript::compile(&insufficiently_refined)
        .expect_err("the else-if field is absent from the earlier Base/Steam branch");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("state field `gogLevel` is conditional")
    }));
}

#[test]
fn conditional_shape_branch_enumeration_has_a_deterministic_bound() {
    let source = r#"
        enum Binary { A, B }
        let a: Binary
        let b: Binary
        let c: Binary
        let d: Binary
        let e: Binary
        let f: Binary
        let g: Binary
        let h: Binary
        let i: Binary
        state "game.exe" {
            if a == Binary.A
                && b == Binary.A
                && c == Binary.A
                && d == Binary.A
                && e == Binary.A
                && f == Binary.A
                && g == Binary.A
                && h == Binary.A
                && i == Binary.A
            {
                value: u8 at 0x100;
            } else {
                other: u8 at 0x200;
            }
        }
        onAttach {
            a = Binary.A
            b = Binary.A
            c = Binary.A
            d = Binary.A
            e = Binary.A
            f = Binary.A
            g = Binary.A
            h = Binary.A
            i = Binary.A
        }
    "#;
    let diagnostics = splitscript::compile(source)
        .expect_err("conditional declarations must not enumerate an unbounded shape product");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("conditional fields require a bounded attachment shape")
            && diagnostic
                .notes
                .iter()
                .any(|note| note.contains("at most 256 shape combinations"))
    }));
}

#[test]
fn managed_fields_share_the_attachment_shape_refinement_model() {
    let source = r#"
        enum Edition { BaseGame, Demo }
        let edition: Edition

        image "Assembly-CSharp" {
            class GameManager {
                static GameManager instance;
                if edition == Edition.BaseGame {
                    u32 level;
                }
                else {
                    u32 scene;
                }
            }
        }

        state Unity ["game.exe"] {}

        onAttach { edition = Edition.BaseGame }

        whileAttached {
            let manager = GameManager.instance else return
            if edition == Edition.BaseGame {
                print(manager.level else 0)
            } else {
                print(manager.scene else 0)
            }
        }
    "#;
    let wasm = splitscript::compile(source)
        .expect("managed fields should consume the global shape predicate");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("conditional managed bindings should produce valid Wasm GC");

    let unrefined = source.replace(
        "if edition == Edition.BaseGame {\n                print(manager.level else 0)\n            }",
        "print(manager.level else 0)",
    );
    let diagnostics =
        splitscript::compile(&unrefined).expect_err("managed fields need shape refinement");
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("unknown field or method `level`")
                || diagnostic.message.contains("conditional")
        }),
        "{diagnostics:#?}"
    );
}

#[test]
fn unity_metadata_can_initialize_an_attachment_shape_global() {
    use splitscript::tooling::database::CompilerDatabase;

    let source = r#"
        enum Edition { BaseGame, Demo }
        let edition: Edition

        image "Assembly-CSharp" {
            class GameManager {
                static GameManager instance;
                if edition is Edition.BaseGame {
                    u32 level;
                } else {
                    u32 scene;
                }
            }
        }

        state Unity ["game.exe"] {
            if edition is Edition.BaseGame {
                level: u32 = GameManager.instance?.level?;
            } else {
                scene: u32 = GameManager.instance?.scene?;
            }
        }

        split {
            if edition is Edition.BaseGame {
                return current.level != old.level
            } else {
                return current.scene != old.scene
            }
        }
    "#;
    let wasm = splitscript::compile(source)
        .expect("unique Unity metadata should initialize the shape global automatically");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("automatic global shape selection should produce valid Wasm GC");

    let mut database = CompilerDatabase::new(source);
    let position = source.find("let edition").unwrap() + "let ".len();
    let hover = database.hover(position).unwrap().unwrap();
    assert!(
        hover
            .markdown
            .contains("initialized automatically from the attached provider's schema")
    );
}

#[test]
fn lunistice_shaped_unity_schema_reads_both_editions_without_manual_offsets() {
    let source = r#"
        enum Edition {
            BaseGame,
            DlcDemo,
        }

        let edition: Edition

        struct LevelTimeParts {
            minutes: f32,
            seconds: f32,
            hundredths: f32,
        }

        image "Assembly-CSharp" {
            class GameManager {
                static GameManager instance from "Instance";
                i32 gameState from ["gameState", "GameState"];
                u32 points from "_points";
                u32 deaths from "_deaths";

                if edition == Edition.BaseGame {
                    i32 level from "currentLevel";
                }

                else {
                    String scene from "_currentScene";
                    String? subtitle;
                }
            }

            class Timer {
                static Timer instance from ["Instance", "_instance"];
                f32 levelTime from "currentLevelTime";
                LevelTimeParts levelTimeParts from "currentLevelTimeVector";
                bool stopped from "timerStopped";
                u32 character;
            }
        }

        state Unity.il2cpp(Il2CppProfile.unity2021_3_11f1X64()) ["Lunistice.exe", "Lunistice-Demo.exe"] {
            gameState: i32 = GameManager.instance?.gameState?;
            points: u32 = GameManager.instance?.points?;
            deaths: u32 = GameManager.instance?.deaths?;
            if edition == Edition.BaseGame {
                level: i32 = GameManager.instance?.level?;
            }
            else {
                scene: String = GameManager.instance?.scene?;
                subtitle: String? = GameManager.instance?.subtitle?;
            }
            levelTime: f32 = Timer.instance?.levelTime?;
            levelTimeParts: LevelTimeParts = Timer.instance?.levelTimeParts?;
            timerStopped: bool = Timer.instance?.stopped?;
            character: u32 = Timer.instance?.character?;
        }

        whileAttached {
            print(current.levelTimeParts)
            if edition == Edition.BaseGame {
                print(current.level)
            } else {
                print(current.scene)
                print(current.subtitle)
            }
        }
    "#;

    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("the Lunistice Unity schema should type check");
    let unused = checked
        .diagnostics()
        .iter()
        .filter(|diagnostic| {
            diagnostic.message.starts_with("unused struct")
                || diagnostic.message.starts_with("unused enum")
        })
        .collect::<Vec<_>>();
    assert!(
        unused.is_empty(),
        "shape declarations are used: {unused:#?}"
    );
    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("the Lunistice-shaped schema should produce valid Wasm GC");
}

#[test]
fn managed_arrays_compose_strings_and_owned_classes() {
    for source in [
        include_str!("../managed_arrays.split"),
        include_str!("../managed_array_tree.split"),
        include_str!("../managed_array_freeze.split"),
        include_str!("../managed_lists.split"),
        include_str!("../managed_maps.split"),
        include_str!("../managed_map_inline.split"),
        include_str!("../managed_sets.split"),
        include_str!("../managed_set_classes.split"),
        include_str!("../managed_map_classes.split"),
        include_str!("../managed_nested_map.split"),
        include_str!("../managed_map_lists.split"),
        include_str!("../managed_set_lists.split"),
        include_str!("../managed_nested_set.split"),
        include_str!("../managed_set_inline.split"),
    ] {
        for profile in [
            splitscript::BuildProfile::Debug,
            splitscript::BuildProfile::Release,
        ] {
            let wasm = splitscript::compile_with_options(
                source,
                splitscript::CompilerOptions {
                    profile,
                    ..Default::default()
                },
            )
            .unwrap();
            Validator::new_with_features(WasmFeatures::all())
                .validate_all(&wasm)
                .unwrap();
        }
    }
}

#[test]
fn managed_maps_project_children_and_compose_with_snapshot_types() {
    for field in [
        "Map<i32, [String?]>",
        "Map<String, List<Map<String, List<String?>>>>",
        "Map<String, Root>",
        "Map<[String?], [i32; 2]>",
        "[Map<String, List<String?>>?]",
        "Map<String, Set<[String?]>>",
        "[Set<String>?]",
        "Set<u8>",
        "Set<List<String>>",
        "Set<List<List<String?>>>",
        "Map<List<String>, List<Map<List<String>, Root>>>",
        "Map<Set<List<String>>, Map<List<Root>, List<String>>>",
        "Set<Set<String>>",
        "Set<Map<String, [String?]>>",
        "Map<Set<String>, Map<String, [String?]>>",
        "Map<Map<String, [String?]>, Root>",
        "Set<Root>",
        "Map<Root, [String?]>",
        "Set<[i32; 2]>",
    ] {
        let source = format!(
            r#"
            image "Assembly-CSharp" {{
                class Root {{ static {field} values; i32 score; }}
            }}
            state Unity ["game.exe"] {{ values = Root.values?; }}
            whileAttached {{ setVariable("values", current.values) }}
        "#
        );
        for profile in [
            splitscript::BuildProfile::Debug,
            splitscript::BuildProfile::Release,
        ] {
            let wasm = splitscript::compile_with_options(
                &source,
                splitscript::CompilerOptions {
                    profile,
                    ..Default::default()
                },
            )
            .unwrap_or_else(|error| panic!("{field}: {error:?}"));
            Validator::new_with_features(WasmFeatures::all())
                .validate_all(&wasm)
                .unwrap_or_else(|error| panic!("{field}: {error:?}"));
        }
    }
    let source = r#"
        image "Assembly-CSharp" {
            class Root { static Map<String, List<Map<i32, List<String?>>>> values; }
        }
        state Unity ["game.exe"] { values: Map<String, [Map<i32, [String?]>]> = Root.values?; }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let semantics = checked.semantics();
    let field = checked.syntax().managed_class_declarations()[0].fields[0].id;
    let storage = semantics.managed_field_type(field).unwrap();
    let owned = semantics.managed_field_snapshot_type(field).unwrap();
    assert_ne!(storage, owned);
    assert_eq!(semantics.managed_field_value_type(field), Some(owned));
    assert!(checked.capabilities().has(
        storage,
        splitscript::compiler::stdlib::StdlibCapabilityId::ManagedReadable,
        semantics
    ));
}

#[test]
fn managed_list_constructor_is_resolved_even_with_an_unused_source_name() {
    let wasm = splitscript::compile(
        r#"
        struct List { value: i32, }
        state "game.exe" {}
        fn unusedStorage(value: List<i32>) {}
    "#,
    )
    .unwrap();
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .unwrap();
}

#[test]
fn managed_list_types_are_storage_schemas_with_owned_array_projections() {
    use splitscript::compiler::stdlib::StdlibCapabilityId;
    use splitscript::compiler::types::TypeKind;
    let checked = splitscript::check(
        splitscript::parse(
            r#"
        image "Assembly-CSharp" {
            class Root { static Root instance; List<[List<String>?]> values; }
        }
        state Unity ["game.exe"] { values: [[[String]?]] = Root.instance?.values?; }
        fn snapshotValues(root: Root) -> [[[String]?]] { return root.values }
        fn unusedStorage(value: List<i32>) {}
    "#,
        )
        .unwrap(),
    )
    .unwrap();
    let semantics = checked.semantics();
    let field = checked.syntax().managed_class_declarations()[0]
        .fields
        .iter()
        .find(|field| field.name == "values")
        .unwrap()
        .id;
    let storage = semantics.managed_field_type(field).unwrap();
    let owned = semantics.managed_field_snapshot_type(field).unwrap();
    assert_ne!(storage, owned);
    assert!(matches!(
        semantics.types().kind(storage),
        TypeKind::Application { .. }
    ));
    assert!(matches!(
        semantics.types().kind(owned),
        TypeKind::Array { length: None, .. }
    ));
    assert_eq!(semantics.managed_field_value_type(field), Some(owned));
    assert!(
        checked
            .capabilities()
            .has(storage, StdlibCapabilityId::ManagedReadable, semantics)
    );
    assert!(
        !checked
            .capabilities()
            .has(storage, StdlibCapabilityId::MemoryReadable, semantics)
    );
    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .unwrap();
}

#[test]
fn collection_snapshots_preserve_direct_live_class_reads() {
    let source = r#"
        image "Assembly-CSharp" {
            class Root {
                static Child one;
                static Child? optional;
                static [Child?] many;
            }
            class Child { i32 value; }
        }
        state Unity ["game.exe"] { many = Root.many?; }
        whileAttached {
            let single: Child.Ref = Root.one else return
            let number = single.value else return
            setVariable("one", number)
            let optional: Child.Ref? = Root.optional else return
            match optional {
                Some(reference) => {
                    let optionalNumber = reference.value else return
                    setVariable("optional", optionalNumber)
                },
                None => {},
            }
        }
    "#;
    for profile in [
        splitscript::BuildProfile::Debug,
        splitscript::BuildProfile::Release,
    ] {
        let wasm = splitscript::compile_with_options(
            source,
            splitscript::CompilerOptions {
                profile,
                ..Default::default()
            },
        )
        .unwrap();
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&wasm)
            .expect("a class decoder retained by an array must not replace a live field read");
    }
}

#[test]
fn managed_array_elements_must_be_owned_managed_readable_values() {
    for ty in [
        "[Player.Ref]",
        "List<Player.Ref>",
        "[List<Player.Ref>]",
        "List<[Player.Ref]>",
    ] {
        let source = format!(
            r#"image "Assembly-CSharp" {{
        class Player {{ {ty} children; }}
    }}
    state Unity ["game.exe"] {{}}"#
        );
        let diagnostics = splitscript::compile(&source)
            .expect_err("live references cannot become owned array elements");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("has no supported managed decoder")
        }));
    }
}

#[test]
fn managed_array_budgets_follow_reachable_child_decoders() {
    for (element, class_fields) in [("String", false), ("Child", true)] {
        let source = format!(
            r#"
            image "Assembly-CSharp" {{
                class Probe {{ static [{element}] values; }}
                class Child {{ i32 score; }}
            }}
            state Unity.il2cpp(Il2CppProfile.unity2022_3_0f1X64()) ["game.exe"] {{
                values = Probe.values?;
            }}
        "#
        );
        let checked = splitscript::check(splitscript::parse(&source).unwrap()).unwrap();
        let (wasm, report) = splitscript::compiler::codegen_with_report(
            &checked,
            splitscript::CompilerOptions {
                profile: splitscript::BuildProfile::Release,
                ..Default::default()
            },
        );
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&wasm)
            .unwrap();
        for helper in [
            "EnterManagedObject",
            "ChargeManagedBytes",
            "ChargeManagedElements",
        ] {
            assert!(report.runtime_helpers.iter().any(|name| name == helper));
        }
        assert_eq!(
            report
                .runtime_helpers
                .iter()
                .any(|name| name == "ChargeManagedWork"),
            class_fields
        );
    }
}

#[test]
fn managed_inline_arrays_freeze_without_object_walk_helpers() {
    let source = r#"
        image "Assembly-CSharp" { class Probe { static [i32; 2] values; } }
        state Unity.il2cpp(Il2CppProfile.unity2022_3_0f1X64()) ["game.exe"] {
            values = Probe.values?;
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let (wasm, report) = splitscript::compiler::codegen_with_report(
        &checked,
        splitscript::CompilerOptions {
            profile: splitscript::BuildProfile::Release,
            ..Default::default()
        },
    );
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .unwrap();
    assert!(
        report
            .functions
            .iter()
            .any(|(_, name)| name.contains("::managed::freeze::"))
    );
    for helper in ["ChargeManagedBytes", "ChargeManagedElements"] {
        assert!(report.runtime_helpers.iter().any(|name| name == helper));
    }
    for helper in [
        "EnterManagedObject",
        "ChargeManagedWork",
        "ReadManagedString",
    ] {
        assert!(!report.runtime_helpers.iter().any(|name| name == helper));
    }
}

#[test]
fn unused_managed_collections_retain_no_reader_or_budget() {
    for selector in [
        "Unity.il2cpp(Il2CppProfile.unity2022_3_0f1X64())",
        "Unity.mono(MonoVersion.V2)",
    ] {
        let compile = |unused: &str| {
            let source = format!(
                r#"
                image "Assembly-CSharp" {{ class Player {{ static i32 score; {unused} }} }}
                state {selector} ["game.exe"] {{ score = Player.score?; }}
                whileAttached {{ setVariable("score", current.score) }}
            "#
            );
            let checked = splitscript::check(splitscript::parse(&source).unwrap()).unwrap();
            splitscript::compiler::codegen_with_report(
                &checked,
                splitscript::CompilerOptions {
                    profile: splitscript::BuildProfile::Release,
                    ..Default::default()
                },
            )
        };
        let (plain, plain_report) = compile("");
        for declaration in [
            "static [[String?]?] unused;",
            "static List<[List<String?>?]> unused;",
            "static Map<String, List<Map<i32, [String?]>>> unused;",
            "static Set<[String?]> unused;",
        ] {
            let (unused, unused_report) = compile(declaration);
            // Declaring String? earlier can renumber an already-reachable option
            // type. Check actual emitted features and section sizes, not type IDs.
            assert_eq!(plain.len(), unused.len());
            assert_eq!(plain_report.runtime_helpers, unused_report.runtime_helpers);
            assert_eq!(plain_report.scratch_bytes, unused_report.scratch_bytes);
            assert_eq!(
                plain_report.abi_read_capacity,
                unused_report.abi_read_capacity
            );
            assert_eq!(
                plain_report.static_data_start,
                unused_report.static_data_start
            );
            assert_eq!(plain_report.static_data_end, unused_report.static_data_end);
            assert_eq!(
                plain_report.minimum_memory_pages,
                unused_report.minimum_memory_pages
            );
            assert_eq!(plain_report.functions.len(), unused_report.functions.len());
            let sections = |wasm: &[u8]| {
                wasmparser::Parser::new(0)
                    .parse_all(wasm)
                    .filter_map(|payload| {
                        payload
                            .unwrap()
                            .as_section()
                            .map(|(id, range)| (id, range.len()))
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(sections(&plain), sections(&unused));
            assert!(
                unused_report
                    .functions
                    .iter()
                    .all(|(_, name)| !name.contains("ListLayout")
                        && !name.contains("CollectionClass")
                        && !name.contains("Keyed")
                        && !name.contains("dictionaryLayout"))
            );
        }
    }
}

#[test]
fn sonic_three_air_shaped_range_discovery_and_filtered_state_compile_cleanly() {
    let source = r#"
        let wramBase

        enum Level {
            AngelIsland1,
            AngelIsland2,
        }

        settings {
            "Angel Island 1" => angelIsland1: true,
            "Angel Island 2" => angelIsland2: true,
        }

        state "Sonic3AIR.exe" {
            rawGameState: u8 = process.read(wramBase.offset(0xf600))?;
            rawSaveSlot: u8 = process.read(wramBase.offset(0xf61a))?;
            rawLevel: u8 = process.read(wramBase.offset(0xee4e))?;
            timeBonus: u16 = process.read<u16>(wramBase.offset(0xf7d2))?.swapBytes();
            saveSlot: u8 = 0;
            level: Level = Level.AngelIsland1;
        }

        onAttach {
            let mapping = await process.findMemoryRange(
                0x521000,
                MemoryRangeAccess.Read,
            )
            wramBase = mapping.address.offset(0x400020)
        }

        whileAttached {
            current.saveSlot = if current.rawSaveSlot < 8 {
                current.rawSaveSlot
            } else {
                old.saveSlot
            }
            current.level = match current.rawLevel {
                0 => Level.AngelIsland1,
                1 => Level.AngelIsland2,
                _ => old.level,
            }
        }

        start {
            return old.rawGameState != 0x0c && current.rawGameState == 0x0c
        }

        reset {
            return old.saveSlot != current.saveSlot
        }

        split {
            return old.level != current.level
                && match current.level {
                    Level.AngelIsland1 => settings.angelIsland1,
                    Level.AngelIsland2 => settings.angelIsland2,
                }
        }
    "#;

    let wasm = splitscript::compile(source)
        .expect("the Sonic 3 A.I.R. memory-range and endian patterns should be first-class");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("the representative port should lower to valid Wasm GC");
}

#[test]
fn attachment_scoped_globals_infer_from_on_attach_and_support_shape_specific_values() {
    let source = r#"
        enum Build { Steam, GOG }
        let build: Build
        let module
        let steamBase
        let gogBase

        state "game.exe" {
            if build == Build.Steam {
                level: u32 = process.read(steamBase)?;
            } else {
                level: u32 = process.read(gogBase)?;
            }
        }

        onAttach {
            module = await process.mainModule()
            if module.size == 0x1000 {
                steamBase = module.address
                build = Build.Steam
                return
            }
            gogBase = module.address
            build = Build.GOG
        }

        split {
            return match build {
                Build.Steam => steamBase != 0 && current.level != old.level,
                Build.GOG => gogBase != 0 && current.level != old.level,
            }
        }
    "#;

    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("attachment globals should infer from assignments and uses");
    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("attachment-global WebAssembly GC should validate");

    let wrong_shape = source.replace(
        "level: u32 = process.read(steamBase)?;",
        "level: u32 = process.read(gogBase)?;",
    );
    let errors = splitscript::compile(&wrong_shape)
        .expect_err("state expressions need the attachment values for their own shape");
    assert!(errors.iter().any(|error| {
        error
            .message
            .contains("attachment-scoped global `gogBase` is not initialized")
    }));
}

#[test]
fn attachment_scoped_gc_values_use_nullable_storage_without_exposing_null() {
    let source = r#"
        let delay: Duration

        state "game.exe" {}

        onAttach {
            delay = Duration.fromSeconds(1)
        }

        split {
            return delay > Duration.zero()
        }
    "#;

    for profile in [
        splitscript::BuildProfile::Debug,
        splitscript::BuildProfile::Release,
    ] {
        let wasm = splitscript::compile_with_options(
            source,
            splitscript::CompilerOptions {
                profile,
                ..splitscript::CompilerOptions::default()
            },
        )
        .expect("attachment-scoped non-null GC values should compile");
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&wasm)
            .expect("nullable attachment storage must validate as a non-null source value");
    }
}

#[test]
fn debug_attachment_globals_are_initialized_and_erased_with_their_profile() {
    let source = r#"
        debug let inspectedAddress

        state "game.exe" {}

        onAttach {
            debug inspectedAddress = 0x1000
        }

        whileAttached {
            debug print(inspectedAddress)
        }
    "#;

    for profile in [
        splitscript::BuildProfile::Debug,
        splitscript::BuildProfile::Release,
    ] {
        let wasm = splitscript::compile_with_options(
            source,
            splitscript::CompilerOptions {
                profile,
                ..splitscript::CompilerOptions::default()
            },
        )
        .expect("debug attachment globals should follow debug statement lifetime");
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&wasm)
            .expect("both attachment-global profiles should validate");
    }
}

#[test]
fn attachment_globals_require_definite_initialization_and_shape_refinement() {
    let missing = r#"
        let base: address
        state "game.exe" {}
        onAttach {
            if process.name() == "game.exe" {
                base = 0x1000
            }
        }
        split { return base != 0 }
    "#;
    let errors = splitscript::compile(missing)
        .expect_err("single-shape attachment values need assignment on every completion path");
    assert!(errors.iter().any(|error| {
        error.message == "attachment-scoped global `base` is never initialized by `onAttach`"
            || error.message.contains("not initialized for the attachment")
    }));

    let unrefined = r#"
        enum Build { Steam, GOG }
        let build: Build
        let steamBase: address
        let gogBase: address
        state "game.exe" {
            if build == Build.Steam { level: u32 at 0x10; }
            else { level: u32 at 0x20; }
        }
        onAttach {
            if process.name() == "game.exe" {
                steamBase = 0x1000
                build = Build.Steam
                return
            }
            gogBase = 0x2000
            build = Build.GOG
        }
        fn steamReady() -> bool { return steamBase != 0 }
        split { return steamReady() }
    "#;
    let errors = splitscript::compile(unrefined)
        .expect_err("a shape-specific helper needs a matching refinement");
    assert!(errors.iter().any(|error| {
        error
            .message
            .contains("`steamReady` requires attachment values unavailable")
    }));

    let refined = unrefined.replace(
        "split { return steamReady() }",
        r#"split {
            return match build {
                Build.Steam => steamReady(),
                Build.GOG => gogBase != 0,
            }
        }"#,
    );
    splitscript::compile(&refined)
        .expect("a direct shape match should prove attachment-global availability");
}

#[test]
fn bare_global_lifetimes_require_one_direct_lifecycle_assignment() {
    let source = r#"
        let value: u32
        state "game.exe" {}
        fn initialize() { value = 1 }
        onAttach { initialize() }
    "#;
    let diagnostics = splitscript::compile(source)
        .expect_err("a helper assignment must not silently establish attachment lifetime");
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.message.contains("bare global `value`"))
        .expect("the bare global should own the lifetime diagnostic");

    assert_eq!(
        diagnostic.message,
        "bare global `value` has no direct lifecycle initializer"
    );
    assert!(
        diagnostic.labels[0]
            .message
            .as_deref()
            .is_some_and(|label| {
                label.contains("directly in exactly one `onAttach` or `onStart` block")
            })
    );
    assert!(diagnostic.notes.iter().any(|note| {
        note == "assignments performed by called helpers do not establish a bare global's lifetime"
    }));
}

#[test]
fn on_attach_rejects_attachment_global_reads_before_assignment() {
    let source = r#"
        let module: Module
        state "game.exe" {}
        onAttach {
            let copy = module
            module = await process.mainModule()
        }
    "#;
    let errors = splitscript::compile(source)
        .expect_err("backend defaults must not be observable during initialization");
    assert!(errors.iter().any(|error| {
        error.message == "attachment-scoped global `module` may be read before it is initialized"
    }));
}

#[test]
fn attachment_globals_are_viral_and_unavailable_after_detach() {
    let source = r#"
        let base: address
        state "game.exe" {}
        onAttach { base = 0x1000 }
        fn hasBase() -> bool { return base != 0 }
        onDetach {
            let copy = base
            base = 0x2000
            hasBase()
        }
    "#;
    let errors = splitscript::compile(source)
        .expect_err("detached code must not observe cleared attachment storage");
    assert!(errors.iter().any(|error| {
        error.message == "attachment-scoped global `base` is unavailable in `onDetach`"
            && error.labels.iter().any(|label| {
                label
                    .message
                    .as_deref()
                    .is_some_and(|message| message.contains("write occurs"))
            })
    }));
    assert!(errors.iter().any(|error| {
        error.message == "`hasBase` requires an attached process and is unavailable in `onDetach`"
    }));
}

#[test]
fn attempt_scoped_globals_infer_from_on_start_and_are_viral() {
    let source = r#"
        let accumulated
        state "game.exe" {}

        onStart {
            accumulated = 0.0
        }

        fn add(value: f64) {
            accumulated += value
        }

        gameTime {
            add(1.5)
            return Duration.fromSeconds(accumulated)
        }
    "#;

    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("onStart should infer attempt-scoped globals and helper requirements");
    let accumulated = checked.syntax().globals[0].id;
    assert!(checked.scoped_globals().is_attempt_global(accumulated));
    let helper = checked.syntax().functions[0].id;
    assert!(checked.scoped_globals().function_requires_attempt(helper));

    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("attempt-scoped global WebAssembly GC should validate");
}

#[test]
fn attempt_scoped_globals_require_one_definite_initializer() {
    let partial = r#"
        let accumulated: f64
        state "game.exe" {}
        onStart {
            if timer.state() == TimerState.Running {
                accumulated = 0.0
            }
        }
        gameTime { return Duration.fromSeconds(accumulated) }
    "#;
    let diagnostics = splitscript::compile(partial)
        .expect_err("attempt globals need assignment on every completed onStart path");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message.contains(
            "attempt-scoped global `accumulated` is not initialized on every `onStart` path",
        )
    }));

    let read_before_write = r#"
        let accumulated: f64
        state "game.exe" {}
        onStart {
            print(accumulated)
            accumulated = 0.0
        }
    "#;
    let diagnostics = splitscript::compile(read_before_write)
        .expect_err("backend defaults must not be observable in onStart");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message
            == "attempt-scoped global `accumulated` may be read before it is initialized"
    }));

    let ambiguous = r#"
        let value: u32
        state "game.exe" {}
        onAttach { value = 1 }
        onStart { value = 2 }
    "#;
    let diagnostics = splitscript::compile(ambiguous)
        .expect_err("one bare global cannot have two lifecycle owners");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message == "bare global `value` has both attachment and attempt initializers"
    }));
}

#[test]
fn attempt_scoped_globals_are_rejected_outside_attempt_actions() {
    for action in ["setup", "onAttach", "onDetach", "whileAttached", "start"] {
        let source = format!(
            r#"
                let accumulated: f64
                state "game.exe" {{}}
                onStart {{ accumulated = 0.0 }}
                {action} {{ print(accumulated) }}
            "#
        );
        let diagnostics = match splitscript::compile(&source) {
            Err(diagnostics) => diagnostics,
            Ok(_) => panic!("`{action}` can execute before attempt initialization"),
        };
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains(&format!(
                    "attempt-scoped global `accumulated` is unavailable in `{action}`"
                )))
        );
    }
}

#[test]
fn renaming_a_shared_conditional_field_updates_the_shared_state_interface() {
    use splitscript::tooling::database::CompilerDatabase;

    let source = r#"
        enum Build { Steam, GOG }
        let build: Build
        state "game.exe" {
            if build == Build.Steam { level: u32 at 0x100; }
            else { level: u32 at 0x200; }
        }
        onAttach { build = Build.Steam }
        split { return current.level != old.level }
    "#;
    let second_declaration = source.match_indices("level: u32").nth(1).unwrap().0;
    let mut database = CompilerDatabase::new(source);

    let plan = database.rename_at(second_declaration, "stage").unwrap();
    assert_eq!(plan.edits.len(), 4);
    assert!(
        plan.edits
            .iter()
            .all(|edit| &source[edit.span.start..edit.span.end] == "level")
    );

    let mut renamed = source.to_owned();
    for edit in plan.edits.iter().rev() {
        renamed.replace_range(edit.span.start..edit.span.end, &edit.replacement);
    }
    splitscript::compile(&renamed).expect("the renamed shared state interface should compile");

    let use_site = source.find("current.level").unwrap() + "current.".len();
    assert_eq!(database.rename_at(use_site, "stage").unwrap(), plan);
}

#[test]
fn renaming_a_conflicting_shape_field_keeps_the_other_branch_independent() {
    use splitscript::tooling::database::CompilerDatabase;

    let source = r#"
        enum Build { V8, V9 }
        let build: Build
        state "game.exe" {
            if build == Build.V8 { bike: i16 at 0x100; }
            else { bike: u16 at 0x200; }
        }
        onAttach { build = Build.V8 }
        split {
            return match build {
                Build.V8 => current.bike == 1,
                Build.V9 => current.bike == 2,
            }
        }
    "#;
    let first_declaration = source.find("bike: i16").unwrap();
    let mut database = CompilerDatabase::new(source);
    let plan = database.rename_at(first_declaration, "vehicle").unwrap();
    assert_eq!(plan.edits.len(), 2);
    assert!(
        plan.edits
            .iter()
            .all(|edit| &source[edit.span.start..edit.span.end] == "bike")
    );
    let v9_declaration = source.find("bike: u16").unwrap();
    let v9_use = source.rfind("current.bike").unwrap() + "current.".len();
    assert!(
        plan.edits
            .iter()
            .all(|edit| edit.span.start != v9_declaration)
    );
    assert!(plan.edits.iter().all(|edit| edit.span.start != v9_use));
}

#[test]
fn shape_refinement_drives_hover_and_definition_identity() {
    use splitscript::tooling::database::{CompilerDatabase, DefinitionTarget};

    let source = r#"
        enum Build { V8, V9 }
        let build: Build
        state "game.exe" {
            if build == Build.V8 { bike: i16 at 0x100; }
            else { bike: u16 at 0x200; }
        }
        onAttach { build = Build.V8 }
        split {
            return match build {
                Build.V8 => current.bike == 1,
                Build.V9 => current.bike == 2,
            }
        }
    "#;
    let uses = source
        .match_indices("current.bike")
        .map(|(offset, _)| offset + "current.".len())
        .collect::<Vec<_>>();
    let mut database = CompilerDatabase::new(source);
    let v8_hover = database.hover(uses[0]).unwrap().unwrap();
    let v9_hover = database.hover(uses[1]).unwrap().unwrap();
    assert!(
        v8_hover.markdown.contains("bike: i16"),
        "{}",
        v8_hover.markdown
    );
    assert!(
        v9_hover.markdown.contains("bike: u16"),
        "{}",
        v9_hover.markdown
    );

    let DefinitionTarget::Source(v8) = database.definition_at(uses[0]).unwrap().unwrap() else {
        panic!("V8 field should navigate to source")
    };
    let DefinitionTarget::Source(v9) = database.definition_at(uses[1]).unwrap().unwrap() else {
        panic!("V9 field should navigate to source")
    };
    assert_ne!(v8.id, v9.id);
    assert_eq!(&source[v8.span.start..v8.span.end], "bike");
    assert_eq!(&source[v9.span.start..v9.span.end], "bike");
}

#[test]
fn instance_enumeration_ignores_unused_address_array_declarations() {
    let source = include_str!("../managed_instances_runtime.split");
    let unused = format!("fn unusedAddresses() -> [address] {{ return [] }}\n{source}");
    for profile in [
        splitscript::BuildProfile::Debug,
        splitscript::BuildProfile::Release,
    ] {
        for source in [source, unused.as_str()] {
            let wasm = splitscript::compile_with_options(
                source,
                splitscript::CompilerOptions {
                    profile,
                    ..Default::default()
                },
            )
            .unwrap();
            Validator::new_with_features(WasmFeatures::all())
                .validate_all(&wasm)
                .unwrap();
        }
    }
}

#[test]
fn managed_collection_constraints_use_owned_types_only_within_field_schemas() {
    let schema = r#"
        image "Assembly-CSharp" { class Root { static Set<List<String>> values; } }
    "#;
    for source in [
        format!("{schema} state Unity [\"game.exe\"] {{ values: Set<[String]> = Root.values?; }}"),
        format!("{schema} state \"game.exe\" {{}}"),
    ] {
        let checked = splitscript::check(splitscript::parse(&source).unwrap()).unwrap();
        let field = checked.syntax().managed_class_declarations()[0].fields[0].id;
        let storage = checked.semantics().managed_field_type(field).unwrap();
        assert!(checked.capabilities().has(
            storage,
            splitscript::compiler::stdlib::StdlibCapabilityId::ManagedReadable,
            checked.semantics()
        ));
        let wasm = splitscript::codegen(&checked);
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&wasm)
            .unwrap();
    }
    // Both uses can share one interned syntax application. The ordinary use
    // must not inherit the field's interpretation of List as remote storage.
    let ordinary = "fn accepts(value: Set<List<String>>) {}";
    for source in [
        format!("{schema} {ordinary}"),
        format!("{ordinary} {schema}"),
    ] {
        let source = format!("{source} state \"game.exe\" {{}}");
        let errors = splitscript::compile(&source)
            .map(|wasm| wasm.len())
            .unwrap_err();
        let ordinary_set = source.find("fn accepts").unwrap() + ordinary.find("Set<").unwrap();
        assert!(errors.iter().any(|error| error.span.start == ordinary_set));
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("Equatable")
                    || error.message.contains("equality")),
            "{errors:?}"
        );
    }
}

#[test]
fn owned_collection_constraints_reject_non_equatable_nested_values() {
    for field in [
        "Set<List<Root.Ref>>",
        "Map<List<Root.Ref>, i32>",
        "Set<Map<String, Root.Ref>>",
    ] {
        let source = format!(
            r#"
            image "Assembly-CSharp" {{ class Root {{ static {field} values; }} }}
            state "game.exe" {{}}
        "#
        );
        let errors = splitscript::compile(&source)
            .map(|wasm| wasm.len())
            .unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("equality")),
            "{field}: {errors:?}"
        );
    }
    // The completed semantic proof also checks ordinary annotations rather
    // than waiting for a constructor or a lookup to force the constraint.
    let source = r#"
        image "Assembly-CSharp" { class Root { Root.Ref live; } }
        state "game.exe" {}
        fn accepts(values: Set<[Root]>) {}
    "#;
    let errors = splitscript::compile(source)
        .map(|wasm| wasm.len())
        .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("equality")),
        "{errors:?}"
    );
}

#[test]
fn managed_error_paths_follow_observed_payloads() {
    for (ty, read, field, index) in [
        ("Child", "root.snapshot()", true, false),
        ("[String?]", "Probe.values", false, true),
        ("List<[String?]>", "Probe.values", false, true),
        ("Map<String, Child>", "Probe.values", true, true),
        ("Set<[String?]>", "Probe.values", false, true),
    ] {
        for observed in [false, true] {
            let action = if observed {
                format!(
                    r#"setVariable("error", match {read} {{ Ok(_) => "ok", Err(error) => error }})"#
                )
            } else {
                format!("let value = {read} else return")
            };
            let prefix = if ty == "Child" {
                "let root = Probe.values else return"
            } else {
                ""
            };
            let action = format!("{prefix}\n{action}");
            let source = format!(
                r#"
                image "Assembly-CSharp" {{
                    class Probe {{ static {ty} values; static Map<String, Child> unused; }}
                    class Child {{ String text; }}
                }}
                state Unity.il2cpp(Il2CppProfile.unity2022_3_0f1X64()) ["game.exe"] {{}}
                whileAttached {{ {action} }}
            "#
            );
            let checked = splitscript::check(splitscript::parse(&source).unwrap()).unwrap();
            let (wasm, report) = splitscript::compiler::codegen_with_report(
                &checked,
                splitscript::CompilerOptions {
                    profile: splitscript::BuildProfile::Release,
                    ..Default::default()
                },
            );
            Validator::new_with_features(WasmFeatures::all())
                .validate_all(&wasm)
                .unwrap();
            if field && !index {
                assert!(
                    !report
                        .runtime_helpers
                        .iter()
                        .any(|helper| helper == "FormatI64")
                );
            }
            for (name, used) in [("ManagedErrorField", field), ("ManagedErrorIndex", index)] {
                assert_eq!(
                    report.runtime_helpers.iter().any(|helper| helper == name),
                    observed && used,
                    "{ty}, observed={observed}: {name}"
                );
            }
        }
    }
}

#[test]
fn collection_storage_resolvers_follow_the_selected_backend() {
    for (selector, mono) in [
        ("Unity.mono(MonoVersion.V2)", true),
        ("Unity.il2cpp(Il2CppProfile.unity2022_3_0f1X64())", false),
    ] {
        for collection in [
            "i32",
            "String",
            "[String]",
            "List<String>",
            "Map<String, String>",
            "Set<String>",
        ] {
            let has_array = collection.starts_with('[');
            let has_collection = collection.contains('<') || has_array;
            let source = format!(
                r#"
                image "Assembly-CSharp" {{ class Probe {{ static {collection} values; static [String] unused; }} }}
                state {selector} ["game.exe"] {{ values = Probe.values?; }}
            "#
            );
            let checked = splitscript::check(splitscript::parse(&source).unwrap()).unwrap();
            let (wasm, report) = splitscript::compiler::codegen_with_report(
                &checked,
                splitscript::CompilerOptions {
                    profile: splitscript::BuildProfile::Release,
                    ..Default::default()
                },
            );
            Validator::new_with_features(WasmFeatures::all())
                .validate_all(&wasm)
                .unwrap();
            assert_eq!(
                report
                    .functions
                    .iter()
                    .any(|(_, name)| name.contains("MonoGenericSize")),
                mono && has_collection,
                "{selector}: {collection}"
            );
            assert_eq!(
                report
                    .functions
                    .iter()
                    .any(|(_, name)| name.contains("VerifyArrayType")),
                collection.starts_with("Map<") || collection.starts_with("Set<"),
                "{selector}: {collection}"
            );
            assert_eq!(
                report
                    .functions
                    .iter()
                    .any(|(_, name)| name.contains("ArrayLayout")),
                has_array || collection.starts_with("List<"),
                "{selector}: {collection}"
            );
            assert_eq!(
                report
                    .functions
                    .iter()
                    .any(|(_, name)| name.contains("Il2CppGenericSize")),
                !mono && has_collection,
                "{selector}: {collection}"
            );
            assert_eq!(
                report
                    .functions
                    .iter()
                    .any(|(_, name)| name.contains("Il2CppTypeTable")),
                !mono && has_collection,
                "{selector}: {collection}"
            );
        }
    }
}
