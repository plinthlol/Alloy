package launcher

import (
	"fmt"
	"io"
	"os"
	"os/exec"
	"os/signal"
	"path/filepath"
	"strings"
	"syscall"

	"alloy/internal/version"
)

type Plan struct {
	Version      version.Version
	JavaPath     string
	GameDir      string
	AssetsDir    string
	NativesDir   string
	LibrariesDir string
	ClientJar    string
	Username     string
	UUID         string
	AccessToken  string
	UserType     string
	MemoryMB     int
	ExtraJVM     []string
	Width        int
	Height       int

	ClasspathOverride []string

	MainClassOverride string

	PrefixGameArgs []string
}

func (p Plan) env() version.Env {
	return version.CurrentEnv()
}

func (p Plan) substitutions(classpath string) map[string]string {
	width := "854"
	height := "480"
	if p.Width > 0 {
		width = fmt.Sprint(p.Width)
	}
	if p.Height > 0 {
		height = fmt.Sprint(p.Height)
	}
	return map[string]string{
		"auth_player_name":  p.Username,
		"version_name":      p.Version.ID,
		"game_directory":    p.GameDir,
		"assets_root":       p.AssetsDir,
		"assets_index_name": p.Version.AssetIndex.ID,
		"auth_uuid":         p.UUID,
		"auth_access_token": p.AccessToken,
		"user_type":         p.UserType,
		"version_type":      p.Version.Type,
		"natives_directory": p.NativesDir,
		"launcher_name":     "alloy",
		"launcher_version":  "0.1.0",
		"classpath":         classpath,
		"resolution_width":  width,
		"resolution_height": height,
		"clientid":          "",
		"auth_xuid":         "",
	}
}

func (p Plan) BuildCommand() ([]string, error) {
	env := p.env()
	classpath := p.Classpath()
	subs := p.substitutions(classpath)

	var argv []string

	if p.Version.Arguments != nil {
		jvm := version.ResolveArguments(p.Version.Arguments.JVM, env, subs)
		argv = append(argv, jvm...)
	} else {
		argv = append(argv, p.legacyJVMArgs(classpath)...)
	}

	argv = append(argv, p.ExtraJVM...)
	argv = append(argv, fmt.Sprintf("-Xms%dM", p.MemoryMB), fmt.Sprintf("-Xmx%dM", p.MemoryMB))

	mainClass := p.Version.MainClass
	if p.MainClassOverride != "" {
		mainClass = p.MainClassOverride
	}
	argv = append(argv, mainClass)
	argv = append(argv, p.PrefixGameArgs...)

	if p.Version.Arguments != nil {
		game := version.ResolveArguments(p.Version.Arguments.Game, env, subs)
		argv = append(argv, game...)
	} else {
		game := version.LegacyArguments(p.Version.MinecraftArgs, subs)
		argv = append(argv, game...)
	}

	return argv, nil
}

func (p Plan) legacyJVMArgs(classpath string) []string {
	args := []string{
		"-Djava.library.path=" + p.NativesDir,
		"-cp", classpath,
	}
	if osIsMac() {
		args = append([]string{"-XstartOnFirstThread"}, args...)
	}
	return args
}

func osIsMac() bool {
	return version.CurrentEnv().OSName == "osx"
}

func (p Plan) ClasspathEntries() []string {
	env := p.env()
	libs := version.ResolveLibraries(p.Version.Libraries, env)

	var parts []string
	for _, lib := range libs {
		if art, ok := lib.ArtifactInfo(); ok {
			parts = append(parts, filepath.Join(p.LibrariesDir, filepath.FromSlash(art.Path)))
		}
	}
	parts = append(parts, p.ClientJar)
	return parts
}

func (p Plan) Classpath() string {
	env := p.env()
	sep := ":"
	if env.OSName == "windows" {
		sep = ";"
	}

	parts := p.ClasspathOverride
	if parts == nil {
		parts = p.ClasspathEntries()
	}
	return strings.Join(parts, sep)
}

func (p Plan) Launch(stdout, stderr io.Writer) error {
	argv, err := p.BuildCommand()
	if err != nil {
		return err
	}
	if err := os.MkdirAll(p.GameDir, 0o755); err != nil {
		return err
	}

	cmd := exec.Command(p.JavaPath, argv...)
	cmd.Dir = p.GameDir
	cmd.Stdout = stdout
	cmd.Stderr = stderr
	cmd.Stdin = os.Stdin

	setProcessGroup(cmd)

	if err := cmd.Start(); err != nil {
		return fmt.Errorf("starting game process: %w", err)
	}

	sigChan := make(chan os.Signal, 1)
	signal.Notify(sigChan, os.Interrupt, syscall.SIGTERM, syscall.SIGHUP)

	doneChan := make(chan error, 1)
	go func() {
		doneChan <- cmd.Wait()
	}()

	select {
	case err := <-doneChan:
		return err
	case sig := <-sigChan:
		if sig == syscall.SIGHUP {
			return nil
		}
		if cmd.Process != nil {
			signalJava(cmd.Process, sig)
		}
		return <-doneChan
	}
}
