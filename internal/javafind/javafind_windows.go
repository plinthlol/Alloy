//go:build windows

package javafind

import (
	"golang.org/x/sys/windows/registry"
)

func platformExtraCandidates() []string {
	return registryJavaHomes()
}

func registryJavaHomes() []string {
	var homes []string
	scan := func(root registry.Key) {
		for _, sub := range []string{`SOFTWARE\JavaSoft\JDK`, `SOFTWARE\JavaSoft\JRE`} {
			key, err := registry.OpenKey(root, sub, registry.READ)
			if err != nil {
				continue
			}

			names, err := key.ReadSubKeyNames(-1)
			key.Close()
			if err != nil {
				continue
			}
			for _, name := range names {
				subkey, err := registry.OpenKey(key, name, registry.READ)
				if err != nil {
					continue
				}
				home, _, err := subkey.GetStringValue("JavaHome")
				subkey.Close()
				if err != nil || home == "" {
					continue
				}
				homes = append(homes, home)
			}
		}
	}

	scan(registry.LOCAL_MACHINE)
	scan(registry.CURRENT_USER)

	return homes
}
