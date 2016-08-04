#![allow(non_snake_case)] // to simplify json decoding for the Response types

extern crate hyper;
extern crate rustc_serialize;
extern crate libc;
extern crate std;
extern crate uuid;

use rustc_serialize::json::{Json, ToJson};
use std::collections::BTreeMap;
use std::path::{PathBuf, Path};
pub use fuse::FileType;

#[derive (RustcDecodable, Debug, Clone)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u32,
    pub id_token: String,
    pub refresh_token: String,
}

impl ToJson for TokenResponse {
    fn to_json(&self) -> Json {
        let mut d = BTreeMap::new();
        d.insert("access_token".to_string(),  self.access_token.to_json());
        d.insert("token_type".to_string(),    self.token_type.to_json());
        d.insert("expires_in".to_string(),    self.expires_in.to_json());
        d.insert("id_token".to_string(),      self.id_token.to_json());
        d.insert("refresh_token".to_string(), self.refresh_token.to_json());
        Json::Object(d)
    }
}

#[derive (RustcDecodable, Debug, Clone)]
pub struct DriveFileResponse {
    pub kind: String,
    pub id: String,
    pub name: String,
    pub mimeType: String,
}

#[derive (Debug, Clone)]
pub enum SourceData {
    Drive(DriveFileResponse),
}

#[derive (Debug, Clone)]
pub struct FileResponse {
    pub uuid: uuid::Uuid,
    pub kind: FileType,
    pub name: String,
    pub source_data: SourceData,
}

#[derive (Debug, Clone)]
pub struct FileData {
    pub id: uuid::Uuid,
    pub kind: FileType,
    pub path: PathBuf,
    pub inode: u64,
    pub parent_inode: u64,
    pub size: Option<u64>,
    pub source_data: SourceData,
}

impl ToJson for DriveFileResponse {
    fn to_json(&self) -> Json {
        let mut d = BTreeMap::new();
        // All standard types implement `to_json()`, so use it
        d.insert("kind".to_string(),     self.kind.to_json());
        d.insert("id".to_string(),       self.id.to_json());
        d.insert("name".to_string(),     self.name.to_json());
        d.insert("mimeType".to_string(), self.mimeType.to_json());
        Json::Object(d)
    }
}

#[derive (RustcDecodable, Debug, Clone)]
pub struct ErrorDetailsResponse {
    pub domain: String,
    pub reason: String,
    pub message: String,
    pub locationType: String,
    pub location: String,
}

#[derive (RustcDecodable, RustcEncodable, Debug, Clone)]
pub struct FileCheckResponse {
    pub md5Checksum: String,
    pub size: String,
}

#[derive (Clone)]
pub struct AuthData {
    pub tr: TokenResponse,
    pub client_id: String,
    pub client_secret: String,
    // maybe this can be converted to a std::path::Path later?
    pub cache_file_path: String,
}

#[derive (Debug)]
pub enum DriveErrorType {
    Hyper(hyper::error::Error),
    JsonDecodeFileList,
    JsonReadError(rustc_serialize::json::BuilderError),
    JsonObjectify,
    JsonInvalidAttribute,
    JsonCannotConvertToArray,
    JsonCannotDecode(rustc_serialize::json::DecoderError),
    JsonCannotEncode(rustc_serialize::json::EncoderError),
    Io(std::io::Error),
    UnsupportedDocumentType,
    FailedChecksum,
    FailedToChecksumExistingFile,
    Tester,
    NoFileName,
    FailedUuidLookup,
}

#[derive (Debug)]
pub struct DriveError {
    pub kind: DriveErrorType,
    pub response: Option<String>,
}

impl From<hyper::error::Error> for DriveError {
    fn from(err: hyper::error::Error) -> DriveError {
        DriveError {
            kind: DriveErrorType::Hyper(err),
            response: None
        }
    }
}

impl From<rustc_serialize::json::BuilderError> for DriveError {
    fn from(err: rustc_serialize::json::BuilderError) -> DriveError {
        DriveError {
            kind: DriveErrorType::JsonReadError(err),
            response: None
        }
    }
}

impl From<rustc_serialize::json::DecoderError> for DriveError {
    fn from(err: rustc_serialize::json::DecoderError) -> DriveError {
        DriveError {
            kind: DriveErrorType::JsonCannotDecode(err),
            response: None
        }
    }
}

impl From<std::io::Error> for DriveError {
    fn from(err: std::io::Error) -> DriveError {
        DriveError {
            kind: DriveErrorType::Io(err),
            response: None
        }
    }
}

impl From<rustc_serialize::json::EncoderError> for DriveError {
    fn from(err: rustc_serialize::json::EncoderError) -> DriveError {
        DriveError {
            kind: DriveErrorType::JsonCannotEncode(err),
            response: None,
        }
    }
}
