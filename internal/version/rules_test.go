package version

import (
	"encoding/json"
	"reflect"
	"testing"
)

func TestRulesAllow_Empty(t *testing.T) {
	if !RulesAllow(nil, Env{OSName: "linux"}) {
		t.Error("empty rule list should always allow")
	}
}

func TestRulesAllow_SingleAllowAll(t *testing.T) {
	rules := []Rule{{Action: "allow"}}
	if !RulesAllow(rules, Env{OSName: "windows"}) {
		t.Error("unconditional allow rule should allow on any OS")
	}
}

func TestRulesAllow_OSRestricted(t *testing.T) {
	rules := []Rule{{Action: "allow", OS: &OSRule{Name: "osx"}}}
	if !RulesAllow(rules, Env{OSName: "osx"}) {
		t.Error("should allow when OS matches")
	}
	if RulesAllow(rules, Env{OSName: "linux"}) {
		t.Error("should not allow when OS doesn't match and no earlier allow-all rule exists")
	}
}

func TestRulesAllow_AllowAllThenDisallowOSX(t *testing.T) {
	rules := []Rule{
		{Action: "allow"},
		{Action: "disallow", OS: &OSRule{Name: "osx"}},
	}
	if !RulesAllow(rules, Env{OSName: "linux"}) {
		t.Error("linux should be allowed (only osx is excluded)")
	}
	if !RulesAllow(rules, Env{OSName: "windows"}) {
		t.Error("windows should be allowed (only osx is excluded)")
	}
	if RulesAllow(rules, Env{OSName: "osx"}) {
		t.Error("osx should be disallowed by the later, more specific rule")
	}
}

func TestRulesAllow_LastMatchingRuleWins(t *testing.T) {
	rules := []Rule{
		{Action: "disallow", OS: &OSRule{Name: "linux"}},
		{Action: "allow"},
	}
	if !RulesAllow(rules, Env{OSName: "linux"}) {
		t.Error("later unconditional allow should override the earlier linux-specific disallow")
	}
}

func TestRulesAllow_FeatureFlags(t *testing.T) {
	rules := []Rule{{Action: "allow", Features: map[string]bool{"is_demo_user": true}}}
	if RulesAllow(rules, Env{IsDemoUser: false}) {
		t.Error("should not allow when required feature flag is absent")
	}
	if !RulesAllow(rules, Env{IsDemoUser: true}) {
		t.Error("should allow when required feature flag is present")
	}
}

func TestRulesAllow_UnknownFeatureTreatedAsAbsent(t *testing.T) {
	rules := []Rule{{Action: "allow", Features: map[string]bool{"some_future_flag": true}}}
	if RulesAllow(rules, Env{}) {
		t.Error("unknown feature flags should be treated as not present, so the rule shouldn't match")
	}
}

func TestRuleApplies_OSVersionRegex(t *testing.T) {
	r := Rule{OS: &OSRule{Name: "windows", Version: `^10\.`}}
	if !RuleApplies(r, Env{OSName: "windows", OSVersion: "10.0.19045"}) {
		t.Error("expected version regex to match 10.0.19045")
	}
	if RuleApplies(r, Env{OSName: "windows", OSVersion: "6.1.7601"}) {
		t.Error("expected version regex not to match 6.1.7601 (Windows 7)")
	}
}

func TestArgumentEntry_UnmarshalPlainString(t *testing.T) {
	var e ArgumentEntry
	if err := json.Unmarshal([]byte(`"--username"`), &e); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(e.Value, []string{"--username"}) {
		t.Errorf("got %#v", e.Value)
	}
	if e.Rules != nil {
		t.Errorf("plain string entry should have no rules, got %#v", e.Rules)
	}
}

func TestArgumentEntry_UnmarshalConditionalSingleValue(t *testing.T) {
	raw := `{"rules":[{"action":"allow","os":{"name":"osx"}}],"value":"-XstartOnFirstThread"}`
	var e ArgumentEntry
	if err := json.Unmarshal([]byte(raw), &e); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(e.Value, []string{"-XstartOnFirstThread"}) {
		t.Errorf("got %#v", e.Value)
	}
	if len(e.Rules) != 1 || e.Rules[0].OS.Name != "osx" {
		t.Errorf("got %#v", e.Rules)
	}
}

func TestArgumentEntry_UnmarshalConditionalArrayValue(t *testing.T) {
	raw := `{"rules":[{"action":"allow","features":{"is_demo_user":true}}],"value":["--demo"]}`
	var e ArgumentEntry
	if err := json.Unmarshal([]byte(raw), &e); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(e.Value, []string{"--demo"}) {
		t.Errorf("got %#v", e.Value)
	}
}

