//! Names and paths Meson passes to Cargo at compile time.
//!
//! A plain Cargo build leaves them unset. That is enough for checks and
//! tests, but the app itself has to be built with Meson.

#![expect(
    clippy::option_env_unwrap,
    reason = "plain Cargo builds must compile, only running them needs Meson"
)]

const UNSET: &str = "BitLog has to be built with Meson";

pub fn app_id() -> &'static str {
    option_env!("MESON_APP_ID").expect(UNSET)
}

pub fn gettext_package() -> &'static str {
    option_env!("MESON_GETTEXT_PACKAGE").expect(UNSET)
}

pub fn localedir() -> &'static str {
    option_env!("MESON_LOCALEDIR").expect(UNSET)
}

pub fn resources_file() -> &'static str {
    option_env!("MESON_RESOURCES_FILE").expect(UNSET)
}

pub fn version() -> &'static str {
    option_env!("MESON_VERSION").expect(UNSET)
}
