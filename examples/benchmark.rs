use std::collections::HashSet;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rust_py3langid::{Identifier, Options};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

struct Case {
    language: String,
    #[cfg(feature = "benchmark-comparison")]
    source_language: String,
    text: String,
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut suite = None;
    let mut passes = 8;
    let mut engine = String::from("rust-py3langid");
    let mut subset = None;
    let mut arguments = std::env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            Some("--suite") => {
                suite = Some(PathBuf::from(
                    arguments.next().ok_or("--suite requires a path")?,
                ));
            }
            Some("--passes") => {
                let value = arguments
                    .next()
                    .ok_or("--passes requires a positive integer")?;
                passes = value
                    .to_str()
                    .ok_or("invalid pass count")?
                    .parse::<usize>()?;
            }
            Some("--engine") => {
                engine = arguments
                    .next()
                    .ok_or("--engine requires a name")?
                    .into_string()
                    .map_err(|_| "invalid engine name")?;
            }
            Some("--subset") => {
                subset = Some(
                    arguments
                        .next()
                        .ok_or("--subset requires whichlang")?
                        .into_string()
                        .map_err(|_| "invalid subset name")?,
                );
            }
            Some("--help" | "-h") => {
                println!("benchmark --suite PATH [--passes 8] [--engine rust-py3langid|whatlang|lingua|whichlang] [--subset whichlang]");
                println!("Other engines and subsets require --features benchmark-comparison at build time.");
                return Ok(());
            }
            _ => return Err(format!("unknown argument: {}", argument.to_string_lossy()).into()),
        }
    }
    if passes == 0 {
        return Err("--passes must be positive".into());
    }
    if !matches!(
        engine.as_str(),
        "rust-py3langid" | "whatlang" | "lingua" | "whichlang"
    ) {
        return Err(format!("unknown engine: {engine}").into());
    }
    if subset.as_deref().is_some_and(|name| name != "whichlang") {
        return Err("the only supported subset is whichlang".into());
    }
    if engine == "whichlang" {
        subset = Some(String::from("whichlang"));
    }
    if !cfg!(feature = "benchmark-comparison") && (engine != "rust-py3langid" || subset.is_some()) {
        return Err(
            "rebuild with --features benchmark-comparison for other engines/subsets".into(),
        );
    }
    let cases = load_suite(&suite.ok_or("--suite is required")?)?;
    #[cfg(feature = "benchmark-comparison")]
    let cases = if subset.is_some() {
        whichlang_subset(cases)?
    } else {
        cases
    };
    #[cfg(feature = "benchmark-comparison")]
    if engine != "rust-py3langid" {
        return run_comparison(&engine, &cases, passes);
    }
    let started = Instant::now();
    let identifier = Identifier::with_options(Options {
        normalized: true,
        min_confidence: None,
    })?;
    let startup_ms = started.elapsed().as_secs_f64() * 1000.0;
    let supported: HashSet<_> = identifier.classes().into_iter().collect();
    if cases.iter().any(|case| !supported.contains(&case.language)) {
        return Err("suite contains an unsupported language; no cases were filtered".into());
    }
    let expected: Vec<_> = cases.iter().map(|case| case.language.clone()).collect();
    run_benchmark(&cases, &expected, passes, startup_ms, |text| {
        identifier.identify(text).language
    })
}

fn run_benchmark<Label: PartialEq>(
    cases: &[Case],
    expected: &[Label],
    passes: usize,
    startup_ms: f64,
    classify: impl Fn(&str) -> Label,
) -> Result<(), Box<dyn Error>> {
    let (_, baseline) = measure_pass(cases, &classify);
    let mut durations = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (duration, predictions) = measure_pass(cases, &classify);
        if predictions != baseline {
            return Err("predictions changed between passes".into());
        }
        durations.push(duration);
    }
    println!(
        "{}",
        json!({
            "startup_ms": startup_ms,
            "pass_ms": median_ms(durations),
            "accuracy_pct": accuracy_pct(expected, &baseline),
        })
    );
    Ok(())
}

