//go:build windows

package launcher

import (
	"os"
	"os/exec"
	"syscall"
)

func setProcessGroup(cmd *exec.Cmd) {
	cmd.SysProcAttr = &syscall.SysProcAttr{
		CreationFlags: syscall.CREATE_NEW_PROCESS_GROUP,
	}
}

func signalJava(proc *os.Process, sig os.Signal) {
	if err := proc.Signal(sig); err != nil {
		_ = proc.Kill()
	}
}
