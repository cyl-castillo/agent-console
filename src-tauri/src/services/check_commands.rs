//! Which shell command lines RUN a test or check suite.
//!
//! The ledger writes a `check_run` line for these, and a `passed` one is what
//! turns "tests pass" in the agent's closing words into `verified`. So the
//! question here is narrower than "does the line mention a runner": the
//! runner has to be the command a shell would execute. `echo cargo test`,
//! `git commit -m 'run npm test later'`, `true # cargo test` and a heredoc
//! whose body says `pytest` all mention one and run none.
//!
//! The line is split the way a POSIX shell splits it — quotes, comments,
//! heredocs, redirections, command and process substitutions, control
//! operators — into simple commands, and each one is matched at its command
//! word, after assignments and a short list of prefixes that run what follows
//! (`env`, `timeout 600`, `uv run`, `npx`, `sh -c '…'`).
//!
//! Conservative in one direction only: a miss records nothing, and a claim
//! without a `check_run` reads `unverified_claim`. Whatever the shell would
//! not plainly execute as a statement (text inside `$(…)`, an unknown
//! launcher, a prefix flag this does not know, a `sh -c` script built from
//! an expansion, a line nested too deep to read) is a miss.
//!
//! What this does NOT decide: whether the runner's exit status is the line's
//! (`cargo test | tail`, `cargo test || true`), or whether the runner is the
//! real one (an alias, a `PATH` override, a `test` script that echoes). The
//! command is recorded verbatim on the `check_run` line for that reason.

/// A nesting no person types; past it the line is not read at all.
const MAX_DEPTH: usize = 16;

/// Stands in a word for text only the running shell knows: an expansion
/// (`$X`, `$(…)`, backticks) or an escape this does not decode. A word that
/// carries it never names a runner, and is never read as a `sh -c` script:
/// `bash -c "echo $X; cargo test"` runs no test when `$X` is `#`.
const UNKNOWN: char = '\u{1}';

/// `$` followed by one of these starts a parameter expansion.
fn starts_expansion(next: Option<char>) -> bool {
    next.is_some_and(|c| {
        c.is_ascii_alphanumeric()
            || matches!(c, '_' | '{' | '@' | '*' | '#' | '?' | '$' | '!' | '-')
    })
}

struct Lexer {
    chars: Vec<char>,
    i: usize,
    /// Heredocs opened on the current line: (delimiter, strip leading tabs).
    /// Their bodies start after the next unquoted newline, at any nesting.
    heredocs: Vec<(String, bool)>,
    depth: usize,
    /// Set when the line cannot be read: nested deeper than `MAX_DEPTH`, or
    /// a quote or substitution left open (a shell refuses that line whole).
    /// Nothing in it is a check.
    bail: bool,
}

/// A statement being assembled: the words of one simple command.
#[derive(Default)]
struct Pending {
    out: Vec<Vec<String>>,
    words: Vec<String>,
    word: String,
    in_word: bool,
    /// The next word is a redirection target, not an argument.
    skip_word: bool,
}

impl Pending {
    fn push(&mut self, c: char) {
        self.word.push(c);
        self.in_word = true;
    }

    fn end_word(&mut self) {
        if !self.in_word {
            return;
        }
        let w = std::mem::take(&mut self.word);
        self.in_word = false;
        if self.skip_word {
            self.skip_word = false;
        } else {
            self.words.push(w);
        }
    }

    fn end_statement(&mut self) {
        self.end_word();
        self.skip_word = false;
        if !self.words.is_empty() {
            self.out.push(std::mem::take(&mut self.words));
        }
    }

    /// `2>` / `1>&2`: the digits before a redirection are a descriptor.
    fn drop_fd_or_end_word(&mut self) {
        if self.in_word && self.word.chars().all(|c| c.is_ascii_digit()) {
            self.word.clear();
            self.in_word = false;
        } else {
            self.end_word();
        }
    }
}

