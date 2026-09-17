//! Read the selected Codex home once without mutating shared credentials.

use std::{
    ffi::OsString,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use base64::{
    engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD},
    Engine,
};
use chrono::{DateTime, Utc};
use serde_json::Value;

const MAX_AUTH_BYTES: u64 = 256 * 1024;

// Deliberately no Debug implementation: these types hold bearer credentials.
#[derive(Default)]
pub(super) struct Credentials {
    pub pat: Option<String>,
    pub oauth: Option<OAuthCredential>,
    pub is_api_key: bool,
}

pub(super) struct OAuthCredential {
    pub access_token: String,
    pub account_id: Option<String>,
    pub email: Option<String>,
    pub plan: Option<String>,
    expires_at: Option<i64>,
    last_refresh: Option<i64>,
}

impl OAuthCredential {
    pub fn needs_refresh(&self, now: DateTime<Utc>) -> bool {
        if let Some(expiry) = self.expires_at {
            return expiry.saturating_sub(now.timestamp()) <= 5 * 60;
        }
        self.last_refresh
            .is_none_or(|last| now.timestamp().saturating_sub(last) > 8 * 24 * 60 * 60)
    }
}

#[derive(Debug, PartialEq)]
pub(super) enum CredentialError {
    Missing,
    Unreadable,
    Invalid,
}

pub(super) fn resolve_home(configured: Option<OsString>, home: Option<PathBuf>) -> Option<PathBuf> {
    // Explicit homes never fall through to ~/.codex, even when their auth is absent.
    let configured = configured.filter(|value| !value.to_string_lossy().trim().is_empty());
    let path = crate::user_paths::config_dir(configured, home, ".codex")?;
    // The CLI starts in the user profile rather than AgentBar's launch directory. Resolve
    // relative CODEX_HOME before handing it to both readers so scope cannot drift.
    Some(if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(&path))
            .unwrap_or(path)
    })
}

pub(super) fn load(home: Option<&Path>) -> Result<Credentials, CredentialError> {
    let path = home.ok_or(CredentialError::Missing)?.join("auth.json");
    let file = File::open(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => CredentialError::Missing,
        _ => CredentialError::Unreadable,
    })?;
    let mut bytes = Vec::new();
    file.take(MAX_AUTH_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| CredentialError::Unreadable)?;
    if bytes.len() as u64 > MAX_AUTH_BYTES {
        return Err(CredentialError::Invalid);
    }
    parse(&bytes)
}

pub(super) fn parse(bytes: &[u8]) -> Result<Credentials, CredentialError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| CredentialError::Invalid)?;
    if !value.is_object() {
        return Err(CredentialError::Invalid);
    }
    let pat = nonempty(value.get("personal_access_token"))
        .or_else(|| nonempty(value.get("personalAccessToken")));
    let is_api_key = nonempty(value.get("OPENAI_API_KEY")).is_some();
    let oauth = value.get("tokens").and_then(|tokens| {
        let access_token = field(tokens, "access_token", "accessToken")?;
        let id_claims = field(tokens, "id_token", "idToken").and_then(|token| claims(&token));
        let access_claims = claims(&access_token);
        let account_id = field(tokens, "account_id", "accountId")
            .or_else(|| id_claims.as_ref().and_then(account_id_from_claims))
            .or_else(|| access_claims.as_ref().and_then(account_id_from_claims));
        let email = id_claims.as_ref().and_then(|claims| {
            nonempty(claims.get("email"))
                .or_else(|| nonempty(claims.get("https://api.openai.com/profile")?.get("email")))
        });
        let plan = id_claims.as_ref().and_then(|claims| {
            nonempty(
                claims
                    .get("https://api.openai.com/auth")?
                    .get("chatgpt_plan_type"),
            )
        });
        let expires_at = access_claims
            .as_ref()
            .and_then(|claims| claims.get("exp"))
            .and_then(Value::as_i64)
            .filter(|value| (0..=253_402_300_799).contains(value));
        let last_refresh = value.get("last_refresh").and_then(|value| {
            value
                .as_str()
                .and_then(|date| DateTime::parse_from_rfc3339(date).ok())
                .map(|date| date.timestamp())
                .or_else(|| value.as_i64())
        });
        Some(OAuthCredential {
            access_token,
            account_id,
            email,
            plan,
            expires_at,
            last_refresh,
        })
    });
    Ok(Credentials {
        pat,
        oauth,
        is_api_key,
    })
}

fn field(value: &Value, snake: &str, camel: &str) -> Option<String> {
    nonempty(value.get(snake)).or_else(|| nonempty(value.get(camel)))
}

