package version

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"strings"
)

type Version struct {
	ID            string          `json:"id"`
	Type          string          `json:"type"`
	MainClass     string          `json:"mainClass"`
	InheritsFrom  string          `json:"inheritsFrom,omitempty"`
	AssetIndex    AssetIndexRef   `json:"assetIndex"`
	Assets        string          `json:"assets"`
	Downloads     Downloads       `json:"downloads"`
	Libraries     []Library       `json:"libraries"`
	JavaVersion   JavaVersionSpec `json:"javaVersion"`
	Arguments     *Arguments      `json:"arguments,omitempty"`
	MinecraftArgs string          `json:"minecraftArguments,omitempty"`
}

type JavaVersionSpec struct {
	Component    string `json:"component"`
	MajorVersion int    `json:"majorVersion"`
}

type AssetIndexRef struct {
	ID        string `json:"id"`
	URL       string `json:"url"`
	SHA1      string `json:"sha1"`
	Size      int64  `json:"size"`
	TotalSize int64  `json:"totalSize"`
}

type Downloads struct {
	Client DownloadArtifact `json:"client"`
	Server DownloadArtifact `json:"server,omitempty"`
}

type DownloadArtifact struct {
	URL  string `json:"url"`
	SHA1 string `json:"sha1"`
	Size int64  `json:"size"`
	Path string `json:"path,omitempty"`
}

type Library struct {
	Name      string          `json:"name"`
	URL       string          `json:"url,omitempty"`
	Downloads LibraryDownload `json:"downloads"`
	Rules     []Rule          `json:"rules,omitempty"`

	Natives map[string]string `json:"natives,omitempty"`
	Extract *struct {
		Exclude []string `json:"exclude,omitempty"`
	} `json:"extract,omitempty"`
}

type LibraryDownload struct {
	Artifact    *DownloadArtifact           `json:"artifact,omitempty"`
	Classifiers map[string]DownloadArtifact `json:"classifiers,omitempty"`
}

type LibraryArtifactInfo struct {
	URL  string
	Path string
	SHA1 string
	Size int64
}

func (l Library) ArtifactInfo() (LibraryArtifactInfo, bool) {
	if l.Downloads.Artifact != nil && l.Downloads.Artifact.Path != "" {
		return LibraryArtifactInfo{
			URL:  l.Downloads.Artifact.URL,
			Path: l.Downloads.Artifact.Path,
			SHA1: l.Downloads.Artifact.SHA1,
			Size: l.Downloads.Artifact.Size,
		}, true
	}

	if l.Name == "" {
		return LibraryArtifactInfo{}, false
	}

	relPath, err := MavenPath(l.Name)
	if err != nil {
		return LibraryArtifactInfo{}, false
	}

	baseURL := l.URL
	if baseURL == "" {
		if strings.HasPrefix(l.Name, "net.fabricmc") {
			baseURL = "https://maven.fabricmc.net/"
		} else if strings.HasPrefix(l.Name, "org.quiltmc") {
			baseURL = "https://maven.quiltmc.org/repository/release/"
		} else {
			baseURL = "https://repo1.maven.org/maven2/"
		}
	}
	if !strings.HasSuffix(baseURL, "/") {
		baseURL += "/"
	}

	return LibraryArtifactInfo{
		URL:  baseURL + relPath,
		Path: relPath,
		SHA1: "",
		Size: 0,
	}, true
}

func MavenPath(name string) (string, error) {
	if name == "" {
		return "", fmt.Errorf("empty maven coordinate")
	}

	ext := "jar"
	if idx := strings.Index(name, "@"); idx != -1 {
		ext = name[idx+1:]
		name = name[:idx]
	}

	parts := strings.Split(name, ":")
	if len(parts) < 3 {
		return "", fmt.Errorf("invalid maven coordinate %q", name)
	}

	group := strings.ReplaceAll(parts[0], ".", "/")
	artifact := parts[1]
	version := parts[2]
	classifier := ""
	if len(parts) >= 4 {
		classifier = parts[3]
	}

	filename := fmt.Sprintf("%s-%s", artifact, version)
	if classifier != "" {
		filename += "-" + classifier
	}
	filename += "." + ext

	return fmt.Sprintf("%s/%s/%s/%s", group, artifact, version, filename), nil
}

type Arguments struct {
	Game []ArgumentEntry `json:"game,omitempty"`
	JVM  []ArgumentEntry `json:"jvm,omitempty"`
}

type ArgumentEntry struct {
	Rules []Rule
	Value []string
}

func (a *ArgumentEntry) UnmarshalJSON(data []byte) error {
	var s string
	if err := json.Unmarshal(data, &s); err == nil {
		a.Value = []string{s}
		a.Rules = nil
		return nil
	}

	var obj struct {
		Rules []Rule          `json:"rules"`
		Value json.RawMessage `json:"value"`
	}
	if err := json.Unmarshal(data, &obj); err != nil {
		return fmt.Errorf("argument entry is neither a string nor a rule object: %w", err)
	}
	a.Rules = obj.Rules

	var single string
	if err := json.Unmarshal(obj.Value, &single); err == nil {
		a.Value = []string{single}
		return nil
	}
	var multi []string
	if err := json.Unmarshal(obj.Value, &multi); err == nil {
		a.Value = multi
		return nil
	}
	return fmt.Errorf("argument entry value is neither string nor []string")
}

type Rule struct {
	Action   string          `json:"action"`
	OS       *OSRule         `json:"os,omitempty"`
	Features map[string]bool `json:"features,omitempty"`
}

type OSRule struct {
	Name    string `json:"name,omitempty"`
	Version string `json:"version,omitempty"`
	Arch    string `json:"arch,omitempty"`
}

func FetchVersionJSON(url string) (Version, []byte, error) {
	resp, err := http.Get(url)
	if err != nil {
		return Version{}, nil, fmt.Errorf("fetching version json: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return Version{}, nil, fmt.Errorf("fetching version json: HTTP %d", resp.StatusCode)
	}
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return Version{}, nil, err
	}
	var v Version
	if err := json.Unmarshal(body, &v); err != nil {
		return Version{}, nil, fmt.Errorf("parsing version json: %w", err)
	}
	return v, body, nil
}

func LoadVersionJSON(path string) (Version, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return Version{}, err
	}
	var v Version
	if err := json.Unmarshal(data, &v); err != nil {
		return Version{}, err
	}
	return v, nil
}
