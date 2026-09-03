package cli

import "fmt"

func ProgressLine(done, total int, label string, skipped bool, failed bool) {
	status := " "
	if skipped {
		status = paint(gray, "=")
	} else if failed {
		status = paint(red, "x")
	} else {
		status = paint(green, "+")
	}
	fmt.Printf("\r\x1b[K[%s] %*d/%d %s", status, digits(total), done, total, label)
	if done == total {
		fmt.Println()
	}
}

func digits(n int) int {
	d := 1
	for n >= 10 {
		n /= 10
		d++
	}
	return d
}