#[cfg(feature = "benchmark-comparison")]
fn run_comparison(engine: &str, cases: &[Case], passes: usize) -> Result<(), Box<dyn Error>> {
    match engine {
        "whatlang" => {
            let expected = cases
                .iter()
                .map(|case| {
                    whatlang_language(&case.source_language)
                        .map(Some)
                        .ok_or_else(|| format!("whatlang does not support {}", case.language))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let started = Instant::now();
            let detector = whatlang::Detector::new();
            let startup_ms = started.elapsed().as_secs_f64() * 1000.0;
            run_benchmark(cases, &expected, passes, startup_ms, |text| {
                detector.detect(text).map(|result| result.lang())
            })
        }
        "lingua" => {
            let languages = lingua::Language::all();
            let expected = cases
                .iter()
                .map(|case| {
                    languages
                        .iter()
                        .find(|language| {
                            language.iso_code_639_1().to_string().to_lowercase() == case.language
                        })
                        .copied()
                        .map(Some)
                        .ok_or_else(|| format!("lingua does not support {}", case.language))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let started = Instant::now();
            let detector = lingua::LanguageDetectorBuilder::from_all_languages()
                .with_preloaded_language_models()
                .build();
            let startup_ms = started.elapsed().as_secs_f64() * 1000.0;
            run_benchmark(cases, &expected, passes, startup_ms, |text| {
                detector.detect_language_of(text)
            })
        }
        "whichlang" => {
            let expected = cases
                .iter()
                .map(|case| {
                    whichlang::LANGUAGES
                        .iter()
                        .find(|language| {
                            language.three_letter_code()
                                == detector_language_code(&case.source_language)
                        })
                        .copied()
                        .ok_or_else(|| format!("whichlang does not support {}", case.language))
                })
                .collect::<Result<Vec<_>, _>>()?;
            run_benchmark(cases, &expected, passes, 0.0, whichlang::detect_language)
        }
        _ => Err(format!("unknown comparison engine: {engine}").into()),
    }
}

#[cfg(feature = "benchmark-comparison")]
fn whatlang_language(source_language: &str) -> Option<whatlang::Lang> {
    whatlang::Lang::from_code(detector_language_code(source_language))
}

#[cfg(feature = "benchmark-comparison")]
fn detector_language_code(source_language: &str) -> &str {
    match source_language {
        "arb" => "ara",
        "zho" => "cmn",
        other => other,
    }
}

#[cfg(feature = "benchmark-comparison")]
fn whichlang_subset(mut cases: Vec<Case>) -> Result<Vec<Case>, Box<dyn Error>> {
    cases.retain(|case| {
        whichlang::LANGUAGES.iter().any(|language| {
            language.three_letter_code() == detector_language_code(&case.source_language)
        })
    });
    if cases.is_empty() {
        return Err("suite contains no languages supported by whichlang".into());
    }
    Ok(cases)
}

fn load_suite(path: &Path) -> Result<Vec<Case>, Box<dyn Error>> {
    let data = std::fs::read(path)?;
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(path.with_file_name("manifest.json"))?)?;
    if manifest["suite_sha256"] != format!("{:x}", Sha256::digest(&data)) {
        return Err("suite SHA-256 does not match its manifest".into());
    }
    let mut cases = Vec::new();
    let mut identifiers = HashSet::new();
    for (index, line) in std::str::from_utf8(&data)?.lines().enumerate() {
        let record: Value = serde_json::from_str(line)?;
        let field = |name: &str| {
            record[name]
                .as_str()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| format!("invalid {name} on suite line {}", index + 1))
        };
        if !identifiers.insert(field("id")?.to_owned()) {
            return Err(format!("duplicate id on suite line {}", index + 1).into());
        }
        let language = field("language")?;
        #[cfg(feature = "benchmark-comparison")]
        let source_language = manifest["languages"][language]
            .as_str()
            .and_then(|source| source.split_once('_'))
            .ok_or_else(|| format!("missing FLORES source language for {language}"))?
            .0;
        cases.push(Case {
            language: language.to_owned(),
            #[cfg(feature = "benchmark-comparison")]
            source_language: source_language.to_owned(),
            text: field("text")?.to_owned(),
        });
    }
    if cases.is_empty() || manifest["sample_count"].as_u64() != Some(cases.len() as u64) {
        return Err("suite is empty or its sample count does not match the manifest".into());
    }
    Ok(cases)
}

fn measure_pass<Label>(
    cases: &[Case],
    classify: &impl Fn(&str) -> Label,
) -> (Duration, Vec<Label>) {
    let mut predictions = Vec::with_capacity(cases.len());
    let started = Instant::now();
    for case in cases {
        predictions.push(classify(&case.text));
    }
    (started.elapsed(), predictions)
}

fn median_ms(mut durations: Vec<Duration>) -> f64 {
    durations.sort_unstable();
    let middle = durations.len() / 2;
    (durations[(durations.len() - 1) / 2].as_secs_f64() + durations[middle].as_secs_f64()) * 500.0
}

fn accuracy_pct<Label: PartialEq>(expected: &[Label], predictions: &[Label]) -> f64 {
    assert_eq!(expected.len(), predictions.len());
    let correct = expected
        .iter()
        .zip(predictions)
        .filter(|(label, predicted)| label == predicted)
        .count();
    100.0 * correct as f64 / expected.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_handles_odd_even_and_single_passes() {
        for (values, expected) in [(vec![9, 1, 3], 3.0), (vec![4, 2], 3.0), (vec![7], 7.0)] {
            assert_eq!(
                median_ms(values.into_iter().map(Duration::from_millis).collect()),
                expected
            );
        }
    }

    #[test]
    fn accuracy_counts_every_exact_label() {
        let expected = ["en", "fr", "zh", "de"];
        let predictions = ["en", "und", "zh-Hans", "de"];
        assert_eq!(accuracy_pct(&expected, &predictions), 50.0);
        assert_eq!(
            accuracy_pct(&[Some("en"), Some("fr")], &[Some("en"), None]),
            50.0
        );
    }

    #[test]
    #[ignore = "requires BENCHMARK_SUITE pointing to the original Go FLORES-200 suite"]
    fn original_suite_inference_parity() {
        let path = PathBuf::from(std::env::var_os("BENCHMARK_SUITE").expect("set BENCHMARK_SUITE"));
        let cases = load_suite(&path).unwrap();
        assert_eq!(cases.len(), 1000);
        let optimized = Identifier::new().unwrap();
        let serial = Identifier::from_path(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("model/py3langid.lidg"),
            Options::default(),
        )
        .unwrap();
        for (index, case) in cases.iter().enumerate() {
            for (actual, expected) in [
                (optimized.rank(&case.text), serial.rank(&case.text)),
                (
                    optimized.rank_normalized(&case.text),
                    serial.rank_normalized(&case.text),
                ),
            ] {
                assert_eq!(actual.len(), expected.len(), "sample {index}");
                for (actual, expected) in actual.iter().zip(&expected) {
                    assert_eq!(actual.language, expected.language, "sample {index}");
                    assert_eq!(
                        actual.score.to_bits(),
                        expected.score.to_bits(),
                        "sample {index}"
                    );
                }
            }
        }
    }

    #[cfg(feature = "benchmark-comparison")]
    #[test]
    fn whatlang_matches_flores_language_codes() {
        assert_eq!(whatlang_language("arb"), Some(whatlang::Lang::Ara));
        assert_eq!(whatlang_language("zho"), Some(whatlang::Lang::Cmn));
        assert_eq!(whatlang_language("deu"), Some(whatlang::Lang::Deu));
        assert_eq!(whatlang_language("xxx"), None);
        assert_eq!(whatlang::Lang::all().len(), 70);
        assert_eq!(lingua::Language::all().len(), 75);
    }

    #[cfg(feature = "benchmark-comparison")]
    #[test]
    fn whichlang_subset_preserves_order_and_text() {
        assert_eq!(whichlang::LANGUAGES.len(), 16);
        let cases = [
            ("id", "ind"),
            ("en", "eng"),
            ("pl", "pol"),
            ("zh", "zho"),
            ("th", "tha"),
            ("de", "deu"),
            ("uk", "ukr"),
        ]
        .map(|(language, source_language)| Case {
            language: language.into(),
            source_language: source_language.into(),
            text: format!("unchanged {language}"),
        });
        let selected = whichlang_subset(cases.into()).unwrap();
        let actual: Vec<_> = selected
            .iter()
            .map(|case| (case.language.as_str(), case.text.as_str()))
            .collect();
        assert_eq!(
            actual,
            [
                ("en", "unchanged en"),
                ("zh", "unchanged zh"),
                ("de", "unchanged de")
            ]
        );
        assert!(whichlang_subset(Vec::new()).is_err());
    }

    #[cfg(feature = "benchmark-comparison")]
    #[test]
    #[ignore = "requires BENCHMARK_SUITE pointing to the original Go FLORES-200 suite"]
    fn original_suite_coverage() {
        let path = PathBuf::from(std::env::var_os("BENCHMARK_SUITE").expect("set BENCHMARK_SUITE"));
        let cases = load_suite(&path).unwrap();
        assert_eq!(cases.len(), 1000);
        let languages: HashSet<_> = cases.iter().map(|case| case.language.as_str()).collect();
        assert_eq!(languages.len(), 20);
        for case in &cases {
            assert!(whatlang_language(&case.source_language).is_some());
            assert!(lingua::Language::all().iter().any(|language| {
                language.iso_code_639_1().to_string().to_lowercase() == case.language
            }));
        }
        let selected = whichlang_subset(cases).unwrap();
        assert_eq!(selected.len(), 800);
        let languages: HashSet<_> = selected.iter().map(|case| case.language.as_str()).collect();
        assert_eq!(languages.len(), 16);
        assert!(["id", "pl", "th", "uk"]
            .iter()
            .all(|language| !languages.contains(language)));
    }
}
