//! Optional sign-in (PRD 3.1). Everything works signed out; sign-in unlocks subscriptions,
//! history and playlists.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rustypipe::error::{AuthError, Error as RpError};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::api::Api;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum SignInMethod {
    Device,
    Cookies,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum AuthEvent {
    /// Show this URL and code exactly as the server sent them.
    DeviceCode {
        verification_url: String,
        user_code: String,
        expires_in_s: u32,
    },
    SignedIn(SignInMethod),
    SignedOut,
    /// The user declined on the other device; back to the sign-in screen.
    Denied,
    Failed(String),
}

/// Default OAuth access-token lifetime used by YouTube's TV client.
const TOKEN_LIFETIME: Duration = Duration::from_secs(3600);

/// Tracks the signed-in method and refreshes the OAuth token behind a single-flight lock,
/// so parallel requests that hit a 401 trigger one refresh.
pub struct Auth {
    api: Arc<Api>,
    state: Mutex<AuthState>,
}

struct AuthState {
    method: Option<SignInMethod>,
    refreshed_at: Option<Instant>,
}

impl Auth {
    pub fn new(api: Arc<Api>) -> Self {
        Self {
            api,
            state: Mutex::new(AuthState {
                method: None,
                refreshed_at: None,
            }),
        }
    }

    pub async fn method(&self) -> Option<SignInMethod> {
        self.state.lock().await.method
    }

    /// Validate the stored session on launch. Any failure drops to signed-out mode.
    pub async fn startup_check(&self) -> Option<SignInMethod> {
        let rp = self.api.rp();
        let method = if rp.user_auth_check_login().await.is_ok() {
            Some(SignInMethod::Device)
        } else if rp.user_auth_check_cookie().await.is_ok() {
            Some(SignInMethod::Cookies)
        } else {
            None
        };
        let mut st = self.state.lock().await;
        st.method = method;
        st.refreshed_at = (method == Some(SignInMethod::Device)).then(Instant::now);
        method
    }

    /// Run the device flow until signed in, denied or failed. Cancel by aborting the task.
    pub async fn device_flow(&self, emit: impl Fn(AuthEvent)) -> Result<(), String> {
        let rp = self.api.rp();
        loop {
            let code = rp.user_auth_get_code().await.map_err(|e| e.to_string())?;
            emit(AuthEvent::DeviceCode {
                verification_url: code.verification_url.clone(),
                user_code: code.user_code.clone(),
                expires_in_s: code.expires_in,
            });
            let deadline = Instant::now() + Duration::from_secs(code.expires_in.into());
            let mut interval = Duration::from_secs(code.interval.max(1).into());
            loop {
                tokio::time::sleep(interval).await;
                match rp.user_auth_login(&code).await {
                    Ok(true) => {
                        let mut st = self.state.lock().await;
                        st.method = Some(SignInMethod::Device);
                        st.refreshed_at = Some(Instant::now());
                        emit(AuthEvent::SignedIn(SignInMethod::Device));
                        return Ok(());
                    }
                    Ok(false) => {}
                    Err(RpError::Auth(AuthError::DeviceCodeExpired)) => break,
                    Err(RpError::Auth(AuthError::Other(msg))) if msg.starts_with("slow_down") => {
                        interval += Duration::from_secs(5);
                    }
                    Err(RpError::Auth(AuthError::Other(msg)))
                        if msg.starts_with("access_denied") =>
                    {
                        emit(AuthEvent::Denied);
                        return Err("access denied".into());
                    }
                    Err(e) => return Err(e.to_string()),
                }
                if Instant::now() >= deadline {
                    break; // expired_token: request a new code
                }
            }
        }
    }

    /// Desktop fallback: a Netscape `cookies.txt` export.
    pub async fn import_cookies(&self, cookies_txt: &str) -> Result<(), String> {
        let rp = self.api.rp();
        rp.user_auth_set_cookie_txt(cookies_txt)
            .await
            .map_err(|e| e.to_string())?;
        rp.user_auth_check_cookie().await.map_err(|e| {
            format!("cookies were read but YouTube did not accept them: {e}")
        })?;
        self.state.lock().await.method = Some(SignInMethod::Cookies);
        tracing::info!("signed in with YouTube cookies");
        Ok(())
    }

    pub async fn logout(&self) {
        let rp = self.api.rp();
        let mut st = self.state.lock().await;
        match st.method {
            Some(SignInMethod::Device) => {
                let _ = rp.user_auth_logout().await;
            }
            Some(SignInMethod::Cookies) => {
                let _ = rp.user_auth_remove_cookie().await;
            }
            None => {}
        }
        st.method = None;
        st.refreshed_at = None;
    }

    /// Refresh when 80% of the token lifetime has passed, or when `force` is set after a 401.
    /// Concurrent callers wait on the lock; whoever arrives second sees a fresh timestamp and
    /// skips the network call.
    pub async fn refresh_if_needed(&self, force: bool) -> Result<(), String> {
        let mut st = self.state.lock().await;
        if st.method != Some(SignInMethod::Device) {
            return Ok(());
        }
        let due = st
            .refreshed_at
            .is_none_or(|t| t.elapsed() >= TOKEN_LIFETIME.mul_f32(0.8));
        let recent = st
            .refreshed_at
            .is_some_and(|t| t.elapsed() < Duration::from_secs(10));
        if !(due || (force && !recent)) {
            return Ok(());
        }
        match self.api.rp().user_auth_check_login().await {
            Ok(()) => {
                st.refreshed_at = Some(Instant::now());
                Ok(())
            }
            Err(e) => {
                tracing::warn!("token refresh failed, signing out: {e}");
                st.method = None;
                st.refreshed_at = None;
                Err(e.to_string())
            }
        }
    }
}
