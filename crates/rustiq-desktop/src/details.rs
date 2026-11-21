use crate::{
    message::Message,
    theme, ui,
    workspace::{Entry, Persistence},
};
use iced::{
    Element, Fill, Task,
    widget::{
        button, checkbox, column, container, pick_list, row, scrollable, text, text_editor,
        text_input,
    },
};
use rustiq_http::{AuthKind, BodyKind, HttpResponse, KeyLocation, OAuthFlow, Pair};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum RequestTab {
    #[default]
    Params,
    Auth,
    Headers,
    Body,
    Cookies,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ResponseTab {
    #[default]
    Body,
    Headers,
    Cookies,
}
#[derive(Debug, Clone, Copy)]
pub enum Table {
    Params,
    Headers,
    Cookies,
    Form,
}
#[derive(Debug, Clone, Copy)]
pub enum AuthField {
    Username,
    Password,
    Token,
    KeyName,
    KeyValue,
    TokenUrl,
    ClientId,
    ClientSecret,
    Scope,
}
#[derive(Clone)]
pub enum Edit {
    Persistence(bool),
    RequestTab(RequestTab),
    ResponseTab(ResponseTab),
    Add(Table),
    Remove(Table, usize),
    Name(Table, usize, String),
    Value(Table, usize, String),
    Enabled(Table, usize, bool),
    AuthKind(AuthKind),
    AuthField(AuthField, String),
    KeyLocation(KeyLocation),
    OAuthFlow(OAuthFlow),
    BodyKind(BodyKind),
    RequestBody(text_editor::Action),
    ResponseBody(text_editor::Action),
    CopyRequest,
    PrettyRequest,
    CopyResponse,
    PrettyResponse,
    RawResponse,
    CopyHeaders,
}
#[derive(Default)]
pub struct Response {
    pub loading: bool,
    pub status: Option<u16>,
    pub body: String,
    pub data: Option<HttpResponse>,
    pub editor: text_editor::Content,
}
impl Response {
    pub fn complete(&mut self, result: Result<HttpResponse, String>) {
        self.loading = false;
        match result {
            Ok(data) => {
                self.status = Some(data.status);
                self.body = data.body.clone();
                self.data = Some(data);
            }
            Err(error) => {
                self.status = None;
                self.data = None;
                self.body = format!("Error: {error}");
            }
        }
        self.editor = text_editor::Content::with_text(&self.body);
    }
}
#[derive(Default)]
pub struct Details {
    pub request_tab: RequestTab,
    pub response_tab: ResponseTab,
    pub editor: text_editor::Content,
    pub notice: Option<String>,
}
fn pretty(value: &str) -> Result<String, String> {
    let value: serde_json::Value =
        serde_json::from_str(value).map_err(|e| format!("Cannot prettify: {e}"))?;
    serde_json::to_string_pretty(&value).map_err(|e| e.to_string())
}
fn pairs(entry: &mut Entry, table: Table) -> &mut Vec<Pair> {
    match table {
        Table::Params => &mut entry.request.params,
        Table::Headers => &mut entry.request.headers,
        Table::Cookies => &mut entry.request.cookies,
        Table::Form => &mut entry.request.body.fields,
    }
}
impl Details {
    pub fn select(&mut self, entry: Option<&Entry>) {
        self.editor =
            text_editor::Content::with_text(entry.map_or("", |e| e.request.body.text.as_str()));
        self.notice = None;
    }
    pub fn update(
        &mut self,
        edit: Edit,
        entry: &mut Entry,
        response: Option<&mut Response>,
    ) -> (Task<Message>, bool) {
        let mut changed = true;
        self.notice = None;
        match edit {
            Edit::Persistence(persist) => {
                entry.persistence = if persist {
                    Persistence::Encrypted
                } else {
                    Persistence::SessionOnly
                }
            }
            Edit::RequestTab(tab) => {
                self.request_tab = tab;
                changed = false;
            }
            Edit::ResponseTab(tab) => {
                self.response_tab = tab;
                changed = false;
            }
            Edit::Add(table) => pairs(entry, table).push(Pair::default()),
            Edit::Remove(table, index) => {
                let pairs = pairs(entry, table);
                if index < pairs.len() {
                    pairs.remove(index);
                }
            }
            Edit::Name(table, index, name) => {
                if let Some(pair) = pairs(entry, table).get_mut(index) {
                    pair.name = name;
                }
            }
            Edit::Value(table, index, value) => {
                if let Some(pair) = pairs(entry, table).get_mut(index) {
                    pair.value = value;
                }
            }
            Edit::Enabled(table, index, enabled) => {
                if let Some(pair) = pairs(entry, table).get_mut(index) {
                    pair.enabled = enabled;
                }
            }
            Edit::AuthKind(kind) => entry.request.auth.kind = kind,
            Edit::KeyLocation(location) => entry.request.auth.key_location = location,
            Edit::OAuthFlow(flow) => entry.request.auth.oauth_flow = flow,
            Edit::AuthField(field, value) => {
                let auth = &mut entry.request.auth;
                *match field {
                    AuthField::Username => &mut auth.username,
                    AuthField::Password => &mut auth.password,
                    AuthField::Token => &mut auth.token,
                    AuthField::KeyName => &mut auth.key_name,
                    AuthField::KeyValue => &mut auth.key_value,
                    AuthField::TokenUrl => &mut auth.token_url,
                    AuthField::ClientId => &mut auth.client_id,
                    AuthField::ClientSecret => &mut auth.client_secret,
                    AuthField::Scope => &mut auth.scope,
                } = value;
            }
            Edit::BodyKind(kind) => entry.request.body.kind = kind,
            Edit::RequestBody(action) => {
                changed = action.is_edit();
                self.editor.perform(action);
                if changed {
                    entry.request.body.text = self.editor.text();
                }
            }
            Edit::ResponseBody(action) => {
                if !action.is_edit()
                    && let Some(response) = response
                {
                    response.editor.perform(action);
                }
                changed = false;
            }
            Edit::PrettyRequest => match pretty(&entry.request.body.text) {
                Ok(body) => {
                    self.editor = text_editor::Content::with_text(&body);
                    entry.request.body.text = body;
                }
                Err(error) => {
                    self.notice = Some(error);
                    changed = false;
                }
            },
            Edit::CopyRequest => {
                self.notice = Some("Request body copied".into());
                let body = match entry.request.body.kind {
                    BodyKind::Form => {
                        let mut url = reqwest::Url::parse("http://localhost/").expect("static URL");
                        url.query_pairs_mut()
                            .extend_pairs(rustiq_http::enabled_pairs(&entry.request.body.fields));
                        url.query().unwrap_or_default().to_owned()
                    }
                    BodyKind::Multipart => rustiq_http::enabled_pairs(&entry.request.body.fields)
                        .map(|(name, value)| format!("{name}={value}"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    BodyKind::None => String::new(),
                    _ => entry.request.body.text.clone(),
                };
                return (iced::clipboard::write(body), false);
            }
            Edit::CopyResponse => {
                if let Some(response) = response {
                    self.notice = Some("Response body copied".into());
                    return (iced::clipboard::write(response.body.clone()), false);
                }
                changed = false;
            }
            Edit::PrettyResponse => {
                if let Some(response) = response {
                    match pretty(&response.body) {
                        Ok(body) => {
                            response.editor = text_editor::Content::with_text(&body);
                            response.body = body;
                        }
                        Err(error) => self.notice = Some(error),
                    }
                }
                changed = false;
            }
            Edit::RawResponse => {
                if let Some(response) = response
                    && let Some(data) = &response.data
                {
                    response.body = data.body.clone();
                    response.editor = text_editor::Content::with_text(&response.body);
                }
                changed = false;
            }
            Edit::CopyHeaders => {
                if let Some(data) = response.and_then(|r| r.data.as_ref()) {
                    return (
                        iced::clipboard::write(
                            data.headers
                                .iter()
                                .map(|(k, v)| format!("{k}: {v}"))
                                .collect::<Vec<_>>()
                                .join("\n"),
                        ),
                        false,
                    );
                }
                changed = false;
            }
        }
        (Task::none(), changed)
    }

    pub fn request_view<'a>(&'a self, entry: &'a Entry, temporary: bool) -> Element<'a, Message> {
        let options = &entry.request;
        let tabs = row![
            tab(
                format!(
                    "Params ({})",
                    options
                        .params
                        .iter()
                        .filter(|p| p.enabled && !p.name.is_empty())
                        .count()
                ),
                self.request_tab == RequestTab::Params,
                Edit::RequestTab(RequestTab::Params)
            ),
            tab(
                "Auth".into(),
                self.request_tab == RequestTab::Auth,
                Edit::RequestTab(RequestTab::Auth)
            ),
            tab(
                format!(
                    "Headers ({})",
                    options
                        .headers
                        .iter()
                        .filter(|p| p.enabled && !p.name.is_empty())
                        .count()
                ),
                self.request_tab == RequestTab::Headers,
                Edit::RequestTab(RequestTab::Headers)
            ),
            tab(
                "Body".into(),
                self.request_tab == RequestTab::Body,
                Edit::RequestTab(RequestTab::Body)
            ),
            tab(
                format!(
                    "Cookies ({})",
                    options
                        .cookies
                        .iter()
                        .filter(|p| p.enabled && !p.name.is_empty())
                        .count()
                ),
                self.request_tab == RequestTab::Cookies,
                Edit::RequestTab(RequestTab::Cookies)
            ),
        ]
        .spacing(4);
        let body: Element<'a, Message> = match self.request_tab {
            RequestTab::Params => table_view(
                &options.params,
                Table::Params,
                "Query parameters are appended to the URL and encoded automatically.",
            ),
            RequestTab::Headers => table_view(
                &options.headers,
                Table::Headers,
                "Enabled headers are sent with this request. Duplicate names are supported.",
            ),
            RequestTab::Cookies => table_view(
                &options.cookies,
                Table::Cookies,
                "These cookies are sent only with this request. Received cookies appear below.",
            ),
            RequestTab::Auth => {
                let auth = &options.auth;
                let mut fields = column![
                    pick_list(AuthKind::ALL, Some(auth.kind), |kind| Message::Edit(
                        Edit::AuthKind(kind)
                    ))
                    .text_size(12)
                    .padding(8)
                ]
                .spacing(10);
                match auth.kind {
                    AuthKind::None => {
                        fields = fields.push(hint(
                            "No authentication. You can supply an Authorization header manually.",
                        ))
                    }
                    AuthKind::Basic => {
                        fields = fields
                            .push(auth_input(
                                "Username",
                                &auth.username,
                                AuthField::Username,
                                false,
                            ))
                            .push(auth_input(
                                "Password",
                                &auth.password,
                                AuthField::Password,
                                true,
                            ))
                    }
                    AuthKind::Bearer => {
                        fields =
                            fields.push(auth_input("Token", &auth.token, AuthField::Token, true))
                    }
                    AuthKind::ApiKey => {
                        fields = fields
                            .push(
                                row![
                                    auth_input(
                                        "Key name",
                                        &auth.key_name,
                                        AuthField::KeyName,
                                        false
                                    ),
                                    auth_input(
                                        "Key value",
                                        &auth.key_value,
                                        AuthField::KeyValue,
                                        true
                                    )
                                ]
                                .spacing(10),
                            )
                            .push(
                                pick_list(KeyLocation::ALL, Some(auth.key_location), |location| {
                                    Message::Edit(Edit::KeyLocation(location))
                                })
                                .text_size(12)
                                .padding(8),
                            );
                    }
                    AuthKind::OAuth2 => {
                        fields = fields.push(
                            pick_list(OAuthFlow::ALL, Some(auth.oauth_flow), |flow| {
                                Message::Edit(Edit::OAuthFlow(flow))
                            })
                            .text_size(12)
                            .padding(8),
                        );
                        if auth.oauth_flow == OAuthFlow::AccessToken {
                            fields = fields.push(auth_input(
                                "Access token",
                                &auth.token,
                                AuthField::Token,
                                true,
                            ));
                        } else {
                            fields = fields.push(auth_input("Token URL", &auth.token_url, AuthField::TokenUrl, false))
                                .push(row![auth_input("Client ID", &auth.client_id, AuthField::ClientId, false), auth_input("Client secret", &auth.client_secret, AuthField::ClientSecret, true)].spacing(10))
                                .push(auth_input("Scope (optional)", &auth.scope, AuthField::Scope, false))
                                .push(hint("Fetches a token on Send using HTTP Basic client authentication."));
                        }
                    }
                }
                if auth.kind != AuthKind::None {
                    fields = fields.push(hint(if temporary || entry.persistence == Persistence::SessionOnly {
                        "Session only: request configuration is not saved."
                    } else { "Request configuration is encrypted with a key held in the OS credential store." }));
                }
                scrollable(fields).height(Fill).into()
            }
            RequestTab::Body => {
                let raw = matches!(options.body.kind, BodyKind::Json | BodyKind::Text);
                let mut tools = row![
                    pick_list(
                        BodyKind::ALL,
                        Some(options.body.kind),
                        |kind| Message::Edit(Edit::BodyKind(kind))
                    )
                    .text_size(12)
                    .padding(8),
                    iced::widget::Space::new().width(Fill)
                ]
                .spacing(8)
                .align_y(iced::Center);
                if options.body.kind != BodyKind::None {
                    tools = tools.push(action(
                        ui::COPY,
                        if raw { "Copy" } else { "Copy fields" },
                        Edit::CopyRequest,
                    ));
                }
                if raw {
                    tools = tools.push(action(ui::CODE, "Prettify JSON", Edit::PrettyRequest));
                }
                let mut body = column![tools].spacing(10).height(Fill);
                body = match options.body.kind {
                    BodyKind::None => body.push(hint("This request has no body.")),
                    BodyKind::Json | BodyKind::Text => body.push(text_editor(&self.editor).style(theme::editor).on_action(|action| Message::Edit(Edit::RequestBody(action))).font(theme::MONO).size(13).padding(10).height(Fill)),
                    BodyKind::Form => body.push(table_view(&options.body.fields, Table::Form, "Fields are URL encoded. Content-Type is generated automatically.")),
                    BodyKind::Multipart => body.push(table_view(&options.body.fields, Table::Form, "Multipart text fields. Content-Type and boundary are generated automatically.")),
                };
                body.into()
            }
        };
        let mut save = checkbox(!temporary && entry.persistence == Persistence::Encrypted)
            .label(if temporary {
                "Session-only workspace: nothing is saved"
            } else {
                "Save request configuration encrypted"
            })
            .text_size(11);
        if !temporary {
            save = save.on_toggle(|persist| Message::Edit(Edit::Persistence(persist)));
        }
        let storage_note = if temporary || entry.persistence == Persistence::SessionOnly {
            "Session only: URL and request fields are discarded on close or lock."
        } else {
            "URL and request fields are encrypted; item names and HTTP methods remain visible."
        };
        container(
            column![
                row![ui::icon(ui::LOCK, theme::GREEN), save]
                    .spacing(8)
                    .align_y(iced::Center),
                hint(storage_note),
                tabs,
                iced::widget::rule::horizontal(1),
                body
            ]
            .spacing(10),
        )
        .padding(12)
        .width(Fill)
        .height(Fill)
        .style(theme::panel)
        .into()
    }

    pub fn response_view<'a>(&'a self, response: Option<&'a Response>) -> Element<'a, Message> {
        let data = response.and_then(|r| r.data.as_ref());
        let (status, color) = match response {
            Some(r) if r.loading => ("IN FLIGHT".into(), theme::YELLOW),
            Some(r) if r.status.is_some() => {
                let status = r.status.unwrap();
                (
                    format!("HTTP {status}"),
                    if status < 400 {
                        theme::GREEN
                    } else {
                        theme::RED
                    },
                )
            }
            Some(r) if !r.body.is_empty() => ("REQUEST FAILED".into(), theme::RED),
            _ => ("IDLE".into(), theme::MUTED),
        };
        let heading = row![
            ui::icon(ui::TERMINAL, theme::BLUE),
            text("RESPONSE").size(12).color(theme::BLUE),
            iced::widget::Space::new().width(Fill),
            text(status).size(12).color(color),
            text(format!(
                "{} ms / {} B",
                data.map_or(0, |d| d.elapsed_ms),
                data.map_or(0, |d| d.size)
            ))
            .size(11)
            .color(theme::MUTED)
        ]
        .spacing(10)
        .align_y(iced::Center);
        let tabs = row![
            tab(
                "Body".into(),
                self.response_tab == ResponseTab::Body,
                Edit::ResponseTab(ResponseTab::Body)
            ),
            tab(
                format!("Headers ({})", data.map_or(0, |d| d.headers.len())),
                self.response_tab == ResponseTab::Headers,
                Edit::ResponseTab(ResponseTab::Headers)
            ),
            tab(
                format!("Cookies ({})", data.map_or(0, |d| d.cookies.len())),
                self.response_tab == ResponseTab::Cookies,
                Edit::ResponseTab(ResponseTab::Cookies)
            ),
        ]
        .spacing(4);
        let body: Element<'a, Message> = match self.response_tab {
            ResponseTab::Body => {
                if let Some(response) = response.filter(|r| !r.body.is_empty()) {
                    column![
                        row![
                            action(ui::COPY, "Copy", Edit::CopyResponse),
                            action(ui::CODE, "Prettify JSON", Edit::PrettyResponse),
                            action(ui::CODE, "Raw", Edit::RawResponse)
                        ]
                        .spacing(6),
                        text_editor(&response.editor)
                            .style(theme::editor)
                            .on_action(|action| Message::Edit(Edit::ResponseBody(action)))
                            .font(theme::MONO)
                            .size(13)
                            .padding(10)
                            .height(Fill)
                    ]
                    .spacing(8)
                    .height(Fill)
                    .into()
                } else {
                    container(
                        column![
                            text(if response.is_some_and(|r| r.loading) {
                                "$ sending request_"
                            } else if data.is_some() {
                                "$ empty response body_"
                            } else {
                                "$ awaiting response_"
                            })
                            .size(16)
                            .color(theme::BLUE),
                            hint("Send a request to inspect its response.")
                        ]
                        .spacing(10),
                    )
                    .center_x(Fill)
                    .center_y(Fill)
                    .into()
                }
            }
            ResponseTab::Headers => {
                let mut rows =
                    column![action(ui::COPY, "Copy headers", Edit::CopyHeaders)].spacing(8);
                if let Some(data) = data {
                    for (name, value) in &data.headers {
                        rows = rows.push(
                            container(
                                row![
                                    text(name).size(12).color(theme::BLUE).width(190),
                                    text(value).size(12).width(Fill)
                                ]
                                .spacing(12),
                            )
                            .padding(8)
                            .width(Fill)
                            .style(theme::panel),
                        );
                    }
                    if data.headers.is_empty() {
                        rows = rows.push(hint("No headers returned."));
                    }
                } else {
                    rows = rows.push(hint("No response headers yet."));
                }
                scrollable(rows).height(Fill).into()
            }
            ResponseTab::Cookies => {
                let mut rows = column![hint(
                    "Captured from Set-Cookie. Cookies are not automatically reused."
                )]
                .spacing(8);
                if let Some(data) = data {
                    for cookie in &data.cookies {
                        rows = rows.push(
                            container(
                                column![
                                    text(format!("{} = {}", cookie.name, cookie.value))
                                        .size(13)
                                        .color(theme::GREEN),
                                    text(format!(
                                        "Domain: {}  |  Path: {}",
                                        cookie.domain, cookie.path
                                    ))
                                    .size(11),
                                    text(format!(
                                        "{}  |  Secure: {}  |  HttpOnly: {}  |  SameSite: {}",
                                        cookie.expires,
                                        cookie.secure,
                                        cookie.http_only,
                                        cookie.same_site
                                    ))
                                    .size(11)
                                    .color(theme::MUTED),
                                    text(&cookie.raw).size(11).color(theme::MUTED)
                                ]
                                .spacing(6),
                            )
                            .padding(10)
                            .width(Fill)
                            .style(theme::panel),
                        );
                    }
                    if data.cookies.is_empty() {
                        rows = rows.push(hint("No cookies returned by this response."));
                    }
                } else {
                    rows = rows.push(hint("No response cookies yet."));
                }
                scrollable(rows).height(Fill).into()
            }
        };
        container(column![heading, tabs, iced::widget::rule::horizontal(1), body].spacing(10))
            .padding(12)
            .width(Fill)
            .height(Fill)
            .style(theme::inset)
            .into()
    }
}

fn hint<'a>(value: &'a str) -> iced::widget::Text<'a> {
    text(value).size(11).color(theme::MUTED)
}
fn tab<'a>(caption: String, selected: bool, edit: Edit) -> Element<'a, Message> {
    button(text(caption).size(12))
        .padding([7, 10])
        .on_press(Message::Edit(edit))
        .style(if selected {
            theme::primary
        } else {
            theme::control
        })
        .into()
}
fn action<'a>(glyph: char, caption: &str, edit: Edit) -> Element<'a, Message> {
    button(ui::label(glyph, caption, theme::BLUE))
        .padding([6, 9])
        .style(theme::control)
        .on_press(Message::Edit(edit))
        .into()
}
fn auth_input<'a>(
    label: &'a str,
    value: &'a str,
    field: AuthField,
    secure: bool,
) -> Element<'a, Message> {
    column![
        hint(label),
        text_input(label, value)
            .on_input(move |value| Message::Edit(Edit::AuthField(field, value)))
            .secure(secure)
            .size(12)
            .padding(8)
            .style(theme::input)
    ]
    .spacing(5)
    .width(Fill)
    .into()
}
fn table_view<'a>(pairs: &'a [Pair], table: Table, description: &'a str) -> Element<'a, Message> {
    let mut rows = column![hint(description)].spacing(8);
    for (index, pair) in pairs.iter().enumerate() {
        rows = rows.push(
            row![
                checkbox(pair.enabled)
                    .on_toggle(move |enabled| Message::Edit(Edit::Enabled(table, index, enabled))),
                text_input("Name", &pair.name)
                    .on_input(move |name| Message::Edit(Edit::Name(table, index, name)))
                    .size(12)
                    .padding(8)
                    .style(theme::input)
                    .width(Fill),
                text_input("Value", &pair.value)
                    .on_input(move |value| Message::Edit(Edit::Value(table, index, value)))
                    .size(12)
                    .padding(8)
                    .style(theme::input)
                    .width(Fill),
                button(ui::icon(ui::TRASH, theme::RED))
                    .padding(8)
                    .on_press(Message::Edit(Edit::Remove(table, index)))
                    .style(theme::control)
            ]
            .spacing(8)
            .align_y(iced::Center),
        );
    }
    rows = rows.push(action(ui::PLUS, "Add row", Edit::Add(table)));
    scrollable(rows).height(Fill).into()
}