func TestResolveArguments_SubstitutionAndFiltering(t *testing.T) {
	entries := []ArgumentEntry{
		{Value: []string{"--username", "${auth_player_name}"}},
		{Rules: []Rule{{Action: "allow", OS: &OSRule{Name: "osx"}}}, Value: []string{"-XstartOnFirstThread"}},
	}
	subs := map[string]string{"auth_player_name": "Notch"}

	linuxArgs := ResolveArguments(entries, Env{OSName: "linux"}, subs)
	want := []string{"--username", "Notch"}
	if !reflect.DeepEqual(linuxArgs, want) {
		t.Errorf("linux: got %#v, want %#v", linuxArgs, want)
	}

	osxArgs := ResolveArguments(entries, Env{OSName: "osx"}, subs)
	want = []string{"--username", "Notch", "-XstartOnFirstThread"}
	if !reflect.DeepEqual(osxArgs, want) {
		t.Errorf("osx: got %#v, want %#v", osxArgs, want)
	}
}

func TestResolveLibraries_FiltersByOS(t *testing.T) {
	libs := []Library{
		{Name: "shared:lib:1.0"},
		{Name: "windows:only:1.0", Rules: []Rule{{Action: "allow", OS: &OSRule{Name: "windows"}}}},
		{Name: "linux:only:1.0", Rules: []Rule{{Action: "allow", OS: &OSRule{Name: "linux"}}}},
	}
	resolved := ResolveLibraries(libs, Env{OSName: "linux"})
	if len(resolved) != 2 {
		t.Fatalf("expected 2 libraries for linux, got %d: %#v", len(resolved), resolved)
	}
	names := map[string]bool{}
	for _, l := range resolved {
		names[l.Name] = true
	}
	if !names["shared:lib:1.0"] || !names["linux:only:1.0"] {
		t.Errorf("unexpected resolved set: %#v", names)
	}
}

func TestNativesClassifier_ArchToken(t *testing.T) {
	lib := Library{Natives: map[string]string{"windows": "natives-windows-${arch}"}}
	c, ok := NativesClassifier(lib, Env{OSName: "windows", Arch: "x86"})
	if !ok || c != "natives-windows-32" {
		t.Errorf("got %q, %v", c, ok)
	}
	c, ok = NativesClassifier(lib, Env{OSName: "windows", Arch: "arm64"})
	if !ok || c != "natives-windows-64" {
		t.Errorf("got %q, %v", c, ok)
	}
}

func TestNativesClassifier_NoMatchForOS(t *testing.T) {
	lib := Library{Natives: map[string]string{"windows": "natives-windows"}}
	_, ok := NativesClassifier(lib, Env{OSName: "linux"})
	if ok {
		t.Error("expected no natives classifier for an OS not present in the map")
	}
}

func TestLegacyArguments_SubstitutesAndSplits(t *testing.T) {
	raw := "--username ${auth_player_name} --version ${version_name}"
	subs := map[string]string{"auth_player_name": "Steve", "version_name": "1.12.2"}
	got := LegacyArguments(raw, subs)
	want := []string{"--username", "Steve", "--version", "1.12.2"}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("got %#v, want %#v", got, want)
	}
}

func TestMavenPath(t *testing.T) {
	tests := []struct {
		coord string
		want  string
	}{
		{"net.fabricmc:fabric-loader:0.15.11", "net/fabricmc/fabric-loader/0.15.11/fabric-loader-0.15.11.jar"},
		{"org.ow2.asm:asm:9.6:shaded@jar", "org/ow2/asm/asm/9.6/asm-9.6-shaded.jar"},
	}

	for _, tc := range tests {
		got, err := MavenPath(tc.coord)
		if err != nil {
			t.Errorf("MavenPath(%q) returned error: %v", tc.coord, err)
			continue
		}
		if got != tc.want {
			t.Errorf("MavenPath(%q) = %q, want %q", tc.coord, got, tc.want)
		}
	}
}

func TestArtifactInfo_MojangAndFabric(t *testing.T) {
	mojangLib := Library{
		Name: "com.mojang:logging:1.0.0",
		Downloads: LibraryDownload{
			Artifact: &DownloadArtifact{
				URL:  "https://libraries.minecraft.net/com/mojang/logging/1.0.0/logging-1.0.0.jar",
				Path: "com/mojang/logging/1.0.0/logging-1.0.0.jar",
				SHA1: "abc",
				Size: 100,
			},
		},
	}
	info, ok := mojangLib.ArtifactInfo()
	if !ok || info.Path != "com/mojang/logging/1.0.0/logging-1.0.0.jar" || info.URL != "https://libraries.minecraft.net/com/mojang/logging/1.0.0/logging-1.0.0.jar" {
		t.Errorf("mojang style failed: %#v", info)
	}

	fabricLib := Library{
		Name: "net.fabricmc:fabric-loader:0.15.11",
		URL:  "https://maven.fabricmc.net/",
	}
	info, ok = fabricLib.ArtifactInfo()
	if !ok || info.Path != "net/fabricmc/fabric-loader/0.15.11/fabric-loader-0.15.11.jar" || info.URL != "https://maven.fabricmc.net/net/fabricmc/fabric-loader/0.15.11/fabric-loader-0.15.11.jar" {
		t.Errorf("fabric style failed: %#v", info)
	}
}
