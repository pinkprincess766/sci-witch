//! Compositional chemical nomenclature: coordination compounds, hydrates,
//! oxidation states and a small ontology of material classes.
//!
//! Everything here goes through the **same public path the CLI and the
//! application use** — `interpret_utterance` for what a user would see, and
//! `interpret` where the AST itself is what is being checked. Nothing calls
//! a private helper, so a test passing here means the feature works where it
//! ships.
//!
//! Every expected value was written from what the name means, not pasted
//! from a run. Where a Russian form was chosen over an equally defensible
//! one, the choice is stated in
//! `docs/development/CHEMISTRY_NOMENCLATURE_RU.md` and pinned below.

use sciwhisper_core::ast::{Node, Part};
use sciwhisper_core::coordination::{
    build_sphere, salt_ratio, Coordination, Ligand, MaterialClasses, Refusal, MAX_COUNTER_IONS,
    MAX_LIGAND_MULTIPLICITY,
};
use sciwhisper_core::utterance::MAX_UTTERANCE_WORDS;
use sciwhisper_core::{
    interpret, interpret_utterance, render, Domain, InterpretOptions, Renderer, UtteranceMode,
    UtteranceOptions,
};

/// What the user sees: the shipped path, default mode.
fn spoken(text: &str) -> String {
    let result = interpret_utterance(
        text,
        UtteranceOptions {
            domain: Domain::Auto,
            mode: UtteranceMode::MixedText,
            allow_shortcuts: true,
        },
    );
    render(&result.document, Renderer::Unicode)
}

fn compiled(text: &str) -> sciwhisper_core::InterpretationResult {
    interpret(
        text,
        InterpretOptions {
            domain: Domain::Auto,
            ..Default::default()
        },
    )
}

fn rendered(text: &str, renderer: Renderer) -> String {
    render(&compiled(text).ast, renderer)
}

/// Whether the whole utterance came back as the words that went in.
fn stayed_prose(text: &str) -> bool {
    spoken(text).trim() == text.trim()
}

// ------------------------------------------------------- coordination: yes

#[test]
fn coordination_compounds_are_built_from_their_components() {
    let cases = [
        // The number names the oxidation state of the centre, as in
        // «гексацианоферрат(III) калия». Fe(III) + 6·CN⁻ = −3, so three K⁺.
        ("гексацианоферрат три калия", "K₃[Fe(CN)₆]"),
        // The counterfactual: one word changes, one number in the AST changes.
        // Fe(II) + 6·CN⁻ = −4, so four K⁺.
        ("гексацианоферрат два калия", "K₄[Fe(CN)₆]"),
        // No oxidation state spoken: zinc has only one, so it is not a guess.
        ("тетрагидроксоцинкат натрия", "Na₂[Zn(OH)₄]"),
        // A complex cation carries its charge instead of a counter-ion.
        ("ион тетрамминмеди два", "[Cu(NH₃)₄]²⁺"),
        // Neutral ligands, trivalent centre.
        ("ион гексаакважелеза три", "[Fe(H₂O)₆]³⁺"),
    ];
    for (said, expected) in cases {
        assert_eq!(spoken(said), expected, "{said}");
    }
}

/// One word differs, and exactly one number in the tree differs with it.
#[test]
fn the_counterfactual_pair_differs_in_one_place() {
    let three = compiled("гексацианоферрат три калия").ast;
    let two = compiled("гексацианоферрат два калия").ast;
    assert_ne!(three, two);

    let sphere = |node: &Node| -> (i32, u32) {
        let Node::Chemical(sciwhisper_core::ast::Chemical::Species(species)) = node else {
            panic!("expected a species, got {node:?}")
        };
        let counter = species
            .formula
            .parts
            .iter()
            .find_map(|part| match part {
                Part::Atom { symbol, count } if symbol == "K" => Some(*count),
                _ => None,
            })
            .expect("potassium counter-ion");
        let complex = species
            .formula
            .parts
            .iter()
            .find_map(|part| match part {
                Part::Complex(complex) => Some(complex),
                _ => None,
            })
            .expect("coordination sphere");
        assert_eq!(complex.center.symbol, "Fe");
        assert_eq!(complex.ligands.len(), 1);
        assert_eq!(complex.ligands[0].count, 6);
        (complex.center.oxidation, counter)
    };
    // The oxidation state and the counter-ion count are the only things that
    // moved; the centre, the ligand and its multiplicity are untouched.
    assert_eq!(sphere(&three), (3, 3));
    assert_eq!(sphere(&two), (2, 4));
}

