//! Lexical RON compaction: never round-trip parsed numbers or fixed-point game data.

/// Removes comments and whitespace without changing literal spellings.
///
/// This deliberately does not validate RON. An unterminated literal is copied through
/// EOF, and an unterminated comment is discarded through EOF. Callers validate game
/// documents separately. Whitespace at potentially ambiguous token boundaries is
/// retained as one ASCII space, including on malformed input.
pub fn minify_ron(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;
    let mut separated = false;
    let mut previous = None;

    while let Some(first) = rest.chars().next() {
        if is_whitespace(first) {
            rest = &rest[first.len_utf8()..];
            separated = true;
            continue;
        }
        if rest.starts_with("//") {
            rest = &rest[rest.find('\n').unwrap_or(rest.len())..];
            separated = true;
            continue;
        }
        if rest.starts_with("/*") {
            rest = &rest[block_comment_len(rest)..];
            separated = true;
            continue;
        }

        if separated && previous.is_some_and(|last| needs_separator(last, first)) {
            output.push(' ');
        }
        let len = literal_len(rest).unwrap_or_else(|| first.len_utf8());
        let token = &rest[..len];
        output.push_str(token);
        previous = token.chars().next_back();
        rest = &rest[len..];
        separated = false;
    }

    output
}

// RON 0.12.2 src/parse.rs::is_whitespace_char, rather than char::is_whitespace:
// direction marks are whitespace, while e.g. NBSP and EM SPACE are not.
const fn is_whitespace(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t'
            | '\n'
            | '\r'
            | '\x0b'
            | '\x0c'
            | '\u{85}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{2028}'
            | '\u{2029}'
    )
}

// RON raw identifiers also accept '.', '+', and '-'. Treat all non-ASCII
// characters conservatively as identifier continuations, avoiding another Unicode
// dependency while covering XID continuations such as combining marks.
const fn is_atom_char(c: char) -> bool {
    !c.is_ascii() || c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '+' | '-')
}

const fn needs_separator(left: char, right: char) -> bool {
    (is_atom_char(left) && (is_atom_char(right) || matches!(right, '#' | '"' | '\'')))
        || (left == '#' && (is_atom_char(right) || matches!(right, '#' | '"' | '\'')))
        || matches!(
            (left, right),
            ('/' | '*', '/') | ('/', '*') | (':', ':') | ('.', '=') | ('#', '!')
        )
}

const fn block_comment_len(input: &str) -> usize {
    let bytes = input.as_bytes();
    let mut depth = 1;
    let mut cursor = 2;
    while cursor + 1 < bytes.len() {
        match (bytes[cursor], bytes[cursor + 1]) {
            (b'/', b'*') => {
                depth += 1;
                cursor += 2;
            }
            (b'*', b'/') => {
                depth -= 1;
                cursor += 2;
                if depth == 0 {
                    return cursor;
                }
            }
            _ => cursor += 1,
        }
    }
    input.len()
}

fn literal_len(input: &str) -> Option<usize> {
    // A raw literal requires a quote after *all* hashes. r#foo is an identifier.
    let raw_prefix = if input.starts_with("br") {
        Some(2)
    } else if input.starts_with('r') {
        Some(1)
    } else {
        None
    };
    if let Some(prefix) = raw_prefix {
        let hashes = input[prefix..].bytes().take_while(|&b| b == b'#').count();
        let quote = prefix + hashes;
        if input.as_bytes().get(quote) == Some(&b'"') {
            let ending = format!("\"{}", &input[prefix..quote]);
            let body = quote + 1;
            return Some(
                input[body..]
                    .find(&ending)
                    .map_or(input.len(), |end| body + end + ending.len()),
            );
        }
    }

    let prefix = usize::from(input.starts_with('b'));
    match input.as_bytes().get(prefix) {
        Some(b'"') => Some(string_len(input, prefix + 1)),
        Some(b'\'') => Some(char_len(input, prefix + 1)),
        _ => None,
    }
}

fn string_len(input: &str, start: usize) -> usize {
    let mut escaped = false;
    for (offset, c) in input[start..].char_indices() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '"' {
            return start + offset + 1;
        }
    }
    input.len()
}

fn char_len(input: &str, start: usize) -> usize {
    // Unlike strings, a character consumes exactly one character or escape before
    // its closing quote. RON even accepts an unescaped quote character: '''.
    let mut chars = input[start..].char_indices();
    match chars.next() {
        Some((_, '\\')) => match chars.next() {
            Some((_, 'u')) => {
                if chars.next().is_none_or(|(_, c)| c != '{') {
                    return input.len();
                }
                if !chars.any(|(_, c)| c == '}') {
                    return input.len();
                }
            }
            Some((_, 'x')) => {
                if chars.next().is_none() || chars.next().is_none() {
                    return input.len();
                }
            }
            Some(_) => {}
            None => return input.len(),
        },
        Some(_) => {}
        None => return input.len(),
    }
    match chars.next() {
        Some((offset, '\'')) => start + offset + 1,
        _ => input.len(),
    }
}

