use crate::details::{Details, Response};
use crate::storage::{NativeKeyStore, Storage, workspace_path};
use crate::{
    message::Message,
    workspace::{Id, Kind, Workspace},
};
use crate::{theme, ui};
use iced::{
    Element, Fill, Task,
    widget::{button, column, container, pick_list, row, scrollable, text, text_input},
};
use reqwest::Method;
use rustiq_http::send_request_with_options;
use std::{collections::HashMap, path::PathBuf};

enum Dialog {
    Name {
        id: Option<Id>,
        kind: Kind,
        parent: Option<Id>,
        value: String,
    },
    Delete(Id),
    LockWorkspace,
}

pub struct App {
    workspace: Workspace,
    details: Details,
    selected: Option<Id>,
    responses: HashMap<Id, Response>,
    pending: HashMap<Id, iced::task::Handle>,
    dialog: Option<Dialog>,
    path: PathBuf,
    storage_error: Option<String>,
    storage: Option<Storage>,
    locked: bool,
    temporary: bool,
}

impl Default for App {
    fn default() -> Self {
        let location = workspace_path();
        let path = location.as_ref().cloned().unwrap_or_default();
        let (storage, workspace, storage_error) =
            match location.and_then(|path| Storage::open(path, NativeKeyStore)) {
                Ok((storage, workspace)) => (Some(storage), workspace, None),
                Err(error) => (None, Workspace::default(), Some(error)),
            };
        let storage_ready = storage.is_some();
        Self {
            workspace,
            details: Details::default(),
            selected: None,
            responses: HashMap::new(),
            pending: HashMap::new(),
            dialog: None,
            path,
            storage_error,
            storage,
            locked: !storage_ready,
            temporary: false,
        }
    }
}

