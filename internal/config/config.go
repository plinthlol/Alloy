package config

import (
	"fmt"
	"os"
	"strconv"
	"strings"

	"github.com/pelletier/go-toml/v2"
)

type globalPaths struct {
	JavaPath string `toml:"java_path,omitempty"`
}

type Defaults struct {
	MemoryMaxMB int
	MemoryMinMB int
}

type Global struct {
	Paths    globalPaths `toml:"paths,omitempty"`
	Defaults Defaults
}

const (
	rmclDefaultMemoryMaxMB = 2048
	rmclDefaultMemoryMinMB = 512
)

func DefaultGlobal() Global {
	return Global{Defaults: Defaults{MemoryMaxMB: rmclDefaultMemoryMaxMB, MemoryMinMB: rmclDefaultMemoryMinMB}}
}

type rawDefaults struct {
	MemoryMax string `toml:"memory_max"`
	MemoryMin string `toml:"memory_min"`
}

type rawGlobal struct {
	Paths    globalPaths `toml:"paths"`
	Defaults rawDefaults `toml:"defaults"`
}

func Load(p Paths) (Global, error) {
	data, err := os.ReadFile(p.GlobalConfigFile())
	if os.IsNotExist(err) {
		return DefaultGlobal(), nil
	}
	if err != nil {
		return Global{}, err
	}
	var raw rawGlobal
	if err := toml.Unmarshal(data, &raw); err != nil {
		return Global{}, err
	}

	g := Global{Paths: raw.Paths}
	g.Defaults.MemoryMaxMB = memoryStringToMB(raw.Defaults.MemoryMax)
	g.Defaults.MemoryMinMB = memoryStringToMB(raw.Defaults.MemoryMin)
	if g.Defaults.MemoryMaxMB == 0 {
		g.Defaults.MemoryMaxMB = rmclDefaultMemoryMaxMB
	}
	if g.Defaults.MemoryMinMB == 0 {
		g.Defaults.MemoryMinMB = rmclDefaultMemoryMinMB
	}
	return g, nil
}

func Save(p Paths, g Global) error {
	if err := os.MkdirAll(p.ConfigDir, 0o755); err != nil {
		return err
	}

	content := ""
	if existing, err := os.ReadFile(p.GlobalConfigFile()); err == nil {
		content = string(existing)
	} else if !os.IsNotExist(err) {
		return err
	}

	if g.Paths.JavaPath != "" {
		content = setKeyInSection(content, "paths", "java_path", tomlQuote(g.Paths.JavaPath))
	} else {
		content = removeKeyInSection(content, "paths", "java_path")
	}

	if g.Defaults.MemoryMaxMB > 0 {
		content = setKeyInSection(content, "defaults", "memory_max", tomlQuote(memoryMBToRmclString(g.Defaults.MemoryMaxMB)))
	}
	if g.Defaults.MemoryMinMB > 0 {
		content = setKeyInSection(content, "defaults", "memory_min", tomlQuote(memoryMBToRmclString(g.Defaults.MemoryMinMB)))
	}

	return os.WriteFile(p.GlobalConfigFile(), []byte(content), 0o600)
}

func memoryStringToMB(s string) int {
	s = strings.TrimSpace(s)
	if s == "" {
		return 0
	}
	last := s[len(s)-1]
	if last >= '0' && last <= '9' {
		n, err := strconv.Atoi(s)
		if err != nil || n <= 0 {
			return 0
		}
		if n < 128 {
			return n * 1024
		}
		return n
	}
	n, err := strconv.Atoi(s[:len(s)-1])
	if err != nil || n <= 0 {
		return 0
	}
	switch last {
	case 'G', 'g':
		return n * 1024
	case 'M', 'm':
		return n
	case 'K', 'k':
		return n / 1024
	default:
		return 0
	}
}

func memoryMBToRmclString(mb int) string {
	if mb <= 0 {
		return ""
	}
	if mb%1024 == 0 {
		return fmt.Sprintf("%dG", mb/1024)
	}
	return fmt.Sprintf("%dM", mb)
}
