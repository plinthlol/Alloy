package lwjgl3ify

import (
	"archive/zip"
	"os"
	"path/filepath"
	"reflect"
	"testing"
)

func makeZip(t *testing.T, dir, name string, entries map[string]string) string {
	t.Helper()
	path := filepath.Join(dir, name)
	f, err := os.Create(path)
	if err != nil {
		t.Fatal(err)
	}
	defer f.Close()
	w := zip.NewWriter(f)
	for entryName, content := range entries {
		fw, err := w.Create(entryName)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := fw.Write([]byte(content)); err != nil {
			t.Fatal(err)
		}
	}
	if err := w.Close(); err != nil {
		t.Fatal(err)
	}
	return path
}

func TestStripReplacedLibsRemovesDominatedPrefixes(t *testing.T) {
	classpath := []string{
		"/libs/launchwrapper-1.12.jar",
		"/libs/asm-all-5.0.3.jar",
		"/libs/lwjgl-2.9.4.jar",
		"/libs/lwjgl_util-2.9.4.jar",
		"/libs/commons-compress-1.4.1.jar",
		"/libs/commons-io-2.4.jar",
		"/libs/guava-15.0.jar",
		"/libs/log4j-core-2.0.jar",
		"/libs/guava-21.0.jar",
	}
	got := stripReplacedLibs(classpath)
	want := []string{"/libs/log4j-core-2.0.jar", "/libs/guava-21.0.jar"}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("got %v, want %v", got, want)
	}
}

func TestStripReplacedLibsKeepsUnrelatedEntries(t *testing.T) {
	classpath := []string{"/libs/log4j-core-2.0.jar", "/libs/mixin-0.8.5.jar"}
	got := stripReplacedLibs(classpath)
	if !reflect.DeepEqual(got, classpath) {
		t.Fatalf("got %v, want unchanged %v", got, classpath)
	}
}

func TestParseAddOpensExtractsModuleArgs(t *testing.T) {
	tmp := t.TempDir()
	manifest := "Manifest-Version: 1.0\nAdd-Opens: java.base/java.lang java.base/java.util\n"
	zipPath := makeZip(t, tmp, "patches.zip", map[string]string{"META-INF/MANIFEST.MF": manifest})

	args, err := parseAddOpens(zipPath)
	if err != nil {
		t.Fatal(err)
	}
	want := []string{
		"--add-opens", "java.base/java.lang=ALL-UNNAMED",
		"--add-opens", "java.base/java.util=ALL-UNNAMED",
	}
	if !reflect.DeepEqual(args, want) {
		t.Fatalf("got %v, want %v", args, want)
	}
}

func TestParseAddOpensHandlesContinuationLines(t *testing.T) {
	tmp := t.TempDir()
	manifest := "Manifest-Version: 1.0\nAdd-Opens: java.base/java.lang \n java.base/sun.security.util\n"
	zipPath := makeZip(t, tmp, "patches-continuation.zip", map[string]string{"META-INF/MANIFEST.MF": manifest})

	args, err := parseAddOpens(zipPath)
	if err != nil {
		t.Fatal(err)
	}
	want := []string{
		"--add-opens", "java.base/java.lang=ALL-UNNAMED",
		"--add-opens", "java.base/sun.security.util=ALL-UNNAMED",
	}
	if !reflect.DeepEqual(args, want) {
		t.Fatalf("got %v, want %v", args, want)
	}
}

func TestParseAddOpensErrorsWhenManifestMissing(t *testing.T) {
	tmp := t.TempDir()
	zipPath := makeZip(t, tmp, "no-manifest.zip", map[string]string{"other.txt": "x"})
	if _, err := parseAddOpens(zipPath); err == nil {
		t.Fatal("expected error for missing manifest")
	}
}

func TestParseAddOpensErrorsForMissingFile(t *testing.T) {
	if _, err := parseAddOpens("/nonexistent/x.zip"); err == nil {
		t.Fatal("expected error for missing file")
	}
}

func TestAddLwjgl3InsertsOnlyJarsThatExist(t *testing.T) {
	tmp := t.TempDir()

	coreJar := filepath.Join(tmp, "org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3.jar")
	if err := os.MkdirAll(filepath.Dir(coreJar), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(coreJar, []byte("jar"), 0o644); err != nil {
		t.Fatal(err)
	}

	classpath := []string{"/leading/forge-patches.jar"}
	got := addLwjgl3(tmp, classpath)

	if got[0] != "/leading/forge-patches.jar" {
		t.Fatalf("forge-patches should stay at index 0, got %v", got)
	}
	if len(got) != 2 || got[1] != coreJar {
		t.Fatalf("expected [forge-patches, coreJar], got %v", got)
	}
}
