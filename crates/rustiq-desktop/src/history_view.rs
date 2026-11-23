use crate::{history::Exchange, message::Message, theme, ui, workspace::Entry};
use iced::{
    Element, Fill, Task,
    widget::{button, column, container, row, scrollable, text, text_editor},
};

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Bodies,
    Headers,
}
#[derive(Clone, Copy)]
pub enum Side {
    Request,
    Response,
}
#[derive(Clone)]
pub enum Action {
    Toggle,
    Select(String),
    Tab(Tab),
    Editor(Side, text_editor::Action),
    Copy(Side),
    Pretty(Side),
    Raw(Side),
    AskClear,
    CancelClear,
    ConfirmClear,
}
#[derive(Default)]
pub struct HistoryView {
    pub open: bool,
    selected: Option<String>,
    tab: Tab,
    request: text_editor::Content,
    response: text_editor::Content,
    confirm_clear: bool,
    pub notice: Option<String>,
}
impl HistoryView {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    fn selected<'a>(&self, entry: &'a Entry) -> Option<&'a Exchange> {
        self.selected
            .as_ref()
            .and_then(|id| entry.history.iter().find(|h| &h.id == id))
    }
    fn raw(&self, exchange: &Exchange, side: Side) -> String {
        let payload = match side {
            Side::Request => &exchange.request,
            Side::Response => &exchange.response,
        };
        match self.tab {
            Tab::Bodies => payload.body.clone(),
            Tab::Headers => payload
                .headers
                .iter()
                .map(|(name, value)| format!("{name}: {value}"))
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
    fn reload(&mut self, entry: &Entry) {
        let (request, response) = self
            .selected(entry)
            .map(|h| (self.raw(h, Side::Request), self.raw(h, Side::Response)))
            .unwrap_or_default();
        self.request = text_editor::Content::with_text(&request);
        self.response = text_editor::Content::with_text(&response);
    }
    pub fn sync(&mut self, entry: &Entry) {
        if self.selected(entry).is_none() {
            self.selected = entry.history.last().map(|h| h.id.clone());
            self.reload(entry);
        }
    }
    pub fn update(&mut self, action: Action, entry: &mut Entry) -> (Task<Message>, bool) {
        self.notice = None;
        match action {
            Action::Toggle => {
                self.open = !self.open;
                self.confirm_clear = false;
                self.sync(entry);
            }
            Action::Select(id) => {
                if entry.history.iter().any(|h| h.id == id) {
                    self.selected = Some(id);
                    self.reload(entry);
                }
            }
            Action::Tab(tab) => {
                self.tab = tab;
                self.reload(entry);
            }
            Action::Editor(side, action) => {
                if !action.is_edit() {
                    match side {
                        Side::Request => &mut self.request,
                        Side::Response => &mut self.response,
                    }
                    .perform(action);
                }
            }
            Action::Copy(side) => {
                let content = match side {
                    Side::Request => self.request.text(),
                    Side::Response => self.response.text(),
                };
                self.notice = Some("Copied historical payload".into());
                return (iced::clipboard::write(content), false);
            }
            Action::Pretty(side) => {
                if self.tab == Tab::Bodies {
                    let editor = match side {
                        Side::Request => &mut self.request,
                        Side::Response => &mut self.response,
                    };
                    match serde_json::from_str::<serde_json::Value>(&editor.text())
                        .and_then(|value| serde_json::to_string_pretty(&value))
                    {
                        Ok(pretty) => *editor = text_editor::Content::with_text(&pretty),
                        Err(error) => self.notice = Some(format!("Cannot prettify JSON: {error}")),
                    }
                }
            }
            Action::Raw(side) => {
                if let Some(exchange) = self.selected(entry) {
                    let content = text_editor::Content::with_text(&self.raw(exchange, side));
                    *match side {
                        Side::Request => &mut self.request,
                        Side::Response => &mut self.response,
                    } = content;
                }
            }
            Action::AskClear => self.confirm_clear = true,
            Action::CancelClear => self.confirm_clear = false,
            Action::ConfirmClear => {
                if self.confirm_clear {
                    entry.history.clear();
                    self.selected = None;
                    self.reload(entry);
                    self.confirm_clear = false;
                    return (Task::none(), true);
                }
            }
        }
        (Task::none(), false)
    }
    pub fn view<'a>(&'a self, entry: &'a Entry) -> Element<'a, Message> {
        let mut heading = row![
            ui::icon(ui::HISTORY, theme::BLUE),
            text(format!("HISTORY ({})", entry.history.len()))
                .size(13)
                .color(theme::BLUE),
            iced::widget::Space::new().width(Fill),
            control(ui::CHEVRON, "Back to editor", Action::Toggle),
        ]
        .spacing(8)
        .align_y(iced::Center);
        if !entry.history.is_empty() {
            heading = heading.push(control(ui::TRASH, "Clear history", Action::AskClear));
        }
        let mut panel = column![heading, text("Completed responses · newest first · cookies excluded · encrypted with this request").size(10).color(theme::MUTED)].spacing(10).height(Fill);
        if self.confirm_clear {
            panel = panel.push(
                container(
                    column![
                        text("Delete all saved exchanges for this request?").size(12),
                        row![
                            control(ui::TRASH, "Delete history", Action::ConfirmClear),
                            control(ui::CHEVRON, "Cancel", Action::CancelClear)
                        ]
                        .spacing(8)
                    ]
                    .spacing(8),
                )
                .padding(10)
                .width(Fill)
                .style(theme::panel),
            );
        }
        if let Some(notice) = &self.notice {
            panel = panel.push(text(notice).size(11).color(theme::YELLOW));
        }
        if entry.history.is_empty() {
            panel = panel.push(
                container(
                    column![
                        text("$ no exchanges recorded_").size(18).color(theme::BLUE),
                        text("Send a request to record its request/response bodies and headers.")
                            .size(12)
                            .color(theme::MUTED),
                        text(
                            "Retention: 100 per request / 8 MiB per workspace / 2 MiB per exchange."
                        )
                        .size(11)
                        .color(theme::MUTED)
                    ]
                    .spacing(14),
                )
                .center_x(Fill)
                .center_y(Fill),
            );
        } else {
            let mut list = column![].spacing(6);
            for exchange in entry.history.iter().rev() {
                let selected = self.selected.as_ref() == Some(&exchange.id);
                list = list.push(
                    button(
                        column![
                            text(timestamp(exchange.sent_at)).size(10),
                            text(format!(
                                "HTTP {} · {} ms",
                                exchange.status, exchange.elapsed_ms
                            ))
                            .size(11),
                            text(format!("{} B", exchange.response_size)).size(10)
                        ]
                        .spacing(5),
                    )
                    .padding(10)
                    .width(Fill)
                    .on_press(Message::History(Action::Select(exchange.id.clone())))
                    .style(if selected {
                        theme::primary
                    } else {
                        theme::control
                    }),
                );
            }
            let selected = self.selected(entry);
            let tabs = row![
                button("Bodies")
                    .padding([6, 10])
                    .on_press(Message::History(Action::Tab(Tab::Bodies)))
                    .style(if self.tab == Tab::Bodies {
                        theme::primary
                    } else {
                        theme::control
                    }),
                button("Headers")
                    .padding([6, 10])
                    .on_press(Message::History(Action::Tab(Tab::Headers)))
                    .style(if self.tab == Tab::Headers {
                        theme::primary
                    } else {
                        theme::control
                    }),
                text(selected.map_or(String::new(), |h| format!(
                    "HTTP {} · {}",
                    h.status,
                    timestamp(h.sent_at)
                )))
                .size(11)
                .color(theme::MUTED),
            ]
            .spacing(8)
            .align_y(iced::Center);
            let details = column![tabs, row![self.payload_view(Side::Request), self.payload_view(Side::Response)].spacing(10).height(Fill),
                text("Read-only snapshot. Formatting does not change saved data. Retention: 100/request, 8 MiB/workspace.").size(10).color(theme::MUTED)
            ].spacing(10).width(Fill).height(Fill);
            panel = panel.push(
                row![scrollable(list).width(180).height(Fill), details]
                    .spacing(12)
                    .height(Fill),
            );
        }
        container(panel)
            .padding(14)
            .width(Fill)
            .height(Fill)
            .style(theme::inset)
            .into()
    }
    fn payload_view(&self, side: Side) -> Element<'_, Message> {
        let (title, editor) = match side {
            Side::Request => ("REQUEST", &self.request),
            Side::Response => ("RESPONSE", &self.response),
        };
        let copy = iced::widget::tooltip(
            button(ui::icon(ui::COPY, theme::BLUE))
                .padding(7)
                .on_press(Message::History(Action::Copy(side)))
                .style(theme::control),
            text("Copy").size(12),
            iced::widget::tooltip::Position::Top,
        );
        let mut tools = row![copy].spacing(6);
        if self.tab == Tab::Bodies {
            tools = tools
                .push(control(ui::CODE, "Prettify", Action::Pretty(side)))
                .push(control(ui::CODE, "Raw", Action::Raw(side)));
        }
        column![
            text(title).size(12).color(theme::BLUE),
            tools,
            text_editor(editor)
                .font(theme::MONO)
                .size(12)
                .padding(10)
                .style(theme::editor)
                .on_action(move |action| Message::History(Action::Editor(side, action)))
                .height(Fill)
        ]
        .spacing(8)
        .width(Fill)
        .height(Fill)
        .into()
    }
}
fn control<'a>(icon: char, label: &str, action: Action) -> Element<'a, Message> {
    button(ui::label(icon, label, theme::BLUE))
        .padding([6, 8])
        .on_press(Message::History(action))
        .style(theme::control)
        .into()
}
fn timestamp(seconds: u64) -> String {
    i64::try_from(seconds)
        .ok()
        .and_then(|seconds| time::OffsetDateTime::from_unix_timestamp(seconds).ok())
        .and_then(|date| {
            date.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .unwrap_or_else(|| "Unknown time".into())
}
impl std::fmt::Debug for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HistoryAction(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{Kind, Workspace};
    #[test]
    fn history_is_read_only_formatting_is_temporary_and_clear_needs_confirmation() {
        let mut workspace = Workspace::default();
        let collection = workspace.create(Kind::Collection, None, "API").unwrap();
        let id = workspace
            .create(Kind::Request, Some(collection), "Request")
            .unwrap();
        let mut response = crate::history::tests::response();
        response.body = "{\"ok\":true}".into();
        crate::history::record(&mut workspace, id, &response).unwrap();
        let entry = workspace.entry_mut(id).unwrap();
        entry.request.body.text = "live draft".into();
        let mut view = HistoryView::default();
        let _ = view.update(Action::Toggle, entry);
        let _ = view.update(Action::Pretty(Side::Response), entry);
        assert!(view.response.text().contains('\n'));
        assert_eq!(entry.history[0].response.body, "{\"ok\":true}");
        assert_eq!(entry.request.body.text, "live draft");
        let _ = view.update(
            Action::Editor(
                Side::Response,
                text_editor::Action::Edit(text_editor::Edit::Insert('x')),
            ),
            entry,
        );
        assert!(!view.response.text().starts_with('x'));
        let _ = view.update(Action::Raw(Side::Response), entry);
        assert_eq!(view.response.text(), "{\"ok\":true}");
        let _ = view.update(Action::Tab(Tab::Headers), entry);
        assert!(view.request.text().contains("Authorization: Bearer secret"));
        let _ = view.view(entry);
        let (_, changed) = view.update(Action::ConfirmClear, entry);
        assert!(!changed);
        let _ = view.update(Action::AskClear, entry);
        let (_, changed) = view.update(Action::ConfirmClear, entry);
        assert!(changed);
        assert!(entry.history.is_empty());
        assert!(view.request.text().is_empty());
    }
}
