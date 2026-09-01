// hand-rolled minecraft-msa-auth replacement: walks a microsoft access
// token through xbox live -> xsts -> minecraft services (the standard
// third-party-launcher chain; see
// https://minecraft.wiki/w/Microsoft_authentication). pulled in-house
// because minecraft-msa-auth drags in a getset/proc-macro-error2 combo
// that trips a rustc warning — it's 3 plain reqwest calls, so the dep
// wasn't worth keeping.

use serde::{Deserialize, Serialize};

const XBL_AUTH_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS_AUTH_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MC_LOGIN_URL: &str = "https://api.minecraftservices.com/authentication/login_with_xbox";

#[derive(Serialize)]
struct XblProperties {
    #[serde(rename = "AuthMethod")]
    auth_method: &'static str,
    #[serde(rename = "SiteName")]
    site_name: &'static str,
    #[serde(rename = "RpsTicket")]
    rps_ticket: String,
}

#[derive(Serialize)]
struct XblRequest {
    #[serde(rename = "Properties")]
    properties: XblProperties,
    #[serde(rename = "RelyingParty")]
    relying_party: &'static str,
    #[serde(rename = "TokenType")]
    token_type: &'static str,
}

#[derive(Serialize)]
struct XstsProperties<'a> {
    #[serde(rename = "SandboxId")]
    sandbox_id: &'static str,
    #[serde(rename = "UserTokens")]
    user_tokens: [&'a str; 1],
}

#[derive(Serialize)]
struct XstsRequest<'a> {
    #[serde(rename = "Properties")]
    properties: XstsProperties<'a>,
    #[serde(rename = "RelyingParty")]
    relying_party: &'static str,
    #[serde(rename = "TokenType")]
    token_type: &'static str,
}

#[derive(Deserialize)]
struct XuiEntry {
    uhs: String,
}

#[derive(Deserialize)]
struct DisplayClaims {
    xui: Vec<XuiEntry>,
}

#[derive(Deserialize)]
struct XboxTokenResponse {
    #[serde(rename = "Token")]
    token: String,
    #[serde(rename = "DisplayClaims")]
    display_claims: DisplayClaims,
}

// only on xsts error bodies (401s); parsed just on the error path.
#[derive(Deserialize)]
struct XstsErrorBody {
    #[serde(rename = "XErr")]
    x_err: Option<i64>,
}

#[derive(Serialize)]
struct McLoginRequest {
    #[serde(rename = "identityToken")]
    identity_token: String,
}

#[derive(Deserialize)]
pub struct McLoginResponse {
    pub access_token: String,
}

// human-readable messages for the xsts error codes worth distinguishing
// (full list: https://minecraft.wiki/w/Microsoft_authentication#Authenticate_with_XSTS)
fn xsts_error_message(x_err: Option<i64>, status: reqwest::StatusCode) -> String {
    match x_err {
        Some(2148916233) => {
            "This Microsoft account has no Xbox Live profile. Sign in once at \
             https://www.xbox.com/live, then try again."
                .to_owned()
        }
        Some(2148916235) => "Xbox Live is not available in this account's region.".to_owned(),
        Some(2148916236) | Some(2148916237) => {
            "This account needs adult verification (South Korea).".to_owned()
        }
        Some(2148916238) => {
            "This is a child account. An adult must add it to a Microsoft family group first."
                .to_owned()
        }
        Some(code) => format!("XSTS error code {code}"),
        None => format!("HTTP {status}"),
    }
}

