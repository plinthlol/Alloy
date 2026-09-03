package main

import (
	"archive/zip"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"time"

	"alloy/internal/cli"
	"alloy/internal/config"
	"alloy/internal/download"
	"alloy/internal/instance"
	"alloy/internal/javafind"
	"alloy/internal/loader/fabric"
	"alloy/internal/loader/forge"
	"alloy/internal/loader/neoforge"
	"alloy/internal/loader/quilt"
	"alloy/internal/version"
)

func cmdInstall(paths config.Paths, args []string) error {
	var fabricFlag, quiltFlag, forgeFlag, neoforgeFlag bool
	var nameFlag string
	var positional []string

	for i := 0; i < len(args); i++ {
		switch args[i] {
		case "--fabric":
			fabricFlag = true
		case "--quilt":
			quiltFlag = true
		case "--forge":
			forgeFlag = true
		case "--neoforge":
			neoforgeFlag = true
		case "--name":
			if i+1 < len(args) {
				i++
				nameFlag = args[i]
			}
		default:
			positional = append(positional, args[i])
		}
	}

	loaderCount := boolCount(fabricFlag, quiltFlag, forgeFlag, neoforgeFlag)
	if loaderCount > 1 {
		return fmt.Errorf("only one of --fabric/--quilt/--forge/--neoforge may be given")
	}
	loaderName := ""
	switch {
	case fabricFlag:
		loaderName = "fabric"
	case quiltFlag:
		loaderName = "quilt"
	case forgeFlag:
		loaderName = "forge"
	case neoforgeFlag:
		loaderName = "neoforge"
	}

	manifest, err := version.LoadOrFetchManifest(paths.ManifestCacheFile())
	if err != nil {
		return fmt.Errorf("loading version manifest: %w", err)
	}

	var mcVersion string
	if len(positional) == 0 {
		mcVersion, err = browseAndPickVersion(manifest)
		if err != nil {
			return err
		}
		if loaderCount == 0 {
			loaderName, err = browseAndPickLoader()
			if err != nil {
				return err
			}
		}
	} else {
		mcVersion = positional[0]
	}

	entry, ok := manifest.Find(mcVersion)
	if !ok {
		return fmt.Errorf("no such Minecraft version %q (run `alloyctl install` with no args to browse)", mcVersion)
	}

	instName := nameFlag
	if instName == "" {
		instName = instance.DefaultName(mcVersion, loaderName)
	}
	if instance.Exists(paths, instName) {
		return fmt.Errorf(
			"instance %q already exists. Rename it first with `alloyctl rename %s <new-name>`, or install this one under a different name with --name",
			instName, instName,
		)
	}

	cli.Info(fmt.Sprintf("Installing %s%s...", mcVersion, loaderSuffix(loaderName)))

	base, baseRaw, err := version.FetchVersionJSON(entry.URL)
	if err != nil {
		return fmt.Errorf("fetching version json: %w", err)
	}

	if err := os.MkdirAll(paths.VersionMetaDir(mcVersion), 0o755); err != nil {
		return fmt.Errorf("creating version meta dir: %w", err)
	}
	if err := os.WriteFile(paths.VersionMetaFile(mcVersion), baseRaw, 0o644); err != nil {
		return fmt.Errorf("saving version meta: %w", err)
	}

	resolved := base
	loaderVersion := ""
	var rawProfile []byte
	var profileFilename string

	switch loaderName {
	case "fabric":
		versions, err := fabric.ListLoaderVersions(mcVersion)
		if err != nil {
			return err
		}
		loaderVersion, err = fabric.LatestStable(versions)
		if err != nil {
			return err
		}
		resolved, rawProfile, err = fabric.FetchProfile(mcVersion, loaderVersion, base)
		if err != nil {
			return err
		}
		profileFilename = fmt.Sprintf("fabric-%s-%s.json", mcVersion, loaderVersion)
	case "quilt":
		versions, err := quilt.ListLoaderVersions(mcVersion)
		if err != nil {
			return err
		}
		loaderVersion, err = quilt.Latest(versions)
		if err != nil {
			return err
		}
		resolved, rawProfile, err = quilt.FetchProfile(mcVersion, loaderVersion, base)
		if err != nil {
			return err
		}
		profileFilename = fmt.Sprintf("quilt-%s-%s.json", mcVersion, loaderVersion)
	case "forge":
		forgeVer, err := forge.GetLatestVersion(mcVersion)
		if err != nil {
			return err
		}
		loaderVersion = forgeVer
	case "neoforge":
		neoVer, err := neoforge.LatestForMC(mcVersion)
		if err != nil {
			return err
		}
		loaderVersion = neoVer
	}

	if len(rawProfile) > 0 {
		if err := saveLoaderProfile(paths, profileFilename, rawProfile); err != nil {
			return fmt.Errorf("caching loader profile: %w", err)
		}
	}

	cacheKey := cacheKeyFor(mcVersion, loaderName, loaderVersion)
	resolvedDir := paths.ResolvedCacheDir(cacheKey)

	if err := resumeOrFreshInstall(resolvedDir, paths.LibrariesDir(), resolved); err != nil {
		return err
	}

	if err := instance.EnsureDataDir(paths, instName); err != nil {
		return err
	}

	switch loaderName {
	case "forge":
		mcForgeVersion := mcVersion + "-" + loaderVersion
		cli.Info(fmt.Sprintf("Downloading Forge installer (%s)...", mcForgeVersion))
		installerPath := filepath.Join(os.TempDir(), fmt.Sprintf("forge-%s-installer.jar", mcForgeVersion))
		if err := forge.DownloadInstaller(mcForgeVersion, "", installerPath); err != nil {
			return fmt.Errorf("downloading forge installer: %w", err)
		}
		defer os.Remove(installerPath)

		profileFilename = fmt.Sprintf("forge-%s-%s.json", mcVersion, loaderVersion)
		var raw []byte

		if forge.HasLegacyInstallProfile(installerPath) {
			cli.Info("Installing legacy Forge from profile...")
			raw, err = forge.InstallFromLegacyProfile(installerPath, paths.LibrariesDir())
			if err != nil {
				return fmt.Errorf("installing legacy forge: %w", err)
			}
		} else {
			g, _ := config.Load(paths)
			candidates := javafind.Candidates(g.Paths.JavaPath)
			var verified []javafind.Verified
			for _, c := range candidates {
				if v, err := javafind.Verify(c); err == nil {
					verified = append(verified, v)
				}
			}
			bestJava, ok := javafind.Best(verified, 8)
			if !ok {
				return fmt.Errorf("Forge installer requires Java 8+, but no Java runtime was found")
			}

			minecraftDir := paths.InstanceMinecraftDir(instName)
			cli.Info("Running Forge installer in headless mode...")
			if err := forge.RunInstallerHeadless(bestJava.Path, installerPath, minecraftDir); err != nil {
				return err
			}

			versionDirName := fmt.Sprintf("%s-forge-%s", mcVersion, loaderVersion)
			raw, err = extractInstallerProfile(minecraftDir, versionDirName)
			if err != nil {
				return fmt.Errorf("extracting forge profile: %w", err)
			}
		}

		if err := saveLoaderProfile(paths, profileFilename, raw); err != nil {
			return fmt.Errorf("caching forge profile: %w", err)
		}
		resolved, err = mergeLoaderVersionJSON(base, raw, mcVersion+"-forge-"+loaderVersion)
		if err != nil {
			return err
		}
	case "neoforge":
		cli.Info(fmt.Sprintf("Downloading NeoForge installer (%s)...", loaderVersion))
		installerPath := filepath.Join(os.TempDir(), fmt.Sprintf("neoforge-%s-installer.jar", loaderVersion))
		if err := neoforge.DownloadInstaller(loaderVersion, "", installerPath); err != nil {
			return fmt.Errorf("downloading neoforge installer: %w", err)
		}
		defer os.Remove(installerPath)

		g, _ := config.Load(paths)
		candidates := javafind.Candidates(g.Paths.JavaPath)
		var verified []javafind.Verified
		for _, c := range candidates {
			if v, err := javafind.Verify(c); err == nil {
				verified = append(verified, v)
			}
		}
		bestJava, ok := javafind.Best(verified, 17)
		if !ok {
			return fmt.Errorf("NeoForge installer requires Java 17+, but no Java runtime was found")
		}

		minecraftDir := paths.InstanceMinecraftDir(instName)
		cli.Info(fmt.Sprintf("Running NeoForge installer in headless mode..."))
		if err := neoforge.RunInstallerHeadless(bestJava.Path, installerPath, minecraftDir); err != nil {
			return err
		}

		versionDirName := fmt.Sprintf("neoforge-%s", loaderVersion)
		profileFilename = fmt.Sprintf("neoforge-%s.json", loaderVersion)
		raw, err := extractInstallerProfile(minecraftDir, versionDirName)
		if err != nil {
			return fmt.Errorf("extracting neoforge profile: %w", err)
		}
		if err := saveLoaderProfile(paths, profileFilename, raw); err != nil {
			return fmt.Errorf("caching neoforge profile: %w", err)
		}
		resolved, err = mergeLoaderVersionJSON(base, raw, mcVersion+"-neoforge-"+loaderVersion)
		if err != nil {
			return err
		}
	}

	phase1 := buildDownloadTasks(
		resolved,
		paths.ClientJarPath(mcVersion),
		paths.LibrariesDir(),
		paths.AssetsDir(),
	)
	cli.Info(fmt.Sprintf("Downloading %d files (client jar + libraries)...", len(phase1)))
	if err := runDownloads(phase1); err != nil {
		return fmt.Errorf("download failed: %w", err)
	}

	if err := extractNatives(resolved, paths.NativesDir(mcVersion)); err != nil {
		return fmt.Errorf("extracting native libraries: %w", err)
	}

	if resolved.AssetIndex.URL != "" {
		indexPath := filepath.Join(paths.AssetsDir(), "indexes", resolved.AssetIndex.ID+".json")
		assetTasks, err := assetObjectTasks(indexPath, paths.AssetsDir())
		if err != nil {
			return fmt.Errorf("expanding asset index: %w", err)
		}
		cli.Info(fmt.Sprintf("Downloading %d asset objects...", len(assetTasks)))
		if err := runDownloads(assetTasks); err != nil {
			return fmt.Errorf("asset download failed: %w", err)
		}
	}

	if err := saveResolvedVersionJSON(resolved, resolvedDir); err != nil {
		return fmt.Errorf("caching resolved version definition: %w", err)
	}

	meta := instance.Meta{
		Name:          instName,
		MCVersion:     mcVersion,
		Loader:        loaderName,
		LoaderVersion: loaderVersion,
		Created:       time.Now().UTC(),
	}
	if err := instance.Save(paths, meta); err != nil {
		return err
	}

	cli.Info(fmt.Sprintf("Installed %s. Run `alloyctl play %s` to launch.", instName, instName))
	return nil
}

