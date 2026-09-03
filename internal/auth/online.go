package auth

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"
	"time"
)

const ClientID = "4264a8fc-fd42-4b07-bade-22e2278fe39f"

const (
	deviceCodeURL = "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode"
	tokenURL      = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token"
	xblAuthURL    = "https://user.auth.xboxlive.com/user/authenticate"
	xstsAuthURL   = "https://xsts.auth.xboxlive.com/xsts/authorize"
	mcLoginURL    = "https://api.minecraftservices.com/authentication/login_with_xbox"
	mcEntitlement = "https://api.minecraftservices.com/entitlements/mcstore"
	mcProfileURL  = "https://api.minecraftservices.com/minecraft/profile"

	scope = "XboxLive.signin offline_access"
)

type OnlineProfile struct {
	Username        string
	UUID            string
	MinecraftToken  string
	RefreshToken    string
	AccessExpiresAt time.Time
}

type DeviceCodeResponse struct {
	DeviceCode      string `json:"device_code"`
	UserCode        string `json:"user_code"`
	VerificationURI string `json:"verification_uri"`
	ExpiresIn       int    `json:"expires_in"`
	Interval        int    `json:"interval"`
	Message         string `json:"message"`
}

func StartDeviceCode() (DeviceCodeResponse, error) {
	form := url.Values{
		"client_id": {ClientID},
		"scope":     {scope},
	}
	body, err := postForm(deviceCodeURL, form)
	if err != nil {
		return DeviceCodeResponse{}, err
	}
	var dc DeviceCodeResponse
	if err := json.Unmarshal(body, &dc); err != nil {
		return DeviceCodeResponse{}, fmt.Errorf("parsing device code response: %w", err)
	}
	return dc, nil
}

type msTokenResponse struct {
	AccessToken  string `json:"access_token"`
	RefreshToken string `json:"refresh_token"`
	ExpiresIn    int    `json:"expires_in"`
	Error        string `json:"error"`
}

func PollDeviceCode(dc DeviceCodeResponse) (msTokenResponse, error) {
	interval := time.Duration(dc.Interval) * time.Second
	if interval <= 0 {
		interval = 5 * time.Second
	}
	deadline := time.Now().Add(time.Duration(dc.ExpiresIn) * time.Second)

	for time.Now().Before(deadline) {
		time.Sleep(interval)

		form := url.Values{
			"client_id":   {ClientID},
			"grant_type":  {"urn:ietf:params:oauth:grant-type:device_code"},
			"device_code": {dc.DeviceCode},
		}
		body, err := postForm(tokenURL, form)
		if err != nil {
			return msTokenResponse{}, err
		}
		var t msTokenResponse
		if err := json.Unmarshal(body, &t); err != nil {
			return msTokenResponse{}, fmt.Errorf("parsing token response: %w", err)
		}
		switch t.Error {
		case "":
			return t, nil
		case "authorization_pending":
			continue
		case "slow_down":
			interval += 5 * time.Second
			continue
		default:
			return msTokenResponse{}, fmt.Errorf("device code auth failed: %s", t.Error)
		}
	}
	return msTokenResponse{}, fmt.Errorf("device code expired before the user authorized")
}

func RefreshMicrosoftToken(refreshToken string) (msTokenResponse, error) {
	form := url.Values{
		"client_id":     {ClientID},
		"grant_type":    {"refresh_token"},
		"refresh_token": {refreshToken},
		"scope":         {scope},
	}
	body, err := postForm(tokenURL, form)
	if err != nil {
		return msTokenResponse{}, err
	}
	var t msTokenResponse
	if err := json.Unmarshal(body, &t); err != nil {
		return msTokenResponse{}, err
	}
	if t.Error != "" {
		return msTokenResponse{}, fmt.Errorf("refreshing token: %s", t.Error)
	}
	return t, nil
}

