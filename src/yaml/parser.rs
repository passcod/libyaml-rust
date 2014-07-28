use libc;

use ffi;
use event::{YamlEvent, YamlNoEvent};
use document::{YamlDocument};
use codecs;

use std::mem;
use std::io;
use std::c_vec::CVec;

#[deriving(Show, PartialEq)]
pub struct YamlMark {
    pub index: uint,
    pub line: uint,
    pub column: uint
}

impl YamlMark {
    pub fn conv(mark: &ffi::yaml_mark_t) -> YamlMark {
        YamlMark {
            index: mark.index as uint,
            line: mark.line as uint,
            column: mark.column as uint
        }
    }
}

#[deriving(Show, PartialEq)]
pub struct YamlError {
    kind: ffi::YamlErrorType,
    problem: Option<String>,
    byte_offset: uint,
    problem_mark: YamlMark,
    context: Option<String>,
    context_mark: YamlMark,
}

pub struct YamlEventStream<P> {
    parser: Box<P>,
}

impl<P:YamlParser> Iterator<Result<YamlEvent, YamlError>> for YamlEventStream<P> {
    fn next(&mut self) -> Option<Result<YamlEvent, YamlError>> {
        unsafe {
            match self.parser.parse_event() {
                Some(YamlNoEvent) => None,
                Some(evt) => Some(Ok(evt)),
                None => Some(Err(self.parser.base_parser_ref().get_error()))
            }
        }
    }
}

pub struct YamlDocumentStream<P> {
    parser: Box<P>,
}

impl<P:YamlParser> Iterator<Result<Box<YamlDocument>, YamlError>> for YamlDocumentStream<P> {
    fn next(&mut self) -> Option<Result<Box<YamlDocument>, YamlError>> {
        unsafe {
            match YamlDocument::parser_load(&mut self.parser.base_parser_ref().parser_mem) {
                Some(doc) => if doc.is_empty() {
                    None
                } else {
                    Some(Ok(doc))
                },
                None => Some(Err(self.parser.base_parser_ref().get_error()))
            }
        }
    }
}

pub struct InternalEvent {
    event_mem: ffi::yaml_event_t
}

impl Drop for InternalEvent {
    fn drop(&mut self) {
        unsafe {
            self.event_mem.delete()
        }
    }
}

pub trait YamlParser {
    unsafe fn base_parser_ref<'r>(&'r mut self) -> &'r mut YamlBaseParser;

    unsafe fn parse_event(&mut self) -> Option<YamlEvent> {
        let mut event = InternalEvent {
            event_mem: mem::uninitialized()
        };

        if !self.base_parser_ref().parse(&mut event.event_mem) {
            None
        } else {
            Some(YamlEvent::load(&event.event_mem))
        }
    }

    fn parse(self: Box<Self>) -> YamlEventStream<Self> {
        YamlEventStream {
            parser: self,
        }
    }

    fn load(self: Box<Self>) -> YamlDocumentStream<Self> {
        YamlDocumentStream {
            parser: self,
        }
    }
}

extern fn handle_reader_cb(data: *mut YamlIoParser, buffer: *mut u8, size: libc::size_t, size_read: *mut libc::size_t) -> libc::c_int {
    unsafe {
        let mut buf = CVec::new(buffer, size as uint);
        let parser = &mut *data;
        match parser.reader.read(buf.as_mut_slice()) {
            Ok(size) => {
                *size_read = size as libc::size_t;
                return 1;
            },
            Err(err) => {
                match err.kind {
                    io::EndOfFile => {
                        *size_read = 0;
                        return 1;
                    },
                    _ => {
                        return 0;
                    }
                }
            }
        }
    }
}

pub struct YamlBaseParser {
    parser_mem: ffi::yaml_parser_t,
}

impl YamlBaseParser {
    unsafe fn new() -> YamlBaseParser {
        YamlBaseParser {
            parser_mem: mem::uninitialized()
        }
    }

    unsafe fn initialize(&mut self) -> bool {
        ffi::yaml_parser_initialize(&mut self.parser_mem) != 0
    }

    unsafe fn set_input_string(&mut self, input: *const u8, size: uint) {
        ffi::yaml_parser_set_input_string(&mut self.parser_mem, input, size as libc::size_t);
    }

    unsafe fn parse(&mut self, event: &mut ffi::yaml_event_t) -> bool {
        ffi::yaml_parser_parse(&mut self.parser_mem, event) != 0
    }

    unsafe fn get_error(&self) -> YamlError {
        YamlError {
            kind: self.parser_mem.error,
            problem: codecs::decode_c_str(self.parser_mem.problem as *const ffi::yaml_char_t),
            byte_offset: self.parser_mem.problem_offset as uint,
            problem_mark: YamlMark::conv(&self.parser_mem.problem_mark),
            context: codecs::decode_c_str(self.parser_mem.context as *const ffi::yaml_char_t),
            context_mark: YamlMark::conv(&self.parser_mem.context_mark),
        }
    }
}

