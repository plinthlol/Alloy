package neoforge

import (
	"encoding/xml"
	"fmt"
	"io"
	"net/http"
	"strings"

	"alloy/internal/download"
	"alloy/internal/loader/forge"
)

const mavenBase = "https://maven.neoforged.net/releases/net/neoforged/neoforge"

func InstallerURL(neoForgeVersion string) string {
	return fmt.Sprintf("%s/%s/neoforge-%s-installer.jar", mavenBase, neoForgeVersion, neoForgeVersion)
}

func DownloadInstaller(neoForgeVersion, sha1, destPath string) error {
	_, err := download.Run([]download.Task{{
		URL:  InstallerURL(neoForgeVersion),
		Dest: destPath,
		SHA1: sha1,
	}}, download.Options{Workers: 1})
	return err
}

func RunInstallerHeadless(javaPath, installerJarPath, targetDir string) error {
	return forge.RunInstallerHeadless(javaPath, installerJarPath, targetDir)
}

const metadataURL = mavenBase + "/maven-metadata.xml"

type mavenMetadata struct {
	Versioning struct {
		Versions struct {
			Version []string `xml:"version"`
		} `xml:"versions"`
	} `xml:"versioning"`
}

func LatestForMC(mcVersion string) (string, error) {
	resp, err := http.Get(metadataURL)
	if err != nil {
		return "", fmt.Errorf("fetching neoforge maven metadata: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return "", fmt.Errorf("fetching neoforge maven metadata: HTTP %d", resp.StatusCode)
	}
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return "", err
	}

	var meta mavenMetadata
	if err := xml.Unmarshal(body, &meta); err != nil {
		return "", fmt.Errorf("parsing neoforge maven metadata: %w", err)
	}

	prefix := mcVersionToNeoForgePrefix(mcVersion)

	var best string
	for _, v := range meta.Versioning.Versions.Version {
		if strings.HasPrefix(v, prefix) && !strings.Contains(v, "beta") {
			best = v
		}
	}
	if best == "" {
		return "", fmt.Errorf("no stable NeoForge version found for Minecraft %s (looked for prefix %q)", mcVersion, prefix)
	}
	return best, nil
}

func mcVersionToNeoForgePrefix(mcVersion string) string {
	dot := strings.Index(mcVersion, ".")
	if dot < 0 {
		return mcVersion + "."
	}
	rest := mcVersion[dot+1:]
	return rest + "."
}
