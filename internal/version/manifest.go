package version

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"time"
)

const manifestURL = "https://launchermeta.mojang.com/mc/game/version_manifest_v2.json"

type ManifestEntry struct {
	ID          string    `json:"id"`
	Type        string    `json:"type"`
	URL         string    `json:"url"`
	ReleaseTime time.Time `json:"releaseTime"`
	SHA1        string    `json:"sha1"`
}

type Manifest struct {
	Latest struct {
		Release  string `json:"release"`
		Snapshot string `json:"snapshot"`
	} `json:"latest"`
	Versions []ManifestEntry `json:"versions"`
}

func FetchManifest(cachePath string) (Manifest, error) {
	resp, err := http.Get(manifestURL)
	if err != nil {
		return Manifest{}, fmt.Errorf("fetching version manifest: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return Manifest{}, fmt.Errorf("fetching version manifest: HTTP %d", resp.StatusCode)
	}
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return Manifest{}, err
	}

	var m Manifest
	if err := json.Unmarshal(body, &m); err != nil {
		return Manifest{}, fmt.Errorf("parsing version manifest: %w", err)
	}

	if cachePath != "" {
		_ = os.MkdirAll(dirOf(cachePath), 0o755)
		_ = os.WriteFile(cachePath, body, 0o644)
	}
	return m, nil
}

func LoadOrFetchManifest(cachePath string) (Manifest, error) {
	m, err := FetchManifest(cachePath)
	if err == nil {
		return m, nil
	}
	if cachePath != "" {
		if data, readErr := os.ReadFile(cachePath); readErr == nil {
			var cached Manifest
			if jsonErr := json.Unmarshal(data, &cached); jsonErr == nil {
				return cached, nil
			}
		}
	}
	return Manifest{}, err
}

func (m Manifest) Find(id string) (ManifestEntry, bool) {
	for _, v := range m.Versions {
		if v.ID == id {
			return v, true
		}
	}
	return ManifestEntry{}, false
}

func (m Manifest) OldestFirst() []ManifestEntry {
	out := make([]ManifestEntry, len(m.Versions))
	for i, v := range m.Versions {
		out[len(m.Versions)-1-i] = v
	}
	return out
}

func dirOf(p string) string {
	for i := len(p) - 1; i >= 0; i-- {
		if p[i] == '/' || p[i] == '\\' {
			return p[:i]
		}
	}
	return "."
}
