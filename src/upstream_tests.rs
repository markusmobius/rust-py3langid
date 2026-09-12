use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::Value;

use crate::{preprocess::encode, Identifier, LanguageScore, Options};

fn case_bytes(case: &Value) -> Vec<u8> {
    let mut bytes = if let Some(hex) = case["hex"].as_str() {
        (0..hex.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).unwrap())
            .collect()
    } else {
        case["text"]
            .as_str()
            .unwrap_or_default()
            .as_bytes()
            .to_vec()
    };
    let trim = case["trim_bytes"].as_u64().unwrap_or_default() as usize;
    bytes.truncate(bytes.len().saturating_sub(trim));
    bytes
}

#[test]
fn preprocessing_matches_all_42_python_reference_cases() {
    assert_eq!(unicode_normalization::UNICODE_VERSION, (17, 0, 0));
    let corpus: Value =
        serde_json::from_str(include_str!("../testdata/py3langid_cases.json")).unwrap();
    let reference: Value =
        serde_json::from_str(include_str!("../testdata/py3langid_reference.json")).unwrap();
    for key in [
        "commit",
        "version",
        "numpy",
        "go_model_sha256",
        "python_model_sha256",
    ] {
        assert_eq!(corpus[key], reference[key], "fixture provenance: {key}");
    }
    let cases = corpus["cases"].as_array().unwrap();
    let samples = reference["samples"].as_array().unwrap();
    assert_eq!(cases.len(), 42);
    assert_eq!(cases.len(), samples.len());
    for (case, sample) in cases.iter().zip(samples) {
        assert_eq!(case["name"], sample["name"]);
        let bytes = case_bytes(case);
        let expected = STANDARD
            .decode(sample["encoded"].as_str().unwrap())
            .unwrap();
        assert_eq!(encode(&bytes).as_ref(), expected, "case {}", case["name"]);
    }
}

#[test]
fn scoring_matches_all_42_python_reference_cases() {
    let corpus: Value =
        serde_json::from_str(include_str!("../testdata/py3langid_cases.json")).unwrap();
    let reference: Value =
        serde_json::from_str(include_str!("../testdata/py3langid_reference.json")).unwrap();
    for (case, sample) in corpus["cases"]
        .as_array()
        .unwrap()
        .iter()
        .zip(reference["samples"].as_array().unwrap())
    {
        let bytes = case_bytes(case);
        let languages: Vec<_> = case["languages"]
            .as_array()
            .map(|labels| labels.iter().map(|label| label.as_str().unwrap()).collect())
            .unwrap_or_default();
        for normalized in [false, true] {
            let mode = if normalized { "normalized" } else { "raw" };
            let identifier = Identifier::with_options(Options {
                normalized,
                min_confidence: if normalized {
                    case["min_confidence"].as_f64()
                } else {
                    None
                },
            })
            .unwrap();
            identifier.set_languages(&languages).unwrap();
            let context = format!("{} / {mode}", case["name"]);
            let result = identifier.identify(&bytes);
            assert_eq!(
                result.language,
                sample[mode]["language"].as_str().unwrap(),
                "{context}"
            );
            assert_score(
                result.score,
                sample[mode]["score"].as_f64().unwrap(),
                normalized,
                &context,
            );
            let ranked = identifier.rank(&bytes);
            let expected = sample[format!("{mode}_scores")].as_object().unwrap();
            assert_eq!(ranked.len(), identifier.classes().len(), "{context}");
            assert_eq!(ranked.len(), expected.len(), "{context}");
            assert!(
                ranked.windows(2).all(|pair| pair[0].score >= pair[1].score),
                "{context}"
            );
            let labels: std::collections::HashSet<_> =
                ranked.iter().map(|item| &item.language).collect();
            assert_eq!(ranked.len(), labels.len(), "{context}");
            for item in &ranked {
                assert_score(
                    item.score,
                    expected[&item.language].as_f64().unwrap(),
                    normalized,
                    &format!("{context} / {}", item.language),
                );
            }
            if result.language != "und" {
                assert_eq!(result, ranked[0], "{context}");
            }
            if normalized {
                assert!(
                    ranked
                        .iter()
                        .all(|item| (0.0..=1.000_001).contains(&item.score)),
                    "{context}"
                );
                assert!(
                    (ranked.iter().map(|item| item.score).sum::<f64>() - 1.0).abs() <= 1e-6,
                    "{context}"
                );
                assert_eq!(identifier.identify_normalized(&bytes), result, "{context}");
                assert_eq!(identifier.rank_normalized(&bytes), ranked, "{context}");
            }
        }
    }
}

