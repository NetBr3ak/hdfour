//! Object Description Language metadata parsing.
#![deny(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use crate::{Error, Limits, Result};

/// An ODL document or block, with entries in source order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Odl {
    pub entries: Vec<Entry>,
}

/// An assignment or a named block. Assignment text retains its original spelling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Assignment {
        name: String,
        value: Value,
        raw: String,
    },
    Group {
        name: String,
        body: Odl,
    },
    Object {
        name: String,
        body: Odl,
    },
}

/// A scalar with quotes removed, or an ordered list of ODL values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Scalar(String),
    List(Vec<Value>),
}

/// Parse structural ODL with the file's combined group, object and list depth limit.
/// Error offsets are UTF-8 byte positions in the metadata text.
pub fn parse(text: &str, limits: &Limits) -> Result<Odl> {
    Parser {
        text: text.trim_end_matches('\0'),
        offset: 0,
        max_depth: limits.max_group_depth,
    }
    .body(None, 0)
}

struct Parser<'a> {
    text: &'a str,
    offset: usize,
    max_depth: u64,
}

impl Parser<'_> {
    fn remaining(&self) -> Result<&str> {
        self.text
            .get(self.offset..)
            .ok_or_else(|| self.malformed("invalid ODL byte position"))
    }

    fn malformed(&self, what: &'static str) -> Error {
        Error::Malformed {
            offset: self.offset as u64,
            what,
        }
    }

    fn advance(&mut self, count: usize) -> Result<()> {
        self.offset = self
            .offset
            .checked_add(count)
            .ok_or_else(|| self.malformed("ODL byte position overflow"))?;
        Ok(())
    }

    fn next(&mut self) -> Result<char> {
        let ch = self
            .remaining()?
            .chars()
            .next()
            .ok_or_else(|| self.malformed("unexpected end of ODL"))?;
        self.advance(ch.len_utf8())?;
        Ok(ch)
    }

    fn whitespace(&mut self) -> Result<()> {
        loop {
            let rest = self.remaining()?;
            if let Some(comment) = rest.strip_prefix("/*") {
                let end = comment
                    .find("*/")
                    .and_then(|end| end.checked_add(4))
                    .ok_or_else(|| self.malformed("unterminated ODL comment"))?;
                self.advance(end)?;
                continue;
            }
            if rest.starts_with(char::is_whitespace) {
                self.next()?;
                continue;
            }
            return Ok(());
        }
    }

    fn symbol(&mut self, expected: char) -> Result<()> {
        self.whitespace()?;
        if !self.remaining()?.starts_with(expected) {
            return Err(self.malformed("unexpected ODL delimiter"));
        }
        self.next()?;
        Ok(())
    }

    fn scalar(&mut self) -> Result<String> {
        self.whitespace()?;
        if self.remaining()?.starts_with('"') {
            return self.quoted();
        }
        let rest = self.remaining()?;
        let count = rest
            .find(|ch: char| {
                ch.is_whitespace() || matches!(ch, '=' | '(' | ')' | ',' | '"' | '/' | '\0')
            })
            .unwrap_or(rest.len());
        if count == 0 {
            return Err(self.malformed("expected ODL scalar"));
        }
        let text = rest
            .get(..count)
            .ok_or_else(|| self.malformed("invalid ODL scalar boundary"))?
            .to_owned();
        self.advance(count)?;
        Ok(text)
    }

    fn quoted(&mut self) -> Result<String> {
        self.next()?;
        let mut text = String::new();
        loop {
            match self.next()? {
                '"' => return Ok(text),
                '\0' => return Err(self.malformed("null inside ODL string")),
                ch => text.push(ch),
            }
        }
    }

    fn nested_depth(&self, depth: u64) -> Result<u64> {
        let next = depth
            .checked_add(1)
            .ok_or_else(|| self.malformed("ODL depth overflow"))?;
        if next > self.max_depth {
            return Err(Error::LimitExceeded {
                limit: "group depth",
                value: next,
                max: self.max_depth,
            });
        }
        Ok(next)
    }

    fn value(&mut self, depth: u64) -> Result<Value> {
        self.whitespace()?;
        if !self.remaining()?.starts_with('(') {
            return Ok(Value::Scalar(self.scalar()?));
        }
        let depth = self.nested_depth(depth)?;
        self.next()?;
        let mut items = Vec::new();
        self.whitespace()?;
        if self.remaining()?.starts_with(')') {
            self.next()?;
            return Ok(Value::List(items));
        }
        loop {
            items.push(self.value(depth)?);
            self.whitespace()?;
            if self.remaining()?.starts_with(')') {
                self.next()?;
                return Ok(Value::List(items));
            }
            self.symbol(',')?;
        }
    }

    fn body(&mut self, closing: Option<(&str, &str)>, depth: u64) -> Result<Odl> {
        let mut entries = Vec::new();
        loop {
            self.whitespace()?;
            if self.remaining()?.is_empty() && closing.is_none() {
                return Ok(Odl { entries });
            }
            let key = self.scalar()?;
            if key == "END" && closing.is_none() {
                self.whitespace()?;
                if !self.remaining()?.is_empty() {
                    return Err(self.malformed("text after ODL END"));
                }
                return Ok(Odl { entries });
            }
            self.symbol('=')?;
            if matches!(key.as_str(), "END_GROUP" | "END_OBJECT") {
                let name = self.scalar()?;
                if closing != Some((key.as_str(), name.as_str())) {
                    return Err(self.malformed("mismatched ODL block terminator"));
                }
                return Ok(Odl { entries });
            }
            // A repeated name is kept: readers take the first one, as the C library's text search does.
            entries.push(self.entry(key, depth)?);
        }
    }

    fn entry(&mut self, name: String, depth: u64) -> Result<Entry> {
        if matches!(name.as_str(), "GROUP" | "OBJECT") {
            let depth = self.nested_depth(depth)?;
            let block_name = self.scalar()?;
            let closing = if name == "GROUP" {
                "END_GROUP"
            } else {
                "END_OBJECT"
            };
            let body = self.body(Some((closing, &block_name)), depth)?;
            return Ok(if name == "GROUP" {
                Entry::Group {
                    name: block_name,
                    body,
                }
            } else {
                Entry::Object {
                    name: block_name,
                    body,
                }
            });
        }
        self.whitespace()?;
        let start = self.offset;
        let value = self.value(depth)?;
        let raw = self
            .text
            .get(start..self.offset)
            .ok_or_else(|| self.malformed("invalid ODL value boundary"))?
            .to_owned();
        Ok(Entry::Assignment { name, value, raw })
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn sq6_odl_parser_obeys_custom_depth_limits() {
        let limits = Limits {
            max_group_depth: 1,
            ..Limits::default()
        };
        for text in [
            "GROUP=A OBJECT=B END_OBJECT=B END_GROUP=A",
            "A=((0))",
            "GROUP=A A=(0) END_GROUP=A",
        ] {
            assert!(matches!(
                super::parse(text, &limits),
                Err(Error::LimitExceeded {
                    limit: "group depth",
                    value: 2,
                    max: 1
                })
            ));
        }
        assert!(super::parse("GROUP=A A=0 END_GROUP=A", &limits).is_ok());
        let deep = format!("A={}0{}", "(".repeat(65), ")".repeat(65));
        let limits = Limits {
            max_group_depth: 65,
            ..Limits::default()
        };
        assert!(super::parse(&deep, &limits).is_ok());
    }

    #[test]
    fn sq6_quoted_backslashes_are_literal() -> Result<()> {
        for text in [r"one\path", r"two\\paths", r"trailing\"] {
            let document = parse(&format!("Name=\"{text}\" END"), &Limits::default())?;
            assert!(matches!(
                &document.entries[0],
                Entry::Assignment { value: Value::Scalar(value), .. } if value == text
            ));
        }
        Ok(())
    }

    #[test]
    fn sq6_backslash_does_not_escape_a_quote() -> Result<()> {
        let document = parse(r#"A="path\" B=2 END"#, &Limits::default())?;
        assert!(matches!(
            &document.entries[0],
            Entry::Assignment { value: Value::Scalar(value), .. } if value == r"path\"
        ));
        assert_eq!(document.entries.len(), 2);
        Ok(())
    }

    #[test]
    fn repeated_names_are_kept_in_source_order() -> Result<()> {
        let document = parse("A=1\nA=2\nEND", &Limits::default())?;
        assert_eq!(document.entries.len(), 2);
        assert!(matches!(
            document.entries.first(),
            Some(Entry::Assignment { raw, .. }) if raw == "1"
        ));
        Ok(())
    }

    #[test]
    fn parses_groups_objects_lists_and_raw_values() {
        let odl = parse("/* header */ GROUP=Grid OBJECT=Field Name=\"Band, one\" Values=(1, (\"x\",-2.5E+1)) END_OBJECT=Field END_GROUP=Grid END", &Limits::default()).expect("ODL");
        let Entry::Group { name, body } = &odl.entries[0] else {
            panic!("group")
        };
        assert_eq!(name, "Grid");
        let Entry::Object { name, body } = &body.entries[0] else {
            panic!("object")
        };
        assert_eq!(name, "Field");
        assert_eq!(
            body.entries[0],
            Entry::Assignment {
                name: "Name".into(),
                value: Value::Scalar("Band, one".into()),
                raw: "\"Band, one\"".into(),
            }
        );
        assert_eq!(
            body.entries[1],
            Entry::Assignment {
                name: "Values".into(),
                value: Value::List(vec![
                    Value::Scalar("1".into()),
                    Value::List(vec![
                        Value::Scalar("x".into()),
                        Value::Scalar("-2.5E+1".into())
                    ])
                ]),
                raw: "(1, (\"x\",-2.5E+1))".into(),
            }
        );
    }

    #[test]
    fn parses_unicode_literal_backslashes_empty_lists_and_padding() {
        let odl = parse(
            "Name=\"測定 band \\\\ path\" Empty=() END\n\0\0",
            &Limits::default(),
        )
        .expect("ODL");
        assert!(
            matches!(&odl.entries[0], Entry::Assignment { value: Value::Scalar(text), .. } if text == "測定 band \\\\ path")
        );
        assert!(
            matches!(&odl.entries[1], Entry::Assignment { value: Value::List(items), .. } if items.is_empty())
        );
    }

    #[test]
    fn rejects_malformed_odl() {
        for text in [
            "GROUP=A END_GROUP=B",
            "OBJECT=A END_GROUP=A",
            "GROUP=A END",
            "END_OBJECT=A",
            "GROUP=A",
            "A=",
            "A=(1 2)",
            "A=(1,)",
            "A=(1",
            "A=\"unterminated",
            "/* unterminated",
            "/*/",
            "END A=1",
            "GROUP=(a) END_GROUP=a",
            "A=1\0B=2",
        ] {
            assert!(
                matches!(
                    parse(text, &Limits::default()),
                    Err(crate::Error::Malformed { .. })
                ),
                "{text:?}"
            );
        }
    }

    #[test]
    fn reports_byte_offset() {
        assert!(matches!(
            parse("A=(1,)", &Limits::default()),
            Err(crate::Error::Malformed { offset: 5, .. })
        ));
    }

    #[test]
    fn enforces_group_depth_limit() {
        assert!(matches!(
            parse(
                include_str!("../../../fixtures/odl/too-deep.odl"),
                &Limits::default()
            ),
            Err(crate::Error::LimitExceeded {
                limit: "group depth",
                value: 65,
                max: 64
            })
        ));
        let boundary = format!(
            "{}{}END",
            "GROUP=A\n".repeat(64),
            "END_GROUP=A\n".repeat(64)
        );
        assert!(parse(&boundary, &Limits::default()).is_ok());
    }

    #[test]
    fn enforces_list_and_mixed_depth_limit() {
        let boundary = format!("A={}0{} END", "(".repeat(64), ")".repeat(64));
        assert!(parse(&boundary, &Limits::default()).is_ok());
        let deep = format!("A={}0{} END", "(".repeat(65), ")".repeat(65));
        assert!(matches!(
            parse(&deep, &Limits::default()),
            Err(crate::Error::LimitExceeded { .. })
        ));
        let mixed = format!(
            "{}A=(0){}",
            "OBJECT=A\n".repeat(64),
            "END_OBJECT=A\n".repeat(64)
        );
        assert!(matches!(
            parse(&mixed, &Limits::default()),
            Err(crate::Error::LimitExceeded { .. })
        ));
    }

    #[test]
    fn parses_all_struct_metadata_fixtures() {
        for text in [
            include_str!("../../../fixtures/odl/grid.odl"),
            include_str!("../../../fixtures/odl/swath.odl"),
            include_str!("../../../fixtures/odl/point.odl"),
        ] {
            assert!(parse(text, &Limits::default()).is_ok());
        }
    }

    proptest! {
        #[test]
        fn arbitrary_text_never_panics(text in any::<String>()) {
            let _ = parse(&text, &Limits::default());
        }
    }
}
