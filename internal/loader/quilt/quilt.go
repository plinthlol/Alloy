package quilt

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"

	"alloy/internal/version"
)

const metaBase = "https://meta.quiltmc.org/v3"

type LoaderVersion struct {
	Loader struct {
		Version string `json:"version"`
	} `json:"loader"`
}

func ListLoaderVersions(mcVersion string) ([]LoaderVersion, error) {
	url := fmt.Sprintf("%s/versions/loader/%s", metaBase, mcVersion)
	body, err := getJSON(url)
	if err != nil {
		return nil, err
	}
	var out []LoaderVersion
	if err := json.Unmarshal(body, &out); err != nil {
		return nil, fmt.Errorf("parsing quilt loader versions: %w", err)
	}
	return out, nil
}

func Latest(versions []LoaderVersion) (string, error) {
	if len(versions) == 0 {
		return "", fmt.Errorf("no quilt loader versions available for this Minecraft version")
	}
	return versions[0].Loader.Version, nil
}

func FetchProfile(mcVersion, loaderVersion string, base version.Version) (version.Version, []byte, error) {
	url := fmt.Sprintf("%s/versions/loader/%s/%s/profile/json", metaBase, mcVersion, loaderVersion)
	body, err := getJSON(url)
	if err != nil {
		return version.Version{}, nil, err
	}

	var profile version.Version
	if err := json.Unmarshal(body, &profile); err != nil {
		return version.Version{}, nil, fmt.Errorf("parsing quilt profile: %w", err)
	}

	merged := base
	merged.ID = fmt.Sprintf("%s-quilt-%s", mcVersion, loaderVersion)
	merged.MainClass = profile.MainClass
	merged.Libraries = append(append([]version.Library{}, profile.Libraries...), base.Libraries...)

	if profile.Arguments != nil {
		if merged.Arguments == nil {
			merged.Arguments = &version.Arguments{}
		}
		merged.Arguments.JVM = append(merged.Arguments.JVM, profile.Arguments.JVM...)
		merged.Arguments.Game = append(merged.Arguments.Game, profile.Arguments.Game...)
	}

	return merged, body, nil
}

func getJSON(url string) ([]byte, error) {
	resp, err := http.Get(url)
	if err != nil {
		return nil, fmt.Errorf("fetching %s: %w", url, err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("fetching %s: HTTP %d", url, resp.StatusCode)
	}
	return io.ReadAll(resp.Body)
}