/// The AST must be able to *say* what it was built from, or a validator
/// would have to consult the ligand table again and re-decide chemistry.
#[test]
fn the_tree_carries_the_evidence_for_its_own_charge() {
    let Node::Chemical(sciwhisper_core::ast::Chemical::Species(species)) =
        compiled("гексацианоферрат три калия").ast
    else {
        panic!("expected a species")
    };
    let complex = species
        .formula
        .parts
        .iter()
        .find_map(|part| match part {
            Part::Complex(complex) => Some(complex),
            _ => None,
        })
        .expect("coordination sphere");
    assert_eq!(complex.center.oxidation, 3);
    assert_eq!(complex.ligands[0].charge, -1);
    assert_eq!(complex.charge, -3);
    // z(centre) + Σ nᵢ·zᵢ = z(sphere), checkable without the lexicon.
    assert_eq!(complex.charge_balances(), Some(true));
}

#[test]
fn every_renderer_shows_the_same_compound() {
    assert_eq!(
        rendered("гексацианоферрат три калия", Renderer::Unicode),
        "K₃[Fe(CN)₆]"
    );
    assert_eq!(
        rendered("гексацианоферрат три калия", Renderer::Latex),
        r"\ce{K3[Fe(CN)6]}"
    );
    assert_eq!(
        rendered("ион тетрамминмеди два", Renderer::Unicode),
        "[Cu(NH₃)₄]²⁺"
    );
    assert_eq!(
        rendered("ион тетрамминмеди два", Renderer::Latex),
        r"\ce{[Cu(NH3)4]^{2+}}"
    );

    // OMML is XML, so it is checked for the structure Word needs rather than
    // as one long string: the brackets are runs and the counts are real
    // subscripts, not characters that merely look like them.
    let omml = rendered("гексацианоферрат три калия", Renderer::Omml);
    assert!(omml.starts_with("<m:oMath"), "{omml}");
    assert!(omml.contains("<m:sSub>"), "counts must be subscripts");
    assert!(omml.contains(">[<"), "the sphere must be bracketed");
    assert!(omml.contains(">]<"), "the sphere must be closed");
    // Two subscripts: the three potassiums and the six cyanides.
    assert_eq!(omml.matches("<m:sSub>").count(), 2);
}

// ------------------------------------------------------------------ hydrates

#[test]
fn hydrates_are_a_counted_part_of_the_tree() {
    assert_eq!(spoken("сульфат меди два пентагидрат"), "CuSO₄·5H₂O");
    assert_eq!(spoken("карбонат натрия декагидрат"), "Na₂CO₃·10H₂O");
    assert_eq!(spoken("сульфат меди два моногидрат"), "CuSO₄·H₂O");

    // The water count is a number in the tree, not a string glued to a
    // formula: nothing downstream has to re-read «пентагидрат».
    let Node::Chemical(sciwhisper_core::ast::Chemical::Species(species)) =
        compiled("сульфат меди два пентагидрат").ast
    else {
        panic!("expected a species")
    };
    assert_eq!(
        species.formula.parts.last(),
        Some(&Part::Hydrate { count: 5 })
    );
    // And the atoms it contributes are counted.
    let atoms = species.formula.atom_counts().expect("no overflow");
    assert_eq!(atoms["H"], 10);
    assert_eq!(atoms["O"], 9); // four from the sulfate, five from the water
}

#[test]
fn a_hydrate_of_nothing_is_not_a_compound() {
    assert!(stayed_prose("пентагидрат"));
    assert!(stayed_prose("декагидрат"));
}

// ---------------------------------------------------------- oxidation states

