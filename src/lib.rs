extern crate byteorder;
extern crate num;

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use std::intrinsics::transmute;
use std::io;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::rc::Rc;
use std::string::FromUtf8Error;

pub static CQL_VERSION: u8 = 0x03;

#[derive(Clone, Debug)]
enum OpcodeReq {
    //requests
    Startup = 0x01,
    Cred = 0x04,
    Opts = 0x05,
    Query = 0x07,
    Prepare = 0x09,
    Register = 0x0B,
}

#[derive(Debug)]
enum OpcodeResp {
    //responces
    Error = 0x00,
    Ready = 0x02,
    Auth = 0x03,
    Supported = 0x06,
    Result = 0x08,
    Exec = 0x0A,
    Event = 0x0C,
}

fn opcode(val: u8) -> OpcodeResp {
    match val {
        //        0x01 => Startup,
        //        0x04 => Cred,
        //        0x05 => Opts,
        //        0x07 => Query,
        //        0x09 => Prepare,
        //        0x0B => Register,
        0x00 => OpcodeResp::Error,
        0x02 => OpcodeResp::Ready,
        0x03 => OpcodeResp::Auth,
        0x06 => OpcodeResp::Supported,
        0x08 => OpcodeResp::Result,
        0x0A => OpcodeResp::Exec,
        0x0C => OpcodeResp::Event,
        _ => OpcodeResp::Error,
    }
}

#[derive(Clone, Debug)]
pub enum Consistency {
    Any = 0x0000,
    One = 0x0001,
    Two = 0x0002,
    Three = 0x0003,
    Quorum = 0x0004,
    All = 0x0005,
    LocalQuorum = 0x0006,
    EachQuorum = 0x0007,
    Unknown,
}

pub fn consistency(val: u16) -> Consistency {
    match val {
        0 => Consistency::Any,
        1 => Consistency::One,
        2 => Consistency::Two,
        3 => Consistency::Three,
        4 => Consistency::Quorum,
        5 => Consistency::All,
        6 => Consistency::LocalQuorum,
        7 => Consistency::EachQuorum,
        _ => Consistency::Unknown,
    }
}

#[derive(Clone, Debug)]
pub enum ColumnType {
    Custom = 0x0000,
    Ascii = 0x0001,
    Bigint = 0x0002,
    Blob = 0x0003,
    Boolean = 0x0004,
    Counter = 0x0005,
    Decimal = 0x0006,
    Double = 0x0007,
    Float = 0x0008,
    Int = 0x0009,
    Text = 0x000A,
    Timestamp = 0x000B,
    UUID = 0x000C,
    VarChar = 0x000D,
    Varint = 0x000E,
    TimeUUID = 0x000F,
    Inet = 0x0010,
    List = 0x0020,
    Map = 0x0021,
    Set = 0x0022,
    Unknown = 0xffff,
}

fn column_type(val: u16) -> ColumnType {
    match val {
        0x0000 => ColumnType::Custom,
        0x0001 => ColumnType::Ascii,
        0x0002 => ColumnType::Bigint,
        0x0003 => ColumnType::Blob,
        0x0004 => ColumnType::Boolean,
        0x0005 => ColumnType::Counter,
        0x0006 => ColumnType::Decimal,
        0x0007 => ColumnType::Double,
        0x0008 => ColumnType::Float,
        0x0009 => ColumnType::Int,
        0x000A => ColumnType::Text,
        0x000B => ColumnType::Timestamp,
        0x000C => ColumnType::UUID,
        0x000D => ColumnType::VarChar,
        0x000E => ColumnType::Varint,
        0x000F => ColumnType::TimeUUID,
        0x0010 => ColumnType::Inet,
        0x0020 => ColumnType::List,
        0x0021 => ColumnType::Map,
        0x0022 => ColumnType::Set,
        _ => ColumnType::Unknown,
    }
}

#[derive(Debug)]
pub enum Error {
    UnexpectedEOF,
    Io(io::Error),
    Utf8(FromUtf8Error),
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        Error::Io(err)
    }
}

