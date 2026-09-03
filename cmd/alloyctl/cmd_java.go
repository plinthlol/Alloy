package main

import (
	"fmt"

	"alloy/internal/cli"
	"alloy/internal/config"
	"alloy/internal/javafind"
)

func cmdJava(paths config.Paths, args []string) error {
	if len(args) == 0 {
		return fmt.Errorf("usage: alloyctl java list | alloyctl java set <path>")
	}
	switch args[0] {
	case "list":
		return javaList(paths)
	case "set":
		if len(args) < 2 {
			return fmt.Errorf("usage: alloyctl java set <path>")
		}
		return javaSet(paths, args[1])
	default:
		return fmt.Errorf("unknown java subcommand %q", args[0])
	}
}

func javaList(paths config.Paths) error {
	g, err := config.Load(paths)
	if err != nil {
		return err
	}

	candidates := javafind.Candidates(g.Paths.JavaPath)
	if len(candidates) == 0 {
		cli.Info("No Java installations found.")
		return nil
	}

	var verified []javafind.Verified
	for _, c := range candidates {
		v, err := javafind.Verify(c)
		if err != nil {
			continue
		}
		verified = append(verified, v)
	}

	if len(verified) == 0 {
		cli.Info("No usable Java installations found.")
		return nil
	}

	best, _ := javafind.Best(verified, 21)

	fmt.Printf("%-45s %-12s %-20s %s\n", "PATH", "VERSION", "SOURCE", "")
	for _, v := range verified {
		mark := ""
		if v.Path == g.Paths.JavaPath {
			mark = "\033[33m(override)\033[0m"
		} else if v.Path == best.Path {
			mark = "\033[32m(selected)\033[0m"
		}
		fmt.Printf("%-45s %-12s %-20s %s\n", v.Path, v.RawVersion, v.Source, mark)
	}
	return nil
}

func javaSet(paths config.Paths, path string) error {
	c := javafind.Candidate{Path: path, Source: "override"}
	v, err := javafind.Verify(c)
	if err != nil {
		return fmt.Errorf("%s does not look like a working java binary: %w", path, err)
	}

	g, err := config.Load(paths)
	if err != nil {
		return err
	}
	g.Paths.JavaPath = path
	if err := config.Save(paths, g); err != nil {
		return err
	}

	cli.Info(fmt.Sprintf("Set global java_path to %s (Java %d, %s).", path, v.MajorVersion, v.RawVersion))
	return nil
}