#[cfg(test)]
mod tests {
    use rts_base::{MapData, Rules, Sim};

    use super::minify_ron;

    const RULES: &str = include_str!("../../../assets/data/core.rules.ron");
    const MAP: &str = include_str!("../../../assets/maps/proving_grounds.map.ron");

    #[test]
    fn bundled_documents_shrink_and_preserve_all_game_fields() {
        #[derive(serde::Deserialize)]
        struct UnitFields {
            units: Vec<rts_base::UnitDef>,
        }

        let rules_text = minify_ron(RULES);
        let map_text = minify_ron(MAP);
        assert!(rules_text.len() < RULES.len());
        assert!(map_text.len() < MAP.len());
        let before_rules = Rules::from_ron(RULES).unwrap();
        let after_rules = Rules::from_ron(&rules_text).unwrap();
        assert_eq!(before_rules.tile_capacity(), after_rules.tile_capacity());
        let terrain_fields = |rules: &Rules| {
            rules
                .terrain_types()
                .iter()
                .map(|t| {
                    (
                        t.id.clone(),
                        t.symbol,
                        t.speed_modifier,
                        t.color.clone(),
                        t.texture.clone(),
                    )
                })
                .collect::<Vec<_>>()
        };
        let unit_fields = |text: &str| {
            ron::from_str::<UnitFields>(text)
                .unwrap()
                .units
                .iter()
                .map(|u| (u.id.clone(), u.size, u.max_health, u.radius, u.speed))
                .collect::<Vec<_>>()
        };
        assert_eq!(terrain_fields(&before_rules), terrain_fields(&after_rules));
        assert_eq!(unit_fields(RULES), unit_fields(&rules_text));

        let before_map = MapData::from_ron(MAP).unwrap();
        let after_map = MapData::from_ron(&map_text).unwrap();
        assert_eq!(before_map.name, after_map.name);
        assert_eq!(before_map.tiles, after_map.tiles);
        let starts = |map: &MapData| map.players.iter().map(|p| p.start).collect::<Vec<_>>();
        let placements = |map: &MapData| {
            map.units
                .iter()
                .map(|u| (u.kind.clone(), u.owner, u.pos))
                .collect::<Vec<_>>()
        };
        assert_eq!(starts(&before_map), starts(&after_map));
        assert_eq!(placements(&before_map), placements(&after_map));
        assert_eq!(
            Sim::new(before_rules, &before_map).unwrap().state_hash(),
            Sim::new(after_rules, &after_map).unwrap().state_hash()
        );
    }