impl Drop for YamlBaseParser {
    fn drop(&mut self) {
        unsafe {
            ffi::yaml_parser_delete(&mut self.parser_mem);
        }
    }
}

pub struct YamlByteParser<'r> {
    base_parser: YamlBaseParser
}

impl<'r> YamlParser for YamlByteParser<'r> {
    unsafe fn base_parser_ref<'r>(&'r mut self) -> &'r mut YamlBaseParser {
        &mut self.base_parser
    }
}

impl<'r> YamlByteParser<'r> {
    pub fn init(bytes: &'r [u8], encoding: ffi::YamlEncoding) -> Box<YamlByteParser<'r>> {
        unsafe {
            let mut parser = box YamlByteParser {
                base_parser: YamlBaseParser::new()
            };

            if !parser.base_parser.initialize() {
                fail!("failed to initialize yaml_parser_t");
            }

            ffi::yaml_parser_set_encoding(&mut parser.base_parser.parser_mem, encoding);
            parser.base_parser.set_input_string(bytes.as_ptr(), bytes.len());

            parser
        }
    }
}

pub struct YamlIoParser<'r> {
    base_parser: YamlBaseParser,
    reader: &'r mut Reader,
}

impl<'r> YamlParser for YamlIoParser<'r> {
    unsafe fn base_parser_ref<'r>(&'r mut self) -> &'r mut YamlBaseParser {
        &mut self.base_parser
    }
}

impl<'r> YamlIoParser<'r> {
    pub fn init<'r>(reader: &'r mut Reader, encoding: ffi::YamlEncoding) -> Box<YamlIoParser<'r>> {
        unsafe {
            let mut parser = box YamlIoParser {
                base_parser: YamlBaseParser::new(),
                reader: reader
            };

            if !parser.base_parser.initialize() {
                fail!("failed to initialize yaml_parser_t");
            }

            ffi::yaml_parser_set_encoding(&mut parser.base_parser.parser_mem, encoding);

            ffi::yaml_parser_set_input(&mut parser.base_parser.parser_mem, handle_reader_cb, mem::transmute(&mut *parser));

            parser
        }
    }
} 

#[cfg(test)]
mod test {
    use event::*;
    use document;
    use parser;
    use parser::{YamlParser, YamlError};
    use ffi;
    use std::io;
    use std::result;

    #[test]
    fn test_byte_parser() {
        let data = "[1, 2, 3]";
        let parser = parser::YamlByteParser::init(data.as_bytes(), ffi::YamlUtf8Encoding);
        let expected = Ok(vec![
            YamlStreamStartEvent(ffi::YamlUtf8Encoding),
            YamlDocumentStartEvent(None, vec![], true),
            YamlSequenceStartEvent(YamlSequenceParam{anchor: None, tag: None, implicit: true, style: ffi::YamlFlowSequenceStyle}),
            YamlScalarEvent(YamlScalarParam{anchor: None, tag: None, value: "1".to_string(), plain_implicit: true, quoted_implicit: false, style: ffi::YamlPlainScalarStyle}),
            YamlScalarEvent(YamlScalarParam{anchor: None, tag: None, value: "2".to_string(), plain_implicit: true, quoted_implicit: false, style: ffi::YamlPlainScalarStyle}),
            YamlScalarEvent(YamlScalarParam{anchor: None, tag: None, value: "3".to_string(), plain_implicit: true, quoted_implicit: false, style: ffi::YamlPlainScalarStyle}),
            YamlSequenceEndEvent,
            YamlDocumentEndEvent(true),
            YamlStreamEndEvent
        ]);

        let stream = parser.parse();

        assert_eq!(expected, result::collect(stream));
    }

    #[test]
    fn test_io_parser() {
        let data = "[1, 2, 3]";
        let mut reader = io::BufReader::new(data.as_bytes());
        let parser = parser::YamlIoParser::init(&mut reader, ffi::YamlUtf8Encoding);
        let expected = Ok(vec![
            YamlStreamStartEvent(ffi::YamlUtf8Encoding),
            YamlDocumentStartEvent(None, vec![], true),
            YamlSequenceStartEvent(YamlSequenceParam{anchor: None, tag: None, implicit: true, style: ffi::YamlFlowSequenceStyle}),
            YamlScalarEvent(YamlScalarParam{anchor: None, tag: None, value: "1".to_string(), plain_implicit: true, quoted_implicit: false, style: ffi::YamlPlainScalarStyle}),
            YamlScalarEvent(YamlScalarParam{anchor: None, tag: None, value: "2".to_string(), plain_implicit: true, quoted_implicit: false, style: ffi::YamlPlainScalarStyle}),
            YamlScalarEvent(YamlScalarParam{anchor: None, tag: None, value: "3".to_string(), plain_implicit: true, quoted_implicit: false, style: ffi::YamlPlainScalarStyle}),
            YamlSequenceEndEvent,
            YamlDocumentEndEvent(true),
            YamlStreamEndEvent
        ]);

        let stream = parser.parse();

        assert_eq!(expected, result::collect(stream));
    }

