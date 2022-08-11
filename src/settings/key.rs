use strum_macros::*;

#[derive(Display, Debug, Clone, EnumString)]
#[strum(serialize_all = "kebab_case")]
pub enum Key {
    // Client Backend
    ApiLookupDomain,

    // User Interface
    DarkMode,
    Notifications,
    WindowWidth,
    WindowHeight,
    IsMaximized,
}