#[test]
fn an_oxidation_state_is_understood_however_it_is_said() {
    // All four name Fe(III) in a context where nothing else it could be.
    for said in [
        "гидроксид железа три",
        "гидроксид железа плюс три",
        "гидроксид железа римское три",
        "гидроксид железа в степени окисления три",
    ] {
        assert_eq!(spoken(said), "Fe(OH)₃", "{said}");
    }
    // Roman numerals as symbols, for when the recogniser writes them.
    assert_eq!(spoken("гидроксид железа III"), "Fe(OH)₃");
    assert_eq!(spoken("гексацианоферрат III калия"), "K₃[Fe(CN)₆]");
}

/// An oxidation state, a charge and a subscript are three different things
/// and must not be confused. A bare number after a lone element could be
/// either a subscript («о два» → O₂) or an oxidation state, so the phrasings
/// that mean an oxidation state and nothing else are the ones extended.
#[test]
fn an_ambiguous_bare_number_is_left_as_it_was() {
    // Documented, not asserted as desirable: a lone «железо три» keeps its
    // existing spelled-formula reading, because the same code path is what
    // turns «о два» into O₂ and narrowing it would break that.
    assert_eq!(spoken("железо три"), "Fe₃");
    assert_eq!(spoken("о два"), "O₂");
}

// ------------------------------------------------------------ material class

#[test]
fn a_material_class_applies_only_where_its_template_is_proved() {
    for said in [
        "феррит цинка",
        "феррита цинка",
        "цинковый феррит",
        "феррит Zn",
    ] {
        assert_eq!(spoken(said), "ZnFe₂O₄", "{said}");
    }
    assert_eq!(spoken("два феррита цинка"), "2ZnFe₂O₄");
}

/// Barium has **both** known phases — the spinel `BaFe₂O₄` and the
/// hexaferrite `BaFe₁₂O₁₉`. So «феррит бария» does not name a single
/// structure at all: a rule that read «феррит любого металла» would have to
/// pick one with no grounds for preferring it. Refusing is the answer.
#[test]
fn a_metal_outside_the_template_is_refused_rather_than_guessed() {
    assert!(stayed_prose("феррит бария"), "{}", spoken("феррит бария"));
    assert!(stayed_prose("феррит меди"));
    assert!(stayed_prose("феррит кобальта"));
}

/// The multivalent refusal, reached directly because no cation in the
/// shipped table is both listed and multivalent — the list is what keeps
/// them out. The rule still has to work, so it is exercised on its own.
#[test]
fn a_multivalent_cation_is_refused_even_where_the_template_lists_it() {
    let classes = MaterialClasses::builtin();
    // Zinc has one state, and it is the one the template names.
    assert!(classes.resolve("spinel_ferrite", "Zn", Some(2)).is_ok());

    // Nothing proved: the template's own number would be taken on trust.
    let refusal = classes
        .resolve("spinel_ferrite", "Zn", None)
        .expect_err("an unproven oxidation state cannot settle the template");
    assert!(
        matches!(refusal, Refusal::ClassNotApplicable { ref reason, .. } if reason.contains("многовалент")),
        "{refusal}"
    );

    // Proved, but not what the template requires.
    let refusal = classes
        .resolve("spinel_ferrite", "Zn", Some(3))
        .expect_err("a state that disagrees with the template must be refused");
    assert!(
        matches!(refusal, Refusal::ClassNotApplicable { ref reason, .. } if reason.contains("доказана 3")),
        "{refusal}"
    );
}

#[test]
fn the_applied_template_can_be_named() {
    let classes = MaterialClasses::builtin();
    let (_, proof) = classes.resolve("spinel_ferrite", "Zn", Some(2)).unwrap();
    assert_eq!(proof.class, "spinel_ferrite");
    assert_eq!(proof.structure, "M(II)Fe2O4");
    assert_eq!(proof.cation, "Zn");
    assert_eq!(proof.oxidation, 2);
    assert!(!proof.source.is_empty(), "a template needs a source");
}

// ------------------------------------------------ ordinary prose stays prose

/// Chemistry words in an ordinary sentence are still an ordinary sentence.
///
/// «Not rewritten» means two different things, and they are asserted
/// separately because the system promises only one of them.
#[test]
fn a_chemical_word_in_prose_does_not_make_the_sentence_a_formula() {
    for said in [
        "мы обсуждали ферриты цинка",
        "феррит оказался нестабильным",
        "комплекс меди изучали вчера",
        "координационное число неизвестно",
        "гексациано соединение",
        "железо трижды промыли водой",
        "лиганд был выбран заранее",
        "степень окисления обсудим позже",
        "гидрат образовался при охлаждении",
    ] {
        assert!(stayed_prose(said), "{said:?} → {:?}", spoken(said));
    }
}

