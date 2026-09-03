package config

import (
	"regexp"
	"strings"
)

var sectionHeaderRe = regexp.MustCompile(`^\s*\[([^\[\]]+)\]\s*$`)

// findSection locates the body of a top-level "[name]" table: the line
// range [bodyStart, bodyEnd) starting after the header and ending at the
// next top-level header or EOF. headerLine is -1 if the section is absent.
func findSection(lines []string, name string) (headerLine, bodyStart, bodyEnd int) {
	headerLine = -1
	for i, line := range lines {
		m := sectionHeaderRe.FindStringSubmatch(line)
		if m == nil {
			continue
		}
		if headerLine == -1 {
			if strings.TrimSpace(m[1]) == name {
				headerLine = i
				bodyStart = i + 1
			}
			continue
		}
		bodyEnd = i
		return
	}
	if headerLine == -1 {
		return -1, -1, -1
	}
	bodyEnd = len(lines)
	return
}

func keyLineRe(key string) *regexp.Regexp {
	return regexp.MustCompile(`^\s*` + regexp.QuoteMeta(key) + `\s*=`)
}

// setKeyInSection ensures "section" exists and contains a line
// "key = value" (value must already be TOML-formatted/quoted by the
// caller). If the key already has a line, only that line is replaced —
// every other line, including comments, is left exactly as it was. If the
// section doesn't exist yet, it's appended at the end of the file.
func setKeyInSection(content, section, key, value string) string {
	lines := strings.Split(content, "\n")
	re := keyLineRe(key)
	headerLine, bodyStart, bodyEnd := findSection(lines, section)
	newLine := key + " = " + value

	if headerLine == -1 {
		if len(lines) > 0 && strings.TrimSpace(lines[len(lines)-1]) != "" {
			lines = append(lines, "")
		}
		lines = append(lines, "["+section+"]", newLine)
		return strings.Join(lines, "\n")
	}

	for i := bodyStart; i < bodyEnd; i++ {
		if re.MatchString(lines[i]) {
			lines[i] = newLine
			return strings.Join(lines, "\n")
		}
	}

	out := make([]string, 0, len(lines)+1)
	out = append(out, lines[:bodyStart]...)
	out = append(out, newLine)
	out = append(out, lines[bodyStart:]...)
	return strings.Join(out, "\n")
}

// removeKeyInSection deletes the "key = ..." line from "section", if
// present, leaving everything else untouched. No-op if the section or key
// isn't there.
func removeKeyInSection(content, section, key string) string {
	lines := strings.Split(content, "\n")
	re := keyLineRe(key)
	_, bodyStart, bodyEnd := findSection(lines, section)
	if bodyStart == -1 {
		return content
	}
	for i := bodyStart; i < bodyEnd; i++ {
		if re.MatchString(lines[i]) {
			lines = append(lines[:i], lines[i+1:]...)
			break
		}
	}
	return strings.Join(lines, "\n")
}

// tomlQuote produces a basic TOML basic-string literal for a value we're
// inserting via setKeyInSection.
func tomlQuote(s string) string {
	s = strings.ReplaceAll(s, `\`, `\\`)
	s = strings.ReplaceAll(s, `"`, `\"`)
	return `"` + s + `"`
}
