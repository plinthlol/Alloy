package auth

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sync"
)

type Account struct {
	UUID                   string `json:"uuid"`
	Username               string `json:"username"`
	AccountType            string `json:"account_type"`
	Active                 bool   `json:"active"`
	RefreshToken           string `json:"refresh_token,omitempty"`
	CachedMCToken          string `json:"cached_mc_token,omitempty"`
	CachedMCTokenExpiresAt *int64 `json:"cached_mc_token_expires_at,omitempty"`
}

const (
	AccountTypeMicrosoft = "Microsoft"
	AccountTypeOffline   = "Offline"
)

type AccountStore struct {
	path     string
	mu       sync.Mutex
	accounts []Account
}

func NewAccountStore(configDir string) *AccountStore {
	return &AccountStore{path: filepath.Join(configDir, "accounts.json")}
}

func (s *AccountStore) load() error {
	data, err := os.ReadFile(s.path)
	if os.IsNotExist(err) {
		s.accounts = nil
		return nil
	}
	if err != nil {
		return fmt.Errorf("reading accounts file: %w", err)
	}
	var accounts []Account
	if err := json.Unmarshal(data, &accounts); err != nil {
		return fmt.Errorf("parsing accounts file: %w", err)
	}
	s.accounts = accounts
	return nil
}

func (s *AccountStore) save() error {
	if err := os.MkdirAll(filepath.Dir(s.path), 0o755); err != nil {
		return fmt.Errorf("creating config directory: %w", err)
	}
	data, err := json.MarshalIndent(s.accounts, "", "  ")
	if err != nil {
		return fmt.Errorf("marshaling accounts: %w", err)
	}
	return os.WriteFile(s.path, data, 0o600)
}

func (s *AccountStore) List() ([]Account, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if err := s.load(); err != nil {
		return nil, err
	}
	return s.accounts, nil
}

func (s *AccountStore) Active() (Account, bool, error) {
	accounts, err := s.List()
	if err != nil {
		return Account{}, false, err
	}
	for _, a := range accounts {
		if a.Active {
			return a, true, nil
		}
	}
	return Account{}, false, nil
}

func (s *AccountStore) Find(username string) (Account, bool, error) {
	accounts, err := s.List()
	if err != nil {
		return Account{}, false, err
	}
	for _, a := range accounts {
		if a.Username == username {
			return a, true, nil
		}
	}
	return Account{}, false, nil
}

func (s *AccountStore) Upsert(a Account, makeActive bool) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if err := s.load(); err != nil {
		return err
	}

	filtered := s.accounts[:0:0]
	for _, existing := range s.accounts {
		if existing.UUID != a.UUID {
			filtered = append(filtered, existing)
		}
	}
	wasEmpty := len(filtered) == 0
	if wasEmpty {
		makeActive = true
	}
	if makeActive {
		for i := range filtered {
			filtered[i].Active = false
		}
		a.Active = true
	}
	s.accounts = append(filtered, a)
	return s.save()
}

func (s *AccountStore) SetActive(username string) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if err := s.load(); err != nil {
		return err
	}
	found := false
	for i := range s.accounts {
		if s.accounts[i].Username == username {
			s.accounts[i].Active = true
			found = true
		} else {
			s.accounts[i].Active = false
		}
	}
	if !found {
		return fmt.Errorf("no account named %q", username)
	}
	return s.save()
}

func (s *AccountStore) Remove(username string) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if err := s.load(); err != nil {
		return err
	}
	idx := -1
	for i, a := range s.accounts {
		if a.Username == username {
			idx = i
			break
		}
	}
	if idx == -1 {
		return fmt.Errorf("no account named %q", username)
	}
	wasActive := s.accounts[idx].Active
	s.accounts = append(s.accounts[:idx], s.accounts[idx+1:]...)
	if wasActive && len(s.accounts) > 0 {
		s.accounts[0].Active = true
	}
	return s.save()
}

func (s *AccountStore) UpdateTokens(uuid, refreshToken, cachedMCToken string, expiresAt *int64) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if err := s.load(); err != nil {
		return err
	}
	found := false
	for i := range s.accounts {
		if s.accounts[i].UUID != uuid {
			continue
		}
		found = true
		if refreshToken != "" {
			s.accounts[i].RefreshToken = refreshToken
		}
		if cachedMCToken != "" {
			s.accounts[i].CachedMCToken = cachedMCToken
		}
		if expiresAt != nil {
			s.accounts[i].CachedMCTokenExpiresAt = expiresAt
		}
	}
	if !found {
		return fmt.Errorf("no account with uuid %q", uuid)
	}
	return s.save()
}
