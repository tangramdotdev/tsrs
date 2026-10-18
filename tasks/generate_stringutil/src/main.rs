use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

use regex::Regex;

const DESTINATION: &str = "crates/tsrs_core/src/stringutil/js_case_generated.rs";
const SOURCE: &str = "ts-ref/tsc/internal/stringutil/js_case_generated.go";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
    let args: Vec<_> = std::env::args().skip(1).collect();
    let (write, compact, destination) = match args.as_slice() {
        [] => (false, false, root.join(DESTINATION)),
        [arg] if arg == "--check" => (false, false, root.join(DESTINATION)),
        [arg] if arg == "--write" => (true, false, root.join(DESTINATION)),
        [arg, path] if arg == "--compact" => (true, true, path.into()),
        [arg] if arg == "--help" || arg == "-h" => {
            println!("cargo run -p tsrs_generate_stringutil -- [--check|--write|--compact PATH]");
            return Ok(());
        }
        _ => return Err("expected --check (default), --write, or --compact PATH".into()),
    };
    let source = std::fs::read_to_string(root.join(SOURCE))
        .map_err(|error| format!("{SOURCE}: {error}; check out the pinned TypeScript reference"))?;
    let generated = generate(&source, compact)?;
    let mut rustfmt = Command::new("rustfmt")
        .current_dir(root)
        .args(["--edition", "2024", "--emit", "stdout"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|error| format!("rustfmt: {error}"))?;
    rustfmt.stdin.take().unwrap().write_all(generated.as_bytes()).map_err(|e| e.to_string())?;
    let formatted = rustfmt.wait_with_output().map_err(|e| e.to_string())?;
    if !formatted.status.success() {
        return Err(format!("rustfmt failed: {}", formatted.status));
    }
    if write {
        std::fs::write(&destination, formatted.stdout).map_err(|e| e.to_string())?;
        println!("wrote {}", destination.display());
    } else if std::fs::read(destination).map_err(|e| e.to_string())? == formatted.stdout {
        println!("{DESTINATION} is up to date");
    } else {
        return Err(
            "casing tables are stale; run cargo run -p tsrs_generate_stringutil -- --write".into(),
        );
    }
    Ok(())
}

fn block<'a>(source: &'a str, prefix: &str, end: &str) -> Result<&'a str, String> {
    source
        .split_once(prefix)
        .and_then(|(_, rest)| rest.split_once(end))
        .map(|(body, _)| body)
        .ok_or_else(|| format!("missing or incomplete {prefix}"))
}

fn decode_string(source: &str) -> Result<String, String> {
    let mut chars = source.chars();
    let mut result = String::new();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            result.push(ch);
            continue;
        }
        let digits = match chars.next() {
            Some('u') => 4,
            Some('U') => 8,
            other => return Err(format!("unsupported Go string escape: {other:?}")),
        };
        let mut value = 0;
        for _ in 0..digits {
            let digit =
                chars.next().and_then(|ch| ch.to_digit(16)).ok_or("invalid Unicode escape")?;
            value = value * 16 + digit;
        }
        result.push(char::from_u32(value).ok_or("invalid Unicode scalar")?);
    }
    Ok(result)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Slice {
    offset: u16,
    length: u8,
}

#[derive(Default)]
struct Pool {
    text: String,
    strings: BTreeMap<String, Slice>,
}

impl Pool {
    fn insert(&mut self, text: &str) -> Result<Slice, String> {
        if let Some(slice) = self.strings.get(text) {
            return Ok(*slice);
        }
        let offset =
            u16::try_from(self.text.len()).map_err(|_| "string pool exceeds u16 offsets")?;
        let length = u8::try_from(text.len()).map_err(|_| "mapping exceeds u8 byte length")?;
        if self.text.len() + text.len() > usize::from(u16::MAX) {
            return Err("string pool exceeds u16 offsets".into());
        }
        self.text.push_str(text);
        let slice = Slice { offset, length };
        self.strings.insert(text.to_owned(), slice);
        Ok(slice)
    }

