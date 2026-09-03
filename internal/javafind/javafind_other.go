//go:build !windows

package javafind

func platformExtraCandidates() []string {
	return nil
}
