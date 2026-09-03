package main

import (
	"fmt"
	"os"
	"path/filepath"

	"alloy/internal/cli"
	"alloy/internal/config"
	"alloy/internal/instance"
	"alloy/internal/javafind"
	"alloy/internal/loader/fabric"
	"alloy/internal/loader/forge"
	"alloy/internal/loader/neoforge"
	"alloy/internal/loader/quilt"
	"alloy/internal/version"
)

// resolveOnDemand rebuilds a resolved version.json for an instance whose
// cache entry under ResolvedCacheDir is missing — the situation you hit when
// launching an instance that alloysh created. alloysh never writes this
// cache; it resolves the merged vanilla+loader profile in memory every time
// it launches (see the Rust side's launch_profile::resolve). This mirrors
// that: it re-derives the same resolved profile alloyctl's own `install`
// would have produced, fetches whatever files are missing, and then writes
// the cache so future launches (from either binary) are instant again.
//
// Crucially this uses meta.LoaderVersion — the version already recorded in
// instance.json — rather than "latest", so an instance built against
// Fabric 0.19.3 stays on Fabric 0.19.3 even if a newer loader has since
// shipped.
func resolveOnDemand(paths config.Paths, meta instance.Meta) (version.Version, error) {
	manifest, err := version.LoadOrFetchManifest(paths.ManifestCacheFile())
	if err != nil {
		return version.Version{}, fmt.Errorf("loading version manifest: %w", err)
	}
	entry, ok := manifest.Find(meta.MCVersion)
	if !ok {
		return version.Version{}, fmt.Errorf("no such Minecraft version %q in the version manifest", meta.MCVersion)
	}

	base, baseRaw, err := version.FetchVersionJSON(entry.URL)
	if err != nil {
		return version.Version{}, fmt.Errorf("fetching version json: %w", err)
	}
	if err := os.MkdirAll(paths.VersionMetaDir(meta.MCVersion), 0o755); err != nil {
		return version.Version{}, fmt.Errorf("creating version meta dir: %w", err)
	}
	if err := os.WriteFile(paths.VersionMetaFile(meta.MCVersion), baseRaw, 0o644); err != nil {
		return version.Version{}, fmt.Errorf("saving version meta: %w", err)
	}

	loaderVersion := meta.LoaderVersion
	resolved := base

	switch meta.Loader {
	case "":
		// vanilla — base is already the full profile

	case "fabric":
		if loaderVersion == "" {
			return version.Version{}, fmt.Errorf("instance has no recorded fabric loader version")
		}
		var raw []byte
		resolved, raw, err = fabric.FetchProfile(meta.MCVersion, loaderVersion, base)
		if err != nil {
			return version.Version{}, err
		}
		if serr := saveLoaderProfile(paths, fmt.Sprintf("fabric-%s-%s.json", meta.MCVersion, loaderVersion), raw); serr != nil {
			cli.Errorf("warning: could not cache fabric profile: %s", serr)
		}

	case "quilt":
		if loaderVersion == "" {
			return version.Version{}, fmt.Errorf("instance has no recorded quilt loader version")
		}
		var raw []byte
		resolved, raw, err = quilt.FetchProfile(meta.MCVersion, loaderVersion, base)
		if err != nil {
			return version.Version{}, err
		}
		if serr := saveLoaderProfile(paths, fmt.Sprintf("quilt-%s-%s.json", meta.MCVersion, loaderVersion), raw); serr != nil {
			cli.Errorf("warning: could not cache quilt profile: %s", serr)
		}

	case "forge", "neoforge":
		resolved, err = resolveForgeFamilyOnDemand(paths, meta, base, loaderVersion)
		if err != nil {
			return version.Version{}, err
		}

	default:
		return version.Version{}, fmt.Errorf("unknown loader %q", meta.Loader)
	}

	// fetch whatever the client jar/libraries/assets need that aren't on
	// disk yet — an alloysh-created instance keeps its own copies under
	// meta_dir, in the same layout, so usually this is a no-op, but we
	// can't assume it.
	phase1 := buildDownloadTasks(resolved, paths.ClientJarPath(meta.MCVersion), paths.LibrariesDir(), paths.AssetsDir())
	if err := runDownloads(phase1); err != nil {
		return version.Version{}, fmt.Errorf("downloading client jar/libraries: %w", err)
	}
	if err := extractNatives(resolved, paths.NativesDir(meta.MCVersion)); err != nil {
		return version.Version{}, fmt.Errorf("extracting native libraries: %w", err)
	}
	if resolved.AssetIndex.URL != "" {
		indexPath := filepath.Join(paths.AssetsDir(), "indexes", resolved.AssetIndex.ID+".json")
		assetTasks, err := assetObjectTasks(indexPath, paths.AssetsDir())
		if err != nil {
			return version.Version{}, fmt.Errorf("expanding asset index: %w", err)
		}
		if err := runDownloads(assetTasks); err != nil {
			return version.Version{}, fmt.Errorf("downloading assets: %w", err)
		}
	}

	cacheKey := cacheKeyFor(meta.MCVersion, meta.Loader, loaderVersion)
	if serr := saveResolvedVersionJSON(resolved, paths.ResolvedCacheDir(cacheKey)); serr != nil {
		// not fatal — we can still launch this time, we just won't be fast
		// next time either
		cli.Errorf("warning: could not cache resolved version definition: %s", serr)
	}

	return resolved, nil
}