fn assert_score(actual: f64, expected: f64, normalized: bool, context: &str) {
    let (absolute, relative) = if normalized {
        (2e-6, 1e-4)
    } else {
        (1e-5, 1e-5)
    };
    assert!(actual.is_finite(), "{context}: non-finite score {actual}");
    assert!(
        (actual - expected).abs() <= absolute + relative * expected.abs(),
        "{context}: got {actual}, expected {expected}"
    );
}

#[test]
fn restrictions_reset_aliases_and_ties() {
    let identifier = Identifier::new().unwrap();
    let all = identifier.classes();
    assert_eq!(all.len(), 140);
    identifier.set_languages(&["en", "de", "en"]).unwrap();
    assert_eq!(identifier.classes(), ["de", "en"]);
    assert!(identifier.set_languages(&["en", "unknown"]).is_err());
    assert_eq!(identifier.classes(), ["de", "en"]);
    assert_eq!(
        identifier.identify(""),
        LanguageScore {
            language: "de".into(),
            score: -f64::from(f32::MAX)
        }
    );
    assert_eq!(
        identifier
            .rank("")
            .iter()
            .map(|item| item.language.as_str())
            .collect::<Vec<_>>(),
        ["de", "en"]
    );
    identifier.set_languages(&["sr"]).unwrap();
    let ranked = identifier.rank_normalized("ovo je tekst za probu");
    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].language, "sr");
    assert_score(ranked[0].score, 1.0, true, "merged Serbian probability");
    identifier.reset_languages();
    assert_eq!(identifier.classes(), all);
    identifier.set_languages(&["en"]).unwrap();
    identifier.set_languages(&[]).unwrap();
    assert_eq!(identifier.classes(), all);
    assert_eq!(Identifier::new().unwrap().classes(), all);
}

#[test]
fn confidence_and_explicit_normalization() {
    for threshold in [-0.1, 1.1, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(Identifier::with_options(Options {
            normalized: true,
            min_confidence: Some(threshold)
        })
        .is_err());
    }
    assert!(Identifier::with_options(Options {
        normalized: false,
        min_confidence: Some(0.5)
    })
    .is_err());
    let identifier = Identifier::new().unwrap();
    assert!(identifier.identify("This text is in English.").score < 0.0);
    assert!(
        identifier
            .identify_normalized("This text is in English.")
            .score
            > 0.0
    );
    assert!(identifier.identify("This text is in English.").score < 0.0);
    let clear = identifier
        .identify_normalized("This is clearly an English sentence with plenty of text.")
        .score;
    let ambiguous = identifier.identify_normalized("ovo je").score;
    assert!(ambiguous < clear && ambiguous < 0.9);
    for input in ["", "a", "hi"] {
        assert_eq!(identifier.identify(input).score, -f64::from(f32::MAX));
        let ranked = identifier.rank_normalized(input);
        assert!((ranked[0].score - 2.0 / 142.0).abs() < 1e-7);
    }
}

