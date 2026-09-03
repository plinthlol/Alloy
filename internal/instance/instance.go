package instance

import (
	"encoding/json"
	"fmt"
	"os"
	"strconv"
	"strings"
	"time"

	"alloy/internal/config"
)

type Meta struct {
	Name          string
	MCVersion     string
	Loader        string
	LoaderVersion string
	JavaPath      string
	MemoryMinMB   int
	MemoryMaxMB   int
	JVMArgs       []string
	Created       time.Time

	extra map[string]json.RawMessage
}

func (m Meta) CacheKey() string {
	if m.Loader == "" {
		return m.MCVersion + "-vanilla"
	}
	return fmt.Sprintf("%s-%s-%s", m.MCVersion, m.Loader, m.LoaderVersion)
}

func DefaultName(mcVersion, loader string) string {
	if loader == "" {
		return strings.ToLower(mcVersion)
	}
	return strings.ToLower(mcVersion + "-" + loader)
}

func loaderToRmcl(l string) string {
	switch l {
	case "":
		return "vanilla"
	case "neoforge":
		return "neo_forge"
	default:
		return l
	}
}

func loaderFromRmcl(l string) string {
	switch l {
	case "vanilla":
		return ""
	case "neo_forge":
		return "neoforge"
	default:
		return l
	}
}

func parseMemoryMB(raw json.RawMessage) int {
	var s string
	if err := json.Unmarshal(raw, &s); err == nil {
		return memoryStringToMB(s)
	}
	var n float64
	if err := json.Unmarshal(raw, &n); err == nil {
		if n <= 0 {
			return 0
		}
		if n < 128 {
			return int(n) * 1024
		}
		return int(n)
	}
	return 0
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

func instanceJSONPath(p config.Paths, name string) string {
	return p.InstanceDir(name) + "/instance.json"
}

func Exists(p config.Paths, name string) bool {
	_, err := os.Stat(instanceJSONPath(p, name))
	return err == nil
}

func Load(p config.Paths, name string) (Meta, error) {
	data, err := os.ReadFile(instanceJSONPath(p, name))
	if err != nil {
		return Meta{}, err
	}
	return parseMeta(data)
}

func parseMeta(data []byte) (Meta, error) {
	var raw map[string]json.RawMessage
	if err := json.Unmarshal(data, &raw); err != nil {
		return Meta{}, err
	}

	m := Meta{extra: raw}

	if v, ok := raw["name"]; ok {
		json.Unmarshal(v, &m.Name)
	}
	if v, ok := raw["game_version"]; ok {
		json.Unmarshal(v, &m.MCVersion)
	}
	if v, ok := raw["loader"]; ok {
		var loaderStr string
		json.Unmarshal(v, &loaderStr)
		m.Loader = loaderFromRmcl(loaderStr)
	}
	if v, ok := raw["loader_version"]; ok {
		var s *string
		json.Unmarshal(v, &s)
		if s != nil {
			m.LoaderVersion = *s
		}
	}
	if v, ok := raw["java_path"]; ok {
		var s *string
		json.Unmarshal(v, &s)
		if s != nil {
			m.JavaPath = *s
		}
	}
	if v, ok := raw["memory_max"]; ok {
		m.MemoryMaxMB = parseMemoryMB(v)
	}
	if v, ok := raw["memory_min"]; ok {
		m.MemoryMinMB = parseMemoryMB(v)
	}
	if v, ok := raw["jvm_args"]; ok {
		json.Unmarshal(v, &m.JVMArgs)
	}
	if v, ok := raw["created"]; ok {
		json.Unmarshal(v, &m.Created)
	}

	for _, k := range []string{
		"name", "game_version", "loader", "loader_version",
		"java_path", "memory_max", "memory_min", "jvm_args", "created",
	} {
		delete(m.extra, k)
	}

	return m, nil
}

func Save(p config.Paths, m Meta) error {
	dir := p.InstanceDir(m.Name)
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return err
	}

	out := map[string]any{}
	for k, v := range m.extra {
		var decoded any
		if err := json.Unmarshal(v, &decoded); err == nil {
			out[k] = decoded
		}
	}

	out["name"] = m.Name
	out["game_version"] = m.MCVersion
	out["loader"] = loaderToRmcl(m.Loader)
	if m.LoaderVersion != "" {
		out["loader_version"] = m.LoaderVersion
	} else {
		out["loader_version"] = nil
	}
	if m.JavaPath != "" {
		out["java_path"] = m.JavaPath
	}
	if m.MemoryMaxMB > 0 {
		out["memory_max"] = memoryMBToRmclString(m.MemoryMaxMB)
	}
	if m.MemoryMinMB > 0 {
		out["memory_min"] = memoryMBToRmclString(m.MemoryMinMB)
	}
	if len(m.JVMArgs) > 0 {
		out["jvm_args"] = m.JVMArgs
	}
	created := m.Created
	if created.IsZero() {
		created = time.Now().UTC()
	}
	out["created"] = created.UTC().Format("2006-01-02T15:04:05.999999999Z07:00")

	data, err := json.MarshalIndent(out, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(dir+"/instance.json", data, 0o644)
}

func EnsureDataDir(p config.Paths, name string) error {
	base := p.InstanceMinecraftDir(name)
	for _, sub := range []string{"mods", "saves", "resourcepacks", "shaderpacks", "config", "logs"} {
		if err := os.MkdirAll(base+"/"+sub, 0o755); err != nil {
			return err
		}
	}
	profilesPath := base + "/launcher_profiles.json"
	if _, err := os.Stat(profilesPath); os.IsNotExist(err) {
		if err := os.WriteFile(profilesPath, []byte("{}"), 0o644); err != nil {
			return err
		}
	}
	return nil
}

func List(p config.Paths) ([]string, error) {
	entries, err := os.ReadDir(p.InstancesDir())
	if os.IsNotExist(err) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	var names []string
	for _, e := range entries {
		if !e.IsDir() {
			continue
		}
		if _, err := os.Stat(p.InstanceDir(e.Name()) + "/instance.json"); err == nil {
			names = append(names, e.Name())
		}
	}
	return names, nil
}

func Rename(p config.Paths, oldName, newName string) error {
	if Exists(p, newName) {
		return fmt.Errorf("instance %q already exists", newName)
	}
	if !Exists(p, oldName) {
		return fmt.Errorf("instance %q does not exist", oldName)
	}

	m, err := Load(p, oldName)
	if err != nil {
		return err
	}
	m.Name = newName

	if err := os.Rename(p.InstanceDir(oldName), p.InstanceDir(newName)); err != nil {
		return err
	}

	return Save(p, m)
}

func Remove(p config.Paths, name string) error {
	return os.RemoveAll(p.InstanceDir(name))
}