    fn mapping(&mut self, source: &str) -> Result<Slice, String> {
        let text = decode_string(source)?;
        let slice = self.insert(&text)?;
        let start = usize::from(slice.offset);
        let end = start + usize::from(slice.length);
        if self.text.get(start..end) != Some(text.as_str()) {
            return Err("mapping did not round-trip through the UTF-8 pool".into());
        }
        Ok(slice)
    }
}

fn rust_string(source: &str) -> Result<String, String> {
    let mut result = String::from("\"");
    for ch in decode_string(source)?.chars() {
        write!(result, "\\u{{{:X}}}", ch as u32).unwrap();
    }
    result.push('"');
    Ok(result)
}

fn generate(source: &str, compact: bool) -> Result<String, String> {
    let body = block(source, "var specialCasingMappings = map[rune]specialCasingMapping{", "\n}")?;
    let entry = Regex::new(
        r#"^\s*(0x[0-9A-F]+):\s+\{lower: "([^"]*)", upper: "([^"]*)"(?:, conditionalLower: "([^"]*)")?, condition: specialCasingCondition(None|FinalSigma)\},$"#,
    )
    .unwrap();
    let mut pool = Pool::default();
    let mut entries = String::new();
    let mut previous = None;
    for line in body.lines().filter(|line| !line.trim().is_empty()) {
        let captures = entry.captures(line).ok_or_else(|| format!("invalid mapping: {line}"))?;
        let key = u32::from_str_radix(&captures[1][2..], 16).map_err(|e| e.to_string())?;
        if char::from_u32(key).is_none() || previous.is_some_and(|prev| key <= prev) {
            return Err(format!("invalid or unsorted mapping key: {}", &captures[1]));
        }
        previous = Some(key);
        let conditional = captures.get(4).map_or("", |m| m.as_str());
        if compact {
            let lower = pool.mapping(&captures[2])?;
            let upper = pool.mapping(&captures[3])?;
            let conditional = pool.mapping(conditional)?;
            writeln!(entries, "(0x{key:X}, SpecialCasingMapping {{ lower_offset: {}, upper_offset: {}, conditional_lower_offset: {}, lower_length: {}, upper_length: {}, conditional_lower_length: {}, condition: SpecialCasingCondition::{} }}),",
            lower.offset, upper.offset, conditional.offset, lower.length, upper.length, conditional.length, &captures[5]).unwrap();
        } else {
            writeln!(entries, "(0x{key:X}, SpecialCasingMapping {{ lower: {}, upper: {}, conditional_lower: {}, condition: SpecialCasingCondition::{} }}),",
                rust_string(&captures[2])?, rust_string(&captures[3])?, rust_string(conditional)?, &captures[5]).unwrap();
        }
    }
    if previous.is_none() {
        return Err("no casing mappings found".into());
    }
    let mut output = String::from(if compact {
        include_str!("compact.rs.txt")
    } else {
        include_str!("table.rs.txt")
    });
    if compact {
        output.push_str("\nstatic CASING_STRINGS: &str = concat!(\n");
        // One scalar per escaped fragment keeps the generated file ASCII and avoids splitting UTF-8.
        for chunk in pool.text.chars().collect::<Vec<_>>().chunks(32) {
            output.push('"');
            for ch in chunk {
                write!(output, "\\u{{{:X}}}", *ch as u32).unwrap();
            }
            output.push_str("\",\n");
        }
        output.push_str(");\n\n");
    }
    output.push_str("// Sorted by code point; looked up with binary search (Go uses a map).\npub(crate) static SPECIAL_CASING_MAPPINGS: &[(u32, SpecialCasingMapping)] = &[\n");
    output.push_str(&entries);
    output.push_str("];\n\n");
    for (go, rust) in [
        ("unicodeCasedRanges", "UNICODE_CASED_RANGES"),
        ("unicodeCaseIgnorableRanges", "UNICODE_CASE_IGNORABLE_RANGES"),
    ] {
        generate_ranges(source, go, rust, &mut output)?;
    }
    Ok(output)
}