func boolCount(bs ...bool) int {
	n := 0
	for _, b := range bs {
		if b {
			n++
		}
	}
	return n
}

func loaderSuffix(loader string) string {
	if loader == "" {
		return " (vanilla)"
	}
	return " + " + loader
}

func cacheKeyFor(mcVersion, loader, loaderVersion string) string {
	if loader == "" {
		return mcVersion + "-vanilla"
	}
	return mcVersion + "-" + loader + "-" + loaderVersion
}

func browseAndPickVersion(m version.Manifest) (string, error) {
	ordered := m.OldestFirst()
	labels := make([]string, len(ordered))
	for i, v := range ordered {
		label := fmt.Sprintf("%s [%s]", v.ID, v.Type)
		labels[i] = cli.ColorForVersionType(v.Type, label)
	}
	idx, err := cli.PromptSelectIndex("Which version would you like to install?", labels)
	if err != nil {
		return "", err
	}
	return ordered[idx].ID, nil
}

func browseAndPickLoader() (string, error) {
	options := []string{"Vanilla", "Fabric", "Quilt", "Forge", "NeoForge"}
	idx, err := cli.PromptSelectIndex("Which mod loader?", options)
	if err != nil {
		return "", err
	}
	loaders := []string{"", "fabric", "quilt", "forge", "neoforge"}
	return loaders[idx], nil
}

