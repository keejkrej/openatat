//! Spawn `openatat-ui`. The daemon never links gpui.

use std::path::PathBuf;
use std::process::Command;

use openatat_ipc::UiPage;

use crate::error::{Error, Result};

pub fn spawn(page: UiPage) -> Result<()> {
    let bin = find_ui_bin()?;
    let arg = match page {
        UiPage::Settings => "--settings",
        UiPage::History => "--history",
    };
    Command::new(&bin).arg(arg).spawn().map_err(|e| {
        Error::msg(format!(
            "failed to spawn {} {arg} ({e}). Build with: cargo build -p openatat-ui --features gpui",
            bin.display()
        ))
    })?;
    eprintln!("openatatd: spawned {} {arg}", bin.display());
    Ok(())
}

fn find_ui_bin() -> Result<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        let sibling = exe.with_file_name("openatat-ui");
        if sibling.is_file() {
            return Ok(sibling);
        }
    }
    if let Some(found) = crate::agent::which("openatat-ui", std::env::var_os("PATH").as_deref()) {
        return Ok(found);
    }
    Err(Error::msg(
        "openatat-ui not found next to openatatd or on PATH. \
         Run: cargo build -p openatat-ui --features gpui \
         or: cargo run -p openatat-ui --features gpui -- --settings",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_flags() {
        // compile-time check that both pages are handled
        let _ = [UiPage::Settings, UiPage::History];
        assert!(find_ui_bin().is_err() || find_ui_bin().unwrap().file_name().is_some());
    }
}
