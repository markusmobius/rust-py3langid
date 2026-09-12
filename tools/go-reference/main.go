package main

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"runtime/debug"
	"strings"
	"unicode"
	"unicode/utf8"

	py3langid "github.com/markusmobius/go-py3langid"
	"golang.org/x/text/cases"
	"golang.org/x/text/language"
	"golang.org/x/text/unicode/norm"
)

const sourceCommit = "d3e0c0861455d7d84daedb994392d2e71a0f6270"
const modelDigest = "da6860a9218a6122bcf26ba6e8752946336dc4f9c0816c6eb4fa790c7444dec2"

type inputCase struct {
	Name          string   `json:"name"`
	Text          string   `json:"text"`
	Hex           string   `json:"hex"`
	TrimBytes     int      `json:"trim_bytes"`
	Languages     []string `json:"languages"`
	MinConfidence *float64 `json:"min_confidence"`
}

type score struct {
	Language string  `json:"language"`
	Score    float64 `json:"score"`
}

type sample struct {
	Name           string   `json:"name"`
	Input          []byte   `json:"input"`
	Encoded        []byte   `json:"encoded"`
	Languages      []string `json:"languages"`
	MinConfidence  *float64 `json:"min_confidence"`
	Raw            score    `json:"raw"`
	Normalized     score    `json:"normalized"`
	RawRank        []score  `json:"raw_rank"`
	NormalizedRank []score  `json:"normalized_rank"`
}

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func run() error {
	casePath := flag.String("cases", "../../testdata/py3langid_cases.json", "pinned input corpus")
	modelPath := flag.String("model", "../../model/py3langid.lidg", "model to fingerprint")
	outputPath := flag.String("output", "../../target/go-reference.json", "generated reference JSON")
	flag.Parse()
	if err := checkVersions(); err != nil {
		return err
	}
	modelBytes, err := os.ReadFile(*modelPath)
	if err != nil {
		return err
	}
	if fmt.Sprintf("%x", sha256.Sum256(modelBytes)) != modelDigest {
		return fmt.Errorf("model does not match pinned v0.4.0 fingerprint")
	}
	corpusBytes, err := os.ReadFile(*casePath)
	if err != nil {
		return err
	}
	var corpus struct {
		Cases []inputCase `json:"cases"`
	}
	if err := json.Unmarshal(corpusBytes, &corpus); err != nil {
		return err
	}
	if len(corpus.Cases) != 42 {
		return fmt.Errorf("expected 42 pinned cases, got %d", len(corpus.Cases))
	}
	corpus.Cases = append(corpus.Cases, extraCases()...)
	raw, err := py3langid.NewDefaultIdentifier()
	if err != nil {
		return err
	}
	normalized, err := py3langid.NewDefaultIdentifier(py3langid.WithNormalizedProbabilities())
	if err != nil {
		return err
	}
	samples := make([]sample, 0, len(corpus.Cases))
	for _, input := range corpus.Cases {
		text := []byte(input.Text)
		if input.Hex != "" {
			text, err = hex.DecodeString(input.Hex)
			if err != nil {
				return err
			}
		}
		if input.TrimBytes < 0 || input.TrimBytes > len(text) {
			return fmt.Errorf("invalid trim in %s", input.Name)
		}
		text = text[:len(text)-input.TrimBytes]
		normalizedID := normalized
		if input.MinConfidence != nil {
			normalizedID, err = py3langid.NewDefaultIdentifier(py3langid.WithNormalizedProbabilities(), py3langid.WithMinConfidence(*input.MinConfidence))
			if err != nil {
				return err
			}
		}
		if err := raw.SetLanguages(input.Languages...); err != nil {
			return err
		}
		if err := normalizedID.SetLanguages(input.Languages...); err != nil {
			return err
		}
		rawResult, err := raw.IdentifyBytes(text)
		if err != nil {
			return err
		}
		normalizedResult, err := normalizedID.IdentifyBytes(text)
		if err != nil {
			return err
		}
		rawRank, err := raw.RankBytes(text)
		if err != nil {
			return err
		}
		normalizedRank, err := normalizedID.RankBytes(text)
		if err != nil {
			return err
		}
		samples = append(samples, sample{
			Name: input.Name, Input: text, Encoded: encodeReference(text),
			Languages: input.Languages, MinConfidence: input.MinConfidence,
			Raw: score(rawResult), Normalized: score(normalizedResult),
			RawRank: convertRank(rawRank), NormalizedRank: convertRank(normalizedRank),
		})
	}
	report := map[string]any{
		"go_module": "github.com/markusmobius/go-py3langid", "go_version": "v0.4.0",
		"go_commit": sourceCommit, "x_text": "v0.42.0", "toolchain": runtime.Version(),
		"unicode": unicode.Version, "normalization_unicode": norm.Version,
		"model_sha256": modelDigest, "cases_sha256": fmt.Sprintf("%x", sha256.Sum256(corpusBytes)),
		"samples": samples,
	}
	data, err := json.MarshalIndent(report, "", "  ")
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(*outputPath), 0o755); err != nil {
		return err
	}
	if err := os.WriteFile(*outputPath, append(data, '\n'), 0o644); err != nil {
		return err
	}
	fmt.Fprintf(os.Stderr, "Generated %d pinned-Go cases at %s\n", len(samples), *outputPath)
	return nil
}