func resumeOrFreshInstall(resolvedDir, librariesDir string, resolved version.Version) error {
	entries, err := os.ReadDir(resolvedDir)
	if err != nil {
		if os.IsNotExist(err) {
			return nil
		}
		return err
	}
	if len(entries) == 0 {
		return nil
	}

	cli.Info(fmt.Sprintf("Found files from a previous, incomplete install at %s.", resolvedDir))
	idx, err := cli.PromptSelectIndex("What would you like to do?", []string{
		"Resume (keep what's already downloaded, re-fetch the rest)",
		"Fresh install (delete it and start over, including cached libraries)",
	})
	if err != nil {
		return err
	}
	if idx == 1 {
		if err := os.RemoveAll(resolvedDir); err != nil {
			return fmt.Errorf("removing previous incomplete download: %w", err)
		}
		for _, libPath := range libraryPathsFor(resolved, librariesDir) {
			if err := os.Remove(libPath); err != nil && !os.IsNotExist(err) {
				return fmt.Errorf("removing cached library %s: %w", libPath, err)
			}
		}
	}
	return nil
}

func libraryPathsFor(v version.Version, librariesDir string) []string {
	var paths []string
	env := version.CurrentEnv()
	for _, lib := range version.ResolveLibraries(v.Libraries, env) {
		if art, ok := lib.ArtifactInfo(); ok {
			paths = append(paths, filepath.Join(librariesDir, filepath.FromSlash(art.Path)))
		}
	}
	return paths
}

