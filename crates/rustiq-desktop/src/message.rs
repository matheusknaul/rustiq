use crate::workspace::{Id, Kind};
use reqwest::Method;
use rustiq_http::HttpResponse;

#[derive(Debug, Clone)]
pub enum Message {
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