    #[test]
    fn test_byte_parser_mapping() {
        let data = "{\"a\": 1, \"b\":2}";
        let parser = parser::YamlByteParser::init(data.as_bytes(), ffi::YamlUtf8Encoding);
        let expected = Ok(vec![
            YamlStreamStartEvent(ffi::YamlUtf8Encoding),
            YamlDocumentStartEvent(None, vec![], true),
            YamlMappingStartEvent(YamlSequenceParam{anchor: None, tag: None, implicit: true, style: ffi::YamlFlowSequenceStyle}),
            YamlScalarEvent(YamlScalarParam{anchor: None, tag: None, value: "a".to_string(), plain_implicit: false, quoted_implicit: true, style: ffi::YamlDoubleQuotedScalarStyle}),
            YamlScalarEvent(YamlScalarParam{anchor: None, tag: None, value: "1".to_string(), plain_implicit: true, quoted_implicit: false, style: ffi::YamlPlainScalarStyle}),
            YamlScalarEvent(YamlScalarParam{anchor: None, tag: None, value: "b".to_string(), plain_implicit: false, quoted_implicit: true, style: ffi::YamlDoubleQuotedScalarStyle}),
            YamlScalarEvent(YamlScalarParam{anchor: None, tag: None, value: "2".to_string(), plain_implicit: true, quoted_implicit: false, style: ffi::YamlPlainScalarStyle}),
            YamlMappingEndEvent,
            YamlDocumentEndEvent(true),
            YamlStreamEndEvent
        ]);

        let stream = parser.parse();

        assert_eq!(expected, result::collect(stream));
    }

    #[test]
    fn test_parser_error() {
        let data = "\"ab";
        let parser = parser::YamlByteParser::init(data.as_bytes(), ffi::YamlUtf8Encoding);
        let mut stream = parser.parse();

        let stream_start = stream.next();
        assert_eq!(Some(Ok(YamlStreamStartEvent(ffi::YamlUtf8Encoding))), stream_start);

        let stream_err = stream.next();
        match stream_err {
            Some(Err(err)) => assert_eq!(ffi::YamlScannerError, err.kind),
            evt => fail!("unexpected result: {:?}", evt),
        }
    }

    #[test]
    fn test_document() {
        let data = "[1, 2, 3]";
        let parser = parser::YamlByteParser::init(data.as_bytes(), ffi::YamlUtf8Encoding);
        let docs_res:Result<Vec<Box<document::YamlDocument>>, YamlError> = result::collect(parser.load());

        match docs_res {
            Err(e) => fail!("unexpected result: {:?}", e),
            Ok(docs) => match docs.as_slice().head().and_then(|doc| doc.root()) {
                Some(document::YamlSequenceNode(seq)) => {
                    let values = seq.values().map(|node| {
                        match node {
                            document::YamlScalarNode(scalar) => scalar.get_value(),
                            _ => fail!("unexpected scalar: {:?}", node)
                        }
                    }).collect();
                    assert_eq!(vec!["1".to_string(), "2".to_string(), "3".to_string()], values)
                },
                _ => fail!("unexpected result: {:?}", docs)
            }
        }
    }

    #[test]
    fn test_mapping_document() {
        let data = "{\"a\": 1, \"b\": 2}";
        let parser = parser::YamlByteParser::init(data.as_bytes(), ffi::YamlUtf8Encoding);
        let docs_res:Result<Vec<Box<document::YamlDocument>>, YamlError> = result::collect(parser.load());

        match docs_res {
            Err(e) => fail!("unexpected result: {:?}", e),
            Ok(docs) => match docs.as_slice().head().and_then(|doc| doc.root()) {
                Some(document::YamlMappingNode(seq)) => {
                    let values = seq.pairs().map(|(key, value)| {
                        (
                            match key {
                                document::YamlScalarNode(scalar) => scalar.get_value(),
                                _ => fail!("unexpected scalar: {:?}", key)
                            },
                            match value {
                                document::YamlScalarNode(scalar) => scalar.get_value(),
                                _ => fail!("unexpected scalar: {:?}", value)
                            }
                        )
                    }).collect();
                    assert_eq!(vec![("a".to_string(), "1".to_string()), ("b".to_string(), "2".to_string())], values)
                },
                _ => fail!("unexpected result: {:?}", docs)
            }
        }
    }
}