func saveLoaderProfile(p config.Paths, filename string, raw []byte) error {
	dir := p.LoaderProfilesDir()
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return err
	}
	return os.WriteFile(filepath.Join(dir, filename), raw, 0o644)
}

func extractInstallerProfile(minecraftDir, versionDirName string) ([]byte, error) {
	p := filepath.Join(minecraftDir, "versions", versionDirName, versionDirName+".json")
	raw, err := os.ReadFile(p)
	if err != nil {
		return nil, fmt.Errorf("reading %s: %w", p, err)
	}
	return raw, nil
}

func mergeLoaderVersionJSON(base version.Version, raw []byte, newID string) (version.Version, error) {
	var profile version.Version
	if err := json.Unmarshal(raw, &profile); err != nil {
		return version.Version{}, fmt.Errorf("parsing loader version json: %w", err)
	}
	merged := base
	merged.ID = newID
	if profile.MainClass != "" {
		merged.MainClass = profile.MainClass
	}
	merged.Libraries = append(append([]version.Library{}, profile.Libraries...), base.Libraries...)
	if profile.Arguments != nil {
		if merged.Arguments == nil {
			merged.Arguments = &version.Arguments{}
		}
		merged.Arguments.JVM = append(merged.Arguments.JVM, profile.Arguments.JVM...)
		merged.Arguments.Game = append(merged.Arguments.Game, profile.Arguments.Game...)
	}
	return merged, nil
}

func buildDownloadTasks(v version.Version, clientJarDest, librariesDir, assetsDir string) []download.Task {
	var tasks []download.Task

	tasks = append(tasks, download.Task{
		URL:  v.Downloads.Client.URL,
		Dest: clientJarDest,
		SHA1: v.Downloads.Client.SHA1,
		Size: v.Downloads.Client.Size,
	})

	env := version.CurrentEnv()
	for _, lib := range version.ResolveLibraries(v.Libraries, env) {
		if art, ok := lib.ArtifactInfo(); ok {
			tasks = append(tasks, download.Task{
				URL:  art.URL,
				Dest: filepath.Join(librariesDir, filepath.FromSlash(art.Path)),
				SHA1: art.SHA1,
				Size: art.Size,
			})
		}
	}

	if v.AssetIndex.URL != "" {
		tasks = append(tasks, download.Task{
			URL:  v.AssetIndex.URL,
			Dest: filepath.Join(assetsDir, "indexes", v.AssetIndex.ID+".json"),
			SHA1: v.AssetIndex.SHA1,
			Size: v.AssetIndex.Size,
		})
	}

	return tasks
}

