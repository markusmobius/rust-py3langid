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

### Results: 2026-09-12

All 1,000 original sentences were tested: 20 languages, 500 short and 500 medium
sentences. Both engines scored **992/1,000 (99.2%)**.

| Engine | Model startup (ms) | Pass time (ms) | Accuracy |
| --- | ---: | ---: | ---: |
| rust-py3langid | 292.47 | 41.88 | 99.2% |
| go-py3langid v0.4.0 | 328.21 | 42.56 | 99.2% |

Measured on WSL2 Linux/x86_64, AMD Ryzen AI 7 PRO 350, pinned to logical CPU 2.
Rust 1.98.1 used the default release profile; Go 1.27.1 used `CGO_ENABLED=0`,
`GOAMD64=v1`, `-trimpath`, and `x/text v0.42.0`. Classification is sequential;
the Go runner sets `GOMAXPROCS=1` and collects corpus-loading garbage before timing.

Each runner discards one complete warm-up pass and reports the median of eight
subsequent passes in the same corpus order, checking that predictions stay stable.
The table takes medians across eight fresh launches per engine, alternating
Rust-Go and Go-Rust order; an initial launch of each was discarded first.

`startup_ms` times model/identifier construction only, with a warm filesystem
cache, not OS process launch or compilation. `pass_ms` includes direct library
classification and storing returned labels; corpus loading, warm-up, accuracy
checks, and JSON output are excluded. These times are not comparable to the
original Python/TCP benchmark's end-to-end pass times.

Steady-state performance is essentially tied in this run: per-launch median pass
times ranged from 40.57-45.70 ms for Rust and 40.77-56.52 ms for Go, much wider than
the 0.68 ms difference between their medians. This selected 20-language corpus
does not establish universal accuracy or a general speedup.

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
manifest is required; unsupported languages are an error, never silently filtered.
Run the metric tests with `cargo test --locked --example benchmark` and
`go -C tools/go-reference test -mod=readonly ./benchmark`.

Corpus SHA-256:
`ec0ea263e5cdea41005cb2bc9fff1612e2b5a937c319a7c25162d2ad8b00e7a5`.
The original corpus and its CC-BY-SA 4.0 attribution remain in go-py3langid's
benchmark suite; no benchmark sentences are redistributed here.

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