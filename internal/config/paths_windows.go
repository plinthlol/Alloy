//go:build windows

package config

import (
	"os"
	"path/filepath"

	"github.com/adrg/xdg"
)

func platformConfigDataDirs() (configDir, dataDir string, err error) {
	roaming := os.Getenv("APPDATA")
	if roaming == "" {
		configFile, err := xdg.ConfigFile(appName + "/config.toml")
		if err != nil {
			return "", "", err
		}
		dataFile, err := xdg.DataFile(appName + "/.keep")
		if err != nil {
			return "", "", err
		}
		return dirOf(configFile), dirOf(dataFile), nil
	}

	dir := filepath.Join(roaming, appName)
	return dir, dir, nil
}

func platformCacheDir() (string, error) {
	local := os.Getenv("LOCALAPPDATA")
	if local == "" {
		cacheFile, err := xdg.CacheFile(appName + "/.keep")
		if err != nil {
			return "", err
		}
		return dirOf(cacheFile), nil
	}
	return filepath.Join(local, appName), nil
}
