package javafind

import (
	"bufio"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"sort"
	"strconv"
	"strings"
)

type Candidate struct {
	Path   string
	Source string
}

type Verified struct {
	Candidate
	MajorVersion int
	RawVersion   string
}

func Candidates(overridePath string) []Candidate {
	var out []Candidate
	seen := map[string]bool{}

	add := func(path, source string) {
		if path == "" {
			return
		}
		abs, err := filepath.Abs(path)
		if err != nil {
			abs = path
		}
		if seen[abs] {
			return
		}
		seen[abs] = true
		out = append(out, Candidate{Path: path, Source: source})
	}

	if overridePath != "" {
		add(overridePath, "override")
	}
	if home := os.Getenv("JAVA_HOME"); home != "" {
		add(javaBinIn(home), "JAVA_HOME")
	}
	if p, err := exec.LookPath(javaExeName()); err == nil {
		add(p, "PATH")
	}
	for _, dir := range wellKnownJDKRoots() {
		for _, home := range expandJDKHomes(dir) {
			add(javaBinIn(home), "well-known:"+dir)
		}
	}
	for _, home := range macOSJavaHomes() {
		add(javaBinIn(home), "well-known:/usr/libexec/java_home")
	}
	for _, home := range platformExtraCandidates() {
		add(javaBinIn(home), "registry")
	}

	return out
}

func javaExeName() string {
	if runtime.GOOS == "windows" {
		return "java.exe"
	}
	return "java"
}

func javaBinIn(jdkHome string) string {
	return filepath.Join(jdkHome, "bin", javaExeName())
}

func wellKnownJDKRoots() []string {
	home, _ := os.UserHomeDir()
	switch runtime.GOOS {
	case "windows":
		pf := os.Getenv("ProgramFiles")
		if pf == "" {
			pf = `C:\Program Files`
		}
		return []string{
			filepath.Join(pf, "Java"),
			filepath.Join(pf, "Eclipse Adoptium"),
			filepath.Join(pf, "Zulu"),
			filepath.Join(pf, "Amazon Corretto"),
		}
	case "darwin":
		return []string{
			"/Library/Java/JavaVirtualMachines",
		}
	default:
		roots := []string{"/usr/lib/jvm", "/opt/java"}
		if home != "" {
			roots = append(roots,
				filepath.Join(home, ".sdkman/candidates/java"),
				filepath.Join(home, ".jabba/jdk"),
			)
		}
		return roots
	}
}

func expandJDKHomes(root string) []string {
	entries, err := os.ReadDir(root)
	if err != nil {
		return nil
	}
	var out []string
	for _, e := range entries {
		if !e.IsDir() {
			continue
		}
		child := filepath.Join(root, e.Name())
		if runtime.GOOS == "darwin" {
			child = filepath.Join(child, "Contents", "Home")
		}
		out = append(out, child)
	}
	return out
}

func macOSJavaHomes() []string {
	if runtime.GOOS != "darwin" {
		return nil
	}
	cmd := exec.Command("/usr/libexec/java_home", "-V")
	var stderr strings.Builder
	cmd.Stderr = &stderr
	_ = cmd.Run()

	var out []string
	scanner := bufio.NewScanner(strings.NewReader(stderr.String()))
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line == "" || strings.HasPrefix(line, "Matching Java Virtual Machines") {
			continue
		}
		fields := strings.Fields(line)
		if len(fields) == 0 {
			continue
		}
		last := fields[len(fields)-1]
		if strings.HasPrefix(last, "/") {
			out = append(out, last)
		}
	}
	return out
}

func Verify(c Candidate) (Verified, error) {
	cmd := exec.Command(c.Path, "-XshowSettings:properties", "-version")
	var out strings.Builder
	cmd.Stdout = &out
	cmd.Stderr = &out
	if err := cmd.Run(); err != nil {
		return Verified{}, fmt.Errorf("running %s: %w", c.Path, err)
	}

	specVersion := parseProperty(out.String(), "java.specification.version")
	fullVersion := parseProperty(out.String(), "java.version")
	if specVersion == "" && fullVersion == "" {
		return Verified{}, fmt.Errorf("could not parse java version output from %s", c.Path)
	}

	major, err := majorFromVersionString(firstNonEmpty(specVersion, fullVersion))
	if err != nil {
		return Verified{}, err
	}

	return Verified{Candidate: c, MajorVersion: major, RawVersion: firstNonEmpty(fullVersion, specVersion)}, nil
}

func firstNonEmpty(a, b string) string {
	if a != "" {
		return a
	}
	return b
}

func parseProperty(output, key string) string {
	scanner := bufio.NewScanner(strings.NewReader(output))
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if !strings.HasPrefix(line, key) {
			continue
		}
		parts := strings.SplitN(line, "=", 2)
		if len(parts) != 2 {
			continue
		}
		if strings.TrimSpace(parts[0]) != key {
			continue
		}
		return strings.TrimSpace(parts[1])
	}
	return ""
}

func majorFromVersionString(v string) (int, error) {
	v = strings.TrimSpace(v)
	if v == "" {
		return 0, fmt.Errorf("empty version string")
	}
	parts := strings.Split(v, ".")
	if parts[0] == "1" && len(parts) > 1 {
		n, err := strconv.Atoi(strings.SplitN(parts[1], "_", 2)[0])
		if err != nil {
			return 0, fmt.Errorf("parsing legacy version %q: %w", v, err)
		}
		return n, nil
	}
	n, err := strconv.Atoi(strings.SplitN(parts[0], "_", 2)[0])
	if err != nil {
		return 0, fmt.Errorf("parsing version %q: %w", v, err)
	}
	return n, nil
}

func Best(candidates []Verified, required int) (Verified, bool) {
	var exact *Verified
	var aboveBest *Verified

	sorted := make([]Verified, len(candidates))
	copy(sorted, candidates)
	sort.Slice(sorted, func(i, j int) bool { return sorted[i].MajorVersion < sorted[j].MajorVersion })

	for i := range sorted {
		v := sorted[i]
		switch {
		case v.MajorVersion == required:
			exact = &v
		case v.MajorVersion > required:
			if aboveBest == nil || v.MajorVersion < aboveBest.MajorVersion {
				aboveBest = &v
			}
		}
	}

	if exact != nil {
		return *exact, true
	}
	if aboveBest != nil {
		return *aboveBest, true
	}
	return Verified{}, false
}

func DescribeAvailable(candidates []Verified) string {
	if len(candidates) == 0 {
		return "no Java installations found"
	}
	majors := map[int]bool{}
	for _, c := range candidates {
		majors[c.MajorVersion] = true
	}
	list := make([]int, 0, len(majors))
	for m := range majors {
		list = append(list, m)
	}
	sort.Ints(list)
	parts := make([]string, len(list))
	for i, m := range list {
		parts[i] = fmt.Sprintf("Java %d", m)
	}
	return strings.Join(parts, ", ")
}