#[test]
fn file_methods_and_independent_configuration() {
    let identifier = Identifier::new().unwrap();
    let other = Identifier::new().unwrap();
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/py3langid_cases.json");
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(
        identifier.identify_file(&path).unwrap(),
        identifier.identify(&bytes)
    );
    assert_eq!(
        identifier.rank_file(&path).unwrap(),
        identifier.rank(&bytes)
    );
    let missing = path.with_file_name("missing-language-input.txt");
    assert!(identifier
        .identify_file(&missing)
        .unwrap_err()
        .to_string()
        .contains("missing-language-input.txt"));
    assert!(identifier.rank_file(&missing).is_err());
    identifier.set_languages(&["fr"]).unwrap();
    assert_eq!(
        identifier.identify("This text is in English.").language,
        "fr"
    );
    assert_eq!(other.identify("This text is in English.").language, "en");
    assert_eq!(other.classes().len(), 140);
    let mut classes = other.classes();
    classes.clear();
    assert_eq!(other.classes().len(), 140);
    assert_eq!(
        crate::classify("This text is in English.").unwrap(),
        other.identify("This text is in English.")
    );
    assert_eq!(
        crate::rank("This text is in English.").unwrap(),
        other.rank("This text is in English.")
    );
}

#[test]
fn reused_buffers_and_concurrent_configuration_snapshots() {
    let identifier = Identifier::with_options(Options {
        normalized: true,
        min_confidence: None,
    })
    .unwrap();
    let text = "This text is in English. ".repeat(100);
    let expected = identifier.rank(&text);
    for _ in 0..20 {
        assert_eq!(identifier.rank(&text), expected);
        assert!(identifier.identify("hi").score < 0.02);
    }
    let barrier = std::sync::Barrier::new(9);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                barrier.wait();
                for _ in 0..30 {
                    let ranked = identifier.rank("This text is in English.");
                    let labels: std::collections::HashSet<_> =
                        ranked.iter().map(|item| item.language.as_str()).collect();
                    match ranked.len() {
                        1 => assert_eq!(labels, ["sr"].into_iter().collect()),
                        2 => assert_eq!(labels, ["de", "en"].into_iter().collect()),
                        140 => assert_eq!(labels.len(), 140),
                        count => panic!("partial configuration: {count} languages"),
                    }
                    assert!((ranked.iter().map(|item| item.score).sum::<f64>() - 1.0).abs() < 1e-6);
                }
            });
        }
        scope.spawn(|| {
            barrier.wait();
            for _ in 0..30 {
                identifier.set_languages(&["en", "de"]).unwrap();
                identifier.set_languages(&["sr"]).unwrap();
                identifier.reset_languages();
            }
        });
    });
}

