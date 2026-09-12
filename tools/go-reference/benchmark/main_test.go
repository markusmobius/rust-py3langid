package main

import (
	"testing"
	"time"
)

func TestMedian(testingContext *testing.T) {
	for _, test := range []struct {
		values []time.Duration
		want   float64
	}{
		{[]time.Duration{9 * time.Millisecond, time.Millisecond, 3 * time.Millisecond}, 3},
		{[]time.Duration{4 * time.Millisecond, 2 * time.Millisecond}, 3},
		{[]time.Duration{7 * time.Millisecond}, 7},
	} {
		if got := medianMS(test.values); got != test.want {
			testingContext.Fatalf("median = %v, want %v", got, test.want)
		}
	}
}

func TestAccuracyCountsEveryExactLabel(testingContext *testing.T) {
	cases := []testCase{{language: "en"}, {language: "fr"}, {language: "zh"}, {language: "de"}}
	predictions := []string{"en", "und", "zh-Hans", "de"}
	if got := accuracyPct(cases, predictions); got != 50 {
		testingContext.Fatalf("accuracy = %v, want 50", got)
	}
}
