package auth

import "testing"

func TestOfflineUUIDString(t *testing.T) {
	cases := map[string]string{
		"Notch": "b50ad385-829d-3141-a216-7e7d7539ba7f",
		"Steve": "5627dd98-e6be-3c21-b8a8-e92344183641",
		"jeb_":  "a762f560-4fce-3236-812a-b80efff0b62b",
	}

	for name, want := range cases {
		got := OfflineUUIDString(name)
		if got != want {
			t.Errorf("OfflineUUIDString(%q) = %q, want %q", name, got, want)
		}
	}
}

func TestOfflineUUIDVersionAndVariant(t *testing.T) {
	for _, name := range []string{"a", "somewhat_longer_username", "🙂", ""} {
		b := OfflineUUID(name)
		version := b[6] >> 4
		variant := b[8] >> 6
		if version != 0x3 {
			t.Errorf("username %q: version nibble = %x, want 3", name, version)
		}
		if variant != 0b10 {
			t.Errorf("username %q: variant bits = %02b, want 10", name, variant)
		}
	}
}

func TestOfflineUUIDDeterministic(t *testing.T) {
	a := OfflineUUIDString("SomePlayer")
	b := OfflineUUIDString("SomePlayer")
	if a != b {
		t.Errorf("offline UUID not deterministic: %q != %q", a, b)
	}
}

func TestOfflineUUIDFormat(t *testing.T) {
	s := OfflineUUIDString("format_check")
	if len(s) != 36 {
		t.Fatalf("expected 36-char UUID string, got %d: %q", len(s), s)
	}
	for i, want := range "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx" {
		if want == '-' && s[i] != '-' {
			t.Fatalf("expected dash at position %d, got %q in %q", i, s[i], s)
		}
	}
}
