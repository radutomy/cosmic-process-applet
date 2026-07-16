// SPDX-License-Identifier: MPL-2.0

mod app;
mod i18n;
mod process;

fn main() -> cosmic::iced::Result {
    i18n::init(&i18n_embed::DesktopLanguageRequester::requested_languages());
    cosmic::applet::run::<app::AppModel>(())
}
