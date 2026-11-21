use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Body {
    pub kind: BodyKind,
    pub text: String,
    pub fields: Vec<Pair>,
}
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RequestOptions {
    pub params: Vec<Pair>,
    pub headers: Vec<Pair>,
    pub cookies: Vec<Pair>,
    pub auth: Auth,
    pub body: Body,
}

pub fn enabled_pairs(pairs: &[Pair]) -> impl Iterator<Item = (&str, &str)> {
    pairs
        .iter()
        .filter(|p| p.enabled && !p.name.trim().is_empty())
        .map(|p| (p.name.as_str(), p.value.as_str()))
}
