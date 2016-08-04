extern crate uuid;
extern crate std;

use hyper::{Client};
use hyper::header::{ContentType, Authorization, Bearer};
use mime::{Mime, TopLevel, SubLevel};

use std::collections::hash_map::HashMap;
use rustc_serialize::json::{Json, ToJson, Decoder, as_pretty_json};
use rustc_serialize::{json, Decodable};
use std::io;
use std::io::prelude::*;

use crypto::md5::Md5;
use crypto::digest::Digest;

use std::fs::{File, OpenOptions};
use std::path::{Path};

use uuid::Uuid;

use types::*;
use filetree::*;

pub struct DriveFileDownloader {
    pub client: Client,
    pub auth_data: AuthData,

    uuid_map: HashMap<Uuid, String>,
}

const CACHE_FILE: &'static str = "access";
const CLIENT_ID: &'static str = "460434421766-0sktb0rkbvbko8omj8vhu8vv83giraao.apps.googleusercontent.com";
const CLIENT_SECRET: &'static str = "m_ILEPtnZI53tXow9hoaabjm";

impl DriveFileDownloader {
    pub fn new(root_uuid: Uuid, root_id: String) -> Result<DriveFileDownloader, DriveError> {
        let c = Client::new();

        let mut handle = try!(OpenOptions::new()
                            .read(true)
                            .write(true)
                            .create(true)
                            .open(CACHE_FILE));
        let mut access_string = String::new();
        try!(handle.read_to_string(&mut access_string));
        println!("{}", access_string);
        let tr: TokenResponse = match json::decode(&access_string) {
            Ok(tr) => tr,
            Err(_) => {
                let tr = try!(request_new_access_code(&c));
                println!("{}", tr.clone().to_json().to_string());
                try!(handle.write_all(tr.to_json().pretty().to_string().as_bytes()));
                tr
            }
        };

        let mut uuid_map = HashMap::new();
        uuid_map.insert(root_uuid, root_id);

        Ok(DriveFileDownloader {
            client: c,
            auth_data: AuthData {
                tr: tr,
                client_id: CLIENT_ID.to_string(),
                client_secret: CLIENT_SECRET.to_string(),
                cache_file_path: CACHE_FILE.to_string(),
            },
            uuid_map: uuid_map,
        })
    }
}

impl FileDownloader for DriveFileDownloader {
    fn get_file_list(&mut self, root_folder_uuid: &uuid::Uuid) -> Result<Vec<FileResponse>, DriveError> {
        let mut resp = {
            let root_folder_id =
                try!(self.uuid_map.get(root_folder_uuid).ok_or(DriveError{
                kind: DriveErrorType::FailedUuidLookup,
                response: None,
            }));
            try!(self.client.get(&format!("https://www.googleapis.com/drive/v3/files\
                              ?corpus=domain\
                              &pageSize=100\
                              &q=%27{}%27+in+parents\
                              +and+trashed+%3D+false"
                              , root_folder_id))
                            .header(Authorization(Bearer{token: self.auth_data.tr.access_token.clone()}))
                            .send())
        };
        // convert the response to a string, then to a JSON object
        let mut resp_string = String::new();
        try!(resp.read_to_string(&mut resp_string));
        let fr_obj = try!((Json::from_str(&resp_string)).map_err(From::from)
            .and_then(|fr_data: Json| -> Result<json::Object, DriveError> {
                fr_data.into_object()
                .ok_or(DriveError {
                    kind: DriveErrorType::JsonObjectify,
                    response: Some(resp_string.clone())
                })
            }).or_else(|err| {
                println!("there's probably something wrong with the get_file_list response, err: {:?}", err);
                Err(err)
            })
        );

        //println!("{}", resp_string);

        let files = try!(fr_obj.get("files")
                               .ok_or(DriveError {
                                    kind: DriveErrorType::JsonInvalidAttribute,
                                    response: Some(resp_string.clone())
                                }));
        let files_array = try!(files.as_array()
                                    .ok_or(DriveError {
                                        kind: DriveErrorType::JsonCannotConvertToArray,
                                        response: Some(resp_string.clone())
                                    }));

        let mut files = Vec::new();
        for i in files_array.into_iter() {
            // we'll try to decode each file's metadata JSON object in memory to a DriveFileResponse struct
            let mut decoder = Decoder::new(i.clone());
            let fr: DriveFileResponse = try!(Decodable::decode(&mut decoder));

            let kind = if fr.mimeType == "application/vnd.google-apps.folder" {
                FileType::Directory
            } else {
                FileType::RegularFile
            };

            let uuid = Uuid::new_v4();
            {
                self.uuid_map.insert(uuid, fr.id.clone());
            }

            files.push(FileResponse {
                uuid: uuid,
                kind: kind,
                name: fr.name.clone(),
                source_data: SourceData::Drive(fr)
            });
        }

        Ok(files)
    }