impl Lexer {
    fn new(cmd: &str) -> Self {
        Lexer {
            chars: cmd.chars().collect(),
            i: 0,
            heredocs: Vec::new(),
            depth: 0,
            bail: false,
        }
    }

    fn peek(&self, ahead: usize) -> Option<char> {
        self.chars.get(self.i + ahead).copied()
    }

    /// Past the closing `'`. `ansi` is `$'…'`, where a backslash escapes.
    fn single_quoted(&mut self, p: &mut Pending, ansi: bool) {
        p.in_word = true;
        while let Some(c) = self.peek(0) {
            self.i += 1;
            match c {
                '\'' => return,
                '\\' if ansi => {
                    if let Some(n) = self.peek(0) {
                        // `\n`, `\x23`, `\047` become characters this
                        // does not compute (a newline, a `#`, a quote).
                        if !matches!(n, '\'' | '"' | '\\') {
                            p.push(UNKNOWN);
                        }
                        p.push(n);
                        self.i += 1;
                    }
                }
                _ => p.push(c),
            }
        }
        self.bail = true;
    }

    /// Past the closing `"`. Substitutions inside are skipped whole, so a
    /// quote within `"$(printf "%s" x)"` does not end the string early.
    fn double_quoted(&mut self, p: &mut Pending) {
        p.in_word = true;
        while let Some(c) = self.peek(0) {
            self.i += 1;
            match c {
                '"' => return,
                // Inside double quotes a backslash escapes only these; before
                // anything else it stays (`"echo \; x"` keeps its `\;`).
                '\\' => match self.peek(0) {
                    Some(n @ ('$' | '`' | '"' | '\\')) => {
                        p.push(n);
                        self.i += 1;
                    }
                    Some('\n') => self.i += 1,
                    _ => p.push('\\'),
                },
                '$' if self.peek(0) == Some('(') => {
                    self.i += 1;
                    self.skip_nested();
                    p.push(UNKNOWN);
                }
                '$' if starts_expansion(self.peek(0)) => {
                    p.push(UNKNOWN);
                    p.push('$');
                }
                '`' => {
                    self.backticks();
                    p.push(UNKNOWN);
                }
                _ => p.push(c),
            }
        }
        self.bail = true;
    }

    /// Past the closing backtick; nothing inside is read as a statement.
    fn backticks(&mut self) {
        while let Some(c) = self.peek(0) {
            self.i += 1;
            match c {
                '`' => return,
                '\\' => self.i += 1,
                _ => {}
            }
        }
        self.bail = true;
    }

    /// A `(`-opened group whose commands do not count (`$(…)`, `<(…)`,
    /// `name=(…)`): lexed with the same rules so quotes and heredocs inside
    /// stay balanced, and discarded.
    fn skip_nested(&mut self) {
        if self.depth >= MAX_DEPTH {
            self.bail = true;
            self.i = self.chars.len();
            return;
        }
        self.depth += 1;
        let _ = self.statements(true);
        self.depth -= 1;
    }

    /// `<<[-]DELIM`: remember the delimiter; the body is skipped at the next
    /// newline. Called with `self.i` just past the `<<`.
    fn heredoc_start(&mut self) {
        let strip_tabs = self.peek(0) == Some('-');
        if strip_tabs {
            self.i += 1;
        }
        while matches!(self.peek(0), Some(' ' | '\t')) {
            self.i += 1;
        }
        let mut delim = String::new();
        while let Some(c) = self.peek(0) {
            if c.is_whitespace() || matches!(c, ';' | '&' | '|' | '(' | ')' | '<' | '>') {
                break;
            }
            self.i += 1;
            if !matches!(c, '\'' | '"' | '\\') {
                delim.push(c);
            }
        }
        if !delim.is_empty() {
            self.heredocs.push((delim, strip_tabs));
        }
    }

