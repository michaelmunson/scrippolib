//! `scrippolib`: install, update and remove scrippo packages and scripts.
pub mod config;
pub mod install;
pub mod registry;
pub mod source;
pub mod store;

use scriptopotamus::paths::NewKind;

pub fn kind_dir(kind: NewKind) -> &'static str {
    match kind {
        NewKind::Package => "packages",
        NewKind::Script => "scripts",
    }
}

pub fn kind_name(kind: NewKind) -> &'static str {
    match kind {
        NewKind::Package => "package",
        NewKind::Script => "script",
    }
}