    fn resolve_error(&mut self, resp_string: &str) -> Result<(), DriveError> {
        println!("attempting to resolve error response: {}", resp_string);

        let err_data = try!(Json::from_str(&resp_string));
        let err_obj  = try!(err_data.as_object()
                                    .ok_or(DriveError {
                                        kind: DriveErrorType::JsonObjectify,
                                        response: Some(resp_string.to_string())
                                    }));

        let error = try!(err_obj.get("error")
                                .ok_or(DriveError {
                                    kind: DriveErrorType::JsonInvalidAttribute,
                                    response: Some(resp_string.to_string())
                                }));
        let errors = try!(error.as_object()
                                    .ok_or(DriveError {
                                        kind: DriveErrorType::JsonObjectify,
                                        response: Some(resp_string.to_string())
                                    })
                                .and_then(|err_obj| err_obj.get("errors")
                                    .ok_or(DriveError {
                                        kind: DriveErrorType::JsonInvalidAttribute,
                                        response: Some(resp_string.to_string())
                                    }))
                                .and_then(|errors| errors.as_array()
                                    .ok_or(DriveError {
                                        kind: DriveErrorType::JsonInvalidAttribute,
                                            response: Some(resp_string.to_string())
                                    })));
        for i in errors {
            let mut decoder = Decoder::new(i.clone());
            let err: ErrorDetailsResponse = match Decodable::decode(&mut decoder) {
                Ok(err) => err,
                Err(error) => panic!("could not decode errorDetailsResponse, error: {}, attempted edr: {}", error, i)
            };

            if err.reason   == "authError"           &&
               err.message  == "Invalid Credentials" &&
               err.location == "Authorization" {
                let mut resp = try!(self.client.post("https://www.googleapis.com/oauth2/v3/token")
                                      .header(ContentType(Mime(TopLevel::Application, SubLevel::WwwFormUrlEncoded, vec![])))
                                      //.header(Host{hostname: "www.googleapis.com".to_owned(), port: None})
                                      .body(&format!("&client_id={}\
                                            &client_secret={}\
                                            &refresh_token={}\
                                            &grant_type=refresh_token", self.auth_data.client_id, self.auth_data.client_secret, self.auth_data.tr.refresh_token))
                                      .send());

                self.auth_data.tr.access_token = {
                    let mut ref_string = String::new();
                    try!(resp.read_to_string(&mut ref_string));
                    let ref_data = try!(Json::from_str(&ref_string));
                    let ref_obj  = try!(ref_data.as_object()
                                                .ok_or(DriveError {
                                                    kind: DriveErrorType::JsonObjectify,
                                                    response: Some(ref_string.clone())
                                                }));
                    try!(ref_obj.get("access_token")
                                .ok_or(DriveError {
                                    kind: DriveErrorType::JsonInvalidAttribute,
                                    response: Some(ref_string.clone())
                                })
                                .and_then(|acc| {
                                    acc.as_string()
                                       .ok_or(DriveError {
                                           kind: DriveErrorType::JsonInvalidAttribute,
                                           response: Some(ref_string)
                                       })
                                })).to_string()
                };

                // we'll open our access_cache file to read its current contents
                let mut f = try!(File::open(self.auth_data.cache_file_path.clone()));
                let mut access_string = String::new();
                try!(f.read_to_string(&mut access_string).map_err(From::from).and_then(|_| -> Result<(), DriveError> {
                    // if the current access file reads as it should, we'll copy the
                    // current token response and edit the   access_token field for later use
                    let mut tr: TokenResponse = try!(json::decode(&access_string));
                    tr.access_token = self.auth_data.tr.access_token.clone();
                    let tr_json = tr.to_json();
                    let tr_str = format!("{}", as_pretty_json(&tr_json));

                    // then we'll truncate the file and paste in the updated token response
                    // preserving all the other unused data
                    let mut f: File = try!(File::create(self.auth_data.cache_file_path.clone()).map_err(From::from) as Result<File, DriveError>);
                    f.write_all(tr_str.as_bytes()).map_err(From::from)
                }))
            }
        }

        // for loop returns (), so a value for the function is needed
        Ok(())
    }