pub(super) fn nonempty(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn claims(token: &str) -> Option<Value> {
    // Claims are unverified display/refresh hints, never authentication decisions;
    // the server still authenticates the original bearer token.
    let parts: Vec<_> = token.split('.').collect();
    if parts.len() != 3 || parts.iter().any(|part| part.is_empty()) {
        return None;
    }
    let decoded = URL_SAFE_NO_PAD
        .decode(parts[1])
        .or_else(|_| URL_SAFE.decode(parts[1]))
        .ok()?;
    serde_json::from_slice(&decoded).ok()
}

fn account_id_from_claims(claims: &Value) -> Option<String> {
    nonempty(claims.get("chatgpt_account_id"))
        .or_else(|| {
            nonempty(
                claims
                    .get("https://api.openai.com/auth")?
                    .get("chatgpt_account_id"),
            )
        })
        .or_else(|| {
            claims
                .get("organizations")?
                .as_array()?
                .iter()
                .find_map(|org| nonempty(org.get("id")))
        })
}

pub(super) fn uses_custom_backend(home: Option<&Path>) -> bool {
    let Some(home) = home else {
        return false;
    };
    let file = match File::open(home.join("config.toml")) {
        Ok(file) => file,
        Err(error) => return error.kind() != std::io::ErrorKind::NotFound,
    };
    let mut config = String::new();
    if file
        .take(MAX_AUTH_BYTES + 1)
        .read_to_string(&mut config)
        .is_err()
        || config.len() as u64 > MAX_AUTH_BYTES
    {
        return true;
    }
    config.lines().any(|line| {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some((key, value)) = line.split_once('=') else {
            return false;
        };
        if key.trim().trim_matches(['\'', '"']) != "chatgpt_base_url" {
            return false;
        }
        let value = value.trim().trim_matches(['\'', '"']).trim_end_matches('/');
        !matches!(
            value,
            "https://chatgpt.com" | "https://chatgpt.com/backend-api"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    pub(super) fn jwt(value: Value) -> String {
        format!(
            "e30.{}.signature",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&value).unwrap())
        )
    }

    #[test]
    fn parses_snake_and_camel_native_auth_without_retaining_refresh_token() {
        let id = jwt(
            json!({"email":" user@example.test ","https://api.openai.com/auth":{"chatgpt_account_id":"jwt-account","chatgpt_plan_type":"pro"}}),
        );
        for tokens in [
            json!({"access_token":jwt(json!({"exp":2000000000})),"id_token":id,"account_id":" explicit "}),
            json!({"accessToken":jwt(json!({"exp":2000000000})),"idToken":id,"accountId":" explicit "}),
        ] {
            let auth = parse(&serde_json::to_vec(&json!({"tokens":tokens})).unwrap()).unwrap();
            let oauth = auth.oauth.unwrap();
            assert_eq!(oauth.account_id.as_deref(), Some("explicit"));
            assert_eq!(oauth.email.as_deref(), Some("user@example.test"));
            assert_eq!(oauth.plan.as_deref(), Some("pro"));
            assert!(!oauth.needs_refresh(DateTime::from_timestamp(1900000000, 0).unwrap()));
        }
    }

    #[test]
    fn claims_recover_account_and_expiry_takes_precedence_over_refresh_age() {
        let now = DateTime::from_timestamp(1900000000, 0).unwrap();
        let auth = parse(&serde_json::to_vec(&json!({"tokens":{"access_token":jwt(json!({"exp":1900000300,"chatgpt_account_id":"claim-account"}))},"last_refresh":now.to_rfc3339()})).unwrap()).unwrap();
        let oauth = auth.oauth.unwrap();
        assert_eq!(oauth.account_id.as_deref(), Some("claim-account"));
        assert!(oauth.needs_refresh(now));
        let opaque =
            parse(br#"{"tokens":{"access_token":"opaque"},"last_refresh":"2030-03-17T17:46:40Z"}"#)
                .unwrap();
        assert!(!opaque.oauth.unwrap().needs_refresh(now));
        let missing = parse(br#"{"tokens":{"access_token":"opaque"}}"#).unwrap();
        assert!(missing.oauth.unwrap().needs_refresh(now));
    }

    #[test]
    fn explicit_home_is_isolated_and_auth_is_read_only() {
        let directory = tempfile::tempdir().unwrap();
        let ambient = directory.path().join("ambient");
        std::fs::create_dir_all(ambient.join(".codex")).unwrap();
        std::fs::write(
            ambient.join(".codex/auth.json"),
            br#"{"personal_access_token":"ambient-secret"}"#,
        )
        .unwrap();
        let scoped = directory.path().join("scoped");
        std::fs::create_dir(&scoped).unwrap();
        let home = resolve_home(Some(scoped.clone().into_os_string()), Some(ambient)).unwrap();
        assert!(matches!(load(Some(&home)), Err(CredentialError::Missing)));
        let contents = br#"{"personalAccessToken":"scoped-secret"}"#;
        std::fs::write(scoped.join("auth.json"), contents).unwrap();
        let auth = load(Some(&home)).unwrap();
        assert_eq!(auth.pat.as_deref(), Some("scoped-secret"));
        assert_eq!(std::fs::read(scoped.join("auth.json")).unwrap(), contents);
        assert!(matches!(
            parse(b"secret malformed credentials"),
            Err(CredentialError::Invalid)
        ));
        let relative = resolve_home(
            Some("scoped-relative".into()),
            Some("/unrelated/home".into()),
        )
        .unwrap();
        assert_eq!(
            relative,
            std::env::current_dir().unwrap().join("scoped-relative")
        );
    }

    #[test]
    fn custom_backend_detection_preserves_cli_configuration() {
        let directory = tempfile::tempdir().unwrap();
        assert!(!uses_custom_backend(Some(directory.path())));
        for (config, expected) in [
            (
                "chatgpt_base_url = 'https://chatgpt.com/backend-api/'",
                false,
            ),
            ("chatgpt_base_url = 'https://example.test/api'", true),
            ("# chatgpt_base_url = 'https://example.test'", false),
        ] {
            std::fs::write(directory.path().join("config.toml"), config).unwrap();
            assert_eq!(uses_custom_backend(Some(directory.path())), expected);
        }
    }
}
