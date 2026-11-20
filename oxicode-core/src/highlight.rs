use crate::theme::{Palette, Rgb};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Plain,
    Keyword,
    String,
    Comment,
    Number,
    Type,
    Punct,
}

#[derive(Debug, Clone, Copy)]
pub struct FileIconSpec {
    pub badge: &'static str,
    pub color: Rgb,
}

pub fn file_icon(path: &str, pal: Palette) -> FileIconSpec {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let name = Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase();

    if name == "cargo.toml" || name == "cargo.lock" {
        return FileIconSpec {
            badge: "Ⓡ",
            color: Rgb::new(222, 130, 84),
        };
    }

    match ext.as_str() {
        "rs" => FileIconSpec {
            badge: "Ⓡ",
            color: Rgb::new(222, 130, 84),
        },
        "py" | "pyw" => FileIconSpec {
            badge: "Ⓟ",
            color: Rgb::new(92, 156, 245),
        },
        "ts" | "tsx" => FileIconSpec {
            badge: "Ⓣ",
            color: Rgb::new(86, 166, 245),
        },
        "js" | "jsx" | "mjs" => FileIconSpec {
            badge: "Ⓙ",
            color: Rgb::new(229, 192, 123),
        },
        "go" => FileIconSpec {
            badge: "Ⓖ",
            color: Rgb::new(86, 182, 194),
        },
        "toml" | "yaml" | "yml" | "ini" => FileIconSpec {
            badge: "⚙",
            color: pal.muted,
        },
        "json" | "jsonc" => FileIconSpec {
            badge: "{}",
            color: pal.yellow,
        },
        "md" | "mdx" | "txt" => FileIconSpec {
            badge: "▤",
            color: pal.cyan,
        },
        "html" | "xml" | "svg" => FileIconSpec {
            badge: "◇",
            color: pal.accent,
        },
        "css" | "scss" => FileIconSpec {
            badge: "◈",
            color: pal.secondary,
        },
        "sh" | "bash" | "ps1" | "bat" => FileIconSpec {
            badge: "$",
            color: pal.green,
        },
        "c" | "cpp" | "cc" | "h" | "hpp" => FileIconSpec {
            badge: "Ⓒ",
            color: pal.secondary,
        },
        _ => FileIconSpec {
            badge: "▪",
            color: pal.primary,
        },
    }
}

pub fn lang_badge(lang: &str, pal: Palette) -> (&'static str, Rgb) {
    match lang.trim().to_lowercase().as_str() {
        "rust" | "rs" => ("Ⓡ rust", Rgb::new(222, 130, 84)),
        "python" | "py" => ("Ⓟ python", Rgb::new(92, 156, 245)),
        "typescript" | "ts" | "tsx" => ("Ⓣ typescript", Rgb::new(86, 166, 245)),
        "javascript" | "js" | "jsx" => ("Ⓙ javascript", Rgb::new(229, 192, 123)),
        "go" | "golang" => ("Ⓖ go", Rgb::new(86, 182, 194)),
        "json" => ("{} json", pal.yellow),
        "toml" => ("⚙ toml", pal.muted),
        "yaml" | "yml" => ("⚙ yaml", pal.muted),
        "bash" | "sh" | "shell" | "powershell" | "ps1" => ("$ shell", pal.green),
        "html" | "xml" | "svg" => ("◇ markup", pal.accent),
        "css" | "scss" => ("◈ css", pal.secondary),
        _ if !lang.trim().is_empty() => ("▪ code", pal.cyan),
        _ => ("▪ code", pal.muted),
    }
}