impl App {
    fn persist(&mut self) {
        if let Some(storage) = &mut self.storage {
            self.storage_error = storage.save(&self.workspace).err();
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        if self.locked
            && !matches!(
                message,
                Message::RetryStorage | Message::UseTemporaryWorkspace
            )
        {
            return Task::none();
        }
        match message {
            Message::LockWorkspace => {
                if !self.temporary && self.storage_error.is_none() {
                    self.dialog = Some(Dialog::LockWorkspace);
                }
            }
            Message::ConfirmLock => {
                if matches!(self.dialog, Some(Dialog::LockWorkspace)) {
                    for handle in self.pending.values() {
                        handle.abort();
                    }
                    self.pending.clear();
                    self.workspace = Workspace::default();
                    self.details = Details::default();
                    self.responses.clear();
                    self.selected = None;
                    self.dialog = None;
                    self.storage = None; // Drops the zeroizing cached encryption key.
                    self.locked = true;
                    self.storage_error = None;
                }
            }
            Message::RetryStorage => {
                if self.locked {
                    let location = if self.path.as_os_str().is_empty() {
                        workspace_path()
                    } else {
                        Ok(self.path.clone())
                    };
                    match location.and_then(|path| {
                        Storage::open(path.clone(), NativeKeyStore).map(|result| (path, result))
                    }) {
                        Ok((path, (storage, workspace))) => {
                            self.path = path;
                            self.storage = Some(storage);
                            self.workspace = workspace;
                            self.locked = false;
                            self.storage_error = None;
                        }
                        Err(error) => self.storage_error = Some(error),
                    }
                } else {
                    self.persist();
                }
            }
            Message::UseTemporaryWorkspace => {
                // Detach from disk entirely. The saved workspace is never overwritten.
                self.storage = None;
                self.locked = false;
                self.temporary = true;
                self.storage_error = None;
            }
            Message::Edit(edit) => {
                if let Some(entry) = self.selected.and_then(|id| self.workspace.entry_mut(id)) {
                    let (task, changed) =
                        self.details
                            .update(edit, entry, self.responses.get_mut(&entry.id));
                    if changed {
                        self.persist();
                    }
                    return task;
                }
            }
            Message::Select(id) => {
                self.selected = Some(id);
                self.details.select(self.workspace.entry(id));
            }
            Message::Toggle(id) => {
                if let Some(entry) = self.workspace.entry_mut(id) {
                    entry.expanded = !entry.expanded;
                }
                self.persist();
            }
            Message::Create(kind) => {
                let parent = if kind == Kind::Collection {
                    None
                } else {
                    self.selected
                        .and_then(|id| self.workspace.entry(id))
                        .and_then(|e| {
                            if e.kind == Kind::Request {
                                e.parent
                            } else {
                                Some(e.id)
                            }
                        })
                };
                if kind == Kind::Collection || parent.is_some() {
                    self.dialog = Some(Dialog::Name {
                        id: None,
                        kind,
                        parent,
                        value: String::new(),
                    });
                    return iced::widget::operation::focus("entry-name");
                }
            }
            Message::Rename => {
                if let Some(entry) = self.selected.and_then(|id| self.workspace.entry(id)) {
                    self.dialog = Some(Dialog::Name {
                        id: Some(entry.id),
                        kind: entry.kind,
                        parent: entry.parent,
                        value: entry.name.clone(),
                    });
                    return iced::widget::operation::focus("entry-name");
                }
            }
            Message::NameChanged(name) => {
                if let Some(Dialog::Name { value, .. }) = &mut self.dialog {
                    *value = name;
                }
            }
            Message::ConfirmName => {
                if let Some(Dialog::Name {
                    id,
                    kind,
                    parent,
                    value,
                }) = self.dialog.take()
                {
                    if value.trim().is_empty() {
                        self.dialog = Some(Dialog::Name {
                            id,
                            kind,
                            parent,
                            value,
                        });
                    } else {
                        if let Some(id) = id {
                            self.workspace.rename(id, &value);
                        } else {
                            self.selected = self.workspace.create(kind, parent, &value);
                            self.details
                                .select(self.selected.and_then(|id| self.workspace.entry(id)));
                        }
                        self.persist();
                    }
                }
            }
            Message::CancelDialog => self.dialog = None,
            Message::Delete => {
                self.dialog = self.selected.map(Dialog::Delete);
            }
            Message::ConfirmDelete => {
                if let Some(Dialog::Delete(id)) = self.dialog.take() {
                    self.workspace.remove(id);
                    self.responses
                        .retain(|id, _| self.workspace.entry(*id).is_some());
                    self.pending.retain(|id, handle| {
                        if self.workspace.entry(*id).is_none() {
                            handle.abort();
                            false
                        } else {
                            true
                        }
                    });
                    self.selected = None;
                    self.persist();
                }
            }
            Message::UrlChanged(url) => {
                if let Some(entry) = self.selected.and_then(|id| self.workspace.entry_mut(id)) {
                    entry.url = url;
                }
                self.persist();
            }
            Message::MethodChanged(method) => {
                if let Some(entry) = self.selected.and_then(|id| self.workspace.entry_mut(id)) {
                    entry.method = method.to_string();
                }
                self.persist();
            }
            Message::SendRequest => {
                let Some(entry) = self.selected.and_then(|id| self.workspace.entry(id)) else {
                    return Task::none();
                };
                if entry.kind != Kind::Request || entry.url.trim().is_empty() {
                    return Task::none();
                }
                let id = entry.id;
                let response = self.responses.entry(id).or_default();
                if response.loading {
                    return Task::none();
                }
                *response = Response {
                    loading: true,
                    ..Response::default()
                };
                let url = entry.url.clone();
                let method = entry.method.parse().unwrap_or(Method::GET);
                let options = entry.request.clone();
                let (task, handle) = Task::perform(
                    async move { send_request_with_options(&url, method, options).await },
                    move |result| Message::RequestFinished(id, result),
                )
                .abortable();
                self.pending.insert(id, handle.abort_on_drop());
                return task;
            }
            Message::RequestFinished(id, result) => {
                self.pending.remove(&id);
                if self.workspace.entry(id).is_some() {
                    let response = self.responses.entry(id).or_default();
                    response.complete(result);
                }
            }
        }
        Task::none()
    }

    fn tree(&self, parent: Option<Id>, depth: usize) -> Element<'_, Message> {
        let mut tree = column![].spacing(3);
        for entry in self.workspace.entries.iter().filter(|e| e.parent == parent) {
            let selected = self.selected == Some(entry.id);
            let color = if selected { theme::GREEN } else { theme::BLUE };
            let glyph = match entry.kind {
                Kind::Collection => ui::BOX,
                Kind::Folder if entry.expanded => ui::OPEN_FOLDER,
                Kind::Folder => ui::FOLDER,
                Kind::Request => ui::REQUEST,
            };
            let mut caption = row![ui::icon(glyph, color)]
                .spacing(9)
                .align_y(iced::Center);
            if entry.kind == Kind::Request {
                caption = caption.push(
                    text(&entry.method)
                        .size(11)
                        .color(theme::method_color(&entry.method)),
                );
            }
            caption = caption.push(text(&entry.name).size(13).color(if selected {
                theme::GREEN
            } else {
                theme::TEXT
            }));
            let select = button(caption)
                .padding([9, 8])
                .width(Fill)
                .on_press(Message::Select(entry.id))
                .style(move |_, status| button::Style {
                    background: Some(
                        if selected || status == button::Status::Hovered {
                            theme::SURFACE
                        } else {
                            theme::BASE
                        }
                        .into(),
                    ),
                    text_color: theme::TEXT,
                    border: iced::Border {
                        color: if selected { theme::GREEN } else { theme::BASE },
                        width: 1.,
                        radius: 2.into(),
                    },
                    ..Default::default()
                });
            let mut line = row![].spacing(3).align_y(iced::Center);
            if entry.kind != Kind::Request {
                line = line.push(
                    button(ui::icon(
                        if entry.expanded {
                            ui::DOWN
                        } else {
                            ui::CHEVRON
                        },
                        theme::MUTED,
                    ))
                    .padding(4)
                    .on_press(Message::Toggle(entry.id))
                    .style(button::text),
                );
            } else {
                line = line.push(iced::widget::Space::new().width(22));
            }
            line = line.push(select);
            tree = tree.push(container(line).padding(iced::Padding {
                left: (depth.min(8) * 14) as f32,
                ..Default::default()
            }));
            if entry.expanded && entry.kind != Kind::Request {
                tree = tree.push(self.tree(Some(entry.id), depth + 1));
            }
        }
        tree.into()
    }