func assetObjectTasks(indexPath, assetsDir string) ([]download.Task, error) {
	data, err := os.ReadFile(indexPath)
	if err != nil {
		return nil, fmt.Errorf("reading asset index %s: %w", indexPath, err)
	}

	var idx struct {
		Objects map[string]struct {
			Hash string `json:"hash"`
			Size int64  `json:"size"`
		} `json:"objects"`
	}
	if err := json.Unmarshal(data, &idx); err != nil {
		return nil, fmt.Errorf("parsing asset index %s: %w", indexPath, err)
	}

	tasks := make([]download.Task, 0, len(idx.Objects))
	for _, obj := range idx.Objects {
		if len(obj.Hash) < 2 {
			continue
		}
		prefix := obj.Hash[:2]
		tasks = append(tasks, download.Task{
			URL:  "https://resources.download.minecraft.net/" + prefix + "/" + obj.Hash,
			Dest: filepath.Join(assetsDir, "objects", prefix, obj.Hash),
			SHA1: obj.Hash,
			Size: obj.Size,
		})
	}
	return tasks, nil
}

func saveResolvedVersionJSON(v version.Version, resolvedDir string) error {
	if err := os.MkdirAll(resolvedDir, 0o755); err != nil {
		return err
	}
	data, err := json.MarshalIndent(v, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(filepath.Join(resolvedDir, "version.json"), data, 0o644)
}

func extractNatives(v version.Version, nativesDir string) error {
	if entries, err := os.ReadDir(nativesDir); err == nil && len(entries) > 0 {
		return nil
	}

	env := version.CurrentEnv()
	type nativeLib struct {
		artifact version.DownloadArtifact
		exclude  []string
	}
	var toExtract []nativeLib
	for _, lib := range version.ResolveLibraries(v.Libraries, env) {
		classifier, ok := version.NativesClassifier(lib, env)
		if !ok {
			continue
		}
		art, ok := lib.Downloads.Classifiers[classifier]
		if !ok {
			continue
		}
		var exclude []string
		if lib.Extract != nil {
			exclude = lib.Extract.Exclude
		}
		toExtract = append(toExtract, nativeLib{art, exclude})
	}
	if len(toExtract) == 0 {
		return nil
	}

	if err := os.MkdirAll(nativesDir, 0o755); err != nil {
		return err
	}
	tmpDir, err := os.MkdirTemp("", "alloyctl-natives-jars-")
	if err != nil {
		return err
	}
	defer os.RemoveAll(tmpDir)

	var tasks []download.Task
	dests := make([]string, len(toExtract))
	for i, item := range toExtract {
		dest := filepath.Join(tmpDir, fmt.Sprintf("%d.jar", i))
		dests[i] = dest
		tasks = append(tasks, download.Task{
			URL:  item.artifact.URL,
			Dest: dest,
			SHA1: item.artifact.SHA1,
			Size: item.artifact.Size,
		})
	}
	if _, err := download.Run(tasks, download.Options{Workers: 8}); err != nil {
		return fmt.Errorf("downloading native libraries: %w", err)
	}

	for i, item := range toExtract {
		if err := extractNativeJar(dests[i], nativesDir, item.exclude); err != nil {
			return err
		}
	}
	return nil
}

func extractNativeJar(jarPath, destDir string, exclude []string) error {
	r, err := zip.OpenReader(jarPath)
	if err != nil {
		return fmt.Errorf("opening native jar %s: %w", jarPath, err)
	}
	defer r.Close()

	for _, f := range r.File {
		if f.FileInfo().IsDir() || strings.HasPrefix(f.Name, "META-INF/") {
			continue
		}
		excluded := false
		for _, pattern := range exclude {
			if strings.HasPrefix(f.Name, pattern) {
				excluded = true
				break
			}
		}
		if excluded {
			continue
		}

		destPath := filepath.Join(destDir, filepath.FromSlash(f.Name))
		if err := os.MkdirAll(filepath.Dir(destPath), 0o755); err != nil {
			return err
		}
		rc, err := f.Open()
		if err != nil {
			return err
		}
		data, err := io.ReadAll(rc)
		rc.Close()
		if err != nil {
			return err
		}
		if err := os.WriteFile(destPath, data, 0o644); err != nil {
			return err
		}
	}
	return nil
}

func runDownloads(tasks []download.Task) error {
	_, err := download.Run(tasks, download.Options{
		Workers: 16,
		Progress: func(done, total int, r download.Result) {
			label := filepath.Base(r.Task.Dest)
			cli.ProgressLine(done, total, label, r.Skipped, r.Err != nil)
		},
	})
	return err
}
