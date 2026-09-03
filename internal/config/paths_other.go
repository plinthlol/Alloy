//go:build !windows

package config

import "github.com/adrg/xdg"

func platformConfigDataDirs() (configDir, dataDir string, err error) {
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

func platformCacheDir() (string, error) {
	cacheFile, err := xdg.CacheFile(appName + "/.keep")
	if err != nil {
		return "", err
	}
	return dirOf(cacheFile), nil
}
