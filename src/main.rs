use std::{collections::HashSet, fmt::Write as _, path::PathBuf, process::Command};

use anyhow::Context;
use clap::Parser;

#[derive(clap::Parser)]
struct App {
    #[command(subcommand)]
    subcommand: Subcommand,
}

#[derive(clap::Subcommand, Clone, Debug)]
enum Subcommand {
    Archive { name: String },
    Restore { name: String },
    List,
}

fn session_name(line: &str) -> Option<&str> {
    let mut it = line.split_whitespace();
    let _pane = it.next();
    it.next()
}

fn is_state_line(line: &str) -> bool {
    line.split_whitespace().next() == Some("state")
}

fn get_sessions(s: &str) -> HashSet<&str> {
    s.lines().filter_map(session_name).collect()
}

/// returns (matching_session, not_matching_session)
fn get_entries_with_session_name<'a>(s: &'a str, name: &str) -> (Vec<&'a str>, Vec<&'a str>) {
    s.lines().partition(|line| session_name(line) == Some(name))
}

fn append_lines(lines: Vec<&str>, mut res: String) -> String {
    for line in lines {
        let _ = writeln!(&mut res, "{line}");
    }
    res
}

fn checked_run(c: &mut Command) -> anyhow::Result<String> {
    let o = c.output().context("failed to spawn save script")?;
    if !o.status.success() {
        anyhow::bail!("failed to save session");
    }

    Ok(String::from_utf8(o.stdout)?)
}

fn main() -> anyhow::Result<()> {
    let app = App::parse();

    let home = PathBuf::from(std::env::var("HOME").expect("have home"));
    let resurrect_file = home.join(".local/share/tmux/resurrect/last");
    let archive_file = home.join(".local/share/tmux/resurrect/archived");

    let save = home.join(".config/tmux/plugins/tmux-resurrect/scripts/save.sh");
    let restore = home.join(".config/tmux/plugins/tmux-resurrect/scripts/restore.sh");

    let active = std::fs::read(&resurrect_file)
        .ok()
        .and_then(|x| String::from_utf8(x).ok())
        .unwrap_or(String::new());

    let archived = std::fs::read(&archive_file)
        .ok()
        .and_then(|x| String::from_utf8(x).ok())
        .unwrap_or(String::new());

    match app.subcommand {
        Subcommand::Archive { name } => {
            checked_run(&mut Command::new(save)).context("failed to save session")?;

            let (mut to_archive, keep) = get_entries_with_session_name(&active, &name);
            let fallback_session = keep.iter().copied().find_map(session_name);

            let mut active = append_lines(keep, String::new());
            if let Some((state, _)) = to_archive
                .iter()
                .enumerate()
                .find(|&(_, x)| is_state_line(x))
            {
                to_archive.remove(state);
                if let Some(fallback_session) = fallback_session {
                    // XXX: we potentially need two fallback sessions
                    writeln!(&mut active, "state {fallback_session}")?;
                }
            }

            let archived = append_lines(to_archive, archived);

            std::fs::write(&resurrect_file, active)?;
            std::fs::write(&archive_file, archived)?;

            if let Some(fallback_session) = fallback_session
                && checked_run(Command::new("tmux").args(["display-message", "-p", "#S"]))?.trim()
                    == name
            {
                checked_run(
                    Command::new("tmux")
                        .args(["switch", "-t"])
                        .arg(fallback_session),
                )
                .context("failed to switch session")?;
            }

            checked_run(Command::new("tmux").args(["kill-session", "-t"]).arg(&name))
                .context("failed to remove session from active")?;
        }
        Subcommand::Restore { name } => {
            let (pull_from_archive, keep_in_archive) =
                get_entries_with_session_name(&archived, &name);

            let active = append_lines(pull_from_archive, String::new()) + &active;
            let archived = append_lines(keep_in_archive, String::new());

            std::fs::write(&resurrect_file, active)?;
            std::fs::write(&archive_file, archived)?;

            checked_run(&mut Command::new(restore)).context("failed to restore with new state")?;
            checked_run(Command::new("tmux").args(["switch", "-t"]).arg(name))
                .context("couldn't switch to new session")?;
        }
        Subcommand::List => {
            let mut sessions = vec![];
            sessions.extend(get_sessions(&active).into_iter().map(|x| (x, true)));
            sessions.extend(get_sessions(&archived).into_iter().map(|x| (x, false)));
            sessions.sort();
            let longest_session_name = sessions.iter().map(|(x, _)| x.len()).max().unwrap_or(0);
            for (session, active) in sessions {
                println!(
                    "{session:<longest_session_name$}   {}",
                    if active { "(active)" } else { "(ARCHIVED)" },
                );
            }
        }
    }

    Ok(())
}