    fn breadcrumb(&self, id: Id) -> String {
        let mut names = Vec::new();
        let mut current = self.workspace.entry(id);
        while let Some(entry) = current {
            names.push(entry.name.as_str());
            current = entry.parent.and_then(|id| self.workspace.entry(id));
        }
        names.reverse();
        names.join(" / ")
    }

    pub fn view(&self) -> Element<'_, Message> {
        if self.locked {
            return container(container(column![
                ui::label(ui::LOCK, "Workspace locked", theme::BLUE),
                text("Unlock your OS credential store and retry. Your saved data remains on disk.").size(14),
                text(self.storage_error.as_deref().unwrap_or("Could not open secure storage")).size(12).color(theme::YELLOW),
                row![button("Retry unlock").on_press(Message::RetryStorage).padding(10).style(theme::primary),
                    button("Use temporary workspace").on_press(Message::UseTemporaryWorkspace).padding(10).style(theme::control)].spacing(10),
                text("Temporary workspaces stay in memory and do not change your saved data.").size(11).color(theme::MUTED),
            ].spacing(18)).padding(24).max_width(680).style(theme::panel)).center_x(Fill).center_y(Fill).into();
        }
        let collection_count = self
            .workspace
            .entries
            .iter()
            .filter(|e| e.kind == Kind::Collection)
            .count();
        let request_count = self
            .workspace
            .entries
            .iter()
            .filter(|e| e.kind == Kind::Request)
            .count();
        let mut folder = button(ui::label(ui::FOLDER, "Folder", theme::BLUE))
            .padding([8, 10])
            .style(theme::control);
        let mut request = button(ui::label(ui::PLUS, "Request", theme::BLUE))
            .padding([8, 10])
            .style(theme::control);
        if self.selected.is_some() {
            folder = folder.on_press(Message::Create(Kind::Folder));
            request = request.on_press(Message::Create(Kind::Request));
        }
        let mut sidebar = column![
            row![
                ui::icon(ui::BOX, theme::BLUE),
                text("COLLECTIONS").size(12).color(theme::BLUE),
                iced::widget::Space::new().width(Fill),
                text(format!("{collection_count:02}"))
                    .size(12)
                    .color(theme::MUTED)
            ]
            .spacing(8)
            .align_y(iced::Center),
            button(ui::label(ui::PLUS, "New collection", theme::BLUE))
                .width(Fill)
                .padding(10)
                .on_press(Message::Create(Kind::Collection))
                .style(theme::control),
            row![folder, request].spacing(6),
            iced::widget::rule::horizontal(1),
        ]
        .spacing(14)
        .padding(16)
        .width(310);
        if self.workspace.entries.is_empty() {
            sidebar = sidebar.push(column![
                text("~/collections").size(13).color(theme::MUTED),
                text("Nothing here yet.\nCreate your first collection\nto start exploring an API.").size(12).color(theme::MUTED)
            ].spacing(12).padding([16, 0]));
        }
        sidebar = sidebar
            .push(scrollable(self.tree(None, 0)).height(Fill))
            .push(
                text("Select a folder to add requests.")
                    .size(10)
                    .color(theme::MUTED),
            );