impl From<FromUtf8Error> for Error {
    fn from(err: FromUtf8Error) -> Self {
        Error::Utf8(err)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

trait CqlSerializable {
    fn len(&self) -> usize;
    fn serialize<T: io::Write>(&self, buf: &mut T) -> Result<()>;
}

trait CqlReader {
    fn read_bytes(&mut self, len: usize) -> Result<Vec<u8>>;
    fn read_cql_str(&mut self) -> Result<String>;
    fn read_cql_long_str(&mut self) -> Result<Option<String>>;
    fn read_cql_rows(&mut self) -> Result<Rows>;

    fn read_cql_metadata(&mut self) -> Result<Metadata>;
    fn read_cql_response(&mut self) -> Result<Response>;
}

fn read_full<R: io::Read>(rdr: &mut R, buf: &mut [u8]) -> Result<()> {
    let mut nread = 0usize;
    while nread < buf.len() {
        match rdr.read(&mut buf[nread..])? {
            0 => return Err(Error::UnexpectedEOF),
            n => nread += n,
        }
    }
    Ok(())
}

impl<'a, T: io::Read> CqlReader for T {
    fn read_bytes(&mut self, len: usize) -> Result<Vec<u8>> {
        let mut vec = vec![0; len];
        read_full(self, vec.as_mut_slice())?;
        Ok(vec)
    }

    fn read_cql_str(&mut self) -> Result<String> {
        let len = self.read_u16::<BigEndian>()?;
        let bytes = self.read_bytes(len as usize)?;
        let s = String::from_utf8(bytes)?;
        Ok(s)
    }

    fn read_cql_long_str(&mut self) -> Result<Option<String>> {
        match self.read_i32::<BigEndian>()? {
            -1 => Ok(None),
            len => {
                let bytes = self.read_bytes(len as usize)?;
                let s = String::from_utf8(bytes)?;
                Ok(Some(s))
            }
        }
    }

    fn read_cql_metadata(&mut self) -> Result<Metadata> {
        let flags = self.read_u32::<BigEndian>()?;
        let column_count = self.read_u32::<BigEndian>()?;
        let (keyspace, table) = if flags == 0x0001 {
            let keyspace_str = self.read_cql_str()?;
            let table_str = self.read_cql_str()?;
            (Some(keyspace_str), Some(table_str))
        } else {
            (None, None)
        };

        let mut row_metadata: Vec<CqlColMetadata> = Vec::with_capacity(column_count as usize);
        for _ in 0..column_count {
            let (keyspace, table) = if flags == 0x0001 {
                (None, None)
            } else {
                let keyspace_str = self.read_cql_str()?;
                let table_str = self.read_cql_str()?;
                (Some(keyspace_str), Some(table_str))
            };
            let col_name = self.read_cql_str()?;
            let type_key = self.read_u16::<BigEndian>()?;
            let type_name = if type_key >= 0x20 {
                column_type(self.read_u16::<BigEndian>()?)
            } else {
                ColumnType::Unknown
            };

            row_metadata.push(CqlColMetadata {
                keyspace: keyspace,
                table: table,
                col_name: col_name,
                col_type: column_type(type_key),
                col_type_name: type_name,
            });
        }

        Ok(Metadata {
            flags: flags,
            column_count: column_count,
            keyspace: keyspace,
            table: table,
            row_metadata: row_metadata,
        })
    }

    fn read_cql_rows(&mut self) -> Result<Rows> {
        let metadata = Rc::new(self.read_cql_metadata()?);
        let rows_count = self.read_u32::<BigEndian>()?;
        let col_count = metadata.row_metadata.len();

        let mut rows: Vec<Row> = Vec::with_capacity(rows_count as usize);
        for _ in (0..rows_count) {
            let mut row = Row {
                cols: Vec::with_capacity(col_count),
                metadata: metadata.clone(),
            };
            for meta in metadata.row_metadata.iter() {
                let col = match meta.col_type.clone() {
                    ColumnType::Ascii => Cql::CqlString(self.read_cql_long_str()?),
                    ColumnType::VarChar => Cql::CqlString(self.read_cql_long_str()?),
                    ColumnType::Text => Cql::CqlString(self.read_cql_long_str()?),

                    ColumnType::Int => Cql::Cqli32(match self.read_i32::<BigEndian>()? {
                        -1 => None,
                        4 => Some(self.read_i32::<BigEndian>()?),
                        len => panic!("Invalid length with i32: {}", len),
                    }),
                    ColumnType::Bigint => Cql::Cqli64(Some(self.read_i64::<BigEndian>()?)),
                    ColumnType::Float => Cql::Cqlf32(unsafe {
                        match self.read_i32::<BigEndian>()? {
                            -1 => None,
                            4 => Some(transmute(self.read_u32::<BigEndian>()?)),
                            len => panic!("Invalid length with f32: {}", len),
                        }
                    }),
                    ColumnType::Double => Cql::Cqlf64(unsafe {
                        match self.read_i32::<BigEndian>()? {
                            -1 => None,
                            4 => Some(transmute(self.read_u64::<BigEndian>()?)),
                            len => panic!("Invalid length with f64: {}", len),
                        }
                    }),

                    ColumnType::List => Cql::CqlList({
                        match self.read_i32::<BigEndian>()? {
                            -1 => None,
                            _ => {
                                //let data = self.read_bytes(len as usize);
                                panic!("List parse not implemented: {}");
                            }
                        }
                    }),

                    //                    Custom => ,
                    //                    Blob => ,
                    //                    Boolean => ,
                    //                    Counter => ,
                    //                    Decimal => ,
                    //                    Timestamp => ,
                    //                    UUID => ,
                    //                    Varint => ,
                    //                    TimeUUID => ,
                    //                    Inet => ,
                    //                    List => ,
                    //                    Map => ,
                    //                    Set => ,
                    _ => {
                        match self.read_i32::<BigEndian>()? {
                            -1 => (),
                            len => {
                                self.read_bytes(len as usize)?;
                            }
                        }
                        Cql::CqlUnknown
                    }
                };

                row.cols.push(col);
            }
            rows.push(row);
        }

        Ok(Rows {
            metadata: metadata,
            rows: rows,
        })
    }

    fn read_cql_response(&mut self) -> Result<Response> {
        let header_data = self.read_bytes(9)?;
        let mut header_reader = io::Cursor::new(header_data.as_slice());

        // 9-byte header
        let version = header_reader.read_u8()?;
        let flags = header_reader.read_u8()?;
        let stream = header_reader.read_i16::<BigEndian>()?;
        let opcode = opcode(header_reader.read_u8()?);
        let length = header_reader.read_u32::<BigEndian>()?;
        eprintln!("len: {:?}, opcode: {:?}", length, opcode);

        let body_data = self.read_bytes(length as usize)?;
        let mut reader = io::Cursor::new(body_data.as_slice());

        let body = match opcode {
            OpcodeResp::Ready => ResponseBody::Ready,
            OpcodeResp::Auth => ResponseBody::Auth(reader.read_cql_str()?),
            OpcodeResp::Error => {
                let code = reader.read_u32::<BigEndian>()?;
                let msg = reader.read_cql_str()?;

                match code {
                    0x2400 => {
                        let _ks = reader.read_cql_str()?;
                        let _namespace = reader.read_cql_str()?;
                    }
                    _ => (),
                }
                ResponseBody::Error(code, msg)
            }
            OpcodeResp::Result => {
                let code = reader.read_u32::<BigEndian>()?;
                match code {
                    0x0001 => ResponseBody::Void,
                    0x0002 => ResponseBody::Rows(reader.read_cql_rows()?),
                    0x0003 => {
                        let msg = reader.read_cql_str()?;
                        ResponseBody::Keyspace(msg)
                    }
                    0x0004 => {
                        let id = reader.read_u8()?;
                        let metadata = reader.read_cql_metadata()?;
                        ResponseBody::Prepared(id, metadata)
                    }
                    0x0005 => {
                        let change = reader.read_cql_str()?;
                        let keyspace = reader.read_cql_str()?;
                        let table = reader.read_cql_str()?;
                        ResponseBody::SchemaChange(change, keyspace, table)
                    }
                    _ => {
                        panic!("Unknown code for result: {}", code);
                    }
                }
            }
            _ => {
                panic!("Invalid response from server");
            }
        };
        eprintln!("body: {:?}", body);

        println!("{:?} {:?}", header_data, body_data);

        if reader.position() != length as u64 {
            eprintln!("short: {} != {}", reader.position(), length);
        }

        Ok(Response {
            version: version,
            flags: flags,
            stream: stream,
            opcode: opcode,
            body: body,
        })
    }
}

#[derive(Debug)]
struct Pair {
    key: Vec<u8>,
    value: Vec<u8>,
}

impl CqlSerializable for Pair {
    fn serialize<T: io::Write>(&self, buf: &mut T) -> Result<()> {
        buf.write_u16::<BigEndian>(self.key.len() as u16)?;
        buf.write_all(self.key.as_slice())?;
        buf.write_u16::<BigEndian>(self.value.len() as u16)?;
        buf.write_all(self.value.as_slice())?;
        Ok(())
    }

