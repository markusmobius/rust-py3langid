use std::io::Read;

use rust_py3langid::{Identifier, Options};
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut options = Options::default();
    let mut ranked = false;
    let mut languages = Vec::new();
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--normalized" => options.normalized = true,
            "--rank" => ranked = true,
            "--languages" => {
                languages = arguments
                    .next()
                    .ok_or("--languages requires comma-separated labels")?
                    .split(',')
                    .map(str::to_owned)
                    .collect();
            }
            "--min-confidence" => {
                options.min_confidence = Some(
                    arguments
                        .next()
                        .ok_or("--min-confidence requires a number")?
                        .parse()?,
                );
            }
            "--help" | "-h" => {
                println!(
                    "classify [--normalized] [--rank] [--languages en,de] [--min-confidence 0.5]"
                );
                println!(
                    "Reads bytes from stdin and writes JSON to stdout. Raw scores are the default."
                );
                return Ok(());
            }
            _ => return Err(format!("unknown argument: {argument}").into()),
        }
    }
    let identifier = Identifier::with_options(options)?;
    identifier.set_languages(&languages.iter().map(String::as_str).collect::<Vec<_>>())?;
    let mut text = Vec::new();
    std::io::stdin().read_to_end(&mut text)?;
    let output = if ranked {
        json!(identifier
            .rank(&text)
            .iter()
            .map(|result| json!({"language": result.language, "score": result.score}))
            .collect::<Vec<_>>())
    } else {
        let result = identifier.identify(&text);
        json!({"language": result.language, "score": result.score})
    };
    println!("{output}");
    Ok(())
}