    /// Called just past a newline: skip the body of every heredoc opened on
    /// the line that ended, up to and including each delimiter line.
    fn heredoc_bodies(&mut self) {
        for (delim, strip_tabs) in std::mem::take(&mut self.heredocs) {
            while self.i < self.chars.len() {
                let start = self.i;
                while self.i < self.chars.len() && self.chars[self.i] != '\n' {
                    self.i += 1;
                }
                let line: String = self.chars[start..self.i].iter().collect();
                if self.i < self.chars.len() {
                    self.i += 1; // the newline
                }
                let line = line.trim_end_matches('\r');
                let line = if strip_tabs {
                    line.trim_start_matches('\t')
                } else {
                    line
                };
                if line == delim {
                    break;
                }
            }
        }
    }

    /// A redirection operator starting at `self.i` (`>`, `>>`, `<`, `>|`,
    /// `>&2`, `<&-`, `<<<`): consumed, and its target word marked to skip.
    fn redirection(&mut self, p: &mut Pending) {
        p.drop_fd_or_end_word();
        while matches!(self.peek(0), Some('>' | '<')) {
            self.i += 1;
        }
        if self.peek(0) == Some('|') {
            self.i += 1;
        }
        if self.peek(0) == Some('&') {
            self.i += 1;
            let mut fd = false;
            while matches!(self.peek(0), Some(c) if c.is_ascii_digit() || c == '-') {
                self.i += 1;
                fd = true;
            }
            if fd {
                return;
            }
        }
        p.skip_word = true;
    }

    /// The simple commands of the line, in order. `nested` reads up to the
    /// `)` that closes the group the caller opened.
    fn statements(&mut self, nested: bool) -> Vec<Vec<String>> {
        let mut p = Pending::default();
        let mut parens = 0usize;
        while let Some(c) = self.peek(0) {
            match c {
                '\\' => match (self.peek(1), self.peek(2)) {
                    (Some('\n'), _) => self.i += 2,
                    (Some('\r'), Some('\n')) => self.i += 3,
                    (Some(n), _) => {
                        p.push(n);
                        self.i += 2;
                    }
                    (None, _) => self.i += 1,
                },
                '\'' => {
                    let ansi = p.in_word && p.word.ends_with('$');
                    if ansi {
                        p.word.pop();
                    }
                    self.i += 1;
                    self.single_quoted(&mut p, ansi);
                }
                '"' => {
                    self.i += 1;
                    self.double_quoted(&mut p);
                }
                '`' => {
                    self.i += 1;
                    self.backticks();
                    p.push(UNKNOWN);
                }
                '$' if self.peek(1) == Some('(') => {
                    self.i += 2;
                    self.skip_nested();
                    p.push(UNKNOWN);
                }
                '$' if starts_expansion(self.peek(1)) => {
                    self.i += 1;
                    p.push(UNKNOWN);
                    p.push('$');
                }
                '#' if !p.in_word => {
                    while self.peek(0).is_some_and(|c| c != '\n') {
                        self.i += 1;
                    }
                }
                '\n' => {
                    self.i += 1;
                    p.end_statement();
                    self.heredoc_bodies();
                }
                ' ' | '\t' | '\r' => {
                    self.i += 1;
                    p.end_word();
                }
                ';' | '|' => {
                    self.i += 1;
                    p.end_statement();
                }
                '&' if self.peek(1) == Some('>') => {
                    // `&>file`, `&>>file`
                    self.i += 1;
                    p.end_word();
                    self.redirection(&mut p);
                }
                '&' => {
                    self.i += 1;
                    p.end_statement();
                }
                '<' if self.peek(1) == Some('<') && self.peek(2) != Some('<') => {
                    p.drop_fd_or_end_word();
                    self.i += 2;
                    self.heredoc_start();
                }
                '<' | '>' if self.peek(1) == Some('(') => {
                    // process substitution: an argument, not a statement
                    self.i += 2;
                    self.skip_nested();
                    p.push(UNKNOWN);
                }
                '<' | '>' => self.redirection(&mut p),
                '(' if p.in_word && p.word.ends_with('=') => {
                    // `name=(a b c)`: an array, not a subshell
                    self.i += 1;
                    self.skip_nested();
                    p.push(UNKNOWN);
                }
                '(' => {
                    self.i += 1;
                    p.end_statement();
                    parens += 1;
                }
                ')' => {
                    self.i += 1;
                    p.end_statement();
                    if parens > 0 {
                        parens -= 1;
                    } else if nested {
                        return p.out;
                    }
                }
                _ => {
                    self.i += 1;
                    p.push(c);
                }
            }
        }
        if nested {
            // the group the caller opened never closed
            self.bail = true;
        }
        p.end_statement();
        p.out
    }
}

