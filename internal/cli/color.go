package cli

import (
	"fmt"
	"os"
)

const (
	reset  = "\x1b[0m"
	green  = "\x1b[1;32m"
	yellow = "\x1b[1;33m"
	gray   = "\x1b[2m"
	red    = "\x1b[1;31m"
	cyan   = "\x1b[1;36m"
)

var colorEnabled = os.Getenv("NO_COLOR") == ""

func paint(code, s string) string {
	if !colorEnabled {
		return s
	}
	return code + s + reset
}

func ColorForVersionType(versionType, text string) string {
	switch versionType {
	case "release":
		return paint(green, text)
	case "snapshot":
		return paint(yellow, text)
	case "old_beta", "old_alpha":
		return paint(gray, text)
	default:
		return text
	}
}

func Errorf(format string, a ...any) {
	fmt.Fprintln(os.Stderr, paint(red, "error: "+fmt.Sprintf(format, a...)))
}

func Info(s string) {
	fmt.Println(paint(cyan, s))
}