fn generate_ranges(source: &str, go: &str, rust: &str, output: &mut String) -> Result<(), String> {
    let body = block(source, &format!("var {go} = &unicode.RangeTable{{"), "\n}")?;
    let entry = Regex::new(r"^\s*\{(0x[0-9A-Fa-f]+), (0x[0-9A-Fa-f]+), (\d+)\},$").unwrap();
    for width in [16, 32] {
        let ranges = block(body, &format!("R{width}: []unicode.Range{width}{{"), "\n\t}")?;
        writeln!(output, "static {rust}_R{width}: &[Range{width}] = &[").unwrap();
        for line in ranges.lines().filter(|line| !line.trim().is_empty()) {
            let captures = entry.captures(line).ok_or_else(|| format!("invalid range: {line}"))?;
            writeln!(
                output,
                "Range{width} {{ lo: {}, hi: {}, stride: {} }},",
                &captures[1], &captures[2], &captures[3]
            )
            .unwrap();
        }
        output.push_str("];\n");
    }
    let offset = block(body, "LatinOffset: ", ",")?.parse::<usize>().map_err(|e| e.to_string())?;
    writeln!(output, "pub(crate) static {rust}: RangeTable = RangeTable {{ r16: {rust}_R16, r32: {rust}_R32, latin_offset: {offset} }};\n").unwrap();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAPPING: &str =
        r#"0x41: {lower: "\u0061", upper: "\u0041", condition: specialCasingConditionNone},"#;

    fn source(mappings: &str) -> String {
        format!(
            "var specialCasingMappings = map[rune]specialCasingMapping{{\n{mappings}\n}}\n\
             var unicodeCasedRanges = &unicode.RangeTable{{\n\
             R16: []unicode.Range16{{\n\t}},\nR32: []unicode.Range32{{\n\t}},\nLatinOffset: 0,\n}}\n\
             var unicodeCaseIgnorableRanges = &unicode.RangeTable{{\n\
             R16: []unicode.Range16{{\n\t}},\nR32: []unicode.Range32{{\n\t}},\nLatinOffset: 0,\n}}\n"
        )
    }

    #[test]
    fn rejects_source_changes_instead_of_silently_dropping_mappings() {
        assert!(generate(&source(MAPPING), false).is_ok());
        assert!(generate(&source(MAPPING), true).is_ok());
        for mappings in [
            String::new(),
            format!("{MAPPING}\n{MAPPING}"),
            format!("{}\n{MAPPING}", MAPPING.replace("0x41", "0x42")),
            MAPPING.replace("0x41", "0xD800"),
            MAPPING.replace("ConditionNone", "ConditionUnknown"),
            MAPPING.replace("lower:", "unexpected:"),
        ] {
            assert!(generate(&source(&mappings), false).is_err(), "{mappings}");
            assert!(generate(&source(&mappings), true).is_err(), "{mappings}");
        }
        assert!(generate(&source(MAPPING).replace("unicodeCasedRanges", "unknown"), true).is_err());
        assert!(generate("truncated source", true).is_err());
    }

    #[test]
    fn unicode_strings_and_pool_bounds() {
        let mut pool = Pool::default();
        let slice = pool.mapping(r"\u0069\u0307\U00010428").unwrap();
        assert_eq!(pool.text, "i\u{307}\u{10428}");
        assert_eq!(slice, Slice { offset: 0, length: 7 });
        assert_eq!(pool.mapping(r"\u0069\u0307\U00010428").unwrap(), slice);
        assert_eq!(pool.mapping("").unwrap(), Slice { offset: 7, length: 0 });
        assert!(pool.insert(&"x".repeat(256)).is_err());
        pool.text = "x".repeat(usize::from(u16::MAX));
        assert!(pool.insert("y").is_err());
        for invalid in [r"\uD800", r"\U00110000", r"\u123", r"\uZZZZ", r"\n"] {
            assert!(decode_string(invalid).is_err(), "{invalid}");
        }
    }
}