    #[test]
    fn numeric_spellings_are_unchanged() {
        let input = "[ 0.30000000000000004, -0.0, +1.25e-3, 1_000.5_0f64, 0xffu16, 0b10, 0o77, 18446744073709551615u64 ]";
        let expected = "[0.30000000000000004,-0.0,+1.25e-3,1_000.5_0f64,0xffu16,0b10,0o77,18446744073709551615u64]";
        assert_eq!(minify_ron(input), expected);
        let original: Vec<f64> = ron::from_str("[0.30000000000000004, -0.0, +1.25e-3]").unwrap();
        let compact: Vec<f64> =
            ron::from_str(&minify_ron("[0.30000000000000004, -0.0, +1.25e-3]")).unwrap();
        assert_eq!(
            original.iter().map(|f| f.to_bits()).collect::<Vec<_>>(),
            compact.iter().map(|f| f.to_bits()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn removes_nested_comments_and_line_comments() {
        assert_eq!(
            minify_ron(
                "/*前 /* nested */ 後*/ ( a: 1, // line\rstill comment\u{2028}also comment\n b: 2 /* \" // /*深*/ */ ) // tail"
            ),
            "(a:1,b:2)"
        );
        assert_eq!(minify_ron("foo/* outer /* nested */ end */bar"), "foo bar");
    }

    #[test]
    fn preserves_escaped_strings_and_character_symbols() {
        let input = r#" ( "a \" // /* still a string */ \\ b", '\'', '\\', '/', '*', '"', ''' , '\u{1f600}', '\x41' ) "#;
        let expected =
            r#"("a \" // /* still a string */ \\ b",'\'','\\','/','*','"',''','\u{1f600}','\x41')"#;
        assert_eq!(minify_ron(input), expected);
        let original: (String, char, char, char, char, char, char, char, char) =
            ron::from_str(input).unwrap();
        let compact: (String, char, char, char, char, char, char, char, char) =
            ron::from_str(&minify_ron(input)).unwrap();
        assert_eq!(original, compact);
    }

    #[test]
    fn preserves_raw_strings_with_arbitrary_hash_delimiters() {
        for hashes in [0, 1, 2, 17, 256] {
            let hashes = "#".repeat(hashes);
            let literal = format!("r{hashes}\" // /* \\ ' 😀 \n body \"{hashes}");
            let input = format!("[ /* outside */ {literal} ]");
            assert_eq!(minify_ron(&input), format!("[{literal}]"));
            assert_eq!(
                ron::from_str::<Vec<String>>(&input).unwrap(),
                ron::from_str::<Vec<String>>(&minify_ron(&input)).unwrap()
            );
        }
        let input = "[r###\"an inner \"# and \"## /* // end\"###]";
        assert_eq!(minify_ron(input), input);
    }

    #[test]
    fn preserves_byte_literals() {
        let input =
            r####" [ b"// \" /* \\ \xff", br###" // "# /* \ "###, b'\'', b'/', b'\x41' ] "####;
        let expected = r####"[b"// \" /* \\ \xff",br###" // "# /* \ "###,b'\'',b'/',b'\x41']"####;
        assert_eq!(minify_ron(input), expected);
        assert_eq!(ron::from_str::<u8>("b'/'").unwrap(), 47);
    }

    #[test]
    fn preserves_unicode_and_raw_identifiers() {
        #[derive(Debug, PartialEq, serde::Deserialize)]
        struct Fields {
            #[serde(rename = "type")]
            kind: String,
            #[serde(rename = "поля")]
            unicode: String,
            #[serde(rename = "a.b+-")]
            raw: u32,
        }
        let input = "( r#type: r#\" // Україна 😀 /* \"#, поля: \"café e\u{301}\", r#a.b+-: 42 )";
        let compact = minify_ron(input);
        assert_eq!(
            compact,
            "(r#type:r#\" // Україна 😀 /* \"#,поля:\"café e\u{301}\",r#a.b+-:42)"
        );
        assert_eq!(
            ron::from_str::<Fields>(input).unwrap(),
            ron::from_str::<Fields>(&compact).unwrap()
        );
    }

    #[test]
    fn preserves_extension_and_metadata_attributes() {
        #[derive(Debug, PartialEq, serde::Deserialize)]
        struct Field {
            value: Option<String>,
        }
        let input = "# /*comment*/ ! [ enable ( implicit_some, ) ]\n#![type = \"doc // /* */\"]\n(value: \"hello\")";
        let compact = minify_ron(input);
        assert_eq!(
            ron::from_str::<Field>(input).unwrap(),
            ron::from_str::<Field>(&compact).unwrap()
        );
        assert!(compact.contains("doc // /* */"));
        assert!(compact.len() < input.len());
    }

    #[test]
    fn preserves_required_token_separators() {
        for (input, expected) in [
            ("foo \t bar", "foo bar"),
            ("1 /*comment*/ 2", "1 2"),
            ("1 e3", "1 e3"),
            ("1 . 2", "1 . 2"),
            ("1 u32", "1 u32"),
            ("- inf", "- inf"),
            ("r #foo", "r #foo"),
            ("r#foo . bar", "r#foo . bar"),
            ("b \"text\"", "b \"text\""),
            ("br # \"text\" #", "br # \"text\"#"),
            ("b 'a'", "b 'a'"),
            ("/ /", "/ /"),
            ("/ *", "/ *"),
            ("* /", "* /"),
            (": :", ": :"),
            (". . =", ". . ="),
            ("α \u{301}β", "α \u{301}β"),
        ] {
            assert_eq!(minify_ron(input), expected, "input: {input}");
            assert_eq!(minify_ron(expected), expected, "idempotence: {input}");
        }
    }

    #[test]
    fn uses_rons_exact_whitespace_set() {
        for whitespace in [
            ' ', '\t', '\n', '\r', '\x0b', '\x0c', '\u{85}', '\u{200e}', '\u{200f}', '\u{2028}',
            '\u{2029}',
        ] {
            let input = format!("{whitespace}[{whitespace}1{whitespace}]{whitespace}");
            assert_eq!(minify_ron(&input), "[1]");
            assert_eq!(ron::from_str::<Vec<u32>>(&input).unwrap(), vec![1]);
        }
        for non_whitespace in ['\u{a0}', '\u{1680}', '\u{2003}', '\u{feff}'] {
            let input = format!("[{non_whitespace}1]");
            assert_eq!(minify_ron(&input), input);
        }
    }

    #[test]
    fn compaction_is_idempotent() {
        for input in [RULES, MAP, "/*gone*/", "( a : 1, b : r##\" // /* \"## )"] {
            let compact = minify_ron(input);
            assert_eq!(minify_ron(&compact), compact);
        }
    }

    #[test]
    fn malformed_unterminated_input_is_infallible() {
        for input in [
            "",
            "/",
            "/*",
            "/* /* nested */",
            "//end",
            "\"",
            "\"a\\",
            "'",
            "'\\",
            "'\\u{1",
            "'\\x1",
            "'😀",
            "r",
            "r#",
            "r###\" // /* ",
            "br##\"é",
            "b\"\\",
            "b'\\u",
            "[ r###\"no matching terminator\"##",
            "[ \"unterminated // /* ",
        ] {
            let compact = minify_ron(input);
            assert_eq!(minify_ron(&compact), compact, "input: {input}");
        }
        assert_eq!(
            minify_ron(" ( \"unterminated // /* "),
            "(\"unterminated // /* "
        );
        assert_eq!(minify_ron("(1, /* unterminated"), "(1,");
    }
}
