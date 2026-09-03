package forge

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"

	"alloy/internal/download"
)

const mavenBase = "https://maven.minecraftforge.net/net/minecraftforge/forge"

func InstallerURL(mcForgeVersion string) string {
	return fmt.Sprintf("%s/%s/forge-%s-installer.jar", mavenBase, mcForgeVersion, mcForgeVersion)
}

const PromotionsURL = "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json"

type Promotions struct {
	Promos map[string]string `json:"promos"`
}

func FetchPromotions() (Promotions, error) {
	resp, err := http.Get(PromotionsURL)
	if err != nil {
		return Promotions{}, fmt.Errorf("fetching forge promotions: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return Promotions{}, fmt.Errorf("fetching forge promotions: HTTP %d", resp.StatusCode)
	}
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return Promotions{}, err
	}
	var p Promotions
	if err := json.Unmarshal(body, &p); err != nil {
		return Promotions{}, fmt.Errorf("parsing forge promotions: %w", err)
	}
	return p, nil
}

func GetLatestVersion(mcVersion string) (string, error) {
	p, err := FetchPromotions()
	if err != nil {
		return "", err
	}
	if v, ok := p.Promos[mcVersion+"-recommended"]; ok {
		return v, nil
	}
	if v, ok := p.Promos[mcVersion+"-latest"]; ok {
		return v, nil
	}
	return "", fmt.Errorf("no Forge loader version found for Minecraft %s", mcVersion)
}

func DownloadInstaller(mcForgeVersion, sha1, destPath string) error {
	_, err := download.Run([]download.Task{{
		URL:  InstallerURL(mcForgeVersion),
		Dest: destPath,
		SHA1: sha1,
	}}, download.Options{Workers: 1})
	return err
}

func RunInstallerHeadless(javaPath, installerJarPath, targetDir string) error {
	if err := os.MkdirAll(targetDir, 0o755); err != nil {
		return err
	}
	profilesPath := filepath.Join(targetDir, "launcher_profiles.json")
	stub := []byte(`{"profiles":{}}`)
	if err := os.WriteFile(profilesPath, stub, 0o644); err != nil {
		return fmt.Errorf("writing launcher_profiles.json stub: %w", err)
	}
	cmd := exec.Command(javaPath, "-jar", installerJarPath, "--installClient", targetDir)
	cmd.Dir = filepath.Dir(installerJarPath)
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	if err := cmd.Run(); err != nil {
		os.Remove(profilesPath)
		return fmt.Errorf("forge installer failed (this loader is the least automated — see internal/loader/forge doc comment): %w", err)
	}
	return nil
}
