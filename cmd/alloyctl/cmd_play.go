package main

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"alloy/internal/auth"
	"alloy/internal/cli"
	"alloy/internal/config"
	"alloy/internal/download"
	"alloy/internal/instance"
	"alloy/internal/javafind"
	"alloy/internal/launcher"
	"alloy/internal/lwjgl3ify"
	"alloy/internal/version"
)

type launchFlags struct {
	memoryMB int
	width    int
	height   int
	jvmArgs  []string
}

func cmdPlay(paths config.Paths, args []string) error {
	if len(args) == 0 {
		return playList(paths)
	}

	var flags launchFlags
	var positional []string
	for i := 0; i < len(args); i++ {
		switch args[i] {
		case "--memory", "-m":
			if i+1 < len(args) {
				i++
				fmt.Sscanf(args[i], "%d", &flags.memoryMB)
			}
		case "--width":
			if i+1 < len(args) {
				i++
				fmt.Sscanf(args[i], "%d", &flags.width)
			}
		case "--height":
			if i+1 < len(args) {
				i++
				fmt.Sscanf(args[i], "%d", &flags.height)
			}
		case "--jvm", "-J":
			if i+1 < len(args) {
				i++
				flags.jvmArgs = append(flags.jvmArgs, args[i])
			}
		default:
			positional = append(positional, args[i])
		}
	}

	if len(positional) == 0 {
		return playList(paths)
	}
	return playLaunch(paths, positional[0], flags)
}

func playList(paths config.Paths) error {
	names, err := instance.List(paths)
	if err != nil {
		return err
	}
	if len(names) == 0 {
		cli.Info("No instances installed yet. Run `alloyctl install <version>` to create one.")
		return nil
	}

	manifest, err := version.LoadOrFetchManifest(paths.ManifestCacheFile())
	if err != nil {
		cli.Errorf("could not load version manifest for coloring (%v); listing without color", err)
	}

	type row struct {
		meta        instance.Meta
		releaseTime string
		vType       string
	}
	rows := make([]row, 0, len(names))
	for _, name := range names {
		m, err := instance.Load(paths, name)
		if err != nil {
			continue
		}
		vType := ""
		releaseTime := ""
		if entry, ok := manifest.Find(m.MCVersion); ok {
			vType = entry.Type
			releaseTime = entry.ReleaseTime.Format("2006-01-02T15:04:05")
		}
		rows = append(rows, row{meta: m, vType: vType, releaseTime: releaseTime})
	}

	sortRows(rows, func(a, b row) bool { return a.releaseTime < b.releaseTime })

	for _, r := range rows {
		label := r.meta.Name
		if r.meta.Loader != "" {
			label += fmt.Sprintf(" (%s %s %s)", r.meta.MCVersion, r.meta.Loader, r.meta.LoaderVersion)
		} else {
			label += fmt.Sprintf(" (%s)", r.meta.MCVersion)
		}
		fmt.Println(cli.ColorForVersionType(r.vType, label))
	}
	return nil
}

func sortRows[T any](rows []T, less func(a, b T) bool) {
	for i := 1; i < len(rows); i++ {
		for j := i; j > 0 && less(rows[j], rows[j-1]); j-- {
			rows[j], rows[j-1] = rows[j-1], rows[j]
		}
	}
}

