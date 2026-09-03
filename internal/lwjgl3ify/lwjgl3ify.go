package lwjgl3ify

import (
	"archive/zip"
	_ "embed"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"runtime"
	"strings"

	"alloy/internal/download"
)

//go:embed assets/alloy-shim.jar
var alloyShim []byte

const log4jFixedBase = "https://files.prismlauncher.org/maven/org/apache/logging/log4j"

type Patches struct {
	JVMArgs   []string
	MainClass string
	ExtraArgs []string
}

func Apply(minecraftDir, libDir string, classpath []string) ([]string, *Patches, error) {
	modsDir := filepath.Join(minecraftDir, "mods")
	jarPath, ok := findLwjgl3ifyJar(modsDir)
	if !ok {
		return classpath, nil, nil
	}

	patchesDest := filepath.Join(minecraftDir, ".forge-patches.jar")
	if err := extractForgePatches(jarPath, patchesDest); err != nil {
		return classpath, nil, fmt.Errorf("extracting lwjgl3ify forge patches: %w", err)
	}

	cp := append([]string{}, classpath...)
	cp = append([]string{patchesDest}, cp...)

	jvmArgs, _ := parseAddOpens(patchesDest)

	jvmArgs = append(jvmArgs,
		"-Djava.system.class.loader=com.gtnewhorizons.retrofuturabootstrap.RfbSystemClassLoader",
		"-Dfile.encoding=UTF-8",
	)

	cp = stripReplacedLibs(cp)
	cp = addLwjgl3(libDir, cp)

	cp = replaceLog4jFixed(libDir, cp)

	writeLog4jConfig(minecraftDir, &jvmArgs)

	shimPath, err := deployShim(minecraftDir)
	if err != nil {
		return classpath, nil, fmt.Errorf("deploying AlloyShim: %w", err)
	}
	cp = append([]string{shimPath}, cp...)

	return cp, &Patches{
		JVMArgs:   jvmArgs,
		MainClass: "AlloyShim",
		ExtraArgs: []string{"com.gtnewhorizons.retrofuturabootstrap.Main"},
	}, nil
}

func deployShim(minecraftDir string) (string, error) {
	dest := filepath.Join(minecraftDir, ".alloy-shim.jar")
	if err := os.WriteFile(dest, alloyShim, 0o644); err != nil {
		return "", err
	}
	return dest, nil
}

func findLwjgl3ifyJar(modsDir string) (string, bool) {
	entries, err := os.ReadDir(modsDir)
	if err != nil {
		return "", false
	}
	for _, e := range entries {
		name := e.Name()
		if strings.HasPrefix(name, "lwjgl3ify") && strings.HasSuffix(name, ".jar") {
			return filepath.Join(modsDir, name), true
		}
	}
	return "", false
}

func extractForgePatches(lwjgl3ifyJar, dest string) error {
	r, err := zip.OpenReader(lwjgl3ifyJar)
	if err != nil {
		return err
	}
	defer r.Close()

	f, err := r.Open("me/eigenraven/lwjgl3ify/relauncher/forgePatches.zip")
	if err != nil {
		return fmt.Errorf("forgePatches.zip not found in %s: %w", filepath.Base(lwjgl3ifyJar), err)
	}
	defer f.Close()

	data, err := io.ReadAll(f)
	if err != nil {
		return err
	}
	return os.WriteFile(dest, data, 0o644)
}

func parseAddOpens(patchesArchive string) ([]string, error) {
	r, err := zip.OpenReader(patchesArchive)
	if err != nil {
		return nil, err
	}
	defer r.Close()

	f, err := r.Open("META-INF/MANIFEST.MF")
	if err != nil {
		return nil, err
	}
	defer f.Close()

	raw, err := io.ReadAll(f)
	if err != nil {
		return nil, err
	}

	manifest := strings.ReplaceAll(string(raw), "\r\n ", "")
	manifest = strings.ReplaceAll(manifest, "\n ", "")

	var args []string
	for _, line := range strings.Split(manifest, "\n") {
		line = strings.TrimRight(line, "\r")
		const prefix = "Add-Opens: "
		if !strings.HasPrefix(line, prefix) {
			continue
		}
		value := strings.TrimPrefix(line, prefix)
		for _, modulePackage := range strings.Fields(value) {
			args = append(args, "--add-opens", modulePackage+"=ALL-UNNAMED")
		}
	}
	return args, nil
}

func stripReplacedLibs(classpath []string) []string {
	replaced := []string{
		"launchwrapper-",
		"asm-all-",
		"lwjgl-2.",
		"lwjgl_util-",
		"commons-compress-",
		"commons-io-",
		"guava-15.",
	}

	out := classpath[:0:0]
	for _, entry := range classpath {
		name := filepath.Base(entry)
		dominated := false
		for _, prefix := range replaced {
			if strings.HasPrefix(name, prefix) {
				dominated = true
				break
			}
		}
		if !dominated {
			out = append(out, entry)
		}
	}
	return out
}

