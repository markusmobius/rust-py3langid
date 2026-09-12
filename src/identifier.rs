use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use crate::{preprocess, Error, Model};

#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    pub normalized: bool,
    pub min_confidence: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LanguageScore {
    pub language: String,
    pub score: f64,
}

pub struct Identifier {
    model: Arc<Model>,
    runtime: RwLock<Arc<Runtime>>,
    options: Options,
}

struct Runtime {
    columns: Vec<usize>,
    label_indices: Vec<usize>,
    alias_pairs: Vec<(usize, usize)>,
    pool: Mutex<Vec<WorkBuffer>>,
    max_cached_buffers: usize,
}

struct WorkBuffer {
    feature_counts: Vec<u32>,
    active_features: Vec<usize>,
    scores: Vec<f32>,
}

impl Identifier {
    pub fn new() -> Result<Self, Error> {
        Self::with_options(Options::default())
    }

    pub fn with_options(options: Options) -> Result<Self, Error> {
        static MODEL: OnceLock<Result<Arc<Model>, String>> = OnceLock::new();
        let model = MODEL.get_or_init(|| {
            Model::embedded()
                .map(Arc::new)
                .map_err(|error| error.to_string())
        });
        match model {
            Ok(model) => Self::from_model(Arc::clone(model), options),
            Err(error) => Err(Error::InvalidModel(error.clone())),
        }
    }

    pub fn from_path(path: impl AsRef<Path>, options: Options) -> Result<Self, Error> {
        Self::from_model(Arc::new(Model::from_path(path)?), options)
    }

    pub fn from_model(model: Arc<Model>, options: Options) -> Result<Self, Error> {
        if let Some(threshold) = options.min_confidence {
            if !threshold.is_finite() || !(0.0..=1.0).contains(&threshold) {
                return Err(Error::InvalidConfiguration(
                    "minimum confidence must be finite and between 0 and 1".into(),
                ));
            }
            if !options.normalized {
                return Err(Error::InvalidConfiguration(
                    "minimum confidence requires normalized probabilities".into(),
                ));
            }
        }
        let runtime = Arc::new(Runtime::new(&model, (0..model.num_languages).collect()));
        Ok(Self {
            model,
            runtime: RwLock::new(runtime),
            options,
        })
    }

    pub fn classes(&self) -> Vec<String> {
        let runtime = self.snapshot();
        runtime
            .label_indices
            .iter()
            .map(|&index| self.model.classes[runtime.columns[index]].clone())
            .collect()
    }

    pub fn set_languages(&self, languages: &[&str]) -> Result<(), Error> {
        if languages.is_empty() {
            self.reset_languages();
            return Ok(());
        }
        for &language in languages {
            if !self.model.classes.iter().any(|label| label == language) {
                return Err(Error::InvalidConfiguration(format!(
                    "unknown language: {language}"
                )));
            }
        }
        let columns = self
            .model
            .classes
            .iter()
            .enumerate()
            .filter_map(|(index, label)| languages.contains(&label.as_str()).then_some(index))
            .collect();
        let runtime = Arc::new(Runtime::new(&self.model, columns));
        *self
            .runtime
            .write()
            .unwrap_or_else(|error| error.into_inner()) = runtime;
        Ok(())
    }

    pub fn reset_languages(&self) {
        let runtime = Arc::new(Runtime::new(
            &self.model,
            (0..self.model.num_languages).collect(),
        ));
        *self
            .runtime
            .write()
            .unwrap_or_else(|error| error.into_inner()) = runtime;
    }

    pub fn identify(&self, text: impl AsRef<[u8]>) -> LanguageScore {
        self.identify_mode(text.as_ref(), self.options.normalized)
    }

    pub fn identify_normalized(&self, text: impl AsRef<[u8]>) -> LanguageScore {
        self.identify_mode(text.as_ref(), true)
    }

    pub fn rank(&self, text: impl AsRef<[u8]>) -> Vec<LanguageScore> {
        self.rank_mode(text.as_ref(), self.options.normalized)
    }

    pub fn rank_normalized(&self, text: impl AsRef<[u8]>) -> Vec<LanguageScore> {
        self.rank_mode(text.as_ref(), true)
    }

    pub fn identify_file(&self, path: impl AsRef<Path>) -> Result<LanguageScore, Error> {
        Ok(self.identify(read_text(path.as_ref())?))
    }

    pub fn rank_file(&self, path: impl AsRef<Path>) -> Result<Vec<LanguageScore>, Error> {
        Ok(self.rank(read_text(path.as_ref())?))
    }

    fn snapshot(&self) -> Arc<Runtime> {
        Arc::clone(
            &self
                .runtime
                .read()
                .unwrap_or_else(|error| error.into_inner()),
        )
    }

    fn identify_mode(&self, text: &[u8], normalized: bool) -> LanguageScore {
        self.with_scores(text, normalized, |runtime, scores| {
            let mut best = runtime.label_indices[0];
            for &index in &runtime.label_indices[1..] {
                if scores[index] > scores[best] {
                    best = index;
                }
            }
            let score = f64::from(scores[best]);
            let language = if normalized
                && self
                    .options
                    .min_confidence
                    .is_some_and(|threshold| score < threshold)
            {
                "und".into()
            } else {
                self.model.classes[runtime.columns[best]].clone()
            };
            LanguageScore { language, score }
        })
    }

