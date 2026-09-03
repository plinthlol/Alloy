// microsoft oauth device-code flow: MSA device code -> token -> xbox/mc
// exchange -> profile fetch. device code needs no redirect URI, which
// suits a TUI.

use std::sync::{Arc, LazyLock, Mutex};

use oauth2::basic::BasicClient;
use oauth2::{
    AuthUrl, ClientId, DeviceAuthorizationUrl, RefreshToken, Scope,
    StandardDeviceAuthorizationResponse, TokenResponse, TokenUrl,
};
use serde::Deserialize;

use super::accounts::{Account, AccountType, AuthResult};
use super::xbox;

const CLIENT_ID: &str = "4264a8fc-fd42-4b07-bade-22e2278fe39f";
const DEVICE_CODE_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const MSA_AUTHORIZE_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize";
const MSA_TOKEN_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const MC_PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";
const MC_TOKEN_CACHE_TTL_SECS: i64 = 20 * 60 * 60;
const MC_TOKEN_CACHE_REFRESH_MARGIN_SECS: i64 = 5 * 60;

#[derive(Debug, Clone, PartialEq)]
pub struct DeviceCodeInfo {
    pub user_code: String,
    pub verification_uri: String,
}

#[derive(Deserialize)]
struct McProfile {
    id: String,
    name: String,
}

// shared slot the TUI polls to show the device code.
pub static DEVICE_CODE_DISPLAY: LazyLock<Arc<Mutex<Option<DeviceCodeInfo>>>> =
    LazyLock::new(|| Arc::new(Mutex::new(None)));

async fn run_full_oauth_flow() -> Result<(String, Option<String>), String> {
    let oauth_client = BasicClient::new(ClientId::new(CLIENT_ID.to_owned()))
        .set_auth_uri(AuthUrl::new(MSA_AUTHORIZE_URL.to_owned()).map_err(|e| e.to_string())?)
        .set_token_uri(TokenUrl::new(MSA_TOKEN_URL.to_owned()).map_err(|e| e.to_string())?)
        .set_device_authorization_url(
            DeviceAuthorizationUrl::new(DEVICE_CODE_URL.to_owned()).map_err(|e| e.to_string())?,
        );

    let http_client = reqwest::Client::new();

    let details: StandardDeviceAuthorizationResponse = oauth_client
        .exchange_device_code()
        .add_scope(Scope::new("XboxLive.signin".to_owned()))
        .add_scope(Scope::new("offline_access".to_owned()))
        .request_async(&http_client)
        .await
        .map_err(|e| format!("Device code request failed: {e}"))?;

    if let Ok(mut slot) = DEVICE_CODE_DISPLAY.lock() {
        *slot = Some(DeviceCodeInfo {
            user_code: details.user_code().secret().to_owned(),
            verification_uri: details.verification_uri().to_string(),
        });
        crate::tui::request_redraw();
    }

    let token = oauth_client
        .exchange_device_access_token(&details)
        .request_async(&http_client, tokio::time::sleep, None)
        .await
        .map_err(|e| format!("Authentication failed: {e}"))?;

    let ms_access_token = token.access_token().secret().to_owned();
    let ms_refresh_token = token.refresh_token().map(|r| r.secret().to_owned());

    Ok((ms_access_token, ms_refresh_token))
}

// runs auth on a background task, returns a mutex to poll for the result;
// the TUI reads DEVICE_CODE_DISPLAY for the code to show.
pub fn start_microsoft_auth() -> Arc<Mutex<Option<AuthResult>>> {
    let result: Arc<Mutex<Option<AuthResult>>> = Arc::new(Mutex::new(None));
    let result_clone = result.clone();

    tokio::spawn(async move {
        let outcome = run_full_auth_flow().await;
        if let Ok(mut slot) = result_clone.lock() {
            *slot = Some(outcome);
            crate::tui::request_redraw();
        }
    });

    result
}

async fn run_full_auth_flow() -> AuthResult {
    let (ms_access_token, ms_refresh_token) = match run_full_oauth_flow().await {
        Ok(pair) => pair,
        Err(e) => return AuthResult::Error(e),
    };

    exchange_and_build_account(&ms_access_token, ms_refresh_token.as_deref()).await
}

