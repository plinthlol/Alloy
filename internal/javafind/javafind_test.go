package javafind

import "testing"

func TestMajorFromVersionString_Modern(t *testing.T) {
	cases := map[string]int{
		"21":     21,
		"21.0.3": 21,
		"17.0.9": 17,
		"16":     16,
	}
	for in, want := range cases {
		got, err := majorFromVersionString(in)
		if err != nil {
			t.Errorf("%q: unexpected error: %v", in, err)
			continue
		}
		if got != want {
			t.Errorf("%q: got %d, want %d", in, got, want)
		}
	}
}

func TestMajorFromVersionString_Legacy(t *testing.T) {
	cases := map[string]int{
		"1.8.0_392": 8,
		"1.8":       8,
		"1.7.0_80":  7,
	}
	for in, want := range cases {
		got, err := majorFromVersionString(in)
		if err != nil {
			t.Errorf("%q: unexpected error: %v", in, err)
			continue
		}
		if got != want {
			t.Errorf("%q: got %d, want %d", in, got, want)
		}
	}
}

func TestParseProperty(t *testing.T) {
	sample := `
    java.class.version = 65.0
    java.specification.version = 21
    java.vendor = Eclipse Adoptium
    java.version = 21.0.3
`
	if got := parseProperty(sample, "java.version"); got != "21.0.3" {
		t.Errorf("got %q", got)
	}
	if got := parseProperty(sample, "java.specification.version"); got != "21" {
		t.Errorf("got %q", got)
	}
	if got := parseProperty(sample, "java.class.version"); got != "65.0" {
		t.Errorf("got %q", got)
	}
}

func mkVerified(major int) Verified {
	return Verified{MajorVersion: major}
}

func TestBest_ExactMatchPreferred(t *testing.T) {
	cands := []Verified{mkVerified(8), mkVerified(17), mkVerified(21)}
	got, ok := Best(cands, 17)
	if !ok || got.MajorVersion != 17 {
		t.Errorf("got %+v, ok=%v", got, ok)
	}
}

func TestBest_PrefersClosestAboveWhenNoExactMatch(t *testing.T) {
	cands := []Verified{mkVerified(8), mkVerified(17)}
	got, ok := Best(cands, 11)
	if !ok || got.MajorVersion != 17 {
		t.Errorf("got %+v, ok=%v, want major 17", got, ok)
	}
}

func TestBest_NeverPicksBelowRequirement(t *testing.T) {
	cands := []Verified{mkVerified(8), mkVerified(11)}
	_, ok := Best(cands, 21)
	if ok {
		t.Error("should not find a suitable candidate when all are below the requirement")
	}
}

func TestBest_PicksClosestAboveNotFurthest(t *testing.T) {
	cands := []Verified{mkVerified(11), mkVerified(17), mkVerified(21)}
	got, ok := Best(cands, 12)
	if !ok || got.MajorVersion != 17 {
		t.Errorf("got %+v, ok=%v, want closest-above (17), not furthest (21)", got, ok)
	}
}

func TestBest_EmptyCandidates(t *testing.T) {
	_, ok := Best(nil, 17)
	if ok {
		t.Error("expected no match with zero candidates")
	}
}