fn simple_commands(cmd: &str) -> Vec<Vec<String>> {
    let mut lx = Lexer::new(cmd);
    let out = lx.statements(false);
    if lx.bail {
        Vec::new()
    } else {
        out
    }
}

/// `./gradlew` and `.venv/bin/pytest` run `gradlew` and `pytest`.
fn basename(word: &str) -> &str {
    word.rsplit('/').next().unwrap_or(word)
}

fn is_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    let mut cs = name.chars();
    cs.next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && cs.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_duration(word: &str) -> bool {
    let digits = word.trim_end_matches(['s', 'm', 'h', 'd']);
    digits.chars().any(|c| c.is_ascii_digit())
        && digits.chars().all(|c| c.is_ascii_digit() || c == '.')
}

/// Reserved words after which a command starts.
const RESERVED: &[&str] = &[
    "if", "then", "elif", "else", "do", "while", "until", "!", "{",
];
/// A word that runs the command after it, and where that command starts.
///
/// - `None`: `words[i]` is not such a prefix.
/// - `Some(Some(j))`: it is, and the command it runs starts at `words[j]`.
/// - `Some(None)`: it is, with a flag that is not listed here. The flag may
///   take the next word as its value (`npx -p eslint echo ok` runs `echo`,
///   `env -u tsc true` runs `true`), so the line is not read: a miss.
///
/// Only flags known to take no value are listed, plus `--name=value`, which
/// carries its own. Not `command`: its everyday form is `command -v pytest`,
/// which looks a runner up and runs nothing.
fn prefix_end(words: &[String], i: usize) -> Option<Option<usize>> {
    let name = basename(&words[i]);
    let next = words.get(i + 1).map(String::as_str);
    let (mut j, flags): (usize, &[&str]) = match (name, next) {
        ("uv", Some("run")) => (
            i + 2,
            &[
                "--frozen",
                "--locked",
                "--no-sync",
                "--isolated",
                "--offline",
                "--no-project",
                "--no-dev",
                "--all-extras",
                "-q",
                "--quiet",
            ],
        ),
        ("poetry" | "pipenv" | "pdm" | "hatch" | "rye", Some("run")) => (i + 2, &[]),
        ("bundle" | "pnpm" | "yarn", Some("exec")) => (i + 2, &[]),
        ("npm", Some("exec")) => (i + 2, &["-y", "--yes"]),
        ("env", _) => (i + 1, &["-i", "--ignore-environment"]),
        ("time", _) => (i + 1, &["-p"]),
        ("nice" | "nohup" | "stdbuf", _) => (i + 1, &[]),
        ("sudo", _) => (i + 1, &["-E", "-H", "-n"]),
        ("timeout", _) => (
            i + 1,
            &["--preserve-status", "--foreground", "-v", "--verbose"],
        ),
        ("xvfb-run", _) => (i + 1, &["-a", "--auto-servernum"]),
        ("npx", _) => (
            i + 1,
            &["-y", "--yes", "--no", "--no-install", "-q", "--quiet"],
        ),
        ("bunx", _) => (i + 1, &["--bun"]),
        _ => return None,
    };
    while let Some(w) = words.get(j).map(String::as_str) {
        let after = words.get(j + 1).map(String::as_str);
        // A word of the prefix's own that carries no separate value.
        let own = flags.contains(&w)
            || (w.starts_with("--") && w.contains('='))
            || (name == "env" && is_assignment(w))
            || (name == "stdbuf"
                && w.len() > 2
                && ["-i", "-o", "-e"].iter().any(|f| w.starts_with(f)));
        if w == "--" {
            j += 1;
            break;
        } else if own {
            j += 1;
        } else if name == "nice" && w == "-n" && after.is_some_and(is_duration) {
            j += 2;
        } else if w.starts_with('-') {
            return Some(None);
        } else {
            break;
        }
    }
    if name == "timeout" {
        // `timeout DURATION command`
        if !words.get(j).is_some_and(|w| is_duration(w)) {
            return Some(None);
        }
        j += 1;
    }
    Some(Some(j))
}

