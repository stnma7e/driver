extern crate uuid;
extern crate std;
extern crate rusqlite;

use url::percent_encoding::{utf8_percent_encode, QUERY_ENCODE_SET};
use time;
use time::{Timespec, Tm};

use hyper::{Client};
use hyper::header::{ContentType, Authorization, Bearer};
use mime::{Mime, TopLevel, SubLevel};

use std::collections::hash_map::HashMap;
use rustc_serialize::json::{Json, ToJson, Decoder, as_pretty_json};
use rustc_serialize::{json, Decodable};
use std::io;
use std::io::prelude::*;
use std::fs::{File, DirBuilder,OpenOptions};

use crypto::md5::Md5;
use crypto::digest::Digest;

use std::path::{Path, PathBuf};

use uuid::Uuid;

use types::*;
use filetree::*;

pub struct DriveFileDownloader {
    pub client: Client,
    pub auth_data: AuthData,

    uuid_map: HashMap<Uuid, DriveFileResponse>,
    conn: rusqlite::Connection,
}

const CACHE_FILE: &'static str = "access";
const CLIENT_ID: &'static str = "460434421766-0sktb0rkbvbko8omj8vhu8vv83giraao.apps.googleusercontent.com";
const CLIENT_SECRET: &'static str = "m_ILEPtnZI53tXow9hoaabjm";

struct DownloadedFileInformation {
    size: u64,
    checksum: String,
    path: PathBuf,
}

impl DriveFileDownloader {
    pub fn new(root_uuid: Uuid, root_id: String, file_path: PathBuf, db_conn: rusqlite::Connection) -> Result<DriveFileDownloader, DriveError> {
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

        db_conn.execute("INSERT INTO files (id, uuid, path)
                           VALUES ($1, $2, $3)",
                           &[&root_id,
                           &root_uuid.clone().as_bytes().to_vec(),
                           &file_path.to_str().unwrap()]).unwrap_or_else(
            |err| {
                println!("root probably already in drive db, err: {:?}", err);
                0
            }
        );

        let mut uuid_map = HashMap::new();
        uuid_map.insert(root_uuid, DriveFileResponse {
            kind: "drive#file".to_string(),
            id: root_id.clone(),
            name: "".to_string(),
            mimeType: "application/vnd.google-apps.folder".to_string(),
            parents: vec!(root_id),
            path: Some(file_path),
        });

        Ok(DriveFileDownloader {
            client: c,
            auth_data: AuthData {
                tr: tr,
                client_id: CLIENT_ID.to_string(),
                client_secret: CLIENT_SECRET.to_string(),
                cache_file_path: CACHE_FILE.to_string(),
            },
            uuid_map: uuid_map,
            conn: db_conn,
        })
    }