func checkVersions() error {
	info, ok := debug.ReadBuildInfo()
	if !ok {
		return fmt.Errorf("missing Go build information")
	}
	for path, version := range map[string]string{
		"github.com/markusmobius/go-py3langid": "v0.4.0",
		"golang.org/x/text":                    "v0.42.0",
	} {
		found := false
		for _, dependency := range info.Deps {
			if dependency.Path == path {
				if dependency.Version != version || dependency.Replace != nil {
					return fmt.Errorf("reference must use %s %s without replacement", path, version)
				}
				found = true
			}
		}
		if !found {
			return fmt.Errorf("missing reference dependency %s", path)
		}
	}
	if unicode.Version != "17.0.0" || norm.Version != "17.0.0" {
		return fmt.Errorf("reference requires Unicode 17.0.0, got %s/%s", unicode.Version, norm.Version)
	}
	return nil
}

func convertRank(ranked []py3langid.Result) []score {
	results := make([]score, len(ranked))
	for index, result := range ranked {
		results[index] = score(result)
	}
	return results
}

func encodeReference(text []byte) []byte {
	if !utf8.Valid(text) {
		valid := false
		for trim := 1; trim <= 3 && trim <= len(text); trim++ {
			candidate := text[:len(text)-trim]
			if utf8.Valid(candidate) {
				text = candidate
				valid = true
				break
			}
		}
		if !valid {
			return text
		}
	}
	allUpper := false
	for _, letter := range string(text) {
		if unicode.IsLower(letter) || unicode.IsTitle(letter) || unicode.Is(unicode.Other_Lowercase, letter) {
			allUpper = false
			break
		}
		if unicode.IsUpper(letter) || unicode.Is(unicode.Other_Uppercase, letter) {
			allUpper = true
		}
	}
	if allUpper {
		text = cases.Lower(language.Und).Bytes(text)
	}
	return norm.NFC.Bytes(text)
}

func extraCases() []inputCase {
	inputs := []inputCase{
		{Name: "go/titlecase", Text: "TEST \u01c5"},
		{Name: "go/other_lowercase", Text: "TEST \u00aa"},
		{Name: "go/other_uppercase", Text: "\u2163 TEST"},
		{Name: "go/sigma_ignorable", Text: "\u039f\u03a3\u0301'"},
		{Name: "go/sigma_following", Text: "\u039f\u03a3'\u0391"},
		{Name: "go/dotted_i", Text: "I \u0130 \u0131"},
		{Name: "go/nfc_reordering", Text: "Test a\u0315\u0300\u05ae\u0301"},
		{Name: "go/hangul", Text: "\u1100\u1161\u11a8 \uac01"},
		{Name: "go/invalid_internal", Hex: "54455354ff4142434445"},
		{Name: "go/one_invalid_byte", Hex: "ff"},
		{Name: "go/three_invalid_bytes", Hex: "fffefd"},
		{Name: "go/four_invalid_bytes", Hex: "fffefdfc"},
		{Name: "go/serbian_uzbek_aliases", Text: "Toshkent shahri markazida", Languages: []string{"uz", "sr", "uz"}},
		{Name: "go/reversed_subset", Text: "This text is in English.", Languages: []string{"en", "fr", "de", "en"}},
	}
	for _, count := range []int{29, 30, 31, 60, 61, 256} {
		inputs = append(inputs, inputCase{Name: fmt.Sprintf("go/stream_safe_%d", count), Text: "a" + strings.Repeat("\u0301", count)})
	}
	for trim := 1; trim <= 3; trim++ {
		inputs = append(inputs, inputCase{Name: fmt.Sprintf("go/truncated_utf8_%d", trim), Text: "THIS IS ENGLISH. \U0001f600", TrimBytes: trim})
	}
	for _, length := range []int{64, 1024, 102400} {
		text := strings.Repeat("This text is in English. ", length/24+1)[:length]
		inputs = append(inputs, inputCase{Name: fmt.Sprintf("go/english_%d_bytes", length), Text: text})
	}
	return inputs
}