    fn len(&self) -> usize {
        return 4 + self.key.len() + self.value.len();
    }
}

#[derive(Debug)]
pub struct StringMap {
    pairs: Vec<Pair>,
}

impl CqlSerializable for StringMap {
    fn serialize<T: io::Write>(&self, buf: &mut T) -> Result<()> {
        buf.write_u16::<BigEndian>(self.pairs.len() as u16)?;
        for pair in self.pairs.iter() {
            pair.serialize(buf)?;
        }
        Ok(())
    }

    fn len(&self) -> usize {
        let mut len = 2usize;
        for pair in self.pairs.iter() {
            len += pair.len();
        }
        len
    }
}

#[derive(Debug)]
struct CqlColMetadata {
    keyspace: Option<String>,
    table: Option<String>,
    col_name: String,
    col_type: ColumnType,
    col_type_name: ColumnType,
}

#[derive(Debug)]
pub struct Metadata {
    flags: u32,
    column_count: u32,
    keyspace: Option<String>,
    table: Option<String>,
    row_metadata: Vec<CqlColMetadata>,
}

#[derive(Clone, Debug)]
pub enum Cql {
    CqlString(Option<String>),

    Cqli32(Option<i32>),
    Cqli64(Option<i64>),

    CqlBlob(Option<Vec<u8>>),
    CqlBool(Option<bool>),

