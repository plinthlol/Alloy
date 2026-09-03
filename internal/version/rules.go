package version

import (
	"regexp"
	"runtime"
	"strings"
)

type Env struct {
	OSName              string
	OSVersion           string
	Arch                string
	IsDemoUser          bool
	HasCustomResolution bool
}

func CurrentEnv() Env {
	return Env{
		OSName: mojangOSName(runtime.GOOS),
		Arch:   mojangArch(runtime.GOARCH),
	}
}

func mojangOSName(goos string) string {
	switch goos {
	case "darwin":
		return "osx"
	case "windows":
		return "windows"
	default:
		return "linux"
	}
}

func mojangArch(goarch string) string {
	switch goarch {
	case "amd64":
		return "x86"
	case "arm64":
		return "arm64"
	case "386":
		return "x86"
	default:
		return goarch
	}
}

func RuleApplies(r Rule, env Env) bool {
	if r.OS != nil {
		if r.OS.Name != "" && r.OS.Name != env.OSName {
			return false
		}
		if r.OS.Arch != "" && r.OS.Arch != env.Arch {
			return false
		}
		if r.OS.Version != "" {
			matched, err := regexp.MatchString(r.OS.Version, env.OSVersion)
			if err != nil || !matched {
				return false
			}
		}
	}
	for feature, want := range r.Features {
		var have bool
		switch feature {
		case "is_demo_user":
			have = env.IsDemoUser
		case "has_custom_resolution":
			have = env.HasCustomResolution
		default:
			have = false
		}
		if have != want {
			return false
		}
	}
	return true
}

func RulesAllow(rules []Rule, env Env) bool {
	if len(rules) == 0 {
		return true
	}
	allowed := false
	for _, r := range rules {
		if RuleApplies(r, env) {
			allowed = r.Action == "allow"
		}
	}
	return allowed
}

func ResolveArguments(entries []ArgumentEntry, env Env, subs map[string]string) []string {
	var out []string
	for _, e := range entries {
		if !RulesAllow(e.Rules, env) {
			continue
		}
		for _, v := range e.Value {
			out = append(out, substitute(v, subs))
		}
	}
	return out
}

func substitute(s string, subs map[string]string) string {
	if !strings.Contains(s, "${") {
		return s
	}
	for k, v := range subs {
		s = strings.ReplaceAll(s, "${"+k+"}", v)
	}
	return s
}

func ResolveLibraries(libs []Library, env Env) []Library {
	out := make([]Library, 0, len(libs))
	for _, l := range libs {
		if RulesAllow(l.Rules, env) {
			out = append(out, l)
		}
	}
	return out
}

func NativesClassifier(l Library, env Env) (string, bool) {
	if l.Natives == nil {
		return "", false
	}
	classifier, ok := l.Natives[env.OSName]
	if !ok {
		return "", false
	}
	arch := "64"
	if env.Arch == "x86" {
		arch = "32"
	}
	classifier = strings.ReplaceAll(classifier, "${arch}", arch)
	return classifier, true
}

func LegacyArguments(raw string, subs map[string]string) []string {
	if raw == "" {
		return nil
	}
	parts := strings.Fields(raw)
	out := make([]string, len(parts))
	for i, p := range parts {
		out[i] = substitute(p, subs)
	}
	return out
}