type xblResponse struct {
	Token   string `json:"Token"`
	Display struct {
		Claims []struct {
			UHS string `json:"uhs"`
		} `json:"xui"`
	} `json:"DisplayClaims"`
}

func xblAuthenticate(msAccessToken string) (xblResponse, error) {
	payload := map[string]any{
		"Properties": map[string]any{
			"AuthMethod": "RPS",
			"SiteName":   "user.auth.xboxlive.com",
			"RpsTicket":  "d=" + msAccessToken,
		},
		"RelyingParty": "http://auth.xboxlive.com",
		"TokenType":    "JWT",
	}
	body, err := postJSON(xblAuthURL, payload, nil)
	if err != nil {
		return xblResponse{}, fmt.Errorf("xbox live auth: %w", err)
	}
	var r xblResponse
	if err := json.Unmarshal(body, &r); err != nil {
		return xblResponse{}, err
	}
	return r, nil
}

func xstsAuthorize(xblToken string) (xblResponse, error) {
	payload := map[string]any{
		"Properties": map[string]any{
			"SandboxId":  "RETAIL",
			"UserTokens": []string{xblToken},
		},
		"RelyingParty": "rp://api.minecraftservices.com/",
		"TokenType":    "JWT",
	}
	body, err := postJSON(xstsAuthURL, payload, nil)

	var xstsErr struct {
		Identity string `json:"Identity"`
		XErr     int    `json:"XErr"`
		Message  string `json:"Message"`
	}
	if json.Unmarshal(body, &xstsErr) == nil && xstsErr.XErr != 0 {
		return xblResponse{}, xstsAuthError(xstsErr.XErr)
	}

	if err != nil {
		return xblResponse{}, fmt.Errorf("xsts authorize: %w", err)
	}
	var r xblResponse
	if err := json.Unmarshal(body, &r); err != nil {
		return xblResponse{}, err
	}
	return r, nil
}

func xstsAuthError(xerr int) error {
	switch xerr {
	case 2148916233:
		return fmt.Errorf("\nYour Microsoft account doesn't have an Xbox account set up.\n\n" +
			"  1. Go to https://www.xbox.com and sign in\n" +
			"  2. Create an Xbox account (choose a gamertag)\n" +
			"  3. Make sure you own Minecraft: Java Edition\n\n" +
			"Then run 'alloyctl auth online' again.")
	case 2148916235:
		return fmt.Errorf("\nThis account is a child account and needs parental consent.\n" +
			"A parent/guardian needs to approve access at account.microsoft.com.")
	case 2148916236, 2148916237:
		return fmt.Errorf("\nXbox Live is not available in your region.\n" +
			"Check https://www.xbox.com/available-regions for details.")
	case 2148916238:
		return fmt.Errorf("\nThis account has been suspended from Xbox Live.\n" +
			"Check your account status at https://support.xbox.com.")
	default:
		return fmt.Errorf("Xbox Live error (code %d). Check your account at https://account.xbox.com", xerr)
	}
}

type mcLoginResponse struct {
	AccessToken string `json:"access_token"`
	ExpiresIn   int    `json:"expires_in"`
}

func mcLoginWithXbox(uhs, xstsToken string) (mcLoginResponse, error) {
	payload := map[string]any{
		"identityToken": fmt.Sprintf("XBL3.0 x=%s;%s", uhs, xstsToken),
	}
	body, err := postJSON(mcLoginURL, payload, nil)
	if err != nil {
		return mcLoginResponse{}, fmt.Errorf("minecraft services login: %w", err)
	}
	var r mcLoginResponse
	if err := json.Unmarshal(body, &r); err != nil {
		return mcLoginResponse{}, err
	}
	return r, nil
}

type entitlementsResponse struct {
	Items []struct {
		Name string `json:"name"`
	} `json:"items"`
}

