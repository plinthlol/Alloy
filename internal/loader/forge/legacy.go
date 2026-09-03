package forge

import (
	"archive/zip"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"

	"alloy/internal/download"
)

func HasLegacyInstallProfile(installerPath string) bool {
	r, err := zip.OpenReader(installerPath)
	if err != nil {
		return false
	}
	defer r.Close()

	for _, f := range r.File {
		if f.Name != "install_profile.json" {
			continue
		}
		rc, err := f.Open()
		if err != nil {
			return false
		}
		var v map[string]json.RawMessage
		err = json.NewDecoder(rc).Decode(&v)
		rc.Close()
		if err != nil {
			return false
		}
		_, ok := v["versionInfo"]
		return ok
	}
	return false
}

func InstallFromLegacyProfile(installerPath, librariesDir string) ([]byte, error) {
	r, err := zip.OpenReader(installerPath)
	if err != nil {
		return nil, fmt.Errorf("opening installer as zip: %w", err)
	}
	defer r.Close()

	var profileData map[string]json.RawMessage
	var universalEntry *zip.File
	for _, f := range r.File {
		if f.Name == "install_profile.json" {
			rc, err := f.Open()
			if err != nil {
				return nil, err
			}
			decodeErr := json.NewDecoder(rc).Decode(&profileData)
			rc.Close()
			if decodeErr != nil {
				return nil, fmt.Errorf("parsing install_profile.json: %w", decodeErr)
			}
		}
	}
	if profileData == nil {
		return nil, fmt.Errorf("install_profile.json not found in installer")
	}

	versionInfoRaw, ok := profileData["versionInfo"]
	if !ok {
		return nil, fmt.Errorf("install_profile.json missing versionInfo")
	}

	installRaw, ok := profileData["install"]
	if !ok {
		return nil, fmt.Errorf("install_profile.json missing install section")
	}
	var install struct {
		FilePath string `json:"filePath"`
		Path     string `json:"path"`
	}
	if err := json.Unmarshal(installRaw, &install); err != nil {
		return nil, fmt.Errorf("parsing install section: %w", err)
	}
	if install.FilePath == "" || install.Path == "" {
		return nil, fmt.Errorf("install_profile.json missing install.filePath/install.path")
	}

	var versionInfo struct {
		Libraries []struct {
			Name string `json:"name"`
			URL  string `json:"url"`
		} `json:"libraries"`
	}
	if err := json.Unmarshal(versionInfoRaw, &versionInfo); err != nil {
		return nil, fmt.Errorf("parsing versionInfo: %w", err)
	}
	if versionInfo.Libraries == nil {
		return nil, fmt.Errorf("missing versionInfo.libraries")
	}

	universalMavenPath, ok := mavenCoordToPath(install.Path)
	if !ok {
		return nil, fmt.Errorf("invalid maven coord in install.path: %s", install.Path)
	}

	for _, f := range r.File {
		if f.Name == install.FilePath {
			universalEntry = f
			break
		}
	}
	if universalEntry == nil {
		return nil, fmt.Errorf("universal JAR %q not found in installer", install.FilePath)
	}

	universalDest := filepath.Join(librariesDir, filepath.FromSlash(universalMavenPath))
	if err := os.MkdirAll(filepath.Dir(universalDest), 0o755); err != nil {
		return nil, err
	}
	rc, err := universalEntry.Open()
	if err != nil {
		return nil, err
	}
	data, err := io.ReadAll(rc)
	rc.Close()
	if err != nil {
		return nil, err
	}
	if err := os.WriteFile(universalDest, data, 0o644); err != nil {
		return nil, err
	}

	var tasks []download.Task
	for _, lib := range versionInfo.Libraries {
		mavenPath, ok := mavenCoordToPath(lib.Name)
		if !ok {
			return nil, fmt.Errorf("invalid maven coordinate: %s", lib.Name)
		}
		dest := filepath.Join(librariesDir, filepath.FromSlash(mavenPath))
		if _, err := os.Stat(dest); err == nil {
			continue
		}
		baseURL := strings.TrimRight(lib.URL, "/")
		if baseURL == "" {
			baseURL = "https://libraries.minecraft.net"
		}
		tasks = append(tasks, download.Task{URL: baseURL + "/" + mavenPath, Dest: dest})
	}
	if len(tasks) > 0 {
		if _, err := download.Run(tasks, download.Options{Workers: 8}); err != nil {
			return nil, fmt.Errorf("downloading legacy forge libraries: %w", err)
		}
	}

	return []byte(versionInfoRaw), nil
}

func mavenCoordToPath(coord string) (string, bool) {
	parts := strings.Split(coord, ":")
	groupPath := func(g string) string { return strings.ReplaceAll(g, ".", "/") }
	switch len(parts) {
	case 3:
		group, artifact, ver := parts[0], parts[1], parts[2]
		return fmt.Sprintf("%s/%s/%s/%s-%s.jar", groupPath(group), artifact, ver, artifact, ver), true
	case 4:
		group, artifact, ver, classifier := parts[0], parts[1], parts[2], parts[3]
		return fmt.Sprintf("%s/%s/%s/%s-%s-%s.jar", groupPath(group), artifact, ver, artifact, ver, classifier), true
	default:
		return "", false
	}
}