async fn exchange_and_build_account(
    ms_access_token: &str,
    ms_refresh_token: Option<&str>,
) -> AuthResult {
    let mc_token = match xbox::exchange_microsoft_token(&reqwest::Client::new(), ms_access_token)
        .await
    {
        Ok(t) => t,
        Err(e) => return AuthResult::Error(format!("Minecraft auth failed: {e}")),
    };

    let client = reqwest::Client::new();
    let profile_resp = match client
        .get(MC_PROFILE_URL)
        .header("Authorization", format!("Bearer {}", mc_token.access_token))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return AuthResult::Error(format!("Profile fetch failed: {e}")),
    };

    if !profile_resp.status().is_success() {
        let status = profile_resp.status();
        let message = if status.as_u16() == 404 {
            "Account does not own Minecraft".to_owned()
        } else {
            format!("Profile fetch failed: profile endpoint returned {status}")
        };
        return AuthResult::Error(message);
    }

    let profile: McProfile = match profile_resp.json().await {
        Ok(p) => p,
        Err(e) => return AuthResult::Error(format!("Profile parse failed: {e}")),
    };

    // mojang returns uuids without dashes
    let uuid = if profile.id.len() == 32 {
        format!(
            "{}-{}-{}-{}-{}",
            &profile.id[..8],
            &profile.id[8..12],
            &profile.id[12..16],
            &profile.id[16..20],
            &profile.id[20..32],
        )
    } else {
        profile.id.clone()
    };

    AuthResult::Success(Account {
        uuid,
        username: profile.name,
        account_type: AccountType::Microsoft,
        active: false,
        refresh_token: ms_refresh_token.map(|s| s.to_owned()),
        cached_mc_token: None,
        cached_mc_token_expires_at: None,
    })
}

fn valid_cached_mc_token(account: &Account, now: i64) -> Option<&str> {
    let (Some(cached), Some(expires_at)) = (
        account.cached_mc_token.as_deref(),
        account.cached_mc_token_expires_at,
    ) else {
        return None;
    };

    if now < expires_at - MC_TOKEN_CACHE_REFRESH_MARGIN_SECS {
        Some(cached)
    } else {
        None
    }
}

// returns (mc_access_token, new_refresh, new_expiry). cached tokens return
// no expiry so callers skip rewriting the account store.
pub async fn refresh_and_get_token(
    account: &Account,
) -> Result<(String, Option<String>, Option<i64>), String> {
    match account.account_type {
        AccountType::Offline => Ok(("0".to_owned(), None, None)),
        AccountType::Microsoft => {
            let now = chrono::Utc::now().timestamp();
            if let Some(cached) = valid_cached_mc_token(account, now) {
                tracing::info!("Using cached Minecraft token for '{}'", account.username);
                return Ok((cached.to_owned(), None, None));
            }

            tracing::info!("Refreshing Minecraft token for '{}'", account.username);

            let refresh = account.refresh_token.as_deref().ok_or_else(|| {
                format!(
                    "No saved credentials for '{}'. Please remove and re-add the account.",
                    account.username
                )
            })?;

            let oauth_client = BasicClient::new(ClientId::new(CLIENT_ID.to_owned()))
                .set_auth_uri(
                    AuthUrl::new(MSA_AUTHORIZE_URL.to_owned()).map_err(|e| e.to_string())?,
                )
                .set_token_uri(TokenUrl::new(MSA_TOKEN_URL.to_owned()).map_err(|e| e.to_string())?);

            let http_client = reqwest::Client::new();

            let token = oauth_client
                .exchange_refresh_token(&RefreshToken::new(refresh.to_owned()))
                .add_scope(Scope::new("XboxLive.signin".to_owned()))
                .add_scope(Scope::new("offline_access".to_owned()))
                .request_async(&http_client)
                .await
                .map_err(|e| format!("Token refresh failed: {e}"))?;

            let ms_access_token = token.access_token().secret().to_owned();
            let new_refresh = token.refresh_token().map(|r| r.secret().to_owned());

            let mc_token = xbox::exchange_microsoft_token(&reqwest::Client::new(), &ms_access_token)
                .await
                .map_err(|e| format!("Minecraft auth failed: {e}"))?;

            // the hand-rolled exchange exposes no expiry either, so cache
            // conservatively and refresh before the token gets close.
            let expires_at = chrono::Utc::now().timestamp() + MC_TOKEN_CACHE_TTL_SECS;

            Ok((mc_token.access_token, new_refresh, Some(expires_at)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn microsoft_account(cached_mc_token_expires_at: Option<i64>) -> Account {
        Account {
            uuid: "00000000-0000-0000-0000-000000000000".to_owned(),
            username: "TestPlayer".to_owned(),
            account_type: AccountType::Microsoft,
            active: true,
            refresh_token: Some("refresh".to_owned()),
            cached_mc_token: Some("cached".to_owned()),
            cached_mc_token_expires_at,
        }
    }

    #[test]
    fn cached_mc_token_is_valid_before_refresh_margin() {
        let now = 1_000;
        let account = microsoft_account(Some(now + MC_TOKEN_CACHE_REFRESH_MARGIN_SECS + 1));

        assert_eq!(valid_cached_mc_token(&account, now), Some("cached"));
    }

    #[test]
    fn cached_mc_token_expires_inside_refresh_margin() {
        let now = 1_000;
        let account = microsoft_account(Some(now + MC_TOKEN_CACHE_REFRESH_MARGIN_SECS));

        assert!(valid_cached_mc_token(&account, now).is_none());
    }

    #[test]
    fn cached_mc_token_requires_expiry() {
        let account = microsoft_account(None);

        assert!(valid_cached_mc_token(&account, 1_000).is_none());
    }
}
