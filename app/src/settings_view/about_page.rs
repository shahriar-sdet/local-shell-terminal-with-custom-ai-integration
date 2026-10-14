use warpui::assets::asset_cache::AssetSource;
use warpui::elements::{
    Align, CacheOption, ConstrainedBox, Container, CrossAxisAlignment, Element, Flex, Image,
    MainAxisAlignment, MouseStateHandle, ParentElement, Wrap,
};
use warpui::fonts::Weight;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{AppContext, Entity, TypedActionView, View, ViewContext, ViewHandle};

use super::SettingsSection;
use super::settings_page::{
    CONTENT_FONT_SIZE, MatchData, PageType, SettingsPageEvent, SettingsPageMeta,
    SettingsPageViewHandle, SettingsWidget,
};
use crate::appearance::Appearance;
use crate::channel::ChannelState;
use crate::themes::theme::ColorScheme;
use crate::workspace::WorkspaceAction;

/// The AGPL's "Appropriate Legal Notices" (section 0): copyright, the absence of warranty, the
/// right to redistribute, and where to read the license.
const LOCAL_SHELL_NOTICES: &[&str] = &[
    "Based on Warp, © 2020–2026 Denver Technologies, Inc. Modifications © 2026 shahriar-sdet.",
    "LocalShell is an independent project, not affiliated with or endorsed by Denver \
     Technologies, Inc. \"Warp\" is a trademark of Denver Technologies, Inc.",
    "This program is free software: you can redistribute it and/or modify it under the terms of \
     the GNU Affero General Public License, version 3. Its UI framework (warpui, warpui_core) is \
     under the MIT license.",
    "It comes with ABSOLUTELY NO WARRANTY, to the extent permitted by applicable law.",
    "The license texts (LICENSE-AGPL, LICENSE-MIT) and the notices of the open-source components \
     it includes (THIRD_PARTY_LICENSES.txt) are in LocalShell.app/Contents/Resources. The AGPL is \
     also at https://www.gnu.org/licenses/agpl-3.0.html.",
    "Source code: https://github.com/shahriar-sdet/warp-with-custom-local-ai",
];

const LOCAL_SHELL_NOTICES_MAX_WIDTH: f32 = 520.;

pub struct AboutPageView {
    page: PageType<Self>,
}

impl AboutPageView {
    pub fn new(_ctx: &mut ViewContext<AboutPageView>) -> Self {
        AboutPageView {
            page: PageType::new_monolith(AboutPageWidget::default(), None, false),
        }
    }
}

impl Entity for AboutPageView {
    type Event = SettingsPageEvent;
}
impl TypedActionView for AboutPageView {
    type Action = ();
}

impl View for AboutPageView {
    fn ui_name() -> &'static str {
        "AboutPage"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        self.page.render(self, app)
    }
}

#[derive(Default)]
struct AboutPageWidget {
    copy_version_button_mouse_state: MouseStateHandle,
}

impl SettingsWidget for AboutPageWidget {
    type View = AboutPageView;

    fn search_terms(&self) -> &str {
        "about warp localshell version license copyright warranty source"
    }

    fn render(
        &self,
        _view: &AboutPageView,
        appearance: &Appearance,
        _app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let ui_builder = appearance.ui_builder();

        let version = ChannelState::app_version().unwrap_or("v#.##.###");

        let version_text = ui_builder
            .span(version.to_string())
            .with_soft_wrap()
            .build()
            .with_margin_top(16.)
            .finish();

        let copy_version_icon = appearance
            .ui_builder()
            .copy_button(16., self.copy_version_button_mouse_state.clone())
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(WorkspaceAction::CopyVersion(version));
            })
            .finish();

        let version_row = Wrap::row()
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_children([
                version_text,
                Container::new(copy_version_icon)
                    .with_margin_top(16.)
                    .with_padding_left(6.)
                    .finish(),
            ]);

        if ChannelState::is_local_shell() {
            return render_local_shell_about(version_row.finish(), appearance);
        }

        let image_path = if theme.inferred_color_scheme() == ColorScheme::LightOnDark {
            "bundled/svg/warp-logo-with-light-title.svg"
        } else {
            "bundled/svg/warp-logo-with-dark-title.svg"
        };

        Align::new(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    ConstrainedBox::new(
                        Image::new(
                            AssetSource::Bundled { path: image_path },
                            CacheOption::BySize,
                        )
                        .finish(),
                    )
                    .with_max_height(100.)
                    .with_max_width(350.)
                    .finish(),
                )
                .with_child(version_row.finish())
                .with_child(
                    ui_builder
                        .span("Copyright 2026 Warp")
                        .build()
                        .with_margin_top(16.)
                        .finish(),
                )
                .finish(),
        )
        .finish()
    }
}

fn render_local_shell_about(
    version_row: Box<dyn Element>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let ui_builder = appearance.ui_builder();

    let icon = ConstrainedBox::new(
        Image::new(
            AssetSource::Bundled {
                path: "bundled/svg/localshell-icon.svg",
            },
            CacheOption::BySize,
        )
        .finish(),
    )
    .with_max_height(96.)
    .with_max_width(96.)
    .finish();

    let name = ui_builder
        .span("LocalShell")
        .with_style(
            UiComponentStyles::default()
                .set_font_weight(Weight::Bold)
                .set_font_size(20.),
        )
        .build()
        .with_margin_top(12.)
        .finish();

    let notice_style = UiComponentStyles {
        font_size: Some(CONTENT_FONT_SIZE),
        font_color: Some(theme.sub_text_color(theme.surface_2()).into_solid()),
        ..Default::default()
    };
    let notices = LOCAL_SHELL_NOTICES.iter().map(|notice| {
        ui_builder
            .span(*notice)
            .with_soft_wrap()
            .with_selectable(true)
            .with_style(notice_style)
            .build()
            .with_margin_top(10.)
            .finish()
    });

    Align::new(
        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(icon)
            .with_child(name)
            .with_child(version_row)
            .with_child(
                Container::new(
                    ConstrainedBox::new(Flex::column().with_children(notices).finish())
                        .with_max_width(LOCAL_SHELL_NOTICES_MAX_WIDTH)
                        .finish(),
                )
                .with_margin_top(14.)
                .finish(),
            )
            .finish(),
    )
    .finish()
}

impl SettingsPageMeta for AboutPageView {
    fn section() -> SettingsSection {
        SettingsSection::About
    }

    fn should_render(&self, _ctx: &AppContext) -> bool {
        true
    }

    fn update_filter(&mut self, query: &str, ctx: &mut ViewContext<Self>) -> MatchData {
        self.page.update_filter(query, ctx)
    }

    fn scroll_to_widget(&mut self, widget_id: &'static str) {
        self.page.scroll_to_widget(widget_id)
    }

    fn clear_highlighted_widget(&mut self) {
        self.page.clear_highlighted_widget();
    }
}

impl From<ViewHandle<AboutPageView>> for SettingsPageViewHandle {
    fn from(view_handle: ViewHandle<AboutPageView>) -> Self {
        SettingsPageViewHandle::About(view_handle)
    }
}
