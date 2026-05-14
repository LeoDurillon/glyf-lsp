use std::{cell::Cell, sync::LazyLock};

use regex::{Captures, Regex};
use tower_lsp::lsp_types::{Position, Range};

/// Strips any complete HTML tags at the start of `s`.
/// Safe because `<` can no longer appear in a Glyf abbreviation
/// (the text content operator is now `>>`), so any leading `<…>`
/// is unambiguously HTML context surrounding the user's abbreviation.
fn strip_html_context(mut s: &str) -> &str {
    loop {
        s = s.trim_start();
        if s.starts_with('<') {
            match s.find('>') {
                Some(pos) => s = &s[pos + 1..],
                None => break, // unclosed tag — stop stripping
            }
        } else {
            break;
        }
    }
    s.trim_start()
}

/// Trims whitespace leading tags and strips an optional `return ` prefix.
pub fn extract_abbreviation(line: &str, cursor: u32) -> &str {
    let abbr = line[..cursor.min(line.len() as u32) as usize].trim();
    let abbr = abbr.strip_prefix("return ").map(str::trim).unwrap_or(abbr);
    strip_html_context(abbr)
}

/// Get the range of the abbreviation, given the cursor position and the abbreviation length
pub fn abbreviation_range(pos: Position, abbr_len: usize) -> Range {
    Range {
        start: Position {
            line: pos.line,
            character: (pos.character as usize).saturating_sub(abbr_len) as u32,
        },
        end: pos,
    }
}

static OPENING_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^/][^>]+/?>").unwrap());

static EMPTY_ATTR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#" ([\w-]+)="\$""#).unwrap());

static EMPTY_ELEM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r">(</[\w-]*>)").unwrap());

pub fn insert_tabstops(html: &str) -> String {
    let counter = Cell::new(1usize);

    let with_attr_stops = OPENING_TAG.replace_all(html, |tag_caps: &Captures| {
        let tag = &tag_caps[0];
        EMPTY_ATTR
            .replace_all(tag, |attr_caps: &Captures| {
                let n = counter.get();
                counter.set(n + 1);
                format!(" {}=\"${{{}}}\"", &attr_caps[1], n)
            })
            .into_owned()
    });

    EMPTY_ELEM
        .replace_all(&with_attr_stops, |capture: &Captures| {
            let n = counter.get();
            counter.set(n + 1);
            format!(">${{{}}}{}", n, &capture[1])
        })
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // extract_abbreviation
    // -------------------------------------------------------------------------
    mod extract_abbreviation_tests {
        use super::*;

        #[test]
        fn returns_text_up_to_cursor() {
            assert_eq!(extract_abbreviation("div", 3), "div");
        }

        #[test]
        fn trims_surrounding_whitespace() {
            assert_eq!(extract_abbreviation("  div  ", 7), "div");
        }

        #[test]
        fn stops_at_cursor_not_end_of_line() {
            // cursor at 3: only "div", not "div>p"
            assert_eq!(extract_abbreviation("div>p", 3), "div");
        }

        #[test]
        fn cursor_clamped_to_line_length() {
            // cursor far beyond line length → whole line
            assert_eq!(extract_abbreviation("div", 999), "div");
        }

        #[test]
        fn strips_return_prefix() {
            assert_eq!(extract_abbreviation("return div", 10), "div");
        }

        #[test]
        fn strips_return_prefix_with_extra_inner_space() {
            // "return  div" — double space after "return"
            assert_eq!(extract_abbreviation("return  div", 11), "div");
        }

        #[test]
        fn does_not_strip_return_without_trailing_space() {
            // "returnValue" has no "return " (with space) prefix
            assert_eq!(extract_abbreviation("returnValue", 11), "returnValue");
        }

        #[test]
        fn empty_line_returns_empty_string() {
            assert_eq!(extract_abbreviation("", 0), "");
        }

        #[test]
        fn cursor_at_zero_returns_empty_string() {
            assert_eq!(extract_abbreviation("div", 0), "");
        }

        #[test]
        fn strips_leading_html_tag() {
            assert_eq!(extract_abbreviation("<div>abbr", 9), "abbr");
        }

        #[test]
        fn strips_tag_with_attributes() {
            assert_eq!(extract_abbreviation("<div class=\"foo\">abbr", 21), "abbr");
        }

        #[test]
        fn strips_multiple_leading_tags() {
            assert_eq!(extract_abbreviation("<div><span>abbr", 15), "abbr");
        }

        #[test]
        fn tag_alone_returns_empty() {
            assert_eq!(extract_abbreviation("<div>", 5), "");
        }

        #[test]
        fn preserves_text_content_operator() {
            assert_eq!(extract_abbreviation("<div>p>>Hello", 13), "p>>Hello");
        }

        #[test]
        fn non_html_prefix_is_untouched() {
            assert_eq!(extract_abbreviation("div>p", 5), "div>p");
        }
    }

    // -------------------------------------------------------------------------
    // abbreviation_range
    // -------------------------------------------------------------------------
    mod abbreviation_range_tests {
        use super::*;

        #[test]
        fn produces_correct_start_and_end() {
            let pos = Position {
                line: 3,
                character: 8,
            };
            let range = abbreviation_range(pos, 3);
            assert_eq!(range.start.line, 3);
            assert_eq!(range.start.character, 5); // 8 − 3
            assert_eq!(range.end.line, 3);
            assert_eq!(range.end.character, 8);
        }

        #[test]
        fn end_always_equals_cursor_position() {
            let pos = Position {
                line: 1,
                character: 10,
            };
            assert_eq!(abbreviation_range(pos, 4).end, pos);
        }

        #[test]
        fn start_saturates_at_zero() {
            // abbreviation longer than cursor position
            let pos = Position {
                line: 0,
                character: 2,
            };
            assert_eq!(abbreviation_range(pos, 10).start.character, 0);
        }

        #[test]
        fn zero_length_abbreviation_gives_empty_range() {
            let pos = Position {
                line: 0,
                character: 5,
            };
            let range = abbreviation_range(pos, 0);
            assert_eq!(range.start.character, 5);
            assert_eq!(range.end.character, 5);
        }
    }
    mod insert_tabstops_tests {
        use super::*;

        #[test]
        fn empty_element_gets_final_stop() {
            assert_eq!(insert_tabstops("<div></div>"), "<div>${1}</div>");
        }

        #[test]
        fn boolean_attribute_gets_numbered_stop() {
            assert_eq!(
                insert_tabstops(r#"<a href="$"></a>"#),
                "<a href=\"${1}\">${2}</a>"
            );
        }

        #[test]
        fn multiple_boolean_attrs_numbered_sequentially() {
            assert_eq!(
                insert_tabstops(r#"<img src="$" alt="$" />"#),
                "<img src=\"${1}\" alt=\"${2}\" />"
            );
        }

        #[test]
        fn valued_attribute_is_not_touched() {
            assert_eq!(
                insert_tabstops("<div class=\"foo\"></div>"),
                "<div class=\"foo\">${1}</div>"
            );
        }

        #[test]
        fn self_closing_without_attrs_unchanged() {
            assert_eq!(insert_tabstops("<br />"), "<br />");
        }

        #[test]
        fn nested_structure_only_empty_child_gets_stop() {
            assert_eq!(
                insert_tabstops("<ul>\n\t<li></li>\n</ul>"),
                "<ul>\n\t<li>${1}</li>\n</ul>"
            );
        }
    }
}