func playLaunch(paths config.Paths, name string, flags launchFlags) error {
	if !instance.Exists(paths, name) {
		return fmt.Errorf("no instance named %q. Run `alloyctl play` with no args to see what's available", name)
	}
	meta, err := instance.Load(paths, name)
	if err != nil {
		return err
	}

	g, err := config.Load(paths)
	if err != nil {
		return err
	}
	store := accountStore(paths)
	account, ok, err := store.Active()
	if err != nil {
		return err
	}
	if !ok {
		return fmt.Errorf("no active account. Run `alloyctl auth offline <username>` or `alloyctl auth online` first")
	}

	resolvedDir := paths.ResolvedCacheDir(meta.CacheKey())
	versionJSONPath := filepath.Join(resolvedDir, "version.json")
	resolved, err := version.LoadVersionJSON(versionJSONPath)
	if err != nil {
		// No cached resolution — most likely this instance was created by
		// alloysh, which resolves the merged version profile in memory at
		// launch time and never writes this cache. Build it now instead of
		// erroring, using the loader version already recorded in
		// instance.json (not "latest") so we match what the instance was
		// actually built with.
		cli.Info(fmt.Sprintf("No cached version definition for %q — resolving it now...", name))
		resolved, err = resolveOnDemand(paths, meta)
		if err != nil {
			return fmt.Errorf(
				"resolving version for instance %q: %w — try reinstalling it with `alloyctl install %s%s --name <new-name>`",
				meta.Name, err, meta.MCVersion, loaderFlagFor(meta.Loader),
			)
		}
	}

	override := meta.JavaPath
	if override == "" {
		override = g.Paths.JavaPath
	}
	candidates := javafind.Candidates(override)
	var verified []javafind.Verified
	for _, c := range candidates {
		if v, err := javafind.Verify(c); err == nil {
			verified = append(verified, v)
		}
	}
	best, ok := javafind.Best(verified, resolved.JavaVersion.MajorVersion)
	if !ok {
		return fmt.Errorf(
			"this version needs Java %d, found %s — install Java %d or set it manually with `alloyctl java set <path>`",
			resolved.JavaVersion.MajorVersion, javafind.DescribeAvailable(verified), resolved.JavaVersion.MajorVersion,
		)
	}

	memMB := flags.memoryMB
	if memMB == 0 {
		memMB = meta.MemoryMaxMB
	}
	if memMB == 0 {
		memMB = g.Defaults.MemoryMaxMB
	}
	if memMB == 0 {
		memMB = 2048
	}

	accessToken := "0"
	userType := "legacy"
	if account.AccountType == auth.AccountTypeMicrosoft {
		userType = "msa"

		const refreshMarginSecs = int64(5 * time.Minute / time.Second)
		hasValidCache := account.CachedMCToken != "" && account.CachedMCTokenExpiresAt != nil &&
			time.Now().Unix() < *account.CachedMCTokenExpiresAt-refreshMarginSecs

		if hasValidCache {
			accessToken = account.CachedMCToken
		} else {
			msResp, err := auth.RefreshMicrosoftToken(account.RefreshToken)
			if err != nil {
				return fmt.Errorf("refreshing Microsoft login token: %w", err)
			}
			prof, err := auth.CompleteAuthChain(msResp.AccessToken, msResp.RefreshToken, msResp.ExpiresIn)
			if err != nil {
				return fmt.Errorf("authenticating Microsoft online profile: %w", err)
			}
			accessToken = prof.MinecraftToken

			newRefresh := ""
			if prof.RefreshToken != "" && prof.RefreshToken != account.RefreshToken {
				newRefresh = prof.RefreshToken
			}
			expiresAt := prof.AccessExpiresAt.Unix()
			if err := store.UpdateTokens(account.UUID, newRefresh, prof.MinecraftToken, &expiresAt); err != nil {
				cli.Errorf("warning: could not cache Minecraft session: %s", err)
			}
		}
	}

	var missingTasks []download.Task
	env := version.CurrentEnv()
	libsDir := paths.LibrariesDir()
	for _, lib := range version.ResolveLibraries(resolved.Libraries, env) {
		if art, ok := lib.ArtifactInfo(); ok {
			destPath := filepath.Join(libsDir, filepath.FromSlash(art.Path))
			if _, err := os.Stat(destPath); os.IsNotExist(err) {
				missingTasks = append(missingTasks, download.Task{
					URL:  art.URL,
					Dest: destPath,
					SHA1: art.SHA1,
					Size: art.Size,
				})
			}
		}
	}
	if len(missingTasks) > 0 {
		cli.Info(fmt.Sprintf("Downloading %d missing library files...", len(missingTasks)))
		if err := runDownloads(missingTasks); err != nil {
			return fmt.Errorf("downloading missing libraries: %w", err)
		}
	}

	if err := extractNatives(resolved, paths.NativesDir(meta.MCVersion)); err != nil {
		return fmt.Errorf("extracting native libraries: %w", err)
	}

	extraJVM := append(append([]string{}, meta.JVMArgs...), flags.jvmArgs...)

	plan := launcher.Plan{
		Version:      resolved,
		JavaPath:     best.Path,
		GameDir:      paths.InstanceMinecraftDir(name),
		AssetsDir:    paths.AssetsDir(),
		NativesDir:   paths.NativesDir(meta.MCVersion),
		LibrariesDir: paths.LibrariesDir(),
		ClientJar:    paths.ClientJarPath(meta.MCVersion),
		Username:     account.Username,
		UUID:         account.UUID,
		AccessToken:  accessToken,
		UserType:     userType,
		MemoryMB:     memMB,
		ExtraJVM:     extraJVM,
		Width:        flags.width,
		Height:       flags.height,
	}

	patchedCP, patches, err := lwjgl3ify.Apply(plan.GameDir, plan.LibrariesDir, plan.ClasspathEntries())
	if err != nil {
		cli.Errorf("warning: lwjgl3ify patching failed, launching without it: %s", err)
	} else if patches != nil {
		cli.Info("Detected lwjgl3ify — patching classpath and JVM args for modern Java")
		plan.ClasspathOverride = patchedCP
		plan.MainClassOverride = patches.MainClass
		plan.PrefixGameArgs = patches.ExtraArgs
		plan.ExtraJVM = append(plan.ExtraJVM, patches.JVMArgs...)
	}

	cli.Info(fmt.Sprintf("Launching %q with Java %d (%s)...", name, best.MajorVersion, best.Path))
	return plan.Launch(os.Stdout, os.Stderr)
}

func loaderFlagFor(loader string) string {
	if loader == "" {
		return ""
	}
	return " --" + strings.ToLower(loader)
}