const SHELLS: &[&str] = &["sh", "bash", "zsh", "dash"];
/// Binaries `pnpm`, `yarn` and `bun` run by name: `pnpm vitest run`.
const NODE_BINS: &[&str] = &[
    "vitest",
    "jest",
    "mocha",
    "playwright",
    "tsc",
    "eslint",
    "prettier",
];

/// The script of `sh -c '…'`: the word after a `-c` (or `-lc`, `-ec`) that
/// comes before any operand. `bash run.sh -c x` passes `-c` to the script.
/// Only a script written out in full: one assembled from an expansion is
/// text this cannot see, and what it sees around it may be a comment.
fn shell_script<'a>(args: &[&'a str]) -> Option<&'a str> {
    let mut i = 0;
    while let Some(a) = args.get(i).copied() {
        if a == "-o" || a == "+o" {
            i += 2;
        } else if a.starts_with('-') && !a.starts_with("--") && a.ends_with('c') {
            return args
                .get(i + 1)
                .copied()
                .filter(|script| !script.contains(UNKNOWN));
        } else if a.starts_with('-') || a.starts_with('+') {
            i += 1;
        } else {
            return None;
        }
    }
    None
}

/// The kind of check one simple command runs, if it runs one.
fn statement_kind(words: &[String], depth: usize) -> Option<&'static str> {
    let mut i = 0;
    loop {
        let w = words.get(i)?.as_str();
        if RESERVED.contains(&w) || is_assignment(w) {
            i += 1;
            continue;
        }
        match prefix_end(words, i) {
            Some(end) => i = end?,
            None => break,
        }
    }
    words.get(i)?;
    let name = basename(&words[i]);
    let args: Vec<&str> = words[i + 1..].iter().map(String::as_str).collect();
    let arg = |n: usize| args.get(n).copied().unwrap_or("");

    // `sh -c 'cd app && npm test'`: the script is a line of its own.
    if SHELLS.contains(&name) {
        if depth >= 2 {
            return None;
        }
        return line_kind(shell_script(&args)?, depth + 1);
    }
    // `pytest --version` and `cargo test --help` ask the runner about
    // itself; nothing is checked.
    if args
        .iter()
        .take_while(|a| **a != "--")
        .any(|a| matches!(*a, "--version" | "--help"))
    {
        return None;
    }

    match name {
        "cargo" => {
            // `cargo +nightly fmt --check`
            let sub = if arg(0).starts_with('+') { 1 } else { 0 };
            match arg(sub) {
                "test" => Some("test"),
                "clippy" => Some("lint"),
                "check" => Some("typecheck"),
                "fmt" if args.contains(&"--check") => Some("format"),
                _ => None,
            }
        }
        "npm" | "pnpm" | "yarn" | "bun" => match (arg(0), arg(1)) {
            ("test", _) | ("run", "test" | "tests") => Some("test"),
            ("run", "lint") => Some("lint"),
            ("run", "typecheck" | "check") => Some("typecheck"),
            ("run", "build") => Some("build"),
            ("run", "format:check") => Some("format"),
            (bin, _) if name != "npm" && NODE_BINS.contains(&bin) => {
                statement_kind(&words[i + 1..], depth)
            }
            _ => None,
        },
        "vitest" | "jest" | "mocha" | "pytest" | "tox" | "nox" | "rspec" | "phpunit" => {
            Some("test")
        }
        "playwright" | "dotnet" | "swift" | "mix" if arg(0) == "test" => Some("test"),
        "tsc" => Some("typecheck"),
        "eslint" => Some("lint"),
        "prettier" if args.contains(&"--check") => Some("format"),
        "python" | "python3" if arg(0) == "-m" && matches!(arg(1), "pytest" | "unittest") => {
            Some("test")
        }
        "go" => match arg(0) {
            "test" => Some("test"),
            "vet" => Some("lint"),
            _ => None,
        },
        "make" => match arg(0) {
            "test" | "check" => Some("test"),
            "lint" => Some("lint"),
            _ => None,
        },
        "mvn" | "mvnw" if matches!(arg(0), "test" | "verify") => Some("test"),
        "gradle" | "gradlew" if matches!(arg(0), "test" | "check") => Some("test"),
        _ => None,
    }
}