    fn create_new_file(&mut self, fd: &FileData, file_path: &Path, metadata_path_str:&Path) -> Result<u64, DriveError> {
        let SourceData::Drive(ref fr) = fd.source_data;
        if fr.mimeType == "application/vnd.google-apps.document"
           || fr.mimeType == "application/vnd.google-apps.form"
           || fr.mimeType == "application/vnd.google-apps.drawing"
           || fr.mimeType == "application/vnd.google-apps.fusiontable"
           || fr.mimeType == "application/vnd.google-apps.map"
           || fr.mimeType == "application/vnd.google-apps.presentation"
           || fr.mimeType == "application/vnd.google-apps.spreadsheet"
           || fr.mimeType == "application/vnd.google-apps.sites" {
            println!("unsupported google filetype");
            return Err(DriveError {
                kind: DriveErrorType::UnsupportedDocumentType,
                response: None
            })
        }

        let mut dotf = try!(File::create(metadata_path_str.clone()));

        // if the metadata file exists, but doesn't read (there was probably some corruption),
        // we'll checksum the file (if we have it) on the system, and compare it to the
        // server, if all is OK, create a new metadata file
        try!(get_file_checksum(file_path).or_else(|_| {
            println!("no file checksum yet");
            Err(DriveError {
                kind: DriveErrorType::FailedToChecksumExistingFile,
                response: None
            })
        }).and_then(|current_file_checksum| {
            self.verify_checksum(&fd, &current_file_checksum).and_then(|check_response| {
                let fcr_string = try!(json::encode(&check_response));
                try!(dotf.write_all(&fcr_string.into_bytes()));
                let size = check_response.size.parse::<u64>().expect("couldn't unwrap size in filecheck");
                Ok(size) // this Ok(()) is necessary for the sam reason that the Ok(()) at the bottom of the function is:
                       // a mixture of io::Result and another Result type in the try block
            })
        }).or_else(|_| -> Result<u64, DriveError>  {
                    // if the checksum matches that from Drive, then file is downloaded
                    // correctly, and all we need to do is cache the checksum
                    // the file data doesn't yet exist, so we'll download it from scratch
                    println!("downloading new file, creating new metadata file: {:?}", metadata_path_str);

                    let mut f = try!(File::create(file_path));
                    let mut resp = try!(self.client.get(&format!("https://www.googleapis.com/drive/v3/files/{}\
                                ?alt=media", fr.id.clone()))
                                         .header(Authorization(Bearer{token: self.auth_data.tr.access_token.clone()}))
                                         .send());

                    let mut resp_string = Vec::<u8>::new();
                    try!(resp.read_to_end(&mut resp_string));
                    println!("len: {}", resp_string.len());
                    try!(f.write_all(&resp_string.clone()));

                    let md5_result = {
                        let mut md5 = Md5::new();
                        md5.input(&resp_string);
                        md5.result_str()
                    };

                    let check_response = try!(self.verify_checksum(&fd, &md5_result));
                    let fcr_string = try!(json::encode(&check_response));
                    try!(dotf.write_all(&fcr_string.into_bytes()));

                    let size = check_response.size.parse::<u64>().expect("couldn't unwrap size in filecheck");
                    Ok(size)
            })
        );

        // i think this is necessary because get_file_checksum() has type io::Result, so it doesn't fit the type of the
        // rest of the try block
        Ok(0)
    }

    fn verify_checksum(&mut self, fd: &FileData, checksum: &String) -> Result<FileCheckResponse, DriveError> {
        let SourceData::Drive(ref fr) = fd.source_data;
        let mut resp = try!(self.client
            .get(&format!(
                "https://www.googleapis.com/drive/v3/files/{}\
                ?fields=md5Checksum%2Csize"
                , fr.id.clone()))
            .header(Authorization(Bearer{token: self.auth_data.tr.access_token.clone()}))
            .send());

        let mut resp_string = String::new();
        try!(resp.read_to_string(&mut resp_string));
        let fcr: FileCheckResponse = try!(json::decode(&resp_string));
        let same = checksum == &fcr.md5Checksum;
        if !same {
            println!("{} =? {}", checksum, fcr.md5Checksum);
            println!("same?: {:?}", same);
            println!("size: {}", fcr.size);
            return Err(DriveError {
                kind: DriveErrorType::FailedChecksum,
                response: Some(resp_string)
            })
        }

        Ok(fcr)
    }

}

pub fn request_new_access_code(c: &Client) -> Result<TokenResponse, DriveError> {
    // the space after client_id={} is necessary to seperate the link from the rest of the
    // prompt
    println!("Visit https://accounts.google.com/o/oauth2/v2/auth\
                    ?scope=email%20profile%20https://www.googleapis.com/auth/drive\
                    &redirect_uri=urn:ietf:wg:oauth:2.0:oob\
                    &response_type=code\
                    &client_id={} \
             to receive the access code.", CLIENT_ID);
    let mut code_string = String::new();
    try!(io::stdin().read_line(&mut code_string));

    let mut resp = c.post("https://accounts.google.com/o/oauth2/token")
        .header(ContentType(Mime(TopLevel::Application, SubLevel::WwwFormUrlEncoded, vec![])))
        .body(&format!("code={}\
               &client_id={}\
               &client_secret={}\
               &redirect_uri=urn:ietf:wg:oauth:2.0:oob\
               &grant_type=authorization_code"
               , code_string, CLIENT_ID, CLIENT_SECRET))
        .send()
        .unwrap();
    print!("{}\n{}\n{}\n\n", resp.url, resp.status, resp.headers);

    let mut resp_string = String::new();
    try!(resp.read_to_string(&mut resp_string));
    json::decode(&resp_string).map_err(From::from)
}