        let mut content = column![].spacing(16).padding(22).width(Fill).height(Fill);
        if self.temporary {
            content = content.push(text("SESSION ONLY — nothing in this workspace will be saved. Restart to reopen your saved workspace.").size(11).color(theme::YELLOW));
        }
        if let Some(entry) = self.selected.and_then(|id| self.workspace.entry(id)) {
            content = content
                .push(
                    text(format!("~/{}", self.breadcrumb(entry.id)))
                        .size(12)
                        .color(theme::MUTED),
                )
                .push(
                    row![
                        ui::icon(
                            if entry.kind == Kind::Request {
                                ui::REQUEST
                            } else {
                                ui::FOLDER
                            },
                            theme::BLUE
                        ),
                        text(&entry.name).size(22),
                        iced::widget::Space::new().width(Fill),
                        button(ui::label(ui::EDIT, "Rename", theme::BLUE))
                            .padding([8, 10])
                            .on_press(Message::Rename)
                            .style(theme::control),
                        button(ui::label(ui::TRASH, "Delete", theme::RED))
                            .padding([8, 10])
                            .on_press(Message::Delete)
                            .style(theme::control)
                    ]
                    .spacing(10)
                    .align_y(iced::Center),
                );
            if entry.kind == Kind::Request {
                let response = self.responses.get(&entry.id);
                let loading = response.is_some_and(|r| r.loading);
                let mut send = button(ui::label(
                    ui::ROCKET,
                    if loading { "Sending..." } else { "Send" },
                    theme::MANTLE,
                ))
                .padding([12, 16])
                .style(theme::primary);
                if !loading && !entry.url.trim().is_empty() {
                    send = send.on_press(Message::SendRequest);
                }
                content = content.push(
                    container(
                        column![
                            text("REQUEST").size(11).color(theme::BLUE),
                            row![
                                pick_list(
                                    [
                                        Method::GET,
                                        Method::POST,
                                        Method::PUT,
                                        Method::PATCH,
                                        Method::DELETE,
                                        Method::HEAD,
                                        Method::OPTIONS
                                    ],
                                    Some(entry.method.parse().unwrap_or(Method::GET)),
                                    Message::MethodChanged
                                )
                                .text_size(13)
                                .padding(12),
                                text_input("https://api.example.com", &entry.url)
                                    .on_input(Message::UrlChanged)
                                    .on_submit(Message::SendRequest)
                                    .size(13)
                                    .padding(12)
                                    .style(theme::input)
                                    .width(Fill),
                                send
                            ]
                            .spacing(8)
                            .align_y(iced::Center)
                        ]
                        .spacing(12),
                    )
                    .padding(16)
                    .width(Fill)
                    .style(theme::panel),
                );

                content = content
                    .push(
                        container(self.details.request_view(entry, self.temporary))
                            .height(iced::Length::FillPortion(1)),
                    )
                    .push(
                        container(self.details.response_view(response))
                            .height(iced::Length::FillPortion(1)),
                    );
                if let Some(notice) = &self.details.notice {
                    content = content.push(text(notice).size(11).color(theme::YELLOW));
                }
            } else {
                let children: Vec<_> = self
                    .workspace
                    .entries
                    .iter()
                    .filter(|e| e.parent == Some(entry.id))
                    .collect();
                let mut listing = column![
                    row![
                        ui::icon(ui::OPEN_FOLDER, theme::BLUE),
                        text("CONTENTS").size(12).color(theme::BLUE),
                        iced::widget::Space::new().width(Fill),
                        text(format!("{} items", children.len()))
                            .size(12)
                            .color(theme::MUTED)
                    ]
                    .spacing(10)
                    .align_y(iced::Center),
                    iced::widget::rule::horizontal(1)
                ]
                .spacing(12);
                for child in children {
                    listing = listing.push(
                        button(
                            row![
                                ui::icon(
                                    if child.kind == Kind::Request {
                                        ui::REQUEST
                                    } else {
                                        ui::FOLDER
                                    },
                                    theme::BLUE
                                ),
                                text(&child.name).size(13),
                                iced::widget::Space::new().width(Fill),
                                text(if child.kind == Kind::Request {
                                    child.method.as_str()
                                } else {
                                    "DIR"
                                })
                                .size(12)
                                .color(theme::method_color(&child.method)),
                                ui::icon(ui::CHEVRON, theme::MUTED)
                            ]
                            .spacing(12)
                            .align_y(iced::Center),
                        )
                        .padding(14)
                        .width(Fill)
                        .on_press(Message::Select(child.id))
                        .style(theme::control),
                    );
                }
                if self
                    .workspace
                    .entries
                    .iter()
                    .all(|e| e.parent != Some(entry.id))
                {
                    listing = listing.push(
                        text("This directory is empty. Add a folder or request below.")
                            .size(12)
                            .color(theme::MUTED),
                    );
                }
                listing = listing.push(
                    row![
                        button(ui::label(ui::FOLDER, "New folder", theme::BLUE))
                            .padding(10)
                            .on_press(Message::Create(Kind::Folder))
                            .style(theme::control),
                        button(ui::label(ui::PLUS, "New request", theme::BLUE))
                            .padding(10)
                            .on_press(Message::Create(Kind::Request))
                            .style(theme::control)
                    ]
                    .spacing(8),
                );
                content = content.push(
                    container(scrollable(listing))
                        .padding(18)
                        .height(Fill)
                        .width(Fill)
                        .style(theme::panel),
                );
            }
        } else {
            content = content.push(container(column![
                text("// YOUR API WORKBENCH").size(12).color(theme::BLUE),
                row![text(">_").size(48).color(theme::GREEN), text("RustiQ").size(48)].spacing(18).align_y(iced::Center),
                text("Small requests. Endless possibilities.").size(16).color(theme::TEXT),
                text("Organize your APIs in collections, nest your folders,\nand send your first request. Everything stays local.").size(13).color(theme::MUTED),
                button(ui::label(ui::PLUS, "Create a collection", theme::MANTLE)).padding([12, 18]).on_press(Message::Create(Kind::Collection)).style(theme::primary),
                row![ui::icon(ui::BOX, theme::BLUE), text("Collect").size(12), text(" / ").color(theme::MUTED), ui::icon(ui::FOLDER, theme::BLUE), text("Organize").size(12), text(" / ").color(theme::MUTED), ui::icon(ui::ROCKET, theme::GREEN), text("Explore").size(12)].spacing(8).align_y(iced::Center)
            ].spacing(24)).center_x(Fill).center_y(Fill).style(theme::panel));
        }
        if let Some(error) = &self.storage_error {
            content = content.push(
                column![
                    text(error).size(12).color(theme::RED),
                    row![
                        button("Retry save")
                            .on_press(Message::RetryStorage)
                            .style(theme::control),
                        button("Keep this session in memory")
                            .on_press(Message::UseTemporaryWorkspace)
                            .style(theme::control)
                    ]
                    .spacing(8)
                ]
                .spacing(6),
            );
        }
        let mut lock_button = button(ui::label(ui::LOCK, "Lock", theme::BLUE))
            .padding([6, 10])
            .style(theme::control);
        if !self.temporary && self.storage_error.is_none() {
            lock_button = lock_button.on_press(Message::LockWorkspace);
        }
        let header = container(
            row![
                ui::icon(ui::TERMINAL, theme::BLUE),
                text("RustiQ").size(18).color(theme::BLUE),
                text("/ API WORKSPACE").size(11).color(theme::MUTED),
                iced::widget::Space::new().width(Fill),
                lock_button,
                ui::icon(ui::CODE, theme::GREEN),
                text("100% open source").size(11).color(theme::MUTED)
            ]
            .spacing(12)
            .align_y(iced::Center),
        )
        .padding([14, 20])
        .width(Fill)
        .style(theme::bar);
        let footer = container(
            row![
                ui::icon(
                    if self.storage_error.is_some() {
                        ui::TRASH
                    } else {
                        ui::CHECK
                    },
                    if self.storage_error.is_some() {
                        theme::RED
                    } else {
                        theme::GREEN
                    }
                ),
                text(if self.storage_error.is_some() {
                    "Storage error"
                } else {
                    if self.temporary {
                        "Session only"
                    } else {
                        "Encrypted workspace"
                    }
                })
                .size(11)
                .color(theme::MUTED),
                text(format!(
                    "| {collection_count} collections · {request_count} requests"
                ))
                .size(11)
                .color(theme::MUTED),
                iced::widget::Space::new().width(Fill),
                ui::icon(ui::HEART, theme::MAUVE),
                text("Macchiato").size(11).color(theme::BLUE),
                text(concat!("v", env!("CARGO_PKG_VERSION")))
                    .size(11)
                    .color(theme::MUTED)
            ]
            .spacing(10)
            .align_y(iced::Center),
        )
        .padding([10, 20])
        .width(Fill)
        .style(theme::bar);
        let layout: Element<'_, Message> = column![
            header,
            iced::widget::rule::horizontal(1),
            row![container(sidebar).height(Fill).style(theme::panel), content].height(Fill),
            iced::widget::rule::horizontal(1),
            footer
        ]
        .height(Fill)
        .into();

        if let Some(dialog) = &self.dialog {
            let panel: Element<'_, Message> = match dialog {
                Dialog::LockWorkspace => column![
                    ui::label(ui::LOCK, "Lock workspace?", theme::BLUE),
                    text("Saved requests stay encrypted. Session-only configuration and responses will be discarded.").size(13),
                    row![button("Cancel").on_press(Message::CancelDialog).padding(10).style(theme::control), button("Lock").on_press(Message::ConfirmLock).padding(10).style(theme::primary)].spacing(10)
                ].spacing(18).into(),
                Dialog::Name {
                    id,
                    kind,
                    parent,
                    value,
                } => {
                    let title = format!(
                        "{} {}",
                        if id.is_some() { "Rename" } else { "Create" },
                        match kind {
                            Kind::Collection => "collection",
                            Kind::Folder => "folder",
                            Kind::Request => "request",
                        }
                    );
                    let mut save = button(ui::label(ui::CHECK, "Save", theme::MANTLE))
                        .padding([10, 18])
                        .style(theme::primary);
                    if !value.trim().is_empty() {
                        save = save.on_press(Message::ConfirmName);
                    }
                    column![
                        ui::label(ui::EDIT, title, theme::BLUE),
                        text(
                            parent
                                .and_then(|id| self.workspace.entry(id))
                                .map_or("~/collections".into(), |e| format!("In: {}", e.name))
                        )
                        .size(12)
                        .color(theme::MUTED),
                        text_input("Enter a name...", value)
                            .id("entry-name")
                            .on_input(Message::NameChanged)
                            .on_submit(Message::ConfirmName)
                            .padding(12)
                            .size(14)
                            .style(theme::input),
                        row![
                            iced::widget::Space::new().width(Fill),
                            button("Cancel")
                                .padding([10, 14])
                                .on_press(Message::CancelDialog)
                                .style(theme::control),
                            save
                        ]
                        .spacing(8)
                    ]
                    .spacing(18)
                    .into()
                }
                Dialog::Delete(id) => {
                    let name = self.workspace.entry(*id).map_or("", |e| e.name.as_str());
                    column![
                        ui::label(ui::TRASH, "Delete item", theme::RED),
                        text(format!("Delete \"{name}\"? ")).size(15),
                        text("All nested folders and requests will also be deleted.")
                            .size(12)
                            .color(theme::MUTED),
                        row![
                            iced::widget::Space::new().width(Fill),
                            button("Cancel")
                                .padding(10)
                                .on_press(Message::CancelDialog)
                                .style(theme::control),
                            button("Delete")
                                .padding(10)
                                .on_press(Message::ConfirmDelete)
                                .style(button::danger)
                        ]
                        .spacing(8)
                    ]
                    .spacing(18)
                    .into()
                }
            };
            iced::widget::stack![
                layout,
                iced::widget::opaque(
                    container(container(panel).padding(24).width(520).style(theme::panel))
                        .center_x(Fill)
                        .center_y(Fill)
                        .style(|_| container::Style {
                            background: Some(iced::Color::from_rgba(0.06, 0.07, 0.12, 0.85).into()),
                            ..Default::default()
                        })
                )
            ]
            .into()
        } else {
            layout
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_app() -> App {
        App {
            workspace: Workspace::default(),
            details: Details::default(),
            selected: None,
            responses: HashMap::new(),
            pending: HashMap::new(),
            dialog: None,
            path: PathBuf::new(),
            storage_error: None,
            storage: None,
            locked: false,
            temporary: true,
        }
    }
    #[test]
    fn locking_blocks_edits_and_discards_session_configuration() {
        let mut app = session_app();
        app.temporary = false;
        let collection = app.workspace.create(Kind::Collection, None, "API").unwrap();
        let id = app
            .workspace
            .create(Kind::Request, Some(collection), "Request")
            .unwrap();
        app.workspace.entry_mut(id).unwrap().request.auth.token = "private-token".into();
        app.selected = Some(id);
        let _ = app.update(Message::LockWorkspace);
        assert!(!app.workspace.entries.is_empty());
        let _ = app.update(Message::ConfirmLock);
        assert!(app.workspace.entries.is_empty());
        assert!(app.locked);
        let _ = app.view();
        let _ = app.update(Message::Create(Kind::Collection));
        assert!(app.dialog.is_none());
        let _ = app.update(Message::UseTemporaryWorkspace);
        assert!(!app.locked);
        assert!(app.temporary);
        let _ = app.update(Message::Create(Kind::Collection));
        assert!(app.dialog.is_some());
    }
    #[test]
    fn debug_messages_redact_secrets() {
        for message in [
            Message::UrlChanged("url-secret".into()),
            Message::Edit(crate::details::Edit::AuthField(
                crate::details::AuthField::Token,
                "token-secret".into(),
            )),
            Message::NameChanged("name-secret".into()),
        ] {
            assert!(!format!("{message:?}").contains("-secret"));
        }
    }

    #[test]
    fn edits_and_async_responses_stay_with_their_request() {
        let mut workspace = Workspace::default();
        let collection = workspace.create(Kind::Collection, None, "API").unwrap();
        let first = workspace
            .create(Kind::Request, Some(collection), "First")
            .unwrap();
        let second = workspace
            .create(Kind::Request, Some(collection), "Second")
            .unwrap();
        let mut app = App {
            workspace,
            details: Details::default(),
            selected: Some(first),
            responses: HashMap::new(),
            pending: HashMap::new(),
            dialog: None,
            path: PathBuf::new(),
            storage_error: None,
            storage: None,
            locked: false,
            temporary: false,
        };
        let _ = app.update(Message::UrlChanged("https://example.com/first".into()));
        let _ = app.update(Message::MethodChanged(Method::POST));
        let _ = app.update(Message::Select(second));
        let _ = app.update(Message::RequestFinished(
            first,
            Ok(rustiq_http::HttpResponse {
                status: 201,
                body: "created".into(),
                ..Default::default()
            }),
        ));
        assert_eq!(app.workspace.entry(first).unwrap().method, "POST");
        assert_eq!(
            app.workspace.entry(first).unwrap().url,
            "https://example.com/first"
        );
        assert!(app.workspace.entry(second).unwrap().url.is_empty());
        assert_eq!(app.responses[&first].status, Some(201));
        assert!(!app.responses.contains_key(&second));
        let _ = app.update(Message::Select(collection));
        let _ = app.update(Message::Delete);
        assert_eq!(app.workspace.entries.len(), 3);
        let _ = app.update(Message::ConfirmDelete);
        assert!(app.workspace.entries.is_empty());
        assert!(app.responses.is_empty());
        let replacement = app.workspace.create(Kind::Collection, None, "New").unwrap();
        assert!(replacement > second);
    }
}