/// **A known substance name inside prose is replaced, by design.**
///
/// `MixedText` — the default mode, and the one the application ships with —
/// keeps the sentence and substitutes the spans it can prove. So «сульфат
/// меди был куплен вчера» becomes «CuSO₄ был куплен вчера»: the prose
/// survives, the substance name does not.
///
/// Whether that is right is a product question, not one this test can
/// settle. `NATURAL_DICTATION_RU.md` asked for exactly this behaviour; a
/// corpus record tagged `substance-mentioned` asks for the opposite, and the
/// evaluation harness measures a code path where the question does not
/// arise. The same tension is recorded in
/// `crates/sciwhisper-eval/src/stability.rs`.
///
/// What is asserted here is what the system does promise and what this task
/// required: nothing is invented, and the sentence around the name is
/// untouched.
#[test]
fn a_substance_name_in_prose_is_substituted_but_nothing_is_invented() {
    let cases = [
        ("сульфат меди был куплен вчера", "CuSO₄ был куплен вчера"),
        (
            "феррит цинка был получен вчера",
            "ZnFe₂O₄ был получен вчера",
        ),
    ];
    for (said, expected) in cases {
        let out = spoken(said);
        assert_eq!(out, expected, "{said}");
        // Every ordinary word survives; only the name became a formula.
        for word in said.split_whitespace().skip(2) {
            assert!(out.contains(word), "{said:?} lost {word:?} → {out:?}");
        }
    }
}

// --------------------------------------------------------- refusals by cause

#[test]
fn an_incomplete_or_impossible_name_is_refused() {
    for said in [
        // no centre after the ligand
        "гексациано",
        // a ligand nobody defined
        "гексаксеноферрат калия",
        // a centre nobody defined
        "гексацианоуглерод калия",
        // no counter-ion named
        "гексацианоферрат три",
    ] {
        assert!(stayed_prose(said), "{said:?} → {:?}", spoken(said));
    }

    // A counter-ion that is not an element. The coordination name itself
    // must not become a formula; what the span search does with the
    // «три вода» tail is a separate, pre-existing question about spans and
    // is recorded in the report rather than asserted here.
    let out = spoken("гексацианоферрат три вода");
    assert!(!out.contains('['), "{out}");
    assert!(!out.contains("Fe"), "{out}");
}

/// A centre whose oxidation state the speaker did not give, and which has
/// more than one, cannot be settled by the parser.
#[test]
fn a_multivalent_centre_without_a_stated_state_is_refused() {
    // Copper is +1 and +2; «гексацианокупрат натрия» does not say which.
    assert!(stayed_prose("гексацианокупрат натрия"));
    // Saying it makes the name answerable.
    assert_eq!(spoken("гексацианокупрат два натрия"), "Na₄[Cu(CN)₆]");
}

// ------------------------------------------------------------ hard limits