    CqlCounter(Option<u64>),

    Cqlf32(Option<f32>),
    Cqlf64(Option<f64>),

    CqlTimestamp(u64),
    CqlBigint(num::BigInt),

    CqlList(Option<Vec<Cql>>),

    CqlUnknown,
}

#[derive(Debug)]
pub struct Row {
    cols: Vec<Cql>,
    metadata: Rc<Metadata>,
}

impl Row {
    pub fn get_column(&self, col_name: &str) -> Option<Cql> {
        let mut i = 0;
        for metadata in self.metadata.row_metadata.iter() {
            if metadata.col_name == col_name {
                return Some(self.cols[i].clone());
            }
            i += 1;
        }
        None
    }
}

#[derive(Debug)]
pub struct Rows {
    metadata: Rc<Metadata>,
    rows: Vec<Row>,
}

#[derive(Debug)]
pub enum RequestBody {
    RequestStartup(StringMap),
    RequestCred(Vec<Vec<u8>>),
    RequestQuery(String, Consistency),
    RequestOptions,
}

#[derive(Debug)]
pub enum ResponseBody {
    Error(u32, String),
    Ready,
    Auth(String),

    Void,
    Rows(Rows),
    Keyspace(String),
    Prepared(u8, Metadata),
    SchemaChange(String, String, String),
}

#[derive(Debug)]
struct Request {
    version: u8,
    flags: u8,
    stream: i16,
    opcode: OpcodeReq,
    body: RequestBody,
}

#[derive(Debug)]
pub struct Response {
    version: u8,
    flags: u8,
    stream: i16,
    opcode: OpcodeResp,
    body: ResponseBody,
}

impl CqlSerializable for Request {
    fn serialize<T: io::Write>(&self, buf: &mut T) -> Result<()> {
        buf.write_u8(self.version)?;
        buf.write_u8(self.flags)?;
        buf.write_i16::<BigEndian>(self.stream)?;
        buf.write_u8(self.opcode.clone() as u8)?;
        buf.write_u32::<BigEndian>((self.len() - 9) as u32)?;

        match self.body {
            RequestBody::RequestStartup(ref map) => map.serialize(buf),
            RequestBody::RequestQuery(ref query_str, ref consistency) => {
                buf.write_u32::<BigEndian>(query_str.len() as u32)?;
                buf.write_all(query_str.as_bytes())?;
                buf.write_u16::<BigEndian>(consistency.clone() as u16)?;
                buf.write_u8(0)?;
                Ok(())
            }
            _ => panic!("not implemented request type"),
        }
    }
    fn len(&self) -> usize {
        9 + match self.body {
            RequestBody::RequestStartup(ref map) => map.len(),
            RequestBody::RequestQuery(ref query_str, _) => 4 + query_str.len() + 3,
            _ => panic!("not implemented request type"),
        }
    }
}

fn startup() -> Request {
    let body = StringMap {
        pairs: vec![Pair {
            key: b"CQL_VERSION".to_vec(),
            value: b"3.0.0".to_vec(),
        }],
    };
    return Request {
        version: CQL_VERSION,
        flags: 0x00,
        stream: 0x01,
        opcode: OpcodeReq::Startup,
        body: RequestBody::RequestStartup(body),
    };
}

fn auth(creds: Vec<Vec<u8>>) -> Request {
    return Request {
        version: CQL_VERSION,
        flags: 0x00,
        stream: 0x01,
        opcode: OpcodeReq::Cred,
        body: RequestBody::RequestCred(creds),
    };
}

fn options() -> Request {
    return Request {
        version: CQL_VERSION,
        flags: 0x00,
        stream: 0x01,
        opcode: OpcodeReq::Opts,
        body: RequestBody::RequestOptions,
    };
}

fn query(stream: i16, query_str: &str, con: Consistency) -> Request {
    return Request {
        version: CQL_VERSION,
        flags: 0x00,
        stream: stream,
        opcode: OpcodeReq::Query,
        body: RequestBody::RequestQuery(query_str.to_string(), con),
    };
}

pub struct Client {
    socket: TcpStream,
}

impl Client {
    pub fn query(&mut self, query_str: &str, con: Consistency) -> Result<Response> {
        let q = query(0, query_str, con);

        let mut writer = Vec::new();
        q.serialize::<Vec<u8>>(&mut writer)?;
        self.socket.write_all(writer.as_slice())?;

        self.socket.read_cql_response()
    }
}

pub fn connect(addr: &str) -> Result<Client> {
    let mut socket = TcpStream::connect(addr)?;
    let msg_startup = startup();

    let mut buf = Vec::new();
    msg_startup.serialize::<Vec<u8>>(&mut buf)?;
    socket.write_all(buf.as_slice())?;

    let response = socket.read_cql_response()?;
    match response.body {
        ResponseBody::Ready => Ok(Client { socket: socket }),
        /*
        Auth(_) => {
            match(creds) {
                Some(cred) => {
                    let msg_auth = Auth(cred);
                    msg_auth.serialize::<net_tcp::TcpSocketBuf>(&buf);
                    let response = buf.read_cql_response();
                    match response.body {
                        Ready => result::Ok(Client { socket: buf }),
                        Error(_, ref msg) => {
                            result::Err(Error(~"Error", copy *msg))
                        }
                        _ => {
                            result::Err(Error(~"Error", ~"Server returned unknown message"))
                        },
                    }
                },
                None => {
                    result::Err(Error(~"Error", ~"Credential should be provided"))
                },
            }

        }
        */
        _ => panic!("invalid opcode: {}", response.opcode as u8),
    }
}

#[cfg(test)]
mod tests {
    use super::CqlReader;