// exchanges a microsoft oauth access token for a minecraft access token.
pub async fn exchange_microsoft_token(
    client: &reqwest::Client,
    ms_access_token: &str,
) -> Result<McLoginResponse, String> {
    // step 1: microsoft token -> xbox live (xbl) token
    let xbl_body = XblRequest {
        properties: XblProperties {
            auth_method: "RPS",
            site_name: "user.auth.xboxlive.com",
            rps_ticket: format!("d={ms_access_token}"),
        },
        relying_party: "http://auth.xboxlive.com",
        token_type: "JWT",
    };

    let xbl_resp = client
        .post(XBL_AUTH_URL)
        .json(&xbl_body)
        .send()
        .await
        .map_err(|e| format!("Xbox Live request failed: {e}"))?;

    if !xbl_resp.status().is_success() {
        return Err(format!(
            "Xbox Live authentication failed: HTTP {}",
            xbl_resp.status()
        ));
    }

    let xbl: XboxTokenResponse = xbl_resp
        .json()
        .await
        .map_err(|e| format!("Xbox Live response parse failed: {e}"))?;

    // step 2: xbl token -> xsts token, scoped to minecraft services
    let xsts_body = XstsRequest {
        properties: XstsProperties {
            sandbox_id: "RETAIL",
            user_tokens: [xbl.token.as_str()],
        },
        relying_party: "rp://api.minecraftservices.com/",
        token_type: "JWT",
    };

    let xsts_resp = client
        .post(XSTS_AUTH_URL)
        .json(&xsts_body)
        .send()
        .await
        .map_err(|e| format!("XSTS request failed: {e}"))?;

    if !xsts_resp.status().is_success() {
        let status = xsts_resp.status();
        let x_err = xsts_resp
            .json::<XstsErrorBody>()
            .await
            .ok()
            .and_then(|b| b.x_err);
        return Err(format!(
            "Xbox Live security token request failed: {}",
            xsts_error_message(x_err, status)
        ));
    }

    let xsts: XboxTokenResponse = xsts_resp
        .json()
        .await
        .map_err(|e| format!("XSTS response parse failed: {e}"))?;

    let uhs = xsts
        .display_claims
        .xui
        .first()
        .map(|entry| entry.uhs.as_str())
        .ok_or_else(|| "XSTS response missing user hash".to_owned())?;

    // step 3: xsts token -> minecraft access token
    let identity_token = format!("XBL3.0 x={uhs};{}", xsts.token);

    let mc_resp = client
        .post(MC_LOGIN_URL)
        .json(&McLoginRequest { identity_token })
        .send()
        .await
        .map_err(|e| format!("Minecraft login request failed: {e}"))?;

    if !mc_resp.status().is_success() {
        return Err(format!("Minecraft login failed: HTTP {}", mc_resp.status()));
    }

    mc_resp
        .json()
        .await
        .map_err(|e| format!("Minecraft login response parse failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // pin the exact wire format microsoft expects (case-sensitive names,
    // specific constants) so a typo fails here, not as a cryptic 400 live.
    #[test]
    fn xbl_request_serializes_to_expected_shape() {
        let body = XblRequest {
            properties: XblProperties {
                auth_method: "RPS",
                site_name: "user.auth.xboxlive.com",
                rps_ticket: "d=some_token".to_owned(),
            },
            relying_party: "http://auth.xboxlive.com",
            token_type: "JWT",
        };

        let json: serde_json::Value = serde_json::to_value(&body).unwrap();
        assert_eq!(json["Properties"]["AuthMethod"], "RPS");
        assert_eq!(json["Properties"]["SiteName"], "user.auth.xboxlive.com");
        assert_eq!(json["Properties"]["RpsTicket"], "d=some_token");
        assert_eq!(json["RelyingParty"], "http://auth.xboxlive.com");
        assert_eq!(json["TokenType"], "JWT");
    }

    #[test]
    fn xsts_request_serializes_to_expected_shape() {
        let body = XstsRequest {
            properties: XstsProperties {
                sandbox_id: "RETAIL",
                user_tokens: ["some_xbl_token"],
            },
            relying_party: "rp://api.minecraftservices.com/",
            token_type: "JWT",
        };

        let json: serde_json::Value = serde_json::to_value(&body).unwrap();
        assert_eq!(json["Properties"]["SandboxId"], "RETAIL");
        assert_eq!(json["Properties"]["UserTokens"][0], "some_xbl_token");
        assert_eq!(json["RelyingParty"], "rp://api.minecraftservices.com/");
    }

    #[test]
    fn xbox_token_response_parses_uhs_from_display_claims() {
        let raw = r#"{
            "Token": "abc123",
            "DisplayClaims": { "xui": [{ "uhs": "deadbeef" }] }
        }"#;
        let parsed: XboxTokenResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.token, "abc123");
        assert_eq!(parsed.display_claims.xui[0].uhs, "deadbeef");
    }

    #[test]
    fn mc_login_response_parses_access_token() {
        let raw = r#"{
            "username": "some-xuid",
            "roles": [],
            "access_token": "mc_token_here",
            "token_type": "Bearer",
            "expires_in": 86400
        }"#;
        let parsed: McLoginResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.access_token, "mc_token_here");
    }

    #[test]
    fn xsts_error_message_maps_known_codes() {
        let status = reqwest::StatusCode::UNAUTHORIZED;
        assert!(xsts_error_message(Some(2148916233), status).contains("no Xbox Live profile"));
        assert!(xsts_error_message(Some(2148916238), status).contains("child account"));
        assert!(xsts_error_message(Some(999999), status).contains("999999"));
        assert!(xsts_error_message(None, status).contains("401"));
    }
}