    fn download_file(&self, uuid: &Uuid, parent_uuid: &Uuid) -> Result<DownloadedFileInformation, DriveError> {
        let fr = try!(self.uuid_map.get(uuid).ok_or(DriveError {
            kind: DriveErrorType::FailedUuidLookup,
            response: None,
        })).clone();
        let parent_path = try!(self.uuid_map.get(parent_uuid).ok_or(DriveError {
            kind: DriveErrorType::FailedUuidLookup,
            response: None,
        })).path.clone();

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

        let mut file_path = try!(parent_path.ok_or(DriveError {
            kind: DriveErrorType::NoPathForParent,
            response: None,
        }));
        file_path.push(fr.name.clone());

        // we'll checksum the file (if we have it) on the system, and compare it to the
        // server, if all is OK, create a new metadata file
        let (maybe_checksum, maybe_path) = try!(self.conn.query_row_named("SELECT checksum, path FROM files
                                                                           WHERE uuid=:uuid"
            , &[(":uuid", &uuid.clone().as_bytes().to_vec())]
            , |row| -> (Option<String>, Option<PathBuf>) {
                ( row.get(0)
                , if let Some(path) = row.get::<i32, Option<String>>(1) {
                    Some(Path::new(&path).to_owned())
                } else { None } )
            }
        ));

        let fn_download_file = |file_path: PathBuf| {
            // the file data doesn't yet exist, so we'll download it from scratch
            println!("downloading new file, creating new file: {:?}", file_path);

            let mut f = try!(File::create(file_path.clone()));
            let mut resp = try!(self.client.get(&format!("https://www.googleapis.com/drive/v3/files/{}\
                        ?alt=media", fr.id.clone()))
                                 .header(Authorization(Bearer{token: self.auth_data.tr.access_token.clone()}))
                                 .send());

            let mut resp_string = Vec::<u8>::new();
            try!(resp.read_to_end(&mut resp_string));
            println!("len: {}", resp_string.len());
            try!(f.write_all(&resp_string.clone())) ;

            let md5_result = {
                let mut md5 = Md5::new();
                md5.input(&resp_string);
                md5.result_str()
            };

            let check_response = try!(self.verify_checksum(uuid, &md5_result));
            let size = check_response.size;

            println!("md5: {}, size: {}", md5_result, size);
            self.conn.execute("UPDATE files
                               SET checksum=$1, size=$2
                               WHERE uuid=$3",
                            &[ &md5_result,
                               &(check_response.size as i64),
                               &uuid.clone().as_bytes().to_vec(),
                             ]).unwrap();

            Ok(DownloadedFileInformation {
                size: size,
                checksum: md5_result,
                path: file_path,
            })
        };

        if let Some(checksum) = maybe_checksum {
            // if the checksum matches that from Drive, then file is downloaded
            // correctly, and all we need to do is cache the checksum
            self.verify_checksum(uuid, &checksum)
            .and_then(|check_response| {
                Ok(DownloadedFileInformation {
                    size: check_response.size,
                    path: file_path.clone(),
                    checksum: checksum,
                })
            }).or_else(|_| {
                fn_download_file(file_path.clone())
            })
        } else if let Some(path) = maybe_path {
            // if we have a path for the file, we can chcek to see if any local data is
            // already valid
            get_file_checksum(&path).and_then(|ck| {
                // check with Drive for a the valid checksum
                self.verify_checksum(uuid, &ck)
                .and_then(|check_response| {
                    Ok(DownloadedFileInformation {
                        size: check_response.size,
                        path: file_path.clone(),
                        checksum: ck,
                    })
                })
            }).or_else(|_| {
                fn_download_file(file_path.clone())
            })
        } else {
            // we have neither a path for local data, nor a local checksum
            fn_download_file(file_path.clone())
        }
    }
}

define_encode_set! {
    /// This encode set is used in the URL parser for query strings.
    pub DRIVE_QUERY_ENCODE_SET = [QUERY_ENCODE_SET] | {':'}
}