// resolveForgeFamilyOnDemand runs the (Neo)Forge installer headlessly, same
// as cmdInstall does, but against an existing instance's game directory
// instead of a freshly created one.
func resolveForgeFamilyOnDemand(paths config.Paths, meta instance.Meta, base version.Version, loaderVersion string) (version.Version, error) {
	if loaderVersion == "" {
		return version.Version{}, fmt.Errorf("instance has no recorded %s loader version", meta.Loader)
	}

	g, _ := config.Load(paths)
	candidates := javafind.Candidates(g.Paths.JavaPath)
	var verified []javafind.Verified
	for _, c := range candidates {
		if v, err := javafind.Verify(c); err == nil {
			verified = append(verified, v)
		}
	}

	minecraftDir := paths.InstanceMinecraftDir(meta.Name)

	if meta.Loader == "forge" {
		mcForgeVersion := meta.MCVersion + "-" + loaderVersion
		installerPath := filepath.Join(os.TempDir(), fmt.Sprintf("forge-%s-installer.jar", mcForgeVersion))
		if err := forge.DownloadInstaller(mcForgeVersion, "", installerPath); err != nil {
			return version.Version{}, fmt.Errorf("downloading forge installer: %w", err)
		}
		defer os.Remove(installerPath)

		var raw []byte
		var err error
		if forge.HasLegacyInstallProfile(installerPath) {
			cli.Info("Installing legacy Forge from profile...")
			raw, err = forge.InstallFromLegacyProfile(installerPath, paths.LibrariesDir())
		} else {
			bestJava, ok := javafind.Best(verified, 8)
			if !ok {
				return version.Version{}, fmt.Errorf("Forge installer requires Java 8+, but no Java runtime was found")
			}
			cli.Info("Running Forge installer in headless mode...")
			if err = forge.RunInstallerHeadless(bestJava.Path, installerPath, minecraftDir); err == nil {
				versionDirName := fmt.Sprintf("%s-forge-%s", meta.MCVersion, loaderVersion)
				raw, err = extractInstallerProfile(minecraftDir, versionDirName)
			}
		}
		if err != nil {
			return version.Version{}, err
		}
		if serr := saveLoaderProfile(paths, fmt.Sprintf("forge-%s-%s.json", meta.MCVersion, loaderVersion), raw); serr != nil {
			cli.Errorf("warning: could not cache forge profile: %s", serr)
		}
		return mergeLoaderVersionJSON(base, raw, meta.MCVersion+"-forge-"+loaderVersion)
	}

	// neoforge
	installerPath := filepath.Join(os.TempDir(), fmt.Sprintf("neoforge-%s-installer.jar", loaderVersion))
	if err := neoforge.DownloadInstaller(loaderVersion, "", installerPath); err != nil {
		return version.Version{}, fmt.Errorf("downloading neoforge installer: %w", err)
	}
	defer os.Remove(installerPath)

	bestJava, ok := javafind.Best(verified, 17)
	if !ok {
		return version.Version{}, fmt.Errorf("NeoForge installer requires Java 17+, but no Java runtime was found")
	}
	cli.Info("Running NeoForge installer in headless mode...")
	if err := neoforge.RunInstallerHeadless(bestJava.Path, installerPath, minecraftDir); err != nil {
		return version.Version{}, err
	}

	versionDirName := fmt.Sprintf("neoforge-%s", loaderVersion)
	raw, err := extractInstallerProfile(minecraftDir, versionDirName)
	if err != nil {
		return version.Version{}, fmt.Errorf("extracting neoforge profile: %w", err)
	}
	if serr := saveLoaderProfile(paths, fmt.Sprintf("neoforge-%s.json", loaderVersion), raw); serr != nil {
		cli.Errorf("warning: could not cache neoforge profile: %s", serr)
	}
	return mergeLoaderVersionJSON(base, raw, meta.MCVersion+"-neoforge-"+loaderVersion)
}
