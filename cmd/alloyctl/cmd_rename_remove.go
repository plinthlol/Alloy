package main

import (
	"fmt"

	"alloy/internal/cli"
	"alloy/internal/config"
	"alloy/internal/instance"
)

func cmdRename(paths config.Paths, args []string) error {
	if len(args) != 2 {
		return fmt.Errorf("usage: alloyctl rename <old-name> <new-name>")
	}
	oldName, newName := args[0], args[1]

	if err := instance.Rename(paths, oldName, newName); err != nil {
		return err
	}
	cli.Info(fmt.Sprintf("Renamed %s to %s.", oldName, newName))
	return nil
}

func cmdRemove(paths config.Paths, args []string) error {
	if len(args) != 1 {
		return fmt.Errorf("usage: alloyctl remove <instance-name>")
	}
	name := args[0]

	if !instance.Exists(paths, name) {
		return fmt.Errorf("no instance named %q. Run `alloyctl play` with no args to see what's available", name)
	}

	confirmed, err := cli.Confirm(fmt.Sprintf("This will permanently delete instance %q, including its worlds and mods. Continue?", name))
	if err != nil {
		return err
	}
	if !confirmed {
		cli.Info("Cancelled.")
		return nil
	}

	if err := instance.Remove(paths, name); err != nil {
		return err
	}
	cli.Info(fmt.Sprintf("Removed %s.", name))
	return nil
}