func checkOwnership(mcToken string) (bool, error) {
	headers := map[string]string{"Authorization": "Bearer " + mcToken}
	body, err := getWithHeaders(mcEntitlement, headers)
	if err != nil {
		return false, err
	}
	var r entitlementsResponse
	if err := json.Unmarshal(body, &r); err != nil {
		return false, err
	}
	return len(r.Items) > 0, nil
}

type mcProfileResponse struct {
	ID   string `json:"id"`
	Name string `json:"name"`
}

func fetchProfile(mcToken string) (mcProfileResponse, error) {
	headers := map[string]string{"Authorization": "Bearer " + mcToken}
	body, err := getWithHeaders(mcProfileURL, headers)
	if err != nil {
		return mcProfileResponse{}, err
	}
	var r mcProfileResponse
	if err := json.Unmarshal(body, &r); err != nil {
		return mcProfileResponse{}, err
	}
	if r.ID == "" {
		return mcProfileResponse{}, fmt.Errorf("empty profile response — account may not own Minecraft, or auth chain failed upstream")
	}
	return r, nil
}

func CompleteAuthChain(msAccessToken, refreshToken string, expiresIn int) (OnlineProfile, error) {
	xbl, err := xblAuthenticate(msAccessToken)
	if err != nil {
		return OnlineProfile{}, err
	}
	if len(xbl.Display.Claims) == 0 {
		return OnlineProfile{}, fmt.Errorf("xbox live auth: no user hash in response")
	}
	uhs := xbl.Display.Claims[0].UHS

	xsts, err := xstsAuthorize(xbl.Token)
	if err != nil {
		return OnlineProfile{}, err
	}

	mcLogin, err := mcLoginWithXbox(uhs, xsts.Token)
	if err != nil {
		return OnlineProfile{}, err
	}

	owns, err := checkOwnership(mcLogin.AccessToken)
	if err != nil {
		return OnlineProfile{}, err
	}
	if !owns {
		return OnlineProfile{}, fmt.Errorf("this Microsoft account does not own Minecraft: Java Edition")
	}

	profile, err := fetchProfile(mcLogin.AccessToken)
	if err != nil {
		return OnlineProfile{}, err
	}

	return OnlineProfile{
		Username:        profile.Name,
		UUID:            dashUUID(profile.ID),
		MinecraftToken:  mcLogin.AccessToken,
		RefreshToken:    refreshToken,
		AccessExpiresAt: time.Now().Add(time.Duration(expiresIn) * time.Second),
	}, nil
}

func dashUUID(undashed string) string {
	if len(undashed) != 32 {
		return undashed
	}
	return strings.Join([]string{
		undashed[0:8], undashed[8:12], undashed[12:16], undashed[16:20], undashed[20:32],
	}, "-")
}

func postForm(u string, form url.Values) ([]byte, error) {
	resp, err := http.PostForm(u, form)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, err
	}
	if resp.StatusCode >= 300 {
		return body, nil
	}
	return body, nil
}

func postJSON(u string, payload any, headers map[string]string) ([]byte, error) {
	data, err := json.Marshal(payload)
	if err != nil {
		return nil, err
	}
	req, err := http.NewRequest(http.MethodPost, u, strings.NewReader(string(data)))
	if err != nil {
		return nil, err
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Accept", "application/json")
	for k, v := range headers {
		req.Header.Set(k, v)
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, err
	}
	if resp.StatusCode >= 300 {
		return nil, fmt.Errorf("HTTP %d: %s", resp.StatusCode, string(body))
	}
	return body, nil
}

func getWithHeaders(u string, headers map[string]string) ([]byte, error) {
	req, err := http.NewRequest(http.MethodGet, u, nil)
	if err != nil {
		return nil, err
	}
	for k, v := range headers {
		req.Header.Set(k, v)
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, err
	}
	if resp.StatusCode >= 300 {
		return nil, fmt.Errorf("HTTP %d: %s", resp.StatusCode, string(body))
	}
	return body, nil
}