impl std::fmt::Debug for Edit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RequestEdit(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{Kind, Workspace};

    #[test]
    fn prettify_preserves_invalid_input_and_raw_response() {
        let mut workspace = Workspace::default();
        let collection = workspace.create(Kind::Collection, None, "API").unwrap();
        let request = workspace
            .create(Kind::Request, Some(collection), "Request")
            .unwrap();
        let entry = workspace.entry_mut(request).unwrap();
        entry.request.body.kind = BodyKind::Json;
        entry.request.body.text = "invalid json".into();
        let mut details = Details::default();
        details.select(Some(entry));
        let (_, changed) = details.update(Edit::PrettyRequest, entry, None);
        assert!(!changed);
        assert_eq!(entry.request.body.text, "invalid json");
        assert!(details.notice.is_some());
        entry.request.body.text = "{\"z\":123456789012345678901234567890,\"a\":true}".into();
        let _ = details.update(Edit::PrettyRequest, entry, None);
        assert!(
            entry
                .request
                .body
                .text
                .contains("123456789012345678901234567890")
        );
        assert!(
            entry.request.body.text.find("\"z\"").unwrap()
                < entry.request.body.text.find("\"a\"").unwrap()
        );
        let mut response = Response::default();
        response.complete(Ok(HttpResponse {
            status: 200,
            body: "{\"ok\":true}".into(),
            ..Default::default()
        }));
        let _ = details.update(Edit::PrettyResponse, entry, Some(&mut response));
        assert!(response.body.contains('\n'));
        assert_eq!(response.data.as_ref().unwrap().body, "{\"ok\":true}");
        let _ = details.update(Edit::RawResponse, entry, Some(&mut response));
        assert_eq!(response.body, "{\"ok\":true}");
        let _ = details.update(
            Edit::ResponseBody(text_editor::Action::Edit(text_editor::Edit::Insert('x'))),
            entry,
            Some(&mut response),
        );
        assert_eq!(response.editor.text(), "{\"ok\":true}");
    }

    #[test]
    fn selection_restores_each_request_body() {
        let mut workspace = Workspace::default();
        let collection = workspace.create(Kind::Collection, None, "API").unwrap();
        let first = workspace
            .create(Kind::Request, Some(collection), "First")
            .unwrap();
        let second = workspace
            .create(Kind::Request, Some(collection), "Second")
            .unwrap();
        let mut details = Details::default();
        details.select(workspace.entry(first));
        let _ = details.update(
            Edit::RequestBody(text_editor::Action::Edit(text_editor::Edit::Paste(
                std::sync::Arc::new("first body".into()),
            ))),
            workspace.entry_mut(first).unwrap(),
            None,
        );
        details.select(workspace.entry(second));
        assert!(details.editor.text().is_empty());
        details.select(workspace.entry(first));
        assert_eq!(details.editor.text(), "first body");
    }
}