#[test]
fn every_limit_is_a_number_and_is_enforced() {
    let coordination = Coordination::builtin();
    let cyanide = Ligand {
        id: "cyano".into(),
        formula: sciwhisper_core::ast::Formula::atom("C", 1),
        charge: -1,
        max_multiplicity: MAX_LIGAND_MULTIPLICITY,
    };

    // Zero is not a smaller compound, it is a different claim.
    assert_eq!(
        build_sphere("Fe", 3, &cyanide, 0),
        Err(Refusal::ZeroMultiplicity("лигандов"))
    );
    // Above the supported coordination number.
    assert!(matches!(
        build_sphere("Fe", 3, &cyanide, MAX_LIGAND_MULTIPLICITY + 1),
        Err(Refusal::OutOfRange { .. })
    ));
    // Counter-ions beyond the limit.
    assert!(matches!(
        salt_ratio(-(MAX_COUNTER_IONS as i32) - 1, 1),
        Err(Refusal::OutOfRange { .. })
    ));
    // `i32::MIN` has no positive counterpart: `abs()` panics on it in debug
    // and wraps back to itself in release, so the same call either killed
    // the process or produced a magnitude that `as u32` turned into two
    // billion counter-ions. Both are now one refusal.
    assert!(
        matches!(salt_ratio(i32::MIN, 1), Err(Refusal::OutOfRange { .. })),
        "salt_ratio(i32::MIN, 1) = {:?}",
        salt_ratio(i32::MIN, 1)
    );
    assert!(matches!(
        salt_ratio(1, i32::MIN),
        Err(Refusal::OutOfRange { .. })
    ));
    // And the magnitude is read correctly rather than wrapping: i32::MIN
    // against a charge that divides it evenly still exceeds the limit.
    assert!(matches!(
        salt_ratio(i32::MIN, 2),
        Err(Refusal::OutOfRange { .. })
    ));
    // A neutral sphere has no salt to form.
    assert!(matches!(
        salt_ratio(0, 1),
        Err(Refusal::ChargeUnsatisfiable { .. })
    ));
    // Two charges of the same sign cannot balance.
    assert!(matches!(
        salt_ratio(3, 1),
        Err(Refusal::ChargeUnsatisfiable { .. })
    ));
    // Exact arithmetic. The previous version of this assertion accepted
    // `Ok(_) | Err(Overflow(_))`, which is every possible outcome — it could
    // not fail. A cationic ligand (NO⁺ is one) pushes the sphere charge past
    // `i32::MAX`, and the result must be exactly an overflow refusal.
    let nitrosyl = Ligand {
        id: "nitrosyl".into(),
        formula: sciwhisper_core::ast::Formula::atom("N", 1),
        charge: 1,
        max_multiplicity: MAX_LIGAND_MULTIPLICITY,
    };
    assert_eq!(
        build_sphere("Fe", i32::MAX, &nitrosyl, 1),
        Err(Refusal::Overflow("заряд сферы"))
    );
    // And one step below the edge still works, so the test is measuring the
    // boundary rather than refusing everything.
    assert!(build_sphere("Fe", i32::MAX - 1, &nitrosyl, 1).is_ok());

    // The hydrate limit is a number too.
    // «декагидрат» is ten waters, so the limit has to admit it.
    assert_eq!(coordination.hydrate("декагидрат"), Some(Ok(10)));
}

#[test]
fn the_salt_ratio_is_the_smallest_whole_one() {
    // Fe(III) hexacyanide is −3 against K⁺: three potassiums, one sphere.
    assert_eq!(salt_ratio(-3, 1), Ok((3, 1)));
    // −4 against +2 reduces to 2:1, not 4:2.
    assert_eq!(salt_ratio(-4, 2), Ok((2, 1)));
    // −2 against +3 has no smaller form than 3:2.
    // Sphere −2 against counter +3: two counter-ions to three spheres, which
    // is `(counters, spheres)` and not the other way round.
    assert_eq!(salt_ratio(-2, 3), Ok((2, 3)));
}

/// A phrase past the utterance limit must take the deterministic early-exit
/// path instead of entering the bounded span search. Wall-clock assertions
/// are deliberately avoided: runner speed is not part of this contract.
#[test]
fn a_very_long_phrase_does_not_run_away() {
    let long = std::iter::repeat_n("феррит цинка", MAX_UTTERANCE_WORDS / 2 + 1)
        .collect::<Vec<_>>()
        .join(" ");
    assert!(long.split_whitespace().count() > MAX_UTTERANCE_WORDS);
    assert_eq!(spoken(&long), long);
}

// ------------------------------------------------------------- data files

#[test]
fn the_shipped_tables_load_and_state_their_limits() {
    let coordination = Coordination::builtin();
    assert!(coordination.is_ion_lead_in("ион"));
    // A partial decomposition is not a coordination name.
    assert!(coordination.split("гексациано").is_err());
    assert!(coordination.split("гексацианоферрат").is_ok());
    assert!(matches!(
        coordination.split("вода"),
        Err(Refusal::NotCoordination)
    ));

    // A material class without a list of cations would be the "any metal"
    // rule this design refuses to have.
    let broken = r#"
schema_version: 1
classes:
  - id: bad
    names: [нечто]
    structure: "MX"
    skeleton: [{symbol: O, count: 1}]
    allowed_cations: []
    refusals:
      cation_not_listed: x
      cation_multivalent: y
"#;
    let error = MaterialClasses::load(broken).expect_err("must be refused");
    assert!(error.to_string().contains("любой металл"), "{error}");
}

