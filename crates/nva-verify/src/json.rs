//! Minimal JSON reader for the `loadplan` fixture (#42).
//!
//! The workspace builds offline with no external crates, so this module is a
//! hand-written parser instead of `serde_json`. It accepts
//! the JSON subset the fixtures use: objects, arrays, strings, integers,
//! booleans and null. Floats are rejected on purpose: no fixture field is a
//! float, and a silent truncation would hide a broken fixture.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(i64),
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Number(number) => Some(*number),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }
}

/// Parse one JSON value. Trailing content other than whitespace is an error.
pub fn parse(input: &str) -> Result<Value, String> {
    let mut parser = Parser {
        bytes: input.as_bytes(),
        pos: 0,
        depth: 0,
    };
    parser.skip_whitespace();
    let value = parser.value()?;
    parser.skip_whitespace();
    if parser.pos != parser.bytes.len() {
        return Err(parser.error("trailing characters"));
    }
    Ok(value)
}

/// Encode a string as a JSON string literal, quotes included.
pub fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

const MAX_DEPTH: usize = 64;

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
    depth: usize,
}

impl Parser<'_> {
    fn error(&self, message: &str) -> String {
        format!("{message} at byte {}", self.pos)
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, want: u8) -> Result<(), String> {
        if self.peek() == Some(want) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.error(&format!("expected '{}'", want as char)))
        }
    }

    fn value(&mut self) -> Result<Value, String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.error("nesting is too deep"));
        }
        let value = match self.peek() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => self.string().map(Value::String),
            Some(b't') => self.literal("true", Value::Bool(true)),
            Some(b'f') => self.literal("false", Value::Bool(false)),
            Some(b'n') => self.literal("null", Value::Null),
            Some(b'-') | Some(b'0'..=b'9') => self.number().map(Value::Number),
            Some(_) => Err(self.error("unexpected character")),
            None => Err(self.error("unexpected end of input")),
        };
        self.depth -= 1;
        value
    }

    fn object(&mut self) -> Result<Value, String> {
        self.expect(b'{')?;
        let mut entries: Vec<(String, Value)> = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Value::Object(entries));
        }
        loop {
            self.skip_whitespace();
            let key = self.string()?;
            self.skip_whitespace();
            self.expect(b':')?;
            self.skip_whitespace();
            let value = self.value()?;
            entries.push((key, value));
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Value::Object(entries));
                }
                _ => return Err(self.error("expected ',' or '}'")),
            }
        }
    }

    fn array(&mut self) -> Result<Value, String> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Value::Array(items));
        }
        loop {
            self.skip_whitespace();
            items.push(self.value()?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Value::Array(items));
                }
                _ => return Err(self.error("expected ',' or ']'")),
            }
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let byte = match self.peek() {
                Some(byte) => byte,
                None => return Err(self.error("unterminated string")),
            };
            self.pos += 1;
            match byte {
                b'"' => break,
                b'\\' => {
                    let escape = match self.peek() {
                        Some(escape) => escape,
                        None => return Err(self.error("unterminated escape")),
                    };
                    self.pos += 1;
                    match escape {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'b' => out.push(0x08),
                        b'f' => out.push(0x0c),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            let ch = self.unicode_escape()?;
                            let mut buffer = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buffer).as_bytes());
                        }
                        _ => return Err(self.error("invalid escape")),
                    }
                }
                0x00..=0x1f => return Err(self.error("control character in string")),
                other => out.push(other),
            }
        }
        String::from_utf8(out).map_err(|_| self.error("invalid UTF-8 in string"))
    }

    fn unicode_escape(&mut self) -> Result<char, String> {
        let mut code = 0u32;
        for _ in 0..4 {
            let byte = match self.peek() {
                Some(byte) => byte,
                None => return Err(self.error("short \\u escape")),
            };
            let digit = match byte {
                b'0'..=b'9' => (byte - b'0') as u32,
                b'a'..=b'f' => (byte - b'a' + 10) as u32,
                b'A'..=b'F' => (byte - b'A' + 10) as u32,
                _ => return Err(self.error("invalid \\u escape")),
            };
            self.pos += 1;
            code = code * 16 + digit;
        }
        char::from_u32(code).ok_or_else(|| self.error("unsupported \\u escape"))
    }

    fn number(&mut self) -> Result<i64, String> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => {
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.pos += 1;
                }
            }
            _ => return Err(self.error("invalid number")),
        }
        if matches!(self.peek(), Some(b'.' | b'e' | b'E')) {
            return Err(self.error("only integer numbers are supported"));
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| self.error("invalid number"))?;
        text.parse::<i64>()
            .map_err(|_| self.error("integer is out of range"))
    }

    fn literal(&mut self, text: &str, value: Value) -> Result<Value, String> {
        let end = self.pos + text.len();
        if self.bytes.get(self.pos..end) == Some(text.as_bytes()) {
            self.pos = end;
            Ok(value)
        } else {
            Err(self.error("invalid literal"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_an_object_with_nested_values() {
        let value = parse(r#"{"a":1,"b":["x",null,true],"c":{"d":-2}}"#).unwrap();
        assert_eq!(value.get("a").and_then(Value::as_i64), Some(1));
        let array = value.get("b").and_then(Value::as_array).unwrap();
        assert_eq!(array.len(), 3);
        assert_eq!(array[0].as_str(), Some("x"));
        assert_eq!(array[1], Value::Null);
        assert_eq!(array[2], Value::Bool(true));
        assert_eq!(
            value
                .get("c")
                .and_then(|c| c.get("d"))
                .and_then(Value::as_i64),
            Some(-2)
        );
    }

    #[test]
    fn parses_string_escapes() {
        let value = parse(r#"{"s":"a\nb\u0041"}"#).unwrap();
        assert_eq!(value.get("s").and_then(Value::as_str), Some("a\nbA"));
    }

    #[test]
    fn quotes_strings_for_output() {
        assert_eq!(quote("a\"b\\c"), "\"a\\\"b\\\\c\"");
    }

    #[test]
    fn rejects_trailing_garbage() {
        assert!(parse("{} x").is_err());
    }

    #[test]
    fn rejects_float_numbers() {
        assert!(parse(r#"{"n":1.5}"#).is_err());
    }

    #[test]
    fn rejects_unterminated_string() {
        assert!(parse("\"abc").is_err());
    }

    #[test]
    fn rejects_empty_input() {
        assert!(parse("").is_err());
    }

    #[test]
    fn rejects_leading_zero() {
        assert!(parse("01").is_err());
    }
}
