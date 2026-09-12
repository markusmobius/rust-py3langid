use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use flate2::read::MultiGzDecoder;

use crate::Error;

pub(crate) const EMBEDDED_MODEL: &[u8] = include_bytes!("../model/py3langid.lidg");
const MAGIC: &[u8; 6] = b"LIDG2\0";
const DIMENSION_LIMITS: [u32; 4] = [1_000_000, 10_000, 500_000, 500_000];
const MAX_WEIGHTS: usize = 50_000_000;

#[derive(Debug)]
pub struct Model {
    pub(crate) num_features: usize,
    pub(crate) num_languages: usize,
    pub(crate) num_states: usize,
    pub(crate) transitions: Vec<u32>,
    pub(crate) rows: Vec<u32>,
    pub(crate) outputs: Vec<i32>,
    pub(crate) priors: Vec<f32>,
    pub(crate) weights: Vec<f32>,
    pub(crate) classes: Vec<String>,
}

impl Model {
    pub fn embedded() -> Result<Self, Error> {
        Self::from_bytes(EMBEDDED_MODEL)
    }

    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, Error> {
        Self::from_reader(File::open(path)?)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        Self::from_reader(bytes)
    }

    pub fn num_features(&self) -> usize {
        self.num_features
    }

    pub fn num_states(&self) -> usize {
        self.num_states
    }

    pub fn classes(&self) -> &[String] {
        &self.classes
    }

    fn from_reader(mut source: impl Read) -> Result<Self, Error> {
        let mut signature = [0; 6];
        source.read_exact(&mut signature)?;
        if &signature != MAGIC {
            return Err(Error::InvalidModel("expected LIDG2 signature".into()));
        }
        let mut reader = BufReader::new(MultiGzDecoder::new(source));
        let dimensions = read_array(&mut reader, 4, u32::from_le_bytes)?;
        for (index, (&value, &limit)) in dimensions.iter().zip(&DIMENSION_LIMITS).enumerate() {
            if value == 0 || value > limit {
                return Err(Error::InvalidModel(format!(
                    "dimension {index}: {value} (limit {limit})"
                )));
            }
        }
        if dimensions[3] > dimensions[2] {
            return Err(Error::InvalidModel(
                "more transition rows than states".into(),
            ));
        }
        let num_features = dimensions[0] as usize;
        let num_languages = dimensions[1] as usize;
        let num_states = dimensions[2] as usize;
        let num_rows = dimensions[3] as usize;
        let num_weights = num_features
            .checked_mul(num_languages)
            .filter(|&count| count <= MAX_WEIGHTS)
            .ok_or_else(|| Error::InvalidModel("likelihood dimensions exceed limit".into()))?;
        let transitions = read_array(&mut reader, num_rows * 256, u32::from_le_bytes)?;
        let rows = read_array(&mut reader, num_states, u32::from_le_bytes)?;
        let outputs = read_array(&mut reader, num_states, i32::from_le_bytes)?;
        let priors = read_array(&mut reader, num_languages, f32::from_le_bytes)?;
        let weights = read_array(&mut reader, num_weights, f32::from_le_bytes)?;
        let mut classes = Vec::with_capacity(num_languages);
        for _ in 0..num_languages {
            let mut length = [0; 2];
            reader.read_exact(&mut length)?;
            let length = usize::from(u16::from_le_bytes(length));
            if length == 0 || length > 256 {
                return Err(Error::InvalidModel(format!("class label length: {length}")));
            }
            let mut label = vec![0; length];
            reader.read_exact(&mut label)?;
            classes.push(
                String::from_utf8(label)
                    .map_err(|_| Error::InvalidModel("class label is not UTF-8".into()))?,
            );
        }
        let mut extra = [0; 1];
        if reader.read(&mut extra)? != 0 {
            return Err(Error::InvalidModel("unexpected trailing model data".into()));
        }
        for (index, &state) in transitions.iter().enumerate() {
            if state as usize >= num_states {
                return Err(Error::InvalidModel(format!(
                    "transition {index}: state {state}"
                )));
            }
        }
        for (state, (&row, &feature)) in rows.iter().zip(&outputs).enumerate() {
            if row as usize >= num_rows {
                return Err(Error::InvalidModel(format!("row {row} for state {state}")));
            }
            if feature < -1 || feature >= num_features as i32 {
                return Err(Error::InvalidModel(format!(
                    "output feature {feature} for state {state}"
                )));
            }
        }
        if priors
            .iter()
            .chain(&weights)
            .any(|score| !score.is_finite())
        {
            return Err(Error::InvalidModel("non-finite score".into()));
        }
        Ok(Self {
            num_features,
            num_languages,
            num_states,
            transitions,
            rows,
            outputs,
            priors,
            weights,
            classes,
        })
    }
}