func addLwjgl3(libDir string, classpath []string) []string {
	modules := []string{
		"lwjgl",
		"lwjgl-freetype",
		"lwjgl-glfw",
		"lwjgl-jemalloc",
		"lwjgl-openal",
		"lwjgl-opengl",
		"lwjgl-stb",
		"lwjgl-tinyfd",
	}

	var osClassifier string
	switch runtime.GOOS {
	case "windows":
		osClassifier = "natives-windows"
	case "darwin":
		osClassifier = "natives-macos"
	default:
		osClassifier = "natives-linux"
	}

	insertPos := 1
	if insertPos > len(classpath) {
		insertPos = len(classpath)
	}

	insert := func(path string) {
		if _, err := os.Stat(path); err != nil {
			return
		}
		classpath = append(classpath, "")
		copy(classpath[insertPos+1:], classpath[insertPos:])
		classpath[insertPos] = path
		insertPos++
	}

	for _, module := range modules {
		var base, natives string
		if module == "lwjgl" {
			base = filepath.Join(libDir, "org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3.jar")
			natives = filepath.Join(libDir, fmt.Sprintf("org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-%s.jar", osClassifier))
		} else {
			base = filepath.Join(libDir, fmt.Sprintf("org/lwjgl/%s/3.3.3/%s-3.3.3.jar", module, module))
			natives = filepath.Join(libDir, fmt.Sprintf("org/lwjgl/%s/3.3.3/%s-3.3.3-%s.jar", module, module, osClassifier))
		}
		insert(base)
		insert(natives)
	}

	return classpath
}

func replaceLog4jFixed(libDir string, classpath []string) []string {
	type replacement struct {
		oldName  string
		fixedRel string
		url      string
	}
	replacements := []replacement{
		{
			oldName:  "log4j-api-2.0-beta9.jar",
			fixedRel: "org/apache/logging/log4j/log4j-api/2.0-beta9-fixed/log4j-api-2.0-beta9-fixed.jar",
			url:      log4jFixedBase + "/log4j-api/2.0-beta9-fixed/log4j-api-2.0-beta9-fixed.jar",
		},
		{
			oldName:  "log4j-core-2.0-beta9.jar",
			fixedRel: "org/apache/logging/log4j/log4j-core/2.0-beta9-fixed/log4j-core-2.0-beta9-fixed.jar",
			url:      log4jFixedBase + "/log4j-core/2.0-beta9-fixed/log4j-core-2.0-beta9-fixed.jar",
		},
	}

	out := append([]string{}, classpath...)

	for _, r := range replacements {
		fixedPath := filepath.Join(libDir, filepath.FromSlash(r.fixedRel))

		if _, err := os.Stat(fixedPath); err != nil {
			if _, dlErr := download.Run([]download.Task{{URL: r.url, Dest: fixedPath}}, download.Options{Workers: 1}); dlErr != nil {
				continue
			}
		}

		for i, entry := range out {
			if filepath.Base(entry) == r.oldName {
				out[i] = fixedPath
			}
		}
	}

	return out
}

func writeLog4jConfig(minecraftDir string, jvmArgs *[]string) {
	configPath := filepath.Join(minecraftDir, ".alloy-log4j2.xml")
	const config = `<?xml version="1.0" encoding="UTF-8"?>
<Configuration status="WARN">
    <Appenders>
        <Console name="SysOut" target="SYSTEM_OUT">
            <PatternLayout pattern="[%d{HH:mm:ss}] [%t/%level] [%logger]: %msg%n"/>
        </Console>
        <Queue name="ServerGuiConsole">
            <PatternLayout pattern="[%d{HH:mm:ss} %level]: %msg%n"/>
        </Queue>
        <RollingRandomAccessFile name="File" fileName="logs/latest.log"
                filePattern="logs/%d{yyyy-MM-dd}-%i.log.gz">
            <PatternLayout pattern="[%d{HH:mm:ss}] [%t/%level]: %msg%n"/>
            <Policies>
                <TimeBasedTriggeringPolicy/>
                <OnStartupTriggeringPolicy/>
            </Policies>
        </RollingRandomAccessFile>
    </Appenders>
    <Loggers>
        <Root level="info">
            <AppenderRef ref="SysOut"/>
            <AppenderRef ref="File"/>
        </Root>
    </Loggers>
</Configuration>
`
	if err := os.WriteFile(configPath, []byte(config), 0o644); err != nil {
		return
	}
	*jvmArgs = append(*jvmArgs, "-Dlog4j.configurationFile="+configPath)
}