// ------------------------------------------------ corrective round v2

/// A number that cannot be an oxidation state must leave the words alone —
/// not answer with a different number, and not build half a formula.
///
/// Each of these produced something before the fix:
/// `минус три` → `Fe(OH)₃` (the sign dropped), `ноль` → `Fe` (the hydroxide
/// deleted), `2147483648` → a panic in `i32::abs`, `4294967295` → `FeOH`
/// (two charges of the same sign, balancing nothing).
#[test]
fn an_impossible_oxidation_state_leaves_the_words_alone() {
    for said in [
        "гидроксид железа минус три",
        "гидроксид железа ноль",
        "гидроксид железа 2147483648",
        "гидроксид железа 4294967295",
    ] {
        assert!(stayed_prose(said), "{said:?} → {:?}", spoken(said));
    }
}

/// The same four through `interpret`, which is where the arithmetic lives.
/// No panic, and no formula.
#[test]
fn the_arithmetic_refuses_instead_of_panicking() {
    for said in [
        "гидроксид железа минус три",
        "гидроксид железа ноль",
        "гидроксид железа 2147483648",
        "гидроксид железа 4294967295",
        "гидроксид железа 4294967296",
    ] {
        let result = compiled(said);
        assert_eq!(result.confidence, 0.0, "{said:?} produced {:?}", result.ast);
    }
}

/// A negative oxidation state is impossible for a cation in a simple salt
/// and perfectly ordinary for a coordination centre. The rule has to know
/// which it is looking at.
#[test]
fn a_negative_oxidation_state_depends_on_where_it_is_read() {
    use sciwhisper_core::coordination::{oxidation_from, OxidationContext, OxidationRead};
    assert!(matches!(
        oxidation_from(3, -1, OxidationContext::SimpleSalt),
        OxidationRead::Refused(_)
    ));
    assert!(matches!(
        oxidation_from(3, -1, OxidationContext::Coordination),
        OxidationRead::Found { value: -3, .. }
    ));
    // Zero is a cation nowhere.
    assert!(matches!(
        oxidation_from(0, 1, OxidationContext::SimpleSalt),
        OxidationRead::Refused(_)
    ));
    // A number too large for the type is refused, not wrapped.
    assert!(matches!(
        oxidation_from(u32::MAX, 1, OxidationContext::SimpleSalt),
        OxidationRead::Refused(_)
    ));
    assert!(matches!(
        oxidation_from(2_147_483_648, 1, OxidationContext::Coordination),
        OxidationRead::Refused(_)
    ));
}

/// A subscript of four billion is not a formula, and a subscript of zero is
/// not one atom.
#[test]
fn a_spoken_subscript_is_bounded() {
    assert_eq!(spoken("о два"), "O₂");
    assert!(stayed_prose("железо ноль"));
    assert!(stayed_prose("железо 4294967295"));
}