fn read_array<Value>(
    reader: &mut impl Read,
    count: usize,
    decode: impl Fn([u8; 4]) -> Value,
) -> Result<Vec<Value>, Error> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|error| Error::InvalidModel(format!("array allocation: {error}")))?;
    for _ in 0..count {
        let mut bytes = [0; 4];
        reader.read_exact(&mut bytes)?;
        values.push(decode(bytes));
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use flate2::{write::GzEncoder, Compression};
    use sha2::{Digest, Sha256};

    use super::*;

    fn synthetic_model() -> Model {
        let mut model = Model {
            num_features: 1,
            num_languages: 3,
            num_states: 65_537,
            transitions: vec![0; 256],
            rows: vec![0; 65_537],
            outputs: vec![-1; 65_537],
            priors: vec![-1.0, -2.0, -3.0],
            weights: vec![-2.0, -3.0, -4.0],
            classes: vec!["en".into(), "sr".into(), "sr".into()],
        };
        model.transitions[usize::from(b'a')] = 65_536;
        model.outputs[65_536] = 0;
        model
    }

    fn encode(model: &Model, trailing: &[u8]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(MAGIC.to_vec(), Compression::fast());
        for dimension in [
            model.num_features,
            model.num_languages,
            model.num_states,
            model.transitions.len() / 256,
        ] {
            encoder
                .write_all(&(dimension as u32).to_le_bytes())
                .unwrap();
        }
        for &value in model.transitions.iter().chain(&model.rows) {
            encoder.write_all(&value.to_le_bytes()).unwrap();
        }
        for &value in &model.outputs {
            encoder.write_all(&value.to_le_bytes()).unwrap();
        }
        for &value in model.priors.iter().chain(&model.weights) {
            encoder.write_all(&value.to_le_bytes()).unwrap();
        }
        for label in &model.classes {
            encoder
                .write_all(&(label.len() as u16).to_le_bytes())
                .unwrap();
            encoder.write_all(label.as_bytes()).unwrap();
        }
        encoder.write_all(trailing).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn shared_rows_keep_32_bit_states_and_aliases() {
        let model = Model::from_bytes(&encode(&synthetic_model(), &[])).unwrap();
        assert_eq!(model.num_states, 65_537);
        assert_eq!(model.transitions[usize::from(b'a')], 65_536);
        assert_eq!(model.outputs[65_536], 0);
        assert_eq!(model.outputs[0], -1);
        assert_eq!(model.classes, ["en", "sr", "sr"]);
    }

    #[test]
    fn custom_model_inference_and_file_loading() {
        let model = Model::from_bytes(&encode(&synthetic_model(), &[])).unwrap();
        let identifier =
            crate::Identifier::from_model(std::sync::Arc::new(model), crate::Options::default())
                .unwrap();
        assert_eq!(identifier.classes(), ["en", "sr"]);
        let result = identifier.identify("a");
        assert_eq!(result.language, "en");
        assert_eq!(
            result.score,
            f64::from(-1.0_f32 - 2.0 * (2.0_f64.ln() as f32))
        );
        identifier.set_languages(&["sr"]).unwrap();
        assert_eq!(identifier.identify("a").language, "sr");
        assert!((identifier.identify_normalized("a").score - 1.0).abs() < 1e-6);
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("model/py3langid.lidg");
        let external = crate::Identifier::from_path(path, crate::Options::default()).unwrap();
        assert_eq!(external.identify("This text is in English.").language, "en");
        assert!(Model::from_path("missing-model.lidg").is_err());
    }

    #[test]
    fn embedded_model_matches_pinned_reference() {
        let reference: serde_json::Value =
            serde_json::from_str(include_str!("../testdata/py3langid_reference.json")).unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(EMBEDDED_MODEL)),
            reference["go_model_sha256"].as_str().unwrap()
        );
        let model = Model::embedded().unwrap();
        assert_eq!(model.num_features, 100_053);
        assert_eq!(model.num_states, 104_583);
        assert_eq!(model.num_languages, 142);
        let classes: Vec<String> = serde_json::from_value(reference["classes"].clone()).unwrap();
        assert_eq!(model.classes, classes);
    }

    #[test]
    fn rejects_invalid_models() {
        type ModelMutation = (&'static str, fn(&mut Model));
        let mutations: Vec<ModelMutation> = vec![
            ("dimension", |model| model.num_features = 0),
            ("dimension", |model| model.num_states = 500_001),
            ("likelihood", |model| {
                model.num_features = 1_000_000;
                model.num_languages = 100;
            }),
            ("transition", |model| model.transitions[0] = 65_537),
            ("row", |model| model.rows[0] = 1),
            ("output feature", |model| model.outputs[0] = 1),
            ("output feature", |model| model.outputs[0] = -2),
            ("label length", |model| model.classes[0] = "x".repeat(257)),
            ("label length", |model| model.classes[0].clear()),
            ("non-finite", |model| model.priors[0] = f32::INFINITY),
            ("non-finite", |model| model.weights[0] = f32::NAN),
        ];
        for (expected, mutate) in mutations {
            let mut model = synthetic_model();
            mutate(&mut model);
            let error = Model::from_bytes(&encode(&model, &[])).unwrap_err();
            assert!(
                error.to_string().contains(expected),
                "expected {expected}, got {error}"
            );
        }
        assert!(Model::from_bytes(b"LIDG1\0").is_err());
        assert!(Model::from_bytes(&encode(&synthetic_model(), &[1])).is_err());
    }

    #[test]
    fn rejects_truncation_and_corrupt_gzip() {
        let mut encoded = encode(&synthetic_model(), &[]);
        for length in [0, 6, 16, encoded.len() - 1] {
            assert!(
                Model::from_bytes(&encoded[..length]).is_err(),
                "accepted {length} bytes"
            );
        }
        let last = encoded.len() - 1;
        encoded[last] ^= 1;
        assert!(Model::from_bytes(&encoded).is_err());
    }
}
