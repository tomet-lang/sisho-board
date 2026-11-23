//! Editor resolution for opening broken link files from `sisho-board`.

use std::path::Path;

use crate::model::ExtensionItem;
use crate::runner::Step;
use crate::trust::TrustState;

/// Resolve an editor command to open `file` at `line`.
///
/// Precedence chain:
/// 1. `$SISHO_EDITOR` (handles `{file}`, `{line}`, `{col}` templates, allowed extension words, or commands)
/// 2. `$VISUAL` (Unix full-screen editor: nvim, vim, hx, etc.)
/// 3. `$EDITOR` (Unix standard editor)
/// 4. `sisho-desk` (if installed, active, and [`TrustState::Allowed`])
/// 5. `nano` (system fallback)
pub fn resolve_editor(
    file: &Path,
    line: Option<usize>,
    column: Option<usize>,
    extensions: &[ExtensionItem],
) -> Step {
    resolve_editor_with_lookup(file, line, column, extensions, |key| {
        std::env::var(key).ok()
    })
}

pub fn resolve_editor_with_lookup<F>(
    file: &Path,
    line: Option<usize>,
    column: Option<usize>,
    extensions: &[ExtensionItem],
    lookup: F,
) -> Step
where
    F: Fn(&str) -> Option<String>,
{
    // 1. $SISHO_EDITOR
    if let Some(cmd) = lookup("SISHO_EDITOR").filter(|s| !s.trim().is_empty()) {
        return build_from_sisho_editor(&cmd, file, line, column, extensions);
    }

    // 2. $VISUAL
    if let Some(cmd) = lookup("VISUAL").filter(|s| !s.trim().is_empty()) {
        return build_standard_editor(&cmd, file, line);
    }

    // 3. $EDITOR
    if let Some(cmd) = lookup("EDITOR").filter(|s| !s.trim().is_empty()) {
        return build_standard_editor(&cmd, file, line);
    }

    // 4. sisho-desk extension (ONLY if active AND explicitly Allowed!)
    if let Some(desk) = extensions
        .iter()
        .find(|e| e.word == "desk" && e.is_active && e.trust == TrustState::Allowed)
    {
        return Step::new("desk", &desk.path)
            .envs(crate::extension::extension_dir_envs(&desk.word))
            .arg(file);
    }

    // 5. System fallback (nano)
    let mut step = Step::new("editor", "nano");
    if let Some(l) = line {
        step = step.arg(format!("+{l}"));
    }
    step.arg(file)
}

fn build_from_sisho_editor(
    cmd: &str,
    file: &Path,
    line: Option<usize>,
    column: Option<usize>,
    extensions: &[ExtensionItem],
) -> Step {
    let trimmed = cmd.trim();

    if trimmed.contains("{file}") {
        let file_str = file.to_string_lossy();
        let line_str = line
            .map(|l| l.to_string())
            .unwrap_or_else(|| "1".to_string());
        let col_str = column
            .map(|c| c.to_string())
            .unwrap_or_else(|| "1".to_string());

        let replaced = trimmed
            .replace("{file}", &file_str)
            .replace("{line}", &line_str)
            .replace("{col}", &col_str);

        let parts = split_cmd(&replaced);
        let program = parts.first().cloned().unwrap_or_else(|| "nano".to_string());
        let mut step = Step::new("editor", program);
        for arg in parts.into_iter().skip(1) {
            step = step.arg(arg);
        }
        return step;
    }

    let word = trimmed.strip_prefix("sisho-").unwrap_or(trimmed);
    if let Some(ext) = extensions
        .iter()
        .find(|e| e.word == word && e.is_active && e.trust == TrustState::Allowed)
    {
        return Step::new(&ext.word, &ext.path)
            .envs(crate::extension::extension_dir_envs(&ext.word))
            .arg(file);
    }

    build_standard_editor(trimmed, file, line)
}

fn build_standard_editor(cmd: &str, file: &Path, line: Option<usize>) -> Step {
    let parts = split_cmd(cmd);
    let program = parts.first().cloned().unwrap_or_else(|| "nano".to_string());
    let mut step = Step::new("editor", &program);
    for arg in parts.into_iter().skip(1) {
        step = step.arg(arg);
    }

    let prog_name = Path::new(&program)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&program);

    match prog_name {
        "hx" => {
            if let Some(l) = line {
                step = step.arg(format!("{}:{l}", file.to_string_lossy()));
            } else {
                step = step.arg(file);
            }
        }
        "code" | "codium" => {
            if let Some(l) = line {
                step = step
                    .arg("--goto")
                    .arg(format!("{}:{l}", file.to_string_lossy()));
            } else {
                step = step.arg(file);
            }
        }
        _ => {
            if let Some(l) = line {
                step = step.arg(format!("+{l}"));
            }
            step = step.arg(file);
        }
    }

    step
}

fn split_cmd(cmd: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;

    for ch in cmd.chars() {
        match ch {
            '\'' if !in_double => {
                in_single = !in_single;
            }
            '"' if !in_single => {
                in_double = !in_double;
            }
            c if c.is_whitespace() && !in_single && !in_double => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::*;

    fn mock_lookup(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key| map.get(key).cloned()
    }

    #[test]
    fn sisho_editor_template_expansion() {
        let file = Path::new("docs/intro.tmt");
        let extensions = [];
        let lookup = mock_lookup(&[("SISHO_EDITOR", "hx '{file}:{line}'")]);

        let step = resolve_editor_with_lookup(file, Some(42), Some(5), &extensions, lookup);
        assert_eq!(step.command_line(), "hx docs/intro.tmt:42");
    }

    #[test]
    fn sisho_editor_extension_word_requires_allowed_trust() {
        let file = Path::new("docs/intro.tmt");
        let untrusted = [ExtensionItem::new("desk", PathBuf::from("/bin/sisho-desk"), true, None)];
        let lookup = mock_lookup(&[]);

        // Untrusted desk must fall back to nano, NEVER execute /bin/sisho-desk
        let step = resolve_editor_with_lookup(file, Some(10), None, &untrusted, &lookup);
        assert_eq!(step.command_line(), "nano +10 docs/intro.tmt");

        // Once Allowed, fallback uses /bin/sisho-desk
        let mut allowed = untrusted;
        allowed[0].trust = TrustState::Allowed;
        let step = resolve_editor_with_lookup(file, Some(10), None, &allowed, &lookup);
        assert_eq!(step.command_line(), "/bin/sisho-desk docs/intro.tmt");
    }

    #[test]
    fn visual_prefers_over_editor_and_adds_plus_line() {
        let file = Path::new("docs/intro.tmt");
        let extensions = [];
        let lookup = mock_lookup(&[("VISUAL", "nvim"), ("EDITOR", "nano")]);

        let step = resolve_editor_with_lookup(file, Some(15), None, &extensions, lookup);
        assert_eq!(step.command_line(), "nvim +15 docs/intro.tmt");
    }

    #[test]
    fn helix_and_vscode_editor_syntax() {
        let file = Path::new("docs/intro.tmt");
        let extensions = [];
        let step = resolve_editor_with_lookup(
            file,
            Some(20),
            None,
            &extensions,
            mock_lookup(&[("EDITOR", "hx")]),
        );
        assert_eq!(step.command_line(), "hx docs/intro.tmt:20");

        let step = resolve_editor_with_lookup(
            file,
            Some(30),
            None,
            &extensions,
            mock_lookup(&[("EDITOR", "code")]),
        );
        assert_eq!(step.command_line(), "code --goto docs/intro.tmt:30");
    }
}