/// A repeated single-atom ligand is one subscript, not a symbol with a
/// digit next to it. Word renders the second as literal text.
#[test]
fn a_single_atom_ligand_is_subscripted_in_word() {
    assert_eq!(spoken("тетрахлоридокупрат два калия"), "K₂[CuCl₄]");
    let omml = rendered("тетрахлоридокупрат два калия", Renderer::Omml);
    // Two subscripts: the two potassiums and the four chlorides.
    assert_eq!(omml.matches("<m:sSub>").count(), 2, "{omml}");
    // And the chloride's count is inside a subscript, not beside it.
    assert!(
        omml.contains(
            r#"<m:sSub><m:e><m:r><m:t xml:space="preserve">Cl</m:t></m:r></m:e><m:sub><m:r><m:t xml:space="preserve">4</m:t></m:r></m:sub></m:sSub>"#
        ),
        "{omml}"
    );
    assert!(
        !omml.contains(r#">Cl</m:t></m:r><m:r><m:t xml:space="preserve">4<"#),
        "the count must not be a run of its own: {omml}"
    );
}

/// The 2005 recommendations are the primary form; the pre-2005 spellings are
/// aliases, and neither is presented as the only one supported.
#[test]
fn both_the_iupac_2005_form_and_the_colloquial_one_are_understood() {
    let pairs = [
        ("гексацианидоферрат три калия", "гексацианоферрат три калия"),
        ("тетрагидроксидоцинкат натрия", "тетрагидроксоцинкат натрия"),
        ("тетрахлоридокупрат два калия", "тетрахлорокупрат два калия"),
    ];
    for (official, colloquial) in pairs {
        let a = spoken(official);
        let b = spoken(colloquial);
        assert_eq!(a, b, "{official} vs {colloquial}");
        assert!(!a.contains(' '), "{official} did not compile: {a}");
    }
}

/// A material template is a claim about structure. These are the ways a
/// table can make one by accident, and each is refused at load.
#[test]
fn a_material_table_cannot_claim_something_by_accident() {
    let base = |body: &str| {
        format!("schema_version: 1\nsources:\n  s: http://example.invalid\nclasses:\n{body}")
    };
    let good = base(
        "  - id: c\n    names: [нечто]\n    structure: MX\n    skeleton: [{symbol: O, count: 1}]\n\
         \n    allowed_cations: [{symbol: Zn, oxidation: 2, source: s}]\n    refusals: {cation_not_listed: x, cation_multivalent: y}\n",
    );
    MaterialClasses::load(&good).expect("the baseline table must load");

    let cases = [
        // a zero atom count is a different composition, not the same one
        (
            "skeleton: [{symbol: O, count: 1}]",
            "skeleton: [{symbol: O, count: 0}]",
            "нулевое число атомов",
        ),
        // a source that is not declared
        ("source: s}", "source: missing}", "не объявлен"),
        // something that is not an element symbol
        (
            "{symbol: O, count: 1}",
            "{symbol: water, count: 1}",
            "символ элемента",
        ),
        (
            "{symbol: Zn, oxidation: 2",
            "{symbol: zN, oxidation: 2",
            "символ элемента",
        ),
    ];
    for (from, to, expected) in cases {
        let broken = good.replace(from, to);
        assert_ne!(broken, good, "the substitution {from:?} did not apply");
        let error = MaterialClasses::load(&broken)
            .expect_err(&format!("{to} must be refused"))
            .to_string();
        assert!(error.contains(expected), "{to}: {error}");
    }

    // Two classes answering to one word: which wins would depend on line order.
    let duplicated = base(
        "  - id: a\n    names: [нечто]\n    structure: MX\n    skeleton: [{symbol: O, count: 1}]\n\
         \n    allowed_cations: [{symbol: Zn, oxidation: 2, source: s}]\n    refusals: {cation_not_listed: x, cation_multivalent: y}\n\
         \n  - id: b\n    names: [нечто]\n    structure: MY\n    skeleton: [{symbol: S, count: 1}]\n\
         \n    allowed_cations: [{symbol: Zn, oxidation: 2, source: s}]\n    refusals: {cation_not_listed: x, cation_multivalent: y}\n",
    );
    let error = MaterialClasses::load(&duplicated)
        .expect_err("two classes cannot share a name")
        .to_string();
    assert!(error.contains("двум классам"), "{error}");
}

// --------------------------------------------------------------- organic

/// Six dictionary entries covered метан…бутан and stopped. Two small tables
/// now cover thirty names, and the formula follows from the name by
/// arithmetic rather than from a lookup.
#[test]
fn hydrocarbons_are_built_from_the_name_not_looked_up() {
    let cases = [
        ("метан", "CH₄"),
        ("этан", "C₂H₆"),
        ("пропан", "C₃H₈"),
        ("бутан", "C₄H₁₀"),
        // Everything from here was not understood before.
        ("пентан", "C₅H₁₂"),
        ("гексан", "C₆H₁₄"),
        ("гептан", "C₇H₁₆"),
        ("октан", "C₈H₁₈"),
        ("нонан", "C₉H₂₀"),
        ("декан", "C₁₀H₂₂"),
        ("этен", "C₂H₄"),
        ("пропен", "C₃H₆"),
        ("бутен", "C₄H₈"),
        ("этин", "C₂H₂"),
        ("пропин", "C₃H₄"),
    ];
    for (said, expected) in cases {
        assert_eq!(spoken(said), expected, "{said}");
    }
    assert_eq!(spoken("два пентана"), "2C₅H₁₂");
}

/// Before «-ен» and «-ин» the ten-carbon stem is written «дец», not «дек».
///
/// Both spellings are accepted, and that is not a guess: «декен» has
/// exactly one possible reading, so refusing it over an orthographic detail
/// would lose a correct utterance. The same reasoning admits «феррит Zn»
/// beside «феррит цинка».
#[test]
fn both_spellings_of_the_ten_carbon_stem_reach_the_same_compound() {
    assert_eq!(spoken("декан"), "C₁₀H₂₂");
    assert_eq!(spoken("децен"), "C₁₀H₂₀");
    assert_eq!(spoken("децин"), "C₁₀H₁₈");
    assert_eq!(spoken("декен"), spoken("децен"));
    assert_eq!(spoken("декин"), spoken("децин"));
}

/// The names that do not exist, and the reason is arithmetic: «метен» would
/// be CH₂ and «метин» would be CH₀.
#[test]
fn a_chain_too_short_for_its_class_keeps_the_words() {
    assert!(stayed_prose("метен"));
    assert!(stayed_prose("метин"));
}

/// Trivial names are not derivable from the rule, so they stay in the
/// dictionary — and both spellings still reach the same compound.
#[test]
fn trivial_names_survive_the_move_to_a_rule() {
    assert_eq!(spoken("этилен"), "C₂H₄");
    assert_eq!(spoken("этен"), "C₂H₄");
    assert_eq!(spoken("ацетилен"), "C₂H₂");
    assert_eq!(spoken("этин"), "C₂H₂");
}

/// A molecular formula is not a structure. Isobutane and butane share
/// C₄H₁₀, so answering «изобутан» with it would claim an understanding of
/// structure this build does not have.
#[test]
fn a_structural_name_is_refused_rather_than_answered_with_an_isomer() {
    for said in ["изобутан", "неопентан", "циклогексан", "2-метилпропан"]
    {
        assert!(stayed_prose(said), "{said} → {:?}", spoken(said));
    }
}

/// «Декан» is also a dean, and «пентан кипит…» is a sentence about pentane
/// rather than a dictated formula. A single word inside prose stays prose —
/// the same rule that keeps «вода закипела в чайнике» intact — and the
/// formula is one click away in «Варианты прочтения».
#[test]
fn a_hydrocarbon_name_inside_prose_stays_prose() {
    for said in [
        "декан факультета подписал приказ",
        "декан вчера уехал",
        "пентан кипит при тридцати шести градусах",
        "мы обсуждали октан и его изомеры",
    ] {
        assert!(stayed_prose(said), "{said} → {:?}", spoken(said));
    }
}

/// Counterfactual: one syllable of the class suffix changes, and exactly
/// the hydrogen count changes with it.
#[test]
fn changing_the_class_changes_only_the_hydrogens() {
    use sciwhisper_core::ast::{Chemical, Formula};
    let read = |said: &str| -> Formula {
        match compiled(said).ast {
            Node::Chemical(Chemical::Species(species)) => species.formula,
            other => panic!("{said}: {other:?}"),
        }
    };
    let carbons = |formula: &Formula| match formula.parts.first() {
        Some(Part::Atom { symbol, count }) if symbol == "C" => *count,
        other => panic!("{other:?}"),
    };
    let hydrogens = |formula: &Formula| match formula.parts.get(1) {
        Some(Part::Atom { symbol, count }) if symbol == "H" => *count,
        other => panic!("{other:?}"),
    };

    let (ane, ene, yne) = (read("пропан"), read("пропен"), read("пропин"));
    assert_eq!(carbons(&ane), 3);
    assert_eq!(
        (carbons(&ene), carbons(&yne)),
        (3, 3),
        "the chain is untouched"
    );
    assert_eq!(
        (hydrogens(&ane), hydrogens(&ene), hydrogens(&yne)),
        (8, 6, 4)
    );
}
