# rust-py3langid

An in-progress Rust port of
[go-py3langid v0.4.0](https://github.com/markusmobius/go-py3langid/tree/v0.4.0),
preserving its py3langid 0.4.0 model and inference behavior. This is the first
component of a faithful Go-Trafilatura v2.2.0 port, not a new language model.

The library runs entirely in Rust with an embedded model. No runtime Go, Python,
network access, model download, native C library, or training step is needed.
It exposes 140 unique labels: 139 languages and `zxx` (non-linguistic content).
`und` is returned only by explicitly configured confidence-based abstention.

## Use

Until a release is explicitly approved, use the local checkout as a dependency:

```toml
[dependencies]
rust-py3langid = { path = "../rust-py3langid" }
```

```rust
use rust_py3langid::{Identifier, Options};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let identifier = Identifier::new()?;
    let result = identifier.identify("This text is in English.");
    assert_eq!(result.language, "en");
    assert_eq!(result.score, -68.56228637695312);

    let confidence = identifier.identify_normalized("This text is in English.");
    assert!(confidence.score > 0.7);

    identifier.set_languages(&["en", "de"])?;
    assert_eq!(identifier.classes(), ["de", "en"]);
    identifier.reset_languages();

    let cautious = Identifier::with_options(Options {
        normalized: true,
        min_confidence: Some(0.5),
    })?;
    assert_eq!(cautious.identify("Hi").language, "und");
    Ok(())
}
```

Keep an `Identifier` alive across calls. Independent identifiers share the
immutable decoded default model, not their configuration. Methods accept both
UTF-8 strings and byte slices through `AsRef<[u8]>`; invalid bytes follow Go's
preprocessing behavior, rather than replacement-character decoding.

`classify` and `rank` use `default_identifier()`, whose configuration is shared.
Use a separate `Identifier::new()` for a Trafilatura instance or independent
language restrictions. Explicit normalized methods do not change the default
score mode. `min_confidence` requires `normalized: true`, affects identification
only, and never replaces labels in a ranking.

### Diagnostic Example

From this repository root, using a POSIX shell:

```sh
printf '%s' 'This text is in English.' | cargo run --locked --release --example classify
printf '%s' 'This text is in English.' | cargo run --locked --release --example classify -- --normalized
printf '%s' 'Hi' | cargo run --locked --release --example classify -- --normalized --min-confidence 0.5
printf '%s' 'This text is in English.' | cargo run --locked --release --example classify -- --rank --languages de,en
```

This example reads all stdin bytes once and emits JSON. It is not the future
`rustHTML` worker, a server, or a port of the complete Go CLI. It uses development
dependencies only; JSON support is not required by the library.

## Speed and Accuracy Benchmark

The [Rust runner](examples/benchmark.rs) and
[equivalent Go runner](tools/go-reference/benchmark/main.go) read the existing
go-py3langid FLORES-200 benchmark corpus and its manifest in place. They do not
copy the data or use Python, worker processes, or TCP for classification.
Both verify the corpus hash/count, retain all 140 candidate labels, use calibrated
probabilities without abstention, and count every exact-label prediction.

All tables below were rerun after the retained
[within-thread optimizations](#within-thread-optimizations). They supersede the
earlier pre-optimization and experimental timing tables.

### Results: 2026-09-12

All 1,000 original sentences were tested: 20 languages, 500 short and 500 medium
sentences. Both engines scored **992/1,000 (99.2%)**.

| Engine | Model startup (ms) | Pass time (ms) | Accuracy |
| --- | ---: | ---: | ---: |
| rust-py3langid, portable release | 292.21 | 17.22 | 99.2% |
| go-py3langid v0.4.0 | 330.12 | 46.92 | 99.2% |

Measured on WSL2 Linux/x86_64, AMD Ryzen AI 7 PRO 350, pinned to logical CPU 2.
Rust 1.98.1 used the default release profile; Go 1.27.1 used `CGO_ENABLED=0`,
`GOAMD64=v1`, `-trimpath`, and `x/text v0.42.0`. Classification is sequential;
the Go runner sets `GOMAXPROCS=1` and collects corpus-loading garbage before timing.

Each runner discards one complete warm-up pass and reports the median of eight
subsequent passes in the same corpus order, checking that predictions stay stable.
The table takes medians across twelve fresh launches per configuration, using
rotating forward and reverse orders across portable Rust, Go, and the optional
native Rust build below. Each configuration occupied each execution position
four times; an initial launch of each was discarded first. No optional competitor
features were enabled in these Rust builds.

`startup_ms` times model/identifier construction only, with a warm filesystem
cache, not OS process launch or compilation. `pass_ms` includes direct library
classification and storing returned labels; corpus loading, warm-up, accuracy
checks, and JSON output are excluded. These times are not comparable to the
original Python/TCP benchmark's end-to-end pass times.

Portable Rust used **63.3% less pass time (2.72x throughput)** than Go in this run.
Per-launch median pass times ranged from 16.38-21.53 ms for Rust and 43.37-62.81 ms
for Go. All measured runs were retained. This selected 20-language corpus does
not establish universal accuracy or a general speedup.

### Build and Run

From this repository root in Linux/WSL, pointing at the existing Go checkout:

```sh
SUITE=../../go-py3langid/benchmarks/language-detection/suite/flores200.jsonl
cargo build --locked --release --example benchmark
GOTOOLCHAIN=go1.27.1 CGO_ENABLED=0 GOAMD64=v1 go -C tools/go-reference build -mod=readonly -trimpath -o ../../target/benchmark-go ./benchmark
taskset -c 2 target/release/examples/benchmark --suite "$SUITE" --passes 8
taskset -c 2 target/benchmark-go --suite "$SUITE" --passes 8
```

Adjust the checkout path and choose an available CPU in place of `2`. Each
executable prints just `startup_ms`, `pass_ms`, and `accuracy_pct` as one JSON
object. `--passes` defaults to eight and must be positive. The neighboring
manifest is required; by default, unsupported languages are an error, never
silently filtered. The optional comparison below adds explicit subset selection.
Run the metric tests with `cargo test --locked --example benchmark` and
`go -C tools/go-reference test -mod=readonly ./benchmark`.

Corpus SHA-256:
`ec0ea263e5cdea41005cb2bc9fff1612e2b5a937c319a7c25162d2ad8b00e7a5`.
The original corpus and its CC-BY-SA 4.0 attribution remain in go-py3langid's
benchmark suite; no benchmark sentences are redistributed here.

### Rust Library Comparison: 2026-09-12

A separate current-build run compares this port with
[Whatlang](https://github.com/greyblake/whatlang-rs),
[Lingua](https://github.com/pemistahl/lingua-rs), and
[Whichlang](https://github.com/quickwit-oss/whichlang). All classification runs
directly in Rust using the same runner and the original corpus in place.

This table uses the optional comparison-feature build. It is a separate execution
batch from the Rust/Go table; compare engines within a table, not timings between
tables. Both use the same retained inference code and portable release profile.

| Engine | Samples | Model startup (ms) | Pass time (ms) | Accuracy |
| --- | ---: | ---: | ---: | ---: |
| rust-py3langid 0.1.0 | 1,000 | 276.79 | 15.15 | 99.20% (992/1,000) |
| Whatlang 0.18.0 | 1,000 | <0.01[^startup] | 36.45 | 99.30% (993/1,000) |
| Lingua 1.8.0, high accuracy | 1,000 | 69.21 | 1,349.42 | 98.70% (987/1,000) |
| Whichlang 0.1.1[^whichlang-subset] | 800 | 0[^startup] | 1.05 | 99.875% (799/800) |
| rust-py3langid, matching subset[^whichlang-subset] | 800 | 286.87 | 12.06 | 99.25% (794/800) |

[^whichlang-subset]: Whichlang supports only 16 languages, so it was run only on
        the 800 supported samples. The 200 Indonesian, Polish, Thai, and Ukrainian
        samples were excluded from both subset rows, not counted as errors. The
        matching rust-py3langid row uses those exact same 800 inputs. Its detector
        still considers all 140 labels; selecting samples does not restrict candidate
        languages. The subset scores and batch times are not directly comparable to
        the 1,000-sample rows.

[^startup]: Whatlang's lightweight constructor measured below 0.01 ms. Whichlang
        has no runtime model-construction step and reports zero. These measurements
        exclude OS process startup and cold executable-page loading, not just disk
        reads of the corpus; they are not end-to-end startup latency.

All supported candidate languages remain enabled: 140 labels for rust-py3langid,
70 for Whatlang, 75 for Lingua, and 16 for Whichlang. Rust uses calibrated
probabilities without abstention. Whatlang uses `Detector::new()` without
reliability filtering. Lingua uses `from_all_languages()` with
`with_preloaded_language_models()`, default high-accuracy mode, and the default
minimum relative distance. Model preloading is included in its startup time.
Unknown results count as incorrect; no confidence-based sample filtering is used.
Native language labels are compared outside timing, with FLORES Arabic/Mandarin
codes `arb`/`zho` mapped to `ara`/`cmn` for Whatlang and Whichlang.

The host, Rust version, release profile, and timing boundaries match the first
comparison. Each of the five configurations ran in ten fresh processes, using
five rotating forward orders followed by five rotating reverse orders, so every
configuration occupied each execution position twice. The initial smoke launch
of each configuration was discarded. Each measured launch discarded one warm-up
pass, then reported the median of eight timed passes; the table takes the median
of those ten reports. CPU affinity was `2`, with `RAYON_NUM_THREADS=1`.

Accuracy was constant across all launches. Launch-median pass ranges were
14.05-18.22 ms for rust-py3langid, 35.41-42.59 ms for Whatlang,
1,302.93-1,618.78 ms for Lingua, 1.01-1.17 ms for Whichlang, and 11.64-15.15 ms
for the matching Rust subset. All runs were retained. These are corpus-specific
results with different candidate sets, not a universal accuracy or speed ranking.

#### Why the Go Lingua Ratio Differs

The [Go repository's comparison](https://github.com/markusmobius/go-py3langid#language-detector-comparison)
reports 2,358.7 ms for Lingua and 498.2 ms for go-py3langid, about 4.7x. Those are
end-to-end pass times including Python, JSON, and one TCP round trip per text.
Here, those costs are excluded. Shared per-request overhead can compress the
ratio between fast and slow classifiers; the 4.7x ratio is not a direct-library
performance ratio.

The configurations were checked: both use all 75 languages, default high
accuracy, and zero minimum relative distance. The Go worker uses lingua-go 1.4.0;
this table uses lingua-rs 1.8.0, not the same implementation. Direct Go spot checks
of both `DetectLanguageOf` and the worker's `ComputeLanguageConfidenceValues`
path, without TCP, took roughly 1.1-1.9 seconds per 1,000 texts and reproduced
987/1,000 correct. These diagnostic observations support the second-scale
inference cost, but are not a separate controlled Go/Rust ranking.

Startup boundaries also differ: the Go worker's launch-to-ready measurement
precedes lazy loading during warm-up. This Rust runner explicitly preloads models
inside the model-construction timer. Both exclude warm-up from measured passes.

#### Run the Comparison

The competitor crates and all 75 Lingua language models are opt-in dependencies;
the default library build does not include them. From the repository root:

```sh
SUITE=../../go-py3langid/benchmarks/language-detection/suite/flores200.jsonl
cargo build --locked --release --example benchmark --features benchmark-comparison
for engine in rust-py3langid whatlang lingua whichlang; do
    RAYON_NUM_THREADS=1 taskset -c 2 target/release/examples/benchmark \
        --suite "$SUITE" --engine "$engine" --passes 8
done
RAYON_NUM_THREADS=1 taskset -c 2 target/release/examples/benchmark \
    --suite "$SUITE" --engine rust-py3langid --subset whichlang --passes 8
```

Each launch still emits only `startup_ms`, `pass_ms`, and `accuracy_pct` as JSON.
`--engine` defaults to `rust-py3langid`; `--engine whichlang` automatically selects
its supported samples. `--subset whichlang` applies that same sample selection
to any engine without changing its candidate languages. The original manifest
hash/count are validated before any selection, and an empty subset is an error.

The optional adapter tests require no corpus. To also verify the exact 1,000/800
sample counts against the existing checkout, run:

```sh
BENCHMARK_SUITE="$SUITE" cargo test --locked --example benchmark \
    --features benchmark-comparison -- --include-ignored
```

### Optional CPU-Specific Build

An opt-in `-C target-cpu=native` build was measured alongside portable Rust and
Go in the same twelve-launch run above, using the current optimized source:

| Build | Model startup (ms) | Pass time (ms) | Accuracy |
| --- | ---: | ---: | ---: |
| Portable release | 292.21 | 17.22 | 99.2% |
| Native CPU release | 279.37 | 16.81 | 99.2% |

The native median was only 2.4% lower in this rerun, with heavily overlapping
pass ranges: 16.38-21.53 ms and 14.84-23.98 ms respectively. That is not a robust
additional speedup claim. All 68 Go reference cases still matched exactly.
Native targeting changes code generation throughout the binary and its
dependencies, not only SIMD width; it does not establish a startup improvement.

For a Linux/WSL build that will run on the build machine's CPU:

```sh
RUSTFLAGS="-C target-cpu=native" cargo build --locked --release \
    --example benchmark --target-dir target/native
```

Run `target/native/release/examples/benchmark` with the same arguments as above.
This remains opt-in: the binary can require instructions missing on other CPUs
and is not a portable distribution build. No manual SIMD, approximate math, or
default compiler flag changes were added.

### Within-Thread Optimizations

The default build now interleaves byte ranges within a single classification
call. An exhaustive DFA-equivalence test proves that the embedded model's state
after six bytes is independent of the initial state. Later ranges are primed
with that lookbehind, then their emitted features are replayed in original byte
order. This exposes independent memory reads without changing feature counts or
their first-encounter order. Inputs of at least 128 encoded bytes use eight
streams; shorter inputs use four where worthwhile, with serial tails.

With all languages enabled, the scoring loop reads contiguous weight rows and
handles four features together, preserving the exact float32 multiplication and
addition order for each language. Restricted-language scoring retains the
original indexed path. There is no approximate math, unsafe code, model change,
batch API, or internal threading. Calls remain thread-safe; application-level
parallelism belongs to the caller. All current tables above include these
optimizations. Model initialization was not optimized.

All 68 pinned-Go reference cases retained exact encoded bytes, labels, ranking
order, and raw/normalized scores. The exhaustive history proof, boundary parity
tests, and full-corpus serial traversal comparison pass on Linux/WSL and
Windows/GNU with Rust 1.98.1. Other platforms have not been executed locally.

The byte-interleaving fast path applies only to the pinned embedded model whose
history bound is proved. Custom models loaded from bytes or files retain serial
DFA traversal. Four-row scoring still applies when all their columns are active.
The widest byte path uses a fixed 8 KiB feature buffer, not a second model copy.
Default builds need no architecture-specific instructions; `target-cpu=native`
remains a separate opt-in build and is not a portable distribution setting.

To check every raw/normalized score against serial traversal on the existing
corpus without copying it:

```sh
SUITE=../../go-py3langid/benchmarks/language-detection/suite/flores200.jsonl
BENCHMARK_SUITE="$SUITE" cargo test --locked --release --example benchmark \
    original_suite_inference_parity -- --ignored
```

## Compatibility

| Go surface | Rust surface / status |
| --- | --- |
| `NewDefaultIdentifier`, options | `Identifier::new`, `with_options`, `Options` |
| `LoadModel` | `Identifier::from_path`, `from_model`; `Model::from_path`, `from_bytes` |
| `IdentifyString`, `IdentifyBytes` | `identify` accepts strings or bytes |
| `RankString`, `RankBytes` | `rank` accepts strings or bytes |
| Explicit normalized methods | `identify_normalized`, `rank_normalized` |
| `IdentifyFile`, `RankFile` | `identify_file`, `rank_file` |
| `Classes`, restrictions/reset | `classes`, `set_languages`, `reset_languages` |
| Package-level helpers | `classify`, `rank`, `default_identifier()` and its methods |
| Concurrent inference/configuration | Immutable per-call snapshots; bounded idle buffer pools |
| Training, HTTP/URL service, complete CLI | Deferred; not needed for Trafilatura's classification call |

The decoder preserves all 142 internal model columns, including Serbian and
Uzbek aliases. Raw alias scores use the maximum; calibrated probabilities are
summed only after normalization. Rankings contain unique labels, ordered by
descending score with stable model-order ties. Restrictions retain model order,
not caller order, and validate all requested labels before replacing a snapshot.

Inference preserves byte-DFA traversal, first-encounter feature order, sparse
`log1p` counts, float32 multiplication/addition and priors. Normalized scoring
uses the encoded byte length's inverse square root before softmax. Featureless
raw input returns `-f32::MAX`, not a prior-based guess. A merged probability can
be slightly above one because of float32 rounding; it is deliberately not clamped.

Preprocessing uses Unicode 17 data, full root-locale lowercase when every cased
character is uppercase, and stream-safe NFC matching Go. ICU case/property data
are pinned to the ICU 78.1 / CLDR 48 generation; unicode-normalization is pinned
to 0.1.25. Do not upgrade these tables as incidental dependency maintenance.

Implementation choices that do not change tested inference: the default model
is shared through `Arc`; restrictions select original matrix columns instead of
copying weights; a short `RwLock` protects snapshot replacement. Inference does
not hold that lock. Each snapshot caches at most the host's available parallelism
in idle work buffers; concurrent active requests are not limited by this cap.

Known custom-model boundary: Rust rejects non-UTF-8 class labels because its
public labels are `String`; Go can construct an identifier containing arbitrary
label bytes. Full compatibility for such malformed labels is unqualified. Error
messages and Rust error types are not promised to be Go-string-identical.

## Verification

The working toolchain is pinned to Rust 1.98.1. Earlier compilers are not yet
qualified. Commit the Cargo lockfile when this work is ready to commit.

```sh
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release --examples
```

Default tests need no Go/Python installation and run offline after Cargo
dependencies have been fetched. They include model hashes/dimensions, malformed
models, all 42 unchanged Python reference cases, complete raw/normalized score
vectors, aliases, restrictions, files, reuse, and concurrent configuration.

Python reference score tolerances are inherited from Go's upstream tests:
raw `1e-5 + 1e-5 * abs(expected)`; normalized
`2e-6 + 1e-4 * abs(expected)`. Encoded bytes, labels, class order, and model hashes
are exact. Reference JSON enables round-trip float parsing so decimal parsing
does not introduce false score differences.

### Live Go Check

Optional, separate from runtime and ordinary tests. Requires Go 1.27.1 for the
recorded baseline, with dependency downloads on first use:

```sh
go -C tools/go-reference run -mod=readonly .
cargo test --locked --lib live_go_parity -- --ignored --nocapture
cargo test --locked --release --lib live_go_parity -- --ignored --nocapture
```

The Go oracle checks its compiled dependency versions and rejects replacements.
It uses go-py3langid v0.4.0 and the target worker's `x/text v0.42.0`, plus Unicode
17.0.0. The private Go preprocessing function is reproduced in the test-only
oracle from the pinned source; scores and rankings call the actual Go library.

It writes ignored `target/go-reference.json`, with fingerprints, toolchain
metadata, the 42 original inputs and 26 extra boundary/long-input cases. The Rust
check compares encoded bytes, winning labels, every ranked label in order, and
every score. Its tolerances are raw `1e-6 + 1e-6 * abs(expected)` and normalized
`1e-7 + 1e-6 * abs(expected)`. An alternate generated report can be supplied using
`GO_PY3LANGID_REFERENCE`. Expected outputs are never generated by Rust.

Observed on 2026-09-12: Linux debug/release and native Windows/GNU release builds
matched all 68 Go cases exactly, with zero raw or normalized score differences.
The 42 Python cases, library tests, doctest, Clippy, formatting, and release builds
also passed locally. This is corpus-scoped evidence, not a claim of universal
equivalence or completed production validation.

### Windows Without MSVC

Normal Windows builds need the Visual C++ tools and Windows SDK. With an existing
MinGW installation, select a GNU-hosted Rust compiler, not merely a GNU target:

```powershell
rustup toolchain install 1.98.1-x86_64-pc-windows-gnu --profile minimal --component rustfmt --component clippy
$env:PATH = "C:\msys64\ucrt64\bin;$env:PATH"
$env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = 'C:\msys64\ucrt64\bin\gcc.exe'
cargo +1.98.1-x86_64-pc-windows-gnu test --locked --target-dir target/windows-gnu
```

The path shown is the toolchain used for local validation, not a required install
location. Keeping its target directory separate avoids mixing Windows host build
artifacts with WSL's. The configured CI runs Linux and Windows/MSVC; macOS,
ARM64, older Rust versions, and remote CI execution are still to be qualified.

## Provenance

Go source: `d3e0c0861455d7d84daedb994392d2e71a0f6270` (`v0.4.0`).
Python source: `3b99caf00d0dcbc9416a9d06f8e5b690e919a74e` (`0.4.0`).
The upstream numeric reference records Python 3.14.6 and NumPy 2.5.1.

The following files were exported unchanged from that Go commit, not regenerated:

| File | SHA-256 |
| --- | --- |
| [model/py3langid.lidg](model/py3langid.lidg) | `da6860a9218a6122bcf26ba6e8752946336dc4f9c0816c6eb4fa790c7444dec2` |
| [testdata/py3langid_cases.json](testdata/py3langid_cases.json) | `142bbd01955c49795377c8d29568260055f2a15ec1fcca1d7beb0339b0f10233` |
| [testdata/py3langid_reference.json](testdata/py3langid_reference.json) | `afcee310dfc8f3647bfe0ec3aee1b42388f00bfb6bf4a68dda2990c3a7915534` |
| [LICENSE](LICENSE) | `6e2225975f1dc3b1668ad6f683c1aecef4a53a6d3012344e85396a127f6c6237` |

The model has 100,053 features, 104,583 states, and 142 internal score columns.
The original Python model fingerprint is
`f4f4a2c3465ca1f081541037f9cac23021d68151c2f709c55ae5eedfca522963`.
The converted `.lidg` format is intentionally reused without retraining.

BSD-3-Clause terms and the original Marco Lui, Adrien Barbaresi and Ilya Pyshkin
notices are preserved in [LICENSE](LICENSE). Binary distributions must retain
these and the notices required by Cargo dependencies, including Unicode data.
No release or publication has been made. Peak/retained-memory measurements and
distribution-notice review remain before the M1 handoff is complete.