/// The kind recorded for a whole line. One line yields one `check_run`, so a
/// line that runs several gates is named after the narrowest one.
fn line_kind(cmd: &str, depth: usize) -> Option<&'static str> {
    const ORDER: &[&str] = &["format", "lint", "typecheck", "build", "test"];
    simple_commands(cmd)
        .iter()
        .filter_map(|words| statement_kind(words, depth))
        .min_by_key(|kind| ORDER.iter().position(|k| k == kind))
}

/// Does this command line run a test/check suite? A hit turns the result
/// into a `check_run` event — the "tests ran, and this is what they said"
/// line a reviewer reads before any prompt.
pub fn is_check_command(cmd: &str) -> bool {
    line_kind(cmd, 0).is_some()
}

/// What a recognized check command checks. Keyed on the runner, not the
/// outcome: a reviewer asks "did the TESTS run?", not "did something pass?".
/// `"test"` for a line that is not a check at all — ask `is_check_command`
/// first.
pub fn check_kind(cmd: &str) -> &'static str {
    line_kind(cmd, 0).unwrap_or("test")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(cmd: &str) -> Vec<Vec<String>> {
        simple_commands(cmd)
    }

    #[test]
    fn a_runner_at_the_command_word_is_a_check() {
        for c in [
            "cargo test",
            "cargo test --workspace -- --nocapture",
            "cd src-tauri && cargo clippy -- -D warnings",
            "cd src-tauri && cargo fmt --check && cargo clippy --workspace --lib --bins -- -D warnings && cargo test --workspace",
            "npm test",
            "npm run lint",
            "pnpm run typecheck",
            "npx vitest run",
            "npx --yes prettier --check .",
            "pytest -q tests/",
            "python -m pytest",
            "go test ./...",
            "make check",
            "./gradlew test",
            "bundle exec rspec",
            "(cd app && npm test)",
            "ls; cargo test",
            "echo starting && npm test",
            "cargo test 2>&1 | tail -20",
            "npm test > out.log 2>&1",
            "CI=1 RUST_BACKTRACE=1 cargo test",
            "time cargo test",
            "timeout 600 npm test",
            "env -i PATH=/usr/bin cargo test",
            "uv run pytest -q",
            "poetry run python -m pytest",
            ".venv/bin/pytest tests",
            "./node_modules/.bin/vitest run",
            "pnpm vitest run",
            "yarn jest --ci",
            "bash -o pipefail -c 'npm test | tee out.log'",
            "nice -n 10 cargo test",
            "npx -y vitest run",
            "uv run --frozen pytest",
            "npm exec -- vitest run",
            "stdbuf -oL cargo test",
            "xvfb-run -a npm test",
            "env --unset=FOO CI=1 npm test",
            "timeout --preserve-status 10m cargo test",
            "bash -c 'echo $HOME && cargo test'",
            "cargo test -p \"$CRATE\" -- $FILTER",
            "cargo +nightly fmt --all --check",
            "if cargo test; then echo ok; fi",
            "for p in a b; do cargo test -p $p; done",
            "bash -lc 'cd app && npm test'",
            "sh -c \"cargo test\"",
            "cargo test \\\n  --workspace",
            "cat <<EOF > notes.txt\nsome text\nEOF\ncargo test",
        ] {
            assert!(is_check_command(c), "{c}");
        }
    }

    #[test]
    fn a_mention_of_a_runner_is_not_a_check() {
        for c in [
            // plain commands
            "ls -la",
            "git status",
            "npm install",
            "cargo build --release",
            "mytest",
            "cat pytest.ini",
            "npx playwright install",
            // the runner is an argument
            "echo test",
            "echo cargo test",
            "echo run npm test before pushing",
            "cat notes about pytest",
            "grep -rn cargo test src",
            "sudo echo cargo test",
            "env echo npm test",
            "timeout 5 echo cargo test",
            "rg -- cargo test",
            // quoted
            "git commit -m 'run npm test later'",
            "git commit -m \"fix: cargo test was flaky\"",
            "echo 'done;' cargo test",
            "echo \"first; cargo test\"",
            "echo \"$(printf \"%s\" \"x; cargo test \")\"",
            "printf 'a\\n' && echo \"ok | npm test\"",
            "echo $'it\\'s; cargo test'",
            // escaped operator
            "echo done \\; cargo test",
            // comments
            "true # cargo test",
            "ls # ; npm test",
            "# cargo test",
            // heredoc bodies and here-strings
            "cat > NOTES.md <<EOF\ncargo test\nEOF",
            "cat > NOTES.md <<'EOF'\nrun this:\nnpm test\nEOF\nls",
            "cat <<-EOF\n\tcargo test\n\tEOF",
            "git commit -F - <<EOF\ntests: cargo test passes\nEOF",
            "git commit -m \"$(cat <<'EOF'\nit's done\n\ncargo test\nEOF\n)\"",
            "cat <<< 'cargo test'",
            // run where the shell does not treat it as a statement
            "echo $(cargo test)",
            "echo `cargo test`",
            "diff <(cargo test) expected.txt",
            "cmds=(cargo test)",
            // redirection targets
            "echo x > pytest",
            // not the subcommand that checks
            "cargo fmt",
            "npx prettier --write .",
            "python -m http.server",
            "yarn add jest",
            // asking about the runner
            "command -v pytest",
            "which pytest && type cargo",
            "pytest --version",
            "npx tsc --version",
            "cargo test --help",
            // a prefix flag that takes the next word as its value
            "npx -p eslint echo ok",
            "npx --package eslint echo ok",
            "env -u tsc true",
            "sudo -u pytest ls",
            "uv run --with pytest ls",
            "timeout -s KILL 5 echo",
            "timeout cargo test",
            "nice -n cargo test",
            // shells without a script, or with one only the shell can read
            "bash script.sh cargo test",
            "bash -c",
            "bash -c \"echo \\; eslint\"",
            "bash -c \"echo $X; cargo test\"",
            "bash -c \"echo $(printf '#'); cargo test\"",
            "bash -c \"echo `printf '#'`; cargo test\"",
            "bash -c $'echo \\x23 ; cargo test'",
            "bash -c $'echo \\047; cargo test\\047'",
            "bash -c \"true; eslint \\\"\"",
            "cargo test 'unterminated",
            "$RUNNER cargo test",
        ] {
            assert!(!is_check_command(c), "{c}");
        }
    }

    #[test]
    fn the_kind_comes_from_the_runner_that_ran() {
        assert_eq!(check_kind("npm test"), "test");
        assert_eq!(check_kind("npm run lint"), "lint");
        assert_eq!(check_kind("npm run build"), "build");
        assert_eq!(check_kind("cargo fmt --check"), "format");
        assert_eq!(check_kind("cd app && npm run typecheck"), "typecheck");
        assert_eq!(check_kind("npx tsc --noEmit"), "typecheck");
        assert_eq!(check_kind("go vet ./..."), "lint");
        assert_eq!(check_kind("make check"), "test");
        // A runner named in an argument does not rename the check.
        assert_eq!(check_kind("cargo test && echo prettier"), "test");
        assert_eq!(check_kind("npm test # then npm run lint"), "test");
        // One line, one check_run: the narrowest gate names it.
        assert_eq!(check_kind("cargo fmt --check && cargo test"), "format");
    }

    #[test]
    fn the_line_is_split_like_a_shell_splits_it() {
        assert_eq!(
            words("FOO=1 cargo test -p 'my crate' 2>&1 | tee \"out file.log\""),
            vec![
                vec!["FOO=1", "cargo", "test", "-p", "my crate"],
                vec!["tee", "out file.log"],
            ]
        );
        assert_eq!(
            words("a && b || c; d & e\nf"),
            vec![
                vec!["a"],
                vec!["b"],
                vec!["c"],
                vec!["d"],
                vec!["e"],
                vec!["f"]
            ]
        );
        // Redirections are not arguments; `&>` is not the background operator.
        assert_eq!(
            words("run >out.txt 2>>err.txt <in.txt &>all.txt arg"),
            vec![vec!["run", "arg"]]
        );
        assert_eq!(words("run 2>&1 >&2 <&- arg"), vec![vec!["run", "arg"]]);
        // Inside double quotes a backslash goes only before $ ` " \ newline.
        assert_eq!(
            words(r#"echo "a\;b \"q\" \\ \x""#),
            vec![vec!["echo", r#"a\;b "q" \ \x"#]]
        );
        // A quote or substitution left open: the shell refuses the line,
        // so none of it is read.
        for open in [
            "echo 'oops; cargo test",
            "cargo test \"",
            "cargo test; echo \"oops",
            "cargo test && echo $(date",
            "cargo test; echo `date",
        ] {
            assert!(words(open).is_empty(), "{open}");
        }
        // A heredoc with no closing delimiter runs to the end of the input.
        assert_eq!(words("cat <<EOF\ncargo test\n"), vec![vec!["cat"]]);
        // CRLF line endings.
        assert_eq!(
            words("cat <<EOF\r\ncargo test\r\nEOF\r\nls"),
            vec![vec!["cat"], vec!["ls"]]
        );
    }

    #[test]
    fn a_line_nested_too_deep_to_read_is_not_a_check() {
        // Model-written input: bounded recursion, and a miss rather than a
        // guess when the bound is hit.
        let deep = format!(
            "cargo test; echo {}x{}",
            "$(".repeat(5000),
            ")".repeat(5000)
        );
        assert!(!is_check_command(&deep));
        assert!(!is_check_command(
            "bash -c \"bash -c \\\"bash -c 'cargo test'\\\"\""
        ));
        // Multi-byte text anywhere a flag or a name is expected: no panic.
        for c in [
            "stdbuf -é cargo test",
            "stdbuf é cargo test",
            "nice -n ñ cargo test",
            "timeout 5é cargo test",
            "env É=1 cargo test",
            "ñ/cargo test",
            "bash -é 'cargo test'",
            "cargo +é test",
            "echo \u{1}; \u{1}",
        ] {
            let _ = is_check_command(c);
            let _ = check_kind(c);
        }
        // Within the bounds it reads normally.
        assert!(is_check_command("bash -c \"bash -c 'cargo test'\""));
        assert!(is_check_command("echo $(echo $(date)); cargo test"));
    }
}
