//! Custom timing follows the pinned Criterion harness's invocation mode.

use std::ffi::OsStr;

pub(crate) fn enabled() -> bool {
    enabled_for(!cfg!(debug_assertions), std::env::args_os().skip(1))
}

pub(crate) fn enabled_for<I, S>(optimized: bool, arguments: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    if !optimized {
        return false;
    }
    let mut bench = false;
    for argument in arguments {
        let argument = argument.as_ref();
        if argument == "--" {
            break;
        }
        if argument == "--test"
            || argument == "--list"
            || argument == "--ignored"
            || argument == "--profile-time"
            || argument
                .to_str()
                .is_some_and(|text| text.starts_with("--profile-time="))
        {
            return false;
        }
        bench |= argument == "--bench";
    }
    bench
}
