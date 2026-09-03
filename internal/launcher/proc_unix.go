//go:build !windows

package launcher

import (
	"os"
	"os/exec"
	"syscall"
)

func setProcessGroup(cmd *exec.Cmd) {
	cmd.SysProcAttr = &syscall.SysProcAttr{
		Setpgid: true,
	}
}

func signalJava(proc *os.Process, sig os.Signal) {
	_ = proc.Signal(sig)
}
