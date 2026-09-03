package main

import (
	"fmt"
	"os"

	"alloy/internal/cli"
	"alloy/internal/config"
)

func main() {
	if err := run(os.Args[1:]); err != nil {
		cli.Errorf("%s", err)
		os.Exit(1)
	}
}

func run(args []string) error {
	if len(args) == 0 {
		printUsage()
		return nil
	}

	paths, err := config.Resolve()
	if err != nil {
		return fmt.Errorf("resolving app directories: %w", err)
	}

	cmd, rest := args[0], args[1:]
	switch cmd {
	case "auth":
		return cmdAuth(paths, rest)
	case "install":
		return cmdInstall(paths, rest)
	case "play":
		return cmdPlay(paths, rest)
	case "rename":
		return cmdRename(paths, rest)
	case "remove":
		return cmdRemove(paths, rest)
	case "java":
		return cmdJava(paths, rest)
	case "-h", "--help", "help":
		printUsage()
		return nil
	default:
		return fmt.Errorf("unknown command %q", cmd)
	}
}

func printUsage() {
	bold := "\033[1m"
	cyan := "\033[36m"
	reset := "\033[0m"

	art := "\n" +
		"                      #############              " + bold + reset + "\n" +
		"                      ####################       " + bold + reset + "\n" +
		"          ######################      ## ####    " + bold + reset + "\n" +
		"      #########################       ##  #####  " + bold + reset + "\n" +
		"   #############################         ########" + bold + reset + "\n" +
		" ################################################" + bold + reset + "\n" +
		"#############################################    " + bold + reset + "\n" +
		"##################     ######                    " + bold + reset + "\n" +
		"############### #####  ### ##                    " + bold + reset + "\n" +
		" ##############  ##############                  " + bold + reset + "\n" +
		"  ############    #####   #######                " + bold + reset + "\n" +
		"   ############    #######  ######               " + bold + reset + "\n" +
		"     ############### ############                " + bold + reset + "\n" +
		"        #######################                  " + bold + reset + "\n" +
		"                ##########                       " + bold + reset + "\n"
	fmt.Println(art)
	fmt.Println()
	fmt.Println(" ", bold+"alloyctl"+reset)
	fmt.Println()
	fmt.Println(" ", bold+"Usage:"+reset)
	fmt.Println(" ", bold+"auth"+reset)
	fmt.Println("       ", cyan+"online"+reset, "                ", cyan+"list"+reset)
	fmt.Println("       ", cyan+"offline <username>"+reset, "    ", cyan+"switch <username>"+reset)
	fmt.Println("       ", cyan+"remove <username>"+reset)
	fmt.Println()
	fmt.Println(" ", bold+"install"+reset)
	fmt.Println("           ", cyan+"<version> [--fabric|--quilt|--forge|--neoforge] [--name <instance>]"+reset)
	fmt.Println()
	fmt.Println(" ", bold+"play"+reset)
	fmt.Println("        ", cyan+"[instance]"+reset)
	fmt.Println("                   ", cyan+"[--memory <MB>] [--width <px>] [--height <px>]"+reset)
	fmt.Println("                   ", cyan+"[--jvm <arg>] (repeatable, e.g. --jvm \"-Xss4m\")"+reset)
	fmt.Println()
	fmt.Println(" ", bold+"manage"+reset)
	fmt.Println("          ", cyan+"rename <old> <new>"+reset)
	fmt.Println("          ", cyan+"remove <instance>"+reset)
	fmt.Println()
	fmt.Println(" ", bold+"java"+reset)
	fmt.Println("        ", cyan+"list"+reset)
	fmt.Println("        ", cyan+"set <path>"+reset)
}
