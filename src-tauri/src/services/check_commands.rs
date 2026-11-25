//! Which shell command lines RUN a test or check suite.
//!
//! The ledger writes a `check_run` line for these, and a `passed` one is what
//! turns "tests pass" in the agent's closing words into `verified`. So the
//! question here is narrower than "does the line mention a runner": the
//! runner has to be the command a shell would execute. `echo cargo test`,
//! `git commit -m 'run npm test later'`, `true # cargo test` and a heredoc
//! whose body says `pytest` all mention one and run none. So do a test
//! (`[[ -n x && pytest ]]`), an arithmetic expression (`(( pytest + 1 ))`)
//! and a `case` pattern (`a|pytest)`), where the word is not a command. And
//! the runner has to be asked to run: `cargo test --no-run` compiles,
//! `pytest --collect-only` lists and `make -n test` echoes the recipe.
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
//! A second question, answered by `exit_speaks_for_checks`: is the status the
//! line ended with a statement about the runner? `cargo test | tail` ends
//! with `tail`'s, `cargo test || true` with `true`'s, and `false && cargo
//! test; true` ends 0 without the runner ever starting. A `passed` or
//! `failed` read off such a status would be about another command.
//!
//! What this does NOT decide: whether the runner is the real one (an alias,
//! a function, a `PATH` override, a `test` script that echoes, a file the
//! line sources). The command is recorded verbatim on the `check_run` line
//! for that reason.

/// A nesting no person types; past it the line is not read at all.
const MAX_DEPTH: usize = 16;

/// How many `((` that are not arithmetic a line may hold. Each one is read
/// twice, as arithmetic and then as two subshells; past this many the line is
/// not read at all, so reading it costs its length and not its length squared.
const MAX_REREADS: usize = 64;

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
    /// Inside an `arithmetic` attempt, which only looks for where the group
    /// ends: a `((` in there is two parentheses and starts no attempt of its
    /// own. Without this an attempt held attempts, each read again when the
    /// outer one failed, and a run of `(` cost more with every one added.
    probing: bool,
    /// `((` groups read again as subshells. Counted across the whole line
    /// and never rewound; past `MAX_REREADS` the line is not read.
    rereads: usize,
}

/// A piece of a line: a simple command, or what joins it to the next one.
#[derive(Debug, Clone, PartialEq)]
enum Tok {
    /// The words of one simple command; empty for one that is only a
    /// redirection (`> out.txt`), which still has a status of its own.
    Cmd(Vec<String>),
    /// `&&`
    And,
    /// `||`
    Or,
    /// `|` and `|&`
    Pipe,
    /// `;` and a newline
    Seq,
    /// `&`
    Background,
    /// `(`
    Open,
    /// `)`
    Close,
    /// `(( … ))`: arithmetic; what is inside is not kept
    Arithmetic,
    /// `;;`, `;&`, `;;&`: the end of a `case` branch
    Branch,
}