impl FileDownloader for DriveFileDownloader {
    fn get_file_list(&mut self, root_folder_uuid: &uuid::Uuid) -> Result<FileUpdates, DriveError> {
        let uuid_vec = root_folder_uuid.clone().as_bytes().to_vec();
        let (parent_id, parent_path) = try!(self.conn.query_row_named("SELECT id, path FROM files WHERE uuid=:uuid"
            , &[(":uuid", &uuid_vec)]
            , |row| -> (String, PathBuf) {
                ( row.get(0)
                , Path::new(&row.get::<i32, String>(1)).to_owned() )
            }
        ));
        println!("getting file list for parent ID: {}", parent_id);

        let lastdate = self.conn.query_row_named("SELECT MAX(last_update) FROM meta
                                                  WHERE uuid = :uuid"
            , &[(":uuid", &uuid_vec)]
            , |row| -> Timespec { row.get(0) }
        ).unwrap_or(Timespec::new(0,0));

        // date from which modified files should be updated
        // older ones should be locally valid
        let lastdate = format!("{}", convert_timespec_to_tm(lastdate).rfc3339());
        let lastdate_encoded = utf8_percent_encode(&lastdate, DRIVE_QUERY_ENCODE_SET{});

        let query = format!("https://www.googleapis.com/drive/v3/files\
                                ?corpus=domain\
                                &pageSize=1000\
                                &fields=files\
                                &q=%27{}%27+in+parents\
                                +and+modifiedTime%3E'{}'\
                                +and+trashed+%3D+"
                                , parent_id
                                , lastdate_encoded);
        println!("{}", query);
        let mut new_resp = {
            println!("{}", parent_id);
            try!(self.client.get(&(query.clone()+"false"))
                            .header(Authorization(Bearer{token: self.auth_data.tr.access_token.clone()}))
                            .send())
        };
        let mut del_resp = {
            println!("{}", parent_id);
            try!(self.client.get(&(query+"true"))
                            .header(Authorization(Bearer{token: self.auth_data.tr.access_token.clone()}))
                            .send())
        };

        let mut resps = vec!(new_resp, del_resp);
        let mut files = vec!(Vec::new(), Vec::new());
        for (resp, files_list) in resps.iter_mut().zip(files.iter_mut()) {

            // convert the response to a string, then to a JSON object
            let mut resp_string = String::new();
            try!(resp.read_to_string(&mut resp_string));
            println!("{}", resp_string);
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

            for i in files_array.into_iter() {
                // we'll try to decode each file's metadata JSON object in memory to a DriveFileResponse struct
                let mut decoder = Decoder::new(i.clone());
                let fr: DriveFileResponse = try!(Decodable::decode(&mut decoder));

                let kind = if fr.mimeType == "application/vnd.google-apps.folder" {
                    FileType::Directory
                } else {
                    FileType::RegularFile
                };

                let mut path = parent_path.clone();
                path.push(fr.name.clone());


                let uuid = self.conn.query_row_named("SELECT uuid FROM files WHERE id=:id"
                    , &[(":id", &fr.id)]
                    , |row| -> Uuid {
                        Uuid::from_bytes(&row.get::<i32, Vec<u8>>(0)).expect("failed to parse Uuid from drive db storage")
                    }
                ).and_then(|uuid| -> Result<Uuid, rusqlite::Error> {
                    self.conn.execute("UPDATE files
                                       SET path=$1, mimetype=$2
                                       WHERE uuid=$3"
                        , &[ &(path.to_str().expect("nilfalsdfs"))
                           , &fr.mimeType
                           , &uuid.clone().as_bytes().to_vec()
                           ]
                    ).unwrap_or_else(|err| {
                        println!("couldn't update file in drive db, err: {:?}", err);
                        0
                    });

                    Ok(uuid)
                }).unwrap_or_else(|_| {
                    let uuid = Uuid::new_v4();
                    self.conn.execute("INSERT INTO files (uuid, id, mimetype, path)
                                       VALUES ($1, $2, $3, $4)"
                        , &[ &uuid.clone().as_bytes().to_vec()
                           , &fr.id
                           , &fr.mimeType
                           , &(path.to_str().expect("fadsfnjfsad"))
                           ]
                    ).unwrap_or_else(|_| {
                        println!("file already in drive db: {}", fr.name);
                        0
                    });

                    uuid
                });

                {
                    self.uuid_map.insert(uuid, fr.clone());
                }

                assert!(fr.parents.len() == 1);
                let parent_uuid = self.conn.query_row_named("SELECT uuid FROM files WHERE id=:id"
                    , &[( ":id", &fr.parents[0] )]
                    , |row| -> Uuid {
                        Uuid::from_bytes(&row.get::<i32, Vec<u8>>(0)).expect("failed to parse parent Uuid from drive db storage")
                    }
                ).unwrap();

    //            println!("drive adding path: {:?} {:?}", uuid.clone().as_bytes().to_vec(), path);
                files_list.push(FileResponse {
                    uuid: uuid,
                    parent_uuid: parent_uuid,
                    kind: kind,
                    name: fr.name.clone(),
                    source_data: SourceData::Drive(fr)
                });
            }

            try!(self.conn.execute("INSERT INTO meta (uuid, last_update, num_files_updated)
                                    VALUES ($1, $2, $3)",
                                 &[ &uuid_vec,
                                    &time::now().to_timespec(),
                                    &(files_list.len() as i64)
                                  ]
            ));
        }

        for ref fr in &files[1] {
            self.conn.execute("DELETE FROM files WHERE uuid=:uuid"
                , &[ &fr.uuid.clone().as_bytes().to_vec() ]
            ).unwrap_or_else(|err| {
                println!("couldn't delte file: {}, err: {:?}", fr.name, err);
                0
            });
        }

        Ok(FileUpdates{
            new_files: Some(files[0].clone()),
            deleted_files: Some(files[1].clone())
        })
    }

