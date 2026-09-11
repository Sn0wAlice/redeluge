// SPDX-License-Identifier: GPL-3.0-or-later
//! Rendering `index.html`.
//!
//! The page is a Mako template, but it uses two constructs and nothing else:
//! `${name}` substitution and `% for x in list:` / `% endfor`. Implementing
//! those keeps the shipped file the source of truth, which matters because the
//! Web UI is supposed to be the same one.
//!
//! Anything else Mako can do is deliberately not supported: if the template
//! grows a condition or a Python expression, this fails loudly at render time
//! rather than emitting something subtly wrong.

use std::collections::HashMap;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("line {line}: unsupported template directive: {directive}")]
    Unsupported { line: usize, directive: String },

    #[error("line {line}: `% for` with no matching `% endfor`")]
    UnclosedLoop { line: usize },

    #[error("line {line}: `% endfor` with no `% for`")]
    StrayEndfor { line: usize },

    #[error("line {line}: no value for `${{{name}}}`")]
    MissingValue { line: usize, name: String },
}

pub type Result<T> = std::result::Result<T, Error>;

/// The values `index.html` asks for.
#[derive(Debug, Default)]
pub struct Context {
    scalars: HashMap<String, String>,
    lists: HashMap<String, Vec<String>>,
}

impl Context {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(mut self, name: &str, value: impl Into<String>) -> Self {
        self.scalars.insert(name.to_owned(), value.into());
        self
    }

    pub fn list(mut self, name: &str, values: Vec<String>) -> Self {
        self.lists.insert(name.to_owned(), values);
        self
    }
}

/// Renders the template.
pub fn render(template: &str, context: &Context) -> Result<String> {
    let mut out = String::with_capacity(template.len() * 2);
    let lines: Vec<&str> = template.lines().collect();
    let mut index = 0;

    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim_start();

        if let Some(directive) = trimmed.strip_prefix('%') {
            let directive = directive.trim();

            if let Some(header) = directive.strip_prefix("for ") {
                let (variable, collection) = parse_for(header, index + 1)?;
                let end = find_endfor(&lines, index + 1, index + 1)?;
                let body = &lines[index + 1..end];

                // An unknown collection renders nothing, matching Mako's
                // behaviour for an empty list rather than failing the page.
                for item in context.lists.get(&collection).into_iter().flatten() {
                    let mut inner = Context::new();
                    inner.scalars.clone_from(&context.scalars);
                    inner.lists.clone_from(&context.lists);
                    inner = inner.set(&variable, item.clone());
                    out.push_str(&render(&body.join("\n"), &inner)?);
                    out.push('\n');
                }

                index = end + 1;
                continue;
            }

            if directive == "endfor" {
                return Err(Error::StrayEndfor { line: index + 1 });
            }

            return Err(Error::Unsupported {
                line: index + 1,
                directive: directive.to_owned(),
            });
        }

        out.push_str(&substitute(line, context, index + 1)?);
        out.push('\n');
        index += 1;
    }

    Ok(out)
}

fn parse_for(header: &str, line: usize) -> Result<(String, String)> {
    let header = header.trim().trim_end_matches(':');
    let (variable, collection) = header
        .split_once(" in ")
        .ok_or_else(|| Error::Unsupported {
            line,
            directive: format!("for {header}"),
        })?;
    Ok((variable.trim().to_owned(), collection.trim().to_owned()))
}

fn find_endfor(lines: &[&str], from: usize, opened_at: usize) -> Result<usize> {
    let mut depth = 1usize;
    for (offset, line) in lines.iter().enumerate().skip(from) {
        let trimmed = line.trim_start();
        if let Some(directive) = trimmed.strip_prefix('%') {
            let directive = directive.trim();
            if directive.starts_with("for ") {
                depth += 1;
            } else if directive == "endfor" {
                depth -= 1;
                if depth == 0 {
                    return Ok(offset);
                }
            }
        }
    }
    Err(Error::UnclosedLoop { line: opened_at })
}

/// Unwraps `_("text")` or `_('text')` to `text`.
fn translation_literal(expression: &str) -> Option<String> {
    let inner = expression.strip_prefix("_(")?.strip_suffix(')')?.trim();
    let quote = inner.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let body = inner.strip_prefix(quote)?.strip_suffix(quote)?;
    // Only a plain literal; anything else is a real expression this engine
    // does not evaluate, and guessing at it would be worse than failing.
    if body.contains(quote) {
        return None;
    }
    Some(body.to_owned())
}

fn substitute(line: &str, context: &Context, line_number: usize) -> Result<String> {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;

    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after.find('}').ok_or_else(|| Error::Unsupported {
            line: line_number,
            directive: "unterminated ${".to_owned(),
        })?;

        let expression = after[..end].trim();

        // `${_("text")}` is a translation lookup. redeluge is English only, so
        // it resolves to the text itself, the same identity gettext.js uses.
        let value = match translation_literal(expression) {
            Some(literal) => literal,
            None => {
                context
                    .scalars
                    .get(expression)
                    .cloned()
                    .ok_or_else(|| Error::MissingValue {
                        line: line_number,
                        name: expression.to_owned(),
                    })?
            }
        };
        out.push_str(&value);
        rest = &after[end + 1..];
    }

    out.push_str(rest);
    Ok(out)
}
