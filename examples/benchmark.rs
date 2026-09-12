use std::collections::HashSet;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rust_py3langid::{Identifier, Options};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

struct Case {
    language: String,
    text: Vec<u8>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut suite = None;
    let mut passes = 8;
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
            Some("--help" | "-h") => {
                println!("benchmark --suite PATH [--passes 8]");
                return Ok(());
            }
            _ => return Err(format!("unknown argument: {}", argument.to_string_lossy()).into()),
        }
    }
    if passes == 0 {
        return Err("--passes must be positive".into());
    }
    let cases = load_suite(&suite.ok_or("--suite is required")?)?;
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
    let (_, baseline) = measure_pass(&identifier, &cases);
    let mut durations = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (duration, predictions) = measure_pass(&identifier, &cases);
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
            "accuracy_pct": accuracy_pct(&cases, &baseline),
        })
    );
    Ok(())
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
        cases.push(Case {
            language: field("language")?.to_owned(),
            text: field("text")?.as_bytes().to_vec(),
        });
    }
    if cases.is_empty() || manifest["sample_count"].as_u64() != Some(cases.len() as u64) {
        return Err("suite is empty or its sample count does not match the manifest".into());
    }
    Ok(cases)
}

fn measure_pass(identifier: &Identifier, cases: &[Case]) -> (Duration, Vec<String>) {
    let mut predictions = Vec::with_capacity(cases.len());
    let started = Instant::now();
    for case in cases {
        predictions.push(identifier.identify(&case.text).language);
    }
    (started.elapsed(), predictions)
}

fn median_ms(mut durations: Vec<Duration>) -> f64 {
    durations.sort_unstable();
    let middle = durations.len() / 2;
    (durations[(durations.len() - 1) / 2].as_secs_f64() + durations[middle].as_secs_f64()) * 500.0
}

fn accuracy_pct(cases: &[Case], predictions: &[String]) -> f64 {
    let correct = cases
        .iter()
        .zip(predictions)
        .filter(|(case, predicted)| case.language == **predicted)
        .count();
    100.0 * correct as f64 / cases.len() as f64
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
        let cases = ["en", "fr", "zh", "de"].map(|language| Case {
            language: language.into(),
            text: Vec::new(),
        });
        let predictions = ["en", "und", "zh-Hans", "de"].map(String::from);
        assert_eq!(accuracy_pct(&cases, &predictions), 50.0);
    }
}
