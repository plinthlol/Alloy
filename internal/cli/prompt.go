package cli

import (
	"bufio"
	"fmt"
	"os"
	"strconv"
	"strings"
)

var stdinReader = bufio.NewReader(os.Stdin)

func PromptString(question string) (string, error) {
	fmt.Print(question + ": ")
	line, err := stdinReader.ReadString('\n')
	if err != nil {
		return "", err
	}
	return strings.TrimSpace(line), nil
}

func PromptSelectIndex(question string, items []string) (int, error) {
	fmt.Println(question)
	for i, item := range items {
		fmt.Printf("  %d) %s\n", i+1, item)
	}
	for {
		raw, err := PromptString("Enter a number")
		if err != nil {
			return 0, err
		}
		n, err := strconv.Atoi(raw)
		if err != nil || n < 1 || n > len(items) {
			fmt.Println("Please enter a number from the list above.")
			continue
		}
		return n - 1, nil
	}
}

func Confirm(question string) (bool, error) {
	raw, err := PromptString(question + " [y/N]")
	if err != nil {
		return false, err
	}
	raw = strings.ToLower(strings.TrimSpace(raw))
	return raw == "y" || raw == "yes", nil
}
