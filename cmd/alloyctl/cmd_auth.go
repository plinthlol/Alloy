package main

import (
	"fmt"

	"alloy/internal/auth"
	"alloy/internal/cli"
	"alloy/internal/config"
)

func accountStore(paths config.Paths) *auth.AccountStore {
	return auth.NewAccountStore(paths.ConfigDir)
}

func cmdAuth(paths config.Paths, args []string) error {
	if len(args) == 0 {
		return fmt.Errorf("usage: alloyctl auth online | offline <username> | list | switch <username> | remove <username>")
	}

	switch args[0] {
	case "offline":
		if len(args) < 2 {
			return fmt.Errorf("usage: alloyctl auth offline <username>")
		}
		return authOffline(paths, args[1])
	case "online":
		return authOnline(paths)
	case "list":
		return authList(paths)
	case "switch":
		if len(args) < 2 {
			return fmt.Errorf("usage: alloyctl auth switch <username>")
		}
		return authSwitch(paths, args[1])
	case "remove":
		if len(args) < 2 {
			return fmt.Errorf("usage: alloyctl auth remove <username>")
		}
		return authRemove(paths, args[1])
	default:
		return fmt.Errorf("unknown auth subcommand %q", args[0])
	}
}

func authOffline(paths config.Paths, username string) error {
	profile := auth.NewOfflineProfile(username)

	store := accountStore(paths)
	if err := store.Upsert(auth.Account{
		UUID:        profile.UUID,
		Username:    profile.Username,
		AccountType: auth.AccountTypeOffline,
	}, true); err != nil {
		return err
	}

	cli.Info(fmt.Sprintf("Logged in as %s", profile.Username))
	return nil
}

func authOnline(paths config.Paths) error {
	dc, err := auth.StartDeviceCode()
	if err != nil {
		return fmt.Errorf("starting Microsoft device code flow: %w", err)
	}

	fmt.Println()
	fmt.Println("  \033[1mOpen:\033[0m", dc.VerificationURI)
	fmt.Println("  \033[1mCode:\033[0m", dc.UserCode)
	fmt.Println()
	cli.Info("Waiting for you to finish signing in...")

	tok, err := auth.PollDeviceCode(dc)
	if err != nil {
		return fmt.Errorf("microsoft sign-in failed: %w", err)
	}

	profile, err := auth.CompleteAuthChain(tok.AccessToken, tok.RefreshToken, tok.ExpiresIn)
	if err != nil {
		return fmt.Errorf("completing Xbox/Minecraft auth chain: %w", err)
	}

	expiresAt := profile.AccessExpiresAt.Unix()
	store := accountStore(paths)
	if err := store.Upsert(auth.Account{
		UUID:                   profile.UUID,
		Username:               profile.Username,
		AccountType:            auth.AccountTypeMicrosoft,
		RefreshToken:           profile.RefreshToken,
		CachedMCToken:          profile.MinecraftToken,
		CachedMCTokenExpiresAt: &expiresAt,
	}, true); err != nil {
		return err
	}

	cli.Info(fmt.Sprintf("Logged in as %s", profile.Username))
	return nil
}

func authList(paths config.Paths) error {
	accounts, err := accountStore(paths).List()
	if err != nil {
		return err
	}

	if len(accounts) == 0 {
		cli.Info("No accounts stored. Run `alloyctl auth online` or `alloyctl auth offline <name>` to add one.")
		return nil
	}

	fmt.Println()
	for _, a := range accounts {
		marker := "  "
		if a.Active {
			marker = "\033[32m*\033[0m "
		}
		typeLabel := a.AccountType
		if typeLabel == auth.AccountTypeMicrosoft {
			typeLabel = "msa"
		} else {
			typeLabel = "offline"
		}
		fmt.Printf("  %s%s \033[2m(%s)\033[0m\n", marker, a.Username, typeLabel)
	}
	fmt.Println()
	return nil
}

func authSwitch(paths config.Paths, username string) error {
	if err := accountStore(paths).SetActive(username); err != nil {
		return err
	}
	cli.Info(fmt.Sprintf("Switched to %s", username))
	return nil
}

func authRemove(paths config.Paths, username string) error {
	if err := accountStore(paths).Remove(username); err != nil {
		return err
	}
	cli.Info(fmt.Sprintf("Removed %s", username))
	return nil
}