#[test]
#[ignore = "generate target/go-reference.json with the pinned tools/go-reference module first"]
fn live_go_parity() {
    use sha2::{Digest, Sha256};

    let path = std::env::var_os("GO_PY3LANGID_REFERENCE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/go-reference.json")
        });
    let report: Value = serde_json::from_slice(
        &std::fs::read(path).expect("run the pinned Go reference generator first"),
    )
    .unwrap();
    assert_eq!(report["go_module"], "github.com/markusmobius/go-py3langid");
    assert_eq!(report["go_version"], "v0.4.0");
    assert_eq!(
        report["go_commit"],
        "d3e0c0861455d7d84daedb994392d2e71a0f6270"
    );
    assert_eq!(report["x_text"], "v0.42.0");
    assert_eq!(report["unicode"], "17.0.0");
    assert_eq!(report["normalization_unicode"], "17.0.0");
    assert_eq!(
        report["model_sha256"],
        format!("{:x}", Sha256::digest(crate::model::EMBEDDED_MODEL))
    );
    assert_eq!(
        report["cases_sha256"],
        format!(
            "{:x}",
            Sha256::digest(include_bytes!("../testdata/py3langid_cases.json"))
        )
    );
    let samples = report["samples"].as_array().unwrap();
    assert_eq!(samples.len(), 68);
    let mut maximum_raw_difference = 0.0_f64;
    let mut maximum_normalized_difference = 0.0_f64;
    for sample in samples {
        let text = STANDARD.decode(sample["input"].as_str().unwrap()).unwrap();
        let encoded = STANDARD
            .decode(sample["encoded"].as_str().unwrap())
            .unwrap();
        assert_eq!(
            encode(&text).as_ref(),
            encoded,
            "preprocessing {}",
            sample["name"]
        );
        let languages: Vec<_> = sample["languages"]
            .as_array()
            .map(|labels| labels.iter().map(|label| label.as_str().unwrap()).collect())
            .unwrap_or_default();
        for normalized in [false, true] {
            let mode = if normalized { "normalized" } else { "raw" };
            let identifier = Identifier::with_options(Options {
                normalized,
                min_confidence: if normalized {
                    sample["min_confidence"].as_f64()
                } else {
                    None
                },
            })
            .unwrap();
            identifier.set_languages(&languages).unwrap();
            let actual = identifier.identify(&text);
            assert_eq!(
                actual.language,
                sample[mode]["language"].as_str().unwrap(),
                "{} / {mode}",
                sample["name"]
            );
            let ranked = identifier.rank(&text);
            let expected_rank = sample[format!("{mode}_rank")].as_array().unwrap();
            assert_eq!(ranked.len(), expected_rank.len());
            for (actual, expected) in
                std::iter::once((&actual, &sample[mode])).chain(ranked.iter().zip(expected_rank))
            {
                assert_eq!(
                    actual.language,
                    expected["language"].as_str().unwrap(),
                    "{} / {mode}: ranking order",
                    sample["name"]
                );
                let expected_score = expected["score"].as_f64().unwrap();
                let difference = (actual.score - expected_score).abs();
                let absolute = if normalized { 1e-7 } else { 1e-6 };
                assert!(
                    actual.score.is_finite()
                        && difference <= absolute + 1e-6 * expected_score.abs(),
                    "{} / {mode} / {}: got {}, expected {expected_score}",
                    sample["name"],
                    actual.language,
                    actual.score
                );
                if normalized {
                    maximum_normalized_difference = maximum_normalized_difference.max(difference);
                } else {
                    maximum_raw_difference = maximum_raw_difference.max(difference);
                }
            }
        }
    }
    eprintln!("Go parity: {} cases; maximum absolute score differences: raw={maximum_raw_difference}, normalized={maximum_normalized_difference}", samples.len());
}

#[test]
fn interleaved_features_match_serial_full_scores_at_boundaries() {
    let fast = Identifier::new().unwrap();
    let model = crate::Model::from_bytes(crate::model::EMBEDDED_MODEL).unwrap();
    assert!(model.history_bytes.is_none());
    let serial = Identifier::from_model(std::sync::Arc::new(model), Options::default()).unwrap();
    let corpus: Value =
        serde_json::from_str(include_str!("../testdata/py3langid_cases.json")).unwrap();
    let mut texts: Vec<_> = corpus["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(case_bytes)
        .collect();
    for length in [
        0, 1, 5, 6, 7, 63, 64, 65, 127, 128, 129, 255, 256, 257, 511, 512, 513, 1023, 1024, 1025,
        2047, 2048, 2049, 4095, 4096, 4097,
    ] {
        texts.push(
            b"English text with repeated features and caf\xc3\xa9. "
                .iter()
                .copied()
                .cycle()
                .take(length)
                .collect(),
        );
        texts.push((0..=255_u8).cycle().take(length).collect());
    }
    for languages in [&[][..], &["en", "sr", "uz"][..]] {
        fast.set_languages(languages).unwrap();
        serial.set_languages(languages).unwrap();
        for text in &texts {
            for (actual, expected) in [
                (fast.rank(text), serial.rank(text)),
                (fast.rank_normalized(text), serial.rank_normalized(text)),
            ] {
                assert_eq!(actual.len(), expected.len());
                for (actual, expected) in actual.iter().zip(&expected) {
                    assert_eq!(
                        actual.language,
                        expected.language,
                        "input length {}",
                        text.len()
                    );
                    assert_eq!(
                        actual.score.to_bits(),
                        expected.score.to_bits(),
                        "input length {}",
                        text.len()
                    );
                }
            }
        }
    }
}
