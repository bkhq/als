//! `als unpin <key>` — drop a local path -> site binding from
//! `~/.config/als/sites/<name>.toml` without touching the server.

use als_core::{Error, Output, sites};

use crate::cli::UnpinArgs;

pub(crate) fn run(out: &mut Output, args: &UnpinArgs) -> Result<(), Error> {
    let removed = sites::remove_by_key(&args.key)?;

    if removed.is_empty() {
        out.quiet_line("No matching pin found.");
        return Ok(());
    }

    for entry in removed {
        out.quiet_line(&format!(
            "\x1b[32m✓\x1b[0m Unpinned {} (was bound to {} \"{}\").",
            entry.path, entry.id, entry.name
        ));
    }
    Ok(())
}