    fn rank_mode(&self, text: &[u8], normalized: bool) -> Vec<LanguageScore> {
        self.with_scores(text, normalized, |runtime, scores| {
            let mut results: Vec<_> = runtime
                .label_indices
                .iter()
                .map(|&index| LanguageScore {
                    language: self.model.classes[runtime.columns[index]].clone(),
                    score: f64::from(scores[index]),
                })
                .collect();
            results.sort_by(|left, right| {
                right
                    .score
                    .partial_cmp(&left.score)
                    .unwrap_or(Ordering::Equal)
            });
            results
        })
    }

    fn with_scores<Value>(
        &self,
        text: &[u8],
        normalized: bool,
        consume: impl FnOnce(&Runtime, &[f32]) -> Value,
    ) -> Value {
        let runtime = self.snapshot();
        let mut buffer = runtime
            .pool
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pop()
            .unwrap_or_else(|| WorkBuffer {
                feature_counts: vec![0; self.model.num_features],
                active_features: Vec::new(),
                scores: vec![0.0; runtime.columns.len()],
            });
        self.score(&runtime, &mut buffer, text, normalized);
        let result = consume(&runtime, &buffer.scores);
        let mut pool = runtime
            .pool
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if pool.len() < runtime.max_cached_buffers {
            pool.push(buffer);
        }
        result
    }

    fn score(&self, runtime: &Runtime, buffer: &mut WorkBuffer, text: &[u8], normalized: bool) {
        let text = preprocess::encode(text);
        let mut state = 0;
        for &letter in text.as_ref() {
            state = self.model.transitions
                [self.model.rows[state] as usize * 256 + usize::from(letter)]
                as usize;
            let feature = self.model.outputs[state];
            if feature >= 0 {
                let feature = feature as usize;
                if buffer.feature_counts[feature] == 0 {
                    buffer.active_features.push(feature);
                }
                buffer.feature_counts[feature] = buffer.feature_counts[feature].wrapping_add(1);
            }
        }
        buffer.scores.fill(0.0);
        if buffer.active_features.is_empty() {
            if !normalized {
                buffer.scores.fill(-f32::MAX);
            }
        } else {
            for &feature in &buffer.active_features {
                let count = f64::from(buffer.feature_counts[feature]).ln_1p() as f32;
                buffer.feature_counts[feature] = 0;
                let base = feature * self.model.num_languages;
                for (score, &column) in buffer.scores.iter_mut().zip(&runtime.columns) {
                    *score += count * self.model.weights[base + column];
                }
            }
            for (score, &column) in buffer.scores.iter_mut().zip(&runtime.columns) {
                *score += self.model.priors[column];
            }
        }
        buffer.active_features.clear();
        if normalized {
            let scale = (1.0 / (text.len().max(1) as f64).sqrt()) as f32;
            let mut maximum = -f32::MAX;
            for score in &mut buffer.scores {
                *score *= scale;
                maximum = maximum.max(*score);
            }
            let mut total = 0.0_f32;
            for score in &mut buffer.scores {
                *score = f64::from(*score - maximum).exp() as f32;
                total += *score;
            }
            for score in &mut buffer.scores {
                *score /= total;
            }
        }
        for &(first, alias) in &runtime.alias_pairs {
            if normalized {
                buffer.scores[first] += buffer.scores[alias];
                buffer.scores[alias] = 0.0;
            } else {
                buffer.scores[first] = buffer.scores[first].max(buffer.scores[alias]);
                buffer.scores[alias] = -f32::MAX;
            }
        }
    }
}

impl Runtime {
    fn new(model: &Model, columns: Vec<usize>) -> Self {
        let mut first = HashMap::new();
        let mut label_indices = Vec::new();
        let mut alias_pairs = Vec::new();
        for (index, &column) in columns.iter().enumerate() {
            let label = &model.classes[column];
            if let Some(&previous) = first.get(label) {
                alias_pairs.push((previous, index));
            } else {
                first.insert(label, index);
                label_indices.push(index);
            }
        }
        Self {
            columns,
            label_indices,
            alias_pairs,
            pool: Mutex::new(Vec::new()),
            max_cached_buffers: std::thread::available_parallelism().map_or(1, |count| count.get()),
        }
    }
}

fn read_text(path: &Path) -> Result<Vec<u8>, Error> {
    std::fs::read(path).map_err(|error| {
        Error::Io(std::io::Error::new(
            error.kind(),
            format!("read text file {}: {error}", path.display()),
        ))
    })
}

pub fn default_identifier() -> Result<&'static Identifier, Error> {
    static IDENTIFIER: OnceLock<Result<Identifier, String>> = OnceLock::new();
    match IDENTIFIER.get_or_init(|| Identifier::new().map_err(|error| error.to_string())) {
        Ok(identifier) => Ok(identifier),
        Err(error) => Err(Error::InvalidModel(error.clone())),
    }
}

pub fn classify(text: impl AsRef<[u8]>) -> Result<LanguageScore, Error> {
    Ok(default_identifier()?.identify(text))
}

pub fn rank(text: impl AsRef<[u8]>) -> Result<Vec<LanguageScore>, Error> {
    Ok(default_identifier()?.rank(text))
}