/// A statement being assembled: the words of one simple command.
#[derive(Default)]
struct Pending {
    out: Vec<Tok>,
    words: Vec<String>,
    word: String,
    in_word: bool,
    /// The next word is a redirection target, not an argument.
    skip_word: bool,
    /// The statement has a redirection: it is a command even with no words.
    redirected: bool,
    /// The word being read has a quote or an escape in it.
    quoted: bool,
    /// The last word was written bare: `]]`, not `"]]"` or `\]\]`.
    bare: bool,
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
        self.bare = !std::mem::take(&mut self.quoted);
        if self.skip_word {
            self.skip_word = false;
        } else {
            self.words.push(w);
        }
    }

    fn end_statement(&mut self) {
        self.end_word();
        self.skip_word = false;
        if !self.words.is_empty() || self.redirected {
            self.out.push(Tok::Cmd(std::mem::take(&mut self.words)));
        }
        self.redirected = false;
    }

    /// The statement ends at an operator, which is kept.
    fn operator(&mut self, op: Tok) {
        self.end_statement();
        self.out.push(op);
    }

    /// `2>` / `1>&2`: the digits before a redirection are a descriptor.
    fn drop_fd_or_end_word(&mut self) {
        if self.in_word && self.word.chars().all(|c| c.is_ascii_digit()) {
            self.word.clear();
            self.in_word = false;
            self.quoted = false;
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
            probing: false,
            rereads: 0,
        }
    }

    /// The line was read to its end and can be trusted.
    fn read(&self) -> bool {
        !self.bail && self.rereads <= MAX_REREADS
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

    /// `(( … ))` at `self.i`: skipped whole, its words being names and numbers
    /// rather than commands. `false`, with nothing consumed, when the group
    /// does not end in `))`: then it is a subshell in a subshell.
    fn arithmetic(&mut self) -> bool {
        if self.probing || self.rereads > MAX_REREADS {
            return false;
        }
        let (start, heredocs, bail) = (self.i, self.heredocs.len(), self.bail);
        self.i += 2;
        self.probing = true;
        self.skip_nested();
        self.probing = false;
        // `1 << 2` in there is a shift, not a heredoc.
        self.heredocs.truncate(heredocs);
        if !self.bail && self.peek(0) == Some(')') {
            self.i += 1;
            return true;
        }
        self.i = start;
        self.bail = bail;
        self.rereads += 1;
        false
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
        p.redirected = true;
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

    /// The simple commands of the line and the operators between them, in
    /// order. `nested` reads up to the `)` that closes the group the caller
    /// opened.
    fn statements(&mut self, nested: bool) -> Vec<Tok> {
        let mut p = Pending::default();
        let mut parens = 0usize;
        // Inside `[[ … ]]`, where `&&`, `||`, `(`, `<` belong to the test.
        let mut test = false;
        while let Some(c) = self.peek(0) {
            if test
                && matches!(
                    c,
                    '&' | '|' | '(' | ')' | '<' | '>' | ';' | '\n' | ' ' | '\t' | '\r'
                )
            {
                p.end_word();
                if p.bare && p.words.last().is_some_and(|w| w == "]]") {
                    // closed: read this character again as part of the line
                    test = false;
                } else {
                    self.i += 1;
                }
                continue;
            }
            match c {
                '\\' => match (self.peek(1), self.peek(2)) {
                    (Some('\n'), _) => self.i += 2,
                    (Some('\r'), Some('\n')) => self.i += 3,
                    (Some(n), _) => {
                        p.push(n);
                        p.quoted = true;
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
                    p.quoted = true;
                    self.single_quoted(&mut p, ansi);
                }
                '"' => {
                    self.i += 1;
                    p.quoted = true;
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
                    p.operator(Tok::Seq);
                    self.heredoc_bodies();
                }
                ' ' | '\t' | '\r' => {
                    self.i += 1;
                    p.end_word();
                    // `[[` where a command goes opens a test.
                    test = matches!(p.words.split_last(), Some((w, before))
                        if w == "[[" && p.bare && before.iter().all(|b| RESERVED.contains(&b.as_str())));
                }
                ';' if matches!(self.peek(1), Some(';' | '&')) => {
                    // `;;`, `;&`, `;;&`
                    self.i += 2;
                    if self.peek(0) == Some('&') {
                        self.i += 1;
                    }
                    p.operator(Tok::Branch);
                }
                ';' => {
                    self.i += 1;
                    p.operator(Tok::Seq);
                }
                '|' if self.peek(1) == Some('|') => {
                    self.i += 2;
                    p.operator(Tok::Or);
                }
                '|' => {
                    // `|` and `|&`
                    self.i += if self.peek(1) == Some('&') { 2 } else { 1 };
                    p.operator(Tok::Pipe);
                }
                '&' if self.peek(1) == Some('>') => {
                    // `&>file`, `&>>file`
                    self.i += 1;
                    p.end_word();
                    self.redirection(&mut p);
                }
                '&' if self.peek(1) == Some('&') => {
                    self.i += 2;
                    p.operator(Tok::And);
                }
                '&' => {
                    self.i += 1;
                    p.operator(Tok::Background);
                }
                '<' if self.peek(1) == Some('<') && self.peek(2) != Some('<') => {
                    p.drop_fd_or_end_word();
                    p.redirected = true;
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
                '(' if self.peek(1) == Some('(') && !p.in_word && self.arithmetic() => {
                    p.operator(Tok::Arithmetic);
                }
                '(' => {
                    self.i += 1;
                    p.operator(Tok::Open);
                    parens += 1;
                }
                ')' => {
                    self.i += 1;
                    if parens > 0 {
                        parens -= 1;
                    } else if nested {
                        p.end_statement();
                        return p.out;
                    }
                    p.operator(Tok::Close);
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

/// The word a statement starts with, past the reserved words that only
/// introduce one (`then case …`, `do case …`).
fn first_word(words: &[String]) -> Option<&str> {
    words
        .iter()
        .map(String::as_str)
        .find(|w| !RESERVED.contains(w))
}

/// `case x in a|pytest) echo ok ;; esac`: what stands between `in` or `;;`
/// and the next `)` is a pattern to match, not a command. Patterns are
/// dropped, with the `|` between them and the `)` that ends them; the
/// branches' commands stay.
fn without_case_patterns(toks: Vec<Tok>) -> Vec<Tok> {
    #[derive(PartialEq)]
    enum At {
        /// After `case x`, before its `in`.
        Head,
        Patterns,
        Body,
    }
    let mut out = Vec::with_capacity(toks.len());
    // One entry per `case` that is open.
    let mut open: Vec<At> = Vec::new();
    for tok in toks {
        let first = match &tok {
            Tok::Cmd(words) => first_word(words),
            _ => None,
        };
        match (open.last(), &tok) {
            (Some(At::Patterns), _) if first == Some("esac") => {
                open.pop();
                out.push(tok);
            }
            (Some(At::Patterns), Tok::Close) => *open.last_mut().expect("open") = At::Body,
            (Some(At::Patterns), _) => {}
            (Some(At::Head), Tok::Seq) => {}
            (Some(At::Head), _) if first == Some("in") => {
                *open.last_mut().expect("open") = At::Patterns;
            }
            (Some(At::Head), _) => {
                // not a `case` this reads
                open.pop();
                out.push(tok);
            }
            (_, Tok::Cmd(words)) if first == Some("case") => {
                // `case x in a` keeps `case x in`: the rest is a pattern.
                let cut = words.iter().position(|w| w == "in").map(|i| i + 1);
                open.push(if cut.is_some() {
                    At::Patterns
                } else {
                    At::Head
                });
                let keep = cut.unwrap_or(words.len());
                out.push(Tok::Cmd(words[..keep].to_vec()));
            }
            (Some(At::Body), Tok::Branch) => {
                *open.last_mut().expect("open") = At::Patterns;
                out.push(tok);
            }
            (Some(At::Body), _) if first == Some("esac") => {
                open.pop();
                out.push(tok);
            }
            _ => out.push(tok),
        }
    }
    out
}

/// The line as commands and operators; `None` when it cannot be read.
fn tokens(cmd: &str) -> Option<Vec<Tok>> {
    let mut lx = Lexer::new(cmd);
    let out = lx.statements(false);
    lx.read().then(|| without_case_patterns(out))
}

fn simple_commands(cmd: &str) -> Vec<Vec<String>> {
    tokens(cmd)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|t| match t {
            Tok::Cmd(words) if !words.is_empty() => Some(words),
            _ => None,
        })
        .collect()
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

/// Where the command of a statement starts: past reserved words, assignments
/// and the prefixes that run what follows them.
fn command_start(words: &[String]) -> Option<usize> {
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
    Some(i)
}

/// Flags that make a runner print, list or prepare instead of run, in
/// whichever of the runners below has them. Each ends 0 with nothing
/// checked: `cargo test --no-run` compiles, `pytest --collect-only` lists,
/// `npm test --if-present` does nothing when there is no script. A flag is
/// listed only if it runs nothing in every runner that takes it (not
/// `--all`: it prints options in `tsc` and is a real run in `cargo test`).
const RUNS_NOTHING: &[&str] = &[
    // any of them
    "--help",
    "-h",
    "--version",
    // cargo test, and `cargo test -- --list`; playwright test, nox
    "--no-run",
    "--list",
    // pytest
    "--collect-only",
    "--co",
    "--fixtures",
    "--markers",
    "--setup-plan",
    "--setup-only",
    // jest, tsc, eslint
    "--listTests",
    "--showConfig",
    "--clearCache",
    "--init",
    "--listFilesOnly",
    "--print-config",
    "--env-info",
    // mocha, rspec, gradle, make
    "--dry-run",
    "--just-print",
    "--recon",
    "--question",
    "--touch",
    // dotnet test, swift test, phpunit
    "--list-tests",
    "--list-groups",
    "--list-suites",
    // tox
    "--listenvs",
    "--showconfig",
    "--notest",
    // npm, pnpm, yarn
    "--if-present",
];

/// Is the runner asked for something other than a run? Long flags are
/// looked for in every argument, past `--` too: there they reach the tool
/// behind `npm test --` and the test binary behind `cargo test --`. Short
/// ones mean different things to different runners (`-n` is `make`'s dry
/// run and pytest-xdist's worker count), so they are each runner's own.
fn runs_nothing(name: &str, args: &[&str]) -> bool {
    let long = |a: &str| {
        RUNS_NOTHING.iter().any(|f| {
            a.strip_prefix(f)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('='))
        })
    };
    if args.iter().any(|a| long(a)) {
        return true;
    }
    let own = args.iter().take_while(|a| **a != "--");
    let arg = |n: usize| args.get(n).copied().unwrap_or("");
    match name {
        // `go test -c` builds the test binary; `-list` and `-n` print.
        "go" => own
            .into_iter()
            .any(|a| matches!(*a, "-c" | "-n" | "-list") || a.starts_with("-list=")),
        "make" => own.into_iter().any(|a| make_runs_nothing(a)),
        // `-m` is the dry run; `-x test` leaves the tests out of `check`.
        "gradle" | "gradlew" => {
            let left_out = args.windows(2).any(|pair| {
                matches!(pair[0], "-x" | "--exclude-task")
                    && (pair[1].ends_with("test") || pair[1].ends_with("check"))
            });
            left_out || own.into_iter().any(|a| *a == "-m")
        }
        // `-DskipTests` skips; `-DskipTests=false` turns them back on.
        "mvn" | "mvnw" => own.into_iter().any(|a| {
            ["-DskipTests", "-Dmaven.test.skip"].iter().any(|skip| {
                a.strip_prefix(skip)
                    .is_some_and(|v| v.is_empty() || (v.starts_with('=') && v != "=false"))
            })
        }),
        "dotnet" => own.into_iter().any(|a| *a == "-t"),
        "tox" => {
            matches!(arg(0), "list" | "l" | "config" | "c")
                || own.into_iter().any(|a| matches!(*a, "-l" | "-a"))
        }
        "nox" => own.into_iter().any(|a| *a == "-l"),
        "pytest" => own.into_iter().any(|a| *a == "-V"),
        "vitest" => matches!(arg(0), "list" | "init"),
        "swift" => arg(1) == "list",
        _ => false,
    }
}

/// `make -n`, `-q` and `-t` run no recipe, alone or in a cluster (`-kn`).
/// The letters after an option that carries a value are that value:
/// `-Iinclude` and `-Otarget` run the recipe.
fn make_runs_nothing(arg: &str) -> bool {
    let Some(letters) = arg.strip_prefix('-').filter(|l| !l.starts_with('-')) else {
        return false;
    };
    letters
        .chars()
        .take_while(|c| !"CEfIjloOW".contains(*c))
        .any(|c| matches!(c, 'n' | 'q' | 't'))
}

/// The kind of check one simple command runs, if it runs one.
fn statement_kind(words: &[String], depth: usize) -> Option<&'static str> {
    let i = command_start(words)?;
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
    if runs_nothing(name, &args) {
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

/// What the exit status of a piece of a line says about the checks in it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Reads {
    /// It runs a check.
    runs: bool,
    /// Status 0 means every check in it ran and exited 0.
    zero: bool,
    /// A non-zero status may be a check's: one that failed, or one the line
    /// never reached because the command before it failed.
    nonzero: bool,
}

/// A command that is not a check: its status says nothing about one.
const PLAIN: Reads = Reads {
    runs: false,
    zero: true,
    nonzero: false,
};
/// A runner: its status is its own.
const CHECK: Reads = Reads {
    runs: true,
    zero: true,
    nonzero: true,
};

/// Command words after which the line's status is not followed: a compound
/// command (`if`, `for`, `{`, `[[`), a negation, or a builtin that ends or
/// replaces the shell reading the line, or runs text as part of it. `exit 0;
/// cargo test` and `exec true && cargo test` end 0 and start no runner.
const NOT_FOLLOWED: &[&str] = &[
    "if", "then", "elif", "else", "fi", "while", "until", "do", "done", "for", "case", "esac",
    "select", "function", "{", "}", "!", "[[", "coproc", "exit", "return", "exec", "logout",
    "trap", "eval", "setopt", "unsetopt",
];

/// The word the shell itself acts on: past assignments and the words that
/// hand the next one to a builtin (`time exit 0`, `builtin exit 0`).
fn shell_word(words: &[String]) -> Option<(&str, &[String])> {
    let i = words.iter().position(|w| {
        !is_assignment(w) && !matches!(w.as_str(), "time" | "-p" | "command" | "builtin")
    })?;
    Some((words[i].as_str(), &words[i + 1..]))
}

/// `set -euo pipefail` / `bash -o pipefail`: `Some(true)` turns the option
/// on, `Some(false)` off, `None` leaves it.
fn pipefail_option(args: &[String]) -> Option<bool> {
    args.windows(2)
        .filter(|pair| pair[1] == "pipefail" && pair[0].len() >= 2 && pair[0].ends_with('o'))
        .filter_map(|pair| match pair[0].chars().next() {
            Some('-') if !pair[0].starts_with("--") => Some(true),
            Some('+') => Some(false),
            _ => None,
        })
        .next_back()
}

/// `set -e`, `set -o errexit`, `bash -ec`: a failure may end the line there.
fn errexit_option(args: &[String]) -> bool {
    args.iter().enumerate().any(|(i, a)| {
        let named = a == "errexit" && i > 0 && args[i - 1].starts_with('-');
        let flag = a.starts_with('-') && !a.starts_with("--") && a.contains('e');
        named || flag
    })
}

/// The options a line is read under.
#[derive(Clone, Copy, Default)]
struct Options {
    /// `pipefail` is on: the line opens by turning it on and never touches
    /// it again, or its shell was started with it.
    pipefail: bool,
    /// The line turns `pipefail` on somewhere, so it may be on.
    may_pipefail: bool,
    /// The line turns `-e` on somewhere, so a failure may end it early.
    may_errexit: bool,
}

/// A `set` that changes only how failures are reported (`-e`, `-u`, `-x`,
/// `-o pipefail`). Any other may change what runs: under `set -n` the shell
/// reads `cargo test`, runs nothing and ends 0.
fn is_plain_set(args: &[String]) -> bool {
    let mut i = 0;
    while let Some(a) = args.get(i).map(String::as_str) {
        let flags = match a.strip_prefix('-').or_else(|| a.strip_prefix('+')) {
            Some(f) if !f.is_empty() && f.chars().all(|c| "euxo".contains(c)) => f,
            _ => return false,
        };
        i += 1;
        if flags.ends_with('o') {
            let known = ["pipefail", "errexit", "nounset", "xtrace"];
            if !args.get(i).is_some_and(|o| known.contains(&o.as_str())) {
                return false;
            }
            i += 1;
        }
    }
    true
}

/// Nothing in the line takes its status somewhere this does not follow.
fn is_followed(toks: &[Tok]) -> bool {
    toks.iter().all(|t| match t {
        Tok::Background | Tok::Arithmetic => false,
        Tok::Cmd(words) => match shell_word(words) {
            // An expansion in the command word may name any of the above.
            Some((w, _)) if NOT_FOLLOWED.contains(&w) || w.contains(UNKNOWN) => false,
            Some(("set", args)) => is_plain_set(args),
            Some(("shopt", args)) => !args.iter().any(|a| a.starts_with('-') && a.contains('o')),
            _ => true,
        },
        _ => true,
    })
}

/// The options in force for a line whose shell started with `shell`
/// (`bash -o pipefail -c …`). `pipefail` is held to be on only when the line
/// opens with the `set`s that turn it on and has no other `set` after them.
fn options_of(toks: &[Tok], shell: Options) -> Options {
    let set_args = |t: &'_ Tok| match t {
        Tok::Cmd(words) => match shell_word(words) {
            Some(("set", args)) => Some(args.to_vec()),
            _ => None,
        },
        _ => None,
    };
    let mut on = shell.pipefail;
    let mut i = 0;
    while let (Some(Tok::Cmd(words)), Some(Tok::Seq | Tok::And)) = (toks.get(i), toks.get(i + 1)) {
        if words.first().map(String::as_str) != Some("set") {
            break;
        }
        on = pipefail_option(&words[1..]).unwrap_or(on);
        i += 2;
    }
    let sets: Vec<Vec<String>> = toks.iter().filter_map(set_args).collect();
    Options {
        pipefail: on && !toks[i..].iter().any(|t| set_args(t).is_some()),
        may_pipefail: shell.may_pipefail || sets.iter().any(|a| pipefail_option(a) == Some(true)),
        may_errexit: shell.may_errexit || sets.iter().any(|a| errexit_option(a)),
    }
}

/// Follows a line's exit status back to the commands it comes from.
struct Reader<'a> {
    toks: &'a [Tok],
    i: usize,
    /// `sh -c` nesting, as in `statement_kind`.
    depth: usize,
    /// `( … )` nesting.
    nest: usize,
    options: Options,
    /// The line has a shape this does not read.
    lost: bool,
}

impl<'a> Reader<'a> {
    fn peek(&self) -> Option<&'a Tok> {
        self.toks.get(self.i)
    }

    /// `a; b; c`: the status is the last command's, and the others' is
    /// discarded. `nested` reads up to the `)` that closes a subshell.
    fn list(&mut self, nested: bool) -> Reads {
        let mut discarded = false;
        let mut last = PLAIN;
        loop {
            match self.peek() {
                None => {
                    self.lost |= nested;
                    break;
                }
                Some(Tok::Close) => {
                    self.i += 1;
                    self.lost |= !nested;
                    break;
                }
                Some(Tok::Seq) => {
                    self.i += 1;
                    continue;
                }
                _ => {}
            }
            discarded |= last.runs;
            last = self.and_or();
            if !matches!(self.peek(), None | Some(Tok::Seq | Tok::Close)) {
                self.lost = true;
                break;
            }
        }
        Reads {
            runs: discarded || last.runs,
            zero: last.zero && !discarded,
            // Under `set -e` the line may have ended at an earlier check.
            nonzero: last.nonzero || (self.options.may_errexit && discarded),
        }
    }

    /// `a && b || c`, left to right.
    fn and_or(&mut self) -> Reads {
        let mut left = self.pipeline();
        loop {
            let or = match self.peek() {
                Some(Tok::And) => false,
                Some(Tok::Or) => true,
                _ => return left,
            };
            self.i += 1;
            let right = self.pipeline();
            let runs = left.runs || right.runs;
            left = if or {
                // 0 when either side is, so a check on either side may have
                // failed or never started; non-zero when both sides failed.
                Reads {
                    runs,
                    zero: !runs,
                    nonzero: left.nonzero || right.nonzero,
                }
            } else {
                // 0 when both sides are; non-zero when the left failed, and
                // then the right never started, or when the right failed.
                Reads {
                    runs,
                    zero: left.zero && right.zero,
                    nonzero: left.nonzero || right.runs,
                }
            };
        }
    }

    /// `a | b | c`: the status is the last command's, unless `pipefail`
    /// makes it the last one that failed.
    fn pipeline(&mut self) -> Reads {
        let mut units = vec![self.unit()];
        while matches!(self.peek(), Some(Tok::Pipe)) {
            self.i += 1;
            units.push(self.unit());
        }
        let (last, before) = units.split_last().expect("one unit at least");
        let runs = units.iter().any(|u| u.runs);
        if self.options.pipefail {
            Reads {
                runs,
                zero: units.iter().all(|u| u.zero),
                nonzero: units.iter().any(|u| u.nonzero),
            }
        } else {
            Reads {
                runs,
                zero: last.zero && !before.iter().any(|u| u.runs),
                nonzero: last.nonzero
                    || (self.options.may_pipefail && before.iter().any(|u| u.nonzero)),
            }
        }
    }

    /// A simple command, or a `( … )` subshell.
    fn unit(&mut self) -> Reads {
        match self.peek() {
            Some(Tok::Cmd(words)) => {
                self.i += 1;
                // `name() { … }` defines a function and runs nothing.
                self.lost |= matches!(self.peek(), Some(Tok::Open));
                self.command(words)
            }
            Some(Tok::Open) if self.nest < MAX_DEPTH => {
                self.i += 1;
                self.nest += 1;
                let inner = self.list(true);
                self.nest -= 1;
                inner
            }
            // Nothing where a command goes, or nested too deep to read.
            _ => {
                self.lost = true;
                self.i = self.toks.len();
                PLAIN
            }
        }
    }

    fn command(&mut self, words: &[String]) -> Reads {
        let Some(i) = command_start(words) else {
            return PLAIN;
        };
        // `sh -c '…'` ends with the status of its script.
        if SHELLS.contains(&basename(&words[i])) {
            let args: Vec<&str> = words[i + 1..].iter().map(String::as_str).collect();
            return match shell_script(&args) {
                Some(script) if self.depth < 2 => {
                    // `bash -o pipefail -c '…'`: the options before the script.
                    let options = args.iter().position(|a| std::ptr::eq(*a, script));
                    let options = &words[i + 1..i + 1 + options.unwrap_or(0)];
                    let on = pipefail_option(options) == Some(true);
                    let shell = Options {
                        pipefail: on,
                        may_pipefail: on,
                        may_errexit: errexit_option(options),
                    };
                    read_line(script, self.depth + 1, shell)
                }
                _ => PLAIN,
            };
        }
        if statement_kind(words, self.depth).is_some() {
            CHECK
        } else {
            PLAIN
        }
    }
}

/// What the exit status of a whole line (or of a `sh -c` script) says.
fn read_line(cmd: &str, depth: usize, shell: Options) -> Reads {
    // A line this does not read: 0 says nothing about its checks. Non-zero
    // is still taken as theirs, as it was before lines were read at all.
    let runs = line_kind(cmd, depth).is_some();
    let unread = Reads {
        runs,
        zero: false,
        nonzero: runs,
    };
    let Some(toks) = tokens(cmd).filter(|toks| is_followed(toks)) else {
        return unread;
    };
    let mut reader = Reader {
        toks: &toks,
        i: 0,
        depth,
        nest: 0,
        options: options_of(&toks, shell),
        lost: false,
    };
    let reads = reader.list(false);
    if reader.lost {
        unread
    } else {
        reads
    }
}

/// Is the status a check line ended with a statement about its checks?
///
/// `passed` and `failed` are read off the exit code, so the code has to be
/// the runner's. With `zero`: does status 0 mean every check in the line ran
/// and exited 0? Not for `cargo test | tail` (the status is `tail`'s),
/// `cargo test || true`, `cargo test; echo done`, `cargo test &`, or
/// `false && cargo test; true`, which never starts the runner. Without
/// `zero`: may a non-zero status be a check's own? Not for `cargo test |
/// grep -q FAILED`, which ends 1 when nothing failed.
///
/// The two directions are not held to the same standard. A wrong yes with
/// `zero` turns "tests pass" into `verified`, so it takes a line read from
/// end to end: commands joined by `&&`, a check at the end of a `;` list, a
/// pipeline under `set -o pipefail`, a `( … )` subshell, `sh -c` of the
/// same. `set -e` is not credited: a shell told to ignore the status of the
/// line (as some agents' wrappers do) ignores `-e` inside it. A wrong yes
/// without `zero` reports a failed line as a failed check, which it nearly
/// always is, so it is a no only where the line plainly ends with another
/// command's status: `cargo test && git push` failing counts, and so does
/// `cargo test; echo done` once the line has a `set -e` that may have ended
/// it at the test.
///
/// A line that is not read (a loop, an `if`, `exit`, `eval`, a command named
/// by an expansion) answers no with `zero`, and yes without.
pub fn exit_speaks_for_checks(cmd: &str, zero: bool) -> bool {
    let reads = read_line(cmd, 0, Options::default());
    reads.runs && if zero { reads.zero } else { reads.nonzero }
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
            // after a test, an arithmetic expression, inside a `case` branch
            "[[ -f Cargo.toml ]] && cargo test",
            "[[ -n $CI || -n $FORCE ]] && npm test",
            "if [[ $1 =~ ^(a|b)$ ]]; then npm test; fi",
            "[[ -f a ]]; cargo test",
            "(( n > 0 )) && cargo test",
            "(( n << 2 ))\ncargo test",
            "((cd app && npm test) && echo ok)",
            "case $1 in\n  unit) cargo test ;;\n  lint) cargo clippy ;;\nesac",
            "case x in a) cd app\n\n npm test;; esac",
            "case x in (a|b) npm test;& c) true;; esac",
            "case x in a) case y in b) pytest;; esac;; esac",
            "case x\nin\n a) pytest;;\nesac",
            "case x in a) true;; esac; cargo test",
            // flags that still run
            "cargo test --all",
            "cargo fmt --all --check",
            "cargo test list",
            "cargo test -- --nocapture --test-threads 1",
            "pytest -x -q -m slow -n 4 -k list",
            "pytest --cov=src --color=yes",
            "go test -run TestList -count=1 -cover ./...",
            "make test -j4 -k",
            "make test -Iinclude -Otarget -f tests.mk",
            "mvn test -DskipTests=false",
            "mvn verify -Dmaven.test.skip=false",
            // a quoted `[[` is a command of that name, not a test
            "\"[[\" -n x ]] && pytest",
            "npx vitest run --reporter=verbose",
            "npm test -- --passWithNoTests",
            "./gradlew check -x lint",
            "mvn verify -q -B",
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
            // a word in a test, an arithmetic expression or a `case` pattern
            "[[ -n x && pytest ]]",
            "[[ -n x || cargo test ]]",
            "[[ -n x && pytest ]] && echo ok",
            "if [[ -n pytest ]]; then echo; fi",
            "! [[ -z x || eslint ]]",
            // a quoted or escaped `]]` is an operand: the test goes on
            "[[ \"]]\" = x && pytest ]]",
            "[[ ']]' = ']]' && pytest ]]",
            "[[ \\]\\] = x || pytest ]]",
            "(( pytest + 1 ))",
            "(( x = 1 | jest ))",
            "for (( i = 0; i < 3; i++ )); do echo $i; done",
            "case x in a|pytest) echo ok;; esac",
            "case $1 in\n  pytest) echo one ;;\n  jest|vitest) echo two ;;\nesac",
            "case x in (a|eslint) true;& tsc) true;; esac",
            "case x in a) case y in pytest) echo;; esac;; jest) echo;; esac",
            // asked to print, list or prepare, not to run
            "cargo test --no-run",
            "cargo test -- --list",
            "cargo test -h",
            "pytest --collect-only",
            "pytest --co -q",
            "python -m pytest --collect-only tests/",
            "python3 -m unittest -h",
            "make -n test",
            "make test --dry-run",
            "make -kn test",
            "make test -q",
            "make test -sn",
            "make test -Onone -n",
            "mvn test -DskipTests=true",
            "go test -list . ./...",
            "go test -list=Foo ./...",
            "go test -c ./pkg",
            "npx jest --listTests",
            "npx vitest list",
            "pnpm vitest list",
            "npx playwright test --list",
            "npm test -- --listTests",
            "npm test -- --help",
            "npm run test --if-present",
            "mvn test -DskipTests",
            "mvn verify -Dmaven.test.skip=true",
            "./gradlew test --dry-run",
            "gradle check -x test",
            "npx tsc --showConfig",
            "npx tsc --init",
            "npx eslint --print-config src/a.ts",
            "dotnet test --list-tests",
            "tox --notest",
            "bash -c 'cd app && pytest --co'",
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
            tokens("a && b || c | d |& e; (f) & g\n> h").unwrap(),
            vec![
                Tok::Cmd(vec!["a".into()]),
                Tok::And,
                Tok::Cmd(vec!["b".into()]),
                Tok::Or,
                Tok::Cmd(vec!["c".into()]),
                Tok::Pipe,
                Tok::Cmd(vec!["d".into()]),
                Tok::Pipe,
                Tok::Cmd(vec!["e".into()]),
                Tok::Seq,
                Tok::Open,
                Tok::Cmd(vec!["f".into()]),
                Tok::Close,
                Tok::Background,
                Tok::Cmd(vec!["g".into()]),
                Tok::Seq,
                // only a redirection: still a command with a status
                Tok::Cmd(vec![]),
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
        // A test and an arithmetic expression are one piece each, whatever
        // operators they hold; `case` patterns are not commands.
        let cmd = |words: &[&str]| Tok::Cmd(words.iter().map(|w| w.to_string()).collect());
        assert_eq!(
            tokens("[[ -n a && ( b || c ) ]] && d; (( e | f )); ((g) | h)").unwrap(),
            vec![
                cmd(&["[[", "-n", "a", "b", "c", "]]"]),
                Tok::And,
                cmd(&["d"]),
                Tok::Seq,
                Tok::Arithmetic,
                Tok::Seq,
                Tok::Open,
                Tok::Open,
                cmd(&["g"]),
                Tok::Close,
                Tok::Pipe,
                cmd(&["h"]),
                Tok::Close,
            ]
        );
        assert_eq!(
            tokens("case x in a|b) c;; (d) (e);& f) g\nesac; h").unwrap(),
            vec![
                cmd(&["case", "x", "in"]),
                cmd(&["c"]),
                Tok::Branch,
                Tok::Open,
                cmd(&["e"]),
                Tok::Close,
                Tok::Branch,
                cmd(&["g"]),
                Tok::Seq,
                cmd(&["esac"]),
                Tok::Seq,
                cmd(&["h"]),
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
    fn a_status_of_zero_is_the_checks_only_when_the_line_ends_with_theirs() {
        for c in [
            "cargo test",
            "cargo test 2>&1",
            "npm test > out.log 2>&1",
            "cd src-tauri && cargo test",
            "cargo fmt --check && cargo clippy -- -D warnings && cargo test",
            "ls; cargo test",
            "echo starting\ncargo test",
            "CI=1 timeout 600 npm test",
            "(cd app && npm test)",
            "source .venv/bin/activate && pytest -q",
            "yes | npm test",
            "cd a || cd b && cargo test",
            "cat <<EOF > notes.txt\nsome text\nEOF\ncargo test",
            // an operator that is not one
            "cargo test # | tail",
            "cargo test -- --nocapture '| tail'",
            // pipefail makes a pipeline end with the status of what failed
            "set -o pipefail; cargo test 2>&1 | tail -20",
            "set -euo pipefail\ncargo test | tee out.log",
            "set -e; set -o pipefail; cargo test | tail",
            "set -o pipefail && cd app && npm test | tail -5",
            "bash -o pipefail -c 'npm test | tee out.log'",
            "bash -c 'set -o pipefail; npm test | tail'",
            // a script ends with the status of its last command
            "bash -c 'cd app && npm test'",
            "sh -c \"cargo test\"",
        ] {
            assert!(exit_speaks_for_checks(c, true), "{c}");
        }
        for c in [
            // the status is another command's
            "cargo test | tail -20",
            "cargo test 2>&1 | tail -20",
            "cargo test |& tee out.log",
            "(cargo test) | tail",
            "cargo test || true",
            "cargo test || echo failed",
            "cargo test; echo done",
            "cargo test\necho done",
            "cargo test; > out.txt",
            "npm run lint; npm test",
            "(cargo test; true)",
            "cargo test && npm test | tail",
            "cargo test &",
            "cargo test & wait",
            "bash -c 'cargo test | tail'",
            "bash -c 'cargo test; true'",
            // the runner may never have started
            "false && cargo test; true",
            "exit 0; cargo test",
            "exec true && cargo test",
            "builtin exit 0; cargo test",
            "time exit 0; cargo test",
            "set -n; cargo test",
            "set -o noexec; cargo test",
            "shopt -so noexec; cargo test",
            "\"$SETUP\" && cargo test",
            "eval \"$SETUP\" && cargo test",
            "trap 'exit 0' EXIT; cargo test",
            // compound commands are not followed
            "! cargo test",
            "if cargo test; then echo ok; fi",
            "for p in a b; do cargo test -p $p; done",
            "while ! cargo test; do sleep 1; done",
            "{ cargo test; } || true",
            "cargo() { true; }; cargo test",
            "[[ -n x && pytest ]]",
            "(( pytest + 1 ))",
            "case x in a|pytest) echo ok;; esac",
            "cargo test | tail; exit ${PIPESTATUS[0]}",
            // pipefail only where the line plainly turns it on
            "bash -c 'npm test | tail' -o pipefail",
            "cd app && set -o pipefail && npm test | tail",
            "set -o pipefail; set +o pipefail; npm test | tail",
            "set -o pipefail || true; npm test | tail",
            "set +o pipefail; npm test | tail",
            "(set -o pipefail; npm test | tail)",
            "bash -o pipefail -c 'set +o pipefail; npm test | tail'",
            // `-e` is not credited: a wrapper may have it ignored
            "set -e; cargo test; echo done",
            // not a check at all
            "ls",
            "echo cargo test",
        ] {
            assert!(!exit_speaks_for_checks(c, true), "{c}");
        }
    }

    #[test]
    fn a_failed_line_is_a_failed_check_unless_the_status_is_another_commands() {
        for c in [
            "cargo test",
            "cd app && cargo test",
            "cargo test && git push",
            "cargo test || echo failed",
            "ls; cargo test",
            "(cd app && npm test)",
            "set -o pipefail; cargo test | tail",
            "bash -c 'cargo test'",
            // `set -e` or pipefail somewhere: the line may have ended at the check
            "set -e; cargo test; echo done",
            "set -euo pipefail\ncargo fmt --check\ncargo test\necho done",
            "bash -ec 'cargo test; echo done'",
            "cd app && set -o pipefail && npm test | tail",
            // not followed: taken as the check's, as before
            "if ! cargo test; then exit 1; fi",
            "for p in a b; do cargo test -p $p || exit 1; done",
            "cargo test | tail; exit ${PIPESTATUS[0]}",
        ] {
            assert!(exit_speaks_for_checks(c, false), "{c}");
        }
        for c in [
            // `grep -q` ends 1 when nothing failed
            "cargo test | grep -q FAILED",
            "cargo test | grep -q FAILED && echo found",
            "bash -c 'cargo test | grep -q FAILED'",
            "cargo test | tail",
            "cargo test; false",
            "cargo test; git push",
            "ls",
        ] {
            assert!(!exit_speaks_for_checks(c, false), "{c}");
        }
    }

    #[test]
    fn a_line_is_followed_within_bounds() {
        // Model-written input: bounded recursion, no panic.
        let deep = format!("{}cargo test{}", "(".repeat(5000), ")".repeat(5000));
        assert!(!exit_speaks_for_checks(&deep, true));
        let spaced = format!("{}cargo test{}", "( ".repeat(5000), " )".repeat(5000));
        assert!(!exit_speaks_for_checks(&spaced, true));
        for c in [
            "",
            ")",
            "&&",
            "| |",
            "cargo test &&",
            "cargo test )",
            "( cargo test",
            "set -o",
            "set é; cargo test",
            "bash -c",
            "\u{1} && cargo test",
        ] {
            let _ = exit_speaks_for_checks(c, true);
            let _ = exit_speaks_for_checks(c, false);
        }
        assert!(exit_speaks_for_checks("( ( cd app && npm test ) )", true));
    }

    #[test]
    fn a_run_of_open_parens_is_read_once_not_once_per_nesting() {
        // `((` is tried as arithmetic and read again as two subshells when
        // it is not. Tried again inside every attempt, that multiplied: 40
        // parentheses took half a minute, with the ledger waiting on it.
        let lines = vec![
            // valid shell: subshells in subshells, closed one by one
            format!("{}true{}", "(".repeat(60), " )".repeat(60)),
            format!("{}cargo test{}", "(".repeat(60), " )".repeat(60)),
            "(".repeat(60),
            format!("echo $({}", "(".repeat(60)),
            format!("{}true{}", "(".repeat(5000), " )".repeat(5000)),
            "(".repeat(100_000),
        ];
        let (done, wait) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in &lines {
                let _ = is_check_command(line);
                let _ = check_kind(line);
                let _ = exit_speaks_for_checks(line, true);
                let _ = exit_speaks_for_checks(line, false);
            }
            let _ = done.send(());
        });
        assert!(
            wait.recv_timeout(std::time::Duration::from_secs(20))
                .is_ok(),
            "a run of `(` is still being read when it should be done"
        );

        // What a person types still reads the same.
        for c in [
            "((cargo test) )",
            "(((cargo test) ) )",
            "((cargo test); (echo done))",
            "((cd app && npm test) | (cat))",
            "(( n = (1 + 2) * 3 )); cargo test",
            "echo $(( (1 + 2) * 3 )) && cargo test",
        ] {
            assert!(is_check_command(c), "{c}");
        }
        for c in [
            "(( pytest + 1 ))",
            "(( (pytest) + ((1)) ))",
            "(( a = ((pytest)) ))",
            "echo $(( ((pytest)) ))",
        ] {
            assert!(!is_check_command(c), "{c}");
        }
        // Past the bound the line is not read: a miss, not a guess.
        let many = format!("{}cargo test{}", "(".repeat(200), " )".repeat(200));
        assert!(!is_check_command(&many));
        assert!(!exit_speaks_for_checks(&many, true));
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
