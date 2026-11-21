use serde::{Deserialize, Serialize};
use std::fmt;
use zeroize::Zeroize;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Pair {
    pub enabled: bool,
    pub name: String,
    pub value: String,
}
impl Default for Pair {
    fn default() -> Self {
        Self {
            enabled: true,
            name: String::new(),
            value: String::new(),
        }
    }
}

macro_rules! choices {
    ($name:ident { $default:ident => $label:literal $(, $variant:ident => $text:literal)* $(,)? }) => {
        #[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        pub enum $name { #[default] $default, $($variant),* }
        impl $name { pub const ALL: &'static [Self] = &[Self::$default, $(Self::$variant),*]; }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(match self { Self::$default => $label, $(Self::$variant => $text),* })
            }
        }
    };
}
choices!(AuthKind { None => "No auth", Basic => "Basic Auth", Bearer => "Bearer Token", ApiKey => "API Key", OAuth2 => "OAuth 2.0" });
choices!(KeyLocation { Header => "Header", Query => "Query param" });
choices!(OAuthFlow { AccessToken => "Access token", ClientCredentials => "Client Credentials" });
choices!(BodyKind { None => "None", Json => "JSON", Text => "Raw text", Form => "x-www-form-urlencoded", Multipart => "multipart/form-data" });

#[derive(Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Auth {
    pub kind: AuthKind,
    pub username: String,
    pub password: String,
    pub token: String,
    pub key_name: String,
    pub key_value: String,
    pub key_location: KeyLocation,
    pub oauth_flow: OAuthFlow,
    pub token_url: String,
    pub client_id: String,
    pub client_secret: String,
    pub scope: String,
}
#[derive(Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Body {
    pub kind: BodyKind,
    pub text: String,
    pub fields: Vec<Pair>,
}
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct RequestOptions {
    #[serde(default)]
    pub params: Vec<Pair>,
    #[serde(default)]
    pub headers: Vec<Pair>,
    #[serde(default)]
    pub cookies: Vec<Pair>,
    #[serde(default)]
    pub auth: Auth,
    #[serde(default)]
    pub body: Body,
}

pub fn enabled_pairs(pairs: &[Pair]) -> impl Iterator<Item = (&str, &str)> {
    pairs
        .iter()
        .filter(|p| p.enabled && !p.name.trim().is_empty())
        .map(|p| (p.name.as_str(), p.value.as_str()))
}

// Deliberately omit secret values from debug output, including disabled fields.
impl fmt::Debug for Pair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pair")
            .field("enabled", &self.enabled)
            .finish_non_exhaustive()
    }
}
impl fmt::Debug for Auth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Auth")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}
impl fmt::Debug for Body {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Body")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}
impl Drop for RequestOptions {
    fn drop(&mut self) {
        for pair in self
            .params
            .iter_mut()
            .chain(self.headers.iter_mut())
            .chain(self.cookies.iter_mut())
            .chain(self.body.fields.iter_mut())
        {
            pair.name.zeroize();
            pair.value.zeroize();
        }
        self.body.text.zeroize();
        let auth = &mut self.auth;
        auth.username.zeroize();
        auth.password.zeroize();
        auth.token.zeroize();
        auth.key_name.zeroize();
        auth.key_value.zeroize();
        auth.token_url.zeroize();
        auth.client_id.zeroize();
        auth.client_secret.zeroize();
        auth.scope.zeroize();
    }
}
