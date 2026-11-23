use crate::workspace::{Id, Kind};
use reqwest::Method;
use rustiq_http::HttpResponse;

#[derive(Clone)]
pub enum Message {
    History(crate::history_view::Action),
    LockWorkspace,
    ConfirmLock,
    RetryStorage,
    UseTemporaryWorkspace,
    Edit(crate::details::Edit),
    UrlChanged(String),
    MethodChanged(Method),
    SendRequest,
    RequestFinished(Id, Result<HttpResponse, String>),
    Select(Id),
    Toggle(Id),
    Create(Kind),
    Rename,
    NameChanged(String),
    ConfirmName,
    CancelDialog,
    Delete,
    ConfirmDelete,
}

impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Iced debug tools must never print input or response values.
        f.write_str(match self {
            Self::History(_) => "History(<redacted>)",
            Self::LockWorkspace => "LockWorkspace",
            Self::ConfirmLock => "ConfirmLock",
            Self::RetryStorage => "RetryStorage",
            Self::UseTemporaryWorkspace => "UseTemporaryWorkspace",
            Self::Edit(_) => "Edit(<redacted>)",
            Self::UrlChanged(_) => "UrlChanged(<redacted>)",
            Self::MethodChanged(_) => "MethodChanged",
            Self::SendRequest => "SendRequest",
            Self::RequestFinished(_, _) => "RequestFinished(<redacted>)",
            Self::Select(_) => "Select",
            Self::Toggle(_) => "Toggle",
            Self::Create(_) => "Create",
            Self::Rename => "Rename",
            Self::NameChanged(_) => "NameChanged(<redacted>)",
            Self::ConfirmName => "ConfirmName",
            Self::CancelDialog => "CancelDialog",
            Self::Delete => "Delete",
            Self::ConfirmDelete => "ConfirmDelete",
        })
    }
}
