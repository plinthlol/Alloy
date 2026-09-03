package config

import (
	"os"
	"path/filepath"
	"strings"

	"github.com/pelletier/go-toml/v2"
)

type Paths struct {
	ConfigDir string
	DataDir   string
	CacheDir  string

	InstancesDirOverride string
	MetaDirOverride      string
}

const appName = "alloy"

func Resolve() (Paths, error) {
	configDir, dataDir, err := platformConfigDataDirs()
	if err != nil {
		return Paths{}, err
	}
	cacheDir, err := platformCacheDir()
	if err != nil {
		return Paths{}, err
	}

	p := Paths{
		ConfigDir: configDir,
		DataDir:   dataDir,
		CacheDir:  cacheDir,
	}

	p.InstancesDirOverride, p.MetaDirOverride = readPathOverrides(p.GlobalConfigFile())

	return p, nil
}

func readPathOverrides(configPath string) (instancesDir, metaDir string) {
	data, err := os.ReadFile(configPath)
	if err != nil {
		return "", ""
	}
	var doc struct {
		Paths struct {
			InstancesDir string `toml:"instances_dir"`
			MetaDir      string `toml:"meta_dir"`
		} `toml:"paths"`
	}
	if err := toml.Unmarshal(data, &doc); err != nil {
		return "", ""
	}
	return expandTilde(doc.Paths.InstancesDir), expandTilde(doc.Paths.MetaDir)
}

func expandTilde(raw string) string {
	if raw == "" {
		return ""
	}
	if raw == "~" {
		if home, err := os.UserHomeDir(); err == nil {
			return home
		}
		return raw
	}
	if strings.HasPrefix(raw, "~/") {
		if home, err := os.UserHomeDir(); err == nil {
			return filepath.Join(home, raw[2:])
		}
	}
	return raw
}

func dirOf(fullPath string) string {
	for i := len(fullPath) - 1; i >= 0; i-- {
		if fullPath[i] == '/' || fullPath[i] == '\\' {
			return fullPath[:i]
		}
	}
	return fullPath
}

func (p Paths) InstancesDir() string {
	if p.InstancesDirOverride != "" {
		return p.InstancesDirOverride
	}
	return p.DataDir + "/instances"
}

func (p Paths) InstanceDir(name string) string {
	return p.InstancesDir() + "/" + name
}

func (p Paths) InstanceMinecraftDir(name string) string {
	return p.InstanceDir(name) + "/.minecraft"
}

func (p Paths) MetaDir() string {
	if p.MetaDirOverride != "" {
		return p.MetaDirOverride
	}
	return p.DataDir + "/meta"
}

func (p Paths) VersionMetaDir(mcVersion string) string {
	return p.MetaDir() + "/versions/" + mcVersion
}

func (p Paths) ClientJarPath(mcVersion string) string {
	return p.VersionMetaDir(mcVersion) + "/" + mcVersion + ".jar"
}

func (p Paths) VersionMetaFile(mcVersion string) string {
	return p.VersionMetaDir(mcVersion) + "/meta.json"
}

func (p Paths) NativesDir(mcVersion string) string {
	return p.VersionMetaDir(mcVersion) + "/natives"
}

func (p Paths) LibrariesDir() string {
	return p.MetaDir() + "/libraries"
}

func (p Paths) AssetsDir() string {
	return p.MetaDir() + "/assets"
}

func (p Paths) LoaderProfilesDir() string {
	return p.MetaDir() + "/loader-profiles"
}

func (p Paths) ResolvedCacheDir(key string) string {
	return p.CacheDir + "/resolved/" + key
}

func (p Paths) ManifestCacheFile() string {
	return p.CacheDir + "/version_manifest_v2.json"
}

func (p Paths) GlobalConfigFile() string {
	return p.ConfigDir + "/config.toml"
}
