// SPDX-License-Identifier: GPL-3.0-or-later
// Making the script bundles smaller.
//
// Included by `build.rs` as well as compiled into the library, so the same
// code the build runs is the code the tests exercise. Plain comments rather
// than doc comments, because `include!` puts this somewhere a module-level doc
// comment is not allowed.

/// Strips comments and indentation from JavaScript.
///
/// Deliberately conservative: line breaks are kept, so automatic semicolon
/// insertion behaves exactly as it did, and nothing is renamed. A real
/// minifier would save more and would have to be trusted against ExtJS-era
/// code, which predates every one of them. This takes about a third off, and
/// the rest is taken by the gzip the server now applies.
///
/// The parts that have to be right are the ones where `//` is not a comment:
/// inside a string, inside a regular expression literal, and after an escape.
pub fn minify(source: &[u8]) -> Vec<u8> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mode {
        Code,
        LineComment,
        BlockComment,
        Quoted(u8),
        Regex,
    }

    let mut out: Vec<u8> = Vec::with_capacity(source.len());
    let mut mode = Mode::Code;
    let mut escaped = false;
    // What came before, for telling a regular expression from a division.
    let mut previous = 0u8;
    let mut index = 0;

    while index < source.len() {
        let byte = source[index];
        let next = source.get(index + 1).copied().unwrap_or(0);

        match mode {
            Mode::Code => match (byte, next) {
                (b'/', b'/') => {
                    mode = Mode::LineComment;
                    index += 2;
                    continue;
                }
                (b'/', b'*') => {
                    mode = Mode::BlockComment;
                    index += 2;
                    continue;
                }
                (b'/', _) if starts_regex(previous) => {
                    mode = Mode::Regex;
                    out.push(byte);
                }
                (b'"', _) | (b'\'', _) => {
                    mode = Mode::Quoted(byte);
                    out.push(byte);
                }
                _ => out.push(byte),
            },
            Mode::LineComment => {
                if byte == b'\n' {
                    mode = Mode::Code;
                    out.push(byte);
                }
            }
            Mode::BlockComment => {
                if byte == b'*' && next == b'/' {
                    mode = Mode::Code;
                    index += 2;
                    continue;
                }
                // A block comment spanning lines would otherwise join the code
                // before it to the code after it.
                if byte == b'\n' {
                    out.push(byte);
                }
            }
            Mode::Quoted(quote) => {
                out.push(byte);
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == quote {
                    mode = Mode::Code;
                }
            }
            Mode::Regex => {
                out.push(byte);
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'/' || byte == b'\n' {
                    mode = Mode::Code;
                }
            }
        }

        if !byte.is_ascii_whitespace() && matches!(mode, Mode::Code | Mode::Quoted(_) | Mode::Regex)
        {
            previous = byte;
        }
        index += 1;
    }

    // Now drop the indentation and the lines that are left empty.
    let text = String::from_utf8_lossy(&out).into_owned();
    let mut result = String::with_capacity(text.len());
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        result.push_str(trimmed);
        result.push('\n');
    }
    result.into_bytes()
}

/// Whether a `/` here opens a regular expression rather than dividing.
///
/// The rule a tokeniser uses: a slash after a value divides, and a slash after
/// an operator or the start of a statement opens a literal. Getting it wrong
/// on ExtJS's own source would eat half a file.
pub fn starts_regex(previous: u8) -> bool {
    matches!(
        previous,
        0 | b'('
            | b','
            | b'='
            | b':'
            | b'['
            | b'!'
            | b'&'
            | b'|'
            | b'?'
            | b'{'
            | b'}'
            | b';'
            | b'+'
            | b'-'
            | b'*'
            | b'<'
            | b'>'
            | b'~'
            | b'^'
            | b'%'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minified(source: &str) -> String {
        String::from_utf8(minify(source.as_bytes())).unwrap()
    }

    #[test]
    fn line_and_block_comments_go() {
        assert_eq!(minified("var a = 1; // why\n"), "var a = 1;\n");
        assert_eq!(minified("/* a note */\nvar a = 1;\n"), "var a = 1;\n");
        assert_eq!(minified("var a = /* inline */ 1;\n"), "var a =  1;\n");
    }

    #[test]
    fn a_block_comment_spanning_lines_leaves_the_lines() {
        // Otherwise the code before it joins the code after it, and automatic
        // semicolon insertion does something different.
        let out = minified("var a = 1\n/* one\ntwo\nthree */\nvar b = 2\n");
        assert_eq!(out, "var a = 1\nvar b = 2\n");
    }

    #[test]
    fn indentation_and_blank_lines_go() {
        assert_eq!(
            minified("    var a = 1;\n\n\n    var b = 2;\n"),
            "var a = 1;\nvar b = 2;\n"
        );
    }

    #[test]
    fn line_breaks_are_kept_so_semicolon_insertion_behaves() {
        // The whole reason this is conservative. Joining these two lines
        // changes what the program means.
        let out = minified("return\n{ a: 1 }\n");
        assert_eq!(out, "return\n{ a: 1 }\n");
    }

    #[test]
    fn a_comment_marker_inside_a_string_is_not_a_comment() {
        assert_eq!(
            minified("var url = 'http://example.com/x';\n"),
            "var url = 'http://example.com/x';\n"
        );
        assert_eq!(
            minified("var a = \"/* not a comment */\";\n"),
            "var a = \"/* not a comment */\";\n"
        );
    }

    #[test]
    fn an_escaped_quote_does_not_end_the_string() {
        assert_eq!(
            minified("var a = 'it\\'s // fine';\n"),
            "var a = 'it\\'s // fine';\n"
        );
    }

    #[test]
    fn a_regular_expression_containing_slashes_survives() {
        // This is the one that eats a file when it is wrong.
        assert_eq!(
            minified("var re = /https?:\\/\\//;\nvar a = 1;\n"),
            "var re = /https?:\\/\\//;\nvar a = 1;\n"
        );
        assert_eq!(
            minified("if (/[/]/.test(x)) { y(); }\n"),
            "if (/[/]/.test(x)) { y(); }\n"
        );
    }

    #[test]
    fn a_division_is_not_mistaken_for_a_regular_expression() {
        assert_eq!(minified("var a = b / c; // half\n"), "var a = b / c;\n");
        assert_eq!(minified("var a = (b + c) / 2;\n"), "var a = (b + c) / 2;\n");
    }

    #[test]
    fn the_output_is_smaller_but_not_empty() {
        let source = concat!(
            "/**\n",
            " * A comment block of the kind every ExtJS file starts with.\n",
            " */\n",
            "Ext.define('A', {\n",
            "    // a note\n",
            "    value: 1,\n",
            "});\n",
        );
        let out = minified(source);
        assert!(out.len() < source.len() / 2, "{out}");
        assert!(out.contains("Ext.define('A'"));
        assert!(out.contains("value: 1"));
        assert!(!out.contains("a note"));
    }

    #[test]
    fn an_unterminated_string_does_not_run_away() {
        // A truncated file must come back truncated, not panic.
        let out = minified("var a = 'unterminated\n");
        assert!(out.starts_with("var a = 'unterminated"));
    }
}