    #[test]
    fn resp_ready() {
        let v = vec![131, 0, 0, 1, 2, 0, 0, 0, 0];
        let mut s = v.as_slice();
        let resp = s.read_cql_response();
        assert!(resp.is_ok())
    }

    #[test]
    fn resp_error() {
        let v = vec![
            131, 0, 0, 0, 0, 0, 0, 0, 49, 0, 0, 36, 0, 0, 35, 67, 97, 110, 110, 111, 116, 32, 97,
            100, 100, 32, 101, 120, 105, 115, 116, 105, 110, 103, 32, 107, 101, 121, 115, 112, 97,
            99, 101, 32, 34, 114, 117, 115, 116, 34, 0, 4, 114, 117, 115, 116, 0, 0,
        ];
        let mut s = v.as_slice();
        let resp = s.read_cql_response();
        assert!(resp.is_ok())
    }

    #[test]
    fn resp_schema_change() {
        let v = vec![
            131, 0, 0, 0, 8, 0, 0, 0, 29, 0, 0, 0, 5, 0, 7, 67, 82, 69, 65, 84, 69, 68, 0, 8, 75,
            69, 89, 83, 80, 65, 67, 69, 0, 4, 114, 117, 115, 116,
        ];

        let mut s = v.as_slice();
        let resp = s.read_cql_response();
        assert!(resp.is_ok())
    }

    #[test]
    fn resp_schema_change_table() {
        let v = vec![
            131, 0, 0, 0, 8, 0, 0, 0, 32, 0, 0, 0, 5, 0, 7, 67, 82, 69, 65, 84, 69, 68, 0, 5, 84,
            65, 66, 76, 69, 0, 4, 114, 117, 115, 116, 0, 4, 116, 101, 115, 116,
        ];

        let mut s = v.as_slice();
        let resp = s.read_cql_response();
        assert!(resp.is_ok())
    }

    #[test]
    fn resp_result_void() {
        let v = vec![131, 0, 0, 0, 8, 0, 0, 0, 4, 0, 0, 0, 1];

        let mut s = v.as_slice();
        let resp = s.read_cql_response();
        assert!(resp.is_ok())
    }

    #[test]
    fn resp_result_select() {
        let v = vec![
            131, 0, 0, 0, 8, 0, 0, 0, 59, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0, 2, 0, 4, 114, 117, 115,
            116, 0, 4, 116, 101, 115, 116, 0, 2, 105, 100, 0, 13, 0, 5, 118, 97, 108, 117, 101, 0,
            8, 0, 0, 0, 1, 0, 0, 0, 4, 97, 115, 100, 102, 0, 0, 0, 4, 63, 158, 4, 25,
        ];

        let mut s = v.as_slice();
        let resp = s.read_cql_response();
        assert!(resp.is_ok())
    }
}