fn is_keyword(word: &str, lang: &str) -> bool {
    let l = lang.to_lowercase();
    let common = matches!(
        word,
        "if" | "else"
            | "for"
            | "while"
            | "loop"
            | "return"
            | "break"
            | "continue"
            | "in"
            | "match"
            | "switch"
            | "case"
            | "true"
            | "false"
            | "null"
            | "None"
            | "Some"
            | "Ok"
            | "Err"
            | "try"
            | "catch"
            | "async"
            | "await"
            | "import"
            | "from"
            | "export"
            | "default"
    );
    if common {
        return true;
    }
    match l.as_str() {
        "rust" | "rs" => matches!(
            word,
            "fn" | "let"
                | "mut"
                | "pub"
                | "struct"
                | "enum"
                | "impl"
                | "trait"
                | "use"
                | "mod"
                | "crate"
                | "self"
                | "Self"
                | "where"
                | "type"
                | "const"
                | "static"
                | "ref"
                | "move"
                | "as"
                | "dyn"
        ),
        "python" | "py" => matches!(
            word,
            "def" | "class"
                | "lambda"
                | "with"
                | "as"
                | "pass"
                | "yield"
                | "raise"
                | "not"
                | "and"
                | "or"
                | "is"
                | "elif"
                | "True"
                | "False"
                | "self"
        ),
        "typescript" | "ts" | "tsx" | "javascript" | "js" | "jsx" => matches!(
            word,
            "function"
                | "const"
                | "let"
                | "var"
                | "class"
                | "interface"
                | "type"
                | "enum"
                | "extends"
                | "implements"
                | "new"
                | "this"
                | "typeof"
                | "undefined"
        ),
        "go" | "golang" => matches!(
            word,
            "func" | "package" | "var" | "const" | "type" | "struct" | "interface" | "go" | "defer" | "chan" | "select"
        ),
        _ => false,
    }
}

pub fn tokenize_code_line<'a>(line: &'a str, lang: &str) -> Vec<(&'a str, TokenKind)> {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") || (trimmed.starts_with('#') && !trimmed.starts_with("#[")) {
        return vec![(line, TokenKind::Comment)];
    }

    let mut tokens = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let b = bytes[i];

        if b == b'"' || b == b'\'' {
            let quote = b;
            let start = i;
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 2;
                    continue;
                }
                if bytes[i] == quote {
                    i += 1;
                    break;
                }
                i += 1;
            }
            tokens.push((&line[start..i], TokenKind::String));
            continue;
        }

        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            tokens.push((&line[i..], TokenKind::Comment));
            break;
        }

        if b.is_ascii_alphabetic() || b == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &line[start..i];
            let kind = if is_keyword(word, lang) {
                TokenKind::Keyword
            } else if word.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                TokenKind::Type
            } else {
                TokenKind::Plain
            };
            tokens.push((word, kind));
            continue;
        }

        if b.is_ascii_digit() {
            let start = i;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.')
            {
                i += 1;
            }
            tokens.push((&line[start..i], TokenKind::Number));
            continue;
        }

        let ch_len = line[i..].chars().next().map(char::len_utf8).unwrap_or(1);
        let start = i;
        i += ch_len;
        while i < bytes.len() {
            let nb = bytes[i];
            if nb == b'"'
                || nb == b'\''
                || nb == b'/'
                || nb.is_ascii_alphabetic()
                || nb == b'_'
                || nb.is_ascii_digit()
            {
                break;
            }
            i += line[i..].chars().next().map(char::len_utf8).unwrap_or(1);
        }
        tokens.push((&line[start..i], TokenKind::Punct));
    }

    if tokens.is_empty() {
        tokens.push(("", TokenKind::Plain));
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ThemeKind;

    #[test]
    fn highlights_rust_line() {
        let tokens = tokenize_code_line("pub fn main() -> usize { 42 }", "rust");
        assert!(tokens.iter().any(|(w, k)| *w == "pub" && *k == TokenKind::Keyword));
        assert!(tokens.iter().any(|(w, k)| *w == "fn" && *k == TokenKind::Keyword));
        assert!(tokens.iter().any(|(w, k)| *w == "42" && *k == TokenKind::Number));
    }

    #[test]
    fn picks_file_icons() {
        let pal = ThemeKind::OpenCodeBeige.palette();
        assert_eq!(file_icon("src/main.rs", pal).badge, "Ⓡ");
        assert_eq!(file_icon("app.py", pal).badge, "Ⓟ");
        assert_eq!(file_icon("Cargo.toml", pal).badge, "Ⓡ");
    }
}
