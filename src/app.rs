// SPDX-License-Identifier: MPL-2.0

use std::{collections::HashSet, time::Duration};

use cosmic::iced::{
    Alignment, Length, Limits, Subscription,
    core::text::EllipsizeHeightLimit,
    platform_specific::shell::wayland::commands::popup::{destroy_popup, get_popup},
    widget::text::{Ellipsize, Wrapping},
    window::Id,
};
use cosmic::prelude::*;
use cosmic::widget;
use humansize::{BINARY, format_size};

use crate::{
    fl,
    process::{self, WorkloadInfo, WorkloadKey},
};

const REFRESH_INTERVAL: Duration = Duration::from_secs(3);
type Command = Task<cosmic::Action<Message>>;

#[derive(Default)]
pub struct AppModel {
    core: cosmic::Core,
    popup: Option<Id>,
    workloads: Vec<WorkloadInfo>,
    error: Option<String>,
    refreshing: bool,
    killing: HashSet<WorkloadKey>,
}

#[derive(Clone, Debug)]
pub enum Message {
    TogglePopup,
    PopupClosed(Id),
    RefreshTick,
    RefreshFinished(Result<Vec<WorkloadInfo>, String>),
    Kill(WorkloadKey),
    KillFinished(WorkloadKey, String, Result<(), String>),
}

impl cosmic::Application for AppModel {
    type Executor = cosmic::executor::Default;
    type Flags = ();
    type Message = Message;

    const APP_ID: &'static str = "com.github.radutomy.cosmic-process-applet";

    fn core(&self) -> &cosmic::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::Core {
        &mut self.core
    }

    fn init(core: cosmic::Core, _flags: Self::Flags) -> (Self, Command) {
        (
            Self {
                core,
                ..Self::default()
            },
            Task::none(),
        )
    }

    fn on_close_requested(&self, id: Id) -> Option<Message> {
        Some(Message::PopupClosed(id))
    }

    fn view(&self) -> Element<'_, Self::Message> {
        self.core
            .applet
            .icon_button("utilities-system-monitor-symbolic")
            .on_press(Message::TogglePopup)
            .into()
    }

    fn view_window(&self, _id: Id) -> Element<'_, Self::Message> {
        let spacing = cosmic::theme::active().cosmic().spacing;
        let mut content = cosmic::iced::widget::column![
            widget::text::title4(fl!("popup-title")),
            widget::divider::horizontal::default()
        ]
        .spacing(spacing.space_xxxs);

        if self.workloads.is_empty() {
            content = content.push(widget::text(if self.refreshing {
                fl!("loading")
            } else {
                fl!("empty-list")
            }));
        } else {
            for workload in &self.workloads {
                let is_killing = self.killing.contains(&workload.key);
                let detail = if workload.members.len() == 1 {
                    fl!("pid", pid = workload.members[0].pid)
                } else {
                    fl!("process-count", count = workload.members.len())
                };
                let details = cosmic::iced::widget::column![
                    widget::text(&workload.name)
                        .width(Length::Fill)
                        .wrapping(Wrapping::None)
                        .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1))),
                    widget::text::caption(detail)
                ]
                .spacing(0);
                let memory = format_size(workload.memory, BINARY);
                let memory = if workload.approximate_memory {
                    format!("~{memory}")
                } else {
                    memory
                };
                let kill_button = widget::button::destructive(if is_killing {
                    fl!("killing")
                } else {
                    fl!("kill")
                })
                .on_press_maybe((!is_killing).then(|| Message::Kill(workload.key.clone())));

                let row = cosmic::iced::widget::row![
                    details.width(Length::Fill),
                    widget::text(memory),
                    kill_button
                ]
                .align_y(Alignment::Center)
                .spacing(spacing.space_xs);

                content = content.push(widget::container(row).padding([2, 0]));
            }
        }

        if let Some(error) = &self.error {
            content = content
                .push(widget::divider::horizontal::default())
                .push(widget::text(error));
        }

        self.core
            .applet
            .popup_container(content.padding(spacing.space_xs))
            .into()
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        if self.popup.is_some() {
            cosmic::iced::time::every(REFRESH_INTERVAL).map(|_| Message::RefreshTick)
        } else {
            Subscription::none()
        }
    }

    fn update(&mut self, message: Self::Message) -> Command {
        match message {
            Message::TogglePopup => {
                if let Some(popup) = self.popup.take() {
                    return destroy_popup(popup);
                }

                let new_id = Id::unique();
                self.popup = Some(new_id);
                let mut settings = self.core.applet.get_popup_settings(
                    self.core
                        .main_window_id()
                        .expect("applet has a main window"),
                    new_id,
                    None,
                    None,
                    None,
                );
                settings.positioner.size_limits = Limits::NONE
                    .min_width(350.0)
                    .max_width(440.0)
                    .min_height(160.0)
                    .max_height(640.0);

                Task::batch([get_popup(settings), self.start_refresh()])
            }
            Message::PopupClosed(id) => {
                if self.popup == Some(id) {
                    self.popup = None;
                }
                Task::none()
            }
            Message::RefreshTick => self.start_refresh(),
            Message::RefreshFinished(result) => {
                self.refreshing = false;
                match result {
                    Ok(mut workloads) => {
                        workloads.retain(|workload| !self.killing.contains(&workload.key));
                        self.workloads = workloads;
                        self.error = None;
                    }
                    Err(error) => {
                        eprintln!("process refresh failed: {error}");
                        self.error = Some(fl!("scan-failed", error = error));
                    }
                }
                Task::none()
            }
            Message::Kill(key) => {
                match self
                    .workloads
                    .iter()
                    .find(|workload| workload.key == key)
                    .cloned()
                {
                    Some(workload) if self.killing.insert(key) => kill_task(workload),
                    _ => Task::none(),
                }
            }
            Message::KillFinished(key, name, result) => {
                self.killing.remove(&key);
                if let Err(error) = result {
                    eprintln!("could not kill {name}: {error}");
                    self.error = Some(fl!("kill-failed", name = name, error = error));
                    return Task::none();
                }
                self.workloads.retain(|workload| workload.key != key);
                self.error = None;
                if self.popup.is_some() {
                    self.start_refresh()
                } else {
                    Task::none()
                }
            }
        }
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}

impl AppModel {
    fn start_refresh(&mut self) -> Command {
        if self.refreshing {
            Task::none()
        } else {
            self.refreshing = true;
            blocking(process::scan_workloads, Message::RefreshFinished)
        }
    }
}

fn kill_task(workload: WorkloadInfo) -> Command {
    let key = workload.key.clone();
    let name = workload.name.clone();
    blocking(
        move || process::kill_workload(&workload),
        move |result| Message::KillFinished(key, name, result),
    )
}

fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
    done: impl FnOnce(Result<T, String>) -> Message + Send + 'static,
) -> Command {
    cosmic::task::future(async move {
        let result = tokio::task::spawn_blocking(work)
            .await
            .map_err(|error| format!("background worker failed: {error}"))
            .and_then(|result| result);
        done(result)
    })
}
