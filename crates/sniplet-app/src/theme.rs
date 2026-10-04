use gpui_kit::{
    App, Context, Subscription, Window, WindowAppearance,
    component::{Theme, ThemeMode},
};
use sniplet_platform::ThemePreference;

pub(crate) fn apply(preference: ThemePreference, window: &mut Window, cx: &mut App) {
    Theme::change(mode(preference, window.appearance()), Some(window), cx);
}

pub(crate) fn observe<T: 'static>(
    window: &mut Window,
    cx: &Context<T>,
    preference: impl Fn(&T) -> ThemePreference + 'static,
) -> Subscription {
    cx.observe_window_appearance(window, move |owner, window, cx| {
        appearance_changed(preference(owner), window, cx);
    })
}

fn appearance_changed(preference: ThemePreference, window: &mut Window, cx: &mut App) {
    if preference == ThemePreference::System {
        apply(ThemePreference::System, window, cx);
    }
}

fn mode(preference: ThemePreference, appearance: WindowAppearance) -> ThemeMode {
    match preference {
        ThemePreference::System => appearance.into(),
        ThemePreference::Light => ThemeMode::Light,
        ThemePreference::Dark => ThemeMode::Dark,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_tracks_all_system_appearances() {
        assert_eq!(
            mode(ThemePreference::System, WindowAppearance::Light),
            ThemeMode::Light
        );
        assert_eq!(
            mode(ThemePreference::System, WindowAppearance::VibrantLight),
            ThemeMode::Light
        );
        assert_eq!(
            mode(ThemePreference::System, WindowAppearance::Dark),
            ThemeMode::Dark
        );
        assert_eq!(
            mode(ThemePreference::System, WindowAppearance::VibrantDark),
            ThemeMode::Dark
        );
    }

    #[test]
    fn explicit_preferences_ignore_system_appearance() {
        for appearance in [
            WindowAppearance::Light,
            WindowAppearance::VibrantLight,
            WindowAppearance::Dark,
            WindowAppearance::VibrantDark,
        ] {
            assert_eq!(mode(ThemePreference::Light, appearance), ThemeMode::Light);
            assert_eq!(mode(ThemePreference::Dark, appearance), ThemeMode::Dark);
        }
    }

    #[cfg(feature = "ui-tests")]
    mod ui {
        use super::*;
        use gpui_kit::{AppContext as _, IntoElement, Render, TestAppContext, div};

        struct Owner {
            preference: ThemePreference,
            _subscription: Subscription,
        }

        impl Render for Owner {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div()
            }
        }

        #[gpui_kit::test]
        fn appearance_callback_tracks_system_and_ignores_explicit_preferences(
            cx: &mut TestAppContext,
        ) {
            let window = cx.update(|cx| {
                gpui_kit::init(cx);
                cx.open_window(Default::default(), |window, cx| {
                    cx.new(|cx| {
                        let preference = ThemePreference::System;
                        apply(preference, window, cx);
                        Owner {
                            preference,
                            _subscription: observe(window, cx, |owner: &Owner| owner.preference),
                        }
                    })
                })
                .unwrap()
            });

            window
                .update(cx, |owner, window, cx| {
                    Theme::change(ThemeMode::Dark, None, cx);
                    appearance_changed(owner.preference, window, cx);
                    assert_eq!(Theme::global(cx).mode, ThemeMode::Light);

                    owner.preference = ThemePreference::Dark;
                    Theme::change(ThemeMode::Dark, None, cx);
                    appearance_changed(owner.preference, window, cx);
                    assert_eq!(Theme::global(cx).mode, ThemeMode::Dark);

                    owner.preference = ThemePreference::System;
                    apply(owner.preference, window, cx);
                    assert_eq!(Theme::global(cx).mode, ThemeMode::Light);
                })
                .unwrap();
        }
    }
}