    fn retreive_file(&mut self, uuid: &Uuid, parent_uuid: &Uuid) -> Result<u64, DriveError> {
        let fr = try!(self.uuid_map.get(uuid).ok_or(DriveError {
            kind: DriveErrorType::FailedUuidLookup,
            response: None,
        })).clone();
        let parent_path = try!(self.uuid_map.get(parent_uuid).ok_or(DriveError {
            kind: DriveErrorType::FailedUuidLookup,
            response: None,
        })).path.clone();

        let mut file_path = try!(parent_path.ok_or(DriveError {
            kind: DriveErrorType::NoPathForParent,
            response: None,
        }));
        file_path.push(fr.name.clone());

//        let (path, mimeType) = try!(self.conn.query_row_named("SELECT path, mimetype FROM files WHERE uuid=:uuid"
//            , &[(":uuid", &uuid.clone().as_bytes().to_vec())]
//            , |row| -> (Option<PathBuf>, Option<String>)
//        ));

        {
            let fr = self.uuid_map.get_mut(uuid).unwrap();
            fr.path = Some(file_path.clone());
            if fr.mimeType == "application/vnd.google-apps.folder" {
                let mut dir_builder = DirBuilder::new();
                dir_builder.recursive(true);
                // create the directory in the system filesystem
                try!(dir_builder.create(&file_path.clone()));
                return Ok(0)
            }
        }

        let (id, maybe_checksum) = try!(self.conn.query_row_named("SELECT id, checksum FROM files WHERE uuid=:uuid"
            , &[(":uuid", &uuid.clone().as_bytes().to_vec())]
            , |row| -> (String, Option<String>) {
                (row.get(0), row.get(1))
            }
        ));
        println!("{:?}, {:?}", id, maybe_checksum);

        let info = if let Some(checksum) = maybe_checksum {
            try!(self.verify_checksum(uuid, &checksum)
            .or_else(|_| -> Result<FileCheckResponse, DriveError> {
                // if the checksum fails, we need to redownload it
                println!("updating metadata for {:?}", file_path);
                let dfi = try!(self.download_file(uuid, parent_uuid));

                self.conn.execute("UPDATE files
                                   SET checksum=$1, size=$2
                                   WHERE uuid=$3",
                                &[ &dfi.checksum,
                                   &(dfi.size as i64),
                                   &uuid.clone().as_bytes().to_vec(),
                                 ]).unwrap();

                self.verify_checksum(uuid, &dfi.checksum)
            }))
        } else {
            // if the checksum fails, we need to redownload it
            println!("updating metadata for {:?}", file_path);
            let dfi = try!(self.download_file(uuid, parent_uuid));

            self.conn.execute("UPDATE files
                               SET checksum=$1, size=$2
                               WHERE uuid=$3",
                            &[ &dfi.checksum,
                               &(dfi.size as i64),
                               &uuid.clone().as_bytes().to_vec(),
                             ]).unwrap();

            try!(self.verify_checksum(uuid, &dfi.checksum))
        };

        println!("shpu;d ne updating");

        Ok(info.size)
    }

    fn create_local_file(&mut self, fd: &FileData, file_path: &Path, metadata_path_str: &Path) -> Result<u64, DriveError> {
        let fr = match fd.source_data {
            SourceData::Drive(ref fr) => fr,
            _ => panic!("fdafasdf")
        };

        let mut dotf = try!(File::create(metadata_path_str.clone()));
        let fcr_string = try!(json::encode(&FileCheckResponse {
            size: 0,
            md5Checksum: "d41d8cd98f00b204e9800998ecf8427e".to_string()
            // seems to be the checksum of an empty file
        }));
        try!(dotf.write_all(&fcr_string.into_bytes()));

        try!(File::create(file_path));
        Ok(0)
    }

    fn read_file(&self, uuid: &Uuid) -> Result<Vec<u8>, DriveError> {
        let path = self.conn.query_row_named("SELECT path FROM files WHERE uuid=:uuid",
            &[(":uuid", &uuid.clone().as_bytes().to_vec())]
            , |row| -> String { row.get(0) }
        ).expect("failure in retrieve_file sql");

        let mut handle = try!(File::open(&path));
        let mut data = Vec::<u8>::new();
        match handle.read_to_end(&mut data) {
            Ok(_) => (),
            Err(error) => println!("couldnt read file handle, {}: error, {}", path, error),
        };

        Ok(data)
    }

    fn verify_checksum(&self, uuid: &Uuid, checksum: &String) -> Result<FileCheckResponse, DriveError> {
        let fr = try!(self.uuid_map.get(uuid).ok_or(DriveError {
            kind: DriveErrorType::FailedUuidLookup,
            response: None
        }));
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
                Err(error) => {
                    println!("could not decode errorDetailsResponse, error: {}, attempted edr: {}", error, i);
                    return Err(From::from(error))
                }
            };

            if err.reason   == "authError"           &&
               err.message  == "Invalid Credentials" {
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
            } else if err.reason  == "userRateLimitExceeded" &&
                      err.message ==  "User Rate Limit Exceeded" {
                          ()
            }
        }

        // for loop returns (), so a value for the function is needed
        Ok(())
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

pub fn convert_timespec_to_tm(ts: Timespec) -> Tm {
    let time_duration = ts - Timespec::new(0,0);
    Tm {
        tm_sec:    0,
        tm_min:    0,
        tm_hour:   0,
        tm_mday:   1, // for some reason, the day is off by 1
        tm_mon:    0,
        tm_year:   70, // for difference b/w UNIX epoch and 1900
        tm_wday:   0,
        tm_yday:   0,
        tm_isdst:  0,
        tm_utcoff: 0,
        tm_nsec:   0,
    } + time_duration
}
