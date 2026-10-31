//! Text scans over program source: includes (`accumulate <iri>.`) and the
//! monotonicity label (`lat.md/elpi#Reconciled Materialization`).

/// The text with `%` comments, `/* */` comments and string literals blanked,
/// so a scan sees only code. Line structure is kept.
pub fn code_only(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '%' => {
                for d in it.by_ref() {
                    if d == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if it.peek() == Some(&'*') => {
                it.next();
                let mut prev = ' ';
                for d in it.by_ref() {
                    if d == '\n' {
                        out.push('\n');
                    }
                    if prev == '*' && d == '/' {
                        break;
                    }
                    prev = d;
                }
                out.push(' ');
            }
            '"' => {
                out.push('"');
                let mut esc = false;
                for d in it.by_ref() {
                    if esc {
                        esc = false;
                        continue;
                    }
                    match d {
                        '\\' => esc = true,
                        '"' => break,
                        '\n' => out.push('\n'),
                        _ => {}
                    }
                }
                out.push('"');
            }
            _ => out.push(c),
        }
    }
    out
}

/// The program IRIs a text includes, in order: each line `accumulate <iri>.`.
pub fn includes(text: &str) -> Vec<String> {
    code_only(text).lines().filter_map(include_of).collect()
}

fn include_of(line: &str) -> Option<String> {
    let rest = line.trim().strip_prefix("accumulate")?.trim_start();
    let rest = rest.strip_prefix('<')?;
    let end = rest.find('>')?;
    let after = rest[end + 1..].trim();
    (after == ".").then(|| rest[..end].to_string())
}

/// The text with its include lines removed (blanked, so line numbers in the
/// engine's messages stay right).
pub fn strip_includes(text: &str) -> String {
    let code = code_only(text);
    text.lines()
        .zip(code.lines().chain(std::iter::repeat("")))
        .map(|(line, c)| if include_of(c).is_some() { "" } else { line })
        .collect::<Vec<_>>()
        .join("\n")
}

/// True when the program may be non-monotonic: its code uses negation as
/// failure, cut, `findall` or `tm.retracted`.
// @tg: implements:: [[elpi#Reconciled Materialization]] {since: "2026-10-31"}
pub fn non_monotonic(text: &str) -> bool {
    let code = code_only(text);
    let word = |w: &str| {
        code.match_indices(w).any(|(i, _)| {
            let before = code[..i].chars().next_back();
            let after = code[i + w.len()..].chars().next();
            let ident = |c: char| c.is_alphanumeric() || c == '_' || c == '.' || c == '-';
            !before.is_some_and(ident) && !after.is_some_and(ident)
        })
    };
    code.contains("\\+")
        || code.contains('!')
        || word("not")
        || word("std.findall")
        || word("findall")
        || word("tm.retracted")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn includes_and_strip() {
        let t = "% lib\naccumulate <urn:ex:lib>.\np X :- q X. % accumulate <urn:ex:no>.\n";
        assert_eq!(includes(t), vec!["urn:ex:lib".to_string()]);
        assert!(!strip_includes(t).contains("accumulate <urn:ex:lib>"));
        assert_eq!(strip_includes(t).lines().count(), 3);
    }

    #[test]
    fn monotonicity() {
        assert!(!non_monotonic("p X :- q X. % not here\nr \"!\"."));
        assert!(non_monotonic("p X :- q X, not (r X)."));
        assert!(non_monotonic("p X :- q X, !."));
        assert!(non_monotonic("p L :- std.findall (q _) L."));
        assert!(!non_monotonic("p X :- notify X."));
    }
}
