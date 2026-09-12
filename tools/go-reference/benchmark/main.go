package main

import (
	"bufio"
	"bytes"
	"crypto/sha256"
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"slices"
	"strings"
	"time"

	py3langid "github.com/markusmobius/go-py3langid"
)

type testCase struct {
	language string
	text     []byte
}

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func run() error {
	suite := flag.String("suite", "", "path to the original labeled JSONL corpus")
	passes := flag.Int("passes", 8, "number of measured passes after one warm-up")
	flag.Parse()
	if *suite == "" || *passes < 1 || flag.NArg() != 0 {
		return fmt.Errorf("usage: benchmark --suite PATH [--passes 8]; passes must be positive")
	}
	runtime.GOMAXPROCS(1)
	cases, err := loadSuite(*suite)
	if err != nil {
		return err
	}
	runtime.GC()
	started := time.Now()
	identifier, err := py3langid.NewDefaultIdentifier(py3langid.WithNormalizedProbabilities())
	startupMS := time.Since(started).Seconds() * 1000
	if err != nil {
		return err
	}
	supported := make(map[string]bool)
	for _, language := range identifier.Classes() {
		supported[language] = true
	}
	for _, sample := range cases {
		if !supported[sample.language] {
			return fmt.Errorf("suite contains unsupported language %q; no cases were filtered", sample.language)
		}
	}
	_, baseline, err := measurePass(identifier, cases)
	if err != nil {
		return err
	}
	durations := make([]time.Duration, 0, *passes)
	for pass := 0; pass < *passes; pass++ {
		duration, predictions, err := measurePass(identifier, cases)
		if err != nil {
			return err
		}
		if !slices.Equal(predictions, baseline) {
			return fmt.Errorf("predictions changed between passes")
		}
		durations = append(durations, duration)
	}
	return json.NewEncoder(os.Stdout).Encode(map[string]float64{
		"startup_ms":   startupMS,
		"pass_ms":      medianMS(durations),
		"accuracy_pct": accuracyPct(cases, baseline),
	})
}

func loadSuite(path string) ([]testCase, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	manifestData, err := os.ReadFile(filepath.Join(filepath.Dir(path), "manifest.json"))
	if err != nil {
		return nil, err
	}
	var manifest struct {
		SHA256      string `json:"suite_sha256"`
		SampleCount int    `json:"sample_count"`
	}
	if err := json.Unmarshal(manifestData, &manifest); err != nil {
		return nil, err
	}
	if manifest.SHA256 != fmt.Sprintf("%x", sha256.Sum256(data)) {
		return nil, fmt.Errorf("suite SHA-256 does not match its manifest")
	}
	var cases []testCase
	identifiers := make(map[string]bool)
	scanner := bufio.NewScanner(bytes.NewReader(data))
	scanner.Buffer(make([]byte, 4096), 1024*1024)
	for scanner.Scan() {
		var record struct {
			ID       string `json:"id"`
			Language string `json:"language"`
			Text     string `json:"text"`
		}
		if err := json.Unmarshal(scanner.Bytes(), &record); err != nil {
			return nil, err
		}
		if strings.TrimSpace(record.ID) == "" || strings.TrimSpace(record.Language) == "" || strings.TrimSpace(record.Text) == "" || identifiers[record.ID] {
			return nil, fmt.Errorf("invalid or duplicate fields on suite line %d", len(cases)+1)
		}
		identifiers[record.ID] = true
		cases = append(cases, testCase{language: record.Language, text: []byte(record.Text)})
	}
	if err := scanner.Err(); err != nil {
		return nil, err
	}
	if len(cases) == 0 || len(cases) != manifest.SampleCount {
		return nil, fmt.Errorf("suite is empty or its sample count does not match the manifest")
	}
	return cases, nil
}

func measurePass(identifier *py3langid.Identifier, cases []testCase) (time.Duration, []string, error) {
	predictions := make([]string, len(cases))
	started := time.Now()
	for index, sample := range cases {
		result, err := identifier.IdentifyBytes(sample.text)
		if err != nil {
			return 0, nil, err
		}
		predictions[index] = result.Language
	}
	return time.Since(started), predictions, nil
}

func medianMS(durations []time.Duration) float64 {
	slices.Sort(durations)
	return (durations[(len(durations)-1)/2].Seconds() + durations[len(durations)/2].Seconds()) * 500
}

func accuracyPct(cases []testCase, predictions []string) float64 {
	correct := 0
	for index, sample := range cases {
		if sample.language == predictions[index] {
			correct++
		}
	}
	return float64(correct) * 100 / float64(len(cases))
}
